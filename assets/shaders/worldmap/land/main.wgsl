// ============================================================================
// Bevy PBR WGSL — Terrain (Three modes: 0 Classic, 1 Enhanced, 2 KR-like)
// - Mode 0: “2D Classic” — faithful brightness model, vertex/Gouraud.
// - Mode 1: “2D Enhanced/Remastered” — per-fragment, subtle improvements.
// - Mode 2: “KR-like” — painterly: warm key, cool ambient/fill, vibrant grading.
// ============================================================================

#import bevy_pbr::{
  forward_io::{Vertex, VertexOutput},
  mesh_functions,
  mesh_view_bindings::globals,
  view_transformations,
}

// ============================================================================
// Compile-time DEV config (unified DEV_* prefix)
// ============================================================================

const USE_VOLUMETRIC_NOISE: u32 = 1u; // 0=flat fog, 1=domain-warped billow modulation

// ============================================================================
// Bindings / Uniform Layouts
// ============================================================================

struct TileUniform {
  tile_height:   f32,
  texture_size:  u32, // 0=small atlas, 1=big atlas
  texture_layer: u32,
  texture_hue:   u32,
};

struct AtlasParams {
  page_texels: vec2<u32>,
  tiles_per_page: vec2<u32>,
  max_layers: u32,
  world_pages_x: u32,
  _pad: vec2<u32>,
  page_to_layer: array<vec4<u32>, 64>, // Maps 256 logic pages
};

struct SceneUniform {
  camera_position: vec3<f32>,
  _pad_cam: f32,
  light_direction: vec3<f32>, // expected normalized by CPU
  // global scene light scaler (pre-tonemap). Default 1.0 from CPU/UI.
  global_lighting: f32,
  // Camera orthographic scale factor.
  render_zoom: f32,
  // 0..1 simplification amount used for zoomed-out checkerboard shading.
  adaptive_zoom_simplification: f32,
  _pad0: vec2<f32>,
};

struct LandEffectsUniform {
  // Modes / toggles
  shading_mode:   u32, // 0=Classic (vertex), 1=Enhanced (frag), 2=KR (frag)
  normal_mode:    u32, // 0=geometric, 1=bicubic
  enable_bent:    u32,
  enable_fog:     u32,

  enable_gloom:   u32,
  enable_tonemap: u32,
  enable_grading: u32,
  enable_blur:    u32,

  // NEW: Graphics settings
  enable_linear_filtering: u32,
  reconstruction_mode:     u32,
  sharpening_amount:       f32,
  _pad_graphics:           f32,

  // intensities (grouped to match std140-ish packing)
  ambient_strength:  f32,
  diffuse_strength:  f32,
  specular_strength: f32,
  rim_strength:      f32,

  fill_strength:     f32,
  sharpness_factor:  f32,
  sharpness_mix:     f32,
  blur_strength:     f32,

  blur_radius:       f32,
  _pad_c1:           f32,
  _pad_c2:           f32,
  _pad_c3:           f32,
};

struct LandLightingUniforms {
    light_color: vec3<f32>,
    _pad0: f32,
    ambient_color: vec3<f32>,
    _pad1: f32,
    exposure: f32,
    gamma: f32,
    _pad2: vec2<f32>,
    fill_sky_color: vec4<f32>,
    fill_ground_color: vec4<f32>,
    rim_color: vec4<f32>,
    grade_warm_color: vec4<f32>,
    grade_cool_color: vec4<f32>,
    grade_params: vec4<f32>,
    grade_extra: vec4<f32>,
    gloom_params: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
};

@group(3) @binding(100) var tex_small_sampler: sampler;
@group(3) @binding(101) var tex_small: texture_2d_array<f32>;
@group(3) @binding(102) var tex_big:   texture_2d_array<f32>;
@group(3) @binding(103) var tile_meta_atlas: texture_2d_array<u32>;
@group(3) @binding(104) var<uniform> ATLAS: AtlasParams;
@group(3) @binding(105) var<uniform> scene:   SceneUniform;
@group(3) @binding(106) var<uniform> effects: LandEffectsUniform;
@group(3) @binding(107) var<uniform> lighting: LandLightingUniforms;

// ============================================================================
// Grid helpers & utilities
// ============================================================================

const CHUNK_TILE_NUM_DIM: u32 = 8u;
const DATA_GRID_BORDER:  i32 = 2;
const DATA_GRID_SIDE:    i32 = 13;  // DATA_GRID_BORDER + CHUNK_TILE_NUM_DIM + DATA_GRID_BORDER
const MESH_GRID_SIDE:    u32 = 9u;

