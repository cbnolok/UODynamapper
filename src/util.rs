//! Misc helpers – **edit this file only** when you hook real art assets.

use bevy::prelude::*;
use bevy::render::{       
    render_asset::RenderAssetUsages,
    render_resource::{Extent3d, TextureDimension, TextureFormat},
    //texture::{ImageSampler, ImageType},
};

/// TEMP stub: synthesises a 44×44 checkerboard.
/// Replace with your disk / MUL-reader code that returns a Bevy `Image`
/// containing RGBA8-SRGB data of exactly 44×44 pixels.
pub(crate) fn get_tile_image(
    art_id: u16,
    _commands: &mut Commands,
    images: &mut ResMut<Assets<Image>>,
) -> Handle<Image> {
    const TILE_PX: u32 = 44;
    let mut rgba = vec![0u8; (TILE_PX * TILE_PX * 4) as usize];

    // Simple magenta/green checker pattern so you see something.
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
/*
    images.add(
        Image::from_buffer(
        &rgba.into(),
        bevy::render::i ,//::ImageType::Uint,
        Extent3d {
            width:  TILE_PX,
            height: TILE_PX,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_resource::CompressedImageFormats::empty(),
    ))
*/

    // ----------------------------------------------------------------
    // Create the Image
    // ----------------------------------------------------------------       
    let size = Extent3d {
        width:  TILE_PX,          // your tile width
        height: TILE_PX,          // your tile height
        depth_or_array_layers: 1,
    };
    
    // rgba is a Vec<u8> of length width * height * 4
    let image = Image::new_fill(
        size,
        TextureDimension::D2,
        &rgba,                     // raw pixel buffer
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(), // keeps the texture in MAIN + RENDER worlds
    );
    
    /*
    // (optional) pick usages / sampler if you need specific values
    image.asset_usage        = RenderAssetUsages::default();
    image.sampler_descriptor = ImageSampler::nearest();

    image.sampler_descriptor.mag_filter = FilterMode::Nearest;
    image.sampler_descriptor.min_filter = FilterMode::Nearest;
    image.sampler_descriptor.address_mode_u = AddressMode::ClampToEdge;
    image.sampler_descriptor.address_mode_v = AddressMode::ClampToEdge;
    */

    // 4. Put it in Assets.
    let handle = images.add(image);
    
    /*
    // 5. Spawn a sprite (example).
    commands.spawn(SpriteBundle {
        texture: handle.clone(),
        ..Default::default()
    });
    */
    
    handle
}