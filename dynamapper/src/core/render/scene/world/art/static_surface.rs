//! Ground-level, world-textured art keeps its own pixels, but exposes the map
//! smoothly at patch boundaries. Neighbor flags are built only on chunk rebuild.
use super::statics_collect::{SpriteInstance, StaticChunkBatchKey};
use crate::core::{statics::LazyStaticsStore, render::scene::world::land::{tile_atlas::TileAtlas, CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM}};
use std::collections::{HashMap, HashSet};
use udd_assets::{tex_art_ec::TexArtEcPackage, tilemeta::{EcMaterialStableRole, TileMetaItemTextureRef, TileMetaItemTile, TileMetaPackage}};

const BLEND_ENABLED: u32 = 1 << 1;
const NEIGHBOR_SHIFT: u32 = 2;
const OFFSETS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
type MaterialKey = (u32, u32);
type SurfaceCell = (u32, u32, i8, MaterialKey);

fn source_key(tile: &TileMetaItemTile, refs: &[TileMetaItemTextureRef]) -> Option<MaterialKey> {
    // The camera-plane terrain-item flag is source-backed. Exclude volumes,
    // roofs, foliage, water and local/clamped art such as rugs or decals.
    if tile.height != 0 || tile.flags & (1u64 << 34) == 0
        || tile.flags & (0x400 | 0x80 | 0x20000 | 0x10000000) != 0 || tile.is_nodraw() {
        return None;
    }
    let base = refs.iter().find(|r| r.block_index == 0 && r.is_primary_selected() && r.is_world_art())?;
    if base.stable_role() != EcMaterialStableRole::Base || base.uv_set != 1
        || !base.texture_stretch.is_finite() || base.texture_stretch <= 0.0 { return None; }
    Some((base.texture_id, base.texture_stretch.to_bits()))
}

pub(super) fn map_height(atlas: &TileAtlas, x: u32, y: u32) -> Option<i8> {
    let texel = atlas.read_cell(x, y)?;
    if !matches!((texel.g >> 8) & 15, 2 | 4) { return None; }
    Some(((texel.g & 255) as i16 - 128) as i8)
}

fn neighbor_mask(cells: &HashSet<SurfaceCell>, cell: SurfaceCell) -> u32 {
    let mut mask = 0;
    for (bit, (dx, dy)) in OFFSETS.into_iter().enumerate() {
        let (Some(x), Some(y)) = (cell.0.checked_add_signed(dx), cell.1.checked_add_signed(dy)) else { continue; };
        if cells.contains(&(x, y, cell.2, cell.3)) { mask |= 1 << bit; }
    }
    mask
}

