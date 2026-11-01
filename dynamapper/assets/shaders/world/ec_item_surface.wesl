// UOStaticTerrainShader binds sources 1,2,0,3: keep source1 RGB in TEMP,
// select source2 alpha, blend source0 RGB against TEMP, then source3 coverage.
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_liquid_sample, ec_liquid_texture_uv, ec_liquid_world_uv};

import package::shaders::world::texture_data::{texture_data_rgb, texture_color_rgb};

const ITEM_SURFACE_WATER: u32 = 16u;

fn ec_item_surface_record(lookup: texture_2d<u32>, index: u32) -> vec4<u32> {
    let width = textureDimensions(lookup).x;
    return textureLoad(lookup, vec2<i32>(i32(index % width), i32(index / width)), 0);
}

fn ec_item_surface_source(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                          linear_sampler: sampler, index: u32, local_uv: vec2<f32>,
                          world_xz: vec2<f32>, linear: bool) -> vec4<f32> {
    let source = ec_item_surface_record(lookup, index);
    let params = ec_item_surface_record(lookup, index + 1u);
    let repetition = bitcast<f32>(params.x);
    let coordinates = select(vec2<f32>(local_uv.x, 1.0 - local_uv.y), ec_liquid_world_uv(world_xz), source.w == 1u);
    let uv = ec_liquid_texture_uv(coordinates, repetition, vec2<f32>(0.0));
    let slot = EcLiquidSlot(true, i32(source.x), vec2<u32>(0u), source.yz, repetition);
    return ec_liquid_sample(atlas, linear_sampler, slot, uv, linear, repetition != 1.0);
}

fn apply_ec_item_surface(base: vec4<f32>, atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                         linear_sampler: sampler, header: vec4<u32>, local_uv: vec2<f32>,
                         world_xz: vec2<f32>, linear: bool, coverage_linear: bool) -> vec4<f32> {
    if (header.x == 0u || (header.y & ITEM_SURFACE_WATER) != 0u) { return base; }
    let encoded_base = vec4<f32>(texture_data_rgb(base.rgb), base.a);
    var temporary = encoded_base;
    if ((header.y & 1u) == 0u) {
        temporary = ec_item_surface_source(atlas, lookup, linear_sampler, header.x, local_uv, world_xz, linear);
    }
    let mask = ec_item_surface_source(atlas, lookup, linear_sampler, header.x + 2u, local_uv, world_xz, coverage_linear);
    var alpha = 1.0;
    if ((header.y & 2u) != 0u) {
        alpha = mask.a;
    } else if ((header.y & 4u) != 0u) {
        alpha = ec_item_surface_source(atlas, lookup, linear_sampler, header.x + 4u, local_uv, world_xz, coverage_linear).a;
    }
    return vec4<f32>(texture_color_rgb(mix(temporary.rgb, encoded_base.rgb, mask.a)), alpha);
}
