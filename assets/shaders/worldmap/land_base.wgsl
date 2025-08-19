// ============================================================================
// Bevy PBR WGSL — Terrain (Three modes: 0 Classic, 1 Enhanced, 2 KR-like)
// - Mode 0: “2D Classic” — faithful brightness model, vertex/Gouraud.
// - Mode 1: “2D Enhanced/Remastered” — per-fragment, subtle improvements,
//          stays close to original colors and contrast.
// - Mode 2: “KR-like” — per-fragment, painterly: warm key, cool ambient/fill,
//          vibrant grading, rim, optional gloom; avoids gray/metallic wash.
// ============================================================================

#import bevy_pbr::{
  forward_io::{Vertex, VertexOutput},
  mesh_functions,
  view_transformations,
}

// ============================================================================
// Compile-time DEV config (unified DEV_* prefix)
// ============================================================================

// If 0, use the DEV_* defaults below. If 1, use CPU uniforms.
const DEV_USE_UNIFORMS: u32 = 1u;

// Shader modes: 0 = Classic (vertex), 1 = Enhanced (fragment), 2 = KR (fragment)
const DEV_SHADING_MODE: u32 = 2u;

// Normal modes: 0 = geometric, 1 = bicubic
const DEV_NORMAL_MODE:  u32 = 1u;

// Feature flags (compile-time defaults; runtime toggles exist too)
const DEV_BENT:    u32 = 1u;
const DEV_FOG:     u32 = 1u;
const DEV_GLOOM:   u32 = 1u;
const DEV_TONEMAP: u32 = 1u;
const DEV_GRADING: u32 = 1u;
const DEV_BLUR:    u32 = 0u;

// Artist defaults when DEV_USE_UNIFORMS == 0
const DEV_AMBIENT:  f32 = 0.18; // shadow light
const DEV_DIFFUSE:  f32 = 1.08; // direct sunlight
const DEV_SPECULAR: f32 = 0.05; // sparkle
const DEV_RIM:      f32 = 0.16; // silhouette
const DEV_FILL:     f32 = 0.34; // environment intensity
const DEV_EXPOSURE: f32 = 1.08; // tonemap exposure
const DEV_BLUR_STRENGTH: f32 = 0.15;
const DEV_BLUR_RADIUS:   f32 = 0.0025;

// Dev palette (warm key, cool ambient)
const DEV_LIGHT_COLOR:   vec3<f32> = vec3<f32>(1.06, 0.99, 0.92);
const DEV_AMBIENT_COLOR: vec3<f32> = vec3<f32>(0.18, 0.22, 0.29);

// Headroom limiting (prevents bleaching in stylized modes). Also has runtime toggle.
const CFG_ENABLE_HEADROOM_LIMIT: bool = true;

// ============================================================================
// Bindings / Uniform Layouts
// ============================================================================

struct TileUniform {
  tile_height:   f32,
  texture_size:  u32, // 0=small atlas, 1=big atlas
  texture_layer: u32,
  texture_hue:   u32,
};

struct LandUniform {
  _light_dir_legacy: vec3<f32>, // kept for ABI safety; ignore
  _pad0: f32,
  chunk_origin: vec2<f32>, // world origin of chunk (x,z) in tile units
  _pad1: vec2<f32>,
  tiles: array<TileUniform, 169>, // 13×13 grid (8×8 core + 2 border)
};

struct SceneUniform {
  camera_position: vec3<f32>,
  time_seconds: f32,
  light_direction: vec3<f32>, // expected normalized by CPU
  _pad1: f32,
};

struct TunablesUniform {
  // Modes / toggles
  shading_mode:   u32, // 0=Classic (vertex), 1=Enhanced (frag), 2=KR (frag)
  normal_mode:    u32, // 0=geometric, 1=bicubic
  enable_bent:    u32,
  enable_fog:     u32,

  enable_gloom:   u32,
  enable_tonemap: u32,
  enable_grading: u32,
  enable_blur:    u32,

