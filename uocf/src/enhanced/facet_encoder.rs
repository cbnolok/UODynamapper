use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, eyre};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use crate::classic::hues::{HueEntry, load_hues};
use crate::classic::map::{MapBlockRelPos, MapCell, MapPlane};
use crate::classic::radarcol::load_radarcol;
use crate::enhanced::facet_decoder::StaticTile;
use crate::uop::file::CompressionFlag;
use crate::uop::package::UopPackage;
use crate::utils::color::Rgb555;

use ddsfile::{D3DFormat, Dds, NewD3dParams};

// region: --- Public API (Convenience & Application Use)

/// Encodes a classic map plane and its associated statics into Enhanced Client formats.
///
/// This utility orchestrates the conversion of classic land blocks and statics into
/// both the binary `.bin` blocks for `facet.uop` and a high-resolution `facet.dds`
/// texture for the terrain renderer.
pub fn encode_map_plane(
    map_plane: &mut MapPlane, // Needs to be mutable to load blocks on demand
    all_statics: &[StaticTile],
    radarcol_path: &Path,
    hues_path: &Path,
    output_dir: &Path,
    map_index: u8,
) -> eyre::Result<()> {
    // 1. Load supporting files
    let radarcol_data: Vec<Rgb555> = load_radarcol(radarcol_path)?;
    let hues_data: Vec<HueEntry> = load_hues(hues_path)?;

    // 2. Pre-process statics into a HashMap for efficient lookup by tile coordinate.
    let mut statics_map: HashMap<(u32, u32), Vec<&StaticTile>> = HashMap::new();
    for static_item in all_statics {
        let key = (static_item.x as u32, static_item.y as u32);
        statics_map.entry(key).or_default().push(static_item);
    }

    // 3. Initialize UOP Package and DDS image buffer
    let mut package = UopPackage::new_default();
    let map_size_cells = crate::classic::map::MapSizeCells {
        width: map_plane.size_blocks.width * 8,
        height: map_plane.size_blocks.height * 8,
    };
    let mut dds_pixel_buffer: Vec<u8> =
        vec![0u8; (map_size_cells.width * map_size_cells.height * 4) as usize]; // RGBA8

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
                        blocks_to_load.push(MapBlockRelPos {
                            x: block_x_to_load,
                            y: block_y_to_load,
                        });
                    }
                }
            }
            map_plane.load_blocks(&mut blocks_to_load)?;

            // 5a. Generate .bin data in a Vec<u8>
            let bin_data = generate_bin_data(
                map_plane,
                &statics_map,
                chunk_base_x,
                chunk_base_y,
                map_index,
                file_id,
            )?;

            // 5c. Calculate tile colors and write to DDS buffer
            for y_offset in 0..64 {
                for x_offset in 0..64 {
                    let global_x: u32 = chunk_base_x + x_offset;
                    let global_y: u32 = chunk_base_y + y_offset;
                    let color_rgb555: Rgb555 = calculate_tile_color(
                        map_plane,
                        &statics_map,
                        &radarcol_data,
                        &hues_data,
                        global_x,
                        global_y,
                    )?;
                    let (r, g, b, a) = color_rgb555.as_rgba8888().components();
                    let dds_index: usize =
                        ((global_y * map_size_cells.width + global_x) * 4) as usize;
                    dds_pixel_buffer[dds_index] = r;
                    dds_pixel_buffer[dds_index + 1] = g;
                    dds_pixel_buffer[dds_index + 2] = b;
                    dds_pixel_buffer[dds_index + 3] = a; // Alpha
                }
            }

            // 5d. Add the .bin buffer to the UopPackage
            use std::io::Write;
            let mut path_buf = [0u8; 64];
            let mut slice = &mut path_buf[..];
            write!(slice, "build/sectors/facet_0{}/{:08}.bin", map_index, file_id).unwrap();
            let len = 64 - slice.len();
            let internal_path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
            package.add_file_from_memory(&bin_data, internal_path, CompressionFlag::Zlib)?;

            file_id += 1;
        }
    }

    // 6. Save the UopPackage
    use std::io::Write;
    let mut package_path_buf = [0u8; 32];
    let facet_filename = {
        let mut slice = &mut package_path_buf[..];
        write!(slice, "facet{}.uop", map_index).unwrap();
        let written = 32 - slice.len();
        unsafe { std::str::from_utf8_unchecked(&package_path_buf[..written]) }
    };
    let uop_path: std::path::PathBuf = output_dir.join(facet_filename);
    package.finalize_and_save(&uop_path)?;

    // 7. Save the DDS buffer
    let mut dds_path_buf = [0u8; 32];
    let dds_filename = {
        let mut slice = &mut dds_path_buf[..];
        write!(slice, "facet{}.dds", map_index).unwrap();
        let written = 32 - slice.len();
        unsafe { std::str::from_utf8_unchecked(&dds_path_buf[..written]) }
    };
    let dds_path: std::path::PathBuf = output_dir.join(dds_filename);
    save_dds(
        &dds_path,
        &dds_pixel_buffer,
        map_size_cells.width,
        map_size_cells.height,
    )?;

    Ok(())
}

