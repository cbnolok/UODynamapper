//! UOP file entry and payload codec handling.
//!
//! Binary layout covered by this module:
//! - Per-file metadata stored inside a block:
//!   - `u64 data_block_address`
//!   - `u32 data_block_length`
//!   - `u32 compressed_size`
//!   - `u32 decompressed_size`
//!   - `u64 filename_hash`
//!   - `u32 data_block_hash`
//!   - `i16 compression_flag`
//! - The metadata points to the file payload stored elsewhere in the package.
//! - The payload bytes are stored either raw or compressed according to
//!   `compression_flag`; this module owns the encode/decode path for those bytes.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use crate::uop_container::codec::{decode_payload, encode_payload, UopCompression};
use crate::uop_container::data_block_header::DataBlockHeader;

/// The compression method used for the file data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFlag {
    /// No compression.
    None = 0,
    /// Zlib compression.
    Zlib = 1,
    /// "Mythic" compression.
    Mythic = 2,
    /// ZlibBwt compression.
    ZlibBwt = 3,
}

impl CompressionFlag {
    pub fn from_raw_i16(value: i16) -> Result<Self, std::io::Error> {
        match value {
            0 => Ok(CompressionFlag::None),
            1 => Ok(CompressionFlag::Zlib),
            2 => Ok(CompressionFlag::Mythic),
            3 => Ok(CompressionFlag::ZlibBwt),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported compression type {value}"),
            )),
        }
    }

    pub fn as_uop_compression(self) -> UopCompression {
        match self {
            CompressionFlag::None => UopCompression::None,
            CompressionFlag::Zlib => UopCompression::Zlib,
            CompressionFlag::Mythic => UopCompression::Mythic,
            CompressionFlag::ZlibBwt => UopCompression::ZlibBwt,
        }
    }
}

impl From<i16> for CompressionFlag {
    fn from(value: i16) -> Self {
        Self::from_raw_i16(value).unwrap_or(CompressionFlag::None)
    }
}

/// Where an entry's data block header and stored payload were in the package file it was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredExtent {
    /// The entry's `data_block_address` in that file.
    pub address: u64,
    /// `data_block_length + compressed_size`: the bytes the entry occupied from `address`.
    pub length: u64,
}

impl StoredExtent {
    /// The offset just past the extent.
    pub fn end(&self) -> u64 {
        self.address + self.length
    }
}

/// Represents a single file within a UOP package.
#[derive(Debug, Clone)]
pub struct UopFile {
    /// The address of the file's data in the UOP file.
    data_block_address: u64,
    /// The length of the data block header.
    data_block_length: u32,
    /// The size of the compressed file data.
    compressed_size: u32,
    /// The size of the decompressed file data.
    decompressed_size: u32,
    /// The hash of the file's name.
    filename_hash: u64,
    /// A hash of the data block header in client packages and in entries replaced or added by the package
    /// editors (`DataBlockHeader::data_block_hash`); of the compressed data for headerless entries created here.
    data_block_hash: u32,
    /// The compression method used.
    compression: CompressionFlag,
    /// The compressed file data. None if not yet loaded into memory.
    data: Option<Arc<[u8]>>,
    /// The `data_block_length` bytes stored before the payload, loaded with it: a timestamp header in version-4
    /// client packages, a signature header in version-5 ones (`data_block_header`). None if not loaded or absent.
    data_block_header: Option<Arc<[u8]>>,
    /// Where the entry was stored in the package file it was read from; None for entries made in memory and
    /// entries that stored no bytes. The layout-preserving writer keeps the entry there when it still fits.
    source_extent: Option<StoredExtent>,
}

impl UopFile {
    /// Creates a new, empty `UopFile`.
    pub fn new() -> Self {
        UopFile {
            data_block_address: 0,
            data_block_length: 0,
            compressed_size: 0,
            decompressed_size: 0,
            filename_hash: 0,
            data_block_hash: 0,
            compression: CompressionFlag::None,
            data: None,
            data_block_header: None,
            source_extent: None,
        }
    }

    /// Creates a new `UopFile` from a reader.
    ///
    /// This method reads the file data from the reader, compresses it if necessary,
    /// and computes the data block hash.
    ///
    /// # Arguments
    ///
    /// * `file_data` - A reader for the file data.
    /// * `file_hash` - The hash of the file name.
    /// * `compression` - The compression method to use.
    pub fn create_file(
        mut self,
        file_data: &mut impl Read,
        filename_hash: u64,
        compression: CompressionFlag,
    ) -> Result<Self, std::io::Error> {
        let mut buffer = Vec::new();
        file_data.read_to_end(&mut buffer)?;

        self.create_file_from_bytes(&buffer, filename_hash, compression)
    }

