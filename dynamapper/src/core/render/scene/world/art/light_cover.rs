//! Whether a static light is hidden by what is drawn over it: the one rule every style applies before it draws a
//! light, to Classic and UOD-Painterly's light.mul decals and to KR-like and Enhanced's light sprites, and so to the
//! land and static local lights they feed. Each style picks its rule with its preset group's `light_cover`
//! (`LightCover`); both rules look at the diagonal neighbour in front of a light, the cell (x+1, y+1) the camera
//! looks across to reach it, and neither counts the light's own cell, so its own wall, the item it sits on and the
//! floor it stands on never cover it.
//!
//! `LightCover::ClassicUo` is ClassicUO's rule (`GameScene.AddLight`, a Classic reference): a light is not added
//! when its cover cell holds a drawn, opaque static whose z is at least `LIGHT_COVER_MIN_RISE` above the light's and
//! below the view's z limit. A roof or an upper floor over a room covers the room's lights; a light on an outer wall
//! facing the camera, an open-air light and a light on top of a tall item keep theirs. Dynamapper adds one exception:
//! a light hung under an eave stays lit (`eave_keeps_light_lit`).
//!
//! `LightCover::Clients` is the Enhanced and KR clients' rule (`UOObjectComponentLighting`, Enhanced 0x00449f60, KR
//! 0x004391d0; see `CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`): a light sprite is culled when an object of its cover cell
//! sits in a higher 20-z band above the land than the light (`client_light_band`), or when its own item is not drawn.
//! The clients draw the sprites that are left with no depth test.
//!
//! The rules take the view's z limit (`max_z`) so a z-view limit composes with them: a static the limit stops drawing
//! no longer covers, and its lights come back; a light at or above the limit is hidden with its item.
//!
//! A cell's objects are its statics and the parts the renderer draws there of the multis around it
//! (`MultiCoverParts`), in place of the multis' anchor statics, as the renderer expands them.

use crate::core::multis::{expanded_multi_part_z, multi_id_from_static_graphic, MultiDefinitionsRes, MultiPart};
use crate::core::render::scene::world::land::mesh_material::LightCover;
use crate::core::render::scene::world::land::MAP_STORAGE_BLOCK_TILE_DIM;
use crate::core::render::scene::world::z_view::MULTI_PART_MAX_REACH;
use crate::core::statics::LazyStaticsStore;
use std::collections::HashMap;
use std::ops::Range;
use udd_assets::tilemeta::TileMetaPackage;
use uocf::enhanced::tileart::TaeFlag;

/// How far above a light a static must start to cover it, in z units: ClassicUO's `obj.Z + 5`. The floor a light
/// stands on and the wall it hangs on start at or below the light, so they never cover it.
pub const LIGHT_COVER_MIN_RISE: i32 = 5;

/// The z limit while there is no z-view limit: every static is drawn.
pub const NO_Z_VIEW_LIMIT: i32 = i32::MAX;

/// How many cells past the cover cell the eave exception walks along the diagonal toward the camera, looking for the
/// open cell in front of an eave. Measured on maps 0 and 1: the eaves over the lights on outer walls end within two
/// cells (1539 of 1564 on map 1), while the cover over a room's lights mostly runs on past that.
pub const EAVE_WALK_CELLS: u32 = 2;

/// The clients' z bands: an object covers a light from the band above the light's, the bands `CLIENT_BAND_HEIGHT`
/// z high and counted from `CLIENT_BAND_BASE` below the land (Enhanced 0x00449f60, KR 0x0043c950).
pub const CLIENT_BAND_HEIGHT: i32 = 20;
/// See `CLIENT_BAND_HEIGHT`.
pub const CLIENT_BAND_BASE: i32 = 140;

/// Statics that never cover a light under ClassicUO's rule: the see-through (tiledata Transparent) and the never
/// drawn (NoDraw).
const NON_COVERING_FLAGS: u64 = TaeFlag::Transparent.bits() | TaeFlag::NoDraw.bits();

/// A static of a cell the cover rules read: its tile flags, z and tiledata height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverStatic {
    pub flags: u64,
    pub z: i8,
    pub height: i8,
}

