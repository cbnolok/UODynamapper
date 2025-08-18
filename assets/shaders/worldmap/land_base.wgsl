#import bevy_pbr::{
  forward_io::{Vertex, VertexOutput},
  mesh_functions,
  view_transformations,
}

// ============================================================================
// Compile-time configuration (development)
// ============================================================================

// Unified dev toggles: when DEV_USE_UNIFORMS == 0, the DEV_* defaults in this
// file are used; otherwise, CPU-provided uniforms drive the look.
const DEV_USE_UNIFORMS: u32 = 0u;

// Shading modes: 0 = Vertex/Gouraud (fast), 1 = Fragment (KR look)
const DEV_SHADING_MODE: u32 = 1u;

// Normal modes: 0 = geometric (quick), 1 = bicubic analytic (smoother)
const DEV_NORMAL_MODE:  u32 = 1u;

// Feature toggles
const DEV_ENABLE_BENT:     u32 = 1u; // bent normal sky bias in crevices
const DEV_ENABLE_TONEMAP:  u32 = 1u;
const DEV_ENABLE_GRADING:  u32 = 0u;
const DEV_ENABLE_FOG:      u32 = 0u;

// Artist-default intensities (used when DEV_USE_UNIFORMS == 0)
const DEV_AMBIENT:  f32 = 0.15; // cool base light in shadowed areas
const DEV_DIFFUSE:  f32 = 0.15; // strength of direct sunlight
const DEV_SPECULAR: f32 = 0.06; // subtle sun glint
const DEV_RIM:      f32 = 0.005; // silhouette lift
const DEV_FILL:     f32 = 0.1; // sky/ground environmental tint
const DEV_EXPOSURE: f32 = 1.05; // HDR exposure for tonemapping

// “Look” defaults for dev mode (warm sun, cool ambient)
const DEV_LIGHT_DIR:    vec3<f32> = vec3<f32>(-0.431922, -0.863845, -0.259153);
const DEV_LIGHT_COLOR:  vec3<f32> = vec3<f32>(1.03, 0.99, 0.92); // gently warm
const DEV_AMBIENT_COLOR:vec3<f32> = vec3<f32>(0.18, 0.22, 0.28); // cool

// Compile-time gate for headroom limiting (prevents color bleaching).
// The runtime on/off toggle lives in lighting.grade_params.w (see below).
const CFG_ENABLE_HEADROOM_LIMIT: bool = true;

// ============================================================================
// Bindings / Uniform Layouts (unchanged; do not reorder fields)
// ============================================================================

struct TileUniform {
  tile_height:   f32,
  texture_size:  u32, // 0=small atlas, 1=big atlas
  texture_layer: u32,
  texture_hue:   u32,
};

struct LandUniform {
  _light_dir:   vec3<f32>, // legacy; prefer scene.light_direction
  _pad0:        f32,
  chunk_origin: vec2<f32>, // world origin of chunk (x,z) in tile units
  _pad1:        vec2<f32>,
  tiles:        array<TileUniform, 169>, // 13×13 grid (8×8 core + 2 border)
};

struct SceneUniform {
  camera_position: vec3<f32>,
  time_seconds: f32,
  light_direction: vec3<f32>, // expected normalized by CPU
  _pad1: f32,
  // Fog (multiplicative tint; optional)
  fog_color: vec4<f32>,
  fog_params: vec4<f32>, // x=strength, y=scale, z=speed_x, w=speed_y
};

struct TunablesUniform {
  // Modes / toggles
  shading_mode:    u32,
  normal_mode:     u32,
  enable_bent:     u32,
  enable_tonemap:  u32,
  enable_grading:  u32,
  enable_fog:      u32,
  _pad1:           vec2<u32>,

  // Intensities
  ambient_strength: f32, // ambient (shadow light) — scalar
  diffuse_strength: f32, // diffuse (sun) — scalar
  specular_strength:f32, // specular highlight — scalar
  rim_strength:     f32, // rim/silhouette — scalar

