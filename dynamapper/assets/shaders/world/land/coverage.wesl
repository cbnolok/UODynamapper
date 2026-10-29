// Original per-material 32x32 ownership masks, evaluated from the paged map.
// A bilinear mask footprint plus its one-cell expansion touches at most 4x4
// ownership cells. Keep every distinct material there and the chunk's base;
// this bound does not truncate the chunk's (up to 49) original material passes.
import package::shaders::world::land::land_bindings::{
    TileUniform, land_page_lookup, ec_terrain_bases, ec_terrain_stacks,
};
import package::shaders::world::land::atlas::atlas_read_meta;

const NO_MATERIAL: u32 = 0xffffffffu;

struct EcTerrainStack {
    layers: array<vec4<u32>, 17>,
    alpha: array<f32, 17>,
    count: u32,
}

fn ec_coverage_params(payload: u32) -> vec4<u32> {
    let index = 6u * 16384u + payload;
    let dims = textureDimensions(land_page_lookup);
    // Classic lookups have no EC coverage records.
    if (payload >= 16384u || index >= dims.x * dims.y) { return vec4<u32>(0u); }
    return textureLoad(land_page_lookup, vec2<i32>(i32(index % dims.x), i32(index / dims.x)), 0);
}

fn ec_material_at(cell: vec2<i32>) -> u32 {
    let tile = atlas_read_meta(cell.x, cell.y);
    if (tile.texture_size != 2u) { return NO_MATERIAL; }
    let params = ec_coverage_params(tile.texture_payload);
    return select(NO_MATERIAL, params.x, params.w != 0u);
}

fn ec_pack_layer(payload: u32, params: vec4<u32>) -> vec4<u32> {
    return vec4<u32>(params.xy, payload, params.z | (params.w << 16u));
}

fn ec_layer_tile(layer: vec4<u32>, height: f32) -> TileUniform {
    let dims = textureDimensions(land_page_lookup);
    let slot = textureLoad(land_page_lookup, vec2<i32>(i32(layer.z % dims.x), i32(layer.z / dims.x)), 0);
    let extent = vec2<u32>(slot.w & 65535u, slot.w >> 16u);
    let flags = layer.w >> 16u;
    let wet = u32((flags & 7u) == 2u);
    return TileUniform(height, select(3u, 2u, all(extent > vec2<u32>(0u))),
        slot.x & 65535u, 0u, slot.yz, extent, wet, (flags >> 4u) & 7u,
        layer.z, f32(slot.x >> 16u) / 256.0);
}

fn ec_mask_byte(owners: array<u32, 16>, material: u32, point: vec2<i32>, corner: vec2<i32>, weights: vec2<f32>) -> f32 {
    let q = point - corner + vec2<i32>(1);
    let index = u32(q.y * 4 + q.x);
    if (owners[index] == material) { return 1.0; }
    let horizontal = point.x > 0 && point.x < 31;
    let vertical = point.y > 0 && point.y < 31;
    if (horizontal && (owners[index - 1u] == material || owners[index + 1u] == material)) {
        return weights.x;
    }
    if (vertical && (owners[index - 4u] == material || owners[index + 4u] == material)) {
        return weights.x;
    }
    if (horizontal && vertical && (owners[index - 5u] == material || owners[index - 3u] == material
            || owners[index + 3u] == material || owners[index + 5u] == material)) {
        return weights.y;
    }
    return 0.0;
}

