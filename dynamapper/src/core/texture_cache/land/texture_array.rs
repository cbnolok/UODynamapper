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

// Texture array for 'small' textures:
pub const TEXARRAY_SMALL_MAX_TILE_LAYERS: u32 = 2_048;
pub const TEXARRAY_BIG_MAX_TILE_LAYERS: u32 = 2_048;

fn max_layers_per_texture_size(tex_size: LandTextureSize) -> u32 {
    match tex_size {
        LandTextureSize::Small => TEXARRAY_SMALL_MAX_TILE_LAYERS,
        LandTextureSize::Big => TEXARRAY_BIG_MAX_TILE_LAYERS,
    }
}

/// Create a GPU texture array (array texture) resource for a given size.
pub fn create_gpu_texture_array(
    label: &'static str,
    image_assets: &mut Assets<Image>,
    tex_size: LandTextureSize,
) -> Handle<Image> {
    let (width, height) = tex_size.dimensions();
    let layers = max_layers_per_texture_size(tex_size);

    // Pre-allocate array data as RGBA8 (4 bytes/pixel)
    let data_bytes = (width * height * layers * 4) as usize;

    let mut array = Image {
        data: Some(vec![0u8; data_bytes]),
        texture_descriptor: bevy::render::render_resource::TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
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
pub fn get_texmap_raw_data<'a>(
    texture_id: u16,
    texmap_2d_res: &'a TexMap2D,
) -> (LandTextureSize, &'a [u8]) {
    fn local_log_warn(msg: &str) {
        logger::one(None, LogSev::Warn, LogAbout::RenderWorldLand, msg);
    }

    let tex_size_and_rgba = {
        match texmap_2d_res.element(texture_id as usize) {
            Some(tex_ref) => Some((tex_ref.size().clone(), tex_ref.pixel_data())),
            None => None,
        }
    };

    if let Some((size, buffer)) = tex_size_and_rgba {
        if !buffer.is_empty() {
            return (size, buffer.as_slice());
        }
        local_log_warn(&format!("Texture {texture_id:#X} has invalid pixel data."));
    } else {
        local_log_warn(&format!(
            "Requested invalid texture {texture_id:#X}. Defaulting to UNUSED."
        ));
    }

    // Fallback error texture
    let err_tex_ref = texmap_2d_res
        .element(DEFAULT_ERROR_TEXTURE_ID as usize)
        .expect("No UNUSED land texture?");
    (err_tex_ref.size().clone(), err_tex_ref.pixel_data().as_slice())
}

/*
// (optional) pick usages / sampler if you need specific values
image.asset_usage        = RenderAssetUsages::default();
image.sampler_descriptor = ImageSampler::nearest();

image.sampler_descriptor.mag_filter = FilterMode::Nearest;
image.sampler_descriptor.min_filter = FilterMode::Nearest;
image.sampler_descriptor.address_mode_u = AddressMode::ClampToEdge;
image.sampler_descriptor.address_mode_v = AddressMode::ClampToEdge;
*/