  // Optional diffuse shaping
  sharpness_factor: f32, // raises lambert^factor to tighten lobes
  sharpness_mix:    f32, // 0=Lambert, 1=sharpened
  _pad2:            vec2<f32>,
};

// Lighting and palette (unchanged layout). We reuse grade_params.yzw
// for “look” tunables to avoid changing the ABI:
//   grade_params.x = grading strength (existing)
//   grade_params.y = headroom_reserve (0..1)
//   grade_params.z = hemi_chroma_tint (0..1)
//   grade_params.w = enable_headroom_limit (0=off, 1=on)
struct LightingUniforms {
  light_dir: vec3<f32>,  // retained for ABI; prefer scene.light_direction
  _pad1: f32,
  light_color: vec3<f32>,    // key light color (sun)
  _pad2: f32,
  ambient_color: vec3<f32>,  // ambient color (cool)
  _pad3: f32,
  fill_dir: vec3<f32>,       // retained for ABI
  fill_strength: f32,        // base fill scalar
  exposure: f32,             // HDR exposure (used if tonemap enabled)
  gamma: f32,                // kept for ABI; not used (textures are sRGB)
  _pad4: vec2<f32>,

  fill_sky_color: vec4<f32>,     // rgb sky tint, a = per-color strength
  fill_ground_color: vec4<f32>,  // rgb ground tint, a = per-color strength
  rim_color: vec4<f32>,          // rgb rim tint, a = rim “power” (2..4 typical)
  grade_warm_color: vec4<f32>,   // color grading (midtones)
  grade_cool_color: vec4<f32>,   // color grading (shadows)
  grade_params: vec4<f32>,       // x=grade_strength, y=headroom_reserve, z=hemi_chroma_tint, w=headroom_on
};

@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small: texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big:   texture_2d_array<f32>;
@group(2) @binding(103) var<uniform> land:    LandUniform;
@group(2) @binding(104) var<uniform> scene:   SceneUniform;
@group(2) @binding(105) var<uniform> tunables: TunablesUniform;
@group(2) @binding(106) var<uniform> lighting: LightingUniforms;

// ============================================================================
// Grid helpers & utilities
// ============================================================================

const CHUNK_TILE_NUM_1D: u32 = 8u;  // 8×8 tiles per chunk
const DATA_GRID_BORDER:  i32 = 2;   // two‑tile safety border on each side
const DATA_GRID_SIDE:    i32 = 13;  // 8 + 2 + 2 + 1
const MESH_GRID_SIDE:    u32 = 9u;  // vertex grid is 9×9 (for 8×8 quads)

// Clamp safe index into the 13×13 “data grid”
fn tile_index_clamped(ix: i32, iz: i32) -> u32 {
  let gx = clamp(ix + DATA_GRID_BORDER, 0, DATA_GRID_SIDE - 1);
  let gz = clamp(iz + DATA_GRID_BORDER, 0, DATA_GRID_SIDE - 1);
  return u32(gz * DATA_GRID_SIDE + gx);
}
fn tile_at_13x13(ix: i32, iz: i32) -> TileUniform {
  return land.tiles[tile_index_clamped(ix, iz)];
}
fn tile_height_at_13x13(ix: i32, iz: i32) -> f32 {
  return tile_at_13x13(ix, iz).tile_height;
}

// Near the chunk edge, blend normals toward the original to hide seams.
fn chunk_edge_blend_factor(local_x: f32, local_z: f32) -> f32 {
  let tx = floor(local_x);
  let tz = floor(local_z);
  let dx = min(tx, f32(CHUNK_TILE_NUM_1D - 1u) - tx);
  let dz = min(tz, f32(CHUNK_TILE_NUM_1D - 1u) - tz);
  let min_dist = min(dx, dz);
  return 1.0 - smoothstep(0.0, 2.0, min_dist);
}

