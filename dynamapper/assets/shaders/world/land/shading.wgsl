// ============================================================================
// land::shading — The three terrain shading models, selectable at runtime
//                 via effects.shading_mode.
//
//  Mode 0: "2D Classic"   — Gouraud/vertex; faithful brightness model.
//  Mode 1: "Enhanced 2D"  — per-fragment; subtle diffuse tint, fill, rim.
//  Mode 2: "KR-like"      — per-fragment; richer style: warm key, cool fill,
//                           stronger rim/spec, gloom integration.
// ============================================================================


#import "shaders/world/common_bindings.wgsl"::{GlobalLightingUniforms}
#import "shaders/world/land/land_bindings.wgsl"::{LandLightingUniforms, effects, global_light, land_light, static_lights}
#import "shaders/world/land/lighting.wgsl"::{luminance, chroma_only, get_lambert, get_specular, get_rim, get_hemisphere_fill, apply_gloom}
#import "shaders/world/land/noise.wgsl"::hash

// ============================================================================
// Mode 0: Classic (Gouraud, vertex-lit)
// ============================================================================

// Simple brightness model: albedo * (ambient + diffuse * N·L).
// N·L is pre-computed in the vertex shader and passed as lam_v.
fn shade_mode0_classic_vertex(base_albedo: vec3<f32>, lam_v: f32,
                              ambient_strength: f32, diffuse_strength: f32) -> vec3<f32> {
  return base_albedo * (ambient_strength + diffuse_strength * lam_v);
}

// ============================================================================
// Mode 1: Enhanced (per-fragment, subtle improvements)
// ============================================================================

// Diffuse tinted by light_color; fill chroma minimal; very subtle rim/spec.
fn shade_mode1_enhanced_fragment(base_albedo_in: vec3<f32>,
                                 world_pos: vec3<f32>, Nw: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
                                 ambient_strength: f32, diffuse_strength: f32,
                                 sharpness_factor: f32, sharpness_mix: f32,
                                 fill_strength: f32, rim_strength: f32,
                                 specular_strength: f32,
                                 enable_gloom: u32) -> vec3<f32> {

  // Diffuse shaping (raises Lambert to sharpen sun side a bit)
  let lam = get_lambert(Nw, L);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, clamp(sharpness_mix, 0.0, 0.4));

  // Energy RGB (allow warm key light tint)
  let diffuse_rgb = global_light.light_color * (diffuse_strength * lam_shaped);
  let hemi_rgb = select(vec3<f32>(0.0), (get_hemisphere_fill(Nw) * fill_strength), (fill_strength > 0.0));

  // Base energy is RGB now: ambient (scalar) + fill luma + tinted diffuse
  let hemi_luma = luminance(hemi_rgb);
  var energy_rgb = vec3<f32>(ambient_strength + hemi_luma) + diffuse_rgb;

  // Headroom limiting via runtime toggle only (prevents bleaching)
  let headroom_reserve = 0.10;
  if (global_light.grade_params.w >= 0.5) {
    energy_rgb = min(energy_rgb, vec3<f32>(1.0 - headroom_reserve));
  }

  var color = base_albedo_in * energy_rgb;

  // Small chroma from fill (keeps shadows colorful but subtle)
  let hemi_chroma_tint = min(global_light.grade_params.z, 0.20);
  let hemi_chroma = chroma_only(hemi_rgb);
  var headroom = 1.0 - max(color.r, max(color.g, color.b));
  let hemi_gain = min(headroom, hemi_luma * hemi_chroma_tint);
  color += base_albedo_in * (hemi_chroma * hemi_gain);

  // Very subtle rim/spec (limited to a quarter of the rim_strength)
  let rim_local = rim_strength * 0.25;
  if (rim_local > 0.001) {
    let rim_raw = get_rim(Nw, V, max(0.1, land_light.rim_color.a));
    let NdotL = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis = rim_raw * (1.0 - smoothstep(0.0, 0.35, NdotL));
    headroom = max(0.0, 1.0 - max(color.r, max(color.g, color.b)));
    color += base_albedo_in * min(headroom, rim_vis * rim_local * 0.30);
  }

  if (specular_strength > 0.0001) {
    let spec_val = get_specular(Nw, L, V, 24.0);
    color += vec3<f32>(1.0) * spec_val * (specular_strength * 0.5);
  }

  // Optional gloom
  if (enable_gloom == 1u) {
    color = apply_gloom(color, world_pos, Nw, L);
  }

  return color;
}

// ============================================================================
// Mode 2: KR-like (per-fragment, painterly / Kingdom Reborn style)
// ============================================================================

