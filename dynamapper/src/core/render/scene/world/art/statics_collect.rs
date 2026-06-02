use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::camera::RenderZoom;
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::multis::{
    expanded_multi_part_z, multi_id_from_static_graphic, MultiDefinitionsRes,
};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::texture_cache::art::{GroundArtPageAtlas, SpriteArtPageAtlas};
use crate::core::uo_files_loader::{
    TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes,
};
use crate::prelude::*;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bytemuck::{Pod, Zeroable};
use std::collections::{BTreeSet, HashMap, HashSet};

const CLASSIC_STATIC_ART_ID_OFFSET: u16 = 0x4000;
const ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS: f32 = 1.41421356237;
const CC_TILE_PIXEL_WIDTH: f32 = 44.0;
const EC_TILE_PIXEL_WIDTH: f32 = 64.0;
const STATIC_WORLD_Y_PER_XZ_PIXEL: f32 = 7.5 * 0.1;
const CC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / CC_TILE_PIXEL_WIDTH;
const CC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * CC_WORLD_XZ_PER_PIXEL;
const EC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / EC_TILE_PIXEL_WIDTH;
const EC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * EC_WORLD_XZ_PER_PIXEL;
const STATIC_ART_Y_BIAS: f32 = 0.002;
const GROUND_ART_Y_BIAS: f32 = 0.0;
const EC_STATIC_TILE_TRANSLATION_X: f32 = 0.5;
const EC_STATIC_TILE_TRANSLATION_Z: f32 = 1.5;
const CLASSIC_WATER_LAND_TILE_ID: u32 = 168;
const EC_WATER_BASE_LAYER_INDEX: u32 = 0;
const STATIC_CHUNK_CACHE_HYSTERESIS_TICKS: u64 = 30;
const UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT: usize = 8;

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

fn apply_static_world_anchor_translation(world_x: f32, world_z: f32) -> (f32, f32) {
    (
        world_x + EC_STATIC_TILE_TRANSLATION_X,
        world_z + EC_STATIC_TILE_TRANSLATION_Z,
    )
}

