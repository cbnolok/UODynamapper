use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::camera::RenderZoom;
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
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
use std::collections::{BTreeSet, HashSet};

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
    pixel_width: u16,
    pixel_height: u16,
) -> StaticBillboardBounds {
    let world_xz_per_pixel = static_world_xz_per_pixel(source);
    let world_y_per_pixel = static_world_y_per_pixel(source);
    let local_min_x = offset_x_pixels as f32 * world_xz_per_pixel;
    let local_max_x = local_min_x + pixel_width as f32 * world_xz_per_pixel;
    let local_min_y = -(offset_y_pixels as f32 * world_y_per_pixel);
    let local_max_y = local_min_y + pixel_height as f32 * world_y_per_pixel;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticVisualKind {
    CcRegularArt { art_id: u16 },
    EcRegularArt { art_id: u32 },
    TexLandEcArt { art_id: u32 },
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
const DEFAULT_PRIORITY_HEIGHT: i8 = 10;
const SURFACE_LIKE_DEPTH_CLASS_OFFSET: f32 = -4.0;
const STATIC_DEPTH_TIE_BREAK_STEP: f32 = 0.000_001;

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

    if meta.is_surface_like() {
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
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> Option<u32> {
    let Some(meta) = tilemeta else {
        return None;
    };
    if !meta.is_surface_like() {
        return None;
    }

    let Some(package) = tex_land_ec else {
        return None;
    };

    if let Some(slot_id) = package.resolve_runtime_slot_id(meta.cc_texture_id) {
        return Some(slot_id);
    }

    let mut unique_slots = BTreeSet::new();
    for record in package
        .terrain_provenance()
        .iter()
        .filter(|record| record.selected_texture_id == meta.ec_texture_id)
    {
        if record.canonical_slot_id != 0
            && record.canonical_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.canonical_slot_id).is_some()
        {
            unique_slots.insert(record.canonical_slot_id);
        }

        if record.alias_slot_id != 0
            && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID
            && package.present_slot(record.alias_slot_id).is_some()
        {
            unique_slots.insert(record.alias_slot_id);
        }

        if unique_slots.len() > 1 {
            return None;
        }
    }

    unique_slots.into_iter().next()
}

fn resolve_static_visual_kind(
    art_source: ClientTextureSource,
    tile_graphic: u16,
    tilemeta: Option<&udd_assets::tilemeta::TileMetaItemTile>,
    tex_art_ec: Option<&udd_assets::tex_art_ec::TexArtEcPackage>,
    tex_land_ec: Option<&udd_assets::tex_land_ec::TexLandEcPackage>,
) -> StaticVisualKind {
    match art_source {
        ClientTextureSource::Cc => {
            let cc_texture_id = tilemeta
                .map(|meta| meta.cc_texture_id as u16)
                .unwrap_or(tile_graphic);
            StaticVisualKind::CcRegularArt {
                art_id: cc_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
            }
        }
        ClientTextureSource::Ec => resolve_ec_static_visual_kind(
            tile_graphic,
            tex_art_ec.is_some_and(|package| package.present_slot(tile_graphic as u32).is_some()),
            resolve_surface_like_tex_land_ec_slot_id(tilemeta, tex_land_ec).is_some(),
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
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
    pub _pad_inst: u32,
    pub color_rgba: [f32; 4], // for dot mode
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
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
    pub _pad_inst: u32,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StaticArtCollectStats {
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
    mut instances: ResMut<RenderStaticInstances>,
    mut land_instances: ResMut<RenderStaticLandInstances>,
    mut chunk_batches: ResMut<RenderStaticChunkBatches>,
    mut debug_state: ResMut<StaticArtCollectDebugState>,
    source_state: Res<StaticArtSourceState>,
    // TODO: Need a way to get the currently visible chunks from the terrain system.
    // We can iterate over existing land chunk entities to find which blocks to draw.
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    instances.0.clear();
    land_instances.0.clear();
    chunk_batches.sprite.clear();
    chunk_batches.ground.clear();

    if !settings.worldmap_rendering.enable_statics {
        return;
    }

    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();

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
    let mut visible_chunks = 0usize;
    let mut visited_blocks = 0usize;
    let mut source_tiles = 0usize;
    let mut ground_land_tiles = 0usize;
    let mut atlas_hits = 0usize;
    let mut atlas_misses = 0usize;
    let mut unique_requested_pages = HashSet::new();

    // Height conversion factor from UO units to our world Y units.
    // Usually z is roughly 1 unit = 0.1 world units (or similar).
    // The land shader does: world.y = z * 0.1
    let height_scale = 0.1;

    for tcm in chunks_q.iter() {
        if tcm.parent_map_id != map_id {
            continue;
        }

        visible_chunks += 1;

        let chunk_key = StaticChunkBatchKey {
            map_id,
            gx: tcm.gx,
            gy: tcm.gy,
            scale: tcm.scale,
        };
        let mut chunk_sprite_instances = Vec::new();
        let mut chunk_ground_instances = Vec::new();

        let chunk_scale = tcm.scale;

        let start_gx = tcm.gx * CHUNK_STORAGE_BLOCKS_DIM;
        let start_gy = tcm.gy * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gx = start_gx + chunk_scale * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gy = start_gy + chunk_scale * CHUNK_STORAGE_BLOCKS_DIM;

        for gy in start_gy..end_gy {
            for gx in start_gx..end_gx {
                visited_blocks += 1;
                let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                    continue;
                };

                source_tiles += tiles.len();

                for tile in tiles {
                    // Very simple culling for dot mode: only render tall or wide things to save instances
                    if is_dot_mode && tile.z < 10 {
                        continue;
                    }

                    let tilemeta = tilemeta_res
                        .as_ref()
                        .and_then(|meta| meta.0.item_tile(tile.graphic as u32));

                    let world_x = (gx * MAP_STORAGE_BLOCK_TILE_DIM) as f32 + tile.x_offset() as f32;
                    let world_z = (gy * MAP_STORAGE_BLOCK_TILE_DIM) as f32 + tile.y_offset() as f32;
                    let depth_class = resolve_static_depth_class(tilemeta);

                    // Compute the is_wet GPU flag from tiledata flags.
                    // Bit 0 of is_wet_flags = 1 when the tile has the IsWet tiledata property.
                    // The art shader uses this flag to apply the animated water UV distortion.
                    let is_wet_flags: u32 =
                        tilemeta.map_or(0, |m| if m.flags & TILE_FLAG_WET != 0 { 1 } else { 0 });

                    if is_dot_mode {
                        let base_world_y = (tile.z as f32) * height_scale;
                        let priority_z_units =
                            resolve_priority_z_units(tile.z, tilemeta, depth_class);
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
                                _pad_inst: 0,
                                color_rgba: [
                                    color[2] as f32 / 255.0,
                                    color[1] as f32 / 255.0,
                                    color[0] as f32 / 255.0,
                                    1.0,
                                ],
                            });
                        }
                    } else {
                        let Some(art_source) = art_source else {
                            continue;
                        };
                        let visual_kind = resolve_static_visual_kind(
                            art_source,
                            tile.graphic,
                            tilemeta,
                            tex_art_ec_res.as_ref().map(|package| &*package.0),
                            tex_land_ec_res.as_ref().map(|package| &*package.0),
                        );

                        let bias = if matches!(visual_kind, StaticVisualKind::TexLandEcArt { .. }) {
                            GROUND_ART_Y_BIAS
                        } else {
                            depth_class_y_bias(depth_class)
                        };
                        let encoded_depth_class = depth_class.encoded();
                        let base_world_y = (tile.z as f32) * height_scale;
                        let priority_z_units =
                            resolve_priority_z_units(tile.z, tilemeta, depth_class);
                        let world_y = base_world_y + bias;
                        let (billboard_source, offset_x_pixels, offset_y_pixels, resolved_sprite) =
                            match visual_kind {
                                StaticVisualKind::CcRegularArt { art_id } => {
                                    let Some(tex_art_cc) = tex_art_cc_res.as_ref().map(|x| &x.0)
                                    else {
                                        continue;
                                    };
                                    let offset_x_pixels =
                                        tilemeta.map(|meta| meta.cc_offset_x).unwrap_or(0);
                                    let offset_y_pixels =
                                        tilemeta.map(|meta| meta.cc_offset_y).unwrap_or(0);

                                    if let Some(slot) = tex_art_cc.present_slot(art_id as u32) {
                                        unique_requested_pages.insert(slot.page_index as u64);
                                    }

                                    (
                                        ClientTextureSource::Cc,
                                        offset_x_pixels,
                                        offset_y_pixels,
                                        sprite_atlas.resolve_cc(tex_art_cc, art_id),
                                    )
                                }
                                StaticVisualKind::TexLandEcArt { .. } => {
                                    let Some(tex_land_ec) = tex_land_ec_res.as_ref().map(|x| &x.0)
                                    else {
                                        continue;
                                    };
                                    let Some(runtime_slot_id) =
                                        resolve_surface_like_tex_land_ec_slot_id(
                                            tilemeta,
                                            Some(tex_land_ec),
                                        )
                                    else {
                                        continue;
                                    };

                                    if let Some(slot) = tex_land_ec.present_slot(runtime_slot_id) {
                                        unique_requested_pages
                                            .insert((1u64 << 63) | slot.page_index as u64);
                                    }

                                    (
                                        ClientTextureSource::Ec,
                                        0,
                                        0,
                                        ground_atlas
                                            .resolve_tex_land_ec(tex_land_ec, runtime_slot_id),
                                    )
                                }
                                StaticVisualKind::EcRegularArt { art_id } => {
                                    let Some(tex_art_ec) = tex_art_ec_res.as_ref().map(|x| &x.0)
                                    else {
                                        continue;
                                    };
                                    let offset_x_pixels =
                                        tilemeta.map(|meta| meta.ec_offset_x).unwrap_or(0);
                                    let offset_y_pixels =
                                        tilemeta.map(|meta| meta.ec_offset_y).unwrap_or(0);

                                    if let Some(slot) = tex_art_ec.present_slot(art_id) {
                                        unique_requested_pages.insert(slot.page_index as u64);
                                    }

                                    (
                                        ClientTextureSource::Ec,
                                        offset_x_pixels,
                                        offset_y_pixels,
                                        sprite_atlas.resolve_ec(tex_art_ec, art_id),
                                    )
                                }
                            };

                        let (anchored_world_x, anchored_world_z) =
                            apply_static_world_anchor_translation(world_x, world_z);

                        if let Some(resolved) = resolved_sprite {
                            atlas_hits += 1;
                            if matches!(visual_kind, StaticVisualKind::TexLandEcArt { .. }) {
                                ground_land_tiles += 1;
                                let bounds = resolve_surface_like_ground_quad_bounds();
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
                                    _pad_inst: 0,
                                    color_rgba: [1.0, 1.0, 1.0, 1.0],
                                });
                            } else {
                                let bounds = resolve_static_billboard_bounds(
                                    billboard_source,
                                    offset_x_pixels,
                                    offset_y_pixels,
                                    resolved.pixel_width,
                                    resolved.pixel_height,
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
                                    _pad_inst: 0,
                                    color_rgba: [1.0, 1.0, 1.0, 1.0],
                                });
                            }
                        } else {
                            atlas_misses += 1;
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

        if !chunk_sprite_instances.is_empty() {
            let start = instances.0.len() as u32;
            let count = chunk_sprite_instances.len() as u32;
            instances.0.extend(chunk_sprite_instances);
            chunk_batches.sprite.push(StaticChunkBatch {
                key: chunk_key,
                start,
                count,
            });
        }

        if !chunk_ground_instances.is_empty() {
            let start = land_instances.0.len() as u32;
            let count = chunk_ground_instances.len() as u32;
            land_instances.0.extend(chunk_ground_instances);
            chunk_batches.ground.push(StaticChunkBatch {
                key: chunk_key,
                start,
                count,
            });
        }
    }

    let stats = StaticArtCollectStats {
        map_id,
        dot_mode: is_dot_mode,
        visible_chunks,
        visited_blocks,
        source_tiles,
        ground_land_tiles,
        unique_requested_pages: unique_requested_pages.len(),
        resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
        pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
        atlas_capacity_pages: sprite_atlas.active_layers as usize
            + ground_atlas.active_layers as usize,
        atlas_hits,
        atlas_misses,
        emitted_instances: instances.0.len() + land_instances.0.len(),
    };

    if debug_state.last != Some(stats) {
        console_logger::one(
            LogSev::Info,
            LogAbout::RenderWorldArt,
            &format!(
                "static art collect: map={} dot_mode={} chunks={} blocks={} tiles={} ground_land_tiles={} unique_pages={} resident_pages={} pending_pages={} capacity={} atlas_hits={} atlas_misses={} emitted={}",
                stats.map_id,
                stats.dot_mode,
                stats.visible_chunks,
                stats.visited_blocks,
                stats.source_tiles,
                stats.ground_land_tiles,
                stats.unique_requested_pages,
                stats.resident_pages,
                stats.pending_pages,
                stats.atlas_capacity_pages,
                stats.atlas_hits,
                stats.atlas_misses,
                stats.emitted_instances,
            ),
        );
        debug_state.last = Some(stats);
    }
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

    #[test]
    fn classic_billboard_bounds_keep_existing_scale() {
        let bounds = resolve_static_billboard_bounds(ClientTextureSource::Cc, 11, 7, 44, 88);

        approx_eq(bounds.local_min_x, 11.0 * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_max_x, (11.0 + 44.0) * CC_WORLD_XZ_PER_PIXEL);
        approx_eq(bounds.local_min_y, -(7.0 * CC_WORLD_Y_PER_PIXEL));
        approx_eq(bounds.local_max_y, (88.0 - 7.0) * CC_WORLD_Y_PER_PIXEL);
    }

    #[test]
    fn enhanced_billboard_bounds_normalize_to_classic_world_footprint() {
        let cc_bounds = resolve_static_billboard_bounds(ClientTextureSource::Cc, 0, 0, 44, 44);
        let ec_bounds = resolve_static_billboard_bounds(ClientTextureSource::Ec, 0, 0, 64, 64);

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
        let bounds = resolve_static_billboard_bounds(ClientTextureSource::Ec, 64, 32, 64, 64);

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
    fn surface_like_ec_tiles_prefer_land_path_over_direct_art_slot() {
        let visual_kind = resolve_ec_static_visual_kind(196, true, true);

        assert_eq!(visual_kind, StaticVisualKind::TexLandEcArt { art_id: 196 },);
    }

    #[test]
    fn non_surface_like_ec_tiles_stay_on_regular_art_path() {
        let visual_kind = resolve_ec_static_visual_kind(196, true, false);

        assert_eq!(visual_kind, StaticVisualKind::EcRegularArt { art_id: 196 },);
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
            _pad_inst: 0,
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
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 96);
    }

    #[test]
    fn ground_instance_stride_stays_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<GroundTileInstance>(), 96);
    }
}
