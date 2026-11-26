// ============================================================================
// fog_pass.wesl — the drifting fog: soft cloud banks over the whole frame.
//
// A fullscreen pass after tonemapping (render/postprocess/fog_pass.rs). Each pixel reads a domain-warped gradient
// noise on the camera plane, at its offset from the camera focus plus the layer's offsets, which hold the focus
// and the wind's travel: the banks are anchored to the map and drift over it. The noise's density between the
// fog's start and full values mixes the frame toward the fog colour, up to the fog's amount; alpha (the client
// shadow buffer) is kept.
//
// The noise is in effects/fog_noise.wesl.
// ============================================================================

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::shaders::world::effects::fog_noise::fog_density;

// The screen texture (rendered world, land + art)
@group(0) @binding(0)
var screen_texture: texture_2d<f32>;
// Sampler for the screen texture
@group(0) @binding(1)
var screen_sampler: sampler;
@group(0) @binding(2)
var<uniform> fog: FogPostProcessUniform;

// Matches FogPostProcessUniform in fog_pass.rs.
struct FogPostProcessUniform {
    // rgb: the fog colour, the night tint mixed in; a: the opacity where the cloud is thickest.
    color: vec4<f32>,
    // xy: half the view in cloud cells (x across, y down); z, w: the noise value where the fog starts and is full.
    view: vec4<f32>,
    // xy: the cloud layer's position on the camera plane, zw: the warp layer's, in cells modulo the noise period.
    offsets: vec4<f32>,
};

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let scene_color = textureSample(screen_texture, screen_sampler, in.uv);
    let s = (in.uv * 2.0 - vec2<f32>(1.0)) * fog.view.xy;
    let fog_mix = fog_density(s, fog.offsets, fog.view.z, fog.view.w) * fog.color.a;
    return vec4<f32>(mix(scene_color.rgb, fog.color.rgb, fog_mix), scene_color.a);
}