// Simple value noise used for animated, multiplicative fog tint.
fn hash(p: vec2<f32>) -> f32 {
  let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
  let p3s = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
  return fract((p3s.x + p3s.y) * p3s.z);
}
fn noise_2d(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = hash(i + vec2<f32>(0.0, 0.0));
  let b = hash(i + vec2<f32>(1.0, 0.0));
  let c = hash(i + vec2<f32>(0.0, 1.0));
  let d = hash(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// ============================================================================
// Cubic interpolation (value + derivative) for heightfield normals
// ============================================================================

fn cubic_interp_value_and_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> vec2<f32> {
  // Catmull-Rom–like cubic: returns (value, derivative) at t in [0,1]
  let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
  let b =        p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
  let c = -0.5 * p0            + 0.5 * p2;
  let d =                   p1;
  let value = ((a * t + b) * t + c) * t + d;
  let deriv = (3.0 * a * t * t) + (2.0 * b * t) + c;
  return vec2<f32>(value, deriv);
}
fn cubic_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
  return cubic_interp_value_and_derivative(p0,p1,p2,p3,t).x;
}
fn cubic_deriv(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
  return cubic_interp_value_and_derivative(p0,p1,p2,p3,t).y;
}

// ============================================================================
// Normal utilities
// ============================================================================

// 1) Geometric normal from the 13×13 grid by central differences (fast).
fn get_geometric_normal_local(node_x: i32, node_z: i32) -> vec3<f32> {
  let hL = tile_height_at_13x13(node_x - 1, node_z);
  let hR = tile_height_at_13x13(node_x + 1, node_z);
  let hD = tile_height_at_13x13(node_x, node_z - 1);
  let hU = tile_height_at_13x13(node_x, node_z + 1);
  let dHdx = 0.5 * (hR - hL);
  let dHdz = 0.5 * (hU - hD);
  return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// 2) Bicubic analytic normal from a 4×4 patch of heights centered on the cell.
fn get_bicubic_normal(world_pos: vec3<f32>) -> vec3<f32> {
    // local coords in tile units relative to chunk origin (x,z)
    let local_x = world_pos.x - land.chunk_origin.x;
    let local_z = world_pos.z - land.chunk_origin.y;

    let base_x = floor(local_x);
    let base_z = floor(local_z);
    let frac_x = local_x - base_x;
    let frac_z = local_z - base_z;

    let ix = i32(base_x);
    let iz = i32(base_z);

    // fetch 4x4 patch
    let h00 = tile_height_at_13x13(ix - 1, iz - 1);
    let h10 = tile_height_at_13x13(ix + 0, iz - 1);
    let h20 = tile_height_at_13x13(ix + 1, iz - 1);
    let h30 = tile_height_at_13x13(ix + 2, iz - 1);

    let h01 = tile_height_at_13x13(ix - 1, iz + 0);
    let h11 = tile_height_at_13x13(ix + 0, iz + 0);
    let h21 = tile_height_at_13x13(ix + 1, iz + 0);
    let h31 = tile_height_at_13x13(ix + 2, iz + 0);

    let h02 = tile_height_at_13x13(ix - 1, iz + 1);
    let h12 = tile_height_at_13x13(ix + 0, iz + 1);
    let h22 = tile_height_at_13x13(ix + 1, iz + 1);
    let h32 = tile_height_at_13x13(ix + 2, iz + 1);

    let h03 = tile_height_at_13x13(ix - 1, iz + 2);
    let h13 = tile_height_at_13x13(ix + 0, iz + 2);
    let h23 = tile_height_at_13x13(ix + 1, iz + 2);
    let h33 = tile_height_at_13x13(ix + 2, iz + 2);

    // Interpolate rows (x) -> values and d/dx
    let row0 = cubic_interp_value_and_derivative(h00, h10, h20, h30, frac_x);
    let row1 = cubic_interp_value_and_derivative(h01, h11, h21, h31, frac_x);
    let row2 = cubic_interp_value_and_derivative(h02, h12, h22, h32, frac_x);
    let row3 = cubic_interp_value_and_derivative(h03, h13, h23, h33, frac_x);

    // dH/dx = cubic interp of the row derivatives (interpolated along z)
    let dHdx = cubic_value(row0.y, row1.y, row2.y, row3.y, frac_z);

    // dH/dz: interpolate columns
    let col0 = cubic_interp_value_and_derivative(h00, h01, h02, h03, frac_z);
    let col1 = cubic_interp_value_and_derivative(h10, h11, h12, h13, frac_z);
    let col2 = cubic_interp_value_and_derivative(h20, h21, h22, h23, frac_z);
    let col3 = cubic_interp_value_and_derivative(h30, h31, h32, h33, frac_z);
    let dHdz = cubic_value(col0.y, col1.y, col2.y, col3.y, frac_x);

    // Heightfield normal
    return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// 3) Bent normal: gently bias normal toward “up” in concavities to fake sky GI.
fn get_bent_normal(world_pos: vec3<f32>, base_normal_world: vec3<f32>) -> vec3<f32> {
  let local_x = world_pos.x - land.chunk_origin.x;
  let local_z = world_pos.z - land.chunk_origin.y;
  let cx = i32(floor(local_x));
  let cz = i32(floor(local_z));

  let hc = tile_height_at_13x13(cx, cz);
  let hl = tile_height_at_13x13(cx - 1, cz);
  let hr = tile_height_at_13x13(cx + 1, cz);
  let hd = tile_height_at_13x13(cx, cz - 1);
  let hu = tile_height_at_13x13(cx, cz + 1);

  let pos_slopes = max(0.0, hl - hc) + max(0.0, hr - hc) + max(0.0, hd - hc) + max(0.0, hu - hc);
  let occl = clamp(pos_slopes * 0.25, 0.0, 1.0);
  let mix_factor = occl * 0.5;
  return normalize(mix(base_normal_world, vec3<f32>(0.0, 1.0, 0.0), mix_factor));
}

// ============================================================================
// Lighting helpers
// ============================================================================

fn luminance(c: vec3<f32>) -> f32 {
  return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
fn chroma_only(c: vec3<f32>) -> vec3<f32> {
  let l = luminance(c);
  return c - vec3<f32>(l);
}

fn get_lambert(N: vec3<f32>, L: vec3<f32>) -> f32 {
  return max(dot(normalize(N), L), 0.0);
}
fn get_specular(N: vec3<f32>, L: vec3<f32>, V: vec3<f32>, shininess: f32) -> f32 {
  let H = normalize(L + V);
  return pow(max(dot(normalize(N), H), 0.0), shininess);
}
fn get_rim(N: vec3<f32>, V: vec3<f32>, power: f32) -> f32 {
  let rim_dot = 1.0 - max(dot(normalize(N), normalize(V)), 0.0);
  return pow(rim_dot, power);
}
fn get_hemisphere_fill(N: vec3<f32>) -> vec3<f32> {
  let upness = clamp(dot(normalize(N), vec3<f32>(0.0,1.0,0.0)) * 0.5 + 0.5, 0.0, 1.0);
  let sky    = lighting.fill_sky_color.rgb    * lighting.fill_sky_color.a;
  let ground = lighting.fill_ground_color.rgb * lighting.fill_ground_color.a;
  return mix(ground, sky, upness);
}
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}
fn grade_color(color: vec3<f32>) -> vec3<f32> {
  let luma = luminance(color);
  let warm_mix = smoothstep(0.0, 1.0, luma);
  let cool_mix = 1.0 - warm_mix;
  let strength = lighting.grade_params.x; // grading strength
  let tint = lighting.grade_warm_color.rgb * warm_mix + lighting.grade_cool_color.rgb * cool_mix;
  return mix(color, color + tint * 0.25 * strength, strength);
}

// ============================================================================
// Vertex shader
// ============================================================================

@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var out: VertexOutput;

  // Resolve dev toggles
  var shading_mode: u32 = tunables.shading_mode;
  var normal_mode:  u32 = tunables.normal_mode;
  if (DEV_USE_UNIFORMS == 0u) {
    shading_mode = DEV_SHADING_MODE;
    normal_mode  = DEV_NORMAL_MODE;
  }

  // Node index in the 9×9 vertex grid
  let grid_x: u32 = vertex_index % MESH_GRID_SIDE;
  let grid_z: u32 = vertex_index / MESH_GRID_SIDE;

  // Map node to 13×13 data index (adds 2-tile border)
  let arr_x = i32(grid_x) + DATA_GRID_BORDER;
  let arr_z = i32(grid_z) + DATA_GRID_BORDER;
  let data_idx = u32(arr_z) * u32(DATA_GRID_SIDE) + u32(arr_x);

  // Displace local vertex by pre-baked height
  var displaced_local_pos = in.position;
  displaced_local_pos.y = land.tiles[data_idx].tile_height;

  // Geometric normal (fast)
  let geometric_normal_local = get_geometric_normal_local(i32(grid_x), i32(grid_z));

  // World transform and clip position
  let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
  out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(displaced_local_pos, 1.0));
  out.position = view_transformations::position_world_to_clip(out.world_position.xyz);

  // Pass-through attributes
  out.uv = in.uv;
  out.instance_index = in.instance_index;
  out.world_normal = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);

  // Vertex/Gouraud path: cache lambert in uv_b.x for cheap fragment usage
  out.uv_b = vec2<f32>(0.0, 0.0);
  if ((DEV_USE_UNIFORMS == 1u && shading_mode == 0u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_SHADING_MODE == 0u)) {
    let lam_v = get_lambert(out.world_normal, scene.light_direction);
    out.uv_b.x = lam_v;
  }

  return out;
}

