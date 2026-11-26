use crate::settings::ClientTextureSource;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::render::view::camera::{PlayerCamera, RenderZoom};
use crate::render::terrain::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::render::assets::tile_atlas::{AtlasCells, LandCells, LandSnapshot, TileAtlas};
use crate::world::multis::{
    expanded_multi_part_z, MultiDefinitionsRes, MultiPart, PlacedMulti, PlacedMultisRes,
};
use crate::world::caves::{is_buried_floor, CaveMap, CaveStatic, CavesRes, PlayerCave};
use crate::world::z_view::{PackageLand, ZViewCut, ZViewLimit};
use crate::render::scene::SceneStateData;
use crate::world::statics::{LazyStaticsStore, StaticsStoreRes};
use super::static_chunk_builder::{ChunkMultis, StaticBuildShared, StaticChunkBuilder, StaticChunkJob};
use crate::render::assets::art::{ArtPageAtlasSet, GroundArtPageAtlas, ResolvedArtSprite, SpriteArtPageAtlas};
use crate::render::assets::art::{ArtPageGeometry, ArtPageKey, ArtPageSources};
use crate::render::assets::ec_item_surface::KR_ITEM_RECORD_BASE;
use crate::content::{
    MapPlanesRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes, TexAuxEcPackageRes,
};
use crate::prelude::*;
use super::client_grunge::STATIC_FLAG_CLIENT_GRUNGE;
use super::static_batches::STATIC_FLAG_CC_WATER_FILL;
use bevy::prelude::*;
use bevy::platform::collections::HashSet;
use std::collections::HashMap;
use super::static_placement::{
    ART_QUAD_CORNERS, CC_FLAT_QUAD_FORWARD_XZ, FLAT_QUAD_CC_FLAT, FLAT_QUAD_EC_CAMERA, FLAT_QUAD_EC_SURFACE,
    GROUND_ART_Y_BIAS, GroundQuadBounds, STATIC_BILLBOARD_RIGHT_XZ, StaticBillboardBounds, StaticDepthClass,
    apply_static_world_anchor_translation, assign_sprite_tie_levels, cc_ground_diamond, cc_static_uses_flat_geometry,
    cc_surface_like_flat_quad_center_offsets, cell_depth_anchor, depth_class_y_bias, ec_static_camera_quad,
    resolve_cc_surface_like_flat_quad_bounds, resolve_priority_z_units, resolve_static_billboard_bounds,
    resolve_static_depth_class, resolve_surface_like_ground_quad_bounds, sprite_depth_anchor, static_depth_key,
    surface_like_static_world_anchor,
};
use super::static_source::{
    KrStaticPackages, KrStaticSources, StaticArtSourceState, StaticLandStyle, StaticSurfaceLikeResolutionCache,
    StaticVisualKind, drawn_tile_data, kingdom_reborn_static_visual_kind, land_grade_flag, resolve_effective_art_source,
    resolve_static_visual_kind_cached, static_tile_renders_wet, static_visual_renders_wet,
    surface_like_ground_material_payload, valid_texture_repetition,
};
use super::static_batches::{
    CLIENT_SHADOW_FLAG_CASTS, CLIENT_SHADOW_FLAG_WALL, GroundTileInstance, RenderStaticChunkBatches,
    RenderStaticInstances, RenderStaticLandInstances, STATIC_FLAG_CAP_MEETS_FLOOR, STATIC_FLAG_FLAT_ART,
    STATIC_HUE_FLAG_EC_SPRITE, SpriteInstance, StaticChunkBatch, StaticChunkBatchKey, StaticPickTile, StaticRenderTile,
    cave_instance_flags, static_hue_payload, static_sprite_opacity,
};
use super::static_chunk_cache::{
    CachedStaticChunk, CachedStaticChunkStats, StaticAssembly, StaticCut, StaticAtlasRetrySignature, StaticChunkCacheConfig,
    StaticChunkRenderCache, aggregate_cached_visible_static_stats, should_skip_unchanged_incomplete_static_retry,
    static_atlas_retry_signature, static_light_signature,
};

/// Tileart flags that cast no shadow: the client tests 0x10004200 (Surface, Roof and its bit 14, which is EC's
/// Rotated; our tilemeta keeps EC's 64-bit layout, where bit 14 is the classic ArticleA) and EC adds NoShadow.
const CLIENT_SHADOW_EXCLUDED_FLAGS: u64 = uocf::enhanced::tileart::TaeFlag::Surface.bits()
    | uocf::enhanced::tileart::TaeFlag::Roof.bits()
    | uocf::enhanced::tileart::TaeFlag::Rotated.bits()
    | uocf::enhanced::tileart::TaeFlag::NoShadow.bits();
/// The client's ground under a caster (0x00587770) takes a Surface or Bridge top at most this far above the caster.
const CLIENT_SHADOW_GROUND_REACH_Z: i32 = 4;
/// The largest 8-bit channel value, which a packed local light channel spans (`unpack4x8unorm` in the shaders).
const LOCAL_LIGHT_CHANNEL_MAX: f32 = u8::MAX as f32;
/// Bits per packed local light channel.
const LOCAL_LIGHT_CHANNEL_BITS: u32 = u8::BITS;
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
pub(crate) fn static_art_billboard_local_light_corners(
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
    /// The visible chunks still building (`StaticCollectCacheDecision::pending_chunk_builds`).
    pub pending_chunk_builds: usize,
}

#[derive(Resource, Default)]
pub struct StaticArtCollectDebugState {
    pub last: Option<StaticArtCollectStats>,
    pub path_counters: StaticArtCollectPathCounters,
    /// The cached chunks built again so far because the land they read changed (`invalidate_changed_land`).
    pub land_invalidated_chunks: u64,
}

#[derive(Clone, Copy)]
pub(super) struct StaticCollectFrameConfig {
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
    terrain_sources_revision: u64,
}

#[derive(Clone, Copy)]
struct StaticCollectResources<'a> {
    tex_art_cc: Option<&'a TexArtCcPackageRes>,
    tex_art_ec: Option<&'a TexArtEcPackageRes>,
    tex_land_ec: Option<&'a TexLandEcPackageRes>,
    tilemeta: Option<&'a TileMetaPackageRes>,
    tex_aux_ec: Option<&'a TexAuxEcPackageRes>,
    static_lights: &'a super::static_lights::RenderStaticLightInstances,
    /// The drawn land (`LandSnapshot`), which the tile atlas holds.
    land: &'a dyn LandCells,
    /// The EC terrain parameters of the land drawn, per atlas payload (`EcTerrainBases::params`).
    terrain: Option<&'a [Option<crate::render::terrain::coverage::EcTerrainParams>]>,
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
    /// What the z-view's cut reads of each of the `pick_tiles`, which assembling the chunk tests.
    cuts: Vec<StaticCut>,
    sprite_pick_indices: Vec<(f64, usize)>,
    ground_pick_indices: Vec<(f64, usize)>,
    /// The art pages of the sprites and of the ground instances: an instance's `layer` holds its page's slot here
    /// (`art_page_slot`) until the chunk is assembled into the outputs, which binds the pages to atlas layers.
    sprite_pages: Vec<ArtPageKey>,
    ground_pages: Vec<ArtPageKey>,
    stats: CachedStaticChunkStats,
    render_tiles: Vec<StaticRenderTile>,
    /// The land the roofs under KR's cut are counted from (`ZViewCut::roof_layer`), and the client shadow's ground: the
    /// map package's, which every cell has whether or not its land is loaded.
    own_land: PackageLand,
    /// The cells of the tile atlas, the land as drawn, the chunk read, so a change of them rebuilds the chunk.
    land_cells: Option<AtlasCells>,
}