// Stronger style: warm key tint, full rim with color, richer gloom.
fn shade_mode2_kr_fragment(base_albedo_in: vec3<f32>,
                           world_pos: vec3<f32>, Nw: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
                           ambient_strength: f32, diffuse_strength: f32,
                           sharpness_factor: f32, sharpness_mix: f32,
                           fill_strength: f32, rim_strength: f32,
                           specular_strength: f32,
                           enable_gloom: u32) -> vec3<f32> {

  // Wrapped diffuse: when wrap > 0, lighting wraps around the surface
  // so shadows never go fully black (KR-style soft shadow terminator).
  // wrap = 0.0 → standard Lambert, wrap = 0.5 → half-Lambert.
  let wrap = land_light.diffuse_wrap;
  let raw_ndotl = dot(normalize(Nw), L);
  let lam = select(max(raw_ndotl, 0.0), max(raw_ndotl * (1.0 - wrap) + wrap, 0.0), wrap > 0.001);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, sharpness_mix);

  // Energy RGB (warm key tint)
  let diffuse_rgb = global_light.light_color * (diffuse_strength * lam_shaped);
  let hemi_rgb = select(vec3<f32>(0.0), (get_hemisphere_fill(Nw) * fill_strength), (fill_strength > 0.0));
  let hemi_luma = luminance(hemi_rgb);

  // Headroom control
  let headroom_reserve = clamp(global_light.grade_params.y, 0.0, 1.0);
  let runtime_headroom_on = global_light.grade_params.w >= 0.5;

  var energy_rgb = vec3<f32>(ambient_strength + hemi_luma) + diffuse_rgb;
  if (runtime_headroom_on) {
    energy_rgb = min(energy_rgb, vec3<f32>(1.0 - headroom_reserve));
  }

  var color = base_albedo_in * energy_rgb;

  // Fill chroma
  let hemi_chroma_tint = clamp(global_light.grade_params.z, 0.0, 1.0);
  let hemi_chroma = chroma_only(hemi_rgb);
  var headroom = 1.0 - max(color.r, max(color.g, color.b));
  let hemi_gain = min(headroom, hemi_luma * hemi_chroma_tint);
  color += base_albedo_in * (hemi_chroma * hemi_gain);

  // KR-style temperature separation: cooled, slightly compressed shadow
  // slopes against a restrained warm sun side.
  let land_temperature_strength = clamp(effects.kr_land_temperature_strength, 0.0, 1.5);
  let sun_mask = smoothstep(0.22, 0.88, lam_shaped);
  let shadow_mask = 1.0 - smoothstep(0.10, 0.58, max(raw_ndotl, 0.0));
  let color_luma = luminance(color);
  color = mix(color, vec3<f32>(color_luma), shadow_mask * 0.16 * land_temperature_strength);
  color *= mix(vec3<f32>(1.0), vec3<f32>(0.82, 0.90, 1.06), shadow_mask * 0.20 * land_temperature_strength);
  color *= mix(vec3<f32>(1.0), vec3<f32>(1.08, 1.03, 0.94), sun_mask * 0.14 * land_temperature_strength);

  // Terrain relief darkening: shaded steep slopes get a little extra weight
  // so heightfield creases do not flatten under the wrapped diffuse.
  let relief_shadow_strength = clamp(effects.kr_land_relief_shadow_strength, 0.0, 1.5);
  let slope_steepness = clamp(1.0 - normalize(Nw).y, 0.0, 1.0);
  let crease_shadow = smoothstep(0.08, 0.42, slope_steepness) * (0.35 + 0.65 * shadow_mask);
  color *= 1.0 - crease_shadow * 0.12 * relief_shadow_strength;

  let shadow_mottle_strength = clamp(effects.kr_land_shadow_mottle_strength, 0.0, 1.5);
  let shadow_mottle = 0.82 + 0.18 * hash(floor(world_pos.xz * 1.15));
  color *= 1.0 - shadow_mask * (1.0 - shadow_mottle) * 0.10 * shadow_mottle_strength;

  // Rim (colored + neutral, headroom-gated)
  if (rim_strength > 0.001) {
    let rim_power = max(0.1, land_light.rim_color.a);
    let rim_raw   = get_rim(Nw, V, rim_power);
    let NdotL     = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis   = rim_raw * (1.0 - smoothstep(0.0, 0.35, NdotL));
    headroom = max(0.0, 1.0 - max(color.r, max(color.g, color.b)));
    let rim_neutral = min(headroom, rim_vis * rim_strength * 0.35);
    color += base_albedo_in * rim_neutral;
    let rim_colored = min(headroom, rim_vis * rim_strength * 0.25);
    color += land_light.rim_color.rgb * rim_colored;
  }

  // Specular
  if (specular_strength > 0.0001) {
    let spec_val = get_specular(Nw, L, V, 32.0);
    color += vec3<f32>(1.0) * spec_val * specular_strength;
  }

  let static_light_count = min(static_lights.params.x, 16u);
  var static_light_accum = 0.0;
  for (var i = 0u; i < 16u; i = i + 1u) {
    if (i >= static_light_count) {
      break;
    }
    let light = static_lights.lights[i];
    let delta = world_pos - light.xyz;
    let radius = max(light.w, 0.01);
    let dist = length(vec3<f32>(delta.x, delta.y * 1.8, delta.z));
    let falloff = max(1.0 - dist / radius, 0.0);
    static_light_accum += falloff * falloff;
  }

  if (static_light_accum > 0.001) {
    let local_light = min(static_light_accum, 1.0);
    let local_warm = vec3<f32>(1.0, 0.72, 0.42);
    color += base_albedo_in * local_warm * local_light * 0.32;
  }

  // Optional gloom
  if (enable_gloom == 1u) {
    color = apply_gloom(color, world_pos, Nw, L);
  }

  return color;
}
