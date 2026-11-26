//! The roof or upper floor over a mobile, under which the art shader orders the pixels of a tall body poking through
//! it (art/depth.wesl `mobile_fragment_view_z`): a dragon in a house shows its head through the roof otherwise, the
//! card standing at the mobile's depth and the roof tiles behind it ordered farther.
//!
//! The ceiling is the player's (z_view.rs `ceiling_over`, ClassicUO's `GameScene.UpdateMaxDrawZ`): a floor on the
//! mobile's cell more than `PLAYER_CEILING_MIN_RISE` above it, or a roof on the cell (x+1, y+1), which caps at the
//! lowest z of its roof; its plane is raised to the mobile's z + `PLAYER_VIEW_HEADROOM`, as the player's cut is. It is
//! found when the mobile's cell, z or map changes, or the map data does, and handed to the shader only while the
//! z-view's cut draws the static that found it: with the player in the same building the roof goes, and the mobile
//! draws whole.

use bevy::prelude::*;
use crate::content::{MapPlanesRes, TileMetaPackageRes};
use crate::render::scene::SceneStateData;
use crate::settings::Settings;
use crate::world::coords::scale_bevy_units_to_uo_z;
use crate::world::multis::{MultiDefinitionsRes, PlacedMultisRes};
use crate::world::statics::StaticsStoreRes;
use crate::world::z_view::{ceiling_over, Ceiling, PackageLand, ZViewCells, ZViewCut, ZViewRegion, NO_Z_VIEW_LIMIT,
    PLAYER_VIEW_HEADROOM, Z_VIEW_REGION_RADIUS};
use uocf::enhanced::tileart::TaeFlag;
use super::draw::mobile_depth_cell;

/// How far around a mobile's cell the map is read first, in cells: its own and the cell (x+1, y+1), the two the
/// ceiling is found on. A roof found there is followed to its lowest z over the player's region
/// (`Z_VIEW_REGION_RADIUS`).
const MOBILE_CEILING_PROBE_RADIUS: i32 = 1;

/// The ceiling over a mobile, found when what it is found from changes (`sys_update_mobile_ceilings`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MobileCeilingCache {
    inputs: Option<MobileCeilingInputs>,
    found: Option<FoundCeiling>,
}

/// What a mobile's ceiling was found from: the map, the cell the mobile is ordered from and its z.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MobileCeilingInputs {
    map_id: u32,
    x: i32,
    y: i32,
    z: i32,
    show_hidden_multi_parts: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FoundCeiling {
    ceiling: Ceiling,
    /// The z of the plane the mobile's pixels above it are ordered under.
    plane_z: i32,
    /// The averaged land z of the cell of a roof that found it, which KR's cut counts the roof's layer from.
    own_land_z: Option<i32>,
}

impl MobileCeilingCache {
    /// The z of the ceiling's plane, while `cut` draws the static that found it.
    pub fn drawn_plane_z(&self, cut: &ZViewCut) -> Option<i32> {
        self.found.filter(|found| !found.ceiling.hidden_by(cut, found.own_land_z)).map(|found| found.plane_z)
    }
}

/// The ceiling over a mobile on (`x`, `y`) at `z`, as over the player (`ceiling_over`), and the z of its plane: the cap
/// raised to the mobile's z + `PLAYER_VIEW_HEADROOM`, as the player's cut is (a floor at z + 15 lies under it).
pub fn mobile_ceiling(x: i32, y: i32, z: i32, cells: &impl ZViewCells) -> Option<(Ceiling, i32)> {
    ceiling_over(x, y, z, cells, true, NO_Z_VIEW_LIMIT)
        .map(|ceiling| (ceiling, ceiling.z.max(z + PLAYER_VIEW_HEADROOM)))
}