  // Intensities (group in vec4 slots for alignment)
  // Slot A
  ambient_strength:  f32, // Ambient: base light in shadows
  diffuse_strength:  f32, // Diffuse: sunlight intensity
  specular_strength: f32, // Specular: sun glint
  rim_strength:      f32, // Rim: silhouette highlight

  // Slot B
  fill_strength:     f32, // Env sky/ground intensity (luma + some chroma)
  sharpness_factor:  f32, // Diffuse shaping (lambert^factor)
  sharpness_mix:     f32, // 0=Lambert, 1=sharpened
  blur_strength:     f32, // 0..1 mix with blurred base albedo

  // Slot C
  blur_radius:       f32, // UV radius for blur taps (e.g., 0.001..0.01)
  _pad_c1:           f32,
  _pad_c2:           f32,
  _pad_c3:           f32,
};

// Lighting / look controls.
// grade_params: [grade_strength, headroom_reserve, hemi_chroma_tint, headroom_on]
// grade_extra:  [vibrance, saturation, contrast, split_strength]
// gloom_params: [amount, height_falloff_height, shadow_bias, unused]
//   - height_falloff_height = world height (in units) where gloom fades out (0..∞)
//   - shadow_bias = 0 → uniform gloom, 1 → strongly biased to shadow
struct LightingUniforms {
  light_color:   vec3<f32>, // key light color (tints diffuse)
  _pad0:         f32,
  ambient_color: vec3<f32>,
  _pad1:         f32,

  exposure: f32,
  gamma:    f32,  // unused (textures are sRGB-view), reserved
  _pad2:    vec2<f32>,

  fill_sky_color:     vec4<f32>, // rgb sky tint, a = per-color strength
  fill_ground_color:  vec4<f32>, // rgb ground tint, a = per-color strength
  rim_color:          vec4<f32>, // rgb rim tint,  a = rim “power” (2..4=thin edge)

  grade_warm_color: vec4<f32>, // warm toning color (rgb)
  grade_cool_color: vec4<f32>, // cool toning color (rgb)
  grade_params:     vec4<f32>, // [grade_strength, headroom_reserve, hemi_chroma_tint, headroom_on]
  grade_extra:      vec4<f32>, // [vibrance, saturation, contrast, split_strength]

  gloom_params:     vec4<f32>, // [amount, height_falloff_height, shadow_bias, _]

