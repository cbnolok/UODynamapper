// Only the optional pixel-query pipeline binds these resources. Main rendering
// retains its original layout, projection, discard rules and color output.
struct PickQuery {
    pixel_view: vec4<f32>,
    pass_origin_samples: vec4<u32>,
}
@if(STATIC_PICK)
@group(2) @binding(0) var<uniform> pick_query: PickQuery;
@if(STATIC_PICK && !STATIC_PICK_MSAA)
@group(2) @binding(1) var opaque_depth: texture_depth_2d;
@if(STATIC_PICK && STATIC_PICK_MSAA)
@group(2) @binding(1) var opaque_depth: texture_depth_multisampled_2d;

@if(STATIC_PICK)
fn pick_clip(position: vec4<f32>) -> vec4<f32> {
    let size = pick_query.pixel_view.zw;
    let pixel = pick_query.pixel_view.xy - vec2<f32>(pick_query.pass_origin_samples.yz) + vec2<f32>(0.5);
    let center = pixel / size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    // A 1x1 target preserves the original pixel's derivatives and sample pattern.
    // Crop only XY, after all original camera/priority calculations.
    return vec4<f32>((position.xy - center * position.w) * size, position.zw);
}

@if(STATIC_PICK)
fn pick_pass() -> u32 { return pick_query.pass_origin_samples.x; }

@if(STATIC_PICK)
fn pick_color(index: u32, ground: bool, depth: f32, sample_index: u32) -> vec4<f32> {
    let pixel = vec2<i32>(pick_query.pixel_view.xy);
    @if(STATIC_PICK_MSAA)
    let opaque = textureLoad(opaque_depth, pixel, i32(sample_index));
    @if(!STATIC_PICK_MSAA)
    let opaque = textureLoad(opaque_depth, pixel, 0);
    if (depth < opaque) { discard; }
    let id = (index + 1u) | select(0u, 0x80000000u, ground);
    return vec4<f32>(f32(id & 255u), f32((id >> 8u) & 255u),
        f32((id >> 16u) & 255u), f32(id >> 24u)) / 255.0;
}