// ============================================================================
// Shading building blocks (kept small and testable)
// ============================================================================

// Vertex/Gouraud model: identical in spirit to your original vertex mode.
// brightness = albedo * (ambient + diffuse*lambert)
fn shade_vertex_model(base_albedo: vec3<f32>, lam_v: f32,
                      ambient_strength: f32, diffuse_strength: f32) -> vec3<f32> {
  return base_albedo * (ambient_strength + diffuse_strength * lam_v);
}

// Fragment (KR) model: hue-preserving composition.
// Theory & effect:
// - Ambient: “room light” in the shadows. Scalar, cool-tinted via ambient_color (in authoring).
//   Here we use only the scalar strength for brightness; the cool feel comes from sky/ground fill.
// - Diffuse: sunlight on faces that point to the light. Scalar lambert shaped by sharpness.
//   End result: bright where the surface faces the sun, zero in back-facing shadows.
// - Fill: environment tint from sky/ground. We convert its color to scalar with luma so it
//   brightens the albedo without bleaching. Then we add only a small chroma component.
//   Result: colors in shadow stay colorful (no gray wash).
// - Rim: thin silhouette glow, mostly on the shadow side. Added into remaining headroom;
//   split into neutral lift along the albedo and a tiny colored rim. Result: readable edges
//   without turning the whole surface yellow/white.
// - Specular: small, neutral-ish sparkle. Result: subtle sun glints on hard angles.
// - Exposure: part of tonemapping; compresses highlights to keep color detail.
fn shade_fragment_model(base_albedo: vec3<f32>,
                        Nw: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
                        ambient_strength: f32, diffuse_strength: f32,
                        sharpness_factor: f32, sharpness_mix: f32,
                        fill_strength: f32, rim_strength: f32,
                        specular_strength: f32) -> vec3<f32> {

  // Diffuse shaping
  let lam = get_lambert(Nw, L);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, sharpness_mix);
  let diffuse_term = diffuse_strength * lam_shaped; // scalar

  // Hemisphere fill (color) -> luma scalar + chroma
  var hemi_color = vec3<f32>(0.0);
  if (fill_strength > 0.0) {
    hemi_color = get_hemisphere_fill(Nw) * fill_strength;
  }
  let hemi_luma   = luminance(hemi_color);
  let hemi_chroma = chroma_only(hemi_color);

  // Ambient (scalar)
  let ambient_term = ambient_strength;

  // Headroom tunables (packed to avoid ABI change)
  let headroom_reserve = clamp(lighting.grade_params.y, 0.0, 1.0); // 0..1
  let hemi_chroma_tint = clamp(lighting.grade_params.z, 0.0, 1.0); // 0..1
  let runtime_headroom_on = lighting.grade_params.w >= 0.5;

  // Base brightness — strictly along the albedo
  var base_energy = diffuse_term + ambient_term + hemi_luma;

  // Optional headroom limiting to leave space for rim/spec
  if (CFG_ENABLE_HEADROOM_LIMIT && runtime_headroom_on) {
    base_energy = clamp(base_energy, 0.0, 1.0 - headroom_reserve);
  }

  var color = base_albedo * base_energy;

  // Add a subtle chroma component from fill (keeps shadows colorful)
  var headroom = 1.0 - max(color.r, max(color.g, color.b));
  let hemi_gain = min(headroom, hemi_luma * hemi_chroma_tint);
  color += base_albedo * (hemi_chroma * hemi_gain);

  // Rim — shadow-side, headroom-limited; neutral + tiny colored contribution
  if (rim_strength > 0.001) {
    let rim_power = max(0.1, lighting.rim_color.a); // 2..4 = thin edge
    let rim_raw   = get_rim(Nw, V, rim_power);

    // Shadow-side bias: fade rim where the surface is lit by the sun
    let NdotL = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis = rim_raw * (1.0 - smoothstep(0.0, 0.35, NdotL));

    headroom = max(0.0, 1.0 - max(color.r, max(color.g, color.b)));

    // Neutral lift along albedo preserves hue
    let rim_neutral = min(headroom, rim_vis * rim_strength * 0.35);
    color += base_albedo * rim_neutral;

    // Tiny colored sparkle
    let rim_colored = min(headroom, rim_vis * rim_strength * 0.25);
    color += lighting.rim_color.rgb * rim_colored;
  }

  // Specular (small, neutral-ish sparkle)
  if (specular_strength > 0.0001) {
    let spec_val = get_specular(Nw, L, V, 32.0);
    color += vec3<f32>(1.0) * spec_val * specular_strength;
  }

  return color;
}

