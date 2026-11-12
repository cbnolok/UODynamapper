// ============================================================================
// land::lighting — Lighting math helpers shared across all shading modes.
//
//  Includes: Lambert, Blinn-Phong specular, rim, hemisphere fill, gloom.
// ============================================================================


import package::shaders::world::common_bindings::{GlobalLightingUniforms};
import package::shaders::world::land::land_bindings::{LandLightingUniforms, global_light, land_light};

// ============================================================================
// Basic math helpers
// ============================================================================

// Perceptual luminance (BT.709 weights)
fn luminance(c: vec3<f32>) -> f32 {
  return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Return only the chromatic (achromatic-subtracted) part of a color.
fn chroma_only(c: vec3<f32>) -> vec3<f32> {
  let l = luminance(c);
  return c - vec3<f32>(l);
}

// ============================================================================
// Light contribution helpers
// ============================================================================

// Lambertian diffuse (saturated N·L)
fn get_lambert(N: vec3<f32>, L: vec3<f32>) -> f32 {
  return max(dot(normalize(N), L), 0.0);
}

// Blinn-Phong specular via half-vector
fn get_specular(N: vec3<f32>, L: vec3<f32>, V: vec3<f32>, shininess: f32) -> f32 {
  let H = normalize(L + V);
  return pow(max(dot(normalize(N), H), 0.0), shininess);
}

// Land rim, for the Enhanced and KR per-pixel shading: a light on the slopes the camera sees at a grazing angle,
// those turned away from it. The grazing is measured from flat ground, which the camera sees at a fixed slant
// (N·V 0.58 for the orthographic camera): 0 on flat ground and on slopes facing the camera, 1 on land seen edge-on,
// raised to the rim power. In full shade (N·L 0) the rim has full weight; with more sun it fades to
// RIM_SUNLIT_WEIGHT, reached at N·L RIM_SUN_FADE_END. Under a sun behind the scene, as UOD-Painterly's, the slopes
// turned away from the camera face the sun, so a rim gated to the shade alone would light almost no land.
const RIM_SUNLIT_WEIGHT: f32 = 0.2;
const RIM_SUN_FADE_END: f32 = 0.35;
// Flat ground seen from at least this close to edge-on still counts as facing the camera, so a camera near the
// horizon does not turn all land into rim.
const RIM_MIN_GROUND_FACING: f32 = 0.1;

// How far land is turned away from the camera, from 0 (as flat ground or more toward it) to 1 (edge-on).
fn rim_grazing(N: vec3<f32>, V: vec3<f32>) -> f32 {
  let ground_facing = max(V.y, RIM_MIN_GROUND_FACING);
  return clamp(1.0 - dot(normalize(N), V) / ground_facing, 0.0, 1.0);
}

// How much of the rim land gets for its sun exposure N·L.
fn rim_sun_weight(n_dot_l: f32) -> f32 {
  return mix(1.0, RIM_SUNLIT_WEIGHT, smoothstep(0.0, RIM_SUN_FADE_END, n_dot_l));
}

// The land rim's visibility, 0..1, for the rim power.
fn get_rim(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, power: f32) -> f32 {
  let n_dot_l = max(dot(normalize(N), normalize(L)), 0.0);
  return pow(rim_grazing(N, V), power) * rim_sun_weight(n_dot_l);
}

// Hemisphere fill light: sky from +Y, ground from -Y, blended by surface upness.
fn get_hemisphere_fill(N: vec3<f32>) -> vec3<f32> {
  let upness = clamp(dot(normalize(N), vec3<f32>(0.0, 1.0, 0.0)) * 0.5 + 0.5, 0.0, 1.0);
  let sky    = land_light.fill_sky_color.rgb    * land_light.fill_sky_color.a;
  let ground = land_light.fill_ground_color.rgb * land_light.fill_ground_color.a;
  return mix(ground, sky, upness);
}

// ============================================================================
// Post-process helpers
// ============================================================================

// Gloom: general darkening, height-fading, optional shadow bias.
// gloom_params: [amount, height_falloff_height, shadow_bias]
fn apply_gloom(color_in: vec3<f32>, world_pos: vec3<f32>, N: vec3<f32>, L: vec3<f32>) -> vec3<f32> {
  let amount             = clamp(global_light.gloom_params.x, 0.0, 1.0);
  if (amount < 1e-4) { return color_in; }
  let falloff_height     = max(global_light.gloom_params.y, 0.0);
  let shadow_bias        = clamp(global_light.gloom_params.z, 0.0, 1.0);

  // Height term: fade out over [0 .. falloff_height]
  let h = max(world_pos.y, 0.0);
  let height_term = select((1.0 - smoothstep(0.0, falloff_height, h)), 1.0, (falloff_height < 1e-6));

  // Optional shadow bias (0 = uniform, 1 = fully biased toward shadow)
  let NdotL = max(dot(normalize(N), normalize(L)), 0.0);
  let shadow_term = pow(1.0 - NdotL, 1.5); // smooth emphasis for shadowed faces
  let bias_term = mix(1.0, shadow_term, shadow_bias);

  let g = clamp(amount * height_term * bias_term, 0.0, 1.0);

  // Dedicated gloom tint: if gloom_color RGB is non-zero, use it; otherwise fall back to atmosphere tint.
  let has_custom_color = dot(global_light.gloom_color.rgb, vec3<f32>(1.0)) > 0.01;
  let tint_source = select(global_light.atmosphere_tint, global_light.gloom_color.rgb, has_custom_color);
  let gloom_tint = mix(vec3<f32>(1.0), tint_source, 0.7);

  // Desaturate in gloomy areas (KR characteristic: shadows lose saturation)
  let desat_amount = clamp(global_light.gloom_color.a, 0.0, 1.0) * g;
  let luma = luminance(color_in);
  let desaturated = mix(color_in, vec3<f32>(luma), desat_amount);

  return desaturated * mix(vec3<f32>(1.0), gloom_tint, g);
}
