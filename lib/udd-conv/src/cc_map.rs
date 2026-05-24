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
//! //! At runtime, the loader only needs to replace the `r` field with the actual
//! GPU texture layer and update the `mode` bits, without re-parsing the
//! variable-length or misaligned original formats.
//!
//! ### Binary Specification (Map Chunk)
//!
//! Each file entry in the `mapX.uddp` package (Data Type: `Map`) contains a raw
//! byte array representing a 32x32 grid of tiles (1024 tiles total).
//!
//! **Structure**: `[Rg16u; 1024]`
//! **Total Size**: 4,096 bytes (uncompressed).
//!
//! **Rg16u Memory Layout**:
//! | Offset | Size | Name | Content |
//! | :--- | :--- | :--- | :--- |
//! | 0 | 2 | `r` | **Graphic ID** (Classic Client 16-bit land ID). |
//! | 2 | 1 | `g_low` | **Height Biased** (i8 height + 128 offset). |
//! | 3 | 1 | `g_high` | **Flags** (Bit 7: IsWet, Bits 0-3: Texture Size Mode). |
//!
//! Note: The `r` field is overwritten by the engine at runtime with the physical
//! texture array layer after the page is resolved.

use color_eyre::eyre::{self};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};

use crate::classic_patches::{
    load_map_diff_if_enabled, load_verdata_if_enabled, ClassicPatchOptions,
};
use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use log::{info, warn};
use uocf::classic::map::MapPlane;
use udd_assets::map_metadata::{encode_map_package_metadata, MapPackageMetadata};
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
    /// Packs tile ID, height, texture source mode, and is_wet flag into Rg16u format.
    ///
    /// This local definition mirrors `tile_atlas::Rg16u::pack` in the dynamapper crate.
    /// G channel high-byte layout:
    ///   bits 0-3: tex_size_bits (mode: 0=cc-small, 1=cc-big, 2=ec-atlas, 3=missing)
    ///   bit  7:   is_wet (IsWet tiledata flag → animated water distortion in shader)
    ///
    /// During build-time map conversion (this tool) we always pass is_wet=false;
    /// the runtime draw_mesh.rs fills the wet bit from TileMetaPackageRes.
    fn pack(tile_id: u16, height_i8: i8, tex_size_bits: u16, is_wet: bool) -> Self {
        let height_biased = (height_i8 as i16 + 128).clamp(0, 255) as u8;
        let wet_bit: u16 = if is_wet { 0x80 } else { 0 };
        let g = (height_biased as u16) | (((tex_size_bits & 0x0F) | wet_bit) << 8);
        Self { r: tile_id, g }
    }
}

const PACKAGE_CHUNK_BLOCK_DIM: u32 = 4;
const PACKAGE_CHUNK_TILE_DIM: usize = 32;
const PACKAGE_CHUNK_TEXEL_COUNT: usize = PACKAGE_CHUNK_TILE_DIM * PACKAGE_CHUNK_TILE_DIM;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CcMapSourcePreference {
    Mul,
    Uop,
}

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
    preference: CcMapSourcePreference,
) -> eyre::Result<CcMapBuildSummary> {
    convert_map_mul_to_uddp_from_sources_with_patches(
        source_dirs,
        output_path,
        map_id,
        preference,
        &ClassicPatchOptions::NONE,
    )
}

pub fn convert_map_mul_to_uddp_from_sources_with_patches(
    source_dirs: &[PathBuf],
    output_path: &Path,
    map_id: u32,
    preference: CcMapSourcePreference,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcMapBuildSummary> {
    let mul_name = format!("map{}.mul", map_id);
    let uop_names = [
        format!("map{}LegacyMUL.uop", map_id),
        format!("map{}.uop", map_id),
        format!("map{}xLegacyMUL.uop", map_id),
        format!("map{}x.uop", map_id),
    ];

    let (map_path, is_uop) = match preference {
        CcMapSourcePreference::Mul => {
            if let Some(path) = find_first_existing_file(source_dirs, &[&mul_name]) {
                (path, false)
            } else if let Some(path) =
                find_first_existing_file(source_dirs, &uop_names.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            {
                warn!("map{}.mul not found, falling back to uop", map_id);
                (path, true)
            } else {
                eyre::bail!("Missing map data for map{} (tried .mul and .uop)", map_id);
            }
        }
        CcMapSourcePreference::Uop => {
            if let Some(path) =
                find_first_existing_file(source_dirs, &uop_names.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            {
                (path, true)
            } else if let Some(path) = find_first_existing_file(source_dirs, &[&mul_name]) {
                warn!("No .uop found, falling back to .mul");
                (path, false)
            } else {
                eyre::bail!("Missing map data for map{} (tried .uop and .mul)", map_id);
            }
        }
    };

    info!(
        "Converting map {} ({}) to {}",
        map_id,
        if is_uop { "UOP" } else { "MUL" },
        output_path.display()
    );
    println!(
        "Using CC map{} source file ({}): {}",
        map_id,
        if is_uop { "UOP" } else { "MUL" },
        map_path.display()
    );

    let mut plane = if is_uop {
        MapPlane::init_uop(map_path, map_id)?
    } else if let Some(map_diff) = load_map_diff_if_enabled(source_dirs, map_id, patch_options)? {
        MapPlane::init_with_diff(map_path, map_id, map_diff)?
    } else {
        MapPlane::init(map_path, map_id)?
    };
    if !is_uop {
        if let Some(verdata) = load_verdata_if_enabled(source_dirs, patch_options)? {
            plane = plane.with_verdata(verdata);
        }
    } else if patch_options.any() {
        warn!("Classic map patch files are ignored when converting map{} from UOP.", map_id);
    }

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
                    texels[texel_index] = Rg16u::pack(cell.id, cell.z, 0, false);
                }
            }

            let chunk_index = chunk_x * height_chunks + chunk_y;
            builder.add_file(AddFileRequest {
                data_type: DataType::Map as u8,
                compression: CompressionFlag::Auto,
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

    let metadata = encode_map_package_metadata(MapPackageMetadata::new(
        map_id,
        width_blocks * 8,
        height_blocks * 8,
        total_chunks,
    ));
    builder.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: CompressionFlag::None,
        width: width_blocks * 8,
        height: height_blocks * 8,
        virtual_path: None,
        path_hash64: None,
        id: Some(total_chunks),
        data: &metadata,
    })?;

    build_and_write_package(&mut builder, output_path)?;

    Ok(CcMapBuildSummary {
        map_id,
        chunk_count: total_chunks,
        width_chunks,
        height_chunks,
    })
}
