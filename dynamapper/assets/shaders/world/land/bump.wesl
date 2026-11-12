// ============================================================================
// land::bump — The land bump: a texture's relief tilting the terrain normal.
//
//  Enhanced and KR land materials with a bump layer (only snow ships one) use
//  that map (sampling::sample_ec_material_normal_world). Every other land
//  texture, the Classic texmaps included, derives its bump from its own
//  luminance read as a height field: central differences of bilinear taps of
//  the already bound texture, one footprint apart. No land texture has mips,
//  so the taps widen with the screen footprint and the derived bump fades out
//  once a pixel covers more texels than the taps can stand for.
//
//  Classic shading and the clients' per-vertex model keep their vertex light
//  and scale it per pixel by land_bump_light_ratio.
// ============================================================================


import package::shaders::world::land::land_bindings::{TileUniform, tex_small, tex_big, land_page_atlas, tex_small_sampler};
import package::shaders::world::land::lighting::luminance;
import package::shaders::world::land::sampling::{
  LandGradients, LandLookupSlot, LAND_PAGE_LOOKUP_ROLE_DETAIL, LAND_PAGE_LOOKUP_ROLE_MASK, LAND_SOURCE_MISSING,
  LAND_SOURCE_CLASSIC_ART, ec_tile_stretch, ec_world_uv, ec_slot_world_uv, read_ec_lookup_slot,
};

// Relief depth, in texels, of a luminance change from black to white; the strength slider scales it.
const LAND_BUMP_DEPTH_TEXELS: f32 = 12.0;
// The smallest half-distance between the taps of a central difference: one texel.
const LAND_BUMP_STEP_MIN_TEXELS: f32 = 1.0;
// Texels per screen pixel at which the derived bump starts to fade, and where it is gone. Bilinear taps one
// footprint apart stand for a footprint of a few texels; beyond it they alias.
const LAND_BUMP_FADE_START_TEXELS: f32 = 3.0;
const LAND_BUMP_FADE_END_TEXELS: f32 = 8.0;
// Limits of the per-pixel scale a bump gives a vertex light (Classic, the clients' per-vertex model), so that it
// neither blackens a face nor lights one the sun barely reaches.
const LAND_BUMP_LIGHT_RATIO_MIN: f32 = 0.5;
const LAND_BUMP_LIGHT_RATIO_MAX: f32 = 1.5;
// The smallest geometric light the ratio divides by.
const LAND_BUMP_LIGHT_EPSILON: f32 = 0.05;

// One bilinear tap of a page-atlas slot at mip 0; `uv` repeats within the slot.
fn land_bump_page_tap(uv: vec2<f32>, layer: u32, origin: vec2<u32>, extent: vec2<u32>) -> vec4<f32> {
  let dims = max(vec2<f32>(extent), vec2<f32>(1.0));
  let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
  let local_px = clamp(fract(uv) * dims, vec2<f32>(0.5), dims - vec2<f32>(0.5));
  return textureSampleLevel(land_page_atlas, tex_small_sampler, (vec2<f32>(origin) + local_px) / atlas_dims, i32(layer), 0.0);
}

// The height of a land texture at a world position: its perceptual luminance. Enhanced pages hold encoded
// colour, which is already perceptual; the Classic textures decode to linear, and their square root stands for
// the encoding. An Enhanced material blends its two colours by the mask alpha read once at the pixel
// (`mask_alpha`), so that the mask's own edges add no relief.
fn land_bump_height(world_xz: vec2<f32>, tile: TileUniform, detail: LandLookupSlot, mask_alpha: f32) -> f32 {
  if (tile.texture_size == 2u) {
    let base = luminance(land_bump_page_tap(ec_world_uv(world_xz, tile), tile.texture_layer, tile.texture_origin, tile.texture_extent).rgb);
    if (!detail.present) {
      return base;
    }
    let detail_height = luminance(land_bump_page_tap(ec_slot_world_uv(world_xz, detail), detail.texture_layer,
      detail.texture_origin, detail.texture_extent).rgb);
    return mix(detail_height, base, mask_alpha);
  }
  // A Classic texture covers one tile.
  let uv = fract(world_xz);
  var rgb: vec3<f32>;
  if (tile.texture_size == 4u) {
    rgb = land_bump_page_tap(uv, tile.texture_layer, tile.texture_origin, tile.texture_extent).rgb;
  } else if (tile.texture_size == 1u) {
    rgb = textureSampleLevel(tex_big, tex_small_sampler, uv, i32(tile.texture_layer), 0.0).rgb;
  } else {
    rgb = textureSampleLevel(tex_small, tex_small_sampler, uv, i32(tile.texture_layer), 0.0).rgb;
  }
  return sqrt(luminance(rgb));
}

