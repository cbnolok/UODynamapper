use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::{LandTextureSource, Settings};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::camera::{PlayerCamera, RenderZoom};
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::render::scene::world::land::tile_atlas::{AtlasCells, TileAtlas};
use crate::core::multis::{
    expanded_multi_part_z, MultiDefinitionsRes, MultiPart, PlacedMulti, PlacedMultisRes,
};
use crate::core::render::scene::world::caves::{is_buried_floor, CaveMap, CaveStatic, CavesRes, PlayerCave};
use crate::core::render::scene::world::z_view::{PackageLand, ZViewCut, ZViewLimit};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::texture_cache::art::{GroundArtPageAtlas, ResolvedArtSprite, SpriteArtPageAtlas};
use crate::core::texture_cache::art::ArtPageKey;
use crate::core::texture_cache::ec_item_surface::KR_ITEM_RECORD_BASE;
use crate::core::uo_files_loader::{
    KrItemPackagesRes, TexLandKrPackageRes,
    MapPlanesRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes, TexAuxEcPackageRes,
};
use crate::prelude::*;
use super::client_grunge::STATIC_FLAG_CLIENT_GRUNGE;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeSet, HashMap, HashSet};

const CLASSIC_STATIC_ART_ID_OFFSET: u32 = 0x4000;
const ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS: f32 = std::f32::consts::SQRT_2;
pub(super) const CC_TILE_PIXEL_WIDTH: f32 = 44.0;
// Original legacy art projects four square sprite pixels per raw Z unit.
pub(super) const STATIC_WORLD_Y_PER_XZ_PIXEL: f32 = UO_Z_TO_WORLD_SCALE / (4.0 * CC_WORLD_XZ_PER_PIXEL);
pub(super) const CC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / CC_TILE_PIXEL_WIDTH;
pub(super) const CC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * CC_WORLD_XZ_PER_PIXEL;
const STATIC_ART_Y_BIAS: f32 = 0.002;
// Ground quads use rasterized depth and need the same small geometric separation
// from terrain as flat sprite quads to avoid coplanar depth fighting.
const GROUND_ART_Y_BIAS: f32 = STATIC_ART_Y_BIAS;
// The original 0x45a080 geometry constructor tests this TileArt flag.
const EC_CAMERA_QUAD_FLAG: u64 = uocf::enhanced::tileart::TaeFlag::Rotated.bits();

// The KR client's static shadow (research P3.4; art/client_shadow.wesl). `SpriteInstance::client_shadow_flags` bits,
// kept in sync with client_shadow.wesl.
pub(crate) const CLIENT_SHADOW_FLAG_CASTS: u32 = 1;
pub(crate) const CLIENT_SHADOW_FLAG_WALL: u32 = 1 << 1;
/// Tileart flags that cast no shadow: the client tests 0x10004200 (Surface, Roof and its bit 14, which is EC's
/// Rotated; our tilemeta keeps EC's 64-bit layout, where bit 14 is the classic ArticleA) and EC adds NoShadow.
const CLIENT_SHADOW_EXCLUDED_FLAGS: u64 = uocf::enhanced::tileart::TaeFlag::Surface.bits()
    | uocf::enhanced::tileart::TaeFlag::Roof.bits()
    | uocf::enhanced::tileart::TaeFlag::Rotated.bits()
    | uocf::enhanced::tileart::TaeFlag::NoShadow.bits();
/// The client's ground under a caster (0x00587770) takes a Surface or Bridge top at most this far above the caster.
const CLIENT_SHADOW_GROUND_REACH_Z: i32 = 4;
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
const CC_STATIC_TILE_TRANSLATION_X: f32 = 1.0;
const CC_STATIC_TILE_TRANSLATION_Z: f32 = 1.0;
const CLASSIC_WATER_LAND_TILE_ID: u32 = 168;
const EC_WATER_BASE_LAYER_INDEX: u32 = 0;
const STATIC_CHUNK_CACHE_HYSTERESIS_TICKS: u64 = 30;
pub(super) const STATIC_BILLBOARD_RIGHT_XZ: Vec2 =
    Vec2::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);
/// The CC flat art's second axis, on-screen down: main.wesl's `FLAT_QUAD_FORWARD_XZ`.
const CC_FLAT_QUAD_FORWARD_XZ: Vec2 =
    Vec2::new(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
/// The unit quad's corners (x, z) in the art batch mesh's vertex order (`build_art_batch_mesh`): corner (x, z) is
/// vertex x + 2 z, the index the art shaders pick a corner's local light with.
const ART_QUAD_CORNERS: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)];
/// The largest 8-bit channel value, which a packed local light channel spans (`unpack4x8unorm` in the shaders).
const LOCAL_LIGHT_CHANNEL_MAX: f32 = u8::MAX as f32;
/// Bits per packed local light channel.
const LOCAL_LIGHT_CHANNEL_BITS: u32 = u8::BITS;
/// `SpriteInstance::is_wet_flags` bit: the upright card shows flat art, a CC ground diamond above the terrain (a floor
/// or a flat roof, `cc_ground_diamond`), which catches the static local lights evenly. Kept in sync with
/// art/shading.wesl.
pub(crate) const STATIC_FLAG_FLAT_ART: u32 = 1 << 11;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is part of a cave drawn over the map
/// (`CaveStatic::Lifted`), drawn in front of the land and statics around it. Kept in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAVE: u32 = 1 << 12;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is outside the player's cave, drawn
/// darkened and partly desaturated (art/depth.wesl `cave_outside_tone`, caves.rs `CaveSurroundings::Dimmed`). Kept
/// in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAVE_OUTSIDE: u32 = 1 << 15;
/// `SpriteInstance::is_wet_flags` bit: the static is a wall with a floor laid on its top in the farther row, a Surface
/// or Bridge in the cell west or north of it whose top is the wall's (`wall_cap_meets_floor_flag`): the perspective
/// camera orders its cap in that row (art/depth.wesl `perspective_cap_row_shift`). Kept in sync with art/depth.wesl.
pub(crate) const STATIC_FLAG_CAP_MEETS_FLOOR: u32 = 1 << 24;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static continues the land
/// (`static_continues_land`) or is foliage, so it takes the land's colour grading, KR-like's chroma knee included; the
/// other statics keep their own colour (`land_grade_flag`). Kept in sync with art/shading.wesl.
pub(crate) const STATIC_FLAG_LAND_GRADE: u32 = 1 << 25;

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

fn static_world_xz_per_pixel(source_pixel_ratio: f32) -> f32 {
    CC_WORLD_XZ_PER_PIXEL * source_pixel_ratio
}

fn static_world_y_per_pixel(source_pixel_ratio: f32) -> f32 {
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
/// and center it on the origin. Combined with the diagonal `BILLBOARD_RIGHT_XZ` /
/// `FLAT_QUAD_FORWARD_XZ` basis applied in main.wesl, the sprite's diamond vertices then land
/// exactly on the cell corners and seam with the terrain water beneath. A 44px-wide tile maps
/// to a half-extent of `44 * 0.5 * (sqrt(2)/44) = sqrt(2)/2` along each diagonal axis, i.e. the
/// visible diamond spans a full 1x1 world cell.
pub(super) fn resolve_cc_surface_like_flat_quad_bounds(
    logical_width: f32,
    logical_height: f32,
) -> GroundQuadBounds {
    let half_width = logical_width * 0.5 * CC_WORLD_XZ_PER_PIXEL;
    let half_height = logical_height * 0.5 * CC_WORLD_XZ_PER_PIXEL;
    GroundQuadBounds {
        local_min_x: -half_width,
        local_max_x: half_width,
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
pub(super) fn cc_surface_like_flat_quad_center_offsets(logical_height: f32) -> (f32, f32) {
    let forward_half_height =
        logical_height * CC_WORLD_XZ_PER_PIXEL * std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    (
        CC_STATIC_TILE_TRANSLATION_X - forward_half_height,
        CC_STATIC_TILE_TRANSLATION_Z - forward_half_height,
    )
}

pub(super) fn apply_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + EC_STATIC_TILE_TRANSLATION_X,
        world_z + EC_STATIC_TILE_TRANSLATION_Z,
    )
}

pub(super) fn apply_cc_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + CC_STATIC_TILE_TRANSLATION_X,
        world_z + CC_STATIC_TILE_TRANSLATION_Z,
    )
}

pub(super) fn surface_like_static_world_anchor(
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum StaticVisualKind {
    CcRegular { art_id: u32, fallback_art_id: u32 },
    EcRegular { art_id: u32 },
    EcSurface { art_id: u32 },
    TexLandEc { resolution: SurfaceLikeTexLandEcResolution },
    Missing,
}

pub(super) fn classic_static_visual_kind(graphic: u16,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> StaticVisualKind {
    StaticVisualKind::CcRegular {
        art_id: u32::from(graphic) + CLASSIC_STATIC_ART_ID_OFFSET,
        fallback_art_id: tilemeta.map_or(u32::from(graphic), |m| m.cc_texture_id)
            .saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
    }
}

pub(super) fn static_visual_renders_wet(kind: StaticVisualKind,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    package: Option<&udd_assets::tilemeta::TileMetaPackage>, graphic: u32) -> bool {
    match kind {
        StaticVisualKind::Missing => false,
        StaticVisualKind::CcRegular { .. } =>
            static_tile_renders_wet(tilemeta, package, graphic, Some(ClientTextureSource::Cc)),
        _ => static_tile_renders_wet(tilemeta, package, graphic, Some(ClientTextureSource::Ec)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StaticRenderTile {
    pub graphic: u16,
    pub world_x: f32,
    pub world_z: f32,
    pub z: i8,
    pub hue: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StaticPickTile {
    pub tile: StaticRenderTile,
    pub is_wet: bool,
}

/// The parts `parts` of the multi `multi` as the renderer draws them, around its anchor, with its hue.
fn collect_multi_render_tiles(multi: &PlacedMulti, parts: &[MultiPart], output: &mut Vec<StaticRenderTile>) {
    output.clear();
    output.extend(parts.iter().map(|part| StaticRenderTile {
        graphic: part.item_id,
        world_x: (i64::from(multi.x) + i64::from(part.x)) as f32,
        world_z: (i64::from(multi.y) + i64::from(part.y)) as f32,
        z: expanded_multi_part_z(multi.z, part.z),
        hue: multi.hue,
    }));
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
    fn encoded(self) -> u32 {
        match self {
            StaticDepthClass::Regular => 0,
            StaticDepthClass::Background => 1,
            StaticDepthClass::Foliage => 2,
            StaticDepthClass::Roof => 3,
            StaticDepthClass::SurfaceLikeFloor => 4,
        }
    }
}

const TILE_FLAG_BACKGROUND: u64 = 0x01;
pub(super) const TILE_FLAG_SURFACE: u64 = 0x200;
const TILE_FLAG_BRIDGE: u64 = 0x400;
const TILE_FLAG_FOLIAGE: u64 = 0x20_000;
const TILE_FLAG_ROOF: u64 = 0x1000_0000;
/// NoShoot: the built floors (`static_continues_land`).
const TILE_FLAG_NO_SHOOT: u64 = 0x2000;
/// IsWet flag — tile has tidal/water properties (matches tiledata.mul bit 0x80). An object property;
/// whether the art shader applies the animated sin/cos UV distortion is `static_tile_renders_wet`.
const TILE_FLAG_WET: u64 = 0x80;
const STATIC_HUE_FLAG_APPLY: u32 = 1;
const STATIC_HUE_FLAG_EC_SPRITE: u32 = 2;
/// MultiMovable (ClassicUO TileFlag 0x10000000000): one more priority z, as ClassicUO's `AddGameObject` gives it.
const TILE_FLAG_MULTI_MOVABLE: u64 = 0x100_0000_0000;
/// Priority z steps in one diagonal row (art/depth.wesl PRIORITY_Z_STEPS_PER_ROW): more than the priority z range, so
/// the nearer row always wins. Keep in sync with the shader.
const PRIORITY_Z_STEPS_PER_ROW: f64 = 264.0;
/// Tie levels in one priority z (art/depth.wesl PRIORITY_TIE_LEVELS_PER_Z). Keep in sync with the shader.
const PRIORITY_TIE_LEVELS_PER_Z: f64 = 9.0;
/// The tie level of the land, the flat quads and the ground art; the upright sprites take the levels above.
const FLAT_TIE_LEVEL: u32 = 0;
const SPRITE_FIRST_TIE_LEVEL: u32 = 1;
/// `SpriteInstance::flat_quad` of the quads that lie flat over their cell (main.wesl): the CC flat art and the EC item
/// surfaces. Their depth follows the fragment's row at tie level 0 (art/depth.wesl).
const FLAT_QUAD_CC_FLAT: u32 = 1;
const FLAT_QUAD_EC_SURFACE: u32 = 2;
/// `SpriteInstance::flat_quad` of an authored EC camera quad (main.wesl): a card at the EC art's anchor, constant depth.
const FLAT_QUAD_EC_CAMERA: u32 = 3;
/// A cell's draw order anchor from its corner along x and along y (`cell_depth_anchor`; art/depth.wesl
/// CELL_DEPTH_ANCHOR): the CC art's anchor, (1, 1), row + 2.
const CELL_DEPTH_ANCHOR_OFFSET: f32 = 1.0;
/// Ranks of a cell's statics at one priority z that keep their own tie level, in emission order; the later ones
/// share the last and keep their draw order within the chunk batch. Each rank takes two levels, one per x parity of
/// the cell, so 4 ranks fill the 8 sprite levels.
const SPRITE_TIE_CELL_RANKS: u32 = 4;
/// Tie levels of one cell rank: the cell's x parity.
const SPRITE_TIE_LEVELS_PER_RANK: u32 = 2;

/// Surface-like: authored SurfaceLike, the Surface flag, or a static that renders wet.
/// `renders_wet` is `static_tile_renders_wet` for the art mode in use, so Classic mode keeps the IsWet
/// flag and Enhanced mode follows the water shader for items that have an EC material.
pub(crate) fn static_tile_is_surface_like(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    renders_wet: bool,
) -> bool {
    let Some(meta) = tilemeta else {
        return false;
    };

    meta.is_surface_like() || meta.flags & TILE_FLAG_SURFACE != 0 || renders_wet
}

fn static_tile_uses_tex_land_ec_surface_path(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    renders_wet: bool,
) -> bool {
    static_tile_is_surface_like(tilemeta, renders_wet)
}

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
fn cc_static_tile_renders_flat(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
) -> bool {
    tilemeta.is_some_and(|meta| meta.flags & TILE_FLAG_WET != 0)
}

// A tile-sized dry ground diamond also needs terrain depth, rather than the
// upright sprite's constant painter depth. Roofs, volume and raised/sloped
// surfaces retain their authored billboard path.
fn cc_ground_diamond(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>, width: f32, height: f32) -> bool {
    tilemeta.is_some_and(|meta| meta.height == 0
        && meta.flags & (TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE) == (TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE)
        && meta.flags & (TILE_FLAG_WET | TILE_FLAG_ROOF | TILE_FLAG_FOLIAGE | TILE_FLAG_BRIDGE) == 0
        && !meta.is_nodraw())
        && width == CC_TILE_PIXEL_WIDTH && height == CC_TILE_PIXEL_WIDTH
}

// Picking and drawing must make the same residency/height decision for dry diamonds.
pub(super) fn cc_static_uses_flat_geometry(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>, width: f32, height: f32,
    world_x: f32, world_z: f32, z: i8, atlas: &TileAtlas,
) -> bool {
    cc_static_tile_renders_flat(tilemeta)
        || (cc_ground_diamond(tilemeta, width, height)
            && [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().all(|(dx, dy)| {
                super::static_surface::map_height(atlas,
                    world_x as u32 + dx, world_z as u32 + dy) == Some(z)
            }))
}

pub(super) fn ec_static_camera_quad(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> bool {
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
fn assign_sprite_tie_levels(instances: &mut [SpriteInstance]) {
    let mut next_ranks = HashMap::<(u32, u32, u32), u32>::new();

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

fn resolve_surface_like_tex_land_ec_slot_id(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> Option<SurfaceLikeTexLandEcResolution> {
    let meta = tilemeta?;
    // This path is Enhanced-only: wetness is the water shader for items with an EC material.
    let renders_wet = static_tile_renders_wet(tilemeta, tilemeta_package, tile_id, Some(ClientTextureSource::Ec));
    if !static_tile_uses_tex_land_ec_surface_path(tilemeta, renders_wet) {
        return None;
    }

    let package = tex_land_ec?;

    // Match the original item material's sampler identities to the terrain
    // material graph, rather than treating an item graphic as a land tile id.
    if let Some(tilemeta_package) = tilemeta_package {
        let refs = tilemeta_package.item_texture_refs(tile_id);
        if let Some(material) = tilemeta_package.item_material(tile_id, 0) {
            if let Some(payload) = package.source_liquid_payload_for_item(material, refs) {
                if let Some(base) = package.resolve_material_render_layers(payload)[0] {
                    if let Some(runtime_slot_id) = base.runtime_slot_id {
                        return Some(SurfaceLikeTexLandEcResolution {
                            runtime_slot_id,
                            texture_repetition: base.texture_repetition,
                            liquid_payload: Some(payload),
                            kingdom_reborn: false,
                        });
                    }
                }
            }
        }
    }

    let main_ec_texture_id = tilemeta_package
        .and_then(|package| surface_like_visible_ec_texture_ref(tile_id, renders_wet, package))
        .map(|texture_ref| texture_ref.texture_id)
        .or_else(|| surface_like_legacy_ec_texture_id_fallback(meta, renders_wet));
    let Some(main_ec_texture_id) = main_ec_texture_id else {
        if let Some(resolution) = wet_surface_like_water_resolution(renders_wet, package) {
            return Some(resolution);
        }

        return package
            .resolve_runtime_slot_id(meta.cc_texture_id)
            .map(|runtime_slot_id| {
                surface_like_tex_land_ec_resolution(package, runtime_slot_id, None)
            });
    };

    if package.present_slot(main_ec_texture_id).is_some() {
        return Some(surface_like_tex_land_ec_resolution(
            package,
            main_ec_texture_id,
            Some(main_ec_texture_id),
        ));
    }

    let mut canonical_slots = BTreeSet::new();
    let mut alias_slots = BTreeSet::new();
    for record in package
        .terrain_provenance()
        .iter()
        .filter(|record| record.selected_texture_id == main_ec_texture_id)
    {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            canonical_slots.insert(record.canonical_slot_id);
        }

        if record.alias_slot_id != 0
            && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            alias_slots.insert(record.alias_slot_id);
        }
    }

    if canonical_slots.len() == 1 {
        return canonical_slots.into_iter().next().map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        });
    }

    if canonical_slots.is_empty() && alias_slots.len() == 1 {
        return alias_slots.into_iter().next().map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        });
    }

    if let Some(resolution) = wet_surface_like_water_resolution(renders_wet, package) {
        return Some(resolution);
    }

    package
        .resolve_runtime_slot_id(meta.cc_texture_id)
        .map(|runtime_slot_id| {
            surface_like_tex_land_ec_resolution(package, runtime_slot_id, Some(main_ec_texture_id))
        })
}

fn surface_like_visible_ec_texture_ref<'a>(
    tile_id: u32,
    renders_wet: bool,
    package: &'a udd_assets::tilemeta::TileMetaPackage,
) -> Option<&'a udd_assets::tilemeta::TileMetaItemTextureRef> {
    if !renders_wet {
        return package.main_ec_texture_ref(tile_id);
    }

    choose_visible_surface_texture_ref(package.item_texture_refs(tile_id), tile_id)
}

fn choose_visible_surface_texture_ref(
    refs: &[udd_assets::tilemeta::TileMetaItemTextureRef],
    tile_id: u32,
) -> Option<&udd_assets::tilemeta::TileMetaItemTextureRef> {
    visible_surface_texture_ref_candidates(refs, tile_id)
        .find(|texture_ref| texture_ref.is_primary_selected())
        .or_else(|| visible_surface_texture_ref_candidates(refs, tile_id).next())
}

fn visible_surface_texture_ref_candidates<'a>(
    refs: &'a [udd_assets::tilemeta::TileMetaItemTextureRef],
    tile_id: u32,
) -> impl Iterator<Item = &'a udd_assets::tilemeta::TileMetaItemTextureRef> {
    refs.iter().filter(move |texture_ref| {
        !texture_ref.is_auxiliary()
            && texture_ref.texture_id != tile_id
            && matches!(
                texture_ref.stable_role(),
                udd_assets::tilemeta::EcMaterialStableRole::Base
                    | udd_assets::tilemeta::EcMaterialStableRole::SecondaryBase
            )
    })
}

fn surface_like_legacy_ec_texture_id_fallback(
    tilemeta: &udd_assets::tilemeta::TileMetaItemTile,
    renders_wet: bool,
) -> Option<u32> {
    if tilemeta.ec_texture_id == 0 || renders_wet {
        return None;
    }

    Some(tilemeta.ec_texture_id)
}

/// The classic-water land fallback for a surface-like static that renders wet in Enhanced mode
/// (`static_tile_renders_wet`), not merely one with the IsWet property.
fn wet_surface_like_water_resolution(
    renders_wet: bool,
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
) -> Option<SurfaceLikeTexLandEcResolution> {
    if !renders_wet {
        return None;
    }

    if let Some(layer) =
        package.resolve_material_layer_slot(CLASSIC_WATER_LAND_TILE_ID, EC_WATER_BASE_LAYER_INDEX)
    {
        if let Some(runtime_slot_id) = layer.runtime_slot_id {
            return Some(SurfaceLikeTexLandEcResolution {
                runtime_slot_id,
                texture_repetition: valid_texture_repetition(layer.texture_repetition),
                liquid_payload: None,
                kingdom_reborn: false,
            });
        }
    }

    package
        .resolve_runtime_slot_id(CLASSIC_WATER_LAND_TILE_ID)
        .map(|runtime_slot_id| surface_like_tex_land_ec_resolution(package, runtime_slot_id, None))
}

fn valid_texture_repetition(repetition: f32) -> f32 {
    if repetition.is_finite() && repetition > 0.0 {
        repetition
    } else {
        1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SurfaceLikeTexLandEcResolution {
    pub(super) runtime_slot_id: u32,
    texture_repetition: f32,
    liquid_payload: Option<u32>,
    /// A slot of the Kingdom Reborn land package (`kingdom_reborn_static_visual_kind`), not of the Enhanced one.
    pub(super) kingdom_reborn: bool,
}

fn surface_like_tex_land_ec_resolution(
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
    runtime_slot_id: u32,
    selected_texture_id: Option<u32>,
) -> SurfaceLikeTexLandEcResolution {
    let texture_repetition = selected_texture_id
        .and_then(|texture_id| {
            package
                .terrain_provenance()
                .iter()
                .filter(|record| record.selected_texture_id == texture_id)
                .find(|record| {
                    provenance_record_matches_runtime_slot(package, record, runtime_slot_id)
                })
                .map(|record| record.selected_texture_repetition)
        })
        .filter(|repetition| repetition.is_finite() && *repetition > 0.0)
        .or_else(|| {
            package.present_slot(runtime_slot_id).map(|slot| {
                let width = f32::from(slot.width).max(1.0);
                width / CC_TILE_PIXEL_WIDTH
            })
        })
        .unwrap_or(1.0);

    SurfaceLikeTexLandEcResolution {
        runtime_slot_id,
        texture_repetition,
        liquid_payload: None,
        kingdom_reborn: false,
    }
}

fn surface_like_ground_material_payload(liquid_payload: Option<u32>) -> (u32, u32) {
    liquid_payload.map_or((0, 0), |payload| (payload, 1))
}

fn provenance_record_matches_runtime_slot(
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
    record: &udd_assets::tex_land_ec::TexLandEcTerrainProvenanceRecord,
    runtime_slot_id: u32,
) -> bool {
    if record.canonical_slot_id != 0
        && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
        && package.present_slot(record.canonical_slot_id).is_some()
    {
        return record.canonical_slot_id == runtime_slot_id;
    }

    if record.alias_slot_id != 0
        && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
        && package.present_slot(record.alias_slot_id).is_some()
    {
        return record.alias_slot_id == runtime_slot_id;
    }

    false
}

#[cfg(test)]
fn resolve_static_visual_kind(
    art_source: ClientTextureSource,
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> StaticVisualKind {
    match art_source {
        ClientTextureSource::Cc => {
            classic_static_visual_kind(tile_graphic, tilemeta)
        }
        ClientTextureSource::Ec => resolve_ec_static_visual_kind(
            tile_graphic,
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_static()),
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_land())
                && tileart_surface_has_visible_primary(tilemeta_package.map_or(&[], |package| package.item_texture_refs(tile_graphic as u32))),
            resolve_surface_like_tex_land_ec_slot_id(
                tile_graphic as u32,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            ),
        ),
    }
}

/// The style of the land (`StaticArtSourceState::land_style`), which the statics that continue it take whatever the art
/// source (`static_continues_land`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StaticLandStyle {
    /// No land cache yet: every static keeps its art source.
    #[default]
    Unknown,
    /// The land page atlas holds Classic land (`LandRoutes::atlas`).
    Classic,
    /// The land page atlas holds Enhanced land.
    Enhanced,
    /// The land page atlas holds Kingdom Reborn land: the statics that continue it are drawn as for Enhanced land, from
    /// the Kingdom Reborn item packages where they hold them (`kingdom_reborn_static_visual_kind`).
    KingdomReborn,
}

impl StaticLandStyle {
    pub(super) fn of_land_atlas(atlas: LandTextureSource) -> Self {
        match atlas {
            LandTextureSource::Kr => Self::KingdomReborn,
            atlas if atlas.uses_enhanced_land() => Self::Enhanced,
            _ => Self::Classic,
        }
    }

    /// EC-layout land: Enhanced or Kingdom Reborn.
    pub(super) fn is_enhanced(self) -> bool {
        matches!(self, Self::Enhanced | Self::KingdomReborn)
    }
}

/// `land_style`: the statics that continue the land take its style (`static_continues_land`). Under Classic land
/// Enhanced art draws them from their Classic art, as Classic art mode does; under Enhanced or KR land Classic art draws
/// them the Enhanced way, as Enhanced art mode does.
pub(super) fn resolve_static_visual_kind_cached(
    art_source: ClientTextureSource,
    land_style: StaticLandStyle,
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_cc: Option<&udd_assets::tex_art_cc::TexArtCcPackage>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> StaticVisualKind {
    let mut enhanced = || enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec,
        tex_land_ec, surface_resolution_cache);
    match art_source {
        ClientTextureSource::Cc => {
            let classic = classic_static_visual_kind(tile_graphic, tilemeta);
            if !land_style.is_enhanced() {
                return classic;
            }
            enhanced().filter(|&kind| static_continues_land(kind, tilemeta,
                surface_resolution_cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)))
                .unwrap_or(classic)
        }
        ClientTextureSource::Ec => {
            let Some(enhanced) = enhanced() else {
                return present_classic_static_visual_kind(tile_graphic, tilemeta, tex_art_cc)
                    .unwrap_or(StaticVisualKind::Missing);
            };
            (land_style == StaticLandStyle::Classic && static_continues_land(enhanced, tilemeta,
                surface_resolution_cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)))
                .then(|| present_classic_static_visual_kind(tile_graphic, tilemeta, tex_art_cc))
                .flatten()
                .unwrap_or(enhanced)
        }
    }
}

