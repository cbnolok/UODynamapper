//! Represents a single file within a UOP package.

use std::io::{Read, Write};
use std::sync::Arc;
use byteorder::{LittleEndian, ReadBytesExt};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

/// The compression method used for the file data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFlag {
    /// No compression.
    None = 0,
    /// Zlib compression.
    Zlib = 1,
    /// ZlibBwt compression.
    ZlibBwt = 2,
    /// "Mythic" compression.
    Mythic = 3,
}

impl From<i16> for CompressionFlag {
    fn from(value: i16) -> Self {
        match value {
            0 => CompressionFlag::None,
            1 => CompressionFlag::Zlib,
            2 => CompressionFlag::ZlibBwt,
            3 => CompressionFlag::Mythic,
            _ => CompressionFlag::None, // Default or error handling
        }
    }
}

/// Represents a single file within a UOP package.
#[derive(Debug)]
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

        let final_data = match compression {
            CompressionFlag::None => {
                self.compressed_size = self.decompressed_size;
                buffer
            }
            CompressionFlag::Zlib => {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(&buffer)?;
                let compressed = encoder.finish()?;
                self.compressed_size = compressed.len() as u32;
                compressed
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unsupported compression type for creation",
                ));
            }
        };

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

        let compression = match compression_flag {
            0 => CompressionFlag::None,
            1 => CompressionFlag::Zlib,
            2 => CompressionFlag::Mythic,
            3 => CompressionFlag::ZlibBwt,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unsupported compression type",
                ));
            }
        };

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

        match self.compression {
            CompressionFlag::None => {
                target.write_all(data)?;
            }
            CompressionFlag::Zlib => {
                let mut decoder = ZlibDecoder::new(&data[..]);
                std::io::copy(&mut decoder, target)?;
            }
            CompressionFlag::Mythic => {
                // TODO: Implement Mythic decompression
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Mythic decompression not implemented",
                ));
            }
            CompressionFlag::ZlibBwt => {
                // TODO: Implement ZlibBwt decompression
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "ZlibBwt decompression not implemented",
                ));
            }
        }
        Ok(())
    }

    /// Decompresses the file data and returns it as a `Vec<u8>`.
    pub fn unpack(&self) -> Result<Vec<u8>, std::io::Error> {
        let mut buffer = Vec::with_capacity(self.decompressed_size as usize);
        self.unpack_to(&mut buffer)?;
        Ok(buffer)
    }
}
