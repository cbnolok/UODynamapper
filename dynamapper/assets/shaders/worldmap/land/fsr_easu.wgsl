// ============================================================================
// land::fsr_easu — FSR (FidelityFX Super Resolution) EASU pass
//
//  Edge Adaptive Spatial Upsampling — a 12-tap directional kernel that
//  detects edges via luma gradients, stretches along them, and applies a
//  Lanczos-like window to preserve detail while avoiding ringing.
//
//  Ported from AMD FidelityFX FSR 1.0 (MIT license).
//  Reference: https://www.shadertoy.com/view/stXSWB
// ============================================================================

#import "shaders/worldmap/land/land_bindings.wgsl"::{TileUniform, tex_small, tex_big, land_page_atlas, tex_small_sampler}
#import "shaders/worldmap/land/lighting.wgsl"::{luminance}

// ============================================================================
// Texel fetch helper — integer-coordinate nearest tap
// ============================================================================

fn fsr_fetch(iuv: vec2<i32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    let local_iuv = clamp(iuv, vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
    let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
    return textureLoad(land_page_atlas, atlas_iuv, layer, 0).rgb;
  }
  if (tile.texture_size == 1u) {
    let dims = vec2<i32>(textureDimensions(tex_big));
    return textureLoad(tex_big, clamp(iuv, vec2<i32>(0), dims - 1), layer, 0).rgb;
  } else {
    let dims = vec2<i32>(textureDimensions(tex_small));
    return textureLoad(tex_small, clamp(iuv, vec2<i32>(0), dims - 1), layer, 0).rgb;
  }
}

// ============================================================================
// EASU constants setup
// ============================================================================

struct FsrEasuCon {
  con0_: vec4<f32>,
  con1_: vec4<f32>,
  con2_: vec4<f32>,
  con3_: vec4<f32>,
}

fn fsr_easu_con(
  input_viewport: vec2<f32>,
  input_size: vec2<f32>,
  output_size: vec2<f32>,
) -> FsrEasuCon {
  var c: FsrEasuCon;

  // Output integer position to a pixel position in viewport.
  c.con0_ = vec4<f32>(
    input_viewport.x / output_size.x,
    input_viewport.y / output_size.y,
    0.5 * input_viewport.x / output_size.x - 0.5,
    0.5 * input_viewport.y / output_size.y - 0.5,
  );

  // Viewport pixel position to normalized image space.
  let inv_input = 1.0 / input_size;
  c.con1_ = vec4<f32>(inv_input.x, inv_input.y, inv_input.x, -inv_input.y);

  // Centers of gather4, offsets from upper-left of 'F'.
  c.con2_ = vec4<f32>(-inv_input.x, 2.0 * inv_input.y, inv_input.x, 2.0 * inv_input.y);
  c.con3_ = vec4<f32>(0.0, 4.0 * inv_input.y, 0.0, 0.0);

  return c;
}

// ============================================================================
// EASU tap — accumulates weighted color using a Lanczos-like window
// ============================================================================

fn fsr_easu_tap(
  aC: ptr<function, vec3<f32>>,
  aW: ptr<function, f32>,
  off: vec2<f32>,   // Pixel offset from resolve position to tap.
  dir: vec2<f32>,   // Gradient direction.
  len: vec2<f32>,   // Anisotropic length.
  lob: f32,         // Negative lobe strength.
  clp: f32,         // Clipping point.
  c: vec3<f32>,     // Tap color.
) {
  // Rotate offset by direction.
  let v = vec2<f32>(
    dot(off, dir),
    dot(off, vec2<f32>(-dir.y, dir.x)),
  ) * len;

  // Distance² clamped to window.
  let d2 = min(dot(v, v), clp);

  // Approximation of Lanczos2 without sin/rcp/sqrt:
  //  (25/16 * (2/5 * x² - 1)² - (25/16 - 1)) * (1/4 * x² - 1)²
  var wB = 0.4 * d2 - 1.0;
  var wA = lob * d2 - 1.0;
  wB *= wB;
  wA *= wA;
  wB = 1.5625 * wB - 0.5625;
  let w = wB * wA;

  *aC += c * w;
  *aW += w;
}

// ============================================================================
// EASU direction/length accumulation
// ============================================================================

