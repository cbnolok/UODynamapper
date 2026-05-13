//! # UO Kingdom Reborn Facet Encoder
//!
//! This module provides functionality to encode classic map data (land tiles and statics)
//! into the Kingdom Reborn (KR) client's facet `.bin` file format and associated `.uop` packages.
//!
//! ## KR Facet `.bin` File Format
//!
//! The KR facet `.bin` files are typically found within `facet.uop` packages. Each `.bin` file
//! represents a 64x64 tile chunk of the map. The data within each bin is structured as follows,
//! laid out in column-major order (X is the outer loop, Y is the inner loop):
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

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, eyre};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;

#[cfg(feature = "tile_mappings_builder")]
use super::tile_mappings_builder::*;
#[cfg(not(feature = "tile_mappings_builder"))]
use super::tile_mappings_loader::*;
use crate::classic::map::{MapCell, MapPlane};
use crate::enhanced::facet_decoder::StaticTile;
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::UopPackage;

/// Encodes a classic map plane and its associated statics into a facet.uop file.
///
/// This function can use either dictionaries loaded from binary files or in-memory dictionaries
/// based on the `in_memory_dictionaries` feature flag.
///
/// # Arguments
///
/// * `map_plane` - The overall `MapPlane` containing the land tile data.
/// * `all_statics` - A slice of all static items.
/// * `output_dir` - The directory where the output UOP file will be saved.
/// * `map_index` - The map index (e.g., 0 for Felucca/Trammel).
/// * `tile_dictionary_path` - The path to the `TileDictionary.bin` file (used if `in_memory_dictionaries` feature is not enabled).
/// * `static_dictionary_path` - The path to the `StaticDictionary.bin` file (used if `in_memory_dictionaries` feature is not enabled).
///
/// # Returns
///
/// A `Result` indicating success or an `eyre::Error` if an issue occurs.
// region: --- Public API (Convenience & Application Use)

/// Encodes a classic map plane and its associated statics into Kingdom Reborn formats.
///
/// This utility orchestrates the conversion of classic land blocks and statics into
/// the binary `.bin` blocks expected by the Kingdom Reborn `facet.uop` package.
/// It handles complex KR-specific features like tile dictionaries and whitelist-based
/// statics translation.
pub fn encode_map_plane(
    map_plane: &mut MapPlane, // Needs to be mutable to load blocks on demand
    all_statics: &[StaticTile],
    output_dir: &Path,
    map_index: u8,
    tile_dictionary_path: &Path,
    static_dictionary_path: &Path,
) -> eyre::Result<()> {
    // 1. Load supporting files (dictionaries)
    let tile_dictionary: HashMap<u16, (u16, u8)>;
    let static_dictionary: HashSet<u16>;

    #[cfg(feature = "tile_mappings_builder")]
    {
        tile_dictionary = get_tile_map();
        static_dictionary = get_statics_table()
            .into_iter()
            .map(|id| id as u16)
            .collect();
    }

    #[cfg(not(feature = "tile_mappings_builder"))]
    {
        tile_dictionary = load_tile_dictionary(tile_dictionary_path)?;
        static_dictionary = load_static_dictionary(static_dictionary_path)?;
    }

    // 2. Pre-process statics into a HashMap for efficient lookup by tile coordinate.
    let mut statics_map: HashMap<(u32, u32), Vec<&StaticTile>> = HashMap::new();
    for static_item in all_statics {
        let key = (static_item.x as u32, static_item.y as u32);
        statics_map.entry(key).or_default().push(static_item);
    }

    // 3. Initialize UOP Package
    let mut package = UopPackage::new_default();
    let map_size_cells = map_plane.size_cells();

    let chunks_x: u32 = map_size_cells.width / 64;
    let chunks_y: u32 = map_size_cells.height / 64;
    let mut file_id: u32 = 0;

    // 4. Loop through the map in 64x64 tile chunks
    for chunk_y in 0..chunks_y {
        for chunk_x in 0..chunks_x {
            let chunk_base_x: u32 = chunk_x * 64;
            let chunk_base_y: u32 = chunk_y * 64;

            // Pre-load all necessary 8x8 blocks for this 64x64 chunk
            let mut blocks_to_load = Vec::new();
            for y in 0..=8 {
                // Load an extra row/col for delimiter checks
                for x in 0..=8 {
                    let block_x_to_load: u32 = chunk_x * 8 + x;
                    let block_y_to_load: u32 = chunk_y * 8 + y;
                    if block_x_to_load < (map_size_cells.width / 8)
                        && block_y_to_load < (map_size_cells.height / 8)
                    {
                        blocks_to_load.push(crate::classic::map::MapBlockRelPos {
                            x: block_x_to_load,
                            y: block_y_to_load,
                        });
                    }
                }
            }
            map_plane.load_blocks(&mut blocks_to_load)?;

            // 5a. Generate .bin data in a Vec<u8>
            let bin_data = generate_kr_bin_data(
                map_plane,
                &statics_map,
                chunk_base_x,
                chunk_base_y,
                map_index,
                file_id,
                &tile_dictionary,
                &static_dictionary,
            )?;

            // 5d. Add the .bin buffer to the UopPackage
            use std::io::Write;
            let mut path_buf = [0u8; 64];
            let mut slice = &mut path_buf[..];
            write!(
                slice,
                "build/sectors/facet_0{}/{:08}.bin",
                map_index, file_id
            )
            .unwrap();
            let len = 64 - slice.len();
            let internal_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };

            package.add_file_from_memory(&bin_data, internal_path, CompressionFlag::Zlib)?; // Zlib compression as per C#

            file_id += 1;
        }
    }

    // 6. Save the UopPackage
    use std::io::Write;
    let mut path_buf = [0u8; 32];
    let mut slice = &mut path_buf[..];
    write!(slice, "facet{}.uop", map_index).unwrap();
    let len = 32 - slice.len();
    let facet_filename = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };

    let uop_path: std::path::PathBuf = output_dir.join(facet_filename);
    package.finalize_and_save(&uop_path)?; // Finalize and save

    Ok(())
}

