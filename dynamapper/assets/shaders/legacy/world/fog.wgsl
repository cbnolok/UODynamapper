// Legacy world-space material fog helper.
//
// This file is intentionally not imported by active shaders. Runtime fog is
// applied by shaders/world/effects/fog_pass.wgsl as a fullscreen post-process pass.

#import bevy_pbr::mesh_view_bindings::globals
#import "shaders/world/land/noise.wgsl"::{hash, fbm_billow, domain_warp}

#import "shaders/world/common_bindings.wgsl"::{SceneUniform, GlobalLightingUniforms}

fn calculate_fog_factor(
    world_pos: vec3<f32>,
    scene: SceneUniform,
    global_light: GlobalLightingUniforms,
) -> f32 {
    if (global_light.enable_fog == 0u) {
        return 0.0;
    }

    let dist_density_ui = clamp(global_light.fog_params.x, 0.0, 1.0);
    let height_density_ui = clamp(global_light.fog_params.y, 0.0, 1.0);
    let noise_scale_ui = clamp(global_light.fog_params.z, 0.0, 2.0);
    let noise_strength_ui = clamp(global_light.fog_params.w, 0.0, 1.0);

    // ---- 1. Linear Distance-based Fog ----
    // Map user dist_density_ui to fog_end (meters).
    let fog_end = mix(6000.0, 40.0, smoothstep(0.0, 0.2, dist_density_ui));
    let fog_start = max(0.0, fog_end * 0.06);
    let d = length(world_pos.xyz - scene.camera_position);
    let dist_factor = smoothstep(fog_start, fog_end, d);

    // ---- 2. Height-based Fog (Exponential) ----
    // Fog height bias: -1 valley, 0 neutral, +1 high-alt haze
    let hBias = clamp(global_light.gloom_params.w, -1.0, 1.0);
    let high_w = max(hBias, 0.0);
    let low_w  = max(-hBias, 0.0);

    let height_falloff = mix(800.0, 6.0, smoothstep(0.0, 0.2, height_density_ui));
    let y = world_pos.y;
    var height_term_high = 0.0;
    var height_term_low  = 0.0;
    if (height_density_ui > 1e-6) {
      height_term_high = 1.0 - exp(-max(y, 0.0) / height_falloff);
      height_term_low  = 1.0 - exp(-max(-y, 0.0) / height_falloff);
    }
    let height_factor = clamp(high_w * height_term_high + low_w * height_term_low, 0.0, 1.0);

    // Combine distance & height (union) -> base_fog [0..1]
    let base_fog = clamp(dist_factor + height_factor - dist_factor * height_factor, 0.0, 1.0);

    // ---- 3. Noise / clouds ----
    let noise_scale_world = mix(0.004, 0.25, clamp(noise_scale_ui / 2.0, 0.0, 1.0));
    let noise_strength = noise_strength_ui;

    let base_time_speed = 0.04;
    let t = globals.time * base_time_speed;
    let wind_dir = normalize(vec2<f32>(-scene.light_direction.z, scene.light_direction.x));
    let wind = wind_dir * (1.0 + 0.5 * noise_strength);

    let p0 = (world_pos.xz * noise_scale_world) + wind * (t * 8.0);

    var n_billow = 0.0;
    if (noise_strength > 0.001) {
      var p_final = p0;
      if (noise_strength > 0.2) {
        let warp_strength = 0.4 * noise_strength + 0.08;
        p_final = domain_warp(p0, warp_strength);
      }

      let n1 = fbm_billow(p_final * 1.0);
      var n2 = 0.0;
      if (scene.render_zoom < 12.0) {
        n2 = fbm_billow(p_final * 2.2 + vec2<f32>(1.7, -2.1));
      }
      n_billow = mix(n1, (n1 + n2 * 0.5) / 1.5, clamp(noise_strength * 1.2, 0.0, 1.0));
    }

    let base_threshold = mix(0.72, 0.46, noise_strength);
    let edge_width = mix(0.12, 0.20, 1.0 - noise_strength);
    let cloud_mask = smoothstep(base_threshold, base_threshold + edge_width, n_billow);

    let baseline = mix(0.03, 0.18, 1.0 - noise_strength);
    let peak_gain = mix(1.0, 2.2, noise_strength);
    let cloud_mod = baseline + (peak_gain - baseline) * cloud_mask;

    var fog_factor = clamp(base_fog * cloud_mod, 0.0, 1.0);

    let breath = 0.5 + 0.5 * sin(globals.time * (0.08 + 0.04 * noise_strength) + (hash(world_pos.xz * 0.11) * 6.2831));
    fog_factor = clamp(fog_factor * mix(0.95, 1.05, (breath - 0.5) * 0.8 * noise_strength), 0.0, 1.0);

    return clamp(fog_factor * global_light.fog_color.a, 0.0, 1.0);
}