    /// Creates a new `UopFile` from an in-memory payload.
    pub fn create_file_from_bytes(
        mut self,
        file_data: &[u8],
        filename_hash: u64,
        compression: CompressionFlag,
    ) -> Result<Self, std::io::Error> {
        self.filename_hash = filename_hash;
        self.decompressed_size = file_data.len() as u32;
        self.compression = compression;

        let final_data = encode_payload(file_data, compression.as_uop_compression())?;
        self.compressed_size = final_data.len() as u32;

        self.data_block_hash = super::hash::hash_data_block(&final_data)?;
        self.data = Some(Arc::from(final_data));

        Ok(self)
    }

    /// Reads a `UopFile` from a reader.
    ///
    /// This method reads the file's metadata from the reader. The file data itself
    /// is not read here.
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to read from.
    pub fn read<R: Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let data_block_address = reader.read_u64::<LittleEndian>()?;
        let data_block_length = reader.read_u32::<LittleEndian>()?;
        let compressed_size = reader.read_u32::<LittleEndian>()?;
        let decompressed_size = reader.read_u32::<LittleEndian>()?;
        let filename_hash = reader.read_u64::<LittleEndian>()?;
        let data_block_hash = reader.read_u32::<LittleEndian>()?;
        let compression_flag = reader.read_i16::<LittleEndian>()?;

        let compression = CompressionFlag::from_raw_i16(compression_flag)?;
        let stored_length = u64::from(data_block_length) + u64::from(compressed_size);
        let source_extent =
            (stored_length != 0).then_some(StoredExtent { address: data_block_address, length: stored_length });

