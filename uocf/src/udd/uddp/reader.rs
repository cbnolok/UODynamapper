//! Reader implementation for UDDP and UDDPI containers.
//!
//! The reader owns the entire package image and builds lightweight indexes over
//! it. This keeps lookup operations cheap while still allowing tooling to
//! reason about the exact on-disk bytes when recomputing canonical hashes,
//! rebuilding packages, or applying patches.

use std::collections::HashMap;

use super::support::{checked_region, zstd_decompress, zstd_decompress_with_dict};
use super::*;

#[derive(Debug, Clone, Copy)]
struct RuntimeDictRef {
    offset: u64,
    size: u32,
    codec: Codec,
}

/// Fully parsed in-memory package view.
///
/// The reader stores the raw package bytes and then materializes the lookup
/// tables needed by the selected package mode:
/// - dense-id packages get a direct locator array
/// - path-hash packages get a sorted `path_hash64 -> locator` table
/// - sparse-id packages get a sorted `id -> locator` table
///
/// Dictionary references are also resolved eagerly so that payload decoding can
/// look up the right implicit dictionary without reparsing the package header.
pub struct UddpReader {
    bytes: Vec<u8>,
    header: UddpHeader,
    lookup_mode: LookupMode,
    dict_by_type: HashMap<u8, RuntimeDictRef>,
    dense_index: Option<Vec<UddpLocator>>,
    path_index: Option<Vec<UddpPathEntry>>,
    sparse_index: Option<Vec<UddpSparseIdEntry>>,
}

impl UddpReader {
    /// Open and fully parse a package from its complete in-memory byte image.
    ///
    /// The reader validates only the structural invariants needed for safe
    /// indexing and decoding. Higher-level semantic validation, such as patch
    /// manifest interpretation, is intentionally deferred to the code that owns
    /// those concepts.
    pub fn open(bytes: Vec<u8>) -> Result<Self, FormatError> {
        let mut cur = Cursor::new(&bytes);
        let header = UddpHeader::read_from(&mut cur)?;

        if header.magic != UDDP_MAGIC && header.magic != UDPI_MAGIC {
            return Err(FormatError::InvalidMagic(header.magic));
        }

        let lookup_mode = LookupMode::from_u8(header.lookup_mode)?;

        let mut this = Self {
            bytes,
            header,
            lookup_mode,
            dict_by_type: HashMap::new(),
            dense_index: None,
            path_index: None,
            sparse_index: None,
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
        canonical_package_hash64(&self.bytes)
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
        self.bytes.get(start..end)
    }

    pub fn package_size_bytes(&self) -> usize {
        self.bytes.len()
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
        let range = checked_region(start, len, UddpDictRef::SERIALIZED_SIZE, self.bytes.len())?;
        let mut cur = Cursor::new(&self.bytes[range]);

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
                let range = checked_region(start, count, UddpLocator::SERIALIZED_SIZE, self.bytes.len())?;
                let mut cur = Cursor::new(&self.bytes[range]);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(UddpLocator::read_from(&mut cur)?);
                }
                self.dense_index = Some(entries);
            }
            LookupMode::VirtualPathHash => {
                let range = checked_region(start, count, UddpPathEntry::SERIALIZED_SIZE, self.bytes.len())?;
                let mut cur = Cursor::new(&self.bytes[range]);
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push(UddpPathEntry::read_from(&mut cur)?);
                }
                self.path_index = Some(entries);
            }
            LookupMode::SparseId => {
                let range = checked_region(start, count, UddpSparseIdEntry::SERIALIZED_SIZE, self.bytes.len())?;
                let mut cur = Cursor::new(&self.bytes[range]);
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
        let offset = usize::try_from(unpack_offset40(locator.pos64)).map_err(|_| FormatError::Overflow)?;
        let stored_size = reconstruct_stored_size(locator.raw_size, locator.meta32, locator.pos64) as usize;
        let end = offset.checked_add(stored_size).ok_or(FormatError::Overflow)?;
        let data = self.bytes.get(offset..end).ok_or(FormatError::Truncated)?;

        match unpack_codec(locator.meta32) {
            Codec::None => Ok(data.to_vec()),
            Codec::ZstdNoDict => zstd_decompress(data, locator.raw_size as usize).map_err(FormatError::Io),
            Codec::ZstdTypeDict => {
                let data_type = unpack_type(locator.meta32);
                let dict = self
                    .dictionary_for_type(data_type)
                    .ok_or(FormatError::MissingDictionary(data_type))?;
                zstd_decompress_with_dict(data, locator.raw_size as usize, dict).map_err(FormatError::Io)
            }
            Codec::Reserved => Err(FormatError::UnsupportedCodec),
        }
    }
}