fn surface_like_static_world_anchor(
    visual_kind: StaticVisualKind,
    world_x: f32,
    world_z: f32,
) -> (f32, f32) {
    match visual_kind {
        StaticVisualKind::TexLandEcArt { .. } => (world_x, world_z),
        StaticVisualKind::CcRegularArt { .. } | StaticVisualKind::EcRegularArt { .. } => {
            apply_static_world_anchor_translation(world_x, world_z)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticVisualKind {
    CcRegularArt { art_id: u16, fallback_art_id: u16 },
    EcRegularArt { art_id: u32 },
    TexLandEcArt { art_id: u32 },
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
    output: &mut Vec<StaticRenderTile>,
) {
    output.clear();

    let base_world_x =
        (block_x * MAP_STORAGE_BLOCK_TILE_DIM) as i32 + source_tile.x_offset() as i32;
    let base_world_z =
        (block_y * MAP_STORAGE_BLOCK_TILE_DIM) as i32 + source_tile.y_offset() as i32;
    let parts = multi_id_from_static_graphic(source_tile.graphic)
        .and_then(|multi_id| multis.and_then(|multis| multis.parts(multi_id)));

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

    if static_tile_is_surface_like(tilemeta) {
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
    let Some(meta) = tilemeta else {
        return None;
    };
    if !static_tile_uses_tex_land_ec_surface_path(tilemeta) {
        return None;
    }

    let Some(package) = tex_land_ec else {
        return None;
    };

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
    }
}

fn surface_like_ground_material_payload(_is_wet_flags: u32) -> (u32, u32) {
    (0, 0)
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
            StaticVisualKind::CcRegularArt {
                art_id: tile_graphic.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
                fallback_art_id: fallback_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
            }
        }
        ClientTextureSource::Ec => resolve_ec_static_visual_kind(
            tile_graphic,
            tex_art_ec.is_some_and(|package| package.present_slot(tile_graphic as u32).is_some()),
            resolve_surface_like_tex_land_ec_slot_id(
                tile_graphic as u32,
                tilemeta_package,
                tilemeta,
                tex_land_ec,
            )
            .is_some(),
        ),
    }
}

fn resolve_ec_static_visual_kind(
    tile_graphic: u16,
    has_tex_art_ec_slot: bool,
    has_surface_like_land_slot: bool,
) -> StaticVisualKind {
    if has_surface_like_land_slot {
        StaticVisualKind::TexLandEcArt {
            art_id: tile_graphic as u32,
        }
    } else if has_tex_art_ec_slot {
        StaticVisualKind::EcRegularArt {
            art_id: tile_graphic as u32,
        }
    } else {
        StaticVisualKind::EcRegularArt {
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
    pub is_wet_flags: u32,
    pub hue_id: u32,
    pub hue_flags: u32,
    pub _pad_inst: u32,
    pub _pad_hue: [u32; 2],
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
struct StaticChunkCacheConfig {
    map_id: u32,
    dot_mode: bool,
    art_source: Option<ClientTextureSource>,
    sprite_atlas_mapping_revision: u64,
    ground_atlas_mapping_revision: u64,
    static_light_signature: u64,
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
            (light.world_y * 100.0).round().max(0.0) as u32,
            (light.width_world * 100.0).round().max(0.0) as u32,
            (light.height_world * 100.0).round().max(0.0) as u32,
        ] {
            signature ^= value as u64;
            signature = signature.wrapping_mul(0x100000001b3);
        }
    }
    signature
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StaticArtCollectStats {
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
}

#[derive(Resource, Default)]
pub struct StaticArtCollectDebugState {
    pub last: Option<StaticArtCollectStats>,
}

#[derive(Resource, Default)]
pub struct StaticArtSourceState {
    pub active_source: Option<ClientTextureSource>,
    pub warned_unavailable_source: Option<ClientTextureSource>,
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
    ),
    mut debug_state: ResMut<StaticArtCollectDebugState>,
    source_state: Res<StaticArtSourceState>,
    static_lights: Res<super::static_lights::RenderStaticLightInstances>,
    // TODO: Need a way to get the currently visible chunks from the terrain system.
    // We can iterate over existing land chunk entities to find which blocks to draw.
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    if !settings.world_rendering.enable_statics {
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
    let mut visited_blocks = 0usize;
    let mut source_tiles = 0usize;
    let mut ground_land_tiles = 0usize;
    let mut unresolved_surface_like_tiles = 0usize;
    let mut unresolved_surface_like_sample = [0u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT];
    let mut unresolved_surface_like_sample_len = 0usize;
    let mut atlas_hits = 0usize;
    let mut atlas_misses = 0usize;
    let mut unique_requested_pages = HashSet::new();
    let cache_config = StaticChunkCacheConfig {
        map_id,
        dot_mode: is_dot_mode,
        art_source,
        sprite_atlas_mapping_revision: sprite_atlas.0.mapping_revision(),
        ground_atlas_mapping_revision: ground_atlas.0.mapping_revision(),
        static_light_signature: static_light_signature(&static_lights, map_id),
    };
    let visible_tick = outputs.3.begin_frame();
    let config_changed = outputs.3.sync_config(cache_config);

    // Height conversion factor from UO units to our world Y units.
    // Usually z is roughly 1 unit = 0.1 world units (or similar).
    // The land shader does: world.y = z * 0.1
    let height_scale = 0.1;

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
    let visible_chunks = visible_chunk_keys.len();

    let visible_set_unchanged = !config_changed && outputs.3.last_visible_chunk_keys == visible_chunk_keys;
    let all_visible_chunks_complete = visible_chunk_keys.iter().all(|chunk_key| {
        outputs
            .3
            .chunks
            .get(chunk_key)
            .is_some_and(|chunk| chunk.stats.atlas_misses == 0)
    });

    if visible_set_unchanged && all_visible_chunks_complete {
        let mut visited_blocks = 0usize;
        let mut source_tiles = 0usize;
        let mut ground_land_tiles = 0usize;
        let mut unresolved_surface_like_tiles = 0usize;
        let mut unresolved_surface_like_sample = [0u16; UNRESOLVED_SURFACE_LIKE_SAMPLE_LIMIT];
        let mut unresolved_surface_like_sample_len = 0usize;
        let mut atlas_hits = 0usize;
        let mut atlas_misses = 0usize;
        let mut unique_requested_pages = HashSet::new();

        for chunk_key in &visible_chunk_keys {
            let Some(chunk) = outputs.3.chunks.get_mut(chunk_key) else {
                continue;
            };
            chunk.last_visible_tick = visible_tick;
            visited_blocks += chunk.stats.visited_blocks;
            source_tiles += chunk.stats.source_tiles;
            ground_land_tiles += chunk.stats.ground_land_tiles;
            unresolved_surface_like_tiles += chunk.stats.unresolved_surface_like_tiles;
            for tile_id in chunk.stats.unresolved_surface_like_sample
                [..chunk.stats.unresolved_surface_like_sample_len]
                .iter()
                .copied()
            {
                push_sample_tile_id(
                    &mut unresolved_surface_like_sample,
                    &mut unresolved_surface_like_sample_len,
                    tile_id,
                );
            }
            atlas_hits += chunk.stats.atlas_hits;
            atlas_misses += chunk.stats.atlas_misses;
            unique_requested_pages.extend(chunk.stats.requested_pages.iter().copied());
        }

        log_static_collect_stats(
            &mut debug_state,
            StaticArtCollectStats {
                map_id,
                dot_mode: is_dot_mode,
                visible_chunks,
                visited_blocks,
                source_tiles,
                ground_land_tiles,
                unresolved_surface_like_tiles,
                unresolved_surface_like_sample,
                unresolved_surface_like_sample_len,
                unique_requested_pages: unique_requested_pages.len(),
                resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
                pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
                atlas_capacity_pages: sprite_atlas.active_layers as usize
                    + ground_atlas.active_layers as usize,
                atlas_hits,
                atlas_misses,
                emitted_instances: outputs.0.0.len() + outputs.1.0.len(),
            },
        );
        return;
    }

    outputs.0.0.clear();
    outputs.1.0.clear();
    outputs.2.sprite.clear();
    outputs.2.ground.clear();

    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();

    for chunk_key in visible_chunk_keys.iter().copied() {
        let reuse_cached_chunk = if let Some(chunk) = outputs.3.chunks.get_mut(&chunk_key) {
            chunk.last_visible_tick = visible_tick;
            chunk.stats.atlas_misses == 0
        } else {
            false
        };

        if !reuse_cached_chunk {
            let mut chunk_sprite_instances = Vec::new();
            let mut chunk_ground_instances = Vec::new();
            let mut chunk_requested_pages = HashSet::new();
            let mut chunk_stats = CachedStaticChunkStats::default();
            let mut render_tiles = Vec::new();

            let start_gx = chunk_key.gx * CHUNK_STORAGE_BLOCKS_DIM;
            let start_gy = chunk_key.gy * CHUNK_STORAGE_BLOCKS_DIM;
            let end_gx = start_gx + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;
            let end_gy = start_gy + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;

            for gy in start_gy..end_gy {
                for gx in start_gx..end_gx {
                    chunk_stats.visited_blocks += 1;
                    let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                        continue;
                    };

                    chunk_stats.source_tiles += tiles.len();

                    for tile in tiles {
                        collect_static_render_tiles(
                            *tile,
                            gx,
                            gy,
                            multis_res.as_deref(),
                            &mut render_tiles,
                        );

                        for render_tile in &render_tiles {
                            if is_dot_mode && render_tile.z < 10 {
                                continue;
                            }

                            let tilemeta = tilemeta_res
                                .as_ref()
                                .and_then(|meta| meta.0.item_tile(render_tile.graphic as u32));

                            let world_x = render_tile.world_x;
                            let world_z = render_tile.world_z;
                            let (hue_id, hue_flags) = static_hue_payload(render_tile.hue);
                            let depth_class = resolve_static_depth_class(tilemeta);
                            let is_wet_flags = tilemeta.map_or(0, |m| {
                                if m.flags & TILE_FLAG_WET != 0 { 1 } else { 0 }
                            });
                            let base_world_y = (render_tile.z as f32) * height_scale;
                            let local_light_rgba = static_local_light_rgba(
                                &static_lights,
                                map_id,
                                world_x,
                                world_z,
                                base_world_y,
                            );

                            if is_dot_mode {
                                let priority_z_units =
                                    resolve_priority_z_units(render_tile.z, tilemeta, depth_class);
                                let bias = depth_class_y_bias(depth_class);
                                let encoded_depth_class = depth_class.encoded();
                                let world_y = base_world_y + bias;

                                if let Some(meta) = tilemeta {
                                    let color = meta.radar_color;
                                    chunk_sprite_instances.push(SpriteInstance {
                                        world_x,
                                        world_z,
                                        world_y,
                                        layer: 0,
                                        depth_class: encoded_depth_class,
                                        base_world_y,
                                        uv_min: [0.0, 0.0],
                                        uv_max: [0.0, 0.0],
                                        local_min: [0.0, 0.0],
                                        local_max: [1.0, 1.0],
                                        tile_x: world_x,
                                        tile_y: world_z,
                                        priority_z_units,
                                        sort_bias_ordinal: 0,
                                        is_wet_flags,
                                        hue_id: 0,
                                        hue_flags: 0,
                                        _pad_inst: 0,
                                        _pad_hue: [0; 2],
                                        local_light_rgba,
                                        color_rgba: [
                                            color[2] as f32 / 255.0,
                                            color[1] as f32 / 255.0,
                                            color[0] as f32 / 255.0,
                                            1.0,
                                        ],
                                    });
                                }
                                continue;
                            }

                            let Some(art_source) = art_source else {
                                continue;
                            };
                            let visual_kind = resolve_static_visual_kind(
                                art_source,
                                render_tile.graphic,
                                tilemeta_res.as_ref().map(|res| &*res.0),
                                tilemeta,
                                tex_art_ec_res.as_ref().map(|package| &*package.0),
                                tex_land_ec_res.as_ref().map(|package| &*package.0),
                            );

                            let bias =
                                if matches!(visual_kind, StaticVisualKind::TexLandEcArt { .. }) {
                                    GROUND_ART_Y_BIAS
                                } else {
                                    depth_class_y_bias(depth_class)
                                };
                            let encoded_depth_class = depth_class.encoded();
                            let priority_z_units =
                                resolve_priority_z_units(render_tile.z, tilemeta, depth_class);
                            let world_y = base_world_y + bias;
                            let (
                                billboard_source,
                                resolved_sprite,
                                texture_stretch,
                            ) =
                                match visual_kind {
                                    StaticVisualKind::CcRegularArt {
                                        art_id,
                                        fallback_art_id,
                                    } => {
                                        let Some(tex_art_cc) =
                                            tex_art_cc_res.as_ref().map(|x| &x.0)
                                        else {
                                            continue;
                                        };
                                        let resolved_sprite = if let Some(slot) =
                                            tex_art_cc.present_slot(art_id as u32)
                                        {
                                            chunk_requested_pages.insert(slot.page_index as u64);
                                            sprite_atlas.resolve_cc(tex_art_cc, art_id)
                                        } else if fallback_art_id != art_id {
                                            if let Some(slot) =
                                                tex_art_cc.present_slot(fallback_art_id as u32)
                                            {
                                                chunk_requested_pages.insert(slot.page_index as u64);
                                            }
                                            sprite_atlas.resolve_cc(tex_art_cc, fallback_art_id)
                                        } else {
                                            None
                                        };

                                        (
                                            ClientTextureSource::Cc,
                                            resolved_sprite,
                                            0.0,
                                        )
                                    }
                                    StaticVisualKind::TexLandEcArt { .. } => {
                                        let Some(tex_land_ec) =
                                            tex_land_ec_res.as_ref().map(|x| &x.0)
                                        else {
                                            continue;
                                        };
                                        let Some(resolution) =
                                            resolve_surface_like_tex_land_ec_slot_id(
                                                render_tile.graphic as u32,
                                                tilemeta_res.as_ref().map(|res| &*res.0),
                                                tilemeta,
                                                Some(tex_land_ec),
                                            )
                                        else {
                                            chunk_stats.unresolved_surface_like_tiles += 1;
                                            push_sample_tile_id(
                                                &mut chunk_stats.unresolved_surface_like_sample,
                                                &mut chunk_stats.unresolved_surface_like_sample_len,
                                                render_tile.graphic,
                                            );
                                            continue;
                                        };

                                        if let Some(slot) =
                                            tex_land_ec.present_slot(resolution.runtime_slot_id)
                                        {
                                            chunk_requested_pages
                                                .insert((1u64 << 63) | slot.page_index as u64);
                                        }

                                        (
                                            ClientTextureSource::Ec,
                                            ground_atlas.resolve_tex_land_ec(
                                                tex_land_ec,
                                                resolution.runtime_slot_id,
                                            ),
                                            resolution.texture_repetition,
                                        )
                                    }
                                    StaticVisualKind::EcRegularArt { art_id } => {
                                        let Some(tex_art_ec) =
                                            tex_art_ec_res.as_ref().map(|x| &x.0)
                                        else {
                                            continue;
                                        };

                                        if let Some(slot) = tex_art_ec.present_slot(art_id) {
                                            chunk_requested_pages.insert(slot.page_index as u64);
                                        }

                                        (
                                            ClientTextureSource::Ec,
                                            sprite_atlas.resolve_ec(tex_art_ec, art_id),
                                            0.0,
                                        )
                                    }
                                };

                            let (anchored_world_x, anchored_world_z) =
                                surface_like_static_world_anchor(visual_kind, world_x, world_z);
                            let local_light_rgba = static_local_light_rgba(
                                &static_lights,
                                map_id,
                                anchored_world_x,
                                anchored_world_z,
                                base_world_y,
                            );

                            if let Some(resolved) = resolved_sprite {
                                chunk_stats.atlas_hits += 1;
                                if matches!(visual_kind, StaticVisualKind::TexLandEcArt { .. }) {
                                    chunk_stats.ground_land_tiles += 1;
                                    let bounds = resolve_surface_like_ground_quad_bounds();
                                    let (material_payload, material_flags) =
                                        surface_like_ground_material_payload(is_wet_flags);
                                    chunk_ground_instances.push(GroundTileInstance {
                                        world_x: anchored_world_x,
                                        world_z: anchored_world_z,
                                        world_y,
                                        layer: resolved.layer,
                                        depth_class: encoded_depth_class,
                                        base_world_y,
                                        uv_min: [resolved.uv_min.x, resolved.uv_min.y],
                                        uv_max: [resolved.uv_max.x, resolved.uv_max.y],
                                        local_min: [bounds.local_min_x, bounds.local_min_z],
                                        local_max: [bounds.local_max_x, bounds.local_max_z],
                                        tile_x: anchored_world_x,
                                        tile_y: anchored_world_z,
                                        priority_z_units,
                                        sort_bias_ordinal: 0,
                                        is_wet_flags,
                                        texture_stretch,
                                        hue_id,
                                        hue_flags,
                                        material_payload,
                                        material_flags,
                                        local_light_rgba,
                                        color_rgba: [1.0, 1.0, 1.0, 1.0],
                                    });
                                } else {
                                    let bounds = resolve_static_billboard_bounds(
                                        billboard_source,
                                        resolved.offset_x,
                                        resolved.offset_y,
                                        resolved.logical_width,
                                        resolved.logical_height,
                                    );

                                    chunk_sprite_instances.push(SpriteInstance {
                                        world_x: anchored_world_x,
                                        world_z: anchored_world_z,
                                        world_y,
                                        layer: resolved.layer,
                                        depth_class: encoded_depth_class,
                                        base_world_y,
                                        uv_min: [resolved.uv_min.x, resolved.uv_min.y],
                                        uv_max: [resolved.uv_max.x, resolved.uv_max.y],
                                        local_min: [bounds.local_min_x, bounds.local_min_y],
                                        local_max: [bounds.local_max_x, bounds.local_max_y],
                                        tile_x: anchored_world_x,
                                        tile_y: anchored_world_z,
                                        priority_z_units,
                                        sort_bias_ordinal: 0,
                                        is_wet_flags,
                                        hue_id,
                                        hue_flags,
                                        _pad_inst: 0,
                                        _pad_hue: [0; 2],
                                        local_light_rgba,
                                        color_rgba: [1.0, 1.0, 1.0, 1.0],
                                    });
                                }
                            } else {
                                chunk_stats.atlas_misses += 1;
                            }
                        }
                    }
                }
            }

            chunk_sprite_instances.sort_by(|a, b| {
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
            assign_sprite_depth_tie_breakers(&mut chunk_sprite_instances);

            chunk_ground_instances.sort_by(|a, b| {
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
            assign_ground_depth_tie_breakers(&mut chunk_ground_instances);

            let mut requested_pages = chunk_requested_pages.into_iter().collect::<Vec<_>>();
            requested_pages.sort_unstable();
            chunk_stats.requested_pages = requested_pages;

            outputs.3.chunks.insert(
                chunk_key,
                CachedStaticChunk {
                    sprite_instances: chunk_sprite_instances,
                    ground_instances: chunk_ground_instances,
                    stats: chunk_stats,
                    last_visible_tick: visible_tick,
                },
            );
        }

        let Some(chunk) = outputs.3.chunks.get(&chunk_key) else {
            continue;
        };

        visited_blocks += chunk.stats.visited_blocks;
        source_tiles += chunk.stats.source_tiles;
        ground_land_tiles += chunk.stats.ground_land_tiles;
        unresolved_surface_like_tiles += chunk.stats.unresolved_surface_like_tiles;
        for tile_id in chunk.stats.unresolved_surface_like_sample
            [..chunk.stats.unresolved_surface_like_sample_len]
            .iter()
            .copied()
        {
            push_sample_tile_id(
                &mut unresolved_surface_like_sample,
                &mut unresolved_surface_like_sample_len,
                tile_id,
            );
        }
        atlas_hits += chunk.stats.atlas_hits;
        atlas_misses += chunk.stats.atlas_misses;
        unique_requested_pages.extend(chunk.stats.requested_pages.iter().copied());

        if !chunk.sprite_instances.is_empty() {
            let start = outputs.0.0.len() as u32;
            let count = chunk.sprite_instances.len() as u32;
            outputs.0.0.extend_from_slice(&chunk.sprite_instances);
            outputs.2.sprite.push(StaticChunkBatch {
                key: chunk_key,
                start,
                count,
            });
        }

        if !chunk.ground_instances.is_empty() {
            let start = outputs.1.0.len() as u32;
            let count = chunk.ground_instances.len() as u32;
            outputs.1.0.extend_from_slice(&chunk.ground_instances);
            outputs.2.ground.push(StaticChunkBatch {
                key: chunk_key,
                start,
                count,
            });
        }
    }

    outputs.3.prune_stale(visible_tick);
    outputs.3.last_visible_chunk_keys = visible_chunk_keys;

    log_static_collect_stats(
        &mut debug_state,
        StaticArtCollectStats {
            map_id,
            dot_mode: is_dot_mode,
            visible_chunks,
            visited_blocks,
            source_tiles,
            ground_land_tiles,
            unresolved_surface_like_tiles,
            unresolved_surface_like_sample,
            unresolved_surface_like_sample_len,
            unique_requested_pages: unique_requested_pages.len(),
            resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
            pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
            atlas_capacity_pages: sprite_atlas.active_layers as usize
                + ground_atlas.active_layers as usize,
            atlas_hits,
            atlas_misses,
            emitted_instances: outputs.0.0.len() + outputs.1.0.len(),
        },
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

    #[test]
    fn classic_billboard_bounds_keep_existing_scale() {
        let bounds = resolve_static_billboard_bounds(ClientTextureSource::Cc, 11, 7, 44.0, 88.0);

        approx_eq(bounds.local_min_x, 11.0 * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_max_x, (11.0 + 44.0) * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_min_y, -(7.0 * CC_WORLD_Y_PER_PIXEL));
        approx_eq(bounds.local_max_y, (88.0 - 7.0) * CC_WORLD_Y_PER_PIXEL);
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
            StaticVisualKind::TexLandEcArt { art_id: 42 },
            10.0,
            20.0,
        );

        approx_eq(anchor.0, 10.0);
        approx_eq(anchor.1, 20.0);
    }

    #[test]
    fn regular_static_art_keeps_sprite_world_anchor() {
        let anchor = surface_like_static_world_anchor(
            StaticVisualKind::CcRegularArt {
                art_id: 42,
                fallback_art_id: 42,
            },
            10.0,
            20.0,
        );

        approx_eq(anchor.0, 10.0 + EC_STATIC_TILE_TRANSLATION_X);
        approx_eq(anchor.1, 20.0 + EC_STATIC_TILE_TRANSLATION_Z);
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
            StaticVisualKind::CcRegularArt {
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
        let (material_payload, material_flags) = surface_like_ground_material_payload(1);

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
    fn surface_like_ec_tiles_prefer_land_path_over_direct_art_slot() {
        let visual_kind = resolve_ec_static_visual_kind(196, true, true);

        assert_eq!(visual_kind, StaticVisualKind::TexLandEcArt { art_id: 196 },);
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
        let visual_kind = resolve_ec_static_visual_kind(196, true, false);

        assert_eq!(visual_kind, StaticVisualKind::EcRegularArt { art_id: 196 },);
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

        collect_static_render_tiles(source_tile, 2, 3, None, &mut output);

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

        collect_static_render_tiles(source_tile, 2, 3, Some(&definitions), &mut output);

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
            _pad_inst: 0,
            _pad_hue: [0; 2],
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
        assert_eq!(std::mem::size_of::<GroundTileInstance>(), 128);
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
            static_light_signature: 0,
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
            static_light_signature: 0,
        });

        assert!(cache.chunks.is_empty());

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
            static_light_signature: 1,
        });

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
