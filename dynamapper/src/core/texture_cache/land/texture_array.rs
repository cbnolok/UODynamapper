#![allow(unused)]

use crate::{
    core::texture_cache::TextureResidencyPlan,
    core::uo_files_loader::TexMap2DRes,
    external_data::settings::{LossyTextureCompressionBackend, SectGraphics},
    prelude::*,
    util_lib::image::*,
};
use bevy::{
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{
        AddressMode, Extent3d, FilterMode, TextureDimension, TextureFormat, TextureUsages,
    },
};
use std::sync::OnceLock;
use uddconv::bc7::{Bc7EncoderBackend, ImageExtent, VramTextureEncoding, VramTextureFormat};
use uocf::classic::land_texture::{LandTextureSize, TexMap};

//pub const TEXTURE_UNUSED_ID: u32 = 0x007F;

////////////////////////////////////////////////////////////////////////////////
// 1. Texture Array Creation
////////////////////////////////////////////////////////////////////////////////

// The layer counts are intentionally set to the total number of unique textures in the UO
// data files (~1869 valid IDs). This allows the app to hold every texture resident at once
// without any LRU eviction, which is required when the user wants to zoom out to see the
// entire map or render it to a disk image.
// If you want to save VRAM at the cost of LRU eviction, lower these values.
//
// Full map VRAM budget:
//   Uncompressed (Rgba8UnormSrgb):
//     Small: 2048 layers × 64×64×4 B   =   32 MB
//     Big:   2048 layers × 128×128×4 B = 128 MB
//   BC7-compressed (Bc7RgbaUnormSrgb), ~8:1 lossless-quality ratio:
//     Small: 2048 layers × 64×64/2 B   =    4 MB
//     Big:   2048 layers × 128×128/2 B =   16 MB
//
// NOTE on GPU texture compression: BCn formats (BC1/BC7) are lossy and must be pre-compressed
// offline or on-the-fly. UODynamapper routes BC7 conversion through the shared `uddconv` crate,
// which prefers `dds` as the portable baseline and can otherwise
// fall back to `block_compression` or the Intel ISPC backend.
// The tile-atlas (Rg16Uint) cannot be compressed at all (integer formats are not supported by BCn).
// ── Texture Array sizing constants ──────────────────────────────────────────
pub const TEXARRAY_SMALL_INITIAL_TILE_LAYERS: u32 = 256;
pub const TEXARRAY_BIG_INITIAL_TILE_LAYERS: u32 = 128;
pub const TEXARRAY_SMALL_MAX_TILE_LAYERS: u32 = 2_048;
pub const TEXARRAY_BIG_MAX_TILE_LAYERS: u32 = 2_048;

// ── Tile Metadata Atlas sizing constants ───────────────────────────────────
/// Texels per atlas page (width & height).  Each texel encodes one tile's
/// metadata in Rg16Uint (4 bytes), so one page = PAGE² × 4 bytes.
pub const TILE_ATLAS_PAGE_TEXELS: u32 = 2_048;
/// Tiles stored per atlas page along each axis (matches page texels 1:1).
pub const TILE_ATLAS_TILES_PER_PAGE: u32 = 2_048;
/// Number of atlas layers allocated at startup.  Britannia (7168×4096 tiles)
/// needs 4×2 = 8 pages; starting with 4 keeps VRAM low and lets the runtime
/// grow on demand.
pub const TILE_ATLAS_INITIAL_LAYERS: u32 = 4;
/// Hard upper limit for atlas layers.  32 pages covers huge custom maps.
pub const TILE_ATLAS_MAX_LAYERS: u32 = 32;
/// World-page stride along X used for flat page-index calculation.
/// Supports maps up to 32 k × 32 k tiles.
pub const TILE_ATLAS_WORLD_PAGES_X: u32 = 16;
/// Bytes per texel in the Rg16Uint format used by the tile metadata atlas.
pub const TILE_ATLAS_BYTES_PER_TEXEL: u32 = 4;

