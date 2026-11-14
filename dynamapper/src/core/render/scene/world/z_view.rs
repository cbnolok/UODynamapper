//! The z-view limit ("Show Only the Player's Floor"): what is above the player is not drawn, as the client does.
//!
//! The rule is ClassicUO's `GameScene.UpdateMaxDrawZ` (`GameSceneDrawingSorting.cs`, a Classic reference), recomputed
//! when the player's cell or z changes:
//! - land on the player's cell at least `PLAYER_VIEW_HEADROOM` above the player (the player is under a mountain, in a
//!   cave) caps statics and land at the player's z + `PLAYER_VIEW_HEADROOM`;
//! - otherwise a static on the player's cell more than `PLAYER_CEILING_MIN_RISE` above the player, not foliage, not
//!   see-through, and a floor if it is a roof (a ceiling or an upper floor), caps statics at its z;
//! - a roof on the cell (x+1, y+1) more than `PLAYER_CEILING_MIN_RISE` above the player and below the cap caps statics
//!   at the lowest z of the roof it belongs to (`near_roof_z`, ClassicUO's `Map.CalculateNearZ`);
//! - a cap below the player's z + `PLAYER_VIEW_HEADROOM` is raised to it, and either ceiling also hides every roof.
//!
//! Statics and multi parts at or above `ZViewCut::max_z` are not drawn, and roofs while `ZViewCut::hide_roofs`
//! (ClassicUO's `ProcessAlpha`, which fades them out; Dynamapper cuts them at once). Land above
//! `ZViewCut::max_ground_z` is not drawn, which only the cave case sets: in the open and in buildings all land stays.
//!
//! KR-like can follow the Kingdom Reborn client's rule instead (`PlayerFloorRule::Kr`, `compute_kr_z_view_cut`), which
//! tests the same two cells with the same threshold (UOKR.exe `0x0043c350`: a static more than
//! `PLAYER_CEILING_MIN_RISE` above the player on the player's cell with flags `0x20004` clear and Roof only with
//! Surface, or a Roof on the next cell, with no other flag test) and then, rather than a cap at the ceiling's z, hides
//! whole 20-z height layers. What KR does:
//! - every static, item and multi part is filed in a layer, clamp((z - land z + 140) / 20, 0, 15), its land z the
//!   average of its own cell's corners (`0x00458160` → `0x0043c8a0` → `0x0043c950`, `0x00587e50`, `0x0043c5b0`; the
//!   layer groups `0x0043d3a0`, `0x0043d2c0`, `0x0043d070`; multis `0x0056a6d0`); mobiles and one kind of multi
//!   sprite go to an unlayered group (`0x00c931b0`, `0x0056c3f0`) that is never cut;
//! - with a ceiling the cut is the player's own layer, counted from the player's cell, without one it is 16, none;
//!   `0x0043c7d0` culls the layer groups above the cut, every frame the cut changes;
//! - land is an unlayered group of its own (`0x00c931a0`, built by `0x004589e1`), never cut by height: it is culled
//!   whole while the player's z is below the averaged land z of the player's cell (`0x00587ad0`, under the viewer's
//!   flag +0x50, set by its constructor `0x00459360` and never cleared), which is how KR shows a cave: no land at
//!   all, the cave's statics on black;
//! - there are no roof flags in it: a roof goes only when its layer does.
//!
//! Dynamapper diverges from KR in one point: every object's layer is counted from the averaged land z of the player's
//! cell, not of its own, so the cut is one z for the whole view, `ZViewCut::max_z`, the bottom of the layer above the
//! player's (`kr_layer_cut_z`). KR's count from each object's own cell drops whole walls where a building stands on
//! uneven land: at Britain 1409,1596 on map 1 the house's west wall stands on the river bank (cells x 1407, averaged
//! land 7, against 30 inside), so with the player in the room at z 30 (layer 7) the wall and its windows, at z 30
//! over land 7, are in layer 8 and go, in KR too. Counting from the player's cell keeps the ceiling test, the 20-z
//! layers, their clamp, the cut at the player's own layer and the land's drop below ground; only the land under an
//! object no longer moves it across a layer.

