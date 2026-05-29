// ============================================================================
// fog_pass.wgsl — Fullscreen fog post-processing pass.
//
// This runs as a fullscreen triangle after the main 3D pass, applying fog
// uniformly over all rendered geometry (land + art tiles + any future passes).
// This avoids the problem of applying fog independently in each material shader,
// which caused inconsistent coverage when art tiles weren't fogged.
//
// The fog calculation is driven by the same parameters as the old per-material
// fog (FogPostProcessUniform), which is kept in sync with GlobalLightingUniforms.
//
// Depth buffer access is not available through FullscreenMaterial, so we use
// a screen-space radial approach: approximate scene depth by the pixel's
// distance from the screen center. For an isometric view with a fixed camera
// angle, this correlates well with world-space distance from the player.
// ============================================================================

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

// The screen texture (rendered world, land + art)
@group(0) @binding(0)
var screen_texture: texture_2d<f32>;
// Sampler for the screen texture
@group(0) @binding(1)
var screen_sampler: sampler;
// Fog parameters (synced from GlobalLightingUniforms each frame)
@group(0) @binding(2)
var<uniform> fog: FogPostProcessUniform;

// ---- Fog uniform (matches FogPostProcessUniform in Rust) ----
// Padded to 16-byte alignment following WGSL/encase rules.
struct FogPostProcessUniform {
    // --- Toggles ---
    enable_fog: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,

    // --- Color ---
    // fog_color.a = max_mix (opacity cap)
    fog_color: vec4<f32>,
    // Night fog color. .a = night blend factor (0 = day, 1 = full night)
    fog_night_color: vec4<f32>,

    // --- Params ---
    // fog_params = [distance_density, height_density, noise_scale, noise_strength]
    fog_params: vec4<f32>,

    // --- Scene info for distance calculation ---
    // screen resolution in pixels
    screen_size: vec2<f32>,
    _pad_ss: vec2<f32>,
};

// ============================================================================
// Fog factor: pure screen-space radial distance from screen centre.
//
// For an isometric top-down view with a fixed camera offset, pixels closer to
// the screen edges correspond to greater world-space distances. A radial
// measure from screen-centre is a cheap and visually plausible proxy.
// ============================================================================
fn fog_hash(p: vec2<f32>) -> f32 {
    let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    let p3s = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
    return fract((p3s.x + p3s.y) * p3s.z);
}

fn fog_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = fog_hash(i + vec2<f32>(0.0, 0.0));
    let b = fog_hash(i + vec2<f32>(1.0, 0.0));
    let c = fog_hash(i + vec2<f32>(0.0, 1.0));
    let d = fog_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fog_fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var f = 1.0;
    sum += amp * fog_noise(p * f); f *= 2.0; amp *= 0.5;
    sum += amp * fog_noise(p * f); f *= 2.0; amp *= 0.5;
    sum += amp * fog_noise(p * f); f *= 2.0; amp *= 0.5;
    sum += amp * fog_noise(p * f);
    return clamp(sum, 0.0, 1.0);
}

fn fog_factor_screen_space(uv: vec2<f32>) -> f32 {
    let dist_density = clamp(fog.fog_params.x, 0.0, 1.0);
    let height_density = clamp(fog.fog_params.y, 0.0, 1.0);
    let noise_scale = max(fog.fog_params.z, 0.0);
    let noise_strength = clamp(fog.fog_params.w, 0.0, 1.0);
    if (dist_density < 0.001 && height_density < 0.001) {
        return 0.0;
    }

    // Signed distance from screen center in UV space ([0,1])
    let center = vec2<f32>(0.5, 0.5);
    let d = length(uv - center); // 0 at center, ~0.707 at corner

    // The UI/presets use small density values in the 0.02..0.08 range. Use
    // sqrt shaping so those values produce visible edge haze instead of falling
    // below the screen's maximum radial distance.
    let fog_radius = mix(0.64, 0.08, sqrt(dist_density));
    let radial_fog = smoothstep(fog_radius, fog_radius + 0.35, d);
    let vertical_fog = smoothstep(0.40, 1.0, uv.y) * height_density * 2.0;

    var factor = clamp(radial_fog + vertical_fog, 0.0, 1.0);
    if (noise_scale > 0.001 && noise_strength > 0.001) {
        let aspect = max(fog.screen_size.x / max(fog.screen_size.y, 1.0), 0.1);
        let p = vec2<f32>(uv.x * aspect, uv.y) * noise_scale * 14.0 + vec2<f32>(31.0, 17.0);
        let n = fog_fbm(p) * 2.0 - 1.0;
        factor = clamp(factor * (1.0 + n * noise_strength * 0.45), 0.0, 1.0);
    }

    return factor;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let scene_color = textureSample(screen_texture, screen_sampler, in.uv);

    // If fog is disabled, pass through unchanged.
    if (fog.enable_fog == 0u) {
        return scene_color;
    }

    let fog_mix = fog_factor_screen_space(in.uv) * fog.fog_color.a;

    if (fog_mix < 0.001) {
        return scene_color;
    }

    // Blend between day and night fog color based on blend factor.
    let night_blend = clamp(fog.fog_night_color.a, 0.0, 1.0);
    let effective_fog_color = mix(fog.fog_color.rgb, fog.fog_night_color.rgb, night_blend);

    let fogged = mix(scene_color.rgb, effective_fog_color, fog_mix);
    return vec4<f32>(fogged, scene_color.a);
}
