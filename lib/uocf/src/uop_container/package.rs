//! UOP package reader/writer built around the original block-chained container.
//!
//! Binary layout covered by this module:
//! - Package header:
//!   - `"MYP\0"` magic
//!   - `u32 version`
//!   - `u32 misc`
//!   - `u64 start_address`
//!   - `u32 block_size`
//!   - `u32 file_count`
//! - Starting at `start_address`, a linked list of blocks is stored.
//! - Each block contains a small block header plus a run of serialized file entries.
//! - File payload bytes do not live inside the header or inside the block header.
//!   They are stored later in the file, after metadata, and each file entry points
//!   at its own payload through `data_block_address`.
//! - A payload may be preceded by a `data_block_length`-byte data block header
//!   (`data_block_header`): a 12-byte timestamp header in version-4 client packages,
//!   a 134- to 137-byte signature header in version-5 ones. `data_block_hash` is the
//!   Adler-32 (timestamp) or lookup3 hash (signature) of that header. Loading keeps the
//!   header bytes and saving writes them back, so a loaded package re-saves intact.
//!   An entry whose payload `replace_file` changes gets a fresh timestamp header stamped
//!   with the package's edit FILETIME (`set_edit_filetime`), in both versions.
//! - Client packages pad their last block to the block size with all-zero entries; the header's file count
//!   excludes them and saving keeps them all-zero (`UopFile::is_empty_slot`).
//!
//! Client package layouts (all 40 Enhanced Client packages of the reference release):
//! - version 4: a 40-byte header, then each block table followed by its payloads; every table reserves room
//!   for `block_size` records, so the last one is followed by zeros;
//! - version 5: a 512-byte header (bytes 28..36 hold `u32 1`, `u32 1`), then payloads, with each block table
//!   of `block_size` records (empty slots included) somewhere among them.
//!
//! Saving ([`SaveLayout`]):
//! - [`SaveLayout::Preserve`] (the default of [`UopPackage::finalize_and_save`]) keeps the layout of the file
//!   the package was loaded from, so an unchanged package re-saves byte for byte:
//!   - the header bytes after the fixed fields are written back, and the file never gets shorter;
//!   - every block table stays at its address; one that grows stays there when the bytes after its source table
//!     are unused, else it moves to the end of the file;
//!   - every entry stays at its address while its data block header and payload fit in the bytes it had
//!     (replaced entries included); a longer one moves to the end of the file, its old bytes are zeroed;
//!   - removed entries become empty slots and their bytes are zeroed; freed bytes are not reused;
//!   - new blocks and moved entries go after the end of the source file, in block chain and entry order; a new
//!     block table reserves room for `block_size` records, as client packages do;
//!   - bytes no block or payload uses are written as zeros.
//! - [`SaveLayout::Compact`] writes the 32-byte header, every block table and then every payload, without gaps.
//!   Packages built in memory have no source layout and are always saved this way.
//!
//! Entry edits: [`UopPackage::replace_file`], [`UopPackage::add_entry`] (fills the first empty slot, else the
//! last block below `block_size` records, else a new block) and [`UopPackage::remove_entry`] (leaves an empty
//! slot). The client does not depend on the layout (`ENHANCED_CLIENT_FILES.md`).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use indexmap::IndexMap;
use nohash_hasher::BuildNoHashHasher;

use crate::uop_container::block::UopBlock;
use crate::uop_container::data_block_header::{filetime_from_system_time, DataBlockHeader, DataBlockHeaderKind};
use crate::uop_container::file::{CompressionFlag, UopFile};
use crate::uop_container::hash;

const UOP_MAGIC: [u8; 4] = *b"MYP\0";
const MIN_SUPPORTED_VERSION: u32 = 4;
const MAX_SUPPORTED_VERSION: u32 = 5;
const DEFAULT_VERSION: u32 = MAX_SUPPORTED_VERSION;
const DEFAULT_BLOCK_SIZE: u32 = 100;
const PACKAGE_MISC: u32 = 0xFD23_EC43;
const PACKAGE_HEADER_SIZE: u64 = 32;
/// Size of the header fields up to `file_count`; [`SaveLayout::Preserve`] writes back the bytes after them.
const PACKAGE_FIXED_HEADER_SIZE: u64 = 28;
const BLOCK_HEADER_SIZE: u64 = 12;
const FILE_ENTRY_SIZE: u64 = 34;
/// Size of the zero buffer the layout-preserving writer fills unused bytes from.
const ZERO_FILL_CHUNK_SIZE: usize = 64 * 1024;

/// Select how package payload bytes are materialized during load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// Parse metadata only and leave payload bytes on disk until requested.
    Lazy,
    /// Parse metadata and preload all compressed payload bytes immediately.
    Eager,
}

/// What [`UopPackage::replace_file`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReplacement {
    /// The payload or compression changed: the entry was re-encoded with a fresh timestamp header.
    Replaced,
    /// The entry already held the payload with that compression: it was left untouched, header included.
    Unchanged,
}

/// How [`UopPackage::finalize_and_save_with_layout`] places block tables and payloads in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveLayout {
    /// Keep the layout of the file the package was loaded from (see the module documentation); a package built
    /// in memory has none and is saved [`SaveLayout::Compact`].
    Preserve,
    /// The 32-byte header, every block table, then every payload in entry order, without gaps.
    Compact,
}

