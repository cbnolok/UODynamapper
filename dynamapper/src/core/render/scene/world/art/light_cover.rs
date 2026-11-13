//! Whether a static light is hidden by what is drawn over it: the one rule every style applies before it draws a
//! light, to Classic and UOD-Painterly's light.mul decals and to KR-like and Enhanced's light sprites, and so to the
//! land and static local lights they feed.
//!
//! The rule is ClassicUO's (`GameScene.AddLight`, a Classic reference): a light is not added when the diagonal
//! neighbour in front of it, the cell (x+1, y+1) the camera looks across to reach it, holds a drawn, opaque static
//! whose z is at least `LIGHT_COVER_MIN_RISE` above the light's and below the view's z limit. A roof or an upper floor
//! over a room covers the room's lights; a light on an outer wall facing the camera, an open-air light and a light on
//! top of a tall item keep theirs, since their own cell and the floor they stand on never count.
//!
//! Neither the Enhanced nor the KR client occludes its light sprites: both draw them with no depth test into a light
//! target whose depth they clear (see `CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`). Dynamapper hides covered lights in
//! every style because, without roof hiding, their glows would otherwise show on the roofs that cover them.
//!
//! The rule takes the view's z limit (`max_z`) so a future z-view limit composes with it: a static the limit stops
//! drawing no longer covers, and its lights come back; a light at or above the limit is hidden with its item.

use crate::core::render::scene::world::land::MAP_STORAGE_BLOCK_TILE_DIM;
use crate::core::statics::LazyStaticsStore;
use udd_assets::tilemeta::TileMetaPackage;
use uocf::enhanced::tileart::TaeFlag;

/// How far above a light a static must start to cover it, in z units: ClassicUO's `obj.Z + 5`. The floor a light
/// stands on and the wall it hangs on start at or below the light, so they never cover it.
pub const LIGHT_COVER_MIN_RISE: i32 = 5;

/// The z limit while there is no z-view limit: every static is drawn.
pub const NO_Z_VIEW_LIMIT: i32 = i32::MAX;

/// Statics that never cover a light: the see-through (tiledata Transparent) and the never drawn (NoDraw).
const NON_COVERING_FLAGS: u64 = TaeFlag::Transparent.bits() | TaeFlag::NoDraw.bits();

/// The cell whose statics can cover a light on (`tile_x`, `tile_y`): the diagonal neighbour toward the camera.
pub fn light_cover_cell(tile_x: u32, tile_y: u32) -> (u32, u32) {
    (tile_x + 1, tile_y + 1)
}

/// Whether a static with tile flags `flags` at `static_z` covers a light at `light_z`, under the z limit `max_z`.
pub fn static_covers_light(flags: u64, static_z: i8, light_z: i8, max_z: i32) -> bool {
    let static_z = i32::from(static_z);
    flags & NON_COVERING_FLAGS == 0 && static_z >= i32::from(light_z) + LIGHT_COVER_MIN_RISE && static_z < max_z
}

/// Whether a light at `light_z` is hidden: it is at or above the z limit `max_z`, as its item is then not drawn, or
/// a static of its cover cell (`light_cover_cell`), given as (tile flags, z), covers it.
pub fn light_is_hidden(light_z: i8, cover_cell: impl IntoIterator<Item = (u64, i8)>, max_z: i32) -> bool {
    i32::from(light_z) >= max_z
        || cover_cell.into_iter().any(|(flags, static_z)| static_covers_light(flags, static_z, light_z, max_z))
}

/// `light_is_hidden` for a light on (`tile_x`, `tile_y`) at `light_z`, reading its cover cell from the statics
/// store. Multis are not expanded: their parts do not cover lights yet. A cover cell the store cannot read covers
/// nothing.
pub fn static_light_is_hidden(
    statics_store: &mut LazyStaticsStore,
    tilemeta: &TileMetaPackage,
    tile_x: u32,
    tile_y: u32,
    light_z: i8,
    max_z: i32,
) -> bool {
    let (cover_x, cover_y) = light_cover_cell(tile_x, tile_y);
    let (block_x, block_y) = (cover_x / MAP_STORAGE_BLOCK_TILE_DIM, cover_y / MAP_STORAGE_BLOCK_TILE_DIM);
    let (offset_x, offset_y) =
        ((cover_x % MAP_STORAGE_BLOCK_TILE_DIM) as u8, (cover_y % MAP_STORAGE_BLOCK_TILE_DIM) as u8);
    let tiles = statics_store.block_tiles(block_x, block_y).unwrap_or(&[]);
    let cover_cell = tiles
        .iter()
        .filter(|tile| tile.x_offset() == offset_x && tile.y_offset() == offset_y)
        .map(|tile| (tilemeta.item_tile(u32::from(tile.graphic)).map_or(0, |meta| meta.flags), tile.z));
    light_is_hidden(light_z, cover_cell, max_z)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE: u64 = TaeFlag::Surface.bits();
    const ROOF: u64 = TaeFlag::Roof.bits();
    const WALL: u64 = TaeFlag::Wall.bits();

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
}