  // Fog: we reinterpret params for clearer control (distance + height + noise)
  //   fog_color.rgb = fog tint
  //   fog_color.a   = max fog mix (0..1)
  //   fog_params.x  = distance_fog_density (0..∞)
  //   fog_params.y  = height_fog_density   (0..∞) (per world-unit height)
  //   fog_params.z  = noise_scale          (e.g., 0.1..1.0)
  //   fog_params.w  = noise_strength       (0..1)
  fog_color: vec4<f32>,
  fog_params: vec4<f32>,
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

const CHUNK_TILE_NUM_1D: u32 = 8u;
const DATA_GRID_BORDER:  i32 = 2;
const DATA_GRID_SIDE:    i32 = 13;
const MESH_GRID_SIDE:    u32 = 9u;

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

// Simple value noise for optional fog modulation.
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
// Normal utilities (geometric, bicubic, bent)
// ============================================================================

fn get_geometric_normal_local(node_x: i32, node_z: i32) -> vec3<f32> {
  let hL = tile_height_at_13x13(node_x - 1, node_z);
  let hR = tile_height_at_13x13(node_x + 1, node_z);
  let hD = tile_height_at_13x13(node_x, node_z - 1);
  let hU = tile_height_at_13x13(node_x, node_z + 1);
  let dHdx = 0.5 * (hR - hL);
  let dHdz = 0.5 * (hU - hD);
  return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

fn get_bicubic_normal(world_pos: vec3<f32>) -> vec3<f32> {
  let local_x = world_pos.x - land.chunk_origin.x;
  let local_z = world_pos.z - land.chunk_origin.y;

  let base_x = floor(local_x);
  let base_z = floor(local_z);
  let frac_x = local_x - base_x;
  let frac_z = local_z - base_z;

  let ix = i32(base_x);
  let iz = i32(base_z);

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

  let row0 = cubic_interp_value_and_derivative(h00, h10, h20, h30, frac_x);
  let row1 = cubic_interp_value_and_derivative(h01, h11, h21, h31, frac_x);
  let row2 = cubic_interp_value_and_derivative(h02, h12, h22, h32, frac_x);
  let row3 = cubic_interp_value_and_derivative(h03, h13, h23, h33, frac_x);

  let dHdx = cubic_value(row0.y, row1.y, row2.y, row3.y, frac_z);

  let col0 = cubic_interp_value_and_derivative(h00, h01, h02, h03, frac_z);
  let col1 = cubic_interp_value_and_derivative(h10, h11, h12, h13, frac_z);
  let col2 = cubic_interp_value_and_derivative(h20, h21, h22, h23, frac_z);
  let col3 = cubic_interp_value_and_derivative(h30, h31, h32, h33, frac_z);
  let dHdz = cubic_value(col0.y, col1.y, col2.y, col3.y, frac_x);

  return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

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

// Contrast S-curve with neutral at contrast=1.0 (k = contrast-1 in [-1,1])
// FIX: do NOT clamp here — keep HDR headroom pre-tonemap.
fn apply_contrast_neutral(x: vec3<f32>, contrast: f32) -> vec3<f32> {
  let t = clamp(contrast - 1.0, -1.0, 1.0);
  // y = x + t * (x - x*x) * 2  (S-curve around ~0.5; zero effect when t=0)
  return x + t * ((x - x * x) * 2.0);
}

// Stronger, stylized color grading (vibrant), with:
//  - truly neutral contrast=1 (no clamp inside),
//  - multiplicative split-toning normalized to luma=1 so mid-grays stay neutral.
fn grade_color_vibrant(color_in: vec3<f32>) -> vec3<f32> {
  let strength   = lighting.grade_params.x;  // overall grade amount
  let headroom_on = lighting.grade_params.w >= 0.5;
  let vibrance   = lighting.grade_extra.x;   // selective saturation
  let saturation = lighting.grade_extra.y;   // global saturation
  let contrast   = lighting.grade_extra.z;   // S-curve; 1.0 = neutral
  let split_str  = lighting.grade_extra.w;   // split-toning strength

  // Global saturation around luminance pivot
  let l = luminance(color_in);
  let sat_col = mix(vec3<f32>(l), color_in, saturation);

  // Vibrance: boost low-sat regions more (mask stronger for low chroma)
  let chroma = sat_col - vec3<f32>(l);
  let sat_mag = max(max(abs(chroma.r), abs(chroma.g)), abs(chroma.b));
  let vib_mask = smoothstep(0.0, 0.7, 1.0 - sat_mag);
  let vib_col = sat_col + chroma * (vibrance * vib_mask);

  // Contrast (neutral at 1.0), keep HDR — no clamp here
  let ctr_col = apply_contrast_neutral(vib_col, contrast);

  // Split-toning by luminance: cool lows, warm highs — multiplicative, luma-normalized
  let warm = lighting.grade_warm_color.rgb;
  let cool = lighting.grade_cool_color.rgb;
  let wmix = smoothstep(0.25, 0.85, l);
  let tint = mix(cool, warm, wmix);

  // Normalize tint so its luma is ~1 → keeps mid-gray unchanged when applied multiplicatively
  let tint_luma = max(luminance(tint), 1e-6);
  let tint_norm = tint / tint_luma;
  let split_mult = mix(vec3<f32>(1.0), tint_norm, split_str);

  // Apply split toning multiplicatively, then blend with original by overall strength
  let graded_hq = ctr_col * split_mult;
  let graded = mix(color_in, graded_hq, clamp(strength, 0.0, 2.0));

  // Keep numeric safety; we still allow HDR >1 pre-tonemap
  return max(graded, vec3<f32>(0.0));
}

// Gloom: general, height-fading, optional shadow bias.
// gloom_params: [amount, height_falloff_height, shadow_bias, _]
fn apply_gloom(color_in: vec3<f32>, world_pos: vec3<f32>, N: vec3<f32>, L: vec3<f32>) -> vec3<f32> {
  let amount             = clamp(lighting.gloom_params.x, 0.0, 1.0);
  if (amount < 1e-4) { return color_in; }
  let falloff_height     = max(lighting.gloom_params.y, 0.0);
  let shadow_bias        = clamp(lighting.gloom_params.z, 0.0, 1.0);

  // Height term: fade out over [0 .. falloff_height]
  let h = max(world_pos.y, 0.0);
  let height_term = select((1.0 - smoothstep(0.0, falloff_height, h)), 1.0, (falloff_height < 1e-6));

  // Optional shadow bias (0 = uniform, 1 = fully biased toward shadow)
  let NdotL = max(dot(normalize(N), normalize(L)), 0.0);
  let shadow_term = pow(1.0 - NdotL, 1.5); // smooth emphasis for shadowed faces
  let bias_term = mix(1.0, shadow_term, shadow_bias);

  let g = clamp(amount * height_term * bias_term, 0.0, 1.0);

  // Cool, moody tint from ambient color; multiplicative keeps hues intact
  let gloom_tint = mix(vec3<f32>(1.0), lighting.ambient_color, 0.7);
  return color_in * mix(vec3<f32>(1.0), gloom_tint, g);
}

// Tonemap (Reinhard + exposure)
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
  let e = max(exposure, 1e-6);
  return (c * e) / (vec3<f32>(1.0) + c * e);
}

// ============================================================================
// Texture sampling helpers (for optional blur of base albedo)
// ============================================================================

fn sample_tile_albedo(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 1u) {
    return textureSample(texarray_big, texarray_sampler, uv, layer).rgb;
  } else {
    return textureSample(texarray_small, texarray_sampler, uv, layer).rgb;
  }
}

// Same as above, but with explicit gradients to keep LOD stable across taps.
fn sample_tile_albedo_grad(uv: vec2<f32>, tile: TileUniform, ddx_uv: vec2<f32>, ddy_uv: vec2<f32>) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 1u) {
    return textureSampleGrad(texarray_big, texarray_sampler, uv, layer, ddx_uv, ddy_uv).rgb;
  } else {
    return textureSampleGrad(texarray_small, texarray_sampler, uv, layer, ddx_uv, ddy_uv).rgb;
  }
}

