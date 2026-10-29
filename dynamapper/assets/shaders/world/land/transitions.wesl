// Exact terrain boundary weights shared by reconstruction and cached grime.
import package::shaders::world::land::land_bindings::TileUniform;
import package::shaders::world::land::atlas::atlas_read_meta;

const TERRAIN_FLAG_REVIEWED_LIQUID: u32 = 0x4u;
const EC_TERRAIN_TRANSITION_WIDTH: f32 = 0.82;

fn ec_transition_edge_weight(edge_distance: f32) -> f32 {
  return 1.0 - smoothstep(0.0, EC_TERRAIN_TRANSITION_WIDTH, edge_distance);
}

fn ec_transition_neighbor_weight(offset: vec2<i32>, uv_in_tile: vec2<f32>) -> f32 {
  var weight = 0.0;

  if (offset.x < 0) {
    weight = max(weight, ec_transition_edge_weight(uv_in_tile.x));
  } else if (offset.x > 0) {
    weight = max(weight, ec_transition_edge_weight(1.0 - uv_in_tile.x));
  }

  if (offset.y < 0) {
    weight = max(weight, ec_transition_edge_weight(uv_in_tile.y));
  } else if (offset.y > 0) {
    weight = max(weight, ec_transition_edge_weight(1.0 - uv_in_tile.y));
  }

  return weight;
}

fn ec_transition_materials_compatible(current: TileUniform, neighbor: TileUniform) -> bool {
  let current_liquid = current.is_wet == 1u || (current.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  let neighbor_liquid = neighbor.is_wet == 1u || (neighbor.terrain_flags & TERRAIN_FLAG_REVIEWED_LIQUID) != 0u;
  // A liquid surface covers its substrate through authored coverage, not the
  // solid-material neighbor blend. Mixing underwater sand into water paints
  // a pale sawtooth border at the terrain/static ownership boundary.
  return current_liquid == neighbor_liquid;
}

fn ec_transition_boundary_neighbor_weight(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  offset: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (neighbor_tile.texture_size != 2u) {
    return 0.0;
  }

  if (!ec_transition_materials_compatible(current_tile, neighbor_tile)) {
    return 0.0;
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return 0.0;
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return 0.0;
  }

  return ec_transition_neighbor_weight(offset, uv_in_tile);
}

fn ec_transition_diagonal_weight(offset: vec2<i32>, uv_in_tile: vec2<f32>) -> f32 {
  let x_weight = ec_transition_neighbor_weight(vec2<i32>(offset.x, 0), uv_in_tile);
  let y_weight = ec_transition_neighbor_weight(vec2<i32>(0, offset.y), uv_in_tile);
  return min(x_weight, y_weight) * 0.75;
}

fn ec_transition_boundary_diagonal_weight(
  current_tile: TileUniform,
  neighbor_tile: TileUniform,
  offset: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (neighbor_tile.texture_size != 2u) {
    return 0.0;
  }

  if (!ec_transition_materials_compatible(current_tile, neighbor_tile)) {
    return 0.0;
  }

  if (neighbor_tile.texture_payload == current_tile.texture_payload) {
    return 0.0;
  }

  if (neighbor_tile.texture_extent.x == 0u || neighbor_tile.texture_extent.y == 0u) {
    return 0.0;
  }

  return ec_transition_diagonal_weight(offset, uv_in_tile);
}

fn ec_transition_boundary_weight(
  current_tile: TileUniform,
  world_tile: vec2<i32>,
  uv_in_tile: vec2<f32>,
) -> f32 {
  if (current_tile.texture_size != 2u) {
    return 0.0;
  }

  var weight = 0.0;
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y), vec2<i32>(-1, 0), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y), vec2<i32>(1, 0), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x, world_tile.y - 1), vec2<i32>(0, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_neighbor_weight(current_tile, atlas_read_meta(world_tile.x, world_tile.y + 1), vec2<i32>(0, 1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y - 1), vec2<i32>(-1, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y - 1), vec2<i32>(1, -1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x - 1, world_tile.y + 1), vec2<i32>(-1, 1), uv_in_tile));
  weight = max(weight, ec_transition_boundary_diagonal_weight(current_tile, atlas_read_meta(world_tile.x + 1, world_tile.y + 1), vec2<i32>(1, 1), uv_in_tile));
  return clamp(weight, 0.0, 1.0);
}

// Bits are west/east/north/south, then NW/NE/SW/SE. Keep the original
// cardinal/diagonal expressions and accumulation order after the neighbor test.
fn ec_transition_boundary_weight_from_mask(current_tile: TileUniform, uv_in_tile: vec2<f32>, mask: u32) -> f32 {
  if (current_tile.texture_size != 2u) { return 0.0; }
  var weight = 0.0;
  if ((mask & 1u) != 0u) { weight = max(weight, ec_transition_neighbor_weight(vec2<i32>(-1, 0), uv_in_tile)); }
  if ((mask & 2u) != 0u) { weight = max(weight, ec_transition_neighbor_weight(vec2<i32>(1, 0), uv_in_tile)); }
  if ((mask & 4u) != 0u) { weight = max(weight, ec_transition_neighbor_weight(vec2<i32>(0, -1), uv_in_tile)); }
  if ((mask & 8u) != 0u) { weight = max(weight, ec_transition_neighbor_weight(vec2<i32>(0, 1), uv_in_tile)); }
  if ((mask & 16u) != 0u) { weight = max(weight, ec_transition_diagonal_weight(vec2<i32>(-1, -1), uv_in_tile)); }
  if ((mask & 32u) != 0u) { weight = max(weight, ec_transition_diagonal_weight(vec2<i32>(1, -1), uv_in_tile)); }
  if ((mask & 64u) != 0u) { weight = max(weight, ec_transition_diagonal_weight(vec2<i32>(-1, 1), uv_in_tile)); }
  if ((mask & 128u) != 0u) { weight = max(weight, ec_transition_diagonal_weight(vec2<i32>(1, 1), uv_in_tile)); }
  return clamp(weight, 0.0, 1.0);
}