/// The cell whose statics can cover a light on (`tile_x`, `tile_y`): the diagonal neighbour toward the camera.
pub fn light_cover_cell(tile_x: u32, tile_y: u32) -> (u32, u32) {
    (tile_x + 1, tile_y + 1)
}

/// Whether a static with tile flags `flags` at `static_z` covers a light at `light_z` under ClassicUO's rule, below
/// the z limit `max_z`.
pub fn static_covers_light(flags: u64, static_z: i8, light_z: i8, max_z: i32) -> bool {
    let static_z = i32::from(static_z);
    flags & NON_COVERING_FLAGS == 0 && static_z >= i32::from(light_z) + LIGHT_COVER_MIN_RISE && static_z < max_z
}

/// Whether ClassicUO's rule hides a light at `light_z`: it is at or above the z limit `max_z`, as its item is then
/// not drawn, or a static of its cover cell (`light_cover_cell`), given as (tile flags, z), covers it.
pub fn light_is_hidden(light_z: i8, cover_cell: impl IntoIterator<Item = (u64, i8)>, max_z: i32) -> bool {
    i32::from(light_z) >= max_z
        || cover_cell.into_iter().any(|(flags, static_z)| static_covers_light(flags, static_z, light_z, max_z))
}

/// Whether a drawn wall of `cell` stands at the height of a light at `light_z`: its span, z up to z + height (at
/// least one), holds the light's z.
fn wall_at_light_height(cell: &[CoverStatic], light_z: i8, max_z: i32) -> bool {
    let light_z = i32::from(light_z);
    cell.iter().any(|item| {
        let (z, height) = (i32::from(item.z), i32::from(item.height).max(1));
        item.flags & TaeFlag::Wall.bits() != 0
            && item.flags & NON_COVERING_FLAGS == 0
            && z < max_z
            && z <= light_z
            && light_z < z + height
    })
}

/// Whether a light ClassicUO's rule hides hangs under an eave, a porch or an overhang, outside the building, and so
/// stays lit. `diagonal` holds the cells in front of the light toward the camera, the cover cell first and then the
/// next `EAVE_WALK_CELLS` cells along the diagonal (x+1+k, y+1+k).
///
/// It does when no wall stands at the light's height in the cover cell, and an uncovered cell comes, walking on
/// along the diagonal, before such a wall and within the walk: the cover in front of the light ends in the open. A
/// wall at the light's height encloses it in a room, and so does a cover that runs on past the walk.
pub fn eave_keeps_light_lit(light_z: i8, diagonal: &[&[CoverStatic]], max_z: i32) -> bool {
    let Some((cover_cell, beyond)) = diagonal.split_first() else {
        return false;
    };
    if wall_at_light_height(cover_cell, light_z, max_z) {
        return false;
    }
    for cell in beyond.iter().take(EAVE_WALK_CELLS as usize) {
        if wall_at_light_height(cell, light_z, max_z) {
            return false;
        }
        if !cell.iter().any(|item| static_covers_light(item.flags, item.z, light_z, max_z)) {
            return true;
        }
    }
    false
}

/// Whether `LightCover::ClassicUo` hides a light at `light_z`: ClassicUO's rule (`light_is_hidden`) on the first cell
/// of `diagonal`, unless the light hangs under an eave (`eave_keeps_light_lit`).
pub fn classicuo_light_is_hidden(light_z: i8, diagonal: &[&[CoverStatic]], max_z: i32) -> bool {
    let cover_cell = diagonal.first().copied().unwrap_or(&[]);
    light_is_hidden(light_z, cover_cell.iter().map(|item| (item.flags, item.z)), max_z)
        && !eave_keeps_light_lit(light_z, diagonal, max_z)
}

/// The clients' z band of `z` above the land z `land_z`: `(z - land_z + CLIENT_BAND_BASE) / CLIENT_BAND_HEIGHT`,
/// truncated toward zero as the clients' integer division.
pub fn client_light_band(z: i8, land_z: i8) -> i32 {
    (i32::from(z) - i32::from(land_z) + CLIENT_BAND_BASE) / CLIENT_BAND_HEIGHT
}

