// ============================================================================
// land::sampling — Texture sampling helpers for terrain tile albedo.
//
//  Provides direct texture sampling, optional 9-tap blur (blurred_albedo), and unsharp-mask
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


#import "shaders/world/common_bindings.wgsl"::{LandEffectsUniform}
#import "shaders/world/land/land_bindings.wgsl"::{TileUniform, tex_small, tex_big, land_page_atlas, land_page_lookup, tex_small_sampler, effects}
#import "shaders/world/pixel_art_filters.wgsl"::pixel_art_uv_iq

// Width in pixels of one Classic Client isometric tile — the world-unit
// denominator shared by both CC and EC coordinate systems.
const CC_TILE_PX: f32 = 44.0;
const LAND_PAGE_LOOKUP_TILE_CAPACITY: u32 = 16384u;
const LAND_PAGE_LOOKUP_ROLE_DETAIL: u32 = 1u;
const LAND_PAGE_LOOKUP_ROLE_MASK: u32 = 2u;
const LAND_PAGE_LOOKUP_ROLE_NORMAL: u32 = 3u;
const TERRAIN_FLAG_FOLLOW_CENTER: u32 = 0x2u;

struct LandLookupSlot {
  present: bool,
  texture_layer: u32,
  texture_origin: vec2<u32>,
  texture_extent: vec2<u32>,
  texture_stretch: f32,
};

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
  let stretch = select(tile_w / CC_TILE_PX, tile.texture_stretch, tile.texture_stretch > 0.0);
  // World-space UV — wraps to [0,1) so the texture tiles infinitely.
  return fract(world_xz / stretch);
}

fn ec_lookup_uv(payload: u32, role: u32) -> vec2<i32> {
  let lookup_dims = textureDimensions(land_page_lookup);
  let lookup_index = role * LAND_PAGE_LOOKUP_TILE_CAPACITY + payload;
  return vec2<i32>(
    i32(lookup_index % lookup_dims.x),
    i32(lookup_index / lookup_dims.x),
  );
}

fn read_ec_lookup_slot(payload: u32, role: u32) -> LandLookupSlot {
  let slot = textureLoad(land_page_lookup, ec_lookup_uv(payload, role), 0);
  let packed_wh = slot.w;
  let w = packed_wh & 0xFFFFu;
  let h = packed_wh >> 16u;
  let page_index = slot.x & 0xFFFFu;
  let stretch_q8 = slot.x >> 16u;
  return LandLookupSlot(
    w != 0u || h != 0u,
    page_index,
    vec2<u32>(slot.y, slot.z),
    vec2<u32>(w, h),
    f32(stretch_q8) / 256.0,
  );
}

fn ec_slot_world_uv(world_xz: vec2<f32>, slot: LandLookupSlot) -> vec2<f32> {
  let tile_w = max(f32(slot.texture_extent.x), 1.0);
  let stretch = select(tile_w / CC_TILE_PX, slot.texture_stretch, slot.texture_stretch > 0.0);
  return fract(world_xz / stretch);
}

fn sample_ec_lookup_slot_rgba(uv: vec2<f32>, slot: LandLookupSlot) -> vec4<f32> {
  let layer: i32 = i32(slot.texture_layer);
  let tile_dims = max(vec2<f32>(slot.texture_extent), vec2<f32>(1.0));
  let use_linear = effects.enable_linear_filtering == 1u;

  if (use_linear) {
    let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
    let seam_uv = pixel_art_uv_iq(uv, tile_dims);
    let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
    let atlas_uv = (vec2<f32>(slot.texture_origin) + local_px) / atlas_dims;
    return textureSample(land_page_atlas, tex_small_sampler, atlas_uv, layer);
  }

  let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(slot.texture_extent) - 1);
  let atlas_iuv = vec2<i32>(slot.texture_origin) + local_iuv;
  return textureLoad(land_page_atlas, atlas_iuv, layer, 0);
}

fn sample_ec_lookup_slot(uv: vec2<f32>, slot: LandLookupSlot) -> vec3<f32> {
  return sample_ec_lookup_slot_rgba(uv, slot).rgb;
}

fn ec_material_mask_value(mask_sample: vec4<f32>) -> f32 {
  let rgb_luma = dot(mask_sample.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
  let alpha_has_signal = abs(mask_sample.a - 1.0) > 0.001;
  return clamp(select(rgb_luma, mask_sample.a, alpha_has_signal), 0.0, 1.0);
}

fn sample_ec_material_albedo(world_xz: vec2<f32>, base_uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let detail = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_DETAIL);
  let mask = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_MASK);
  if (!detail.present || !mask.present) {
    return sample_tile_albedo(base_uv, tile);
  }

  let base = sample_tile_albedo(base_uv, tile);
  let detail_uv = ec_slot_world_uv(world_xz, detail);
  let mask_uv = ec_slot_world_uv(world_xz, mask);
  let detail_color = sample_ec_lookup_slot(detail_uv, detail);
  let mask_value = ec_material_mask_value(sample_ec_lookup_slot_rgba(mask_uv, mask));
  return mix(base, detail_color, mask_value);
}

fn sample_ec_material_albedo_at_world(world_xz: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  return sample_ec_material_albedo(world_xz, ec_world_uv(world_xz, tile), tile);
}

fn ec_material_has_liquid_normal(tile: TileUniform) -> bool {
  return read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_NORMAL).present;
}

