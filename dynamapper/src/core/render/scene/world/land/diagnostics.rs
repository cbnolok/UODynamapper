#![allow(unused)]
use bevy::prelude::*;
use crate::prelude::*;

const DIAG_OUTPUT_PERIOD_SECONDS: f32 = 2.0;

// Contains full performance and memory resource tracking fields for debugging and profiling.
#[derive(Resource, Default)]
pub struct LandChunkMeshDiagnostics {
    pub mesh_allocs: usize,
    pub alloc_high_water: usize,
    pub build_avg: f32,
    pub build_last: f32,
    pub build_peak: f32,
    pub pool_in_positions: usize,
    pub pool_in_normals: usize,
    pub pool_in_uvs: usize,
    pub pool_in_indices: usize,
    pub chunks_on_screen: usize, // Number of rendered chunk meshes (diagnostic log field)
}
impl LandChunkMeshDiagnostics {
    pub fn log(&self) {
        logger::one(
            None,
            LogSev::Diagnostics,
            LogAbout::RenderWorldLand,
            &format!(
                // ChunksOnScreen: actual rendered chunk mesh count this frame.
                "ChunksOnScreen: {} | Pool avail: {} | Allocs: {} (peak {}) | Mesh ms (avg/latest/peak): {:.1}/{:.1}/{:.1}",
                self.chunks_on_screen,
                self.pool_in_positions,
                self.mesh_allocs,
                self.alloc_high_water,
                self.build_avg,
                self.build_last,
                self.build_peak,
            ),
        );
    }
}

/// Simple circular buffer for logging history of mesh build times.
/// This is great for understanding steady-state vs. peak/burst mesh gen.
#[derive(Resource)]
pub struct MeshBuildPerfHistory {
    buckets: Vec<f32>,
    pos: usize,
    count: usize,
}
impl MeshBuildPerfHistory {
    pub fn new(size: usize) -> Self {
        Self {
            buckets: vec![0.0; size],
            pos: 0,
            count: 0,
        }
    }
    pub fn push(&mut self, val: f32) {
        self.buckets[self.pos] = val;
        self.pos = (self.pos + 1) % self.buckets.len();
        if self.count < self.buckets.len() {
            self.count += 1;
        }
    }
    pub fn avg(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            self.buckets.iter().take(self.count).sum::<f32>() / (self.count as f32)
        }
    }
    /// Highest mesh-building time (ms) observed in window (all history).
    pub fn peak(&self) -> f32 {
        self.buckets
            .iter()
            .take(self.count)
            .copied()
            .fold(0.0, f32::max)
    }
}

/// Print key diagnostics to stdout at a throttled interval (every 2 seconds by default).
fn print_render_stats(
    mut timer: Local<Option<Timer>>,
    time: Res<Time>,
    diag: Res<LandChunkMeshDiagnostics>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(DIAG_OUTPUT_PERIOD_SECONDS, TimerMode::Repeating));
    timer.tick(time.delta());
    if timer.finished() {
        diag.log();
    }
}

