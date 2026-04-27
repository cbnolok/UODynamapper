//! This module provides functionality for loading and accessing static map data
//! from Ultima Online's `statics{}.mul` and `staidx{}.mul` files using zero-copy
//! optimized structures.

crate::eyre_imports!();
use crate::generic_index::IndexFile;
use bytemuck::{Pod, Zeroable};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Represents a single static item entry optimized for SIMD/GPU alignment (8 bytes).
/// The original disk format is 7 bytes; this struct includes 1 byte of padding.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct StaticTile {
    /// The graphic ID (or tile ID) of the static item.
    pub graphic: u16,
    /// The X-coordinate offset within its 8x8 map block (0-7).
    pub x_offset: u8,
    /// The Y-coordinate offset within its 8x8 map block (0-7).
    pub y_offset: u8,
    /// The Z-coordinate (altitude) of the item.
    pub z: i8,
    /// Internal padding to achieve 8-byte alignment (helps GPU uploading and caching).
    pub _pad: u8,
    /// The hue (or color) of the item. 0 indicates default color.
    pub hue: u16,
}

impl StaticTile {
    pub const RAW_SIZE: usize = 7;
}

/// A highly optimized reader for `statics.mul` and `staidx.mul` files.
pub struct StaticsReader<R: Read + Seek> {
    pub index: IndexFile,
    pub mul_reader: BufReader<R>,
    pub block_width: u32,
    pub block_height: u32,
    read_buffer: Vec<u8>,
}

impl StaticsReader<File> {
    /// Creates a new `StaticsReader` from file paths.
    pub fn new(index_path: &Path, mul_path: &Path, width: u32, height: u32) -> eyre::Result<Self> {
        let index = IndexFile::load(index_path.to_path_buf())?;
        let mul_file = File::open(mul_path).wrap_err("Failed to open statics.mul")?;
        let mul_reader = BufReader::with_capacity(1024 * 1024, mul_file); // 1MB buffer

        Ok(Self {
            index,
            mul_reader,
            block_width: width / 8,
            block_height: height / 8,
            read_buffer: Vec::new(),
        })
    }
}

impl<R: Read + Seek> StaticsReader<R> {
    /// Reads the static tiles for a given map block.
    pub fn read_block(&mut self, block_x: u32, block_y: u32) -> eyre::Result<Vec<StaticTile>> {
        if block_x >= self.block_width || block_y >= self.block_height {
            eyre::bail!("Block coordinates out of bounds");
        }

        let block_id = block_x * self.block_height + block_y;
        let index_element = self.index.element(block_id as usize)?;

        if let (Some(lookup), Some(size)) = (index_element.lookup(), index_element.len()) {
            if size == 0 {
                return Ok(Vec::new());
            }

            self.mul_reader.seek(SeekFrom::Start(lookup as u64))?;

            let count = (size as usize) / StaticTile::RAW_SIZE;
            self.read_buffer.resize(size as usize, 0);
            self.mul_reader.read_exact(&mut self.read_buffer)?;

            let mut tiles = Vec::with_capacity(count);
            let raw_bytes = &self.read_buffer;

            for i in 0..count {
                let base = i * StaticTile::RAW_SIZE;
                tiles.push(StaticTile {
                    graphic: u16::from_le_bytes([raw_bytes[base], raw_bytes[base + 1]]),
                    x_offset: raw_bytes[base + 2],
                    y_offset: raw_bytes[base + 3],
                    z: raw_bytes[base + 4] as i8,
                    _pad: 0,
                    hue: u16::from_le_bytes([raw_bytes[base + 5], raw_bytes[base + 6]]),
                });
            }

            Ok(tiles)
        } else {
            Ok(Vec::new())
        }
    }
}