/// Finds the ceiling over each mobile whose cell, z or map changed, or all when the map data did.
pub fn sys_update_mobile_ceilings(
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    multis: (Option<Res<MultiDefinitionsRes>>, Res<PlacedMultisRes>),
    map_planes: Option<Res<MapPlanesRes>>,
    mut mobiles_q: Query<(&Transform, &mut MobileCeilingCache)>,
    mut land: Local<PackageLand>,
) {
    let Some(tilemeta) = tilemeta_res else { return };
    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(Option::as_ref) else { return };
    let data_changed = statics_res.is_changed() || tilemeta.is_changed()
        || multis.0.as_ref().is_some_and(|definitions| definitions.is_changed()) || multis.1.is_changed();
    let show_hidden_multi_parts = settings.world_rendering.show_hidden_multi_parts;
    for (transform, mut cache) in &mut mobiles_q {
        let (x, y) = mobile_depth_cell(transform.translation.x, transform.translation.z);
        let inputs = MobileCeilingInputs { map_id, x: x as i32, y: y as i32,
            z: scale_bevy_units_to_uo_z(transform.translation.y).round() as i32, show_hidden_multi_parts };
        if !data_changed && cache.inputs == Some(inputs) {
            continue;
        }
        let read = |radius| ZViewRegion::read(&mut statics_store.lock(), tilemeta.for_world_analyses(),
            multis.0.as_deref(), &multis.1, show_hidden_multi_parts, map_id, (inputs.x, inputs.y), radius);
        let mut found = mobile_ceiling(inputs.x, inputs.y, inputs.z, &read(MOBILE_CEILING_PROBE_RADIUS));
        if found.is_some_and(|(ceiling, _)| (ceiling.x, ceiling.y) != (inputs.x, inputs.y)) {
            found = mobile_ceiling(inputs.x, inputs.y, inputs.z, &read(Z_VIEW_REGION_RADIUS));
        }
        cache.found = found.map(|(ceiling, plane_z)| FoundCeiling { ceiling, plane_z,
            own_land_z: (ceiling.flags & TaeFlag::Roof.bits() != 0).then(|| land.averaged_land_z(map_planes.as_deref(),
                map_id, ceiling.x, ceiling.y)).flatten() });
        cache.inputs = Some(inputs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::z_view::compute_z_view_cut;
    use std::collections::HashMap;

    const FLOOR: u64 = TaeFlag::Surface.bits() | TaeFlag::Background.bits();
    const ROOF: u64 = TaeFlag::Roof.bits();
    const FOLIAGE: u64 = TaeFlag::Foliage.bits();
    const TRANSPARENT_ROOF: u64 = TaeFlag::Roof.bits() | TaeFlag::Transparent.bits();

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

    fn cache(found: Option<(Ceiling, i32)>) -> MobileCeilingCache {
        MobileCeilingCache { inputs: None,
            found: found.map(|(ceiling, plane_z)| FoundCeiling { ceiling, plane_z, own_land_z: None }) }
    }

    /// The ceiling over a mobile is the one the player's rule finds: a floor above on its cell, the lowest of the roof
    /// on the next cell, and nothing in the open or under foliage and see-through roofs.
    #[test]
    fn a_mobile_takes_the_players_ceiling() {
        let room = Cells::default().with(10, 10, FLOOR, 20).with(10, 10, FLOOR, 40).with(11, 11, ROOF, 60)
            .with(12, 12, ROOF, 66);
        let (floor, plane_z) = mobile_ceiling(10, 10, 20, &room).unwrap();
        assert_eq!((floor.z, floor.x, floor.y, plane_z), (40, 10, 10, 40), "the storey above");
        let (roof, plane_z) = mobile_ceiling(10, 10, 40, &room).unwrap();
        assert_eq!((roof.z, roof.x, roof.y, plane_z), (60, 11, 11, 60), "the roof on the next cell");
        assert_eq!(compute_z_view_cut(10, 10, 40, Some(0), &room).max_z, 60, "as the player's cut");
        // The roof's lowest z, its eaves, under the tile on the next cell.
        let eaves = Cells::default().with(11, 11, ROOF, 70).with(12, 11, ROOF, 66).with(13, 11, ROOF, 62);
        assert_eq!(mobile_ceiling(10, 10, 40, &eaves).map(|(ceiling, plane_z)| (ceiling.z, plane_z)), Some((62, 62)));
        assert_eq!(mobile_ceiling(10, 10, 0, &Cells::default()), None, "in the open");
        let skipped = Cells::default().with(10, 10, FOLIAGE, 30).with(11, 11, TRANSPARENT_ROOF, 30);
        assert_eq!(mobile_ceiling(10, 10, 0, &skipped), None, "under a tree's crown and a see-through roof");
        assert_eq!(mobile_ceiling(10, 10, 0, &Cells::default().with(10, 10, FLOOR, 14)), None, "no higher than 14");
    }

    /// A low cap is raised to the mobile's z + 16, as the player's cut is: a floor at z + 15 lies under the plane.
    #[test]
    fn a_low_ceiling_plane_is_raised_to_the_headroom() {
        let cells = Cells::default().with(10, 10, FLOOR, 15);
        let (floor, plane_z) = mobile_ceiling(10, 10, 0, &cells).unwrap();
        assert_eq!((floor.z, plane_z), (15, PLAYER_VIEW_HEADROOM));
    }

    /// The ceiling is handed to the shader only while the player's cut draws it: in the open the roof over a mobile in
    /// a house is drawn; with the player under the same roof, or on the floor below the storey over the mobile, it is
    /// not, and the mobile draws whole.
    #[test]
    fn only_a_drawn_ceiling_orders_the_pixels_above_it() {
        let house = Cells::default().with(10, 10, FLOOR, 20).with(11, 11, ROOF, 45).with(5, 5, FLOOR, 20)
            .with(6, 6, ROOF, 45);
        let mobile = cache(mobile_ceiling(10, 10, 20, &house));
        assert_eq!(mobile.drawn_plane_z(&ZViewCut::NONE), Some(45), "the player outside, without the z-view");
        let outside = compute_z_view_cut(0, 0, 0, Some(0), &house);
        assert_eq!(mobile.drawn_plane_z(&outside), Some(45), "the player outside");
        let inside = compute_z_view_cut(5, 5, 20, Some(0), &house);
        assert!(inside.hide_roofs);
        assert_eq!(mobile.drawn_plane_z(&inside), None, "the player under the roof: the roofs go");
        let storeys = Cells::default().with(10, 10, FLOOR, 0).with(10, 10, FLOOR, 20).with(5, 5, FLOOR, 0)
            .with(5, 5, FLOOR, 20);
        let downstairs = cache(mobile_ceiling(10, 10, 0, &storeys));
        assert_eq!(downstairs.drawn_plane_z(&ZViewCut::NONE), Some(20));
        assert_eq!(downstairs.drawn_plane_z(&compute_z_view_cut(5, 5, 0, Some(0), &storeys)), None,
            "the player downstairs too: the storey above goes");
        assert_eq!(cache(None).drawn_plane_z(&ZViewCut::NONE), None);
    }
}