/// What Enhanced art draws for `tile_graphic`, or `None` when the EC art package has no image for it (Enhanced art mode
/// then falls back to its Classic art).
fn enhanced_static_visual_kind(
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> Option<StaticVisualKind> {
    let slot = tex_art_ec?.present_slot(u32::from(tile_graphic))?;
    Some(resolve_ec_static_visual_kind(
        tile_graphic,
        slot.is_static(),
        slot.is_land()
            && tileart_surface_has_visible_primary(tilemeta_package.map_or(&[], |package| package.item_texture_refs(tile_graphic as u32))),
        surface_resolution_cache.resolve_surface_like_tex_land_ec_slot_id(
            ClientTextureSource::Ec,
            tile_graphic as u32,
            tilemeta_package,
            tilemeta,
            tex_land_ec,
        ),
    ))
}

/// The Classic art of `tile_graphic` (`classic_static_visual_kind`), when the Classic art package has it.
fn present_classic_static_visual_kind(
    tile_graphic: u16,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_cc: Option<&udd_assets::tex_art_cc::TexArtCcPackage>,
) -> Option<StaticVisualKind> {
    let classic = classic_static_visual_kind(tile_graphic, tilemeta);
    let StaticVisualKind::CcRegular { art_id, fallback_art_id } = classic else { unreachable!() };
    tex_art_cc.is_some_and(|p| p.present_slot(art_id).is_some() || p.present_slot(fallback_art_id).is_some())
        .then_some(classic)
}

/// The Kingdom Reborn packages the statics that continue the Kingdom Reborn land are drawn from
/// (`kingdom_reborn_static_visual_kind`).
#[derive(bevy::ecs::system::SystemParam)]
pub struct KrStaticPackages<'w> {
    items: Option<Res<'w, KrItemPackagesRes>>,
    land: Option<Res<'w, TexLandKrPackageRes>>,
}

impl KrStaticPackages<'_> {
    /// The packages, when the item packages and the land package are all there.
    pub(super) fn sources(&self) -> Option<KrStaticSources<'_>> {
        let (items, land) = (self.items.as_deref()?, self.land.as_deref()?);
        Some(KrStaticSources { tilemeta: &items.tilemeta, art: &items.art, land: &land.0 })
    }

    /// Whether a package arrived or changed.
    pub(super) fn is_changed(&self) -> bool {
        self.items.as_ref().is_some_and(|items| items.is_changed())
            || self.land.as_ref().is_some_and(|land| land.is_changed())
    }
}

/// The Kingdom Reborn tile data, item art (in the Enhanced layouts) and land.
#[derive(Clone, Copy)]
pub(super) struct KrStaticSources<'a> {
    pub(super) tilemeta: &'a udd_assets::tilemeta::TileMetaPackage,
    /// An overlay over the Enhanced art: the items whose world art Kingdom Reborn draws differently ([`Self::art_slot`]).
    pub(super) art: &'a udd_assets::tex_art_ec::TexArtEcPackage,
    pub(super) land: &'a udd_assets::tex_land_ec::TexLandEcPackage,
}

impl<'a> KrStaticSources<'a> {
    /// The present Kingdom Reborn art slot of `graphic`, read through the overlay over the Enhanced art `enhanced`, with
    /// the package holding it.
    pub(super) fn art_slot(self, enhanced: Option<&'a udd_assets::tex_art_ec::TexArtEcPackage>, graphic: u32)
        -> Option<(&'a udd_assets::tex_art_ec::TexArtEcPackage, &'a udd_assets::tex_art_ec::TexArtEcSlotRecord)> {
        self.art.overlay_slot(enhanced.unwrap_or(self.art), graphic)
    }
}

impl StaticVisualKind {
    /// The kind drawn from the Kingdom Reborn packages: its art ids and `material_item_id` from
    /// `KR_ITEM_RECORD_BASE`, its ground slots in the Kingdom Reborn land.
    fn kingdom_reborn(self) -> Self {
        match self {
            Self::EcRegular { art_id } => Self::EcRegular { art_id: art_id + KR_ITEM_RECORD_BASE },
            Self::EcSurface { art_id } => Self::EcSurface { art_id: art_id + KR_ITEM_RECORD_BASE },
            Self::TexLandEc { resolution } => Self::TexLandEc {
                resolution: SurfaceLikeTexLandEcResolution { kingdom_reborn: true, ..resolution },
            },
            kind @ (Self::CcRegular { .. } | Self::Missing) => kind,
        }
    }

    fn is_kingdom_reborn(self) -> bool {
        match self {
            Self::EcRegular { art_id } | Self::EcSurface { art_id } => art_id >= KR_ITEM_RECORD_BASE,
            Self::TexLandEc { resolution } => resolution.kingdom_reborn,
            Self::CcRegular { .. } | Self::Missing => false,
        }
    }

    /// The item of the item lookups (`ec_item_surface`, `ec_item_liquid`) a static of `graphic` drawn as this kind reads.
    fn material_item_id(self, graphic: u16) -> u32 {
        u32::from(graphic) + if self.is_kingdom_reborn() { KR_ITEM_RECORD_BASE } else { 0 }
    }
}

/// Under the Kingdom Reborn land (`StaticLandStyle::KingdomReborn`), a static that continues the land
/// (`static_continues_land`) by the Enhanced data or by Kingdom Reborn's is drawn as Kingdom Reborn draws it: its item
/// art, material and land textures, so it matches the land under and around it (the grass, liquids, lava and swamp, and
/// the water 0x34D2, a sprite in KR). `None` keeps the kind of `resolve_static_visual_kind_cached`: a static that does
/// not continue the land, or one the Kingdom Reborn art lacks (logged once).
pub(super) fn kingdom_reborn_static_visual_kind(
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    kr: KrStaticSources<'_>,
    cache: &mut StaticSurfaceLikeResolutionCache,
) -> Option<StaticVisualKind> {
    if let Some(&cached) = cache.kingdom_reborn_kinds.get(&tile_graphic) {
        return cached;
    }
    let graphic = u32::from(tile_graphic);
    let enhanced_continues = enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec,
        tex_land_ec, cache).is_some_and(|kind| static_continues_land(kind, tilemeta,
            cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)));
    let kr_tilemeta = kr.tilemeta.item_tile(graphic);
    let kr_kind = kr.art_slot(tex_art_ec, graphic).map(|(_, slot)| resolve_ec_static_visual_kind(
        tile_graphic,
        slot.is_static(),
        slot.is_land() && tileart_surface_has_visible_primary(kr.tilemeta.item_texture_refs(graphic)),
        resolve_surface_like_tex_land_ec_slot_id(graphic, Some(kr.tilemeta), kr_tilemeta, Some(kr.land)),
    ));
    let kr_terrain_textures = cache.kingdom_reborn_terrain_textures.get_or_insert_with(|| terrain_texture_ids(kr.land));
    let kr_continues = kr_kind.is_some_and(|kind| static_continues_land(kind, kr_tilemeta,
        item_draws_terrain_texture(graphic, Some(kr.tilemeta), kr_terrain_textures)));
    let kind = if !(enhanced_continues || kr_continues) {
        None
    } else if let Some(kind) = kr_kind {
        Some(kind.kingdom_reborn())
    } else {
        console_logger::one(LogSev::Info, LogAbout::RenderWorldArt, &format!(
            "Static {tile_graphic:#06x} continues the land, but the Kingdom Reborn art (the overlay in \
             tex_art_ec.uddp) lacks it: it keeps the Enhanced item data under the Kingdom Reborn land."));
        None
    };
    cache.kingdom_reborn_kinds.insert(tile_graphic, kind);
    kind
}

/// The tile data a static of `graphic` drawn as `kind` is drawn and picked by, the package and its item: Kingdom
/// Reborn's (`kr`) for a kind drawn from the Kingdom Reborn packages, as the KR client draws its art by its own tileart
/// (its NoDraw, Surface and Rotated flags differ from the Enhanced ones on some of the statics that continue the land);
/// `cc_art`, the clients' tables without the tilemeta overrides (`TileMetaPackageRes::for_art`), for one drawn with
/// Classic art, which the overrides were not measured against; else the Enhanced `enhanced`.
pub(super) fn drawn_tile_data<'a>(
    kind: Option<StaticVisualKind>,
    graphic: u16,
    enhanced: Option<&'a udd_assets::tilemeta::TileMetaPackage>,
    cc_art: Option<&'a udd_assets::tilemeta::TileMetaPackage>,
    kr: Option<KrStaticSources<'a>>,
) -> (Option<&'a udd_assets::tilemeta::TileMetaPackage>, Option<&'a udd_assets::tilemeta::TileMetaItemTile>) {
    let package = match (kind, kr) {
        (Some(kind), Some(kr)) if kind.is_kingdom_reborn() => Some(kr.tilemeta),
        (Some(StaticVisualKind::CcRegular { .. }), _) => cc_art,
        _ => enhanced,
    };
    (package, package.and_then(|package| package.item_tile(u32::from(graphic))))
}

/// Whether a static continues the land, so it takes the land's style (`StaticLandStyle`) whatever the art source.
/// `enhanced` is what Enhanced art draws for it; `terrain_texture` is `landlike_item_texture`. The data decides:
/// - its Enhanced material is drawn from the terrain: a land liquid (`TexLandEc` with a liquid payload: the water and
///   foam 0x1796–0x17B2 and 0x346E–0x34D2, the acid and the lava the same water shader draws), or an item surface or
///   land-textured item whose visible texture a terrain material draws (grass, dirt, snow, swamp, the lava fields);
/// - and it is not a built floor (NoShoot): cobblestones, pavers, marble, flagstones, bricks and the other town floors
///   share the terrain textures of the town floor land, but the maps place them almost only raised or roofed, as house,
///   castle and dungeon floors, so they keep their art source (docs/agent_notes/water_terrain_static_resolution.md).
fn static_continues_land(
    enhanced: StaticVisualKind,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    terrain_texture: bool,
) -> bool {
    let built_floor = tilemeta.is_some_and(|meta| meta.flags & TILE_FLAG_NO_SHOOT != 0);
    match enhanced {
        StaticVisualKind::TexLandEc { resolution } if resolution.liquid_payload.is_some() => true,
        StaticVisualKind::TexLandEc { .. } | StaticVisualKind::EcSurface { .. } => terrain_texture && !built_floor,
        StaticVisualKind::CcRegular { .. } | StaticVisualKind::EcRegular { .. } | StaticVisualKind::Missing => false,
    }
}

/// `STATIC_FLAG_LAND_GRADE` for a static of `tile_graphic` drawn as `visual_kind`: set when it continues the land by
/// the Enhanced data (`static_continues_land`, whatever the art source and land), or is drawn from the Kingdom Reborn
/// packages, which only the statics that continue the KR land are (`kingdom_reborn_static_visual_kind`), or is foliage
/// by the tile data it is drawn by (`drawn_flags`, `drawn_tile_data`): the leaves take the knee tuned for the grass, as
/// without it they stood out of the KR screenshots' foliage, while trunks without the Foliage flag keep their colour.
fn land_grade_flag(
    visual_kind: StaticVisualKind,
    tile_graphic: u16,
    drawn_flags: u64,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    cache: &mut StaticSurfaceLikeResolutionCache,
) -> u32 {
    let continues = visual_kind.is_kingdom_reborn()
        || drawn_flags & TILE_FLAG_FOLIAGE != 0
        || enhanced_static_visual_kind(tile_graphic, tilemeta_package, tilemeta, tex_art_ec, tex_land_ec, cache)
            .is_some_and(|kind| static_continues_land(kind, tilemeta,
                cache.landlike_item_texture(tile_graphic, tilemeta_package, tex_land_ec)));
    if continues { STATIC_FLAG_LAND_GRADE } else { 0 }
}

/// Whether one of the visible Enhanced textures of item `tile_id` (its world-art base layers) is a texture a terrain
/// material of the EC land package draws.
fn item_draws_terrain_texture(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    terrain_textures: &HashSet<u32>,
) -> bool {
    tilemeta_package.is_some_and(|package| package.item_texture_refs(tile_id).iter().any(|texture_ref|
        texture_ref.is_world_art() && !texture_ref.is_auxiliary()
            && matches!(texture_ref.stable_role(), udd_assets::tilemeta::EcMaterialStableRole::Base
                | udd_assets::tilemeta::EcMaterialStableRole::SecondaryBase)
            && terrain_textures.contains(&texture_ref.texture_id)))
}

/// The textures the terrain materials of the EC land package draw: each terrain record's selected and primary texture.
fn terrain_texture_ids(tex_land_ec: &udd_assets::tex_land_ec::TexLandEcPackage) -> HashSet<u32> {
    tex_land_ec.terrain_provenance().iter()
        .flat_map(|record| [record.selected_texture_id, record.primary_texture_id])
        .filter(|&texture_id| texture_id != udd_assets::tex_land_ec::MISSING_TEXTURE_ID)
        .collect()
}

fn tileart_surface_has_visible_primary(
    refs: &[udd_assets::tilemeta::TileMetaItemTextureRef],
) -> bool {
    use udd_assets::tilemeta::EcMaterialStableRole;
    // Some packed land slots contain the first source support image, not albedo.
    // Water's primary-selected alpha/normal input must use its visible material
    // route instead of painting that support image onto a tileart ground quad.
    // The packer uses ArtData.ec_texture, preserved as PRIMARY_SELECTED. The
    // runtime's role-aware main selector can choose a different source image;
    // it cannot change the pixels already stored in the tileart atlas.
    // The rejected roles are the shader inputs that the former file-name roles (alpha mask, generic
    // mask, noise, normal-like, image support, effect-only) were mapped to.
    !refs.iter().find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected())
        .is_some_and(|texture_ref| matches!(texture_ref.stable_role(),
        EcMaterialStableRole::BlendAlpha | EcMaterialStableRole::HueMask
            | EcMaterialStableRole::Noise | EcMaterialStableRole::WaterBump
            | EcMaterialStableRole::Unused | EcMaterialStableRole::Unknown
    ))
}

fn resolve_ec_static_visual_kind(
    tile_graphic: u16,
    has_tex_art_ec_static_slot: bool,
    has_tex_art_ec_surface_slot: bool,
    surface_like_land_resolution: Option<SurfaceLikeTexLandEcResolution>,
) -> StaticVisualKind {
    if has_tex_art_ec_static_slot {
        // Walkable sprites (stairs, bridges, furniture) still own their art.
        // The Surface tiledata flag does not make their graphic id a land id.
        StaticVisualKind::EcRegular { art_id: tile_graphic as u32 }
    } else if let Some(resolution) = surface_like_land_resolution.filter(|r| r.liquid_payload.is_some()) {
        StaticVisualKind::TexLandEc { resolution }
    } else if has_tex_art_ec_surface_slot {
        // Tileart owns this image and its sampling window. A terrain texture with
        // the same source id cannot replace an authored item-owned surface.
        StaticVisualKind::EcSurface { art_id: tile_graphic as u32 }
    } else if let Some(resolution) = surface_like_land_resolution {
        StaticVisualKind::TexLandEc { resolution }
    } else {
        StaticVisualKind::EcRegular {
            art_id: tile_graphic as u32,
        }
    }
}

/// Whether a static takes the wet render path (animated water UV distortion). This is a render
/// decision, not the object property: `TileMetaItemTile::is_wet` (IsWet) stays the gameplay flag.
/// - Classic art mode: the IsWet flag.
/// - Enhanced art mode, item with an EC material: only its shader decides (`UOWaterShader`); IsWet
///   does not override it, so water-like sprite-shader items render dry.
/// - Enhanced art mode, item without an EC material: it draws the Classic fallback art, so IsWet.
pub(crate) fn static_tile_renders_wet(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    art_id: u32,
    art_source: Option<ClientTextureSource>,
) -> bool {
    let classic_wet = tilemeta.is_some_and(udd_assets::tilemeta::TileMetaItemTile::is_wet);
    if art_source != Some(ClientTextureSource::Ec) {
        return classic_wet;
    }
    match tilemeta_package.filter(|package| package.has_ec_material(art_id)) {
        Some(package) => package.uses_water_shader(art_id),
        None => classic_wet,
    }
}

fn static_hue_payload(hue_id: u16) -> (u32, u32) {
    if hue_id == 0 {
        (0, 0)
    } else {
        (u32::from(hue_id), STATIC_HUE_FLAG_APPLY)
    }
}

pub(crate) fn static_sprite_opacity(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    legacy_art: bool) -> f32 {
    use uocf::enhanced::tileart::TaeFlag;
    if !legacy_art { return 1.0; }
    let Some(tile) = tilemeta else { return 1.0; };
    if tile.flags & TaeFlag::Translucent.bits() == 0 { return 1.0; }
    // Original CC art dispatcher 0x44e3a0 selects the 3/4-source blitter
    // 0x5d8000, or its reverse at 0x5d8050 for Foliage. Keep original flags
    // intact; this is sprite material opacity, never a movement property.
    if tile.flags & TaeFlag::Foliage.bits() != 0 { 0.25 } else { 0.75 }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, ShaderType)]
pub struct SpriteInstance {
    pub world_x: f32,        // tile_x + x_offset
    pub world_z: f32,        // tile_y + y_offset
    pub world_y: f32,        // z * height_scale (isometric altitude)
    pub layer: u32,          // atlas page layer
    pub depth_class: u32,    // encoded StaticDepthClass for future logical-depth policy
    pub base_world_y: f32,   // raw tile-base height for logical depth
    pub uv_min: [f32; 2],    // normalized UV
    pub uv_max: [f32; 2],    // normalized UV
    pub local_min: [f32; 2], // local quad bounds from tile origin
    pub local_max: [f32; 2], // local quad bounds from tile origin
    /// The draw order's anchor (art/depth.wesl): the cell + 2 in x + y for an upright sprite.
    pub tile_x: f32,
    pub tile_y: f32,
    /// ClassicUO's priority z (`resolve_priority_z_units`).
    pub priority_z_units: f32,
    /// The draw order's tie level (`assign_sprite_tie_levels`): 1..8 for an upright sprite, 0 for a flat quad.
    pub sort_bias_ordinal: u32,
    /// Tiledata flags packed for the GPU.
    /// Bit 0: is_wet (`static_tile_renders_wet` → animated water UV distortion in sprite shader).
    /// Bit 1: ground surface blend; bits 2..9: NW,N,NE,W,E,SW,S,SE same-material neighbors.
    /// Bit 10: `STATIC_FLAG_CLIENT_GRUNGE`, the KR client grunges the static (art/client_grunge.wesl).
    /// Bit 11: `STATIC_FLAG_FLAT_ART`, the upright card shows flat art (art/shading.wesl).
    /// Bit 24: `STATIC_FLAG_CAP_MEETS_FLOOR`, a wall with a floor on its top in the farther row (art/depth.wesl).
    /// Bit 25: `STATIC_FLAG_LAND_GRADE`, the static continues the land and takes its grading (art/shading.wesl).
    pub is_wet_flags: u32,
    pub hue_id: u32,
    pub hue_flags: u32,
    /// Flat-quad flag: 1 = lay this instance flat in the XZ plane (CC surface-like
    /// water statics), 2 = axis-aligned EC surface, 3 = authored EC camera quad,
    /// 0 = upright billboard.
    /// See main.wesl.
    pub flat_quad: u32,
    pub texture_stretch: f32,
    pub material_item_id: u32,
    /// The KR client's static shadow: `CLIENT_SHADOW_FLAG_*` bits (art/client_shadow.wesl).
    pub client_shadow_flags: u32,
    /// The caster's height in UO z units above the client's ground below it; 0 takes the shadow's h = 0 branch.
    pub client_shadow_height: f32,
    /// The KR client's base point of the static on the ground, (tile x + 0.5, tile y + 1.5) in world x and z: the
    /// client grunge maps from it (art/client_grunge.wesl). Takes the former `_pad_client_shadow` slot.
    pub client_grunge_anchor: [f32; 2],
    /// The static local lights at the quad's four corners, in `ART_QUAD_CORNERS` order, each RGBA8 packed
    /// (`static_local_light_corners`); the shaders interpolate them across the quad.
    pub local_light_corners: [u32; 4],
    pub color_rgba: [f32; 4], // for dot mode
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, ShaderType)]
pub struct GroundTileInstance {
    pub world_x: f32,
    pub world_z: f32,
    pub world_y: f32,
    pub layer: u32,
    pub depth_class: u32,
    pub base_world_y: f32,
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub local_min: [f32; 2],
    pub local_max: [f32; 2],
    pub tile_x: f32,
    pub tile_y: f32,
    pub priority_z_units: f32,
    pub sort_bias_ordinal: u32,
    /// Tiledata flags packed for the GPU.
    /// Bit 0: is_wet (IsWet tiledata flag → animated water UV distortion in ground shader).
    /// Bit 10: `STATIC_FLAG_CLIENT_GRUNGE`, the KR client grunges the static (art/client_grunge.wesl).
    /// Bit 25: `STATIC_FLAG_LAND_GRADE`, the static continues the land and takes its grading (art/shading.wesl).
    pub is_wet_flags: u32,
    pub texture_stretch: f32,
    pub hue_id: u32,
    pub hue_flags: u32,
    pub material_payload: u32,
    pub material_flags: u32,
    /// Original item graphic selects its own optional fourth water source.
    pub material_item_id: u32,
    pub _pad_material: u32,
    /// The KR client's base point of the static, as `SpriteInstance::client_grunge_anchor`: the client grunge maps
    /// from it (art/client_grunge.wesl).
    pub client_grunge_anchor: [f32; 2],
    /// The static local lights at the quad's corners, as `SpriteInstance::local_light_corners`.
    pub local_light_corners: [u32; 4],
    pub color_rgba: [f32; 4],
}

#[derive(Resource, Default)]
pub struct RenderStaticInstances(pub Vec<SpriteInstance>);

#[derive(Resource, Default)]
pub struct RenderStaticLandInstances(pub Vec<GroundTileInstance>);

