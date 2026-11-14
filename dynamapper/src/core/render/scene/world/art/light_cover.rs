//! Whether a static light is hidden by what is drawn over it: the one rule every style applies before it draws a
//! light, to Classic's light.mul decals and to KR-like, Enhanced and UOD-Painterly's light sprites, and so to the
//! land and static local lights they feed. Each style picks its rule with its preset group's `light_cover`
//! (`LightCover`); the clients' and ClassicUO's rules look at the diagonal neighbour in front of a light, the cell
//! (x+1, y+1) the camera looks across to reach it, and none counts the light's own cell, so its own wall, the item it
//! sits on and the floor it stands on never cover it.
//!
//! `LightCover::Uod` is Dynamapper's: it hides no light for being covered, only the lights whose item is not drawn
//! (at or above the view's z limit). The glow is clipped pixel by pixel instead (`light_scene`), and what it takes
//! from here is the light's ceiling (`static_light_ceiling_z`): the lowest floor or roof over it, from which up the
//! walls and solid statics past the light are not lit.
//!
//! `LightCover::ClassicUo` is ClassicUO's rule (`GameScene.AddLight`, a Classic reference): a light is not added
//! when its cover cell holds a drawn, opaque static whose z is at least `LIGHT_COVER_MIN_RISE` above the light's and
//! below the view's z limit. A roof or an upper floor over a room covers the room's lights; a light on an outer wall
//! facing the camera, an open-air light and a light on top of a tall item keep theirs.
//!
//! `LightCover::Clients` is the Enhanced and KR clients' rule (`UOObjectComponentLighting`, Enhanced 0x00449f60, KR
//! 0x004391d0; see `CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`): a light sprite is culled when an object of its cover cell
//! sits in a higher 20-z band above the land than the light (`client_light_band`), or when its own item is not drawn.
//! The clients draw the sprites that are left with no depth test. The two clients count the bands differently, and
//! each style follows one of them (`LightCoverRule`): KR-like and UOD-Painterly the KR client's 16 clamped layers,
//! each object's from the average land z of its own cell (`kr_light_is_hidden`), the other styles the Enhanced
//! client's unclamped bands, all from the raw land z of the light's cell (`client_light_is_hidden`). Both clients'
//! bands let a light show through a floor or roof just above it in its own band, and KR's in a lower layer too when
//! the land under the two cells differs (the clients draw such glows over the floor); no style follows that: a floor
//! or roof of the cover cell at least `LIGHT_COVER_MIN_RISE` above the light also hides it there
//! (`floor_covers_light`).
//!
//! The rules take the view's z limit (`max_z`) so a z-view limit composes with them: a static the limit stops drawing
//! no longer covers, and its lights come back; a light at or above the limit is hidden with its item. Under KR's own
//! cut (KR-like's and UOD-Painterly's `player_floor_rule = "kr"`: the layers above the player's, 0x0043c9f0) the limit is the bottom of the
//! layer above the player's, every layer counted from the player's cell (`z_view::kr_layer_cut_z`), so a light in a
//! hidden layer goes and an object counts only in the layers drawn: the light cover follows what is drawn, one limit
//! for the light and its cover cell wherever the land under them differs.
//!
//! A cell's objects are its statics, each as itself whatever its graphic (map statics are never multis:
//! `core::multis`), and the parts the renderer draws there of the multis placed around it (`MultiCoverParts`).

use crate::core::multis::{expanded_multi_part_z, MultiDefinitionsRes, MultiPart, PlacedMultisRes};
use crate::core::render::scene::world::land::mesh_material::LightCover;
use crate::core::render::scene::world::land::MAP_STORAGE_BLOCK_TILE_DIM;
use crate::core::render::scene::world::z_view::{average_land_z, MULTI_PART_MAX_REACH};
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