// Query the world page coordinates and map them through LRU to a physical GPU Array Layer
fn atlas_read_meta(world_x: i32, world_z: i32) -> TileUniform {
  if (world_x < 0 || world_z < 0) {
    return TileUniform(0.0, 0u, 0u, 0u);
  }

  let wx = u32(world_x);
  let wz = u32(world_z);

  let pw = ATLAS.page_texels.x;
  let ph = ATLAS.page_texels.y;

  let page_x = wx / pw;
  let page_y = wz / ph;

  let off_x = wx % pw;
  let off_y = wz % ph;

  let page_index = page_y * ATLAS.world_pages_x + page_x;
  var layer: u32 = 0xFFFFFFFFu;
  if (page_index < 256u) {
    let arr_idx = page_index / 4u;
    let comp = page_index % 4u;
    layer = ATLAS.page_to_layer[arr_idx][comp];
  }

  if (layer >= ATLAS.max_layers) {
    return TileUniform(0.0, 0u, 0u, 0u);
  }

  // Load from Rg16Uint texture array
  let packed = textureLoad(tile_meta_atlas, vec2<i32>(i32(off_x), i32(off_y)), i32(layer), 0);
  let r = packed.x;
  let g = packed.y;

  let layer_idx = r; // contains only texture layer index (16 bit)

  let height_biased = g & 0xFFu;
  let z_i32 = i32(height_biased) - 128;
  let tile_height = f32(z_i32) * 0.1;

  let tex_size = (g >> 8u) & 1u;

  return TileUniform(tile_height, tex_size, layer_idx, 0u);
}

fn atlas_read_height(world_x: i32, world_z: i32) -> f32 {
  return atlas_read_meta(world_x, world_z).tile_height;
}

// Near the chunk edge, blend normals toward the original to hide seams.
fn chunk_edge_blend_factor(world_x: f32, world_z: f32) -> f32 {
  let local_x = fract(world_x / 8.0) * 8.0;
  let local_z = fract(world_z / 8.0) * 8.0;
  let tx = floor(local_x);
  let tz = floor(local_z);
  let dx = min(tx, f32(CHUNK_TILE_NUM_DIM - 1u) - tx);
  let dz = min(tz, f32(CHUNK_TILE_NUM_DIM - 1u) - tz);
  let min_dist = min(dx, dz);
  return 1.0 - smoothstep(0.0, 2.0, min_dist);
}

// ============================================================================
// Random / noise helpers
//  - hash + smooth value noise
//  - FBM (fractal Brownian motion)
//  - "Billow" transform (|2n-1|) to get fluffy cloud lobes
//  - Domain warp (offset the sampling coords by a low-frequency field)
// ============================================================================

fn hash(p: vec2<f32>) -> f32 {
  // Cheap hash → [0,1). Good enough for value noise base.
  let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
  let p3s = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
  return fract((p3s.x + p3s.y) * p3s.z);
}

// Smooth value noise in [0,1]
fn noise_2d(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f); // smoothstep-like fade
  let a = hash(i + vec2<f32>(0.0, 0.0));
  let b = hash(i + vec2<f32>(1.0, 0.0));
  let c = hash(i + vec2<f32>(0.0, 1.0));
  let d = hash(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 3–4 octave FBM; returns ~[0,1] range
fn fbm_value(p: vec2<f32>) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var f   = 1.0;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);  f *= 2.0; amp *= 0.5;
  sum += amp * noise_2d(p * f);
  return clamp(sum, 0.0, 1.0);
}

// Billow transform → fluffy “cloud”-like blobs (still ~[0,1])
fn fbm_billow(p: vec2<f32>) -> f32 {
  let n = fbm_value(p);
  return 1.0 - abs(2.0 * n - 1.0);
}

// Domain warp: offset p by two low-frequency FBMs to break grid patterns.
fn domain_warp(p: vec2<f32>, strength: f32) -> vec2<f32> {
  let w1 = fbm_value(p * 0.5 + vec2<f32>(13.37, -7.21));
  let w2 = fbm_value(p * 0.5 + vec2<f32>(-5.73, 4.11));
  return p + vec2<f32>(w1, w2) * strength;
}

// ============================================================================
// Cubic interpolation (value + derivative) for heightfield normals
// ============================================================================