use crate::configs::settings::Settings;
use crate::core::multis::{expanded_multi_part_z, MultiDefinitionsRes, PlacedMultisRes};
use crate::core::render::scene::player::Player;
use crate::core::render::scene::world::art::light_cover::{
    kr_light_layer, CLIENT_BAND_BASE, CLIENT_BAND_HEIGHT, KR_LIGHT_TOP_LAYER, NO_Z_VIEW_LIMIT,
};
use crate::core::render::scene::world::land::mesh_material::PlayerFloorRule;
use crate::core::render::scene::world::land::mesh_material::LAND_NO_Z_LIMIT;
use crate::core::render::scene::world::land::MAP_STORAGE_BLOCK_TILE_DIM;
use crate::core::render::scene::SceneStateData;
use crate::configs::shader_presets::UniformState;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::uo_files_loader::{MapPlanesRes, TileMetaPackageRes};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use udd_assets::tilemeta::TileMetaPackage;
use uocf::enhanced::tileart::TaeFlag;

/// How far above the player a ceiling must start, in z units: ClassicUO's `playerZ + 14`.
pub const PLAYER_CEILING_MIN_RISE: i32 = 14;
/// The lowest cap above the player, and how far above the player land must rise to hide the land: ClassicUO's
/// `playerZ + 16`.
pub const PLAYER_VIEW_HEADROOM: i32 = 16;
/// How far apart in z two neighbouring roof tiles may be and still count as one roof (ClassicUO's `CalculateNearZ`).
pub const NEAR_ROOF_MAX_STEP: i32 = 6;
/// How far from the player, in cells along x or y, the rule reads the map, so the roof flood stays bounded: half of
/// ClassicUO's 64 x 64 visited-cell window.
pub const Z_VIEW_REGION_RADIUS: i32 = 32;
/// How far a multi's parts may lie from its anchor, in cells, so the multis around the region are found.
pub const MULTI_PART_MAX_REACH: i32 = 32;

/// Statics on the player's cell that never cap the view: foliage and the see-through (ClassicUO's `0x20004`).
const CEILING_SKIP_FLAGS: u64 = TaeFlag::Foliage.bits() | TaeFlag::Transparent.bits();
/// Roofs on the next cell that never cap the view: the see-through and the walkable (ClassicUO's `0x204`).
const ROOF_SKIP_FLAGS: u64 = TaeFlag::Transparent.bits() | TaeFlag::Surface.bits();
/// Statics the rule ignores, as they are never drawn.
const NEVER_DRAWN_FLAGS: u64 = TaeFlag::NoDraw.bits();
/// `ZViewCut::max_ground_z` when every land cell is hidden, below the lowest land z.
pub const ALL_LAND_HIDDEN_Z: i32 = i8::MIN as i32 - 1;

/// What the z-view limit hides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZViewCut {
    /// Statics and multi parts at or above this z are not drawn.
    pub max_z: i32,
    /// Land above this z is not drawn.
    pub max_ground_z: i32,
    /// Roofs are not drawn.
    pub hide_roofs: bool,
}

impl ZViewCut {
    /// Nothing is hidden.
    pub const NONE: Self = Self { max_z: NO_Z_VIEW_LIMIT, max_ground_z: NO_Z_VIEW_LIMIT, hide_roofs: false };

    /// Whether a static or multi part with tile flags `flags` at `z` is hidden.
    pub fn hides_static(&self, flags: u64, z: i8) -> bool {
        i32::from(z) >= self.max_z || (self.hide_roofs && flags & TaeFlag::Roof.bits() != 0)
    }

    /// `max_ground_z` for the land shader (`SceneUniform::land_max_z`).
    pub fn land_max_z(&self) -> f32 {
        if self.max_ground_z == NO_Z_VIEW_LIMIT { LAND_NO_Z_LIMIT } else { self.max_ground_z as f32 }
    }
}

impl Default for ZViewCut {
    fn default() -> Self {
        Self::NONE
    }
}

/// The statics and multi parts the rule reads, as (tile flags, z), per cell.
pub trait ZViewCells {
    fn objects(&self, x: i32, y: i32) -> &[(u64, i8)];
}

