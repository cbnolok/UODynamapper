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


import package::shaders::world::common_bindings::{LandEffectsUniform};
import package::shaders::world::land::land_bindings::{TileUniform, tex_small, tex_big, land_page_atlas, land_page_lookup, LAND_PAGE_LOOKUP_TILE_CAPACITY, tex_small_sampler, effects, scene, classic_land_art_atlas, classic_land_art_lookup, texture_variation, LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY};
import package::shaders::world::pixel_art_filters::{pixel_art_uv_iq, pixel_art_uv_iq_grad};
import package::shaders::world::client_filter::{client_filtered_rect_sampled, TEXTURE_FILTER_CLIENT_BILINEAR};
import package::shaders::world::texture_data::texture_color_rgb;

// Width in pixels of one Classic Client isometric tile — the world-unit
// denominator shared by both CC and EC coordinate systems.
const CC_TILE_PX: f32 = 44.0;
const LAND_PAGE_LOOKUP_ROLE_DETAIL: u32 = 1u;
const LAND_PAGE_LOOKUP_ROLE_MASK: u32 = 2u;
const LAND_PAGE_LOOKUP_ROLE_NORMAL: u32 = 3u;
// The smallest texture blur radius in screen pixels: a smaller one blurs exactly as this one does.
const BLUR_RADIUS_MIN_PIXELS: f32 = 0.5;

// Derivatives of unwrapped world XZ, captured before per-material control flow.
struct LandGradients {
  dx: vec2<f32>,
  dy: vec2<f32>,
}

fn ec_tile_stretch(tile: TileUniform) -> f32 {
  let tile_w = max(f32(tile.texture_extent.x), 1.0);
  return select(tile_w / CC_TILE_PX, tile.texture_stretch, tile.texture_stretch > 0.0);
}

fn ec_tile_gradients(world: LandGradients, tile: TileUniform) -> LandGradients {
  let stretch = ec_tile_stretch(tile);
  return LandGradients(world.dx / stretch, world.dy / stretch);
}

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
  let stretch = ec_tile_stretch(tile);
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

fn sample_ec_lookup_slot_rgba(uv: vec2<f32>, slot: LandLookupSlot, world: LandGradients) -> vec4<f32> {
  let layer: i32 = i32(slot.texture_layer);
  let tile_dims = max(vec2<f32>(slot.texture_extent), vec2<f32>(1.0));
  let use_linear = effects.enable_linear_filtering == 1u;

  if (effects.texture_filter == TEXTURE_FILTER_CLIENT_BILINEAR) {
    // The page textures repeat across the world, so the client's taps wrap within the slot.
    let tile_w = max(f32(slot.texture_extent.x), 1.0);
    let stretch = select(tile_w / CC_TILE_PX, slot.texture_stretch, slot.texture_stretch > 0.0);
    return client_filtered_rect_sampled(land_page_atlas, tex_small_sampler, layer, vec2<i32>(slot.texture_origin),
      vec2<i32>(slot.texture_extent), uv * tile_dims, true,
      (world.dx / stretch) * tile_dims, (world.dy / stretch) * tile_dims, scene.client_view_scale);
  }

  if (use_linear) {
    let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
    let tile_w = max(f32(slot.texture_extent.x), 1.0);
    let stretch = select(tile_w / CC_TILE_PX, slot.texture_stretch, slot.texture_stretch > 0.0);
    let seam_uv = pixel_art_uv_iq_grad(uv, tile_dims, world.dx / stretch, world.dy / stretch);
    let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
    let atlas_uv = (vec2<f32>(slot.texture_origin) + local_px) / atlas_dims;
    let atlas_scale = tile_dims / atlas_dims;
    return textureSampleGrad(land_page_atlas, tex_small_sampler, atlas_uv, layer,
      (world.dx / stretch) * atlas_scale, (world.dy / stretch) * atlas_scale);
  }

  let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(slot.texture_extent) - 1);
  let atlas_iuv = vec2<i32>(slot.texture_origin) + local_iuv;
  return textureLoad(land_page_atlas, atlas_iuv, layer, 0);
}

fn sample_ec_lookup_slot(uv: vec2<f32>, slot: LandLookupSlot, world: LandGradients) -> vec3<f32> {
  return texture_color_rgb(sample_ec_lookup_slot_rgba(uv, slot, world).rgb);
}

// The blend factor is the mask texel's alpha alone, as in the client's SetupFourStages (0x005974f0):
// stage 1 selects D3DTA_TEXTURE alpha (no modifier) of item 2, and stage 2's BLENDCURRENTALPHA weights
// item 0 by it. The mask's RGB is never read (the shipped masks hold placeholder art there), and a
// mask without an alpha channel samples as alpha 1, item 0 alone, as Direct3D does.
fn ec_material_mask_value(mask_sample: vec4<f32>) -> f32 {
  return mask_sample.a;
}

// ============================================================================
// Land texture variation
// ============================================================================
//
// The KR and Enhanced terrain textures repeat every few cells in a grid the eye picks out. With a style's
// land_texture_variation on, the materials LandTextureVariation.kdl lists (texture_variation, land_bindings.wesl)
// draw their two colour layers through Mikkelsen's hex tiling ("Practical Real-Time Hex-Tiling", JCGT 2022): a
// triangle grid whose three vertices around a point each show the texture moved by a random offset, and turned by
// a random quarter turn where the list allows it, blended by sharpened barycentric weights that favour the
// brighter sample, so that the blend keeps the texture's contrast. The blend mask (item 2) moves through a coarser
// grid of its own, so its blobs keep their size but no longer repeat with the colours. Every tap keeps explicit
// gradients turned with it, so the filters see the same footprint as the plain tiling and draw no mip seams. The
// transform depends on the world position alone: transitions, coverage layers, blur, sharpening, the bump and the
// normal map follow it. The variation fades out between two zooms; past the second every land sample takes the
// plain tiling at no extra cost.
//
// Every consumer walks the same four taps: the three hex vertices, then the plain tiling (LAND_VARIATION_PLAIN_TAP),
// which carries the weight the zoom fade leaves it (land_variation_first_tap and land_variation_end_tap). Variation
// off, the plain tap alone has weight 1, and every sum below reduces exactly to the plain sample. A frame keeps only
// the point's triangle; land_variation_tap derives each tap's offset and turn when a loop reaches it.