/// Whether `LightCover::Clients` hides a light at `light_z` over the land z `land_z` of its own cell: it is at or
/// above the z limit `max_z`, as its item is then not drawn, or a static of its cover cell, given by z, drawn below
/// the limit, sits in a higher band (`client_light_band`, both from `land_z`). The clients count every object there,
/// whatever its flags.
pub fn client_light_is_hidden(light_z: i8, land_z: i8, cover_cell: impl IntoIterator<Item = i8>, max_z: i32) -> bool {
    let light_band = client_light_band(light_z, land_z);
    i32::from(light_z) >= max_z
        || cover_cell
            .into_iter()
            .any(|static_z| i32::from(static_z) < max_z && client_light_band(static_z, land_z) > light_band)
}

/// The parts of the multis the renderer draws that land on the cover cells of a scan's lights, by cell, and which
/// anchor statics the renderer replaces with their parts. Empty while no multi definitions are loaded, when the
/// renderer draws the anchors as statics.
#[derive(Default)]
pub struct MultiCoverParts<'a> {
    multis: Option<&'a MultiDefinitionsRes>,
    show_hidden_parts: bool,
    cells: HashMap<(u32, u32), Vec<CoverStatic>>,
}

impl<'a> MultiCoverParts<'a> {
    /// Reads the multi parts that land on the cover cells, and the cells past them the eave exception walks, of the
    /// lights in the blocks `blocks_x` × `blocks_y`: those of every multi anchored within `MULTI_PART_MAX_REACH` of
    /// them, the drawn parts or, with `show_hidden_parts`, all, as the renderer expands them.
    pub fn read(
        statics_store: &mut LazyStaticsStore,
        tilemeta: &TileMetaPackage,
        multis: Option<&'a MultiDefinitionsRes>,
        show_hidden_parts: bool,
        blocks_x: Range<u32>,
        blocks_y: Range<u32>,
    ) -> Self {
        let mut cover = Self { multis, show_hidden_parts, cells: HashMap::new() };
        let Some(multis) = multis else {
            return cover;
        };
        if blocks_x.is_empty() || blocks_y.is_empty() {
            return cover;
        }
        let cover_tiles = |blocks: &Range<u32>| {
            let (first, _) = light_cover_cell(blocks.start * MAP_STORAGE_BLOCK_TILE_DIM, 0);
            first..blocks.end * MAP_STORAGE_BLOCK_TILE_DIM + 1 + EAVE_WALK_CELLS
        };
        let (tiles_x, tiles_y) = (cover_tiles(&blocks_x), cover_tiles(&blocks_y));
        let reach = MULTI_PART_MAX_REACH as u32;
        let anchor_blocks = |tiles: &Range<u32>| {
            tiles.start.saturating_sub(reach) / MAP_STORAGE_BLOCK_TILE_DIM..(tiles.end + reach) / MAP_STORAGE_BLOCK_TILE_DIM + 1
        };
        let meta = |graphic: u16| {
            tilemeta.item_tile(u32::from(graphic)).map_or((0, 0), |meta| (meta.flags, meta.height))
        };
        for block_y in anchor_blocks(&tiles_y) {
            for block_x in anchor_blocks(&tiles_x) {
                let Ok(tiles) = statics_store.block_tiles(block_x, block_y) else {
                    continue;
                };
                for tile in tiles {
                    let Some(parts) = multi_id_from_static_graphic(tile.graphic)
                        .and_then(|multi_id| multis.parts(multi_id, show_hidden_parts))
                    else {
                        continue;
                    };
                    let anchor = (
                        block_x * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.x_offset()),
                        block_y * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.y_offset()),
                    );
                    for (cell, part) in multi_parts_on_cells(anchor, tile.z, parts, &tiles_x, &tiles_y, meta) {
                        cover.cells.entry(cell).or_default().push(part);
                    }
                }
            }
        }
        cover
    }

    /// Whether the renderer draws the static `graphic` as the parts of a multi rather than as itself.
    fn expands(&self, graphic: u16) -> bool {
        self.multis.is_some_and(|multis| {
            multi_id_from_static_graphic(graphic).is_some_and(|multi_id| multis.parts(multi_id, self.show_hidden_parts).is_some())
        })
    }

    fn cell(&self, x: u32, y: u32) -> &[CoverStatic] {
        self.cells.get(&(x, y)).map_or(&[], Vec::as_slice)
    }
}

