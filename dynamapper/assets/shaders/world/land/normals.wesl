// ============================================================================
// land::normals — Normal vector computation for the terrain surface.
//
//  Three modes, selectable at runtime via effects.normal_mode:
//   0 = Geometric  — fast central-difference from neighbor heights.
//   1 = Bicubic    — smooth analytic normal via Catmull-Rom interpolation
//                    over a 4×4 height neighborhood.
//  Optional bent normal bent against local occlusion proxy (enable_bent).
//  The clients' per-vertex terrain shade (terrain_shading_model) is at the end.
// ============================================================================


import package::shaders::world::land::atlas::{atlas_read_height, atlas_read_z};

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

// ---------------------- Client per-vertex terrain shade ----------------------
/*
 The KR and Enhanced clients bake one grey per terrain vertex, g = (N . L + 1) / 2, and Gouraud-interpolate it over
 two triangles per cell split on the (x + 1, z)-(x, z + 1) diagonal (UOKR UOTerrain::BuildTerrainMesh 0x0044b7f0).
 N is UOTerrain::ComputeVertexNormal (UOKR 0x004488a0, UOSA 0x00460c70). It reads the vertex and its north (z - 1),
 west (x - 1) and north-west neighbours, takes the plane normal (-dh/dx, 1, -dh/dz) of each of the three triangles
 they form, normalizes the first and folds the others in as normalize(previous + next). Heights are in the
 client's units, where a cell edge is 1.

 Our mesh splits cells on the other diagonal and merges flat cells into LOD quads, so the fragment rebuilds the
 four corner greys of its cell from the 3 x 3 heights around it and interpolates them over the client's triangles.
*/

// One UO z unit in the client's terrain space, where a cell edge is 1 (UOKR 0x00b86f38: 4 * sqrt(2) / 44).
const CLIENT_TERRAIN_Z_UNIT: f32 = 0.12856454;

fn client_height(world_x: i32, world_z: i32) -> f32 {
  return f32(atlas_read_z(world_x, world_z)) * CLIENT_TERRAIN_Z_UNIT;
}

// The client's normal at a vertex of height c with north neighbour n, west neighbour w and north-west neighbour nw.
fn client_vertex_normal(c: f32, n: f32, w: f32, nw: f32) -> vec3<f32> {
  // Triangle vertex, north, west.
  var normal = normalize(vec3<f32>(w - c, 1.0, n - c));
  // Triangle vertex, north, north-west.
  normal = normalize(normal + vec3<f32>(nw - n, 1.0, n - c));
  // Triangle vertex, north-west, west.
  return normalize(normal + vec3<f32>(w - c, 1.0, nw - w));
}

// Half-Lambert: N . L from [-1, 1] to [0, 1].
fn client_vertex_grey(c: f32, n: f32, w: f32, nw: f32, L: vec3<f32>) -> f32 {
  return (dot(client_vertex_normal(c, n, w, nw), L) + 1.0) * 0.5;
}

// The client's terrain grey at a world position, for a light direction L (pointing towards the light).
fn client_vertex_terrain_shade(world_pos: vec3<f32>, L: vec3<f32>) -> f32 {
  let cell = floor(world_pos.xz);
  let f = world_pos.xz - cell;
  let x = i32(cell.x);
  let z = i32(cell.y);
  // h_<x offset><z offset>, m = -1, 0, p = +1.
  let h_mm = client_height(x - 1, z - 1);
  let h_0m = client_height(x, z - 1);
  let h_pm = client_height(x + 1, z - 1);
  let h_m0 = client_height(x - 1, z);
  let h_00 = client_height(x, z);
  let h_p0 = client_height(x + 1, z);
  let h_mp = client_height(x - 1, z + 1);
  let h_0p = client_height(x, z + 1);
  let h_pp = client_height(x + 1, z + 1);
  // Corner greys: g_<x corner><z corner>.
  let g_00 = client_vertex_grey(h_00, h_0m, h_m0, h_mm, L);
  let g_10 = client_vertex_grey(h_p0, h_pm, h_00, h_0m, L);
  let g_01 = client_vertex_grey(h_0p, h_00, h_mp, h_m0, L);
  let g_11 = client_vertex_grey(h_pp, h_p0, h_0p, h_00, L);
  if (f.x + f.y <= 1.0) {
    // Triangle (x, z), (x, z + 1), (x + 1, z).
    return g_00 + f.x * (g_10 - g_00) + f.y * (g_01 - g_00);
  }
  // Triangle (x + 1, z), (x, z + 1), (x + 1, z + 1).
  return g_11 + (1.0 - f.x) * (g_01 - g_11) + (1.0 - f.y) * (g_10 - g_11);
}
