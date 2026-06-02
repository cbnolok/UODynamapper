//! Reader implementation for UDDP and UDDPI containers.
//!
//! The reader owns the entire package image and builds lightweight indexes over
//! it. This keeps lookup operations cheap while still allowing tooling to
//! reason about the exact on-disk bytes when recomputing canonical hashes,
//! rebuilding packages, or applying patches.

use std::collections::HashMap;

use std::sync::{Arc, RwLock};

use super::support::{checked_region, zstd_decompress, zstd_decompress_with_dict, jxl_decompress, Cursor};
use memmap2::Mmap;
use super::*;

#[derive(Debug, Clone, Copy)]
struct RuntimeDictRef {
    offset: u64,
    size: u32,
    codec: Codec,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UddpReaderOptions {
    pub enable_decoded_entry_cache: bool,
}

impl UddpReaderOptions {
    pub const fn disabled() -> Self {
        Self {
            enable_decoded_entry_cache: false,
        }
    }

    pub const fn enable_decoded_entry_cache() -> Self {
        Self {
            enable_decoded_entry_cache: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DecodedEntryCacheKey {
    raw_size: u32,
    meta32: Meta32,
    pos64: Pos64,
}

impl From<&UddpLocator> for DecodedEntryCacheKey {
    fn from(locator: &UddpLocator) -> Self {
        Self {
            raw_size: locator.raw_size,
            meta32: locator.meta32,
            pos64: locator.pos64,
        }
    }
}

struct DecodedEntryCache {
    enabled: bool,
    entries: RwLock<HashMap<DecodedEntryCacheKey, Arc<[u8]>>>,
}

impl DecodedEntryCache {
    fn new(options: UddpReaderOptions) -> Self {
        Self {
            enabled: options.enable_decoded_entry_cache,
            entries: RwLock::new(HashMap::new()),
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn clear(&self) {
        self.entries
            .write()
            .expect("decoded entry cache poisoned")
            .clear();
    }

    fn get_or_load<F>(&self, locator: &UddpLocator, load: F) -> Result<Vec<u8>, FormatError>
    where
        F: FnOnce() -> Result<Vec<u8>, FormatError>,
    {
        if !self.enabled {
            return load();
        }

        let key = DecodedEntryCacheKey::from(locator);
        if let Some(bytes) = self
            .entries
            .read()
            .expect("decoded entry cache poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(bytes.as_ref().to_vec());
        }

        let bytes = load()?;
        let cached: Arc<[u8]> = Arc::from(bytes.as_slice());
        let mut entries = self.entries.write().expect("decoded entry cache poisoned");
        entries.entry(key).or_insert(cached);
        Ok(bytes)
    }
}

pub enum UddpData {
    Owned(Vec<u8>),
    Mmap(Mmap),
}

impl std::ops::Deref for UddpData {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(v) => v,
            Self::Mmap(m) => m,
        }
    }
}

/// Fully parsed package view.
///
/// The reader can either own the entire package image in-memory or
/// use a memory-mapped view for zero-copy access to large packages.
#[derive(Clone)]
pub struct UddpReader {
    data: Arc<UddpData>,
    header: UddpHeader,
    lookup_mode: LookupMode,
    dict_by_type: HashMap<u8, RuntimeDictRef>,
    dense_index: Option<Vec<UddpLocator>>,
    path_index: Option<Vec<UddpPathEntry>>,
    sparse_index: Option<Vec<UddpSparseIdEntry>>,
    decoded_entry_cache: Arc<DecodedEntryCache>,
}

impl UddpReader {
    /// Open and fully parse a package from a file using memory mapping.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, FormatError> {
        Self::load_with_options(path, UddpReaderOptions::disabled())
    }

    pub fn load_with_options(
        path: impl AsRef<std::path::Path>,
        options: UddpReaderOptions,
    ) -> Result<Self, FormatError> {
        let file = std::fs::File::open(path).map_err(FormatError::Io)?;
        let mmap = unsafe { Mmap::map(&file).map_err(FormatError::Io)? };
        Self::open_data(UddpData::Mmap(mmap), options)
    }

    /// Open and fully parse a package from its complete in-memory byte image.
    pub fn open(bytes: Vec<u8>) -> Result<Self, FormatError> {
        Self::open_with_options(bytes, UddpReaderOptions::disabled())
    }

    pub fn open_with_options(
        bytes: Vec<u8>,
        options: UddpReaderOptions,
    ) -> Result<Self, FormatError> {
        Self::open_data(UddpData::Owned(bytes), options)
    }

    /// Read a file completely into RAM and then parse it as a package.
    ///
    /// Use this if you want to avoid memory-mapped I/O overhead or if you
    /// need to modify the bytes (though UddpReader is currently read-only).
    pub fn load_in_memory(path: impl AsRef<std::path::Path>) -> Result<Self, FormatError> {
        Self::load_in_memory_with_options(path, UddpReaderOptions::disabled())
    }

    pub fn load_in_memory_with_options(
        path: impl AsRef<std::path::Path>,
        options: UddpReaderOptions,
    ) -> Result<Self, FormatError> {
        let bytes = std::fs::read(path).map_err(FormatError::Io)?;
        Self::open_with_options(bytes, options)
    }

    fn open_data(data: UddpData, options: UddpReaderOptions) -> Result<Self, FormatError> {
        let mut cur = Cursor::new(&*data);
        let header = UddpHeader::read_from(&mut cur)?;

        if header.magic != UDDP_MAGIC && header.magic != UDPI_MAGIC {
            return Err(FormatError::InvalidMagic(header.magic));
        }

        let lookup_mode = LookupMode::from_u8(header.lookup_mode)?;

        let mut this = Self {
            data: Arc::new(data),
            header,
            lookup_mode,
            dict_by_type: HashMap::new(),
            dense_index: None,
            path_index: None,
            sparse_index: None,
            decoded_entry_cache: Arc::new(DecodedEntryCache::new(options)),
        };

        this.read_dictionary_table()?;
        this.read_index()?;
        Ok(this)
    }

    /// Return the parsed package header exactly as stored on disk.
    pub fn header(&self) -> &UddpHeader {
        &self.header
    }

    /// Return the logical lookup mode chosen by the package producer.
    pub fn lookup_mode(&self) -> LookupMode {
        self.lookup_mode
    }

    /// Recompute the canonical package hash by zeroing the embedded hash field.
    pub fn computed_package_hash64(&self) -> u64 {
        canonical_package_hash64(&self.data)
    }

    /// Return the package hash stored inside the serialized header.
    pub fn stored_package_hash64(&self) -> u64 {
        self.header.package_hash64
    }

    /// Resolve a dense logical id without decoding the payload.
    pub fn find_by_dense_id(&self, id: u32) -> Option<ResolvedFile> {
        let dense = self.dense_index.as_ref()?;
        dense.get(id as usize).map(Self::resolved_from_locator)
    }

    /// Resolve a sparse logical id without decoding the payload.
    pub fn find_by_sparse_id(&self, id: u32) -> Option<ResolvedFile> {
        let sparse = self.sparse_index.as_ref()?;
        let pos = sparse.binary_search_by_key(&id, |e| e.id).ok()?;
        Some(Self::resolved_from_locator(&sparse[pos].locator))
    }

    /// Resolve a path hash without decoding the payload.
    pub fn find_by_path_hash(&self, path_hash64: u64) -> Option<ResolvedFile> {
        let path = self.path_index.as_ref()?;
        let pos = path.binary_search_by_key(&path_hash64, |e| e.path_hash64).ok()?;
        Some(Self::resolved_from_locator(&path[pos].locator))
    }

    /// Decode the file addressed by a dense logical id.
    pub fn read_file_by_dense_id(&self, id: u32) -> Result<Vec<u8>, FormatError> {
        let dense = self.dense_index.as_ref().ok_or(FormatError::WrongLookupMode)?;
        let loc = dense.get(id as usize).ok_or(FormatError::FileNotFound)?;
        self.decode_locator(loc)
    }

    /// Decode the file addressed by a sparse logical id.
    pub fn read_file_by_sparse_id(&self, id: u32) -> Result<Vec<u8>, FormatError> {
        let sparse = self.sparse_index.as_ref().ok_or(FormatError::WrongLookupMode)?;
        let pos = sparse
            .binary_search_by_key(&id, |e| e.id)
            .map_err(|_| FormatError::FileNotFound)?;
        self.decode_locator(&sparse[pos].locator)
    }

    /// Decode the file addressed by a normalized path hash.
    pub fn read_file_by_path_hash(&self, path_hash64: u64) -> Result<Vec<u8>, FormatError> {
        let path = self.path_index.as_ref().ok_or(FormatError::WrongLookupMode)?;
        let pos = path
            .binary_search_by_key(&path_hash64, |e| e.path_hash64)
            .map_err(|_| FormatError::FileNotFound)?;
        self.decode_locator(&path[pos].locator)
    }

    /// Enumerate all logical records in the package.
    ///
    /// Tooling uses this to rebuild packages, compare two package images, or
    /// apply patches while preserving logical keys.
    pub fn records(&self) -> Vec<FileRecord> {
        match self.lookup_mode {
            LookupMode::DenseId => self
                .dense_index
                .as_ref()
                .unwrap_or(&Vec::new())
                .iter()
                .enumerate()
                .map(|(i, loc)| FileRecord {
                    key: FileKey::Id(i as u32),
                    locator: *loc,
                })
                .collect(),
            LookupMode::VirtualPathHash => self
                .path_index
                .as_ref()
                .unwrap_or(&Vec::new())
                .iter()
                .map(|entry| FileRecord {
                    key: FileKey::PathHash(entry.path_hash64),
                    locator: entry.locator,
                })
                .collect(),
            LookupMode::SparseId => self
                .sparse_index
                .as_ref()
                .unwrap_or(&Vec::new())
                .iter()
                .map(|entry| FileRecord {
                    key: FileKey::Id(entry.id),
                    locator: entry.locator,
                })
                .collect(),
        }
    }

    /// Return the trained dictionary bytes associated with a data type.
    pub fn dictionary_for_type(&self, data_type: u8) -> Option<&[u8]> {
        let dict = self.dict_by_type.get(&data_type)?;
        let start = usize::try_from(dict.offset).ok()?;
        let end = start.checked_add(dict.size as usize)?;
        self.data.get(start..end)
    }

    pub fn package_size_bytes(&self) -> usize {
        self.data.len()
    }

    pub fn decoded_entry_cache_enabled(&self) -> bool {
        self.decoded_entry_cache.is_enabled()
    }

    pub fn clear_decoded_entry_cache(&self) {
        self.decoded_entry_cache.clear();
    }

    /// Reads raw bytes from the package image at the given offset.
    pub fn read_entry(&self, offset: u64, dest: &mut [u8]) -> Result<(), FormatError> {
        let offset = usize::try_from(offset).map_err(|_| FormatError::Overflow)?;
        let end = offset.checked_add(dest.len()).ok_or(FormatError::Overflow)?;
        let data = self.data.get(offset..end).ok_or(FormatError::Truncated)?;
        dest.copy_from_slice(data);
        Ok(())
    }

    pub fn dictionary_records(&self) -> Vec<(u8, Codec, u32)> {
        let mut records = self
            .dict_by_type
            .iter()
            .map(|(&data_type, dict)| (data_type, dict.codec, dict.size))
            .collect::<Vec<_>>();
        records.sort_by_key(|(data_type, _, _)| *data_type);
        records
    }

    fn resolved_from_locator(locator: &UddpLocator) -> ResolvedFile {
        ResolvedFile {
            raw_size: locator.raw_size,
            stored_size: reconstruct_stored_size(locator.raw_size, locator.meta32, locator.pos64),
            data_type: unpack_type(locator.meta32),
            codec: unpack_codec(locator.meta32),
            payload_offset: unpack_offset40(locator.pos64),
        }
    }

    fn read_dictionary_table(&mut self) -> Result<(), FormatError> {
        let start = usize::try_from(self.header.dict_table_offset).map_err(|_| FormatError::Overflow)?;
        let len = self.header.dict_count as usize;
        let range = checked_region(start, len, UddpDictRef::SERIALIZED_SIZE, self.data.len())?;
        let mut cur = Cursor::new(&self.data[range]);

        for _ in 0..len {
            let dict = UddpDictRef::read_from(&mut cur)?;
            self.dict_by_type.insert(
                dict.data_type,
                RuntimeDictRef {
                    offset: dict.offset,
                    size: dict.size,
                    codec: Codec::from_u8(dict.codec)?,
                },
            );
        }

        Ok(())
    }

    fn read_index(&mut self) -> Result<(), FormatError> {
        let start = usize::try_from(self.header.index_offset).map_err(|_| FormatError::Overflow)?;
        let count = self.header.file_count as usize;

        match self.lookup_mode {
            LookupMode::DenseId => {
                let range = checked_region(start, count, UddpLocator::SERIALIZED_SIZE, self.data.len())?;
                let mut cur = Cursor::new(&self.data[range]);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(UddpLocator::read_from(&mut cur)?);
                }
                self.dense_index = Some(entries);
            }
            LookupMode::VirtualPathHash => {
                let range = checked_region(start, count, UddpPathEntry::SERIALIZED_SIZE, self.data.len())?;
                let mut cur = Cursor::new(&self.data[range]);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(UddpPathEntry::read_from(&mut cur)?);
                }
                self.path_index = Some(entries);
            }
            LookupMode::SparseId => {
                let range = checked_region(start, count, UddpSparseIdEntry::SERIALIZED_SIZE, self.data.len())?;
                let mut cur = Cursor::new(&self.data[range]);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(UddpSparseIdEntry::read_from(&mut cur)?);
                }
                self.sparse_index = Some(entries);
            }
        }

