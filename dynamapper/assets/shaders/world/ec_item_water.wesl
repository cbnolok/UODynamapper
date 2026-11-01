// Source-authored UOWaterShader stages on an item-owned quad. No terrain graph
// is needed: map 1 owns the art window, map 0 bumps map 2, map 3 composes last.
import package::shaders::world::ec_item_surface::{ec_item_surface_record, ITEM_SURFACE_WATER};
import package::shaders::world::ec_item_liquid::compose_ec_item_liquid;
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_liquid_sample, ec_liquid_texture_uv, ec_liquid_world_uv};
import package::shaders::world::texture_data::texture_color_rgb;

fn ec_item_is_water(header: vec4<u32>) -> bool {
    return header.x != 0u && (header.y & ITEM_SURFACE_WATER) != 0u;
}

fn ec_item_water_coordinates(local_uv: vec2<f32>, world_xz: vec2<f32>, uv_set: u32) -> vec2<f32> {
    return select(vec2<f32>(local_uv.x, 1.0 - local_uv.y), ec_liquid_world_uv(world_xz), uv_set == 1u);
}

fn ec_item_water_sample(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                        linear_sampler: sampler, index: u32, coordinates: vec2<f32>,
                        scroll: vec2<f32>, linear: bool, force_wrap: bool) -> vec4<f32> {
    let source = ec_item_surface_record(lookup, index);
    let params = ec_item_surface_record(lookup, index + 1u);
    let repetition = bitcast<f32>(params.x);
    let slot = EcLiquidSlot(true, i32(source.x), vec2<u32>(0u), source.yz, repetition);
    let uv = ec_liquid_texture_uv(coordinates, repetition, scroll);
    return ec_liquid_sample(atlas, linear_sampler, slot, uv, linear, force_wrap || repetition != 1.0);
}

fn sample_ec_item_water(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                        linear_sampler: sampler, header: vec4<u32>, local_uv: vec2<f32>,
                        world_xz: vec2<f32>, reflection_uv: vec2<f32>, time: f32,
                        animate: bool, linear: bool, diffuse_energy: f32) -> vec4<f32> {
    let index = header.x;
    let window = bitcast<vec4<f32>>(ec_item_surface_record(lookup, index + 6u));
    let authored_uv = window.xy + local_uv * window.zw;
    let clock = select(0.0, time, animate);
    let bump_source = ec_item_surface_record(lookup, index);
    let bump_params = ec_item_surface_record(lookup, index + 1u);
    let bump = ec_item_water_sample(atlas, lookup, linear_sampler, index,
        ec_item_water_coordinates(authored_uv, world_xz, bump_source.w),
        bitcast<vec2<f32>>(bump_params.yz) * clock, linear, false);
    let surface_source = ec_item_surface_record(lookup, index + 2u);
    let surface_params = ec_item_surface_record(lookup, index + 3u);
    var coordinates = reflection_uv;
    if (surface_params.w == 1u) {
        coordinates = ec_item_water_coordinates(authored_uv, world_xz, surface_source.w);
    } else if (surface_params.w == 2u) {
        coordinates = vec2<f32>(authored_uv.x, 1.0 - authored_uv.y);
    }
    let repetition = bitcast<f32>(surface_params.x);
    let uv = ec_liquid_texture_uv(coordinates, repetition, vec2<f32>(0.0)) + bump.rg;
    let slot = EcLiquidSlot(true, i32(surface_source.x), vec2<u32>(0u), surface_source.yz, repetition);
    let surface = ec_liquid_sample(atlas, linear_sampler, slot, uv, linear, true);
    // 0x59c1e3..0x59c283 selects bump TEXTURE alpha. The reflection stage at
    // 0x59c418..0x59c437 preserves CURRENT alpha, not reflection-map alpha.
    var color = vec4<f32>(clamp(surface.rgb * diffuse_energy, vec3<f32>(0.0), vec3<f32>(1.0)), bump.a);
    let fourth_source = ec_item_surface_record(lookup, index + 4u);
    if (fourth_source.x != 0u) {
        let fourth_params = ec_item_surface_record(lookup, index + 5u);
        let overlay = ec_item_water_sample(atlas, lookup, linear_sampler, index + 4u,
            ec_item_water_coordinates(authored_uv, world_xz, fourth_source.w),
            bitcast<vec2<f32>>(fourth_params.yz) * clock, linear, false);
        color = compose_ec_item_liquid(color, overlay, fourth_params.w);
    }
    return vec4<f32>(texture_color_rgb(color.rgb), color.a);
}
