//! The per-chunk cache of collected statics: what a cached chunk keeps, when it is reused or rebuilt, and the
//! signatures that assemble the chunks again when their atlas pages arrive or move, or rebuild them when their lights
//! change.

use crate::settings::ClientTextureSource;
use crate::render::assets::tile_atlas::{AtlasCells, TileAtlas};
use crate::world::z_view::ZViewCut;
use crate::render::assets::art::{ArtPageGeometry, ArtPageKey, GroundArtPageAtlas, SpriteArtPageAtlas};
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
    // The art pages' atlas layers are not here: the chunks keep their pages, which assembling them binds to layers.
    /// The sprite and the ground atlases' pages, which the instances are placed on (`ArtPageGeometry`).
    pub(super) art_pages: (ArtPageGeometry, ArtPageGeometry),
    // The land the chunks read from the tile atlas is not here: a change rebuilds only the chunks that read the
    // changed cells (`StaticChunkRenderCache::invalidate_changed_land`).
    pub(super) static_art_local_light_signature: Option<u64>,
    pub(super) show_hidden_multi_parts: bool,
    /// The statics store's item routes (`LazyStaticsStore::item_routes_revision`).
    pub(super) item_routes_revision: u64,
    // The z-view's cut is not here: the chunks keep what it reads of their statics (`StaticCut`), and assembling them
    // leaves out what it hides.
    /// The caves drawn over the map (`CavesRes::revision`).
    pub(super) caves_revision: u64,
    /// The land's EC terrain parameters (`EcTerrainBases::sources_revision`), whose hard edges close ground patch sides
    /// (static_surface.rs).
    pub(super) terrain_sources_revision: u64,
}

