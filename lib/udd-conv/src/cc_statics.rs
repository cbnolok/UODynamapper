//! Build-time support for `staticsX.uddp`.
//!
//! This module converts Classic Client `staticsX.mul` and `staidxX.mul` files
//! into UODynamapper's compressed runtime format.
//!
//! Each 32x32 map chunk has a corresponding entry in the `.uddp` package,
//! containing the list of static items for that chunk with offsets relative to
//! the 32x32 chunk origin.
//! The lookup mode is `DenseId` where ID is the block index.

use color_eyre::eyre::{self};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use std::path::{Path, PathBuf};

use crate::classic_patches::{
    load_static_diff_if_enabled, load_verdata_if_enabled, ClassicPatchOptions,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use uocf::classic::map::MapPlane;
use uocf::classic::statics::{StaticTile, StaticsReader};
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;

pub struct CcStaticsBuildSummary {
    pub map_id: u32,
    pub chunk_count: u32,
    pub total_statics: u64,
}

pub fn convert_statics_mul_to_uddp_from_sources(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
) -> eyre::Result<CcStaticsBuildSummary> {
    convert_statics_mul_to_uddp_from_sources_with_patches(
        source_dirs,
        output_path,
        map_id,
        &ClassicPatchOptions::NONE,
    )
}

pub fn convert_statics_mul_to_uddp_from_sources_with_patches(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcStaticsBuildSummary> {
    let map_file_name = format!("map{}.mul", map_id);
    let map_uop_names = [
        format!("map{}LegacyMUL.uop", map_id),
        format!("map{}.uop", map_id),
        format!("map{}xLegacyMUL.uop", map_id),
        format!("map{}x.uop", map_id),
    ];
    let idx_file_name = format!("staidx{}.mul", map_id);
    let mul_file_name = format!("statics{}.mul", map_id);

    let (map_path, map_is_uop) = if let Some(path) =
        find_first_existing_file(source_dirs, &[&map_file_name])
    {
        (path, false)
    } else if let Some(path) = find_first_existing_file(
        source_dirs,
        &map_uop_names.iter().map(|name| name.as_str()).collect::<Vec<_>>(),
    ) {
        (path, true)
    } else {
        eyre::bail!("missing {} or map{} LegacyMUL UOP", map_file_name, map_id);
    };
    let idx_path = find_first_existing_file(source_dirs, &[&idx_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", idx_file_name))?;
    let mul_path = find_first_existing_file(source_dirs, &[&mul_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", mul_file_name))?;

    info!(
        "Converting statics for map {} to {}",
        map_id,
        output_path.display()
    );
    println!(
        "Using CC map{} source file for statics dimensions ({}): {}",
        map_id,
        if map_is_uop { "UOP" } else { "MUL" },
        map_path.display()
    );
    println!("Using CC staidx{} source file: {}", map_id, idx_path.display());
    println!("Using CC statics{} source file: {}", map_id, mul_path.display());

    // We use MapPlane just to resolve the map dimensions correctly.
    let plane = if map_is_uop {
        MapPlane::init_uop(map_path, map_id)?
    } else {
        MapPlane::init(map_path, map_id)?
    };
    let width_blocks = plane.size_blocks.width;
    let height_blocks = plane.size_blocks.height;

    convert_statics_mul_to_uddp_from_resolved_sources_with_dimensions(
        source_dirs,
        output_path,
        map_id,
        &idx_path,
        &mul_path,
        width_blocks,
        height_blocks,
        patch_options,
    )
}

pub fn convert_statics_mul_to_uddp_from_sources_with_dimensions_and_patches(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
    width_blocks: u32,
    height_blocks: u32,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcStaticsBuildSummary> {
    let idx_file_name = format!("staidx{}.mul", map_id);
    let mul_file_name = format!("statics{}.mul", map_id);

    let idx_path = find_first_existing_file(source_dirs, &[&idx_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", idx_file_name))?;
    let mul_path = find_first_existing_file(source_dirs, &[&mul_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", mul_file_name))?;

    println!("Using CC staidx{} source file: {}", map_id, idx_path.display());
    println!("Using CC statics{} source file: {}", map_id, mul_path.display());

    convert_statics_mul_to_uddp_from_resolved_sources_with_dimensions(
        source_dirs,
        output_path,
        map_id,
        &idx_path,
        &mul_path,
        width_blocks,
        height_blocks,
        patch_options,
    )
}

fn convert_statics_mul_to_uddp_from_resolved_sources_with_dimensions(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
    idx_path: &Path,
    mul_path: &Path,
    width_blocks: u32,
    height_blocks: u32,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcStaticsBuildSummary> {
    let width_chunks = width_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
    let height_chunks = height_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
    let total_chunks = width_chunks * height_chunks;

    let reader = StaticsReader::new_with_patches(
        idx_path,
        mul_path,
        width_blocks * 8,
        height_blocks * 8,
        load_static_diff_if_enabled(source_dirs, map_id, patch_options)?,
        load_verdata_if_enabled(source_dirs, patch_options)?,
    )?;
    info!("Loading statics into memory...");
    let store = reader.load_all()?;

    let mut builder = UddpBuilder::new(LookupMode::DenseId);

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut total_statics = 0u64;
    let mut scratch_tiles = Vec::new();

    for chunk_x in 0..width_chunks {
        for chunk_y in 0..height_chunks {
            scratch_tiles.clear();

            let block_origin_x = chunk_x * PACKAGE_CHUNK_BLOCK_DIM;
            let block_origin_y = chunk_y * PACKAGE_CHUNK_BLOCK_DIM;
            let block_end_x = (block_origin_x + PACKAGE_CHUNK_BLOCK_DIM).min(width_blocks);
            let block_end_y = (block_origin_y + PACKAGE_CHUNK_BLOCK_DIM).min(height_blocks);

            for block_x in block_origin_x..block_end_x {
                for block_y in block_origin_y..block_end_y {
                    let packed_tiles = store.block_tiles(block_x, block_y);
                    total_statics += packed_tiles.len() as u64;

                    let block_offset_x = ((block_x - block_origin_x) * 8) as u8;
                    let block_offset_y = ((block_y - block_origin_y) * 8) as u8;
                    for p in packed_tiles {
                        scratch_tiles.push(StaticTile {
                            graphic: p.graphic,
                            x_offset: block_offset_x + p.x_offset(),
                            y_offset: block_offset_y + p.y_offset(),
                            z: p.z,
                            _pad: 0,
                            hue: p.hue,
                        });
                    }
                }
            }

            let chunk_index = chunk_x * height_chunks + chunk_y;

            if scratch_tiles.is_empty() {
                // Must add an empty file to maintain DenseId continuity
                builder.add_file(AddFileRequest {
                    data_type: DataType::Static as u8,
                    compression: CompressionFlag::None, // No point compressing empty
                    width: 0,
                    height: 0,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(chunk_index),
                    data: &[],
                })?;
            } else {
                builder.add_file(AddFileRequest {
                    data_type: DataType::Static as u8,
                    compression: CompressionFlag::ZstdNoDict,
                    width: 0,
                    height: 0,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(chunk_index),
                    data: bytemuck::cast_slice(&scratch_tiles),
                })?;
            }
            pb.inc(1);
        }
    }

    pb.finish_with_message("Statics chunks packed");

    build_and_write_package(&mut builder, output_path)?;

    Ok(CcStaticsBuildSummary {
        map_id,
        chunk_count: total_chunks,
        total_statics,
    })
}
