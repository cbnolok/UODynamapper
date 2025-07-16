use bevy::{
    asset::RenderAssetUsages,
    image::Image,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub fn image_from_rgba8(width: u32, height: u32, rgba_buffer_ref: &Vec<u8>) -> Image {
    let mut img = Image::new_fill(
        Extent3d{
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &rgba_buffer_ref, // raw pixel buffer
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(), // keeps the texture in MAIN + RENDER worlds
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}
