//! Background chunk-data loader thread.
//!
//! Offloads map-block I/O to a dedicated OS thread, keeping the main (Bevy) thread free from disk stalls during large zoom-outs.
//!
//! ## Protocol
//!
//! 1. The main-thread draw system sends a [`LoadRequest`] with the list of
//!    uncached block coordinates and the map package reader.
//! 2. This thread breaks the request into sub-batches of `SUB_BATCH_SIZE`
//!    blocks, loading each batch and sending a [`LoadResult`] back immediately.
//!    This lets the main thread start rendering deferred chunks progressively
//!    instead of waiting for the entire 100K+ block request to finish.
//! 3. The final sub-batch has `is_final = true`, signalling to the main thread
//!    that the request is complete and a new one can be dispatched.

use std::sync::{mpsc, Arc};
use std::time::Instant;

use uocf::classic::map::{MapBlock, MapBlockRelPos};
use udd_container::UddpReader;

use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::maps;

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

// ---------------------------------------------------------------------------
// ChunkLoaderThread — owns the send/receive ends visible to the main thread
// ---------------------------------------------------------------------------

pub struct ChunkLoaderThread {
    request_tx: mpsc::Sender<LoadRequest>,
    result_rx: mpsc::Receiver<LoadResult>,
}

impl ChunkLoaderThread {
    /// Spawns the background loader thread.  Call once (from a `Local`).
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LoadRequest>();
        let (result_tx, result_rx) = mpsc::channel::<LoadResult>();

        std::thread::Builder::new()
            .name("chunk-loader".into())
            .spawn(move || {
                loader_thread_main(request_rx, result_tx);
            })
            .expect("Failed to spawn chunk-loader thread");

        Self {
            request_tx,
            result_rx,
        }
    }

    #[inline]
    pub fn send_request(&self, request: LoadRequest) {
        let _ = self.request_tx.send(request);
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

impl Default for ChunkLoaderThread {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Thread entry-point
// ---------------------------------------------------------------------------

fn loader_thread_main(rx: mpsc::Receiver<LoadRequest>, tx: mpsc::Sender<LoadResult>) {
    while let Ok(req) = rx.recv() {
        let t0 = Instant::now();
        let total_blocks = req.blocks_to_load.len();
        let _trace_span = crate::tracy_span!(
            "worldmap::chunk_loader_request",
            requested_blocks = total_blocks
        );

        // ── Process in sub-batches ───────────────────────────────────────
        let chunks_iter = req.blocks_to_load.chunks(SUB_BATCH_SIZE);
        let num_sub_batches = (total_blocks + SUB_BATCH_SIZE - 1) / SUB_BATCH_SIZE;
        let mut batch_idx = 0usize;
        let mut total_loaded = 0usize;

        for batch_slice in chunks_iter {
            batch_idx += 1;
            let is_final = batch_idx == num_sub_batches;
            let _batch_span = crate::tracy_span!(
                "worldmap::chunk_loader_batch",
                batch_index = batch_idx,
                batch_blocks = batch_slice.len(),
                is_final = is_final
            );

            let loaded_blocks = maps::load_blocks_from_package(
                req.map_package.as_ref(),
                batch_slice,
                req.size_blocks_height,
            )
            .unwrap_or_else(|e| {
                eprintln!("chunk-loader: load_blocks failed: {e}");
                Vec::new()
            });
            total_loaded += loaded_blocks.len();

            // Send this sub-batch to the main thread immediately.
            let _ = tx.send(LoadResult {
                loaded_blocks,
                is_final,
            });
        }

        // Handle empty request edge case.
        if total_blocks == 0 {
            let _ = tx.send(LoadResult {
                loaded_blocks: Vec::new(),
                is_final: true,
            });
        }

        let elapsed_us = t0.elapsed().as_micros();
        if elapsed_us > 500 {
            console_logger::one(
                LogSev::Debug,
                LogAbout::Performance,
                format!(
                    "Perf: Slow land chunk-loader run: loaded {} blocks ({} sub-batches) in {} µs",
                    total_loaded, num_sub_batches, elapsed_us,
                )
                .as_str(),
            );
        }
    }
}
