// UOWaterShader's optional fourth stage uses the item's own material parameters.
// It is composed over the lit liquid, before the frame's color processing.
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_aux_sample, ec_liquid_texture_uv, ec_liquid_world_uv};

fn apply_ec_item_liquid(current: vec4<f32>, atlas: texture_2d_array<f32>,
                        lookup: texture_2d<u32>, liquid_sampler: sampler, item: u32,
                        local_uv: vec2<f32>, world_xz: vec2<f32>, time: f32,
                        animate: bool, linear: bool) -> vec4<f32> {
    let dims = textureDimensions(lookup);
    let index = item * 2u;
    let source = textureLoad(lookup, vec2<i32>(i32(index % dims.x), i32(index / dims.x)), 0);
    if (source.x == 0u) { return current; }
    let params = textureLoad(lookup, vec2<i32>(i32((index + 1u) % dims.x), i32((index + 1u) / dims.x)), 0);
    let repetition = bitcast<f32>(params.x);
    let velocity = bitcast<vec2<f32>>(params.yz);
    // The original mesh stores local V as 1-v before the mode-2 transform
    // reverses it again. World UVs have their own signed map-coordinate domain.
    let coordinates = select(vec2<f32>(local_uv.x, 1.0 - local_uv.y), ec_liquid_world_uv(world_xz), params.w == 1u);
    let transformed = ec_liquid_texture_uv(coordinates, repetition, velocity * select(0.0, time, animate));
    // Unit-repetition item masks clamp; tiled sources wrap. Reflection source 2
    // forces WRAP separately in the original draw path.
    let slot = EcLiquidSlot(true, i32(source.x), vec2<u32>(0u), source.yz, repetition);
    let overlay = ec_aux_sample(atlas, liquid_sampler, slot, transformed, linear, repetition != 1.0);
    if (source.w == 2u) {
        // D3DTOP_BLENDTEXTUREALPHA retains current alpha.
        return vec4<f32>(mix(current.rgb, overlay.rgb, overlay.a), current.a);
    }
    var rgb = current.rgb + overlay.rgb;
    if (source.w == 1u) { rgb = current.rgb * overlay.rgb; }
    // Both MODULATE and the default ADD branch select texture alpha.
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), overlay.a);
}