// ============================================================================
// Fragment shader
// ============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
  // Resolve toggles/intensities from uniforms or dev defaults
  var shading_mode: u32 = tunables.shading_mode;
  var normal_mode:  u32 = tunables.normal_mode;
  var enable_bent:  u32 = tunables.enable_bent;
  var enable_tonemap: u32 = tunables.enable_tonemap;
  var enable_grading: u32 = tunables.enable_grading;
  var enable_fog:   u32 = tunables.enable_fog;

  var ambient_strength:  f32 = tunables.ambient_strength;
  var diffuse_strength:  f32 = tunables.diffuse_strength;
  var specular_strength: f32 = tunables.specular_strength;
  var rim_strength:      f32 = tunables.rim_strength;

  var sharpness_factor:  f32 = tunables.sharpness_factor;
  var sharpness_mix:     f32 = tunables.sharpness_mix;

  var fill_strength: f32 = lighting.fill_strength;
  var exposure: f32 = lighting.exposure;

  if (DEV_USE_UNIFORMS == 0u) {
    shading_mode     = DEV_SHADING_MODE;
    normal_mode      = DEV_NORMAL_MODE;
    enable_bent      = DEV_ENABLE_BENT;
    enable_tonemap   = DEV_ENABLE_TONEMAP;
    enable_grading   = DEV_ENABLE_GRADING;
    enable_fog       = DEV_ENABLE_FOG;

    ambient_strength  = DEV_AMBIENT;
    diffuse_strength  = DEV_DIFFUSE;
    specular_strength = DEV_SPECULAR;
    rim_strength      = DEV_RIM;
    fill_strength     = DEV_FILL;
    exposure          = DEV_EXPOSURE;
  }

  // Sample albedo from tile texture array (textures are assumed sRGB-view)
  let local_x = in.world_position.x - land.chunk_origin.x;
  let local_z = in.world_position.z - land.chunk_origin.y;
  let uv_in_tile = vec2<f32>(fract(local_x), fract(local_z));
  let tile = tile_at_13x13(i32(floor(local_x)), i32(floor(local_z)));

  var base_rgba: vec4<f32>;
  if (tile.texture_size == 1u) {
    base_rgba = textureSample(texarray_big, texarray_sampler, uv_in_tile, i32(tile.texture_layer));
  } else {
    base_rgba = textureSample(texarray_small, texarray_sampler, uv_in_tile, i32(tile.texture_layer));
  }
  let base_albedo = base_rgba.rgb; // already linear due to sRGB view
  let base_alpha  = base_rgba.a;

  // Normal selection: geometric vs bicubic; optional bent
  var Nw = normalize(in.world_normal);
  if ((DEV_USE_UNIFORMS == 1u && normal_mode == 1u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_NORMAL_MODE == 1u)) {
    let smooth_local = get_bicubic_normal(in.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(local_x, local_z);
    Nw = normalize(mix(smooth_world, Nw, blend_edge));
  }
  if ((DEV_USE_UNIFORMS == 1u && enable_bent == 1u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_ENABLE_BENT == 1u)) {
    Nw = get_bent_normal(in.world_position.xyz, Nw);
  }

  // Light & view vectors
  let L = scene.light_direction; // CPU supplies normalized
  let V = normalize(scene.camera_position - in.world_position.xyz);

  // Call the chosen shading model (small branch, implementation separated)
  var hdr_rgb = vec3<f32>(0.0);
  if ((DEV_USE_UNIFORMS == 1u && shading_mode == 0u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_SHADING_MODE == 0u)) {
    // Vertex/Gouraud
    hdr_rgb = shade_vertex_model(base_albedo, in.uv_b.x, ambient_strength, diffuse_strength);
  } else {
    // Fragment (KR)
    hdr_rgb = shade_fragment_model(
      base_albedo, Nw, V, L,
      ambient_strength, diffuse_strength,
      sharpness_factor, sharpness_mix,
      fill_strength, rim_strength,
      specular_strength
    );
  }

  // Optional fog (multiplicative tint)
  if ((DEV_USE_UNIFORMS == 1u && enable_fog == 1u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_ENABLE_FOG == 1u)) {
    let world_uv = in.world_position.xz * scene.fog_params.y;
    let time_off = scene.fog_params.zw * scene.time_seconds;
    let cloud_uv = world_uv + time_off;
    let mask = noise_2d(cloud_uv * 0.25) * 0.5 + 0.5;
    let fog_strength = clamp(scene.fog_params.x, 0.0, 1.0);
    let fog_opacity  = scene.fog_color.a;
    let fog_mask = clamp(mask * fog_opacity * fog_strength, 0.0, 1.0);
    hdr_rgb = hdr_rgb * mix(vec3<f32>(1.0), scene.fog_color.rgb, fog_mask);
  }

  // Optional grading + tonemap
  var post = hdr_rgb;
  if ((DEV_USE_UNIFORMS == 1u && enable_grading == 1u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_ENABLE_GRADING == 1u)) {
    post = grade_color(post);
  }
  var final_rgb = post;
  if ((DEV_USE_UNIFORMS == 1u && enable_tonemap == 1u) ||
      (DEV_USE_UNIFORMS == 0u && DEV_ENABLE_TONEMAP == 1u)) {
    final_rgb = tonemap_reinhard_with_exposure(post, exposure);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0)); // numeric safety
  return vec4<f32>(final_rgb, base_alpha);
}
