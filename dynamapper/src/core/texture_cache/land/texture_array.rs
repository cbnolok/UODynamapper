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
pub const TEXARRAY_SMALL_MAX_TILE_LAYERS: u32 = 2_048;
pub const TEXARRAY_BIG_MAX_TILE_LAYERS: u32 = 2_048;

fn max_layers_per_texture_size(tex_size: LandTextureSize) -> u32 {
    match tex_size {
        LandTextureSize::Small => TEXARRAY_SMALL_MAX_TILE_LAYERS,
        LandTextureSize::Big => TEXARRAY_BIG_MAX_TILE_LAYERS,
    }
}

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
) -> Handle<Image> {
    let (width, height) = tex_size.dimensions();
    let layers = max_layers_per_texture_size(tex_size);
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
pub fn get_texmap_raw_data(
    texture_id: u16,
    texmap_2d_res: &TexMap2D,
) -> (LandTextureSize, std::sync::Arc<Vec<u8>>) {
    fn local_log_warn(msg: &str) {
        console_logger::one(None, LogSev::Warn, LogAbout::RenderWorldLand, msg);
    }

    let tex_size_and_rgba = {
        texmap_2d_res.get_pixel_data(texture_id as usize).map(|data| {
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
        .get_pixel_data(DEFAULT_ERROR_TEXTURE_ID as usize)
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

/// Returns raw RGBA8 data. BC7 compression is now handled by the renderer via GPU compute.
pub fn compress_rgba8_to_bc7(rgba8_data: &[u8], _tex_size: LandTextureSize) -> Vec<u8> {
    rgba8_data.to_vec()
}
