//! Where a static's quad goes and in what order it draws: the world anchors and bounds of billboards, flat
//! and ground quads, the depth classes, and the draw-order keys and tie levels.

use crate::render::assets::tile_atlas::LandCells;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::platform::collections::HashMap;
use super::static_source::{
    StaticVisualKind, TILE_FLAG_BACKGROUND, TILE_FLAG_BRIDGE, TILE_FLAG_FOLIAGE, TILE_FLAG_MULTI_MOVABLE,
    TILE_FLAG_ROOF, TILE_FLAG_SURFACE, TILE_FLAG_WET, static_tile_is_surface_like,
};
use super::static_batches::SpriteInstance;

pub(super) const ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS: f32 = std::f32::consts::SQRT_2;
pub(crate) const CC_TILE_PIXEL_WIDTH: f32 = 44.0;
// Original legacy art projects four square sprite pixels per raw Z unit.
pub(crate) const STATIC_WORLD_Y_PER_XZ_PIXEL: f32 = UO_Z_TO_WORLD_SCALE / (4.0 * CC_WORLD_XZ_PER_PIXEL);
pub(crate) const CC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / CC_TILE_PIXEL_WIDTH;
pub(crate) const CC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * CC_WORLD_XZ_PER_PIXEL;
pub(super) const STATIC_ART_Y_BIAS: f32 = 0.002;
// Ground quads use rasterized depth and need the same small geometric separation
// from terrain as flat sprite quads to avoid coplanar depth fighting.
pub(super) const GROUND_ART_Y_BIAS: f32 = STATIC_ART_Y_BIAS;
// The original 0x45a080 geometry constructor tests this TileArt flag.
pub(super) const EC_CAMERA_QUAD_FLAG: u64 = uocf::enhanced::tileart::TaeFlag::Rotated.bits();

const EC_STATIC_TILE_TRANSLATION_X: f32 = 0.5;
const EC_STATIC_TILE_TRANSLATION_Z: f32 = 1.5;
// CC billboard anchor: feet-center at the tile's near (south) vertex = cell corner (X+1, Z+1).
//
// A CC static sprite is force-centered horizontally in its bounding box (`classic_static_draw_offset`
// uses `offset_x = -(width/2)`), so its on-screen horizontal center sits exactly at the anchor
// `(world_x, world_z)`. In the diagonal billboard basis (BILLBOARD_RIGHT_XZ = on-screen horizontal),
// a translation `(tx, tz)` displaces the sprite horizontally by `(tx - tz)/sqrt(2)`; horizontal
// alignment with the tile therefore *requires* `tx == tz` (the tile's screen-horizontal center lies
// on the line `X - Z = const`). The verified surface-like flat-quad path already uses a symmetric
// `(0.5, 0.5)`. The old shared EC value `(0.5, 1.5)` — bundled in underived (commit a538d3b) and only
// ever validated for EC, which carries its own real per-sprite offsets — is asymmetric: it shoves
// every CC billboard ~1/sqrt(2) (~half a tile) sideways from the tile beneath it. `(1.0, 1.0)` is the
// unique symmetric value with the *same* forward/depth component as `(0.5, 1.5)` (both = sqrt(2), the
// near-vertex depth), so it fixes the horizontal drift without disturbing the vertical grounding
// tuned earlier for the same billboards. EC keeps its own value above (its real offsets rely on it).
pub(super) const CC_STATIC_TILE_TRANSLATION_X: f32 = 1.0;
pub(super) const CC_STATIC_TILE_TRANSLATION_Z: f32 = 1.0;
pub(crate) const STATIC_BILLBOARD_RIGHT_XZ: Vec2 =
    Vec2::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);
