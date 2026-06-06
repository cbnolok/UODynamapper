//! This module provides functionality for loading and accessing static map data
//! from Ultima Online's `statics{}.mul` and `staidx{}.mul` files using zero-copy
//! optimized structures.

crate::eyre_imports!();
use crate::classic::generic_index::IndexFile;
use crate::classic::map_statics_diff::StaticDiff;
use crate::classic::verdata::{VerFileId, Verdata};
use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

pub const UO_BLOCK_DIM: u32 = 8;

/// Represents a single static item entry optimized for SIMD/GPU alignment (8 bytes).
///
/// ### Architecture Note:
/// Although the original Classic Client stores statics in 8x8 blocks, UODynamapper
/// packs statics into 32x32 "chunks" within `.uddp` packages.
///
/// In this 32x32 context:
/// - X/Y offsets require 5 bits each (0-31) to address the full chunk.
/// - The original 7-byte disk format is expanded to 8 bytes here for alignment.
/// - Halving this to 4 bytes (32-bit packing) is currently unfeasible without
///   loss, as Graphic(16) + Hue(16) + X(5) + Y(5) + Z(8) = 50 bits.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct StaticTile {
    /// The graphic ID (or tile ID) of the static item.
    pub graphic: u16,
    /// The X-coordinate offset within the 32x32 map chunk (0-31).
    pub x_offset: u8,
    /// The Y-coordinate offset within the 32x32 map chunk (0-31).
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
    pub graphic: u16,  // art_id for tex_art_cc lookup
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
    pub index_file: IndexFile,
    pub mul_mmap: Mmap,
    pub block_width: u32,
    pub block_height: u32,
    static_diff: Option<StaticDiff>,
    verdata: Option<Arc<Verdata>>,
}

impl StaticsReader {
    /// Creates a new `StaticsReader` using memory mapping.
    pub fn new(index_path: &Path, mul_path: &Path, width: u32, height: u32) -> eyre::Result<Self> {
        let index = IndexFile::load(index_path.to_path_buf())?;
        let mul_file = File::open(mul_path).wrap_err("Failed to open statics.mul")?;
        let mul_mmap = unsafe { Mmap::map(&mul_file)? };

        Ok(Self {
            index_file: index,
            mul_mmap,
            block_width: width / UO_BLOCK_DIM,
            block_height: height / UO_BLOCK_DIM,
            static_diff: None,
            verdata: None,
        })
    }

    pub fn with_static_diff(mut self, static_diff: StaticDiff) -> Self {
        self.static_diff = Some(static_diff);
        self
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    pub fn new_with_patches(
        index_path: &Path,
        mul_path: &Path,
        width: u32,
        height: u32,
        static_diff: Option<StaticDiff>,
        verdata: Option<Arc<Verdata>>,
    ) -> eyre::Result<Self> {
        let mut reader = Self::new(index_path, mul_path, width, height)?;
        reader.static_diff = static_diff;
        reader.verdata = verdata;
        Ok(reader)
    }

    /// Reads the static tiles for a given map block.
    pub fn read_block(&self, block_x: u32, block_y: u32) -> eyre::Result<Vec<StaticTile>> {
        if block_x >= self.block_width || block_y >= self.block_height {
            eyre::bail!("Block coordinates out of bounds");
        }

        let block_id = block_x * self.block_height + block_y;
        let raw_bytes = self.raw_block_bytes(block_id)?;
        Ok(parse_static_tiles(raw_bytes.as_deref()))
    }

    /// Reads every block from statics.mul into a compact in-memory store.
    pub fn load_all(&self) -> eyre::Result<StaticsStore> {
        if self.static_diff.is_some() || self.verdata.is_some() {
            return self.load_all_with_patches();
        }

        let num_blocks = self.block_width * self.block_height;

        // Step 1: Pre-calculate offsets and total count in a single fast pass over the index.
        let mut total_tile_count = 0;
        let mut offsets = Vec::with_capacity(num_blocks as usize + 1);
        offsets.push(0);
        for i in 0..num_blocks {
            let entry = self.index_file.element(i as usize)?;
            total_tile_count += (entry.len().unwrap_or(0) as usize) / StaticTile::RAW_SIZE;
            offsets.push(total_tile_count as u32);
        }

        let mut tiles = vec![PackedStaticTile::default(); total_tile_count];

        // Step 2: Parse blocks using parallel processing over the in-memory buffer.
        let mul_mmap_ref = &self.mul_mmap;
        let index = &self.index_file;
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

    fn load_all_with_patches(&self) -> eyre::Result<StaticsStore> {
        let num_blocks = self.block_width * self.block_height;
        let mut offsets = Vec::with_capacity(num_blocks as usize + 1);
        let mut tiles = Vec::new();
        offsets.push(0);

        for block_id in 0..num_blocks {
            let block_tiles = parse_static_tiles(self.raw_block_bytes(block_id)?.as_deref());
            tiles.reserve(block_tiles.len());
            for tile in block_tiles {
                tiles.push(PackedStaticTile {
                    graphic: tile.graphic,
                    xy_packed: (tile.x_offset & 0x07) | ((tile.y_offset & 0x07) << 3),
                    z: tile.z,
                    hue: tile.hue,
                });
            }
            offsets.push(tiles.len() as u32);
        }

        Ok(StaticsStore {
            block_width: self.block_width,
            block_height: self.block_height,
            offsets,
            tiles,
        })
    }
}

impl StaticsReader {
    fn raw_block_bytes(&self, block_id: u32) -> eyre::Result<Option<Vec<u8>>> {
        if let Some(diff) = &self.static_diff {
            if let Some(bytes) = diff.raw_block(block_id)? {
                return Ok(Some(bytes.to_vec()));
            }
        }

        if let Some(verdata) = &self.verdata {
            if let Some(bytes) = verdata.read_patch(VerFileId::Statics, block_id as i32)? {
                return Ok(Some(bytes));
            }
        }

        let mut lookup_size = None;
        if let Some(verdata) = &self.verdata {
            if let Some((lookup, size, _extra)) =
                verdata.index_patch(VerFileId::StaIdx, block_id as i32)
            {
                lookup_size = Some((lookup, size));
            }
        }

        let (lookup, size) = match lookup_size {
            Some(values) => values,
            None => {
                let index_element = self.index_file.element(block_id as usize)?;
                let (Some(lookup), Some(size)) = (index_element.lookup(), index_element.len()) else {
                    return Ok(None);
                };
                (lookup, size)
            }
        };

        if size == 0 {
            return Ok(None);
        }

        let start = lookup as usize;
        let end = start + size as usize;
        if end > self.mul_mmap.len() {
            eyre::bail!("Statics index points outside mul_mmap range");
        }

        Ok(Some(self.mul_mmap[start..end].to_vec()))
    }
}

fn parse_static_tiles(raw_bytes: Option<&[u8]>) -> Vec<StaticTile> {
    let Some(raw_bytes) = raw_bytes else {
        return Vec::new();
    };

    let count = raw_bytes.len() / StaticTile::RAW_SIZE;
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
    tiles
}