/// The `layer` of an instance on no art page (dot mode), which assembling keeps as layer 0.
const NO_ART_PAGE_SLOT: u32 = 0;

/// The slot of `page` in a chunk's `pages`, from 1 (`NO_ART_PAGE_SLOT` is no page), which it joins when new.
fn art_page_slot(pages: &mut Vec<ArtPageKey>, page: ArtPageKey) -> u32 {
    let index = pages.iter().position(|&known| known == page).unwrap_or_else(|| {
        pages.push(page);
        pages.len() - 1
    });
    index as u32 + 1
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
    unique_requested_pages: HashSet<ArtPageKey>,
    copied_sprite_instances: usize,
    copied_ground_instances: usize,
    /// The atlases' mapping revisions before the assembly (`StaticAssembly::signature`).
    mapping_revisions_before: (u64, u64),
}

impl StaticCollectSlowStats {
    fn absorb_cached_chunk(&mut self, chunk: &CachedStaticChunk) {
        self.visited_blocks += chunk.stats.visited_blocks;
        self.source_tiles += chunk.stats.source_tiles;
        self.ground_land_tiles += chunk.stats.ground_land_tiles;
        self.unique_requested_pages.extend(chunk.sprite_pages.iter().chain(&chunk.ground_pages).copied());
    }
}

#[derive(Clone, Copy)]
struct StaticCollectCacheDecision {
    visible_set_unchanged: bool,
    all_visible_chunks_complete: bool,
    skip_unchanged_incomplete_retry: bool,
    /// The visible chunks not built yet, or built for an earlier generation or land (`StaticChunkRenderCache::is_current`).
    pending_chunk_builds: usize,
}

/// The resources of a build, read from `shared` and the drawn land `land`.
fn shared_resources<'a>(shared: &'a StaticBuildShared, land: &'a dyn LandCells) -> StaticCollectResources<'a> {
    StaticCollectResources {
        tex_art_cc: shared.tex_art_cc.as_ref(),
        tex_art_ec: shared.tex_art_ec.as_ref(),
        tex_land_ec: shared.tex_land_ec.as_ref(),
        tilemeta: shared.tilemeta.as_ref(),
        tex_aux_ec: shared.tex_aux_ec.as_ref(),
        static_lights: &shared.static_lights,
        land,
        terrain: shared.terrain.as_deref(),
        caves: shared.caves.as_deref(),
        player_cave: shared.player_cave.as_deref(),
        kingdom_reborn: shared.kingdom_reborn.as_ref().map(|kr| kr.sources()),
    }
}

/// `static_art_local_light_signature` is the lights' `static_light_signature` with the local lights on.
fn static_chunk_cache_config(
    frame: StaticCollectFrameConfig,
    static_art_local_light_signature: Option<u64>,
    art_pages: (ArtPageGeometry, ArtPageGeometry),
) -> StaticChunkCacheConfig {
    StaticChunkCacheConfig {
        map_id: frame.map_id,
        dot_mode: frame.is_dot_mode,
        art_source: frame.art_source,
        land_style: frame.land_style,
        art_pages,
        static_art_local_light_signature,
        show_hidden_multi_parts: frame.show_hidden_multi_parts,
        item_routes_revision: frame.item_routes_revision,
        caves_revision: frame.caves_revision,
        terrain_sources_revision: frame.terrain_sources_revision,
    }
}

/// The chunks whose land is drawn: `sys_draw_spawned_land_chunks` gives a chunk its mesh in the frame it writes the
/// chunk's land to the tile atlas, which the statics read (`StaticChunkBuildState::land_cells`). A chunk collected
/// before that would be built twice, the second time when its land arrives (`invalidate_changed_land`).
fn collect_visible_static_chunk_keys(
    map_id: u32,
    chunks_q: &Query<&crate::render::terrain::LCMesh, With<Mesh3d>>,
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
    z_view: ZViewCut,
    atlas_retry_signature: StaticAtlasRetrySignature,
) -> StaticCollectCacheDecision {
    // A change of the z-view's cut assembles the chunks again, without the statics it hides, and so do chunks built.
    let visible_set_unchanged = !config_changed && !cache.chunks_arrived
        && cache.last_visible_chunk_keys == visible_chunk_keys
        && cache.last_assembly.is_some_and(|assembly| assembly.z_view == z_view);
    let pending_chunk_builds = visible_chunk_keys.iter().filter(|chunk_key| !cache.is_current(chunk_key)).count();
    // Complete: every chunk is built, the last assembly drew every instance, and the atlases kept the pages it bound.
    let all_visible_chunks_complete = pending_chunk_builds == 0
        && cache.last_assembly.is_some_and(|assembly| assembly.atlas_misses == 0
            && assembly.signature.sprite_mapping_revision == atlas_retry_signature.sprite_mapping_revision
            && assembly.signature.ground_mapping_revision == atlas_retry_signature.ground_mapping_revision);
    let skip_unchanged_incomplete_retry = should_skip_unchanged_incomplete_static_retry(
        cache,
        visible_set_unchanged,
        all_visible_chunks_complete,
        atlas_retry_signature,
    );

    StaticCollectCacheDecision {
        visible_set_unchanged,
        all_visible_chunks_complete,
        skip_unchanged_incomplete_retry,
        pending_chunk_builds,
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

    let (aggregate, unique_requested_pages) = aggregate_cached_visible_static_stats(cache, visible_chunk_keys, visible_tick);
    let (atlas_hits, atlas_misses) = cache.last_assembly
        .map_or((0, 0), |assembly| (assembly.atlas_hits, assembly.atlas_misses));
    let path = if decision.skip_unchanged_incomplete_retry {
        StaticArtCollectPath::IncompleteWaitFastPath
    } else {
        StaticArtCollectPath::CompleteFastPath
    };
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
            unique_requested_pages,
            resident_pages: sprite_atlas.resident_page_count() + ground_atlas.resident_page_count(),
            pending_pages: sprite_atlas.pending_page_count() + ground_atlas.pending_page_count(),
            atlas_capacity_pages: sprite_atlas.active_layers() as usize
                + ground_atlas.active_layers() as usize,
            atlas_hits,
            atlas_misses,
            emitted_instances: sprite_output.0.len() + ground_output.0.len(),
            copied_sprite_instances: 0,
            copied_ground_instances: 0,
            pending_chunk_builds: decision.pending_chunk_builds,
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
        true
    } else {
        false
    }
}

/// Assembles the visible chunks built so far into the outputs (`append_cached_static_chunk_outputs`).
fn collect_visible_static_chunks_slow(
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_tick: u64,
    frame: StaticCollectFrameConfig,
    sources: ArtPageSources<'_>,
    sprite_atlas: &mut SpriteArtPageAtlas,
    ground_atlas: &mut GroundArtPageAtlas,
    sprite_output: &mut RenderStaticInstances,
    ground_output: &mut RenderStaticLandInstances,
    batches: &mut RenderStaticChunkBatches,
    cache: &mut StaticChunkRenderCache,
) -> StaticCollectSlowStats {
    let mut slow_stats = StaticCollectSlowStats {
        mapping_revisions_before: (sprite_atlas.0.mapping_revision(), ground_atlas.0.mapping_revision()),
        ..default()
    };
    cache.assembled_picks.iter_mut().for_each(HashMap::clear);

    for chunk_key in visible_chunk_keys.iter().copied() {
        if !reuse_cached_static_chunk(cache, chunk_key, visible_tick) {
            continue;
        }
        let Some(chunk) = cache.chunks.get(&chunk_key) else {
            continue;
        };

        slow_stats.absorb_cached_chunk(chunk);
        append_cached_static_chunk_outputs(
            chunk_key,
            chunk,
            &frame.z_view,
            (&mut *sprite_atlas, &mut *ground_atlas),
            sources,
            (sprite_output, ground_output),
            batches,
            &mut cache.assembled_picks,
            &mut slow_stats,
        );
    }

    // Assign after assembling every visible chunk: restarting at each chunk
    // leaves opaque coplanar sprites dependent on GPU batch submission order.
    assign_sprite_tie_levels(&mut sprite_output.0);
    slow_stats
}