pub(super) fn apply_boundary_coverage(
    instances: &mut [SpriteInstance], chunk: StaticChunkBatchKey, height_scale: f32,
    statics: &mut LazyStaticsStore, tilemeta: &TileMetaPackage, art: &TexArtEcPackage,
    atlas: &TileAtlas,
) {
    let mut keys: HashMap<u32, Option<MaterialKey>> = HashMap::new();
    let mut key_for = |id: u32| *keys.entry(id).or_insert_with(|| {
        if !art.present_slot(id).is_some_and(|slot| slot.is_land()) { return None; }
        source_key(tilemeta.item_tile(id)?, tilemeta.item_texture_refs(id))
    });
    let mut candidates = Vec::new();
    for (index, instance) in instances.iter().enumerate() {
        if instance.flat_quad < 2 { continue; }
        let Some(key) = key_for(instance.material_item_id) else { continue; };
        let (x, y) = (instance.world_x as u32, instance.world_z as u32);
        let Some(z) = map_height(atlas, x, y) else { continue; };
        if (instance.base_world_y - f32::from(z) * height_scale).abs() > 0.001 { continue; }
        candidates.push((index, (x, y, z, key)));
    }
    if candidates.is_empty() { return; }

    // A one-cell halo crosses both native-block and cached-chunk boundaries.
    // Read source statics, not just the currently visible instances, so panning
    // cannot turn an internal patch edge into an exposed border.
    let (start_x, start_y) = (chunk.gx * CHUNK_STORAGE_BLOCKS_DIM, chunk.gy * CHUNK_STORAGE_BLOCKS_DIM);
    let (end_x, end_y) = (start_x + chunk.scale * CHUNK_STORAGE_BLOCKS_DIM, start_y + chunk.scale * CHUNK_STORAGE_BLOCKS_DIM);
    let mut cells = HashSet::new();
    for by in start_y.saturating_sub(1)..=end_y {
        for bx in start_x.saturating_sub(1)..=end_x {
            let Ok(tiles) = statics.block_tiles(bx, by) else { continue; };
            for tile in tiles {
                let (x, y) = (bx * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.x_offset()), by * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.y_offset()));
                if x < (start_x * MAP_STORAGE_BLOCK_TILE_DIM).saturating_sub(1) || x > end_x * MAP_STORAGE_BLOCK_TILE_DIM
                    || y < (start_y * MAP_STORAGE_BLOCK_TILE_DIM).saturating_sub(1) || y > end_y * MAP_STORAGE_BLOCK_TILE_DIM { continue; }
                if let Some(key) = key_for(u32::from(tile.graphic)) { cells.insert((x, y, tile.z, key)); }
            }
        }
    }
    for (index, cell) in candidates {
        // Multis/raised floors must not become ground blends just because their
        // emitted sprite resembles a raw background static.
        if !cells.contains(&cell) { continue; }
        instances[index].is_wet_flags |= BLEND_ENABLED | (neighbor_mask(&cells, cell) << NEIGHBOR_SHIFT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn surface_policy_uses_source_world_color_and_excludes_authored_local_art_and_volumes() {
        let mut tile = TileMetaItemTile::zeroed();
        tile.flags = 0x400002201;
        let base = TileMetaItemTextureRef { texture_id: 2000090, flags: 2,
            texture_type: 1, stable_role: EcMaterialStableRole::Base as u8,
            texture_stretch: 8.0, uv_set: 1, ..Default::default() };
        assert_eq!(source_key(&tile, &[base]), Some((2000090, 8.0f32.to_bits())));
        for invalid in [TileMetaItemTextureRef { uv_set: 0, ..base },
            TileMetaItemTextureRef { stable_role: EcMaterialStableRole::BlendAlpha as u8, ..base }] {
            assert_eq!(source_key(&tile, &[invalid]), None);
        }
        tile.height = 1;
        assert_eq!(source_key(&tile, &[base]), None);
        tile.height = 0;
        tile.flags |= 0x400;
        assert_eq!(source_key(&tile, &[base]), None);
    }

    #[test]
    fn neighbor_coverage_crosses_chunk_edges_and_distinguishes_height_and_material() {
        let key = (2000090, 8.0f32.to_bits());
        let center = (31, 31, 0, key);
        let mut cells = HashSet::from([center, (32, 31, 0, key), (31, 32, 0, key),
            (32, 32, 5, key), (30, 31, 0, (2000010, 10.0f32.to_bits()))]);
        assert_eq!(neighbor_mask(&cells, center), (1 << 4) | (1 << 6));
        for (dx, dy) in OFFSETS {
            cells.insert(((31i32 + dx) as u32, (31i32 + dy) as u32, 0, key));
        }
        assert_eq!(neighbor_mask(&cells, center), 255);
    }

    #[test]
    fn ground_height_requires_a_resident_nonmissing_map_page() {
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u};
        use bevy::math::UVec2;
        let mut atlas = TileAtlas::new(AtlasParams::default(), 1);
        assert_eq!(map_height(&atlas, 0, 0), None);
        atlas.params.page_texels = UVec2::splat(2);
        atlas.params.world_pages_x = 2;
        atlas.params.page_to_layer[0][1] = 0;
        atlas.cpu_mirror.push(Some(vec![Rg16u::pack(3, -7, 2, false, 0); 4]));
        assert_eq!(map_height(&atlas, 2, 0), Some(-7));
        assert_eq!(map_height(&atlas, 1, 0), None);
        atlas.cpu_mirror[0].as_mut().unwrap()[0] = Rg16u::pack(3, 0, 3, false, 0);
        assert_eq!(map_height(&atlas, 2, 0), None);
        assert_eq!(map_height(&atlas, 0, 256), None);
    }

    #[test]
    fn map_dependency_changes_when_data_arrives_but_not_for_identical_uploads() {
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u};
        use bevy::math::UVec2;
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(2),
            max_layers: 1, world_pages_x: 1, ..Default::default() }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(bevy::math::IVec2::ZERO);
        let before = atlas.content_revision;
        atlas.enqueue_rg16u_block(layer, UVec2::ZERO, UVec2::ONE, &[Rg16u::pack(3, 0, 4, false, 0)]);
        assert_ne!(atlas.content_revision, before);
        let loaded = atlas.content_revision;
        atlas.enqueue_rg16u_block(layer, UVec2::ZERO, UVec2::ONE, &[Rg16u::pack(3, 0, 4, false, 0)]);
        assert_eq!(atlas.content_revision, loaded);
        atlas.clear_all_mappings();
        assert_ne!(atlas.content_revision, loaded);
        assert_eq!(map_height(&atlas, 0, 0), None);
    }
}
