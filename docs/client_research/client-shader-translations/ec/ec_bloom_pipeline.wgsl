// ============================================================================
// ec_bloom_pipeline.wgsl
//
// WGSL translation of the Enhanced Client (EC) post-process shader
// (source: Shaders.uop_B0_F6, "//#target ps_2_0").
//
// ---- WHAT THIS SHADER IS ----
// This is a SCREEN-SPACE post-processing pipeline applied to the fully
// composited scene *after* all terrain, statics, mobiles, and UI have been
// rendered into the main framebuffer.  It implements classic HDR bloom:
//
//   1. Render scene to HDR framebuffer.
//   2. Bright-pass: extract bright regions above a luma threshold.
//   3. Downscale: reduce bright-pass result to 1/4 (2×2) or 1/16 (4×4).
//   4. Blur: separable Gaussian (first horizontal, then vertical pass).
//   5. Bloom final: composite blurred bloom back on top of the scene.
//   6. Tone-map: compress HDR range to [0,1] display range.
//   7. Monochrome: optional desaturation (e.g. death / fog-of-war effect).
//
// ---- WHAT IT IS APPLIED ON ----
// Entire rendered frame texture (screen-space quad — two triangles covering
// the full viewport).  In Bevy this corresponds to a custom
// `RenderPass` that runs after `MainPass` and writes to the swap-chain.
//
// ---- BEVY INTEGRATION NOTE ----
// These are standalone fragment functions intended to be assembled into a
// Bevy `SpecializedRenderPipeline`.  Each "technique" becomes a separate
// pipeline variant (or a single pipeline with a push-constant/uniform
// selecting the technique at runtime).
//
// Bindings match the convention used by Bevy's built-in post-process passes
// (group 0, binding 0 = main texture, binding 1 = sampler, binding 2 =
// optional second texture for the bloom-final composite pass).
// ============================================================================


// ============================================================================
// Shared constants
// ============================================================================

// Maximum sample count for the downscale and blur passes.
const MAX_SAMPLES: u32 = 16u;

// EC luma scale applied during downscaling — slightly attenuates the
// downsampled colour so the initial bright-pass is not clipped immediately.
const LUMA_SCALE: f32 = 0.1;

// Reference luminance for the tone-mapper (scene average luma estimate).
// Higher values darken the image; lower values brighten it.
const LUMINANCE: f32 = 2.0;

// Bloom intensity multiplier applied in the final composite.
const BLOOM_SCALE: f32 = 2.0;

// Middle-grey reference and white-point cutoff for the Reinhard tone-mapper.
const F_MIDDLE_GRAY: f32  = 0.18;
const F_WHITE_CUTOFF: f32 = 0.8;

// Standard luminance weights (Rec.709).
const LUMINANCE_CONV: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);


// ============================================================================
// GPU resource bindings
// ============================================================================
// Binding layout (group 0) mirrors Bevy's PostProcess conventions:
//   binding(0) — primary input texture  (scene / bright-pass / blurred result)
//   binding(1) — linear clamp sampler for the primary texture
//   binding(2) — secondary texture      (original HDR scene for bloom-final)
//   binding(3) — linear clamp sampler for the secondary texture
//   binding(4) — uniform block          (sample offsets + weights)

@group(0) @binding(0) var base_texture: texture_2d<f32>;
@group(0) @binding(1) var base_sampler: sampler;        // point-clamp or linear-clamp
@group(0) @binding(2) var blend_texture: texture_2d<f32>; // only used by bloom_final
@group(0) @binding(3) var blend_sampler: sampler;       // linear-clamp

// Sample offsets and weights uploaded from the CPU per-pass.
// The EC packs these into float2/float4 arrays; we use vec2/vec4 arrays here.
struct BloomParams {
    // Screen-space UV offsets for the kernel taps.  The CPU pre-computes
    // these in normalised UV space (0..1) for the current render-target size.
    sample_offsets: array<vec2<f32>, 16>,
    // Per-tap RGBA weights.  Only the RGB channels are used; alpha is padding.
    sample_weights: array<vec4<f32>, 16>,
}

@group(0) @binding(4) var<uniform> params: BloomParams;


// ============================================================================
// Technique 0 — DOWNSCALE_4×4
//
// PURPOSE: Reduces the input texture to 1/16 of its original resolution by
//          averaging 16 taps spread across a 4×4 neighbourhood.  Used as the
//          first stage of the bloom pipeline to cheaply obtain a low-resolution
//          version of the bright-pass result before blurring.
//
// APPLIED ON: Bright-pass render target → quarter-resolution bloom buffer.
// ============================================================================

