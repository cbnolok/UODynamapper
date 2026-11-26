// Optional Classic terrain blending. Where a neighbouring Classic texmap meets this one along a visible seam, its
// texture fades in across the shared edge through a soft, noise-broken boundary. Every weight is a function of the
// world position alone, so a shared edge or corner has the same material weights from every owning cell.
import package::shaders::world::land::land_bindings::TileUniform;
import package::shaders::world::land::atlas::{atlas_read_meta, atlas_read_cc_edge_colors};
import package::shaders::world::land::sampling::{LandGradients, sample_tile_albedo_grad};
import package::shaders::world::land::transitions::terrain_never_blends;
import package::shaders::world::land::noise::noise_2d;

// The sides of a texmap in atlas_read_cc_edge_colors' order (land.rs `cc_texmap_edge_colors`): low and high u, low and
// high v; u runs along world x, v along world y.
const CC_EDGE_LOW_X: u32 = 0u;
const CC_EDGE_HIGH_X: u32 = 1u;
const CC_EDGE_LOW_Y: u32 = 2u;
const CC_EDGE_HIGH_Y: u32 = 3u;
// Set in the alpha byte of the first edge colour when the texmap may blend: it is not drawn for a built surface
// (paving, floors, brick, land.rs CC_TEXMAP_BLENDS), whose patterns end exactly at the cell's edge.
const CC_TRANSITION_BLENDS: u32 = 0xFF000000u;
// Two texmaps whose facing edges differ by less than this (sRGB distance) already meet without a seam: the same
// material's variants, and the Classic hand-drawn transition texmaps, whose edges were painted to match their
// neighbours. They do not blend. The blend reaches full strength at CC_TRANSITION_SEAM_FULL.
const CC_TRANSITION_SEAM_MIN: f32 = 0.05;
const CC_TRANSITION_SEAM_FULL: f32 = 0.12;
// The boundary between two cells: half the width of its soft ramp, and how far the noise moves it either way, in
// fractions of the distance between the two cells' centres. Their sum stays below a half, so a cell's centre is always
// its own texture.
const CC_TRANSITION_SOFTNESS: f32 = 0.2;
const CC_TRANSITION_NOISE_AMPLITUDE: f32 = 0.28;
// The boundary noise: two octaves of value noise, this many cells to a tile for the first, the second at
// CC_TRANSITION_NOISE_OCTAVE_SCALE times the frequency and half the weight, its spread around a half stretched by
// CC_TRANSITION_NOISE_CONTRAST (the sum of two octaves rarely strays far from a half), and the offset that
// decorrelates the noise moving the y boundaries from the one moving the x boundaries.
const CC_TRANSITION_NOISE_CELLS_PER_TILE: f32 = 1.7;
const CC_TRANSITION_NOISE_OCTAVE_SCALE: f32 = 2.17;
const CC_TRANSITION_NOISE_CONTRAST: f32 = 2.5;
const CC_TRANSITION_NOISE_Y_OFFSET: vec2<f32> = vec2<f32>(37.1, 11.3);

// Black land takes no part in the blend either way (transitions.wesl terrain_never_blends), nor do wet or liquid
// tiles (shorelines keep their authored art), Classic land art (land without a texmap), a missing texmap or, through
// its edge colours' flag (cc_transition_blends), a built surface.
fn cc_transition_dry_material(tile: TileUniform) -> bool {
  return tile.texture_size == 4u && tile.is_wet == 0u && !terrain_never_blends(tile)
    && (tile.terrain_flags & 4u) == 0u && all(tile.texture_extent > vec2<u32>(0u));
}

fn cc_transition_same_texture(left: TileUniform, right: TileUniform) -> bool {
  // Raw land IDs can alias the same packaged texmap.
  return left.texture_layer == right.texture_layer
    && all(left.texture_origin == right.texture_origin)
    && all(left.texture_extent == right.texture_extent);
}

fn cc_transition_blends(colors: vec4<u32>) -> bool {
  return (colors[CC_EDGE_LOW_X] & CC_TRANSITION_BLENDS) != 0u;
}

// The mean colour of a texmap, its edge colours `colors`, along the side `side` (CC_EDGE_*), or, when `corner_side`
// names the other axis' side, at that corner.
fn cc_transition_edge_color(colors: vec4<u32>, side: u32, corner_side: u32) -> vec3<f32> {
  let edge = unpack4x8unorm(colors[side]).rgb;
  if (corner_side == side) { return edge; }
  return 0.5 * (edge + unpack4x8unorm(colors[corner_side]).rgb);
}