/// The clients' z bands: an object covers a light from the band above the light's, the bands `CLIENT_BAND_HEIGHT`
/// z high and counted from `CLIENT_BAND_BASE` below the land (Enhanced 0x00449f60, KR 0x0043c950).
pub const CLIENT_BAND_HEIGHT: i32 = 20;
/// See `CLIENT_BAND_HEIGHT`.
pub const CLIENT_BAND_BASE: i32 = 140;
/// The KR client's top height layer: it clamps the bands to its 16 layers, 0 to 15 (0x0043c5b0).
pub const KR_LIGHT_TOP_LAYER: i32 = 15;

/// Statics that never cover a light under ClassicUO's rule: the see-through (tiledata Transparent) and the never
/// drawn (NoDraw).
const NON_COVERING_FLAGS: u64 = TaeFlag::Transparent.bits() | TaeFlag::NoDraw.bits();

/// The statics that are floors or roofs, which always cover a light under them with the clients' rules
/// (`floor_covers_light`) and make the ceiling of `LightCover::Uod` (`light_ceiling_z`).
const FLOOR_FLAGS: u64 = TaeFlag::Surface.bits() | TaeFlag::Roof.bits();

/// The rule a style's `LightCover` gives: `LightCover::Clients` is the KR client's rule in KR-like and UOD-Painterly
/// and the Enhanced client's in the other styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightCoverRule {
    /// `LightCover::Uod`: only a light whose item is not drawn is hidden; its glow is clipped (`light_scene`).
    Uod,
    /// `LightCover::ClassicUo`: `light_is_hidden` on the cover cell.
    ClassicUo,
    /// `LightCover::Clients` in Enhanced and Classic: `client_light_is_hidden`.
    EnhancedClient,
    /// `LightCover::Clients` in KR-like and UOD-Painterly: `kr_light_is_hidden`.
    KrClient,
}

impl LightCoverRule {
    /// The rule `light_cover` gives in a style, `kr_rules` when the style follows the KR client's light rules
    /// (KR-like, and UOD-Painterly, which draws KR-like's light sprites).
    pub fn of_style(light_cover: LightCover, kr_rules: bool) -> Self {
        match (light_cover, kr_rules) {
            (LightCover::Uod, _) => Self::Uod,
            (LightCover::ClassicUo, _) => Self::ClassicUo,
            (LightCover::Clients, false) => Self::EnhancedClient,
            (LightCover::Clients, true) => Self::KrClient,
        }
    }
}

/// A static of a cell the cover rules read: its tile flags and z.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverStatic {
    pub flags: u64,
    pub z: i8,
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

/// The KR client's height layer of `z` over the land z `land_z` of its own cell: the clients' band
/// (`client_light_band`) clamped to 0 to `KR_LIGHT_TOP_LAYER` (0x0043c950 → 0x0043c5b0).
pub fn kr_light_layer(z: i8, land_z: i8) -> i32 {
    client_light_band(z, land_z).clamp(0, KR_LIGHT_TOP_LAYER)
}

/// Whether a floor or roof of the cover cell `cover_cell` covers a light at `light_z` under ClassicUO's rule
/// (`static_covers_light`): a Surface or Roof static, neither Transparent nor NoDraw, at least `LIGHT_COVER_MIN_RISE`
/// above the light and below the z limit `max_z`. Both clients' rules take it on top of their bands, which let such a
/// floor through when it sits in the light's band or below (Britain 1407,1594 on map 1: windows at 50 under the
/// pavers at 70), which the clients then draw as a glow over the floor.
pub fn floor_covers_light(light_z: i8, cover_cell: &[CoverStatic], max_z: i32) -> bool {
    cover_cell.iter().any(|item| covers_as_floor(item, light_z, max_z))
}

/// Whether `item` is a floor or roof over a light at `light_z`, below the z limit `max_z` (`floor_covers_light`).
fn covers_as_floor(item: &CoverStatic, light_z: i8, max_z: i32) -> bool {
    item.flags & FLOOR_FLAGS != 0 && static_covers_light(item.flags, item.z, light_z, max_z)
}

