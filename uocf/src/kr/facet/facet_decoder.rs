//! # UO Kingdom Reborn Facet Decoder
//!
//! This module provides functionality to decode `.bin` files from Kingdom Reborn facet `.uop` packages
//! back into a structured representation of map data, including land tiles, delimiters, and statics.
//!
//! ## KR Facet `.bin` (or .dat?) File Format
//!
//! The `facet.uop` file is a standard UOP package containing Zlib-compressed `.bin` files.
//! Each `.bin` file represents a 64x64 tile chunk of the map. The data within each bin is structured
//! as follows, laid out in column-major order (X is the outer loop, Y is the inner loop):
//!
//! 1.  **Header** (3 bytes)
//!     - `BYTE`: Facet ID (the map index, e.g., 0 for Felucca/Trammel).
//!     - `WORD` (Little Endian): File ID (the index of the 64x64 block in the map).
//!
//! 2.  **Tile Data** (4096 entries for a 64x64 grid)
//!     For each tile (X then Y):
//!     - `sbyte`: Z-coordinate of the land tile.
//!     - `ushort` (Little Endian): Graphic ID of the land tile (from `TileCollection.Values`).
//!     - `byte`: Unknown byte (from `TileCollection.Values`).
//!     - `byte`: Original tile ID low byte.
//!     - `byte`: Original tile ID high byte.
//!     - **Delimiters** (Variable size)
//!         - `BYTE`: `count` of delimiter entries. If 0, no further delimiter data for this tile.
//!         - For each entry:
//!             - `BYTE`: `direction` (0-7, representing relative position of neighbor).
//!             - `sbyte`: Z-coordinate of the adjacent tile.
//!             - `ushort` (Little Endian): Graphic ID of the adjacent tile (from `TileCollection.Values`).
//!             - `byte`: Unknown byte (from `TileCollection.Values`).
//!     - **Statics** (Variable size)
//!         - `BYTE`: Always 0.
//!         - `BYTE`: `count` of static items on this tile (only for the first static, 0 for others).
//!         - For each static:
//!             - `ushort` (Little Endian): Graphic ID of the static item.
//!             - `ushort` (Little Endian): Unknown (always 0).
//!             - `sbyte`: Z-coordinate of the static item.
//!             - `ushort` (Little Endian): Hue of the static item (with conditional logic).

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, eyre};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use crate::classic::map::MapBlock;
use crate::enhanced::facet_decoder::StaticTile;
use crate::uop;
use crate::uop::package::UopPackage;

// TODO: WE HAVE TO USE THOSE!
/*
#[cfg(feature ="tile_mappings_builder")]
use super::tile_mappings_builder::*;
#[cfg(not(feature ="tile_mappings_builder"))]
use super::tile_mappings_loader::*;
*/
// The encoder crate does the following, we have to do the reverse:
/*
    #[cfg(feature = "tile_mappings_builder")]
    {
        tile_dictionary = in_memory_dictionaries::get_in_memory_tile_dictionary();
        static_dictionary = in_memory_dictionaries::get_in_memory_static_dictionary();
    }

    #[cfg(not(feature = "tile_mappings_builder"))]
    {
        tile_dictionary = load_tile_dictionary(tile_dictionary_path)?;
        static_dictionary = load_static_dictionary(static_dictionary_path)?;
    }
*/

// region: --- Internal Raw File-Mapping Structs (Binary Format)

/// Represents a single delimiter entry within a KR facet tile.
///
/// This structure mirrors the binary layout used in Kingdom Reborn facets for stitching.
/// It is primarily used as an intermediate representation during decoding.
#[derive(Debug, Clone)]
pub struct KrFacetDelimiter {
    pub direction: u8,
    pub z: i8,
    pub graphic: u16,
    pub unknown: u8,
}

/// Represents a single static item within a facet tile.
#[derive(Debug, Clone)]
pub struct KrFacetStatic {
    pub graphic: u16,
    pub unknown1: u16,
    pub z: i8,
    pub hue: u16,
}

/// Represents a single tile within a 64x64 facet sector.
#[derive(Debug, Clone)]
pub struct KrFacetTile {
    pub z: i8,
    pub land_graphic: u16,
    pub unknown_byte: u8,
    pub original_id_low: u8,
    pub original_id_high: u8,
    pub delimiters: Vec<KrFacetDelimiter>,
    pub statics: Vec<KrFacetStatic>,
}

/// Represents the complete decoded content of a single 64x64 facet `.bin` file.
#[derive(Debug, Clone)]
pub struct DecodedKrFacet {
    pub facet_id: u8,
    pub file_id: u16,
    pub tiles: Vec<Vec<KrFacetTile>>, // 64x64 grid of tiles
}

// endregion: --- Internal Raw File-Mapping Structs

// region: --- Public API (Convenience & Application Use)