fn ec_terrain_stack_uncached(world: vec2<f32>) -> EcTerrainStack {
    var stack: EcTerrainStack;
    let chunk = vec2<i32>(floor(world / 32.0));
    let base_dims = vec2<i32>(textureDimensions(ec_terrain_bases));
    if (any(chunk < vec2<i32>(0)) || any(chunk >= base_dims)) { return stack; }
    let limits = textureLoad(ec_terrain_bases, chunk, 0).xy;
    let payload = limits.x;
    if (payload == 65535u) { return stack; }
    let params = ec_coverage_params(payload);
    if (params.w == 0u) { return stack; }
    let last_key = i32(ec_coverage_params(limits.y).y);
    stack.layers[0] = ec_pack_layer(payload, params);
    stack.count = 1u;
    let origin = chunk * 32;
    let local = world - vec2<f32>(origin);
    let sample_position = clamp(local - vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(31.0));
    let corner = vec2<i32>(floor(sample_position));
    var owners: array<u32, 16>;
    for (var index = 0u; index < 16u; index++) {
        owners[index] = NO_MATERIAL;
        let point = corner + vec2<i32>(i32(index % 4u), i32(index / 4u)) - vec2<i32>(1);
        if (any(point < vec2<i32>(0)) || any(point > vec2<i32>(31))) { continue; }
        let cell = origin + point;
        let tile = atlas_read_meta(cell.x, cell.y);
        if (tile.texture_size != 2u) { continue; }
        let record = ec_coverage_params(tile.texture_payload);
        if (record.w == 0u || i32(record.y) > last_key) { continue; }
        var owned = true;
        if ((record.w & 8u) != 0u) {
            // Positive-exponent membership at an edge also requires the
            // adjoining exterior cell, or all three exterior corner cells.
            let edge = vec2<i32>(select(0, -1, point.x == 0) + select(0, 1, point.x == 31),
                select(0, -1, point.y == 0) + select(0, 1, point.y == 31));
            if (edge.x != 0) { owned = owned && ec_material_at(cell + vec2<i32>(edge.x, 0)) == record.x; }
            if (edge.y != 0) { owned = owned && ec_material_at(cell + vec2<i32>(0, edge.y)) == record.x; }
            if (all(edge != vec2<i32>(0))) { owned = owned && ec_material_at(cell + edge) == record.x; }
        }
        if (!owned) { continue; }
        owners[index] = record.x;
        var found = false;
        for (var layer = 0u; layer < stack.count; layer++) {
            found = found || stack.layers[layer].x == record.x;
        }
        if (!found) {
            stack.layers[stack.count] = ec_pack_layer(tile.texture_payload, record);
            stack.count++;
        }
    }
    // Source keys sort water after solids; aliases of one material are one pass.
    for (var index = 1u; index < stack.count; index++) {
        let value = stack.layers[index];
        var previous = index;
        while (previous > 0u && i32(stack.layers[previous - 1u].y) > i32(value.y)) {
            stack.layers[previous] = stack.layers[previous - 1u];
            previous--;
        }
        stack.layers[previous] = value;
    }
    stack.alpha[0] = 1.0;
    let fraction = fract(sample_position);
    for (var index = 1u; index < stack.count; index++) {
        let layer = stack.layers[index];
        let flags = layer.w >> 16u;
        let weights = vec2<f32>(f32(layer.w & 255u), f32((layer.w >> 8u) & 255u)) / 255.0;
        if ((flags & 128u) == 0u) {
            // The zero-exponent mask has point min/mag filtering.
            let nearest = clamp(vec2<i32>(floor(local)), vec2<i32>(0), vec2<i32>(31));
            stack.alpha[index] = ec_mask_byte(owners, layer.x, nearest, corner, weights);
        } else {
            let next = min(corner + vec2<i32>(1), vec2<i32>(31));
            let north = mix(ec_mask_byte(owners, layer.x, corner, corner, weights),
                ec_mask_byte(owners, layer.x, vec2<i32>(next.x, corner.y), corner, weights), fraction.x);
            let south = mix(ec_mask_byte(owners, layer.x, vec2<i32>(corner.x, next.y), corner, weights),
                ec_mask_byte(owners, layer.x, next, corner, weights), fraction.x);
            stack.alpha[index] = mix(north, south, fraction.y);
        }
    }
    return stack;
}

struct EcTerrainCache {
    count: u32,
    slot: u32,
    at: vec2<i32>,
    fraction: vec2<f32>,
    delta: vec2<i32>,
}

struct EcTerrainLayer {
    material: vec4<u32>,
    alpha: f32,
}

// This view contains no fallback arrays to initialize in the fragment entry.
fn ec_terrain_cache(world: vec2<f32>) -> EcTerrainCache {
    var cache: EcTerrainCache;
    let chunk = vec2<i32>(floor(world / 32.0));
    let dims = vec2<i32>(textureDimensions(ec_terrain_bases));
    if (all(chunk >= vec2<i32>(0)) && all(chunk < dims)) {
        let limits = textureLoad(ec_terrain_bases, chunk, 0);
        if (limits.z < 16u) {
            let guard = textureLoad(ec_terrain_stacks, vec2<i32>(0, 32), i32(limits.z), 0);
            if (all(guard.xy == vec2<u32>(chunk)) && guard.z == limits.w && guard.w == 1u) {
                let local = world - vec2<f32>(chunk * 32);
                let sample_position = clamp(local - vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(31.0));
                let corner = vec2<i32>(floor(sample_position));
                let header_at = vec2<i32>(corner.x * 35, corner.y);
                let header = textureLoad(ec_terrain_stacks, header_at, i32(limits.z), 0);
                let nearest = clamp(vec2<i32>(floor(local)), vec2<i32>(0), vec2<i32>(31));
                let delta = min(nearest - corner, vec2<i32>(1));
                let pair = select(header.y, header.z, delta.y != 0);
                let expected = (pair >> (u32(delta.x) * 16u)) & 65535u;
                let cell = vec2<i32>(floor(world));
                let current = atlas_read_meta(cell.x, cell.y);
                if (header.x > 0u && header.x <= 17u && current.texture_size == 2u && current.texture_payload == expected) {
                    cache.count = header.x;
                    cache.slot = limits.z;
                    cache.at = header_at;
                    cache.fraction = fract(sample_position);
                    cache.delta = delta;
                    return cache;
                }
            }
        }
    }
    return cache;
}

fn ec_cache_layer(cache: EcTerrainCache, index: u32) -> EcTerrainLayer {
    let at = cache.at + vec2<i32>(1 + i32(index) * 2, 0);
    let layer = textureLoad(ec_terrain_stacks, at, i32(cache.slot), 0);
    let values = bitcast<vec4<f32>>(textureLoad(ec_terrain_stacks, at + vec2<i32>(1, 0), i32(cache.slot), 0));
    var alpha: f32;
    if (((layer.w >> 16u) & 128u) == 0u) {
        alpha = values[u32(cache.delta.y * 2 + cache.delta.x)];
    } else {
        let fraction = cache.fraction;
        alpha = mix(mix(values.x, values.y, fraction.x), mix(values.z, values.w, fraction.x), fraction.y);
    }
    return EcTerrainLayer(layer, alpha);
}

// Cached layers are already sorted. Their former key word stores four exact
// grime-neighbor masks, selected with the same owning-cell delta as raw guards.
fn ec_cache_grime_mask(cache: EcTerrainCache, material: vec4<u32>) -> u32 {
    let shift = u32(cache.delta.y * 2 + cache.delta.x) * 8u;
    return (material.y >> shift) & 255u;
}