/// Builds the chunk `chunk_key` off the main thread (`StaticChunkJob`): from its statics in `statics_store`, the drawn
/// land `land`, its `multis` and its map's land `own_land`.
pub(super) fn build_static_chunk(
    chunk_key: StaticChunkBatchKey,
    frame: StaticCollectFrameConfig,
    shared: &StaticBuildShared,
    land: &LandSnapshot,
    multis: &ChunkMultis,
    own_land: PackageLand,
    statics_store: &mut LazyStaticsStore,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
) -> CachedStaticChunk {
    let resources = shared_resources(shared, land);
    let (sprite_atlas, ground_atlas) = (&shared.art_pages.0, &shared.art_pages.1);
    let mut build = StaticChunkBuildState { own_land, ..default() };
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

            // Every static draws as itself, whatever its graphic: map statics are never multis (`world::multis`).
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
            let (block_x, block_y) = (block_cells(gx), block_cells(gy));
            for (placed_multi, parts) in multis.iter()
                .filter(|(multi, _)| block_x.contains(&multi.x) && block_y.contains(&multi.y)) {
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
                statics_store, tilemeta, art, resources.kingdom_reborn.map(|kr| (kr.tilemeta, kr.art.as_ref())),
                resources.land, resources.terrain,
            ) {
                build.read_land(cells);
            }
        }
    }
    // The collection marks it visible as it takes it (`StaticChunkRenderCache::insert_built`).
    finish_rebuilt_static_chunk(build, 0)
}

/// The build of the chunk `key` for the cache's `generation`, with copies of the cells of `tile_atlas` it reads, its
/// multis (`multis`) and the land of its map (`map_planes`).
fn static_chunk_job(
    key: StaticChunkBatchKey,
    generation: u64,
    frame: StaticCollectFrameConfig,
    shared: &std::sync::Arc<StaticBuildShared>,
    tile_atlas: &TileAtlas,
    (definitions, placed): (Option<&MultiDefinitionsRes>, &PlacedMultisRes),
    map_planes: Option<&MapPlanesRes>,
) -> StaticChunkJob {
    const CHUNK_CELLS_DIM: u32 = CHUNK_STORAGE_BLOCKS_DIM * MAP_STORAGE_BLOCK_TILE_DIM;
    let (xs, ys) = (key.gx * CHUNK_CELLS_DIM..(key.gx + key.scale) * CHUNK_CELLS_DIM,
        key.gy * CHUNK_CELLS_DIM..(key.gy + key.scale) * CHUNK_CELLS_DIM);
    let multis: ChunkMultis = placed.anchored_in(key.map_id, xs.clone(), ys.clone())
        .filter_map(|multi| Some((*multi, definitions?.parts(multi.multi_id, frame.show_hidden_multi_parts)?.to_vec())))
        .collect();
    let mut cells = AtlasCells::new(UVec2::new(xs.start, ys.start), UVec2::new(xs.end - 1, ys.end - 1));
    for (multi, parts) in &multis {
        for part in parts {
            let cell = (i64::from(multi.x) + i64::from(part.x), i64::from(multi.y) + i64::from(part.y));
            if let (Ok(x), Ok(y)) = (u32::try_from(cell.0), u32::try_from(cell.1)) {
                cells = cells.union(AtlasCells::new(UVec2::new(x, y), UVec2::new(x, y)));
            }
        }
    }
    // And the cells around them: a ground patch reads its neighbours' land (`apply_boundary_coverage`), a Classic dry
    // diamond the cells east and south (`cc_static_uses_flat_geometry`).
    let cells = AtlasCells::new(cells.min.saturating_sub(UVec2::ONE), cells.max + UVec2::ONE);
    StaticChunkJob {
        key,
        generation,
        land_revision: tile_atlas.content_revision,
        frame,
        shared: shared.clone(),
        land: LandSnapshot::new(tile_atlas, cells),
        multis,
        own_land: PackageLand::opened(map_planes, key.map_id),
        store: None,
    }
}

