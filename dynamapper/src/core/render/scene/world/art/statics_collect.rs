use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::camera::RenderZoom;
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::render::scene::world::land::tile_atlas::TileAtlas;
use crate::core::multis::{
    expanded_multi_part_z, multi_id_from_static_graphic, MultiDefinitionsRes,
};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::{LazyStaticsStore, StaticsStoreRes};
use crate::core::texture_cache::art::{GroundArtPageAtlas, ResolvedArtSprite, SpriteArtPageAtlas};
use crate::core::uo_files_loader::{
    TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes,
};
use crate::prelude::*;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeSet, HashMap, HashSet};

const CLASSIC_STATIC_ART_ID_OFFSET: u16 = 0x4000;
const ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS: f32 = std::f32::consts::SQRT_2;
const CC_TILE_PIXEL_WIDTH: f32 = 44.0;
const EC_TILE_PIXEL_WIDTH: f32 = 64.0;
// Original legacy art projects four square sprite pixels per raw Z unit.
const STATIC_WORLD_Y_PER_XZ_PIXEL: f32 = UO_Z_TO_WORLD_SCALE / (4.0 * CC_WORLD_XZ_PER_PIXEL);
const CC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / CC_TILE_PIXEL_WIDTH;
const CC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * CC_WORLD_XZ_PER_PIXEL;
const EC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / EC_TILE_PIXEL_WIDTH;
const EC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * EC_WORLD_XZ_PER_PIXEL;
const STATIC_ART_Y_BIAS: f32 = 0.002;
// Ground quads use rasterized depth and need the same small geometric separation
// from terrain as flat sprite quads to avoid coplanar depth fighting.
const GROUND_ART_Y_BIAS: f32 = STATIC_ART_Y_BIAS;
// The original 0x45a080 geometry constructor tests this TileArt flag.
const EC_CAMERA_QUAD_FLAG: u64 = uocf::enhanced::tileart::TaeFlag::Rotated.bits();
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
const UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT: usize = 8;
const STATIC_BILLBOARD_RIGHT_XZ: Vec2 =
    Vec2::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);

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

fn static_world_xz_per_pixel(source: ClientTextureSource) -> f32 {
    match source {
        ClientTextureSource::Cc => CC_WORLD_XZ_PER_PIXEL,
        ClientTextureSource::Ec => EC_WORLD_XZ_PER_PIXEL,
    }
}

fn static_world_y_per_pixel(source: ClientTextureSource) -> f32 {
    match source {
        ClientTextureSource::Cc => CC_WORLD_Y_PER_PIXEL,
        ClientTextureSource::Ec => EC_WORLD_Y_PER_PIXEL,
    }
}

pub(crate) fn resolve_static_billboard_bounds(
    source: ClientTextureSource,
    offset_x_pixels: i16,
    offset_y_pixels: i16,
    logical_width: f32,
    logical_height: f32,
) -> StaticBillboardBounds {
    let world_xz_per_pixel = static_world_xz_per_pixel(source);
    let world_y_per_pixel = static_world_y_per_pixel(source);
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
fn resolve_cc_surface_like_flat_quad_bounds(
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
fn cc_surface_like_flat_quad_center_offsets(logical_height: f32) -> (f32, f32) {
    let forward_half_height =
        logical_height * CC_WORLD_XZ_PER_PIXEL * std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    (
        CC_STATIC_TILE_TRANSLATION_X - forward_half_height,
        CC_STATIC_TILE_TRANSLATION_Z - forward_half_height,
    )
}

fn apply_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + EC_STATIC_TILE_TRANSLATION_X,
        world_z + EC_STATIC_TILE_TRANSLATION_Z,
    )
}

fn apply_cc_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + CC_STATIC_TILE_TRANSLATION_X,
        world_z + CC_STATIC_TILE_TRANSLATION_Z,
    )
}