/// A hash of every collected static instance in output order, without its art page placement (`layer`, `uv_min`,
/// `uv_max`, which follow the order the pages arrived in): two runs that collected the same statics report the same
/// digest, however the land and the art loaded (`status.art.digest` on the control socket).
pub(crate) fn static_instances_digest(sprites: &[SpriteInstance], ground: &[GroundTileInstance]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    for instance in sprites {
        let unplaced = SpriteInstance { layer: 0, uv_min: [0.0; 2], uv_max: [0.0; 2], ..*instance };
        hasher.write(bytemuck::bytes_of(&unplaced));
    }
    for instance in ground {
        let unplaced = GroundTileInstance { layer: 0, uv_min: [0.0; 2], uv_max: [0.0; 2], ..*instance };
        hasher.write(bytemuck::bytes_of(&unplaced));
    }
    hasher.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StaticChunkBatchKey {
    pub map_id: u32,
    pub gx: u32,
    pub gy: u32,
    pub scale: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticChunkBatch {
    pub key: StaticChunkBatchKey,
    pub start: u32,
    pub count: u32,
}

#[derive(Resource, Default)]
pub struct RenderStaticChunkBatches {
    pub sprite: Vec<StaticChunkBatch>,
    pub ground: Vec<StaticChunkBatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StaticAtlasRetrySignature {
    sprite_mapping_revision: u64,
    ground_mapping_revision: u64,
    sprite_resident_pages: usize,
    ground_resident_pages: usize,
    sprite_pending_pages: usize,
    ground_pending_pages: usize,
    sprite_active_layers: u32,
    ground_active_layers: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StaticChunkCacheConfig {
    map_id: u32,
    dot_mode: bool,
    art_source: Option<ClientTextureSource>,
    /// `StaticArtSourceState::land_style`: a change redraws the statics that continue the land from the other art.
    land_style: StaticLandStyle,
    sprite_atlas_mapping_revision: u64,
    ground_atlas_mapping_revision: u64,
    // The land the chunks read from the tile atlas is not here: a change rebuilds only the chunks that read the
    // changed cells (`StaticChunkRenderCache::invalidate_changed_land`).
    static_art_local_light_signature: Option<u64>,
    show_hidden_multi_parts: bool,
    /// The statics store's item routes (`LazyStaticsStore::item_routes_revision`).
    item_routes_revision: u64,
    /// The z-view limit: a change rebuilds the cached chunks without the statics it hides.
    z_view: ZViewCut,
    /// The caves drawn over the map (`CavesRes::revision`).
    caves_revision: u64,
}

#[derive(Clone, Debug, Default)]
struct CachedStaticChunkStats {
    visited_blocks: usize,
    source_tiles: usize,
    ground_land_tiles: usize,
    atlas_hits: usize,
    atlas_misses: usize,
    requested_pages: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
struct CachedStaticChunk {
    sprite_instances: Vec<SpriteInstance>,
    ground_instances: Vec<GroundTileInstance>,
    pick_tiles: Vec<StaticPickTile>,
    sprite_pick_indices: Vec<usize>,
    ground_pick_indices: Vec<usize>,
    stats: CachedStaticChunkStats,
    last_visible_tick: u64,
    /// The cells of the tile atlas the chunk read (`StaticChunkBuildState::land_cells`).
    land_cells: Option<AtlasCells>,
}

#[derive(Resource, Default)]
pub struct StaticChunkRenderCache {
    config: Option<StaticChunkCacheConfig>,
    current_tick: u64,
    chunks: HashMap<StaticChunkBatchKey, CachedStaticChunk>,
    last_visible_chunk_keys: Vec<StaticChunkBatchKey>,
    last_incomplete_retry_atlas_signature: Option<StaticAtlasRetrySignature>,
    /// The tile atlas's `content_revision` the cached chunks were checked against (`invalidate_changed_land`).
    land_revision: Option<u64>,
}

impl StaticChunkRenderCache {
    /// Only admitted objects in the current rendered set; includes expanded multi
    /// parts and large sprites whose owning cell is far from the ground cursor.
    /// Snapshot the logical owner in the exact order used by each GPU instance
    /// buffer. Pixel-query replies retain this snapshot across asynchronous frames.
    pub(crate) fn gpu_pick_targets(&self, batches: &RenderStaticChunkBatches) -> [Vec<StaticPickTile>; 2] {
        std::array::from_fn(|kind| {
            let batches = if kind == 0 { &batches.sprite } else { &batches.ground };
            let mut targets = Vec::new();
            for batch in batches {
                let Some(chunk) = self.chunks.get(&batch.key) else { return Vec::new(); };
                let indices = if kind == 0 { &chunk.sprite_pick_indices } else { &chunk.ground_pick_indices };
                if targets.len() != batch.start as usize || indices.len() != batch.count as usize { return Vec::new(); }
                targets.extend(indices.iter().map(|&index| chunk.pick_tiles[index]));
            }
            targets
        })
    }
    fn begin_frame(&mut self) -> u64 {
        self.current_tick = self.current_tick.saturating_add(1);
        self.current_tick
    }

    /// Drops every cached chunk on the next collection, as a change of the configuration does (the control socket's
    /// `reload_land`).
    pub(crate) fn rebuild_all(&mut self) {
        self.config = None;
    }

    fn sync_config(&mut self, config: StaticChunkCacheConfig) -> bool {
        let changed = self.config != Some(config);
        if changed {
            self.config = Some(config);
            self.chunks.clear();
            self.last_visible_chunk_keys.clear();
            self.last_incomplete_retry_atlas_signature = None;
        }
        changed
    }

    /// Drops the cached chunks that read tile atlas cells changed since the last call: the land loading, a neighbouring
    /// chunk's land arriving after the chunk, a texture bake or the land of a cave. Every chunk goes when the atlas no
    /// longer says which cells changed (`TileAtlas::changed_cells_after`). Returns how many chunks went.
    fn invalidate_changed_land(&mut self, atlas: &TileAtlas) -> usize {
        let seen = self.land_revision.replace(atlas.content_revision);
        if seen == Some(atlas.content_revision) {
            return 0;
        }
        let before = self.chunks.len();
        match seen.and_then(|seen| atlas.changed_cells_after(seen)) {
            Some(changed) => {
                let changed = changed.collect::<Vec<_>>();
                self.chunks.retain(|_, chunk| chunk.land_cells
                    .is_none_or(|read| !changed.iter().any(|cells| cells.intersects(&read))));
            }
            None => self.chunks.clear(),
        }
        before - self.chunks.len()
    }

    fn prune_stale(&mut self, visible_tick: u64) {
        self.chunks.retain(|_, chunk| {
            visible_tick.saturating_sub(chunk.last_visible_tick)
                <= STATIC_CHUNK_CACHE_HYSTERESIS_TICKS
        });
    }
}

fn static_atlas_retry_signature(
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
) -> StaticAtlasRetrySignature {
    StaticAtlasRetrySignature {
        sprite_mapping_revision: sprite_atlas.0.mapping_revision(),
        ground_mapping_revision: ground_atlas.0.mapping_revision(),
        sprite_resident_pages: sprite_atlas.resident_page_count(),
        ground_resident_pages: ground_atlas.resident_page_count(),
        sprite_pending_pages: sprite_atlas.pending_page_count(),
        ground_pending_pages: ground_atlas.pending_page_count(),
        sprite_active_layers: sprite_atlas.active_layers,
        ground_active_layers: ground_atlas.active_layers,
    }
}

fn aggregate_cached_visible_static_stats(
    cache: &mut StaticChunkRenderCache,
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_tick: u64,
) -> CachedStaticChunkStats {
    let mut aggregate = CachedStaticChunkStats::default();

    for chunk_key in visible_chunk_keys {
        let Some(chunk) = cache.chunks.get_mut(chunk_key) else {
            continue;
        };
        chunk.last_visible_tick = visible_tick;
        aggregate.visited_blocks += chunk.stats.visited_blocks;
        aggregate.source_tiles += chunk.stats.source_tiles;
        aggregate.ground_land_tiles += chunk.stats.ground_land_tiles;
        aggregate.atlas_hits += chunk.stats.atlas_hits;
        aggregate.atlas_misses += chunk.stats.atlas_misses;
        aggregate
            .requested_pages
            .extend(chunk.stats.requested_pages.iter().copied());
    }

    aggregate.requested_pages.sort_unstable();
    aggregate.requested_pages.dedup();
    aggregate
}

fn should_skip_unchanged_incomplete_static_retry(
    cache: &StaticChunkRenderCache,
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_set_unchanged: bool,
    all_visible_chunks_complete: bool,
    atlas_retry_signature: StaticAtlasRetrySignature,
) -> bool {
    visible_set_unchanged
        && !all_visible_chunks_complete
        && visible_chunk_keys
            .iter()
            .all(|chunk_key| cache.chunks.contains_key(chunk_key))
        && cache.last_incomplete_retry_atlas_signature == Some(atlas_retry_signature)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticArtCollectPath {
    CompleteFastPath,
    IncompleteWaitFastPath,
    SlowRebuild,
}

impl StaticArtCollectPath {
    fn label(self) -> &'static str {
        match self {
            Self::CompleteFastPath => "complete_fast_path",
            Self::IncompleteWaitFastPath => "incomplete_wait_fast_path",
            Self::SlowRebuild => "slow_rebuild",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticArtCollectPathCounters {
    pub complete_fast_path: u64,
    pub incomplete_wait_fast_path: u64,
    pub slow_rebuild: u64,
    pub copied_sprite_instances: u64,
    pub copied_ground_instances: u64,
}

fn record_static_collect_path(
    debug_state: &mut StaticArtCollectDebugState,
    path: StaticArtCollectPath,
    copied_sprite_instances: usize,
    copied_ground_instances: usize,
) {
    match path {
        StaticArtCollectPath::CompleteFastPath => {
            debug_state.path_counters.complete_fast_path =
                debug_state.path_counters.complete_fast_path.saturating_add(1);
        }
        StaticArtCollectPath::IncompleteWaitFastPath => {
            debug_state.path_counters.incomplete_wait_fast_path = debug_state
                .path_counters
                .incomplete_wait_fast_path
                .saturating_add(1);
        }
        StaticArtCollectPath::SlowRebuild => {
            debug_state.path_counters.slow_rebuild =
                debug_state.path_counters.slow_rebuild.saturating_add(1);
        }
    }
    debug_state.path_counters.copied_sprite_instances = debug_state
        .path_counters
        .copied_sprite_instances
        .saturating_add(copied_sprite_instances as u64);
    debug_state.path_counters.copied_ground_instances = debug_state
        .path_counters
        .copied_ground_instances
        .saturating_add(copied_ground_instances as u64);
}

fn log_static_collect_stats(
    debug_state: &mut StaticArtCollectDebugState,
    stats: StaticArtCollectStats,
) {
    if debug_state.last != Some(stats) {
        console_logger::one(
            LogSev::DebugVerbose,
            LogAbout::RenderWorldArt,
            &format!(
                "static art collect (1/2): map={} dot_mode={} chunks={} blocks={} tiles={} ground_land_tiles={}",
                stats.map_id,
                stats.dot_mode,
                stats.visible_chunks,
                stats.visited_blocks,
                stats.source_tiles,
                stats.ground_land_tiles,
            ),
        );
        console_logger::one(
            LogSev::DebugVerbose,
            LogAbout::RenderWorldArt,
            &format!(
                "static art collect (2/2):unique_pages={} resident_pages={} pending_pages={} capacity={} atlas_hits={} atlas_misses={} emitted={}",
                stats.unique_requested_pages,
                stats.resident_pages,
                stats.pending_pages,
                stats.atlas_capacity_pages,
                stats.atlas_hits,
                stats.atlas_misses,
                stats.emitted_instances,
            ),
        );
        console_logger::one(
            LogSev::DebugVerbose,
            LogAbout::RenderWorldArt,
            &format!(
                "static art collect (3/3):path={} copied_sprite={} copied_ground={} path_counts complete={} incomplete_wait={} slow={} total_copied_sprite={} total_copied_ground={}",
                stats.path.label(),
                stats.copied_sprite_instances,
                stats.copied_ground_instances,
                debug_state.path_counters.complete_fast_path,
                debug_state.path_counters.incomplete_wait_fast_path,
                debug_state.path_counters.slow_rebuild,
                debug_state.path_counters.copied_sprite_instances,
                debug_state.path_counters.copied_ground_instances,
            ),
        );
        debug_state.last = Some(stats);
    }
}

fn static_light_signature(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
) -> u64 {
    let mut signature = 0xcbf29ce484222325u64;
    for light in lights.0.iter().filter(|light| light.key.map_id == map_id) {
        for value in [
            light.key.tile_x,
            light.key.tile_y,
            light.light_id,
            light.hue_id as u32,
            (light.world_y * 100.0).round().max(0.0) as u32,
            (light.width_world * 100.0).round().max(0.0) as u32,
            (light.height_world * 100.0).round().max(0.0) as u32,
            (light.color_rgb[0].clamp(0.0, 1.0) * 255.0).round() as u32,
            (light.color_rgb[1].clamp(0.0, 1.0) * 255.0).round() as u32,
            (light.color_rgb[2].clamp(0.0, 1.0) * 255.0).round() as u32,
        ] {
            signature ^= value as u64;
            signature = signature.wrapping_mul(0x100000001b3);
        }
    }
    signature
}

/// A static's local light packed for the GPU as RGBA8, as the art shaders read it back (`unpack4x8unorm`: red in the
/// low byte).
fn pack_local_light_rgba(rgba: [f32; 4]) -> u32 {
    rgba.iter().rev().fold(0, |packed, channel| {
        (packed << LOCAL_LIGHT_CHANNEL_BITS) | (channel.clamp(0.0, 1.0) * LOCAL_LIGHT_CHANNEL_MAX).round() as u32
    })
}

/// The static local lights at a quad's four corners (`SpriteInstance::local_light_corners`), each packed RGBA8.
/// `corner_world` gives the world point (x, z, y) of the unit quad's corner (x, z), as the art shaders lay the quad
/// out. The shaders interpolate the corners across the quad, so a light falls off smoothly over a floor or a roof
/// made of many statics, as it does over the land, and an upright static brightens towards the light. Lighting each
/// static evenly, with the strongest of a few samples, drew a block per tile.
fn static_local_light_corners(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    corner_world: impl Fn(f32, f32) -> (f32, f32, f32),
) -> [u32; 4] {
    ART_QUAD_CORNERS.map(|(corner_x, corner_z)| {
        let (world_x, world_z, world_y) = corner_world(corner_x, corner_z);
        pack_local_light_rgba(static_local_light_rgba(lights, map_id, world_x, world_z, world_y))
    })
}

/// The local light at one point, the same at every corner: the dot mode's statics.
fn static_art_point_local_light_corners(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    world_y: f32,
) -> [u32; 4] {
    if enabled {
        [pack_local_light_rgba(static_local_light_rgba(lights, map_id, world_x, world_z, world_y)); 4]
    } else {
        [0; 4]
    }
}

fn static_local_light_rgba(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    world_y: f32,
) -> [f32; 4] {
    let mut intensity = 0.0f32;
    let mut rgb = [0.0f32; 3];
    for light in lights.0.iter().filter(|light| light.key.map_id == map_id) {
        let radius = light.width_world.max(light.height_world).max(1.0) * 0.62 + 1.25;
        let dx = world_x - light.world_x;
        let dz = world_z - light.world_z;
        let dy = (world_y - light.world_y).abs() * 1.8;
        let distance = (dx * dx + dz * dz + dy * dy).sqrt();
        if distance >= radius {
            continue;
        }

        let falloff = 1.0 - distance / radius;
        let area_scale = (light.width_world * light.height_world).sqrt().clamp(1.0, 5.0) / 5.0;
        let contribution = falloff * falloff * (0.45 + area_scale * 0.55);
        intensity += contribution;
        rgb[0] += light.color_rgb[0] * contribution;
        rgb[1] += light.color_rgb[1] * contribution;
        rgb[2] += light.color_rgb[2] * contribution;
    }

    let intensity = intensity.clamp(0.0, 1.0);
    [
        rgb[0].clamp(0.0, 1.0),
        rgb[1].clamp(0.0, 1.0),
        rgb[2].clamp(0.0, 1.0),
        intensity,
    ]
}

/// The local lights at an upright billboard's corners, laid out as main.wesl does: x along the billboard's right, its
/// top (`local_max_y`) at corner z 0. A card showing flat art (`STATIC_FLAG_FLAT_ART`) takes them at the ground points
/// its corners show, down the view ray at its base height, so the light runs on across neighbouring floor or roof
/// tiles as over the flat statics.
fn static_art_billboard_local_light_corners(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: StaticBillboardBounds,
    flat_art: bool,
) -> [u32; 4] {
    if !enabled {
        return [0; 4];
    }
    let view_ray = PlayerCamera::BASE_OFFSET_FROM_PLAYER;
    static_local_light_corners(lights, map_id, |corner_x, corner_z| {
        let local_x = bounds.local_min_x + (bounds.local_max_x - bounds.local_min_x) * corner_x;
        let local_y = bounds.local_max_y - (bounds.local_max_y - bounds.local_min_y) * corner_z;
        let (sample_x, sample_z, sample_y) =
            static_billboard_sample_world(world_x, world_z, base_world_y, local_x, local_y);
        if flat_art {
            let ground = Vec3::new(sample_x, sample_y, sample_z) - view_ray * (local_y / view_ray.y);
            (ground.x, ground.z, ground.y)
        } else {
            (sample_x, sample_z, sample_y)
        }
    })
}

fn static_billboard_sample_world(
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    local_x: f32,
    local_y: f32,
) -> (f32, f32, f32) {
    (
        world_x + local_x * STATIC_BILLBOARD_RIGHT_XZ.x,
        world_z + local_x * STATIC_BILLBOARD_RIGHT_XZ.y,
        base_world_y + local_y,
    )
}

/// The local lights at the corners of a quad lying flat along the world axes: the ground pipeline's statics and the
/// EC item surfaces (main.wesl's `flat_quad` 2 and 3).
fn static_art_ground_local_light_corners(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: GroundQuadBounds,
) -> [u32; 4] {
    if !enabled {
        return [0; 4];
    }
    static_local_light_corners(lights, map_id, |corner_x, corner_z| {
        (
            world_x + bounds.local_min_x + (bounds.local_max_x - bounds.local_min_x) * corner_x,
            world_z + bounds.local_min_z + (bounds.local_max_z - bounds.local_min_z) * corner_z,
            base_world_y,
        )
    })
}

/// The local lights at the corners of a CC flat static, laid out as main.wesl lays `flat_quad` 1: x along the
/// billboard's right, z along `CC_FLAT_QUAD_FORWARD_XZ`, around its centre.
fn static_art_cc_flat_local_light_corners(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    center_world_x: f32,
    center_world_z: f32,
    base_world_y: f32,
    bounds: GroundQuadBounds,
) -> [u32; 4] {
    if !enabled {
        return [0; 4];
    }
    static_local_light_corners(lights, map_id, |corner_x, corner_z| {
        let local_x = bounds.local_min_x + (bounds.local_max_x - bounds.local_min_x) * corner_x;
        let local_z = bounds.local_min_z + (bounds.local_max_z - bounds.local_min_z) * corner_z;
        let world = Vec2::new(center_world_x, center_world_z)
            + STATIC_BILLBOARD_RIGHT_XZ * local_x
            + CC_FLAT_QUAD_FORWARD_XZ * local_z;
        (world.x, world.y, base_world_y)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StaticArtCollectStats {
    pub path: StaticArtCollectPath,
    pub map_id: u32,
    pub dot_mode: bool,
    pub visible_chunks: usize,
    pub visited_blocks: usize,
    pub source_tiles: usize,
    pub ground_land_tiles: usize,
    pub unique_requested_pages: usize,
    pub resident_pages: usize,
    pub pending_pages: usize,
    pub atlas_capacity_pages: usize,
    pub atlas_hits: usize,
    pub atlas_misses: usize,
    pub emitted_instances: usize,
    pub copied_sprite_instances: usize,
    pub copied_ground_instances: usize,
}

#[derive(Resource, Default)]
pub struct StaticArtCollectDebugState {
    pub last: Option<StaticArtCollectStats>,
    pub path_counters: StaticArtCollectPathCounters,
    /// The cached chunks dropped so far because the land they read changed (`invalidate_changed_land`).
    pub land_invalidated_chunks: u64,
}

#[derive(Resource, Default)]
pub struct StaticArtSourceState {
    pub active_source: Option<ClientTextureSource>,
    pub warned_unavailable_source: Option<ClientTextureSource>,
    /// The style of the land page atlas (`LandRoutes::atlas`), which the statics that continue the land take
    /// (`static_continues_land`).
    pub land_style: StaticLandStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StaticSurfaceLikeResolutionCacheKey {
    art_source: u8,
    tile_id: u32,
}

#[derive(Resource, Default)]
pub struct StaticSurfaceLikeResolutionCache {
    entries: HashMap<StaticSurfaceLikeResolutionCacheKey, Option<SurfaceLikeTexLandEcResolution>>,
    /// `terrain_texture_ids`, read once from the EC land package.
    terrain_textures: Option<HashSet<u32>>,
    /// `item_draws_terrain_texture` by item.
    terrain_texture_items: HashMap<u16, bool>,
    /// `kingdom_reborn_static_visual_kind` by item.
    kingdom_reborn_kinds: HashMap<u16, Option<StaticVisualKind>>,
    /// `terrain_texture_ids` of the Kingdom Reborn land package.
    kingdom_reborn_terrain_textures: Option<HashSet<u32>>,
}

impl StaticSurfaceLikeResolutionCache {
    /// Whether item `tile_graphic` draws a terrain texture in Enhanced art (`item_draws_terrain_texture`).
    fn landlike_item_texture(
        &mut self,
        tile_graphic: u16,
        tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
        tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    ) -> bool {
        if let Some(&cached) = self.terrain_texture_items.get(&tile_graphic) {
            return cached;
        }
        let Some(tex_land_ec) = tex_land_ec else { return false; };
        let terrain_textures = self.terrain_textures.get_or_insert_with(|| terrain_texture_ids(tex_land_ec));
        let draws = item_draws_terrain_texture(u32::from(tile_graphic), tilemeta_package, terrain_textures);
        self.terrain_texture_items.insert(tile_graphic, draws);
        draws
    }

    fn resolve_surface_like_tex_land_ec_slot_id(
        &mut self,
        art_source: ClientTextureSource,
        tile_id: u32,
        tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
        tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
        tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    ) -> Option<SurfaceLikeTexLandEcResolution> {
        let key = StaticSurfaceLikeResolutionCacheKey {
            art_source: static_surface_like_resolution_art_source_tag(art_source),
            tile_id,
        };
        self.get_or_insert_with(key, || {
            resolve_surface_like_tex_land_ec_slot_id(
                tile_id,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            )
        })
    }

    fn get_or_insert_with(
        &mut self,
        key: StaticSurfaceLikeResolutionCacheKey,
        resolve: impl FnOnce() -> Option<SurfaceLikeTexLandEcResolution>,
    ) -> Option<SurfaceLikeTexLandEcResolution> {
        if let Some(cached) = self.entries.get(&key).copied() {
            return cached;
        }

        let resolved = resolve();
        self.entries.insert(key, resolved);
        resolved
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn static_surface_like_resolution_art_source_tag(source: ClientTextureSource) -> u8 {
    match source {
        ClientTextureSource::Cc => 0,
        ClientTextureSource::Ec => 1,
    }
}

fn source_available(
    source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> bool {
    match source {
        ClientTextureSource::Cc => tex_art_cc_res.is_some(),
        ClientTextureSource::Ec => {
            tex_art_ec_res.is_some() && tex_land_ec_res.is_some() && tilemeta_res.is_some()
        }
    }
}

fn resolve_effective_art_source(
    requested_source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> Option<ClientTextureSource> {
    if source_available(
        requested_source,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
    ) {
        return Some(requested_source);
    }

    ClientTextureSource::ALL.into_iter().find(|candidate| {
        *candidate != requested_source
            && source_available(
                *candidate,
                tex_art_cc_res,
                tex_art_ec_res,
                tex_land_ec_res,
                tilemeta_res,
            )
    })
}

pub fn sys_sync_static_art_source(
    settings: Res<Settings>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    land_cache: Option<Res<crate::core::texture_cache::land::cache::LandTextureCache>>,
    mut source_state: ResMut<StaticArtSourceState>,
) {
    let land_style = land_cache.map_or(StaticLandStyle::Unknown,
        |cache| StaticLandStyle::of_land_atlas(cache.routes().atlas()));
    // Written only on a change, like `warned_unavailable_source` below.
    if source_state.land_style != land_style {
        source_state.land_style = land_style;
    }

    let requested_source = settings.graphics.art_texture_source;
    let effective_source = resolve_effective_art_source(
        requested_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    if effective_source != Some(requested_source)
        && source_state.warned_unavailable_source != Some(requested_source)
    {
        source_state.warned_unavailable_source = Some(requested_source);
        let fallback_label = effective_source
            .map(|source| source.art_label())
            .unwrap_or("none");
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "Requested {} for static sprites, but the required package/metadata is unavailable. Falling back to {}.",
                requested_source.art_label(),
                fallback_label,
            ),
        );
    } else if effective_source == Some(requested_source) && source_state.warned_unavailable_source.is_some() {
        // A redundant write here invalidates asynchronous visibility queries on
        // every frame even though the selected artwork has not changed.
        source_state.warned_unavailable_source = None;
    }

    if source_state.active_source == effective_source {
        return;
    }

    log_system_add_update::<super::DrawStaticSpritesPlugin>(fname!());

    source_state.active_source = effective_source;

    match effective_source {
        Some(source) if source == requested_source => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!("Switched static sprite source to {}.", source.art_label()),
            );
        }
        Some(source) => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!(
                    "Switched static sprite source to {} (requested {}).",
                    source.art_label(),
                    requested_source.art_label(),
                ),
            );
        }
        None => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::RenderWorldArt,
                "No static sprite art package is available; non-dot statics will be skipped.",
            );
        }
    }
}

#[derive(Clone, Copy)]
struct StaticCollectFrameConfig {
    map_id: u32,
    art_source: Option<ClientTextureSource>,
    /// `StaticArtSourceState::land_style`.
    land_style: StaticLandStyle,
    is_dot_mode: bool,
    static_art_local_lights: bool,
    show_hidden_multi_parts: bool,
    height_scale: f32,
    item_routes_revision: u64,
    z_view: ZViewCut,
    caves_revision: u64,
}

#[derive(Clone, Copy)]
struct StaticCollectResources<'a> {
    tex_art_cc: Option<&'a TexArtCcPackageRes>,
    tex_art_ec: Option<&'a TexArtEcPackageRes>,
    tex_land_ec: Option<&'a TexLandEcPackageRes>,
    tilemeta: Option<&'a TileMetaPackageRes>,
    tex_aux_ec: Option<&'a TexAuxEcPackageRes>,
    multis: Option<&'a MultiDefinitionsRes>,
    placed_multis: &'a PlacedMultisRes,
    static_lights: &'a super::static_lights::RenderStaticLightInstances,
    tile_atlas: &'a TileAtlas,
    map_planes: Option<&'a MapPlanesRes>,
    /// The caves drawn over the map, if any.
    caves: Option<&'a CaveMap>,
    /// The player's cave, with the player in one.
    player_cave: Option<&'a PlayerCave>,
    /// The Kingdom Reborn packages, when the land is Kingdom Reborn's (`StaticLandStyle::KingdomReborn`).
    kingdom_reborn: Option<KrStaticSources<'a>>,
}

impl<'a> StaticCollectResources<'a> {
    fn tex_art_cc_package(self) -> Option<&'a udd_assets::tex_art_cc::TexArtCcPackage> {
        self.tex_art_cc.map(|package| package.0.as_ref())
    }

    fn tex_art_ec_package(self) -> Option<&'a udd_assets::tex_art_ec::TexArtEcPackage> {
        self.tex_art_ec.map(|package| package.0.as_ref())
    }

    fn tex_land_ec_package(self) -> Option<&'a udd_assets::tex_land_ec::TexLandEcPackage> {
        self.tex_land_ec.map(|package| package.0.as_ref())
    }

    fn tilemeta_package(self) -> Option<&'a udd_assets::tilemeta::TileMetaPackage> {
        self.tilemeta.map(|package| package.0.as_ref())
    }

    /// The tile data of the statics drawn from `art` (`TileMetaPackageRes::for_art`).
    fn tilemeta_package_for_art(self, art: Option<ClientTextureSource>)
        -> Option<&'a udd_assets::tilemeta::TileMetaPackage> {
        self.tilemeta.map(|package| package.for_art(art).as_ref())
    }

    fn tex_aux_ec_package(self) -> Option<&'a udd_assets::tex_aux_ec::TexAuxEcPackage> {
        self.tex_aux_ec.map(|package| package.0.as_ref())
    }
}

#[derive(Clone, Copy)]
struct StaticTileRenderInfo<'a> {
    tile: StaticRenderTile,
    tilemeta: Option<&'a udd_assets::tilemeta::TileMetaItemTile>,
    world_x: f32,
    world_z: f32,
    depth_class: StaticDepthClass,
    is_wet_flags: u32,
    base_world_y: f32,
    client_shadow_flags: u32,
    /// The tile's height in UO z units above the client's ground below it (`client_shadow_ground_z`).
    client_shadow_height: f32,
    /// `STATIC_FLAG_CLIENT_GRUNGE` when the KR client grunges the item, else 0: or'ed into the sprite's flags.
    client_grunge_flag: u32,
}

#[derive(Default)]
struct StaticChunkBuildState {
    sprite_instances: Vec<SpriteInstance>,
    ground_instances: Vec<GroundTileInstance>,
    pick_tiles: Vec<StaticPickTile>,
    sprite_pick_indices: Vec<(f64, usize)>,
    ground_pick_indices: Vec<(f64, usize)>,
    requested_pages: HashSet<u64>,
    stats: CachedStaticChunkStats,
    render_tiles: Vec<StaticRenderTile>,
    /// The land the roofs under KR's cut are counted from (`ZViewCut::roof_layer`), and the client shadow's ground: the
    /// map package's, which every cell has whether or not its land is loaded.
    own_land: PackageLand,
    /// The cells of the tile atlas, the land as drawn, the chunk read, so a change of them rebuilds the chunk.
    land_cells: Option<AtlasCells>,
}

impl StaticChunkBuildState {
    /// Records a read of the tile atlas's `cells`.
    fn read_land(&mut self, cells: AtlasCells) {
        self.land_cells = Some(self.land_cells.map_or(cells, |read| read.union(cells)));
    }
}

#[derive(Default)]
struct StaticCollectSlowStats {
    visited_blocks: usize,
    source_tiles: usize,
    ground_land_tiles: usize,
    atlas_hits: usize,
    atlas_misses: usize,
    unique_requested_pages: HashSet<u64>,
    copied_sprite_instances: usize,
    copied_ground_instances: usize,
}

impl StaticCollectSlowStats {
    fn absorb_cached_chunk(&mut self, chunk: &CachedStaticChunk) {
        self.visited_blocks += chunk.stats.visited_blocks;
        self.source_tiles += chunk.stats.source_tiles;
        self.ground_land_tiles += chunk.stats.ground_land_tiles;
        self.atlas_hits += chunk.stats.atlas_hits;
        self.atlas_misses += chunk.stats.atlas_misses;
        self.unique_requested_pages
            .extend(chunk.stats.requested_pages.iter().copied());
    }
}

#[derive(Clone, Copy)]
struct StaticCollectCacheDecision {
    visible_set_unchanged: bool,
    all_visible_chunks_complete: bool,
    skip_unchanged_incomplete_retry: bool,
}

fn static_collect_resources<'a>(
    tex_art_cc_res: &'a Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: &'a Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: &'a Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: &'a Option<Res<TileMetaPackageRes>>,
    tex_aux_ec_res: &'a Option<Res<TexAuxEcPackageRes>>,
    multis_res: &'a Option<Res<MultiDefinitionsRes>>,
    placed_multis: &'a PlacedMultisRes,
    static_lights: &'a super::static_lights::RenderStaticLightInstances,
    tile_atlas: &'a TileAtlas,
    map_planes: &'a Option<Res<MapPlanesRes>>,
    caves: &'a CavesRes,
    kingdom_reborn: Option<KrStaticSources<'a>>,
) -> StaticCollectResources<'a> {
    StaticCollectResources {
        tex_art_cc: tex_art_cc_res.as_ref().map(|res| &**res),
        tex_art_ec: tex_art_ec_res.as_ref().map(|res| &**res),
        tex_land_ec: tex_land_ec_res.as_ref().map(|res| &**res),
        tilemeta: tilemeta_res.as_ref().map(|res| &**res),
        tex_aux_ec: tex_aux_ec_res.as_ref().map(|res| &**res),
        multis: multis_res.as_ref().map(|res| &**res),
        placed_multis,
        static_lights,
        tile_atlas,
        map_planes: map_planes.as_ref().map(|res| &**res),
        caves: caves.active(),
        player_cave: caves.player(),
        kingdom_reborn,
    }
}