// 9-tap blur with radius in *screen pixels* via fwidth — visible regardless of UV scale.
fn blurred_albedo(uv: vec2<f32>, tile: TileUniform, radius_in_pixels: f32) -> vec3<f32> {
  // Approximate one screen pixel in UV space
  let fw = fwidth(uv);
  let px_uv = max(fw.x, fw.y) + 1e-6;
  let r = max(radius_in_pixels, 0.0) * px_uv;

  // Keep LOD stable for all taps
  let ddx_uv = dpdx(uv);
  let ddy_uv = dpdy(uv);

  // Clamp taps to [0,1] to avoid bleeding across tile edges if sampler wraps
  let c = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
  let u1 = clamp(uv + vec2<f32>( r, 0.0), vec2<f32>(0.0), vec2<f32>(1.0));
  let u2 = clamp(uv + vec2<f32>(-r, 0.0), vec2<f32>(0.0), vec2<f32>(1.0));
  let u3 = clamp(uv + vec2<f32>(0.0,  r), vec2<f32>(0.0), vec2<f32>(1.0));
  let u4 = clamp(uv + vec2<f32>(0.0, -r), vec2<f32>(0.0), vec2<f32>(1.0));
  let u5 = clamp(uv + vec2<f32>( r,  r), vec2<f32>(0.0), vec2<f32>(1.0));
  let u6 = clamp(uv + vec2<f32>(-r,  r), vec2<f32>(0.0), vec2<f32>(1.0));
  let u7 = clamp(uv + vec2<f32>( r, -r), vec2<f32>(0.0), vec2<f32>(1.0));
  let u8 = clamp(uv + vec2<f32>(-r, -r), vec2<f32>(0.0), vec2<f32>(1.0));

  // Weights (normalized 9-tap kernel)
  let wc = 0.24;
  let w1 = 0.10;
  let w2 = 0.10;
  let w3 = 0.10;
  let w4 = 0.10;
  let w5 = 0.09;
  let w6 = 0.09;
  let w7 = 0.09;
  let w8 = 0.09;

  let s0 = sample_tile_albedo_grad(c,  tile, ddx_uv, ddy_uv);
  let s1 = sample_tile_albedo_grad(u1, tile, ddx_uv, ddy_uv);
  let s2 = sample_tile_albedo_grad(u2, tile, ddx_uv, ddy_uv);
  let s3 = sample_tile_albedo_grad(u3, tile, ddx_uv, ddy_uv);
  let s4 = sample_tile_albedo_grad(u4, tile, ddx_uv, ddy_uv);
  let s5 = sample_tile_albedo_grad(u5, tile, ddx_uv, ddy_uv);
  let s6 = sample_tile_albedo_grad(u6, tile, ddx_uv, ddy_uv);
  let s7 = sample_tile_albedo_grad(u7, tile, ddx_uv, ddy_uv);
  let s8 = sample_tile_albedo_grad(u8, tile, ddx_uv, ddy_uv);

  return s0*wc + s1*w1 + s2*w2 + s3*w3 + s4*w4 + s5*w5 + s6*w6 + s7*w7 + s8*w8;
}