/// Saves a raw RGBA8 pixel buffer as a DXT1 compressed DDS file.
fn save_dds(path: &Path, data: &[u8], width: u32, height: u32) -> eyre::Result<()> {
    // This requires the `ddsfile` crate. Add `ddsfile = "0.6.1"` to Cargo.toml
    let params = NewD3dParams {
        height,
        width,
        depth: None,
        format: D3DFormat::DXT1,
        mipmap_levels: None,
        caps2: None,
    };
    let mut dds = Dds::new_d3d(params)?;
    dds.data = data.to_vec();

    let mut file = fs::File::create(path)?;
    dds.write(&mut file)?;
    Ok(())
}

/// Calculates the final color of a map tile for the DDS texture.
fn calculate_tile_color(
    map_plane: &mut MapPlane,
    statics_map: &HashMap<(u32, u32), Vec<&StaticTile>>,
    radarcol: &[Rgb555],
    hues: &[HueEntry],
    global_x: u32,
    global_y: u32,
) -> eyre::Result<Rgb555> {
    let land_cell: &MapCell = get_cell_from_plane(map_plane, global_x, global_y)
        .ok_or_else(|| eyre!("Missing land cell at ({}, {})", global_x, global_y))?;

    let statics_on_tile: Option<&Vec<&StaticTile>> = statics_map.get(&(global_x, global_y));

    let highest_z = land_cell.z;
    let mut highest_is_static = false;
    let mut highest_static: Option<&StaticTile> = None;

    if let Some(statics) = statics_on_tile {
        if let Some(top_static) = statics.iter().max_by_key(|s| s.z) {
            if top_static.z > highest_z {
                //highest_z = top_static.z;
                highest_is_static = true;
                highest_static = Some(top_static);
            }
        }
    }

    if highest_is_static {
        let static_item: &StaticTile = highest_static.unwrap();
        let graphic: u32 = static_item.graphic_id;
        if graphic as usize >= radarcol.len() {
            return Ok(Rgb555::new_from_val(0)); // Invalid graphic ID
        }

        if static_item.hue > 0 && (static_item.hue as usize) < hues.len() {
            let hue_entry: &HueEntry = &hues[static_item.hue as usize];
            let color_index: u8 = radarcol[graphic as usize].r();
            Ok(Rgb555::new_from_val(
                hue_entry.color_table[color_index as usize],
            ))
        } else {
            Ok(radarcol[graphic as usize])
        }
    } else {
        let graphic: u16 = land_cell.id;
        if graphic as usize >= radarcol.len() {
            return Ok(Rgb555::new_from_val(0)); // Invalid graphic ID
        }
        Ok(radarcol[graphic as usize])
    }
}

/// Helper to safely get a map cell from the plane, assuming blocks are pre-cached.
fn get_cell_from_plane(map_plane: &mut MapPlane, global_x: u32, global_y: u32) -> Option<&MapCell> {
    // TODO: comment divisions and reminders and implement them with bitwise operations for speed.
    let block_x: u32 = global_x / 8;
    let block_y: u32 = global_y / 8;
    let cell_x: u32 = global_x % 8;
    let cell_y: u32 = global_y % 8;

    map_plane
        .block(MapBlockRelPos {
            x: block_x,
            y: block_y,
        })
        .and_then(|block: &crate::classic::map::MapBlock| block.cell(cell_x, cell_y).ok())
}