/// The ceiling of a light at `light_z` under `LightCover::Uod`: the z of the lowest floor or roof over it
/// (`floor_covers_light`) in `cells`, its own cell and its cover cell, below the z limit `max_z`. The light does not
/// light the walls and solid statics past it from there up (`light_scene`); `None` when nothing covers it.
pub fn light_ceiling_z(light_z: i8, cells: &[&[CoverStatic]], max_z: i32) -> Option<i8> {
    cells
        .iter()
        .flat_map(|cell| cell.iter())
        .filter(|item| covers_as_floor(item, light_z, max_z))
        .map(|item| item.z)
        .min()
}

/// Whether `LightCover::Clients` hides a light at `light_z` in KR-like and UOD-Painterly, the KR client's rule
/// (0x004391d0): it is at or above the z limit `max_z`, as its item is then not drawn, or a static of its cover cell,
/// given by z, drawn below the limit, sits in a higher layer (`kr_light_layer`), the light's from `light_land_z`, the
/// land z of its own cell, and the statics' from `cover_land_z`, the land z of the cover cell. Like the Enhanced client it counts every object
/// there, whatever its flags.
pub fn kr_light_is_hidden(
    light_z: i8,
    light_land_z: i8,
    cover_land_z: i8,
    cover_cell: impl IntoIterator<Item = i8>,
    max_z: i32,
) -> bool {
    let light_layer = kr_light_layer(light_z, light_land_z);
    i32::from(light_z) >= max_z
        || cover_cell
            .into_iter()
            .any(|static_z| i32::from(static_z) < max_z && kr_light_layer(static_z, cover_land_z) > light_layer)
}

/// The land z the KR client keeps for the cell (`x`, `y`), `land_z` giving each cell's raw land z: ClassicUO's
/// average of its corners (`average_land_z`), stored at +0x10 of its cell record (`BuildChunkFromSector` 0x0058d890).
/// `None` while a corner is not known.
fn kr_cell_land_z(land_z: &impl Fn(u32, u32) -> Option<i8>, x: u32, y: u32) -> Option<i8> {
    let corner = |x, y| land_z(x, y).map(i32::from);
    let average = average_land_z(corner(x, y)?, corner(x + 1, y)?, corner(x + 1, y + 1)?, corner(x, y + 1)?);
    i8::try_from(average).ok()
}

/// The parts of the multis the renderer draws that land on the cover cells of a scan's lights, by cell. Empty while
/// no multi is placed or no multi definitions are loaded.
#[derive(Default)]
pub struct MultiCoverParts {
    cells: HashMap<(u32, u32), Vec<CoverStatic>>,
}

impl MultiCoverParts {
    /// Reads the multi parts that land on the cells, and the cover cells, of the lights in the blocks
    /// `blocks_x` × `blocks_y` of the map `map_id`: those of every multi placed within `MULTI_PART_MAX_REACH` of them,
    /// the drawn parts or, with `show_hidden_parts`, all, as the renderer expands them.
    pub fn read(
        tilemeta: &TileMetaPackage,
        multis: Option<&MultiDefinitionsRes>,
        placed_multis: &PlacedMultisRes,
        show_hidden_parts: bool,
        map_id: u32,
        blocks_x: Range<u32>,
        blocks_y: Range<u32>,
    ) -> Self {
        let mut cover = Self::default();
        let Some(multis) = multis else {
            return cover;
        };
        if blocks_x.is_empty() || blocks_y.is_empty() {
            return cover;
        }
        let cover_tiles = |blocks: &Range<u32>| {
            let (last_cover, _) = light_cover_cell(blocks.end * MAP_STORAGE_BLOCK_TILE_DIM - 1, 0);
            blocks.start * MAP_STORAGE_BLOCK_TILE_DIM..last_cover + 1
        };
        let (tiles_x, tiles_y) = (cover_tiles(&blocks_x), cover_tiles(&blocks_y));
        let reach = MULTI_PART_MAX_REACH as u32;
        let anchor_cells = |tiles: &Range<u32>| tiles.start.saturating_sub(reach)..tiles.end.saturating_add(reach);
        let flags = |graphic: u16| tilemeta.item_tile(u32::from(graphic)).map_or(0, |meta| meta.flags);
        for multi in placed_multis.anchored_in(map_id, anchor_cells(&tiles_x), anchor_cells(&tiles_y)) {
            let Some(parts) = multis.parts(multi.multi_id, show_hidden_parts) else {
                continue;
            };
            for (cell, part) in multi_parts_on_cells((multi.x, multi.y), multi.z, parts, &tiles_x, &tiles_y, flags) {
                cover.cells.entry(cell).or_default().push(part);
            }
        }
        cover
    }

