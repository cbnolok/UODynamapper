// ============================================================================
// land::shading — The three terrain shading models, selectable at runtime
//                 via effects.shading_mode.
//
//  Mode 0: "2D Classic"   — Gouraud/vertex; faithful brightness model.
//  Mode 1: "Enhanced 2D"  — per-fragment; subtle diffuse tint, fill, rim.
//  Mode 2: "KR-like"      — per-fragment; richer style: warm key, cool fill,
//                           stronger rim/spec, gloom integration.
// ============================================================================


import package::shaders::world::common_bindings::{GlobalLightingUniforms};
import package::shaders::world::land::land_bindings::{LandLightingUniforms, effects, global_light, land_light, static_lights};
import package::shaders::world::land::lighting::{luminance, chroma_only, get_lambert, get_specular, get_rim, get_hemisphere_fill, apply_gloom};
import package::shaders::world::land::noise::noise_2d;

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

fn apply_static_light_decals(color_in: vec3<f32>,
                             base_albedo_in: vec3<f32>,
                             world_pos: vec3<f32>) -> vec3<f32> {
  let decal_strength = clamp(effects.light_decal_intensity, 0.0, 2.0);
  if (decal_strength <= 0.001) {
    return color_in;
  }

  let static_light_count = min(static_lights.params.x, 16u);
  var static_light_rgb = vec3<f32>(0.0);
  var static_light_accum = 0.0;
  for (var i = 0u; i < 16u; i = i + 1u) {
    if (i >= static_light_count) {
      break;
    }
    let light = static_lights.lights[i];
    let light_color = static_lights.colors[i].rgb;
    let delta = world_pos - light.xyz;
    let radius = max(light.w, 0.01);
    let dist = length(vec3<f32>(delta.x, delta.y * 1.8, delta.z));
    let falloff = max(1.0 - dist / radius, 0.0);
    let contribution = falloff * falloff;
    static_light_accum += contribution;
    static_light_rgb += light_color * contribution;
  }

  if (static_light_accum <= 0.001) {
    return color_in;
  }

  let local_light = min(static_light_accum, 1.0) * decal_strength;
  let local_color = static_light_rgb / max(static_light_accum, 0.001);
  return color_in + base_albedo_in * local_color * local_light * 0.32;
}

// ============================================================================
// Mode 2: KR-like (per-fragment, painterly / Kingdom Reborn style)
// ============================================================================

// KR land shadow mottle: soft blotches about two tiles wide that darken the shade on land.
// Its strength slider ends at KR_SHADOW_MOTTLE_STRENGTH_MAX. Up to the knee, the highest preset strength
// (UOD-Painterly by day), it keeps its former depth, KR_SHADOW_MOTTLE_LINEAR_DEPTH per unit of strength (at most 1.08%
// darker there), so the presets look as before. Past the knee a quadratic term, flat where it starts, deepens the
// darkest blotches to KR_SHADOW_MOTTLE_MAX_DEPTH at the slider's end. The former cap was the depth at 1.5, 2.7%.
// Past the knee the blotches also reach sunlit land, up to KR_SHADOW_MOTTLE_LIT_REACH of their depth in the shade
// at the slider's end: under a high sun the shaded slopes are too few for the blotches to read as a mood.
const KR_SHADOW_MOTTLE_STRENGTH_MAX: f32 = 1.5;
const KR_SHADOW_MOTTLE_LINEAR_DEPTH: f32 = 0.018;
const KR_SHADOW_MOTTLE_KNEE: f32 = 0.6;
const KR_SHADOW_MOTTLE_MAX_DEPTH: f32 = 0.5;
const KR_SHADOW_MOTTLE_LIT_REACH: f32 = 0.6;
// Value-noise cells per world tile: blotches about 1 / 0.45 = 2.2 tiles wide.
const KR_SHADOW_MOTTLE_FREQUENCY: f32 = 0.45;

