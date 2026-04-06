//! # UO Map Data Parser (`map.mul`)
//!
//! This module handles the loading and parsing of Ultima Online map data from the `map.mul` files.
//! The map is a 2D grid of tiles, which are organized into blocks of 8x8 tiles.
//!
//! ## File Format
//!
//! `map.mul` contains a sequence of map blocks. Each block is 196 bytes long and has the
//! following structure:
//!
//! - **Header** (4 bytes, u32, little-endian): An unknown header value.
//! - **64 Map Cells**: Each cell is 3 bytes long:
//!   - **Tile ID** (2 bytes, u16, little-endian): The ID of the tile.
//!   - **Altitude** (1 byte, i8): The Z-coordinate of the tile.
//!
//! The blocks are stored in column-major order (top to bottom, then left to right).
//!
//! ## Design Choices
//!
//! - **4-Byte Alignment**: `MapCell` is aligned to 4 bytes in memory (using 1 byte of padding).
//!   While the file format uses 3 bytes, 4-byte alignment significantly improves CPU cache performance
//!   and enables more efficient SIMD processing during texture mapping.
//! - **Inlined Cells**: `MapBlock` stores its cells in a fixed-size array instead of a `Box`.
//!   This removes thousands of small heap allocations, reducing memory fragmentation and pressure on the allocator.
//! - **Fast Parsing**: We use a `RawMapBlock` struct that matches the disk format for initial loading,
//!   then convert to the aligned `MapCell` format in a tight loop.

#![allow(dead_code)]

crate::eyre_imports!();
use bytemuck::{cast_slice, Pod, Zeroable};
use glam::Vec3; // Bevy uses glam::Vec3 under the hood.
use std::fs::File;
use std::io::{prelude::*, BufReader, SeekFrom};
use std::path::PathBuf;

/// Represents a single cell (or tile) in the map.
#[repr(C, align(4))]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct MapCell {
    // Cells are loaded from blocks in the mul file: left-to-right then top-to-bottom.
    /// The texture ID of the tile.
    pub id: u16,
    /// The altitude of the tile.
    pub z: i8,
    /// Padding for 4-byte alignment
    pub _pad: i8,
}
impl MapCell {
    // Relative position of the cell inside the block
    #[inline(always)]
    pub fn coords_in_block_x(cell_x: u32) -> u32 {
        // Since we want the modulo for a power of 2, we can use the AND operator.
        cell_x & (MapBlock::CELLS_PER_ROW - 1)
    }
    #[inline(always)]
    pub fn coords_in_block_y(cell_y: u32) -> u32 {
        cell_y & (MapBlock::CELLS_PER_COLUMN - 1)
    }
    #[inline(always)]
    pub fn coords_in_block(cell: &MapCellCoords) -> MapCellRelPos {
        MapCellRelPos {
            x: Self::coords_in_block_x(cell.x),
            y: Self::coords_in_block_y(cell.y),
        }
    }

    // Coordinates of the block the cell belongs to.
    #[inline(always)]
    pub fn coords_of_parent_block_x(cell_map_x: u32) -> u32 {
        // Since we want to divide for a power of 2 (CELLS_PER_ROW = 2^3 = 8), we can use the bitshift operator to shift *exponent* bytes.
        cell_map_x >> ((MapBlock::CELLS_PER_ROW / 2) - 1)
    }
    #[inline(always)]
    pub fn coords_of_parent_block_y(cell_map_y: u32) -> u32 {
        cell_map_y >> ((MapBlock::CELLS_PER_COLUMN / 2) - 1)
    }
    #[inline(always)]
    pub fn coords_of_parent_block(cell: &MapCellCoords) -> MapBlockRelPos {
        MapBlockRelPos {
            x: Self::coords_of_parent_block_x(cell.x),
            y: Self::coords_of_parent_block_y(cell.y),
        }
    }
}

/// Size of a map block in the file (4 bytes header + 64 * 3 bytes cells)
const MAP_BLOCK_FILE_SIZE: usize = 196;