fn fsr_easu_set(
  dir: ptr<function, vec2<f32>>,
  len: ptr<function, f32>,
  w: f32,
  lA: f32, lB: f32, lC: f32, lD: f32, lE: f32,
) {
  // Direction from the '+' pattern:
  //    a
  //  b c d
  //    e
  let lenX = max(abs(lD - lC), abs(lC - lB));
  let dirX = lD - lB;
  (*dir).x += dirX * w;
  var lx = 0.0;
  if (lenX > 0.0) {
    lx = clamp(abs(dirX) / lenX, 0.0, 1.0);
  }
  lx *= lx;
  *len += lx * w;

  let lenY = max(abs(lE - lC), abs(lC - lA));
  let dirY = lE - lA;
  (*dir).y += dirY * w;
  var ly = 0.0;
  if (lenY > 0.0) {
    ly = clamp(abs(dirY) / lenY, 0.0, 1.0);
  }
  ly *= ly;
  *len += ly * w;
}

// ============================================================================
// Approximate luma: green + half of (red + blue)
// ============================================================================

fn fsr_luma(c: vec3<f32>) -> f32 {
  return c.g + 0.5 * (c.r + c.b);
}

// ============================================================================
// EASU main — 12-tap edge-adaptive upsampling kernel
// ============================================================================