fn surface_like_static_world_anchor(
    visual_kind: StaticVisualKind,
    world_x: f32,
    world_z: f32,
) -> (f32, f32) {
    match visual_kind {
        StaticVisualKind::TexLandEc { .. } | StaticVisualKind::EcSurface { .. } => (world_x, world_z),
        // CC billboards are force-centered in their bbox, so they need a symmetric anchor
        // (see CC_STATIC_TILE_TRANSLATION_*); EC billboards carry their own per-sprite offsets
        // and keep the legacy asymmetric anchor they were validated against.
        StaticVisualKind::CcRegular { .. } => {
            apply_cc_static_world_anchor_translation(world_x, world_z)
        }
        StaticVisualKind::EcRegular { .. } => {
            apply_static_world_anchor_translation(world_x, world_z)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum StaticVisualKind {
    CcRegular { art_id: u16, fallback_art_id: u16 },
    EcRegular { art_id: u32 },
    EcSurface { art_id: u32 },
    TexLandEc { resolution: SurfaceLikeTexLandEcResolution },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StaticRenderTile {
    graphic: u16,
    world_x: f32,
    world_z: f32,
    z: i8,
    hue: u16,
}

fn collect_static_render_tiles(
    source_tile: uocf::classic::statics::PackedStaticTile,
    block_x: u32,
    block_y: u32,
    multis: Option<&MultiDefinitionsRes>,
    show_hidden_multi_parts: bool,
    output: &mut Vec<StaticRenderTile>,
) {
    output.clear();

    let base_world_x =
        (block_x * MAP_STORAGE_BLOCK_TILE_DIM) as i32 + source_tile.x_offset() as i32;
    let base_world_z =
        (block_y * MAP_STORAGE_BLOCK_TILE_DIM) as i32 + source_tile.y_offset() as i32;
    let parts = multi_id_from_static_graphic(source_tile.graphic)
        .and_then(|multi_id| {
            multis.and_then(|multis| multis.parts(multi_id, show_hidden_multi_parts))
        });

    let Some(parts) = parts else {
        output.push(StaticRenderTile {
            graphic: source_tile.graphic,
            world_x: base_world_x as f32,
            world_z: base_world_z as f32,
            z: source_tile.z,
            hue: source_tile.hue,
        });
        return;
    };

    for part in parts {
        output.push(StaticRenderTile {
            graphic: part.item_id,
            world_x: (base_world_x + part.x as i32) as f32,
            world_z: (base_world_z + part.y as i32) as f32,
            z: expanded_multi_part_z(source_tile.z, part.z),
            hue: source_tile.hue,
        });
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
const TILE_FLAG_SURFACE: u64 = 0x200;
const TILE_FLAG_BRIDGE: u64 = 0x400;
const TILE_FLAG_FOLIAGE: u64 = 0x20_000;
const TILE_FLAG_ROOF: u64 = 0x1000_0000;
/// IsWet flag — tile has tidal/water properties (matches tiledata.mul bit 0x80).
/// When set, the art shader applies the animated sin/cos UV distortion.
const TILE_FLAG_WET: u64 = 0x80;
const STATIC_HUE_FLAG_APPLY: u32 = 1;
const STATIC_HUE_FLAG_EC_SPRITE: u32 = 2;
const DEFAULT_PRIORITY_HEIGHT: i8 = 10;
const SURFACE_LIKE_DEPTH_CLASS_OFFSET: f32 = -4.0;
const STATIC_DEPTH_TIE_BREAK_STEP: f32 = 0.000_001;

pub(crate) fn static_tile_is_surface_like(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
) -> bool {
    let Some(meta) = tilemeta else {
        return false;
    };

    meta.is_surface_like() || meta.flags & (TILE_FLAG_SURFACE | TILE_FLAG_WET) != 0
}

fn static_tile_uses_tex_land_ec_surface_path(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
) -> bool {
    static_tile_is_surface_like(tilemeta)
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

fn ec_static_camera_quad(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> bool {
    tilemeta.is_some_and(|meta| meta.flags & EC_CAMERA_QUAD_FLAG != 0)
}

pub(crate) fn resolve_static_depth_class(
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
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
    // flat floor. Keep its effective height and regular sprite depth priority.
    if static_tile_is_surface_like(tilemeta) && meta.flags & TILE_FLAG_BRIDGE == 0 {
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

fn effective_priority_height(tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>) -> i8 {
    let Some(meta) = tilemeta else {
        return 0;
    };

    let mut height = meta.height;
    let has_background_or_surface = meta.flags & (TILE_FLAG_BACKGROUND | TILE_FLAG_SURFACE) != 0;

    if height == 0 && !has_background_or_surface {
        height = DEFAULT_PRIORITY_HEIGHT;
    }

    if meta.flags & TILE_FLAG_BRIDGE != 0 {
        height /= 2;
    }

    height
}

pub(crate) fn resolve_priority_z_units(
    tile_z: i8,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    depth_class: StaticDepthClass,
) -> f32 {
    if depth_class == StaticDepthClass::SurfaceLikeFloor {
        return tile_z as f32;
    }

    tile_z as f32 + effective_priority_height(tilemeta) as f32
}

fn depth_class_logical_offset(depth_class: StaticDepthClass) -> f32 {
    match depth_class {
        StaticDepthClass::SurfaceLikeFloor => SURFACE_LIKE_DEPTH_CLASS_OFFSET,
        StaticDepthClass::Background => -0.001,
        StaticDepthClass::Roof => 0.002,
        StaticDepthClass::Foliage => 2.0,
        StaticDepthClass::Regular => 0.0,
    }
}

fn decode_depth_class(encoded: u32) -> StaticDepthClass {
    match encoded {
        1 => StaticDepthClass::Background,
        2 => StaticDepthClass::Foliage,
        3 => StaticDepthClass::Roof,
        4 => StaticDepthClass::SurfaceLikeFloor,
        _ => StaticDepthClass::Regular,
    }
}

pub(crate) fn static_depth_key(
    tile_x: f32,
    tile_y: f32,
    priority_z_units: f32,
    depth_class: StaticDepthClass,
) -> f32 {
    static_depth_key_with_tie_break(tile_x, tile_y, priority_z_units, depth_class, 0)
}

fn static_depth_key_with_tie_break(
    tile_x: f32,
    tile_y: f32,
    priority_z_units: f32,
    depth_class: StaticDepthClass,
    tie_break_ordinal: u32,
) -> f32 {
    (tile_x + tile_y)
        + (127.0 + priority_z_units) * 0.01
        + depth_class_logical_offset(depth_class)
        + tie_break_ordinal as f32 * STATIC_DEPTH_TIE_BREAK_STEP
}

fn assign_sprite_depth_tie_breakers(instances: &mut [SpriteInstance]) {
    let mut last_base_key: Option<f32> = None;
    let mut tie_break_ordinal = 0u32;

    for instance in instances.iter_mut() {
        let base_key = static_depth_key(
            instance.tile_x,
            instance.tile_y,
            instance.priority_z_units,
            decode_depth_class(instance.depth_class),
        );

        if last_base_key.is_some_and(|last| last == base_key) {
            tie_break_ordinal = tie_break_ordinal.saturating_add(1);
        } else {
            last_base_key = Some(base_key);
            tie_break_ordinal = 0;
        }

        instance.sort_bias_ordinal = tie_break_ordinal;
    }
}

fn push_sample_tile_id(
    sample: &mut [u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT],
    sample_len: &mut usize,
    tile_id: u16,
) {
    if sample[..*sample_len].contains(&tile_id) {
        return;
    }

    if *sample_len < UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT {
        sample[*sample_len] = tile_id;
        *sample_len += 1;
    }
}

fn assign_ground_depth_tie_breakers(instances: &mut [GroundTileInstance]) {
    let mut last_base_key: Option<f32> = None;
    let mut tie_break_ordinal = 0u32;

    for instance in instances.iter_mut() {
        let base_key = static_depth_key(
            instance.tile_x,
            instance.tile_y,
            instance.priority_z_units,
            decode_depth_class(instance.depth_class),
        );

        if last_base_key.is_some_and(|last| last == base_key) {
            tie_break_ordinal = tie_break_ordinal.saturating_add(1);
        } else {
            last_base_key = Some(base_key);
            tie_break_ordinal = 0;
        }

        instance.sort_bias_ordinal = tie_break_ordinal;
    }
}

fn resolve_surface_like_tex_land_ec_slot_id(
    tile_id: u32,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> Option<SurfaceLikeTexLandEcResolution> {
    let meta = tilemeta?;
    if !static_tile_uses_tex_land_ec_surface_path(tilemeta) {
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
                        });
                    }
                }
            }
        }
    }

    let main_ec_texture_id = tilemeta_package
        .and_then(|package| surface_like_visible_ec_texture_ref(tile_id, meta, package))
        .map(|texture_ref| texture_ref.texture_id)
        .or_else(|| surface_like_legacy_ec_texture_id_fallback(meta));
    let Some(main_ec_texture_id) = main_ec_texture_id else {
        if let Some(resolution) = wet_surface_like_water_resolution(meta, package) {
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

    if let Some(resolution) = wet_surface_like_water_resolution(meta, package) {
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
    tilemeta: &udd_assets::tilemeta::TileMetaItemTile,
    package: &'a udd_assets::tilemeta::TileMetaPackage,
) -> Option<&'a udd_assets::tilemeta::TileMetaItemTextureRef> {
    if tilemeta.flags & TILE_FLAG_WET == 0 {
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
) -> Option<u32> {
    if tilemeta.ec_texture_id == 0 || tilemeta.flags & TILE_FLAG_WET != 0 {
        return None;
    }

    Some(tilemeta.ec_texture_id)
}

fn wet_surface_like_water_resolution(
    tilemeta: &udd_assets::tilemeta::TileMetaItemTile,
    package: &udd_assets::tex_land_ec::TexLandEcPackage,
) -> Option<SurfaceLikeTexLandEcResolution> {
    if tilemeta.flags & TILE_FLAG_WET == 0 {
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
struct SurfaceLikeTexLandEcResolution {
    runtime_slot_id: u32,
    texture_repetition: f32,
    liquid_payload: Option<u32>,
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
            let fallback_texture_id = tilemeta
                .map(|meta| meta.cc_texture_id as u16)
                .unwrap_or(tile_graphic);
            StaticVisualKind::CcRegular {
                art_id: tile_graphic.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
                fallback_art_id: fallback_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
            }
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

fn resolve_static_visual_kind_cached(
    art_source: ClientTextureSource,
    tile_graphic: u16,
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> StaticVisualKind {
    match art_source {
        ClientTextureSource::Cc => {
            let fallback_texture_id = tilemeta
                .map(|meta| meta.cc_texture_id as u16)
                .unwrap_or(tile_graphic);
            StaticVisualKind::CcRegular {
                art_id: tile_graphic.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
                fallback_art_id: fallback_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
            }
        }
        ClientTextureSource::Ec => resolve_ec_static_visual_kind(
            tile_graphic,
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_static()),
            tex_art_ec.and_then(|package| package.present_slot(tile_graphic as u32))
                .is_some_and(|slot| slot.is_land())
                && tileart_surface_has_visible_primary(tilemeta_package.map_or(&[], |package| package.item_texture_refs(tile_graphic as u32))),
            surface_resolution_cache.resolve_surface_like_tex_land_ec_slot_id(
                art_source,
                tile_graphic as u32,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            ),
        ),
    }
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
    !refs.iter().find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected())
        .is_some_and(|texture_ref| matches!(texture_ref.stable_role(),
        EcMaterialStableRole::AlphaMask | EcMaterialStableRole::GenericMask
            | EcMaterialStableRole::Noise | EcMaterialStableRole::NormalLike
            | EcMaterialStableRole::ImageSupport | EcMaterialStableRole::EffectOnlyMetadata
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

fn static_hue_payload(hue_id: u16) -> (u32, u32) {
    if hue_id == 0 {
        (0, 0)
    } else {
        (u32::from(hue_id), STATIC_HUE_FLAG_APPLY)
    }
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
    pub tile_x: f32,
    pub tile_y: f32,
    pub priority_z_units: f32,
    pub sort_bias_ordinal: u32,
    /// Tiledata flags packed for the GPU.
    /// Bit 0: is_wet (IsWet tiledata flag → animated water UV distortion in sprite shader).
    /// Bit 1: ground surface blend; bits 2..9: NW,N,NE,W,E,SW,S,SE same-material neighbors.
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
    pub local_light_rgba: [f32; 4],
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
    pub is_wet_flags: u32,
    pub texture_stretch: f32,
    pub hue_id: u32,
    pub hue_flags: u32,
    pub material_payload: u32,
    pub material_flags: u32,
    /// Original item graphic selects its own optional fourth water source.
    pub material_item_id: u32,
    pub _pad_material: [u32; 3],
    pub local_light_rgba: [f32; 4],
    pub color_rgba: [f32; 4],
}

#[derive(Resource, Default)]
pub struct RenderStaticInstances(pub Vec<SpriteInstance>);

#[derive(Resource, Default)]
pub struct RenderStaticLandInstances(pub Vec<GroundTileInstance>);

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
    sprite_atlas_mapping_revision: u64,
    ground_atlas_mapping_revision: u64,
    land_metadata_revision: u64,
    static_art_local_light_signature: Option<u64>,
    show_hidden_multi_parts: bool,
}

#[derive(Clone, Debug, Default)]
struct CachedStaticChunkStats {
    visited_blocks: usize,
    source_tiles: usize,
    ground_land_tiles: usize,
    unresolved_surface_like_tiles: usize,
    unresolved_surface_like_sample: [u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT],
    unresolved_surface_like_sample_len: usize,
    atlas_hits: usize,
    atlas_misses: usize,
    requested_pages: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
struct CachedStaticChunk {
    sprite_instances: Vec<SpriteInstance>,
    ground_instances: Vec<GroundTileInstance>,
    stats: CachedStaticChunkStats,
    last_visible_tick: u64,
}

#[derive(Resource, Default)]
pub struct StaticChunkRenderCache {
    config: Option<StaticChunkCacheConfig>,
    current_tick: u64,
    chunks: HashMap<StaticChunkBatchKey, CachedStaticChunk>,
    last_visible_chunk_keys: Vec<StaticChunkBatchKey>,
    last_incomplete_retry_atlas_signature: Option<StaticAtlasRetrySignature>,
}

impl StaticChunkRenderCache {
    fn begin_frame(&mut self) -> u64 {
        self.current_tick = self.current_tick.saturating_add(1);
        self.current_tick
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
        aggregate.unresolved_surface_like_tiles += chunk.stats.unresolved_surface_like_tiles;
        for tile_id in chunk.stats.unresolved_surface_like_sample
            [..chunk.stats.unresolved_surface_like_sample_len]
            .iter()
            .copied()
        {
            push_sample_tile_id(
                &mut aggregate.unresolved_surface_like_sample,
                &mut aggregate.unresolved_surface_like_sample_len,
                tile_id,
            );
        }
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
                "static art collect (1/2): map={} dot_mode={} chunks={} blocks={} tiles={} ground_land_tiles={} unresolved_surface_like={}",
                stats.map_id,
                stats.dot_mode,
                stats.visible_chunks,
                stats.visited_blocks,
                stats.source_tiles,
                stats.ground_land_tiles,
                stats.unresolved_surface_like_tiles,
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
        if stats.unresolved_surface_like_tiles > 0 {
            let sample = stats.unresolved_surface_like_sample[..stats.unresolved_surface_like_sample_len]
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            console_logger::one(
                LogSev::Warn,
                LogAbout::RenderWorldArt,
                &format!(
                    "Skipped {} unresolved surface-like EC static tiles; sample tile ids: [{}].",
                    stats.unresolved_surface_like_tiles,
                    sample,
                ),
            );
        }
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

fn static_art_local_light_rgba(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    world_y: f32,
) -> [f32; 4] {
    if enabled {
        static_local_light_rgba(lights, map_id, world_x, world_z, world_y)
    } else {
        [0.0; 4]
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

fn strongest_static_local_light_sample(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    samples: &[(f32, f32, f32)],
) -> [f32; 4] {
    let mut best = [0.0; 4];
    for &(world_x, world_z, world_y) in samples {
        let sample = static_local_light_rgba(lights, map_id, world_x, world_z, world_y);
        if sample[3] > best[3] {
            best = sample;
        }
    }
    best
}

fn static_art_billboard_local_light_rgba(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: StaticBillboardBounds,
) -> [f32; 4] {
    if enabled {
        static_billboard_local_light_rgba(lights, map_id, world_x, world_z, base_world_y, bounds)
    } else {
        [0.0; 4]
    }
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

fn static_billboard_local_light_rgba(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: StaticBillboardBounds,
) -> [f32; 4] {
    let center_x = (bounds.local_min_x + bounds.local_max_x) * 0.5;
    let center_y = (bounds.local_min_y + bounds.local_max_y) * 0.5;
    let samples = [
        (world_x, world_z, base_world_y),
        static_billboard_sample_world(world_x, world_z, base_world_y, center_x, center_y),
        static_billboard_sample_world(
            world_x,
            world_z,
            base_world_y,
            bounds.local_min_x,
            bounds.local_min_y,
        ),
        static_billboard_sample_world(
            world_x,
            world_z,
            base_world_y,
            bounds.local_max_x,
            bounds.local_min_y,
        ),
        static_billboard_sample_world(
            world_x,
            world_z,
            base_world_y,
            center_x,
            bounds.local_max_y,
        ),
    ];
    strongest_static_local_light_sample(lights, map_id, &samples)
}

fn static_art_ground_local_light_rgba(
    enabled: bool,
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: GroundQuadBounds,
) -> [f32; 4] {
    if enabled {
        static_ground_local_light_rgba(lights, map_id, world_x, world_z, base_world_y, bounds)
    } else {
        [0.0; 4]
    }
}

fn static_ground_local_light_rgba(
    lights: &super::static_lights::RenderStaticLightInstances,
    map_id: u32,
    world_x: f32,
    world_z: f32,
    base_world_y: f32,
    bounds: GroundQuadBounds,
) -> [f32; 4] {
    let center_x = (bounds.local_min_x + bounds.local_max_x) * 0.5;
    let center_z = (bounds.local_min_z + bounds.local_max_z) * 0.5;
    let samples = [
        (world_x, world_z, base_world_y),
        (world_x + center_x, world_z + center_z, base_world_y),
        (
            world_x + bounds.local_min_x,
            world_z + bounds.local_min_z,
            base_world_y,
        ),
        (
            world_x + bounds.local_max_x,
            world_z + bounds.local_min_z,
            base_world_y,
        ),
        (
            world_x + bounds.local_max_x,
            world_z + bounds.local_max_z,
            base_world_y,
        ),
        (
            world_x + bounds.local_min_x,
            world_z + bounds.local_max_z,
            base_world_y,
        ),
    ];
    strongest_static_local_light_sample(lights, map_id, &samples)
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
    pub unresolved_surface_like_tiles: usize,
    pub unresolved_surface_like_sample: [u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT],
    pub unresolved_surface_like_sample_len: usize,
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
}

#[derive(Resource, Default)]
pub struct StaticArtSourceState {
    pub active_source: Option<ClientTextureSource>,
    pub warned_unavailable_source: Option<ClientTextureSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StaticSurfaceLikeResolutionCacheKey {
    art_source: u8,
    tile_id: u32,
}

#[derive(Resource, Default)]
pub struct StaticSurfaceLikeResolutionCache {
    entries: HashMap<StaticSurfaceLikeResolutionCacheKey, Option<SurfaceLikeTexLandEcResolution>>,
}

impl StaticSurfaceLikeResolutionCache {
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
    mut source_state: ResMut<StaticArtSourceState>,
) {
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
    } else if effective_source == Some(requested_source) {
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
    is_dot_mode: bool,
    static_art_local_lights: bool,
    show_hidden_multi_parts: bool,
    height_scale: f32,
}

#[derive(Clone, Copy)]
struct StaticCollectResources<'a> {
    tex_art_cc: Option<&'a TexArtCcPackageRes>,
    tex_art_ec: Option<&'a TexArtEcPackageRes>,
    tex_land_ec: Option<&'a TexLandEcPackageRes>,
    tilemeta: Option<&'a TileMetaPackageRes>,
    multis: Option<&'a MultiDefinitionsRes>,
    static_lights: &'a super::static_lights::RenderStaticLightInstances,
    tile_atlas: &'a TileAtlas,
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
}

#[derive(Default)]
struct StaticChunkBuildState {
    sprite_instances: Vec<SpriteInstance>,
    ground_instances: Vec<GroundTileInstance>,
    requested_pages: HashSet<u64>,
    stats: CachedStaticChunkStats,
    render_tiles: Vec<StaticRenderTile>,
}

#[derive(Default)]
struct StaticCollectSlowStats {
    visited_blocks: usize,
    source_tiles: usize,
    ground_land_tiles: usize,
    unresolved_surface_like_tiles: usize,
    unresolved_surface_like_sample: [u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT],
    unresolved_surface_like_sample_len: usize,
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
        self.unresolved_surface_like_tiles += chunk.stats.unresolved_surface_like_tiles;
        for tile_id in chunk.stats.unresolved_surface_like_sample
            [..chunk.stats.unresolved_surface_like_sample_len]
            .iter()
            .copied()
        {
            push_sample_tile_id(
                &mut self.unresolved_surface_like_sample,
                &mut self.unresolved_surface_like_sample_len,
                tile_id,
            );
        }
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
    multis_res: &'a Option<Res<MultiDefinitionsRes>>,
    static_lights: &'a super::static_lights::RenderStaticLightInstances,
    tile_atlas: &'a TileAtlas,
) -> StaticCollectResources<'a> {
    StaticCollectResources {
        tex_art_cc: tex_art_cc_res.as_ref().map(|res| &**res),
        tex_art_ec: tex_art_ec_res.as_ref().map(|res| &**res),
        tex_land_ec: tex_land_ec_res.as_ref().map(|res| &**res),
        tilemeta: tilemeta_res.as_ref().map(|res| &**res),
        multis: multis_res.as_ref().map(|res| &**res),
        static_lights,
        tile_atlas,
    }
}

fn static_chunk_cache_config(
    frame: StaticCollectFrameConfig,
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
    static_lights: &super::static_lights::RenderStaticLightInstances,
    tile_atlas: &TileAtlas,
) -> StaticChunkCacheConfig {
    StaticChunkCacheConfig {
        map_id: frame.map_id,
        dot_mode: frame.is_dot_mode,
        art_source: frame.art_source,
        sprite_atlas_mapping_revision: sprite_atlas.0.mapping_revision(),
        ground_atlas_mapping_revision: ground_atlas.0.mapping_revision(),
        land_metadata_revision: tile_atlas.content_revision,
        static_art_local_light_signature: frame
            .static_art_local_lights
            .then(|| static_light_signature(static_lights, frame.map_id)),
        show_hidden_multi_parts: frame.show_hidden_multi_parts,
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
            unresolved_surface_like_tiles: aggregate.unresolved_surface_like_tiles,
            unresolved_surface_like_sample: aggregate.unresolved_surface_like_sample,
            unresolved_surface_like_sample_len: aggregate.unresolved_surface_like_sample_len,
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
            let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                continue;
            };

            build.stats.source_tiles += tiles.len();

            for tile in tiles {
                collect_static_render_tiles(
                    *tile,
                    gx,
                    gy,
                    resources.multis,
                    frame.show_hidden_multi_parts,
                    &mut build.render_tiles,
                );

                for render_tile_index in 0..build.render_tiles.len() {
                    let render_tile = build.render_tiles[render_tile_index];
                    emit_static_render_tile(
                        render_tile,
                        frame,
                        resources,
                        sprite_atlas,
                        ground_atlas,
                        surface_resolution_cache,
                        &mut build,
                    );
                }
            }
        }
    }

    if !frame.is_dot_mode && frame.art_source == Some(ClientTextureSource::Ec) {
        if let (Some(tilemeta), Some(art)) = (resources.tilemeta_package(), resources.tex_art_ec_package()) {
            super::static_surface::apply_boundary_coverage(
                &mut build.sprite_instances, chunk_key, frame.height_scale,
                statics_store, tilemeta, art, resources.tile_atlas,
            );
        }
    }
    finish_rebuilt_static_chunk(build, visible_tick)
}

fn emit_static_render_tile(
    render_tile: StaticRenderTile,
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

    let tilemeta_package = resources.tilemeta_package();
    let tilemeta = tilemeta_package.and_then(|meta| meta.item_tile(render_tile.graphic as u32));
    if !frame.is_dot_mode && tilemeta.is_some_and(|meta| meta.is_nodraw()) {
        return;
    }
    let info = StaticTileRenderInfo {
        tile: render_tile,
        tilemeta,
        world_x: render_tile.world_x,
        world_z: render_tile.world_z,
        depth_class: resolve_static_depth_class(tilemeta),
        is_wet_flags: {
            let classic_wet = tilemeta.map_or(false, |meta| meta.flags & TILE_FLAG_WET != 0);
            let ec_wet = tilemeta_package.map_or(false, |pkg| {
                pkg.item_texture_refs(render_tile.graphic as u32).iter().any(|r| {
                    use udd_assets::tilemeta::EcMaterialSpeculativeRole;
                    matches!(
                        r.speculative_role(),
                        EcMaterialSpeculativeRole::LiquidRipple
                            | EcMaterialSpeculativeRole::LiquidReflectionSupport
                            | EcMaterialSpeculativeRole::LiquidEnvProbe
                            | EcMaterialSpeculativeRole::FoamHighlight
                            | EcMaterialSpeculativeRole::WaterfallSupport
                            | EcMaterialSpeculativeRole::LavaBubbleSupport
                    )
                })
            });
            // TODO Phase 10G: FoamHighlight, WaterfallSupport, LavaBubbleSupport, and
            // RefractionDistortionLike roles currently contribute to is_wet_flags (water
            // animation) but deserve dedicated extra-pass rendering (additive post-lighting
            // compositing for emissive/foam, screen-space copy for refraction).  The
            // speculative_role data is already stored in TileMetaItemTextureRef.
            u32::from(classic_wet || ec_wet)
        },
        base_world_y: (render_tile.z as f32) * frame.height_scale,
    };

    if frame.is_dot_mode {
        emit_dot_static_tile(info, frame, resources, build);
    } else {
        emit_art_static_tile(
            info,
            frame,
            resources,
            tilemeta_package,
            sprite_atlas,
            ground_atlas,
            surface_resolution_cache,
            build,
        );
    }
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

    let local_light_rgba = static_art_local_light_rgba(
        frame.static_art_local_lights,
        resources.static_lights,
        frame.map_id,
        info.world_x,
        info.world_z,
        info.base_world_y,
    );
    let priority_z_units =
        resolve_priority_z_units(info.tile.z, info.tilemeta, info.depth_class);
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
        local_light_rgba,
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
    tilemeta_package: Option<&udd_assets::tilemeta::TileMetaPackage>,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
    build: &mut StaticChunkBuildState,
) {
    let Some(art_source) = frame.art_source else {
        return;
    };
    let visual_kind = resolve_static_visual_kind_cached(
        art_source,
        info.tile.graphic,
        tilemeta_package,
        info.tilemeta,
        resources.tex_art_ec_package(),
        resources.tex_land_ec_package(),
        surface_resolution_cache,
    );

    let bias = if matches!(visual_kind, StaticVisualKind::TexLandEc { .. }) {
        GROUND_ART_Y_BIAS
    } else {
        depth_class_y_bias(info.depth_class)
    };
    let encoded_depth_class = info.depth_class.encoded();
    let priority_z_units =
        resolve_priority_z_units(info.tile.z, info.tilemeta, info.depth_class);
    let world_y = info.base_world_y + bias;
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
    let cc_surface_like_flat = matches!(art_source, ClientTextureSource::Cc)
        && (cc_static_tile_renders_flat(info.tilemeta)
            || resolved_sprite.as_ref().is_some_and(|sprite| {
                cc_ground_diamond(info.tilemeta, sprite.logical_width, sprite.logical_height)
                    && [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().all(|(dx, dy)| {
                        super::static_surface::map_height(resources.tile_atlas,
                            info.tile.world_x as u32 + dx, info.tile.world_z as u32 + dy) == Some(info.tile.z)
                    })
            }));

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
            let tex_land_ec = resources.tex_land_ec_package()?;

            if let Some(slot) = tex_land_ec.present_slot(resolution.runtime_slot_id) {
                requested_pages.insert((1u64 << 63) | slot.page_index as u64);
            }

            Some((
                ClientTextureSource::Ec,
                ground_atlas.resolve_tex_land_ec(tex_land_ec, resolution.runtime_slot_id),
                resolution.texture_repetition,
            ))
        }
        StaticVisualKind::EcRegular { art_id } | StaticVisualKind::EcSurface { art_id } => {
            let tex_art_ec = resources.tex_art_ec_package()?;

            if let Some(slot) = tex_art_ec.present_slot(art_id) {
                requested_pages.insert(slot.page_index as u64);
            }

            let texture_stretch = if matches!(visual_kind, StaticVisualKind::EcSurface { .. }) {
                resources.tilemeta_package()
                    .and_then(|package| package.item_texture_refs(art_id).iter()
                        .find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected()))
                    // Ripple-like inputs on small authored stains are not tiled albedo.
                    .filter(|texture_ref| texture_ref.speculative_role()
                        != udd_assets::tilemeta::EcMaterialSpeculativeRole::LiquidRipple)
                    .map(|texture_ref| valid_texture_repetition(texture_ref.texture_stretch))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            Some((ClientTextureSource::Ec, sprite_atlas.resolve_ec(tex_art_ec, art_id), texture_stretch))
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
    hue_id: u32,
    hue_flags: u32,
    build: &mut StaticChunkBuildState,
) {
    build.stats.ground_land_tiles += 1;
    let bounds = resolve_surface_like_ground_quad_bounds();
    let local_light_rgba = static_art_ground_local_light_rgba(
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
        tile_x: anchored_world_x,
        tile_y: anchored_world_z,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags,
        texture_stretch,
        hue_id,
        hue_flags,
        material_payload,
        material_flags,
        material_item_id: info.tile.graphic as u32,
        _pad_material: [0; 3],
        local_light_rgba,
        color_rgba: [1.0, 1.0, 1.0, 1.0],
    });
}

fn emit_billboard_static_instance(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    resolved: ResolvedArtSprite,
    billboard_source: ClientTextureSource,
    ec_surface: bool,
    texture_stretch: f32,
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
            billboard_source,
            resolved.offset_x,
            resolved.offset_y,
            resolved.logical_width,
            resolved.logical_height,
        )
    };
    let local_light_rgba = if ec_surface {
        static_art_ground_local_light_rgba(
            frame.static_art_local_lights,
            resources.static_lights,
            frame.map_id,
            anchored_world_x,
            anchored_world_z,
            info.base_world_y,
            resolve_surface_like_ground_quad_bounds(),
        )
    } else {
        static_art_billboard_local_light_rgba(
            frame.static_art_local_lights,
            resources.static_lights,
            frame.map_id,
            anchored_world_x,
            anchored_world_z,
            info.base_world_y,
            bounds,
        )
    };

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
        tile_x: anchored_world_x,
        tile_y: anchored_world_z,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags,
        hue_id,
        hue_flags: hue_flags | if billboard_source == ClientTextureSource::Ec { STATIC_HUE_FLAG_EC_SPRITE } else { 0 },
        flat_quad: if ec_surface { if ec_static_camera_quad(info.tilemeta) { 3 } else { 2 } } else { 0 },
        texture_stretch,
        material_item_id: info.tile.graphic as u32,
        local_light_rgba,
        color_rgba: [1.0, 1.0, 1.0, 1.0],
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
    let local_light_rgba = static_art_ground_local_light_rgba(
        frame.static_art_local_lights,
        resources.static_lights,
        frame.map_id,
        anchored_world_x,
        anchored_world_z,
        info.base_world_y,
        bounds,
    );
    // Anchor the flat diamond feet-first by the sprite's real pixel height (see
    // `cc_surface_like_flat_quad_center_offsets`): for the 44px plain-water sprites this lands
    // exactly on the cell center, so the diagonal basis in main.wesl maps the baked diamond's
    // edge vertices onto the cell corners as before; taller wave sprites extend further
    // up-screen from the same feet anchor instead of being wrongly re-centered. Depth-sort
    // keys (tile_x/tile_y below) stay at the cell origin to remain consistent with the EC
    // ground path.
    let (center_offset_x, center_offset_z) =
        cc_surface_like_flat_quad_center_offsets(resolved.logical_height);
    let centered_world_x = anchored_world_x + center_offset_x;
    let centered_world_z = anchored_world_z + center_offset_z;

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
        tile_x: anchored_world_x,
        tile_y: anchored_world_z,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags,
        hue_id,
        hue_flags,
        flat_quad: 1,
        texture_stretch: 0.0,
        material_item_id: 0,
        local_light_rgba,
        color_rgba: [1.0, 1.0, 1.0, 1.0],
    });
}

fn finish_rebuilt_static_chunk(
    mut build: StaticChunkBuildState,
    visible_tick: u64,
) -> CachedStaticChunk {
    sort_static_chunk_instances(&mut build.sprite_instances, &mut build.ground_instances);

    let mut requested_pages = build.requested_pages.into_iter().collect::<Vec<_>>();
    requested_pages.sort_unstable();
    build.stats.requested_pages = requested_pages;

    CachedStaticChunk {
        sprite_instances: build.sprite_instances,
        ground_instances: build.ground_instances,
        stats: build.stats,
        last_visible_tick: visible_tick,
    }
}

fn sort_static_chunk_instances(
    sprite_instances: &mut [SpriteInstance],
    ground_instances: &mut [GroundTileInstance],
) {
    sprite_instances.sort_by(|a, b| {
        let depth_a = static_depth_key(
            a.tile_x,
            a.tile_y,
            a.priority_z_units,
            decode_depth_class(a.depth_class),
        );
        let depth_b = static_depth_key(
            b.tile_x,
            b.tile_y,
            b.priority_z_units,
            decode_depth_class(b.depth_class),
        );
        depth_a
            .partial_cmp(&depth_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    assign_sprite_depth_tie_breakers(sprite_instances);

    ground_instances.sort_by(|a, b| {
        let depth_a = static_depth_key(
            a.tile_x,
            a.tile_y,
            a.priority_z_units,
            decode_depth_class(a.depth_class),
        );
        let depth_b = static_depth_key(
            b.tile_x,
            b.tile_y,
            b.priority_z_units,
            decode_depth_class(b.depth_class),
        );
        depth_a
            .partial_cmp(&depth_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    assign_ground_depth_tie_breakers(ground_instances);
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
            unresolved_surface_like_tiles: slow_stats.unresolved_surface_like_tiles,
            unresolved_surface_like_sample: slow_stats.unresolved_surface_like_sample,
            unresolved_surface_like_sample_len: slow_stats.unresolved_surface_like_sample_len,
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
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    settings: Res<crate::configs::settings::Settings>,
    scene_state: Res<SceneStateData>,
    zoom: Res<RenderZoom>,
    multis_res: Option<Res<MultiDefinitionsRes>>,
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
    let (chunks_q, tile_atlas) = terrain;
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
        is_dot_mode,
        static_art_local_lights: settings.world_rendering.enable_static_art_local_lights,
        show_hidden_multi_parts: settings.world_rendering.show_hidden_multi_parts,
        height_scale,
    };
    let resources = static_collect_resources(
        &tex_art_cc_res,
        &tex_art_ec_res,
        &tex_land_ec_res,
        &tilemeta_res,
        &multis_res,
        &static_lights,
        &tile_atlas,
    );
    let cache_config =
        static_chunk_cache_config(frame, &sprite_atlas, &ground_atlas, &static_lights, &tile_atlas);
    let visible_tick = outputs.3.begin_frame();
    let config_changed = outputs.3.sync_config(cache_config);

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
        let bounds = resolve_static_billboard_bounds(ClientTextureSource::Cc, 11, 7, 44.0, 88.0);

        approx_eq(bounds.local_min_x, 11.0 * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_max_x, (11.0 + 44.0) * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_min_y, -(7.0 * CC_WORLD_Y_PER_PIXEL));
        approx_eq(bounds.local_max_y, (88.0 - 7.0) * CC_WORLD_Y_PER_PIXEL);
    }

    #[test]
    fn upright_art_projects_square_pixels_and_four_pixels_per_z_unit() {
        let vertical_projection = (2.0f32 / 3.0).sqrt()
            * crate::core::render::scene::camera::ORTHO_WIDTH_SCALE_FACTOR;
        for source in [ClientTextureSource::Cc, ClientTextureSource::Ec] {
            approx_eq(static_world_y_per_pixel(source) * vertical_projection,
                static_world_xz_per_pixel(source));
        }
        approx_eq(UO_Z_TO_WORLD_SCALE / CC_WORLD_Y_PER_PIXEL, 4.0);
    }

    #[test]
    fn enhanced_billboard_bounds_normalize_to_classic_world_footprint() {
        let cc_bounds = resolve_static_billboard_bounds(ClientTextureSource::Cc, 0, 0, 44.0, 44.0);
        let ec_bounds = resolve_static_billboard_bounds(ClientTextureSource::Ec, 0, 0, 64.0, 64.0);

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
        let bounds = resolve_static_billboard_bounds(ClientTextureSource::Ec, 64, 32, 64.0, 64.0);

        approx_eq(bounds.local_min_x, ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS);
        approx_eq(bounds.local_min_y, -(32.0 * EC_WORLD_Y_PER_PIXEL));
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
    fn wet_surface_like_tiles_keep_surface_depth_and_land_redirection() {
        let tile = item_tile_with_flags(
            TILE_FLAG_WET,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );

        assert!(static_tile_is_surface_like(Some(&tile)));
        assert!(static_tile_uses_tex_land_ec_surface_path(Some(&tile)));
    }

    #[test]
    fn cc_flat_render_gate_targets_wet_water_not_surface_floors() {
        // Real "slate roof": SURFACE | Background, authored SurfaceLike, NOT wet, elevated.
        // It is surface-like (would enter the EC land path) but must stay a CC billboard.
        let roof = item_tile_with_flags(
            TILE_FLAG_SURFACE | TILE_FLAG_BACKGROUND,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );
        assert!(static_tile_is_surface_like(Some(&roof)));
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
                udd_assets::tilemeta::EcMaterialStableRole::NormalLike,
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

        assert!(static_tile_uses_tex_land_ec_surface_path(Some(&tile)));
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

        assert_eq!(surface_like_legacy_ec_texture_id_fallback(&tile), None);
    }

    #[test]
    fn non_wet_surface_like_tiles_can_use_legacy_ec_texture_fallback() {
        let mut tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);
        tile.ec_texture_id = 12_345;

        assert_eq!(surface_like_legacy_ec_texture_id_fallback(&tile), Some(12_345));
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
            udd_assets::tilemeta::EcMaterialStableRole::AlphaMask,
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
    fn terrain_surface_resolution_remains_available_without_tileart_material() {
        let resolution = test_surface_like_resolution();
        assert_eq!(
            resolve_ec_static_visual_kind(196, false, false, Some(resolution)),
            StaticVisualKind::TexLandEc { resolution },
        );
    }

    #[test]
    fn disabling_statics_clears_draw_batches_and_invalidates_output_reuse() {
        use bevy::ecs::system::RunSystemOnce;
        use udd_assets::tex_art_cc::PagePixelFormat;
        let mut world = World::new();
        let mut settings = crate::configs::settings::load_from_files();
        settings.world_rendering.enable_statics = false;
        let key = StaticChunkBatchKey { map_id: 0, gx: 1, gy: 2, scale: 1 };
        let batch = StaticChunkBatch { key, start: 0, count: 1 };
        let mut cache = StaticChunkRenderCache::default();
        cache.last_visible_chunk_keys.push(key);
        cache.chunks.insert(key, CachedStaticChunk::default());
        world.insert_resource(settings);
        world.insert_resource(StaticsStoreRes(Vec::new()));
        world.insert_resource(SceneStateData { map_id: 0 });
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
                udd_assets::tilemeta::EcMaterialStableRole::NormalLike,
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
    fn regular_static_tile_collects_without_multi_expansion() {
        let source_tile = uocf::classic::statics::PackedStaticTile {
            graphic: 7,
            xy_packed: 3 | (4 << 3),
            z: 5,
            hue: 0,
        };
        let mut output = Vec::new();

        collect_static_render_tiles(source_tile, 2, 3, None, false, &mut output);

        assert_eq!(
            output,
            vec![StaticRenderTile {
                graphic: 7,
                world_x: 19.0,
                world_z: 28.0,
                z: 5,
                hue: 0,
            }],
        );
    }

    #[test]
    fn multi_static_tile_expands_hidden_parts_only_on_request() {
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
        let source_tile = uocf::classic::statics::PackedStaticTile {
            graphic: crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET,
            xy_packed: 0,
            z: 0,
            hue: 0,
        };
        let mut output = Vec::new();

        collect_static_render_tiles(source_tile, 0, 0, Some(&definitions), false, &mut output);
        let graphics: Vec<u16> = output.iter().map(|tile| tile.graphic).collect();
        assert_eq!(graphics, vec![10]);

        collect_static_render_tiles(source_tile, 0, 0, Some(&definitions), true, &mut output);
        let graphics: Vec<u16> = output.iter().map(|tile| tile.graphic).collect();
        assert_eq!(graphics, vec![10, 11]);
    }

    #[test]
    fn multi_static_tile_expands_visible_parts_at_world_offsets() {
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
        let source_tile = uocf::classic::statics::PackedStaticTile {
            graphic: crate::core::multis::MULTI_STATIC_GRAPHIC_OFFSET,
            xy_packed: 3 | (4 << 3),
            z: 5,
            hue: 9,
        };
        let mut output = Vec::new();

        collect_static_render_tiles(source_tile, 2, 3, Some(&definitions), false, &mut output);

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
        assert_eq!(resolve_static_depth_class(None), StaticDepthClass::Regular);
    }

    #[test]
    fn depth_class_promotes_surface_like_floor_tiles() {
        let tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike);

        assert_eq!(
            resolve_static_depth_class(Some(&tile)),
            StaticDepthClass::SurfaceLikeFloor,
        );
    }

    #[test]
    fn bridge_stone_and_stairs_retain_volume_depth_and_height_priority() {
        // Installed bridge fixture: stone 1872 and stair pieces 1874/1895.
        for (flags, height, priority) in [
            (0x04002600, 10, 20.0),
            (0x80002600, 5, 17.0),
            (0x2600, 5, 17.0),
        ] {
            let mut tile = item_tile_with_flags(
                flags,
                udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
            );
            tile.height = height;
            assert!(static_tile_is_surface_like(Some(&tile)));
            let class = resolve_static_depth_class(Some(&tile));
            assert_eq!(class, StaticDepthClass::Regular);
            assert_eq!(resolve_priority_z_units(15, Some(&tile), class), priority);
        }
    }

    #[test]
    fn depth_class_gives_roof_priority_over_surface_like_hint() {
        let tile = item_tile_with_flags(
            TILE_FLAG_ROOF,
            udd_assets::tilemeta::TileMetaItemVisualKind::SurfaceLike,
        );

        assert_eq!(
            resolve_static_depth_class(Some(&tile)),
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
            resolve_static_depth_class(Some(&tile)),
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
            resolve_static_depth_class(Some(&tile)),
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
            resolve_static_depth_class(Some(&background)),
            StaticDepthClass::Background,
        );
        assert_eq!(
            resolve_static_depth_class(Some(&foliage)),
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
    fn effective_priority_height_defaults_zero_height_regulars_to_ten() {
        let tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);

        assert_eq!(effective_priority_height(Some(&tile)), 10);
    }

    #[test]
    fn effective_priority_height_keeps_background_zero_height_at_zero() {
        let tile = item_tile_with_flags(
            TILE_FLAG_BACKGROUND,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );

        assert_eq!(effective_priority_height(Some(&tile)), 0);
    }

    #[test]
    fn effective_priority_height_halves_bridge_height() {
        let mut tile = item_tile_with_flags(
            TILE_FLAG_BRIDGE,
            udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt,
        );
        tile.height = 12;

        assert_eq!(effective_priority_height(Some(&tile)), 6);
    }

    #[test]
    fn surface_like_priority_z_units_stay_on_base_z() {
        let priority_z_units =
            resolve_priority_z_units(7, None, StaticDepthClass::SurfaceLikeFloor);

        assert_eq!(priority_z_units, 7.0);
    }

    #[test]
    fn regular_priority_z_units_include_effective_height() {
        let mut tile =
            item_tile_with_flags(0, udd_assets::tilemeta::TileMetaItemVisualKind::RegularArt);
        tile.height = 12;

        let priority_z_units = resolve_priority_z_units(7, Some(&tile), StaticDepthClass::Regular);

        assert_eq!(priority_z_units, 19.0);
    }

    #[test]
    fn static_depth_key_applies_background_offset() {
        let regular = static_depth_key(100.0, 200.0, 19.0, StaticDepthClass::Regular);
        let background = static_depth_key(100.0, 200.0, 19.0, StaticDepthClass::Background);

        assert!(background < regular);
    }

    #[test]
    fn static_depth_key_places_surface_like_floor_below_background() {
        let surface_like = static_depth_key(100.0, 200.0, 7.0, StaticDepthClass::SurfaceLikeFloor);
        let background = static_depth_key(100.0, 200.0, 7.0, StaticDepthClass::Background);

        assert!(surface_like < background);
    }

    #[test]
    fn tie_breakers_increase_for_equal_depth_sprite_instances() {
        let base_instance = SpriteInstance {
            world_x: 0.0,
            world_z: 0.0,
            world_y: 0.0,
            layer: 0,
            depth_class: StaticDepthClass::Regular.encoded(),
            base_world_y: 0.0,
            uv_min: [0.0, 0.0],
            uv_max: [0.0, 0.0],
            local_min: [0.0, 0.0],
            local_max: [0.0, 0.0],
            tile_x: 10.0,
            tile_y: 20.0,
            priority_z_units: 15.0,
            sort_bias_ordinal: 0,
            is_wet_flags: 0,
            hue_id: 0,
            hue_flags: 0,
            flat_quad: 0,
            texture_stretch: 0.0,
            material_item_id: 0,
            local_light_rgba: [0.0; 4],
            color_rgba: [1.0, 1.0, 1.0, 1.0],
        };
        let mut instances = vec![
            base_instance,
            SpriteInstance {
                tile_x: 10.0,
                tile_y: 20.0,
                priority_z_units: 15.0,
                ..base_instance
            },
            SpriteInstance {
                tile_x: 11.0,
                tile_y: 20.0,
                priority_z_units: 15.0,
                ..base_instance
            },
        ];

        assign_sprite_depth_tie_breakers(&mut instances);

        assert_eq!(instances[0].sort_bias_ordinal, 0);
        assert_eq!(instances[1].sort_bias_ordinal, 1);
        assert_eq!(instances[2].sort_bias_ordinal, 0);
    }

    #[test]
    fn sprite_instance_stride_stays_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 128);
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
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
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
            sprite_atlas_mapping_revision: 1,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
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
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
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
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
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
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: Some(0),
            show_hidden_multi_parts: false,
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
            sprite_atlas_mapping_revision: 0,
            ground_atlas_mapping_revision: 0,
            land_metadata_revision: 0,
            static_art_local_light_signature: Some(1),
            show_hidden_multi_parts: false,
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

    #[test]
    fn static_art_local_light_sampling_requires_opt_in() {
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], 0.0, 0.0, 0.0),
        ]);

        assert_eq!(
            static_art_local_light_rgba(false, &lights, 1, 0.0, 0.0, 0.0),
            [0.0; 4]
        );
        assert!(static_art_local_light_rgba(true, &lights, 1, 0.0, 0.0, 0.0)[3] > 0.0);
    }

    #[test]
    fn billboard_local_light_samples_bounds_not_only_origin() {
        let bounds = StaticBillboardBounds {
            local_min_x: 4.0,
            local_max_x: 4.0,
            local_min_y: 0.0,
            local_max_y: 0.0,
        };
        let sample = static_billboard_sample_world(0.0, 0.0, 0.0, 4.0, 0.0);
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], sample.0, sample.1, sample.2),
        ]);

        assert_eq!(static_local_light_rgba(&lights, 1, 0.0, 0.0, 0.0)[3], 0.0);
        assert!(static_billboard_local_light_rgba(&lights, 1, 0.0, 0.0, 0.0, bounds)[3] > 0.0);
    }

    #[test]
    fn ground_local_light_samples_quad_extent() {
        let bounds = GroundQuadBounds {
            local_min_x: 0.0,
            local_max_x: 4.0,
            local_min_z: 0.0,
            local_max_z: 0.0,
        };
        let lights = super::super::static_lights::RenderStaticLightInstances(vec![
            test_static_light_at(0, [1.0, 0.72, 0.42], 4.0, 0.0, 0.0),
        ]);

        assert_eq!(static_local_light_rgba(&lights, 1, 0.0, 0.0, 0.0)[3], 0.0);
        assert!(static_ground_local_light_rgba(&lights, 1, 0.0, 0.0, 0.0, bounds)[3] > 0.0);
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