// How much darker the darkest shadow-mottle blotch is in full shade, for a strength on the slider.
fn kr_shadow_mottle_depth(strength: f32) -> f32 {
  let s = clamp(strength, 0.0, KR_SHADOW_MOTTLE_STRENGTH_MAX);
  let past_knee = max(s - KR_SHADOW_MOTTLE_KNEE, 0.0);
  let span = KR_SHADOW_MOTTLE_STRENGTH_MAX - KR_SHADOW_MOTTLE_KNEE;
  let deepening = (KR_SHADOW_MOTTLE_MAX_DEPTH - KR_SHADOW_MOTTLE_LINEAR_DEPTH * KR_SHADOW_MOTTLE_STRENGTH_MAX)
    / (span * span);
  return KR_SHADOW_MOTTLE_LINEAR_DEPTH * s + deepening * past_knee * past_knee;
}

// How much of the shadow mottle sunlit land gets, for a strength on the slider: none up to the knee.
fn kr_shadow_mottle_lit_reach(strength: f32) -> f32 {
  return smoothstep(KR_SHADOW_MOTTLE_KNEE, KR_SHADOW_MOTTLE_STRENGTH_MAX, strength) * KR_SHADOW_MOTTLE_LIT_REACH;
}

// KR land rim: a light on the slopes the camera sees at a grazing angle, those turned away from it. The grazing is
// measured from flat ground, which the camera sees at a fixed slant (N·V 0.58 for the orthographic camera): 0 on
// flat ground and on slopes facing the camera, 1 on land seen edge-on, raised to the rim power. In full shade (N·L 0)
// the rim has full weight; with more sun it fades to KR_RIM_SUNLIT_WEIGHT, reached at N·L KR_RIM_SUN_FADE_END. Under
// a sun behind the scene, as UOD-Painterly's, the slopes turned away from the camera face the sun, so a rim gated
// to the shade alone would light almost no land.
const KR_RIM_SUNLIT_WEIGHT: f32 = 0.2;
const KR_RIM_SUN_FADE_END: f32 = 0.35;
// Flat ground seen from at least this close to edge-on still counts as facing the camera, so a camera near the
// horizon does not turn all land into rim.
const KR_RIM_MIN_GROUND_FACING: f32 = 0.1;

// How far land is turned away from the camera, from 0 (as flat ground or more toward it) to 1 (edge-on).
fn kr_rim_grazing(N: vec3<f32>, V: vec3<f32>) -> f32 {
  let ground_facing = max(V.y, KR_RIM_MIN_GROUND_FACING);
  return clamp(1.0 - dot(normalize(N), V) / ground_facing, 0.0, 1.0);
}

// How much of the rim land gets for its sun exposure N·L.
fn kr_rim_sun_weight(n_dot_l: f32) -> f32 {
  return mix(1.0, KR_RIM_SUNLIT_WEIGHT, smoothstep(0.0, KR_RIM_SUN_FADE_END, n_dot_l));
}

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

  let shadow_mottle = noise_2d(world_pos.xz * KR_SHADOW_MOTTLE_FREQUENCY);
  let mottle_strength = effects.kr_land_shadow_mottle_strength;
  let mottle_mask = max(shadow_mask, kr_shadow_mottle_lit_reach(mottle_strength));
  color *= 1.0 - mottle_mask * (1.0 - shadow_mottle) * kr_shadow_mottle_depth(mottle_strength);

  // Rim (colored + neutral, headroom-gated)
  if (rim_strength > 0.001) {
    let rim_power = max(0.1, land_light.rim_color.a);
    let rim_raw   = pow(kr_rim_grazing(Nw, V), rim_power);
    let NdotL     = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis   = rim_raw * kr_rim_sun_weight(NdotL);
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

  // Optional gloom
  if (enable_gloom == 1u) {
    color = apply_gloom(color, world_pos, Nw, L);
  }

  return color;
}
