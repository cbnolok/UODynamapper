// ============================================================================
// land::lighting — Lighting math helpers shared across all shading modes.
//
//  Includes: Lambert, Blinn-Phong specular, rim, hemisphere fill, gloom.
// ============================================================================


#import "shaders/world/common_bindings.wgsl"::{GlobalLightingUniforms}
#import "shaders/world/land/land_bindings.wgsl"::{LandLightingUniforms, global_light, land_light}

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

// Rim / fresnel-like factor (silhouette brightening)
fn get_rim(N: vec3<f32>, V: vec3<f32>, power: f32) -> f32 {
  let rim_dot = 1.0 - max(dot(normalize(N), normalize(V)), 0.0);
  return pow(rim_dot, power);
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
// gloom_params: [amount, height_falloff_height, shadow_bias, fog_height_bias]
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
