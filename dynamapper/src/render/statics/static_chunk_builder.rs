//! The builds of the static chunks, off the main thread: a lane of the background pool (`StaticChunkBuilder`) builds each chunk
//! from a copy of what it reads (`StaticChunkJob`), while the chunks built before keep drawing, and the collection
//! takes the chunks into its cache as they come back (`StaticChunkBuilder::receive`). A chunk entering the view draws
//! once built; a chunk whose settings or land changed draws as it was until built again.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc};
use bevy::prelude::*;
use parking_lot::Mutex;
use crate::app::background::{background_pool, Lane, Priority};
use crate::content::{TexArtCcPackageRes, TexArtEcPackageRes, TexAuxEcPackageRes, TexLandEcPackageRes, TileMetaPackageRes};
use crate::render::assets::art::ArtPageGeometry;
use crate::render::assets::tile_atlas::{LandSnapshot, TileAtlas};
use crate::render::terrain::coverage::{EcTerrainBases, EcTerrainParams};
use crate::world::caves::{CaveMap, PlayerCave};
use crate::world::multis::{MultiPart, PlacedMulti};
use crate::world::statics::LazyStaticsStore;
use crate::world::z_view::PackageLand;
use super::static_batches::StaticChunkBatchKey;
use super::static_chunk_cache::{CachedStaticChunk, StaticChunkRenderCache};
use super::static_lights::RenderStaticLightInstances;
use super::static_source::{KrStaticShared, StaticSurfaceLikeResolutionCache};
use super::statics_collect::{StaticCollectFrameConfig, build_static_chunk};

/// The builds sent to the lane and not back yet, at most: the others wait in the collection, which sends the chunks
/// nearest the view's centre first, so few builds are spent on chunks the view has left.
const MAX_STATIC_CHUNK_BUILDS_IN_FLIGHT: usize = 8;

/// What the builds read besides each chunk's statics, land and multis, shared by the builds sent together.
pub(super) struct StaticBuildShared {
    pub(super) tex_art_cc: Option<TexArtCcPackageRes>,
    pub(super) tex_art_ec: Option<TexArtEcPackageRes>,
    pub(super) tex_land_ec: Option<TexLandEcPackageRes>,
    pub(super) tilemeta: Option<TileMetaPackageRes>,
    pub(super) tex_aux_ec: Option<TexAuxEcPackageRes>,
    /// The Kingdom Reborn packages, when the land is Kingdom Reborn's.
    pub(super) kingdom_reborn: Option<KrStaticShared>,
    /// The lights the statics' local lights read: none while those are off.
    pub(super) static_lights: Arc<RenderStaticLightInstances>,
    /// The land's EC terrain parameters (`EcTerrainBases::params`).
    pub(super) terrain: Option<Arc<[Option<EcTerrainParams>]>>,
    pub(super) caves: Option<Arc<CaveMap>>,
    pub(super) player_cave: Option<Arc<PlayerCave>>,
    /// The sprite and the ground atlases' pages, which place the instances on them.
    pub(super) art_pages: (ArtPageGeometry, ArtPageGeometry),
}

/// The multis placed in a chunk, with the parts drawn.
pub(super) type ChunkMultis = Vec<(PlacedMulti, Vec<MultiPart>)>;

/// The build of a chunk, with copies of what it reads.
pub(super) struct StaticChunkJob {
    pub(super) key: StaticChunkBatchKey,
    /// The cache's generation (`StaticChunkRenderCache::generation`) the chunk is built for.
    pub(super) generation: u64,
    /// The tile atlas's `content_revision` the land was copied at.
    pub(super) land_revision: u64,
    pub(super) frame: StaticCollectFrameConfig,
    pub(super) shared: Arc<StaticBuildShared>,
    /// The cells of the tile atlas the build reads.
    pub(super) land: LandSnapshot,
    pub(super) multis: ChunkMultis,
    /// The map package's land of the chunk's map.
    pub(super) own_land: PackageLand,
    /// The statics store of the chunk's map, when the builder's is of another map or other item routes.
    pub(super) store: Option<LazyStaticsStore>,
}

struct StaticChunkBuilt {
    key: StaticChunkBatchKey,
    generation: u64,
    land_revision: u64,
    chunk: CachedStaticChunk,
}

/// What builds the chunks keeps from build to build, in the lane or, in the tests, on the collection's thread.
#[derive(Default)]
struct StaticChunkWorker {
    /// The statics stores, by map (`LazyStaticsStore::fork`).
    stores: HashMap<u32, LazyStaticsStore>,
    surface_resolution_cache: StaticSurfaceLikeResolutionCache,
    /// The generation the resolution cache was filled at: a later one may read other packages, and empties it.
    generation: u64,
}

impl StaticChunkWorker {
    fn build(&mut self, job: StaticChunkJob) -> StaticChunkBuilt {
        let StaticChunkJob { key, generation, land_revision, frame, shared, land, multis, own_land, store } = job;
        if let Some(store) = store {
            self.stores.insert(key.map_id, store);
        }
        if generation != self.generation {
            self.surface_resolution_cache = default();
            self.generation = generation;
        }
        let store = self.stores.get_mut(&key.map_id).expect("the first build of a map's chunks brings its store");
        let chunk = build_static_chunk(key, frame, &shared, &land, &multis, own_land, store,
            &mut self.surface_resolution_cache);
        StaticChunkBuilt { key, generation, land_revision, chunk }
    }
}

/// A build's chunk, or the panic it ended in, which `receive` raises on the main thread.
type BuildOutcome = Result<StaticChunkBuilt, Box<dyn std::any::Any + Send>>;