// ── Shrink / grow hysteresis ───────────────────────────────────────────────
/// How long (seconds) usage must stay below the shrink threshold before we
/// actually downsize a texture array or the tile atlas.
pub const RESOURCE_SHRINK_TIMEOUT_SECS: u64 = 120;
/// Fraction of capacity below which we consider shrinking.
pub const RESOURCE_SHRINK_THRESHOLD: f32 = 0.40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainTextureCompression {
    Rgba8,
    Bc7(LossyTextureCompressionBackend),
}

impl TerrainTextureCompression {
    pub fn from_graphics_settings(graphics: &SectGraphics) -> Self {
        match graphics.active_lossy_texture_compression_backend() {
            Some(backend) => Self::Bc7(backend),
            None => Self::Rgba8,
        }
    }

    pub fn lossy_backend(self) -> Option<LossyTextureCompressionBackend> {
        match self {
            Self::Rgba8 => None,
            Self::Bc7(backend) => Some(backend),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rgba8 => "RGBA8",
            Self::Bc7(LossyTextureCompressionBackend::Dds) => "BC7/dds",
            Self::Bc7(LossyTextureCompressionBackend::BlockCompression) => {
                "BC7/block_compression"
            }
            Self::Bc7(LossyTextureCompressionBackend::Ispc) => "BC7/ispc",
        }
    }
}

pub fn build_texture_residency_plan(texmap_2d_res: &TexMap) -> TextureResidencyPlan<LandTextureSize> {
    let mut plan = TextureResidencyPlan::new();

    for texture_id in 0..texmap_2d_res.len() {
        let Some(element) = texmap_2d_res.element(texture_id) else {
            continue;
        };

        plan.push(*element.size(), texture_id as u16);
    }

    plan
}

pub fn texture_extent(tex_size: LandTextureSize) -> ImageExtent {
    let (width, height) = tex_size.dimensions();
    ImageExtent::new(width, height).expect("terrain textures must have valid size")
}

pub fn terrain_texture_vram_encoding(
    compression: TerrainTextureCompression,
) -> VramTextureEncoding {
    match compression {
        TerrainTextureCompression::Rgba8 => VramTextureEncoding::Rgba8UnormSrgb,
        TerrainTextureCompression::Bc7(LossyTextureCompressionBackend::Dds) => {
            VramTextureEncoding::Bc7(Bc7EncoderBackend::Dds)
        }
        TerrainTextureCompression::Bc7(LossyTextureCompressionBackend::BlockCompression) => {
            VramTextureEncoding::Bc7(Bc7EncoderBackend::BlockCompression)
        }
        TerrainTextureCompression::Bc7(LossyTextureCompressionBackend::Ispc) => {
            VramTextureEncoding::Bc7(Bc7EncoderBackend::Ispc)
        }
    }
}

pub fn terrain_texture_vram_format(compression: TerrainTextureCompression) -> VramTextureFormat {
    terrain_texture_vram_encoding(compression).format()
}

/// Returns the GPU TextureFormat to use for terrain texture arrays, based on whether
/// lossy BC7 compression has been requested by the user in the settings.
///
/// - Uncompressed (`Rgba8UnormSrgb`): ~160 MB VRAM total, highest quality.
/// - BC7 compressed (`Bc7RgbaUnormSrgb`): ~20 MB VRAM total, near-lossless quality,
///   but requires BC texture compression GPU support and CPU encoding time per tile.
pub fn terrain_texarray_format(compression: TerrainTextureCompression) -> TextureFormat {
    match terrain_texture_vram_format(compression) {
        VramTextureFormat::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
        VramTextureFormat::Bc7RgbaUnormSrgb => TextureFormat::Bc7RgbaUnormSrgb,
    }
}

/// Compute the byte size of a single layer in the texture array, for the chosen format.
pub fn bytes_per_layer(tex_size: LandTextureSize, compression: TerrainTextureCompression) -> usize {
    terrain_texture_vram_format(compression).expected_byte_len(texture_extent(tex_size))
}

