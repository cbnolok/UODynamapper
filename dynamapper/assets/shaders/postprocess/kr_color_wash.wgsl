// postprocess::kr_color_wash — Low-frequency KR-style painted illumination.

#import "shaders/world/land/noise.wgsl"::noise_2d

fn apply_kr_color_wash(
  rgb: vec3<f32>,
  world_xz: vec2<f32>,
  shadow_factor: f32,
  local_light: f32,
  enable: u32,
  strength: f32,
  scale: f32,
  profile: u32,
) -> vec3<f32> {
  let s = clamp(strength, 0.0, 1.5);
  if (profile != 2u || enable != 1u || s <= 0.0001) {
    return rgb;
  }

  let wash_scale = max(scale, 8.0);
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

  let tint_amount = clamp(s * (0.045 + shadow * 0.075 + abs(broken) * 0.045), 0.0, 0.18);
  let value_shift = clamp(1.0 + s * (broken * 0.11 - shadow * 0.055 + light * 0.08), 0.84, 1.16);
  let washed = rgb * mix(vec3<f32>(1.0), tint, tint_amount) * value_shift;

  return max(washed, vec3<f32>(0.0));
}