// ============================================================================
// Vertex shader
// ============================================================================

@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var out: VertexOutput;

  // Resolve mode toggles
  var shading_mode: u32   = tunables.shading_mode;
  var normal_mode:  u32   = tunables.normal_mode;

  if (DEV_USE_UNIFORMS == 0u) {
    shading_mode = DEV_SHADING_MODE;
    normal_mode  = DEV_NORMAL_MODE;
  }

  // Node indices in 9×9 grid
  let grid_x: u32 = vertex_index % MESH_GRID_SIDE;
  let grid_z: u32 = vertex_index / MESH_GRID_SIDE;

  // Map node to 13×13 data index (+2 border)
  let arr_x = i32(grid_x) + DATA_GRID_BORDER;
  let arr_z = i32(grid_z) + DATA_GRID_BORDER;
  let data_idx = u32(arr_z) * u32(DATA_GRID_SIDE) + u32(arr_x);

  // Displace by pre-baked height
  var displaced_local_pos = in.position;
  displaced_local_pos.y = land.tiles[data_idx].tile_height;

  // Geometric normal (fast)
  let geometric_normal_local = get_geometric_normal_local(i32(grid_x), i32(grid_z));

  // World transform / clip
  let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
  out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(displaced_local_pos, 1.0));
  out.position       = view_transformations::position_world_to_clip(out.world_position.xyz);

  out.uv = in.uv;
  out.instance_index = in.instance_index;
  out.world_normal   = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);

  // Classic vertex path: precompute lambert in uv_b.x
  out.uv_b = vec2<f32>(0.0, 0.0);
  if ( ((DEV_USE_UNIFORMS == 1u) && (shading_mode == 0u))
    || ((DEV_USE_UNIFORMS == 0u) && (DEV_SHADING_MODE == 0u)) ) {
    out.uv_b.x = get_lambert(out.world_normal, scene.light_direction);
  }

  return out;
}

// ============================================================================
// Shading models (small focused functions)
// ============================================================================

fn shade_mode0_classic_vertex(base_albedo: vec3<f32>, lam_v: f32,
                              ambient_strength: f32, diffuse_strength: f32) -> vec3<f32> {
  return base_albedo * (ambient_strength + diffuse_strength * lam_v);
}