/// Represents a block of 8x8 cells.
#[derive(Clone)]
pub struct MapBlock {
    // Blocks are loaded from the mul file: top-to-bottom then left-to-right.
    /// The coordinates of the block in the map plane.
    pub internal_coords: MapBlockRelPos,
    //header: u32, // unused
    /// The cells in the block.
    pub cells: [MapCell; Self::CELLS_PER_BLOCK as usize],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawMapCell {
    id: u16,
    z: i8,
}

#[repr(C, packed)] // Ensure C-compatible layout and no padding
#[derive(Copy, Clone, Pod, Zeroable)] // Add bytemuck traits
struct RawMapBlock {
    header: u32,
    cells: [RawMapCell; MapBlock::CELLS_PER_BLOCK as usize],
}

impl Default for MapBlock {
    fn default() -> Self {
        Self {
            internal_coords: MapBlockRelPos::default(),
            cells: [MapCell::default(); Self::CELLS_PER_BLOCK as usize],
        }
    }
}
impl MapBlock {
    // Blocks are loaded top-to-bottom then left-to-right.
    pub const CELLS_PER_ROW: u32 = 8;
    pub const CELLS_PER_COLUMN: u32 = 8;
    pub const CELLS_PER_BLOCK: u32 = Self::CELLS_PER_ROW * Self::CELLS_PER_COLUMN;
    pub const PACKED_SIZE: usize = MAP_BLOCK_FILE_SIZE;

    #[inline(always)]
    pub fn coords_first_cell(block_coords: &MapBlockRelPos) -> MapCellCoords {
        // Top-left cell in the block.
        MapCellCoords {
            x: block_coords.x * Self::CELLS_PER_ROW,
            y: block_coords.y * Self::CELLS_PER_COLUMN,
        }
    }

    #[inline(always)]
    fn coords_from_idx(block_idx: u32, map_height_blocks: u32) -> MapBlockRelPos {
        MapBlockRelPos {
            x: block_idx / map_height_blocks,
            y: block_idx % map_height_blocks,
        }
    }
    #[inline(always)]
    fn idx_from_coords(block_coords: &MapBlockRelPos, map_height_blocks: u32) -> u32 {
        (block_coords.x * map_height_blocks) + block_coords.y
    }

    // Cells are loaded from blocks left-to-right then top-to-bottom.
    const ERR_CELL_OUT_RANGE: &'static str = "Map Cell out of range";
    pub fn cell(&self, x: u32, y: u32) -> eyre::Result<&MapCell> {
        if x >= Self::CELLS_PER_ROW || y >= Self::CELLS_PER_COLUMN {
            Err(eyre!(Self::ERR_CELL_OUT_RANGE.to_owned()))
        } else {
            Ok(&self.cells[((Self::CELLS_PER_COLUMN * y) + x) as usize])
        }
    }
    pub fn cell_as_mut(&mut self, x: u32, y: u32) -> eyre::Result<&mut MapCell> {
        if x >= Self::CELLS_PER_ROW || y >= Self::CELLS_PER_COLUMN {
            Err(eyre!(Self::ERR_CELL_OUT_RANGE.to_owned()))
        } else {
            Ok(&mut self.cells[((Self::CELLS_PER_COLUMN * y) + x) as usize])
        }
    }

    #[inline(always)]
    fn from_raw_block(raw_block: &RawMapBlock, new_block: &mut MapBlock) -> eyre::Result<()> {
        // We can't cast_slice the cells because memory layout differs (3 bytes vs 4 bytes).
        // Extract cells individually in a tight loop.
        // OPTIMIZATION: Unrolled processing of 4 tiles at a time (12 bytes -> 16 bytes).
        // This avoids loop overhead and provides the compiler with a clear structure
        // to apply auto-vectorization and instruction-level parallelism.
        //
        // ENDIANNESS: We use `u16::from_le_bytes` to explicitly handle the Little-Endian
        // format of UO .mul files. This ensures correct data extraction regardless
        // of whether the host CPU is Little-Endian (x86/ARM) or Big-Endian.
        let raw_bytes: &[u8] = bytemuck::cast_slice(&raw_block.cells);
        let out_cells: &mut [MapCell; 64] = &mut new_block.cells;

        for i in 0..16 {
            let base_in = (i << 2) + (i << 3); // Correctly calculate i * 12
            let base_out = i << 2; // i * 4

            // Tile 0: Extract 2-byte ID and 1-byte Z. Skip 1-byte pad in output.
            out_cells[base_out + 0] = MapCell {
                id: u16::from_le_bytes([raw_bytes[base_in + 0], raw_bytes[base_in + 1]]),
                z: raw_bytes[base_in + 2] as i8,
                _pad: 0,
            };
            // Tile 1
            out_cells[base_out + 1] = MapCell {
                id: u16::from_le_bytes([raw_bytes[base_in + 3], raw_bytes[base_in + 4]]),
                z: raw_bytes[base_in + 5] as i8,
                _pad: 0,
            };
            // Tile 2
            out_cells[base_out + 2] = MapCell {
                id: u16::from_le_bytes([raw_bytes[base_in + 6], raw_bytes[base_in + 7]]),
                z: raw_bytes[base_in + 8] as i8,
                _pad: 0,
            };
            // Tile 3
            out_cells[base_out + 3] = MapCell {
                id: u16::from_le_bytes([raw_bytes[base_in + 9], raw_bytes[base_in + 10]]),
                z: raw_bytes[base_in + 11] as i8,
                _pad: 0,
            };
        }

        Ok(())
    }
}

/// Represents a map plane, which is a 2D grid of blocks.
pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    map_file_path: PathBuf,
    map_file_mul_rdr: BufReader<File>,
    cached_block_indices: Vec<u32>,
    cached_blocks_arena: Vec<CachedBlock>,
    cached_blocks_free_list: Vec<u32>,
    cached_blocks_bitmask: Vec<u64>,
    read_buffer: Vec<u8>,
    pub blocks_loaded_version: u64,
}

