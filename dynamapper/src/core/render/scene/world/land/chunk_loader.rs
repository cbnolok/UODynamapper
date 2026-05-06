//! Background chunk-data loader thread.
//!
//! Offloads map-block I/O **and** texture-cache warming to a dedicated OS thread,
//! keeping the main (Bevy) thread free from disk stalls during large zoom-outs.
//!
//! ## Protocol
//!
//! 1. The main-thread draw system sends a [`LoadRequest`] with the list of
//!    uncached block coordinates, the map package reader, and an `Arc<TexMap2D>`.
//! 2. This thread breaks the request into sub-batches of `SUB_BATCH_SIZE`
//!    blocks, loading each batch and sending a [`LoadResult`] back immediately.
//!    This lets the main thread start rendering deferred chunks progressively
//!    instead of waiting for the entire 100K+ block request to finish.
//! 3. The final sub-batch has `is_final = true`, signalling to the main thread
//!    that the request is complete and a new one can be dispatched.

use std::sync::{mpsc, Arc};
use std::time::Instant;

use uocf::classic::land_texture::TexMap;
use uocf::classic::map::{MapBlock, MapBlockRelPos};
use uocf::udd::UddpReader;

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
    /// Shared handle used to warm the texture pixel-data cache on this thread
    /// so the main thread's `precache_textures_parallel` hits only cache lookups.
    pub texmap_2d: Arc<TexMap>,
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

        // Deduplication bitmask: one bit per possible u16 tile ID (0..65535).
        // 1024 u64 words × 64 bits = 65,536 bits = 8 KB on the stack-ish heap.
        //
        // For a given tile ID `id`:
        //   word index = id >> 6   (id / 64)
        //   bit  index = id & 63   (id % 64)
        //
        // Why a bitmask instead of a HashSet<u16> or Vec<u16>?
        //  • Fixed 8 KB footprint — no allocator churn from bucket resizing.
        //  • O(1) test-and-set with bit ops (shift + mask + OR), zero hashing overhead.
        //  • Cache-friendly: 8 KB fits in L1; a HashSet would scatter across many cache lines.
        //
        //
        // This ensures each unique tile ID is preloaded exactly once per request,
        // even though the same ID may appear in thousands of cells across hundreds
        // of map blocks.
        let mut seen_count = 0usize;
        let mut seen_ids = Box::new([0u64; 1024]);

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

            // Warm texture cache for this sub-batch's tile IDs.
            for block in &loaded_blocks {
                for cell in &block.cells {
                    let word = (cell.id as usize) >> 6;
                    let bit = (cell.id as usize) & 63;
                    if (seen_ids[word] & (1u64 << bit)) == 0 {
                        seen_ids[word] |= 1u64 << bit;
                        seen_count += 1;
                        let _ = req.texmap_2d.preload_pixel_data(cell.id as usize);
                    }
                }
            }

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
                    "Perf: Slow land chunk-loader run: loaded {} blocks ({} sub-batches) + warmed {} textures in {} µs",
                    total_loaded, num_sub_batches, seen_count, elapsed_us,
                )
                .as_str(),
            );
        }
    }
}
