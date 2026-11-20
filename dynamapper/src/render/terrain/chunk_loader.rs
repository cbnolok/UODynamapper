//! Background loading of the map blocks the land draws.
//!
//! Map-block reads run on a lane of the background pool, keeping the main (Bevy) thread free from disk stalls during
//! large zoom-outs.
//!
//! ## Protocol
//!
//! 1. The main-thread draw system sends a [`LoadRequest`] with the list of
//!    uncached block coordinates and the map package reader.
//! 2. The request is split into sub-batches of `SUB_BATCH_SIZE` blocks, one lane job each, and each sends its
//!    [`LoadResult`] back as soon as it is read. This lets the main thread start rendering deferred chunks progressively
//!    instead of waiting for the entire 100K+ block request to finish, and lets other background work through between
//!    sub-batches.
//! 3. The final sub-batch has `is_final = true`, signalling to the main thread
//!    that the request is complete and a new one can be dispatched.

use std::sync::{mpsc, Arc};

use uocf::classic::map::{MapBlock, MapBlockRelPos};
use udd_container::UddpReader;

use crate::app::background::{background_pool, Lane, Priority};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::world::maps;

/// Number of blocks per sub-batch.  Tuned so each sub-batch takes ~10-30 ms,
/// giving the main thread frequent opportunities to poll and render while
/// reducing the number of channel sends and poll round-trips.
const SUB_BATCH_SIZE: usize = 8192;

// ---------------------------------------------------------------------------
// Public request / result types
// ---------------------------------------------------------------------------

pub struct LoadRequest {
    pub map_package: Arc<UddpReader>,
    pub size_blocks_height: u32,
    pub blocks_to_load: Vec<MapBlockRelPos>,
}

pub struct LoadResult {
    pub loaded_blocks: Vec<MapBlock>,
    /// True when this is the last sub-batch of the current request.
    pub is_final: bool,
}

/// Sends the map block reads to the background pool and hands their results back.
pub struct ChunkLoader {
    lane: Lane<()>,
    result_tx: mpsc::Sender<LoadResult>,
    result_rx: mpsc::Receiver<LoadResult>,
}

impl ChunkLoader {
    pub fn new() -> Self {
        let (result_tx, result_rx) = mpsc::channel::<LoadResult>();
        Self { lane: Lane::new("map block load", ()), result_tx, result_rx }
    }

    /// Queues the request's sub-batches on the lane, in order.
    pub fn send_request(&self, request: LoadRequest) {
        let LoadRequest { map_package, size_blocks_height, blocks_to_load } = request;
        let total_blocks = blocks_to_load.len();
        if total_blocks == 0 {
            let _ = self.result_tx.send(LoadResult { loaded_blocks: Vec::new(), is_final: true });
            return;
        }
        let blocks_to_load = Arc::new(blocks_to_load);
        let num_sub_batches = total_blocks.div_ceil(SUB_BATCH_SIZE);
        for batch_index in 0..num_sub_batches {
            let (map_package, blocks, tx) = (Arc::clone(&map_package), Arc::clone(&blocks_to_load), self.result_tx.clone());
            self.lane.submit(background_pool(), Priority::Visible, move |()| {
                let start = batch_index * SUB_BATCH_SIZE;
                let batch_slice = &blocks[start..(start + SUB_BATCH_SIZE).min(blocks.len())];
                let is_final = batch_index + 1 == num_sub_batches;
                load_sub_batch(&map_package, batch_slice, size_blocks_height, is_final, &tx);
            });
        }
    }

    /// Non-blocking drain: returns all currently available sub-batch results.
    #[inline]
    pub fn drain_results(&self) -> Vec<LoadResult> {
        let mut results = Vec::new();
        while let Ok(r) = self.result_rx.try_recv() {
            results.push(r);
        }
        results
    }
}

impl Default for ChunkLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// The time past which a sub-batch's read is logged as slow.
const SLOW_SUB_BATCH_US: u128 = 500;

fn load_sub_batch(
    map_package: &UddpReader,
    batch_slice: &[MapBlockRelPos],
    size_blocks_height: u32,
    is_final: bool,
    tx: &mpsc::Sender<LoadResult>,
) {
    let t0 = std::time::Instant::now();
    let _batch_span = crate::tracy_span!(
        "worldmap::chunk_loader_batch",
        batch_blocks = batch_slice.len(),
        is_final = is_final
    );
    let loaded_blocks = maps::load_blocks_from_package(map_package, batch_slice, size_blocks_height).unwrap_or_else(|e| {
        eprintln!("chunk-loader: load_blocks failed: {e}");
        Vec::new()
    });
    let total_loaded = loaded_blocks.len();
    // Send this sub-batch to the main thread immediately.
    let _ = tx.send(LoadResult { loaded_blocks, is_final });
    let elapsed_us = t0.elapsed().as_micros();
    if elapsed_us > SLOW_SUB_BATCH_US {
        console_logger::one(
            LogSev::Debug,
            LogAbout::Performance,
            format!("Perf: Slow land chunk-loader sub-batch: loaded {total_loaded} blocks in {elapsed_us} µs").as_str(),
        );
    }
}