        Ok(())
    }

    fn decode_locator(&self, locator: &UddpLocator) -> Result<Vec<u8>, FormatError> {
        self.decoded_entry_cache.get_or_load(locator, || {
            let offset = usize::try_from(unpack_offset40(locator.pos64)).map_err(|_| FormatError::Overflow)?;
            let stored_size = reconstruct_stored_size(locator.raw_size, locator.meta32, locator.pos64) as usize;
            let end = offset.checked_add(stored_size).ok_or(FormatError::Overflow)?;
            let data = self.data.get(offset..end).ok_or(FormatError::Truncated)?;

            let decoded = match unpack_codec(locator.meta32) {
                Codec::None => data.to_vec(),
                Codec::ZstdNoDict => zstd_decompress(data, locator.raw_size as usize).map_err(FormatError::Io)?,
                Codec::ZstdTypeDict => {
                    let data_type = unpack_type(locator.meta32);
                    let dict = self
                        .dictionary_for_type(data_type)
                        .ok_or(FormatError::MissingDictionary(data_type))?;
                    zstd_decompress_with_dict(data, locator.raw_size as usize, dict).map_err(FormatError::Io)?
                }
                Codec::JpegXl => {
                    jxl_decompress(data)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                        .map_err(FormatError::Io)?
                }
            };

            Ok(decoded)
        })
    }
}