/// Emits `render_tile` over the ground it stands on (`client_shadow_ground_z`): the highest of the `surface_tops` on its
/// cell within its reach, or the land. The `surface_tops` of the cells west and north of it tell whether a floor lies
/// on its top in the farther row (`wall_cap_meets_floor_flag`).
fn emit_static_render_tile_on_surfaces(
    render_tile: StaticRenderTile,
    surface_tops: &[&[((i32, i32), i32)]],
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &ArtPageGeometry,
    ground_atlas: &ArtPageGeometry,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
    build: &mut StaticChunkBuildState,
) {
    let cell = (render_tile.world_x as i32, render_tile.world_z as i32);
    let all_surface_tops = surface_tops;
    let surface_tops = surface_tops.iter().flat_map(|tops| tops.iter()).filter(|top| top.0 == cell).map(|top| top.1);
    let land_z = build.own_land.land_z(None, frame.map_id, cell.0, cell.1)
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
/// own corner's `land_z`, where the renderer draws that land. Land never drawn (`LandCells::land_is_nodraw`, the
/// Classic client's NoDraw land) covers nothing, nor does land the z-view's cut leaves out (`StaticCut::hides`).
fn covering_land_z(land_z: Option<i32>, land: &dyn LandCells, x: i32, y: i32) -> Option<i32> {
    let nodraw = || matches!((u32::try_from(x), u32::try_from(y)), (Ok(x), Ok(y)) if land.land_is_nodraw(x, y));
    land_z.filter(|_| !nodraw())
}

fn emit_static_render_tile(
    render_tile: StaticRenderTile,
    ground_z: i32,
    farther_floor_at: impl Fn(i32) -> bool,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &ArtPageGeometry,
    ground_atlas: &ArtPageGeometry,
    surface_resolution_cache: &mut StaticSurfaceLikeResolutionCache,
    build: &mut StaticChunkBuildState,
) {
    let enhanced_package = resources.tilemeta_package();
    let graphic = u32::from(render_tile.graphic);
    let enhanced_tilemeta = enhanced_package.and_then(|meta| meta.item_tile(graphic));
    // Resolve the same art and material as at street level for liquid statics. Avoid
    // resolving the full art inventory for ordinary radar statics.
    let visual_kind = frame.art_source.filter(|_| !frame.is_dot_mode
        || enhanced_tilemeta.is_some_and(|meta| meta.is_wet())
        || enhanced_package.is_some_and(|package| package.uses_water_shader(graphic))
        || resources.kingdom_reborn.is_some_and(|kr| kr.tilemeta.uses_water_shader(graphic))).map(|source| {
        let kind = resolve_static_visual_kind_cached(source, frame.land_style, render_tile.graphic, enhanced_package,
            enhanced_tilemeta, resources.tex_art_cc_package(), resources.tex_art_ec_package(),
            resources.tex_land_ec_package(), surface_resolution_cache);
        resources.kingdom_reborn.and_then(|kr| kingdom_reborn_static_visual_kind(render_tile.graphic, enhanced_package,
            enhanced_tilemeta, resources.tex_art_ec_package(), resources.tex_land_ec_package(), kr,
            surface_resolution_cache))
            .unwrap_or(kind)
    }).filter(|kind| !frame.is_dot_mode || match kind {
        StaticVisualKind::TexLandEc { resolution } => resolution.liquid_payload.is_some(),
        StaticVisualKind::CcRegular { .. } | StaticVisualKind::EcSurface { .. } => {
            let (package, meta) = drawn_tile_data(Some(*kind), render_tile.graphic, enhanced_package,
                resources.tilemeta_package_for_art(Some(ClientTextureSource::Cc)), resources.kingdom_reborn);
            static_visual_renders_wet(*kind, meta, package, graphic)
        }
        _ => false,
    });
    // From here on the static is drawn by the tile data of the art it is drawn from (`drawn_tile_data`).
    let (tilemeta_package, tilemeta) =
        drawn_tile_data(visual_kind, render_tile.graphic, enhanced_package,
            resources.tilemeta_package_for_art(Some(ClientTextureSource::Cc)), resources.kingdom_reborn);
    if visual_kind.is_some() && tilemeta.is_some_and(|meta| meta.is_nodraw()) {
        return;
    }
    let (cell_x, cell_y) = (render_tile.world_x as i32, render_tile.world_z as i32);
    let flags = tilemeta.map_or(0, |meta| meta.flags);
    // In the player's cave, and outside it (caves.rs): the cave's statics are drawn whole, whatever the z-view limit.
    let player_cave = resources.player_cave.map(|cave| {
        let land_z = || build.own_land.land_z(None, frame.map_id, cell_x, cell_y);
        cave.static_view(cell_x, cell_y, render_tile.z, flags, tilemeta.map_or(0, |meta| meta.height), land_z)
    });
    // Above the player's floor, with the z-view limit on, the static is left out when the chunk is assembled, and
    // picking skips it too (`StaticCut`). A roof under KR's cut reads its own cell's land.
    let own_land_z = (flags & uocf::enhanced::tileart::TaeFlag::Roof.bits() != 0)
        .then(|| build.own_land.averaged_land_z(None, frame.map_id, cell_x, cell_y))
        .flatten();
    // Over a cave drawn over the map, and in it (caves.rs).
    let cave = player_cave.unwrap_or_else(|| resources.caves.map_or(CaveStatic::Outside, |caves|
        caves.static_view(cell_x, cell_y, render_tile.z)));
    if cave == CaveStatic::Hidden {
        return;
    }
    // Outside the caves, a cave's floor under its cell's land, where that land is drawn, is covered, as the Classic
    // client draws a cell's land after the floors under it: its art, standing upright at one depth, would show through
    // the lower part of a steep land, as teeth along a cave mouth's black opening.
    let mut covering_land_z = || {
        // The NoDraw test reads the drawn land: its arriving or changing rebuilds the chunk.
        if let (Ok(x), Ok(y)) = (u32::try_from(cell_x), u32::try_from(cell_y)) {
            build.read_land(AtlasCells::new(UVec2::new(x, y), UVec2::new(x, y)));
        }
        covering_land_z(build.own_land.land_z(None, frame.map_id, cell_x, cell_y),
            resources.land, cell_x, cell_y)
    };
    // Read only for a floor, as a read of the drawn land rebuilds the chunk when that land changes.
    let mut covering = None;
    let buried = player_cave.is_none() && cave == CaveStatic::Outside
        && is_buried_floor(flags, tilemeta.map_or(0, |meta| meta.height), render_tile.z, || {
            covering = covering_land_z();
            covering
        });
    let buried_by_land_z = covering.filter(|_| buried);
    let cut = StaticCut {
        flags,
        z: render_tile.z,
        own_land_z,
        uncut: player_cave == Some(CaveStatic::Lifted),
        x: cell_x,
        y: cell_y,
        buried_by_land_z,
    };
    let cave_flags = cave_instance_flags(cave, player_cave.is_some());
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

    let before = (build.sprite_instances.len(), build.ground_instances.len());
    if let Some(visual_kind) = visual_kind {
        emit_art_static_tile(
            info,
            frame,
            resources,
            visual_kind,
            sprite_atlas,
            ground_atlas,
            build,
        );
    }
    if frame.is_dot_mode && before == (build.sprite_instances.len(), build.ground_instances.len()) {
        emit_dot_static_tile(info, frame, resources, build);
    }
    // Every instance has its pick tile and its cut, the dots' too.
    if before != (build.sprite_instances.len(), build.ground_instances.len()) {
        let index = build.pick_tiles.len();
        build.pick_tiles.push(StaticPickTile { tile: render_tile, is_wet: info.is_wet_flags & 1 != 0 });
        build.cuts.push(cut);
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
pub(crate) fn static_land_z(land: &std::cell::RefCell<PackageLand>, map_planes: Option<&MapPlanesRes>, map_id: u32,
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
    // Use the same cell depth as the terrain, so radar dots sort by height within the cell.
    // Liquid statics cover the cell itself, like the terrain underneath them.
    let (tile_x, tile_y) = cell_depth_anchor(info.world_x, info.world_z);
    let flat_quad = if info.is_wet_flags & 1 != 0 { FLAT_QUAD_EC_SURFACE } else { 0 };

    build.sprite_instances.push(SpriteInstance {
        world_x: info.world_x,
        world_z: info.world_z,
        world_y,
        layer: NO_ART_PAGE_SLOT,
        depth_class: encoded_depth_class,
        base_world_y: info.base_world_y,
        uv_min: [0.0, 0.0],
        uv_max: [0.0, 0.0],
        local_min: [0.0, 0.0],
        local_max: [1.0, 1.0],
        tile_x,
        tile_y,
        priority_z_units,
        sort_bias_ordinal: 0,
        is_wet_flags: info.is_wet_flags,
        hue_id: 0,
        hue_flags: 0,
        flat_quad,
        texture_stretch: 0.0,
        material_item_id: 0,
        client_shadow_flags: 0,
        client_shadow_height: 0.0,
        client_grunge_anchor: [0.0; 2],
        local_light_corners,
        color_rgba: [
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
            1.0,
        ],
    });
}

fn emit_art_static_tile(
    info: StaticTileRenderInfo<'_>,
    frame: StaticCollectFrameConfig,
    resources: StaticCollectResources<'_>,
    visual_kind: StaticVisualKind,
    sprite_atlas: &ArtPageGeometry,
    ground_atlas: &ArtPageGeometry,
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
                info.world_x, info.world_z, info.tile.z, resources.land)
        });

    let (anchored_world_x, anchored_world_z) = if cc_surface_like_flat {
        // Flat ground quad: no upright-billboard centering translation (mirrors TexLandEc).
        (info.world_x, info.world_z)
    } else {
        surface_like_static_world_anchor(visual_kind, info.world_x, info.world_z)
    };

    if let Some(resolved) = resolved_sprite {
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
    }
}

