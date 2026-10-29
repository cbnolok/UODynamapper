//! Cache the original sorted ownership stack at each possible bilinear corner.
//! Shading stays in its existing fragment call sites; only reconstruction moves.
use super::EcTerrainParams;
use bevy::math::UVec2;
mod runtime;
pub(crate) use runtime::{StackCache, sys_prepare};
#[cfg(test)]
mod gpu_tests;

pub(super) const SIDE: usize = 32;
pub(super) const HALO: usize = SIDE + 2;
pub(super) const RECORDS_PER_CORNER: usize = 1 + 17 * 2;
pub(super) const WIDTH: usize = SIDE * RECORDS_PER_CORNER;
pub(super) const HEIGHT: usize = SIDE + 1;
pub(super) const NO_MATERIAL: u32 = u32::MAX;

fn source_params(raw: u16, params: &[Option<EcTerrainParams>], available: &[bool]) -> Option<[u32; 4]> {
    if !available.get(usize::from(raw)).copied().unwrap_or(false) { return None; }
    params.get(usize::from(raw))?.map(|p| p.0).filter(|p| p[3] != 0)
}

fn packed_layer(payload: u16, params: [u32; 4]) -> [u32; 4] {
    [params[0], params[1], u32::from(payload), params[2] | (params[3] << 16)]
}

fn mask_byte(owners: &[u32; SIDE * SIDE], material: u32, x: usize, y: usize, weights: u32) -> u8 {
    let index = y * SIDE + x;
    if owners[index] == material { return 255; }
    let horizontal = x > 0 && x < SIDE - 1;
    let vertical = y > 0 && y < SIDE - 1;
    if horizontal && (owners[index - 1] == material || owners[index + 1] == material) {
        return weights as u8;
    }
    if vertical && (owners[index - SIDE] == material || owners[index + SIDE] == material) {
        return weights as u8;
    }
    if horizontal && vertical && [index - SIDE - 1, index - SIDE + 1, index + SIDE - 1, index + SIDE + 1]
        .into_iter().any(|at| owners[at] == material) {
        return (weights >> 8) as u8;
    }
    0
}

