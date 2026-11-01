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
use memmap2::Mmap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use crate::classic::map_statics_diff::MapDiff;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::uop_container::package::LoadMode;

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

/// The height in cells of each Classic facet, by map index (Felucca, Trammel, Ilshenar, Malas, Tokuno, Ter Mur).
/// The heights never changed across client eras, while the width did: Felucca and Trammel were 6144 cells wide
/// up to Mondain's Legacy (UO Gold's `map0.mul` is 77,070,336 bytes, 768 x 512 blocks) and 7168 wide since. So the
/// width comes from the file: its block count over the facet's block rows.
pub const CLASSIC_FACET_HEIGHTS: [u32; 6] = [4096, 4096, 1600, 2048, 1448, 4096];

/// Map blocks in each `build/mapNlegacymul/NNNNNNNN.dat` entry of a `mapNLegacyMUL.uop` package.
pub const UOP_MAP_BLOCKS_PER_ENTRY: u32 = 4096;

/// Blocks a shipped `mapNLegacyMUL.uop` package holds past the facet: its last entry is one extra block.
pub const UOP_MAP_TRAILING_BLOCKS: u64 = 1;

/// The size of Classic map `map_index` holding `block_count` blocks: the facet's height
/// ([`CLASSIC_FACET_HEIGHTS`]) and the width those blocks fill. Up to `trailing_blocks` blocks past the last full
/// column are allowed (a UOP package's trailing block); any other remainder fails.
/// The entry editor's check of a `mapNLegacyMUL.uop` payload: whole map blocks, at least one.
pub fn check_uop_map_payload(bytes: &[u8]) -> eyre::Result<()> {
    if bytes.is_empty() || bytes.len() % MAP_BLOCK_FILE_SIZE != 0 {
        eyre::bail!("{} bytes are not whole map blocks of {MAP_BLOCK_FILE_SIZE} bytes", bytes.len());
    }
    Ok(())
}

pub fn classic_map_size(map_index: u32, block_count: u64, trailing_blocks: u64) -> eyre::Result<MapSizeCells> {
    let height = *CLASSIC_FACET_HEIGHTS
        .get(map_index as usize)
        .ok_or_else(|| eyre!("map{map_index} has no known facet height; pass the map size"))?;
    let height_blocks = u64::from(height / MapBlock::CELLS_PER_COLUMN);
    let width_blocks = block_count / height_blocks;
    let remainder = block_count % height_blocks;
    if width_blocks == 0 || remainder > trailing_blocks {
        eyre::bail!(
            "map{map_index}: {block_count} blocks do not fill whole columns of {height_blocks} blocks \
             (facet height {height}); pass the map size"
        );
    }
    let width = u32::try_from(width_blocks * u64::from(MapBlock::CELLS_PER_ROW))
        .map_err(|_| eyre!("map{map_index}: {block_count} blocks give an oversized map"))?;
    Ok(MapSizeCells { width, height })
}

/// The name of entry `entry_index` of a `map{map_index}LegacyMUL.uop` package.
fn uop_map_entry_name(map_index: u32, entry_index: u32) -> String {
    format!("build/map{map_index}legacymul/{entry_index:08}.dat")
}

/// The bytes of block data in a `map{map_index}LegacyMUL.uop` package: its entries from the first, in order,
/// up to the first missing one. The `x` variant packages (`map0xLegacyMUL.uop`, ...) name their entries like the
/// base map's.
pub fn uop_map_data_size(package: &crate::uop_container::package::UopPackage, map_index: u32) -> u64 {
    (0..)
        .map_while(|entry_index| package.get_file_by_name(&uop_map_entry_name(map_index, entry_index)))
        .map(|file| u64::from(file.decompressed_size()))
        .sum()
}

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
            out_cells[base_out] = MapCell {
                id: u16::from_le_bytes([raw_bytes[base_in], raw_bytes[base_in + 1]]),
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

    fn from_packed_bytes(bytes: &[u8], new_block: &mut MapBlock) -> eyre::Result<()> {
        if bytes.len() != Self::PACKED_SIZE {
            eyre::bail!("Invalid map block byte length {}", bytes.len());
        }
        let raw_block: &RawMapBlock = &cast_slice(bytes)[0];
        Self::from_raw_block(raw_block, new_block)
    }
}



/// Represents a map plane, which is a 2D grid of blocks.
pub struct MapPlane {
    pub index: u32,
    pub size_blocks: MapSizeBlocks,
    map_file_path: PathBuf,
    cached_block_indices: Vec<u32>,
    cached_blocks_arena: Vec<CachedBlock>,
    cached_blocks_free_list: Vec<u32>,
    cached_blocks_bitmask: Vec<u64>,
    read_buffer: Vec<u8>,
    pub blocks_loaded_version: u64,
    pub source: MapSource,
    /// The map index the file was written for, which names the entries of a UOP package. It differs from
    /// `index` when another facet's file stands in for this facet ([`MapPlane::for_facet`]).
    file_map_index: u32,
    map_diff: Option<MapDiff>,
    verdata: Option<Arc<Verdata>>,
}