pub struct CachedBlock {
    pub block: MapBlock,
    pub last_accessed: std::time::Instant,
}

impl MapPlane {
    pub const EXTRA_BLOCKS_TO_CACHE_PER_SIDE: u32 = 8;

    pub fn block(&mut self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            let cached: &mut CachedBlock = &mut self.cached_blocks_arena[arena_idx as usize];
            cached.last_accessed = std::time::Instant::now();
            //let block_ptr = &cached.block as *const MapBlock;
            //Some(unsafe { &*block_ptr })
            Some(&cached.block)
        } else {
            None
        }
    }

    pub fn block_no_update(&self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            Some(&self.cached_blocks_arena[arena_idx as usize].block)
        } else {
            None
        }
    }

    pub fn block_as_mut(&mut self, pos: MapBlockRelPos) -> Option<&mut MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            let cached = &mut self.cached_blocks_arena[arena_idx as usize];
            cached.last_accessed = std::time::Instant::now();
            Some(&mut cached.block)
        } else {
            None
        }
    }

    pub fn evict_idle_blocks(&mut self, timeout: std::time::Duration) -> usize {
        let now = std::time::Instant::now();
        let mut evicted = 0;

        for word_idx in 0..self.cached_blocks_bitmask.len() {
            let mut bits = self.cached_blocks_bitmask[word_idx];
            let mut cleared_bits = 0;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let idx = (word_idx * 64 + bit) as u32;

                if idx >= self.size_blocks.width * self.size_blocks.height {
                    bits &= bits - 1;
                    continue;
                }

                let arena_idx = self.cached_block_indices[idx as usize];
                if arena_idx != u32::MAX {
                    let cached = &self.cached_blocks_arena[arena_idx as usize];
                    if now.duration_since(cached.last_accessed) > timeout {
                        self.cached_blocks_free_list.push(arena_idx);
                        self.cached_block_indices[idx as usize] = u32::MAX;
                        evicted += 1;
                        cleared_bits |= 1 << bit;
                    }
                }
                bits &= bits - 1;
            }
            if cleared_bits != 0 {
                self.cached_blocks_bitmask[word_idx] &= !cleared_bits;
            }
        }
        evicted
    }

    /// Returns the canonical path to the map file.
    pub fn file_path(&self) -> &std::path::Path {
        &self.map_file_path
    }

    /// Returns true if the block at `pos` is already loaded in the in-memory cache.
    #[inline(always)]
    pub fn is_block_cached(&self, pos: &MapBlockRelPos) -> bool {
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let word_idx = (idx >> 6) as usize; // Equivalent to / 64
        if word_idx >= self.cached_blocks_bitmask.len() {
            return false;
        }
        let bit_idx = (idx & 63) as usize; // Equivalent to % 64
        (self.cached_blocks_bitmask[word_idx] & (1 << bit_idx)) != 0
    }

    /// Inserts blocks that were loaded externally (e.g. by a background thread)
    pub fn insert_preloaded_blocks(&mut self, blocks: Vec<MapBlock>) {
        let now = std::time::Instant::now();
        let h = self.size_blocks.height;
        for block in blocks {
            let pos = &block.internal_coords;
            let idx = (pos.x * h) + pos.y;
            let arena_idx = self.cached_block_indices[idx as usize];
            if arena_idx == u32::MAX {
                let new_arena_idx = if let Some(free_idx) = self.cached_blocks_free_list.pop() {
                    self.cached_blocks_arena[free_idx as usize] = CachedBlock {
                        block,
                        last_accessed: now,
                    };
                    free_idx
                } else {
                    let next_idx = self.cached_blocks_arena.len() as u32;
                    self.cached_blocks_arena.push(CachedBlock {
                        block,
                        last_accessed: now,
                    });
                    next_idx
                };
                self.cached_block_indices[idx as usize] = new_arena_idx;

                let word_idx = (idx / 64) as usize;
                let bit_idx = (idx % 64) as usize;
                if word_idx < self.cached_blocks_bitmask.len() {
                    self.cached_blocks_bitmask[word_idx] |= 1 << bit_idx;
                }
                self.blocks_loaded_version += 1;
            }
        }
    }
}

