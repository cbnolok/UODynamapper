// Original UOWaterShader separates the static fallback (source 1) from the
// bump-environment surface (source 2). Both terrain and item water use this
// sampler so source selection, displacement, and color are owner-independent.

import package::shaders::world::uo_coords::UO_Z_TO_WORLD_SCALE;

const LIQUID_LOOKUP_CAPACITY: u32 = 16384u;
// UOWaterTerrainLayer::Draw (0x005988a0) sets item 0's texture translation
// (0x005101c0, NiTextureTransform translate + dirty flag) to t * tweaks[0..2].
// t is UOApplication +0x58, advanced each frame by the frame time in seconds
// (0x00482530: QPC milliseconds / 1000). Bevy's globals.time is also seconds.
const EC_WATER_CLIENT_TIME_PER_SECOND: f32 = 1.0;

struct EcLiquidSlot {
    present: bool,
    layer: i32,
    origin: vec2<u32>,
    extent: vec2<u32>,
    stretch: f32,
}

fn ec_liquid_slot(lookup: texture_2d<u32>, payload: u32, role: u32) -> EcLiquidSlot {
    let dims = textureDimensions(lookup);
    let index = role * LIQUID_LOOKUP_CAPACITY + payload;
    let value = textureLoad(lookup, vec2<i32>(i32(index % dims.x), i32(index / dims.x)), 0);
    let extent = vec2<u32>(value.w & 0xffffu, value.w >> 16u);
    return EcLiquidSlot(all(extent > vec2<u32>(0u)), i32(value.x & 0xffffu), value.yz,
        extent, max(f32(value.x >> 16u) / 256.0, 0.01));
}

fn ec_liquid_load(atlas: texture_2d_array<f32>, slot: EcLiquidSlot,
                  texel: vec2<i32>, repeat: bool) -> vec4<f32> {
    let extent = vec2<i32>(slot.extent);
    var local = clamp(texel, vec2<i32>(0), extent - vec2<i32>(1));
    if (repeat) {
        local = ((texel % extent) + extent) % extent;
    }
    return textureLoad(atlas, vec2<i32>(slot.origin) + local, slot.layer, 0);
}

fn ec_liquid_sample(atlas: texture_2d_array<f32>, linear_sampler: sampler, slot: EcLiquidSlot,
                    uv: vec2<f32>, linear: bool, repeat: bool) -> vec4<f32> {
    let addressed_uv = select(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), fract(uv), repeat);
    let local = addressed_uv * vec2<f32>(slot.extent);
    if (!linear) {
        return ec_liquid_load(atlas, slot, vec2<i32>(floor(local)), repeat);
    }
    // EC page atlases use UNORM, matching the original disabled SRGBTEXTURE.
    // Hardware filters both encoded color and bump data in their authored domain.
    // Wrapped border footprints still need source-bounded explicit taps.
    let interior = all(local >= vec2<f32>(0.5)) && all(local <= vec2<f32>(slot.extent) - vec2<f32>(0.5));
    if (!repeat || interior) {
        let px = vec2<f32>(slot.origin) + clamp(local, vec2<f32>(0.5), vec2<f32>(slot.extent) - vec2<f32>(0.5));
        return textureSampleLevel(atlas, linear_sampler, px / vec2<f32>(textureDimensions(atlas)), slot.layer, 0.0);
    }
    let footprint = local - vec2<f32>(0.5);
    let first = vec2<i32>(floor(footprint));
    let weight = fract(footprint);
    let upper_left = ec_liquid_load(atlas, slot, first, repeat);
    let upper_right = ec_liquid_load(atlas, slot, first + vec2<i32>(1, 0), repeat);
    let lower_left = ec_liquid_load(atlas, slot, first + vec2<i32>(0, 1), repeat);
    let lower_right = ec_liquid_load(atlas, slot, first + vec2<i32>(1, 1), repeat);
    return mix(mix(upper_left, upper_right, weight.x), mix(lower_left, lower_right, weight.x), weight.y);
}

fn ec_liquid_reflection_uv(view_position: vec3<f32>) -> vec2<f32> {
    // Both original liquid geometry owners omit the normal stream. Fixed-function
    // reflection with that zero normal reduces to the normalized view position.
    // Generate this at vertices and interpolate XY, before the source affine.
    return view_position.xy * inverseSqrt(max(dot(view_position, view_position), 1e-20));
}