// Distance between the hex centres of the colour layers, and of the blend mask, in world cells.
const LAND_VARIATION_HEX_EDGE_CELLS: f32 = 2.0;
const LAND_VARIATION_MASK_HEX_EDGE_CELLS: f32 = 12.0;
// The random offsets span this many cells, in steps of 1 / LAND_VARIATION_OFFSET_STEPS_PER_CELL.
const LAND_VARIATION_OFFSET_RANGE_CELLS: f32 = 64.0;
const LAND_VARIATION_OFFSET_STEPS_PER_CELL: f32 = 8.0;
// Mikkelsen's barycentric weight exponent, and how strongly the blend favours the brighter sample (0 not at all).
const LAND_VARIATION_WEIGHT_EXPONENT: f32 = 7.0;
const LAND_VARIATION_LUMA_CONTRAST: f32 = 0.6;
const LAND_VARIATION_LUMA_WEIGHTS: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);
// Taps whose weight is at most this are skipped.
const LAND_VARIATION_MIN_WEIGHT: f32 = 0.002;
// The smallest sum of luminance weights a blend divides by.
const LAND_VARIATION_WEIGHT_EPSILON: f32 = 1e-6;
// Added to the grid ids of the mask's grid, so that its offsets are not the colours'; added to a grid id for its
// quarter turn, so that the turn is not its offset.
const LAND_VARIATION_MASK_SALT: vec2<i32> = vec2<i32>(15485863, 32452843);
const LAND_VARIATION_TURN_SALT: vec2<i32> = vec2<i32>(7919, 104729);
// The skewed triangle grid: a world point st (in hex edges) is (st.x - st.y / sqrt(3), st.y * 2 / sqrt(3)).
const LAND_VARIATION_SKEW_X: f32 = 0.57735027;
const LAND_VARIATION_SKEW_Y: f32 = 1.15470054;
// The zooms (scene.render_zoom, larger is farther out) over which the variation fades out. Its cost grows as the
// view widens (measured on Enhanced grass: +0.5 ms at zoom 1, +2 ms at 6, +4 ms at 11) while the repetition it hides
// does not fade, so it ends where shade_land's level of detail drops blur and sharpening (zoom 10, main.wesl).
const LAND_VARIATION_FADE_START_ZOOM: f32 = 8.0;
const LAND_VARIATION_FADE_END_ZOOM: f32 = 10.0;
// A pixel keeps a material's normal map where the taps reading one weigh more than this, else it takes the derived
// bump (main.wesl), as the plain tiling does where its tap reads none.
const LAND_VARIATION_MAPPED_HALF: f32 = 0.5;
// The taps of a variation frame: three hex vertices, then the plain tiling.
const LAND_VARIATION_TAP_COUNT: u32 = 4u;
const LAND_VARIATION_PLAIN_TAP: u32 = 3u;
// The quarter turns a rotating material's hex vertex picks from.
const LAND_VARIATION_QUARTER_TURNS: u32 = 4u;
// The texture source of the EC-layout materials, KR's and Enhanced's (tile_atlas.rs LAND_SOURCE_EC_ATLAS).
const LAND_SOURCE_EC_ATLAS: u32 = 2u;
// The coverage parameters' lookup role (coverage.wesl), whose x is the material id.
const LAND_PAGE_LOOKUP_ROLE_COVERAGE_PARAMS: u32 = 6u;
// The package-local material id: the secondary EC-layout package's ids carry 1 << 24 on top
// (SECONDARY_PACKAGE_MATERIAL_ID_OFFSET in texture_cache/land/coverage.rs), and both packages number alike.
const LAND_MATERIAL_ID_MASK: u32 = 0xFFFFFFu;
// The bitmask's layout (configs/land_texture_variation.rs): two bits per material, sixteen materials per word.
const LAND_VARIATION_BITS_PER_MATERIAL: u32 = 2u;
const LAND_VARIATION_MATERIALS_PER_WORD: u32 = 16u;
const LAND_VARIATION_WORDS_PER_VEC4: u32 = 4u;
const LAND_VARIATION_ELIGIBLE_BIT: u32 = 1u;
const LAND_VARIATION_ROTATE_BIT: u32 = 2u;

// Whether and how strongly a tile's material varies.
struct LandVariation {
  // 1 fully varied, 0 the plain tiling.
  fade: f32,
  rotate: bool,
}

// The triangle of the hex grid around a world point: its three vertices are taps 0 to 2, the plain tiling tap 3.
struct LandVariationFrame {
  // The skewed grid cell, and whether the point is in its upper triangle (1) or its lower one (0).
  cell: vec2<i32>,
  upper: i32,
  // The hex vertices' weights, summing to 1.
  weights: vec3<f32>,
  fade: f32,
  rotate: bool,
  edge: f32,
  salt: vec2<i32>,
}