/// How [`UopPackage::add_entry`] compresses a new entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewEntryCompression {
    /// With this compression. The Enhanced Client reads only [`CompressionFlag::None`] and [`CompressionFlag::Zlib`].
    Flag(CompressionFlag),
    /// Like most entries of the package: uncompressed when more entries are uncompressed than zlib-compressed,
    /// else zlib (also in a package without entries).
    LikePackage,
}

/// One entry of a package, as [`UopPackage::list_entries`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UopEntryInfo {
    pub filename_hash: u64,
    /// The entry's file name, when the caller's resolver knows the hash.
    pub name: Option<String>,
    pub header: DataBlockHeaderKind,
    pub data_block_hash: u32,
    pub compression: CompressionFlag,
    pub compressed_size: u32,
    pub decompressed_size: u32,
    /// The index of the block holding the entry, in block chain order.
    pub block_index: usize,
    /// The entry's slot in its block.
    pub slot_index: usize,
}

/// What [`SaveLayout::Preserve`] needs from the package file a package was loaded from, besides the block and
/// entry addresses kept in `UopBlock::source` and `UopFile::source_extent`.
#[derive(Debug, Clone)]
struct SourceLayout {
    /// The bytes after the fixed header fields, up to the first block table or payload.
    header_tail: Vec<u8>,
    file_length: u64,
}

/// Byte ranges of a package file given to the header, a block table or an entry, keyed by start.
#[derive(Default)]
struct ClaimedRanges {
    ends_by_start: BTreeMap<u64, u64>,
}

impl ClaimedRanges {
    /// Claims `start..end` when it overlaps no claimed range; returns whether it did. An empty range is not
    /// stored and always succeeds.
    fn claim(&mut self, start: u64, end: u64) -> bool {
        if start >= end {
            return true;
        }
        if let Some((_, &previous_end)) = self.ends_by_start.range(..end).next_back() {
            if previous_end > start {
                return false;
            }
        }
        self.ends_by_start.insert(start, end);
        true
    }

    /// The end of the last claimed range, 0 when none is claimed.
    fn end(&self) -> u64 {
        self.ends_by_start.last_key_value().map_or(0, |(_, &end)| end)
    }
}

/// A block table or entry bytes to write, at the offset it is sorted by.
#[derive(Debug, Clone, Copy)]
enum PlacedRegion {
    Block(usize),
    File { block_index: usize, file_index: usize },
}

/// The bytes a block table of `file_count` records occupies.
fn block_table_length(file_count: u64) -> u64 {
    BLOCK_HEADER_SIZE + FILE_ENTRY_SIZE * file_count
}

/// UOP package reader/writer with selectable payload loading and a package-level hash index.
#[derive(Clone)]
pub struct UopPackage {
    /// The version of the UOP package.
    version: u32,
    misc: u32,
    start_address: u64,
    block_size: u32,
    file_count: u32,
    package_path: Option<PathBuf>,
    load_mode: LoadMode,
    blocks: Vec<UopBlock>,
    files_by_hash: IndexMap<u64, UopFile, BuildNoHashHasher<u64>>,
    hash_index_dirty: bool,
    /// FILETIME stamped into the data block header of edited entries; None for the current system time.
    edit_filetime: Option<u64>,
    /// The layout of the file the package was loaded from; None for packages built in memory.
    source_layout: Option<SourceLayout>,
}