    fn cell(&self, x: u32, y: u32) -> &[CoverStatic] {
        self.cells.get(&(x, y)).map_or(&[], Vec::as_slice)
    }
}

/// The parts of a multi anchored at `anchor`, at `anchor_z`, that land on the cells `tiles_x` × `tiles_y`, with their
/// tile flags from `flags` and their z as the renderer expands it.
fn multi_parts_on_cells(
    anchor: (u32, u32),
    anchor_z: i8,
    parts: &[MultiPart],
    tiles_x: &Range<u32>,
    tiles_y: &Range<u32>,
    flags: impl Fn(u16) -> u64,
) -> Vec<((u32, u32), CoverStatic)> {
    parts
        .iter()
        .filter_map(|part| {
            let x = u32::try_from(i64::from(anchor.0) + i64::from(part.x)).ok()?;
            let y = u32::try_from(i64::from(anchor.1) + i64::from(part.y)).ok()?;
            if !tiles_x.contains(&x) || !tiles_y.contains(&y) {
                return None;
            }
            Some(((x, y), CoverStatic { flags: flags(part.item_id), z: expanded_multi_part_z(anchor_z, part.z) }))
        })
        .collect()
}

/// Whether `rule` hides a light on (`tile_x`, `tile_y`) at `light_z`, reading its cover cell from the statics store
/// and the multi parts `multi_parts` holds; `LightCoverRule::Uod` reads nothing and hides only a light at or above the
/// z limit `max_z`, whose item is not drawn. `land_z` gives the raw
/// land z of a cell, which the clients' bands start from: the Enhanced client's the light cell's own, the KR client's
/// each cell's average (`kr_cell_land_z`). While the light cell's is not known (its map block is not loaded) the
/// light's own z stands in for it, and while the cover cell's is not known, the light cell's. A cell the store cannot
/// read covers nothing.
pub fn static_light_is_hidden(
    statics_store: &mut LazyStaticsStore,
    tilemeta: &TileMetaPackage,
    multi_parts: &MultiCoverParts,
    rule: LightCoverRule,
    land_z: impl Fn(u32, u32) -> Option<i8>,
    tile_x: u32,
    tile_y: u32,
    light_z: i8,
    max_z: i32,
) -> bool {
    let (cover_x, cover_y) = light_cover_cell(tile_x, tile_y);
    match rule {
        LightCoverRule::Uod => i32::from(light_z) >= max_z,
        LightCoverRule::ClassicUo => {
            let cover_cell = cell_statics(statics_store, tilemeta, multi_parts, cover_x, cover_y);
            light_is_hidden(light_z, cover_cell.iter().map(|item| (item.flags, item.z)), max_z)
        }
        LightCoverRule::EnhancedClient => {
            let cover_cell = cell_statics(statics_store, tilemeta, multi_parts, cover_x, cover_y);
            let light_land_z = land_z(tile_x, tile_y).unwrap_or(light_z);
            client_light_is_hidden(light_z, light_land_z, cover_cell.iter().map(|item| item.z), max_z)
                || floor_covers_light(light_z, &cover_cell, max_z)
        }
        LightCoverRule::KrClient => {
            let cover_cell = cell_statics(statics_store, tilemeta, multi_parts, cover_x, cover_y);
            let light_land_z = kr_cell_land_z(&land_z, tile_x, tile_y).unwrap_or(light_z);
            let cover_land_z = kr_cell_land_z(&land_z, cover_x, cover_y).unwrap_or(light_land_z);
            kr_light_is_hidden(light_z, light_land_z, cover_land_z, cover_cell.iter().map(|item| item.z), max_z)
                || floor_covers_light(light_z, &cover_cell, max_z)
        }
    }
}