fn static_chunk_cache_config(
    frame: StaticCollectFrameConfig,
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
    static_lights: &super::static_lights::RenderStaticLightInstances,
) -> StaticChunkCacheConfig {
    StaticChunkCacheConfig {
        map_id: frame.map_id,
        dot_mode: frame.is_dot_mode,
        art_source: frame.art_source,
        land_style: frame.land_style,
        sprite_atlas_mapping_revision: sprite_atlas.0.mapping_revision(),
        ground_atlas_mapping_revision: ground_atlas.0.mapping_revision(),
        static_art_local_light_signature: frame
            .static_art_local_lights
            .then(|| static_light_signature(static_lights, frame.map_id)),
        show_hidden_multi_parts: frame.show_hidden_multi_parts,
        item_routes_revision: frame.item_routes_revision,
        z_view: frame.z_view,
        caves_revision: frame.caves_revision,
    }
}

fn collect_visible_static_chunk_keys(
    map_id: u32,
    chunks_q: &Query<&crate::core::render::scene::world::land::LCMesh>,
) -> Vec<StaticChunkBatchKey> {
    let mut visible_chunk_keys = chunks_q
        .iter()
        .filter(|chunk| chunk.parent_map_id == map_id)
        .map(|chunk| StaticChunkBatchKey {
            map_id,
            gx: chunk.gx,
            gy: chunk.gy,
            scale: chunk.scale,
        })
        .collect::<Vec<_>>();
    visible_chunk_keys.sort_by_key(|key| (key.gy, key.gx, key.scale));
    visible_chunk_keys
}

fn evaluate_static_collect_cache_decision(
    cache: &StaticChunkRenderCache,
    visible_chunk_keys: &[StaticChunkBatchKey],
    config_changed: bool,
    atlas_retry_signature: StaticAtlasRetrySignature,
) -> StaticCollectCacheDecision {
    let visible_set_unchanged =
        !config_changed && cache.last_visible_chunk_keys == visible_chunk_keys;
    let all_visible_chunks_present = visible_chunk_keys
        .iter()
        .all(|chunk_key| cache.chunks.contains_key(chunk_key));
    let all_visible_chunks_complete = all_visible_chunks_present
        && visible_chunk_keys.iter().all(|chunk_key| {
            cache
                .chunks
                .get(chunk_key)
                .is_some_and(|chunk| chunk.stats.atlas_misses == 0)
        });
    let skip_unchanged_incomplete_retry = should_skip_unchanged_incomplete_static_retry(
        cache,
        visible_chunk_keys,
        visible_set_unchanged,
        all_visible_chunks_complete,
        atlas_retry_signature,
    );

    StaticCollectCacheDecision {
        visible_set_unchanged,
        all_visible_chunks_complete,
        skip_unchanged_incomplete_retry,
    }
}

fn try_finish_cached_visible_static_collect(
    frame: StaticCollectFrameConfig,
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_tick: u64,
    decision: StaticCollectCacheDecision,
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
    sprite_output: &RenderStaticInstances,
    ground_output: &RenderStaticLandInstances,
    cache: &mut StaticChunkRenderCache,
    debug_state: &mut StaticArtCollectDebugState,
) -> bool {
    let use_cached_path = (decision.visible_set_unchanged && decision.all_visible_chunks_complete)
        || decision.skip_unchanged_incomplete_retry;
    if !use_cached_path {
        return false;
    }

    let aggregate = aggregate_cached_visible_static_stats(cache, visible_chunk_keys, visible_tick);
    let path = if decision.skip_unchanged_incomplete_retry {
        StaticArtCollectPath::IncompleteWaitFastPath
    } else {
        StaticArtCollectPath::CompleteFastPath
    };
    if decision.all_visible_chunks_complete {
        cache.last_incomplete_retry_atlas_signature = None;
    }
    record_static_collect_path(debug_state, path, 0, 0);
    log_static_collect_stats(
        debug_state,
        StaticArtCollectStats {
            path,
            map_id: frame.map_id,
            dot_mode: frame.is_dot_mode,
            visible_chunks: visible_chunk_keys.len(),
            visited_blocks: aggregate.visited_blocks,
            source_tiles: aggregate.source_tiles,
            ground_land_tiles: aggregate.ground_land_tiles,
            unique_requested_pages: aggregate.requested_pages.len(),
            resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
            pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
            atlas_capacity_pages: sprite_atlas.active_layers as usize
                + ground_atlas.active_layers as usize,
            atlas_hits: aggregate.atlas_hits,
            atlas_misses: aggregate.atlas_misses,
            emitted_instances: sprite_output.0.len() + ground_output.0.len(),
            copied_sprite_instances: 0,
            copied_ground_instances: 0,
        },
    );
    true
}

fn clear_static_collect_outputs(
    sprite_output: &mut RenderStaticInstances,
    ground_output: &mut RenderStaticLandInstances,
    batches: &mut RenderStaticChunkBatches,
) {
    sprite_output.0.clear();
    ground_output.0.clear();
    batches.sprite.clear();
    batches.ground.clear();
}

fn reuse_cached_static_chunk(
    cache: &mut StaticChunkRenderCache,
    chunk_key: StaticChunkBatchKey,
    visible_tick: u64,
) -> bool {
    if let Some(chunk) = cache.chunks.get_mut(&chunk_key) {
        chunk.last_visible_tick = visible_tick;
        chunk.stats.atlas_misses == 0
    } else {
        false
    }
}

fn collect_visible_static_chunks_slow(
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_tick: u64,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    statics_store: &mut LazyStaticsStore,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    sprite_output: &mut RenderStaticInstances,
    ground_output: &mut RenderStaticLandInstances,
    batches: &mut RenderStaticChunkBatches,
    cache: &mut StaticChunkRenderCache,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> StaticCollectSlowStats {
    let mut slow_stats = StaticCollectSlowStats::default();

    for chunk_key in visible_chunk_keys.iter().copied() {
        if !reuse_cached_static_chunk(cache, chunk_key, visible_tick) {
            let chunk = rebuild_static_chunk(
                chunk_key,
                visible_tick,
                frame,
                resources,
                statics_store,
                sprite_atlas,
                ground_atlas,
                surface_resolution_cache,
            );
            cache.chunks.insert(chunk_key, chunk);
        }

        let Some(chunk) = cache.chunks.get(&chunk_key) else {
            continue;
        };

        slow_stats.absorb_cached_chunk(chunk);
        append_cached_static_chunk_outputs(
            chunk_key,
            chunk,
            sprite_output,
            ground_output,
            batches,
            &mut slow_stats,
        );
    }

    // Assign after assembling every visible chunk: restarting at each chunk
    // leaves opaque coplanar sprites dependent on GPU batch submission order.
    assign_sprite_tie_levels(&mut sprite_output.0);
    slow_stats
}

fn rebuild_static_chunk(
    chunk_key: StaticChunkBatchKey,
    visible_tick: u64,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    statics_store: &mut LazyStaticsStore,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> CachedStaticChunk {
    let mut build = StaticChunkBuildState::default();
    let start_gx = chunk_key.gx * CHUNK_STORAGE_BLOCKS_DIM;
    let start_gy = chunk_key.gy * CHUNK_STORAGE_BLOCKS_DIM;
    let end_gx = start_gx + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;
    let end_gy = start_gy + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;

    for gy in start_gy..end_gy {
        for gx in start_gx..end_gx {
            build.stats.visited_blocks += 1;
            // The floors of the blocks west and north of this one, in the farther row of its edge walls
            // (`wall_cap_meets_floor_flag`).
            let mut neighbour_surface_tops = Vec::new();
            for (nx, ny) in [(gx.checked_sub(1), Some(gy)), (Some(gx), gy.checked_sub(1))] {
                let (Some(nx), Some(ny)) = (nx, ny) else {
                    continue;
                };
                if let Ok(tiles) = statics_store.block_tiles(nx, ny) {
                    neighbour_surface_tops.extend(client_shadow_surface_tops(
                        tiles.iter().map(|tile| static_cell_tile(nx, ny, *tile)),
                        resources.tilemeta_package_for_art(frame.art_source),
                    ));
                }
            }
            let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                continue;
            };

            build.stats.source_tiles += tiles.len();
            let block_surface_tops = client_shadow_surface_tops(
                tiles.iter().map(|tile| static_cell_tile(gx, gy, *tile)),
                resources.tilemeta_package_for_art(frame.art_source),
            );

            // Every static draws as itself, whatever its graphic: map statics are never multis (`core::multis`).
            for tile in tiles {
                emit_static_render_tile_on_surfaces(
                    static_cell_tile(gx, gy, *tile),
                    &[&block_surface_tops, &neighbour_surface_tops],
                    frame,
                    resources,
                    sprite_atlas,
                    ground_atlas,
                    surface_resolution_cache,
                    &mut build,
                );
            }

            // The multis placed in the block draw as their parts.
            let mut render_tiles = std::mem::take(&mut build.render_tiles);
            let block_cells = |block: u32| block * MAP_STORAGE_BLOCK_TILE_DIM..(block + 1) * MAP_STORAGE_BLOCK_TILE_DIM;
            for placed_multi in resources.placed_multis.anchored_in(frame.map_id, block_cells(gx), block_cells(gy)) {
                let Some(parts) = resources.multis
                    .and_then(|multis| multis.parts(placed_multi.multi_id, frame.show_hidden_multi_parts))
                else {
                    continue;
                };
                collect_multi_render_tiles(placed_multi, parts, &mut render_tiles);
                // A multi's own parts are the surfaces under its parts; a plain static sits on the block's statics.
                let multi_surface_tops =
                    client_shadow_surface_tops(render_tiles.iter().copied(),
                        resources.tilemeta_package_for_art(frame.art_source));
                for render_tile in &render_tiles {
                    emit_static_render_tile_on_surfaces(
                        *render_tile,
                        &[&block_surface_tops, &neighbour_surface_tops, &multi_surface_tops],
                        frame,
                        resources,
                        sprite_atlas,
                        ground_atlas,
                        surface_resolution_cache,
                        &mut build,
                    );
                }
            }
            build.render_tiles = render_tiles;
        }
    }

    // Classic art under Enhanced land draws the item surfaces that continue the land too (`static_continues_land`).
    let draws_enhanced_surfaces = match frame.art_source {
        Some(ClientTextureSource::Ec) => true,
        Some(ClientTextureSource::Cc) => frame.land_style.is_enhanced(),
        None => false,
    };
    if !frame.is_dot_mode && draws_enhanced_surfaces {
        if let (Some(tilemeta), Some(art)) = (resources.tilemeta_package(), resources.tex_art_ec_package()) {
            if let Some(cells) = super::static_surface::apply_boundary_coverage(
                &mut build.sprite_instances, chunk_key, frame.height_scale,
                statics_store, tilemeta, art, resources.kingdom_reborn.map(|kr| (kr.tilemeta, kr.art)),
                resources.tile_atlas,
            ) {
                build.read_land(cells);
            }
        }
    }
    finish_rebuilt_static_chunk(build, visible_tick)
}

/// Emits `render_tile` over the ground it stands on (`client_shadow_ground_z`): the highest of the `surface_tops` on its
/// cell within its reach, or the land. The `surface_tops` of the cells west and north of it tell whether a floor lies
/// on its top in the farther row (`wall_cap_meets_floor_flag`).
fn emit_static_render_tile_on_surfaces(
    render_tile: StaticRenderTile,
    surface_tops: &[&[((i32, i32), i32)]],
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
    build: &mut StaticChunkBuildState,
) {
    let cell = (render_tile.world_x as i32, render_tile.world_z as i32);
    let all_surface_tops = surface_tops;
    let surface_tops = surface_tops.iter().flat_map(|tops| tops.iter()).filter(|top| top.0 == cell).map(|top| top.1);
    let land_z = build.own_land.land_z(resources.map_planes, frame.map_id, cell.0, cell.1)
        .and_then(|land_z| i8::try_from(land_z).ok());
    let ground_z = client_shadow_ground_z(render_tile.z, land_z, surface_tops);
    let farther_cells = [(cell.0 - 1, cell.1), (cell.0, cell.1 - 1)];
    let farther_floor_at = |top_z: i32| all_surface_tops.iter().flat_map(|tops| tops.iter())
        .any(|top| top.1 == top_z && farther_cells.contains(&top.0));
    emit_static_render_tile(
        render_tile,
        ground_z,
        farther_floor_at,
        frame,
        resources,
        sprite_atlas,
        ground_atlas,
        surface_resolution_cache,
        build,
    );
}

/// The z of the land that covers a floor buried under it on the cell (`x`, `y`) (`caves::is_buried_floor`): the cell's
/// own corner's `land_z`, where the renderer draws that land. Land the z-view's cut leaves out (over `max_ground_z`)
/// and land never drawn (`TileAtlas::land_is_nodraw`, the Classic client's NoDraw land) cover nothing.
fn covering_land_z(land_z: Option<i32>, max_ground_z: i32, tile_atlas: &TileAtlas, x: i32, y: i32) -> Option<i32> {
    let nodraw = || matches!((u32::try_from(x), u32::try_from(y)), (Ok(x), Ok(y)) if tile_atlas.land_is_nodraw(x, y));
    land_z.filter(|&land_z| land_z <= max_ground_z && !nodraw())
}

fn emit_static_render_tile(
    render_tile: StaticRenderTile,
    ground_z: i32,
    farther_floor_at: impl Fn(i32) -> bool,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
    build: &mut StaticChunkBuildState,
) {
    if frame.is_dot_mode && render_tile.z < 10 {
        return;
    }

    let enhanced_package = resources.tilemeta_package();
    let enhanced_tilemeta = enhanced_package.and_then(|meta| meta.item_tile(render_tile.graphic as u32));
    let visual_kind = frame.art_source.filter(|_| !frame.is_dot_mode).map(|source| {
        let kind = resolve_static_visual_kind_cached(source, frame.land_style, render_tile.graphic, enhanced_package,
            enhanced_tilemeta, resources.tex_art_cc_package(), resources.tex_art_ec_package(),
            resources.tex_land_ec_package(), surface_resolution_cache);
        resources.kingdom_reborn.and_then(|kr| kingdom_reborn_static_visual_kind(render_tile.graphic, enhanced_package,
            enhanced_tilemeta, resources.tex_art_ec_package(), resources.tex_land_ec_package(), kr,
            surface_resolution_cache))
            .unwrap_or(kind)
    });
    // From here on the static is drawn by the tile data of the art it is drawn from (`drawn_tile_data`).
    let (tilemeta_package, tilemeta) =
        drawn_tile_data(visual_kind, render_tile.graphic, enhanced_package,
            resources.tilemeta_package_for_art(Some(ClientTextureSource::Cc)), resources.kingdom_reborn);
    if !frame.is_dot_mode && tilemeta.is_some_and(|meta| meta.is_nodraw()) {
        return;
    }
    let (cell_x, cell_y) = (render_tile.world_x as i32, render_tile.world_z as i32);
    let flags = tilemeta.map_or(0, |meta| meta.flags);
    // In the player's cave, and outside it (caves.rs): the cave's statics are drawn whole, whatever the z-view limit.
    let player_cave = resources.player_cave.map(|cave| {
        let land_z = || build.own_land.land_z(resources.map_planes, frame.map_id, cell_x, cell_y);
        cave.static_view(cell_x, cell_y, render_tile.z, flags, tilemeta.map_or(0, |meta| meta.height), land_z)
    });
    // Above the player's floor, with the z-view limit on; picking skips them too, as they are never emitted. A roof
    // under KR's cut reads its own cell's land.
    let own_land_z = || build.own_land.averaged_land_z(resources.map_planes, frame.map_id, cell_x, cell_y);
    if player_cave != Some(CaveStatic::Lifted) && frame.z_view.hides_static_over_land(flags, render_tile.z, own_land_z) {
        return;
    }
    // Over a cave drawn over the map, and in it (caves.rs).
    let cave = player_cave.unwrap_or_else(|| resources.caves.map_or(CaveStatic::Outside, |caves|
        caves.static_view(cell_x, cell_y, render_tile.z)));
    if cave == CaveStatic::Hidden {
        return;
    }
    // Outside the caves, a cave's floor under its cell's land, where that land is drawn, is covered, as the Classic
    // client draws a cell's land after the floors under it: its art, standing upright at one depth, would show through
    // the lower part of a steep land, as teeth along a cave mouth's black opening.
    let covering_land_z = || {
        // The NoDraw test reads the drawn land: its arriving or changing rebuilds the chunk.
        if let (Ok(x), Ok(y)) = (u32::try_from(cell_x), u32::try_from(cell_y)) {
            build.read_land(AtlasCells::new(UVec2::new(x, y), UVec2::new(x, y)));
        }
        covering_land_z(build.own_land.land_z(resources.map_planes, frame.map_id, cell_x, cell_y),
            frame.z_view.max_ground_z, resources.tile_atlas, cell_x, cell_y)
    };
    if player_cave.is_none() && cave == CaveStatic::Outside
        && is_buried_floor(flags, tilemeta.map_or(0, |meta| meta.height), render_tile.z, covering_land_z) {
        return;
    }
    let cave_flags = match cave {
        CaveStatic::Lifted => STATIC_FLAG_CAVE,
        CaveStatic::Outside if player_cave.is_some() => STATIC_FLAG_CAVE_OUTSIDE,
        _ => 0,
    };
    // A static drawn from the Kingdom Reborn packages renders wet by its Kingdom Reborn material.
    let renders_wet = visual_kind.map_or_else(|| static_tile_renders_wet(tilemeta, tilemeta_package,
        u32::from(render_tile.graphic), frame.art_source), |kind|
        static_visual_renders_wet(kind, tilemeta, tilemeta_package, u32::from(render_tile.graphic)));
    let info = StaticTileRenderInfo {
        tile: render_tile,
        tilemeta,
        world_x: render_tile.world_x,
        world_z: render_tile.world_z,
        depth_class: resolve_static_depth_class(tilemeta, renders_wet),
        is_wet_flags: u32::from(renders_wet) | cave_flags
            | super::light_scene::static_light_class_flags(tilemeta.map_or(0, |meta| meta.flags),
                tilemeta.map_or(0, |meta| meta.height))
            | wall_cap_meets_floor_flag(flags, render_tile.z, tilemeta.map_or(0, |meta| meta.height), farther_floor_at)
            | visual_kind.map_or(0, |kind| land_grade_flag(kind, render_tile.graphic, flags, enhanced_package,
                enhanced_tilemeta, resources.tex_art_ec_package(), resources.tex_land_ec_package(),
                surface_resolution_cache)),
        base_world_y: (render_tile.z as f32) * frame.height_scale,
        client_shadow_flags: client_shadow_flags(tilemeta),
        client_shadow_height: (i32::from(render_tile.z) - ground_z) as f32,
        client_grunge_flag: client_grunge_flag(tilemeta_package, render_tile.graphic),
    };

    if frame.is_dot_mode {
        emit_dot_static_tile(info, frame, resources, build);
    } else if let Some(visual_kind) = visual_kind {
        let before = (build.sprite_instances.len(), build.ground_instances.len());
        emit_art_static_tile(
            info,
            frame,
            resources,
            visual_kind,
            sprite_atlas,
            ground_atlas,
            build,
        );
        if before != (build.sprite_instances.len(), build.ground_instances.len()) {
            let index = build.pick_tiles.len();
            build.pick_tiles.push(StaticPickTile { tile: render_tile, is_wet: info.is_wet_flags & 1 != 0 });
            if before.0 < build.sprite_instances.len() {
                let inst = build.sprite_instances.last().unwrap();
                build.sprite_pick_indices.push((static_depth_key(inst.tile_x, inst.tile_y,
                    inst.priority_z_units), index));
            } else {
                let inst = build.ground_instances.last().unwrap();
                build.ground_pick_indices.push((static_depth_key(inst.tile_x, inst.tile_y,
                    inst.priority_z_units), index));
            }
        }
    }
}

/// `STATIC_FLAG_CAP_MEETS_FLOOR` for a wall to the light scene (`static_light_class_flags` of its tile `flags`), at `z`
/// and `height` tall, whose top a floor lies on in the farther row (`farther_floor_at` of the top's z, a Surface's or
/// Bridge's top in the cell west or north of it): the castles' wall-walks. Else 0: a wall under a roof, or under
/// another wall stacked on it, keeps its cap in its row, in front of what stands on its top there.
fn wall_cap_meets_floor_flag(flags: u64, z: i8, height: i8, farther_floor_at: impl Fn(i32) -> bool) -> u32 {
    let light_class = super::light_scene::static_light_class_flags(flags, height);
    if light_class & super::light_scene::STATIC_FLAG_LIGHT_WALL == 0 || height <= 0
        || !farther_floor_at(i32::from(z) + i32::from(height)) {
        return 0;
    }
    STATIC_FLAG_CAP_MEETS_FLOOR
}

/// A packed static's cell as a render tile, for the ground lookup.
fn static_cell_tile(block_x: u32, block_y: u32, tile: uocf::classic::statics::PackedStaticTile) -> StaticRenderTile {
    StaticRenderTile {
        graphic: tile.graphic,
        world_x: (block_x * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.x_offset())) as f32,
        world_z: (block_y * MAP_STORAGE_BLOCK_TILE_DIM + u32::from(tile.y_offset())) as f32,
        z: tile.z,
        hue: tile.hue,
    }
}

/// The tops of the Surface and Bridge tiles among `tiles`, by cell.
fn client_shadow_surface_tops(
    tiles: impl Iterator<Item = StaticRenderTile>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaPackage>,
) -> Vec<((i32, i32), i32)> {
    use uocf::enhanced::tileart::TaeFlag;
    let Some(tilemeta) = tilemeta else {
        return Vec::new();
    };
    let ground_flags = TaeFlag::Surface.bits() | TaeFlag::Bridge.bits();
    tiles
        .filter_map(|tile| {
            let meta = tilemeta.item_tile(u32::from(tile.graphic))?;
            (meta.flags & ground_flags != 0).then(|| {
                ((tile.world_x as i32, tile.world_z as i32), i32::from(tile.z) + i32::from(meta.height))
            })
        })
        .collect()
}

/// The land z of a cell, its own corner's, read from the map package through `land` (`PackageLand`), which holds every
/// cell whether or not its map block is loaded, or was evicted: the lights read the same land however it loaded.
/// `None` off the map, or while the map is not open.
pub(super) fn static_land_z(land: &std::cell::RefCell<PackageLand>, map_planes: Option<&MapPlanesRes>, map_id: u32,
    cell: (i32, i32)) -> Option<i8> {
    let land_z = land.borrow_mut().land_z(map_planes, map_id, cell.0, cell.1)?;
    i8::try_from(land_z).ok()
}

/// The client's ground under a shadow caster at `z` (0x00587770): the land z capped at `z`, or the top of a Surface or
/// Bridge in the cell at most `CLIENT_SHADOW_GROUND_REACH_Z` above `z`, whichever is higher. While the land's block is
/// not loaded, the land counts as level with the caster (a caster on the ground, h = 0).
fn client_shadow_ground_z(z: i8, land_z: Option<i8>, surface_tops: impl Iterator<Item = i32>) -> i32 {
    let z = i32::from(z);
    let land = land_z.map_or(z, |land| i32::from(land).min(z));
    surface_tops
        .filter(|top| *top <= z + CLIENT_SHADOW_GROUND_REACH_Z)
        .fold(land, i32::max)
}

/// `STATIC_FLAG_CLIENT_GRUNGE` when the KR client grunges `graphic`, by item id whatever art draws it.
fn client_grunge_flag(tilemeta: Option<&udd_assets::tilemeta::TileMetaPackage>, graphic: u16) -> u32 {
    if tilemeta.is_some_and(|package| package.item_has_client_grunge(u32::from(graphic))) {
        STATIC_FLAG_CLIENT_GRUNGE
    } else {
        0
    }
}

/// `SpriteInstance::client_grunge_anchor` of a tile: the KR client's base point, the EC billboard anchor.
fn client_grunge_anchor(tile: StaticRenderTile) -> [f32; 2] {
    let (world_x, world_z) = apply_static_world_anchor_translation(tile.world_x, tile.world_z);
    [world_x, world_z]
}

