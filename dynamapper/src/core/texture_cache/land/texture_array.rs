#![allow(unused)]

use crate::{core::uo_files_loader::TexMap2DRes, prelude::*, util_lib::image::*};
use bevy::{
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{
        AddressMode, Extent3d, FilterMode, TextureDimension, TextureFormat, TextureUsages,
    },
};
use std::sync::OnceLock;
use uocf::geo::land_texture_2d::{LandTextureSize, TexMap2D};

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
// offline or on-the-fly. We previously used `intel_tex_2` (CPU-side), but now use
// `block_compression` (GPU compute) for better performance and smaller binary size.
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

/// Returns the GPU TextureFormat to use for terrain texture arrays, based on whether
/// lossy BC7 compression has been requested by the user in the settings.
///
/// - Uncompressed (`Rgba8UnormSrgb`): ~160 MB VRAM total, highest quality.
/// - BC7 compressed (`Bc7RgbaUnormSrgb`): ~20 MB VRAM total, near-lossless quality,
///   but requires BC texture compression GPU support and CPU encoding time per tile.
pub fn terrain_texarray_format(lossy_compression: bool) -> TextureFormat {
    if lossy_compression {
        // BC7 is a 4-bpp block format (4×4 pixels = 16 bytes per block of 16 pixels).
        // It supports RGBA and near-lossless quality with 8:1 compression over RGBA8.
        TextureFormat::Bc7RgbaUnormSrgb
    } else {
        // Standard uncompressed 32bpp RGBA, sRGB color space.
        TextureFormat::Rgba8UnormSrgb
    }
}

/// Compute the byte size of a single layer in the texture array, for the chosen format.
pub fn bytes_per_layer(tex_size: LandTextureSize, lossy_compression: bool) -> usize {
    let (w, h) = tex_size.dimensions();
    let (w, h) = (w as usize, h as usize);
    if lossy_compression {
        // BC7: each 4×4 block = 16 bytes. Number of blocks = ceil(w/4) * ceil(h/4).
        let block_w = w.div_ceil(4);
        let block_h = h.div_ceil(4);
        block_w * block_h * 16
    } else {
        // Uncompressed RGBA8: 4 bytes per pixel.
        w * h * 4
    }
}

/// Create a GPU texture array (array texture) resource for a given size.
pub fn create_gpu_texture_array(
    label: &'static str,
    image_assets: &mut Assets<Image>,
    tex_size: LandTextureSize,
    lossy_compression: bool,
    layers: u32,
) -> Handle<Image> {
    let (width, height) = tex_size.dimensions();
    let format = terrain_texarray_format(lossy_compression);

    // Pre-allocate zeroed data to trigger a full initial GPU upload (clearing all layers).
    // This zero-initialises every layer so the shader always sees a valid (black) texture
    // even for layers that haven't been populated yet.
    let data_bytes = bytes_per_layer(tex_size, lossy_compression) * layers as usize;

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
            mag_filter: FilterMode::Nearest.into(),
            min_filter: FilterMode::Nearest.into(),
            mipmap_filter: FilterMode::Nearest.into(),
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
const DEFAULT_ERROR_TEXTURE_ID: u32 = 0x4C; // Sea floor

/// Try to get actual texture for provided texture_id.
/// If invalid, return UNUSED texture.
pub fn get_texmap_size_only(
    texture_id: u16,
    texmap_2d_res: &TexMap2D,
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
    texmap_2d_res: &TexMap2D,
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

////////////////////////////////////////////////////////////////////////////////
// 3. Optional BC7 Compression
////////////////////////////////////////////////////////////////////////////////

/// Returns BC7 compressed bytes. BC7 compression is perfectly handled by intel_tex_2 on the CPU.
pub fn compress_rgba8_to_bc7(rgba8_data: &[u8], tex_size: LandTextureSize) -> Vec<u8> {
    let (width, height) = tex_size.dimensions();
    let surface = intel_tex_2::RgbaSurface {
        data: rgba8_data,
        width,
        height,
        stride: width * 4,
    };
    let settings = intel_tex_2::bc7::alpha_basic_settings();
    intel_tex_2::bc7::compress_blocks(&settings, &surface)
}