// Enhanced: subtle; diffuse tinted by light_color; fill chroma minimal.
fn shade_mode1_enhanced_fragment(base_albedo_in: vec3<f32>,
                                 world_pos: vec3<f32>, Nw: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
                                 ambient_strength: f32, diffuse_strength: f32,
                                 sharpness_factor: f32, sharpness_mix: f32,
                                 fill_strength: f32, rim_strength: f32,
                                 specular_strength: f32,
                                 enable_gloom: u32) -> vec3<f32> {

  // Diffuse shaping
  let lam = get_lambert(Nw, L);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, clamp(sharpness_mix, 0.0, 0.4));

  // Energy RGB (allow warm key light tint)
  let diffuse_rgb = lighting.light_color * (diffuse_strength * lam_shaped);
  let hemi_rgb = select(vec3<f32>(0.0), (get_hemisphere_fill(Nw) * fill_strength), (fill_strength > 0.0));

  // Base energy is RGB now: ambient (scalar) + fill luma + tinted diffuse
  let hemi_luma = luminance(hemi_rgb);
  var energy_rgb = vec3<f32>(ambient_strength + hemi_luma) + diffuse_rgb;

  // Clamp energy to keep room if headroom enabled
  let headroom_reserve = 0.10;
  if (CFG_ENABLE_HEADROOM_LIMIT && lighting.grade_params.w >= 0.5) {
    energy_rgb = min(energy_rgb, vec3<f32>(1.0 - headroom_reserve));
  }

  var color = base_albedo_in * energy_rgb;

  // Small chroma from fill (keeps shadows colorful but subtle)
  let hemi_chroma_tint = min(lighting.grade_params.z, 0.20);
  let hemi_chroma = chroma_only(hemi_rgb);
  var headroom = 1.0 - max(color.r, max(color.g, color.b));
  let hemi_gain = min(headroom, hemi_luma * hemi_chroma_tint);
  color += base_albedo_in * (hemi_chroma * hemi_gain);

  // Very subtle rim/spec
  let rim_local = rim_strength * 0.25;
  if (rim_local > 0.001) {
    let rim_raw = get_rim(Nw, V, max(0.1, lighting.rim_color.a));
    let NdotL = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis = rim_raw * (1.0 - smoothstep(0.0, 0.35, NdotL));
    headroom = max(0.0, 1.0 - max(color.r, max(color.g, color.b)));
    color += base_albedo_in * min(headroom, rim_vis * rim_local * 0.30);
  }

  if (specular_strength > 0.0001) {
    let spec_val = get_specular(Nw, L, V, 24.0);
    color += vec3<f32>(1.0) * spec_val * (specular_strength * 0.5);
  }

  // Optional gloom
  if (enable_gloom == 1u) {
    color = apply_gloom(color, world_pos, Nw, L);
  }

  return color;
}

