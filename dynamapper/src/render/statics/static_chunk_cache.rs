//! The per-chunk cache of collected statics: what a cached chunk keeps, when it is reused or rebuilt, and the
//! signatures that retry a chunk whose atlas pages or lights were incomplete.

use crate::configs::settings::ClientTextureSource;
use crate::render::assets::tile_atlas::{AtlasCells, TileAtlas};
use crate::world::z_view::ZViewCut;
use crate::render::assets::art::{GroundArtPageAtlas, SpriteArtPageAtlas};
use bevy::prelude::*;
use std::collections::HashMap;
use super::static_source::StaticLandStyle;
use super::static_batches::{
    GroundTileInstance, RenderStaticChunkBatches, SpriteInstance, StaticChunkBatchKey, StaticPickTile,
};

pub(super) const STATIC_CHUNK_CACHE_HYSTERESIS_TICKS: u64 = 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StaticAtlasRetrySignature {
    pub(super) sprite_mapping_revision: u64,
    pub(super) ground_mapping_revision: u64,
    pub(super) sprite_resident_pages: usize,
    pub(super) ground_resident_pages: usize,
    pub(super) sprite_pending_pages: usize,
    pub(super) ground_pending_pages: usize,
    pub(super) sprite_active_layers: u32,
    pub(super) ground_active_layers: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StaticChunkCacheConfig {
    pub(super) map_id: u32,
    pub(super) dot_mode: bool,
    pub(super) art_source: Option<ClientTextureSource>,
    /// `StaticArtSourceState::land_style`: a change redraws the statics that continue the land from the other art.
    pub(super) land_style: StaticLandStyle,
    pub(super) sprite_atlas_mapping_revision: u64,
    pub(super) ground_atlas_mapping_revision: u64,
    // The land the chunks read from the tile atlas is not here: a change rebuilds only the chunks that read the
    // changed cells (`StaticChunkRenderCache::invalidate_changed_land`).
    pub(super) static_art_local_light_signature: Option<u64>,
    pub(super) show_hidden_multi_parts: bool,
    /// The statics store's item routes (`LazyStaticsStore::item_routes_revision`).
    pub(super) item_routes_revision: u64,
    /// The z-view limit: a change rebuilds the cached chunks without the statics it hides.
    pub(super) z_view: ZViewCut,
    /// The caves drawn over the map (`CavesRes::revision`).
    pub(super) caves_revision: u64,
    /// The land's EC terrain parameters (`EcTerrainBases::sources_revision`), whose hard edges close ground patch sides
    /// (static_surface.rs).
    pub(super) terrain_sources_revision: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedStaticChunkStats {
    pub(super) visited_blocks: usize,
    pub(super) source_tiles: usize,
    pub(super) ground_land_tiles: usize,
    pub(super) atlas_hits: usize,
    pub(super) atlas_misses: usize,
    pub(super) requested_pages: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedStaticChunk {
    pub(super) sprite_instances: Vec<SpriteInstance>,
    pub(super) ground_instances: Vec<GroundTileInstance>,
    pub(super) pick_tiles: Vec<StaticPickTile>,
    pub(super) sprite_pick_indices: Vec<usize>,
    pub(super) ground_pick_indices: Vec<usize>,
    pub(super) stats: CachedStaticChunkStats,
    pub(super) last_visible_tick: u64,
    /// The cells of the tile atlas the chunk read (`StaticChunkBuildState::land_cells`).
    pub(super) land_cells: Option<AtlasCells>,
}

#[derive(Resource, Default)]
pub struct StaticChunkRenderCache {
    config: Option<StaticChunkCacheConfig>,
    current_tick: u64,
    pub(super) chunks: HashMap<StaticChunkBatchKey, CachedStaticChunk>,
    pub(super) last_visible_chunk_keys: Vec<StaticChunkBatchKey>,
    pub(super) last_incomplete_retry_atlas_signature: Option<StaticAtlasRetrySignature>,
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
    pub(super) fn begin_frame(&mut self) -> u64 {
        self.current_tick = self.current_tick.saturating_add(1);
        self.current_tick
    }

    /// Drops every cached chunk on the next collection, as a change of the configuration does (the control socket's
    /// `reload_land`).
    pub(crate) fn rebuild_all(&mut self) {
        self.config = None;
    }

    pub(super) fn sync_config(&mut self, config: StaticChunkCacheConfig) -> bool {
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
    pub(super) fn invalidate_changed_land(&mut self, atlas: &TileAtlas) -> usize {
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

    pub(super) fn prune_stale(&mut self, visible_tick: u64) {
        self.chunks.retain(|_, chunk| {
            visible_tick.saturating_sub(chunk.last_visible_tick)
                <= STATIC_CHUNK_CACHE_HYSTERESIS_TICKS
        });
    }
}

pub(super) fn static_atlas_retry_signature(
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

pub(super) fn aggregate_cached_visible_static_stats(
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

pub(super) fn should_skip_unchanged_incomplete_static_retry(
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

pub(super) fn static_light_signature(
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