// Texels of the tile's texture along one world tile.
fn land_bump_texels_per_tile(tile: TileUniform) -> f32 {
  if (tile.texture_size == 2u) {
    return f32(max(tile.texture_extent.x, 1u)) / ec_tile_stretch(tile);
  }
  if (tile.texture_size == 4u) {
    return f32(max(tile.texture_extent.x, 1u));
  }
  if (tile.texture_size == 1u) {
    return f32(textureDimensions(tex_big).x);
  }
  return f32(textureDimensions(tex_small).x);
}

// The derived bump's tilt of the normal in world XZ (y is 0), at full strength and faded by the footprint: add it
// to the geometric normal and normalize. Zero for a missing texture, for Classic land drawn from its flat land art
// (no texmap), and once the bump has faded out.
fn land_derived_bump_tilt(world_xz: vec2<f32>, tile: TileUniform, world: LandGradients) -> vec3<f32> {
  if (tile.texture_size == LAND_SOURCE_MISSING || tile.texture_size == LAND_SOURCE_CLASSIC_ART) {
    return vec3<f32>(0.0);
  }
  let texels_per_tile = land_bump_texels_per_tile(tile);
  let footprint = max(length(world.dx), length(world.dy)) * texels_per_tile;
  let fade = 1.0 - smoothstep(LAND_BUMP_FADE_START_TEXELS, LAND_BUMP_FADE_END_TEXELS, footprint);
  if (fade <= 0.0) {
    return vec3<f32>(0.0);
  }
  let step_texels = max(footprint, LAND_BUMP_STEP_MIN_TEXELS);
  let step_world = step_texels / texels_per_tile;

  var detail = LandLookupSlot(false, 0u, vec2<u32>(0u), vec2<u32>(0u), 0.0);
  var mask_alpha = 1.0;
  if (tile.texture_size == 2u) {
    detail = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_DETAIL);
    let mask = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_MASK);
    if (detail.present && mask.present) {
      mask_alpha = land_bump_page_tap(ec_slot_world_uv(world_xz, mask), mask.texture_layer, mask.texture_origin, mask.texture_extent).a;
    } else {
      detail.present = false;
    }
  }

  // Taps +X, -X, +Z, -Z. One call site in a loop: every land sample inlines its texture reads.
  var heights: array<f32, 4>;
  for (var i = 0u; i < 4u; i++) {
    let axis = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), i < 2u);
    let direction = select(-1.0, 1.0, (i & 1u) == 0u);
    heights[i] = land_bump_height(world_xz + axis * (direction * step_world), tile, detail, mask_alpha);
  }
  // The normal tilts away from rising height.
  let scale = LAND_BUMP_DEPTH_TEXELS * fade / (2.0 * step_texels);
  return vec3<f32>((heights[1] - heights[0]) * scale, 0.0, (heights[3] - heights[2]) * scale);
}

// The per-pixel scale of a vertex light for a bumped normal: the light of the bumped normal over the light of the
// geometric one, clamped. `half_lambert` selects the clients' (N.L + 1) / 2, else Classic's max(N.L, 0).
fn land_bump_light_ratio(geometric: vec3<f32>, bumped: vec3<f32>, L: vec3<f32>, half_lambert: bool) -> f32 {
  let geometric_ndotl = dot(normalize(geometric), L);
  let bumped_ndotl = dot(normalize(bumped), L);
  let geometric_light = select(max(geometric_ndotl, 0.0), (geometric_ndotl + 1.0) * 0.5, half_lambert);
  let bumped_light = select(max(bumped_ndotl, 0.0), (bumped_ndotl + 1.0) * 0.5, half_lambert);
  return clamp(bumped_light / max(geometric_light, LAND_BUMP_LIGHT_EPSILON), LAND_BUMP_LIGHT_RATIO_MIN, LAND_BUMP_LIGHT_RATIO_MAX);
}