fn fsr_easu(
  ip: vec2<f32>,   // Integer pixel position in output space.
  con: FsrEasuCon,
  tile: TileUniform,
  dims: vec2<f32>,
) -> vec3<f32> {
  // Map output pixel to input pixel/subpixel.
  let pp_raw = ip * con.con0_.xy + con.con0_.zw;
  let fp = floor(pp_raw);
  let pp = pp_raw - fp;

  // Gather base positions.
  let p0 = fp * con.con1_.xy + con.con1_.zw;
  let p1 = p0 + con.con2_.xy;
  let p2 = p0 + con.con2_.zw;
  let p3 = p0 + con.con3_.xy;

  // Offset for manual "gather4" emulation.
  let off = vec4<f32>(-0.5, 0.5, -0.5, 0.5) * vec4<f32>(con.con1_.x, con.con1_.x, con.con1_.y, con.con1_.y);

  // 12-tap fetch: b c / e f g h / i j k l / n o
  let i_fp = vec2<i32>(fp);
  let bC = fsr_fetch(vec2<i32>((p0 + off.xw) * dims), tile); let bL = fsr_luma(bC);
  let cC = fsr_fetch(vec2<i32>((p0 + off.yw) * dims), tile); let cL = fsr_luma(cC);
  let iC = fsr_fetch(vec2<i32>((p1 + off.xw) * dims), tile); let iL = fsr_luma(iC);
  let jC = fsr_fetch(vec2<i32>((p1 + off.yw) * dims), tile); let jL = fsr_luma(jC);
  let fC = fsr_fetch(vec2<i32>((p1 + off.yz) * dims), tile); let fL = fsr_luma(fC);
  let eC = fsr_fetch(vec2<i32>((p1 + off.xz) * dims), tile); let eL = fsr_luma(eC);
  let kC = fsr_fetch(vec2<i32>((p2 + off.xw) * dims), tile); let kL = fsr_luma(kC);
  let lC = fsr_fetch(vec2<i32>((p2 + off.yw) * dims), tile); let lL = fsr_luma(lC);
  let hC = fsr_fetch(vec2<i32>((p2 + off.yz) * dims), tile); let hL = fsr_luma(hC);
  let gC = fsr_fetch(vec2<i32>((p2 + off.xz) * dims), tile); let gL = fsr_luma(gC);
  let oC = fsr_fetch(vec2<i32>((p3 + off.yz) * dims), tile); let oL = fsr_luma(oC);
  let nC = fsr_fetch(vec2<i32>((p3 + off.xz) * dims), tile); let nL = fsr_luma(nC);

  // Accumulate direction and length from the 4 bilinear sub-regions.
  var dir = vec2<f32>(0.0);
  var len_acc = 0.0;

  fsr_easu_set(&dir, &len_acc, (1.0 - pp.x) * (1.0 - pp.y), bL, eL, fL, gL, jL);
  fsr_easu_set(&dir, &len_acc,        pp.x  * (1.0 - pp.y), cL, fL, gL, hL, kL);
  fsr_easu_set(&dir, &len_acc, (1.0 - pp.x) *        pp.y,  fL, iL, jL, kL, nL);
  fsr_easu_set(&dir, &len_acc,        pp.x  *        pp.y,   gL, jL, kL, lL, oL);

  // Normalize direction, guard near-zero.
  let dir2 = dir * dir;
  let dirR_sq = dir2.x + dir2.y;
  let zro = dirR_sq < (1.0 / 32768.0);
  let dirR = inverseSqrt(max(dirR_sq, 1.0 / 32768.0));
  var norm_dir = select(dir * dirR, vec2<f32>(1.0, 0.0), zro);

  // Shape length: {0..2} → {0..1}, squared.
  var len_shaped = len_acc * 0.5;
  len_shaped *= len_shaped;

  // Stretch kernel along edge: {1.0 vert|horz} to {sqrt(2) on diagonal}.
  let stretch = dot(norm_dir, norm_dir) / max(abs(norm_dir.x), abs(norm_dir.y));

  // Anisotropic length:
  //   x = 1.0 lerp to 'stretch' on edges
  //   y = 1.0 lerp to 2× on edges
  let len2 = vec2<f32>(
    1.0 + (stretch - 1.0) * len_shaped,
    1.0 - 0.5 * len_shaped,
  );

  // Window shift: +/-{sqrt(2) to slightly beyond 2.0} based on edge amount.
  let lob = 0.5 - 0.29 * len_shaped;
  let clp = 1.0 / lob;

  // De-ringing bounds from 4 nearest texels.
  let min4 = min(min(fC, gC), min(jC, kC));
  let max4 = max(max(fC, gC), max(jC, kC));

  // Accumulate 12 weighted taps.
  var aC = vec3<f32>(0.0);
  var aW = 0.0;
  fsr_easu_tap(&aC, &aW, vec2<f32>( 0.0, -1.0) - pp, norm_dir, len2, lob, clp, bC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 1.0, -1.0) - pp, norm_dir, len2, lob, clp, cC);
  fsr_easu_tap(&aC, &aW, vec2<f32>(-1.0,  1.0) - pp, norm_dir, len2, lob, clp, iC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 0.0,  1.0) - pp, norm_dir, len2, lob, clp, jC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 0.0,  0.0) - pp, norm_dir, len2, lob, clp, fC);
  fsr_easu_tap(&aC, &aW, vec2<f32>(-1.0,  0.0) - pp, norm_dir, len2, lob, clp, eC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 1.0,  1.0) - pp, norm_dir, len2, lob, clp, kC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 2.0,  1.0) - pp, norm_dir, len2, lob, clp, lC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 2.0,  0.0) - pp, norm_dir, len2, lob, clp, hC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 1.0,  0.0) - pp, norm_dir, len2, lob, clp, gC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 1.0,  2.0) - pp, norm_dir, len2, lob, clp, oC);
  fsr_easu_tap(&aC, &aW, vec2<f32>( 0.0,  2.0) - pp, norm_dir, len2, lob, clp, nC);

  // Normalize and de-ring: clamp to [min4, max4] of nearest texels.
  return min(max4, max(min4, aC / aW));
}

// ============================================================================
// Public entry point — called from sampling.wgsl
// ============================================================================

/// FSR EASU tile reconstruction.
///
/// `uv`   — tile-local UV in [0,1].
/// `tile` — per-tile metadata (layer, size flag).
///
/// The upscale ratio is derived from the texture dimensions vs. an effective
/// higher output resolution (2× in each dimension, matching classic FSR usage).
fn sample_tile_fsr_easu(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  var dims = vec2<f32>(0.0);
  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
  } else {
    dims = select(
      vec2<f32>(textureDimensions(tex_small)),
      vec2<f32>(textureDimensions(tex_big)),
      tile.texture_size == 1u,
    );
  }

  // Treat the texture as the input viewport and upscale 2× for the EASU pass.
  let output_size = dims * 2.0;
  let con = fsr_easu_con(dims, dims, output_size);

  // Convert tile UV to output-space pixel coordinate.
  let ip = uv * output_size;

  return fsr_easu(ip, con, tile, dims);
}