/// The cut for a player on (`x`, `y`) at `z`, over land `player_land_z` (the cell's average z where it is stretched;
/// `None` while unknown, which never caps).
pub fn compute_z_view_cut(x: i32, y: i32, z: i32, player_land_z: Option<i32>, cells: &impl ZViewCells) -> ZViewCut {
    let ceiling_min = z + PLAYER_CEILING_MIN_RISE;
    let headroom = z + PLAYER_VIEW_HEADROOM;
    let mut max_z = NO_Z_VIEW_LIMIT;
    let mut max_ground_z = NO_Z_VIEW_LIMIT;
    let mut hide_roofs = false;

    if player_land_z.is_some_and(|land_z| land_z >= headroom) {
        max_z = headroom;
        max_ground_z = headroom;
    } else {
        for &(flags, object_z) in cells.objects(x, y) {
            let object_z = i32::from(object_z);
            let is_roof = flags & TaeFlag::Roof.bits() != 0;
            let is_surface = flags & TaeFlag::Surface.bits() != 0;
            if object_z > ceiling_min && max_z > object_z && flags & CEILING_SKIP_FLAGS == 0 && (!is_roof || is_surface)
            {
                max_z = object_z;
                hide_roofs = true;
            }
        }
    }

    let mut roof_cap = max_z;
    let (next_x, next_y) = (x + 1, y + 1);
    for &(flags, object_z) in cells.objects(next_x, next_y) {
        let object_z = i32::from(object_z);
        if object_z > ceiling_min
            && max_z > object_z
            && flags & ROOF_SKIP_FLAGS == 0
            && flags & TaeFlag::Roof.bits() != 0
        {
            max_z = object_z;
            roof_cap = near_roof_z(cells, object_z, next_x, next_y, object_z, (x, y));
            hide_roofs = true;
        }
    }

    max_z = roof_cap;
    if roof_cap < headroom {
        max_z = headroom;
    }
    ZViewCut { max_z, max_ground_z, hide_roofs }
}

/// The KR client's cut for a player on (`x`, `y`) at `z`, over the averaged land z `player_land_z` of the cell (`None`
/// while unknown: then nothing hides the land and the player's own z stands in for it to count the layers). A static
/// more than `PLAYER_CEILING_MIN_RISE` above the player on the player's cell, neither foliage nor see-through and a
/// floor if it is a roof, or a roof on the cell (x+1, y+1), hides the layers above the player's (UOKR.exe
/// `0x0043c350`), every object's counted from the player's cell (`kr_layer_cut_z`); the land goes whole while the
/// player is below it (`0x00587ad0`).
pub fn compute_kr_z_view_cut(x: i32, y: i32, z: i32, player_land_z: Option<i32>, cells: &impl ZViewCells) -> ZViewCut {
    let ceiling_min = z + PLAYER_CEILING_MIN_RISE;
    let under_ceiling = cells.objects(x, y).iter().any(|&(flags, object_z)| {
        let is_roof = flags & TaeFlag::Roof.bits() != 0;
        let is_surface = flags & TaeFlag::Surface.bits() != 0;
        i32::from(object_z) > ceiling_min && flags & CEILING_SKIP_FLAGS == 0 && (!is_roof || is_surface)
    }) || cells
        .objects(x + 1, y + 1)
        .iter()
        .any(|&(flags, object_z)| i32::from(object_z) > ceiling_min && flags & TaeFlag::Roof.bits() != 0);
    let below_land = player_land_z.is_some_and(|land_z| z < land_z);
    ZViewCut {
        max_z: if under_ceiling { kr_layer_cut_z(z, player_land_z.unwrap_or(z)) } else { NO_Z_VIEW_LIMIT },
        max_ground_z: if below_land { ALL_LAND_HIDDEN_Z } else { NO_Z_VIEW_LIMIT },
        hide_roofs: false,
    }
}

/// The z from which KR's cut hides the view for a player at `z` over the averaged land z `land_z`: the bottom of the
/// layer above the player's (`kr_light_layer`: `(z - land_z + CLIENT_BAND_BASE) / CLIENT_BAND_HEIGHT`, clamped to 0 to
/// `KR_LIGHT_TOP_LAYER`), every object's layer counted from the same land z; none while the player is in the top layer.
pub fn kr_layer_cut_z(z: i32, land_z: i32) -> i32 {
    let player_layer = kr_light_layer(clamp_to_i8(z), clamp_to_i8(land_z));
    if player_layer >= KR_LIGHT_TOP_LAYER {
        NO_Z_VIEW_LIMIT
    } else {
        land_z + CLIENT_BAND_HEIGHT * (player_layer + 1) - CLIENT_BAND_BASE
    }
}

/// `z` in the map's z range.
fn clamp_to_i8(z: i32) -> i8 {
    z.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
}