/// The CC flat art's second axis, on-screen down: main.wesl's `FLAT_QUAD_FORWARD_XZ`.
pub(super) const CC_FLAT_QUAD_FORWARD_XZ: Vec2 =
    Vec2::new(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
/// The unit quad's corners (x, z) in the art batch mesh's vertex order (`build_art_batch_mesh`): corner (x, z) is
/// vertex x + 2 z, the index the art shaders pick a corner's local light with.
pub(super) const ART_QUAD_CORNERS: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StaticBillboardBounds {
    pub local_min_x: f32,
    pub local_max_x: f32,
    pub local_min_y: f32,
    pub local_max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroundQuadBounds {
    pub local_min_x: f32,
    pub local_max_x: f32,
    pub local_min_z: f32,
    pub local_max_z: f32,
}

pub(super) fn static_world_xz_per_pixel(source_pixel_ratio: f32) -> f32 {
    CC_WORLD_XZ_PER_PIXEL * source_pixel_ratio
}

pub(super) fn static_world_y_per_pixel(source_pixel_ratio: f32) -> f32 {
    CC_WORLD_Y_PER_PIXEL * source_pixel_ratio
}

pub(crate) fn resolve_static_billboard_bounds(
    source_pixel_ratio: f32,
    offset_x_pixels: i16,
    offset_y_pixels: i16,
    logical_width: f32,
    logical_height: f32,
) -> StaticBillboardBounds {
    let world_xz_per_pixel = static_world_xz_per_pixel(source_pixel_ratio);
    let world_y_per_pixel = static_world_y_per_pixel(source_pixel_ratio);
    let local_min_x = offset_x_pixels as f32 * world_xz_per_pixel;
    let local_max_x = local_min_x + logical_width * world_xz_per_pixel;
    let local_min_y = -(offset_y_pixels as f32 * world_y_per_pixel);
    let local_max_y = local_min_y + logical_height * world_y_per_pixel;

    StaticBillboardBounds {
        local_min_x,
        local_max_x,
        local_min_y,
        local_max_y,
    }
}

pub(crate) fn resolve_surface_like_ground_quad_bounds() -> GroundQuadBounds {
    GroundQuadBounds {
        local_min_x: 0.0,
        local_max_x: 1.0,
        local_min_z: 0.0,
        local_max_z: 1.0,
    }
}

/// Local quad bounds for a CC surface-like (shoreline water) static rendered flat.
///
/// EC terrain textures are plain square texmaps whose isometric look comes purely from the
/// camera projecting a 1x1 axis-aligned cell — so the EC ground path can use a plain 0..1
/// quad (see `resolve_surface_like_ground_quad_bounds`). A CC static sprite is different: the
/// isometric *diamond* is baked into its pixels (a 44x44 rhombus with transparent corners,
/// like art.mul land art). Laying that diamond on a plain axis-aligned 0..1 quad would project
/// it a second time, collapsing the visible water to a half-size inscribed blob — the reported
/// "too small" symptom. Instead we size the quad in world units from the sprite's real pixel
/// dimensions via `CC_WORLD_XZ_PER_PIXEL` (the same per-pixel scale the billboard path uses)
/// and center it on the origin, its left edge on a whole pixel as the Classic client draws its art (`-(width >> 1)`):
/// an odd-width sprite centred exactly would put every screen pixel on the edge between two texels at 1:1, and which
/// one it showed would follow the sprite's place in its atlas page. Combined with the diagonal `BILLBOARD_RIGHT_XZ` /
/// `FLAT_QUAD_FORWARD_XZ` basis applied in main.wesl, the sprite's diamond vertices then land
/// exactly on the cell corners and seam with the terrain water beneath. A 44px-wide tile maps
/// to a half-extent of `44 * 0.5 * (sqrt(2)/44) = sqrt(2)/2` along each diagonal axis, i.e. the
/// visible diamond spans a full 1x1 world cell.
pub(crate) fn resolve_cc_surface_like_flat_quad_bounds(
    logical_width: f32,
    logical_height: f32,
) -> GroundQuadBounds {
    let local_min_x = -(logical_width * 0.5).floor() * CC_WORLD_XZ_PER_PIXEL;
    let half_height = logical_height * 0.5 * CC_WORLD_XZ_PER_PIXEL;
    GroundQuadBounds {
        local_min_x,
        local_max_x: local_min_x + logical_width * CC_WORLD_XZ_PER_PIXEL,
        local_min_z: -half_height,
        local_max_z: half_height,
    }
}

/// Per-axis world offsets from the cell origin to a CC surface-like flat quad's *center*,
/// for a sprite of the given pixel height.
///
/// Feet-first anchor matching the validated CC billboard convention: the sprite box's
/// bottom-center sits at the cell's near vertex (`+CC_STATIC_TILE_TRANSLATION_X/Z`), and
/// laying the box flat extends it up-screen along `-FLAT_QUAD_FORWARD_XZ`, so the box center
/// sits half the sprite's world-space height back along the forward diagonal — per axis,
/// `CC_STATIC_TILE_TRANSLATION_* - h * CC_WORLD_XZ_PER_PIXEL * (1/sqrt(2)) * 0.5`
/// (= `1 - h/88` at the CC pixel scale). For the 44px plain-water sprites this is exactly the
/// cell center (`0.5`, matching the previously verified water placement); taller sprites (the
/// 45px/89px `wave` sets 0x1fa3-0x1fca, whose crest content is deliberately off-center in the
/// bitmap) get the anchor classic art actually assumes, instead of a hardcoded cell-center
/// that silently assumed `h == 44` and pushed the ~89px breaking waves ~one cell up-screen.
pub(crate) fn cc_surface_like_flat_quad_center_offsets(logical_height: f32) -> (f32, f32) {
    let forward_half_height =
        logical_height * CC_WORLD_XZ_PER_PIXEL * std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    (
        CC_STATIC_TILE_TRANSLATION_X - forward_half_height,
        CC_STATIC_TILE_TRANSLATION_Z - forward_half_height,
    )
}

pub(crate) fn apply_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + EC_STATIC_TILE_TRANSLATION_X,
        world_z + EC_STATIC_TILE_TRANSLATION_Z,
    )
}

