// Level liquids must not inherit the upward reach of a sloping ownership quad.
// This is a renderer policy layered over the original material coverage masks.
import package::shaders::world::land::land_bindings::TileUniform;
import package::shaders::world::land::atlas::atlas_read_meta;
import package::shaders::world::land::coverage::ec_coverage_params;

struct EcLiquidBank {
    cell: vec2<i32>,
    weight: f32,
}

fn ec_liquid_bank_weight(surface_height: f32, liquid_level: f32) -> f32 {
    // Half to one-and-a-half native height units (runtime scale is 0.1).
    return smoothstep(0.05, 0.15, surface_height - liquid_level);
}

fn ec_liquid_bank(world: vec3<f32>, current: TileUniform, liquid_material: u32) -> EcLiquidBank {
    let cell = vec2<i32>(floor(world.xz));
    var result = EcLiquidBank(cell, 0.0);
    if (current.texture_size != 2u) { return result; }
    let current_liquid = (current.terrain_flags & 4u) != 0u;
    // Homogeneous, level liquid incurs no neighborhood fetches or extra shading.
    if (current_liquid && world.y <= current.tile_height + 0.05) { return result; }
    if (current_liquid && ec_coverage_params(current.texture_payload).x != liquid_material) { return result; }

    let local = fract(world.xz);
    var level = current.tile_height;
    var nearest = 1e20;
    var found = false;
    for (var index = 0; index < 9; index++) {
        if (index == 4) { continue; } // The owning cell cannot be an opposite-class neighbor.
        let offset = vec2<i32>(index % 3 - 1, index / 3 - 1);
        let candidate = atlas_read_meta(cell.x + offset.x, cell.y + offset.y);
        if (candidate.texture_size != 2u || any(candidate.texture_extent == vec2<u32>(0u))) { continue; }
        let candidate_liquid = (candidate.terrain_flags & 4u) != 0u;
        if (candidate_liquid == current_liquid) { continue; }
        let delta = vec2<f32>(offset) + vec2<f32>(0.5) - local;
        let distance = dot(delta, delta);
        if (distance >= nearest) { continue; }
        if (current_liquid) {
            // Do not pull underwater sand or a lower shelf into a liquid surface.
            if (candidate.tile_height <= level + 0.05) { continue; }
            result.cell = cell + offset;
        } else {
            // Payload aliases of one original liquid share the same level rule.
            if (ec_coverage_params(candidate.texture_payload).x != liquid_material) { continue; }
            level = candidate.tile_height;
        }
        nearest = distance;
        found = true;
    }
    if (found) { result.weight = ec_liquid_bank_weight(world.y, level); }
    return result;
}
