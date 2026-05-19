// ============================================================================
// postprocess::tonemapping — Shared tonemapping helpers.
// ============================================================================

// Reinhard tonemap with configurable exposure.
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}

fn tonemap_ec_kr_profile(c: vec3<f32>, exposure: f32, profile: u32) -> vec3<f32> {
  if (profile == 0u) {
    return tonemap_reinhard_with_exposure(c, exposure);
  }

  let luminance = select(0.18, 2.0, profile == 2u);
  let middle_gray = 0.18;
  let white_cutoff = select(1.6, 0.8, profile == 2u);

  var out = c * max(exposure, 1e-6) * (middle_gray / (luminance + 0.001));
  out *= (vec3<f32>(1.0) + (out / (white_cutoff * white_cutoff)));
  out /= (vec3<f32>(1.0) + out);
  return out;
}
