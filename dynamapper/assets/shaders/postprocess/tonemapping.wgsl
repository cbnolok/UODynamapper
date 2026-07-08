// ============================================================================
// postprocess::tonemapping — Shared tonemapping helpers.
// ============================================================================

// Reinhard tonemap with configurable exposure.
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}

fn tonemap_ec_kr_profile(c: vec3<f32>, exposure: f32, profile: u32, kr_tonemap_gain: f32) -> vec3<f32> {
  if (profile == 0u) {
    return tonemap_reinhard_with_exposure(c, exposure);
  }

  let middle_gray = 0.18;
  // EC keeps its derived pre-scale. KR uses the exposed, tunable gain, falling
  // back to the legacy ~middle_gray/2.0 crush value when it is unset (<= 0, e.g.
  // an uninitialized Default uniform) so KR never renders black before sync.
  var pre_gain = middle_gray / (0.18 + 0.001);
  var white_cutoff = 1.6;
  if (profile == 2u) {
    pre_gain = kr_tonemap_gain;
    if (kr_tonemap_gain <= 0.0) {
      pre_gain = 0.09;
    }
    white_cutoff = 0.8;
  }

  var out = c * max(exposure, 1e-6) * pre_gain;
  out *= (vec3<f32>(1.0) + (out / (white_cutoff * white_cutoff)));
  out /= (vec3<f32>(1.0) + out);
  return out;
}