// KR: stronger style; diffuse tinted by light_color; rim/gloom headroom-limited.
fn shade_mode2_kr_fragment(base_albedo_in: vec3<f32>,
                           world_pos: vec3<f32>, Nw: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
                           ambient_strength: f32, diffuse_strength: f32,
                           sharpness_factor: f32, sharpness_mix: f32,
                           fill_strength: f32, rim_strength: f32,
                           specular_strength: f32,
                           enable_gloom: u32) -> vec3<f32> {

  let lam = get_lambert(Nw, L);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, sharpness_mix);

  // Energy RGB (warm key tint)
  let diffuse_rgb = lighting.light_color * (diffuse_strength * lam_shaped);
  let hemi_rgb = select(vec3<f32>(0.0), (get_hemisphere_fill(Nw) * fill_strength), (fill_strength > 0.0));
  let hemi_luma = luminance(hemi_rgb);

  // Headroom control
  let headroom_reserve = clamp(lighting.grade_params.y, 0.0, 1.0);
  let runtime_headroom_on = lighting.grade_params.w >= 0.5;

  var energy_rgb = vec3<f32>(ambient_strength + hemi_luma) + diffuse_rgb;
  if (CFG_ENABLE_HEADROOM_LIMIT && runtime_headroom_on) {
    energy_rgb = min(energy_rgb, vec3<f32>(1.0 - headroom_reserve));
  }

  var color = base_albedo_in * energy_rgb;

  // Fill chroma
  let hemi_chroma_tint = clamp(lighting.grade_params.z, 0.0, 1.0);
  let hemi_chroma = chroma_only(hemi_rgb);
  var headroom = 1.0 - max(color.r, max(color.g, color.b));
  let hemi_gain = min(headroom, hemi_luma * hemi_chroma_tint);
  color += base_albedo_in * (hemi_chroma * hemi_gain);

  // Rim
  if (rim_strength > 0.001) {
    let rim_power = max(0.1, lighting.rim_color.a);
    let rim_raw   = get_rim(Nw, V, rim_power);
    let NdotL     = max(dot(normalize(Nw), normalize(L)), 0.0);
    let rim_vis   = rim_raw * (1.0 - smoothstep(0.0, 0.35, NdotL));
    headroom = max(0.0, 1.0 - max(color.r, max(color.g, color.b)));
    let rim_neutral = min(headroom, rim_vis * rim_strength * 0.35);
    color += base_albedo_in * rim_neutral;
    let rim_colored = min(headroom, rim_vis * rim_strength * 0.25);
    color += lighting.rim_color.rgb * rim_colored;
  }

  // Specular
  if (specular_strength > 0.0001) {
    let spec_val = get_specular(Nw, L, V, 32.0);
    color += vec3<f32>(1.0) * spec_val * specular_strength;
  }

  // Optional gloom
  if (enable_gloom == 1u) {
    color = apply_gloom(color, world_pos, Nw, L);
  }

  return color;
}