// Position of a cell in the map plane
#[derive(Clone, Copy)]
pub struct MapCellCoords {
    pub x: u32,
    pub y: u32,
}
impl MapCellCoords {
    pub fn from_vec3uo(position_vector: &Vec3) -> Self {
        Self {
            x: position_vector.x as u32,
            y: position_vector.y as u32,
        }
    }
}

// Position of a block relative to the parent map plane.
#[derive(Clone, Copy, Default, Hash, PartialEq, PartialOrd, Eq, Ord)]
pub struct MapBlockRelPos {
    pub x: u32,
    pub y: u32,
}
impl MapBlockRelPos {
    #[inline(always)]
    pub fn as_u64(&self) -> u64 {
        (self.x as u64) | ((self.y as u64) << 32)
    }
}
// Position of a cell relative to the parent block.
#[derive(Clone, Copy, Default, Hash, PartialEq, PartialOrd, Eq, Ord)]
pub struct MapCellRelPos {
    pub x: u32,
    pub y: u32,
}

// Size of a map plane, expressed in cells/tiles.
#[derive(Clone, Copy)]
pub struct MapSizeCells {
    pub width: u32,
    pub height: u32,
}
// Size of a map plane, expressed in blocks.
#[derive(Clone, Copy)]
pub struct MapSizeBlocks {
    pub width: u32,
    pub height: u32,
}

// A rectangle in the map; always in tiles/cells.
#[derive(Clone, Copy)]
pub struct MapRectCells {
    pub x0: u32,
    pub y0: u32,
    pub width: u32,
    pub height: u32,
}
impl MapRectCells {
    pub fn to_blocks_rect(&self) -> MapRectBlocks {
        let x0 = MapCell::coords_of_parent_block_x(self.x0);
        let y0 = MapCell::coords_of_parent_block_y(self.y0);
        MapRectBlocks {
            x0,
            y0,
            width: MapCell::coords_of_parent_block_x(self.x0 + self.width)
                .checked_sub(x0)
                .unwrap(),
            height: MapCell::coords_of_parent_block_y(self.y0 + self.height)
                .checked_sub(y0)
                .unwrap(),
        }
    }
}

// A rectangle in the map, in blocks.
#[derive(Clone, Copy)]
pub struct MapRectBlocks {
    pub x0: u32,
    pub y0: u32,
    pub width: u32,
    pub height: u32,
}

impl MapPlane {
    pub fn init(map_file_mul_path: PathBuf, map_index: u32) -> eyre::Result<MapPlane> {
        Self::init_with_size(map_file_mul_path, map_index, None)
    }

