#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::Section;
use glam::Vec3; // Bevy uses glam::Vec3 under the hood.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, SeekFrom, prelude::*};
use std::path::PathBuf;

#[derive(Clone, Copy, Default)]
pub struct MapCell {
    // Cells are loaded from blocks in the mul file: left-to-right then top-to-bottom.
    pub id: u16,
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

#[derive(Clone)]
pub struct MapBlock {
    // Blocks are loaded from the mul file: top-to-bottom then left-to-right.
    pub internal_coords: MapBlockRelPos,
    //header: u32, // unused
    cells: Box<[MapCell; Self::CELLS_PER_BLOCK as usize]>,
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
}

pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    map_file_mul_path: PathBuf,
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

        let map_size_tiles = match map_index {
            0..=1 => {
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
            map_file_mul_path: map_file_mul_path,
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

    pub fn load_blocks(&mut self,   blocks_to_load: &mut Vec<MapBlockRelPos>) -> eyre::Result<()> {
        const MAP_FILE_MAX_SEQ_BLOCKS: usize = 10_000; // Cap of blocks to be read sequentially.
        const MAP_FILE_MAX_CHUNK_SIZE: usize = MapBlock::PACKED_SIZE * MAP_FILE_MAX_SEQ_BLOCKS;

        if blocks_to_load.is_empty() {
            //println!("Received empty load request (no blocks).");
            return Ok(());
        }

        // First, check if we lack some block in our cache.
        if !self.cached_blocks.is_empty() {
            let mut missing_key = false;
            for block_pos in &*blocks_to_load {
                if !self.cached_blocks.contains_key(block_pos) {
                    missing_key = true;
                    break;
                }
            }
            if !missing_key {
                return Ok(());
            }
        }

        // We don't have every requested block in the cache, so we need to retrieve them.
        let mut map_file_mul_handle = File::open(&self.map_file_mul_path).wrap_err_with(|| {
            format!(
                "Open map mul at '{}'",
                self.map_file_mul_path.to_string_lossy()
            )
        })?;

        // Having it sorted allows us to perform less file reads by acquiring blocks stored sequentially in the map file.
        blocks_to_load.sort(); // Sort first by x, then by y.

        // Start reading blocks.
        let mut blocks_buffer: Vec<u8> = Vec::with_capacity(MAP_FILE_MAX_CHUNK_SIZE);
        let mut blocks_read: usize = 0;
        let mut chunk_blocks_to_read_seq_count: usize;
        'read_chunks: while blocks_read < blocks_to_load.len() {
            chunk_blocks_to_read_seq_count = 1;
            if blocks_to_load.len() - blocks_read > 1 {
                // Given the list of blocks to load, how many of them can i load sequentially?
                //  (in order to execute the minimum amount of file read operations)
                'count_sequential_blocks: loop {
                    if blocks_to_load.len() == blocks_read + chunk_blocks_to_read_seq_count {
                        // Reached after reading the last chunk
                        break 'count_sequential_blocks;
                    }

                    // Blocks are stored sequentially top to bottom, then left to right.
                    let block_pos_prev: &MapBlockRelPos =
                        &blocks_to_load[blocks_read + chunk_blocks_to_read_seq_count - 1];
                    let block_pos: &MapBlockRelPos =
                        &blocks_to_load[blocks_read + chunk_blocks_to_read_seq_count];
                    /*
                    if (block_pos.y < block_pos_prev.y || block_pos.x > block_pos_prev.x)
                        || (chunk_blocks_to_read_seq_count >= MAP_FILE_MAX_SEQ_BLOCKS)
                    {
                        // Reached after reading the last block in every chunk (except the last chunk)
                        break 'count_sequential_blocks;
                    }
                    */
                    let block_idx_prev =
                        MapBlock::idx_from_coords(block_pos_prev, self.size_blocks.height);
                    let block_idx = MapBlock::idx_from_coords(block_pos, self.size_blocks.height);

                    if block_idx != block_idx_prev + 1 {
                        // The blocks are not sequential in the file, so break the chunk.
                        break 'count_sequential_blocks;
                    }
                    chunk_blocks_to_read_seq_count += 1;
                }
            }
            if chunk_blocks_to_read_seq_count == 0 {
                break 'read_chunks;
            }

            // Read the current chunk of blocks.
            let block_to_seek = blocks_to_load[blocks_read];
            //let max_blocks = self.size_blocks.width * self.size_blocks.height;
            if block_to_seek.x >= self.size_blocks.width
                || block_to_seek.y >= self.size_blocks.height
            {
                Err(eyre!(
                    "Requested map block out of bounds {block_to_seek:?}.".to_owned()
                ))?;
            }

            let block_idx = MapBlock::idx_from_coords(&block_to_seek, self.size_blocks.height);
            let off = (MapBlock::PACKED_SIZE * block_idx as usize) as u64;
            map_file_mul_handle
                .seek(SeekFrom::Start(off))
                .wrap_err(format!("Failed to seek to {off} for block {block_idx}."))?;

            blocks_buffer.resize(chunk_blocks_to_read_seq_count * MapBlock::PACKED_SIZE, 0);
            let read_result = map_file_mul_handle
                .read(blocks_buffer.as_mut())
                .wrap_err("Read map chunk")?;
            if 0 == read_result {
                // EOF
                return Err(eyre!("Encountered unexpected End Of File.".to_owned()));
            }

            let mut rdr = Cursor::new(&blocks_buffer);
            let chunk_slice_to_loop =
                &blocks_to_load[blocks_read..blocks_read + chunk_blocks_to_read_seq_count];

            'block_store: for block_pos in chunk_slice_to_loop.iter() {
                if self.cached_blocks.contains_key(block_pos) {
                    rdr.seek(SeekFrom::Current(MapBlock::PACKED_SIZE as i64))
                        .wrap_err(format!(
                            "Failed to seek after already cached block {:?}.",
                            block_pos
                        ))?;
                    blocks_read += 1;
                    continue 'block_store;
                }

                // Block header
                let mut new_block = MapBlock::default();

                let _block_header = rdr
                    .read_u32::<LittleEndian>()
                    .wrap_err("Read map block: header")?;

                //println!("READING BLOCK {:?}. Header: {_block_header}", block_pos);
                // Cells inside the block; stored sequentially left to right, then top to bottom.
                for y_cell in 0..MapBlock::CELLS_PER_COLUMN {
                    for x_cell in 0..MapBlock::CELLS_PER_ROW {
                        let new_cell = new_block.cell_as_mut(x_cell, y_cell).unwrap();
                        new_cell.id = rdr
                            .read_u16::<LittleEndian>()
                            .wrap_err("Read map block: cell: id")?;
                        new_cell.z = rdr.read_i8().wrap_err("Read map block: cell: z")?;
                        //println!("Reading CELL {x_cell},{y_cell}. ID: 0x{:.X}, Z: {}", new_cell.id, new_cell.z);
                    }
                }
                new_block.internal_coords = block_pos.clone();
                self.cached_blocks.insert(*block_pos, new_block);
                blocks_read += 1;
            }
        }

        //println!("Done reading block.");

        Ok(())
    }
}