        Ok(UopFile {
            data_block_address,
            data_block_length,
            compressed_size,
            decompressed_size,
            filename_hash,
            data_block_hash,
            compression,
            data: None,
            data_block_header: None,
            source_extent,
        })
    }

    /// Load the compressed payload bytes referenced by this entry, and the data block header before them, from a
    /// package reader.
    pub fn load_data_from<R: Read + Seek>(&mut self, reader: &mut R) -> Result<(), std::io::Error> {
        if !self.has_size() || self.data.is_some() {
            return Ok(());
        }

        reader.seek(SeekFrom::Start(self.data_block_address))?;
        let header_length = self.data_block_length as usize;
        let mut buffer = vec![0u8; header_length + self.compressed_size as usize];
        reader.read_exact(&mut buffer)?;
        if header_length != 0 {
            self.data_block_header = Some(Arc::from(&buffer[..header_length]));
        }
        self.data = Some(Arc::from(&buffer[header_length..]));
        Ok(())
    }

    /// Returns the address of the file's data in the UOP file.
    pub fn data_block_address(&self) -> u64 {
        self.data_block_address
    }

    /// Sets the address of the file's data in the UOP file.
    pub fn set_data_block_address(&mut self, address: u64) {
        self.data_block_address = address;
    }

    /// Where the entry was stored in the package file it was read from (see [`StoredExtent`]). Replacing or
    /// recompressing the entry keeps it, so the layout-preserving writer can leave the new bytes in place.
    pub fn source_extent(&self) -> Option<StoredExtent> {
        self.source_extent
    }

    pub(crate) fn set_source_extent(&mut self, extent: Option<StoredExtent>) {
        self.source_extent = extent;
    }

    /// `data_block_length + compressed_size`: the bytes the entry stores at its `data_block_address`.
    pub fn stored_length(&self) -> u64 {
        u64::from(self.data_block_length) + u64::from(self.compressed_size)
    }

    /// Returns the length of the data block header.
    pub fn data_block_length(&self) -> u32 {
        self.data_block_length
    }

    /// Returns the data block header bytes stored before the payload, once loaded.
    pub fn data_block_header(&self) -> Option<&Arc<[u8]>> {
        self.data_block_header.as_ref()
    }

    /// Sets the data block header bytes stored before the payload, and `data_block_length` to their length.
    /// The `data_block_hash` is left unchanged.
    pub fn set_data_block_header(&mut self, header: Arc<[u8]>) {
        self.data_block_length = header.len() as u32;
        self.data_block_header = Some(header);
    }

    /// Stores `header` before the payload and sets `data_block_hash` to its hash
    /// (`DataBlockHeader::data_block_hash`). Fails when the header's hash algorithm is not known.
    pub fn set_data_block_header_and_hash(&mut self, header: &DataBlockHeader) -> Result<(), std::io::Error> {
        let hash = header.data_block_hash()?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the data block header has no known hash algorithm",
            )
        })?;
        self.set_data_block_header(Arc::from(header.to_bytes()));
        self.data_block_hash = hash;
        Ok(())
    }

    /// Parses the loaded data block header. None if not loaded or absent.
    pub fn parse_data_block_header(&self) -> Option<Result<DataBlockHeader, std::io::Error>> {
        self.data_block_header.as_deref().map(DataBlockHeader::parse)
    }

    /// Whether `data_block_hash` is the hash of the loaded data block header. None if no header is loaded or
    /// its hash algorithm is not known (`DataBlockHeader::hash_algorithm`).
    pub fn data_block_hash_matches_header(&self) -> Result<Option<bool>, std::io::Error> {
        let Some(header) = self.parse_data_block_header().transpose()? else {
            return Ok(None);
        };
        Ok(header.data_block_hash()?.map(|hash| hash == self.data_block_hash))
    }

    /// Returns the size of the compressed file data.
    pub fn compressed_size(&self) -> u32 {
        self.compressed_size
    }

    /// Sets the size of the compressed file data.
    pub fn set_compressed_size(&mut self, size: u32) {
        self.compressed_size = size;
    }

    pub fn has_size(&self) -> bool {
        self.compressed_size != 0 && self.decompressed_size != 0
    }

    /// Whether this is an unused slot of a block: client packages fill their last block up to the block size
    /// with all-zero entries, which the package header's file count does not include.
    pub fn is_empty_slot(&self) -> bool {
        self.filename_hash == 0 && !self.has_size() && self.data_block_length == 0 && self.data.is_none()
    }

    /// Returns the size of the decompressed file data.
    pub fn decompressed_size(&self) -> u32 {
        self.decompressed_size
    }

    /// Sets the size of the decompressed file data.
    pub fn set_decompressed_size(&mut self, size: u32) {
        self.decompressed_size = size;
    }

    /// Sets the compressed file data.
    pub fn set_data(&mut self, data: Arc<[u8]>) {
        self.data = Some(data);
    }

    /// Unloads the memory representation of the compressed file data.
    pub fn unload_data(&mut self) {
        self.data = None;
    }

    /// Returns the hash of the file's name.
    pub fn filename_hash(&self) -> u64 {
        self.filename_hash
    }

    /// Returns the stored data block hash.
    pub fn data_block_hash(&self) -> u32 {
        self.data_block_hash
    }

    /// Sets the stored data block hash.
    pub fn set_data_block_hash(&mut self, hash: u32) {
        self.data_block_hash = hash;
    }

    /// Returns the compression method used.
    pub fn compression(&self) -> CompressionFlag {
        self.compression
    }

    /// Sets the compression method used.
    pub fn set_compression(&mut self, compression: CompressionFlag) {
        self.compression = compression;
    }

    /// Returns the compressed file data.
    pub fn data(&self) -> Option<&Arc<[u8]>> {
        self.data.as_ref()
    }

    /// Decompresses the file data and writes it to a target.
    ///
    /// # Arguments
    ///
    /// * `target` - The writer to write the decompressed data to.
    pub fn unpack_to(&self, target: &mut impl Write) -> Result<(), std::io::Error> {
        let Some(ref data) = self.data else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File data has not been loaded into memory",
            ));
        };

        let decompressed = decode_payload(
            data,
            self.decompressed_size as usize,
            self.compression.as_uop_compression(),
        )?;
        target.write_all(&decompressed)?;
        Ok(())
    }

    /// Decompresses the file data and returns it as a `Vec<u8>`.
    pub fn unpack(&self) -> Result<Vec<u8>, std::io::Error> {
        let Some(ref data) = self.data else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File data has not been loaded into memory",
            ));
        };

        decode_payload(
            data,
            self.decompressed_size as usize,
            self.compression.as_uop_compression(),
        )
    }

    /// Decompresses the file data and returns shared bytes.
    ///
    /// Uncompressed UOP entries already store their final payload in `data`, so this
    /// can return an `Arc` clone instead of copying bytes into a fresh `Vec`.
    pub fn unpack_arc(&self) -> Result<Arc<[u8]>, std::io::Error> {
        let Some(ref data) = self.data else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File data has not been loaded into memory",
            ));
        };

        if self.compression == CompressionFlag::None {
            Ok(Arc::clone(data))
        } else {
            self.unpack().map(Arc::from)
        }
    }
}
