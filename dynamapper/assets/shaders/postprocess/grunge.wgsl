// ============================================================================
// postprocess::grunge — Procedural world-space weathering for EC/KR surfaces.
// ============================================================================

fn grunge_hash(p: vec2<f32>) -> f32 {
  let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
  let p3s = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
  return fract((p3s.x + p3s.y) * p3s.z);
}

fn grunge_noise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = grunge_hash(i + vec2<f32>(0.0, 0.0));
  let b = grunge_hash(i + vec2<f32>(1.0, 0.0));
  let c = grunge_hash(i + vec2<f32>(0.0, 1.0));
  let d = grunge_hash(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn grunge_fbm(p: vec2<f32>) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var f = 1.0;
  sum += amp * grunge_noise(p * f); f *= 2.0; amp *= 0.5;
  sum += amp * grunge_noise(p * f); f *= 2.0; amp *= 0.5;
  sum += amp * grunge_noise(p * f); f *= 2.0; amp *= 0.5;
  sum += amp * grunge_noise(p * f);
  return clamp(sum, 0.0, 1.0);
}

fn apply_visual_grunge(rgb: vec3<f32>, world_xz: vec2<f32>, strength: f32, profile: u32) -> vec3<f32> {
  let s = clamp(strength, 0.0, 1.0);
  if (s <= 0.0001) {
    return rgb;
  }

  let is_kr = profile == 2u;
  let scale = select(0.035, 0.055, is_kr);
  let p = world_xz * scale + select(vec2<f32>(17.0, -11.0), vec2<f32>(41.0, 23.0), is_kr);
  let coarse = grunge_fbm(p);
  let fine = grunge_fbm(p * 3.7 + vec2<f32>(9.2, -4.6));
  let n = mix(coarse, fine, select(0.18, 0.35, is_kr));

  let dark = select(0.76, 0.48, is_kr);
  let light = select(1.08, 1.16, is_kr);
  let factor = mix(light, dark, smoothstep(0.18, 0.92, n));
  var tint = vec3<f32>(factor);
  if (is_kr) {
    tint = vec3<f32>(factor * 0.94, factor * 0.97, factor * 1.04);
  }

  let profile_gain = select(0.70, 1.35, is_kr);
  return rgb * mix(vec3<f32>(1.0), tint, s * profile_gain);
}

fn apply_shadow_aware_land_grunge(
  rgb: vec3<f32>,
  world_xz: vec2<f32>,
  strength: f32,
  profile: u32,
  shadow_factor: f32,
) -> vec3<f32> {
  if (profile != 2u) {
    return apply_visual_grunge(rgb, world_xz, strength, profile);
  }

  let s = clamp(strength, 0.0, 1.0);
  if (s <= 0.0001) {
    return rgb;
  }

  let p = world_xz * 0.055 + vec2<f32>(41.0, 23.0);
  let coarse = grunge_fbm(p);
  let fine = grunge_fbm(p * 3.7 + vec2<f32>(9.2, -4.6));
  let n = mix(coarse, fine, 0.35);
  let shadow = clamp(shadow_factor, 0.0, 1.0);

  let deposit = smoothstep(0.16, 0.82, n) * (0.45 + shadow * 0.85);
  let highlight_protection = 1.0 - shadow * 0.35;
  let grime_dark = vec3<f32>(0.46, 0.50, 0.58);
  let dry_light = vec3<f32>(1.14, 1.10, 1.03);
  let weather_tint = mix(dry_light, grime_dark, deposit);

  return rgb * mix(vec3<f32>(1.0), weather_tint, min(s * 1.25 * highlight_protection, 1.0));
}
