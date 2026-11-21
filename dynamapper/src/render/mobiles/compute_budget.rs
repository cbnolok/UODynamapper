//! The GPU time the mobiles' background compute may take per frame, shared by its two users in the render world: the
//! RIFE in-betweens (rife_in_betweens/render.rs) spend first, and the SPAN upscales (upscales/render.rs) take what is
//! left, since they upscale the sequences RIFE makes. Each estimates its steps' cost with a model of its own, scaled to
//! match what its [`GpuClock`] times now and then.

use std::sync::Arc;
use bevy::prelude::*;
use parking_lot::Mutex;

/// The estimated GPU time the mobiles' compute may take per frame. A step longer than the budget runs alone in a frame
/// nothing else ran in. On the Radeon 880M the timed work stays within 0.1 ms of it.
pub const MOBILE_COMPUTE_FRAME_BUDGET_MS: f64 = 2.0;
/// Where the GPU has timestamp queries, a frame's work now and then is timed and the estimates scaled to match: each
/// timing moves the scale this much of the way, within these bounds.
const COST_SCALE_WEIGHT: f64 = 0.25;
const COST_SCALE_BOUNDS: (f64, f64) = (0.25, 16.0);
const TIMESTAMPS: u32 = 2;
const TIMESTAMP_BYTES: u64 = TIMESTAMPS as u64 * std::mem::size_of::<u64>() as u64;
const NANOS_PER_MS: f64 = 1e6;

/// The estimated GPU time spent this frame by the mobiles' compute.
#[derive(Resource, Default)]
pub struct MobileComputeBudget {
    spent_ms: f64,
}

impl MobileComputeBudget {
    /// The estimated ms spent so far this frame.
    pub fn spent_ms(&self) -> f64 {
        self.spent_ms
    }

    pub fn spend(&mut self, ms: f64) {
        self.spent_ms += ms;
    }
}

/// Starts the frame's budget, before its users run.
pub fn sys_start_mobile_compute_frame(mut budget: ResMut<MobileComputeBudget>) {
    budget.spent_ms = 0.0;
}

/// How many of `costs` (ms, in order) run within what is left of the budget after `spent_ms`: as many as fit, and at
/// least one when nothing ran yet this frame. Returns them and the ms spent after them.
pub fn steps_within_budget(costs: impl IntoIterator<Item = f64>, spent_ms: f64) -> (usize, f64) {
    let mut spent = spent_ms;
    let mut count = 0;
    for cost in costs {
        if spent + cost > MOBILE_COMPUTE_FRAME_BUDGET_MS && spent > 0.0 {
            break;
        }
        spent += cost;
        count += 1;
    }
    (count, spent)
}

/// The GPU time per estimated ms, as timed.
#[derive(Clone, Copy, Debug)]
pub struct CostScale(f64);

impl Default for CostScale {
    fn default() -> Self {
        Self(1.0)
    }
}

impl CostScale {
    pub fn get(self) -> f64 {
        self.0
    }

    /// Moves the scale toward the ratio of a timing, `measured` ms for `estimate` ms unscaled.
    pub fn update(&mut self, measured: f64, estimate: f64) {
        let (low, high) = COST_SCALE_BOUNDS;
        self.0 = (self.0 + (measured / estimate - self.0) * COST_SCALE_WEIGHT).clamp(low, high);
    }
}

/// Times a frame's work on the GPU, one frame at a time: two timestamps around it, read back once the GPU wrote them.
pub struct GpuClock {
    queries: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    readback: wgpu::Buffer,
    label: &'static str,
    /// The estimate of the frame timed, until its timestamps are read.
    timing: Option<f64>,
    /// Whether the readback mapped, once it did.
    mapped: Arc<Mutex<Option<bool>>>,
}

impl GpuClock {
    /// A clock where the device has timestamp queries.
    pub fn new(device: &wgpu::Device, label: &'static str) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let buffer = |usage| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size: TIMESTAMP_BYTES, usage, mapped_at_creation: false,
        });
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some(label), ty: wgpu::QueryType::Timestamp, count: TIMESTAMPS,
            }),
            resolved: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readback: buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            label,
            timing: None,
            mapped: Arc::new(Mutex::new(None)),
        })
    }

    /// The GPU time of the frame timed, in ms, once read, with its estimate.
    pub fn read(&mut self, period_ns: f32) -> Option<(f64, f64)> {
        let mapped = self.mapped.lock().take()?;
        let estimate = self.timing.take()?;
        let measured = mapped.then(|| self.readback.slice(..).get_mapped_range().ok().and_then(|bytes| {
            let stamps: &[u64] = bytemuck::cast_slice(&bytes);
            stamps[1].checked_sub(stamps[0]).map(|ticks| ticks as f64 * period_ns as f64 / NANOS_PER_MS)
        })).flatten();
        if mapped {
            self.readback.unmap();
        }
        measured.filter(|&ms| ms > 0.0).map(|ms| (ms, estimate))
    }

    /// Reads a timing if one is in and moves `scale` by it.
    pub fn update_scale(&mut self, period_ns: f32, scale: &mut CostScale) {
        if let Some((measured, estimate)) = self.read(period_ns) {
            scale.update(measured, estimate);
            debug!("{}: {measured:.2} ms on the GPU for {estimate:.2} ms estimated; scale {:.2}.", self.label,
                scale.get());
        }
    }

    /// Whether a frame can be timed now: the last timing was read.
    pub fn idle(&self) -> bool {
        self.timing.is_none()
    }

    /// An empty pass writing timestamp `index`.
    pub fn stamp(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(self.label),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries, beginning_of_pass_write_index: Some(index), end_of_pass_write_index: None,
            }),
        });
    }

    /// Resolves the timestamps, after the work: the readback maps once the GPU wrote them.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..TIMESTAMPS, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, TIMESTAMP_BYTES);
    }

    /// Maps the readback after the submission of a frame timed, whose work was estimated at `estimate` ms unscaled.
    pub fn map(&mut self, estimate: f64) {
        self.timing = Some(estimate);
        let mapped = self.mapped.clone();
        self.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| *mapped.lock() = Some(result.is_ok()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_takes_what_fits_and_at_least_one_step() {
        let half = MOBILE_COMPUTE_FRAME_BUDGET_MS / 2.0;
        assert_eq!(steps_within_budget([half, half, half], 0.0).0, 2);
        assert_eq!(steps_within_budget([half, half], half).0, 1);
        // A step over the budget runs alone in a frame that ran nothing yet, and waits in one that did.
        let over = MOBILE_COMPUTE_FRAME_BUDGET_MS * 3.0;
        assert_eq!(steps_within_budget([over, half], 0.0), (1, over));
        assert_eq!(steps_within_budget([over], half).0, 0);
        // What the in-betweens spent is gone for the upscales.
        assert_eq!(steps_within_budget([half], MOBILE_COMPUTE_FRAME_BUDGET_MS).0, 0);
    }

    #[test]
    fn the_cost_scale_moves_toward_the_timings_within_bounds() {
        let mut scale = CostScale::default();
        scale.update(2.0, 1.0);
        assert!((scale.get() - (1.0 + COST_SCALE_WEIGHT)).abs() < 1e-9);
        for _ in 0..100 {
            scale.update(1000.0, 1.0);
        }
        assert_eq!(scale.get(), COST_SCALE_BOUNDS.1);
    }
}
