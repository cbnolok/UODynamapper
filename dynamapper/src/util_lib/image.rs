use bevy::{
    asset::RenderAssetUsages,
    image::Image,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub fn image_from_rgba8(width: u32, height: u32, rgba_buffer_ref: &[u8]) -> Image {
    let mut img = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba_buffer_ref, // raw pixel buffer
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(), // keeps the texture in MAIN + RENDER worlds
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}

pub fn image_from_uddp_page(
    width: u32,
    height: u32,
    data: Vec<u8>,
    format: udd_assets::tex_art_cc::PagePixelFormat,
) -> Image {
    let texture_format = match format {
        udd_assets::tex_art_cc::PagePixelFormat::Rgba8888 => TextureFormat::Rgba8UnormSrgb,
        udd_assets::tex_art_cc::PagePixelFormat::Bc7 => TextureFormat::Bc7RgbaUnormSrgb,
    };

    let mut img = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        texture_format,
        RenderAssetUsages::default(),
    );
    img.sampler = bevy::image::ImageSampler::linear();
    img
}
