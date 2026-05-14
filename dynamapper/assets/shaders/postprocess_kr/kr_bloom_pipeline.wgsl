// ============================================================================
// kr_bloom_pipeline.wgsl
//
// WGSL translation of the Kingdom Reborn (KR) post-process shader
// (source: Shaders.uop_B0_F0, "//#target ps_2_0").
//
// ---- WHAT THIS SHADER IS ----
// This is identical to the Enhanced Client (EC) bloom pipeline.
// It implements a SCREEN-SPACE post-processing pipeline applied to the fully
// composited scene. It includes 9 techniques for downscaling, Gaussian blur,
// bloom compositing, monochrome, tonemapping, and bright-pass extraction.
//
// ---- KR vs EC DIFFERENCES ----
// The KR F0 post-processing shader is byte-for-byte identical to the EC F6
// post-processing shader. The same techniques and logic apply.
// ============================================================================

// ============================================================================
// Shared constants
// ============================================================================

const MAX_SAMPLES: u32 = 16u;
const LUMA_SCALE: f32 = 0.1;
const LUMINANCE: f32 = 2.0;
const BLOOM_SCALE: f32 = 2.0;
const F_MIDDLE_GRAY: f32  = 0.18;
const F_WHITE_CUTOFF: f32 = 0.8;
const LUMINANCE_CONV: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);

// ============================================================================
// GPU resource bindings
// ============================================================================

@group(0) @binding(0) var base_texture: texture_2d<f32>;
@group(0) @binding(1) var base_sampler: sampler;
@group(0) @binding(2) var blend_texture: texture_2d<f32>;
@group(0) @binding(3) var blend_sampler: sampler;

struct BloomParams {
    sample_offsets: array<vec2<f32>, 16>,
    sample_weights: array<vec4<f32>, 16>,
}
@group(0) @binding(4) var<uniform> params: BloomParams;

// ============================================================================
// Techniques
// ============================================================================

fn downscale_4x4(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);
    for (var i: u32 = 0u; i < 16u; i++) {
        sample_acc += textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    sample_acc /= 16.0;
    return vec4<f32>(sample_acc.rgb * LUMA_SCALE, 1.0);
}

fn gauss_blur_5x5(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);
    for (var i: u32 = 0u; i <= 12u; i++) {
        sample_acc += params.sample_weights[i] *
                      textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc;
}

fn downscale_2x2(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);
    for (var i: u32 = 0u; i < 4u; i++) {
        sample_acc += textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc / 4.0;
}

fn bloom(uv: vec2<f32>) -> vec4<f32> {
    var sample_acc = vec4<f32>(0.0);
    for (var i: u32 = 0u; i < 15u; i++) {
        sample_acc += params.sample_weights[i] *
                      textureSample(base_texture, base_sampler, uv + params.sample_offsets[i]);
    }
    return sample_acc;
}

fn bloom_final(uv: vec2<f32>) -> vec4<f32> {
    let bloom_sample = textureSample(blend_texture, blend_sampler, uv);
    return bloom_sample * BLOOM_SCALE;
}

fn monochrome(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);
    let luma = dot(color.rgb, LUMINANCE_CONV);
    return vec4<f32>(luma, luma, luma, color.a);
}

fn tonemap(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);
    var out = color * (F_MIDDLE_GRAY / (LUMINANCE + 0.001));
    out *= (1.0 + (out / (F_WHITE_CUTOFF * F_WHITE_CUTOFF)));
    out /= (1.0 + out);
    return vec4<f32>(out.rgb, color.a);
}

fn bright_pass(uv: vec2<f32>) -> vec4<f32> {
    let color = textureSample(base_texture, base_sampler, uv);
    var out_rgb = color.rgb * (F_MIDDLE_GRAY / (LUMINANCE + 0.001));
    out_rgb *= (1.0 + (out_rgb / (F_WHITE_CUTOFF * F_WHITE_CUTOFF)));
    out_rgb = max(out_rgb - 5.0, vec3<f32>(0.0));
    out_rgb /= (10.0 + out_rgb);
    return vec4<f32>(out_rgb, 1.0);
}

fn test_passthrough(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(base_texture, base_sampler, uv);
}

// ============================================================================
// Fragment entry point
// ============================================================================

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct TechniqueUniform {
    index: u32,
    _pad: vec3<u32>,
}
@group(1) @binding(0) var<uniform> technique: TechniqueUniform;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
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
