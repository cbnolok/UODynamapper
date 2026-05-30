//! Build-time support for creating radar maps (`facet0X.dds` or `.ktx2`).
//!
//! This module emulates the logic from the Enhanced Map Converter to create
//! top-down radar textures for the Enhanced Client, using `tilemeta.uddp` as
//! the color source and various compression options.

use color_eyre::eyre::{self, Context};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use crate::bc7::{
    encode_to_bc7, preferred_bc7_encoder_backend, Bc7EncoderBackend, Bc7TextureData, ImageExtent,
    RawImageFormat,
};
use crate::classic_patches::{
    load_static_diff_if_enabled, load_verdata_if_enabled, ClassicPatchOptions,
};
use crate::classic_sources::{resolve_classic_map_source, SourceFormatPreference};
use crate::source_paths::find_first_existing_file;
use udd_assets::TileMetaPackage;
use uocf::classic::hues::load_hues;
use uocf::classic::statics::{PackedStaticTile, StaticsReader};

const fn bc7_backend_label(backend: Bc7EncoderBackend) -> &'static str {
    match backend {
        Bc7EncoderBackend::Analytical => "analytical (project BC7 encoder)",
        Bc7EncoderBackend::AnalyticalWide => "analytical-wide (project BC7 encoder)",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RadarFormat {
    /// Uncompressed RGBA8 (DDS)
    Rgba8,
    /// BC7 Compressed (DDS)
    Bc7,
    /// BC7 Compressed + KTX2 Container + Zstd Supercompression
    Bc7Ktx2,
}

impl RadarFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Rgba8 | Self::Bc7 => "dds",
            Self::Bc7Ktx2 => "ktx2",
        }
    }
}

pub struct RadarBuildOptions {
    pub format: RadarFormat,
    pub zstd_level: i32,
    pub classic_patches: ClassicPatchOptions,
    pub map_source_preference: SourceFormatPreference,
}

fn radar_static_top_z(statics_tile: PackedStaticTile, height: i8) -> i16 {
    i16::from(statics_tile.z) + i16::from(height)
}

// Classic radar selection is based on the rendered top of a static, not only its base Z.
fn select_radar_static<F>(
    static_tiles: &[PackedStaticTile],
    cell_x: u32,
    cell_y: u32,
    mut item_height: F,
) -> Option<PackedStaticTile>
where
    F: FnMut(u16) -> i8,
{
    let mut highest_static = None;

    for &statics_tile in static_tiles {
        if statics_tile.x_offset() as u32 != cell_x || statics_tile.y_offset() as u32 != cell_y {
            continue;
        }

        match highest_static {
            None => highest_static = Some(statics_tile),
            Some(previous) => {
                let candidate_top =
                    radar_static_top_z(statics_tile, item_height(statics_tile.graphic));
                let previous_top = radar_static_top_z(previous, item_height(previous.graphic));

                if candidate_top > previous_top
                    || (statics_tile.z > previous.z && candidate_top >= previous_top)
                {
                    highest_static = Some(statics_tile);
                }
            }
        }
    }

    highest_static
}

