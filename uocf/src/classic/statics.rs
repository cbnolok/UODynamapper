//! This module provides functionality for loading and accessing static map data
//! from Ultima Online's `statics{}.mul` and `staidx{}.mul` files using zero-copy
//! optimized structures.

crate::eyre_imports!();
use crate::classic::generic_index::IndexFile;
use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use std::fs::File;
use std::path::Path;

pub const UO_BLOCK_DIM: u32 = 8;

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

/// The raw 7-byte structure as it appears on disk in `statics.mul`.
#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawStaticTile {
    graphic: u16,
    x: u8,
    y: u8,
    z: i8,
    hue: u16,
}

/// Packed 6-byte static tile for bulk in-memory storage.
/// Trades alignment for density.
#[repr(C, packed)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct PackedStaticTile {
    pub graphic: u16,  // art_id for cc_art lookup
    pub xy_packed: u8, // x_offset:3 | y_offset:3 | _reserved:2
    pub z: i8,         // altitude
    pub hue: u16,      // color index (0 = default)
}

impl PackedStaticTile {
    pub fn x_offset(self) -> u8 {
        self.xy_packed & 0x07
    }
    pub fn y_offset(self) -> u8 {
        (self.xy_packed >> 3) & 0x07
    }
}

/// All statics for one map plane, stored as a flat CSR array.
/// offsets[block_id] .. offsets[block_id+1] gives the tile slice.
pub struct StaticsStore {
    pub block_width: u32,
    pub block_height: u32,
    pub offsets: Vec<u32>,            // len = num_blocks + 1
    pub tiles: Vec<PackedStaticTile>, // all tiles, contiguous
}

impl StaticsStore {
    pub fn block_tiles(&self, block_x: u32, block_y: u32) -> &[PackedStaticTile] {
        if block_x >= self.block_width || block_y >= self.block_height {
            return &[];
        }
        let block_id = (block_x * self.block_height + block_y) as usize;
        let start = self.offsets[block_id] as usize;
        let end = self.offsets[block_id + 1] as usize;
        &self.tiles[start..end]
    }
}

use memmap2::Mmap;

/// A highly optimized reader for `statics.mul` and `staidx.mul` files.
/// Uses memory mapping for zero-copy access to static data.
pub struct StaticsReader {
    pub index: IndexFile,
    pub mul_mmap: Mmap,
    pub block_width: u32,
    pub block_height: u32,
}

impl StaticsReader {
    /// Creates a new `StaticsReader` using memory mapping.
    pub fn new(index_path: &Path, mul_path: &Path, width: u32, height: u32) -> eyre::Result<Self> {
        let index = IndexFile::load(index_path.to_path_buf())?;
        let mul_file = File::open(mul_path).wrap_err("Failed to open statics.mul")?;
        let mul_mmap = unsafe { Mmap::map(&mul_file)? };

        Ok(Self {
            index,
            mul_mmap,
            block_width: width / UO_BLOCK_DIM,
            block_height: height / UO_BLOCK_DIM,
        })
    }

    /// Reads the static tiles for a given map block.
    pub fn read_block(&self, block_x: u32, block_y: u32) -> eyre::Result<Vec<StaticTile>> {
        if block_x >= self.block_width || block_y >= self.block_height {
            eyre::bail!("Block coordinates out of bounds");
        }

        let block_id = block_x * self.block_height + block_y;
        let index_element = self.index.element(block_id as usize)?;

        if let (Some(lookup), Some(size)) = (index_element.lookup(), index_element.len()) {
            if size == 0 {
                return Ok(Vec::new());
            }

            let lookup = lookup as usize;
            let size = size as usize;
            let end = lookup + size;

            if end > self.mul_mmap.len() {
                eyre::bail!("Statics index points outside mul_mmap range");
            }

            let raw_bytes = &self.mul_mmap[lookup..end];
            let count = size / StaticTile::RAW_SIZE;

            let mut tiles = Vec::with_capacity(count);
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

    /// Reads every block from statics.mul into a compact in-memory store.
    pub fn load_all(&self) -> eyre::Result<StaticsStore> {
        let num_blocks = self.block_width * self.block_height;

        // Step 1: Pre-calculate offsets and total count in a single fast pass over the index.
        let mut total_tile_count = 0;
        let mut offsets = Vec::with_capacity(num_blocks as usize + 1);
        offsets.push(0);
        for i in 0..num_blocks {
            let entry = self.index.element(i as usize)?;
            total_tile_count += (entry.len().unwrap_or(0) as usize) / StaticTile::RAW_SIZE;
            offsets.push(total_tile_count as u32);
        }

        let mut tiles = vec![PackedStaticTile::default(); total_tile_count];

        // Step 2: Parse blocks using parallel processing over the in-memory buffer.
        let mul_mmap_ref = &self.mul_mmap;
        let index = &self.index;
        let offsets_ref = &offsets;

        let tiles_ptr = tiles.as_mut_ptr() as usize;
        (0..num_blocks as usize)
            .into_par_iter()
            .for_each(|block_id| {
                let start_idx = offsets_ref[block_id] as usize;
                let end_idx = offsets_ref[block_id + 1] as usize;
                if start_idx == end_idx {
                    return;
                }

                let entry = index.element(block_id).unwrap();
                let lookup = entry.lookup().unwrap() as usize;
                let count = end_idx - start_idx;

                let src = &mul_mmap_ref[lookup..lookup + count * StaticTile::RAW_SIZE];

                // Safety: Each parallel iteration writes to a disjoint range of the 'tiles' vector
                unsafe {
                    let dst = (tiles_ptr as *mut PackedStaticTile).add(start_idx);
                    for j in 0..count {
                        let base = j * StaticTile::RAW_SIZE;
                        let x = src[base + 2];
                        let y = src[base + 3];
                        let xy_packed = (x & 0x07) | ((y & 0x07) << 3);

                        *dst.add(j) = PackedStaticTile {
                            graphic: u16::from_le_bytes([src[base], src[base + 1]]),
                            xy_packed,
                            z: src[base + 4] as i8,
                            hue: u16::from_le_bytes([src[base + 5], src[base + 6]]),
                        };
                    }
                }
            });

        Ok(StaticsStore {
            block_width: self.block_width,
            block_height: self.block_height,
            offsets,
            tiles,
        })
    }
}