/// Generates the in-memory `.bin` data for a single 64x64 map chunk for the Kingdom Reborn client.
///
/// This function takes a section of the `MapPlane` and associated static data, and converts it
/// into the specific binary format expected by the KR client's facet files.
///
/// # Arguments
///
/// * `map_plane` - The overall `MapPlane` containing the land tile data.
/// * `statics_map` - A HashMap of static items, keyed by their global (x,y) coordinates.
/// * `chunk_base_x` - The global X-coordinate of the top-left corner of this 64x64 chunk.
/// * `chunk_base_y` - The global Y-coordinate of the top-left corner of this 64x64 chunk.
/// * `map_index` - The map index (e.g., 0 for Felucca/Trammel).
/// * `file_id` - The file ID for this 64x64 block within the UOP package.
/// * `tile_dictionary` - A mapping from classic tile IDs to KR-specific graphic IDs and unknown byte.
/// * `static_dictionary` - A whitelist of valid static IDs.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the generated binary data, or an `eyre::Error` if an issue occurs.
pub fn generate_kr_bin_data(
    map_plane: &mut MapPlane,
    statics_map: &HashMap<(u32, u32), Vec<&StaticTile>>,
    chunk_base_x: u32,
    chunk_base_y: u32,
    map_index: u8,
    file_id: u32,
    tile_dictionary: &HashMap<u16, (u16, u8)>,
    static_dictionary: &HashSet<u16>,
) -> eyre::Result<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let mut cursor: Cursor<&mut Vec<u8>> = Cursor::new(&mut data);

    // 1. Write Header
    cursor.write_u8(map_index)?;
    cursor.write_u16::<LittleEndian>(file_id as u16)?;

    // 2. Write Tile Data (64x64 grid, column-major: X then Y)
    for x_offset in 0..64 {
        for y_offset in 0..64 {
            let global_x: u32 = chunk_base_x + x_offset;
            let global_y: u32 = chunk_base_y + y_offset;

            let cell: &MapCell = get_cell_from_plane(map_plane, global_x, global_y)
                .ok_or_else(|| eyre!("Cell not found at ({}, {})", global_x, global_y))?;

            // Land Tile
            cursor.write_i8(cell.z)?;
            let (kr_land_graphic_id, kr_land_unknown_byte) =
                *tile_dictionary.get(&cell.id).unwrap_or(&(0, 0)); // Default to 0 if not found
            cursor.write_u16::<LittleEndian>(kr_land_graphic_id)?;
            cursor.write_u8(kr_land_unknown_byte)?;
            cursor.write_u8((cell.id & 0xFF) as u8)?;
            cursor.write_u8(((cell.id >> 8) & 0xFF) as u8)?;

            // Delimiters
            write_kr_delimiters(&mut cursor, map_plane, global_x, global_y, tile_dictionary)?;

            // Statics
            if let Some(statics_on_tile) = statics_map.get(&(global_x, global_y)) {
                // The C# code writes a byte 0, then a byte for count (only for the first static), then the static data.
                // Subsequent statics on the same tile write byte 0, then byte 0, then static data.
                // This is unusual, so we replicate it exactly.
                for (i, static_item) in statics_on_tile.iter().enumerate() {
                    cursor.write_u8(0)?; // Always 0
                    if i == 0 {
                        cursor.write_u8(statics_on_tile.len() as u8)?; // Count for the first static
                    } else {
                        cursor.write_u8(0)?; // 0 for subsequent statics
                    }

                    // Only write if the static is in the whitelist
                    if static_dictionary.contains(&(static_item.graphic_id as u16)) {
                        cursor.write_u16::<LittleEndian>(static_item.graphic_id as u16)?;
                        cursor.write_u16::<LittleEndian>(0)?; // Unknown (always 0 in C#)
                        cursor.write_i8(static_item.z)?;
                        // Hue (with conditional logic from C#)
                        let kr_static_hue = if static_item.hue > 0 && static_item.hue < 0xB27 {
                            static_item.hue as u16
                        } else {
                            static_item.hue as u16 // This needs to be mapped from a hue dictionary if available
                        };
                        cursor.write_u16::<LittleEndian>(kr_static_hue)?;
                    } else {
                        // If not in whitelist, write default/empty static data
                        cursor.write_u16::<LittleEndian>(0)?;
                        cursor.write_u16::<LittleEndian>(0)?;
                        cursor.write_i8(0)?;
                        cursor.write_u16::<LittleEndian>(0)?;
                    }
                }
            } else {
                cursor.write_u8(0)?; // No statics, write 0 count
                cursor.write_u8(0)?; // No statics, write 0 count
            }
        }
    }

    Ok(data)
}