// One tap of a frame: it reads the world point turn * (p - origin) + shift; the plain tap reads p.
struct LandVariationTap {
  plain: bool,
  turn: mat2x2<f32>,
  origin: vec2<f32>,
  shift: vec2<f32>,
  // The tap's weight in the sample, and a hex vertex's share of the varied part.
  weight: f32,
  share: f32,
}

// The bits of a material in the list: LAND_VARIATION_ELIGIBLE_BIT, LAND_VARIATION_ROTATE_BIT.
fn land_texture_variation_bits(material_id: u32) -> u32 {
  if (material_id >= LAND_TEXTURE_VARIATION_MATERIAL_CAPACITY) {
    return 0u;
  }
  let word = material_id / LAND_VARIATION_MATERIALS_PER_WORD;
  let packed = texture_variation.materials[word / LAND_VARIATION_WORDS_PER_VEC4][word % LAND_VARIATION_WORDS_PER_VEC4];
  let shift = (material_id % LAND_VARIATION_MATERIALS_PER_WORD) * LAND_VARIATION_BITS_PER_MATERIAL;
  return (packed >> shift) & ((1u << LAND_VARIATION_BITS_PER_MATERIAL) - 1u);
}

// The variation of a tile: none unless the style has it on, the view is near enough and the tile is an EC-layout
// material the list names. The zoom and the flag come first, so that a plain view reads no lookup.
fn land_texture_variation(tile: TileUniform) -> LandVariation {
  var variation = LandVariation(0.0, false);
  if (effects.land_texture_variation == 0u || tile.texture_size != LAND_SOURCE_EC_ATLAS) {
    return variation;
  }
  let fade = 1.0 - smoothstep(LAND_VARIATION_FADE_START_ZOOM, LAND_VARIATION_FADE_END_ZOOM, scene.render_zoom);
  if (fade <= 0.0 || tile.texture_payload >= LAND_PAGE_LOOKUP_TILE_CAPACITY) {
    return variation;
  }
  let params = textureLoad(land_page_lookup, ec_lookup_uv(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_COVERAGE_PARAMS), 0);
  if (params.w == 0u) {
    return variation;
  }
  let bits = land_texture_variation_bits(params.x & LAND_MATERIAL_ID_MASK);
  if ((bits & LAND_VARIATION_ELIGIBLE_BIT) == 0u) {
    return variation;
  }
  variation.fade = fade;
  variation.rotate = (bits & LAND_VARIATION_ROTATE_BIT) != 0u;
  return variation;
}

// A hash of a grid id to two values in [0, 1) (pcg2d, Jarzynski and Olano 2020).
fn land_variation_hash(id: vec2<i32>) -> vec2<f32> {
  const PCG_MULTIPLIER: u32 = 1664525u;
  const PCG_INCREMENT: u32 = 1013904223u;
  const PCG_SHIFT: u32 = 16u;
  const TWO_POW_32: f32 = 4294967296.0;
  var v = vec2<u32>(bitcast<u32>(id.x), bitcast<u32>(id.y)) * PCG_MULTIPLIER + vec2<u32>(PCG_INCREMENT);
  v.x += v.y * PCG_MULTIPLIER;
  v.y += v.x * PCG_MULTIPLIER;
  v = v ^ (v >> vec2<u32>(PCG_SHIFT));
  v.x += v.y * PCG_MULTIPLIER;
  v.y += v.x * PCG_MULTIPLIER;
  v = v ^ (v >> vec2<u32>(PCG_SHIFT));
  return vec2<f32>(v) / TWO_POW_32;
}

// A quarter turn: 0 to 3 times 90 degrees.
fn land_variation_quarter_turn(turns: u32) -> mat2x2<f32> {
  switch (turns % LAND_VARIATION_QUARTER_TURNS) {
    case 1u: { return mat2x2<f32>(0.0, 1.0, -1.0, 0.0); }
    case 2u: { return mat2x2<f32>(-1.0, 0.0, 0.0, -1.0); }
    case 3u: { return mat2x2<f32>(0.0, -1.0, 1.0, 0.0); }
    default: { return mat2x2<f32>(1.0, 0.0, 0.0, 1.0); }
  }
}

// The frame of the triangle grid of edge `edge` cells around a world point, its grid ids moved by `salt`. A plain
// variation (fade 0) leaves the hex weights 0.
fn land_variation_frame(world_xz: vec2<f32>, edge: f32, salt: vec2<i32>, variation: LandVariation) -> LandVariationFrame {
  var frame: LandVariationFrame;
  frame.fade = variation.fade;
  if (variation.fade <= 0.0) {
    return frame;
  }
  frame.rotate = variation.rotate;
  frame.edge = edge;
  frame.salt = salt;
  let st = world_xz / edge;
  let skewed = vec2<f32>(st.x - LAND_VARIATION_SKEW_X * st.y, LAND_VARIATION_SKEW_Y * st.y);
  frame.cell = vec2<i32>(floor(skewed));
  let f = fract(skewed);
  // The point's triangle, lower (s 0) or upper (s 1), and its barycentric weights for the vertices
  // cell + (s, s), cell + (s, 1 - s) and cell + (1 - s, s).
  let z = 1.0 - f.x - f.y;
  let s = select(0.0, 1.0, z < 0.0);
  let s2 = 2.0 * s - 1.0;
  let barycentric = vec3<f32>(-z * s2, s - f.y * s2, s - f.x * s2);
  frame.upper = i32(s);
  let sharpened = pow(max(barycentric, vec3<f32>(0.0)), vec3<f32>(LAND_VARIATION_WEIGHT_EXPONENT));
  frame.weights = sharpened / (sharpened.x + sharpened.y + sharpened.z);
  return frame;
}