    pub fn init_with_size(
        map_file_mul_path: PathBuf,
        map_index: u32,
        map_size_tiles_override: Option<MapSizeCells>,
    ) -> eyre::Result<MapPlane> {
        // We need to use PathBuf instead of String, because the latter has a UTF-8 encoding, while the former
        //  can have different encodings, even not valid UTF-*, which can be valid for the used OS.
        let map_file_mul_path = map_file_mul_path
            .canonicalize()
            .wrap_err_with(|| format!("Check map{map_index}.mul path"))?;

        let map_file_mul_handle = File::open(&map_file_mul_path).wrap_err_with(|| {
            format!(
                "Open map{map_index}.mul at '{}'",
                map_file_mul_path.to_string_lossy()
            )
        })?;
        let map_file_mul_metadata = map_file_mul_handle
            .metadata()
            .wrap_err_with(|| format!("Get map{map_index}.mul metadata"))?;

        let map_file_mul_rdr = BufReader::new(map_file_mul_handle);

        let map_size_tiles = match map_size_tiles_override {
            Some(size) => {
                if size.width % MapBlock::CELLS_PER_ROW != 0
                    || size.height % MapBlock::CELLS_PER_COLUMN != 0
                {
                    Err(eyre!("Invalid manual map size"))
                } else {
                    Ok(size)
                }
            }
            None => match map_index {
                0..=1 => {
                    // Determine if this is a pre-ML (6144x4096) or post-ML (7168x4096) map by checking file size.
                    if map_file_mul_metadata.len() < 77070336 {
                        Ok(MapSizeCells {
                            width: 6144,
                            height: 4096,
                        }) // pre-ML
                    } else {
                        Ok(MapSizeCells {
                            width: 7168,
                            height: 4096,
                        })
                    }
                }
                2 => Ok(MapSizeCells {
                    width: 2304,
                    height: 1600,
                }),
                3 => Ok(MapSizeCells {
                    width: 2560,
                    height: 2048,
                }),
                4 => Ok(MapSizeCells {
                    width: 1448,
                    height: 1448,
                }),
                5 => Ok(MapSizeCells {
                    width: 1280,
                    height: 4096,
                }),
                _ => Err(eyre!("Invalid map number")),
            },
        }?;

        let map_size_blocks = MapSizeBlocks {
            width: map_size_tiles.width / MapBlock::CELLS_PER_ROW,
            height: map_size_tiles.height / MapBlock::CELLS_PER_COLUMN,
        };

        let map_file_expected_size = MapBlock::PACKED_SIZE as u64
            * map_size_blocks.width as u64
            * map_size_blocks.height as u64;
        if map_file_mul_metadata.len() != map_file_expected_size {
            return Err(eyre!(
                "Malformed map file: expected size doesn't match the real file size"
            ));
        }

        let cached_block_indices =
            vec![u32::MAX; (map_size_blocks.width * map_size_blocks.height) as usize];

        let map_plane = MapPlane {
            index: map_index,
            size_blocks: map_size_blocks,
            map_file_path: map_file_mul_path.clone(),
            map_file_mul_rdr,
            cached_block_indices,
            cached_blocks_arena: Vec::new(),
            cached_blocks_free_list: Vec::new(),
            cached_blocks_bitmask: vec![
                0;
                (((map_size_blocks.width * map_size_blocks.height) + 63) / 64)
                    as usize
            ],
            read_buffer: Vec::new(),
            blocks_loaded_version: 0,
        };
        Ok(map_plane)
    }

    pub fn calc_blocks_to_load(&self, map_rect_to_show: &MapRectCells) -> Vec<MapBlockRelPos> {
        let block_x_start = MapCell::coords_of_parent_block_x(map_rect_to_show.x0)
            .saturating_sub(Self::EXTRA_BLOCKS_TO_CACHE_PER_SIDE);
        let block_y_start = MapCell::coords_of_parent_block_y(map_rect_to_show.y0)
            .saturating_sub(Self::EXTRA_BLOCKS_TO_CACHE_PER_SIDE);
        let block_x_end =
            MapCell::coords_of_parent_block_x(map_rect_to_show.x0 + map_rect_to_show.width)
                + Self::EXTRA_BLOCKS_TO_CACHE_PER_SIDE;
        let block_y_end =
            MapCell::coords_of_parent_block_y(map_rect_to_show.y0 + map_rect_to_show.height)
                + Self::EXTRA_BLOCKS_TO_CACHE_PER_SIDE;

        //println!("MapRect to load: {:?}", map_rect_to_show);
        //println!("Blocks requested (+extra for cache): (X:{block_x_start},Y:{block_y_start}) to (X:{block_x_end},Y:{block_y_end})");
        let mut ret: Vec<MapBlockRelPos> = Vec::with_capacity(
            ((block_x_end - block_x_start) * (block_y_end - block_y_start)) as usize,
        );

        for x in block_x_start..=block_x_end {
            for y in block_y_start..=block_y_end {
                let p = MapBlockRelPos { x, y };
                if !self.is_block_cached(&p) {
                    ret.push(p);
                    //println!("Block {:?} marked to be LOADED", p);
                } else {
                    //println!("Already in CACHE: Block {:?}", p);
                }
            }
        }
        ret
    }