/// Helper to safely get a map cell from the plane, assuming blocks are pre-cached.
///
/// This function is a utility to retrieve a `MapCell` from the `MapPlane` given its
/// global (x,y) coordinates. It handles the conversion from global coordinates to
/// block and cell coordinates within the `MapPlane`.
///
/// # Arguments
///
/// * `map_plane` - The `MapPlane` containing the map data.
/// * `global_x` - The global X-coordinate of the cell.
/// * `global_y` - The global Y-coordinate of the cell.
///
/// # Returns
///
/// An `Option<&MapCell>` which is `Some` if the cell is found, or `None` otherwise.
fn get_cell_from_plane(map_plane: &mut MapPlane, global_x: u32, global_y: u32) -> Option<&MapCell> {
    // Calculate the block coordinates (8x8 blocks)
    let block_x: u32 = global_x / 8;
    let block_y: u32 = global_y / 8;
    // Calculate the cell coordinates within the block
    let cell_x: u32 = global_x % 8;
    let cell_y: u32 = global_y % 8;

    // Retrieve the block and then the cell
    map_plane
        .block(crate::classic::map::MapBlockRelPos {
            x: block_x,
            y: block_y,
        })
        .and_then(|block: &crate::classic::map::MapBlock| block.cell(cell_x, cell_y).ok())
}