/// Decodes a raw KR facet `.bin` file directly into a classic 8x8 block representation.
///
/// This bypasses the intermediate `KrFacetTile` allocation entirely, ensuring
/// zero redundant vector allocations. The output represents the 64 classic 8x8 MapBlocks.
///
/// # Arguments
/// * `data` - The raw byte slice of the `.bin` file.
/// * `tile_dictionary` - The KR to Classic tile mapping.
/// * `static_dictionary` - A whitelist of valid classic static tile IDs.
pub fn decode_facet_bin(
    data: &[u8],
    tile_dictionary: &HashMap<u16, (u16, u8)>,
    static_dictionary: &HashSet<u16>,
) -> eyre::Result<crate::enhanced::facet_decoder::DecodedFacet> {
    let mut cursor: Cursor<&[u8]> = Cursor::new(data);

    // 1. Read Header
    let _facet_id: u8 = cursor.read_u8()?;
    let _file_id: u16 = cursor.read_u16::<LittleEndian>()?;

    // Initialize the 64 classic empty blocks
    let mut decoded_blocks: Vec<crate::enhanced::facet_decoder::DecodedBlockClassic> = (0..64)
        .map(|i| {
            let block_x = (i % 8) as u32;
            let block_y = (i / 8) as u32;
            let mut block = MapBlock::default();
            block.internal_coords.x = block_x;
            block.internal_coords.y = block_y;
            crate::enhanced::facet_decoder::DecodedBlockClassic {
                block,
                statics: Vec::new(),
            }
        })
        .collect();

    // 2. Read Tile Data (64x64 grid, column-major: X then Y)
    for x_64 in 0..64_u32 {
        for y_64 in 0..64_u32 {
            // Determine block coordinates
            let block_x: u32 = x_64 / 8;
            let block_y: u32 = y_64 / 8;
            let block_index: usize = (block_y * 8 + block_x) as usize;
            let cell_x: u32 = x_64 % 8;
            let cell_y: u32 = y_64 % 8;

            // --- Land Tile ---
            let z: i8 = cursor.read_i8()?;
            let kr_land_graphic: u16 = cursor.read_u16::<LittleEndian>()?;
            let _unknown_byte_land: u8 = cursor.read_u8()?;
            let _original_id_low: u8 = cursor.read_u8()?;
            let _original_id_high: u8 = cursor.read_u8()?;

            // Apply reverse mapping for the land graphic
            let classic_graphic_id = tile_dictionary
                .get(&kr_land_graphic)
                .map(|r| r.0)
                .unwrap_or(0);

            if let Ok(cell) = decoded_blocks[block_index]
                .block
                .cell_as_mut(cell_x, cell_y)
            {
                cell.id = classic_graphic_id;
                cell.z = z;
            }

            // --- Delimiters ---
            let delimiter_count: u8 = cursor.read_u8()?;
            if delimiter_count > 0 {
                // Skip delimiters (1 + 1 + 2 + 1 = 5 bytes per delimiter)
                cursor.set_position(cursor.position() + (delimiter_count as u64 * 5));
            }

            // --- Statics ---
            let always_0_byte1: u8 = cursor.read_u8()?; // Always 0
            let count_or_0_byte2: u8 = cursor.read_u8()?; // Count for first static, 0 for others

            let static_count = if always_0_byte1 == 0 && count_or_0_byte2 > 0 {
                count_or_0_byte2
            } else {
                0
            };

            for _ in 0..static_count {
                let graphic: u16 = cursor.read_u16::<LittleEndian>()?;
                let _unknown1: u16 = cursor.read_u16::<LittleEndian>()?;
                let static_z: i8 = cursor.read_i8()?;
                let hue: u16 = cursor.read_u16::<LittleEndian>()?;

                // Statics translation rule: The static is only pushed if it exists in the whitelist dictionary.
                if static_dictionary.contains(&graphic) {
                    decoded_blocks[block_index].statics.push(StaticTile {
                        graphic_id: graphic as u32,
                        x: cell_x as u8,
                        y: cell_y as u8,
                        z: static_z,
                        hue: hue as u32,
                    });
                }
            }
        }
    }

    Ok(crate::enhanced::facet_decoder::DecodedFacet {
        blocks: decoded_blocks,
    })
}

/// Reads and decodes a single 64x64 block from a KR facet UOP file.
pub fn read_facet_block(
    package: &UopPackage,
    map_index: u8,
    block_id: u32,
    tile_dictionary: &HashMap<u16, (u16, u8)>,
    static_dictionary: &HashSet<u16>,
) -> eyre::Result<crate::enhanced::facet_decoder::DecodedFacet> {
    use std::io::Write;
    let mut path_buf = [0u8; 64];
    let mut slice = &mut path_buf[..];
    write!(
        slice,
        "build/sectors/facet_0{}/{:08}.bin",
        map_index, block_id
    )
    .unwrap();
    let len = 64 - slice.len();
    let target_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
    let target_hash: u64 = uop::hash::hash_file_name_single(target_path);

    for file in package.iter_files() {
        if file.filename_hash() == target_hash {
            let decompressed_data = file.unpack()?;
            return decode_facet_bin(&decompressed_data, tile_dictionary, static_dictionary);
        }
    }
    Err(eyre!("File not found in package: {}", target_path))
}