// How strongly `neighbor`, at `offset` from `current`, blends into it: 0 when either takes no part, when both are one
// texmap, or when their facing edges already match; 1 along a plain seam between two different textures.
fn cc_transition_seam(current: TileUniform, current_colors: vec4<u32>, neighbor: TileUniform,
    offset: vec2<i32>) -> f32 {
  if (!cc_transition_dry_material(neighbor) || cc_transition_same_texture(current, neighbor)) { return 0.0; }
  let neighbor_colors = atlas_read_cc_edge_colors(neighbor.texture_payload);
  if (!cc_transition_blends(neighbor_colors)) { return 0.0; }
  let current_x = select(CC_EDGE_LOW_X, CC_EDGE_HIGH_X, offset.x > 0);
  let neighbor_x = select(CC_EDGE_HIGH_X, CC_EDGE_LOW_X, offset.x > 0);
  let current_y = select(CC_EDGE_LOW_Y, CC_EDGE_HIGH_Y, offset.y > 0);
  let neighbor_y = select(CC_EDGE_HIGH_Y, CC_EDGE_LOW_Y, offset.y > 0);
  let current_side = select(current_y, current_x, offset.x != 0);
  let neighbor_side = select(neighbor_y, neighbor_x, offset.x != 0);
  let current_corner = select(current_side, current_y, offset.x != 0 && offset.y != 0);
  let neighbor_corner = select(neighbor_side, neighbor_y, offset.x != 0 && offset.y != 0);
  let difference = distance(cc_transition_edge_color(current_colors, current_side, current_corner),
    cc_transition_edge_color(neighbor_colors, neighbor_side, neighbor_corner));
  return smoothstep(CC_TRANSITION_SEAM_MIN, CC_TRANSITION_SEAM_FULL, difference);
}

fn cc_transition_noise(world: vec2<f32>) -> f32 {
  let p = world * CC_TRANSITION_NOISE_CELLS_PER_TILE;
  let octaves = (2.0 * noise_2d(p) + noise_2d(p * CC_TRANSITION_NOISE_OCTAVE_SCALE)) / 3.0;
  return saturate(0.5 + (octaves - 0.5) * CC_TRANSITION_NOISE_CONTRAST);
}

// The weight of the higher cell along one axis, `ramp` running from the lower cell's centre (0) to the higher one's (1)
// and `noise` (0 to 1) moving the boundary between them.
fn cc_transition_axis_weight(ramp: f32, noise: f32) -> f32 {
  let boundary = 0.5 + (2.0 * noise - 1.0) * CC_TRANSITION_NOISE_AMPLITUDE;
  return smoothstep(boundary - CC_TRANSITION_SOFTNESS, boundary + CC_TRANSITION_SOFTNESS, ramp);
}

// The weight of the neighbour toward `direction` (-1 or 1) along one axis at `uv` (0 to 1 across the cell).
fn cc_transition_neighbor_weight(uv: f32, direction: i32, noise: f32) -> f32 {
  if (direction > 0) { return cc_transition_axis_weight(uv - 0.5, noise); }
  return 1.0 - cc_transition_axis_weight(uv + 0.5, noise);
}

fn blend_cc_terrain_transitions(base_color: vec3<f32>, current: TileUniform,
    cell: vec2<i32>, uv: vec2<f32>, gradients: LandGradients) -> vec3<f32> {
  if (!cc_transition_dry_material(current)) { return base_color; }
  let current_colors = atlas_read_cc_edge_colors(current.texture_payload);
  if (!cc_transition_blends(current_colors)) { return base_color; }
  let direction = select(vec2<i32>(-1), vec2<i32>(1), uv >= vec2<f32>(0.5));
  let offsets = array<vec2<i32>, 3>(vec2<i32>(direction.x, 0), vec2<i32>(0, direction.y), direction);
  var seams: array<f32, 3>;
  for (var index = 0u; index < 3u; index++) {
    let offset = offsets[index];
    seams[index] = cc_transition_seam(current, current_colors, atlas_read_meta(cell.x + offset.x, cell.y + offset.y),
      offset);
  }
  // Keep homogeneous regions and matching seams exact, and spare them the noise.
  if (max(seams[0], max(seams[1], seams[2])) <= 0.0) { return base_color; }
  let world = vec2<f32>(cell) + uv;
  let blend = vec2<f32>(
    cc_transition_neighbor_weight(uv.x, direction.x, cc_transition_noise(world)),
    cc_transition_neighbor_weight(uv.y, direction.y, cc_transition_noise(world + CC_TRANSITION_NOISE_Y_OFFSET)));
  let weights = array<f32, 3>(blend.x * (1.0 - blend.y), blend.y * (1.0 - blend.x), blend.x * blend.y);
  var color = base_color;
  // One sampling call site in a loop (docs/agent_notes/shader_compile_progress.md).
  for (var index = 0u; index < 3u; index++) {
    let weight = weights[index] * seams[index];
    if (weight <= 0.0) { continue; }
    let offset = offsets[index];
    let neighbor = atlas_read_meta(cell.x + offset.x, cell.y + offset.y);
    // The neighbour's texmap mirrored across the shared edge: its own texels next to the seam, never its far side.
    // Classic transition texmaps do not tile, so repeating one would bring its other material up to the seam.
    let neighbor_uv = select(uv, vec2<f32>(1.0) - uv, offset != vec2<i32>(0));
    let sample = sample_tile_albedo_grad(neighbor_uv, neighbor, gradients.dx, gradients.dy);
    color += (sample - base_color) * weight;
  }
  return color;
}