/// The ceiling of a light on (`tile_x`, `tile_y`) at `light_z` under `LightCover::Uod` (`light_ceiling_z`), reading
/// its own cell and its cover cell from the statics store and the multi parts `multi_parts` holds, below the z limit
/// `max_z`.
pub fn static_light_ceiling_z(
    statics_store: &mut LazyStaticsStore,
    tilemeta: &TileMetaPackage,
    multi_parts: &MultiCoverParts,
    tile_x: u32,
    tile_y: u32,
    light_z: i8,
    max_z: i32,
) -> Option<i8> {
    let (cover_x, cover_y) = light_cover_cell(tile_x, tile_y);
    let own_cell = cell_statics(statics_store, tilemeta, multi_parts, tile_x, tile_y);
    let cover_cell = cell_statics(statics_store, tilemeta, multi_parts, cover_x, cover_y);
    light_ceiling_z(light_z, &[&own_cell, &cover_cell], max_z)
}

/// The objects the renderer draws on the cell (`x`, `y`), with their tile flags: its statics and the multi parts
/// landing there. No statics while the store cannot read its block.
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
        .filter(|tile| tile.x_offset() == offset_x && tile.y_offset() == offset_y)
        .map(|tile| CoverStatic {
            flags: tilemeta.item_tile(u32::from(tile.graphic)).map_or(0, |meta| meta.flags),
            z: tile.z,
        })
        .chain(multi_parts.cell(x, y).iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::render::scene::world::z_view::kr_layer_cut_z;

    const OPAQUE: u64 = TaeFlag::Surface.bits();
    const ROOF: u64 = TaeFlag::Roof.bits();
    const WALL: u64 = TaeFlag::Wall.bits();

    fn item(flags: u64, z: i8) -> CoverStatic {
        CoverStatic { flags, z }
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
        let parts = multi_parts_on_cells((10, 10), 5, &[floor, far, behind], &(0..20), &(0..20), |_| OPAQUE);
        assert_eq!(parts, [((11, 11), item(OPAQUE, 25))]);
        // A part over a light's cover cell covers it as a static does.
        let cover = parts.iter().map(|(_, part)| *part).collect::<Vec<_>>();
        assert!(light_is_hidden(5, cover.iter().map(|part| (part.flags, part.z)), NO_Z_VIEW_LIMIT));
        assert!(client_light_is_hidden(5, 5, cover.iter().map(|part| part.z), NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn without_placed_multis_no_part_covers() {
        let none = MultiCoverParts::default();
        assert!(none.cell(11, 11).is_empty());
    }

    #[test]
    fn the_kr_layers_are_the_bands_clamped_to_sixteen() {
        assert_eq!(kr_light_layer(0, 0), 7);
        assert_eq!(kr_light_layer(20, 0), 8);
        // Far below the land the layer stays 0, far above it stops at the top layer.
        assert_eq!(kr_light_layer(-128, 40), 0);
        assert_eq!(kr_light_layer(-128, 0), 0);
        assert_eq!(kr_light_layer(100, -100), KR_LIGHT_TOP_LAYER);
        assert_eq!(kr_light_layer(127, -128), KR_LIGHT_TOP_LAYER);
    }

    #[test]
    fn kr_hides_a_light_under_a_higher_layer() {
        // Britain 1407,1595 on map 1: the light at 30 over the average land 7 of its cell in layer 8, the pavers at 70
        // over the cover cell's land 30 in layer 9 (the boards at 50 in layer 8).
        assert!(kr_light_is_hidden(30, 7, 30, [50, 70], NO_Z_VIEW_LIMIT));
        assert!(!kr_light_is_hidden(30, 7, 30, [50], NO_Z_VIEW_LIMIT));
        // The keep's sconces at 63 under the floor at 72, all over land 50.
        assert!(kr_light_is_hidden(63, 50, 50, [72], NO_Z_VIEW_LIMIT));
        // Nothing in front, or only the light's own layer: lit.
        assert!(!kr_light_is_hidden(30, 0, 0, [], NO_Z_VIEW_LIMIT));
        assert!(!kr_light_is_hidden(0, 0, 0, [19], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn kr_counts_each_layer_from_its_own_cells_land() {
        // Map 1, 337,1284: mushrooms at -3 over the average land -2 (layer 6), another static at 0 in front over land
        // 0 (layer 7). KR hides them; the Enhanced client counts both from the light cell's raw land -3, band 7.
        assert!(kr_light_is_hidden(-3, -2, 0, [0], NO_Z_VIEW_LIMIT));
        assert!(!client_light_is_hidden(-3, -3, [0], NO_Z_VIEW_LIMIT));
        // A cover cell whose land rises with the cover keeps it in the light's layer.
        assert!(!kr_light_is_hidden(0, 0, 20, [20], NO_Z_VIEW_LIMIT));
        assert!(client_light_is_hidden(0, 0, [20], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn kr_clamps_a_high_light_and_its_cover_into_the_top_layer() {
        // A light 200 z above the land and a cover 20 z above it: both in the top layer, so KR keeps the light lit;
        // the Enhanced client's unclamped bands hide it.
        assert!(!kr_light_is_hidden(100, -100, -100, [120], NO_Z_VIEW_LIMIT));
        assert!(client_light_is_hidden(100, -100, [120], NO_Z_VIEW_LIMIT));
    }

    #[test]
    fn the_kr_rule_follows_the_z_limit() {
        // The ceiling over the player's floor at the limit no longer covers the floor's lights; the player's own floor
        // below the limit still covers the lights of the floor beneath.
        assert!(!kr_light_is_hidden(0, 0, 0, [20], 20));
        assert!(kr_light_is_hidden(0, 0, 0, [20, 40], 41));
        // A light at or above the limit goes with its item.
        assert!(kr_light_is_hidden(20, 0, 0, [], 20));
    }

    #[test]
    fn floors_and_roofs_over_a_light_cover_it_whatever_the_client_bands() {
        // Britain 1407,1594 on map 1: the KR layers put the window at 50 and the pavers at 70 in front in one layer.
        assert!(!kr_light_is_hidden(50, 7, 30, [70], NO_Z_VIEW_LIMIT));
        // The Enhanced client's bands too: the window at 50 over land 30 in band 8, the pavers at 70 in band 9 hide it,
        // but a roof at 55 sits in its band.
        assert!(!client_light_is_hidden(50, 30, [55], NO_Z_VIEW_LIMIT));
        assert!(floor_covers_light(50, &[item(OPAQUE, 70)], NO_Z_VIEW_LIMIT));
        assert!(floor_covers_light(50, &[item(ROOF, 55)], NO_Z_VIEW_LIMIT));
        // Too close above, see-through, a wall, or cut by the z limit: no cover.
        assert!(!floor_covers_light(50, &[item(OPAQUE, 54)], NO_Z_VIEW_LIMIT));
        assert!(!floor_covers_light(50, &[item(OPAQUE | TaeFlag::Transparent.bits(), 70)], NO_Z_VIEW_LIMIT));
        assert!(!floor_covers_light(50, &[item(WALL, 70)], NO_Z_VIEW_LIMIT));
        assert!(!floor_covers_light(50, &[item(OPAQUE, 70)], 70));
    }

    #[test]
    fn under_krs_cut_the_players_floor_covers_the_lights_below_it() {
        // Britain 1409,1596 on map 1: the player upstairs at 50 over land 30. The west wall's windows at 30 stand on
        // the river bank (their cells over land 7), their cover cell inside (over land 30) holds the floor at 50. The
        // cut counts from the player's cell, 70 on both cells, so the floor stays drawn and covers the windows.
        let max_z = kr_layer_cut_z(50, 30);
        let floor = [item(OPAQUE, 50)];
        assert!(floor_covers_light(30, &floor, max_z));
        // KR's own layers would let it through: the windows in layer 8 over land 7, the floor in layer 8 over land 30.
        assert!(!kr_light_is_hidden(30, 7, 30, [50], max_z));
        // The player downstairs at 30: the cut at 50 drops the floor, and the windows light the player's floor.
        let max_z = kr_layer_cut_z(30, 30);
        assert!(!floor_covers_light(30, &floor, max_z) && !kr_light_is_hidden(30, 7, 30, [50], max_z));
    }

    #[test]
    fn kr_reads_the_average_land_z_of_each_cell() {
        // A cell sloping from 0 to 10 along its 0-to-10 diagonal and flat 4 across the other: the flatter diagonal.
        let land = |x: u32, y: u32| -> Option<i8> {
            Some(match (x, y) {
                (10, 10) => 0,
                (11, 11) => 10,
                _ => 4,
            })
        };
        assert_eq!(kr_cell_land_z(&land, 10, 10), Some(4));
        assert_eq!(kr_cell_land_z(&|_, _| Some(-7), 10, 10), Some(-7));
        // A corner not known: none.
        assert_eq!(kr_cell_land_z(&|x, _| (x == 10).then_some(0), 10, 10), None);
    }

    #[test]
    fn clients_is_the_kr_rule_only_in_kr_like() {
        assert_eq!(LightCoverRule::of_style(LightCover::Clients, true), LightCoverRule::KrClient);
        assert_eq!(LightCoverRule::of_style(LightCover::Clients, false), LightCoverRule::EnhancedClient);
        assert_eq!(LightCoverRule::of_style(LightCover::ClassicUo, true), LightCoverRule::ClassicUo);
        assert_eq!(LightCoverRule::of_style(LightCover::ClassicUo, false), LightCoverRule::ClassicUo);
        assert_eq!(LightCoverRule::of_style(LightCover::Uod, true), LightCoverRule::Uod);
        assert_eq!(LightCoverRule::of_style(LightCover::Uod, false), LightCoverRule::Uod);
    }

    #[test]
    fn a_lights_ceiling_is_the_lowest_floor_or_roof_over_it() {
        // Britain 1407,1595: a window at 30 under the boards at 50 and the pavers at 70.
        let own: &[CoverStatic] = &[item(OPAQUE, 30), item(WALL, 30)];
        let cover: &[CoverStatic] = &[item(OPAQUE, 70), item(OPAQUE, 50)];
        assert_eq!(light_ceiling_z(30, &[own, cover], NO_Z_VIEW_LIMIT), Some(50));
        // The roof over its own cell counts as well as the one in front.
        assert_eq!(light_ceiling_z(30, &[&[item(ROOF, 45)], cover], NO_Z_VIEW_LIMIT), Some(45));
        // Its own floor, a wall, a see-through floor and a floor within the rise are no ceiling.
        let none: &[CoverStatic] =
            &[item(OPAQUE, 34), item(WALL, 50), item(OPAQUE | TaeFlag::Transparent.bits(), 50)];
        assert_eq!(light_ceiling_z(30, &[own, none], NO_Z_VIEW_LIMIT), None);
        // The z limit cuts the floors it stops drawing.
        assert_eq!(light_ceiling_z(30, &[own, cover], 70), Some(50));
        assert_eq!(light_ceiling_z(30, &[own, cover], 50), None);
    }

    #[test]
    fn the_client_rule_follows_the_z_limit() {
        // A cover the limit stops drawing no longer covers; a light at or above the limit goes with its item.
        assert!(!client_light_is_hidden(0, 0, [20], 20));
        assert!(client_light_is_hidden(0, 0, [20], 21));
        assert!(client_light_is_hidden(20, 0, [], 20));
    }
}
