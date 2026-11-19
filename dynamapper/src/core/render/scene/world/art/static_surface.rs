//! Ground-level, world-textured art keeps its own pixels, but exposes the map
//! smoothly at patch boundaries. Neighbor flags are built only on chunk rebuild.
use super::statics_collect::{SpriteInstance, StaticChunkBatchKey, TILE_FLAG_SURFACE};
use crate::core::texture_cache::{ec_item_surface::KR_ITEM_RECORD_BASE, land::coverage::EcTerrainParams};
use crate::world::statics::LazyStaticsStore;
use crate::core::{render::scene::world::land::{tile_atlas::{AtlasCells, TileAtlas,
    LAND_SOURCE_EC_ATLAS, TERRAIN_SHADER_FLAG_NEVER_BLENDS}, CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM}};
use bevy::math::UVec2;
use std::collections::{HashMap, HashSet};
use udd_assets::{tex_art_ec::TexArtEcPackage, tilemeta::{EcMaterialStableRole, TileMetaItemTextureRef, TileMetaItemTile, TileMetaPackage}};

const BLEND_ENABLED: u32 = 1 << 1;
const NEIGHBOR_SHIFT: u32 = 2;
const OFFSETS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// Tiledata Wall flag. With `TILE_FLAG_SURFACE`, `TILE_FLAG_DOOR` and `TILE_FLAG_TRANSPARENT` it marks the statics
/// that close a patch's side (`CLOSING_STATIC_FLAGS`): walls (with their posts, windows and door frames) and surfaces
/// (floors, boards, steps and tables).
const TILE_FLAG_WALL: u64 = 0x10;
/// Tiledata Door flag: the doors and gates, a third of which (the gates, the bar doors, 0x241F..) carry no Wall flag.
const TILE_FLAG_DOOR: u64 = 0x2000_0000;
/// Tiledata Transparent flag: the fixtures set at a building's edge or along a fence, which the flag lets one see
/// past: the signposts and the signs they hold, banners, sconces and torch holders, lantern posts, rails and
/// banisters. A signpost at a house corner stands on the land beside the street's cobblestones.
const TILE_FLAG_TRANSPARENT: u64 = 0x4;
/// Tiledata Impassable flag. A Rotated ground piece with it is not a patch (`source_key`): the lava rims fade no more
/// into the land beside them, which showed the dirt between the lava and the crust (fixture `lava-rim-crust`).
const TILE_FLAG_IMPASSABLE: u64 = 0x40;
/// The tiledata flags of the statics that close a patch's side (`ClosingStatics`).
const CLOSING_STATIC_FLAGS: u64 = TILE_FLAG_WALL | TILE_FLAG_SURFACE | TILE_FLAG_DOOR | TILE_FLAG_TRANSPARENT;
/// How far, in Z units, a closing static may start above a patch and still close its side: a building's floor or
/// foundation often sits a step above the street it meets.
const CLOSING_STATIC_Z_REACH: i16 = 2;
type MaterialKey = (u32, u32);
type SurfaceCell = (u32, u32, i8, MaterialKey);

/// The Z spans (bottom, top) of the walls, surfaces, doors and edge fixtures (`CLOSING_STATIC_FLAGS`) of each cell
/// that are not patches themselves. The clients draw a floor right up to the wall or floor beside it (the KR yard at
/// Minoc 2476,408), so a patch side against one is not exposed: fading it revealed a band of the map along the wall,
/// as if the floor stopped short. A door or a gate without the Wall flag, or a signpost (Minoc 2448,415), left the
/// same band beside it.
#[derive(Default)]
struct ClosingStatics(HashMap<(u32, u32), Vec<(i16, i16)>>);

impl ClosingStatics {
    fn insert(&mut self, x: u32, y: u32, z: i8, tile: &TileMetaItemTile) {
        if tile.flags & CLOSING_STATIC_FLAGS == 0 || tile.is_nodraw() { return; }
        let bottom = i16::from(z);
        self.0.entry((x, y)).or_default().push((bottom, bottom + i16::from(tile.height)));
    }

