// ============================================================================
// client_night_composite.wesl — The KR and Enhanced clients' night lighting.
//
// Mirrors `client_composite`, `night_overlay`, `light_buffer_over_night` and
// `night_map_uv` in src/core/render/scene/world/effects/client_night.rs, which
// documents the formulas and their sources in the clients. Per channel, on the
// 8-bit (sRGB-encoded) values the clients work on:
//
//     out = clamp(4 * (c * level + B * (1 - level)) * scene, 0, 1)
//     B   = lights.rgb + N' * (1 - lights.a)
//     N'  = clamp(luma(T) * (1 - strength) + T * strength, 0, 1),  T = N * color_ratio
//
// The frame arrives in linear light (the world camera's sRGB main texture);
// it is encoded to 8-bit values, composited, and decoded back.
// ============================================================================

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

// Must match ClientNightCompositeUniform in client_night.rs.
struct ClientNightCompositeUniform {
    level: f32,
    overlay_strength: f32,
    night_map_present: u32,
    _pad0: u32,
    overlay_color_ratio: vec4<f32>,
    flat_night_color: vec4<f32>,
    // xy: half the view in map repeats, zw: the focus in repeats.
    night_uv: vec4<f32>,
}

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> night: ClientNightCompositeUniform;
@group(0) @binding(2) var night_map: texture_2d<f32>;
@group(0) @binding(3) var night_map_sampler: sampler;
@group(0) @binding(4) var light_buffer: texture_2d<f32>;

// CLIENT_AMBIENT_TERM, CLIENT_COMPOSITE_GAIN, CLIENT_FULL_DAY_LEVEL and NIGHT_OVERLAY_LUMA_WEIGHTS in client_night.rs.
const CLIENT_AMBIENT_TERM: f32 = 0.25;
const CLIENT_COMPOSITE_GAIN: f32 = 4.0;
const CLIENT_FULL_DAY_LEVEL: f32 = 1.0;
const NIGHT_OVERLAY_LUMA_WEIGHTS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The sRGB transfer functions (IEC 61966-2-1), as the GPU's sRGB formats apply them.
const SRGB_LINEAR_LIMIT: f32 = 0.0031308;
const SRGB_ENCODED_LIMIT: f32 = 0.04045;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_SCALE: f32 = 1.055;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_GAMMA: f32 = 2.4;

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = c * SRGB_LINEAR_SLOPE;
    let high = SRGB_SCALE * pow(c, vec3<f32>(1.0 / SRGB_GAMMA)) - SRGB_OFFSET;
    return select(high, low, c <= vec3<f32>(SRGB_LINEAR_LIMIT));
}

fn decode_srgb(encoded: vec3<f32>) -> vec3<f32> {
    let low = encoded / SRGB_LINEAR_SLOPE;
    let high = pow((encoded + SRGB_OFFSET) / SRGB_SCALE, vec3<f32>(SRGB_GAMMA));
    return select(high, low, encoded <= vec3<f32>(SRGB_ENCODED_LIMIT));
}

fn night_overlay(texel: vec3<f32>) -> vec3<f32> {
    let tinted = texel * night.overlay_color_ratio.rgb;
    let luma = vec3<f32>(dot(tinted, NIGHT_OVERLAY_LUMA_WEIGHTS));
    let strength = night.overlay_strength;
    return clamp(luma * (1.0 - strength) + tinted * strength, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let scene = textureLoad(scene_texture, pixel, 0);
    if night.level >= CLIENT_FULL_DAY_LEVEL {
        return scene;
    }

    var night_texel = night.flat_night_color.rgb;
    if night.night_map_present != 0u {
        let uv = (in.uv * 2.0 - vec2<f32>(1.0)) * night.night_uv.xy + night.night_uv.zw;
        night_texel = textureSampleLevel(night_map, night_map_sampler, uv, 0.0).rgb;
    }

    // The light buffer matches the frame's size; clamp for the frame after a resize.
    let light_size = vec2<i32>(textureDimensions(light_buffer)) - vec2<i32>(1);
    let lights = textureLoad(light_buffer, clamp(pixel, vec2<i32>(0), light_size), 0);
    let light_buffer_rgb = lights.rgb + night_overlay(night_texel) * (1.0 - lights.a);

    let level = night.level;
    let multiplier = CLIENT_COMPOSITE_GAIN * (vec3<f32>(CLIENT_AMBIENT_TERM * level) + light_buffer_rgb * (1.0 - level));
    let composited = clamp(multiplier * encode_srgb(scene.rgb), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(decode_srgb(composited), scene.a);
}
