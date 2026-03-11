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

#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::Section;
use glam::Vec3; // Bevy uses glam::Vec3 under the hood.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Cursor, SeekFrom, prelude::*};
use bytemuck::{Pod, Zeroable};
use std::path::PathBuf;

/// Represents a single cell (or tile) in the map.
#[derive(Clone, Copy, Default)]
pub struct MapCell {
    // Cells are loaded from blocks in the mul file: left-to-right then top-to-bottom.
    /// The texture ID of the tile.
    pub id: u16,
    /// The altitude of the tile.
    pub z: i8,
}
impl MapCell {
    // Cells are loaded from blocks left-to-right then top-to-bottom.
    // Cell = Tile.
    pub const PACKED_SIZE: usize = 2 + 1;

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

/// Represents a block of 8x8 cells.
#[derive(Clone)]
pub struct MapBlock {
    // Blocks are loaded from the mul file: top-to-bottom then left-to-right.
    /// The coordinates of the block in the map plane.
    pub internal_coords: MapBlockRelPos,
    //header: u32, // unused
    /// The cells in the block.
    cells: Box<[MapCell; Self::CELLS_PER_BLOCK as usize]>,
}

#[repr(C, packed)] // Ensure C-compatible layout and no padding
#[derive(Copy, Clone, Pod, Zeroable)] // Add bytemuck traits
struct RawMapBlock {
    header: u32,
    cells: [RawMapCell; MapBlock::CELLS_PER_BLOCK as usize],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawMapCell {
    id: u16,
    z: i8,
}

impl Default for MapBlock {
    fn default() -> Self {
        Self {
            internal_coords: MapBlockRelPos::default(),
            cells: Box::new([MapCell::default(); Self::CELLS_PER_BLOCK as usize]),
        }
    }
}
impl MapBlock {
    // Blocks are loaded top-to-bottom then left-to-right.
    pub const CELLS_PER_ROW: u32 = 8;
    pub const CELLS_PER_COLUMN: u32 = 8;
    pub const CELLS_PER_BLOCK: u32 = Self::CELLS_PER_ROW * Self::CELLS_PER_COLUMN;
    pub const PACKED_SIZE: usize = 4 + (Self::CELLS_PER_BLOCK as usize * MapCell::PACKED_SIZE);

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

    pub fn from_reader(rdr: &mut Cursor<&[u8]>) -> eyre::Result<MapBlock> {
        let bytes = rdr.get_ref(); // Get the underlying byte slice
        let offset = rdr.position() as usize; // Get the current position of the cursor

        // Read the raw block as a byte slice
        let raw_block_bytes = &bytes[offset..offset + MapBlock::PACKED_SIZE];

        // Cast the byte slice to RawMapBlock. This is where endianness needs to be handled for fields.
        let raw_block: &RawMapBlock = bytemuck::from_bytes(raw_block_bytes);

        let mut new_block = MapBlock::default();

        // Handle endianness for the header
        let _header = u32::from_le_bytes(raw_block.header.to_le_bytes()); // If we need the header, use this. Otherwise, just skip.

        for y_cell in 0..MapBlock::CELLS_PER_COLUMN {
            for x_cell in 0..MapBlock::CELLS_PER_ROW {
                let new_cell = new_block.cell_as_mut(x_cell, y_cell).unwrap();
                let raw_cell = &raw_block.cells[((MapBlock::CELLS_PER_COLUMN * y_cell) + x_cell) as usize];

                // Handle endianness for the id
                new_cell.id = u16::from_le_bytes(raw_cell.id.to_le_bytes());
                new_cell.z = raw_cell.z; // i8 is single byte, no endianness issue
            }
        }
        // Advance the cursor by the size of the block
        rdr.seek(SeekFrom::Current(MapBlock::PACKED_SIZE as i64))?;
        Ok(new_block)
    }
}

/// Represents a map plane, which is a 2D grid of blocks.
pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    map_file_mul_rdr: BufReader<File>,
    cached_blocks: BTreeMap<MapBlockRelPos, MapBlock>,
}
impl MapPlane {
    pub const EXTRA_BLOCKS_TO_CACHE_PER_SIDE: u32 = 8;

    //pub fn block(&self, x: u32, y: u32) -> Option<&MapBlock> {
    //    self.cached_blocks.get(&MapBlockRelPos { x, y })
    //}
    pub fn block(&self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        self.cached_blocks.get(&pos)
    }
    //pub fn block_as_mut(&mut self, x: u32, y: u32) -> Option<&mut MapBlock> {
    //    self.cached_blocks.get_mut(&MapBlockRelPos { x, y })
    //}
    pub fn block_as_mut(&mut self, pos: MapBlockRelPos) -> Option<&mut MapBlock> {
        self.cached_blocks.get_mut(&pos)
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
            cached_blocks: BTreeMap::new(),
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

    pub fn load_blocks(&mut self, blocks_to_load: &mut Vec<MapBlockRelPos>) -> eyre::Result<()> {
        if blocks_to_load.is_empty() {
            return Ok(());
        }

        // Sort the blocks to load by their coordinates.
        // This makes it more likely that sequential blocks are next to each other in the vector.
        blocks_to_load.sort_unstable();

        // Group the blocks into ranges of sequential blocks.
        let mut ranges = Vec::new();
        if !blocks_to_load.is_empty() {
            let mut current_range_start = blocks_to_load[0];
            let mut current_range_end = blocks_to_load[0];

            for i in 1..blocks_to_load.len() {
                let prev_idx =
                    MapBlock::idx_from_coords(&blocks_to_load[i - 1], self.size_blocks.height);
                let current_idx =
                    MapBlock::idx_from_coords(&blocks_to_load[i], self.size_blocks.height);

                if current_idx == prev_idx + 1 {
                    current_range_end = blocks_to_load[i];
                } else {
                    ranges.push((current_range_start, current_range_end));
                    current_range_start = blocks_to_load[i];
                    current_range_end = blocks_to_load[i];
                }
            }
            ranges.push((current_range_start, current_range_end));
        }

        // Read each range of blocks in a single operation.
        for (start_block, end_block) in ranges {
            let start_idx = MapBlock::idx_from_coords(&start_block, self.size_blocks.height);
            let end_idx = MapBlock::idx_from_coords(&end_block, self.size_blocks.height);
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

            let mut buffer = vec![0; num_blocks * MapBlock::PACKED_SIZE];
            self.map_file_mul_rdr
                .read_exact(&mut buffer)
                .wrap_err_with(|| {
                    format!(
                        "Failed to read {} blocks from offset {}",
                        num_blocks, offset
                    )
                })?;

            let mut cursor = Cursor::new(buffer.as_slice());
            for i in 0..num_blocks {
                let block_pos =
                    MapBlock::coords_from_idx(start_idx + i as u32, self.size_blocks.height);
                if self.cached_blocks.contains_key(&block_pos) {
                    cursor.seek(SeekFrom::Current(MapBlock::PACKED_SIZE as i64))?;
                    continue;
                }

                let mut new_block = MapBlock::from_reader(&mut cursor)?;
                new_block.internal_coords = block_pos;
                self.cached_blocks.insert(block_pos, new_block);
            }
        }

        Ok(())
    }
}
