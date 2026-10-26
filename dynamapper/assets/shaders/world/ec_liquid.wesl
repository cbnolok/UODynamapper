// Original UOWaterShader separates the static fallback (source 1) from the
// bump-environment surface (source 2). Both terrain and item water use this
// sampler so source selection, displacement, and color are owner-independent.
import package::shaders::world::texture_data::texture_data_rgb;

const LIQUID_LOOKUP_CAPACITY: u32 = 16384u;

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

fn ec_liquid_texel(atlas: texture_2d_array<f32>, atlas_sampler: sampler,
                   slot: EcLiquidSlot, uv: vec2<f32>, linear: bool) -> vec4<f32> {
    return ec_liquid_texel_at_uv(atlas, atlas_sampler, slot, fract(uv), linear);
}

fn ec_liquid_texel_at_uv(atlas: texture_2d_array<f32>, atlas_sampler: sampler,
                         slot: EcLiquidSlot, uv: vec2<f32>, linear: bool) -> vec4<f32> {
    let local = uv * vec2<f32>(slot.extent);
    if (linear) {
        let px = vec2<f32>(slot.origin) + clamp(local, vec2<f32>(0.5), vec2<f32>(slot.extent) - vec2<f32>(0.5));
        return textureSample(atlas, atlas_sampler, px / vec2<f32>(textureDimensions(atlas)), slot.layer);
    }
    return textureLoad(atlas, vec2<i32>(slot.origin) + clamp(vec2<i32>(local), vec2<i32>(0), vec2<i32>(slot.extent) - vec2<i32>(1)), slot.layer, 0);
}

fn ec_liquid_reflection_uv(view_position: vec3<f32>, view_normal: vec3<f32>) -> vec2<f32> {
    let direction = reflect(normalize(view_position), normalize(view_normal));
    return direction.xy;
}

fn ec_liquid_texture_uv(uv: vec2<f32>, stretch: f32, scroll: vec2<f32>) -> vec2<f32> {
    // The source loader installs NiTextureTransform mode 2, scale 1/repetition.
    // With zero rotation its matrix maps (u,v) to (u/stretch+tx,1-v/stretch-ty).
    let transformed = uv / stretch + scroll;
    return vec2<f32>(transformed.x, 1.0 - transformed.y);
}

fn sample_ec_liquid(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>, atlas_sampler: sampler,
                    payload: u32, world_xz: vec2<f32>, reflection_uv: vec2<f32>,
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
        // Techniques 1/2 use explicit coordinates (lava/acid), whereas the
        // original water's Technique 0 generates a camera reflection vector.
        if (params.x == 1u || params.x == 2u) {
            coordinates = world_xz;
        }
        uv = ec_liquid_texture_uv(coordinates, animated.stretch, vec2<f32>(0.0));
    }
    if (!surface.present) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    if (normal.present) {
        var velocity = vec2<f32>(0.0, -1.0 / normal.stretch);
        if (params.w != 0u) {
            velocity = bitcast<vec2<f32>>(params.yz);
        }
        let scroll = velocity * select(0.0, time, animate);
        var bump_uv = world_xz / normal.stretch + scroll;
        if (params.w != 0u) {
            bump_uv = ec_liquid_texture_uv(world_xz, normal.stretch, scroll);
        }
        var bump = texture_data_rgb(ec_liquid_texel(atlas, atlas_sampler, normal, bump_uv, linear).rgb).rg;
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
    return ec_liquid_texel(atlas, atlas_sampler, surface, uv, linear);
}
