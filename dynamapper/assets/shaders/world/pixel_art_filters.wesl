// Pixel-art seam filters for fractional-scale rendering.
//
// These helpers keep texel interiors nearest-like while letting the hardware
// bilinear sampler anti-alias only the seam region.

fn pixel_art_uv_iq(uv: vec2<f32>, texture_size: vec2<f32>) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixel = uv * size;
    let seam = floor(pixel + vec2<f32>(0.5));
    let width = max(fwidth(pixel), vec2<f32>(0.000001));
    let snapped = seam + clamp((pixel - seam) / width, vec2<f32>(-0.5), vec2<f32>(0.5));
    return snapped / size;
}

fn pixel_art_uv_aa_linear(uv: vec2<f32>, texture_size: vec2<f32>, width_scale: f32) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixel = uv * size;
    let seam = floor(pixel + vec2<f32>(0.5));
    let width = max(fwidth(pixel) * max(width_scale, 0.000001), vec2<f32>(0.000001));
    let snapped = seam + clamp((pixel - seam) / width, vec2<f32>(-0.5), vec2<f32>(0.5));
    return snapped / size;
}

fn pixel_art_uv_aa_smoothstep(uv: vec2<f32>, texture_size: vec2<f32>, width_scale: f32) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixel = uv * size;
    let seam = floor(pixel + vec2<f32>(0.5));
    let width = max(fwidth(pixel) * max(width_scale, 0.000001), vec2<f32>(0.000001));
    let shaped = smoothstep(vec2<f32>(-0.5), vec2<f32>(0.5), (pixel - seam) / width) - vec2<f32>(0.5);
    return (seam + shaped) / size;
}

fn pixel_art_uv_klems(uv: vec2<f32>, texture_size: vec2<f32>) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixels = uv * size + vec2<f32>(0.5);
    let fl = floor(pixels);
    var fr = fract(pixels);
    let aa = max(fwidth(pixels) * 0.75, vec2<f32>(0.000001));
    fr = smoothstep(vec2<f32>(0.5) - aa, vec2<f32>(0.5) + aa, fr);
    return (fl + fr - vec2<f32>(0.5)) / size;
}

fn pixel_art_uv_fat_pixel(uv: vec2<f32>, texture_size: vec2<f32>, texels_per_pixel: vec2<f32>) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixel = uv * size;
    var fat_pixel = floor(pixel) + vec2<f32>(0.5);
    fat_pixel += vec2<f32>(1.0) - clamp((vec2<f32>(1.0) - fract(pixel)) * max(texels_per_pixel, vec2<f32>(0.000001)), vec2<f32>(0.0), vec2<f32>(1.0));
    return fat_pixel / size;
}

fn pixel_art_atlas_uv_iq(
    uv: vec2<f32>,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    atlas_size: vec2<f32>,
) -> vec2<f32> {
    let extent_uv = max(uv_max - uv_min, vec2<f32>(0.000001));
    let texture_size = max(extent_uv * atlas_size, vec2<f32>(1.0));
    let local_uv = clamp((uv - uv_min) / extent_uv, vec2<f32>(0.0), vec2<f32>(1.0));
    return uv_min + pixel_art_uv_iq(local_uv, texture_size) * extent_uv;
}