// Tap i of a frame. A hex vertex's offset and turn come from a hash of its grid id, computed here rather than kept in
// arrays in the frame: dynamically indexed arrays made the land shader much slower to compile.
fn land_variation_tap(frame: LandVariationFrame, i: u32) -> LandVariationTap {
  var tap = LandVariationTap(true, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), vec2<f32>(0.0), vec2<f32>(0.0), 1.0 - frame.fade, 0.0);
  if (i == LAND_VARIATION_PLAIN_TAP) {
    return tap;
  }
  let s = frame.upper;
  let vertex_id = frame.cell + vec2<i32>(select(s, 1 - s, i == 2u), select(s, 1 - s, i == 1u));
  let vertex = vec2<f32>(vertex_id);
  let centre = vec2<f32>(vertex.x + 0.5 * vertex.y, vertex.y / LAND_VARIATION_SKEW_Y) * frame.edge;
  let id = vertex_id + frame.salt;
  let offset = floor(land_variation_hash(id) * LAND_VARIATION_OFFSET_RANGE_CELLS * LAND_VARIATION_OFFSET_STEPS_PER_CELL)
    / LAND_VARIATION_OFFSET_STEPS_PER_CELL;
  let turns = select(0u, u32(land_variation_hash(id + LAND_VARIATION_TURN_SALT).x * f32(LAND_VARIATION_QUARTER_TURNS)),
    frame.rotate);
  tap.plain = false;
  tap.turn = land_variation_quarter_turn(turns);
  tap.origin = centre;
  tap.shift = centre + offset;
  tap.share = frame.weights[i];
  tap.weight = tap.share * frame.fade;
  return tap;
}

// The taps a loop over a frame walks, first to end (exclusive): the plain tiling alone when the variation is off, the
// hex vertices alone when it is in full. The bounds depend on the frame, so that the shader compilers keep each loop
// rolled: unrolled, every land sample site would inline its texture reads four times over.
fn land_variation_first_tap(frame: LandVariationFrame) -> u32 {
  return select(LAND_VARIATION_PLAIN_TAP, 0u, frame.fade > 0.0);
}

fn land_variation_end_tap(frame: LandVariationFrame) -> u32 {
  return select(LAND_VARIATION_TAP_COUNT, LAND_VARIATION_PLAIN_TAP, frame.fade >= 1.0);
}

// Whether tap i is sampled: a hex vertex above LAND_VARIATION_MIN_WEIGHT, the plain tiling whenever it weighs
// anything, so that the plain path stays exact.
fn land_variation_tap_active(frame: LandVariationFrame, i: u32) -> bool {
  if (i == LAND_VARIATION_PLAIN_TAP) {
    return frame.fade < 1.0;
  }
  return frame.weights[i] * frame.fade > LAND_VARIATION_MIN_WEIGHT;
}

// The world point a tap reads for the world point p.
fn land_variation_tap_point(tap: LandVariationTap, p: vec2<f32>) -> vec2<f32> {
  if (tap.plain) {
    return p;
  }
  return tap.turn * (p - tap.origin) + tap.shift;
}

// The screen gradients of a tap's world point: the world's, turned with it.
fn land_variation_tap_gradients(tap: LandVariationTap, world: LandGradients) -> LandGradients {
  if (tap.plain) {
    return world;
  }
  return LandGradients(tap.turn * world.dx, tap.turn * world.dy);
}

// A world direction a tap reads, back in world axes: the transpose of its turn.
fn land_variation_tap_unturn(tap: LandVariationTap, v: vec2<f32>) -> vec2<f32> {
  if (tap.plain) {
    return v;
  }
  return transpose(tap.turn) * v;
}

// How much a hex vertex's colour counts in the blend: brighter samples more, keeping the texture's contrast.
fn land_variation_luma_weight(color: vec3<f32>) -> f32 {
  return mix(1.0, dot(color, LAND_VARIATION_LUMA_WEIGHTS), LAND_VARIATION_LUMA_CONTRAST);
}

// Accumulates the colours of a frame's taps: the hex vertices by their weights and luminance, the plain tap apart.
struct LandVariationColorSum {
  varied: vec3<f32>,
  varied_weight: f32,
  plain: vec3<f32>,
}

fn land_variation_add_color(sum: ptr<function, LandVariationColorSum>, tap: LandVariationTap, color: vec3<f32>) {
  if (tap.plain) {
    (*sum).plain = color;
    return;
  }
  let weight = tap.share * land_variation_luma_weight(color);
  (*sum).varied += color * weight;
  (*sum).varied_weight += weight;
}

// The varied colour faded with the plain one; the plain colour exactly when the variation is off.
fn land_variation_resolve_color(sum: LandVariationColorSum, frame: LandVariationFrame) -> vec3<f32> {
  let varied = select(vec3<f32>(0.0), sum.varied / max(sum.varied_weight, LAND_VARIATION_WEIGHT_EPSILON),
    sum.varied_weight > 0.0);
  return varied * frame.fade + sum.plain * (1.0 - frame.fade);
}

// The blend mask's value at a world point, through the mask's own grid when the material varies.
fn ec_material_mask_value_varied(world_xz: vec2<f32>, mask: LandLookupSlot, world: LandGradients, variation: LandVariation) -> f32 {
  let frame = land_variation_frame(world_xz, LAND_VARIATION_MASK_HEX_EDGE_CELLS, LAND_VARIATION_MASK_SALT, variation);
  var value = 0.0;
  // One call site in a loop: every land sample inlines its texture reads.
  for (var i = land_variation_first_tap(frame); i < land_variation_end_tap(frame); i++) {
    if (land_variation_tap_active(frame, i)) {
      let tap = land_variation_tap(frame, i);
      let mask_uv = ec_slot_world_uv(land_variation_tap_point(tap, world_xz), mask);
      value += ec_material_mask_value(sample_ec_lookup_slot_rgba(mask_uv, mask, land_variation_tap_gradients(tap, world)))
        * tap.weight;
    }
  }
  return value;
}