/// The parts of a multi anchored at `anchor`, at `anchor_z`, that land on the cells `tiles_x` × `tiles_y`, with their
/// tile flags and heights from `meta` and their z as the renderer expands it.
fn multi_parts_on_cells(
    anchor: (u32, u32),
    anchor_z: i8,
    parts: &[MultiPart],
    tiles_x: &Range<u32>,
    tiles_y: &Range<u32>,
    meta: impl Fn(u16) -> (u64, i8),
) -> Vec<((u32, u32), CoverStatic)> {
    parts
        .iter()
        .filter_map(|part| {
            let x = u32::try_from(i64::from(anchor.0) + i64::from(part.x)).ok()?;
            let y = u32::try_from(i64::from(anchor.1) + i64::from(part.y)).ok()?;
            if !tiles_x.contains(&x) || !tiles_y.contains(&y) {
                return None;
            }
            let (flags, height) = meta(part.item_id);
            Some(((x, y), CoverStatic { flags, z: expanded_multi_part_z(anchor_z, part.z), height }))
        })
        .collect()
}

/// Whether `rule` hides a light on (`tile_x`, `tile_y`) at `light_z`, reading its cover cell, and for the eave
/// exception the cells past it, from the statics store and the multi parts `multi_parts` holds. `land_z` is the land z
/// of the light's cell, which the clients' bands start from; while it is not known (its map block is not loaded) the
/// light's own z stands in for it. A cell the store cannot read covers nothing.
pub fn static_light_is_hidden(
    statics_store: &mut LazyStaticsStore,
    tilemeta: &TileMetaPackage,
    multi_parts: &MultiCoverParts,
    rule: LightCover,
    land_z: Option<i8>,
    tile_x: u32,
    tile_y: u32,
    light_z: i8,
    max_z: i32,
) -> bool {
    let (cover_x, cover_y) = light_cover_cell(tile_x, tile_y);
    match rule {
        LightCover::ClassicUo => {
            let diagonal = (0..=EAVE_WALK_CELLS)
                .map(|step| cell_statics(statics_store, tilemeta, multi_parts, cover_x + step, cover_y + step))
                .collect::<Vec<_>>();
            let diagonal = diagonal.iter().map(Vec::as_slice).collect::<Vec<_>>();
            classicuo_light_is_hidden(light_z, &diagonal, max_z)
        }
        LightCover::Clients => {
            let cover_cell = cell_statics(statics_store, tilemeta, multi_parts, cover_x, cover_y);
            client_light_is_hidden(light_z, land_z.unwrap_or(light_z), cover_cell.iter().map(|item| item.z), max_z)
        }
    }
}

