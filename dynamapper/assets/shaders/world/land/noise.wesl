// ============================================================================
// land::noise — Random / noise helpers used by fog and other effects.
//
//  - hash + smooth value noise
//  - FBM (fractal Brownian motion)
//  - "Billow" transform (|2n-1|) to get fluffy cloud lobes
//  - Domain warp (offset the sampling coords by a low-frequency field)
// ============================================================================


// Cheap hash → [0,1). Good enough for value noise base.
fn hash(p: vec2<f32>) -> f32 {
  let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
  let p3s = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
  return fract((p3s.x + p3s.y) * p3s.z);
}

// Smooth value noise in [0,1]
fn noise_2d(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f); // smoothstep-like fade
  let a = hash(i + vec2<f32>(0.0, 0.0));
  let b = hash(i + vec2<f32>(1.0, 0.0));
  let c = hash(i + vec2<f32>(0.0, 1.0));
  let d = hash(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 3–4 octave FBM; returns ~[0,1] range
fn fbm_value(p: vec2<f32>) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var f   = 1.0;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);
  return clamp(sum, 0.0, 1.0);
}

// Billow transform → fluffy "cloud"-like blobs (still ~[0,1])
fn fbm_billow(p: vec2<f32>) -> f32 {
  let n = fbm_value(p);
  return 1.0 - abs(2.0 * n - 1.0);
}

// Domain warp: offset p by two low-frequency FBMs to break grid patterns.
fn domain_warp(p: vec2<f32>, strength: f32) -> vec2<f32> {
  let w1 = fbm_value(p * 0.5 + vec2<f32>(13.37, -7.21));
  let w2 = fbm_value(p * 0.5 + vec2<f32>(-5.73, 4.11));
  return p + vec2<f32>(w1, w2) * strength;
}