/// `SpriteInstance::client_shadow_flags` for a tile: whether the KR client casts its shadow, and through the wall
/// branch.
fn client_shadow_flags(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> u32 {
    use uocf::enhanced::tileart::TaeFlag;
    let Some(meta) = tilemeta else {
        return 0;
    };
    if meta.flags & CLIENT_SHADOW_EXCLUDED_FLAGS != 0 {
        return 0;
    }
    let wall = if meta.flags & TaeFlag::Wall.bits() != 0 { CLIENT_SHADOW_FLAG_WALL } else { 0 };
    CLIENT_SHADOW_FLAG_CASTS | wall
}

fn emit_dot_static_tile(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    build: &mut StaticChunkBuildState,
) {
    let Some(meta) = info.tilemeta else {
        return;
    };

    let local_light_corners = static_art_point_local_light_corners(
        frame.static_art_local_lights,
        resources.static_lights,
        frame.map_id,
        info.world_x,
        info.world_z,
        info.base_world_y,
    );
    let priority_z_units =
        resolve_priority_z_units(info.tile.z, info.tilemeta);
    let bias = depth_class_y_bias(info.depth_class);
    let encoded_depth_class = info.depth_class.encoded();
    let world_y = info.base_world_y + bias;
    let color = meta.radar_color;

    build.sprite_instances.push(SpriteInstance {
        world_x: info.world_x,
        world_z: info.world_z,
        world_y,
        layer: 0,
        depth_class: encoded_depth_class,
        base_world_y: info.base_world_y,
        uv_min: [0.0, 0.0],
        uv_max: [0.0, 0.0],
        local_min: [0.0, 0.0],
        local_max: [1.0, 1.0],
        tile_x: info.world_x,
        tile_y: info.world_z,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags,
        hue_id: 0,
        hue_flags: 0,
        flat_quad: 0,
        texture_stretch: 0.0,
        material_item_id: 0,
        client_shadow_flags: 0,
        client_shadow_height: 0.0,
        client_grunge_anchor: [0.0; 2],
        local_light_corners,
        color_rgba: [
            color[2] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[0] as f32 / 255.0,
            1.0,
        ],
    });
}

fn emit_art_static_tile(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    visual_kind: StaticVisualKind,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    build: &mut StaticChunkBuildState,
) {
    let bias = if matches!(visual_kind, StaticVisualKind::TexLandEc { .. }) {
        GROUND_ART_Y_BIAS
    } else {
        depth_class_y_bias(info.depth_class)
    };
    let encoded_depth_class = info.depth_class.encoded();
    let priority_z_units =
        resolve_priority_z_units(info.tile.z, info.tilemeta);
    let world_y = info.base_world_y + bias;
    let material_item_id = visual_kind.material_item_id(info.tile.graphic);
    let Some((billboard_source, resolved_sprite, texture_stretch)) = resolve_art_static_sprite(
        visual_kind,
        resources,
        sprite_atlas,
        ground_atlas,
        &mut build.requested_pages,
    ) else {
        return;
    };

    // CC water/shoreline statics (placed as items) must lie flat like the terrain water
    // beneath them, not stand up as billboards. They stay in the sprite pipeline but render a
    // flat XZ quad (see emit_cc_surface_like_flat_static_instance / main.wesl).
    // Gate on IsWet only (cc_static_tile_renders_flat), NOT the broad static_tile_is_surface_like:
    // the latter also matches non-water "surface-like" tiles (floors, elevated roofs) which are
    // upright CC sprite art and must stay billboards. See cc_static_tile_renders_flat.
    // Separately admit tile-sized dry diamonds only on a resident, level map cell.
    let cc_surface_like_flat = matches!(billboard_source, ClientTextureSource::Cc)
        && resolved_sprite.as_ref().is_some_and(|sprite| {
            let cell = UVec2::new(info.world_x as u32, info.world_z as u32);
            build.read_land(AtlasCells::new(cell, cell + UVec2::ONE));
            cc_static_uses_flat_geometry(info.tilemeta, sprite.logical_width, sprite.logical_height,
                info.world_x, info.world_z, info.tile.z, resources.tile_atlas)
        });

    let (anchored_world_x, anchored_world_z) = if cc_surface_like_flat {
        // Flat ground quad: no upright-billboard centering translation (mirrors TexLandEc).
        (info.world_x, info.world_z)
    } else {
        surface_like_static_world_anchor(visual_kind, info.world_x, info.world_z)
    };

    if let Some(resolved) = resolved_sprite {
        build.stats.atlas_hits += 1;
        let (hue_id, hue_flags) = static_hue_payload(info.tile.hue);
        if matches!(visual_kind, StaticVisualKind::TexLandEc { .. }) {
            emit_ground_static_instance(
                info,
                frame,
                resources,
                resolved,
                anchored_world_x,
                anchored_world_z,
                world_y,
                encoded_depth_class,
                priority_z_units,
                texture_stretch,
                match visual_kind { StaticVisualKind::TexLandEc { resolution } => resolution.liquid_payload, _ => None },
                material_item_id,
                hue_id,
                hue_flags,
                build,
            );
        } else if cc_surface_like_flat {
            emit_cc_surface_like_flat_static_instance(
                info,
                frame,
                resources,
                resolved,
                anchored_world_x,
                anchored_world_z,
                world_y,
                encoded_depth_class,
                priority_z_units,
                hue_id,
                hue_flags,
                build,
            );
        } else {
            emit_billboard_static_instance(
                info,
                frame,
                resources,
                resolved,
                billboard_source,
                matches!(visual_kind, StaticVisualKind::EcSurface { .. }),
                texture_stretch,
                material_item_id,
                anchored_world_x,
                anchored_world_z,
                world_y,
                encoded_depth_class,
                priority_z_units,
                hue_id,
                hue_flags,
                build,
            );
        }
    } else {
        build.stats.atlas_misses += 1;
    }
}

/// `requested_pages` flags of the Kingdom Reborn item art and land pages, beside the Enhanced art (1 << 62), land
/// (1 << 63) and missing texture (1 << 61) ones.
const KR_ART_REQUESTED_PAGE_FLAG: u64 = 1 << 60;
const KR_TERRAIN_REQUESTED_PAGE_FLAG: u64 = 1 << 59;

fn resolve_art_static_sprite(
    visual_kind: StaticVisualKind,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    requested_pages: &mut HashSet<u64>,
) -> Option<(ClientTextureSource, Option<ResolvedArtSprite>, f32)> {
    match visual_kind {
        StaticVisualKind::CcRegular {
            art_id,
            fallback_art_id,
        } => {
            let tex_art_cc = resources.tex_art_cc_package()?;
            let resolved_sprite = if let Some(slot) = tex_art_cc.present_slot(art_id as u32) {
                requested_pages.insert(slot.page_index as u64);
                sprite_atlas.resolve_cc(tex_art_cc, art_id)
            } else if fallback_art_id != art_id {
                if let Some(slot) = tex_art_cc.present_slot(fallback_art_id as u32) {
                    requested_pages.insert(slot.page_index as u64);
                }
                sprite_atlas.resolve_cc(tex_art_cc, fallback_art_id)
            } else {
                None
            };

            Some((ClientTextureSource::Cc, resolved_sprite, 0.0))
        }
        StaticVisualKind::TexLandEc { resolution } => {
            let (tex_land_ec, page_key, page_flag): (_, fn(u32) -> ArtPageKey, _) = if resolution.kingdom_reborn {
                (resources.kingdom_reborn?.land, ArtPageKey::KrTerrain, KR_TERRAIN_REQUESTED_PAGE_FLAG)
            } else {
                (resources.tex_land_ec_package()?, ArtPageKey::Terrain, 1u64 << 63)
            };

            if let Some(slot) = tex_land_ec.present_slot(resolution.runtime_slot_id) {
                requested_pages.insert(page_flag | slot.page_index as u64);
            }

            Some((
                ClientTextureSource::Ec,
                ground_atlas.resolve_tex_land_ec_pages(tex_land_ec, resolution.runtime_slot_id, page_key),
                resolution.texture_repetition,
            ))
        }
        StaticVisualKind::EcRegular { art_id } | StaticVisualKind::EcSurface { art_id } => {
            let (tex_art_ec, tilemeta_package, art_id, page_key, page_flag): (_, _, _, fn(u32) -> ArtPageKey, _) =
                if art_id >= KR_ITEM_RECORD_BASE {
                    // The Kingdom Reborn records, drawn from the Kingdom Reborn art or the Enhanced art it leaves them to.
                    let kr = resources.kingdom_reborn?;
                    let art_id = art_id - KR_ITEM_RECORD_BASE;
                    match kr.art_slot(resources.tex_art_ec_package(), art_id) {
                        Some((art, _)) if !std::ptr::eq(art, kr.art) =>
                            (art, Some(kr.tilemeta), art_id, ArtPageKey::Enhanced, 1u64 << 62),
                        _ => (kr.art, Some(kr.tilemeta), art_id, ArtPageKey::KrEnhanced, KR_ART_REQUESTED_PAGE_FLAG),
                    }
                } else {
                    (resources.tex_art_ec_package()?, resources.tilemeta_package(), art_id, ArtPageKey::Enhanced,
                        1u64 << 62)
                };

            if let Some(slot) = tex_art_ec.present_slot(art_id) {
                requested_pages.insert(page_flag | u64::from(slot.page_index));
            }

            let texture_stretch = if matches!(visual_kind, StaticVisualKind::EcSurface { .. }) {
                tilemeta_package
                    .and_then(|package| package.item_texture_refs(art_id).iter()
                        .find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected()))
                    .map(|texture_ref| valid_texture_repetition(texture_ref.texture_stretch))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            Some((ClientTextureSource::Ec, sprite_atlas.resolve_ec_pages(tex_art_ec, art_id, page_key), texture_stretch))
        }
        StaticVisualKind::Missing => {
            requested_pages.insert(1u64 << 61);
            Some((ClientTextureSource::Ec, sprite_atlas.resolve_missing(resources.tex_aux_ec_package()?), 0.0))
        }
    }
}

fn emit_ground_static_instance(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    resolved: ResolvedArtSprite,
    anchored_world_x: f32,
    anchored_world_z: f32,
    world_y: f32,
    encoded_depth_class: u32,
    priority_z_units: f32,
    texture_stretch: f32,
    liquid_payload: Option<u32>,
    material_item_id: u32,
    hue_id: u32,
    hue_flags: u32,
    build: &mut StaticChunkBuildState,
) {
    build.stats.ground_land_tiles += 1;
    let bounds = resolve_surface_like_ground_quad_bounds();
    let local_light_corners = static_art_ground_local_light_corners(
        frame.static_art_local_lights,
        resources.static_lights,
        frame.map_id,
        anchored_world_x,
        anchored_world_z,
        info.base_world_y,
        bounds,
    );
    let (material_payload, material_flags) =
        surface_like_ground_material_payload(liquid_payload);
    let (tile_x, tile_y) = cell_depth_anchor(anchored_world_x, anchored_world_z);
    build.ground_instances.push(GroundTileInstance {
        world_x: anchored_world_x,
        world_z: anchored_world_z,
        world_y,
        layer: resolved.layer,
        depth_class: encoded_depth_class,
        base_world_y: info.base_world_y,
        uv_min: [resolved.uv_min.x, resolved.uv_min.y],
        uv_max: [resolved.uv_max.x, resolved.uv_max.y],
        local_min: [bounds.local_min_x, bounds.local_min_z],
        local_max: [bounds.local_max_x, bounds.local_max_z],
        tile_x,
        tile_y,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags | info.client_grunge_flag,
        texture_stretch,
        hue_id,
        hue_flags,
        material_payload,
        material_flags,
        material_item_id,
        _pad_material: 0,
        client_grunge_anchor: client_grunge_anchor(info.tile),
        local_light_corners,
        color_rgba: [1.0, 1.0, 1.0, 1.0],
    });
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
fn sprite_depth_anchor(flat_quad: u32, world_x: f32, world_z: f32) -> (f32, f32) {
    match flat_quad {
        FLAT_QUAD_EC_CAMERA => apply_static_world_anchor_translation(world_x, world_z),
        FLAT_QUAD_CC_FLAT | FLAT_QUAD_EC_SURFACE => cell_depth_anchor(world_x, world_z),
        _ => (world_x, world_z),
    }
}

fn emit_billboard_static_instance(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    resolved: ResolvedArtSprite,
    billboard_source: ClientTextureSource,
    ec_surface: bool,
    texture_stretch: f32,
    material_item_id: u32,
    anchored_world_x: f32,
    anchored_world_z: f32,
    world_y: f32,
    encoded_depth_class: u32,
    priority_z_units: f32,
    hue_id: u32,
    hue_flags: u32,
    build: &mut StaticChunkBuildState,
) {
    let bounds = if ec_surface {
        StaticBillboardBounds {
            local_min_x: 0.0,
            local_min_y: 0.0,
            local_max_x: 1.0,
            local_max_y: 1.0,
        }
    } else {
        resolve_static_billboard_bounds(
            resolved.source_pixel_ratio,
            resolved.offset_x,
            resolved.offset_y,
            resolved.logical_width,
            resolved.logical_height,
        )
    };
    let flat_art = !ec_surface
        && billboard_source == ClientTextureSource::Cc
        && cc_ground_diamond(info.tilemeta, resolved.logical_width, resolved.logical_height);
    let local_light_corners = if ec_surface {
        static_art_ground_local_light_corners(
            frame.static_art_local_lights,
            resources.static_lights,
            frame.map_id,
            anchored_world_x,
            anchored_world_z,
            info.base_world_y,
            resolve_surface_like_ground_quad_bounds(),
        )
    } else {
        static_art_billboard_local_light_corners(
            frame.static_art_local_lights,
            resources.static_lights,
            frame.map_id,
            anchored_world_x,
            anchored_world_z,
            info.base_world_y,
            bounds,
            flat_art,
        )
    };
    let flat_quad = if ec_surface {
        if ec_static_camera_quad(info.tilemeta) { FLAT_QUAD_EC_CAMERA } else { FLAT_QUAD_EC_SURFACE }
    } else {
        0
    };
    let (tile_x, tile_y) = sprite_depth_anchor(flat_quad, anchored_world_x, anchored_world_z);

    build.sprite_instances.push(SpriteInstance {
        world_x: anchored_world_x,
        world_z: anchored_world_z,
        world_y,
        layer: resolved.layer,
        depth_class: encoded_depth_class,
        base_world_y: info.base_world_y,
        uv_min: [resolved.uv_min.x, resolved.uv_min.y],
        uv_max: [resolved.uv_max.x, resolved.uv_max.y],
        local_min: [bounds.local_min_x, bounds.local_min_y],
        local_max: [bounds.local_max_x, bounds.local_max_y],
        tile_x,
        tile_y,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags | info.client_grunge_flag | if flat_art { STATIC_FLAG_FLAT_ART } else { 0 },
        hue_id,
        hue_flags: hue_flags | if billboard_source == ClientTextureSource::Ec { STATIC_HUE_FLAG_EC_SPRITE } else { 0 },
        flat_quad,
        texture_stretch,
        material_item_id,
        client_shadow_flags: info.client_shadow_flags,
        client_shadow_height: info.client_shadow_height,
        client_grunge_anchor: client_grunge_anchor(info.tile),
        local_light_corners,
        color_rgba: [1.0, 1.0, 1.0, static_sprite_opacity(info.tilemeta, resolved.legacy_art && !ec_surface)],
    });
}

/// Emit a CC surface-like static (shoreline water) as a flat XZ-plane quad within the
/// sprite pipeline, instead of an upright billboard, so the tile lies flat and seams with
/// the terrain water beneath it while still sampling the CC sprite atlas and keeping the
/// `is_wet` water animation. Unlike the EC ground path (plain square texmap on a plain 0..1
/// cell), the CC sprite has the isometric diamond baked into its pixels, so the quad is sized
/// from the sprite's real pixel dimensions and centered (see
/// `resolve_cc_surface_like_flat_quad_bounds`) and laid out on the diagonal
/// `BILLBOARD_RIGHT_XZ` / `FLAT_QUAD_FORWARD_XZ` basis in main.wesl. The `flat_quad` flag
/// drives that flat vertex transform and the `is_ground_art` shading branch in main.wesl.
fn emit_cc_surface_like_flat_static_instance(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    resolved: ResolvedArtSprite,
    anchored_world_x: f32,
    anchored_world_z: f32,
    world_y: f32,
    encoded_depth_class: u32,
    priority_z_units: f32,
    hue_id: u32,
    hue_flags: u32,
    build: &mut StaticChunkBuildState,
) {
    let bounds = resolve_cc_surface_like_flat_quad_bounds(
        resolved.logical_width,
        resolved.logical_height,
    );
    // Anchor the flat diamond feet-first by the sprite's real pixel height (see
    // `cc_surface_like_flat_quad_center_offsets`): for the 44px plain-water sprites this lands
    // exactly on the cell center, so the diagonal basis in main.wesl maps the baked diamond's
    // edge vertices onto the cell corners as before; taller wave sprites extend further
    // up-screen from the same feet anchor instead of being wrongly re-centered. The draw
    // order's anchor (tile_x/tile_y below) is the cell's (`cell_depth_anchor`), as the EC
    // ground path's.
    let (center_offset_x, center_offset_z) =
        cc_surface_like_flat_quad_center_offsets(resolved.logical_height);
    let centered_world_x = anchored_world_x + center_offset_x;
    let centered_world_z = anchored_world_z + center_offset_z;
    let local_light_corners = static_art_cc_flat_local_light_corners(
        frame.static_art_local_lights,
        resources.static_lights,
        frame.map_id,
        centered_world_x,
        centered_world_z,
        info.base_world_y,
        bounds,
    );
    let (tile_x, tile_y) = sprite_depth_anchor(FLAT_QUAD_CC_FLAT, anchored_world_x, anchored_world_z);

    build.sprite_instances.push(SpriteInstance {
        world_x: centered_world_x,
        world_z: centered_world_z,
        world_y,
        layer: resolved.layer,
        depth_class: encoded_depth_class,
        base_world_y: info.base_world_y,
        uv_min: [resolved.uv_min.x, resolved.uv_min.y],
        uv_max: [resolved.uv_max.x, resolved.uv_max.y],
        local_min: [bounds.local_min_x, bounds.local_min_z],
        local_max: [bounds.local_max_x, bounds.local_max_z],
        tile_x,
        tile_y,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags | info.client_grunge_flag,
        hue_id,
        hue_flags,
        flat_quad: FLAT_QUAD_CC_FLAT,
        texture_stretch: 0.0,
        material_item_id: 0,
        client_shadow_flags: info.client_shadow_flags,
        client_shadow_height: info.client_shadow_height,
        client_grunge_anchor: client_grunge_anchor(info.tile),
        local_light_corners,
        color_rgba: [1.0, 1.0, 1.0, static_sprite_opacity(info.tilemeta, resolved.legacy_art)],
    });
}

fn finish_rebuilt_static_chunk(
    mut build: StaticChunkBuildState,
    visible_tick: u64,
) -> CachedStaticChunk {
    sort_static_chunk_instances(&mut build.sprite_instances, &mut build.ground_instances);
    // Match the stable instance sort, including equal-depth emission order.
    let sorted_indices = |mut indices: Vec<(f64, usize)>| {
        indices.sort_by(|a, b| a.0.total_cmp(&b.0));
        indices.into_iter().map(|(_, index)| index).collect()
    };

    let mut requested_pages = build.requested_pages.into_iter().collect::<Vec<_>>();
    requested_pages.sort_unstable();
    build.stats.requested_pages = requested_pages;

    CachedStaticChunk {
        sprite_instances: build.sprite_instances,
        ground_instances: build.ground_instances,
        pick_tiles: build.pick_tiles,
        sprite_pick_indices: sorted_indices(build.sprite_pick_indices),
        ground_pick_indices: sorted_indices(build.ground_pick_indices),
        stats: build.stats,
        last_visible_tick: visible_tick,
        land_cells: build.land_cells,
    }
}

fn sort_static_chunk_instances(
    sprite_instances: &mut [SpriteInstance],
    ground_instances: &mut [GroundTileInstance],
) {
    sprite_instances.sort_by(|a, b| {
        static_depth_key(a.tile_x, a.tile_y, a.priority_z_units)
            .total_cmp(&static_depth_key(b.tile_x, b.tile_y, b.priority_z_units))
    });
    ground_instances.sort_by(|a, b| {
        static_depth_key(a.tile_x, a.tile_y, a.priority_z_units)
            .total_cmp(&static_depth_key(b.tile_x, b.tile_y, b.priority_z_units))
    });
}

fn append_cached_static_chunk_outputs(
    chunk_key: StaticChunkBatchKey,
    chunk: &CachedStaticChunk,
    sprite_output: &mut RenderStaticInstances,
    ground_output: &mut RenderStaticLandInstances,
    batches: &mut RenderStaticChunkBatches,
    slow_stats: &mut StaticCollectSlowStats,
) {
    if !chunk.sprite_instances.is_empty() {
        let start = sprite_output.0.len() as u32;
        let count = chunk.sprite_instances.len() as u32;
        sprite_output.0.extend_from_slice(&chunk.sprite_instances);
        slow_stats.copied_sprite_instances += count as usize;
        batches.sprite.push(StaticChunkBatch {
            key: chunk_key,
            start,
            count,
        });
    }

    if !chunk.ground_instances.is_empty() {
        let start = ground_output.0.len() as u32;
        let count = chunk.ground_instances.len() as u32;
        ground_output.0.extend_from_slice(&chunk.ground_instances);
        slow_stats.copied_ground_instances += count as usize;
        batches.ground.push(StaticChunkBatch {
            key: chunk_key,
            start,
            count,
        });
    }
}

fn finish_static_collect_slow_path(
    frame: StaticCollectFrameConfig,
    visible_chunk_keys: Vec<StaticChunkBatchKey>,
    visible_tick: u64,
    slow_stats: StaticCollectSlowStats,
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
    sprite_output: &RenderStaticInstances,
    ground_output: &RenderStaticLandInstances,
    cache: &mut StaticChunkRenderCache,
    debug_state: &mut StaticArtCollectDebugState,
) {
    cache.last_incomplete_retry_atlas_signature = if slow_stats.atlas_misses > 0 {
        Some(static_atlas_retry_signature(sprite_atlas, ground_atlas))
    } else {
        None
    };
    cache.prune_stale(visible_tick);
    let visible_chunks = visible_chunk_keys.len();
    cache.last_visible_chunk_keys = visible_chunk_keys;

    let path = StaticArtCollectPath::SlowRebuild;
    record_static_collect_path(
        debug_state,
        path,
        slow_stats.copied_sprite_instances,
        slow_stats.copied_ground_instances,
    );
    log_static_collect_stats(
        debug_state,
        StaticArtCollectStats {
            path,
            map_id: frame.map_id,
            dot_mode: frame.is_dot_mode,
            visible_chunks,
            visited_blocks: slow_stats.visited_blocks,
            source_tiles: slow_stats.source_tiles,
            ground_land_tiles: slow_stats.ground_land_tiles,
            unique_requested_pages: slow_stats.unique_requested_pages.len(),
            resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
            pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
            atlas_capacity_pages: sprite_atlas.active_layers as usize
                + ground_atlas.active_layers as usize,
            atlas_hits: slow_stats.atlas_hits,
            atlas_misses: slow_stats.atlas_misses,
            emitted_instances: sprite_output.0.len() + ground_output.0.len(),
            copied_sprite_instances: slow_stats.copied_sprite_instances,
            copied_ground_instances: slow_stats.copied_ground_instances,
        },
    );
}

pub fn sys_collect_visible_statics(
    statics_res: Res<StaticsStoreRes>,
    packages: (Option<Res<TexArtCcPackageRes>>, Option<Res<TexArtEcPackageRes>>,
        Option<Res<TexLandEcPackageRes>>, Option<Res<TileMetaPackageRes>>, Option<Res<TexAuxEcPackageRes>>,
        KrStaticPackages),
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    settings: Res<crate::configs::settings::Settings>,
    scene_state: Res<SceneStateData>,
    view_cuts: (Res<ZViewLimit>, Res<CavesRes>),
    zoom: Res<RenderZoom>,
    multis: (Option<Res<MultiDefinitionsRes>>, Res<PlacedMultisRes>),
    map_planes: Option<Res<MapPlanesRes>>,
    mut outputs: (
        ResMut<RenderStaticInstances>,
        ResMut<RenderStaticLandInstances>,
        ResMut<RenderStaticChunkBatches>,
        ResMut<StaticChunkRenderCache>,
        ResMut<StaticSurfaceLikeResolutionCache>,
    ),
    mut debug_state: ResMut<StaticArtCollectDebugState>,
    source_state: Res<StaticArtSourceState>,
    static_lights: Res<super::static_lights::RenderStaticLightInstances>,
    // TODO: Need a way to get the currently visible chunks from the terrain system.
    // We can iterate over existing land chunk entities to find which blocks to draw.
    terrain: (Query<&crate::core::render::scene::world::land::LCMesh>, Res<TileAtlas>),
) {
    let (tex_art_cc_res, tex_art_ec_res, tex_land_ec_res, tilemeta_res, tex_aux_ec_res, kr_packages) = packages;
    let (chunks_q, tile_atlas) = terrain;
    let (z_view, caves) = view_cuts;
    let (multis_res, placed_multis) = multis;
    if !settings.world_rendering.enable_statics {
        let (sprite_output, ground_output, batches, cache, _) = &mut outputs;
        clear_static_collect_outputs(sprite_output, ground_output, batches);
        // Re-enabling must rebuild the output lists, even if the visible chunks
        // and their cached instances are unchanged.
        cache.last_visible_chunk_keys.clear();
        cache.last_incomplete_retry_atlas_signature = None;
        debug_state.last = None;
        return;
    }

    let map_id = scene_state.map_id;
    let requested_art_source = settings.graphics.art_texture_source;
    let art_source = resolve_effective_art_source(
        requested_art_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        tilemeta_res.as_ref(),
    )
    .or(source_state.active_source);

    let is_dot_mode = zoom.0 >= 20.0;
    // Height conversion factor from UO units to our world Y units.
    // The old arbitrary value was roughly 1 unit = 0.1 world units.
    // The land shader uses the same calibrated UO_Z_TO_WORLD_SCALE.
    let height_scale = UO_Z_TO_WORLD_SCALE;
    let frame = StaticCollectFrameConfig {
        map_id,
        art_source,
        land_style: source_state.land_style,
        is_dot_mode,
        static_art_local_lights: settings.world_rendering.enable_static_art_local_lights,
        show_hidden_multi_parts: settings.world_rendering.show_hidden_multi_parts,
        height_scale,
        // A change of the route file's item routes rebuilds the cached chunks from the blocks read again.
        item_routes_revision: statics_res.0.get(map_id as usize).and_then(|store| store.as_ref())
            .map_or(0, |store| store.lock().item_routes_revision()),
        z_view: z_view.0,
        caves_revision: caves.revision(),
    };
    let resources = static_collect_resources(
        &tex_art_cc_res,
        &tex_art_ec_res,
        &tex_land_ec_res,
        &tilemeta_res,
        &tex_aux_ec_res,
        &multis_res,
        &placed_multis,
        &static_lights,
        &tile_atlas,
        &map_planes,
        &caves,
        kr_packages.sources().filter(|_| frame.land_style == StaticLandStyle::KingdomReborn),
    );
    let cache_config =
        static_chunk_cache_config(frame, &sprite_atlas, &ground_atlas, &static_lights);
    let visible_tick = outputs.3.begin_frame();
    let config_changed = outputs.3.sync_config(cache_config);
    debug_state.land_invalidated_chunks += outputs.3.invalidate_changed_land(&tile_atlas) as u64;

    let visible_chunk_keys = collect_visible_static_chunk_keys(map_id, &chunks_q);
    let atlas_retry_signature = static_atlas_retry_signature(&sprite_atlas, &ground_atlas);
    let cache_decision = evaluate_static_collect_cache_decision(
        &outputs.3,
        &visible_chunk_keys,
        config_changed,
        atlas_retry_signature,
    );
    if try_finish_cached_visible_static_collect(
        frame,
        &visible_chunk_keys,
        visible_tick,
        cache_decision,
        &sprite_atlas,
        &ground_atlas,
        &outputs.0,
        &outputs.1,
        &mut outputs.3,
        &mut debug_state,
    ) {
        return;
    }

    clear_static_collect_outputs(&mut outputs.0, &mut outputs.1, &mut outputs.2);

    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();
    let slow_stats = collect_visible_static_chunks_slow(
        &visible_chunk_keys,
        visible_tick,
        frame,
        resources,
        &mut statics_store,
        &mut sprite_atlas,
        &mut ground_atlas,
        &mut outputs.0,
        &mut outputs.1,
        &mut outputs.2,
        &mut outputs.3,
        &mut outputs.4,
    );
    finish_static_collect_slow_path(
        frame,
        visible_chunk_keys,
        visible_tick,
        slow_stats,
        &sprite_atlas,
        &ground_atlas,
        &outputs.0,
        &outputs.1,
        &mut outputs.3,
        &mut debug_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(left: f32, right: f32) {
        assert!((left - right).abs() < 1.0e-6, "left={left} right={right}");
    }

    fn item_tile_with_flags(
        flags: u64,
        visual_kind: udd_assets::tilemeta::TileMetaItemVisualKind,
    ) -> udd_assets::tilemeta::TileMetaItemTile {
        let mut tile = udd_assets::tilemeta::TileMetaItemTile::zeroed();
        tile.flags = flags;
        tile.set_visual_kind(visual_kind);
        tile
    }

    /// A tilemeta package of `items`, with no light, and the tilemeta overrides `record` when given.
    fn tilemeta_package(items: &[udd_assets::tilemeta::TileMetaItemTile],
        record: Option<&udd_assets::tilemeta::TileMetaOverridesRecord>) -> udd_assets::tilemeta::TileMetaPackage {
        use udd_assets::tilemeta::*;
        let refs = encode_item_texture_refs(&[]);
        let lights = encode_item_lights(&[]);
        let definitions = TileMetaItemDefinitions::default().to_bytes().unwrap();
        let tileart = TileMetaTileArtRecords::default().to_bytes();
        let record = record.map(TileMetaOverridesRecord::to_bytes);
        let mut builder = udd_container::UddpBuilder::new(udd_container::LookupMode::VirtualPathHash);
        for (path, data) in [
            (TILEMETA_LAND_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_ENTRY_PATH, bytemuck::cast_slice(items)),
            (TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, b"".as_slice()),
            (TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, refs.as_slice()),
            (TILEMETA_ITEM_MATERIAL_ENTRY_PATH, b"{}".as_slice()),
            (TILEMETA_ITEM_DEFINITION_ENTRY_PATH, definitions.as_slice()),
            (TILEMETA_ITEM_LIGHT_ENTRY_PATH, lights.as_slice()),
            (TILEMETA_TILEART_RECORD_ENTRY_PATH, tileart.as_slice()),
        ].into_iter().chain(record.as_deref().map(|record| (TILEMETA_OVERRIDES_ENTRY_PATH, record))) {
            builder.add_file(udd_container::AddFileRequest {
                data_type: udd_container::DataType::Metadata as u8,
                compression: udd_container::CompressionFlag::ZstdNoDict,
                width: 0, height: 0, virtual_path: Some(path), path_hash64: None, id: None, data,
            }).unwrap();
        }
        TileMetaPackage::from_uddp_package(udd_container::UddpReader::open(builder.build().unwrap()).unwrap()).unwrap()
    }

    /// The tilemeta overrides were measured against the Enhanced and Kingdom Reborn art: a static drawn with Classic
    /// art is drawn by the clients' flags, one drawn with Enhanced art, or as a dot, by the overridden ones.
    #[test]
    fn statics_drawn_with_classic_art_keep_the_clients_tile_data() {
        const SWAMP: u16 = 0x3236;
        let mut items = (0..=u32::from(SWAMP))
            .map(|tile_id| udd_assets::tilemeta::TileMetaItemTile { tile_id, ..Zeroable::zeroed() })
            .collect::<Vec<_>>();
        items[usize::from(SWAMP)].flags = TILE_FLAG_SURFACE;
        let record = udd_assets::tilemeta::TileMetaOverridesRecord { digest: 1,
            original_flags: vec![(u32::from(SWAMP), 0)], original_lights: Vec::new() };
        let tables = TileMetaPackageRes::new(std::sync::Arc::new(tilemeta_package(&items, Some(&record)))).unwrap();
        let surface = |kind| drawn_tile_data(kind, SWAMP, Some(tables.0.as_ref()),
            Some(tables.for_art(Some(ClientTextureSource::Cc)).as_ref()), None)
            .1.map(|item| item.flags & TILE_FLAG_SURFACE != 0);
        assert_eq!(surface(Some(classic_static_visual_kind(SWAMP, None))), Some(false));
        for kind in [Some(StaticVisualKind::EcSurface { art_id: u32::from(SWAMP) }),
            Some(StaticVisualKind::EcRegular { art_id: u32::from(SWAMP) }), Some(StaticVisualKind::Missing), None] {
            assert_eq!(surface(kind), Some(true), "{kind:?}");
        }
        for art in [Some(ClientTextureSource::Ec), None] {
            assert!(std::sync::Arc::ptr_eq(tables.for_art(art), &tables.0));
        }
        // A package built without the overrides has one set of tables.
        let plain = TileMetaPackageRes::new(std::sync::Arc::new(tilemeta_package(&items, None))).unwrap();
        assert!(std::sync::Arc::ptr_eq(plain.for_art(Some(ClientTextureSource::Cc)), &plain.0));
    }

    #[test]
    fn the_client_shadow_ground_is_the_land_or_a_surface_within_reach() {
        const CASTER_Z: i8 = 20;
        // The land below: the caster is that far above it.
        assert_eq!(client_shadow_ground_z(CASTER_Z, Some(5), std::iter::empty()), 5);
        // Land above the caster is capped at the caster: h = 0.
        assert_eq!(client_shadow_ground_z(CASTER_Z, Some(30), std::iter::empty()), 20);
        // A surface top up to 4 above the caster counts; one higher does not.
        assert_eq!(client_shadow_ground_z(CASTER_Z, Some(5), [24].into_iter()), 24);
        assert_eq!(client_shadow_ground_z(CASTER_Z, Some(5), [25].into_iter()), 5);
        assert_eq!(client_shadow_ground_z(CASTER_Z, Some(5), [10, 18, 2].into_iter()), 18);
        // While the land block is not loaded the caster stands on the ground.
        assert_eq!(client_shadow_ground_z(CASTER_Z, None, std::iter::empty()), 20);
    }

    #[test]
    fn only_a_wall_with_a_floor_on_its_top_in_the_farther_row_moves_its_cap() {
        use uocf::enhanced::tileart::TaeFlag;
        // Britain castle's wall-walk (1321,1618): a wall at z 72, 22 tall, under cobblestones at z 94 in (1320,1618).
        const WALL_Z: i8 = 72;
        const WALL_HEIGHT: i8 = 22;
        const TOP_Z: i32 = WALL_Z as i32 + WALL_HEIGHT as i32;
        let wall = TaeFlag::Wall.bits();
        let floor_on_top = |top_z: i32| top_z == TOP_Z;
        assert_eq!(wall_cap_meets_floor_flag(wall, WALL_Z, WALL_HEIGHT, floor_on_top), STATIC_FLAG_CAP_MEETS_FLOOR);
        // A wall under a roof or under another wall stacked on it: no floor at its top.
        assert_eq!(wall_cap_meets_floor_flag(wall, WALL_Z, WALL_HEIGHT, |_| false), 0);
        // Not walls to the light scene, or with no top.
        assert_eq!(wall_cap_meets_floor_flag(wall | TaeFlag::Roof.bits(), WALL_Z, WALL_HEIGHT, floor_on_top), 0);
        assert_eq!(wall_cap_meets_floor_flag(TaeFlag::Impassable.bits(), WALL_Z, WALL_HEIGHT, floor_on_top), 0);
        assert_eq!(wall_cap_meets_floor_flag(wall, TOP_Z as i8, 0, floor_on_top), 0);
    }

    #[test]
    fn the_client_shadow_casters_follow_the_tileart_flags() {
        use udd_assets::tilemeta::TileMetaItemVisualKind;
        use uocf::enhanced::tileart::TaeFlag;
        let casts = |flags: u64| client_shadow_flags(Some(&item_tile_with_flags(flags, TileMetaItemVisualKind::RegularArt)));
        assert_eq!(casts(0), CLIENT_SHADOW_FLAG_CASTS);
        assert_eq!(casts(TaeFlag::Foliage.bits()), CLIENT_SHADOW_FLAG_CASTS);
        assert_eq!(casts(TaeFlag::Wall.bits()), CLIENT_SHADOW_FLAG_CASTS | CLIENT_SHADOW_FLAG_WALL);
        for excluded in [TaeFlag::Surface, TaeFlag::Roof, TaeFlag::Rotated, TaeFlag::NoShadow] {
            assert_eq!(casts(excluded.bits() | TaeFlag::Wall.bits()), 0);
        }
        // Bit 14 is the classic ArticleA in our tilemeta, not KR's Rotated: it still casts.
        assert_eq!(casts(TaeFlag::ArticleA.bits()), CLIENT_SHADOW_FLAG_CASTS);
        assert_eq!(client_shadow_flags(None), 0);
    }

    fn texture_ref_with_role(
        texture_id: u32,
        role: udd_assets::tilemeta::EcMaterialStableRole,
        flags: u8,
    ) -> udd_assets::tilemeta::TileMetaItemTextureRef {
        udd_assets::tilemeta::TileMetaItemTextureRef {
            texture_id,
            texture_type: udd_assets::tilemeta::EcMaterialLogicalFamily::WorldArt as u8,
            stable_role: role as u8,
            flags,
            ..udd_assets::tilemeta::TileMetaItemTextureRef::zeroed()
        }
    }

    fn test_surface_like_resolution() -> SurfaceLikeTexLandEcResolution {
        SurfaceLikeTexLandEcResolution {
            runtime_slot_id: 42,
            texture_repetition: 1.0,
            liquid_payload: None,
            kingdom_reborn: false,
        }
    }

    #[test]
    fn surface_like_resolution_cache_stores_hits_and_misses() {
        let mut cache = StaticSurfaceLikeResolutionCache::default();
        let hit_key = StaticSurfaceLikeResolutionCacheKey {
            art_source: static_surface_like_resolution_art_source_tag(ClientTextureSource::Ec),
            tile_id: 7,
        };
        let miss_key = StaticSurfaceLikeResolutionCacheKey {
            art_source: static_surface_like_resolution_art_source_tag(ClientTextureSource::Ec),
            tile_id: 8,
        };
        let resolution = test_surface_like_resolution();
        let mut hit_resolves = 0;
        let mut miss_resolves = 0;

        assert_eq!(
            cache.get_or_insert_with(hit_key, || {
                hit_resolves += 1;
                Some(resolution)
            }),
            Some(resolution)
        );
        assert_eq!(cache.get_or_insert_with(hit_key, || None), Some(resolution));
        assert_eq!(hit_resolves, 1);

        assert_eq!(
            cache.get_or_insert_with(miss_key, || {
                miss_resolves += 1;
                None
            }),
            None
        );
        assert_eq!(
            cache.get_or_insert_with(miss_key, || Some(resolution)),
            None
        );
        assert_eq!(miss_resolves, 1);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn classic_billboard_offsets_use_the_calibrated_pixel_scale() {
        let bounds = resolve_static_billboard_bounds(1.0, 11, 7, 44.0, 88.0);

        approx_eq(bounds.local_min_x, 11.0 * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_max_x, (11.0 + 44.0) * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_min_y, -(7.0 * CC_WORLD_Y_PER_PIXEL));
        approx_eq(bounds.local_max_y, (88.0 - 7.0) * CC_WORLD_Y_PER_PIXEL);
    }

    #[test]
    fn upright_art_projects_square_pixels_and_four_pixels_per_z_unit() {
        let vertical_projection = (2.0f32 / 3.0).sqrt()
            * crate::core::render::scene::camera::ORTHO_WIDTH_SCALE_FACTOR;
        for ratio in [1.0, 2.0 / 3.0, 0.8 / 1.5, 1.2 / 1.5] {
            approx_eq(static_world_y_per_pixel(ratio) * vertical_projection,
                static_world_xz_per_pixel(ratio));
        }
        approx_eq(UO_Z_TO_WORLD_SCALE / CC_WORLD_Y_PER_PIXEL, 4.0);
    }

    #[test]
    fn enhanced_billboard_bounds_normalize_to_classic_world_footprint() {
        let cc_bounds = resolve_static_billboard_bounds(1.0, 0, 0, 44.0, 44.0);
        let ec_bounds = resolve_static_billboard_bounds(2.0 / 3.0, 0, 0, 66.0, 66.0);

        approx_eq(
            ec_bounds.local_max_x - ec_bounds.local_min_x,
            cc_bounds.local_max_x - cc_bounds.local_min_x,
        );
        approx_eq(
            ec_bounds.local_max_y - ec_bounds.local_min_y,
            cc_bounds.local_max_y - cc_bounds.local_min_y,
        );
    }

    #[test]
    fn enhanced_billboard_offsets_use_enhanced_pixel_ratio() {
        let bounds = resolve_static_billboard_bounds(2.0 / 3.0, 66, 33, 66.0, 66.0);

        approx_eq(bounds.local_min_x, ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS);
        approx_eq(bounds.local_min_y, -(22.0 * CC_WORLD_Y_PER_PIXEL));
    }

    /// The clients' WorldArt rule (UOSA 0x00459b00/0x004598d0, UOKR 0x00442430/0x004422b0), in Classic pixels from the
    /// tile's south vertex: the sprite's left edge at `-22 + x_off * s / 1.5`, its bottom at `y_off * s / 1.5`. A
    /// Classic sprite stands centred on the south vertex. Measured over 7015 scale-1 items, the two silhouettes then
    /// coincide within a pixel (docs/agent_notes/ec_sprite_descriptor_geometry.md).
    #[test]
    fn enhanced_sprites_stand_where_the_clients_place_world_art() {
        // Pixel quantities tens of pixels large carry a few ulps of f32 error.
        const PIXEL_EPSILON: f32 = 1.0e-4;
        let approx_eq = |left: f32, right: f32| assert!((left - right).abs() < PIXEL_EPSILON, "left={left} right={right}");
        let screen_right_pixels = |world_x: f32, world_z: f32| (Vec2::new(world_x, world_z)
            .dot(STATIC_BILLBOARD_RIGHT_XZ)) / CC_WORLD_XZ_PER_PIXEL;
        let screen_forward = |world_x: f32, world_z: f32| world_x + world_z;
        let (ec_x, ec_z) = apply_static_world_anchor_translation(0.0, 0.0);
        let (cc_x, cc_z) = apply_cc_static_world_anchor_translation(0.0, 0.0);
        // The Enhanced base point is the west vertex's column on the south vertex's row.
        approx_eq(screen_right_pixels(ec_x, ec_z) - screen_right_pixels(cc_x, cc_z), -CC_TILE_PIXEL_WIDTH / 2.0);
        approx_eq(screen_forward(ec_x, ec_z), screen_forward(cc_x, cc_z));
        // A log wall's KR art (x_off 22, y_off 0, 46 x 104 at scale 1) against a scale-0.8 item's (x_off 7, y_off -24).
        for (scale, x_off, y_off) in [(1.0f32, 22, 0), (0.8, 7, -24)] {
            let ratio = scale / udd_assets::tex_art_ec::LEGACY_PIXEL_UNIT;
            let bounds = resolve_static_billboard_bounds(ratio, x_off, y_off, 46.0, 104.0);
            let left_pixels = screen_right_pixels(ec_x, ec_z) - screen_right_pixels(cc_x, cc_z)
                + bounds.local_min_x / CC_WORLD_XZ_PER_PIXEL;
            approx_eq(left_pixels, -CC_TILE_PIXEL_WIDTH / 2.0 + x_off as f32 * ratio);
            approx_eq(-bounds.local_min_y / CC_WORLD_Y_PER_PIXEL, y_off as f32 * ratio);
        }
    }

    #[test]
    fn surface_like_ground_quad_bounds_keep_full_tile_coverage() {
        let bounds = resolve_surface_like_ground_quad_bounds();

        approx_eq(bounds.local_min_x, 0.0);
        approx_eq(bounds.local_max_x, 1.0);
        approx_eq(bounds.local_min_z, 0.0);
        approx_eq(bounds.local_max_z, 1.0);
    }

    #[test]
    fn surface_like_land_art_keeps_raw_world_anchor() {
        let anchor = surface_like_static_world_anchor(
            StaticVisualKind::TexLandEc {
                resolution: test_surface_like_resolution(),
            },
            10.0,
            20.0,
        );

        approx_eq(anchor.0, 10.0);
        approx_eq(anchor.1, 20.0);
    }

    #[test]
    fn dry_cc_picking_and_rendering_require_all_four_resident_level_corners() {
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u};
        let mut tile = item_tile_with_flags(TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        tile.height = 0;
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(2),
            max_layers: 1, world_pages_x: 1, ..Default::default() }, 1);
        let flat = |tile: &_, atlas: &_| cc_static_uses_flat_geometry(Some(tile), 44.0, 44.0, 0.0, 0.0, -7, atlas);
        assert!(!flat(&tile, &atlas));
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        atlas.cpu_mirror[layer as usize] = Some(vec![Rg16u::pack(3, -7, 2, false, 0); 4]);
        assert!(flat(&tile, &atlas));
        atlas.cpu_mirror[layer as usize].as_mut().unwrap()[3] = Rg16u::pack(3, -6, 2, false, 0);
        assert!(!flat(&tile, &atlas));
        atlas.cpu_mirror[layer as usize].as_mut().unwrap()[3] = Rg16u::pack(3, -7, 3, false, 0);
        assert!(!flat(&tile, &atlas));
        tile.flags |= TILE_FLAG_WET;
        assert!(flat(&tile, &atlas));
    }

    /// A floor buried under its cell's land is covered only by land the renderer draws: not by land over the z-view's
    /// cut, nor by NoDraw land, whose fragments are discarded.
    #[test]
    fn only_drawn_land_covers_a_buried_floor() {
        use crate::core::render::scene::world::caves::CAVE_MIN_BURY_DEPTH;
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u, LAND_SOURCE_CC_ATLAS,
            LAND_SOURCE_NODRAW, LAND_NODRAW_ID};
        use crate::core::render::scene::world::art::light_cover::NO_Z_VIEW_LIMIT;
        use uocf::enhanced::tileart::TaeFlag;
        const DRAWN_LAND_ID: u16 = 3;
        let floor = TaeFlag::Surface.bits();
        let land_z = CAVE_MIN_BURY_DEPTH;
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(2),
            max_layers: 1, world_pages_x: 1, ..Default::default() }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        let drawn = Rg16u::pack(DRAWN_LAND_ID, land_z as i8, LAND_SOURCE_CC_ATLAS as u16, false, 0);
        let nodraw = Rg16u::pack(LAND_NODRAW_ID as u16, land_z as i8, LAND_SOURCE_NODRAW as u16, false, 0);
        atlas.cpu_mirror[layer as usize] = Some(vec![drawn, nodraw, drawn, drawn]);
        let buried = |atlas: &TileAtlas, max_ground_z: i32, x: i32| is_buried_floor(floor, 0, 0,
            || covering_land_z(Some(land_z), max_ground_z, atlas, x, 0));
        assert!(buried(&atlas, NO_Z_VIEW_LIMIT, 0));
        assert!(!buried(&atlas, NO_Z_VIEW_LIMIT, 1));
        assert!(!buried(&atlas, land_z - 1, 0));
        // A cell the atlas does not hold yet is taken as drawn; the chunk is built again once it does
        // (`StaticChunkCacheConfig::land_metadata_revision`).
        assert!(buried(&TileAtlas::new(AtlasParams::default(), 1), NO_Z_VIEW_LIMIT, 1));
    }

    #[test]
    fn cc_flat_quad_center_offsets_track_sprite_height() {
        // 44px plain water: exactly the cell center — the previously verified placement.
        let (x, z) = cc_surface_like_flat_quad_center_offsets(44.0);
        approx_eq(x, 0.5);
        approx_eq(z, 0.5);

        // 88px (two-cell breaking-wave scale): feet stay at the near vertex, so the center
        // pulls back to the cell origin instead of being wrongly re-centered on the cell.
        let (x, z) = cc_surface_like_flat_quad_center_offsets(88.0);
        approx_eq(x, 0.0);
        approx_eq(z, 0.0);
    }

    #[test]
    fn regular_static_art_keeps_sprite_world_anchor() {
        let anchor = surface_like_static_world_anchor(
            StaticVisualKind::CcRegular {
                art_id: 42,
                fallback_art_id: 42,
            },
            10.0,
            20.0,
        );

        approx_eq(anchor.0, 10.0 + CC_STATIC_TILE_TRANSLATION_X);
        approx_eq(anchor.1, 20.0 + CC_STATIC_TILE_TRANSLATION_Z);
    }

    #[test]
    fn cc_visual_kind_prefers_owner_slot_with_texture_fallback() {
        let mut tile = item_tile_with_flags(
            0,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );
        tile.cc_texture_id = 99;

        let visual_kind =
            resolve_static_visual_kind(ClientTextureSource::Cc, 7, None, Some(&tile), None, None);

        assert_eq!(
            visual_kind,
            StaticVisualKind::CcRegular {
                art_id: 0x4007,
                fallback_art_id: 0x4063,
            }
        );
    }

    #[test]
    fn classic_fallback_preserves_high_graphics_and_actual_source_wetness() {
        let tile = item_tile_with_flags(TILE_FLAG_WET | uocf::enhanced::tileart::TaeFlag::Translucent.bits(),
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        let classic = classic_static_visual_kind(60_000, Some(&tile));
        assert!(matches!(classic, StaticVisualKind::CcRegular { art_id: 76_384, .. }));
        assert_eq!(surface_like_static_world_anchor(classic, 10.0, 20.0), (11.0, 21.0));
        assert!(static_visual_renders_wet(classic, Some(&tile), None, 60_000));
        assert!(!static_visual_renders_wet(StaticVisualKind::Missing, Some(&tile), None, 60_000));
        assert_eq!(resolve_static_visual_kind_cached(ClientTextureSource::Ec, StaticLandStyle::Unknown, 60_000,
            None, Some(&tile), None, None, None, &mut StaticSurfaceLikeResolutionCache::default()),
            StaticVisualKind::Missing);
    }

    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn canonical_art_inventory_uses_kr_then_classic_then_missing() {
        let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let cc = udd_assets::tex_art_cc::TexArtCcPackage::load(root.join("tex_art_cc.uddp")).unwrap();
        let ec = udd_assets::tex_art_ec::TexArtEcPackage::load(root.join("tex_art_ec.uddp")).unwrap();
        let land = udd_assets::tex_land_ec::TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
        let meta = udd_assets::tilemeta::TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
        let mut cache = StaticSurfaceLikeResolutionCache::default();
        let (mut world, mut classic) = (0, 0);
        for tile in meta.item_tiles().iter().filter(|t| !t.is_nodraw()) {
            let Ok(graphic) = u16::try_from(tile.tile_id) else { continue; };
            let selected = resolve_static_visual_kind_cached(ClientTextureSource::Ec, StaticLandStyle::Unknown, graphic,
                Some(&meta), Some(tile), Some(&cc), Some(&ec), Some(&land), &mut cache);
            if ec.present_slot(tile.tile_id).is_some() {
                assert!(!matches!(selected, StaticVisualKind::CcRegular { .. } | StaticVisualKind::Missing));
                world += 1;
            } else {
                let owner = tile.tile_id + CLASSIC_STATIC_ART_ID_OFFSET;
                let alias = tile.cc_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET);
                if cc.present_slot(owner).is_some() || cc.present_slot(alias).is_some() {
                    assert_eq!(selected, classic_static_visual_kind(graphic, Some(tile)));
                    classic += 1;
                } else { assert_eq!(selected, StaticVisualKind::Missing); }
            }
        }
        assert!(world > 0 && classic > 0,
            "inventory did not exercise both artwork sources: KR={world}, Classic={classic}");
    }

    #[test]
    fn statics_continue_the_land_when_enhanced_draws_them_from_the_terrain_and_they_are_not_built_floors() {
        let liquid = SurfaceLikeTexLandEcResolution { liquid_payload: Some(7), ..test_surface_like_resolution() };
        let liquid = StaticVisualKind::TexLandEc { resolution: liquid };
        let surface = StaticVisualKind::EcSurface { art_id: 0x1782 };
        let dry_land = StaticVisualKind::TexLandEc { resolution: test_surface_like_resolution() };
        let nature = item_tile_with_flags(TILE_FLAG_SURFACE, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        let floor = item_tile_with_flags(TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE | TILE_FLAG_NO_SHOOT,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        // A land liquid always continues the land, whatever else.
        assert!(static_continues_land(liquid, Some(&floor), false));
        // A surface continues it when it draws a terrain texture, unless it is a built floor.
        for kind in [surface, dry_land] {
            assert!(static_continues_land(kind, Some(&nature), true), "{kind:?}");
            assert!(!static_continues_land(kind, Some(&nature), false), "{kind:?}");
            assert!(!static_continues_land(kind, Some(&floor), true), "{kind:?}");
        }
        for other in [StaticVisualKind::EcRegular { art_id: 0x1797 }, StaticVisualKind::Missing,
            classic_static_visual_kind(0x1797, Some(&nature))] {
            assert!(!static_continues_land(other, Some(&nature), true), "{other:?}");
        }
    }

    #[test]
    fn kingdom_reborn_kinds_read_the_kingdom_reborn_records() {
        const GRASS: u16 = 0x1782;
        let surface = StaticVisualKind::EcSurface { art_id: u32::from(GRASS) };
        assert!(!surface.is_kingdom_reborn());
        assert_eq!(surface.material_item_id(GRASS), u32::from(GRASS));
        let kr = surface.kingdom_reborn();
        assert_eq!(kr, StaticVisualKind::EcSurface { art_id: u32::from(GRASS) + KR_ITEM_RECORD_BASE });
        assert!(kr.is_kingdom_reborn());
        assert_eq!(kr.material_item_id(GRASS), u32::from(GRASS) + KR_ITEM_RECORD_BASE);
        let liquid = StaticVisualKind::TexLandEc { resolution: test_surface_like_resolution() }.kingdom_reborn();
        assert!(liquid.is_kingdom_reborn());
        assert_eq!(liquid.material_item_id(GRASS), u32::from(GRASS) + KR_ITEM_RECORD_BASE);
        let classic = classic_static_visual_kind(GRASS, None);
        assert_eq!(classic.kingdom_reborn(), classic);
        assert!(!classic.is_kingdom_reborn());
    }

    /// Under the Kingdom Reborn land the statics that continue it are drawn from the Kingdom Reborn packages, and the
    /// others keep their kind. Prints the Kingdom Reborn kinds that differ from the Enhanced ones.
    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn canonical_statics_that_continue_the_kingdom_reborn_land_take_its_items() {
        const GRASS: u16 = 0x1782;
        const ACID: u16 = 0x2E02;
        const KR_SPRITE_WATER: u16 = 0x34D2;
        const TABLE: u16 = 0x0B90;
        const KR_ONLY_ROTATED_SURFACE: u16 = 0x3236;
        let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let ec = udd_assets::tex_art_ec::TexArtEcPackage::load(root.join("tex_art_ec.uddp")).unwrap();
        let land = udd_assets::tex_land_ec::TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
        let meta = udd_assets::tilemeta::TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
        let kr_meta = meta.kr_tilemeta().unwrap().expect("the Kingdom Reborn tile data");
        let kr_art = ec.kr_overlay(udd_assets::AtlasCacheOptions::disabled()).unwrap().expect("the Kingdom Reborn art");
        let kr_land = land.kr_land(udd_container::UddpReaderOptions::disabled(), udd_assets::AtlasCacheOptions::disabled())
            .unwrap().expect("the Kingdom Reborn land");
        let kr = KrStaticSources { tilemeta: &kr_meta, art: &kr_art, land: &kr_land };
        let mut cache = StaticSurfaceLikeResolutionCache::default();
        let mut resolve = |graphic: u16| kingdom_reborn_static_visual_kind(graphic, Some(&meta),
            meta.item_tile(u32::from(graphic)), Some(&ec), Some(&land), kr, &mut cache);
        let base = |graphic: u16| u32::from(graphic) + KR_ITEM_RECORD_BASE;
        assert_eq!(resolve(GRASS), Some(StaticVisualKind::EcSurface { art_id: base(GRASS) }));
        assert!(matches!(resolve(ACID), Some(StaticVisualKind::TexLandEc { resolution })
            if resolution.kingdom_reborn && resolution.liquid_payload.is_some()), "{:?}", resolve(ACID));
        // A water sprite in Kingdom Reborn, a land liquid in the Enhanced Client.
        assert_eq!(resolve(KR_SPRITE_WATER), Some(StaticVisualKind::EcRegular { art_id: base(KR_SPRITE_WATER) }));
        assert_eq!(resolve(TABLE), None);
        // Drawn from the Kingdom Reborn packages, by Kingdom Reborn's tile data: this surface is Rotated (a camera
        // quad) in KR's tileart, and in the Enhanced one by the tilemeta overrides, not in the clients' Enhanced
        // tables Classic art is drawn by; the KR water sprite is not Rotated in KR's.
        let clients = meta.without_overrides().unwrap().expect("the tilemeta overrides");
        let rotated = |data: (Option<&udd_assets::tilemeta::TileMetaPackage>,
            Option<&udd_assets::tilemeta::TileMetaItemTile>)| ec_static_camera_quad(data.1);
        let drawn = |kind, graphic, kr| drawn_tile_data(kind, graphic, Some(&meta), Some(&clients), kr);
        assert!(rotated(drawn(resolve(KR_ONLY_ROTATED_SURFACE), KR_ONLY_ROTATED_SURFACE, Some(kr))));
        assert!(rotated(drawn(resolve(KR_ONLY_ROTATED_SURFACE), KR_ONLY_ROTATED_SURFACE, None)));
        assert!(!rotated(drawn(Some(classic_static_visual_kind(KR_ONLY_ROTATED_SURFACE, None)), KR_ONLY_ROTATED_SURFACE,
            None)));
        assert!(!rotated(drawn(resolve(KR_SPRITE_WATER), KR_SPRITE_WATER, Some(kr))));
        assert!(rotated(drawn(resolve(KR_SPRITE_WATER), KR_SPRITE_WATER, None)));
        // A static that does not continue the land keeps the Enhanced tile data under the Kingdom Reborn land.
        assert!(std::ptr::eq(drawn(resolve(TABLE), TABLE, Some(kr)).0.unwrap(), &meta));
        let mut kinds = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut enhanced_cache = StaticSurfaceLikeResolutionCache::default();
        for tile in meta.item_tiles().iter().filter(|t| !t.is_nodraw()) {
            let Ok(graphic) = u16::try_from(tile.tile_id) else { continue; };
            let Some(kind) = resolve(graphic) else { continue; };
            assert!(kind.is_kingdom_reborn(), "{graphic:#x}");
            let name = match kind {
                StaticVisualKind::EcRegular { .. } => "sprite",
                StaticVisualKind::EcSurface { .. } => "surface",
                StaticVisualKind::TexLandEc { resolution } if resolution.liquid_payload.is_some() => "liquid",
                StaticVisualKind::TexLandEc { .. } => "land",
                StaticVisualKind::CcRegular { .. } | StaticVisualKind::Missing => "other",
            };
            kinds.entry(name.to_string()).or_default().push(format!("{graphic:#06x}"));
            let enhanced_continues = enhanced_static_visual_kind(graphic, Some(&meta), Some(tile), Some(&ec),
                Some(&land), &mut enhanced_cache).is_some_and(|kind| static_continues_land(kind, Some(tile),
                    enhanced_cache.landlike_item_texture(graphic, Some(&meta), Some(&land))));
            if !enhanced_continues {
                kinds.entry("by the Kingdom Reborn data only".to_string()).or_default().push(format!("{graphic:#06x}"));
            }
        }
        for (name, graphics) in kinds {
            println!("{name}: {} {}", graphics.len(), graphics.join(" "));
        }
    }

    #[test]
    fn land_style_follows_the_land_atlas() {
        assert_eq!(StaticLandStyle::of_land_atlas(LandTextureSource::Cc), StaticLandStyle::Classic);
        assert_eq!(StaticLandStyle::of_land_atlas(LandTextureSource::Ec), StaticLandStyle::Enhanced);
        assert_eq!(StaticLandStyle::of_land_atlas(LandTextureSource::Kr), StaticLandStyle::KingdomReborn);
        assert!(StaticLandStyle::KingdomReborn.is_enhanced() && StaticLandStyle::Enhanced.is_enhanced());
        assert!(!StaticLandStyle::Classic.is_enhanced() && !StaticLandStyle::Unknown.is_enhanced());
    }

    #[test]
    fn classic_art_keeps_its_art_without_enhanced_land() {
        let tile = item_tile_with_flags(TILE_FLAG_WET, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        for style in [StaticLandStyle::Unknown, StaticLandStyle::Classic, StaticLandStyle::Enhanced] {
            // Without the Enhanced packages Classic art draws its art under any land.
            assert_eq!(resolve_static_visual_kind_cached(ClientTextureSource::Cc, style, 0x1797, None, Some(&tile),
                None, None, None, &mut StaticSurfaceLikeResolutionCache::default()),
                classic_static_visual_kind(0x1797, Some(&tile)), "{style:?}");
        }
    }

    /// The statics that continue the land take its style under the mixed sources, and the matched sources draw as
    /// before. Prints the set by Enhanced kind.
    #[test]
    #[ignore = "requires current canonical packages in DYNAMAPPER_TEST_PACKAGES"]
    fn canonical_statics_that_continue_the_land_take_its_style() {
        const LANDLIKE: [(u16, &str); 7] = [(0x1797, "river water"), (0x179F, "river foam"), (0x2E02, "acid"),
            (0x12EE, "lava liquid"), (0x1782, "grass"), (0x3209, "swamp"), (0x329C, "lava field")];
        const BUILT_FLOORS: [(u16, &str); 3] = [(0x0515, "cobblestones"), (0x051D, "stone pavers"),
            (0x04A9, "wooden boards")];
        let root = std::path::PathBuf::from(std::env::var_os("DYNAMAPPER_TEST_PACKAGES")
            .expect("set DYNAMAPPER_TEST_PACKAGES to the matching current package directory"));
        let cc = udd_assets::tex_art_cc::TexArtCcPackage::load(root.join("tex_art_cc.uddp")).unwrap();
        let ec = udd_assets::tex_art_ec::TexArtEcPackage::load(root.join("tex_art_ec.uddp")).unwrap();
        let land = udd_assets::tex_land_ec::TexLandEcPackage::load(root.join("tex_land_ec.uddp")).unwrap();
        let meta = udd_assets::tilemeta::TileMetaPackage::load(root.join("tilemeta.uddp")).unwrap();
        let mut cache = StaticSurfaceLikeResolutionCache::default();
        let mut landlike = Vec::new();
        let (mut liquids, mut surfaces) = (0, 0);
        for tile in meta.item_tiles().iter().filter(|t| !t.is_nodraw()) {
            let Ok(graphic) = u16::try_from(tile.tile_id) else { continue; };
            let mut resolve = |art, style| resolve_static_visual_kind_cached(art, style, graphic, Some(&meta),
                Some(tile), Some(&cc), Some(&ec), Some(&land), &mut cache);
            let (enhanced, classic) = (resolve(ClientTextureSource::Ec, StaticLandStyle::Unknown),
                resolve(ClientTextureSource::Cc, StaticLandStyle::Unknown));
            // Matched sources draw as before.
            assert_eq!(resolve(ClientTextureSource::Ec, StaticLandStyle::Enhanced), enhanced, "{graphic:#x}");
            assert_eq!(resolve(ClientTextureSource::Cc, StaticLandStyle::Classic), classic, "{graphic:#x}");
            let (enhanced_art_classic_land, classic_art_enhanced_land) = (
                resolve(ClientTextureSource::Ec, StaticLandStyle::Classic),
                resolve(ClientTextureSource::Cc, StaticLandStyle::Enhanced));
            let continues = ec.present_slot(tile.tile_id).is_some() && static_continues_land(enhanced, Some(tile),
                cache.landlike_item_texture(graphic, Some(&meta), Some(&land)));
            if continues {
                landlike.push(graphic);
                match enhanced {
                    StaticVisualKind::TexLandEc { .. } => liquids += 1,
                    _ => surfaces += 1,
                }
                assert_eq!(classic_art_enhanced_land, enhanced, "{graphic:#x}");
                let has_classic = present_classic_static_visual_kind(graphic, Some(tile), Some(&cc)).is_some();
                assert_eq!(enhanced_art_classic_land, if has_classic { classic } else { enhanced }, "{graphic:#x}");
            } else {
                assert_eq!(classic_art_enhanced_land, classic, "{graphic:#x}");
                assert_eq!(enhanced_art_classic_land, enhanced, "{graphic:#x}");
            }
            // They and the foliage take the land's grading under either art source; the other statics keep their own
            // colour.
            let foliage = tile.flags & TILE_FLAG_FOLIAGE != 0;
            let land_grade = if continues || foliage { STATIC_FLAG_LAND_GRADE } else { 0 };
            for kind in [enhanced, classic] {
                assert_eq!(land_grade_flag(kind, graphic, tile.flags, Some(&meta), Some(tile), Some(&ec), Some(&land),
                    &mut cache),
                    land_grade, "{graphic:#x}");
            }
        }
        println!("statics that continue the land: {} ({liquids} land liquids, {surfaces} terrain-textured surfaces)",
            landlike.len());
        for (graphic, name) in LANDLIKE {
            assert!(landlike.contains(&graphic), "{name} {graphic:#x} does not continue the land");
        }
        for (graphic, name) in BUILT_FLOORS {
            assert!(!landlike.contains(&graphic), "{name} {graphic:#x} is a built floor");
        }
    }

    #[test]
    fn gpu_pick_owner_order_matches_sorted_sprite_and_ground_instances() {
        let owner = |graphic, z| StaticPickTile { tile: StaticRenderTile {
            graphic, world_x: 10.0, world_z: 20.0, z, hue: 33 }, is_wet: false };
        let mut build = StaticChunkBuildState::default();
        build.pick_tiles = vec![owner(18910, -5), owner(2901, 0), owner(6044, -5)];
        let mut front = SpriteInstance::zeroed();
        front.tile_x = 2.0;
        let mut back = front;
        back.tile_x = 1.0;
        build.sprite_instances = vec![front, back];
        build.sprite_pick_indices = vec![(static_depth_key(2.0, 0.0, 0.0), 0),
            (static_depth_key(1.0, 0.0, 0.0), 1)];
        build.ground_instances = vec![GroundTileInstance::zeroed()];
        build.ground_pick_indices = vec![(0.0, 2)];
        let chunk = finish_rebuilt_static_chunk(build, 1);
        let key = StaticChunkBatchKey { map_id: 0, gx: 1, gy: 2, scale: 1 };
        let mut cache = StaticChunkRenderCache::default();
        cache.chunks.insert(key, chunk);
        let batch = StaticChunkBatch { key, start: 0, count: 2 };
        let batches = RenderStaticChunkBatches { sprite: vec![batch],
            ground: vec![StaticChunkBatch { count: 1, ..batch }] };
        let targets = cache.gpu_pick_targets(&batches);
        assert_eq!(targets, [vec![owner(2901, 0), owner(18910, -5)], vec![owner(6044, -5)]]);
        assert_eq!(cache.chunks[&key].sprite_instances[0].tile_x, 1.0);
    }

    #[test]
    fn wet_surface_like_tiles_keep_surface_depth_and_land_redirection() {
        let tile = item_tile_with_flags(
            TILE_FLAG_WET,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );

        assert!(static_tile_is_surface_like(Some(&tile), tile.is_wet()));
        assert!(static_tile_uses_tex_land_ec_surface_path(Some(&tile), tile.is_wet()));
    }

    #[test]
    fn cc_flat_render_gate_targets_wet_water_not_surface_floors() {
        // Real "slate roof": SURFACE | Background, authored SurfaceLike, NOT wet, elevated.
        // It is surface-like (would enter the EC land path) but must stay a CC billboard.
        let roof = item_tile_with_flags(
            TILE_FLAG_SURFACE | TILE_FLAG_BACKGROUND,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );
        assert!(static_tile_is_surface_like(Some(&roof), roof.is_wet()));
        assert!(!cc_static_tile_renders_flat(Some(&roof)));

        // Real shoreline water / "wave" statics carry IsWet -> lie flat.
        let water = item_tile_with_flags(
            TILE_FLAG_WET | TILE_FLAG_BACKGROUND,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );
        assert!(cc_static_tile_renders_flat(Some(&water)));

        assert!(!cc_static_tile_renders_flat(None));
    }

    #[test]
    fn cc_ground_diamond_requires_tile_sized_dry_background_without_volume() {
        let mut tile = item_tile_with_flags(TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        assert!(cc_ground_diamond(Some(&tile), 44.0, 44.0));
        assert!(!cc_ground_diamond(Some(&tile), 44.0, 88.0));
        assert!(!cc_ground_diamond(Some(&tile), 43.0, 44.0));
        for excluded in [TILE_FLAG_ROOF, TILE_FLAG_BRIDGE, TILE_FLAG_FOLIAGE, TILE_FLAG_WET] {
            tile.flags = TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE | excluded;
            assert!(!cc_ground_diamond(Some(&tile), 44.0, 44.0));
        }
        tile.flags = TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE;
        tile.height = 1;
        assert!(!cc_ground_diamond(Some(&tile), 44.0, 44.0));
        assert!(!cc_ground_diamond(None, 44.0, 44.0));
    }

    #[test]
    fn wet_surface_like_tiles_consider_visible_tileart_refs_before_water_fallback() {
        let refs = [
            texture_ref_with_role(
                200,
                udd_assets::tilemeta::EcMaterialStableRole::WaterBump,
                udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(
                201,
                udd_assets::tilemeta::EcMaterialStableRole::Base,
                udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let chosen = choose_visible_surface_texture_ref(&refs, 42).expect("visible ref");

        assert_eq!(chosen.texture_id, 201);
    }

    #[test]
    fn wet_surface_like_ground_payload_does_not_force_ec_water_material() {
        let (material_payload, material_flags) = surface_like_ground_material_payload(None);

        assert_eq!(material_flags, 0);
        assert_eq!(material_payload, 0);
    }

    #[test]
    fn non_wet_surface_like_tiles_still_use_land_redirection() {
        let tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);

        assert!(static_tile_uses_tex_land_ec_surface_path(Some(&tile), tile.is_wet()));
    }

    #[test]
    fn valid_texture_repetition_rejects_invalid_values() {
        assert_eq!(valid_texture_repetition(4.0), 4.0);
        assert_eq!(valid_texture_repetition(0.0), 1.0);
        assert_eq!(valid_texture_repetition(f32::NAN), 1.0);
    }

    #[test]
    fn wet_surface_like_tiles_do_not_use_legacy_ec_texture_fallback() {
        let mut tile = item_tile_with_flags(
            TILE_FLAG_WET,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );
        tile.ec_texture_id = 12_345;

        assert_eq!(surface_like_legacy_ec_texture_id_fallback(&tile, tile.is_wet()), None);
    }

    #[test]
    fn non_wet_surface_like_tiles_can_use_legacy_ec_texture_fallback() {
        let mut tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        tile.ec_texture_id = 12_345;

        assert_eq!(surface_like_legacy_ec_texture_id_fallback(&tile, tile.is_wet()), Some(12_345));
    }

    #[test]
    fn tileart_owned_surfaces_keep_their_own_atlas_slot() {
        let resolution = test_surface_like_resolution();
        let visual_kind = resolve_ec_static_visual_kind(196, false, true, Some(resolution));

        assert_eq!(visual_kind, StaticVisualKind::EcSurface { art_id: 196 });
    }

    #[test]
    fn walkable_sprite_art_takes_precedence_over_terrain_id_collisions() {
        let resolution = test_surface_like_resolution();
        for tile_graphic in [1874, 1993, 2880] {
            assert_eq!(
                resolve_ec_static_visual_kind(tile_graphic, true, false, Some(resolution)),
                StaticVisualKind::EcRegular { art_id: u32::from(tile_graphic) },
            );
        }
    }

    #[test]
    fn source_matched_liquid_shares_terrain_material_without_replacing_sprite_art() {
        let resolution = SurfaceLikeTexLandEcResolution {
            liquid_payload: Some(168),
            ..test_surface_like_resolution()
        };
        assert_eq!(
            resolve_ec_static_visual_kind(6038, false, true, Some(resolution)),
            StaticVisualKind::TexLandEc { resolution },
        );
        assert_eq!(surface_like_ground_material_payload(resolution.liquid_payload), (168, 1));
        assert_eq!(
            resolve_ec_static_visual_kind(6038, true, true, Some(resolution)),
            StaticVisualKind::EcRegular { art_id: 6038 },
        );
    }

    #[test]
    fn tileart_water_support_slot_uses_visible_terrain_material() {
        let primary = texture_ref_with_role(
            1000013,
            udd_assets::tilemeta::EcMaterialStableRole::BlendAlpha,
            udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
        );
        let resolution = test_surface_like_resolution();
        assert_eq!(
            resolve_ec_static_visual_kind(6038, false, tileart_surface_has_visible_primary(&[primary]), Some(resolution)),
            StaticVisualKind::TexLandEc { resolution },
        );
        let base = texture_ref_with_role(200, udd_assets::tilemeta::EcMaterialStableRole::Base, 0);
        assert!(!tileart_surface_has_visible_primary(&[primary, base]));
        assert!(tileart_surface_has_visible_primary(&[base]));
        assert!(tileart_surface_has_visible_primary(&[]));
    }

    #[test]
    fn tileart_owned_surfaces_render_without_a_terrain_texture_copy() {
        assert_eq!(
            resolve_ec_static_visual_kind(1169, false, true, None),
            StaticVisualKind::EcSurface { art_id: 1169 },
        );
        assert_eq!(
            surface_like_static_world_anchor(StaticVisualKind::EcSurface { art_id: 1169 }, 10.0, 20.0),
            (10.0, 20.0),
        );
    }

    #[test]
    fn ec_camera_quad_uses_the_authored_flag_instead_of_wet_classification() {
        let solid = item_tile_with_flags(EC_CAMERA_QUAD_FLAG,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        let wet = item_tile_with_flags(TILE_FLAG_WET,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        assert!(ec_static_camera_quad(Some(&solid)));
        assert!(!ec_static_camera_quad(Some(&wet)));
        assert!(!ec_static_camera_quad(None));
    }

    #[test]
    fn without_an_ec_material_the_classic_wet_flag_picks_the_wet_render_path() {
        const ART_ID: u32 = 0x1797;
        let wet = item_tile_with_flags(TILE_FLAG_WET, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        let dry = item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        for source in [None, Some(ClientTextureSource::Cc), Some(ClientTextureSource::Ec)] {
            assert!(static_tile_renders_wet(Some(&wet), None, ART_ID, source));
            assert!(!static_tile_renders_wet(Some(&dry), None, ART_ID, source));
            assert!(!static_tile_renders_wet(None, None, ART_ID, source));
        }
    }

    #[test]
    fn terrain_surface_resolution_remains_available_without_tileart_material() {
        let resolution = test_surface_like_resolution();
        assert_eq!(
            resolve_ec_static_visual_kind(196, false, false, Some(resolution)),
            StaticVisualKind::TexLandEc { resolution },
        );
    }

    #[test]
    fn legacy_translucency_follows_original_foliage_and_source_dispatch() {
        use uocf::enhanced::tileart::TaeFlag;
        let tile = |flags| item_tile_with_flags(flags, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        let wave = tile((TaeFlag::Translucent | TaeFlag::Wet).bits());
        let foliage = tile((TaeFlag::Translucent | TaeFlag::Foliage).bits());
        let surface = tile((TaeFlag::Translucent | TaeFlag::Surface).bits());
        let partial = tile((TaeFlag::Translucent | TaeFlag::PartialHue).bits());
        assert_eq!(TaeFlag::Foliage.bits(), 0x20000);
        assert_eq!(TaeFlag::Surface.bits(), 0x200);
        assert_eq!(static_sprite_opacity(Some(&wave), true), 0.75);
        assert_eq!(static_sprite_opacity(Some(&foliage), true), 0.25);
        assert_eq!(static_sprite_opacity(Some(&surface), true), 0.75);
        assert_eq!(static_sprite_opacity(Some(&partial), true), 0.75);
        assert_eq!(static_sprite_opacity(Some(&wave), false), 1.0);
        assert_eq!(static_sprite_opacity(Some(&tile(TaeFlag::Wet.bits())), true), 1.0);
        assert_eq!(static_sprite_opacity(Some(&tile(TaeFlag::Transparent.bits())), true), 1.0);
        assert_eq!(static_sprite_opacity(None, true), 1.0);
    }

    #[test]
    fn picking_uses_only_current_batches_and_preserves_expanded_tile_data() {
        let visible = StaticChunkBatchKey { map_id: 0, gx: 2, gy: 3, scale: 1 };
        let other = StaticChunkBatchKey { gx: 20, ..visible };
        let expanded = StaticPickTile { tile: StaticRenderTile { graphic: 3240,
            world_x: 1046.0, world_z: 2253.0, z: -7, hue: 22 }, is_wet: false };
        let mut cache = StaticChunkRenderCache::default();
        for key in [visible, other] {
            cache.chunks.insert(key, CachedStaticChunk { pick_tiles: vec![expanded],
                sprite_pick_indices: vec![0], ..Default::default() });
        }
        let mut batches = RenderStaticChunkBatches { sprite: vec![StaticChunkBatch {
            key: visible, start: 0, count: 1 }], ..Default::default() };
        assert_eq!(cache.gpu_pick_targets(&batches), [vec![expanded], Vec::new()]);
        batches.sprite.clear();
        assert_eq!(cache.gpu_pick_targets(&batches), [Vec::new(), Vec::new()]);
        batches.sprite.push(StaticChunkBatch { key: visible, start: 1, count: 1 });
        assert!(cache.gpu_pick_targets(&batches)[0].is_empty(), "invalid GPU indices must not select another object");
    }

    #[test]
    fn disabling_statics_clears_draw_batches_and_invalidates_output_reuse() {
        use bevy::ecs::system::RunSystemOnce;
        use udd_assets::tex_art_cc::PagePixelFormat;
        let mut world = World::new();
        let mut settings = crate::configs::settings::load_from_files().expect("settings load");
        settings.world_rendering.enable_statics = false;
        let key = StaticChunkBatchKey { map_id: 0, gx: 1, gy: 2, scale: 1 };
        let batch = StaticChunkBatch { key, start: 0, count: 1 };
        let mut cache = StaticChunkRenderCache::default();
        cache.last_visible_chunk_keys.push(key);
        cache.chunks.insert(key, CachedStaticChunk::default());
        world.insert_resource(settings);
        world.insert_resource(StaticsStoreRes(Vec::new()));
        world.insert_resource(SceneStateData { map_id: 0 });
        world.init_resource::<ZViewLimit>();
        world.init_resource::<CavesRes>();
        world.init_resource::<PlacedMultisRes>();
        world.insert_resource(TileAtlas::new(Default::default(), 0));
        world.insert_resource(RenderZoom::default());
        world.insert_resource(SpriteArtPageAtlas(crate::core::texture_cache::art::ArtPageAtlas::new(
            Handle::default(), 64, 64, 1, 1, PagePixelFormat::Rgba8888,
        )));
        world.insert_resource(GroundArtPageAtlas(crate::core::texture_cache::art::ArtPageAtlas::new(
            Handle::default(), 64, 64, 1, 1, PagePixelFormat::Rgba8888,
        )));
        world.insert_resource(RenderStaticInstances(vec![SpriteInstance::zeroed()]));
        world.insert_resource(RenderStaticLandInstances(vec![GroundTileInstance::zeroed()]));
        world.insert_resource(RenderStaticChunkBatches { sprite: vec![batch], ground: vec![batch] });
        world.insert_resource(cache);
        world.init_resource::<StaticSurfaceLikeResolutionCache>();
        world.init_resource::<StaticArtCollectDebugState>();
        world.init_resource::<StaticArtSourceState>();
        world.init_resource::<super::super::static_lights::RenderStaticLightInstances>();
        world.run_system_once(sys_collect_visible_statics).unwrap();
        assert!(world.resource::<RenderStaticInstances>().0.is_empty());
        assert!(world.resource::<RenderStaticLandInstances>().0.is_empty());
        let batches = world.resource::<RenderStaticChunkBatches>();
        assert!(batches.sprite.is_empty() && batches.ground.is_empty());
        let cache = world.resource::<StaticChunkRenderCache>();
        assert!(cache.last_visible_chunk_keys.is_empty());
        assert!(cache.chunks.contains_key(&key), "decoded chunks remain reusable after enabling");
    }

    #[test]
    fn wet_surface_like_visible_ref_rejects_normal_maps() {
        let refs = [
            texture_ref_with_role(
                200,
                udd_assets::tilemeta::EcMaterialStableRole::WaterBump,
                udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
            texture_ref_with_role(201, udd_assets::tilemeta::EcMaterialStableRole::Base, 0),
        ];

        let chosen = choose_visible_surface_texture_ref(&refs, 42).expect("visible ref");

        assert_eq!(chosen.texture_id, 201);
    }

    #[test]
    fn wet_surface_like_visible_ref_prefers_primary_base() {
        let refs = [
            texture_ref_with_role(200, udd_assets::tilemeta::EcMaterialStableRole::Base, 0),
            texture_ref_with_role(
                201,
                udd_assets::tilemeta::EcMaterialStableRole::Base,
                udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
            ),
        ];

        let chosen = choose_visible_surface_texture_ref(&refs, 42).expect("visible ref");

        assert_eq!(chosen.texture_id, 201);
    }

    #[test]
    fn wet_surface_like_visible_ref_rejects_exact_tile_id_support_fallback() {
        let refs = [texture_ref_with_role(
            42,
            udd_assets::tilemeta::EcMaterialStableRole::Base,
            udd_assets::tilemeta::TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED,
        )];

        assert!(choose_visible_surface_texture_ref(&refs, 42).is_none());
    }

    #[test]
    fn static_hue_payload_encodes_nonzero_hues() {
        assert_eq!(static_hue_payload(1), (1, STATIC_HUE_FLAG_APPLY));
        assert_eq!(static_hue_payload(0), (0, 0));
    }

    #[test]
    fn non_surface_like_ec_tiles_stay_on_regular_art_path() {
        let visual_kind = resolve_ec_static_visual_kind(196, false, false, None);

        assert_eq!(visual_kind, StaticVisualKind::EcRegular { art_id: 196 },);
    }

    #[test]
    fn statics_of_every_graphic_collect_as_themselves() {
        // Graphics from 0x4000 up are ordinary items in the map statics (thousands on maps 0, 1 and 5), never multis.
        for graphic in [7, 0x4000, 0x4007, 0xFFFF] {
            let source_tile = uocf::classic::statics::PackedStaticTile {
                graphic,
                xy_packed: 3 | (4 << 3),
                z: 5,
                hue: 9,
            };

            assert_eq!(
                static_cell_tile(2, 3, source_tile),
                StaticRenderTile {
                    graphic,
                    world_x: 19.0,
                    world_z: 28.0,
                    z: 5,
                    hue: 9,
                },
            );
        }
    }

    #[test]
    fn placed_multi_expands_hidden_parts_only_on_request() {
        let definitions = MultiDefinitionsRes::from_classic_parts(vec![vec![
            uocf::classic::multi::MultiPart {
                item_id: 10,
                x: 0,
                y: 0,
                z: 0,
                flags: crate::core::multis::MULTI_PART_VISIBLE_FLAG,
            },
            uocf::classic::multi::MultiPart {
                item_id: 11,
                x: 0,
                y: 0,
                z: 0,
                flags: 0,
            },
        ]]);
        let multi = PlacedMulti { multi_id: 0, x: 0, y: 0, z: 0, hue: 0 };
        let mut output = Vec::new();

        collect_multi_render_tiles(&multi, definitions.parts(multi.multi_id, false).unwrap(), &mut output);
        let graphics: Vec<u16> = output.iter().map(|tile| tile.graphic).collect();
        assert_eq!(graphics, vec![10]);

        collect_multi_render_tiles(&multi, definitions.parts(multi.multi_id, true).unwrap(), &mut output);
        let graphics: Vec<u16> = output.iter().map(|tile| tile.graphic).collect();
        assert_eq!(graphics, vec![10, 11]);
    }

    #[test]
    fn placed_multi_expands_visible_parts_at_world_offsets() {
        let definitions = MultiDefinitionsRes::from_classic_parts(vec![vec![
            uocf::classic::multi::MultiPart {
                item_id: 10,
                x: -1,
                y: 2,
                z: 3,
                flags: crate::core::multis::MULTI_PART_VISIBLE_FLAG,
            },
            uocf::classic::multi::MultiPart {
                item_id: 11,
                x: 9,
                y: -4,
                z: -2,
                flags: crate::core::multis::MULTI_PART_VISIBLE_FLAG,
            },
        ]]);
        let multi = PlacedMulti { multi_id: 0, x: 19, y: 28, z: 5, hue: 9 };
        let mut output = Vec::new();

        collect_multi_render_tiles(&multi, definitions.parts(multi.multi_id, false).unwrap(), &mut output);

        assert_eq!(
            output,
            vec![
                StaticRenderTile {
                    graphic: 10,
                    world_x: 18.0,
                    world_z: 30.0,
                    z: 8,
                    hue: 9,
                },
                StaticRenderTile {
                    graphic: 11,
                    world_x: 28.0,
                    world_z: 24.0,
                    z: 3,
                    hue: 9,
                },
            ],
        );
    }

    #[test]
    fn depth_class_defaults_to_regular_without_metadata() {
        assert_eq!(resolve_static_depth_class(None, false), StaticDepthClass::Regular);
    }

    #[test]
    fn depth_class_promotes_surface_like_floor_tiles() {
        let tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);

        assert_eq!(
            resolve_static_depth_class(Some(&tile), tile.is_wet()),
            StaticDepthClass::SurfaceLikeFloor,
        );
    }

    #[test]
    fn bridge_stone_and_stairs_retain_volume_depth_and_height_priority() {
        // Installed bridge fixture: stone 1872 and stair pieces 1874/1895. ClassicUO's +1 for a non-zero height.
        for (flags, height, priority) in [
            (0x04002600, 10, 16.0),
            (0x80002600, 5, 16.0),
            (0x2600, 5, 16.0),
        ] {
            let mut tile = item_tile_with_flags(
                flags,
                udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
            );
            tile.height = height;
            assert!(static_tile_is_surface_like(Some(&tile), tile.is_wet()));
            let class = resolve_static_depth_class(Some(&tile), tile.is_wet());
            assert_eq!(class, StaticDepthClass::Regular);
            assert_eq!(resolve_priority_z_units(15, Some(&tile)), priority);
        }
    }

    #[test]
    fn depth_class_gives_roof_priority_over_surface_like_hint() {
        let tile = item_tile_with_flags(
            TILE_FLAG_ROOF,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );

        assert_eq!(
            resolve_static_depth_class(Some(&tile), tile.is_wet()),
            StaticDepthClass::Roof
        );
    }

    #[test]
    fn depth_class_gives_background_priority_over_roof_and_foliage() {
        let tile = item_tile_with_flags(
            TILE_FLAG_BACKGROUND | TILE_FLAG_ROOF | TILE_FLAG_FOLIAGE,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );

        assert_eq!(
            resolve_static_depth_class(Some(&tile), tile.is_wet()),
            StaticDepthClass::Background,
        );
    }

    #[test]
    fn depth_class_gives_roof_priority_over_foliage() {
        let tile = item_tile_with_flags(
            TILE_FLAG_ROOF | TILE_FLAG_FOLIAGE,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );

        assert_eq!(
            resolve_static_depth_class(Some(&tile), tile.is_wet()),
            StaticDepthClass::Roof
        );
    }

    #[test]
    fn depth_class_separates_background_and_foliage_tiles() {
        let background = item_tile_with_flags(
            TILE_FLAG_BACKGROUND,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );
        let foliage = item_tile_with_flags(
            TILE_FLAG_FOLIAGE,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );

        assert_eq!(
            resolve_static_depth_class(Some(&background), background.is_wet()),
            StaticDepthClass::Background,
        );
        assert_eq!(
            resolve_static_depth_class(Some(&foliage), foliage.is_wet()),
            StaticDepthClass::Foliage,
        );
    }

    #[test]
    fn depth_class_y_bias_keeps_surface_like_tiles_on_ground_bias() {
        assert_eq!(
            depth_class_y_bias(StaticDepthClass::SurfaceLikeFloor),
            GROUND_ART_Y_BIAS,
        );
    }

    #[test]
    fn depth_class_y_bias_keeps_non_surface_tiles_on_static_bias() {
        assert_eq!(
            depth_class_y_bias(StaticDepthClass::Regular),
            STATIC_ART_Y_BIAS,
        );
        assert_eq!(
            depth_class_y_bias(StaticDepthClass::Background),
            STATIC_ART_Y_BIAS,
        );
        assert_eq!(
            depth_class_y_bias(StaticDepthClass::Foliage),
            STATIC_ART_Y_BIAS,
        );
        assert_eq!(
            depth_class_y_bias(StaticDepthClass::Roof),
            STATIC_ART_Y_BIAS,
        );
    }

    #[test]
    fn depth_class_encoding_is_stable() {
        assert_eq!(StaticDepthClass::Regular.encoded(), 0);
        assert_eq!(StaticDepthClass::Background.encoded(), 1);
        assert_eq!(StaticDepthClass::Foliage.encoded(), 2);
        assert_eq!(StaticDepthClass::Roof.encoded(), 3);
        assert_eq!(StaticDepthClass::SurfaceLikeFloor.encoded(), 4);
    }

    #[test]
    fn priority_z_is_classicuo_z_with_its_adjustments() {
        let regular = |flags, height| {
            let mut tile = item_tile_with_flags(flags, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
            tile.height = height;
            resolve_priority_z_units(7, Some(&tile))
        };
        assert_eq!(resolve_priority_z_units(7, None), 7.0);
        // A flat item keeps its z: no default height of 10.
        assert_eq!(regular(0, 0), 7.0);
        // Any height adds one, not the height: a 20 z wall and a 1 z rug at one z tie on the z.
        assert_eq!(regular(0, 20), 8.0);
        assert_eq!(regular(0, 1), 8.0);
        assert_eq!(regular(TILE_FLAG_BACKGROUND, 0), 6.0);
        assert_eq!(regular(TILE_FLAG_BACKGROUND, 20), 7.0);
        // Bridge is not halved, Surface takes nothing.
        assert_eq!(regular(TILE_FLAG_BRIDGE, 12), 8.0);
        assert_eq!(regular(TILE_FLAG_SURFACE, 6), 8.0);
        assert_eq!(regular(TILE_FLAG_MULTI_MOVABLE, 0), 8.0);
    }

    #[test]
    fn surface_like_tables_keep_classicuo_priority() {
        // A table (Surface, height 6) at z 7 on a floor (Background | Surface, height 0) at z 7: the table in front.
        let mut table = item_tile_with_flags(TILE_FLAG_SURFACE, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        table.height = 6;
        let floor = item_tile_with_flags(TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        assert_eq!(resolve_priority_z_units(7, Some(&table)), 8.0);
        assert_eq!(resolve_priority_z_units(7, Some(&floor)), 6.0);
    }

    // The highest and lowest priority z: UO z 127 with + 1 height and + 1 MultiMovable, land at -128 - 2.
    const TEST_PRIORITY_Z_MAX: f64 = 129.0;
    const TEST_PRIORITY_Z_MIN: f64 = -130.0;
    const TEST_TIE_LEVEL_MAX: u32 = 8;

    #[test]
    fn a_nearer_row_beats_any_priority_z() {
        for row in [0.0, 2.0, 4000.0, 11_262.0] {
            let far_highest = logical_draw_distance(row, TEST_PRIORITY_Z_MAX, TEST_TIE_LEVEL_MAX);
            let near_lowest = logical_draw_distance(row + 1.0, TEST_PRIORITY_Z_MIN, FLAT_TIE_LEVEL);
            assert!(near_lowest > far_highest, "row {row}: {near_lowest} <= {far_highest}");
        }
        // The keys of upright sprites (anchors one row apart) at the extremes.
        assert!(static_depth_key(101.0, 201.0, -128.0) > static_depth_key(100.0, 201.0, 128.0));
    }

    #[test]
    fn within_a_row_priority_z_then_tie_level_decide() {
        let row = 3000.0;
        assert!(logical_draw_distance(row, 8.0, FLAT_TIE_LEVEL) > logical_draw_distance(row, 7.0, TEST_TIE_LEVEL_MAX));
        assert!(logical_draw_distance(row, 7.0, 2) > logical_draw_distance(row, 7.0, 1));
        // Land at z 7 (priority 5) under a floor at z 7 (Background, 6) under a table at z 7 (8).
        assert!(logical_draw_distance(row, 6.0, FLAT_TIE_LEVEL) > logical_draw_distance(row, 5.0, FLAT_TIE_LEVEL));
        assert!(logical_draw_distance(row, 8.0, SPRITE_FIRST_TIE_LEVEL) > logical_draw_distance(row, 6.0, 0));
    }

    fn tie_level_test_sprite(tile_x: f32, tile_y: f32, priority_z_units: f32) -> SpriteInstance {
        SpriteInstance { tile_x, tile_y, priority_z_units, ..SpriteInstance::zeroed() }
    }

    #[test]
    fn later_statics_of_one_cell_and_priority_z_draw_nearer() {
        let mut instances = vec![
            tie_level_test_sprite(10.0, 20.0, 15.0),
            tie_level_test_sprite(10.0, 20.0, 15.0),
            tie_level_test_sprite(10.0, 20.0, 16.0),
            tie_level_test_sprite(10.0, 20.0, 15.0),
            tie_level_test_sprite(10.0, 20.0, 15.0),
            tie_level_test_sprite(10.0, 20.0, 15.0),
        ];
        assign_sprite_tie_levels(&mut instances);
        let levels: Vec<u32> = instances.iter().map(|instance| instance.sort_bias_ordinal).collect();
        // Even x: levels 1, 3, 5, 7, the fifth sharing the last.
        assert_eq!(levels, [1, 3, 1, 5, 7, 7]);
        assert!(levels.iter().all(|level| (SPRITE_FIRST_TIE_LEVEL..=TEST_TIE_LEVEL_MAX).contains(level)));
    }

    #[test]
    fn equal_statics_of_neighbouring_cells_differ_by_x_parity_across_chunks() {
        // Bridge planks at raw (2820,991) and (2819,992) occupy adjacent 32-cell chunks: whichever batch draws first,
        // the odd cell's plank is the nearer.
        let plank = SpriteInstance { depth_class: StaticDepthClass::Background.encoded(),
            ..tie_level_test_sprite(2820.5, 992.5, 21.0) };
        let mut instances = [plank,
            SpriteInstance { tile_x: 2821.5, ..plank },
            SpriteInstance { tile_x: 2819.5, tile_y: 993.5, ..plank },
            SpriteInstance { tile_x: 2820.5, tile_y: 993.5, ..plank }];
        assign_sprite_tie_levels(&mut instances);
        assert_eq!(instances.map(|instance| instance.sort_bias_ordinal), [1, 2, 2, 1]);
        // Reassembling cached chunks must not accumulate the previous levels.
        assign_sprite_tie_levels(&mut instances);
        assert_eq!(instances.map(|instance| instance.sort_bias_ordinal), [1, 2, 2, 1]);
    }

    #[test]
    fn ec_camera_quads_take_the_ec_art_anchor_row() {
        let (x, z) = (10.0, 20.0);
        let (tile_x, tile_y) = sprite_depth_anchor(FLAT_QUAD_EC_CAMERA, x, z);
        assert_eq!((tile_x, tile_y), apply_static_world_anchor_translation(x, z));
        // Ahead of every land fragment of its own cell at its land's priority z (z - 2 under a static at z - 1).
        let land_front = logical_draw_distance(f64::from(x + z) + 2.0, 13.0, FLAT_TIE_LEVEL);
        assert!(static_depth_key(tile_x, tile_y, 14.0) > land_front);
        assert_eq!(sprite_depth_anchor(0, x, z), (x, z));
    }

    #[test]
    fn a_cells_surfaces_share_its_anchor_row_ahead_of_its_land() {
        let (x, z) = (10.0, 20.0);
        let anchor = cell_depth_anchor(x, z);
        assert_eq!(anchor, (x + 1.0, z + 1.0));
        for flat_quad in [FLAT_QUAD_CC_FLAT, FLAT_QUAD_EC_SURFACE] {
            assert_eq!(sprite_depth_anchor(flat_quad, x, z), anchor);
        }
        // A Background floor at the land's z (priority z - 1) over its own land (z - 2), and under the next row's land
        // however low.
        let floor = static_depth_key(anchor.0, anchor.1, 14.0);
        assert!(floor > static_depth_key(anchor.0, anchor.1, 13.0));
        let (next_x, next_z) = cell_depth_anchor(x + 1.0, z);
        assert!(static_depth_key(next_x, next_z, TEST_PRIORITY_Z_MIN as f32) > floor);
    }

    #[test]
    fn flat_quads_keep_tie_level_zero() {
        let mut instances = [SpriteInstance { flat_quad: FLAT_QUAD_CC_FLAT, ..tie_level_test_sprite(1.0, 2.0, 3.0) },
            SpriteInstance { flat_quad: FLAT_QUAD_EC_SURFACE, ..tie_level_test_sprite(1.0, 2.0, 3.0) },
            tie_level_test_sprite(1.0, 2.0, 3.0)];
        assign_sprite_tie_levels(&mut instances);
        assert_eq!(instances.map(|instance| instance.sort_bias_ordinal), [FLAT_TIE_LEVEL, FLAT_TIE_LEVEL, 2]);
    }

    #[test]
    fn sprite_instance_stride_stays_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 144);
    }

    #[test]
    fn ground_instance_stride_stays_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<GroundTileInstance>(), 144);
    }

    #[test]
    fn static_chunk_cache_invalidates_when_config_changes() {
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        });
        cache.chunks.insert(
            StaticChunkBatchKey {
                map_id: 1,
                gx: 0,
                gy: 0,
                scale: 1,
            },
            CachedStaticChunk::default(),
        );

        cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 1,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        });

        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn static_chunk_cache_ignores_static_light_changes_when_local_lights_disabled() {
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        });

        cache.chunks.insert(
            StaticChunkBatchKey {
                map_id: 1,
                gx: 0,
                gy: 0,
                scale: 1,
            },
            CachedStaticChunk::default(),
        );

        assert!(!cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        }));

        assert!(!cache.chunks.is_empty());
    }

    #[test]
    fn static_chunk_cache_tracks_static_light_changes_when_local_lights_enabled() {
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: Some(0),
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        });
        cache.chunks.insert(
            StaticChunkBatchKey {
                map_id: 1,
                gx: 0,
                gy: 0,
                scale: 1,
            },
            CachedStaticChunk::default(),
        );

        assert!(cache.sync_config(StaticChunkCacheConfig {
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            static_art_local_light_signature: Some(1),
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            z_view: ZViewCut::NONE,
            caves_revision: 0,
        }));

        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn unchanged_incomplete_static_retry_waits_for_atlas_state_change() {
        let chunk_key = StaticChunkBatchKey {
            map_id: 1,
            gx: 0,
            gy: 0,
            scale: 1,
        };
        let retry_signature = StaticAtlasRetrySignature {
            sprite_mapping_revision: 0,
            ground_mapping_revision: 0,
            sprite_resident_pages: 1,
            ground_resident_pages: 0,
            sprite_pending_pages: 1,
            ground_pending_pages: 0,
            sprite_active_layers: 4,
            ground_active_layers: 4,
        };
        let mut cache = StaticChunkRenderCache::default();
        cache.chunks.insert(
            chunk_key,
            CachedStaticChunk {
                stats: CachedStaticChunkStats {
                    atlas_misses: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        cache.last_visible_chunk_keys = vec![chunk_key];
        cache.last_incomplete_retry_atlas_signature = Some(retry_signature);

        assert!(should_skip_unchanged_incomplete_static_retry(
            &cache,
            &[chunk_key],
            true,
            false,
            retry_signature,
        ));

        let changed_retry_signature = StaticAtlasRetrySignature {
            sprite_pending_pages: 0,
            ..retry_signature
        };
        assert!(!should_skip_unchanged_incomplete_static_retry(
            &cache,
            &[chunk_key],
            true,
            false,
            changed_retry_signature,
        ));
    }

    #[test]
    fn static_collect_path_counters_accumulate_paths_and_copies() {
        let mut debug_state = StaticArtCollectDebugState::default();

        record_static_collect_path(
            &mut debug_state,
            StaticArtCollectPath::CompleteFastPath,
            0,
            0,
        );
        record_static_collect_path(
            &mut debug_state,
            StaticArtCollectPath::IncompleteWaitFastPath,
            0,
            0,
        );
        record_static_collect_path(
            &mut debug_state,
            StaticArtCollectPath::SlowRebuild,
            11,
            7,
        );

        assert_eq!(debug_state.path_counters.complete_fast_path, 1);
        assert_eq!(debug_state.path_counters.incomplete_wait_fast_path, 1);
        assert_eq!(debug_state.path_counters.slow_rebuild, 1);
        assert_eq!(debug_state.path_counters.copied_sprite_instances, 11);
        assert_eq!(debug_state.path_counters.copied_ground_instances, 7);
    }

    fn test_static_light(
        hue_id: u16,
        color_rgb: [f32; 3],
    ) -> super::super::static_lights::StaticLightInstance {
        test_static_light_at(hue_id, color_rgb, 10.5, 20.5, 0.0)
    }

    fn test_static_light_at(
        hue_id: u16,
        color_rgb: [f32; 3],
        world_x: f32,
        world_z: f32,
        world_y: f32,
    ) -> super::super::static_lights::StaticLightInstance {
        super::super::static_lights::StaticLightInstance {
            key: super::super::static_lights::StaticLightKey {
                map_id: 1,
                tile_x: 10,
                tile_y: 20,
                z: 0,
                graphic: 100,
                light_id: 5,
                hue_id,
            },
            light_id: 5,
            hue_id,
            color_rgb,
            world_x,
            world_z,
            world_y,
            width_world: 2.0,
            height_world: 2.0,
            ceiling: None,
            cave_outside: false,
        }
    }

    #[test]
    fn static_light_signature_changes_when_light_color_changes() {
        let warm = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light(0, [1.0, 0.72, 0.42]),
        ]);
        let blue = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light(0, [0.25, 0.5, 1.0]),
        ]);

        assert_ne!(static_light_signature(&warm, 1), static_light_signature(&blue, 1));
    }

    #[test]
    fn static_light_signature_changes_when_light_hue_changes() {
        let first = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light(10, [1.0, 0.72, 0.42]),
        ]);
        let second = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light(11, [1.0, 0.72, 0.42]),
        ]);

        assert_ne!(static_light_signature(&first, 1), static_light_signature(&second, 1));
    }

    fn unpack_local_light(packed: u32) -> [f32; 4] {
        std::array::from_fn(|channel| {
            ((packed >> (channel as u32 * LOCAL_LIGHT_CHANNEL_BITS)) & u32::from(u8::MAX)) as f32 / LOCAL_LIGHT_CHANNEL_MAX
        })
    }

    #[test]
    fn local_light_packs_red_in_the_low_byte() {
        assert_eq!(pack_local_light_rgba([1.0, 0.0, 0.0, 0.0]), 0x0000_00ff);
        assert_eq!(pack_local_light_rgba([0.0, 0.0, 0.0, 1.0]), 0xff00_0000);
        assert_eq!(pack_local_light_rgba([2.0, -1.0, 0.5, 0.0]), 0x0080_00ff);
    }

    #[test]
    fn static_art_local_light_sampling_requires_opt_in() {
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], 0.0, 0.0, 0.0),
        ]);

        assert_eq!(static_art_point_local_light_corners(false, &lights, 1, 0.0, 0.0, 0.0), [0; 4]);
        let lit = static_art_point_local_light_corners(true, &lights, 1, 0.0, 0.0, 0.0);
        assert!(unpack_local_light(lit[0])[3] > 0.0);
        assert!(lit.iter().all(|corner| *corner == lit[0]), "a dot is lit evenly");
    }

    #[test]
    fn billboard_local_light_corners_follow_the_quad_layout() {
        let bounds = StaticBillboardBounds {
            local_min_x: -4.0,
            local_max_x: 4.0,
            local_min_y: 0.0,
            local_max_y: 0.0,
        };
        // A light at the billboard's right end lights its right corners (x 1) only.
        let right = static_billboard_sample_world(0.0, 0.0, 0.0, bounds.local_max_x, 0.0);
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], right.0, right.1, right.2),
        ]);

        let corners = static_art_billboard_local_light_corners(true, &lights, 1, 0.0, 0.0, 0.0, bounds, false)
            .map(|corner| unpack_local_light(corner)[3]);
        assert_eq!(corners[0], 0.0);
        assert_eq!(corners[2], 0.0);
        assert!(corners[1] > 0.0 && corners[3] > 0.0);
    }

    #[test]
    fn flat_art_cards_take_their_corner_lights_on_the_ground_they_show() {
        let bounds = StaticBillboardBounds {
            local_min_x: 0.0,
            local_max_x: 0.0,
            local_min_y: 0.0,
            local_max_y: 4.0,
        };
        // The card's top shows the ground its height further back along the view ray.
        let view_ray = PlayerCamera::BASE_OFFSET_FROM_PLAYER;
        let shown = -view_ray * (bounds.local_max_y / view_ray.y);
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], shown.x, shown.z, 0.0),
        ]);
        let corners = |flat_art| {
            static_art_billboard_local_light_corners(true, &lights, 1, 0.0, 0.0, 0.0, bounds, flat_art)
                .map(|corner| unpack_local_light(corner)[3])
        };

        let flat = corners(true);
        assert!(flat[0] > 0.0 && flat[1] > 0.0, "the top corners show the lit ground");
        assert_eq!([flat[2], flat[3]], [0.0; 2]);
        assert_eq!(corners(false), [0.0; 4], "an upright card's top stands above its base, away from the light");
    }

    #[test]
    fn neighbouring_flat_statics_share_their_corner_lights() {
        let bounds = resolve_surface_like_ground_quad_bounds();
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], 1.3, 0.4, 0.0),
        ]);
        let west = static_art_ground_local_light_corners(true, &lights, 1, 0.0, 0.0, 0.0, bounds);
        let east = static_art_ground_local_light_corners(true, &lights, 1, 1.0, 0.0, 0.0, bounds);

        // The west tile's east edge (corners x 1) is the east tile's west edge (corners x 0): no seam between them.
        assert!(unpack_local_light(west[1])[3] > 0.0);
        assert_eq!(west[1], east[0]);
        assert_eq!(west[3], east[2]);
        assert_ne!(west[0], west[1], "the light falls off across a tile");
    }

    #[test]
    fn cc_flat_local_light_corners_lie_on_the_diagonal_basis() {
        let bounds = GroundQuadBounds {
            local_min_x: -1.0,
            local_max_x: 1.0,
            local_min_z: -1.0,
            local_max_z: 1.0,
        };
        // Corner (1, 0): right along the billboard axis, back (up-screen) along the flat forward axis.
        let corner = STATIC_BILLBOARD_RIGHT_XZ * bounds.local_max_x + CC_FLAT_QUAD_FORWARD_XZ * bounds.local_min_z;
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], corner.x, corner.y, 0.0),
        ]);

        let corners = static_art_cc_flat_local_light_corners(true, &lights, 1, 0.0, 0.0, 0.0, bounds)
            .map(|corner| unpack_local_light(corner)[3]);
        assert!(corners[1] > corners[0] && corners[1] > corners[2] && corners[1] > corners[3]);
    }

    #[test]
    fn static_chunk_cache_rebuilds_only_the_chunks_that_read_changed_land() {
        use crate::core::render::scene::world::land::tile_atlas::{AtlasParams, Rg16u};
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(64), max_layers: 1, world_pages_x: 1,
            ..Default::default() }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        let key = |gx| StaticChunkBatchKey { map_id: 1, gx, gy: 0, scale: 1 };
        let read = |min: (u32, u32), max: (u32, u32)| CachedStaticChunk {
            land_cells: Some(AtlasCells::new(UVec2::new(min.0, min.1), UVec2::new(max.0, max.1))),
            ..Default::default()
        };
        let mut cache = StaticChunkRenderCache::default();
        // The first check has nothing to compare with: the chunks built before it go.
        cache.chunks.insert(key(0), CachedStaticChunk::default());
        assert_eq!(cache.invalidate_changed_land(&atlas), 1);
        // The chunk at x 0..31 read the cells up to x 32, one cell into its east neighbour's land (a Classic ground
        // diamond on its last column); the chunk at x 32..63 read its own; the third read no land.
        cache.chunks.insert(key(0), read((0, 0), (32, 31)));
        cache.chunks.insert(key(1), read((32, 0), (63, 31)));
        cache.chunks.insert(key(2), CachedStaticChunk::default());
        assert_eq!(cache.invalidate_changed_land(&atlas), 0);
        // The east neighbour's land loads after both: both read it.
        atlas.enqueue_rg16u_block(layer, UVec2::new(32, 8), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(cache.invalidate_changed_land(&atlas), 2);
        assert_eq!(cache.chunks.keys().copied().collect::<Vec<_>>(), vec![key(2)]);
        // Land only the second read.
        cache.chunks.insert(key(0), read((0, 0), (32, 31)));
        cache.chunks.insert(key(1), read((32, 0), (63, 31)));
        atlas.enqueue_rg16u_block(layer, UVec2::new(40, 8), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(cache.invalidate_changed_land(&atlas), 1);
        assert!(cache.chunks.contains_key(&key(0)) && !cache.chunks.contains_key(&key(1)));
        // Nothing changed: nothing goes.
        assert_eq!(cache.invalidate_changed_land(&atlas), 0);
        // Every cell changed: every chunk goes, the one that read no land too.
        atlas.clear_all_mappings();
        assert_eq!(cache.invalidate_changed_land(&atlas), 2);
        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn static_chunk_cache_prunes_after_hysteresis_window() {
        let mut cache = StaticChunkRenderCache::default();
        cache.chunks.insert(
            StaticChunkBatchKey {
                map_id: 1,
                gx: 0,
                gy: 0,
                scale: 1,
            },
            CachedStaticChunk {
                last_visible_tick: 1,
                ..Default::default()
            },
        );

        cache.prune_stale(1 + STATIC_CHUNK_CACHE_HYSTERESIS_TICKS + 1);

        assert!(cache.chunks.is_empty());
    }
}
