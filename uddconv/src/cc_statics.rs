//! Build-time support for `staticsX.uddp`.
//!
//! This module converts Classic Client `staticsX.mul` and `staidxX.mul` files
//! into UODynamapper's compressed runtime format.
//!
//! Each 8x8 map block has a corresponding entry in the `.uddp` package,
//! containing the list of static items for that block.
//! The lookup mode is `DenseId` where ID is the block index.

use std::path::{Path, PathBuf};
use color_eyre::eyre::{self};
use indicatif::{ProgressBar, ProgressStyle};

use uocf::classic::map::MapPlane;
use uocf::classic::statics::{StaticsReader, StaticTile};
use uocf::udd::{UddpBuilder, LookupMode, AddFileRequest, DataType, CompressionFlag};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;

pub struct CcStaticsBuildSummary {
    pub map_id: u32,
    pub block_count: u32,
    pub total_statics: u64,
}

pub fn convert_statics_mul_to_uddp_from_sources(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
) -> eyre::Result<CcStaticsBuildSummary> {
    let map_file_name = format!("map{}.mul", map_id);
    let idx_file_name = format!("staidx{}.mul", map_id);
    let mul_file_name = format!("statics{}.mul", map_id);

    let map_path = find_first_existing_file(source_dirs, &[&map_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", map_file_name))?;
    let idx_path = find_first_existing_file(source_dirs, &[&idx_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", idx_file_name))?;
    let mul_path = find_first_existing_file(source_dirs, &[&mul_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", mul_file_name))?;

    println!("Converting statics for map {} to {}", map_id, output_path.display());

    // We use MapPlane just to resolve the map dimensions correctly.
    let plane = MapPlane::init(map_path, map_id)?;
    let width_blocks = plane.size_blocks.width;
    let height_blocks = plane.size_blocks.height;
    let total_blocks = width_blocks * height_blocks;

    let mut reader = StaticsReader::new(&idx_path, &mul_path, width_blocks * 8, height_blocks * 8)?;
    println!("Loading statics into memory...");
    let store = reader.load_all()?;
    
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    
    let pb = ProgressBar::new(total_blocks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} blocks ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut total_statics = 0u64;
    let mut scratch_tiles = Vec::new();

    for x in 0..width_blocks {
        for y in 0..height_blocks {
            let packed_tiles = store.block_tiles(x, y);
            let block_index = x * height_blocks + y;
            
            if packed_tiles.is_empty() {
                // Must add an empty file to maintain DenseId continuity
                builder.add_file(AddFileRequest {
                    data_type: DataType::Static as u8,
                    compression: CompressionFlag::None, // No point compressing empty
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(block_index),
                    data: &[],
                })?;
            } else {
                total_statics += packed_tiles.len() as u64;
                scratch_tiles.clear();
                for p in packed_tiles {
                    scratch_tiles.push(StaticTile {
                        graphic: p.graphic,
                        x_offset: p.x_offset(),
                        y_offset: p.y_offset(),
                        z: p.z,
                        _pad: 0,
                        hue: p.hue,
                    });
                }
                
                builder.add_file(AddFileRequest {
                    data_type: DataType::Static as u8,
                    compression: CompressionFlag::ZstdNoDict,
                    virtual_path: None,
                    path_hash64: None,
                    id: Some(block_index),
                    data: bytemuck::cast_slice(&scratch_tiles),
                })?;
            }
            pb.inc(1);
        }
    }
    
    pb.finish_with_message("Statics packed");
    
    build_and_write_package(&mut builder, output_path)?;

    Ok(CcStaticsBuildSummary {
        map_id,
        block_count: total_blocks,
        total_statics,
    })
}