fn resolve_art_static_sprite(
    visual_kind: StaticVisualKind,
    resources: StaticCollectResources<'_>,
    sprite_atlas: &ArtPageGeometry,
    ground_atlas: &ArtPageGeometry,
) -> Option<(ClientTextureSource, Option<ResolvedArtSprite>, f32)> {
    match visual_kind {
        StaticVisualKind::CcRegular {
            art_id,
            fallback_art_id,
        } => {
            let tex_art_cc = &resources.tex_art_cc?.0;
            let resolved_sprite = if tex_art_cc.present_slot(art_id as u32).is_some() {
                sprite_atlas.sprite_cc(tex_art_cc, art_id)
            } else if fallback_art_id != art_id {
                sprite_atlas.sprite_cc(tex_art_cc, fallback_art_id)
            } else {
                None
            };

            Some((ClientTextureSource::Cc, resolved_sprite, 0.0))
        }
        StaticVisualKind::TexLandEc { resolution } => {
            let (tex_land_ec, page_key): (_, fn(u32) -> ArtPageKey) = if resolution.kingdom_reborn {
                (resources.kingdom_reborn?.land, ArtPageKey::KrTerrain)
            } else {
                (&resources.tex_land_ec?.0, ArtPageKey::Terrain)
            };

            Some((
                ClientTextureSource::Ec,
                ground_atlas.sprite_tex_land_ec_pages(tex_land_ec, resolution.runtime_slot_id, page_key),
                resolution.texture_repetition,
            ))
        }
        StaticVisualKind::EcRegular { art_id } | StaticVisualKind::EcSurface { art_id } => {
            let (tex_art_ec, tilemeta_package, art_id, page_key): (_, _, _, fn(u32) -> ArtPageKey) =
                if art_id >= KR_ITEM_RECORD_BASE {
                    // The Kingdom Reborn records, drawn from the Kingdom Reborn art or the Enhanced art it leaves them to.
                    let kr = resources.kingdom_reborn?;
                    let art_id = art_id - KR_ITEM_RECORD_BASE;
                    match kr.art_slot(resources.tex_art_ec_package(), art_id) {
                        Some((art, _)) if !std::ptr::eq(art, kr.art.as_ref()) =>
                            (&resources.tex_art_ec?.0, Some(kr.tilemeta), art_id, ArtPageKey::Enhanced),
                        _ => (kr.art, Some(kr.tilemeta), art_id, ArtPageKey::KrEnhanced),
                    }
                } else {
                    (&resources.tex_art_ec?.0, resources.tilemeta_package(), art_id, ArtPageKey::Enhanced)
                };

            let texture_stretch = if matches!(visual_kind, StaticVisualKind::EcSurface { .. }) {
                tilemeta_package
                    .and_then(|package| package.item_texture_refs(art_id).iter()
                        .find(|texture_ref| texture_ref.is_world_art() && texture_ref.is_primary_selected()))
                    .map(|texture_ref| valid_texture_repetition(texture_ref.texture_stretch))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            Some((ClientTextureSource::Ec, sprite_atlas.sprite_ec_pages(tex_art_ec, art_id, page_key), texture_stretch))
        }
        StaticVisualKind::Missing => {
            Some((ClientTextureSource::Ec, sprite_atlas.sprite_missing(&resources.tex_aux_ec?.0), 0.0))
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
    let layer = art_page_slot(&mut build.ground_pages, resolved.page);
    build.ground_instances.push(GroundTileInstance {
        world_x: anchored_world_x,
        world_z: anchored_world_z,
        world_y,
        layer,
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

    let layer = art_page_slot(&mut build.sprite_pages, resolved.page);
    build.sprite_instances.push(SpriteInstance {
        world_x: anchored_world_x,
        world_z: anchored_world_z,
        world_y,
        layer,
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
    // The six opaque water diamonds share the Classic sea's art family.
    // Adjacent foam graphics have their own authored shape and are filtered.
    const CLASSIC_WATER_FILLS: std::ops::RangeInclusive<u16> = 0x1797..=0x179C;
    let water_fill = if frame.is_dot_mode && CLASSIC_WATER_FILLS.contains(&info.tile.graphic) {
        STATIC_FLAG_CC_WATER_FILL
    } else {
        0
    };
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

    let layer = art_page_slot(&mut build.sprite_pages, resolved.page);
    build.sprite_instances.push(SpriteInstance {
        world_x: centered_world_x,
        world_z: centered_world_z,
        world_y,
        layer,
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
        is_wet_flags: info.is_wet_flags | info.client_grunge_flag | water_fill,
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

    CachedStaticChunk {
        sprite_instances: build.sprite_instances,
        ground_instances: build.ground_instances,
        pick_tiles: build.pick_tiles,
        cuts: build.cuts,
        sprite_pick_indices: sorted_indices(build.sprite_pick_indices),
        ground_pick_indices: sorted_indices(build.ground_pick_indices),
        sprite_pages: build.sprite_pages,
        ground_pages: build.ground_pages,
        stats: build.stats,
        last_visible_tick: visible_tick,
        land_cells: build.land_cells,
        generation: 0,
        land_stale: false,
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

/// The packages the statics' art pages are read from.
fn art_page_sources(resources: StaticCollectResources<'_>) -> ArtPageSources<'_> {
    ArtPageSources {
        classic: resources.tex_art_cc.map(|res| &res.0),
        enhanced: resources.tex_art_ec.map(|res| &res.0),
        kr_enhanced: resources.kingdom_reborn.map(|kr| kr.art),
        terrain: resources.tex_land_ec.map(|res| &res.0),
        kr_terrain: resources.kingdom_reborn.map(|kr| kr.land),
        missing: resources.tex_aux_ec.map(|res| &res.0),
    }
}

/// The atlas layer of each page slot of a chunk (`art_page_slot`), `None` for a page not yet resident.
fn bind_chunk_pages(pages: &[ArtPageKey], atlas: &mut ArtPageAtlasSet, sources: ArtPageSources<'_>) -> Vec<Option<u32>> {
    std::iter::once(Some(0)).chain(pages.iter().map(|&page| atlas.bind_page(page, sources))).collect()
}

/// Appends the `instances` `drawn` (by index) to `output`, their page slots replaced by the layers `layers` binds them
/// to. Returns the indices of the instances appended when some were left out, by `drawn` or for want of their pages
/// (`None` when none were), and how many lacked their pages.
fn append_drawn_instances<T: Copy>(
    instances: &[T],
    layers: &[Option<u32>],
    layer: fn(&mut T) -> &mut u32,
    drawn: impl Fn(usize) -> bool,
    output: &mut Vec<T>,
) -> (Option<Vec<u32>>, usize) {
    let mut kept: Option<Vec<u32>> = None;
    let mut unbound = 0;
    for (index, instance) in instances.iter().enumerate() {
        let mut instance = *instance;
        let slot = layer(&mut instance);
        let bound = layers[*slot as usize];
        match bound.filter(|_| drawn(index)) {
            Some(bound) => {
                *slot = bound;
                output.push(instance);
                if let Some(kept) = &mut kept {
                    kept.push(index as u32);
                }
            }
            None => {
                unbound += usize::from(bound.is_none());
                kept.get_or_insert_with(|| (0..index as u32).collect());
            }
        }
    }
    (kept, unbound)
}

/// Appends the chunk's instances to the outputs, on the atlas layers of their pages, but those `z_view` cuts: an
/// instance whose page is not resident yet is left out, its page requested, until an assembly after the page arrives.
fn append_cached_static_chunk_outputs(
    chunk_key: StaticChunkBatchKey,
    chunk: &CachedStaticChunk,
    z_view: &ZViewCut,
    (sprite_atlas, ground_atlas): (&mut SpriteArtPageAtlas, &mut GroundArtPageAtlas),
    sources: ArtPageSources<'_>,
    (sprite_output, ground_output): (&mut RenderStaticInstances, &mut RenderStaticLandInstances),
    batches: &mut RenderStaticChunkBatches,
    assembled_picks: &mut [HashMap<StaticChunkBatchKey, Vec<u32>>; 2],
    slow_stats: &mut StaticCollectSlowStats,
) {
    if !chunk.sprite_instances.is_empty() {
        let start = sprite_output.0.len() as u32;
        let layers = bind_chunk_pages(&chunk.sprite_pages, sprite_atlas, sources);
        let drawn = |index: usize| !chunk.cuts[chunk.sprite_pick_indices[index]].hides(z_view);
        let (kept, unbound) = append_drawn_instances(&chunk.sprite_instances, &layers, |instance| &mut instance.layer,
            drawn, &mut sprite_output.0);
        let count = sprite_output.0.len() as u32 - start;
        slow_stats.copied_sprite_instances += count as usize;
        slow_stats.atlas_hits += count as usize;
        slow_stats.atlas_misses += unbound;
        if let Some(kept) = kept {
            assembled_picks[0].insert(chunk_key, kept);
        }
        if count > 0 {
            batches.sprite.push(StaticChunkBatch { key: chunk_key, start, count });
        }
    }

    if !chunk.ground_instances.is_empty() {
        let start = ground_output.0.len() as u32;
        let layers = bind_chunk_pages(&chunk.ground_pages, ground_atlas, sources);
        let drawn = |index: usize| !chunk.cuts[chunk.ground_pick_indices[index]].hides(z_view);
        let (kept, unbound) = append_drawn_instances(&chunk.ground_instances, &layers, |instance| &mut instance.layer,
            drawn, &mut ground_output.0);
        let count = ground_output.0.len() as u32 - start;
        slow_stats.copied_ground_instances += count as usize;
        slow_stats.atlas_hits += count as usize;
        slow_stats.atlas_misses += unbound;
        if let Some(kept) = kept {
            assembled_picks[1].insert(chunk_key, kept);
        }
        if count > 0 {
            batches.ground.push(StaticChunkBatch { key: chunk_key, start, count });
        }
    }
}

fn finish_static_collect_slow_path(
    frame: StaticCollectFrameConfig,
    visible_chunk_keys: Vec<StaticChunkBatchKey>,
    visible_tick: u64,
    slow_stats: StaticCollectSlowStats,
    pending_chunk_builds: usize,
    sprite_atlas: &SpriteArtPageAtlas,
    ground_atlas: &GroundArtPageAtlas,
    sprite_output: &RenderStaticInstances,
    ground_output: &RenderStaticLandInstances,
    cache: &mut StaticChunkRenderCache,
    debug_state: &mut StaticArtCollectDebugState,
) {
    let (sprite_mapping_revision, ground_mapping_revision) = slow_stats.mapping_revisions_before;
    cache.last_assembly = Some(StaticAssembly {
        signature: StaticAtlasRetrySignature {
            sprite_mapping_revision,
            ground_mapping_revision,
            ..static_atlas_retry_signature(sprite_atlas, ground_atlas)
        },
        atlas_hits: slow_stats.atlas_hits,
        atlas_misses: slow_stats.atlas_misses,
        z_view: frame.z_view,
    });
    cache.chunks_arrived = false;
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
            atlas_capacity_pages: sprite_atlas.active_layers() as usize
                + ground_atlas.active_layers() as usize,
            atlas_hits: slow_stats.atlas_hits,
            atlas_misses: slow_stats.atlas_misses,
            emitted_instances: sprite_output.0.len() + ground_output.0.len(),
            copied_sprite_instances: slow_stats.copied_sprite_instances,
            copied_ground_instances: slow_stats.copied_ground_instances,
            pending_chunk_builds,
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
    settings: Res<crate::settings::Settings>,
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
        ResMut<StaticChunkBuilder>,
    ),
    mut debug_state: ResMut<StaticArtCollectDebugState>,
    source_state: Res<StaticArtSourceState>,
    static_lights: Res<super::static_lights::RenderStaticLightInstances>,
    // The land chunks drawn are the chunks whose statics are drawn (`collect_visible_static_chunk_keys`).
    terrain: (
        Query<&crate::render::terrain::LCMesh, With<Mesh3d>>,
        Res<TileAtlas>,
        Option<Res<crate::render::terrain::coverage::EcTerrainBases>>,
    ),
) {
    let (tex_art_cc_res, tex_art_ec_res, tex_land_ec_res, tilemeta_res, tex_aux_ec_res, kr_packages) = packages;
    let (chunks_q, tile_atlas, terrain_bases) = terrain;
    let (z_view, caves) = view_cuts;
    let (multis_res, placed_multis) = multis;
    if !settings.world_rendering.enable_statics {
        let (sprite_output, ground_output, batches, cache, _) = &mut outputs;
        // Cleared once: clearing them on every frame would flag them changed, and upload them, on every frame.
        if !(sprite_output.0.is_empty() && ground_output.0.is_empty() && batches.sprite.is_empty()
            && batches.ground.is_empty())
        {
            clear_static_collect_outputs(sprite_output, ground_output, batches);
        }
        // Re-enabling must rebuild the output lists, even if the visible chunks
        // and their cached instances are unchanged.
        cache.last_visible_chunk_keys.clear();
        cache.last_assembly = None;
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
        terrain_sources_revision: terrain_bases.as_ref().map_or(0, |bases| bases.sources_revision()),
    };
    let (sprite_output, ground_output, batches, cache, builder) = &mut outputs;
    let static_art_local_light_signature =
        frame.static_art_local_lights.then(|| static_light_signature(&static_lights, frame.map_id));
    let art_pages = (sprite_atlas.geometry(), ground_atlas.geometry());
    let (active_caves, player_cave) = caves.shared();
    let shared = std::sync::Arc::new(StaticBuildShared {
        tex_art_cc: tex_art_cc_res.as_deref().cloned(),
        tex_art_ec: tex_art_ec_res.as_deref().cloned(),
        tex_land_ec: tex_land_ec_res.as_deref().cloned(),
        tilemeta: tilemeta_res.as_deref().cloned(),
        tex_aux_ec: tex_aux_ec_res.as_deref().cloned(),
        kingdom_reborn: kr_packages.shared().filter(|_| frame.land_style == StaticLandStyle::KingdomReborn),
        static_lights: builder.lights(static_art_local_light_signature, &static_lights),
        terrain: builder.terrain(terrain_bases.as_deref()),
        caves: active_caves,
        player_cave,
        art_pages,
    });
    let cache_config = static_chunk_cache_config(frame, static_art_local_light_signature, art_pages);
    let visible_tick = cache.begin_frame();
    let config_changed = cache.sync_config(cache_config);
    debug_state.land_invalidated_chunks += cache.invalidate_changed_land(&tile_atlas) as u64;

    let visible_chunk_keys = collect_visible_static_chunk_keys(map_id, &chunks_q);
    let statics_store = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref());
    if let Some(statics_store) = statics_store {
        let generation = cache.generation();
        builder.send(cache, &visible_chunk_keys, statics_store, frame.item_routes_revision, |key| static_chunk_job(
            key, generation, frame, &shared, &tile_atlas, (multis_res.as_deref(), &placed_multis), map_planes.as_deref()));
    }
    builder.receive(cache, &tile_atlas, visible_tick);

    let atlas_retry_signature = static_atlas_retry_signature(&sprite_atlas, &ground_atlas);
    let cache_decision = evaluate_static_collect_cache_decision(
        cache,
        &visible_chunk_keys,
        config_changed,
        frame.z_view,
        atlas_retry_signature,
    );
    if try_finish_cached_visible_static_collect(
        frame,
        &visible_chunk_keys,
        visible_tick,
        cache_decision,
        &sprite_atlas,
        &ground_atlas,
        sprite_output,
        ground_output,
        cache,
        &mut debug_state,
    ) {
        return;
    }

    clear_static_collect_outputs(sprite_output, ground_output, batches);
    if statics_store.is_none() {
        return;
    }
    let slow_stats = collect_visible_static_chunks_slow(
        &visible_chunk_keys,
        visible_tick,
        frame,
        art_page_sources(shared_resources(&shared, &*tile_atlas)),
        &mut sprite_atlas,
        &mut ground_atlas,
        sprite_output,
        ground_output,
        batches,
        cache,
    );
    finish_static_collect_slow_path(
        frame,
        visible_chunk_keys,
        visible_tick,
        slow_stats,
        cache_decision.pending_chunk_builds,
        &sprite_atlas,
        &ground_atlas,
        sprite_output,
        ground_output,
        cache,
        &mut debug_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LandTextureSource;
    use bytemuck::Zeroable;
    use super::super::static_placement::*;
    use super::super::static_source::*;
    use super::super::static_batches::*;
    use super::super::static_chunk_cache::*;

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

    /// Pins an open item (STYLE_RULES §7), not a rule: the z-view and the caves read the overridden Enhanced tables
    /// whatever the art source, so with Classic art a swamp piece the clients' tables leave without Surface is still a
    /// floor to them. When the item is settled, this test changes with it.
    #[test]
    fn the_world_analyses_read_the_overridden_tables_whatever_the_art() {
        const SWAMP: u16 = 0x3236;
        const SWAMP_Z: i8 = 0;
        let mut items = (0..=u32::from(SWAMP))
            .map(|tile_id| udd_assets::tilemeta::TileMetaItemTile { tile_id, ..Zeroable::zeroed() })
            .collect::<Vec<_>>();
        items[usize::from(SWAMP)].flags = TILE_FLAG_SURFACE;
        let record = udd_assets::tilemeta::TileMetaOverridesRecord { digest: 1,
            original_flags: vec![(u32::from(SWAMP), 0)], original_lights: Vec::new() };
        let tables = TileMetaPackageRes::new(std::sync::Arc::new(tilemeta_package(&items, Some(&record)))).unwrap();
        let floor = |tables: &udd_assets::tilemeta::TileMetaPackage| {
            let item = &tables.item_tiles()[usize::from(SWAMP)];
            crate::world::caves::surface_top(item.flags, item.height, SWAMP_Z)
        };
        for art in [Some(ClientTextureSource::Ec), None] {
            assert!(std::sync::Arc::ptr_eq(tables.for_world_analyses(), tables.for_art(art)));
        }
        assert!(!std::sync::Arc::ptr_eq(tables.for_world_analyses(), tables.for_art(Some(ClientTextureSource::Cc))));
        assert_eq!(floor(tables.for_art(Some(ClientTextureSource::Cc))), None, "drawn with Classic art: no floor");
        assert_eq!(floor(tables.for_world_analyses()), Some(i32::from(SWAMP_Z)), "the z-view and caves: a floor");
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
            * crate::render::view::camera::ORTHO_WIDTH_SCALE_FACTOR;
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
        use crate::render::assets::tile_atlas::{AtlasParams, Rg16u};
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
        use crate::world::caves::CAVE_MIN_BURY_DEPTH;
        use crate::render::assets::tile_atlas::{AtlasParams, Rg16u, LAND_SOURCE_CC_ATLAS,
            LAND_SOURCE_NODRAW, LAND_NODRAW_ID};
        use crate::world::z_view::NO_Z_VIEW_LIMIT;
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
        // Hidden at assembly, the z-view's cut leaving the land over `max_ground_z` out (`StaticCut::hides`).
        let buried = |atlas: &TileAtlas, max_ground_z: i32, x: i32| {
            let covering = covering_land_z(Some(land_z), atlas, x, 0);
            let buried_by_land_z = covering.filter(|_| is_buried_floor(floor, 0, 0, || covering));
            StaticCut { buried_by_land_z, ..Default::default() }.hides(&ZViewCut { max_ground_z, ..ZViewCut::NONE })
        };
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
        let kr_art = std::sync::Arc::new(ec.kr_overlay(udd_assets::AtlasCacheOptions::disabled()).unwrap().expect("the Kingdom Reborn art"));
        let kr_land = std::sync::Arc::new(land.kr_land(udd_container::UddpReaderOptions::disabled(), udd_assets::AtlasCacheOptions::disabled())
            .unwrap().expect("the Kingdom Reborn land"));
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
        let mut world = World::new();
        let mut settings = crate::settings::load_from_files().expect("settings load");
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
        world.insert_resource(SpriteArtPageAtlas(ArtPageAtlasSet::blank_for_tests()));
        world.insert_resource(GroundArtPageAtlas(ArtPageAtlasSet::blank_for_tests()));
        world.insert_resource(RenderStaticInstances(vec![SpriteInstance::zeroed()]));
        world.insert_resource(RenderStaticLandInstances(vec![GroundTileInstance::zeroed()]));
        world.insert_resource(RenderStaticChunkBatches { sprite: vec![batch], ground: vec![batch] });
        world.insert_resource(cache);
        world.insert_resource(super::super::static_chunk_builder::StaticChunkBuilder::inline());
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
                flags: crate::world::multis::MULTI_PART_VISIBLE_FLAG,
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
                flags: crate::world::multis::MULTI_PART_VISIBLE_FLAG,
            },
            uocf::classic::multi::MultiPart {
                item_id: 11,
                x: 9,
                y: -4,
                z: -2,
                flags: crate::world::multis::MULTI_PART_VISIBLE_FLAG,
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

    /// A change of the settings starts a generation: the chunks built for the last one draw until built again.
    #[test]
    fn static_chunk_cache_builds_again_when_config_changes() {
        let key = StaticChunkBatchKey { map_id: 1, gx: 0, gy: 0, scale: 1 };
        let atlas = TileAtlas::new(Default::default(), 0);
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            caves_revision: 0,
            terrain_sources_revision: 0,
        });
        cache.insert_built(key, CachedStaticChunk::default(), atlas.content_revision, &atlas, 1);
        assert!(cache.is_current(&key));

        assert!(cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 1,
            caves_revision: 0,
            terrain_sources_revision: 0,
        }));

        assert!(cache.chunks.contains_key(&key) && !cache.is_current(&key));
        cache.insert_built(key, CachedStaticChunk::default(), atlas.content_revision, &atlas, 2);
        assert!(cache.is_current(&key));
        // Under another art, it goes at once.
        assert!(cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Ec),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 1,
            caves_revision: 0,
            terrain_sources_revision: 0,
        }));
        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn static_chunk_cache_ignores_static_light_changes_when_local_lights_disabled() {
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            caves_revision: 0,
            terrain_sources_revision: 0,
        });

        let key = StaticChunkBatchKey { map_id: 1, gx: 0, gy: 0, scale: 1 };
        let atlas = TileAtlas::new(Default::default(), 0);
        cache.insert_built(key, CachedStaticChunk::default(), atlas.content_revision, &atlas, 1);

        assert!(!cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: None,
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            caves_revision: 0,
            terrain_sources_revision: 0,
        }));

        assert!(cache.is_current(&key));
    }

    #[test]
    fn static_chunk_cache_tracks_static_light_changes_when_local_lights_enabled() {
        let mut cache = StaticChunkRenderCache::default();
        cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: Some(0),
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            caves_revision: 0,
            terrain_sources_revision: 0,
        });
        let key = StaticChunkBatchKey { map_id: 1, gx: 0, gy: 0, scale: 1 };
        let atlas = TileAtlas::new(Default::default(), 0);
        cache.insert_built(key, CachedStaticChunk::default(), atlas.content_revision, &atlas, 1);

        assert!(cache.sync_config(StaticChunkCacheConfig {
            art_pages: Default::default(),
            map_id: 1,
            dot_mode: false,
            art_source: Some(ClientTextureSource::Cc),
            land_style: StaticLandStyle::Unknown,
            static_art_local_light_signature: Some(1),
            show_hidden_multi_parts: false,
            item_routes_revision: 0,
            caves_revision: 0,
            terrain_sources_revision: 0,
        }));

        assert!(!cache.is_current(&key));
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
        cache.chunks.insert(chunk_key, CachedStaticChunk::default());
        cache.last_visible_chunk_keys = vec![chunk_key];
        cache.last_assembly = Some(StaticAssembly { signature: retry_signature, atlas_hits: 0, atlas_misses: 1,
            z_view: ZViewCut::NONE });

        assert!(should_skip_unchanged_incomplete_static_retry(
            &cache,
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
            true,
            false,
            changed_retry_signature,
        ));
    }

    /// An assembly that drew everything is complete until the atlas moves a page, or the z-view's cut changes: then the
    /// chunks assemble again, without being rebuilt.
    #[test]
    fn an_atlas_page_moving_assembles_the_chunks_again() {
        let chunk_key = StaticChunkBatchKey { map_id: 1, gx: 0, gy: 0, scale: 1 };
        let signature = StaticAtlasRetrySignature {
            sprite_mapping_revision: 3,
            ground_mapping_revision: 0,
            sprite_resident_pages: 1,
            ground_resident_pages: 0,
            sprite_pending_pages: 0,
            ground_pending_pages: 0,
            sprite_active_layers: 4,
            ground_active_layers: 4,
        };
        let mut cache = StaticChunkRenderCache::default();
        cache.chunks.insert(chunk_key, CachedStaticChunk::default());
        cache.last_visible_chunk_keys = vec![chunk_key];
        cache.last_assembly = Some(StaticAssembly { signature, atlas_hits: 1, atlas_misses: 0, z_view: ZViewCut::NONE });

        let decide = |signature, z_view| evaluate_static_collect_cache_decision(&cache, &[chunk_key], false, z_view,
            signature);
        let decision = decide(signature, ZViewCut::NONE);
        assert!(decision.visible_set_unchanged && decision.all_visible_chunks_complete);
        let moved = StaticAtlasRetrySignature { sprite_mapping_revision: 4, ..signature };
        let decision = decide(moved, ZViewCut::NONE);
        assert!(!decision.all_visible_chunks_complete && !decision.skip_unchanged_incomplete_retry);
        // So does a change of the z-view's cut.
        let decision = decide(signature, ZViewCut { max_z: 20, ..ZViewCut::NONE });
        assert!(!decision.visible_set_unchanged && !decision.skip_unchanged_incomplete_retry);
        assert_eq!(cache.chunks.len(), 1);
    }

    /// Assembling replaces the page slots with the pages' layers, leaves out the instances of pages not resident and
    /// those cut, and says which instances it kept, which picking follows.
    #[test]
    fn assembly_binds_page_slots_and_leaves_out_unresident_pages_and_cut_statics() {
        let instance = |layer, tile_x| SpriteInstance { layer, tile_x, ..SpriteInstance::zeroed() };
        let instances = [instance(NO_ART_PAGE_SLOT, 0.0), instance(1, 1.0), instance(2, 2.0), instance(1, 3.0)];
        fn layer(instance: &mut SpriteInstance) -> &mut u32 { &mut instance.layer }
        let mut output = vec![instance(9, -1.0)];
        assert_eq!(append_drawn_instances(&instances, &[Some(0), Some(7), Some(5)], layer, |_| true, &mut output),
            (None, 0));
        assert_eq!(output.iter().map(|i| i.layer).collect::<Vec<_>>(), [9, 0, 7, 5, 7]);

        let mut output = Vec::new();
        let (kept, unbound) = append_drawn_instances(&instances, &[Some(0), Some(7), None], layer, |_| true,
            &mut output);
        assert_eq!((kept.clone(), unbound), (Some(vec![0, 1, 3]), 1));
        assert_eq!(output.iter().map(|i| (i.layer, i.tile_x)).collect::<Vec<_>>(), [(0, 0.0), (7, 1.0), (7, 3.0)]);
        let mut cut_output = Vec::new();
        assert_eq!(append_drawn_instances(&instances, &[Some(0), Some(7), Some(5)], layer, |index| index != 0,
            &mut cut_output), (Some(vec![1, 2, 3]), 0));

        let key = StaticChunkBatchKey { map_id: 0, gx: 0, gy: 0, scale: 1 };
        let pick = |graphic| StaticPickTile { tile: StaticRenderTile { graphic, world_x: 0.0, world_z: 0.0, z: 0,
            hue: 0 }, is_wet: false };
        let mut cache = StaticChunkRenderCache::default();
        cache.chunks.insert(key, CachedStaticChunk { pick_tiles: (0..4).map(pick).collect(),
            sprite_pick_indices: vec![3, 2, 1, 0], ..Default::default() });
        cache.assembled_picks[0].insert(key, kept.unwrap());
        let batches = RenderStaticChunkBatches { sprite: vec![StaticChunkBatch { key, start: 0, count: 3 }],
            ..Default::default() };
        assert_eq!(cache.gpu_pick_targets(&batches)[0], [pick(3), pick(2), pick(0)]);
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

    /// The chunks that read changed land are stale: they draw until built again. A chunk built from land that changed
    /// before it arrived is stale from the start.
    #[test]
    fn static_chunk_cache_rebuilds_only_the_chunks_that_read_changed_land() {
        use crate::render::assets::tile_atlas::{AtlasParams, Rg16u};
        let mut atlas = TileAtlas::new(AtlasParams { page_texels: UVec2::splat(64), max_layers: 1, world_pages_x: 1,
            ..Default::default() }, 1);
        let (layer, _) = atlas.ensure_layer_for_page(IVec2::ZERO);
        let key = |gx| StaticChunkBatchKey { map_id: 1, gx, gy: 0, scale: 1 };
        let read = |min: (u32, u32), max: (u32, u32)| CachedStaticChunk {
            land_cells: Some(AtlasCells::new(UVec2::new(min.0, min.1), UVec2::new(max.0, max.1))),
            ..Default::default()
        };
        let stale = |cache: &StaticChunkRenderCache| {
            let mut stale = cache.chunks.iter().filter(|(_, chunk)| chunk.land_stale).map(|(key, _)| key.gx)
                .collect::<Vec<_>>();
            stale.sort();
            stale
        };
        let mut cache = StaticChunkRenderCache::default();
        // The first check has nothing to compare with.
        assert_eq!(cache.invalidate_changed_land(&atlas), 0);
        // The chunk at x 0..31 read the cells up to x 32, one cell into its east neighbour's land (a Classic ground
        // diamond on its last column); the chunk at x 32..63 read its own; the third read no land.
        let built_at = atlas.content_revision;
        cache.insert_built(key(0), read((0, 0), (32, 31)), built_at, &atlas, 1);
        cache.insert_built(key(1), read((32, 0), (63, 31)), built_at, &atlas, 1);
        cache.insert_built(key(2), CachedStaticChunk::default(), built_at, &atlas, 1);
        assert_eq!(cache.invalidate_changed_land(&atlas), 0);
        // The east neighbour's land loads after both: both read it, and draw until built again.
        atlas.enqueue_rg16u_block(layer, UVec2::new(32, 8), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(cache.invalidate_changed_land(&atlas), 2);
        assert_eq!(stale(&cache), [0, 1]);
        assert_eq!(cache.chunks.len(), 3);
        // Built again; then land only the second read.
        let built_at = atlas.content_revision;
        cache.insert_built(key(0), read((0, 0), (32, 31)), built_at, &atlas, 1);
        cache.insert_built(key(1), read((32, 0), (63, 31)), built_at, &atlas, 1);
        atlas.enqueue_rg16u_block(layer, UVec2::new(40, 8), UVec2::splat(8), &[Rg16u::pack(3, 0, 4, false, 0); 64]);
        assert_eq!(cache.invalidate_changed_land(&atlas), 1);
        assert_eq!(stale(&cache), [1]);
        // Nothing changed: nothing more.
        assert_eq!(cache.invalidate_changed_land(&atlas), 0);
        // Built from the land before that change, it arrives stale.
        cache.insert_built(key(1), read((32, 0), (63, 31)), built_at, &atlas, 1);
        assert_eq!(stale(&cache), [1]);
        // Every cell changed: every chunk that read land is stale.
        atlas.clear_all_mappings();
        assert_eq!(cache.invalidate_changed_land(&atlas), 1);
        assert_eq!(stale(&cache), [0, 1]);
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
