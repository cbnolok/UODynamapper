// ============================================================================
// land::sampling — Texture sampling helpers for terrain tile albedo.
//
//  Supports three reconstruction modes (selected via effects.reconstruction_mode):
//   0 = Nearest (direct textureLoad or textureSample)
//   1 = Bicubic  (Mitchell-Netravali 4×4 kernel)
//   2 = FSR EASU edge-adaptive (AMD FidelityFX Super Resolution 1.0)
//
//  Also provides an optional 9-tap blur (blurred_albedo) and unsharp-mask
//  sharpening (apply_sharpening), plus gradient-stable variants for the blur.
// ============================================================================
//
// EC land texture world-space tiling
// ===================================
// The Enhanced Client (EC) stores terrain textures as large, tileable images
// (often 64×64 up to 512×512 pixels).  They are NOT designed to fit inside a
// single tile; instead, the EC renders them using world-space UV coordinates:
//
//   uv = world_position.xz / stretch
//
// where "stretch" is the number of world tiles covered by one texture
// repetition.  We derive stretch from the texture's pixel width:
//
//   stretch = texture_extent.x / CC_TILE_PX
//
// CC_TILE_PX = 44.0  — the classic-client isometric tile width in pixels,
// which is also the world-unit denominator used by the EC renderer.
// A 44-px texture repeats once per tile (stretch = 1).  A 176-px texture
// repeats once every 4 tiles (stretch = 4), and so on.
//
// The sampler is configured with AddressMode::Repeat so the fract()-wrapped
// UV tiles seamlessly across the terrain grid.
// ============================================================================


#import "shaders/worldmap/land/bindings.wgsl"::{TileUniform, tex_small, tex_big, tex_land_ec_page_atlas, tex_small_sampler, effects}
#import "shaders/worldmap/land/lighting.wgsl"::{luminance}
#import "shaders/worldmap/land/fsr_easu.wgsl"::{sample_tile_fsr_easu}

// Width in pixels of one Classic Client isometric tile — the world-unit
// denominator shared by both CC and EC coordinate systems.
const CC_TILE_PX: f32 = 44.0;

// ============================================================================
// EC-specific world-space UV helper
// ============================================================================

// Compute the world-space tiling UV for an EC texture.
// The stretch (tiles per repetition) is derived from the texture's pixel width:
//   stretch = texture_extent.x / CC_TILE_PX
// We then wrap with fract() so the texture tiles indefinitely.
// world_xz: the fragment's world-space X and Z coordinates (in tile units).
fn ec_world_uv(world_xz: vec2<f32>, tile: TileUniform) -> vec2<f32> {
  // How many world tiles one texture repetition covers.
  // A 44-px EC texture maps exactly to 1 tile; a 176-px one to 4 tiles, etc.
  let tile_w = max(f32(tile.texture_extent.x), 1.0);
  let stretch = tile_w / CC_TILE_PX;
  // World-space UV — wraps to [0,1) so the texture tiles infinitely.
  return fract(world_xz / stretch);
}

// ============================================================================
// Basic albedo sampling
// ============================================================================

// Single-tap albedo: linear (textureSample) or nearest (textureLoad).
// For CC tiles, uv is the [0,1) coordinate within the tile.
// For EC tiles, uv must already be the world-space tiling UV (see ec_world_uv);
// pass world_xz = in.world_position.xz from the fragment shader.
fn sample_tile_albedo(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == 3u) {
    return vec3<f32>(0.0);
  }

  if (tile.texture_size == 2u) {
    // EC atlas path: uv is already the world-space tiling UV in [0,1).
    // Map the tiling UV → atlas pixel coordinates within this texture's slot.
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    let atlas_dims = vec2<f32>(textureDimensions(tex_land_ec_page_atlas));
    if (use_linear) {
      // Sub-pixel bias keeps samples inside the texture's atlas region.
      let local_px = clamp(uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      return textureSample(tex_land_ec_page_atlas, tex_small_sampler, atlas_uv, layer).rgb;
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      return textureLoad(tex_land_ec_page_atlas, atlas_iuv, layer, 0).rgb;
    }
  }

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
// For EC tiles, iuv should be derived from the world-space tiling UV
// (see ec_world_uv) scaled to the texture's pixel dimensions.
fn sample_tile_albedo_at(iuv: vec2<i32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 3u) {
    return vec3<f32>(0.0);
  }
  if (tile.texture_size == 2u) {
    // iuv here is already a [0..extent) pixel coordinate within the texture slot.
    let local_iuv = clamp(iuv, vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
    let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
    return textureLoad(tex_land_ec_page_atlas, atlas_iuv, layer, 0).rgb;
  }
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

  if (tile.texture_size == 3u) {
    return vec3<f32>(0.0);
  }

  if (tile.texture_size == 2u) {
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    if (use_linear) {
      let atlas_dims = vec2<f32>(textureDimensions(tex_land_ec_page_atlas));
      let local_px = clamp(uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      let atlas_ddx = ddx_uv * (tile_dims / atlas_dims);
      let atlas_ddy = ddy_uv * (tile_dims / atlas_dims);
      return textureSampleGrad(tex_land_ec_page_atlas, tex_small_sampler, atlas_uv, layer, atlas_ddx, atlas_ddy).rgb;
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      return textureLoad(tex_land_ec_page_atlas, atlas_iuv, layer, 0).rgb;
    }
  }

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
  var dims = vec2<f32>(0.0);
  if (tile.texture_size == 2u) {
    dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
  } else {
    dims = select(vec2<f32>(textureDimensions(tex_small)), vec2<f32>(textureDimensions(tex_big)), tile.texture_size == 1u);
  }
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

// FSR EASU edge-adaptive reconstruction (12-tap directional Lanczos-like kernel).
// Full AMD FidelityFX Super Resolution 1.0 EASU — see fsr_easu.wgsl.
fn sample_tile_fsr(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  return sample_tile_fsr_easu(uv, tile);
}

// Dispatch to the correct reconstruction mode (0=nearest, 1=bicubic, 2=FSR-like)
fn sample_tile_reconstructed(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  if (tile.texture_size == 3u) {
    return vec3<f32>(0.0);
  }

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
