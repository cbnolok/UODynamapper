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
    /// A hash of the compressed data.
    data_block_hash: u32,
    /// The compression method used.
    compression: CompressionFlag,
    /// The compressed file data. None if not yet loaded into memory.
    data: Option<Arc<[u8]>>,
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

        self.filename_hash = filename_hash;
        self.decompressed_size = buffer.len() as u32;
        self.compression = compression;

        let final_data = encode_payload(&buffer, compression.as_uop_compression())?;
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

        Ok(UopFile {
            data_block_address,
            data_block_length,
            compressed_size,
            decompressed_size,
            filename_hash,
            data_block_hash,
            compression,
            data: None,
        })
    }

    /// Load the compressed payload bytes referenced by this entry from a package reader.
    pub fn load_data_from<R: Read + Seek>(&mut self, reader: &mut R) -> Result<(), std::io::Error> {
        if !self.has_size() || self.data.is_some() {
            return Ok(());
        }

        let payload_address = self.data_block_address + self.data_block_length as u64;
        reader.seek(SeekFrom::Start(payload_address))?;
        let mut buffer = vec![0u8; self.compressed_size as usize];
        reader.read_exact(&mut buffer)?;
        self.data = Some(Arc::from(buffer));
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

    /// Returns the length of the data block header.
    pub fn data_block_length(&self) -> u32 {
        self.data_block_length
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

    /// Returns a hash of the compressed data.
    pub fn data_block_hash(&self) -> u32 {
        self.data_block_hash
    }

    /// Sets the hash of the compressed data.
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
        let mut buffer = Vec::with_capacity(self.decompressed_size as usize);
        self.unpack_to(&mut buffer)?;
        Ok(buffer)
    }
}
