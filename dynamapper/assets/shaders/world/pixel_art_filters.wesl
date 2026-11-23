// Pixel-art seam filters for fractional-scale rendering.
//
// These helpers keep texel interiors nearest-like while letting the hardware
// bilinear sampler anti-alias only the seam region. They suit a linear sampler only (land); a nearest atlas takes
// `pixel_art_atlas_nearest_uv` instead.

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

// The sub-texel steps a nearest tap rounds the texel position to before it takes the texel: the 8 bits of sub-texel
// precision Direct3D requires of texture sampling.
const NEAREST_SUBTEXEL_STEPS: f32 = 256.0;

// The centre of the atlas texel `uv` lies in, kept inside the sprite's rectangle `uv_min`..`uv_max`, for the atlases
// that sample nearest: the art pages and the mobile frames (Bevy's default sampler, `ImagePlugin::default_nearest` in
// app.rs). A seam filter would only move the point inside that texel, and float rounding put it in the neighbouring
// one along texel edges. The texel position is rounded to 1/NEAREST_SUBTEXEL_STEPS of a texel first, as the sampler
// does, so a screen pixel centred on a texel edge (at fractional zooms) takes the same texel, the right or lower one,
// in the draw and in the 1x1 pick pass (art/picking.wesl) whatever the rounding of its interpolated uv; the tap at the
// texel's centre leaves the sampler nothing to round.
fn pixel_art_atlas_nearest_uv(
    uv: vec2<f32>,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    atlas_size: vec2<f32>,
) -> vec2<f32> {
    let first = round(uv_min * atlas_size);
    let last = max(round(uv_max * atlas_size) - vec2<f32>(1.0), first);
    let position = round(uv * atlas_size * NEAREST_SUBTEXEL_STEPS) / NEAREST_SUBTEXEL_STEPS;
    return (clamp(floor(position), first, last) + vec2<f32>(0.5)) / atlas_size;
}
