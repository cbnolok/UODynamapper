use bevy::{
    asset::RenderAssetUsages,
    image::Image,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
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

pub fn visual_grunge_image(size: u32) -> Image {
    let size = size.max(16);
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let coarse = periodic_noise(u, v, 10);
            let fine = periodic_noise(u + 0.31, v - 0.17, 29);
            let streak = ((u * std::f32::consts::TAU * 3.0 + fine * 2.4).sin()
                * (v * std::f32::consts::TAU * 5.0 + coarse * 1.7).cos()
                * 0.5)
                + 0.5;
            let deposit = (coarse * 0.62 + fine * 0.28 + streak * 0.10).clamp(0.0, 1.0);
            let luma = (0.58 + deposit * 0.44).clamp(0.0, 1.0);
            data.push((luma * 235.0) as u8);
            data.push((luma * 241.0) as u8);
            data.push((luma * 255.0) as u8);
            data.push(255);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

fn periodic_noise(u: f32, v: f32, cells: u32) -> f32 {
    let x = u * cells as f32;
    let y = v * cells as f32;
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let tx = smooth_interp(x - xi as f32);
    let ty = smooth_interp(y - yi as f32);

    let a = periodic_hash(xi, yi, cells);
    let b = periodic_hash(xi + 1, yi, cells);
    let c = periodic_hash(xi, yi + 1, cells);
    let d = periodic_hash(xi + 1, yi + 1, cells);
    let ab = a + (b - a) * tx;
    let cd = c + (d - c) * tx;
    ab + (cd - ab) * ty
}

fn smooth_interp(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn periodic_hash(x: i32, y: i32, period: u32) -> f32 {
    let period = period as i32;
    let x = x.rem_euclid(period) as u32;
    let y = y.rem_euclid(period) as u32;
    let mut n = x.wrapping_mul(0x85eb_ca6b) ^ y.wrapping_mul(0xc2b2_ae35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    (n & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}
