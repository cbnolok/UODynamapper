// ============================================================================
// postprocess::color_grading — Shared scene color grading helpers.
// ============================================================================

#import "shaders/world/common_bindings.wgsl"::{GlobalLightingUniforms}

fn color_grading_luminance(c: vec3<f32>) -> f32 {
  return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Contrast S-curve with neutral at contrast=1.0 (k = contrast-1 in [-1,1]).
// Keep HDR headroom for the subsequent tonemap.
fn apply_contrast_neutral(x: vec3<f32>, contrast: f32) -> vec3<f32> {
  let t = clamp(contrast - 1.0, -1.0, 1.0);
  return x + t * ((x - x * x) * 2.0);
}

fn grade_color_vibrant(color_in: vec3<f32>, global_light: GlobalLightingUniforms) -> vec3<f32> {
  let strength    = global_light.grade_params.x;
  let vibrance    = global_light.grade_extra.x;
  let saturation  = global_light.grade_extra.y;
  let contrast    = global_light.grade_extra.z;
  let split_str   = global_light.grade_extra.w;

  let l = color_grading_luminance(color_in);
  let sat_col = mix(vec3<f32>(l), color_in, saturation);

  let chroma = sat_col - vec3<f32>(l);
  let sat_mag = max(max(abs(chroma.r), abs(chroma.g)), abs(chroma.b));
  let vib_mask = smoothstep(0.0, 0.7, 1.0 - sat_mag);
  let vib_col = sat_col + chroma * (vibrance * vib_mask);

  let ctr_col = apply_contrast_neutral(vib_col, contrast);

  let warm = global_light.grade_warm_color.rgb;
  let cool = global_light.grade_cool_color.rgb;
  let wmix = smoothstep(0.25, 0.85, l);
  let tint = mix(cool, warm, wmix);

  let tint_luma = max(color_grading_luminance(tint), 1e-6);
  let tint_norm = tint / tint_luma;
  let split_mult = mix(vec3<f32>(1.0), tint_norm, split_str);

  let graded_hq = ctr_col * split_mult;
  let graded = mix(color_in, graded_hq, clamp(strength, 0.0, 2.0));
  return max(graded, vec3<f32>(0.0));
}