/// The lowest z of the roof through (`x`, `y`): from a roof tile at `z`, every roof tile reachable over the four
/// neighbours with each step at most `NEAR_ROOF_MAX_STEP` in z, never farther than `Z_VIEW_REGION_RADIUS` from
/// `center`; `default_z` when it is lower. ClassicUO's `Map.CalculateNearZ`, which takes the first roof of a cell in
/// draw order, the lowest.
pub fn near_roof_z(cells: &impl ZViewCells, default_z: i32, x: i32, y: i32, z: i32, center: (i32, i32)) -> i32 {
    let mut lowest = default_z;
    let mut visited = HashSet::new();
    let mut stack = vec![(x, y, z)];
    while let Some((cell_x, cell_y, reference_z)) = stack.pop() {
        if (cell_x - center.0).abs() > Z_VIEW_REGION_RADIUS
            || (cell_y - center.1).abs() > Z_VIEW_REGION_RADIUS
            || !visited.insert((cell_x, cell_y))
        {
            continue;
        }
        let roof_z = cells
            .objects(cell_x, cell_y)
            .iter()
            .filter(|(flags, roof_z)| {
                flags & TaeFlag::Roof.bits() != 0 && (reference_z - i32::from(*roof_z)).abs() <= NEAR_ROOF_MAX_STEP
            })
            .map(|(_, roof_z)| i32::from(*roof_z))
            .min();
        let Some(roof_z) = roof_z else {
            continue;
        };
        lowest = lowest.min(roof_z);
        // Pushed in reverse, so they are visited in ClassicUO's order: west, east, north, south.
        stack.extend([(cell_x, cell_y + 1, roof_z), (cell_x, cell_y - 1, roof_z), (cell_x + 1, cell_y, roof_z), (cell_x - 1, cell_y, roof_z)]);
    }
    lowest
}

/// The statics and multi parts within `Z_VIEW_REGION_RADIUS` of a cell, read from the map data.
#[derive(Default)]
pub struct ZViewRegion {
    cells: HashMap<(i32, i32), Vec<(u64, i8)>>,
}

impl ZViewCells for ZViewRegion {
    fn objects(&self, x: i32, y: i32) -> &[(u64, i8)] {
        self.cells.get(&(x, y)).map_or(&[], Vec::as_slice)
    }
}

impl ZViewRegion {
    /// Reads the region around (`center_x`, `center_y`) on the map `map_id`: every static on it, each as itself
    /// whatever its graphic (map statics are never multis), and every part of the multis placed within
    /// `MULTI_PART_MAX_REACH` of it that lands on it (the drawn parts, or all with `show_hidden_multi_parts`).
    pub fn read(
        statics_store: &mut LazyStaticsStore,
        tilemeta: &TileMetaPackage,
        multis: Option<&MultiDefinitionsRes>,
        placed_multis: &PlacedMultisRes,
        show_hidden_multi_parts: bool,
        map_id: u32,
        center_x: i32,
        center_y: i32,
    ) -> Self {
        let mut region = Self::default();
        let in_region = |x: i32, y: i32| {
            (x - center_x).abs() <= Z_VIEW_REGION_RADIUS && (y - center_y).abs() <= Z_VIEW_REGION_RADIUS
        };
        let block_dim = MAP_STORAGE_BLOCK_TILE_DIM as i32;
        let block_range = |center: i32| {
            ((center - Z_VIEW_REGION_RADIUS).max(0) / block_dim)..=((center + Z_VIEW_REGION_RADIUS).max(0) / block_dim)
        };
        for block_y in block_range(center_y) {
            for block_x in block_range(center_x) {
                let Ok(tiles) = statics_store.block_tiles(block_x as u32, block_y as u32) else {
                    continue;
                };
                for tile in tiles {
                    let x = block_x * block_dim + i32::from(tile.x_offset());
                    let y = block_y * block_dim + i32::from(tile.y_offset());
                    if in_region(x, y) {
                        region.push(tilemeta, x, y, tile.graphic, tile.z);
                    }
                }
            }
        }
        let Some(multis) = multis else {
            return region;
        };
        let reach = Z_VIEW_REGION_RADIUS + MULTI_PART_MAX_REACH;
        let anchor_cells = |center: i32| (center - reach).max(0) as u32..(center + reach + 1).max(0) as u32;
        for multi in placed_multis.anchored_in(map_id, anchor_cells(center_x), anchor_cells(center_y)) {
            let Some(parts) = multis.parts(multi.multi_id, show_hidden_multi_parts) else {
                continue;
            };
            let (x, y) = (multi.x as i32, multi.y as i32);
            for part in parts {
                let (part_x, part_y) = (x + i32::from(part.x), y + i32::from(part.y));
                if in_region(part_x, part_y) {
                    region.push(tilemeta, part_x, part_y, part.item_id, expanded_multi_part_z(multi.z, part.z));
                }
            }
        }
        region
    }

