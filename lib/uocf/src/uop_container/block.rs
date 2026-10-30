//! UOP block reader for chained groups of file entries.
//!
//! Binary layout covered by this module:
//! - Block header:
//!   - `u32 file_count`
//!   - `u64 next_block_address`
//! - Followed immediately by `file_count` serialized UOP file entries.
//! - `next_block_address == 0` marks the final block in the package.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::uop_container::file::UopFile;

/// Where a block's table was in the package file it was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSource {
    /// The offset of the block header.
    pub address: u64,
    /// The block's `file_count`: how many entry records its table held.
    pub file_count: u32,
}

/// Represents a block of files within a UOP package.
#[derive(Clone)]
pub struct UopBlock {
    /// The address of the next block in the UOP file.
    next_block_address: u64,
    /// The files contained within this block.
    files: Vec<UopFile>,
    /// Where the block was in the package file it was read from; None for blocks made in memory.
    source: Option<BlockSource>,
}

impl UopBlock {
    /// Creates a new, empty `UopBlock`.
    pub fn new() -> Self {
        UopBlock {
            next_block_address: 0,
            files: Vec::new(),
            source: None,
        }
    }

    /// Adds a file to the block.
    ///
    /// # Arguments
    ///
    /// * `file` - The `UopFile` to add.
    pub fn add_file(&mut self, file: UopFile) {
        self.files.push(file);
    }

    /// Reads a `UopBlock` from a reader.
    ///
    /// This method reads the block header and then iterates through the files,
    /// reading each file's metadata and data.
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to read from.
    pub fn read(reader: &mut File) -> Result<Self, std::io::Error> {
        let address = reader.stream_position()?;
        let file_count = reader.read_u32::<LittleEndian>()?;
        let next_block_address = reader.read_u64::<LittleEndian>()?;

        let mut files = Vec::with_capacity(file_count as usize);
        for _ in 0..file_count {
            let file = UopFile::read(reader)?;
            files.push(file);
        }

        Ok(UopBlock {
            next_block_address,
            files,
            source: Some(BlockSource { address, file_count }),
        })
    }

    /// Preloads data for all files within the block from the disk.
    ///
    /// This method eagerly loads the compressed byte chunks of all files
    /// present in the block.
    pub fn preload_data(&mut self, reader: &mut File) -> Result<(), std::io::Error> {
        let current_pos = reader.stream_position()?;

        for file in &mut self.files {
            file.load_data_from(reader)?;
        }

        reader.seek(SeekFrom::Start(current_pos))?;
        Ok(())
    }

    /// Returns the address of the next block in the UOP file.
    pub fn next_block_address(&self) -> u64 {
        self.next_block_address
    }

    /// Sets the address of the next block in the UOP file.
    pub fn set_next_block_address(&mut self, address: u64) {
        self.next_block_address = address;
    }

    /// Where the block was in the package file it was read from (see [`BlockSource`]).
    pub fn source(&self) -> Option<BlockSource> {
        self.source
    }

    /// Returns a reference to the files in the block.
    pub fn files(&self) -> &Vec<UopFile> {
        &self.files
    }

    /// Returns a mutable reference to the files in the block.
    pub fn files_mut(&mut self) -> &mut Vec<UopFile> {
        &mut self.files
    }
}
