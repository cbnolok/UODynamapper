// postprocess::kr_color_wash — Low-frequency KR-style painted illumination.

import package::shaders::world::land::noise::noise_2d;

// The strength slider ends at KR_COLOR_WASH_STRENGTH_MAX. Up to the knee, above the highest preset strength
// (UOD-Painterly in caves, 0.74), the wash keeps its former amount, the strength itself, so the presets look as before;
// there it shifts the colour by a few percent at most, too little to see. Past the knee a quadratic term, flat where it
// starts, raises the amount to KR_COLOR_WASH_MAX_AMOUNT at the slider's end, where the patches shift the value by up to
// KR_COLOR_WASH_MAX_VALUE_SHIFT and take the warm or the cool wash's colour in full.
const KR_COLOR_WASH_STRENGTH_MAX: f32 = 1.5;
const KR_COLOR_WASH_KNEE: f32 = 0.75;
const KR_COLOR_WASH_MAX_AMOUNT: f32 = 10.0;
// The bounds of the colour pull and of the value shift: above any the presets reach (0.11 and 1 +- 0.11), so they stay
// as they were, and wide enough for the amount past the knee.
const KR_COLOR_WASH_MAX_TINT: f32 = 1.0;
const KR_COLOR_WASH_MAX_VALUE_SHIFT: f32 = 0.4;
// The smallest patch size, in world tiles, that the noise is read at: the Scale slider's minimum.
const KR_COLOR_WASH_MIN_SCALE: f32 = 4.0;

// The amount of wash for a strength on the slider: the strength up to the knee, rising faster past it.
fn kr_color_wash_amount(strength: f32) -> f32 {
  let s = clamp(strength, 0.0, KR_COLOR_WASH_STRENGTH_MAX);
  let past_knee = max(s - KR_COLOR_WASH_KNEE, 0.0);
  let span = KR_COLOR_WASH_STRENGTH_MAX - KR_COLOR_WASH_KNEE;
  let rise = (KR_COLOR_WASH_MAX_AMOUNT - KR_COLOR_WASH_STRENGTH_MAX) / (span * span);
  return s + rise * past_knee * past_knee;
}

fn apply_kr_color_wash(
  rgb: vec3<f32>,
  world_xz: vec2<f32>,
  shadow_factor: f32,
  local_light: f32,
  enabled: u32,
  strength: f32,
  scale: f32,
  profile: u32,
) -> vec3<f32> {
  let s = kr_color_wash_amount(strength);
  if (profile != 2u || enabled != 1u || s <= 0.0001) {
    return rgb;
  }

  let wash_scale = max(scale, KR_COLOR_WASH_MIN_SCALE);
  let p = world_xz / wash_scale;
  let broad = noise_2d(p + vec2<f32>(19.0, -37.0));
  let medium = noise_2d(p * 2.45 + vec2<f32>(7.0, 11.0));
  let broken = clamp((broad - 0.5) * 0.92 + (medium - 0.5) * 0.38, -0.62, 0.62);
  let shadow = clamp(shadow_factor, 0.0, 1.0);
  let light = clamp(local_light, 0.0, 1.0);

  let warm_wash = mix(vec3<f32>(1.12, 1.04, 0.86), vec3<f32>(1.03, 1.10, 0.82), medium);
  let cool_wash = mix(vec3<f32>(0.76, 0.86, 1.08), vec3<f32>(0.70, 0.86, 0.94), broad);
  let tint_bias = clamp(shadow * 0.72 + (0.5 - broad) * 0.42 - light * 0.28, 0.0, 1.0);
  let tint = mix(warm_wash, cool_wash, tint_bias);

  let tint_amount = clamp(s * (0.045 + shadow * 0.075 + abs(broken) * 0.045), 0.0, KR_COLOR_WASH_MAX_TINT);
  let value_shift = clamp(1.0 + s * (broken * 0.11 - shadow * 0.055 + light * 0.08),
    1.0 - KR_COLOR_WASH_MAX_VALUE_SHIFT, 1.0 + KR_COLOR_WASH_MAX_VALUE_SHIFT);
  let washed = rgb * mix(vec3<f32>(1.0), tint, tint_amount) * value_shift;

  return max(washed, vec3<f32>(0.0));
}
