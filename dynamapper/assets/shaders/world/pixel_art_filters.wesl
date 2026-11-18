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

// Explicit gradients keep the seam footprint valid inside material-dependent
// branches. Compute the screen derivatives before entering those branches.
fn pixel_art_uv_iq_grad(uv: vec2<f32>, texture_size: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    let size = max(texture_size, vec2<f32>(1.0));
    let pixel = uv * size;
    let seam = floor(pixel + vec2<f32>(0.5));
    let width = max(abs(dx * size) + abs(dy * size), vec2<f32>(0.000001));
    let snapped = seam + clamp((pixel - seam) / width, vec2<f32>(-0.5), vec2<f32>(0.5));
    return snapped / size;
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