    fn push(&mut self, tilemeta: &TileMetaPackage, x: i32, y: i32, graphic: u16, z: i8) {
        let flags = tilemeta.item_tile(u32::from(graphic)).map_or(0, |meta| meta.flags);
        if flags & NEVER_DRAWN_FLAGS == 0 {
            self.cells.entry((x, y)).or_default().push((flags, z));
        }
    }
}

/// The z-view limit in effect: `ZViewCut::NONE` while the setting is off.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZViewLimit(pub ZViewCut);

/// What the limit was last computed from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ZViewInputs {
    rule: PlayerFloorRule,
    map_id: u32,
    x: u16,
    y: u16,
    z: i8,
    show_hidden_multi_parts: bool,
    land_known: bool,
}

/// The land z ClassicUO tests on the player's cell: the average of the two diagonals' ends that differ least, which a
/// flat cell's own z equals (`Land.AverageZ`). `None` while a corner's block is not loaded.
fn player_cell_land_z(map_planes: Option<&MapPlanesRes>, map_id: u32, x: i32, y: i32) -> Option<i32> {
    let land_z = |cell_x: i32, cell_y: i32| -> Option<i32> {
        use uocf::classic::map::{MapCell, MapCellCoords};
        let plane = map_planes?.0.get(usize::try_from(map_id).ok()?)?.as_ref()?;
        let coords = MapCellCoords { x: u32::try_from(cell_x).ok()?, y: u32::try_from(cell_y).ok()? };
        let rel = MapCell::coords_in_block(&coords);
        let block = plane.block_no_update(MapCell::coords_of_parent_block(&coords))?;
        Some(i32::from(block.cell(rel.x, rel.y).ok()?.z))
    };
    cell_average_land_z(land_z, x, y)
}

/// The averaged land z of the cell (`x`, `y`) (`average_land_z`), `land_z` giving each cell's raw land z; `None` while
/// a corner is not known.
fn cell_average_land_z(land_z: impl Fn(i32, i32) -> Option<i32>, x: i32, y: i32) -> Option<i32> {
    Some(average_land_z(land_z(x, y)?, land_z(x + 1, y)?, land_z(x + 1, y + 1)?, land_z(x, y + 1)?))
}

/// ClassicUO's `Land.AverageZ` from the cell's corners (`top` its own z, then east, south-east and south).
pub fn average_land_z(top: i32, right: i32, bottom: i32, left: i32) -> i32 {
    if (top - bottom).abs() <= (left - right).abs() { (top + bottom) >> 1 } else { (left + right) >> 1 }
}

