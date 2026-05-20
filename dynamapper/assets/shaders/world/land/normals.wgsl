// ============================================================================
// land::normals — Normal vector computation for the terrain surface.
//
//  Three modes, selectable at runtime via effects.normal_mode:
//   0 = Geometric  — fast central-difference from neighbor heights.
//   1 = Bicubic    — smooth analytic normal via Catmull-Rom interpolation
//                    over a 4×4 height neighborhood.
//  Optional bent normal bent against local occlusion proxy (enable_bent).
// ============================================================================


#import "shaders/world/land/atlas.wgsl"::{atlas_read_height}

// ============================================================================
// Cubic interpolation helpers (value + derivative) for heightfield normals
// ============================================================================

// Catmull-Rom-like cubic — returns (value, derivative) at parameter t in [0,1].
// p0..p3 are the four control heights along one axis.
fn cubic_interp_value_and_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> vec2<f32> {
  let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
  let b =        p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
  let c = -0.5 * p0            + 0.5 * p2;
  let d =                   p1;
  let value = ((a * t + b) * t + c) * t + d;
  let deriv = (3.0 * a * t * t) + (2.0 * b * t) + c;
  return vec2<f32>(value, deriv);
}

fn cubic_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
  return cubic_interp_value_and_derivative(p0, p1, p2, p3, t).x;
}

// ============================================================================
// Normal calculation
// ============================================================================

// Fast central-difference normal from 4 immediate neighbors.
fn get_geometric_normal_local(world_pos: vec3<f32>) -> vec3<f32> {
  let wx = i32(round(world_pos.x));
  let wz = i32(round(world_pos.z));
  let hL = atlas_read_height(wx - 1, wz);
  let hR = atlas_read_height(wx + 1, wz);
  let hD = atlas_read_height(wx, wz - 1);
  let hU = atlas_read_height(wx, wz + 1);
  let dHdx = 0.5 * (hR - hL);
  let dHdz = 0.5 * (hU - hD);
  return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// Smooth analytic normal via bicubic interpolation of the surrounding
// 4×4 tile heights. Greatly reduces shading "jaggies" vs the geometric normal.
fn get_bicubic_normal(world_pos: vec3<f32>) -> vec3<f32> {
  let base_x = floor(world_pos.x);
  let base_z = floor(world_pos.z);
  let frac_x = world_pos.x - base_x;
  let frac_z = world_pos.z - base_z;

  let ix = i32(base_x);
  let iz = i32(base_z);

  let h00 = atlas_read_height(ix - 1, iz - 1);
  let h10 = atlas_read_height(ix + 0, iz - 1);
  let h20 = atlas_read_height(ix + 1, iz - 1);
  let h30 = atlas_read_height(ix + 2, iz - 1);

  let h01 = atlas_read_height(ix - 1, iz + 0);
  let h11 = atlas_read_height(ix + 0, iz + 0);
  let h21 = atlas_read_height(ix + 1, iz + 0);
  let h31 = atlas_read_height(ix + 2, iz + 0);

  let h02 = atlas_read_height(ix - 1, iz + 1);
  let h12 = atlas_read_height(ix + 0, iz + 1);
  let h22 = atlas_read_height(ix + 1, iz + 1);
  let h32 = atlas_read_height(ix + 2, iz + 1);

  let h03 = atlas_read_height(ix - 1, iz + 2);
  let h13 = atlas_read_height(ix + 0, iz + 2);
  let h23 = atlas_read_height(ix + 1, iz + 2);
  let h33 = atlas_read_height(ix + 2, iz + 2);

  // dH/dx: interpolate row-derivatives along Z
  let row0 = cubic_interp_value_and_derivative(h00, h10, h20, h30, frac_x);
  let row1 = cubic_interp_value_and_derivative(h01, h11, h21, h31, frac_x);
  let row2 = cubic_interp_value_and_derivative(h02, h12, h22, h32, frac_x);
  let row3 = cubic_interp_value_and_derivative(h03, h13, h23, h33, frac_x);

  let dHdx = cubic_value(row0.y, row1.y, row2.y, row3.y, frac_z);

  // dH/dz: interpolate column-derivatives along X
  let col0 = cubic_interp_value_and_derivative(h00, h01, h02, h03, frac_z);
  let col1 = cubic_interp_value_and_derivative(h10, h11, h12, h13, frac_z);
  let col2 = cubic_interp_value_and_derivative(h20, h21, h22, h23, frac_z);
  let col3 = cubic_interp_value_and_derivative(h30, h31, h32, h33, frac_z);
  let dHdz = cubic_value(col0.y, col1.y, col2.y, col3.y, frac_x);

  return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// ------------------------------- Bent normals --------------------------------
/*
 Bends the surface normal toward the up-vector when neighbor tiles are
 significantly higher, approximating a cheap per-vertex AO / occlusion proxy.

 OLD (for reference):
   - Looked at *positive* height steps around center and *summed* them.
   - Mix factor = (sum of positive neighbor deltas) * 0.25 → could flip
     direction depending on which side had the larger positive step.
   - Result: on steep, step-like terrain, adjacent triangles sometimes "chose"
     different dominant neighbors → visible zig-zag in shading.

  let pos_slopes = max(0.0, hl - hc) + max(0.0, hr - hc) + max(0.0, hd - hc) + max(0.0, hu - hc);
  let occl = clamp(pos_slopes * 0.25, 0.0, 1.0);
  let mix_factor = occl * 0.5;

 NEW (below):
   - Uses only the single *maximum* neighbor over-height relative to center.
   - That makes the occlusion proxy monotonic and stable (no left/right flip).
   - Softens with smoothstep and keeps the bend conservative.
   - Same function is reused in BOTH fragment and vertex/Gouraud paths.
*/
fn get_bent_normal(world_pos: vec3<f32>, base_normal_world: vec3<f32>) -> vec3<f32> {
  let cx = i32(floor(world_pos.x));
  let cz = i32(floor(world_pos.z));

  let hc = atlas_read_height(cx, cz);
  let hl = atlas_read_height(cx - 1, cz);
  let hr = atlas_read_height(cx + 1, cz);
  let hd = atlas_read_height(cx, cz - 1);
  let hu = atlas_read_height(cx, cz + 1);

  // Use only the *max* positive step: stable across ridges.
  let hmax = max(max(hl, hr), max(hd, hu));
  let occl = max(hmax - hc, 0.0);        // how much neighbors overshadow center
  let k    = smoothstep(0.0, 1.5, occl); // soften response
  let mix_factor = k * 0.45;             // conservative bend to avoid "melting"

  return normalize(mix(base_normal_world, vec3<f32>(0.0, 1.0, 0.0), mix_factor));
}
