//! Build-time support for `mapX.uddp`.
//!
//! This module converts Classic Client `mapX.mul` files into UODynamapper's
//! compressed runtime format.
//!
//! The goal is to store each 8x8 map block as a separate entry in a `.uddp`
//! package, using a format that matches the GPU's metadata atlas:
//! - Each block is 64 tiles.
//! - Each tile is stored as a 4-byte `Rg16u` (matching the shader's `TileUniform`).
//! - `Rg16u.r`: Initialized with the Classic `tile_id`.
//! - `Rg16u.g`: Initialized with `[height_biased:low 8 | mode:high 8]`.
//!
//! At runtime, the loader only needs to replace the `r` field with the actual
//! GPU texture layer and update the `mode` bits, without re-parsing the
//! variable-length or misaligned original formats.

use std::path::{Path, PathBuf};
use color_eyre::eyre::{self};
use indicatif::{ProgressBar, ProgressStyle};

use uocf::classic::map::{MapPlane};
use uocf::udd::{UddpBuilder, LookupMode, AddFileRequest, DataType, CompressionFlag};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;

// We need Rg16u from dynamapper, but uddconv doesn't depend on dynamapper.
// We'll redefine a compatible struct here or use a raw [u8; 4].
// Since this is a build-time tool, we just need to match the packing logic.

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Rg16u {
    r: u16,
    g: u16,
}

impl Rg16u {
    fn pack(tile_id: u16, height_i8: i8, tex_size_bits: u16) -> Self {
        let height_biased = (height_i8 as i16 + 128).clamp(0, 255) as u8;
        let g = (height_biased as u16) | ((tex_size_bits & 0xFF) << 8);
        Self { r: tile_id, g }
    }
}

pub struct CcMapBuildSummary {
    pub map_id: u32,
    pub block_count: u32,
    pub width_blocks: u32,
    pub height_blocks: u32,
}

pub fn convert_map_mul_to_uddp_from_sources(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
) -> eyre::Result<CcMapBuildSummary> {
    let map_file_name = format!("map{}.mul", map_id);
    let map_path = find_first_existing_file(source_dirs, &[&map_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", map_file_name))?;

    println!("Converting {} to {}", map_path.display(), output_path.display());

    let mut plane = MapPlane::init(map_path, map_id)?;
    let width_blocks = plane.size_blocks.width;
    let height_blocks = plane.size_blocks.height;
    let total_blocks = width_blocks * height_blocks;

    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    
    let pb = ProgressBar::new(total_blocks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} blocks ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    // We process the map in batches of blocks to improve disk I/O.
    let batch_size = 1024;
    let mut current_batch = Vec::with_capacity(batch_size);

    for x in 0..width_blocks {
        for y in 0..height_blocks {
            current_batch.push(uocf::classic::map::MapBlockRelPos { x, y });

            if current_batch.len() >= batch_size || (x == width_blocks - 1 && y == height_blocks - 1) {
                plane.load_blocks(&mut current_batch)?;

                for &pos in &current_batch {
                    let block = plane.block(pos).ok_or_else(|| eyre::eyre!("failed to load block at {},{}", pos.x, pos.y))?;
                    
                    let mut texels = [Rg16u { r: 0, g: 0 }; 64];
                    for (i, cell) in block.cells.iter().enumerate() {
                        texels[i] = Rg16u::pack(cell.id, cell.z, 0);
                    }
                    
                    let block_index = pos.x * height_blocks + pos.y;
                    builder.add_file(AddFileRequest {
                        data_type: DataType::Map as u8,
                        compression: CompressionFlag::ZstdNoDict,
                        virtual_path: None,
                        path_hash64: None,
                        id: Some(block_index),
                        data: bytemuck::cast_slice(&texels),
                    })?;
                    
                    pb.inc(1);
                }
                
                // Evict blocks from plane to keep memory usage low.
                // MapPlane doesn't have a direct "clear cache" but we can use evict_idle_blocks with 0 timeout.
                plane.evict_idle_blocks(std::time::Duration::from_secs(0));
                current_batch.clear();
            }
        }
    }
    
    pb.finish_with_message("Map blocks packed");
    
    build_and_write_package(&mut builder, output_path)?;

    Ok(CcMapBuildSummary {
        map_id,
        block_count: total_blocks,
        width_blocks,
        height_blocks,
    })
}