// An EC material's colour: item 0 (base) and item 1 (detail) blended by the alpha of item 2 (mask). `base_uv` is the
// plain tiling's base uv (the adaptive cheap path passes it quantized); the varied taps take theirs from their point.
fn sample_ec_material_albedo(world_xz: vec2<f32>, base_uv: vec2<f32>, tile: TileUniform, world: LandGradients) -> vec3<f32> {
  let detail = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_DETAIL);
  let mask = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_MASK);
  let layered = detail.present && mask.present;
  let variation = land_texture_variation(tile);
  var mask_value = 1.0;
  if (layered) {
    mask_value = ec_material_mask_value_varied(world_xz, mask, world, variation);
  }
  let frame = land_variation_frame(world_xz, LAND_VARIATION_HEX_EDGE_CELLS, vec2<i32>(0), variation);
  var sum = LandVariationColorSum(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
  // One base and one detail sample site: every land sample inlines its filter, so fewer call sites compile faster.
  for (var i = land_variation_first_tap(frame); i < land_variation_end_tap(frame); i++) {
    if (land_variation_tap_active(frame, i)) {
      let tap = land_variation_tap(frame, i);
      let p = land_variation_tap_point(tap, world_xz);
      let gradients = land_variation_tap_gradients(tap, world);
      let uv = select(ec_world_uv(p, tile), base_uv, i == LAND_VARIATION_PLAIN_TAP);
      var color = sample_ec_tile_albedo(uv, tile, ec_tile_gradients(gradients, tile));
      if (layered) {
        let detail_color = sample_ec_lookup_slot(ec_slot_world_uv(p, detail), detail, gradients);
        // The client's SetupFourStages: item0 · a + item1 · (1 − a), a = the blend item alpha
        // (base = item 0, detail = item 1).
        color = mix(detail_color, color, mask_value);
      }
      land_variation_add_color(&sum, tap, color);
    }
  }
  return land_variation_resolve_color(sum, frame);
}

fn sample_ec_material_albedo_at_world(world_xz: vec2<f32>, tile: TileUniform, world: LandGradients) -> vec3<f32> {
  return sample_ec_material_albedo(world_xz, ec_world_uv(world_xz, tile), tile, world);
}

fn ec_material_has_liquid_normal(tile: TileUniform) -> bool {
  return read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_NORMAL).present;
}

// The static (non-water) normal map. Water materials take the original
// UOWaterShader path (`ec_liquid::sample_ec_liquid`), which scrolls item 0 by
// the material's tweaks; this sampler never animates.
// With the land texture variation, each tap of the colour layers' frame reads the map at its moved and turned point,
// turns its normal back into world axes and weighs it as the colour; the pixel keeps the map where taps reading
// one outweigh those that read none.
fn sample_ec_material_normal_world(world_xz: vec2<f32>, tile: TileUniform, world: LandGradients) -> vec4<f32> {
  let normal = read_ec_lookup_slot(tile.texture_payload, LAND_PAGE_LOOKUP_ROLE_NORMAL);
  if (!normal.present) {
    return vec4<f32>(0.0, 1.0, 0.0, 0.0);
  }

  let frame = land_variation_frame(world_xz, LAND_VARIATION_HEX_EDGE_CELLS, vec2<i32>(0), land_texture_variation(tile));
  var normal_sum = vec3<f32>(0.0);
  var mapped_weight = 0.0;
  // One call site in a loop: every land sample inlines its texture reads.
  for (var i = land_variation_first_tap(frame); i < land_variation_end_tap(frame); i++) {
    if (land_variation_tap_active(frame, i)) {
      let tap = land_variation_tap(frame, i);
      let normal_uv = ec_slot_world_uv(land_variation_tap_point(tap, world_xz), normal);
      let normal_sample = sample_ec_lookup_slot_rgba(normal_uv, normal, land_variation_tap_gradients(tap, world)).rgb;
      if (normal_sample.b >= 0.35) {
        let tangent_normal = normal_sample * 2.0 - vec3<f32>(1.0);
        let world_normal = normalize(vec3<f32>(
          tangent_normal.x,
          max(tangent_normal.z, 0.08),
          tangent_normal.y,
        ));
        let world_xz_normal = land_variation_tap_unturn(tap, world_normal.xz);
        normal_sum += vec3<f32>(world_xz_normal.x, world_normal.y, world_xz_normal.y) * tap.weight;
        mapped_weight += tap.weight;
      }
    }
  }
  if (mapped_weight <= LAND_VARIATION_MAPPED_HALF) {
    return vec4<f32>(0.0, 1.0, 0.0, 0.0);
  }
  // The plain tiling alone is its one normal, as it is.
  return vec4<f32>(select(normalize(normal_sum), normal_sum, frame.fade <= 0.0), 1.0);
}

// ============================================================================
// Basic albedo sampling
// ============================================================================

struct ClassicLandArtSlot {
  layer: u32,
  extent: vec2<u32>,
};

fn read_classic_land_art_slot(tile_payload: u32) -> ClassicLandArtSlot {
  let lookup_dims = textureDimensions(classic_land_art_lookup);
  let lookup_uv = vec2<i32>(
    i32(tile_payload % lookup_dims.x),
    i32(tile_payload / lookup_dims.x),
  );
  let slot = textureLoad(classic_land_art_lookup, lookup_uv, 0);
  return ClassicLandArtSlot(slot.x, vec2<u32>(slot.y, slot.z));
}

