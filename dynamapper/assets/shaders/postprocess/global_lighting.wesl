// ============================================================================
// postprocess::global_lighting — Shared scene-wide lighting helpers.
// ============================================================================

fn apply_global_lighting_rgb(rgb: vec3<f32>, global_lighting: f32) -> vec3<f32> {
  return rgb * max(global_lighting, 0.0);
}

