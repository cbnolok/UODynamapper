// ============================================================================
// land::lighting — Lighting math helpers shared across all shading modes.
//
//  Includes: Lambert, Blinn-Phong specular, rim, hemisphere fill,
//  contrast S-curve, vibrant color grading, gloom, Reinhard tonemap.
// ============================================================================


#import "shaders/worldmap/land/bindings.wgsl"::{GlobalLightingUniforms, LandLightingUniforms, global_light, land_light}

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

// Contrast S-curve with neutral at contrast=1.0 (k = contrast-1 in [-1,1]).
// NOTE: Do NOT clamp here — keep HDR headroom pre-tonemap.
fn apply_contrast_neutral(x: vec3<f32>, contrast: f32) -> vec3<f32> {
  let t = clamp(contrast - 1.0, -1.0, 1.0);
  // y = x + t * (x - x*x) * 2  (S-curve around ~0.5; zero effect when t=0)
  return x + t * ((x - x * x) * 2.0);
}

// Strong, stylized color grading (vibrant), with:
//  - truly neutral contrast=1 (no clamp inside),
//  - multiplicative split-toning normalized to luma=1 so mid-grays stay neutral.
fn grade_color_vibrant(color_in: vec3<f32>) -> vec3<f32> {
  let strength    = global_light.grade_params.x;  // overall grade amount
  let vibrance    = global_light.grade_extra.x;   // selective saturation
  let saturation  = global_light.grade_extra.y;   // global saturation
  let contrast    = global_light.grade_extra.z;   // S-curve; 1.0 = neutral
  let split_str   = global_light.grade_extra.w;   // split-toning strength

  // Global saturation around luminance pivot
  let l = luminance(color_in);
  let sat_col = mix(vec3<f32>(l), color_in, saturation);

  // Vibrance: boost low-sat regions more (mask stronger for low chroma)
  let chroma = sat_col - vec3<f32>(l);
  let sat_mag = max(max(abs(chroma.r), abs(chroma.g)), abs(chroma.b));
  let vib_mask = smoothstep(0.0, 0.7, 1.0 - sat_mag);
  let vib_col = sat_col + chroma * (vibrance * vib_mask);

  // Contrast (neutral at 1.0), keep HDR — no clamp here
  let ctr_col = apply_contrast_neutral(vib_col, contrast);

  // Split-toning by luminance: cool lows, warm highs — multiplicative, luma-normalized
  let warm = global_light.grade_warm_color.rgb;
  let cool = global_light.grade_cool_color.rgb;
  let wmix = smoothstep(0.25, 0.85, l);
  let tint = mix(cool, warm, wmix);

  // Normalize tint so its luma is ~1 → keeps mid-gray unchanged when applied multiplicatively
  let tint_luma = max(luminance(tint), 1e-6);
  let tint_norm = tint / tint_luma;
  let split_mult = mix(vec3<f32>(1.0), tint_norm, split_str);

  // Apply split toning multiplicatively, then blend with original by overall strength
  let graded_hq = ctr_col * split_mult;
  let graded = mix(color_in, graded_hq, clamp(strength, 0.0, 2.0));
  return max(graded, vec3<f32>(0.0));
}

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

  // Cool, moody tint from ambient color; multiplicative keeps hues intact
  let gloom_tint = mix(vec3<f32>(1.0), global_light.ambient_color, 0.7);
  return color_in * mix(vec3<f32>(1.0), gloom_tint, g);
}

// Tonemap: Reinhard with configurable exposure
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}