/// The objects the renderer draws on the cell (`x`, `y`), with their tile flags and heights: its statics, but for the
/// multi anchors it expands, and the multi parts landing there. No statics while the store cannot read its block.
fn cell_statics(
    statics_store: &mut LazyStaticsStore,
    tilemeta: &TileMetaPackage,
    multi_parts: &MultiCoverParts,
    x: u32,
    y: u32,
) -> Vec<CoverStatic> {
    let (block_x, block_y) = (x / MAP_STORAGE_BLOCK_TILE_DIM, y / MAP_STORAGE_BLOCK_TILE_DIM);
    let (offset_x, offset_y) = ((x % MAP_STORAGE_BLOCK_TILE_DIM) as u8, (y % MAP_STORAGE_BLOCK_TILE_DIM) as u8);
    let tiles = statics_store.block_tiles(block_x, block_y).unwrap_or(&[]);
    tiles
        .iter()
        .filter(|tile| tile.x_offset() == offset_x && tile.y_offset() == offset_y && !multi_parts.expands(tile.graphic))
        .map(|tile| {
            let meta = tilemeta.item_tile(u32::from(tile.graphic));
            CoverStatic { flags: meta.map_or(0, |meta| meta.flags), z: tile.z, height: meta.map_or(0, |meta| meta.height) }
        })
        .chain(multi_parts.cell(x, y).iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE: u64 = TaeFlag::Surface.bits();
    const ROOF: u64 = TaeFlag::Roof.bits();
    const WALL: u64 = TaeFlag::Wall.bits();
    /// A wall's tiledata height, as the stone walls of the real cases.
    const WALL_HEIGHT: i8 = 20;

    fn item(flags: u64, z: i8) -> CoverStatic {
        CoverStatic { flags, z, height: if flags & WALL != 0 { WALL_HEIGHT } else { 0 } }
    }

    fn classicuo(light_z: i8, diagonal: &[&[CoverStatic]]) -> bool {
        classicuo_light_is_hidden(light_z, diagonal, NO_Z_VIEW_LIMIT)
    }

    #[test]
    fn a_roof_or_upper_floor_in_front_covers_a_room_light() {
        // Britain 1407,1595: a window light at z 30 under wooden boards at 50 and pavers at 70 in its cover cell.
        assert!(light_is_hidden(30, [(OPAQUE, 50), (OPAQUE, 70)], NO_Z_VIEW_LIMIT));
        // The castle keep's wall sconces at z 63 under the cobblestone floor at 72.
        assert!(light_is_hidden(63, [(OPAQUE, 72)], NO_Z_VIEW_LIMIT));
        assert!(light_is_hidden(0, [(ROOF, 20)], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn open_air_and_outer_wall_lights_stay_lit() {
        // The lava pool and a window on a wall facing the camera: nothing over the cover cell.
        assert!(!light_is_hidden(30, [], NO_Z_VIEW_LIMIT));
        // The ground and the floor the light stands on start at or below it.
        assert!(!light_is_hidden(30, [(OPAQUE, 30), (OPAQUE, 0)], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn a_static_just_above_the_light_does_not_cover_it() {
        // A wall sconce's own wall, or the top of a tall item it sits on, starts below the rise.
        assert!(!light_is_hidden(20, [(WALL, 20 + LIGHT_COVER_MIN_RISE as i8 - 1)], NO_Z_VIEW_LIMIT));
        assert!(light_is_hidden(20, [(WALL, 20 + LIGHT_COVER_MIN_RISE as i8)], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn see_through_and_never_drawn_statics_do_not_cover() {
        assert!(!light_is_hidden(0, [(TaeFlag::Transparent.bits(), 20)], NO_Z_VIEW_LIMIT));
        assert!(!light_is_hidden(0, [(TaeFlag::NoDraw.bits() | ROOF, 20)], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn the_z_limit_uncovers_lights_and_hides_its_own_cut() {
        // A z limit at the roof stops drawing it: the room's light comes back.
        assert!(!light_is_hidden(0, [(ROOF, 20)], 20));
        assert!(light_is_hidden(0, [(ROOF, 20)], 21));
        // A light at or above the limit goes with its item.
        assert!(light_is_hidden(20, [], 20));
        assert!(!light_is_hidden(19, [], 20));
    }

    #[test]
    fn the_cover_cell_is_the_diagonal_neighbour_toward_the_camera() {
        assert_eq!(light_cover_cell(1407, 1595), (1408, 1596));
    }

    #[test]
    fn a_light_under_an_eave_stays_lit() {
        // A window light at z 0 on a south wall, an eave at 20 over the cell in front of it, open ground beyond
        // (map 1, 285,991: the most common case, windows under the eaves of Yew's houses).
        let eave: &[CoverStatic] = &[item(ROOF, 20)];
        assert!(!classicuo(0, &[eave, &[], &[]]));
        // A deeper overhang, two cells of cover, still ends in the open within the walk.
        assert!(!classicuo(0, &[eave, eave, &[]]));
        // ClassicUO itself hides it: the exception is Dynamapper's.
        assert!(light_is_hidden(0, [(ROOF, 20)], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn a_light_in_a_covered_room_stays_hidden() {
        let floor: &[CoverStatic] = &[item(OPAQUE, 20)];
        // The roof runs on past the walk: the light is inside.
        assert!(classicuo(0, &[floor, floor, floor]));
        // A wall at the light's height ahead, the room's far wall, encloses it even when the cover ends after it.
        let far_wall: &[CoverStatic] = &[item(WALL, 0), item(OPAQUE, 20)];
        assert!(classicuo(0, &[floor, far_wall, &[]]));
        // A wall at the light's height in the cover cell itself: a room one cell deep.
        let wall_in_front: &[CoverStatic] = &[item(WALL, 0), item(OPAQUE, 20)];
        assert!(classicuo(0, &[wall_in_front, &[], &[]]));
        // A wall below or above the light's height does not enclose it.
        let low_wall: &[CoverStatic] = &[item(WALL, -WALL_HEIGHT), item(OPAQUE, 20)];
        assert!(!classicuo(0, &[low_wall, &[], &[]]));
    }

    #[test]
    fn the_eave_walk_needs_the_cells_beyond() {
        // Without the cells past the cover cell (a map edge) the rule is ClassicUO's.
        assert!(classicuo(0, &[&[item(ROOF, 20)]]));
        assert!(!classicuo(0, &[&[]]));
        assert!(!classicuo(0, &[]));
    }

    #[test]
    fn the_client_bands_count_twenty_z_above_the_land() {
        assert_eq!(client_light_band(0, 0), 7);
        assert_eq!(client_light_band(19, 0), 7);
        assert_eq!(client_light_band(20, 0), 8);
        assert_eq!(client_light_band(30, -15), 9);
        // Truncated toward zero, as the clients' integer division, far below the land.
        assert_eq!(client_light_band(-128, 40), -1);
    }

    #[test]
    fn the_clients_hide_a_light_under_a_higher_band() {
        // Britain 1407,1595 over land -15: the light at 30 in band 9, the boards at 50 in band 10.
        assert!(client_light_is_hidden(30, -15, [50, 70], NO_Z_VIEW_LIMIT));
        // The keep's sconces at 63 over land 50: band 7, the floor at 72 in band 8.
        assert!(client_light_is_hidden(63, 50, [72], NO_Z_VIEW_LIMIT));
        // An eave hides as any roof: the clients have no eave exception.
        assert!(client_light_is_hidden(0, 0, [20], NO_Z_VIEW_LIMIT));
        // Nothing in front, or only the light's own band: lit.
        assert!(!client_light_is_hidden(30, 0, [], NO_Z_VIEW_LIMIT));
        assert!(!client_light_is_hidden(0, 0, [19], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn the_client_bands_differ_from_classicuos_rise() {
        // 6 z above, in the same band: ClassicUO hides, the clients do not.
        assert!(light_is_hidden(0, [(OPAQUE, 6)], NO_Z_VIEW_LIMIT));
        assert!(!client_light_is_hidden(0, 0, [6], NO_Z_VIEW_LIMIT));
        // 2 z above, across a band boundary: the clients hide, ClassicUO does not.
        assert!(!light_is_hidden(18, [(OPAQUE, 20)], NO_Z_VIEW_LIMIT));
        assert!(client_light_is_hidden(18, 0, [20], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn multi_parts_land_on_their_cells_at_their_expanded_z() {
        let floor = MultiPart { item_id: 1, x: 1, y: 1, z: 20 };
        let far = MultiPart { item_id: 1, x: 40, y: 0, z: 20 };
        let behind = MultiPart { item_id: 1, x: -11, y: 0, z: 0 };
        let meta = |_| (OPAQUE, 0);
        let parts = multi_parts_on_cells((10, 10), 5, &[floor, far, behind], &(0..20), &(0..20), meta);
        assert_eq!(parts, [((11, 11), item(OPAQUE, 25))]);
        // A part over a light's cover cell covers it as a static does.
        let cover = parts.iter().map(|(_, part)| *part).collect::<Vec<_>>();
        assert!(classicuo(5, &[&cover]));
        assert!(client_light_is_hidden(5, 5, cover.iter().map(|part| part.z), NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn without_multi_definitions_no_static_is_expanded() {
        let none = MultiCoverParts::default();
        assert!(!none.expands(0x4001));
        assert!(none.cell(11, 11).is_empty());
    }

    #[test]
    fn the_client_rule_follows_the_z_limit() {
        // A cover the limit stops drawing no longer covers; a light at or above the limit goes with its item.
        assert!(!client_light_is_hidden(0, 0, [20], 20));
        assert!(client_light_is_hidden(0, 0, [20], 21));
        assert!(client_light_is_hidden(20, 0, [], 20));
    }
}