fn downscale_4x4(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);

    for (var i: u32 = 0u; i < 16u; i++) {
        sample_acc += textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    sample_acc /= 16.0;

    // Attenuate each channel by LUMA_SCALE so the accumulated bloom
    // result does not over-brighten after compositing.
    return vec4<f32>(sample_acc.rgb * LUMA_SCALE, 1.0);
}


// ============================================================================
// Technique 1 — GAUSS_BLUR_5×5
//
// PURPOSE: Approximates a 5×5 Gaussian kernel by sampling the 13 closest
//          points around the centre texel and applying pre-computed Gaussian
//          weights (passed via sample_weights).  This is the separable-blur
//          workhorse: the same shader is called twice — once per axis — with
//          offsets along X (horizontal) and Y (vertical) respectively.
//
// APPLIED ON: Downscaled bloom buffer → blurred bloom buffer.
// ============================================================================

fn gauss_blur_5x5(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);

    // 13-tap kernel (the classic dx9 / SM2.0 approximation of a full 5×5)
    for (var i: u32 = 0u; i <= 12u; i++) {
        sample_acc += params.sample_weights[i] *
                      textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc;
}


// ============================================================================
// Technique 2 — DOWNSCALE_2×2
//
// PURPOSE: Reduces the input texture to 1/4 of its resolution by averaging
//          a 2×2 block of taps.  Lighter than DOWNSCALE_4×4; used as an
//          intermediate step when the full 16-tap reduction is not needed.
//
// APPLIED ON: Scene/bright-pass buffer → half-resolution buffer.
// ============================================================================

fn downscale_2x2(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);

    for (var i: u32 = 0u; i < 4u; i++) {
        sample_acc += textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc / 4.0;
}


// ============================================================================
// Technique 3 — BLOOM (separable one-directional Gaussian)
//
// PURPOSE: A 15-tap one-directional Gaussian blur.  Because Gaussian filters
//          are mathematically separable, this pass is executed *twice*:
//            Pass A — horizontal blur (offsets along X).
//            Pass B — vertical blur   (offsets along Y).
//          The result is a smooth, isotropic blur at a fraction of the cost
//          of a full 2D kernel.
//
// APPLIED ON: Downscaled bright-pass buffer → blurred bloom buffer.
// ============================================================================

fn bloom(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);

    for (var i: u32 = 0u; i < 15u; i++) {
        sample_acc += params.sample_weights[i] *
                      textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc;
}


// ============================================================================
// Technique 4 — BLOOM_FINAL (HDR composite)
//
// PURPOSE: Composites the blurred bloom result back over the original scene:
//
//   output = bloom_buffer * BLOOM_SCALE
//
// NOTE: In the original EC code the full-scene addition
//   "output = scene + BLOOM_SCALE * bloom_buffer"
// was commented out; only the raw scaled bloom is written.  This means the
// final composite was done at a higher level by the EC renderer (likely using
// additive blending in the render state rather than in the shader itself).
// For UODynamapper you will want to re-enable the additive composite shown
// in the commented variant below.
//
// APPLIED ON: Blurred bloom buffer → final swap-chain texture.
// ============================================================================

fn bloom_final(uv: vec2<f32>) -> vec4<f32> {
    // ---- Original EC behaviour (bloom-only additive output) ----
    let bloom_sample = textureSample(blend_texture, blend_sampler, uv);
    return bloom_sample * BLOOM_SCALE;

    // ---- Recommended UODynamapper variant (full composite) ----
    // let scene_sample = textureSample(base_texture, base_sampler, uv);
    // let bloom_sample = textureSample(blend_texture, blend_sampler, uv);
    // return scene_sample + bloom_sample * BLOOM_SCALE;
}


// ============================================================================
// Technique 5 — MONOCHROME (full desaturation)
//
// PURPOSE: Converts the entire screen to greyscale using Rec.709 luma
//          weights.  Applied as a full-screen pass to indicate a dramatic
//          game state (e.g. character is dead, frozen, stunned, or the
//          world is drained of colour for a cinematic effect).
//
// APPLIED ON: Full composited scene → greyscale frame.
// ============================================================================

fn monochrome(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);
    // dot() with luma weights collapses RGB to a single luminance value;
    // returning it as a vec4 gives a greyscale image.
    let luma = dot(color.rgb, LUMINANCE_CONV);
    return vec4<f32>(luma, luma, luma, color.a);
}


