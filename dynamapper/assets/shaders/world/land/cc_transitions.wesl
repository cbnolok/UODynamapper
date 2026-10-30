// Optional Classic terrain blending. Interpolate between cell centers so a
// shared edge/corner has the same material weights from every owning cell.
import package::shaders::world::land::land_bindings::TileUniform;
import package::shaders::world::land::atlas::atlas_read_meta;
import package::shaders::world::land::sampling::{LandGradients, sample_tile_albedo_grad};

fn cc_transition_dry_material(tile: TileUniform) -> bool {
  return tile.texture_size == 4u && tile.is_wet == 0u
    && (tile.terrain_flags & 4u) == 0u && all(tile.texture_extent > vec2<u32>(0u));
}

fn cc_transition_same_texture(left: TileUniform, right: TileUniform) -> bool {
  // Raw land IDs can alias the same packaged texmap.
  return left.texture_layer == right.texture_layer
    && all(left.texture_origin == right.texture_origin)
    && all(left.texture_extent == right.texture_extent);
}

fn blend_cc_terrain_transitions(base_color: vec3<f32>, current: TileUniform,
    cell: vec2<i32>, uv: vec2<f32>, gradients: LandGradients) -> vec3<f32> {
  if (!cc_transition_dry_material(current)) { return base_color; }
  let direction = select(vec2<i32>(-1), vec2<i32>(1), uv >= vec2<f32>(0.5));
  let blend = smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), abs(uv - vec2<f32>(0.5)));
  let offsets = array<vec2<i32>, 3>(vec2<i32>(direction.x, 0),
    vec2<i32>(0, direction.y), direction);
  let weights = array<f32, 3>(blend.x * (1.0 - blend.y),
    blend.y * (1.0 - blend.x), blend.x * blend.y);
  var total = (1.0 - blend.x) * (1.0 - blend.y);
  var color = base_color * total;
  var different = false;
  for (var index = 0u; index < 3u; index++) {
    if (weights[index] <= 0.0) { continue; }
    let neighbor = atlas_read_meta(cell.x + offsets[index].x, cell.y + offsets[index].y);
    // Never soften authored shorelines or pull missing/foreign sources into CC.
    if (!cc_transition_dry_material(neighbor)) { continue; }
    var sample = base_color;
    if (!cc_transition_same_texture(current, neighbor)) {
      // CC images can contain authored edge details. Extend the adjacent edge
      // into this cell instead of repeating the neighbor's whole image here.
      let neighbor_uv = clamp(uv - vec2<f32>(offsets[index]), vec2<f32>(0.0), vec2<f32>(1.0));
      sample = sample_tile_albedo_grad(neighbor_uv, neighbor, gradients.dx, gradients.dy);
      different = true;
    }
    color += sample * weights[index];
    total += weights[index];
  }
  // Keep homogeneous regions exact, including aliases of one atlas slot.
  if (!different) { return base_color; }
  return color / total;
}