    fn closes(&self, x: u32, y: u32, z: i8) -> bool {
        let z = i16::from(z);
        self.0.get(&(x, y)).is_some_and(|spans| spans.iter()
            .any(|&(bottom, top)| bottom <= z + CLOSING_STATIC_Z_REACH && top >= z))
    }
}

fn source_key(tile: &TileMetaItemTile, refs: &[TileMetaItemTextureRef]) -> Option<MaterialKey> {
    // The camera-plane terrain-item flag is source-backed. Exclude volumes,
    // roofs, foliage, water and local/clamped art such as rugs or decals.
    // Impassable pieces are no walkable ground continuing the land either: the lava rims (0x1327..0x134D) draw their
    // whole cell as authored, crust up to the trench wall in the KR client (`TILE_FLAG_IMPASSABLE`).
    if tile.height != 0 || tile.flags & (1u64 << 34) == 0
        || tile.flags & (0x400 | 0x80 | 0x20000 | 0x10000000 | TILE_FLAG_IMPASSABLE) != 0 || tile.is_nodraw() {
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

/// The EC terrain parameters of the land the cell draws, as the land coverage takes them: `terrain` is
/// `EcTerrainBases::params`, built from the EC-layout package the land source routes each row to (Enhanced or Kingdom
/// Reborn land), indexed by the cell's payload. None for land outside the coverage: Classic land, land missing, black
/// land and a cell that never blends (`TERRAIN_SHADER_FLAG_NEVER_BLENDS`).
fn land_terrain(atlas: &TileAtlas, terrain: &[Option<EcTerrainParams>], x: u32, y: u32) -> Option<EcTerrainParams> {
    let texel = atlas.read_cell(x, y)?;
    if texel.texture_source() != LAND_SOURCE_EC_ATLAS || texel.terrain_flags() & TERRAIN_SHADER_FLAG_NEVER_BLENDS != 0 {
        return None;
    }
    *terrain.get(usize::from(texel.r))?
}

/// `hard_land(x, y)` tells whether the land of the cell (`x`, `y`) meets the patch cell's land with a hard edge.
fn neighbor_mask(cells: &HashSet<SurfaceCell>, closing: &ClosingStatics, hard_land: impl Fn(u32, u32) -> bool,
    cell: SurfaceCell) -> u32 {
    let mut mask = 0;
    for (bit, (dx, dy)) in OFFSETS.into_iter().enumerate() {
        let (Some(x), Some(y)) = (cell.0.checked_add_signed(dx), cell.1.checked_add_signed(dy)) else { continue; };
        if cells.contains(&(x, y, cell.2, cell.3)) || closing.closes(x, y, cell.2) || hard_land(x, y) {
            mask |= 1 << bit;
        }
    }
    mask
}

/// `kingdom_reborn` is the Kingdom Reborn tile data and art (an overlay over `art`) the instances with a
/// `material_item_id` from `KR_ITEM_RECORD_BASE` were drawn from; the source statics take both their Enhanced and their
/// Kingdom Reborn keys. `terrain` is the land coverage's terrain parameters (`land_terrain`), when the land draws
/// them. Returns the cells of `atlas` it read, if any, which a change of rebuilds the chunk.
///
/// A patch side is also closed where the land beside it meets the patch cell's land with a hard edge
/// (`EcTerrainParams::meets_with_hard_edge`): the wooden floor of a doorway with no door static (Minoc 2476,404), or
/// the stone floor inside a hall whose walls stand on cobblestones. Fading it showed a band of the patch cell's own land, grass under cobblestones, which the land
/// itself never shows there.
pub(super) fn apply_boundary_coverage(
    instances: &mut [SpriteInstance], chunk: StaticChunkBatchKey, height_scale: f32,
    statics: &mut LazyStaticsStore, tilemeta: &TileMetaPackage, art: &TexArtEcPackage,
    kingdom_reborn: Option<(&TileMetaPackage, &TexArtEcPackage)>,
    atlas: &TileAtlas, terrain: Option<&[Option<EcTerrainParams>]>,
) -> Option<AtlasCells> {
    let mut keys: HashMap<u32, Option<MaterialKey>> = HashMap::new();
    let mut key_for = |item: u32| *keys.entry(item).or_insert_with(|| {
        let (tilemeta, slot, id) = match kingdom_reborn {
            Some((kr_tilemeta, kr_art)) if item >= KR_ITEM_RECORD_BASE => {
                let id = item - KR_ITEM_RECORD_BASE;
                (kr_tilemeta, kr_art.overlay_slot(art, id).map(|(_, slot)| slot), id)
            }
            _ if item >= KR_ITEM_RECORD_BASE => return None,
            _ => (tilemeta, art.present_slot(item), item),
        };
        if !slot.is_some_and(|slot| slot.is_land()) { return None; }
        source_key(tilemeta.item_tile(id)?, tilemeta.item_texture_refs(id))
    });
    let mut candidates = Vec::new();
    let mut read: Option<AtlasCells> = None;
    for (index, instance) in instances.iter().enumerate() {
        if instance.flat_quad < 2 { continue; }
        let Some(key) = key_for(instance.material_item_id) else { continue; };
        let (x, y) = (instance.world_x as u32, instance.world_z as u32);
        // With the land's terrain read, the land of the cells around the patch decides its sides too.
        let cell = match terrain {
            Some(_) => AtlasCells::new(UVec2::new(x.saturating_sub(1), y.saturating_sub(1)), UVec2::new(x + 1, y + 1)),
            None => AtlasCells::new(UVec2::new(x, y), UVec2::new(x, y)),
        };
        read = Some(read.map_or(cell, |read| read.union(cell)));
        let Some(z) = map_height(atlas, x, y) else { continue; };
        if (instance.base_world_y - f32::from(z) * height_scale).abs() > 0.001 { continue; }
        candidates.push((index, (x, y, z, key)));
    }
    if candidates.is_empty() { return read; }

    // A one-cell halo crosses both native-block and cached-chunk boundaries.
    // Read source statics, not just the currently visible instances, so panning
    // cannot turn an internal patch edge into an exposed border.
    let (start_x, start_y) = (chunk.gx * CHUNK_STORAGE_BLOCKS_DIM, chunk.gy * CHUNK_STORAGE_BLOCKS_DIM);
    let (end_x, end_y) = (start_x + chunk.scale * CHUNK_STORAGE_BLOCKS_DIM, start_y + chunk.scale * CHUNK_STORAGE_BLOCKS_DIM);
    let mut cells = HashSet::new();
    let mut closing = ClosingStatics::default();
    for by in start_y.saturating_sub(1)..=end_y {
        for bx in start_x.saturating_sub(1)..=end_x {
            let Ok(tiles) = statics.block_tiles(bx, by) else { continue; };
            for tile in tiles {
                let (x, y) = (bx * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.x_offset()), by * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.y_offset()));
                if x < (start_x * MAP_STORAGE_BLOCK_TILE_DIM).saturating_sub(1) || x > end_x * MAP_STORAGE_BLOCK_TILE_DIM
                    || y < (start_y * MAP_STORAGE_BLOCK_TILE_DIM).saturating_sub(1) || y > end_y * MAP_STORAGE_BLOCK_TILE_DIM { continue; }
                let graphic = u32::from(tile.graphic);
                let kr_key = kingdom_reborn.and_then(|_| key_for(graphic + KR_ITEM_RECORD_BASE));
                if let Some(key) = kr_key { cells.insert((x, y, tile.z, key)); }
                if let Some(key) = key_for(graphic) { cells.insert((x, y, tile.z, key)); }
                else if kr_key.is_none() {
                    if let Some(meta) = tilemeta.item_tile(graphic) { closing.insert(x, y, tile.z, meta); }
                }
            }
        }
    }
    for (index, cell) in candidates {
        // Multis/raised floors must not become ground blends just because their
        // emitted sprite resembles a raw background static.
        if !cells.contains(&cell) { continue; }
        let own = terrain.and_then(|terrain| land_terrain(atlas, terrain, cell.0, cell.1).map(|own| (terrain, own)));
        let hard_land = |x, y| own.is_some_and(|(terrain, own)|
            land_terrain(atlas, terrain, x, y).is_some_and(|other| own.meets_with_hard_edge(other)));
        instances[index].is_wet_flags |= BLEND_ENABLED | (neighbor_mask(&cells, &closing, hard_land, cell) << NEIGHBOR_SHIFT);
    }
    read
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn no_hard_land(_: u32, _: u32) -> bool { false }

    #[test]
    fn floor_land_meeting_a_patch_with_a_hard_edge_closes_its_side() {
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u, LAND_SOURCE_CC_ATLAS};
        use crate::core::texture_cache::land::coverage::COVERAGE_FLAG_LINEAR_FILTERING;
        // The Minoc yard's grass (land 0x3, a soft material) under the cobblestones, and the wooden floor of the north
        // doorway and of the house east of the yard (land 0x406, material 11, coverage exponent 0).
        const GRASS: u16 = 0x3;
        const FLOOR: u16 = 0x406;
        const PAGE_CELLS: u32 = 4;
        /// coverage.rs `GPU_LAYER_CLASS_DEFAULT`.
        const DEFAULT_LAYER_CLASS: u32 = 1;
        let mut terrain = vec![None; usize::from(FLOOR) + 1];
        terrain[usize::from(GRASS)] = Some(EcTerrainParams([3, 3, 242 | (153 << 8),
            DEFAULT_LAYER_CLASS | COVERAGE_FLAG_LINEAR_FILTERING]));
        terrain[usize::from(FLOOR)] = Some(EcTerrainParams([11, 11, 0, DEFAULT_LAYER_CLASS]));
        let mut atlas = TileAtlas::new(AtlasParams::default(), 1);
        atlas.params.page_texels = UVec2::splat(PAGE_CELLS);
        atlas.params.world_pages_x = 1;
        atlas.params.page_to_layer[0][0] = 0;
        let mut texels = vec![Rg16u::pack(GRASS, 15, LAND_SOURCE_EC_ATLAS as u16, false, 0); (PAGE_CELLS * PAGE_CELLS) as usize];
        let at = |x: u32, y: u32| (y * PAGE_CELLS + x) as usize;
        // Floor east and south-east of the patch at (1,1) closes those sides.
        texels[at(2, 1)] = Rg16u::pack(FLOOR, 15, LAND_SOURCE_EC_ATLAS as u16, false, 0);
        texels[at(2, 2)] = texels[at(2, 1)];
        // Floor drawn from a Classic texmap, or a cell that never blends, is outside the coverage: its side stays open.
        texels[at(0, 1)] = Rg16u::pack(FLOOR, 15, LAND_SOURCE_CC_ATLAS as u16, false, 0);
        texels[at(1, 0)] = Rg16u::pack(FLOOR, 15, LAND_SOURCE_EC_ATLAS as u16, false, TERRAIN_SHADER_FLAG_NEVER_BLENDS);
        atlas.cpu_mirror.push(Some(texels));

        let key = (2000090, 8.0f32.to_bits());
        let center = (1, 1, 15, key);
        let cells = HashSet::from([center]);
        let own = land_terrain(&atlas, &terrain, 1, 1).unwrap();
        let hard_land = |x, y| land_terrain(&atlas, &terrain, x, y).is_some_and(|other| own.meets_with_hard_edge(other));
        // Bits: NW 0, N 1, NE 2, W 3, E 4, SW 5, S 6, SE 7.
        assert_eq!(neighbor_mask(&cells, &ClosingStatics::default(), hard_land, center), (1 << 4) | (1 << 7));
        // Grass beside grass, the patch cell's own land, has no edge.
        assert_eq!(land_terrain(&atlas, &terrain, 0, 0), Some(own));
        assert_eq!(land_terrain(&atlas, &terrain, 0, 1), None);
        assert_eq!(land_terrain(&atlas, &terrain, 1, 0), None);
    }

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
    fn impassable_lava_rims_are_not_patches() {
        // The lava rim 0x132A: Background, Impassable, Rotated, its crust Lava_Grey (2000103) repeated every 4 cells.
        let mut rim = TileMetaItemTile::zeroed();
        rim.flags = 0x400000001 | TILE_FLAG_IMPASSABLE;
        let crust = TileMetaItemTextureRef { texture_id: 2000103, flags: 2,
            texture_type: 1, stable_role: EcMaterialStableRole::Base as u8,
            texture_stretch: 4.0, uv_set: 1, ..Default::default() };
        assert_eq!(source_key(&rim, &[crust]), None);
        rim.flags &= !TILE_FLAG_IMPASSABLE;
        assert_eq!(source_key(&rim, &[crust]), Some((2000103, 4.0f32.to_bits())));
    }

    #[test]
    fn neighbor_coverage_crosses_chunk_edges_and_distinguishes_height_and_material() {
        let key = (2000090, 8.0f32.to_bits());
        let center = (31, 31, 0, key);
        let mut cells = HashSet::from([center, (32, 31, 0, key), (31, 32, 0, key),
            (32, 32, 5, key), (30, 31, 0, (2000010, 10.0f32.to_bits()))]);
        assert_eq!(neighbor_mask(&cells, &ClosingStatics::default(), no_hard_land, center), (1 << 4) | (1 << 6));
        for (dx, dy) in OFFSETS {
            cells.insert(((31i32 + dx) as u32, (31i32 + dy) as u32, 0, key));
        }
        assert_eq!(neighbor_mask(&cells, &ClosingStatics::default(), no_hard_land, center), 255);
    }

    #[test]
    fn walls_and_floors_beside_a_patch_close_its_side() {
        let key = (2000090, 8.0f32.to_bits());
        // The Minoc yard's corner cell (2473,405) at Z 15: log walls west, north-west and north-east of it.
        let center = (2473, 405, 15, key);
        let cells = HashSet::from([center]);
        let mut wall = TileMetaItemTile::zeroed();
        wall.flags = TILE_FLAG_WALL;
        wall.height = 20;
        let mut floor = TileMetaItemTile::zeroed();
        floor.flags = TILE_FLAG_SURFACE;
        let mut closing = ClosingStatics::default();
        closing.insert(2472, 405, 15, &wall);
        closing.insert(2472, 404, 15, &wall);
        closing.insert(2474, 404, 15, &wall);
        // A floor a step above the yard closes its side; one far above it, or a wall that ends below it, does not.
        closing.insert(2473, 406, (15 + CLOSING_STATIC_Z_REACH) as i8, &floor);
        closing.insert(2474, 406, 30, &floor);
        closing.insert(2474, 405, -10, &wall);
        // Bits: NW 0, N 1, NE 2, W 3, E 4, SW 5, S 6, SE 7.
        assert_eq!(neighbor_mask(&cells, &closing, no_hard_land, center), (1 << 0) | (1 << 2) | (1 << 3) | (1 << 6));
        // Statics with none of the closing flags (a plant, a barrel) leave the side exposed.
        let mut plain = ClosingStatics::default();
        plain.insert(2472, 405, 15, &TileMetaItemTile::zeroed());
        assert_eq!(neighbor_mask(&cells, &plain, no_hard_land, center), 0);
    }

    #[test]
    fn doors_gates_and_signposts_beside_a_patch_close_its_side() {
        let key = (2000090, 8.0f32.to_bits());
        // The Minoc street cell (2448,416) at Z 15, with the wooden signpost 0x0B98 (Transparent, height 0) north of it.
        let center = (2448, 416, 15, key);
        let cells = HashSet::from([center]);
        let mut signpost = TileMetaItemTile::zeroed();
        signpost.flags = TILE_FLAG_TRANSPARENT;
        // An iron gate (0x0824: Door and Impassable, no Wall flag) on both sides.
        let mut gate = TileMetaItemTile::zeroed();
        gate.flags = TILE_FLAG_DOOR | TILE_FLAG_IMPASSABLE;
        gate.height = 20;
        let mut closing = ClosingStatics::default();
        closing.insert(2448, 415, 15, &signpost);
        closing.insert(2447, 416, 15, &gate);
        closing.insert(2449, 416, 15, &gate);
        // Bits: NW 0, N 1, NE 2, W 3, E 4, SW 5, S 6, SE 7.
        assert_eq!(neighbor_mask(&cells, &closing, no_hard_land, center), (1 << 1) | (1 << 3) | (1 << 4));
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