// ============================================================================
// Technique 6 — TONEMAP (Reinhard with middle-grey)
//
// PURPOSE: Compresses the HDR scene luminance range to [0,1] so it can be
//          displayed on standard 8-bit monitors.  Uses a filmic-flavoured
//          Reinhard operator:
//
//   L'  = L * fMiddleGray / (Luminance + ε)        [exposure adjustment]
//   L'' = L' * (1 + L' / fWhiteCutoff²)            [shoulder expansion]
//   out = L'' / (1 + L'')                           [Reinhard normalisation]
//
//          The shoulder term lifts near-white values before normalisation,
//          giving a subtle "filmic" roll-off rather than a hard clip.
//
// APPLIED ON: Full HDR scene texture → LDR output before display.
// ============================================================================

fn tonemap(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);

    // Exposure adjustment: scale by middle-grey relative to average luma.
    var out = color * (F_MIDDLE_GRAY / (LUMINANCE + 0.001));

    // Shoulder: amplify near-whites for a filmic roll-off.
    out *= (1.0 + (out / (F_WHITE_CUTOFF * F_WHITE_CUTOFF)));

    // Reinhard normalisation: maps (0..∞) to (0..1).
    out /= (1.0 + out);

    return vec4<f32>(out.rgb, color.a);
}


// ============================================================================
// Technique 7 — BRIGHT_PASS (high-pass / bloom extraction filter)
//
// PURPOSE: Isolates the luminous parts of the scene that will feed the bloom
//          blur.  Applies the same exposure + shoulder curve as the tone-
//          mapper, then *subtracts 5* (in HDR space) so that anything below
//          that energy threshold is clamped to zero.  The result is divided
//          by (10 + result) to keep the bright highlights in a manageable
//          range before downscaling and blurring.
//
//   L'  = L * fMiddleGray / (Luminance + ε)
//   L'' = L' * (1 + L' / fWhiteCutoff²)
//   out = max(0, L'' - 5) / (10 + max(0, L'' - 5))
//
// APPLIED ON: Full HDR scene → bright-pass buffer (input to downscale).
// ============================================================================

fn bright_pass(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);

    // Reuse the exposure + shoulder curve.
    var out_rgb = color.rgb * (F_MIDDLE_GRAY / (LUMINANCE + 0.001));
    out_rgb *= (1.0 + (out_rgb / (F_WHITE_CUTOFF * F_WHITE_CUTOFF)));

    // Subtract threshold: anything below 5 HDR units is discarded.
    out_rgb = max(out_rgb - 5.0, vec3<f32>(0.0));

    // Normalise to keep values in range.
    out_rgb /= (10.0 + out_rgb);

    return vec4<f32>(out_rgb, 1.0);
}


// ============================================================================
// Technique 8 — TEST (pass-through)
//
// PURPOSE: Debug/identity pass — outputs the input texture unchanged.
//          Useful for verifying bind-group layout and pass connectivity
//          without applying any effect.
//
// APPLIED ON: Any input texture → unchanged output.
// ============================================================================

fn test_passthrough(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(base_texture, base_sampler, uv);
}


// ============================================================================
// Fragment entry point
//
// A uniform `technique` value (0-8) selects which effect runs.
// In practice each technique is compiled into its own Bevy pipeline variant
// to avoid runtime branching on the GPU.
// ============================================================================

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Technique selector — set via a push-constant or a single-field uniform.
struct TechniqueUniform {
    index: u32,
    _pad: vec3<u32>,
}
@group(1) @binding(0) var<uniform> technique: TechniqueUniform;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // NOTE: In real Bevy usage each technique would be a separate pipeline
    // so the `if` chain below is never compiled together; it is shown here
    // only for clarity/reference.
    if      (technique.index == 0u) { return downscale_4x4(in.uv); }
    else if (technique.index == 1u) { return gauss_blur_5x5(in.uv); }
    else if (technique.index == 2u) { return downscale_2x2(in.uv); }
    else if (technique.index == 3u) { return bloom(in.uv); }
    else if (technique.index == 4u) { return bloom_final(in.uv); }
    else if (technique.index == 5u) { return monochrome(in.uv); }
    else if (technique.index == 6u) { return tonemap(in.uv); }
    else if (technique.index == 7u) { return bright_pass(in.uv); }
    else                            { return test_passthrough(in.uv); }
}