fn build_radar_rgba_pixels(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    map_id: u32,
    map_source_preference: SourceFormatPreference,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<(u32, u32, Vec<u8>)> {
    // 1. Locate source files
    let map_source = resolve_classic_map_source(source_dirs, map_id, map_source_preference)?;

    let statics_file_name = format!("statics{}.mul", map_id);
    let statics_path = find_first_existing_file(source_dirs, &[&statics_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", statics_file_name))?;

    let staidx_file_name = format!("staidx{}.mul", map_id);
    let staidx_path = find_first_existing_file(source_dirs, &[&staidx_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", staidx_file_name))?;

    let hues_path = find_first_existing_file(source_dirs, &["hues.mul"])
        .ok_or_else(|| eyre::eyre!("missing hues.mul"))?;

    println!(
        "Using CC radar map source file ({}): {}",
        map_source.format.label(),
        map_source.path.display()
    );
    println!("Using CC radar statics source file: {}", statics_path.display());
    println!("Using CC radar staidx source file: {}", staidx_path.display());
    println!("Using CC radar hues source file: {}", hues_path.display());

    println!("Loading tilemeta.uddp from {}...", tilemeta_path.display());
    let tilemeta = TileMetaPackage::load(tilemeta_path)?;

    println!("Loading hues.mul...");
    let hues = load_hues(&hues_path)?;

    println!("Initializing MapPlane...");
    let mut plane = map_source.load_plane(source_dirs, patch_options)?;
    let width_tiles = plane.size_cells().width;
    let height_tiles = plane.size_cells().height;

    println!("Loading all statics into memory...");
    let statics_reader = StaticsReader::new_with_patches(
        &staidx_path,
        &statics_path,
        width_tiles,
        height_tiles,
        load_static_diff_if_enabled(source_dirs, map_id, patch_options)?,
        load_verdata_if_enabled(source_dirs, patch_options)?,
    )?;
    let statics_store = statics_reader.load_all()?;

    println!(
        "Generating radar image ({}x{})...",
        width_tiles, height_tiles
    );

    let mut rgba_pixels = vec![0u8; (width_tiles * height_tiles * 4) as usize];

    let pb = ProgressBar::new(width_tiles as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} columns ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    for x in 0..width_tiles {
        let block_x = x / 8;
        let cell_x = x % 8;

        for y in 0..height_tiles {
            let block_y = y / 8;
            let cell_y = y % 8;

            let block_pos = uocf::classic::map::MapBlockRelPos {
                x: block_x,
                y: block_y,
            };
            if !plane.is_block_cached(&block_pos) {
                plane.load_blocks(&mut [block_pos])?;
            }

            let land_cell = plane.block(block_pos).unwrap().cell(cell_x, cell_y)?;
            let land_z = land_cell.z;
            let land_id = land_cell.id as u32;

            let static_tiles = statics_store.block_tiles(block_x, block_y);
            let highest_static = select_radar_static(static_tiles, cell_x, cell_y, |graphic| {
                tilemeta
                    .item_tile(graphic as u32)
                    .map(|meta| meta.height)
                    .unwrap_or(0)
            });

            let final_rgba = match highest_static {
                Some(statics_tile)
                    if radar_static_top_z(
                        statics_tile,
                        tilemeta
                            .item_tile(statics_tile.graphic as u32)
                            .map(|meta| meta.height)
                            .unwrap_or(0),
                    ) >= i16::from(land_z) =>
                {
                    if let Some(meta) = tilemeta.item_tile(statics_tile.graphic as u32) {
                        let base_color = meta.radar_color;

                        if statics_tile.hue > 0 && (statics_tile.hue as usize) < hues.len() {
                            let r_idx = (base_color[0] >> 3) as usize;
                            let hued_15 =
                                hues[statics_tile.hue as usize].color_table[r_idx.min(31)];

                            let hr = ((hued_15 >> 10) & 0x1F) as u8;
                            let hg = ((hued_15 >> 5) & 0x1F) as u8;
                            let hb = (hued_15 & 0x1F) as u8;

                            [
                                (hr << 3) | (hr >> 2),
                                (hg << 3) | (hg >> 2),
                                (hb << 3) | (hb >> 2),
                                255,
                            ]
                        } else {
                            [base_color[0], base_color[1], base_color[2], 255]
                        }
                    } else {
                        [0, 0, 0, 255]
                    }
                }
                _ => {
                    if let Some(meta) = tilemeta.land_tile(land_id) {
                        let base_color = meta.radar_color;
                        [base_color[0], base_color[1], base_color[2], 255]
                    } else {
                        [0, 0, 0, 255]
                    }
                }
            };

            let offset = ((y * width_tiles + x) * 4) as usize;
            rgba_pixels[offset..offset + 4].copy_from_slice(&final_rgba);
        }

        if x % 64 == 0 {
            plane.evict_idle_blocks(std::time::Duration::from_secs(0));
        }

        pb.inc(1);
    }
    pb.finish_with_message("Radar pixels generated");

    Ok((width_tiles, height_tiles, rgba_pixels))
}

pub fn build_facet_radar_bc7(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    map_id: u32,
) -> eyre::Result<Bc7TextureData> {
    build_facet_radar_bc7_with_options(
        source_dirs,
        tilemeta_path,
        map_id,
        SourceFormatPreference::Mul,
        &ClassicPatchOptions::NONE,
    )
}

pub fn build_facet_radar_bc7_with_patches(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    map_id: u32,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<Bc7TextureData> {
    build_facet_radar_bc7_with_options(
        source_dirs,
        tilemeta_path,
        map_id,
        SourceFormatPreference::Mul,
        patch_options,
    )
}

pub fn build_facet_radar_bc7_with_options(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    map_id: u32,
    map_source_preference: SourceFormatPreference,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<Bc7TextureData> {
    let (width_tiles, height_tiles, rgba_pixels) =
        build_radar_rgba_pixels(
            source_dirs,
            tilemeta_path,
            map_id,
            map_source_preference,
            patch_options,
        )?;
    build_radar_bc7_from_rgba(width_tiles, height_tiles, &rgba_pixels)
}

fn build_radar_bc7_from_rgba(
    width_tiles: u32,
    height_tiles: u32,
    rgba_pixels: &[u8],
) -> eyre::Result<Bc7TextureData> {
    let backend = preferred_bc7_encoder_backend();
    println!(
        "Compressing payload to BC7 with {} backend...",
        bc7_backend_label(backend)
    );
    let extent = ImageExtent::new(width_tiles, height_tiles)?;
    Ok(encode_to_bc7(
        rgba_pixels,
        extent,
        RawImageFormat::Rgba8888,
        backend,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn static_tile(graphic: u16, x: u8, y: u8, z: i8) -> PackedStaticTile {
        PackedStaticTile {
            graphic,
            xy_packed: (x & 0x07) | ((y & 0x07) << 3),
            z,
            hue: 0,
        }
    }

    fn height_for(graphic: u16) -> i8 {
        match graphic {
            1 => 20,
            2 => 1,
            3 => 4,
            _ => 0,
        }
    }

    #[test]
    fn radar_static_selection_prefers_highest_top_not_highest_base_z() {
        let tiles = [
            static_tile(1, 2, 3, 0),
            static_tile(2, 2, 3, 15),
        ];

        let selected = select_radar_static(&tiles, 2, 3, height_for).expect("selected static");
        let selected_graphic = selected.graphic;

        assert_eq!(selected_graphic, 1);
    }

    #[test]
    fn radar_static_selection_uses_base_z_as_tie_break() {
        let tiles = [
            static_tile(3, 1, 1, 6),
            static_tile(0, 1, 1, 10),
        ];

        let selected = select_radar_static(&tiles, 1, 1, height_for).expect("selected static");
        let selected_graphic = selected.graphic;

        assert_eq!(selected_graphic, 0);
    }

    #[test]
    fn radar_static_selection_filters_to_requested_cell() {
        let tiles = [
            static_tile(1, 2, 3, 0),
            static_tile(2, 4, 3, 15),
        ];

        let selected = select_radar_static(&tiles, 2, 3, height_for).expect("selected static");
        let selected_graphic = selected.graphic;

        assert_eq!(selected_graphic, 1);
    }
}

pub fn build_facet_radar_dds(
    source_dirs: &[PathBuf],
    tilemeta_path: &Path,
    output_path: &Path,
    map_id: u32,
    options: &RadarBuildOptions,
) -> eyre::Result<()> {
    let (width_tiles, height_tiles, rgba_pixels) =
        build_radar_rgba_pixels(
            source_dirs,
            tilemeta_path,
            map_id,
            options.map_source_preference,
            &options.classic_patches,
        )?;

    match options.format {
        RadarFormat::Bc7 => {
            let bc7_data = build_radar_bc7_from_rgba(width_tiles, height_tiles, &rgba_pixels)?;
            println!(
                "Writing DDS file with BC7 payload ({}x{})...",
                width_tiles, height_tiles
            );
            let mut output_file = std::fs::File::create(output_path)
                .wrap_err_with(|| format!("Failed to create output file: {:?}", output_path))?;
            use ddsfile::{Dds, DxgiFormat, NewDxgiParams};
            let params = NewDxgiParams {
                height: height_tiles,
                width: width_tiles,
                depth: None,
                format: DxgiFormat::BC7_UNorm,
                mipmap_levels: None,
                array_layers: None,
                caps2: None,
                is_cubemap: false,
                resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
                alpha_mode: ddsfile::AlphaMode::Opaque,
            };
            let mut dds = Dds::new_dxgi(params)?;
            dds.data = bc7_data.into_blocks();
            dds.write(&mut output_file)
                .map_err(|error| eyre::eyre!("Failed to write DDS: {}", error))?;
            println!("Successfully created facet0{}.dds (BC7 compressed)", map_id);
        }
        RadarFormat::Bc7Ktx2 => {
            return Err(eyre::eyre!(
                "KTX2 radar export lives in the uddconv_ktx2 helper crate; use the CLI or GUI path that routes through it"
            ));
        }
        RadarFormat::Rgba8 => {
            println!("Saving as uncompressed RGBA8 DDS...");
            let mut output_file = std::fs::File::create(output_path)
                .wrap_err_with(|| format!("Failed to create output file: {:?}", output_path))?;
            use ddsfile::{D3DFormat, Dds, NewD3dParams};
            let params = NewD3dParams {
                height: height_tiles,
                width: width_tiles,
                depth: None,
                format: D3DFormat::A8R8G8B8,
                mipmap_levels: None,
                caps2: None,
            };
            let mut dds = Dds::new_d3d(params)?;
            dds.data = rgba_pixels;
            dds.write(&mut output_file)
                .map_err(|e| eyre::eyre!("Failed to write DDS: {}", e))?;
            println!("Successfully created facet0{}.dds (RGBA8888)", map_id);
        }
    }

    Ok(())
}