pub(crate) fn apply_cc_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + CC_STATIC_TILE_TRANSLATION_X,
        world_z + CC_STATIC_TILE_TRANSLATION_Z,
    )
}

pub(crate) fn surface_like_static_world_anchor(
    visual_kind: StaticVisualKind,
    world_x: f32,
    world_z: f32,
) -> (f32, f32) {
    match visual_kind {
        StaticVisualKind::TexLandEc { .. } | StaticVisualKind::EcSurface { .. } => (world_x, world_z),
        // CC billboards are force-centered in their bbox, so they need a symmetric anchor
        // (see CC_STATIC_TILE_TRANSLATION_*); EC billboards carry their own per-sprite offsets
        // and keep the legacy asymmetric anchor they were validated against.
        StaticVisualKind::CcRegular { .. } | StaticVisualKind::Missing => {
            apply_cc_static_world_anchor_translation(world_x, world_z)
        }
        StaticVisualKind::EcRegular { .. } => {
            apply_static_world_anchor_translation(world_x, world_z)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StaticDepthClass {
    Regular,
    Background,
    Foliage,
    Roof,
    SurfaceLikeFloor,
}

impl StaticDepthClass {
    pub(super) fn encoded(self) -> u32 {
        match self {
            StaticDepthClass::Regular => 0,
            StaticDepthClass::Background => 1,
            StaticDepthClass::Foliage => 2,
            StaticDepthClass::Roof => 3,
            StaticDepthClass::SurfaceLikeFloor => 4,
        }
    }
}

/// Priority z steps in one diagonal row (art/depth.wesl PRIORITY_Z_STEPS_PER_ROW): more than the priority z range, so
/// the nearer row always wins. Keep in sync with the shader.
const PRIORITY_Z_STEPS_PER_ROW: f64 = 264.0;
/// Tie levels in one priority z (art/depth.wesl PRIORITY_TIE_LEVELS_PER_Z). Keep in sync with the shader.
const PRIORITY_TIE_LEVELS_PER_Z: f64 = 9.0;
/// The tie level of the land, the flat quads and the ground art; the upright sprites take the levels above.
pub(super) const FLAT_TIE_LEVEL: u32 = 0;
pub(super) const SPRITE_FIRST_TIE_LEVEL: u32 = 1;
/// `SpriteInstance::flat_quad` of the quads that lie flat over their cell (main.wesl): the CC flat art and the EC item
/// surfaces. Their depth follows the fragment's row at tie level 0 (art/depth.wesl).
pub(super) const FLAT_QUAD_CC_FLAT: u32 = 1;
pub(super) const FLAT_QUAD_EC_SURFACE: u32 = 2;
/// `SpriteInstance::flat_quad` of an authored EC camera quad (main.wesl): a card at the EC art's anchor, constant depth.
pub(super) const FLAT_QUAD_EC_CAMERA: u32 = 3;
/// A cell's draw order anchor from its corner along x and along y (`cell_depth_anchor`; art/depth.wesl
/// CELL_DEPTH_ANCHOR): the CC art's anchor, (1, 1), row + 2.
const CELL_DEPTH_ANCHOR_OFFSET: f32 = 1.0;
/// Ranks of a cell's statics at one priority z that keep their own tie level, in emission order; the later ones
/// share the last and keep their draw order within the chunk batch. Each rank takes two levels, one per x parity of
/// the cell, so 4 ranks fill the 8 sprite levels.
const SPRITE_TIE_CELL_RANKS: u32 = 4;
/// Tie levels of one cell rank: the cell's x parity.
const SPRITE_TIE_LEVELS_PER_RANK: u32 = 2;

/// CC-specific gate for the flat-quad (lie-flat) static render path.
///
/// Only genuine water/shoreline statics should lie flat like the terrain water beneath them.
/// The broader `static_tile_is_surface_like` predicate — reused by the EC land-redirection
/// path — also matches walkable/authored "surface-like" tiles (floors, and roofs such as
/// "slate roof", tiledata flags `0x04006201` = SURFACE | Background, authored SurfaceLike, not
/// WET, sitting at elevated z). Those are upright/elevated CC sprite art and must stay
/// billboards: flattening them laid them as horizontal diamonds at their own elevation,
/// mis-anchored and z-fighting with the other statics stacked in the same cell. Keying purely
/// on the IsWet tiledata flag keeps every real water and "wave" static flat (all carry IsWet,
/// verified against tiledata.mul) while excluding roofs/floors.
pub(super) fn cc_static_tile_renders_flat(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
) -> bool {
    tilemeta.is_some_and(|meta| meta.flags & TILE_FLAG_WET != 0)
}

// A tile-sized dry ground diamond also needs terrain depth, rather than the
// upright sprite's constant painter depth. Roofs, volume and raised/sloped
// surfaces retain their authored billboard path.
pub(super) fn cc_ground_diamond(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>, width: f32, height: f32) -> bool {
    tilemeta.is_some_and(|meta| meta.height == 0
        && meta.flags & (TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE) == (TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE)
        && meta.flags & (TILE_FLAG_WET | TILE_FLAG_ROOF | TILE_FLAG_FOLIAGE | TILE_FLAG_BRIDGE) == 0
        && !meta.is_nodraw())
        && width == CC_TILE_PIXEL_WIDTH && height == CC_TILE_PIXEL_WIDTH
}

// Picking and drawing must make the same residency/height decision for dry diamonds.
pub(crate) fn cc_static_uses_flat_geometry(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>, width: f32, height: f32,
    world_x: f32, world_z: f32, z: i8, atlas: &(impl LandCells + ?Sized),
) -> bool {
    cc_static_tile_renders_flat(tilemeta)
        || (cc_ground_diamond(tilemeta, width, height)
            && [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().all(|(dx, dy)| {
                super::static_surface::map_height(atlas,
                    world_x as u32 + dx, world_z as u32 + dy) == Some(z)
            }))
}

pub(crate) fn ec_static_camera_quad(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> bool {
    tilemeta.is_some_and(|meta| meta.flags & EC_CAMERA_QUAD_FLAG != 0)
}

/// `renders_wet`: `static_tile_renders_wet` for the art mode in use (see `static_tile_is_surface_like`).
pub(crate) fn resolve_static_depth_class(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    renders_wet: bool,
) -> StaticDepthClass {
    let Some(meta) = tilemeta else {
        return StaticDepthClass::Regular;
    };

    if meta.flags & TILE_FLAG_BACKGROUND != 0 {
        return StaticDepthClass::Background;
    }

    if meta.flags & TILE_FLAG_ROOF != 0 {
        return StaticDepthClass::Roof;
    }

    if meta.flags & TILE_FLAG_FOLIAGE != 0 {
        return StaticDepthClass::Foliage;
    }

    // Bridge marks walkable volume (stairs and stacked stone blocks), not a
    // flat floor. Keep it Regular: an upright sprite on the static art bias.
    if static_tile_is_surface_like(tilemeta, renders_wet) && meta.flags & TILE_FLAG_BRIDGE == 0 {
        return StaticDepthClass::SurfaceLikeFloor;
    }

    StaticDepthClass::Regular
}

pub(crate) fn depth_class_y_bias(depth_class: StaticDepthClass) -> f32 {
    match depth_class {
        StaticDepthClass::SurfaceLikeFloor => GROUND_ART_Y_BIAS,
        StaticDepthClass::Regular
        | StaticDepthClass::Background
        | StaticDepthClass::Foliage
        | StaticDepthClass::Roof => STATIC_ART_Y_BIAS,
    }
}

/// ClassicUO's priority z of a static at `tile_z` (Map.cs `AddGameObject`, 2026-11-16 decision): one less for
/// Background, one more for a non-zero tiledata height and one more for MultiMovable. Surface takes none (ClassicUO
/// has that line commented out). It replaces the former effective priority height (the full height, 10 for a flat
/// Regular item, half for a Bridge), under which a tall static one row behind covered a short one in front.
pub(crate) fn resolve_priority_z_units(tile_z: i8, tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> f32 {
    let Some(meta) = tilemeta else {
        return tile_z as f32;
    };
    let mut priority_z = i32::from(tile_z);
    if meta.flags & TILE_FLAG_BACKGROUND != 0 {
        priority_z -= 1;
    }
    if meta.height != 0 {
        priority_z += 1;
    }
    if meta.flags & TILE_FLAG_MULTI_MOVABLE != 0 {
        priority_z += 1;
    }
    priority_z as f32
}

/// The draw order's logical distance (art/depth.wesl): the diagonal row, then the priority z, then the tie level,
/// each step smaller than the whole range of the next finer one, so the nearer row always wins. The shaders project
/// the same quantity as a point's logical height; f64 keeps the tie levels at any map position.
pub(crate) fn logical_draw_distance(row: f64, priority_z: f64, tie_level: u32) -> f64 {
    row + (priority_z + f64::from(tie_level) / PRIORITY_TIE_LEVELS_PER_Z) / PRIORITY_Z_STEPS_PER_ROW
}

/// An instance's sort key: `logical_draw_distance` of its anchor and priority z, before its tie level.
pub(crate) fn static_depth_key(tile_x: f32, tile_y: f32, priority_z_units: f32) -> f64 {
    logical_draw_distance(f64::from(tile_x) + f64::from(tile_y), f64::from(priority_z_units), 0)
}

/// The tie levels of upright sprites (art/depth.wesl): statics of one cell and priority z take the levels in their
/// emission order, the later one nearer, as ClassicUO draws a later static of the same priority over an earlier one;
/// equal statics of neighbouring cells in one row differ by the cell's x parity, so whichever chunk batch draws first,
/// the same one wins. Assigned over the whole assembled set, as cached chunk batches are concatenated in stable
/// (gy, gx, scale) order. Replaces the earlier-wins tie-break ordinal (an earlier static drew nearer).
pub(super) fn assign_sprite_tie_levels(instances: &mut [SpriteInstance]) {
    let mut next_ranks = HashMap::<(u32, u32, u32), u32>::with_capacity(instances.len());

    for instance in instances.iter_mut() {
        if matches!(instance.flat_quad, FLAT_QUAD_CC_FLAT | FLAT_QUAD_EC_SURFACE) {
            instance.sort_bias_ordinal = FLAT_TIE_LEVEL;
            continue;
        }
        let cell = (instance.tile_x.to_bits(), instance.tile_y.to_bits(), instance.priority_z_units.to_bits());
        let rank = next_ranks.entry(cell).or_insert(0);
        let parity = (instance.tile_x.floor() as i64).rem_euclid(2) as u32;
        instance.sort_bias_ordinal =
            SPRITE_FIRST_TIE_LEVEL + SPRITE_TIE_LEVELS_PER_RANK * (*rank).min(SPRITE_TIE_CELL_RANKS - 1) + parity;
        *rank = rank.saturating_add(1);
    }
}

/// The draw order's anchor (art/depth.wesl) of the cell whose corner is at `cell_x`, `cell_z`: row + 2, the upright
/// sprites' convention, for every fragment of the land, a flat quad or a ground art over the cell. A cell's surfaces
/// then take one depth, as ClassicUO draws a cell's land and statics as whole objects in its row.
pub(crate) fn cell_depth_anchor(cell_x: f32, cell_z: f32) -> (f32, f32) {
    (cell_x + CELL_DEPTH_ANCHOR_OFFSET, cell_z + CELL_DEPTH_ANCHOR_OFFSET)
}

/// The draw order's anchor (art/depth.wesl) of a billboard-path instance laid out at `world_x`, `world_z`: an
/// authored EC camera quad stands as a card at the EC art's anchor (ec_item_quad.wesl), so it takes that anchor's row,
/// as an upright EC sprite does; at its cell's corner it would lie two rows behind its own cell's land and lose to it.
/// A flat quad, laid out from its cell's corner, takes its cell's anchor. An upright sprite keeps its layout position,
/// already its anchor.
pub(super) fn sprite_depth_anchor(flat_quad: u32, world_x: f32, world_z: f32) -> (f32, f32) {
    match flat_quad {
        FLAT_QUAD_EC_CAMERA => apply_static_world_anchor_translation(world_x, world_z),
        FLAT_QUAD_CC_FLAT | FLAT_QUAD_EC_SURFACE => cell_depth_anchor(world_x, world_z),
        _ => (world_x, world_z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_flat_quads_start_on_a_whole_pixel() {
        for (width, left_pixels) in [(44.0, -22.0), (45.0, -22.0), (47.0, -23.0)] {
            let bounds = resolve_cc_surface_like_flat_quad_bounds(width, 44.0);
            assert!((bounds.local_min_x / CC_WORLD_XZ_PER_PIXEL - left_pixels).abs() < 1e-4, "width {width}");
            assert!(((bounds.local_max_x - bounds.local_min_x) / CC_WORLD_XZ_PER_PIXEL - width).abs() < 1e-4,
                "width {width}");
        }
    }
}