fn classic_land_art_is_present(tile_payload: u32) -> bool {
  return read_classic_land_art_slot(tile_payload).layer != 0u;
}

// Classic land art stores an isometric diamond in a square bitmap, while the
// terrain mesh is an axis-aligned XZ cell. Inverse of the flat static quad's
// geometry (world_x = art_u + art_v - 0.5, world_z = art_v - art_u + 0.5).
fn classic_flat_art_uv_from_world_cell_uv(world_cell_uv: vec2<f32>) -> vec2<f32> {
  return vec2<f32>(
    0.5 + 0.5 * (world_cell_uv.x - world_cell_uv.y),
    0.5 * (world_cell_uv.x + world_cell_uv.y),
  );
}

fn sample_classic_land_art(world_cell_uv: vec2<f32>, tile_payload: u32) -> vec3<f32> {
  let slot = read_classic_land_art_slot(tile_payload);
  let tile_dims = max(vec2<f32>(slot.extent), vec2<f32>(1.0));
  let atlas_dims = vec2<f32>(textureDimensions(classic_land_art_atlas).xy);
  let art_uv = classic_flat_art_uv_from_world_cell_uv(world_cell_uv);
  let layer = i32(slot.layer);

  if (effects.enable_linear_filtering == 1u) {
    let seam_uv = pixel_art_uv_iq(art_uv, tile_dims);
    let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
    let atlas_uv = local_px / atlas_dims;
    return textureSample(classic_land_art_atlas, tex_small_sampler, atlas_uv, layer).rgb;
  }

  let local_iuv = clamp(
    vec2<i32>(art_uv * tile_dims),
    vec2<i32>(0),
    vec2<i32>(slot.extent) - vec2<i32>(1),
  );
  return textureLoad(classic_land_art_atlas, local_iuv, layer, 0).rgb;
}

// Texture source modes of a tile (tile_atlas.rs LAND_SOURCE_*; atlas.wesl): no land source has the id, or it is
// Classic land without a texmap, drawn from its land art as the Classic client draws it (routing.rs).
const LAND_SOURCE_MISSING: u32 = 3u;
const LAND_SOURCE_CLASSIC_ART: u32 = 5u;
// Classic land the Classic client does not draw: the fragment is discarded (main.wesl). A neighbour's sample of it,
// for a transition or a blur tap, is black, the colour behind the land.
const LAND_SOURCE_NODRAW: u32 = 6u;
const LAND_NODRAW_COLOR: vec3<f32> = vec3<f32>(0.0);
// The missing-texture pattern: a checker of this many cells per tile side, in two colours no terrain has.
const MISSING_TEXTURE_CHECKER_CELLS: f32 = 4.0;
const MISSING_TEXTURE_COLOR_A: vec3<f32> = vec3<f32>(1.0, 0.0, 1.0);
const MISSING_TEXTURE_COLOR_B: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);

fn missing_texture_pattern(uv: vec2<f32>) -> vec3<f32> {
  let cell = vec2<i32>(floor(fract(uv) * MISSING_TEXTURE_CHECKER_CELLS));
  return select(MISSING_TEXTURE_COLOR_B, MISSING_TEXTURE_COLOR_A, ((cell.x + cell.y) & 1) == 0);
}

// Single-tap albedo: linear (textureSample) or nearest (textureLoad).
// For CC/array tiles, uv is the [0,1) coordinate within the tile.
// For EC atlas tiles, uv must already be the world-space tiling UV (see ec_world_uv);
// gradients are the continuous world footprint divided by this material's stretch.
fn sample_tile_albedo(uv: vec2<f32>, tile: TileUniform, gradients: LandGradients) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == LAND_SOURCE_MISSING) {
    return missing_texture_pattern(uv);
  }
  if (tile.texture_size == LAND_SOURCE_NODRAW) {
    return LAND_NODRAW_COLOR;
  }
  if (tile.texture_size == LAND_SOURCE_CLASSIC_ART) {
    return sample_classic_land_art(fract(uv), tile.texture_payload);
  }

  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    // Page-atlas path. For EC (2) the caller passes world-space tiling UVs.
    // For CC atlas (4) the caller passes per-tile local UVs.
    // Map the tiling UV → atlas pixel coordinates within this texture's slot.
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
    if (use_linear) {
      // Sub-pixel bias keeps samples inside the texture's atlas region.
      var seam_uv: vec2<f32>;
      if (tile.texture_size == 2u) {
        seam_uv = pixel_art_uv_iq_grad(uv, tile_dims, gradients.dx, gradients.dy);
      } else {
        seam_uv = pixel_art_uv_iq(uv, tile_dims);
      }
      let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      if (tile.texture_size == 2u) {
        let atlas_scale = tile_dims / atlas_dims;
        let color = textureSampleGrad(land_page_atlas, tex_small_sampler, atlas_uv, layer,
          gradients.dx * atlas_scale, gradients.dy * atlas_scale).rgb;
        return texture_color_rgb(color);
      }
      return textureSample(land_page_atlas, tex_small_sampler, atlas_uv, layer).rgb;
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      let color = textureLoad(land_page_atlas, atlas_iuv, layer, 0).rgb;
      return select(color, texture_color_rgb(color), tile.texture_size == 2u);
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

// A tile's own texture under the selected texture filter (`effects.texture_filter`): the KR client's filter, or
// the pixel-art sample_tile_albedo. Only the tile's own texture, the EC material layers and their transition
// neighbours take the client filter. Blur and sharpening taps, the adaptive cheap path and the CC transitions
// keep sample_tile_albedo: every land sample inlines its filter, and the client's on all of them made the land
// shader twice as slow to compile.
fn sample_tile_albedo_filtered(uv: vec2<f32>, tile: TileUniform, gradients: LandGradients) -> vec3<f32> {
  if (tile.texture_size != LAND_SOURCE_MISSING && tile.texture_size != LAND_SOURCE_CLASSIC_ART
      && tile.texture_size != LAND_SOURCE_NODRAW && effects.texture_filter == TEXTURE_FILTER_CLIENT_BILINEAR) {
    return sample_tile_albedo_client(uv, tile, gradients);
  }
  return sample_tile_albedo(uv, tile, gradients);
}

// As sample_tile_albedo_filtered, for an EC material's base texture, which is always on the EC page atlas:
// one client branch instead of three.
fn sample_ec_tile_albedo(uv: vec2<f32>, tile: TileUniform, gradients: LandGradients) -> vec3<f32> {
  if (effects.texture_filter == TEXTURE_FILTER_CLIENT_BILINEAR) {
    return sample_page_tile_albedo_client(uv, tile, gradients);
  }
  return sample_tile_albedo(uv, tile, gradients);
}

// The KR client's filter (client_filter.wesl) on a page-atlas tile texture. EC page textures repeat across the
// world, so their taps wrap; a CC atlas slot holds one tile's texture and its taps clamp to the slot.
// `gradients` are the screen derivatives of `uv`.
fn sample_page_tile_albedo_client(uv: vec2<f32>, tile: TileUniform, gradients: LandGradients) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
  let color = client_filtered_rect_sampled(land_page_atlas, tex_small_sampler, layer, vec2<i32>(tile.texture_origin),
    vec2<i32>(tile.texture_extent), uv * tile_dims, tile.texture_size == 2u,
    gradients.dx * tile_dims, gradients.dy * tile_dims, scene.client_view_scale).rgb;
  return select(color, texture_color_rgb(color), tile.texture_size == 2u);
}

