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
  if (s <= 0.0001 || profile == 0u) {
    return rgb;
  }

  let is_kr = profile == 2u;
  let scale = select(0.035, 0.055, is_kr);
  let p = world_xz * scale + select(vec2<f32>(17.0, -11.0), vec2<f32>(41.0, 23.0), is_kr);
  let coarse = grunge_fbm(p);
  let fine = grunge_fbm(p * 3.7 + vec2<f32>(9.2, -4.6));
  let n = mix(coarse, fine, select(0.18, 0.35, is_kr));

  let dark = select(0.84, 0.64, is_kr);
  let light = select(1.04, 1.10, is_kr);
  let factor = mix(light, dark, smoothstep(0.18, 0.92, n));
  var tint = vec3<f32>(factor);
  if (is_kr) {
    tint = vec3<f32>(factor * 0.94, factor * 0.97, factor * 1.04);
  }

  let profile_gain = select(0.40, 1.0, is_kr);
  return rgb * mix(vec3<f32>(1.0), tint, s * profile_gain);
}