pub enum MapSource {
    Mul(Arc<Mmap>),
    Uop(Arc<crate::uop_container::package::UopPackage>),
}

pub struct CachedBlock {
    pub block: MapBlock,
    pub last_accessed: std::time::Instant,
}

impl MapPlane {
    pub const EXTRA_BLOCKS_TO_CACHE_PER_SIDE: u32 = 8;

    pub fn size_cells(&self) -> MapSizeCells {
        MapSizeCells {
            width: self.size_blocks.width * MapBlock::CELLS_PER_ROW,
            height: self.size_blocks.height * MapBlock::CELLS_PER_COLUMN,
        }
    }

    pub fn block(&mut self, pos: MapBlockRelPos) -> Option<&MapBlock> {
        if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
            return None;
        }
        let idx = (pos.x * self.size_blocks.height) + pos.y;
        let arena_idx = self.cached_block_indices[idx as usize];
        if arena_idx != u32::MAX {
            let cached: &mut CachedBlock = &mut self.cached_blocks_arena[arena_idx as usize];
            cached.last_accessed = std::time::Instant::now();
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
    pub fn from_xy(x: f32, y: f32) -> Self {
        Self {
            x: x as u32,
            y: y as u32,
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

    pub fn init_with_diff(
        map_file_mul_path: PathBuf,
        map_index: u32,
        map_diff: MapDiff,
    ) -> eyre::Result<MapPlane> {
        let mut plane = Self::init_with_size(map_file_mul_path, map_index, None)?;
        plane.map_diff = Some(map_diff);
        Ok(plane)
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    /// Applies `map_diff` (a facet's `mapdifN`/`mapdiflN`): its blocks replace the file's.
    pub fn with_map_diff(mut self, map_diff: MapDiff) -> Self {
        self.map_diff = Some(map_diff);
        self
    }

    /// Reads this plane as facet `facet`, while the file keeps the map index it was opened with: pre-7.0 clients
    /// read Trammel from `map0.mul` (with the facet-1 difs).
    pub fn for_facet(mut self, facet: u32) -> Self {
        self.index = facet;
        self
    }

    /// The map index the file was written for (see [`MapPlane::for_facet`]).
    pub fn file_map_index(&self) -> u32 {
        self.file_map_index
    }

    pub fn init_with_size_and_diff(
        map_file_mul_path: PathBuf,
        map_index: u32,
        map_size_tiles_override: Option<MapSizeCells>,
        map_diff: MapDiff,
    ) -> eyre::Result<MapPlane> {
        let mut plane = Self::init_with_size(
            map_file_mul_path,
            map_index,
            map_size_tiles_override,
        )?;
        plane.map_diff = Some(map_diff);
        Ok(plane)
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

        let map_mmap = unsafe { Mmap::map(&map_file_mul_handle)? };
        let map_mmap = Arc::new(map_mmap);

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
            None => {
                // The size comes from the file: a pre-ML map0.mul/map1.mul is 6144 cells wide, a later one 7168.
                let file_size = map_file_mul_metadata.len();
                if file_size % MapBlock::PACKED_SIZE as u64 != 0 {
                    Err(eyre!(
                        "Malformed map{map_index}.mul: {file_size} bytes is not a whole number of {}-byte blocks",
                        MapBlock::PACKED_SIZE
                    ))
                } else {
                    classic_map_size(map_index, file_size / MapBlock::PACKED_SIZE as u64, 0)
                }
            }
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
                "Malformed map file: a {}x{} map takes {map_file_expected_size} bytes, the file has {}",
                map_size_tiles.width,
                map_size_tiles.height,
                map_file_mul_metadata.len()
            ));
        }

        let cached_block_indices =
            vec![u32::MAX; (map_size_blocks.width * map_size_blocks.height) as usize];

        let map_plane = MapPlane {
            index: map_index,
            size_blocks: map_size_blocks,
            map_file_path: map_file_mul_path.clone(),
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
            source: MapSource::Mul(map_mmap),
            file_map_index: map_index,
            map_diff: None,
            verdata: None,
        };
        Ok(map_plane)
    }

    pub fn init_uop(map_file_uop_path: PathBuf, map_index: u32) -> eyre::Result<MapPlane> {
        Self::init_uop_with_size(map_file_uop_path, map_index, None)
    }

    pub fn init_uop_with_size(
        map_file_uop_path: PathBuf,
        map_index: u32,
        map_size_tiles_override: Option<MapSizeCells>,
    ) -> eyre::Result<MapPlane> {
        let map_file_uop_path = map_file_uop_path
            .canonicalize()
            .wrap_err_with(|| format!("Check map{map_index}.uop path"))?;

        let package = crate::uop_container::package::UopPackage::load_with_mode(&map_file_uop_path, LoadMode::Eager)?;

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
            None => {
                // The size comes from the blocks the package's entries hold.
                let data_size = uop_map_data_size(&package, map_index);
                if data_size % MapBlock::PACKED_SIZE as u64 != 0 {
                    Err(eyre!(
                        "Malformed map{map_index} UOP: {data_size} bytes of entries is not a whole number of {}-byte blocks",
                        MapBlock::PACKED_SIZE
                    ))
                } else {
                    classic_map_size(map_index, data_size / MapBlock::PACKED_SIZE as u64, UOP_MAP_TRAILING_BLOCKS)
                }
            }
        }?;

        let map_size_blocks = MapSizeBlocks {
            width: map_size_tiles.width / MapBlock::CELLS_PER_ROW,
            height: map_size_tiles.height / MapBlock::CELLS_PER_COLUMN,
        };

        let cached_block_indices =
            vec![u32::MAX; (map_size_blocks.width * map_size_blocks.height) as usize];

        let map_plane = MapPlane {
            index: map_index,
            size_blocks: map_size_blocks,
            map_file_path: map_file_uop_path.clone(),
            source: MapSource::Uop(Arc::new(package)),
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
            file_map_index: map_index,
            map_diff: None,
            verdata: None,
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

        match &self.source {
            MapSource::Mul(mmap) => {
                let mmap = mmap.clone();
                self.load_blocks_mul(&mmap, blocks_to_load)
            }
            MapSource::Uop(package) => {
                let package = package.clone();
                self.load_blocks_uop(&package, blocks_to_load)
            }
        }
    }

    fn load_blocks_mul(&mut self, mmap: &Mmap, blocks_to_load: &[MapBlockRelPos]) -> eyre::Result<()> {
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

            let offset = start_idx as usize * MapBlock::PACKED_SIZE;
            let end = offset + num_blocks * MapBlock::PACKED_SIZE;

            if end > mmap.len() {
                eyre::bail!("Map index out of range for map plane {}", self.index);
            }

            let raw_blocks: &[RawMapBlock] = cast_slice(&mmap[offset..end]);
            for (i, raw_block) in raw_blocks.iter().enumerate() {
                let block_pos = indexed_blocks[range_start + i].pos;
                if block_pos.x >= self.size_blocks.width || block_pos.y >= self.size_blocks.height {
                    continue;
                }
                let idx = (block_pos.x * self.size_blocks.height) + block_pos.y;
                let arena_idx = self.cached_block_indices[idx as usize];
                if arena_idx == u32::MAX {
                    let mut new_block = MapBlock::default();
                    let block_idx = indexed_blocks[range_start + i].idx;
                    if let Some(diff_bytes) = self
                        .map_diff
                        .as_ref()
                        .map(|diff| diff.raw_block(block_idx))
                        .transpose()?
                        .flatten()
                    {
                        MapBlock::from_packed_bytes(diff_bytes, &mut new_block)?;
                    } else if let Some(verdata_bytes) = self
                        .verdata
                        .as_ref()
                        .map(|verdata| verdata.read_patch(VerFileId::Map, block_idx as i32))
                        .transpose()?
                        .flatten()
                    {
                        MapBlock::from_packed_bytes(&verdata_bytes, &mut new_block)?;
                    } else {
                        MapBlock::from_raw_block(raw_block, &mut new_block)?;
                    }
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

    fn load_blocks_uop(
        &mut self,
        package: &crate::uop_container::package::UopPackage,
        blocks_to_load: &[MapBlockRelPos],
    ) -> eyre::Result<()> {
        let mut blocks_to_load = blocks_to_load.to_vec();
        blocks_to_load.sort_unstable();
        blocks_to_load.dedup();

        for pos in blocks_to_load {
            if pos.x >= self.size_blocks.width || pos.y >= self.size_blocks.height {
                continue;
            }
            let idx = (pos.x * self.size_blocks.height) + pos.y;

            // In UOP maps, each entry contains 4096 blocks (64x64 blocks = 512x512 tiles).
            let chunk_idx = idx / UOP_MAP_BLOCKS_PER_ENTRY;
            let block_offset_in_chunk = (idx % UOP_MAP_BLOCKS_PER_ENTRY) as usize;

            let arena_idx = self.cached_block_indices[idx as usize];
            if arena_idx == u32::MAX {
                // Find the chunk in the UOP package.
                let chunk_name = uop_map_entry_name(self.file_map_index, chunk_idx);
                let file_entry = package
                    .get_file_by_name(&chunk_name)
                    .ok_or_else(|| eyre::eyre!("Missing UOP chunk: {}", chunk_name))?;

                let data = file_entry
                    .data()
                    .ok_or_else(|| eyre::eyre!("UOP chunk data not loaded: {}", chunk_name))?;

                let start = block_offset_in_chunk * MapBlock::PACKED_SIZE;
                let end = start + MapBlock::PACKED_SIZE;

                if end > data.len() {
                    eyre::bail!("Block offset out of range in UOP chunk {}", chunk_name);
                }

                let raw_block: &RawMapBlock = &cast_slice(&data[start..end])[0];

                let mut new_block = MapBlock::default();
                if let Some(diff_bytes) = self
                    .map_diff
                    .as_ref()
                    .map(|diff| diff.raw_block(idx))
                    .transpose()?
                    .flatten()
                {
                    MapBlock::from_packed_bytes(diff_bytes, &mut new_block)?;
                } else {
                    MapBlock::from_raw_block(raw_block, &mut new_block)?;
                }
                new_block.internal_coords = pos;

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
        Ok(())
    }
}

/// Loads map blocks from a memory-mapped source, returning them
/// without inserting into any cache.
pub fn load_blocks_from_mmap(
    mmap: &Mmap,
    blocks_to_load: &[MapBlockRelPos],
    size_blocks_height: u32,
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

            let offset = start_idx as usize * MapBlock::PACKED_SIZE;
            let end = offset + num_blocks * MapBlock::PACKED_SIZE;

            if end > mmap.len() {
                eyre::bail!("bg-loader: Map index out of range");
            }

            let raw_blocks: &[RawMapBlock] = cast_slice(&mmap[offset..end]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uop_map_check_wants_whole_blocks() {
        assert!(check_uop_map_payload(&[0; MAP_BLOCK_FILE_SIZE * 2]).is_ok());
        assert!(check_uop_map_payload(&[0; MAP_BLOCK_FILE_SIZE + 1]).is_err());
        assert!(check_uop_map_payload(&[]).is_err());
    }

    /// Felucca's map index.
    const FELUCCA: u32 = 0;
    /// Tokuno's map index.
    const TOKUNO: u32 = 4;
    /// UO Gold's `map0.mul` (6144 x 4096) and a later one (7168 x 4096), in bytes.
    const GOLD_MAP0_SIZE: u64 = 77_070_336;
    const ML_MAP0_SIZE: u64 = 89_915_392;
    const PRE_ML_FELUCCA_WIDTH: u32 = 6144;
    const ML_FELUCCA_WIDTH: u32 = 7168;
    const FELUCCA_HEIGHT: u32 = 4096;
    const TOKUNO_SIDE: u32 = 1448;

    fn blocks(file_size: u64) -> u64 {
        file_size / MapBlock::PACKED_SIZE as u64
    }

    #[test]
    fn felucca_width_comes_from_the_file() {
        let gold = classic_map_size(FELUCCA, blocks(GOLD_MAP0_SIZE), 0).unwrap();
        assert_eq!((gold.width, gold.height), (PRE_ML_FELUCCA_WIDTH, FELUCCA_HEIGHT));
        let ml = classic_map_size(FELUCCA, blocks(ML_MAP0_SIZE), 0).unwrap();
        assert_eq!((ml.width, ml.height), (ML_FELUCCA_WIDTH, FELUCCA_HEIGHT));
    }

    #[test]
    fn uop_packages_may_end_with_a_trailing_block() {
        let tokuno_blocks = u64::from(TOKUNO_SIDE / MapBlock::CELLS_PER_ROW).pow(2);
        let size = classic_map_size(TOKUNO, tokuno_blocks + UOP_MAP_TRAILING_BLOCKS, UOP_MAP_TRAILING_BLOCKS).unwrap();
        assert_eq!((size.width, size.height), (TOKUNO_SIDE, TOKUNO_SIDE));
        assert!(classic_map_size(TOKUNO, tokuno_blocks + UOP_MAP_TRAILING_BLOCKS, 0).is_err());
        assert!(classic_map_size(TOKUNO, tokuno_blocks + UOP_MAP_TRAILING_BLOCKS + 1, UOP_MAP_TRAILING_BLOCKS).is_err());
    }

    #[test]
    fn unknown_facets_need_a_size() {
        assert!(classic_map_size(CLASSIC_FACET_HEIGHTS.len() as u32, blocks(ML_MAP0_SIZE), 0).is_err());
    }
}