// The KR client's filter on any tile texture: the page atlases, or the texmap arrays, whose textures repeat
// across the world. `gradients` are the screen derivatives of `uv`.
fn sample_tile_albedo_client(uv: vec2<f32>, tile: TileUniform, gradients: LandGradients) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    return sample_page_tile_albedo_client(uv, tile, gradients);
  }
  if (tile.texture_size == 1u) {
    let dims = vec2<i32>(textureDimensions(tex_big));
    return client_filtered_rect_sampled(tex_big, tex_small_sampler, layer, vec2<i32>(0), dims, uv * vec2<f32>(dims), true,
      gradients.dx * vec2<f32>(dims), gradients.dy * vec2<f32>(dims), scene.client_view_scale).rgb;
  }
  let dims = vec2<i32>(textureDimensions(tex_small));
  return client_filtered_rect_sampled(tex_small, tex_small_sampler, layer, vec2<i32>(0), dims, uv * vec2<f32>(dims), true,
    gradients.dx * vec2<f32>(dims), gradients.dy * vec2<f32>(dims), scene.client_view_scale).rgb;
}

// Gradient-stable tap: explicit dPdx/dPdy keep LOD consistent across multi-tap
// blur kernels, matching the mip chosen at the center tap.
// NOTE: the WGSL signature is textureSampleGrad(tex, sampler, uv, layer, ddx, ddy).
fn sample_tile_albedo_grad(uv: vec2<f32>, tile: TileUniform, ddx_uv: vec2<f32>, ddy_uv: vec2<f32>) -> vec3<f32> {
  let layer: i32 = i32(tile.texture_layer);
  let use_linear = effects.enable_linear_filtering == 1u;

  if (tile.texture_size == LAND_SOURCE_MISSING) {
    return missing_texture_pattern(uv);
  }
  if (tile.texture_size == LAND_SOURCE_NODRAW) {
    return LAND_NODRAW_COLOR;
  }
  if (tile.texture_size == LAND_SOURCE_CLASSIC_ART) {
    return sample_classic_land_art(fract(uv), tile.texture_payload);
  }

  if (tile.texture_size == 2u || tile.texture_size == 4u) {
    let tile_dims = max(vec2<f32>(tile.texture_extent), vec2<f32>(1.0));
    if (use_linear) {
      let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
      var seam_uv: vec2<f32>;
      if (tile.texture_size == 2u || (tile.texture_size == 4u && tile.is_wet == 0u
          && (tile.terrain_flags & 4u) == 0u && (effects.terrain_transition_flags & 1u) != 0u)) {
        seam_uv = pixel_art_uv_iq_grad(uv, tile_dims, ddx_uv, ddy_uv);
      } else {
        seam_uv = pixel_art_uv_iq(uv, tile_dims);
      }
      let local_px = clamp(seam_uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
      let atlas_uv = (vec2<f32>(tile.texture_origin) + local_px) / atlas_dims;
      let atlas_ddx = ddx_uv * (tile_dims / atlas_dims);
      let atlas_ddy = ddy_uv * (tile_dims / atlas_dims);
      let color = textureSampleGrad(land_page_atlas, tex_small_sampler, atlas_uv, layer, atlas_ddx, atlas_ddy).rgb;
      return select(color, texture_color_rgb(color), tile.texture_size == 2u);
    } else {
      let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(tile.texture_extent) - 1);
      let atlas_iuv = vec2<i32>(tile.texture_origin) + local_iuv;
      let color = textureLoad(land_page_atlas, atlas_iuv, layer, 0).rgb;
      return select(color, texture_color_rgb(color), tile.texture_size == 2u);
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
// With the land texture variation, each tap of the colour layers' frame reads its neighbours around its own point,
// so that the plain tiling does not show through. `world_xz` is the pixel's world position.
fn apply_sharpening(color: vec3<f32>, uv: vec2<f32>, tile: TileUniform, amount: f32, world_xz: vec2<f32>,
    gradients: LandGradients) -> vec3<f32> {
  if (amount <= 0.0) { return color; }

  // Approximate a 1-pixel offset in UV space
  var fw: vec2<f32>;
  if (tile.texture_size == 2u) { fw = abs(gradients.dx) + abs(gradients.dy); }
  else { fw = fwidth(uv); }
  // A quarter turn swaps the gradients' axes: the offset holds for every tap.
  let off = max(fw.x, fw.y);

  let frame = land_variation_frame(world_xz, LAND_VARIATION_HEX_EDGE_CELLS, vec2<i32>(0), land_texture_variation(tile));
  var sum = LandVariationColorSum(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
  // One call site in a loop: every land sample inlines its texture reads.
  for (var i = land_variation_first_tap(frame); i < land_variation_end_tap(frame); i++) {
    if (land_variation_tap_active(frame, i)) {
      let tap = land_variation_tap(frame, i);
      let tap_uv = select(ec_world_uv(land_variation_tap_point(tap, world_xz), tile), uv, tap.plain);
      let tap_gradients = land_variation_tap_gradients(tap, gradients);
      let s1 = sample_tile_albedo(tap_uv + vec2<f32>(off, 0.0), tile, tap_gradients);
      let s2 = sample_tile_albedo(tap_uv - vec2<f32>(off, 0.0), tile, tap_gradients);
      let s3 = sample_tile_albedo(tap_uv + vec2<f32>(0.0, off), tile, tap_gradients);
      let s4 = sample_tile_albedo(tap_uv - vec2<f32>(0.0, off), tile, tap_gradients);
      land_variation_add_color(&sum, tap, (s1 + s2 + s3 + s4) * 0.25);
    }
  }

  let neighbor_avg = land_variation_resolve_color(sum, frame);
  return color + (color - neighbor_avg) * amount;
}

// Cheap per-tile random in [0,1) for blur kernel decorrelation.
fn rand01_from_tile(ix: i32, iz: i32, layer: u32) -> f32 {
  let p = vec2<f32>(f32(ix) + f32(layer) * 0.618, f32(iz) + f32(layer) * 1.732);
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// 9-tap blur with radius in *screen pixels* via the continuous EC footprint
// (or the existing per-tile fwidth for Classic textures).
// LOD is kept stable by using the same gradients for all taps.
// Kernel direction is randomly rotated per tile to decorrelate tiling artifacts.
fn blurred_albedo(uv: vec2<f32>, tile: TileUniform, radius_in_pixels: f32, world_xz: vec2<f32>, gradients: LandGradients) -> vec3<f32> {
  // Approximate one screen pixel in UV space for this fragment
  var fw: vec2<f32>;
  if (tile.texture_size == 2u) { fw = abs(gradients.dx) + abs(gradients.dy); }
  else { fw = fwidth(uv); }
  let px_uv = max(fw.x, fw.y) + 1e-6;

  // Ensure at least half-pixel radius so it's *noticeable* even at low zoom.
  // The Blur Radius slider starts at this floor (BLUR_RADIUS_MIN_PIXELS in mesh_material.rs).
  let r = max(radius_in_pixels, BLUR_RADIUS_MIN_PIXELS) * px_uv;

  // Keep LOD stable for all taps
  var ddx_uv: vec2<f32>;
  var ddy_uv: vec2<f32>;
  if (tile.texture_size == 2u) { ddx_uv = gradients.dx; ddy_uv = gradients.dy; }
  else { ddx_uv = dpdx(uv); ddy_uv = dpdy(uv); }

  // Decorrelation: derive a tiny rotation per tile/layer (+tiny world jitter)
  let jitter = rand01_from_tile(i32(floor(world_xz.x)), i32(floor(world_xz.y)), tile.texture_layer)
             + fract(world_xz.x * 0.173 + world_xz.y * 0.271) * 0.125;
  let ang = (jitter * 6.2831853); // 2π
  let ca = cos(ang);
  let sa = sin(ang);
  let rot = mat2x2<f32>(ca, -sa, sa, ca);

  // With the land texture variation, each tap of the colour layers' frame blurs around its own point with its turned
  // gradients, so that the plain tiling does not show through. A quarter turn swaps the gradients' axes: the radius
  // holds for every tap.
  let frame = land_variation_frame(world_xz, LAND_VARIATION_HEX_EDGE_CELLS, vec2<i32>(0), land_texture_variation(tile));
  var sum = LandVariationColorSum(vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
  // One call site in a loop: every land sample inlines its texture reads.
  for (var i = land_variation_first_tap(frame); i < land_variation_end_tap(frame); i++) {
    if (land_variation_tap_active(frame, i)) {
      let tap = land_variation_tap(frame, i);
      let tap_uv = select(ec_world_uv(land_variation_tap_point(tap, world_xz), tile), uv, tap.plain);
      let tap_gradients = land_variation_tap_gradients(tap, LandGradients(ddx_uv, ddy_uv));
      land_variation_add_color(&sum, tap, blurred_albedo_kernel(tap_uv, tile, r, rot, tap_gradients.dx, tap_gradients.dy));
    }
  }
  return land_variation_resolve_color(sum, frame);
}

// blurred_albedo's 9-tap kernel of radius r (uv units), turned by rot, around uv.
fn blurred_albedo_kernel(uv: vec2<f32>, tile: TileUniform, r: f32, rot: mat2x2<f32>, ddx_uv: vec2<f32>, ddy_uv: vec2<f32>) -> vec3<f32> {
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