fn cubic_interp_value_and_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> vec2<f32> {
  // Catmull-Rom-like cubic with derivative output.
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

// ============================================================================
// Normal utilities (geometric, bicubic, bent)
// ============================================================================

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

fn get_bicubic_normal(world_pos: vec3<f32>) -> vec3<f32> {
  // Smooth analytic normal via bicubic interpolation of the 13×13 tile heights.
  // Greatly reduces shading “jaggies” compared to geometric normal above.
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

// ------------------------------- Bent normals --------------------------------
/*
 OLD (for reference):
   - Looked at *positive* height steps around center and *summed* them.
   - Mix factor = (sum of positive neighbor deltas) * 0.25 → could flip
     direction depending on which side had the larger positive step.
   - Result: on steep, step-like terrain, adjacent triangles sometimes “chose”
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
  let mix_factor = k * 0.45;             // conservative bend to avoid “melting”

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
  // Sky from +Y, ground from -Y, blended by “upness”.
  let upness = clamp(dot(normalize(N), vec3<f32>(0.0,1.0,0.0)) * 0.5 + 0.5, 0.0, 1.0);
  let sky    = lighting.fill_sky_color.rgb    * lighting.fill_sky_color.a;
  let ground = lighting.fill_ground_color.rgb * lighting.fill_ground_color.a;
  return mix(ground, sky, upness);
}

// Contrast S-curve with neutral at contrast=1.0 (k = contrast-1 in [-1,1])
// NOTE: Do NOT clamp here — keep HDR headroom pre-tonemap.
fn apply_contrast_neutral(x: vec3<f32>, contrast: f32) -> vec3<f32> {
  let t = clamp(contrast - 1.0, -1.0, 1.0);
  // y = x + t * (x - x*x) * 2  (S-curve around ~0.5; zero effect when t=0)
  return x + t * ((x - x * x) * 2.0);
}

// Strong, stylized color grading (vibrant), with:
//  - truly neutral contrast=1 (no clamp inside),
//  - multiplicative split-toning normalized to luma=1 so mid-grays stay neutral.
fn grade_color_vibrant(color_in: vec3<f32>) -> vec3<f32> {
  let strength    = lighting.grade_params.x;  // overall grade amount
  let vibrance    = lighting.grade_extra.x;   // selective saturation
  let saturation  = lighting.grade_extra.y;   // global saturation
  let contrast    = lighting.grade_extra.z;   // S-curve; 1.0 = neutral
  let split_str   = lighting.grade_extra.w;   // split-toning strength

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
  return max(graded, vec3<f32>(0.0));
}

// Gloom: general, height-fading, optional shadow bias.
// gloom_params: [amount, height_falloff_height, shadow_bias, fog_height_bias]
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
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == 1u) {
    if (use_linear) {
      return textureSample(tex_big, tex_small_sampler, uv, layer).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_big));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_big, iuv, layer, 0).rgb;
    }
  } else {
    if (use_linear) {
      return textureSample(tex_small, tex_small_sampler, uv, layer).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_small));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_small, iuv, layer, 0).rgb;
    }
  }
}

// Bicubic reconstruction for albedo (smooth)
fn sample_tile_bicubic(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let dims = select(vec2<f32>(textureDimensions(tex_small)), vec2<f32>(textureDimensions(tex_big)), tile.texture_size == 1u);
  let f_uv = uv * dims - 0.5;
  let i_uv = floor(f_uv);
  let f = fract(f_uv);

  var result = vec3<f32>(0.0);
  var total_weight = 0.0;

  for (var j: i32 = -1; j <= 2; j++) {
    let v_weight = cubic_weight(f.y, f32(j));
    for (var i: i32 = -1; i <= 2; i++) {
        let h_weight = cubic_weight(f.x, f32(i));
        let weight = h_weight * v_weight;
        let sample_uv = (i_uv + vec2<f32>(f32(i), f32(j)) + 0.5) / dims;
        result += sample_tile_albedo(clamp(sample_uv, vec2<f32>(0.0), vec2<f32>(1.0)), tile) * weight;
        total_weight += weight;
    }
  }
  return result / total_weight;
}

fn cubic_weight(f: f32, i: f32) -> f32 {
    let x = abs(f - i);
    if (x <= 1.0) {
        return (1.5 * x - 2.5) * x * x + 1.0;
    } else if (x <= 2.0) {
        return ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0;
    }
    return 0.0;
}

// Simplified Edge-Adaptive Reconstruction (FSR-like)
fn sample_tile_fsr(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let dims = select(vec2<f32>(textureDimensions(tex_small)), vec2<f32>(textureDimensions(tex_big)), tile.texture_size == 1u);
  let pos = uv * dims;
  let i_pos = vec2<i32>(floor(pos));
  let f = fract(pos);

  // 4 main taps
  let c00 = sample_tile_albedo_at(i_pos + vec2<i32>(0, 0), tile);
  let c10 = sample_tile_albedo_at(i_pos + vec2<i32>(1, 0), tile);
  let c01 = sample_tile_albedo_at(i_pos + vec2<i32>(0, 1), tile);
  let c11 = sample_tile_albedo_at(i_pos + vec2<i32>(1, 1), tile);

  // Calculate luma-based gradients
  let l00 = luminance(c00);
  let l10 = luminance(c10);
  let l01 = luminance(c01);
  let l11 = luminance(c11);

  // Horizontal/Vertical differences
  let gx = abs(l10 - l00) + abs(l11 - l01);
  let gy = abs(l01 - l00) + abs(l11 - l10);

  // Edge-aware weighting
  let wx = 1.0 / (1.0 + gx * 4.0);
  let wy = 1.0 / (1.0 + gy * 4.0);

  // Bilinear blend biased by edges
  let res = mix(mix(c00, c10, f.x * wx), mix(c01, c11, f.x * wx), f.y * wy);
  return res / (mix(mix(1.0, wx, f.x), mix(1.0, wx, f.x), f.y) * wy); // approximate normalization
}

fn sample_tile_albedo_at(iuv: vec2<i32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 1u) {
    let dims = vec2<i32>(textureDimensions(tex_big));
    return textureLoad(tex_big, clamp(iuv, vec2<i32>(0), dims - 1), layer, 0).rgb;
  } else {
    let dims = vec2<i32>(textureDimensions(tex_small));
    return textureLoad(tex_small, clamp(iuv, vec2<i32>(0), dims - 1), layer, 0).rgb;
  }
}

fn sample_tile_reconstructed(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
    let mode = effects.reconstruction_mode;
    if (mode == 1u) {
        return sample_tile_bicubic(uv, tile);
    } else if (mode == 2u) {
        // We do FSR reconstruction by applying bicubic then sharpening,
        // OR using a dedicated edge-aware sampler.
        // For now, let's use the edge-adaptive one.
        // IMPORTANT TODO !!!!! this is just a false AMD FSR! Implement the real one!
        return sample_tile_fsr(uv, tile);
    } else {
        return sample_tile_albedo(uv, tile);
    }
}

// Simple sharpening filter (Unsharp Masking style)
fn apply_sharpening(color: vec3<f32>, uv: vec2<f32>, tile: TileUniform, amount: f32) -> vec3<f32> {
  if (amount <= 0.0) { return color; }

  // Approximate a 1-pixel offset in UV space
  let fw = fwidth(uv);
  let off = max(fw.x, fw.y);

  let s1 = sample_tile_albedo(uv + vec2<f32>(off, 0.0), tile);
  let s2 = sample_tile_albedo(uv - vec2<f32>(off, 0.0), tile);
  let s3 = sample_tile_albedo(uv + vec2<f32>(0.0, off), tile);
  let s4 = sample_tile_albedo(uv - vec2<f32>(0.0, off), tile);

  let neighbor_avg = (s1 + s2 + s3 + s4) * 0.25;
  return color + (color - neighbor_avg) * amount;
}

// Same as above, but with explicit gradients to keep LOD stable across taps.
// NOTE: the WGSL signature is textureSampleGrad(tex, sampler, uv, layer, ddx, ddy).
fn sample_tile_albedo_grad(uv: vec2<f32>, tile: TileUniform, ddx_uv: vec2<f32>, ddy_uv: vec2<f32>) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == 1u) {
    if (use_linear) {
      return textureSampleGrad(tex_big,   tex_small_sampler, uv, layer, ddx_uv, ddy_uv).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_big));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_big, iuv, layer, 0).rgb;
    }
  } else {
    if (use_linear) {
      return textureSampleGrad(tex_small, tex_small_sampler, uv, layer, ddx_uv, ddy_uv).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_small));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_small, iuv, layer, 0).rgb;
    }
  }
}

// cheap per-tile random in [0,1)
fn rand01_from_tile(ix: i32, iz: i32, layer: u32) -> f32 {
  let p = vec2<f32>(f32(ix) + f32(layer) * 0.618, f32(iz) + f32(layer) * 1.732);
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// 9-tap blur with radius in *screen pixels* via fwidth — visible regardless of UV scale.
// LOD is kept stable by using the same gradients for all taps.
// 9-tap blur with decorrelated directions per tile
fn blurred_albedo(uv: vec2<f32>, tile: TileUniform, radius_in_pixels: f32, world_xz: vec2<f32>) -> vec3<f32> {
  // Approximate one screen pixel in UV space for this fragment
  let fw = fwidth(uv);
  let px_uv = max(fw.x, fw.y) + 1e-6;

  // Ensure at least half-pixel radius so it’s *noticeable* even at low zoom
  let min_px = 0.5;
  let r = max(radius_in_pixels, min_px) * px_uv;

  // Keep LOD stable for all taps
  let ddx_uv = dpdx(uv);
  let ddy_uv = dpdy(uv);

  // decorrelation: derive a tiny rotation per tile/layer (+tiny world jitter)
  let jitter = rand01_from_tile(i32(floor(world_xz.x)), i32(floor(world_xz.y)), tile.texture_layer)
             + fract(world_xz.x * 0.173 + world_xz.y * 0.271) * 0.125;
  let ang = (jitter * 6.2831853); // 2π
  let ca = cos(ang);
  let sa = sin(ang);
  let rot = mat2x2<f32>(ca, -sa, sa, ca);

  // rotated offsets
  let o1 = rot * vec2<f32>( r, 0.0);
  let o2 = rot * vec2<f32>(-r, 0.0);
  let o3 = rot * vec2<f32>(0.0,  r);
  let o4 = rot * vec2<f32>(0.0, -r);
  let o5 = rot * vec2<f32>( r,  r);
  let o6 = rot * vec2<f32>(-r,  r);
  let o7 = rot * vec2<f32>( r, -r);
  let o8 = rot * vec2<f32>(-r, -r);

  // Clamp taps to [0,1] to avoid bleeding across tile edges if sampler wraps
  let c  = clamp(uv,                 vec2<f32>(0.0), vec2<f32>(1.0));
  let u1 = clamp(uv + o1,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u2 = clamp(uv + o2,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u3 = clamp(uv + o3,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u4 = clamp(uv + o4,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u5 = clamp(uv + o5,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u6 = clamp(uv + o6,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u7 = clamp(uv + o7,            vec2<f32>(0.0), vec2<f32>(1.0));
  let u8 = clamp(uv + o8,            vec2<f32>(0.0), vec2<f32>(1.0));

  // Slightly stronger normalized kernel to make effect pop
  let wc = 0.20;
  let w1 = 0.12; let w2 = 0.12; let w3 = 0.12; let w4 = 0.12;
  let w5 = 0.08; let w6 = 0.08; let w7 = 0.08; let w8 = 0.08;

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
//  - Also fixes zig-zag visible in classic Gouraud path by using the same
//    smoothed/bent normal pipeline here when enabled.
// ============================================================================

@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var out: VertexOutput;

  let shading_mode: u32 = effects.shading_mode;
  let normal_mode:  u32 = effects.normal_mode;
  let enable_bent:  u32 = effects.enable_bent;

  // Apply mesh local_to_world ON THE FLAT GRID FIRST to get actual world tile coords
  let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
  var flat_local_pos = in.position;
  flat_local_pos.y = 0.0;
  let flat_world_pos = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(flat_local_pos, 1.0));

  let wx = i32(round(flat_world_pos.x));
  let wz = i32(round(flat_world_pos.z));
  let final_y = atlas_read_height(wx, wz);

  var final_world_pos = flat_world_pos;
  final_world_pos.y += final_y;

  out.world_position = final_world_pos;
  out.position       = view_transformations::position_world_to_clip(out.world_position.xyz);
  out.uv = in.uv;
  out.instance_index = in.instance_index;

  // Base geometric normal (fast)
  let geometric_normal_local = get_geometric_normal_local(out.world_position.xyz);
  var Nw = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);

  // Optional smooth/bicubic normal with edge blend to avoid seams
  if (normal_mode == 1u) {
    let smooth_local = get_bicubic_normal(out.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(out.world_position.x, out.world_position.z);
    Nw = normalize(mix(smooth_world, Nw, blend_edge));
  }

  // Optional bent normal (same as fragment path)
  if (enable_bent == 1u) {
    Nw = get_bent_normal(out.world_position.xyz, Nw);
  }

  out.world_normal = Nw;

  // Classic vertex path: precompute lambert in uv_b.x using the final Nw
  out.uv_b = vec2<f32>(0.0, 0.0);
  if (shading_mode == 0u) {
    out.uv_b.x = get_lambert(Nw, scene.light_direction);
  }

  return out;
}

// ============================================================================
// Shading models
// ============================================================================

fn shade_mode0_classic_vertex(base_albedo: vec3<f32>, lam_v: f32,
                              ambient_strength: f32, diffuse_strength: f32) -> vec3<f32> {
  // Simple brightness model: albedo * (ambient + diffuse * N·L)
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

  // Diffuse shaping (raises Lambert to sharpen sun side a bit)
  let lam = get_lambert(Nw, L);
  let lam_sharp  = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
  let lam_shaped = mix(lam, lam_sharp, clamp(sharpness_mix, 0.0, 0.4));

  // Energy RGB (allow warm key light tint)
  let diffuse_rgb = lighting.light_color * (diffuse_strength * lam_shaped);
  let hemi_rgb = select(vec3<f32>(0.0), (get_hemisphere_fill(Nw) * fill_strength), (fill_strength > 0.0));

  // Base energy is RGB now: ambient (scalar) + fill luma + tinted diffuse
  let hemi_luma = luminance(hemi_rgb);
  var energy_rgb = vec3<f32>(ambient_strength + hemi_luma) + diffuse_rgb;

  // Clamp energy to keep room if headroom enabled (prevents bleaching)
  // Headroom limiting via runtime toggle only
  let headroom_reserve = 0.10;
  if (lighting.grade_params.w >= 0.5) {
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
  if (runtime_headroom_on) {
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
  let shading_mode   = effects.shading_mode;
  let enable_tonemap = effects.enable_tonemap;
  let enable_grading = effects.enable_grading;

  // ---- Zoom-based shader LOD: disable expensive features when zoomed out ----
  let zoom = scene.render_zoom;
  // zoom > 5  : bicubic/FSR reconstruction → nearest (saves ~15 tex reads)
  // zoom > 10 : disable blur + sharpening + bicubic normals (saves ~30 tex reads)
  // zoom > 20 : disable bent normals (saves ~4 tex reads)
  // zoom > 30 : disable volumetric fog → flat fog (saves heavy FBM ALU)
  var normal_mode    = effects.normal_mode;
  var enable_bent    = effects.enable_bent;
  var enable_fog     = effects.enable_fog;
  var enable_gloom   = effects.enable_gloom;
  var enable_blur    = effects.enable_blur;
  var force_nearest  = false;
  var disable_sharpen = false;
  var disable_volumetric = false;

  if (zoom > 5.0) {
    force_nearest = true;  // skip bicubic/FSR, use direct nearest sample
  }
  if (zoom > 10.0) {
    enable_blur = 0u;      // skip 9-tap blur
    disable_sharpen = true; // skip sharpening (4 extra taps)
    normal_mode = 0u;       // geometric normals only (skip bicubic 4×4 kernel)
  }
  if (zoom > 20.0) {
    enable_bent = 0u;       // skip bent normals (4 extra height reads)
  }
  if (zoom > 30.0) {
    disable_volumetric = true; // fall back to flat fog (skip domain-warp FBM)
  }

  let ambient_strength  = effects.ambient_strength;
  let diffuse_strength  = effects.diffuse_strength;
  let specular_strength = effects.specular_strength;
  let rim_strength      = effects.rim_strength;

  let fill_strength     = effects.fill_strength;
  let sharpness_factor  = effects.sharpness_factor;
  let sharpness_mix     = effects.sharpness_mix;

  let blur_strength     = effects.blur_strength;
  let blur_radius       = effects.blur_radius;

  let exposure          = lighting.exposure;

  // Local coords and tile selection
  let uv_in_tile = vec2<f32>(fract(in.world_position.x), fract(in.world_position.z));
  let tile = atlas_read_meta(i32(floor(in.world_position.x)), i32(floor(in.world_position.z)));
  let base_alpha: f32 = 1.0; // tile textures assumed opaque for terrain

  // Zoom-adaptive cheap path:
  // IMPORTANT: this branch happens BEFORE expensive reconstruction/blur/sharpen/lighting,
  // so skipped pixels are truly cheaper to compute.
  let adaptive = clamp(scene.adaptive_zoom_simplification, 0.0, 1.0);
  var use_cheap_path = false;
  if (adaptive > 0.001) {
    let px = i32(in.position.x);
    let py = i32(in.position.y);
    var skip_expensive = false;

    if (adaptive < 0.5) {
      // 50% checkerboard keep-rate.
      skip_expensive = ((px + py) & 1) != 0;
    } else {
      // ~25% keep-rate: only one pixel every 2x2 quad keeps full shading.
      skip_expensive = ((px & 1) != 0) || ((py & 1) != 0);
    }

    if (skip_expensive) {
      use_cheap_path = true;
    }
  }

  // Base albedo
  var base_albedo = vec3<f32>(0.0);
  if (use_cheap_path) {
    // Quantize UV to a coarser grid and use direct nearest sample.
    // We still run full lighting/fog/shadows below to keep visual consistency.
    let cells = mix(2.0, 4.0, smoothstep(0.0, 1.0, adaptive));
    let uv_q = (floor(uv_in_tile * cells) + vec2<f32>(0.5)) / cells;
    base_albedo = sample_tile_albedo(uv_q, tile);
  } else {
    // force_nearest: skip bicubic/FSR at high zoom to save ~15 tex reads per pixel
    if (force_nearest) {
      base_albedo = sample_tile_albedo(uv_in_tile, tile);
    } else {
      base_albedo = sample_tile_reconstructed(uv_in_tile, tile);
    }
    if (enable_blur == 1u && blur_strength > 0.001 && blur_radius > 0.0) {
      let blurred = blurred_albedo(uv_in_tile, tile, blur_radius, vec2<f32>(in.world_position.x, in.world_position.z));
      base_albedo = mix(base_albedo, blurred, clamp(blur_strength, 0.0, 1.0));
    }
    if (!disable_sharpen && effects.sharpening_amount > 0.0) {
      base_albedo = apply_sharpening(base_albedo, uv_in_tile, tile, effects.sharpening_amount);
    }
  }

  // Normals: we already computed in vertex and passed in.world_normal.
  // For non-classic modes we can still override with bicubic if desired.
  var Nw = normalize(in.world_normal);
  if (normal_mode == 1u) {
    let smooth_local = get_bicubic_normal(in.world_position.xyz);
    let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
    let blend_edge = chunk_edge_blend_factor(in.world_position.x, in.world_position.z);
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

  // Apply global scene lighting scaler (UI: "Global Lighting / Scene Luminosity")
  hdr_rgb *= max(scene.global_lighting, 0.0);

// ----------------------------------------------------------------------------
  // Fog (NEW implementation)
  // ----------------------------------------------------------------------------
  // IMPORTANT: UI-controlled inputs are remapped to internal ranges for intuitive control:
  // - lighting.fog_params.x -> distance_density (0..~0.2 in UI) mapped to fog_end distance
  // - lighting.fog_params.y -> height_density (0..~0.2 in UI) mapped to vertical falloff
  // - lighting.fog_params.z -> noise_scale (0..2 in UI) mapped to world noise scale (bigger => coarser clouds)
  // - lighting.fog_params.w -> noise_strength (0..1 in UI) mapped to cloud contrast/detail/coverage
  if (enable_fog == 1u) {
    // Read raw UI uniforms (defensive clamps)
    let dist_density_ui   = clamp(lighting.fog_params.x, 0.0, 1.0);  // user slider 0..0.2 but clamp anyway
    let height_density_ui = clamp(lighting.fog_params.y, 0.0, 1.0);
    let noise_scale_ui    = clamp(lighting.fog_params.z, 0.0, 2.0);
    let noise_strength_ui = clamp(lighting.fog_params.w, 0.0, 1.0);

    // Fog height bias: -1 valley, 0 neutral, +1 high-alt haze
    let hBias = clamp(lighting.gloom_params.w, -1.0, 1.0);
    let high_w = max(hBias, 0.0);
    let low_w  = max(-hBias, 0.0);

    // -------------------------
    // Distance mapping: translate UI density -> fog_end (meters)
    // Small UI values -> very far (clear). Larger UI -> closer fog end.
    // tweak these ranges if your world units are scaled differently.
    let fog_end = mix(6000.0, 40.0, smoothstep(0.0, 0.2, dist_density_ui));
    let fog_start = max(0.0, fog_end * 0.06);
    let d = length(in.world_position.xyz - scene.camera_position);
    // softer ramp for distance-based fog
    let dist_factor = smoothstep(fog_start, fog_end, d);

    // -------------------------
    // Height mapping: UI -> falloff scale in meters (0.2 UI -> short falloff)
    let height_falloff = mix(800.0, 6.0, smoothstep(0.0, 0.2, height_density_ui));
    let y = in.world_position.y;
    var height_term_high = 0.0;
    var height_term_low  = 0.0;
    if (height_density_ui > 1e-6) {
      height_term_high = 1.0 - exp(-max(y, 0.0) / height_falloff);
      height_term_low  = 1.0 - exp(-max(-y, 0.0) / height_falloff);
    }
    let height_factor = clamp(high_w * height_term_high + low_w * height_term_low, 0.0, 1.0);

    // -------------------------
    // Combine distance & height (union) -> base_fog (0..1)
    let base_fog = clamp(dist_factor + height_factor - dist_factor * height_factor, 0.0, 1.0);

    // -------------------------
    // Noise / clouds — robust mapping
    // Map user noise_scale_ui [0..2] to an internal world-scale: smaller value -> finer clouds,
    // larger value -> coarser/bigger clouds. This remapping makes slider intuitive.
    let noise_scale_world = mix(0.004, 0.25, clamp(noise_scale_ui / 2.0, 0.0, 1.0));
    // noise_strength controls contrast/coverage/detail
    let noise_strength = noise_strength_ui;

    // Build wind & time for animation.
    // Uses Bevy's built-in globals.time (auto-updated every frame, wraps at 1h).
    let base_time_speed = 0.02; // base slow speed
    let time_speed = base_time_speed + noise_strength * 0.08;
    let t = globals.time * time_speed;

    // Wind derived from sun direction (perpendicular flow across sun)
    let sun2 = normalize(vec2<f32>(L.x, L.z));
    // defensively handle degenerate light_dir
    var wind = vec2<f32>(0.7, 0.3);
    if (length(sun2) > 1e-5) {
      wind = normalize(vec2<f32>(L.z, -L.x));
    }

    // Sample coords in world meters; domain-warp to break tiling
    // Note: multiply world.xz by noise_scale_world to get appropriate density
    let p0 = (in.world_position.xz * noise_scale_world) + wind * (t * 6.0);

    // Gentle domain warp — smaller strength so we don't over-warp tiny structures
    let warp_strength = 0.4 * noise_strength + 0.08;
    let pWarp = domain_warp(p0, warp_strength);

    // Multi-scale billow FBM to get both big blobs and small fluff
    let n1 = fbm_billow(pWarp * 1.0);
    let n2 = fbm_billow(pWarp * 2.3) * 0.55;
    let n_billow = clamp(n1 * 0.7 + n2 * 0.3, 0.0, 1.0);

    // Control coverage/contrast:
    // - Lower threshold -> more coverage.
    // - width controls softness of cloud edges.
    let base_threshold = mix(0.72, 0.46, noise_strength); // higher noise_strength -> lower threshold -> more clouds
    let edge_width = mix(0.12, 0.20, 1.0 - noise_strength); // stronger noise -> sharper edges (smaller width)
    let cloud_mask = smoothstep(base_threshold, base_threshold + edge_width, n_billow);

    // Baseline ensures there are some clear areas when desired:
    let baseline = mix(0.03, 0.18, 1.0 - noise_strength); // low baseline -> more clear sky by default
    let peak_gain = mix(1.0, 1.8, noise_strength);

    let cloud_mod = baseline + (peak_gain - baseline) * cloud_mask;

    // Combine
    var fog_factor = clamp(base_fog * cloud_mod, 0.0, 1.0);

    // Subtle breathing so it doesn't look totally static; scaled by noise_strength
    let breath = 0.5 + 0.5 * sin(globals.time * (0.06 + 0.02 * noise_strength) + (hash(in.world_position.xz * 0.11) * 6.2831));
    fog_factor = clamp(fog_factor * mix(0.97, 1.03, (breath - 0.5) * 0.6 * noise_strength), 0.0, 1.0);

    // Final cap set by UI alpha
    let fog_mix = clamp(fog_factor * lighting.fog_color.a, 0.0, 1.0);

    // If user opted out of volumetric noise OR zoom-LOD disabled it, use flat fog:
    if (USE_VOLUMETRIC_NOISE == 1u && !disable_volumetric) {
      hdr_rgb = mix(hdr_rgb, lighting.fog_color.rgb, fog_mix);
    } else {
      // simple fallback: linearized distance*height blend capped by alpha
      let flat_mix = clamp(base_fog * lighting.fog_color.a, 0.0, 1.0);
      hdr_rgb = mix(hdr_rgb, lighting.fog_color.rgb, flat_mix);
    }
  }

  // Grading (vibrant with neutral contrast) + Tonemap
  var post = hdr_rgb;
  if (enable_grading == 1u) {
    post = grade_color_vibrant(post); // no pre-tonemap clamping
  }
  var final_rgb = post;
  if (enable_tonemap == 1u) {
    final_rgb = tonemap_reinhard_with_exposure(max(post, vec3<f32>(0.0)), exposure);
  }

  final_rgb = max(final_rgb, vec3<f32>(0.0));
  return vec4<f32>(final_rgb, base_alpha);
}
