// ============================================================================
// ec_death_effect.wgsl
//
// WGSL translation of the Enhanced Client (EC) death/monochrome effect shader
// (source: Shaders.uop_B0_F4, "//#target ps_2_0 — Death effect shader").
//
// ---- WHAT THIS SHADER IS ----
// A dead-simple full-screen greyscale filter applied when the player's
// character dies.  The entire rendered frame is converted to luminance using
// Rec.709 weights, producing the dramatic "world drains of colour" effect
// seen in the EC when your avatar is killed.
//
// It is conceptually identical to Technique 5 (MONOCHROME) in the bloom-
// pipeline shader (ec_bloom_pipeline.wgsl), but it was a *dedicated* shader
// file in the EC — likely for pipeline-state simplicity (no need to set a
// technique index; just swap the active shader).
//
// ---- WHAT IT IS APPLIED ON ----
// Full composited scene texture (screen-space quad).  It runs as a final
// post-process pass *after* all game geometry, particles, and UI have been
// rendered.  In Bevy this would be a `RenderPass` with a full-screen triangle
// that runs at the very end of the frame pipeline, writing to the swap-chain.
//
// ---- WHEN IT IS ACTIVE ----
// Toggled on/off by the EC game logic (C++ / script side) when the player
// character's health drops to zero.  UODynamapper would control this via a
// `PostProcessSettings` resource or a dedicated ECS component/state.
//
// ---- INTENSITY EXTENSION ----
// The original EC shader is binary (on/off).  A `strength` uniform has been
// added here so the effect can be faded in/out smoothly over time (e.g.
// lerp from colour → grey over 0.5 s after death).
// ============================================================================


// ============================================================================
// GPU resource bindings (group 0 — Bevy PostProcess convention)
// ============================================================================

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

// Strength of the desaturation effect:
//   0.0 = full colour (effect disabled / fading out)
//   1.0 = fully greyscale (classic EC death state)
// Set to 1.0 to replicate the original EC binary behaviour.
struct DeathEffectParams {
    strength: f32,
    _pad: vec3<f32>,
}
@group(0) @binding(2) var<uniform> params: DeathEffectParams;


// ============================================================================
// Rec.709 luma weights — same constants used across all EC shaders.
// ============================================================================

const LUMINANCE_CONV: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);


// ============================================================================
// Fragment shader
// ============================================================================

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(scene_texture, scene_sampler, in.uv);

    // Compute luminance (Rec.709 weights: perceptually-correct greyscale).
    let luma = dot(color.rgb, LUMINANCE_CONV);
    let grey = vec3<f32>(luma);

    // Blend between original colour and greyscale according to strength.
    // strength = 1.0 reproduces the original EC binary death effect exactly.
    let out_rgb = mix(color.rgb, grey, params.strength);

    return vec4<f32>(out_rgb, color.a);
}
