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
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::Section;
use glam::Vec3; // Bevy uses glam::Vec3 under the hood.
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, SeekFrom, prelude::*};
use bytemuck::{cast_slice, Pod, Zeroable};
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
    fn cell_as_mut(&mut self, x: u32, y: u32) -> eyre::Result<&mut MapCell> {
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
        for (i, raw_cell) in raw_block.cells.iter().enumerate() {
            let id = raw_cell.id;
            #[cfg(target_endian = "big")]
            let id = id.swap_bytes();

            new_block.cells[i] = MapCell {
                id,
                z: raw_cell.z,
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
    map_file_mul_rdr: BufReader<File>,
    cached_blocks: HashMap<MapBlockRelPos, CachedBlock>,
    read_buffer: Vec<u8>,
}

pub struct CachedBlock {
    pub block: MapBlock,
    pub last_accessed: std::time::Instant,
}

impl MapPlane {
    pub const EXTRA_BLOCKS_TO_CACHE_PER_SIDE: u32 = 8;

    pub fn block(&mut self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if let Some(cached) = self.cached_blocks.get_mut(&pos) {
            cached.last_accessed = std::time::Instant::now();
            // We have to return an immutable borrow to the block now
            let block_ptr = &cached.block as *const MapBlock;
            Some(unsafe { &*block_ptr })
        } else {
            None
        }
    }

    pub fn block_as_mut(&mut self, pos: MapBlockRelPos) -> Option<&mut MapBlock> {
        if let Some(cached) = self.cached_blocks.get_mut(&pos) {
            cached.last_accessed = std::time::Instant::now();
            Some(&mut cached.block)
        } else {
            None
        }
    }

    pub fn evict_idle_blocks(&mut self, timeout: std::time::Duration) -> usize {
        let now = std::time::Instant::now();
        let initial_len = self.cached_blocks.len();
        self.cached_blocks.retain(|_, cached| {
            now.duration_since(cached.last_accessed) <= timeout // Keep if newer than timeout
        });
        initial_len - self.cached_blocks.len()
    }
}

// Position of a cell in the map plane
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, PartialOrd, Eq, Ord)]
pub struct MapBlockRelPos {
    pub x: u32,
    pub y: u32,
}
// Position of a cell relative to the parent block.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, PartialOrd, Eq, Ord)]
pub struct MapCellRelPos {
    pub x: u32,
    pub y: u32,
}

// Size of a map plane, expressed in cells/tiles.
#[derive(Clone, Copy, Debug)]
pub struct MapSizeCells {
    pub width: u32,
    pub height: u32,
}
// Size of a map plane, expressed in blocks.
#[derive(Clone, Copy, Debug)]
pub struct MapSizeBlocks {
    pub width: u32,
    pub height: u32,
}

// A rectangle in the map; always in tiles/cells.
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Copy, Debug)]
pub struct MapRectBlocks {
    pub x0: u32,
    pub y0: u32,
    pub width: u32,
    pub height: u32,
}

impl MapPlane {
    pub fn init(map_file_mul_path: PathBuf, map_index: u32) -> eyre::Result<MapPlane> {
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

        // The dimensions of the maps are hardcoded based on the map index.
        let map_size_tiles = match map_index {
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
                "Malformed map file: expected size doesn't match the real file size".to_owned()
            ));
        }

        let map_plane = MapPlane {
            index: map_index,
            size_blocks: map_size_blocks,
            map_file_mul_rdr,
            cached_blocks: HashMap::new(),
            read_buffer: Vec::new(),
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
                if !self.cached_blocks.contains_key(&p) {
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
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.cached_blocks.entry(block_pos)
                {
                    let mut new_block = MapBlock::default();
                    MapBlock::from_raw_block(raw_block, &mut new_block)?;
                    new_block.internal_coords = block_pos;
                    entry.insert(CachedBlock {
                        block: new_block,
                        last_accessed: std::time::Instant::now(),
                    });
                }
            }
        }

        Ok(())
    }
}