// ============================================================================
// Fragment shader
// ============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
  // Resolve toggles/intensities from uniforms or DEV defaults
  var shading_mode:   u32 = tunables.shading_mode;
  var normal_mode:    u32 = tunables.normal_mode;
  var enable_bent:    u32 = tunables.enable_bent;
  var enable_fog:     u32 = tunables.enable_fog;
  var enable_gloom:   u32 = tunables.enable_gloom;
  var enable_tonemap: u32 = tunables.enable_tonemap;
  var enable_grading: u32 = tunables.enable_grading;
  var enable_blur:    u32 = tunables.enable_blur;

  var ambient_strength:  f32 = tunables.ambient_strength;
  var diffuse_strength:  f32 = tunables.diffuse_strength;
  var specular_strength: f32 = tunables.specular_strength;
  var rim_strength:      f32 = tunables.rim_strength;

  var fill_strength:     f32 = tunables.fill_strength;
  var sharpness_factor:  f32 = tunables.sharpness_factor;
  var sharpness_mix:     f32 = tunables.sharpness_mix;

  var blur_strength:     f32 = tunables.blur_strength;
  var blur_radius:       f32 = tunables.blur_radius;

  var exposure: f32 = lighting.exposure;

  if (DEV_USE_UNIFORMS == 0u) {
    shading_mode   = DEV_SHADING_MODE;
    normal_mode    = DEV_NORMAL_MODE;
    enable_bent    = DEV_BENT;
    enable_fog     = DEV_FOG;
    enable_gloom   = DEV_GLOOM;
    enable_tonemap = DEV_TONEMAP;
    enable_grading = DEV_GRADING;
    enable_blur    = DEV_BLUR;

    ambient_strength  = DEV_AMBIENT;
    diffuse_strength  = DEV_DIFFUSE;
    specular_strength = DEV_SPECULAR;
    rim_strength      = DEV_RIM;
    fill_strength     = DEV_FILL;
    exposure          = DEV_EXPOSURE;

    blur_strength     = DEV_BLUR_STRENGTH;
    blur_radius       = DEV_BLUR_RADIUS;
  }

  // Local coords and tile
  let local_x = in.world_position.x - land.chunk_origin.x;
  let local_z = in.world_position.z - land.chunk_origin.y;
  let uv_in_tile = vec2<f32>(fract(local_x), fract(local_z));
  let tile = tile_at_13x13(i32(floor(local_x)), i32(floor(local_z)));

  // Base albedo (optionally blurred with screen-pixel radius)
  var base_albedo = sample_tile_albedo(uv_in_tile, tile);
  if (enable_blur == 1u && blur_strength > 0.001 && blur_radius > 0.0) {
    let blurred = blurred_albedo(uv_in_tile, tile, blur_radius);
    base_albedo = mix(base_albedo, blurred, clamp(blur_strength, 0.0, 1.0));
  }
  let base_alpha: f32 = 1.0; // tile textures assumed opaque for terrain

  // Normals: geometric vs bicubic; optional bent
  var Nw = normalize(in.world_normal);
  if (normal_mode == 1u) {
    let smooth_local = get_bicubic_normal(in.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(local_x, local_z);
    Nw = normalize(mix(smooth_world, Nw, blend_edge));
  }
  if (enable_bent == 1u) {
    Nw = get_bent_normal(in.world_position.xyz, Nw);
  }

  // Light & view
  let L = scene.light_direction; // normalized by CPU
  let V = normalize(scene.camera_position - in.world_position.xyz);

  // Shade
  var hdr_rgb = vec3<f32>(0.0);
  if (shading_mode == 0u) {
    hdr_rgb = shade_mode0_classic_vertex(base_albedo, in.uv_b.x, ambient_strength, diffuse_strength);
  } else if (shading_mode == 1u) {
    hdr_rgb = shade_mode1_enhanced_fragment(
      base_albedo, in.world_position.xyz, Nw, V, L,
      ambient_strength, diffuse_strength, sharpness_factor, sharpness_mix,
      fill_strength, rim_strength, specular_strength, enable_gloom
    );
  } else { // 2 = KR-like
    hdr_rgb = shade_mode2_kr_fragment(
      base_albedo, in.world_position.xyz, Nw, V, L,
      ambient_strength, diffuse_strength, sharpness_factor, sharpness_mix,
      fill_strength, rim_strength, specular_strength, enable_gloom
    );
  }

  // Fog (distance + optional height + optional noise), visible and tunable
  if (enable_fog == 1u) {
    let cam_to_p = in.world_position.xyz - scene.camera_position;
    let d = length(cam_to_p);
    let dist_density   = max(lighting.fog_params.x, 0.0);
    let height_density = max(lighting.fog_params.y, 0.0);
    let noise_scale    = lighting.fog_params.z;
    let noise_strength = clamp(lighting.fog_params.w, 0.0, 1.0);

    // Exponential distance fog
    let dist_fog   = 1.0 - exp(-dist_density * d);
    // Height fog: increases with height above y=0 (or flip sign if you prefer ground-hugging)
    let y = max(in.world_position.y, 0.0);
    let height_fog = 1.0 - exp(-height_density * y);

    // Combine as union: a + b - a*b (either density contributes)
    var fog_factor = clamp(dist_fog + height_fog - dist_fog * height_fog, 0.0, 1.0);

    // Optional soft noise modulation (stable over time)
    if (noise_strength > 0.001 && abs(noise_scale) > 1e-6) {
      let n = noise_2d(in.world_position.xz * noise_scale + scene.time_seconds * vec2<f32>(0.07, 0.05));
      let modificator = mix(1.0 - 0.3 * noise_strength, 1.0 + 0.3 * noise_strength, n);
      fog_factor = clamp(fog_factor * modificator, 0.0, 1.0);
    }

    let fog_mix = clamp(fog_factor * lighting.fog_color.a, 0.0, 1.0);
    hdr_rgb = mix(hdr_rgb, lighting.fog_color.rgb, fog_mix);
  }

  // Grading (vibrant with neutral contrast) + Tonemap
  var post = hdr_rgb;
  if (enable_grading == 1u) {
    post = grade_color_vibrant(post); // no pre-tonemap clamping anymore
  }
  var final_rgb = post;
  if (enable_tonemap == 1u) {
    // Optional safety before tonemap if upstream ops ever go negative
    final_rgb = tonemap_reinhard_with_exposure(max(post, vec3<f32>(0.0)), exposure);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0));
  return vec4<f32>(final_rgb, base_alpha);
}