impl UopPackage {
    /// Creates a new package with an explicit version and block capacity.
    ///
    /// The writer keeps the historical UOP constraints explicit here instead of
    /// silently clamping values because package versions and per-block capacity
    /// are part of the binary layout, not just runtime tuning knobs.
    pub fn new(version: u32, block_size: u32) -> io::Result<Self> {
        if !(MIN_SUPPORTED_VERSION..=MAX_SUPPORTED_VERSION).contains(&version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unsupported UOP version {version}; expected {}..={}",
                    MIN_SUPPORTED_VERSION, MAX_SUPPORTED_VERSION
                ),
            ));
        }

        if block_size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "block_size must be greater than zero",
            ));
        }

        Ok(Self {
            version,
            misc: PACKAGE_MISC,
            start_address: PACKAGE_HEADER_SIZE,
            block_size,
            file_count: 0,
            package_path: None,
            load_mode: LoadMode::Eager,
            blocks: Vec::new(),
            files_by_hash: IndexMap::with_hasher(BuildNoHashHasher::default()),
            hash_index_dirty: false,
            edit_filetime: None,
            source_layout: None,
        })
    }

    /// Creates a package using the default writer settings.
    pub fn new_default() -> Self {
        Self::new(DEFAULT_VERSION, DEFAULT_BLOCK_SIZE).expect("default UOP package settings are valid")
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::load_with_mode(path, LoadMode::Eager)
    }

    /// Load a package while choosing whether payload bytes are read eagerly or lazily.
    pub fn load_with_mode(path: impl AsRef<Path>, load_mode: LoadMode) -> io::Result<Self> {
        let path = path.as_ref();
        let mut reader = File::open(path)?;

        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != UOP_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid UOP magic",
            ));
        }

        let version = reader.read_u32::<LittleEndian>()?;
        if !(MIN_SUPPORTED_VERSION..=MAX_SUPPORTED_VERSION).contains(&version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported UOP version {version}; expected {}..={}",
                    MIN_SUPPORTED_VERSION, MAX_SUPPORTED_VERSION
                ),
            ));
        }

        let misc = reader.read_u32::<LittleEndian>()?;
        let start_address = reader.read_u64::<LittleEndian>()?;
        let block_size = reader.read_u32::<LittleEndian>()?;
        let file_count = reader.read_u32::<LittleEndian>()?;

        if start_address == 0 && file_count != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "non-empty UOP package has no first block address",
            ));
        }

        let mut blocks = Vec::new();
        let mut next_block_address = start_address;

        while next_block_address != 0 {
            reader.seek(SeekFrom::Start(next_block_address))?;
            let mut block = UopBlock::read(&mut reader)?;
            next_block_address = block.next_block_address();
            if load_mode == LoadMode::Eager {
                block.preload_data(&mut reader)?;
            }
            blocks.push(block);
        }

        // The header's bytes after the fixed fields run up to the first block table or payload.
        let file_length = reader.metadata()?.len();
        let first_region = blocks
            .iter()
            .filter_map(UopBlock::source)
            .map(|source| source.address)
            .chain(
                blocks
                    .iter()
                    .flat_map(|block| block.files().iter())
                    .filter_map(UopFile::source_extent)
                    .map(|extent| extent.address),
            )
            .min()
            .unwrap_or(file_length);
        let header_end = first_region.min(file_length).max(PACKAGE_FIXED_HEADER_SIZE);
        let mut header_tail = vec![0u8; (header_end - PACKAGE_FIXED_HEADER_SIZE) as usize];
        reader.seek(SeekFrom::Start(PACKAGE_FIXED_HEADER_SIZE))?;
        reader.read_exact(&mut header_tail)?;

        let mut package = Self {
            version,
            misc,
            start_address,
            block_size,
            file_count,
            package_path: Some(path.to_path_buf()),
            load_mode,
            blocks,
            files_by_hash: IndexMap::with_hasher(BuildNoHashHasher::default()),
            hash_index_dirty: false,
            edit_filetime: None,
            source_layout: Some(SourceLayout { header_tail, file_length }),
        };
        package.refresh_hash_index();
        Ok(package)
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    /// Return the configured payload load mode for this package instance.
    pub fn load_mode(&self) -> LoadMode {
        self.load_mode
    }

    /// Return the maximum number of file records each block may hold.
    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Sets the FILETIME stamped into the data block header of the entries replaced or added from now on
    /// ([`Self::edit_data_block_header`]); None (the default) stamps the current system time. A fixed value
    /// makes edited packages reproducible.
    pub fn set_edit_filetime(&mut self, filetime: Option<u64>) {
        self.edit_filetime = filetime;
    }

    /// The FILETIME stamped into edited entries: the one set by [`Self::set_edit_filetime`], else now.
    pub fn edit_filetime(&self) -> u64 {
        self.edit_filetime.unwrap_or_else(|| filetime_from_system_time(std::time::SystemTime::now()))
    }

    /// The data block header of an entry replaced or added in this package: one timestamp record holding
    /// [`Self::edit_filetime`], whatever the package version. A version-5 signature cannot be made for a new
    /// payload, and the new FILETIME changes the `data_block_hash` the client uses as a cache stamp.
    pub fn edit_data_block_header(&self) -> DataBlockHeader {
        DataBlockHeader::from_timestamp(self.edit_filetime())
    }

    /// Return the total logical file count advertised by the package.
    pub fn file_count(&self) -> u32 {
        self.file_count
    }

    /// Expose the parsed block list for tooling and tests.
    pub fn blocks(&self) -> &Vec<UopBlock> {
        &self.blocks
    }

    /// Expose mutable block access for low-level package editing tools.
    pub fn blocks_mut(&mut self) -> &mut Vec<UopBlock> {
        self.hash_index_dirty = true;
        &mut self.blocks
    }

    /// Return the package-owned hash index keyed by filename hash.
    pub fn files_by_hash(&self) -> &IndexMap<u64, UopFile, BuildNoHashHasher<u64>> {
        &self.files_by_hash
    }

    /// Iterate over all logical file entries in block order.
    pub fn iter_files(&self) -> impl Iterator<Item = &UopFile> {
        self.blocks.iter().flat_map(|block| block.files().iter())
    }

    /// Iterate mutably over all logical file entries in block order.
    pub fn iter_files_mut(&mut self) -> impl Iterator<Item = &mut UopFile> {
        self.hash_index_dirty = true;
        self.blocks.iter_mut().flat_map(|block| block.files_mut().iter_mut())
    }

    /// Find a file entry by its normalized filename hash.
    pub fn get_file_by_hash(&self, filename_hash: u64) -> Option<&UopFile> {
        if self.hash_index_dirty {
            return self
                .blocks
                .iter()
                .flat_map(|block| block.files().iter())
                .find(|file| file.filename_hash() == filename_hash);
        }

        self.files_by_hash.get(&filename_hash)
    }

    /// Find the first file entry matching one of the provided hashes.
    ///
    /// Candidate order is preserved, so callers can list preferred paths first
    /// and still hash or probe those candidates as one batch.
    pub fn get_file_by_hash_batch(&self, filename_hashes: &[u64]) -> Option<(usize, u64, &UopFile)> {
        for (index, &filename_hash) in filename_hashes.iter().enumerate() {
            if let Some(file) = self.get_file_by_hash(filename_hash) {
                return Some((index, filename_hash, file));
            }
        }

        None
    }

    fn file_position_by_hash(&self, filename_hash: u64) -> Option<(usize, usize)> {
        for (block_index, block) in self.blocks.iter().enumerate() {
            for (file_index, file) in block.files().iter().enumerate() {
                if file.filename_hash() == filename_hash {
                    return Some((block_index, file_index));
                }
            }
        }
        None
    }

    /// Find a mutable file entry by its normalized filename hash.
    pub fn get_file_by_hash_mut(&mut self, filename_hash: u64) -> Option<&mut UopFile> {
        self.hash_index_dirty = true;
        for block in &mut self.blocks {
            for file in block.files_mut() {
                if file.filename_hash() == filename_hash {
                    return Some(file);
                }
            }
        }
        None
    }

    /// Find a file entry by its internal packed path.
    ///
    /// This is a small convenience wrapper over `get_file_by_hash` for callers
    /// that still have the original UOP logical path string available.
    pub fn get_file_by_name(&self, packed_file_name: &str) -> Option<&UopFile> {
        self.get_file_by_hash(hash::hash_file_name_single(packed_file_name))
    }

    /// Find the first file entry matching one of the provided internal packed paths.
    ///
    /// The caller-owned scratch buffer is resized and filled with SIMD-batched
    /// UOP filename hashes before probing the package index.
    pub fn get_file_by_name_batch<'a>(
        &'a self,
        packed_file_names: &[&str],
        scratch_hashes: &mut Vec<u64>,
    ) -> Option<(usize, u64, &'a UopFile)> {
        scratch_hashes.resize(packed_file_names.len(), 0);
        hash::hash_file_name_simd_batch_strs_into(packed_file_names, scratch_hashes);
        self.get_file_by_hash_batch(scratch_hashes)
    }

    /// Read bytes from an arbitrary source and append them as one UOP file.
    ///
    /// This is the most general high-level insertion helper. The reader is
    /// consumed immediately, the payload is compressed according to the chosen
    /// legacy compression flag, and the resulting `UopFile` is added to the
    /// final block chain.
    pub fn add_file_from_reader(
        &mut self,
        reader: &mut impl Read,
        packed_file_name: &str,
        compression: CompressionFlag,
    ) -> io::Result<()> {
        // UOP indexes files by a pre-hashed normalized internal path.
        let filename_hash = hash::hash_file_name_single(packed_file_name);
        let file = UopFile::new().create_file(reader, filename_hash, compression)?;
        self.push_file(file);
        Ok(())
    }

    /// Append an in-memory byte slice as one UOP file.
    pub fn add_file_from_memory(
        &mut self,
        file_content: &[u8],
        packed_file_name: &str,
        compression: CompressionFlag,
    ) -> io::Result<()> {
        let filename_hash = hash::hash_file_name_single(packed_file_name);
        let file = UopFile::new().create_file_from_bytes(file_content, filename_hash, compression)?;
        self.push_file(file);
        Ok(())
    }

    /// Replaces the payload of the entry `filename_hash`, encoding it with `compression`.
    ///
    /// A changed entry gets [`Self::edit_data_block_header`] and its hash. An entry that already holds
    /// `payload` with `compression` is left untouched, keeping its header and hash, so re-saving unchanged
    /// content does not change its stamp. Fails with `NotFound` when there is no such entry.
    pub fn replace_file(
        &mut self,
        filename_hash: u64,
        payload: &[u8],
        compression: CompressionFlag,
    ) -> io::Result<FileReplacement> {
        self.ensure_file_data_loaded_by_hash(filename_hash)?;
        let header = self.edit_data_block_header();
        let file = self.get_file_by_hash_mut(filename_hash).expect("the entry was just loaded");
        if file.compression() == compression {
            // Entries without stored bytes have nothing to load: their payload is empty. An entry that does not
            // decode counts as changed, so it can still be replaced.
            let unchanged =
                file.data().map_or(payload.is_empty(), |_| file.unpack().is_ok_and(|current| current == payload));
            if unchanged {
                return Ok(FileReplacement::Unchanged);
            }
        }
        let mut replacement = UopFile::new().create_file_from_bytes(payload, filename_hash, compression)?;
        replacement.set_data_block_header_and_hash(&header)?;
        replacement.set_source_extent(file.source_extent());
        *file = replacement;
        Ok(FileReplacement::Replaced)
    }

    /// Adds the entry `filename_hash` holding `payload`, with [`Self::edit_data_block_header`] and its hash.
    ///
    /// The entry fills the first empty slot in block chain order, else goes at the end of the last block when it
    /// holds fewer than `block_size` records, else opens a new block. Fails with `AlreadyExists` when the package
    /// has an entry `filename_hash` (the client reads only the first of duplicates) and with `InvalidInput` for an
    /// empty payload (the client fails to read entries of size zero).
    pub fn add_entry(
        &mut self,
        filename_hash: u64,
        payload: &[u8],
        compression: NewEntryCompression,
    ) -> io::Result<()> {
        if payload.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("entry {filename_hash:016X} has an empty payload"),
            ));
        }
        if self.get_file_by_hash(filename_hash).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("the package already has an entry {filename_hash:016X}"),
            ));
        }
        let compression = self.new_entry_compression(compression);
        let mut file = UopFile::new().create_file_from_bytes(payload, filename_hash, compression)?;
        file.set_data_block_header_and_hash(&self.edit_data_block_header())?;
        self.insert_entry(file);
        Ok(())
    }

    /// [`Self::add_entry`] for the entry named `packed_file_name`; returns its hash.
    pub fn add_entry_by_name(
        &mut self,
        packed_file_name: &str,
        payload: &[u8],
        compression: NewEntryCompression,
    ) -> io::Result<u64> {
        let filename_hash = hash::hash_file_name_single(packed_file_name);
        self.add_entry(filename_hash, payload, compression)?;
        Ok(filename_hash)
    }

    /// Removes the entry `filename_hash` (every entry with that hash); returns whether one existed. Its slot
    /// becomes an empty slot, as in client packages, so the other entries keep their blocks and slots.
    pub fn remove_entry(&mut self, filename_hash: u64) -> bool {
        let mut removed = 0u32;
        for file in self
            .blocks
            .iter_mut()
            .flat_map(|block| block.files_mut().iter_mut())
            .filter(|file| file.filename_hash() == filename_hash && !file.is_empty_slot())
        {
            *file = UopFile::new();
            removed += 1;
        }
        if removed == 0 {
            return false;
        }
        self.files_by_hash.shift_remove(&filename_hash);
        self.file_count = self.file_count.saturating_sub(removed);
        true
    }

    /// Lists the entries in block chain and slot order, empty slots excluded. `name_of` gives the name of a hash
    /// when known (for instance `|hash| dictionary.resolve(hash).map(str::to_owned)` with a `HashDictionary`).
    /// The data block headers of a lazily loaded package are read from its file.
    pub fn list_entries(&self, mut name_of: impl FnMut(u64) -> Option<String>) -> io::Result<Vec<UopEntryInfo>> {
        let mut reader: Option<File> = None;
        let mut entries = Vec::new();
        for (block_index, block) in self.blocks.iter().enumerate() {
            for (slot_index, file) in block.files().iter().enumerate() {
                if file.is_empty_slot() {
                    continue;
                }
                let header = match file.data_block_header() {
                    Some(bytes) => DataBlockHeaderKind::of_bytes(bytes),
                    None if file.data_block_length() == 0 => DataBlockHeaderKind::None,
                    None => {
                        if reader.is_none() {
                            reader = Some(self.open_payload_reader()?);
                        }
                        let reader = reader.as_mut().expect("the reader was just opened");
                        reader.seek(SeekFrom::Start(file.data_block_address()))?;
                        let mut bytes = vec![0u8; file.data_block_length() as usize];
                        reader.read_exact(&mut bytes)?;
                        DataBlockHeaderKind::of_bytes(&bytes)
                    }
                };
                entries.push(UopEntryInfo {
                    filename_hash: file.filename_hash(),
                    name: name_of(file.filename_hash()),
                    header,
                    data_block_hash: file.data_block_hash(),
                    compression: file.compression(),
                    compressed_size: file.compressed_size(),
                    decompressed_size: file.decompressed_size(),
                    block_index,
                    slot_index,
                });
            }
        }
        Ok(entries)
    }

    /// Finalize layout-sensitive fields and write a complete package to disk, keeping the layout of the file the
    /// package was loaded from ([`SaveLayout::Preserve`]).
    pub fn finalize_and_save(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        self.finalize_and_save_with_layout(path, SaveLayout::Preserve)
    }

    /// Finalize layout-sensitive fields and write a complete package to disk with the chosen `layout`.
    pub fn finalize_and_save_with_layout(&mut self, path: impl AsRef<Path>, layout: SaveLayout) -> io::Result<()> {
        self.ensure_all_data_loaded()?;
        if let (SaveLayout::Preserve, Some(source)) = (layout, self.source_layout.clone()) {
            self.file_count = self.entry_count();
            let (block_addresses, file_length) = self.plan_preserved_layout(&source);
            self.refresh_hash_index();
            let mut writer = BufWriter::new(File::create(path)?);
            self.write_preserved(&mut writer, &source.header_tail, &block_addresses, file_length)?;
            return writer.flush();
        }

        self.recompute_layout();
        self.refresh_hash_index();

        let mut writer = File::create(path)?;
        self.write_header(&mut writer)?;
        self.write_blocks(&mut writer)?;
        self.write_payloads(&mut writer)
    }

    /// Load one payload into memory when this package was opened lazily.
    pub fn ensure_file_data_loaded_by_hash(&mut self, filename_hash: u64) -> io::Result<()> {
        let (block_index, file_index) = self
            .file_position_by_hash(filename_hash)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("file payload for hash {filename_hash:016X} not found"),
                )
            })?;

        let needs_load = {
            let file = &self.blocks[block_index].files()[file_index];
            file.data().is_none() && file.has_size()
        };
        if !needs_load {
            return Ok(());
        }

        let update_clean_hash_index = !self.hash_index_dirty;
        let package_path = self.package_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "package has no backing file path for lazy payload loading",
            )
        })?;
        let mut reader = File::open(package_path)?;
        let indexed_file = {
            let file = &mut self.blocks[block_index].files_mut()[file_index];
            file.load_data_from(&mut reader)?;
            file.clone()
        };
        if update_clean_hash_index {
            self.files_by_hash.insert(filename_hash, indexed_file);
        }

        Ok(())
    }

    /// Unpack one file payload by hash, caching compressed payload bytes in this
    /// package when it was opened lazily.
    pub fn unpack_file_by_hash_cached(&mut self, filename_hash: u64) -> io::Result<Option<Vec<u8>>> {
        if self.get_file_by_hash(filename_hash).is_none() {
            return Ok(None);
        }
        self.ensure_file_data_loaded_by_hash(filename_hash)?;
        self.get_file_by_hash(filename_hash)
            .expect("file payload was just checked")
            .unpack()
            .map(Some)
    }

    /// Open a reader positioned by per-entry payload offsets for repeated lazy
    /// payload reads.
    pub fn open_payload_reader(&self) -> io::Result<File> {
        let package_path = self.package_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "package has no backing file path for lazy payload loading",
            )
        })?;
        File::open(package_path)
    }

    /// Unpack one file payload by hash using a caller-owned package reader.
    ///
    /// This avoids reopening the package for each payload while keeping the
    /// package itself immutable, which is useful for parallel metadata scans.
    pub fn unpack_file_by_hash_from_reader<R: Read + Seek>(
        &self,
        filename_hash: u64,
        reader: &mut R,
    ) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.get_file_by_hash(filename_hash) else {
            return Ok(None);
        };

        let mut file = file.clone();
        if file.data().is_none() && file.has_size() {
            file.load_data_from(reader)?;
        }

        file.unpack().map(Some)
    }

    /// Unpack one file payload by hash into shared bytes without storing the
    /// payload in this package.
    ///
    /// For uncompressed entries this can share the loaded payload bytes directly
    /// with the caller instead of copying them into a fresh `Vec`.
    pub fn unpack_file_arc_by_hash(&self, filename_hash: u64) -> io::Result<Option<Arc<[u8]>>> {
        let Some(file) = self.get_file_by_hash(filename_hash) else {
            return Ok(None);
        };

        let mut file = file.clone();
        if file.data().is_none() && file.has_size() {
            let package_path = self.package_path.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "package has no backing file path for lazy payload loading",
                )
            })?;
            let mut reader = File::open(package_path)?;
            file.load_data_from(&mut reader)?;
        }

        file.unpack_arc().map(Some)
    }

    /// Unpack one file payload by hash without storing the payload in this package.
    ///
    /// This is intended for inspector and tooling paths that keep a package open
    /// lazily and want to read only the selected entry from disk.
    pub fn unpack_file_by_hash(&self, filename_hash: u64) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.get_file_by_hash(filename_hash) else {
            return Ok(None);
        };

        let mut file = file.clone();
        if file.data().is_none() && file.has_size() {
            let package_path = self.package_path.clone().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "package has no backing file path for lazy payload loading",
                )
            })?;
            let mut reader = File::open(package_path)?;
            file.load_data_from(&mut reader)?;
        }

        file.unpack().map(Some)
    }

    /// Load all payloads into memory when the package was opened lazily.
    pub fn ensure_all_data_loaded(&mut self) -> io::Result<()> {
        if !self.iter_files().any(|file| file.has_size() && file.data().is_none()) {
            return Ok(());
        }

        let package_path = self.package_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "package has no backing file path for lazy payload loading",
            )
        })?;

        let mut reader = File::open(package_path)?;
        for block in &mut self.blocks {
            block.preload_data(&mut reader)?;
        }

        self.refresh_hash_index();
        Ok(())
    }

    /// Rebuild compressed payloads in-place using the current package file set.
    ///
    /// The payloads do not change, so every entry keeps its data block header and hash.
    pub fn recompress(&mut self, _compression_level: flate2::Compression) -> io::Result<()> {
        self.ensure_all_data_loaded()?;

        for file in self.iter_files_mut().filter(|file| !file.is_empty_slot()) {
            let unpacked = file.unpack_arc()?;
            let filename_hash = file.filename_hash();
            let recompressed_flag = file.compression();

            let mut recompressed = UopFile::new().create_file_from_bytes(
                unpacked.as_ref(),
                filename_hash,
                recompressed_flag,
            )?;
            if let Some(header) = file.data_block_header() {
                recompressed.set_data_block_header(Arc::clone(header));
                recompressed.set_data_block_hash(file.data_block_hash());
            }
            recompressed.set_source_extent(file.source_extent());
            *file = recompressed;
        }

        self.refresh_hash_index();
        Ok(())
    }

    fn push_file(&mut self, file: UopFile) {
        // UOP blocks are fixed-capacity groups. Once the active block reaches the
        // advertised capacity, the writer starts a new block and updates the file
        // count at package scope.
        let needs_new_block = self
            .blocks
            .last()
            .map(|block| block.files().len() >= self.block_size as usize)
            .unwrap_or(true);

        if needs_new_block {
            self.blocks.push(UopBlock::new());
        }

        self.blocks
            .last_mut()
            .expect("a block exists after allocation")
            .add_file(file.clone());
        self.files_by_hash.insert(file.filename_hash(), file);
        self.file_count += 1;
        self.hash_index_dirty = false;
    }

    /// The compression [`Self::add_entry`] gives a new entry.
    pub fn new_entry_compression(&self, compression: NewEntryCompression) -> CompressionFlag {
        match compression {
            NewEntryCompression::Flag(flag) => flag,
            NewEntryCompression::LikePackage => {
                let (mut uncompressed, mut zlib) = (0usize, 0usize);
                for file in self.iter_files().filter(|file| !file.is_empty_slot()) {
                    match file.compression() {
                        CompressionFlag::None => uncompressed += 1,
                        CompressionFlag::Zlib => zlib += 1,
                        CompressionFlag::Mythic | CompressionFlag::ZlibBwt => {}
                    }
                }
                if uncompressed > zlib {
                    CompressionFlag::None
                } else {
                    CompressionFlag::Zlib
                }
            }
        }
    }

    /// Puts `file` in the first empty slot, else at the end of the last block below `block_size` records, else
    /// in a new block.
    fn insert_entry(&mut self, file: UopFile) {
        if !self.hash_index_dirty {
            self.files_by_hash.insert(file.filename_hash(), file.clone());
        }
        self.file_count += 1;
        let block_size = self.block_size as usize;
        if let Some(slot) =
            self.blocks.iter_mut().flat_map(|block| block.files_mut().iter_mut()).find(|file| file.is_empty_slot())
        {
            *slot = file;
            return;
        }
        match self.blocks.last_mut() {
            Some(block) if block.files().len() < block_size => block.add_file(file),
            _ => {
                let mut block = UopBlock::new();
                block.add_file(file);
                self.blocks.push(block);
            }
        }
    }

    /// The number of entries, empty slots excluded: the header's `file_count`.
    fn entry_count(&self) -> u32 {
        self.iter_files().filter(|file| !file.is_empty_slot()).count() as u32
    }

    /// Gives every block and entry its address under [`SaveLayout::Preserve`] (see the module documentation) and
    /// links the block chain. Returns the block addresses and the file length.
    fn plan_preserved_layout(&mut self, source: &SourceLayout) -> (Vec<u64>, u64) {
        let mut claimed = ClaimedRanges::default();
        claimed.claim(0, PACKAGE_FIXED_HEADER_SIZE + source.header_tail.len() as u64);

        // Block tables stay where they were, at their source size.
        let mut block_addresses: Vec<Option<u64>> = self
            .blocks
            .iter()
            .map(|block| {
                block
                    .source()
                    .filter(|source| {
                        let end = source.address + block_table_length(u64::from(source.file_count));
                        claimed.claim(source.address, end)
                    })
                    .map(|source| source.address)
            })
            .collect();

        // Entries stay where they were while their bytes fit there.
        let mut unplaced_files = Vec::new();
        for (block_index, block) in self.blocks.iter_mut().enumerate() {
            for (file_index, file) in block.files_mut().iter_mut().enumerate() {
                if file.is_empty_slot() {
                    file.set_data_block_address(0);
                    continue;
                }
                // An entry that stores no bytes keeps its address.
                let length = file.stored_length();
                if length == 0 {
                    continue;
                }
                match file.source_extent() {
                    Some(extent) if length <= extent.length && claimed.claim(extent.address, extent.end()) => {
                        file.set_data_block_address(extent.address);
                    }
                    _ => unplaced_files.push((block_index, file_index)),
                }
            }
        }

        // A block table that grew stays when the bytes after its source table are unused.
        for (block, address) in self.blocks.iter().zip(block_addresses.iter_mut()) {
            let (Some(source), Some(start)) = (block.source(), *address) else {
                continue;
            };
            let source_end = start + block_table_length(u64::from(source.file_count));
            let end = start + block_table_length(block.files().len() as u64);
            if end > source_end && !claimed.claim(source_end, end) {
                *address = None;
            }
        }

        // New and moved block tables and entries go after the source file, in block chain and entry order.
        let mut cursor = source.file_length.max(claimed.end());
        let mut unplaced_files = unplaced_files.into_iter().peekable();
        for (block_index, address) in block_addresses.iter_mut().enumerate() {
            let block = &mut self.blocks[block_index];
            if address.is_none() {
                *address = Some(cursor);
                let capacity = (block.files().len() as u64).max(u64::from(self.block_size));
                cursor += block_table_length(capacity);
            }
            while let Some((_, file_index)) = unplaced_files.next_if(|(index, _)| *index == block_index) {
                let file = &mut block.files_mut()[file_index];
                file.set_data_block_address(cursor);
                cursor += file.stored_length();
            }
        }

        let block_addresses: Vec<u64> =
            block_addresses.into_iter().map(|address| address.expect("every block is placed")).collect();
        self.start_address = block_addresses.first().copied().unwrap_or(0);
        for (index, block) in self.blocks.iter_mut().enumerate() {
            block.set_next_block_address(block_addresses.get(index + 1).copied().unwrap_or(0));
        }
        (block_addresses, cursor)
    }

    /// Writes the package laid out by [`Self::plan_preserved_layout`], filling unused bytes with zeros.
    fn write_preserved<W: Write>(
        &self,
        writer: &mut W,
        header_tail: &[u8],
        block_addresses: &[u64],
        file_length: u64,
    ) -> io::Result<()> {
        let mut regions: Vec<(u64, PlacedRegion)> =
            block_addresses.iter().enumerate().map(|(index, &address)| (address, PlacedRegion::Block(index))).collect();
        for (block_index, block) in self.blocks.iter().enumerate() {
            for (file_index, file) in block.files().iter().enumerate() {
                if !file.is_empty_slot() && file.stored_length() != 0 {
                    regions.push((file.data_block_address(), PlacedRegion::File { block_index, file_index }));
                }
            }
        }
        regions.sort_by_key(|(address, _)| *address);

        self.write_header_fields(writer)?;
        writer.write_all(header_tail)?;
        let mut position = PACKAGE_FIXED_HEADER_SIZE + header_tail.len() as u64;
        for (address, region) in regions {
            let Some(gap) = address.checked_sub(position) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("package regions overlap at offset {address}"),
                ));
            };
            write_zeros(writer, gap)?;
            position = address
                + match region {
                    PlacedRegion::Block(index) => {
                        let block = &self.blocks[index];
                        self.write_block_table(writer, block)?;
                        block_table_length(block.files().len() as u64)
                    }
                    PlacedRegion::File { block_index, file_index } => {
                        let file = &self.blocks[block_index].files()[file_index];
                        write_stored_bytes(writer, file)?;
                        file.stored_length()
                    }
                };
        }
        write_zeros(writer, file_length.saturating_sub(position))
    }

    fn refresh_hash_index(&mut self) {
        let mut files_by_hash = IndexMap::with_hasher(BuildNoHashHasher::default());
        for file in self.iter_files() {
            files_by_hash.insert(file.filename_hash(), file.clone());
        }
        self.files_by_hash = files_by_hash;
        self.hash_index_dirty = false;
    }

    fn recompute_layout(&mut self) {
        self.file_count = self.iter_files().filter(|file| !file.is_empty_slot()).count() as u32;

        if self.blocks.is_empty() {
            self.start_address = 0;
            return;
        }

        // UOP stores all block tables first and appends payload blobs after the
        // last block table. Recomputing both regions together keeps the writer
        // deterministic and makes patching tests stable.
        let mut block_addresses = Vec::with_capacity(self.blocks.len());
        let mut offset = PACKAGE_HEADER_SIZE;

        for block in &self.blocks {
            block_addresses.push(offset);
            offset += BLOCK_HEADER_SIZE + FILE_ENTRY_SIZE * block.files().len() as u64;
        }

        let mut payload_offset = offset;
        self.start_address = block_addresses[0];

        for (index, block) in self.blocks.iter_mut().enumerate() {
            let next_block_address = block_addresses.get(index + 1).copied().unwrap_or(0);
            block.set_next_block_address(next_block_address);

            for file in block.files_mut() {
                if file.is_empty_slot() {
                    // Unused slots stay all-zero, as in client packages.
                    file.set_data_block_address(0);
                    continue;
                }
                file.set_data_block_address(payload_offset);
                payload_offset += file.data_block_length() as u64 + file.compressed_size() as u64;
            }
        }
    }

    fn write_header(&self, writer: &mut File) -> io::Result<()> {
        // The final reserved `u32` is kept at zero to preserve the historical
        // 32-byte package header layout expected by existing tooling.
        self.write_header_fields(writer)?;
        writer.write_u32::<LittleEndian>(0)?;
        Ok(())
    }

    /// Writes the [`PACKAGE_FIXED_HEADER_SIZE`] bytes of header fields.
    fn write_header_fields<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&UOP_MAGIC)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.misc)?;
        writer.write_u64::<LittleEndian>(self.start_address)?;
        writer.write_u32::<LittleEndian>(self.block_size)?;
        writer.write_u32::<LittleEndian>(self.file_count)?;
        Ok(())
    }

    fn write_blocks(&self, writer: &mut File) -> io::Result<()> {
        for block in &self.blocks {
            self.write_block_table(writer, block)?;
        }

        Ok(())
    }

    fn write_block_table<W: Write>(&self, writer: &mut W, block: &UopBlock) -> io::Result<()> {
        writer.write_u32::<LittleEndian>(block.files().len() as u32)?;
        writer.write_u64::<LittleEndian>(block.next_block_address())?;

        for file in block.files() {
            // The legacy metadata record matches the on-disk layout read by
            // `UopFile::read` exactly, so this writer stays byte-for-byte
            // compatible with the existing loader.
            writer.write_u64::<LittleEndian>(file.data_block_address())?;
            writer.write_u32::<LittleEndian>(file.data_block_length())?;
            writer.write_u32::<LittleEndian>(file.compressed_size())?;
            writer.write_u32::<LittleEndian>(file.decompressed_size())?;
            writer.write_u64::<LittleEndian>(file.filename_hash())?;
            writer.write_u32::<LittleEndian>(file.data_block_hash())?;
            writer.write_i16::<LittleEndian>(file.compression() as i16)?;
        }

        Ok(())
    }

    fn write_payloads(&self, writer: &mut File) -> io::Result<()> {
        // Payloads are emitted in the same order used during layout computation,
        // so every previously assigned `data_block_address` remains valid.
        for file in self.iter_files().filter(|file| !file.is_empty_slot()) {
            write_stored_bytes(writer, file)?;
        }

        Ok(())
    }
}

/// Writes an entry's stored bytes: the data block header loaded from a source package, if any, then the payload.
fn write_stored_bytes<W: Write>(writer: &mut W, file: &UopFile) -> io::Result<()> {
    if file.data_block_length() != 0 {
        let header = file
            .data_block_header()
            .filter(|header| header.len() == file.data_block_length() as usize)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "data block header for hash {:016X} has not been loaded into memory",
                        file.filename_hash()
                    ),
                )
            })?;
        writer.write_all(header)?;
    }
    let data = file.data().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "file payload for hash {:016X} has not been loaded into memory",
                file.filename_hash()
            ),
        )
    })?;
    writer.write_all(data)
}

/// Writes `count` zero bytes.
fn write_zeros<W: Write>(writer: &mut W, count: u64) -> io::Result<()> {
    static ZEROS: [u8; ZERO_FILL_CHUNK_SIZE] = [0; ZERO_FILL_CHUNK_SIZE];
    let mut remaining = count;
    while remaining != 0 {
        let chunk = remaining.min(ZERO_FILL_CHUNK_SIZE as u64) as usize;
        writer.write_all(&ZEROS[..chunk])?;
        remaining -= chunk as u64;
    }
    Ok(())
}
