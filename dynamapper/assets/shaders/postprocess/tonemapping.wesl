// ============================================================================
// postprocess::tonemapping — Shared tonemapping helpers.
// ============================================================================

// Reinhard tonemap with configurable exposure.
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}

// GlobalLightingUniforms.tone_curve values (TONE_CURVE_* in mesh_material.rs).
const TONE_CURVE_PROFILE: u32 = 0u;
const TONE_CURVE_IDENTITY: u32 = 1u;

// The tone stage enable_tonemap switches on. The profile curve applies exposure inside it; the identity
// curve applies it as plain brightness and clamps like the clients' 8-bit target.
fn tonemap_with_curve(c: vec3<f32>, exposure: f32, profile: u32, tone_curve: u32) -> vec3<f32> {
  if (tone_curve == TONE_CURVE_IDENTITY) {
    return clamp(c * max(exposure, 0.0), vec3<f32>(0.0), vec3<f32>(1.0));
  }
  return tonemap_ec_kr_profile(c, exposure, profile);
}

fn tonemap_ec_kr_profile(c: vec3<f32>, exposure: f32, profile: u32) -> vec3<f32> {
  if (profile == 0u) {
    return tonemap_reinhard_with_exposure(c, exposure);
  }

  let middle_gray = 0.18;
  // EC keeps its derived pre-scale. KR has none: its former separate gain
  // (kr_tonemap_params.x) multiplied exactly like exposure, so the presets fold
  // it into their exposure and exposure alone sets KR's brightness.
  var pre_gain = middle_gray / (0.18 + 0.001);
  var white_cutoff = 1.6;
  if (profile == 2u) {
    pre_gain = 1.0;
    white_cutoff = 0.8;
  }

  var out = c * max(exposure, 1e-6) * pre_gain;
  out *= (vec3<f32>(1.0) + (out / (white_cutoff * white_cutoff)));
  out /= (vec3<f32>(1.0) + out);
  return out;
}