enum StaticChunkRunner {
    Lane {
        lane: Lane<StaticChunkWorker>,
        built_sender: mpsc::Sender<BuildOutcome>,
        built: Mutex<mpsc::Receiver<BuildOutcome>>,
    },
    /// Builds as the jobs are sent, for the tests.
    #[cfg(test)]
    Inline { worker: Mutex<StaticChunkWorker>, built: Vec<StaticChunkBuilt> },
}

/// Builds the static chunks on a lane of the background pool, one at a time, in the order sent.
#[derive(Resource)]
pub struct StaticChunkBuilder {
    runner: StaticChunkRunner,
    /// The chunks sent and not back yet.
    in_flight: HashSet<StaticChunkBatchKey>,
    /// The item routes revision of the store sent for each map.
    sent_stores: HashMap<u32, u64>,
    /// The lights sent, by their signature (`static_light_signature`), and the empty lights.
    lights: Option<(u64, Arc<RenderStaticLightInstances>)>,
    no_lights: Arc<RenderStaticLightInstances>,
    terrain: Option<Arc<[Option<EcTerrainParams>]>>,
}

impl StaticChunkBuilder {
    pub fn spawn() -> Self {
        let (built_sender, built) = mpsc::channel();
        let lane = Lane::new("static chunk build", StaticChunkWorker::default());
        Self::with_runner(StaticChunkRunner::Lane { lane, built_sender, built: Mutex::new(built) })
    }

    #[cfg(test)]
    pub(super) fn inline() -> Self {
        Self::with_runner(StaticChunkRunner::Inline { worker: default(), built: Vec::new() })
    }

    fn with_runner(runner: StaticChunkRunner) -> Self {
        Self {
            runner,
            in_flight: HashSet::new(),
            sent_stores: HashMap::new(),
            lights: None,
            no_lights: default(),
            terrain: None,
        }
    }

    /// The lights of the signature `signature` (`static_light_signature`), the same as sent before while it holds;
    /// none without one, the local lights off.
    pub(super) fn lights(&mut self, signature: Option<u64>, lights: &RenderStaticLightInstances)
        -> Arc<RenderStaticLightInstances> {
        let Some(signature) = signature else {
            return self.no_lights.clone();
        };
        match &self.lights {
            Some((sent, lights)) if *sent == signature => lights.clone(),
            _ => self.lights.insert((signature, Arc::new(lights.clone()))).1.clone(),
        }
    }

    /// The terrain parameters of `bases`, the same as sent before while they hold.
    pub(super) fn terrain(&mut self, bases: Option<&EcTerrainBases>) -> Option<Arc<[Option<EcTerrainParams>]>> {
        let params = bases?.params.as_slice();
        if self.terrain.as_deref() != Some(params) {
            self.terrain = Some(Arc::from(params));
        }
        self.terrain.clone()
    }

    /// Sends the builds of the `keys` not current in the cache (`StaticChunkRenderCache::is_current`) nor sent yet, nearest
    /// the centre of `keys` first, while fewer than `MAX_STATIC_CHUNK_BUILDS_IN_FLIGHT` are out. `make_job` makes a build;
    /// the first of a map, or after its item routes changed (`item_routes_revision`), brings a fork of `store`.
    pub(super) fn send(
        &mut self,
        cache: &StaticChunkRenderCache,
        keys: &[StaticChunkBatchKey],
        store: &Mutex<LazyStaticsStore>,
        item_routes_revision: u64,
        mut make_job: impl FnMut(StaticChunkBatchKey) -> StaticChunkJob,
    ) {
        let room = MAX_STATIC_CHUNK_BUILDS_IN_FLIGHT.saturating_sub(self.in_flight.len());
        let mut wanted = keys.iter().copied()
            .filter(|key| !cache.is_current(key) && !self.in_flight.contains(key))
            .collect::<Vec<_>>();
        if room == 0 || wanted.is_empty() {
            return;
        }
        let centre = |key: &StaticChunkBatchKey| Vec2::new(key.gx as f32, key.gy as f32) + Vec2::splat(key.scale as f32 / 2.0);
        let view_centre = keys.iter().map(centre).sum::<Vec2>() / keys.len() as f32;
        wanted.sort_by(|a, b| centre(a).distance_squared(view_centre).total_cmp(&centre(b).distance_squared(view_centre)));
        for key in wanted.into_iter().take(room) {
            let mut job = make_job(key);
            if self.sent_stores.insert(key.map_id, item_routes_revision) != Some(item_routes_revision) {
                job.store = Some(store.lock().fork());
            }
            self.in_flight.insert(key);
            match &mut self.runner {
                StaticChunkRunner::Lane { lane, built_sender, .. } => {
                    let built_sender = built_sender.clone();
                    lane.submit(background_pool(), Priority::Visible, move |worker| {
                        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker.build(job)));
                        let _ = built_sender.send(outcome);
                    });
                }
                #[cfg(test)]
                StaticChunkRunner::Inline { worker, built } => built.push(worker.get_mut().build(job)),
            }
        }
    }

    /// Takes the chunks built into `cache`, but those built for an earlier generation (`StaticChunkRenderCache::insert_built`).
    pub(super) fn receive(&mut self, cache: &mut StaticChunkRenderCache, atlas: &TileAtlas, visible_tick: u64) {
        let mut take = |built: StaticChunkBuilt| {
            self.in_flight.remove(&built.key);
            if built.generation == cache.generation() {
                cache.insert_built(built.key, built.chunk, built.land_revision, atlas, visible_tick);
            }
        };
        match &mut self.runner {
            // The runner holds a sender, so the channel never disconnects.
            StaticChunkRunner::Lane { built, .. } => while let Ok(outcome) = built.get_mut().try_recv() {
                match outcome {
                    Ok(chunk) => take(chunk),
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            },
            #[cfg(test)]
            StaticChunkRunner::Inline { built, .. } => built.drain(..).for_each(take),
        }
    }
}