    pub fn load_blocks(&mut self, blocks_to_load: &mut [MapBlockRelPos]) -> eyre::Result<()> {
        if blocks_to_load.is_empty() {
            return Ok(());
        }

        // Sort the blocks to load by their coordinates.
        // This makes it more likely that sequential blocks are next to each other in the vector.
        let mut blocks_to_load = blocks_to_load.to_vec();
        blocks_to_load.sort_unstable();
        blocks_to_load.dedup();

        #[derive(Clone, Copy)]
        struct IndexedBlock {
            pos: MapBlockRelPos,
            idx: u32,
        }

        let indexed_blocks: Vec<IndexedBlock> = blocks_to_load
            .iter()
            .map(|&pos| IndexedBlock {
                pos,
                idx: MapBlock::idx_from_coords(&pos, self.size_blocks.height),
            })
            .collect();

        // Group the blocks into ranges of sequential blocks.
        let mut ranges = Vec::with_capacity(indexed_blocks.len());
        if !indexed_blocks.is_empty() {
            let mut current_range_start = 0usize;
            let mut current_range_end = 0usize;

            for i in 1..indexed_blocks.len() {
                let prev_idx = indexed_blocks[i - 1].idx;
                let current_idx = indexed_blocks[i].idx;

                if current_idx == prev_idx + 1 {
                    current_range_end = i;
                } else {
                    ranges.push((current_range_start, current_range_end));
                    current_range_start = i;
                    current_range_end = i;
                }
            }
            ranges.push((current_range_start, current_range_end));
        }

        // Read each range of blocks in a single operation, then decode the batch in-place.
        for (range_start, range_end) in ranges {
            let start_idx = indexed_blocks[range_start].idx;
            let end_idx = indexed_blocks[range_end].idx;
            let num_blocks = (end_idx - start_idx + 1) as usize;

            let offset = (start_idx as usize * MapBlock::PACKED_SIZE) as u64;
            self.map_file_mul_rdr
                .seek(SeekFrom::Start(offset))
                .wrap_err_with(|| {
                    format!(
                        "Failed to seek to offset {} for block index {}",
                        offset, start_idx
                    )
                })?;

            let buffer_len = num_blocks * MapBlock::PACKED_SIZE;
            self.read_buffer.resize(buffer_len, 0);
            self.map_file_mul_rdr
                .read_exact(&mut self.read_buffer)
                .wrap_err_with(|| {
                    format!(
                        "Failed to read {} blocks from offset {}",
                        num_blocks, offset
                    )
                })?;

            let raw_blocks: &[RawMapBlock] = cast_slice(&self.read_buffer);
            for (i, raw_block) in raw_blocks.iter().enumerate() {
                let block_pos = indexed_blocks[range_start + i].pos;
                if block_pos.x >= self.size_blocks.width || block_pos.y >= self.size_blocks.height {
                    continue;
                }
                let idx = (block_pos.x * self.size_blocks.height) + block_pos.y;
                let arena_idx = self.cached_block_indices[idx as usize];
                if arena_idx == u32::MAX {
                    let mut new_block = MapBlock::default();
                    MapBlock::from_raw_block(raw_block, &mut new_block)?;
                    new_block.internal_coords = block_pos;

                    let new_arena_idx = if let Some(free_idx) = self.cached_blocks_free_list.pop() {
                        self.cached_blocks_arena[free_idx as usize] = CachedBlock {
                            block: new_block,
                            last_accessed: std::time::Instant::now(),
                        };
                        free_idx
                    } else {
                        let next_idx = self.cached_blocks_arena.len() as u32;
                        self.cached_blocks_arena.push(CachedBlock {
                            block: new_block,
                            last_accessed: std::time::Instant::now(),
                        });
                        next_idx
                    };
                    self.cached_block_indices[idx as usize] = new_arena_idx;

                    let word_idx = (idx / 64) as usize;
                    let bit_idx = (idx % 64) as usize;
                    if word_idx < self.cached_blocks_bitmask.len() {
                        self.cached_blocks_bitmask[word_idx] |= 1 << bit_idx;
                    }
                    self.blocks_loaded_version += 1;
                }
            }
        }

        Ok(())
    }
}