pub(super) fn build_stack_layer(chunk: UVec2, map: u32, raw: &[u16; HALO * HALO],
    params: &[Option<EcTerrainParams>], available: &[bool], limits: [u16; 2]) -> Option<Vec<[u32; 4]>> {
    // Base selection is independent of image residency, just as in the shader.
    let base = params.get(usize::from(limits[0]))?.as_ref()?.0;
    let last_key = params.get(usize::from(limits[1]))?.as_ref()?.0[1] as i32;
    if base[3] == 0 { return None; }
    let material = |x: usize, y: usize| source_params(raw[y * HALO + x], params, available)
        .map(|p| p[0]).unwrap_or(NO_MATERIAL);
    let mut owners = [NO_MATERIAL; SIDE * SIDE];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let Some(p) = source_params(raw[(y + 1) * HALO + x + 1], params, available) else { continue; };
            if p[1] as i32 > last_key { continue; }
            let mut owned = true;
            if p[3] & 8 != 0 {
                let dx = if x == 0 { -1 } else if x == SIDE - 1 { 1 } else { 0 };
                let dy = if y == 0 { -1 } else if y == SIDE - 1 { 1 } else { 0 };
                let outside_x = (x as i32 + 1 + dx) as usize;
                let outside_y = (y as i32 + 1 + dy) as usize;
                if dx != 0 { owned &= material(outside_x, y + 1) == p[0]; }
                if dy != 0 { owned &= material(x + 1, outside_y) == p[0]; }
                if dx != 0 && dy != 0 { owned &= material(outside_x, outside_y) == p[0]; }
            }
            if owned { owners[y * SIDE + x] = p[0]; }
        }
    }
    let mut output = vec![[0; 4]; WIDTH * HEIGHT];
    let mut layers = Vec::with_capacity(17);
    for y in 0..SIDE {
        for x in 0..SIDE {
            layers.clear();
            layers.push(packed_layer(limits[0], base));
            // Match the shader's clipped 4x4 row-major scan, including its first
            // alias. Raw payloads also seed grain and legacy auxiliary lookups.
            for dy in -1..=2 {
                for dx in -1..=2 {
                    let (px, py) = (x as i32 + dx, y as i32 + dy);
                    if !(0..SIDE as i32).contains(&px) || !(0..SIDE as i32).contains(&py) { continue; }
                    let (px, py) = (px as usize, py as usize);
                    let owner = owners[py * SIDE + px];
                    if owner == NO_MATERIAL || layers.iter().any(|p| p[0] == owner) { continue; }
                    let payload = raw[(py + 1) * HALO + px + 1];
                    layers.push(packed_layer(payload, source_params(payload, params, available)?));
                }
            }
            // Equal signed keys retain scan order, as in the insertion sort.
            layers.sort_by_key(|p| p[1] as i32);
            let offset = y * WIDTH + x * RECORDS_PER_CORNER;
            let next_x = (x + 1).min(SIDE - 1);
            let next_y = (y + 1).min(SIDE - 1);
            let raw_pair = |row: usize| u32::from(raw[(row + 1) * HALO + x + 1])
                | (u32::from(raw[(row + 1) * HALO + next_x + 1]) << 16);
            output[offset] = [layers.len() as u32, raw_pair(y), raw_pair(next_y), 0];
            for (index, layer) in layers.iter().copied().enumerate() {
                let alpha = if index == 0 { [1.0f32.to_bits(); 4] } else {
                    [(x, y), (next_x, y), (x, next_y), (next_x, next_y)].map(|(px, py)| {
                        let byte = mask_byte(&owners, layer[0], px, py, layer[3]);
                        // Match the original shader's literal owned value and
                        // constant-reciprocal cardinal/diagonal normalization.
                        let value = if byte == 255 { 1.0 } else { f32::from(byte) * (1.0f32 / 255.0) };
                        value.to_bits()
                    })
                };
                output[offset + 1 + index * 2] = layer;
                output[offset + 2 + index * 2] = alpha;
            }
        }
    }
    output[SIDE * WIDTH] = [chunk.x, chunk.y, map, 1];
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<Option<EcTerrainParams>>, Vec<bool>, [u16; HALO * HALO]) {
        let mut params = vec![None; 64];
        params[1] = Some(EcTerrainParams([1, 1, 242 | (153 << 8), 137]));
        params[2] = Some(EcTerrainParams([2, 2, 242 | (153 << 8), 137]));
        params[3] = params[2];
        let mut raw = [2; HALO * HALO];
        raw[HALO + 1] = 1;
        (params, vec![true; 64], raw)
    }

    #[test]
    fn preserves_canonical_base_and_first_nonbase_alias() {
        let (params, available, mut raw) = fixture();
        raw[15 * HALO + 15] = 3; // First candidate for corner (15,15).
        let output = build_stack_layer(UVec2::new(4, 5), 7, &raw, &params, &available, [1, 2]).unwrap();
        let at = 15 * WIDTH + 15 * RECORDS_PER_CORNER;
        assert_eq!(output[at][0], 2);
        assert_eq!(output[at + 1][2], 1);
        assert_eq!(output[at + 3][2], 3);
        assert_eq!(output[at + 4], [1.0f32.to_bits(); 4]);
        assert_eq!(output[SIDE * WIDTH], [4, 5, 7, 1]);
    }

    #[test]
    fn positive_edge_membership_requires_exterior_owners() {
        let (mut params, available, mut raw) = fixture();
        for x in 0..HALO { raw[x] = 1; }
        for y in 0..HALO { raw[y * HALO] = 1; }
        raw[HALO + 1] = 2;
        let output = build_stack_layer(UVec2::ZERO, 0, &raw, &params, &available, [1, 2]).unwrap();
        assert_eq!(output[4][0], 0.0f32.to_bits());
        params[2].as_mut().unwrap().0[3] = 1; // Point mask, no positive exponent.
        let output = build_stack_layer(UVec2::ZERO, 0, &raw, &params, &available, [1, 2]).unwrap();
        assert_eq!(output[4][0], 1.0f32.to_bits());
    }
}
