use bevy::{
    image::{ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{AddressMode, Extent3d, FilterMode, TextureDimension, TextureFormat, TextureUsages},
};
use std::sync::OnceLock;

use crate::{core::uo_files_loader::UoFileData, prelude::*, util_lib::image::*};

pub const LAND_TEX_SIZE_SMALL: u32 = uocf::geo::land_texture_2d::TextureSize::SMALL_X;
pub const TEXARRAY_MAX_TILE_LAYERS: u32 = 2_048;

pub const TEXTURE_UNUSED_ID: u32 = 0x007F;

// ------------

/// Helper that builds the empty texture array on startup.
pub fn create_gpu_texture_array(
    label: &'static str,
    images: &mut Assets<Image>,
    //render_device: &RenderDevice,
) -> Handle<Image> {
    let mut array = Image {
        data: Some(vec![
            0u8;
            (LAND_TEX_SIZE_SMALL * LAND_TEX_SIZE_SMALL * 4 * TEXARRAY_MAX_TILE_LAYERS)
                as usize
        ]),
        texture_descriptor: bevy::render::render_resource::TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: LAND_TEX_SIZE_SMALL,
                height: LAND_TEX_SIZE_SMALL,
                depth_or_array_layers: TEXARRAY_MAX_TILE_LAYERS,
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
    array.reinterpret_size(array.texture_descriptor.size);
    images.add(array)
}

/// TEMP stub: synthesises a 44×44 checkerboard.
pub fn get_texmap_image(
    art_id: u16,
    _commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
    uo_data: &Res<UoFileData>,
) -> Handle<Image> {
    // ----------------------------------------------------------------
    // Create the Image
    // ----------------------------------------------------------------

    /*
        // Debug: Simple magenta/green checker pattern so we see something.
        for y in 0..TILE_PX {
            for x in 0..TILE_PX {
                let idx = ((y * TILE_PX + x) * 4) as usize;
                let on  = ((x ^ y ^ art_id as u32) & 1) == 0;
                rgba[idx..idx + 4].copy_from_slice(if on {
                    &[0x00, 0xFF, 0x00, 0xFF] // green
                } else {
                    &[0xFF, 0x00, 0xFF, 0xFF] // magenta
                });
            }
        }
    */

    // Ensure we load once and store an UNUSED texture, for errors.
    static UNUSED_TEXTURE: OnceLock<Handle<Image>> = OnceLock::new();
    let _ = UNUSED_TEXTURE.set({
        let texmap_lock = uo_data.texmap_2d.read().expect("Can't acquire texmap data lock.");
        let texture_ref = &texmap_lock
            .element(TEXTURE_UNUSED_ID as usize)
            .expect("Can't get UNUSED land texture?");
        let img = image_from_rgba8(texture_ref.size_x(), texture_ref.size_y(), &texture_ref.pixel_data);
        images.add(img)
    });

    // Get texture data from cached texmap.mul.
    let (tex_width, tex_height, tex_rgba) = {
        let texmap_lock = uo_data.texmap_2d.read().expect("Can't acquire texmap data lock.");
        let texture_from_texmap_ref = texmap_lock.element(art_id as usize);
        if let Some(unwrapped) = texture_from_texmap_ref {
            (
                unwrapped.size_x(),
                unwrapped.size_y(),
                Some(unwrapped.pixel_data.clone()),
            )
        } else {
            (LAND_TEX_SIZE_SMALL, LAND_TEX_SIZE_SMALL, None)
        }
    };

    // Use nodraw texture if we had errors above.
    let mut use_nodraw: bool = false;
    if tex_rgba.is_none() {
        use_nodraw = true;
    }
    // TODO: For now, we support only small "classic" textures.
    if tex_width != LAND_TEX_SIZE_SMALL || tex_height != LAND_TEX_SIZE_SMALL {
        logger::one(
            None,
            logger::LogSev::Warn,
            logger::LogAbout::RenderWorldLand,
            &format!("Requested non-small texture id={art_id}, using UNUSED."),
        );
        use_nodraw = true;
    }

    let handle = {
        if use_nodraw {
            UNUSED_TEXTURE.get().unwrap().clone()
        } else {
            // Create the bevy::Image from raw pixel data and
            let img = image_from_rgba8(tex_width, tex_height, &tex_rgba.unwrap());
            images.add(img)
        }
    };

    /*
    // (optional) pick usages / sampler if you need specific values
    image.asset_usage        = RenderAssetUsages::default();
    image.sampler_descriptor = ImageSampler::nearest();

    image.sampler_descriptor.mag_filter = FilterMode::Nearest;
    image.sampler_descriptor.min_filter = FilterMode::Nearest;
    image.sampler_descriptor.address_mode_u = AddressMode::ClampToEdge;
    image.sampler_descriptor.address_mode_v = AddressMode::ClampToEdge;
    */

    handle
}

// -------------