fn ec_liquid_original_view(delta_world: vec3<f32>) -> vec3<f32> {
    // UOSA 0x471580: Rz(pi/2) * Rx(pi/4-.001) * Rz(.05), with
    // columns direction/up/right. Original world Z is negative map Y.
    // Height moves along camera up (0x4598d0/0x460650), not world Y.
    let right = vec2<f32>(0.706399321, -0.707813534);
    let up = vec2<f32>(-0.706928952, -0.705516506);
    let direction = vec2<f32>(-0.0353759324, -0.0353052512);
    return vec3<f32>(dot(right, delta_world.xz),
        dot(up, delta_world.xz) + (delta_world.y / UO_Z_TO_WORLD_SCALE) * 0.12856454,
        10.0 + dot(direction, delta_world.xz));
}

fn ec_liquid_world_uv(world_xz: vec2<f32>) -> vec2<f32> {
    // Original terrain stream 0 and item world-plane UVs follow (map X, -map Y).
    // Bevy world Z follows positive map Y; convert before the authored transform.
    return vec2<f32>(world_xz.x, -world_xz.y);
}

fn ec_liquid_texture_uv(uv: vec2<f32>, stretch: f32, scroll: vec2<f32>) -> vec2<f32> {
    // The source loader installs NiTextureTransform mode 2, scale 1/repetition.
    // With zero rotation its matrix maps (u,v) to (u/stretch+tx,1-v/stretch-ty).
    let transformed = uv / stretch + scroll;
    return vec2<f32>(transformed.x, 1.0 - transformed.y);
}

fn sample_ec_liquid(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>, linear_sampler: sampler,
                    payload: u32, world_xz: vec2<f32>, uv_zero: vec2<f32>, reflection_uv: vec2<f32>,
                    time: f32, animate: bool, linear: bool) -> vec4<f32> {
    let fallback = ec_liquid_slot(lookup, payload, 0u);
    let normal = ec_liquid_slot(lookup, payload, 3u);
    let animated = ec_liquid_slot(lookup, payload, 4u);
    let lookup_dims = textureDimensions(lookup);
    let params_index = 5u * LIQUID_LOOKUP_CAPACITY + payload;
    let params = textureLoad(lookup, vec2<i32>(i32(params_index % lookup_dims.x), i32(params_index / lookup_dims.x)), 0);
    var surface = fallback;
    var uv = world_xz / fallback.stretch;
    if (animated.present) {
        surface = animated;
        var coordinates = reflection_uv;
        // Technique 1 retains the source's world UV set. Technique 2 forces
        // set 0: world UVs on terrain, local art UVs on items. The caller
        // supplies that stream in the original coordinate convention.
        // Original water's Technique 0 generates a camera reflection vector.
        if (params.x == 1u) {
            coordinates = ec_liquid_world_uv(world_xz);
        } else if (params.x == 2u) {
            coordinates = uv_zero;
        }
        uv = ec_liquid_texture_uv(coordinates, animated.stretch, vec2<f32>(0.0));
    }
    if (!surface.present) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    if (normal.present) {
        var velocity = vec2<f32>(0.0, -1.0 / normal.stretch);
        if (params.w != 0u) {
            velocity = bitcast<vec2<f32>>(params.yz);
        }
        let scroll = velocity * select(0.0, time * EC_WATER_CLIENT_TIME_PER_SECOND, animate);
        var bump_uv = world_xz / normal.stretch + scroll;
        if (params.w != 0u) {
            bump_uv = ec_liquid_texture_uv(ec_liquid_world_uv(world_xz), normal.stretch, scroll);
        }
        var bump = ec_liquid_sample(atlas, linear_sampler, normal, bump_uv, linear, true).rg;
        if (params.w == 0u) {
            bump = bump * 2.0 - vec2<f32>(1.0);
        }
        // The original source is an unsigned DXT1 image. BUMPENVMAP consumes
        // its sampled RG directly; expanding it as a signed normal doubles the
        // displacement range instead of preserving the authored offsets.
        // The original bump-environment matrix is identity. Keep the existing
        // smaller perturbation only when no original animated surface is packed.
        uv += bump * select(0.30, 1.0, animated.present);
    }
    return ec_liquid_sample(atlas, linear_sampler, surface, uv, linear, true);
}