fn sample_ec_material_normal_world(world_xz: vec2<f32>, tile: TileUniform, time_seconds: f32, animate_liquid: bool) -> vec4<f32> {
  let normal = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_NORMAL);
  if (!normal.present) {
    return vec4<f32>(0.0, 1.0, 0.0, 0.0);
  }

  let follow_center = (tile.terrain_flags & TERRAIN_FLAG_FOLLOW_CENTER) != 0u;
  let wind_force = select(0.01, 0.1, follow_center);
  let normal_world_xz = select(
    world_xz,
    vec2<f32>(world_xz.x, world_xz.y - time_seconds * wind_force * 10.0),
    animate_liquid,
  );
  let normal_uv = ec_slot_world_uv(normal_world_xz, normal);
  let normal_sample = sample_ec_lookup_slot_rgba(normal_uv, normal).rgb;
  if (normal_sample.b < 0.35) {
    return vec4<f32>(0.0, 1.0, 0.0, 0.0);
  }

  let tangent_normal = normal_sample * 2.0 - vec3<f32>(1.0);
  let world_normal = normalize(vec3<f32>(
    tangent_normal.x,
    max(tangent_normal.z, 0.08),
    tangent_normal.y,
  ));
  return vec4<f32>(world_normal, 1.0);
}

fn ec_liquid_perturbed_base_uv(world_xz: vec2<f32>, base_uv: vec2<f32>, tile: TileUniform, time_seconds: f32) -> vec2<f32> {
  let normal = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_NORMAL);
  if (!normal.present) {
    return base_uv;
  }

  let follow_center = (tile.terrain_flags & TERRAIN_FLAG_FOLLOW_CENTER) != 0u;
  let wind_force = select(0.01, 0.1, follow_center);
  let wave_height = select(0.24, 0.30, follow_center);
  let moving_world_xz = vec2<f32>(world_xz.x, world_xz.y - time_seconds * wind_force * 10.0);
  let normal_uv = ec_slot_world_uv(moving_world_xz, normal);
  let normal_sample = sample_ec_lookup_slot_rgba(normal_uv, normal);
  let perturbation = wave_height * (normal_sample.rg - vec2<f32>(0.5)) * 2.0;
  return fract(base_uv + perturbation);
}

// ============================================================================
// Basic albedo sampling
// ============================================================================

// Single-tap albedo: linear (textureSample) or nearest (textureLoad).
// For CC/array tiles, uv is the [0,1) coordinate within the tile.
// For EC atlas tiles, uv must already be the world-space tiling UV (see ec_world_uv);
// pass world_xz = in.world_position.xz from the fragment shader.
fn sample_tile_albedo(uv: vec2<f32>, tile: TileUniform) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == 3u) {
    return vec3<f32>(0.0);
  }

  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    // Page-atlas path. For EC (2) the caller passes world-space tiling UVs.
    // For CC atlas (4) the caller passes per-tile local UVs.
    // Map the tiling UV → atlas pixel coordinates within this texture's slot.
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
    if (use_linear) {
      // Sub-pixel bias keeps samples inside the texture's atlas region.
      let seam_uv = pixel_art_uv_iq(uv, tile_dims);
      let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      return textureSample(land_page_atlas, tex_small_sampler, atlas_uv, layer).rgb;
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      return textureLoad(land_page_atlas, atlas_iuv, layer, 0).rgb;
    }
  }

  if (tile.texture_size == 1u) {
    if (use_linear) {
      let dims = vec2<f32>(textureDimensions(tex_big));
      return textureSample(tex_big, tex_small_sampler, pixel_art_uv_iq(uv, dims), layer).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_big));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_big, iuv, layer, 0).rgb;
    }
  } else {
    if (use_linear) {
      let dims = vec2<f32>(textureDimensions(tex_small));
      return textureSample(tex_small, tex_small_sampler, pixel_art_uv_iq(uv, dims), layer).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_small));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_small, iuv, layer, 0).rgb;
    }
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

  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    if (use_linear) {
      let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
      let seam_uv = pixel_art_uv_iq(uv, tile_dims);
      let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      let atlas_ddx = ddx_uv * (tile_dims / atlas_dims);
      let atlas_ddy = ddy_uv * (tile_dims / atlas_dims);
      return textureSampleGrad(land_page_atlas, tex_small_sampler, atlas_uv, layer, atlas_ddx, atlas_ddy).rgb;
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      return textureLoad(land_page_atlas, atlas_iuv, layer, 0).rgb;
    }
  }

  if (tile.texture_size == 1u) {
    if (use_linear) {
      let dims = vec2<f32>(textureDimensions(tex_big));
      return textureSampleGrad(tex_big,   tex_small_sampler, pixel_art_uv_iq(uv, dims), layer, ddx_uv, ddy_uv).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_big));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_big, iuv, layer, 0).rgb;
    }
  } else {
    if (use_linear) {
      let dims = vec2<f32>(textureDimensions(tex_small));
      return textureSampleGrad(tex_small, tex_small_sampler, pixel_art_uv_iq(uv, dims), layer, ddx_uv, ddy_uv).rgb;
    } else {
      let dims = vec2<f32>(textureDimensions(tex_small));
      let iuv = vec2<i32>(uv * dims);
      return textureLoad(tex_small, iuv, layer, 0).rgb;
    }
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