/// Recomputes `ZViewLimit` when the player's cell, z or map, the setting, or the style's rule
/// (`UniformState::effective_player_floor_rule`) changes.
pub fn sys_update_z_view_limit(
    settings: Res<Settings>,
    uniforms: Option<Res<UniformState>>,
    scene_state: Res<SceneStateData>,
    player_q: Query<&Player>,
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    multis: (Option<Res<MultiDefinitionsRes>>, Res<PlacedMultisRes>),
    map_planes: Option<Res<MapPlanesRes>>,
    mut limit: ResMut<ZViewLimit>,
    mut last_inputs: Local<Option<ZViewInputs>>,
) {
    let player_pos = player_q.iter().find_map(|player| player.current_pos);
    let (Some(pos), Some(tilemeta), true) =
        (player_pos, tilemeta_res, settings.world_rendering.show_only_player_floor)
    else {
        *last_inputs = None;
        limit.set_if_neq(ZViewLimit(ZViewCut::NONE));
        return;
    };
    let map_id = scene_state.map_id;
    let (x, y, z) = (i32::from(pos.x), i32::from(pos.y), i32::from(pos.z));
    let land_z = player_cell_land_z(map_planes.as_deref(), map_id, x, y);
    let rule = uniforms.map_or(PlayerFloorRule::ClassicUo, |uniforms| uniforms.effective_player_floor_rule());
    let inputs = ZViewInputs {
        rule,
        map_id,
        x: pos.x,
        y: pos.y,
        z: pos.z,
        show_hidden_multi_parts: settings.world_rendering.show_hidden_multi_parts,
        land_known: land_z.is_some(),
    };
    if *last_inputs == Some(inputs) {
        return;
    }
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let region = ZViewRegion::read(
        &mut statics_store.lock(),
        &tilemeta.0,
        multis.0.as_deref(),
        &multis.1,
        inputs.show_hidden_multi_parts,
        map_id,
        x,
        y,
    );
    *last_inputs = Some(inputs);
    let cut = match rule {
        PlayerFloorRule::ClassicUo => compute_z_view_cut(x, y, z, land_z, &region),
        PlayerFloorRule::Kr => compute_kr_z_view_cut(x, y, z, land_z, &region),
    };
    limit.set_if_neq(ZViewLimit(cut));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: u64 = TaeFlag::Surface.bits() | TaeFlag::Background.bits();
    const ROOF: u64 = TaeFlag::Roof.bits();
    const WALL: u64 = TaeFlag::Wall.bits();
    const FOLIAGE: u64 = TaeFlag::Foliage.bits();

    #[derive(Default)]
    struct Cells(HashMap<(i32, i32), Vec<(u64, i8)>>);

    impl Cells {
        fn with(mut self, x: i32, y: i32, flags: u64, z: i8) -> Self {
            self.0.entry((x, y)).or_default().push((flags, z));
            self
        }
    }

    impl ZViewCells for Cells {
        fn objects(&self, x: i32, y: i32) -> &[(u64, i8)] {
            self.0.get(&(x, y)).map_or(&[], Vec::as_slice)
        }
    }

    /// A room: the floor above caps the view at its z and hides the roof; land stays.
    #[test]
    fn room_under_an_upper_floor_caps_at_the_floor() {
        // The Britain cell (map 1, 1413,1595): wooden floor land at 30, a table, boards at 50, pavers at 70.
        let cells = Cells::default().with(10, 10, 0, 30).with(10, 10, FLOOR, 50).with(10, 10, FLOOR, 70);
        let cut = compute_z_view_cut(10, 10, 30, Some(30), &cells);
        assert_eq!(cut, ZViewCut { max_z: 50, max_ground_z: NO_Z_VIEW_LIMIT, hide_roofs: true });
        assert!(cut.hides_static(FLOOR, 50) && cut.hides_static(WALL, 70) && cut.hides_static(ROOF, 40));
        assert!(!cut.hides_static(WALL, 49));
    }

    /// The upper floor of a two-storey building: the floor the player stands on and its walls stay, the storey above
    /// and the roof go.
    #[test]
    fn upper_storey_caps_at_the_next_floor() {
        let cells = Cells::default()
            .with(10, 10, FLOOR, 20)
            .with(10, 10, FLOOR, 40)
            .with(11, 11, ROOF, 60)
            .with(12, 12, ROOF, 63);
        let cut = compute_z_view_cut(10, 10, 20, Some(0), &cells);
        assert_eq!(cut, ZViewCut { max_z: 40, max_ground_z: NO_Z_VIEW_LIMIT, hide_roofs: true });
        let upstairs = compute_z_view_cut(10, 10, 40, Some(0), &cells);
        assert_eq!(upstairs.max_z, 60, "the roof one cell south-east caps the top storey");
        assert!(upstairs.hide_roofs && !upstairs.hides_static(FLOOR, 40));
    }

    /// Under a roof alone: the cap is the lowest z of the connected roof (its eaves), not the tile over the player.
    #[test]
    fn roof_only_caps_at_the_eaves_of_the_connected_roof() {
        let cells = Cells::default()
            .with(11, 11, ROOF, 30)
            .with(12, 11, ROOF, 27)
            .with(13, 11, ROOF, 24)
            .with(14, 11, ROOF, 21)
            // A separate roof more than `NEAR_ROOF_MAX_STEP` lower is not part of it.
            .with(15, 11, ROOF, 5);
        let cut = compute_z_view_cut(10, 10, 0, Some(0), &cells);
        assert_eq!(cut, ZViewCut { max_z: 21, max_ground_z: NO_Z_VIEW_LIMIT, hide_roofs: true });
    }

    /// A cap below the player's headroom is raised to it.
    #[test]
    fn low_roof_cap_is_raised_to_the_headroom() {
        let cells = Cells::default().with(11, 11, ROOF, 15).with(12, 11, ROOF, 10);
        let cut = compute_z_view_cut(10, 10, 0, Some(0), &cells);
        assert_eq!(cut.max_z, PLAYER_VIEW_HEADROOM);
        assert!(cut.hide_roofs);
    }

    /// In the open, and next to tall items that are not ceilings, nothing is hidden.
    #[test]
    fn open_air_hides_nothing() {
        let cells = Cells::default()
            .with(10, 10, FOLIAGE, 40)
            .with(10, 10, TaeFlag::Transparent.bits(), 40)
            .with(10, 10, ROOF, 40)
            .with(10, 10, FLOOR, 14)
            // A walkable roof, or a roof right overhead on the player's own cell, is no roof one cell away.
            .with(11, 11, ROOF | TaeFlag::Surface.bits(), 40)
            .with(11, 11, WALL, 40);
        assert_eq!(compute_z_view_cut(10, 10, 0, Some(0), &cells), ZViewCut::NONE);
        assert_eq!(compute_z_view_cut(10, 10, 0, None, &Cells::default()), ZViewCut::NONE);
    }

    /// In a cave under a mountain: the land over the player hides land and statics above the headroom; the cave
    /// floor and the player's surroundings stay. At the entrance, with the land below the headroom, nothing is cut.
    #[test]
    fn cave_under_land_cuts_land_and_statics_at_the_headroom() {
        // The Minoc mine: rock land at 79, cave floor statics at 0.
        let cells = Cells::default().with(10, 10, FLOOR, 0).with(10, 10, WALL, 40);
        let cut = compute_z_view_cut(10, 10, 0, Some(79), &cells);
        assert_eq!(cut, ZViewCut { max_z: 16, max_ground_z: 16, hide_roofs: false });
        assert!(cut.hides_static(WALL, 40) && !cut.hides_static(FLOOR, 0));
        let entrance = Cells::default().with(10, 10, FLOOR, 0);
        assert_eq!(compute_z_view_cut(10, 10, 0, Some(15), &entrance), ZViewCut::NONE);
    }

    /// The flood stops at the region's edge on a roof wider than it.
    #[test]
    fn near_roof_flood_is_bounded() {
        let mut cells = Cells::default();
        for x in 0..(Z_VIEW_REGION_RADIUS * 4) {
            cells = cells.with(x, 0, ROOF, if x == Z_VIEW_REGION_RADIUS * 3 { 26 } else { 30 });
        }
        assert_eq!(near_roof_z(&cells, 30, 1, 0, 30, (0, 0)), 30);
        assert_eq!(near_roof_z(&cells, 30, Z_VIEW_REGION_RADIUS * 2, 0, 30, (Z_VIEW_REGION_RADIUS * 2, 0)), 26);
    }

    /// KR: the boards over a Britain room (map 1, 1413,1595: land at 30, boards at 50, pavers at 70) put the player
    /// at 30 under a ceiling; the layers above the player's, 7 ((30 - 30 + 140) / 20), go: from z 50.
    #[test]
    fn kr_ceiling_hides_the_layers_above_the_players() {
        let cells = Cells::default().with(10, 10, FLOOR, 50).with(10, 10, FLOOR, 70);
        let cut = compute_kr_z_view_cut(10, 10, 30, Some(30), &cells);
        assert_eq!(cut, ZViewCut { max_z: 50, ..ZViewCut::NONE });
        assert!(!cut.hides_static(FLOOR, 49) && cut.hides_static(FLOOR, 50));
        // A roof stays while its layer does: no roof flag in the KR rule.
        assert!(!cut.hides_static(ROOF, 45));
        // Upstairs on the boards, layer 8, the pavers at 70 are the ceiling and go; on the pavers, the open.
        let upstairs = compute_kr_z_view_cut(10, 10, 50, Some(30), &cells);
        assert_eq!(upstairs.max_z, 70);
        assert!(!upstairs.hides_static(FLOOR, 50) && upstairs.hides_static(FLOOR, 70));
        assert_eq!(compute_kr_z_view_cut(10, 10, 70, Some(30), &cells), ZViewCut::NONE);
    }

    /// KR's layers counted from the player's cell: anywhere in the player's layer the cut is the next layer's bottom,
    /// so one z for the whole view, and the land under another cell no longer moves its objects across a layer.
    #[test]
    fn kr_layers_count_from_the_players_cell() {
        // Britain 1409,1596 on map 1: the player in the room at 30 over land 30 (layer 7). The west wall on the river
        // bank, its cells over land 7, has its windows at 30: KR files them in layer 8 and hides them; the cut at 50
        // keeps them.
        let cells = Cells::default().with(10, 10, FLOOR, 50);
        let cut = compute_kr_z_view_cut(10, 10, 30, Some(30), &cells);
        assert!(!cut.hides_static(WALL, 30) && !cut.hides_static(WALL, 49));
        // The whole of the player's layer gives one cut.
        for z in [30, 40, 49] {
            assert_eq!(kr_layer_cut_z(z, 30), 50);
        }
        assert_eq!(kr_layer_cut_z(50, 30), 70);
        // Over land 7 the layers start 7 z higher: a player at 30 is in layer 8, from 27, cut from 47.
        assert_eq!(kr_layer_cut_z(30, 7), 47);
    }

    /// KR: the layers clamp, so a player in the top layer hides nothing, and one far below the land is in layer 0.
    #[test]
    fn kr_layers_clamp_to_the_client_range() {
        assert_eq!(kr_layer_cut_z(100, -100), NO_Z_VIEW_LIMIT);
        assert_eq!(kr_layer_cut_z(0, 0), CLIENT_BAND_HEIGHT * 8 - CLIENT_BAND_BASE);
        let cells = Cells::default().with(10, 10, FLOOR, 127);
        let cut = compute_kr_z_view_cut(10, 10, -100, Some(100), &cells);
        assert_eq!(cut.max_z, 100 + CLIENT_BAND_HEIGHT - CLIENT_BAND_BASE, "layer 0's top");
    }

    /// KR: the roof on the cell in front counts whatever its other flags; the player's own cell skips foliage,
    /// see-through and roofs that are no floor, as ClassicUO.
    #[test]
    fn kr_ceiling_tests_the_two_cells() {
        let open = Cells::default()
            .with(10, 10, FOLIAGE, 40)
            .with(10, 10, TaeFlag::Transparent.bits(), 40)
            .with(10, 10, ROOF, 40)
            .with(11, 11, WALL, 40)
            .with(11, 11, ROOF, 14);
        assert_eq!(compute_kr_z_view_cut(10, 10, 0, Some(0), &open), ZViewCut::NONE);
        let walkable_roof = Cells::default().with(11, 11, ROOF | TaeFlag::Surface.bits(), 15);
        assert_eq!(compute_kr_z_view_cut(10, 10, 0, Some(0), &walkable_roof).max_z, 20);
        let roof_floor = Cells::default().with(10, 10, ROOF | TaeFlag::Surface.bits(), 15);
        assert_eq!(compute_kr_z_view_cut(10, 10, 0, Some(0), &roof_floor).max_z, 20);
    }

    /// KR in the Minoc mine (rock land at 79, cave floor statics at 0): no ceiling statics, so no layer goes, but
    /// the player is below the land of the cell, so all land goes. At the entrance, level with the land, it stays.
    #[test]
    fn kr_player_below_the_land_hides_all_land() {
        let cells = Cells::default().with(10, 10, FLOOR, 0).with(11, 11, WALL, 40);
        let cut = compute_kr_z_view_cut(10, 10, 0, Some(79), &cells);
        assert_eq!(cut, ZViewCut { max_ground_z: ALL_LAND_HIDDEN_Z, ..ZViewCut::NONE });
        assert!(cut.land_max_z() < f32::from(i8::MIN));
        assert_eq!(compute_kr_z_view_cut(10, 10, 0, Some(0), &cells), ZViewCut::NONE);
        assert_eq!(compute_kr_z_view_cut(10, 10, 0, None, &cells), ZViewCut::NONE);
    }

    #[test]
    fn average_land_z_takes_the_flatter_diagonal() {
        assert_eq!(average_land_z(10, 10, 10, 10), 10);
        assert_eq!(average_land_z(0, 20, 4, 40), 2);
        assert_eq!(average_land_z(0, 20, 40, 24), 22);
    }
}
