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

/// Create and preserve a placeholder texture for fallback/error.
fn get_error_texture(
    _texture_size: LandTextureSize,
    image_assets: &mut ResMut<Assets<Image>>,
    texmap_2d: &TexMap2D,
) -> Handle<Image> {
    static UNUSED_SMALL: OnceLock<Handle<Image>> = OnceLock::new();
    //static UNUSED_BIG: OnceLock<Handle<Image>> = OnceLock::new();

    // Use one placeholder for each canonical size.
    //if texture_size == LandTextureSize::Small {
    UNUSED_SMALL
        .get_or_init(|| {
            let texture_ref = texmap_2d
                .element(DEFAULT_ERROR_TEXTURE_ID as usize)
                .expect("No UNUSED land texture?");
            let img = image_from_rgba8(
                texture_ref.size_x(),
                texture_ref.size_y(),
                &texture_ref.pixel_data(),
            );
            image_assets.add(img)
        })
        .clone()
    /*
        } else {
            UNUSED_BIG
                .get_or_init(|| {
                    let texmap_lock = uo_data
                        .texmap_2d
                        .read()
                        .expect("Can't acquire texmap data lock.");
                    let texture_ref = texmap_lock
                        .element(DEFAULT_ERROR_TEXTURE_ID as usize)
                        .expect("No UNUSED land texture?");
                    let mut img = image_from_rgba8(
                        texture_ref.size_x(),
                        texture_ref.size_y(),
                        &texture_ref.pixel_data(),
                    );
                    // UNUSED texture is small. Let's scale it up and make it grayscale, to make clear visually that we
                    //  requested an invalid big texture, not a small one.
                    let asset_usage = img.asset_usage;
                    let dynamic_img = img
                        .try_into_dynamic()
                        .unwrap()
                        .resize(
                            LandTextureSize::BIG_X,
                            LandTextureSize::BIG_Y,
                            image::imageops::FilterType::Nearest,
                        )
                        .grayscale();
                    img = Image::from_dynamic(dynamic_img, false, asset_usage);
                    image_assets.add(img)
                })
                .clone()
        }
    */
}

/// Try to get actual texture for provided texture_id.
/// If invalid, return UNUSED texture.
pub fn get_texmap_image(
    texture_id: u16,
    image_assets_resmut: &mut ResMut<Assets<Image>>,
    texmap_2d_res: &TexMap2D,
) -> (LandTextureSize, Handle<Image>) {
    fn local_log_warn(msg: &str) {
        logger::one(None, LogSev::Warn, LogAbout::RenderWorldLand, msg);
    }

    let tex_size_and_rgba = {
        match texmap_2d_res.element(texture_id as usize) {
            Some(tex_ref) => Some((tex_ref.size().clone(), tex_ref.pixel_data().clone())),
            None => None,
        }
    };

    // Validate size and pixel data. If missing or wrong size, fallback to unused placeholder.
    let (texture_size, texture_rgba_buffer) = match tex_size_and_rgba {
        Some((size, buffer)) if !buffer.is_empty() => (size, buffer),
        _ => {
            if tex_size_and_rgba.is_none() {
                local_log_warn(&format!(
                    "Requested invalid texture {texture_id:#X}. Defaulting to UNUSED."
                ));
            } else {
                local_log_warn(&format!("Texture {texture_id:#X} has invalid pixel data."));
            }
            let err_tex: Handle<Image> = get_error_texture(
                DEFAULT_ERROR_TEXTURE_SIZE,
                image_assets_resmut,
                texmap_2d_res,
            );
            return (DEFAULT_ERROR_TEXTURE_SIZE, err_tex);
        }
    };

    let (tw, th) = texture_size.dimensions();
    let img: Image = image_from_rgba8(tw, th, &texture_rgba_buffer);
    let img_handle: Handle<Image> = image_assets_resmut.add(img);
    (texture_size, img_handle)
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
