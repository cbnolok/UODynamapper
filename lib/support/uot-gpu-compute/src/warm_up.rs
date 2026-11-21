//! Compiles every compute pipeline of the crate ahead of use: RIFE's, the SPAN upscaler's, block_compression's BC1
//! to BC7 encoders and the BC1 punch-through, without uploading weights or running anything. An app calls
//! [`warm_up`] once, off its frame path (the first compile on a cold driver cache takes tens of seconds), and builds
//! its models from what it returns ([`crate::rife::RifeModel::with_pipelines`],
//! [`crate::upscale::UpscaleModel::with_pipelines`]), so nothing compiles again when they are first needed. The
//! driver's own shader cache (Mesa's on disk) makes a later warm-up, in another run, much shorter; this crate keeps no
//! pipeline cache of its own.
//!
//! The callback hears each compile twice, as it starts and as it ends, with its name, its place among the
//! [`WARM_UP_COMPILES`] and, at the end, its time: enough for a progress display.

use std::time::{Duration, Instant};
use crate::block_encode::{MobileBlockEncoder, BLOCK_ENCODER_COMPILES};
use crate::rife::{RifePipelines, RIFE_COMPILES};
use crate::upscale::{UpscalePipelines, UPSCALE_COMPILES};

/// The compiles [`warm_up`] reports.
pub const WARM_UP_COMPILES: usize = RIFE_COMPILES + UPSCALE_COMPILES + BLOCK_ENCODER_COMPILES;

/// A compile starting or ending.
#[derive(Debug, Clone, Copy)]
pub struct PipelineCompile<'a> {
    /// What compiles: a pipeline, or a few of one shader (`"RIFE convolution Stride1 Leaky 64x32"`, `"BC1
    /// punch-through"`).
    pub name: &'a str,
    /// Its place among the compiles of the run (0-based), and how many there are.
    pub index: usize,
    pub total: usize,
    /// `false` as it starts, `true` once it ended.
    pub done: bool,
    /// How long it took; zero as it starts.
    pub elapsed: Duration,
}

/// Tells a callback of each compile in turn.
pub struct CompileReport<'a> {
    callback: Box<dyn FnMut(PipelineCompile) + 'a>,
    next: usize,
    total: usize,
}

impl<'a> CompileReport<'a> {
    /// A report of `total` compiles to `callback`.
    pub fn new(total: usize, callback: impl FnMut(PipelineCompile) + 'a) -> Self {
        Self { callback: Box::new(callback), next: 0, total }
    }

    /// A report nobody hears.
    pub fn silent(total: usize) -> Self {
        Self::new(total, |_| {})
    }

    /// Runs `make`, the compile `name`, telling the callback as it starts and ends.
    pub fn compile<T>(&mut self, name: &str, make: impl FnOnce() -> T) -> T {
        let (index, total) = (self.next, self.total);
        (self.callback)(PipelineCompile { name, index, total, done: false, elapsed: Duration::ZERO });
        let started = Instant::now();
        let made = make();
        (self.callback)(PipelineCompile { name, index, total, done: true, elapsed: started.elapsed() });
        self.next += 1;
        made
    }
}

/// Every pipeline of the crate on one device, compiled.
pub struct WarmPipelines {
    pub rife: RifePipelines,
    pub upscale: UpscalePipelines,
    pub block_encoder: MobileBlockEncoder,
}

/// Compiles every pipeline of the crate on `device`, telling `callback` of each of the [`WARM_UP_COMPILES`] as it
/// starts and ends.
pub fn warm_up(device: &wgpu::Device, queue: &wgpu::Queue, callback: impl FnMut(PipelineCompile)) -> WarmPipelines {
    let mut report = CompileReport::new(WARM_UP_COMPILES, callback);
    let rife = RifePipelines::new(device, &mut report);
    let upscale = UpscalePipelines::new(device, &mut report);
    let block_encoder = MobileBlockEncoder::compile(device, queue, &mut report);
    WarmPipelines { rife, upscale, block_encoder }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Times each compile of a warm-up. Run it with `MESA_SHADER_CACHE_DIR` set to a new folder for a cold driver
    /// cache, and again for a warm one.
    #[test]
    #[ignore = "requires a GPU adapter; run explicitly to time the warm-up"]
    fn time_the_warm_up() {
        let Some((device, queue)) = pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
            }).await.ok()?;
            adapter.request_device(&wgpu::DeviceDescriptor::default()).await.ok()
        }) else {
            println!("skipped: no GPU adapter");
            return;
        };
        let started = Instant::now();
        let mut ended = 0;
        let mut previous = None;
        warm_up(&device, &queue, |compile| {
            assert_eq!(compile.total, WARM_UP_COMPILES);
            if compile.done {
                assert_eq!(previous, Some((compile.index, false)), "{} ends after it starts", compile.name);
                ended += 1;
                println!("{:>3}/{} {:>9.1} ms  {}", compile.index + 1, compile.total,
                    compile.elapsed.as_secs_f64() * 1e3, compile.name);
            }
            previous = Some((compile.index, compile.done));
        });
        assert_eq!(ended, WARM_UP_COMPILES);
        println!("warm-up: {:.1} ms", started.elapsed().as_secs_f64() * 1e3);
    }
}