/// Writes the delimiter data for a single tile for the Kingdom Reborn client.
///
/// This function implements the complex delimiter logic observed in the C# reference,
/// checking for tile edges and writing information about neighboring tiles.
///
/// # Arguments
///
/// * `cursor` - The `Cursor` to write the binary data to.
/// * `map_plane` - The `MapPlane` containing the map data.
/// * `global_x` - The global X-coordinate of the current tile.
/// * `global_y` - The global Y-coordinate of the current tile.
/// * `tile_dictionary` - A mapping from classic tile IDs to KR-specific graphic IDs and unknown byte.
///
/// # Returns
///
/// A `Result` indicating success or an `eyre::Error` if an issue occurs.
fn write_kr_delimiters(
    cursor: &mut Cursor<&mut Vec<u8>>,
    map_plane: &mut MapPlane,
    global_x: u32,
    global_y: u32,
    tile_dictionary: &HashMap<u16, (u16, u8)>,
) -> eyre::Result<()> {
    // The C# implementation checks for edges of 64x64 blocks, not 8x8 blocks.
    // The logic is based on `x - Math.Floor( x / 64 ) == 0` etc.
    // This means we are checking if the current tile is on the edge of the *current 64x64 chunk*.

    let on_left_edge_64: bool = global_x % 64 == 0;
    let on_top_edge_64: bool = global_y % 64 == 0;
    let on_right_edge_64: bool = global_x % 64 == 63;
    let on_bottom_edge_64: bool = global_y % 64 == 63;

    let map_size_cells = map_plane.size_cells();
    let mut delimiters_to_write: Vec<(u8, i8, u16, u8)> = Vec::new(); // (direction, z, graphic_id, unknown_byte)

    // Helper to get neighbor cell and add to delimiters_to_write
    let mut add_delimiter = |dx: i32, dy: i32, direction_byte: u8| -> eyre::Result<()> {
        let nx = global_x as i32 + dx;
        let ny = global_y as i32 + dy;

        // Ensure neighbor coordinates are within valid bounds of the map plane
        if nx >= 0
            && ny >= 0
            && (nx as u32) < map_size_cells.width
            && (ny as u32) < map_size_cells.height
        {
            if let Some(cell) = get_cell_from_plane(map_plane, nx as u32, ny as u32) {
                let (kr_land_graphic_id, kr_land_unknown_byte) =
                    *tile_dictionary.get(&cell.id).unwrap_or(&(0, 0));
                delimiters_to_write.push((
                    direction_byte,
                    cell.z,
                    kr_land_graphic_id,
                    kr_land_unknown_byte,
                ));
            }
        }
        Ok(())
    };

    // Replicating C# logic for different delimiter combinations
    // The C# code writes a count, then the delimiters. The count is determined by the number of delimiters written.
    // I will collect all delimiters first, then write the count, then the delimiters.

    // Left edge
    if on_left_edge_64 && global_x > 0 {
        add_delimiter(-1, 0, 0)?; // Left (0)
    }
    // Top edge
    if on_top_edge_64 && global_y > 0 {
        add_delimiter(0, -1, 2)?; // Top (2)
    }
    // Right edge
    if on_right_edge_64 && global_x < map_size_cells.width - 1 {
        add_delimiter(1, 0, 3)?; // Right (3)
    }
    // Bottom edge
    if on_bottom_edge_64 && global_y < map_size_cells.height - 1 {
        add_delimiter(0, 1, 5)?; // Bottom (5)
    }

    // Corner cases
    // TopLeft corner
    if on_top_edge_64 && on_left_edge_64 && global_x > 0 && global_y > 0 {
        add_delimiter(-1, -1, 1)?; // TopLeft (1)
    }
    // TopRight corner
    if on_top_edge_64 && on_right_edge_64 && global_x < map_size_cells.width - 1 && global_y > 0 {
        add_delimiter(1, -1, 7)?; // TopRight (7)
    }
    // BottomLeft corner
    if on_bottom_edge_64 && on_left_edge_64 && global_x > 0 && global_y < map_size_cells.height - 1
    {
        add_delimiter(-1, 1, 6)?; // BottomLeft (6)
    }
    // BottomRight corner
    if on_bottom_edge_64
        && on_right_edge_64
        && global_x < map_size_cells.width - 1
        && global_y < map_size_cells.height - 1
    {
        add_delimiter(1, 1, 4)?; // BottomRight (4)
    }

    // Write the actual delimiters to the cursor
    cursor.write_u8(delimiters_to_write.len() as u8)?;
    for (direction, z, graphic, unknown) in delimiters_to_write {
        cursor.write_u8(direction)?;
        cursor.write_i8(z)?;
        cursor.write_u16::<LittleEndian>(graphic)?;
        cursor.write_u8(unknown)?;
    }

    Ok(())
}

// endregion: --- Public API