/// Writes the delimiter data for a single tile.
fn write_delimiters(
    cursor: &mut Cursor<&mut Vec<u8>>,
    map_plane: &mut MapPlane,
    global_x: u32,
    global_y: u32,
) -> eyre::Result<()> {
    // TODO: comment divisions and reminders and implement them with bitwise operations for speed.

    let map_size_tiles = crate::classic::map::MapSizeCells {
        width: map_plane.size_blocks.width * 8,
        height: map_plane.size_blocks.height * 8,
    };
    let mut delimiters: Vec<(u8, i8, u16)> = Vec::new();

    let on_left_edge: bool = global_x % 8 == 0;
    let on_top_edge: bool = global_y % 8 == 0;
    let on_right_edge: bool = global_x % 8 == 7;
    let on_bottom_edge: bool = global_y % 8 == 7;

    if !on_left_edge && !on_top_edge && !on_right_edge && !on_bottom_edge {
        cursor.write_u8(0)?;
        return Ok(());
    }

    let neighbors = [
        (global_x.wrapping_sub(1), global_y, 0), // Left
        (global_x.wrapping_sub(1), global_y.wrapping_sub(1), 1), // TopLeft
        (global_x, global_y.wrapping_sub(1), 2), // Top
        (global_x + 1, global_y, 3),             // Right
        (global_x + 1, global_y + 1, 4),         // BottomRight
        (global_x, global_y + 1, 5),             // Bottom
        (global_x.wrapping_sub(1), global_y + 1, 6), // BottomLeft
        (global_x + 1, global_y.wrapping_sub(1), 7), // TopRight
    ];

    for (nx, ny, direction) in neighbors.iter() {
        if *nx < map_size_tiles.width && *ny < map_size_tiles.height {
            let should_check = match *direction {
                0 => on_left_edge,
                1 => on_left_edge && on_top_edge,
                2 => on_top_edge,
                3 => on_right_edge,
                4 => on_right_edge && on_bottom_edge,
                5 => on_bottom_edge,
                6 => on_left_edge && on_bottom_edge,
                7 => on_right_edge && on_top_edge,
                _ => false,
            };

            if should_check {
                if let Some(cell) = get_cell_from_plane(map_plane, *nx, *ny) {
                    delimiters.push((*direction, cell.z, cell.id));
                }
            }
        }
    }

    cursor.write_u8(delimiters.len() as u8)?;
    for (direction, z, graphic) in delimiters {
        cursor.write_u8(direction)?;
        cursor.write_i8(z)?;
        cursor.write_u32::<LittleEndian>(graphic as u32)?;
    }

    Ok(())
}

/// Generates the in-memory `.bin` data for a single 64x64 map chunk.
fn generate_bin_data(
    map_plane: &mut MapPlane,
    statics_map: &HashMap<(u32, u32), Vec<&StaticTile>>,
    chunk_base_x: u32,
    chunk_base_y: u32,
    map_index: u8,
    file_id: u32,
) -> eyre::Result<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let mut cursor: Cursor<&mut Vec<u8>> = Cursor::new(&mut data);

    cursor.write_u8(map_index)?;
    cursor.write_u16::<LittleEndian>(file_id as u16)?;

    for x_offset in 0..64 {
        for y_offset in 0..64 {
            let global_x: u32 = chunk_base_x + x_offset;
            let global_y: u32 = chunk_base_y + y_offset;

            let cell: &MapCell = get_cell_from_plane(map_plane, global_x, global_y)
                .ok_or_else(|| eyre!("Cell not found at ({}, {})", global_x, global_y))?;

            cursor.write_i8(cell.z)?;
            cursor.write_u16::<LittleEndian>(cell.id)?;

            write_delimiters(&mut cursor, map_plane, global_x, global_y)?;

            if let Some(statics_on_tile) = statics_map.get(&(global_x, global_y)) {
                cursor.write_u8(statics_on_tile.len() as u8)?;
                for static_item in statics_on_tile {
                    cursor.write_u32::<LittleEndian>(static_item.graphic_id)?;
                    cursor.write_i8(static_item.z)?;
                    cursor.write_u32::<LittleEndian>(static_item.hue)?;
                }
            } else {
                cursor.write_u8(0)?;
            }
        }
    }

    Ok(data)
}

// endregion: --- Public API
