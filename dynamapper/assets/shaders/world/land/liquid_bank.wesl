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
    // Finish the height fade at the crust plane; the liquid-side shore distance
    // below supplies the soft horizontal blend beneath its authored edge.
    return smoothstep(-0.05, 0.0, surface_height - liquid_level);
}

// XZ is a small corner relaxation; Y is the final terrain height.
fn ec_bank_vertex(cell: vec2<i32>, current: TileUniform) -> vec3<f32> {
    let unchanged = vec3<f32>(0.0, current.tile_height, 0.0);
    if (current.texture_size != 2u) { return unchanged; }
    let current_liquid = (current.terrain_flags & 4u) != 0u;
    var total = current.tile_height * 4.0;
    var weight = 4.0;
    var liquid_level = -1e20;
    var diagonal_bank = vec2<f32>(0.0);
    var diagonal_count = 0.0;
    var cardinal_bank = false;
    for (var index = 0; index < 9; index++) {
        if (index == 4) { continue; }
        let offset = vec2<i32>(index % 3 - 1, index / 3 - 1);
        let candidate = atlas_read_meta(cell.x + offset.x, cell.y + offset.y);
        if (candidate.texture_size != 2u || any(candidate.texture_extent == vec2<u32>(0u))) { continue; }
        let kernel = select(1.0, 2.0, offset.x == 0 || offset.y == 0);
        total += candidate.tile_height * kernel;
        weight += kernel;
        if (current_liquid && (candidate.terrain_flags & 4u) == 0u && candidate.tile_height > current.tile_height + 0.05) {
            if (kernel == 2.0) { cardinal_bank = true; }
            else { diagonal_bank += vec2<f32>(offset); diagonal_count += 1.0; }
        }
        if ((candidate.terrain_flags & 4u) != 0u && candidate.tile_height < current.tile_height - 0.05) {
            liquid_level = max(liquid_level, candidate.tile_height);
        }
    }
    if (current_liquid) {
        if (!cardinal_bank && diagonal_count > 0.0) {
            // Bevel the inset corner of a diagonal shore, leaving straight
            // banks and the liquid/crust height fixed. A quarter cell cannot
            // cross the neighboring grid vertices or fold the shared mesh.
            let offset = diagonal_bank * (0.25 / diagonal_count);
            return vec3<f32>(offset.x, current.tile_height, offset.y);
        }
        return unchanged;
    }
    if (liquid_level == -1e20) { return unchanged; }
    // Keep liquid height fixed and only ease nearby solid peaks/valleys.
    // Global cell coordinates make shared chunk-edge vertices agree.
    return vec3<f32>(0.0, max(liquid_level, current.tile_height + clamp((total / weight - current.tile_height) * 0.35, -0.15, 0.15)), 0.0);
}

fn ec_liquid_bank(world: vec3<f32>, current: TileUniform, liquid_material: u32, liquid_coverage: f32) -> EcLiquidBank {
    let cell = vec2<i32>(floor(world.xz));
    var result = EcLiquidBank(cell, 0.0);
    if (current.texture_size != 2u) { return result; }
    let current_liquid = (current.terrain_flags & 4u) != 0u;
    // Homogeneous, level liquid incurs no neighborhood fetches or extra shading.
    if (current_liquid && world.y <= current.tile_height && liquid_coverage >= 1.0) { return result; }
    if (current_liquid && ec_coverage_params(current.texture_payload).x != liquid_material) { return result; }

    // Linear ownership masks reach a second cell beyond the fragment's owner.
    // Search the same 4x4 footprint as coverage, including across chunk edges.
    let origin = vec2<i32>(floor(world.xz - vec2<f32>(0.5))) - vec2<i32>(1);
    var level = current.tile_height;
    var nearest = 1e20;
    var found = false;
    for (var index = 0; index < 16; index++) {
        let candidate_cell = origin + vec2<i32>(index % 4, index / 4);
        if (all(candidate_cell == cell)) { continue; }
        let candidate = atlas_read_meta(candidate_cell.x, candidate_cell.y);
        if (candidate.texture_size != 2u || any(candidate.texture_extent == vec2<u32>(0u))) { continue; }
        let candidate_liquid = (candidate.terrain_flags & 4u) != 0u;
        if (candidate_liquid == current_liquid) { continue; }
        let delta = vec2<f32>(candidate_cell) + vec2<f32>(0.5) - world.xz;
        let distance = dot(delta, delta);
        if (distance >= nearest) { continue; }
        if (current_liquid) {
            // Do not pull underwater sand or a lower shelf into a liquid surface.
            if (candidate.tile_height <= level + 0.05) { continue; }
            result.cell = candidate_cell;
        } else {
            // Payload aliases of one original liquid share the same level rule.
            if (ec_coverage_params(candidate.texture_payload).x != liquid_material) { continue; }
            level = candidate.tile_height;
        }
        nearest = distance;
        found = true;
    }
    if (found) {
        result.weight = ec_liquid_bank_weight(world.y, level);
        if (!current_liquid && current.tile_height <= level + 0.05) {
            // Level marsh/substrate ownership is not a raised bank. Preserve
            // its original liquid coverage unless the mesh actually rises.
            result.weight = smoothstep(0.0, 0.025, world.y - level);
        }
        if (current_liquid) {
            // Limit the below-plane half of the blend to the bank's edge.
            // Interior flat liquid stays fully covered, even in mixed chunks.
            let distance = length(max(abs(world.xz - vec2<f32>(result.cell) - vec2<f32>(0.5)) - vec2<f32>(0.5), vec2<f32>(0.0)));
            let shore = 1.0 - smoothstep(0.0, 0.5, distance);
            result.weight = max(result.weight * shore, smoothstep(0.0, 0.025, world.y - level));
        }
    }
    return result;
}
