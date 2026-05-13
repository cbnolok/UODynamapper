//! Build-time support for `mapX.uddp`.
//!
//! This module converts Classic Client `mapX.mul` files into UODynamapper's
//! compressed runtime format.
//!
//! The goal is to store each 32x32 map chunk as a separate entry in a `.uddp`
//! package, using a format that matches the GPU's metadata atlas:
//! - Each package chunk is 1024 tiles, laid out as a 32x32 texel grid.
//! - Each tile is stored as a 4-byte `Rg16u` (matching the shader's `TileUniform`).
//! - `Rg16u.r`: Initialized with the Classic `tile_id`.
//! - `Rg16u.g`: Initialized with `[height_biased:low 8 | mode:high 8]`.
//!
//! At runtime, the loader only needs to replace the `r` field with the actual
//! GPU texture layer and update the `mode` bits, without re-parsing the
//! variable-length or misaligned original formats.

use color_eyre::eyre::{self};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use uocf::classic::map::MapPlane;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

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

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;
const PACKAGE_CHUNK_TILE_DIM: usize = 32;
const PACKAGE_CHUNK_TEXEL_COUNT: usize = PACKAGE_CHUNK_TILE_DIM * PACKAGE_CHUNK_TILE_DIM;

pub struct CcMapBuildSummary {
    pub map_id: u32,
    pub chunk_count: u32,
    pub width_chunks: u32,
    pub height_chunks: u32,
}

pub fn convert_map_mul_to_uddp_from_sources(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
) -> eyre::Result<CcMapBuildSummary> {
    let map_file_name = format!("map{}.mul", map_id);
    let map_path = find_first_existing_file(source_dirs, &[&map_file_name])
        .ok_or_else(|| eyre::eyre!("missing {}", map_file_name))?;

    println!(
        "Converting {} to {}",
        map_path.display(),
        output_path.display()
    );

    let mut plane = MapPlane::init(map_path, map_id)?;
    let width_blocks = plane.size_blocks.width;
    let height_blocks = plane.size_blocks.height;
    let width_chunks = width_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
    let height_chunks = height_blocks.div_ceil(PACKAGE_CHUNK_BLOCK_DIM);
    let total_chunks = width_chunks * height_chunks;

    let mut builder = UddpBuilder::new(LookupMode::DenseId);

    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    for chunk_x in 0..width_chunks {
        for chunk_y in 0..height_chunks {
            let block_origin_x = chunk_x * PACKAGE_CHUNK_BLOCK_DIM;
            let block_origin_y = chunk_y * PACKAGE_CHUNK_BLOCK_DIM;
            let block_end_x = (block_origin_x + PACKAGE_CHUNK_BLOCK_DIM).min(width_blocks);
            let block_end_y = (block_origin_y + PACKAGE_CHUNK_BLOCK_DIM).min(height_blocks);

            let mut block_positions = Vec::with_capacity(PACKAGE_CHUNK_TEXEL_COUNT / 64);
            for block_x in block_origin_x..block_end_x {
                for block_y in block_origin_y..block_end_y {
                    block_positions.push(uocf::classic::map::MapBlockRelPos {
                        x: block_x,
                        y: block_y,
                    });
                }
            }

            plane.load_blocks(&mut block_positions)?;

            let mut texels = [Rg16u { r: 0, g: 0 }; PACKAGE_CHUNK_TEXEL_COUNT];
            for &pos in &block_positions {
                let block = plane
                    .block(pos)
                    .ok_or_else(|| eyre::eyre!("failed to load block at {},{}", pos.x, pos.y))?;
                let block_base_x = ((pos.x - block_origin_x) * 8) as usize;
                let block_base_y = ((pos.y - block_origin_y) * 8) as usize;

                for (cell_index, cell) in block.cells.iter().enumerate() {
                    let local_x = cell_index & 7;
                    let local_y = cell_index >> 3;
                    let texel_index = (block_base_y + local_y) * PACKAGE_CHUNK_TILE_DIM
                        + (block_base_x + local_x);
                    texels[texel_index] = Rg16u::pack(cell.id, cell.z, 0);
                }
            }

            let chunk_index = chunk_x * height_chunks + chunk_y;
            builder.add_file(AddFileRequest {
                data_type: DataType::Map as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: None,
                path_hash64: None,
                id: Some(chunk_index),
                data: bytemuck::cast_slice(&texels),
            })?;

            plane.evict_idle_blocks(std::time::Duration::from_secs(0));
            pb.inc(1);
        }
    }

    pb.finish_with_message("Map chunks packed");

    build_and_write_package(&mut builder, output_path)?;

    Ok(CcMapBuildSummary {
        map_id,
        chunk_count: total_chunks,
        width_chunks,
        height_chunks,
    })
}