impl StaticChunkCacheConfig {
    /// Whether the chunks built for `self` can draw under `other` until built again: their instances' pages are of the
    /// same art, in pages of the same size. Under another art they would bind pages the atlases no longer hold.
    fn same_art(&self, other: &Self) -> bool {
        (self.dot_mode, self.art_source, self.land_style, self.art_pages)
            == (other.dot_mode, other.art_source, other.land_style, other.art_pages)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedStaticChunkStats {
    pub(super) visited_blocks: usize,
    pub(super) source_tiles: usize,
    pub(super) ground_land_tiles: usize,
}

/// The last assembly of the cached chunks into the outputs.
#[derive(Clone, Copy, Debug)]
pub(super) struct StaticAssembly {
    /// The atlases after it, with their mapping revisions from before it: a page evicted while it bound the pages
    /// leaves a revision behind, which assembles again.
    pub(super) signature: StaticAtlasRetrySignature,
    /// The instances drawn, and those left out for want of their pages.
    pub(super) atlas_hits: usize,
    pub(super) atlas_misses: usize,
    /// The z-view's cut it left out statics by.
    pub(super) z_view: ZViewCut,
}

/// What the z-view's cut (`ZViewCut`) reads of a static, which assembling its chunk tests (`hides`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct StaticCut {
    /// The static's tile flags and z.
    pub(super) flags: u64,
    pub(super) z: i8,
    /// A roof's averaged land z of its own cell, which KR's cut counts its layer from (`ZViewCut::roof_layer`).
    pub(super) own_land_z: Option<i32>,
    /// Drawn whatever the cut: the player's cave lifts it (`CaveStatic::Lifted`).
    pub(super) uncut: bool,
    /// The static's cell, which the building-only cut keeps to (`ZViewCut::cuts_cell`).
    pub(super) x: i32,
    pub(super) y: i32,
    /// For a cave's floor buried under its cell's land outside the caves, that land's z: the floor is covered while the
    /// cut draws that land (`max_ground_z`), and shows when the cut leaves the land out (`caves::is_buried_floor`).
    pub(super) buried_by_land_z: Option<i32>,
}

impl StaticCut {
    pub(super) fn hides(&self, cut: &ZViewCut) -> bool {
        (!self.uncut && cut.cuts_cell(self.x, self.y)
            && cut.hides_static_over_land(self.flags, self.z, || self.own_land_z))
            || self.buried_by_land_z.is_some_and(|land_z| land_z <= cut.max_ground_z)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedStaticChunk {
    pub(super) sprite_instances: Vec<SpriteInstance>,
    pub(super) ground_instances: Vec<GroundTileInstance>,
    pub(super) pick_tiles: Vec<StaticPickTile>,
    /// What the z-view's cut reads of each of the `pick_tiles`, the statics of the chunk's instances.
    pub(super) cuts: Vec<StaticCut>,
    pub(super) sprite_pick_indices: Vec<usize>,
    pub(super) ground_pick_indices: Vec<usize>,
    /// The pages of the sprites' and the ground instances' page slots (`StaticChunkBuildState::sprite_pages`).
    pub(super) sprite_pages: Vec<ArtPageKey>,
    pub(super) ground_pages: Vec<ArtPageKey>,
    pub(super) stats: CachedStaticChunkStats,
    pub(super) last_visible_tick: u64,
    /// The cells of the tile atlas the chunk read (`StaticChunkBuildState::land_cells`).
    pub(super) land_cells: Option<AtlasCells>,
    /// The cache's generation the chunk was built for (`StaticChunkRenderCache::generation`).
    pub(super) generation: u64,
    /// Whether the land the chunk read changed since (`StaticChunkRenderCache::invalidate_changed_land`).
    pub(super) land_stale: bool,
}

#[derive(Resource, Default)]
pub struct StaticChunkRenderCache {
    config: Option<StaticChunkCacheConfig>,
    /// Counts the changes of `config`: a chunk built for an earlier one draws until built again, under the same art
    /// (`StaticChunkCacheConfig::same_art`).
    generation: u64,
    /// Whether chunks were built since the last assembly (`insert_built`).
    pub(super) chunks_arrived: bool,
    current_tick: u64,
    pub(super) chunks: HashMap<StaticChunkBatchKey, CachedStaticChunk>,
    pub(super) last_visible_chunk_keys: Vec<StaticChunkBatchKey>,
    pub(super) last_assembly: Option<StaticAssembly>,
    /// The chunk instances in the outputs, by chunk, of the sprites and the ground, where the last assembly left some
    /// out: their order in the batch's output range.
    pub(super) assembled_picks: [HashMap<StaticChunkBatchKey, Vec<u32>>; 2],
    /// The tile atlas's `content_revision` the cached chunks were checked against (`invalidate_changed_land`).
    land_revision: Option<u64>,
}

/// Whether the land a chunk read (`CachedStaticChunk::land_cells`) changed after the tile atlas's `content_revision`
/// was `revision`: every land did when the atlas no longer says which cells changed (`TileAtlas::changed_cells_after`).
fn land_changed_after(atlas: &TileAtlas, revision: u64, read: Option<AtlasCells>) -> bool {
    read.is_some_and(|read| atlas.changed_cells_after(revision)
        .is_none_or(|mut changed| changed.any(|cells| cells.intersects(&read))))
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
                let kept = self.assembled_picks[kind].get(&batch.key);
                if targets.len() != batch.start as usize || kept.map_or(indices.len(), Vec::len) != batch.count as usize {
                    return Vec::new();
                }
                match kept {
                    Some(kept) => targets.extend(kept.iter().map(|&kept| chunk.pick_tiles[indices[kept as usize]])),
                    None => targets.extend(indices.iter().map(|&index| chunk.pick_tiles[index])),
                }
            }
            targets
        })
    }
    pub(super) fn begin_frame(&mut self) -> u64 {
        self.current_tick = self.current_tick.saturating_add(1);
        self.current_tick
    }

    /// Builds every cached chunk again from the next collection, as a change of the configuration does (the control
    /// socket's `reload_land`).
    pub(crate) fn rebuild_all(&mut self) {
        self.config = None;
    }

    /// A change of the configuration starts a generation: every chunk is built again, the chunks built for the last
    /// one drawing meanwhile.
    pub(super) fn sync_config(&mut self, config: StaticChunkCacheConfig) -> bool {
        let changed = self.config != Some(config);
        if changed {
            if !self.config.is_some_and(|old| old.same_art(&config)) {
                self.chunks.clear();
            }
            self.config = Some(config);
            self.generation += 1;
            self.last_visible_chunk_keys.clear();
            self.last_assembly = None;
        }
        changed
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the chunk `key` is cached, built for this generation from the land as it is.
    pub(super) fn is_current(&self, key: &StaticChunkBatchKey) -> bool {
        self.chunks.get(key).is_some_and(|chunk| chunk.generation == self.generation && !chunk.land_stale)
    }

    /// Caches `chunk`, built for this generation from the land at the tile atlas's `land_revision`: stale already when
    /// that land changed since (`land_changed_after`).
    pub(super) fn insert_built(&mut self, key: StaticChunkBatchKey, mut chunk: CachedStaticChunk, land_revision: u64,
        atlas: &TileAtlas, visible_tick: u64) {
        chunk.generation = self.generation;
        chunk.land_stale = land_changed_after(atlas, land_revision, chunk.land_cells);
        chunk.last_visible_tick = visible_tick;
        self.chunks.insert(key, chunk);
        self.chunks_arrived = true;
    }

    /// Marks stale the cached chunks that read tile atlas cells changed since the last call: the land loading, a
    /// neighbouring chunk's land arriving after the chunk, a texture bake or the land of a cave. They draw until built
    /// again. Returns how many chunks it marked.
    pub(super) fn invalidate_changed_land(&mut self, atlas: &TileAtlas) -> usize {
        let Some(seen) = self.land_revision.replace(atlas.content_revision) else {
            return 0;
        };
        if seen == atlas.content_revision {
            return 0;
        }
        let mut marked = 0;
        for chunk in self.chunks.values_mut().filter(|chunk| !chunk.land_stale) {
            if land_changed_after(atlas, seen, chunk.land_cells) {
                chunk.land_stale = true;
                marked += 1;
            }
        }
        marked
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

/// The visible chunks' stats, and how many art pages they draw from.
pub(super) fn aggregate_cached_visible_static_stats(
    cache: &mut StaticChunkRenderCache,
    visible_chunk_keys: &[StaticChunkBatchKey],
    visible_tick: u64,
) -> (CachedStaticChunkStats, usize) {
    let mut aggregate = CachedStaticChunkStats::default();
    let mut pages = std::collections::HashSet::new();

    for chunk_key in visible_chunk_keys {
        let Some(chunk) = cache.chunks.get_mut(chunk_key) else {
            continue;
        };
        chunk.last_visible_tick = visible_tick;
        aggregate.visited_blocks += chunk.stats.visited_blocks;
        aggregate.source_tiles += chunk.stats.source_tiles;
        aggregate.ground_land_tiles += chunk.stats.ground_land_tiles;
        pages.extend(chunk.sprite_pages.iter().chain(&chunk.ground_pages).copied());
    }

    (aggregate, pages.len())
}

/// Whether the last assembly stands though it was incomplete: no chunk arrived and no page moved since (the chunks
/// still building arrive in `chunks_arrived`).
pub(super) fn should_skip_unchanged_incomplete_static_retry(
    cache: &StaticChunkRenderCache,
    visible_set_unchanged: bool,
    all_visible_chunks_complete: bool,
    atlas_retry_signature: StaticAtlasRetrySignature,
) -> bool {
    visible_set_unchanged
        && !all_visible_chunks_complete
        && cache.last_assembly.is_some_and(|assembly| assembly.signature == atlas_retry_signature)
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
