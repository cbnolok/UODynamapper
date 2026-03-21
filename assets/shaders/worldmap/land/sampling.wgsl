// ============================================================================
// land::sampling — Texture sampling helpers for terrain tile albedo.
//
//  Supports three reconstruction modes (selected via effects.reconstruction_mode):
//   0 = Nearest (direct textureLoad or textureSample)
//   1 = Bicubic  (Mitchell-Netravali 4×4 kernel)
//   2 = FSR-like edge-adaptive (approximate; TODO: replace with real AMD FSR)
//
//  Also provides an optional 9-tap blur (blurred_albedo) and unsharp-mask
//  sharpening (apply_sharpening), plus gradient-stable variants for the blur.
// ============================================================================


#import "shaders/worldmap/land/bindings.wgsl"::{TileUniform, tex_small, tex_big, tex_small_sampler, effects}
#import "shaders/worldmap/land/lighting.wgsl"::{luminance}

// ============================================================================
// Basic albedo sampling
// ============================================================================

// Single-tap albedo: linear (textureSample) or nearest (textureLoad).
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

// Integer-coordinate nearest tap — used by FSR and blur.
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

// Gradient-stable tap: explicit dPdx/dPdy keep LOD consistent across multi-tap
// blur kernels, matching the mip chosen at the center tap.
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

// ============================================================================
// Reconstruction modes
// ============================================================================

// Mitchell-Netravali cubic weight (used by bicubic reconstruction)
fn cubic_weight(f: f32, i: f32) -> f32 {
    let x = abs(f - i);
    if (x <= 1.0) {
        return (1.5 * x - 2.5) * x * x + 1.0;
    } else if (x <= 2.0) {
        return ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0;
    }
    return 0.0;
}

// Bicubic reconstruction: smooth 4×4 tap kernel around the sample point.
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

// Simplified Edge-Adaptive Reconstruction (FSR-like, luma-weighted bilinear).
// IMPORTANT TODO: this is a simplified approximation. Implement real AMD FSR!
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

  // Luma-based gradients for edge detection
  let l00 = luminance(c00);
  let l10 = luminance(c10);
  let l01 = luminance(c01);
  let l11 = luminance(c11);

  // Horizontal/Vertical differences
  let gx = abs(l10 - l00) + abs(l11 - l01);
  let gy = abs(l01 - l00) + abs(l11 - l10);

  // Edge-aware weighting: smooth edges blend more gradually
  let wx = 1.0 / (1.0 + gx * 4.0);
  let wy = 1.0 / (1.0 + gy * 4.0);

  // Bilinear blend biased by edges
  let res = mix(mix(c00, c10, f.x * wx), mix(c01, c11, f.x * wx), f.y * wy);
  return res / (mix(mix(1.0, wx, f.x), mix(1.0, wx, f.x), f.y) * wy); // approximate normalization
}

// Dispatch to the correct reconstruction mode (0=nearest, 1=bicubic, 2=FSR-like)
fn sample_tile_reconstructed(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
    let mode = effects.reconstruction_mode;
    if (mode == 1u) {
        return sample_tile_bicubic(uv, tile);
    } else if (mode == 2u) {
        return sample_tile_fsr(uv, tile);
    } else {
        return sample_tile_albedo(uv, tile);
    }
}

// ============================================================================
// Sharpening & blur
// ============================================================================

// Simple sharpening filter (Unsharp Masking style).
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

// Cheap per-tile random in [0,1) for blur kernel decorrelation.
fn rand01_from_tile(ix: i32, iz: i32, layer: u32) -> f32 {
  let p = vec2<f32>(f32(ix) + f32(layer) * 0.618, f32(iz) + f32(layer) * 1.732);
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// 9-tap blur with radius in *screen pixels* via fwidth.
// LOD is kept stable by using the same gradients for all taps.
// Kernel direction is randomly rotated per tile to decorrelate tiling artifacts.
fn blurred_albedo(uv: vec2<f32>, tile: TileUniform, radius_in_pixels: f32, world_xz: vec2<f32>) -> vec3<f32> {
  // Approximate one screen pixel in UV space for this fragment
  let fw = fwidth(uv);
  let px_uv = max(fw.x, fw.y) + 1e-6;

  // Ensure at least half-pixel radius so it's *noticeable* even at low zoom
  let min_px = 0.5;
  let r = max(radius_in_pixels, min_px) * px_uv;

  // Keep LOD stable for all taps
  let ddx_uv = dpdx(uv);
  let ddy_uv = dpdy(uv);

  // Decorrelation: derive a tiny rotation per tile/layer (+tiny world jitter)
  let jitter = rand01_from_tile(i32(floor(world_xz.x)), i32(floor(world_xz.y)), tile.texture_layer)
             + fract(world_xz.x * 0.173 + world_xz.y * 0.271) * 0.125;
  let ang = (jitter * 6.2831853); // 2π
  let ca = cos(ang);
  let sa = sin(ang);
  let rot = mat2x2<f32>(ca, -sa, sa, ca);

  // Rotated offsets for the 8 surrounding taps
  let o1 = rot * vec2<f32>( r, 0.0);
  let o2 = rot * vec2<f32>(-r, 0.0);
  let o3 = rot * vec2<f32>(0.0,  r);
  let o4 = rot * vec2<f32>(0.0, -r);
  let o5 = rot * vec2<f32>( r,  r);
  let o6 = rot * vec2<f32>(-r,  r);
  let o7 = rot * vec2<f32>( r, -r);
  let o8 = rot * vec2<f32>(-r, -r);

  // Clamp taps to [0,1] to avoid bleeding across tile edges if sampler wraps
  let c  = clamp(uv,       vec2<f32>(0.0), vec2<f32>(1.0));
  let u1 = clamp(uv + o1,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u2 = clamp(uv + o2,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u3 = clamp(uv + o3,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u4 = clamp(uv + o4,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u5 = clamp(uv + o5,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u6 = clamp(uv + o6,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u7 = clamp(uv + o7,  vec2<f32>(0.0), vec2<f32>(1.0));
  let u8 = clamp(uv + o8,  vec2<f32>(0.0), vec2<f32>(1.0));

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
