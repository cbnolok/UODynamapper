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

use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use indexmap::IndexMap;
use nohash_hasher::BuildNoHashHasher;

use crate::uop::block::UopBlock;
use crate::uop::file::{CompressionFlag, UopFile};
use crate::uop::hash;

const UOP_MAGIC: [u8; 4] = *b"MYP\0";
const MIN_SUPPORTED_VERSION: u32 = 4;
const MAX_SUPPORTED_VERSION: u32 = 5;
const DEFAULT_VERSION: u32 = MAX_SUPPORTED_VERSION;
const DEFAULT_BLOCK_SIZE: u32 = 100;
const PACKAGE_MISC: u32 = 0xFD23_EC43;
const PACKAGE_HEADER_SIZE: u64 = 32;
const BLOCK_HEADER_SIZE: u64 = 12;
const FILE_ENTRY_SIZE: u64 = 34;

/// Select how package payload bytes are materialized during load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    /// Parse metadata only and leave payload bytes on disk until requested.
    Lazy,
    /// Parse metadata and preload all compressed payload bytes immediately.
    Eager,
}

/// UOP package reader/writer with selectable payload loading and a package-level hash index.
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
        self.add_file_from_reader(&mut Cursor::new(file_content), packed_file_name, compression)
    }

    /// Finalize layout-sensitive fields and write a complete package to disk.
    pub fn finalize_and_save(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        self.ensure_all_data_loaded()?;
        self.recompute_layout();
        self.refresh_hash_index();

        let mut writer = File::create(path)?;
        self.write_header(&mut writer)?;
        self.write_blocks(&mut writer)?;
        self.write_payloads(&mut writer)
    }

    /// Load one payload into memory when this package was opened lazily.
    pub fn ensure_file_data_loaded_by_hash(&mut self, filename_hash: u64) -> io::Result<()> {
        let package_path = self.package_path.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "package has no backing file path for lazy payload loading",
            )
        })?;

        let mut reader = File::open(package_path)?;
        let mut loaded = false;
        {
            let file = self.get_file_by_hash_mut(filename_hash).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("file payload for hash {filename_hash:016X} not found"),
                )
            })?;
            if file.data().is_none() && file.has_size() {
                file.load_data_from(&mut reader)?;
                loaded = true;
            }
        }

        if loaded {
            self.refresh_hash_index();
        }

        Ok(())
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
    pub fn recompress(&mut self, _compression_level: flate2::Compression) -> io::Result<()> {
        self.ensure_all_data_loaded()?;

        for file in self.iter_files_mut() {
            let unpacked = file.unpack()?;
            let recompressed_flag = match file.compression() {
                CompressionFlag::None | CompressionFlag::Zlib => CompressionFlag::Zlib,
                CompressionFlag::Mythic => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Mythic recompression is not implemented",
                    ));
                }
                CompressionFlag::ZlibBwt => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "ZlibBwt recompression is not implemented",
                    ));
                }
            };

            *file = UopFile::new().create_file(
                &mut Cursor::new(unpacked),
                file.filename_hash(),
                recompressed_flag,
            )?;
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

    fn refresh_hash_index(&mut self) {
        let mut files_by_hash = IndexMap::with_hasher(BuildNoHashHasher::default());
        for file in self.iter_files() {
            files_by_hash.insert(file.filename_hash(), file.clone());
        }
        self.files_by_hash = files_by_hash;
        self.hash_index_dirty = false;
    }

    fn recompute_layout(&mut self) {
        self.file_count = self.blocks.iter().map(|block| block.files().len() as u32).sum();

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
                file.set_data_block_address(payload_offset);
                payload_offset += file.compressed_size() as u64;
            }
        }
    }

    fn write_header(&self, writer: &mut File) -> io::Result<()> {
        // The final reserved `u32` is kept at zero to preserve the historical
        // 32-byte package header layout expected by existing tooling.
        writer.write_all(&UOP_MAGIC)?;
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.misc)?;
        writer.write_u64::<LittleEndian>(self.start_address)?;
        writer.write_u32::<LittleEndian>(self.block_size)?;
        writer.write_u32::<LittleEndian>(self.file_count)?;
        writer.write_u32::<LittleEndian>(0)?;
        Ok(())
    }

    fn write_blocks(&self, writer: &mut File) -> io::Result<()> {
        for block in &self.blocks {
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
        }

        Ok(())
    }

    fn write_payloads(&self, writer: &mut File) -> io::Result<()> {
        // Payloads are emitted in the same order used during layout computation,
        // so every previously assigned `data_block_address` remains valid.
        for file in self.iter_files() {
            let data = file.data().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "file payload for hash {:016X} has not been loaded into memory",
                        file.filename_hash()
                    ),
                )
            })?;
            writer.write_all(data)?;
        }

        Ok(())
    }
}