/// Create a GPU texture array (array texture) resource for a given size.
pub fn create_gpu_texture_array(
    label: &'static str,
    image_assets: &mut Assets<Image>,
    tex_size: LandTextureSize,
    compression: TerrainTextureCompression,
    layers: u32,
) -> Handle<Image> {
    let (width, height) = tex_size.dimensions();
    let format = terrain_texarray_format(compression);

    // Pre-allocate zeroed data to trigger a full initial GPU upload (clearing all layers).
    // This zero-initialises every layer so the shader always sees a valid (black) texture
    // even for layers that haven't been populated yet.
    let data_bytes = bytes_per_layer(tex_size, compression) * layers as usize;

    let mut array = Image {
        data: Some(vec![0u8; data_bytes]),
        // RENDER_WORLD only: Bevy uploads the zeroed data to the GPU, then frees the CPU
        // copy. This saves ~160 MB of RAM. We don't need the CPU copy because all
        // subsequent tile updates are done via `queue.write_texture`, which writes directly
        // to the GPU without going through Assets<Image>.
        asset_usage: bevy::asset::RenderAssetUsages::RENDER_WORLD,
        texture_descriptor: bevy::render::render_resource::TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            dimension: TextureDimension::D2,
            format,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge.into(),
            address_mode_v: AddressMode::ClampToEdge.into(),
            mag_filter: FilterMode::Linear.into(),
            min_filter: FilterMode::Linear.into(),
            mipmap_filter: FilterMode::Linear.into(),
            ..default()
        }),
        ..default()
    };
    // Make sure the image view is consistent with array sizing
    array.reinterpret_size(array.texture_descriptor.size);

    image_assets.add(array)
}

////////////////////////////////////////////////////////////////////////////////
// 2. Loading an Image for a Specific Art ID and Texture Size
////////////////////////////////////////////////////////////////////////////////

//const DEFAULT_ERROR_TEXTURE_SIZE: LandTextureSize = LandTextureSize::Small;
//const DEFAULT_ERROR_TEXTURE_ID: u32 = TEXTURE_UNUSED_ID;

const DEFAULT_ERROR_TEXTURE_SIZE: LandTextureSize = LandTextureSize::Big;
pub const DEFAULT_ERROR_TEXTURE_ID: u16 = 0x4C; // Sea floor

/// Try to get actual texture for provided texture_id.
/// If invalid, return UNUSED texture.
pub fn get_texmap_size_only(
    texture_id: u16,
    texmap_2d_res: &TexMap,
) -> LandTextureSize {
    if let Some(element) = texmap_2d_res.element(texture_id as usize) {
        return *element.size();
    }

    let err_size = *texmap_2d_res
        .element(DEFAULT_ERROR_TEXTURE_ID as usize)
        .unwrap()
        .size();
    err_size
}

pub fn get_texmap_raw_data(
    texture_id: u16,
    texmap_2d_res: &TexMap,
    now: std::time::Instant,
) -> (LandTextureSize, std::sync::Arc<[u8]>) {
    fn local_log_warn(msg: &str) {
        console_logger::one(LogSev::Warn, LogAbout::RenderWorldLand, msg);
    }

    let tex_size_and_rgba = {
        texmap_2d_res.get_pixel_data(texture_id as usize, now).map(|data| {
            let size = *texmap_2d_res.element(texture_id as usize).unwrap().size();
            (size, data)
        })
    };

    if let Some((size, buffer)) = tex_size_and_rgba {
        if !buffer.is_empty() {
            return (size, buffer);
        }
        local_log_warn(&format!("Texture {texture_id:#X} has invalid pixel data."));
    } else {
        local_log_warn(&format!(
            "Requested invalid texture {texture_id:#X}. Defaulting to UNUSED."
        ));
    }

    // Fallback error texture
    let err_data = texmap_2d_res
        .get_pixel_data(DEFAULT_ERROR_TEXTURE_ID as usize, now)
        .expect("No UNUSED land texture?");
    let err_size = *texmap_2d_res
        .element(DEFAULT_ERROR_TEXTURE_ID as usize)
        .unwrap()
        .size();
    (err_size, err_data)
}