/// Loads map blocks from an arbitrary [`Read`]+[`Seek`] source, returning them
/// without inserting into any cache.  Designed for the background chunk-loader
/// thread which opens its own file handle to avoid holding a lock on the
/// main-thread [`MapPlane`].
///
/// `read_buffer` is a caller-owned scratch buffer that is reused across calls
/// to avoid repeated heap allocation.
pub fn load_blocks_from_reader<R: Read + Seek>(
    reader: &mut R,
    blocks_to_load: &[MapBlockRelPos],
    size_blocks_height: u32,
    read_buffer: &mut Vec<u8>,
) -> eyre::Result<Vec<MapBlock>> {
    if blocks_to_load.is_empty() {
        return Ok(Vec::new());
    }

    // Keep the caller's order so higher-priority visible blocks are loaded and
    // emitted first. We still sort *within* each batch for coalesced disk I/O.
    let mut ordered = blocks_to_load.to_vec();
    if ordered.len() > 1 {
        // Use a bitmask for deduplication to avoid HashSet overhead while preserving order.
        let mut max_idx = 0;
        for pos in &ordered {
            let idx = (pos.x * size_blocks_height) + pos.y;
            if idx > max_idx {
                max_idx = idx;
            }
        }
        let mut bitmask = vec![0u64; (max_idx as usize / 64) + 1];
        ordered.retain(|pos| {
            let idx = (pos.x * size_blocks_height) + pos.y;
            let word = (idx / 64) as usize;
            let bit = (idx % 64) as usize;
            if (bitmask[word] & (1 << bit)) == 0 {
                bitmask[word] |= 1 << bit;
                true
            } else {
                false
            }
        });
    }

    const PRIORITY_BATCH_SIZE: usize = 512;

    #[derive(Clone, Copy)]
    struct IndexedBlock {
        pos: MapBlockRelPos,
        idx: u32,
    }

    let mut result = Vec::with_capacity(ordered.len());

    for batch in ordered.chunks(PRIORITY_BATCH_SIZE) {
        let mut indexed_blocks: Vec<IndexedBlock> = batch
            .iter()
            .map(|&pos| IndexedBlock {
                pos,
                idx: MapBlock::idx_from_coords(&pos, size_blocks_height),
            })
            .collect();
        indexed_blocks.sort_unstable_by_key(|b| b.idx);

        // Group into contiguous index ranges for coalesced I/O.
        let mut ranges = Vec::with_capacity(indexed_blocks.len());
        if !indexed_blocks.is_empty() {
            let mut start = 0usize;
            let mut end = 0usize;
            for i in 1..indexed_blocks.len() {
                if indexed_blocks[i].idx == indexed_blocks[i - 1].idx + 1 {
                    end = i;
                } else {
                    ranges.push((start, end));
                    start = i;
                    end = i;
                }
            }
            ranges.push((start, end));
        }

        for (range_start, range_end) in ranges {
            let start_idx = indexed_blocks[range_start].idx;
            let end_idx = indexed_blocks[range_end].idx;
            let num_blocks = (end_idx - start_idx + 1) as usize;

            let offset = (start_idx as usize * MapBlock::PACKED_SIZE) as u64;
            reader
                .seek(SeekFrom::Start(offset))
                .wrap_err_with(|| format!("bg-loader: seek to offset {offset}"))?;

            let buffer_len = num_blocks * MapBlock::PACKED_SIZE;
            read_buffer.resize(buffer_len, 0);
            reader.read_exact(read_buffer).wrap_err_with(|| {
                format!("bg-loader: read {num_blocks} blocks at offset {offset}")
            })?;

            let raw_blocks: &[RawMapBlock] = cast_slice(read_buffer);
            for (i, raw_block) in raw_blocks.iter().enumerate() {
                let block_pos = indexed_blocks[range_start + i].pos;
                let mut new_block = MapBlock::default();
                MapBlock::from_raw_block(raw_block, &mut new_block)?;
                new_block.internal_coords = block_pos;
                result.push(new_block);
            }
        }
    }

    Ok(result)
}
