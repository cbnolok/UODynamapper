//! The GPU compute pipelines of `uot_gpu_compute` (RIFE, the SPAN upscaler, the BC1/BC7 encoders), compiled once at
//! startup on the render device by [`uot_gpu_compute::warm_up`], on a thread of their own: a cold driver cache takes
//! tens of seconds, which would hold a worker of the background pool (whose jobs are never interrupted) all that time.
//! The RIFE in-betweens take the warmed pipelines when first turned on, and wait for them while they compile, so
//! nothing compiles twice. SPAN and the encoders are not drawn with yet: their pipelines wait here.
//!
//! Each compile is logged as it ends ([`report_compile`]): the place a progress display hooks in.

use std::thread::JoinHandle;
use bevy::prelude::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use uot_gpu_compute::block_encode::MobileBlockEncoder;
use uot_gpu_compute::rife::RifePipelines;
use uot_gpu_compute::upscale::UpscalePipelines;
use uot_gpu_compute::{PipelineCompile, WarmPipelines};

const THREAD_NAME: &str = "gpu compute warm-up";
const MILLIS_PER_SEC: f64 = 1000.0;

/// The pipelines warmed, until their users take them.
struct Warmed {
    rife: Option<RifePipelines>,
    #[expect(dead_code, reason = "the SPAN upscaler is not wired to the mobiles yet; its pipelines wait for it")]
    upscale: UpscalePipelines,
    #[expect(dead_code, reason = "the BC encoders are not wired to the frame atlas yet; their pipelines wait for it")]
    block_encoder: MobileBlockEncoder,
}

#[derive(Default)]
enum WarmUpState {
    #[default]
    NotStarted,
    Compiling(JoinHandle<WarmPipelines>),
    Ready(Warmed),
    /// The warm-up thread did not start or panicked (logged): users compile their own pipelines.
    Failed,
}

/// The render world's warm-up.
#[derive(Resource, Default)]
pub struct ComputeWarmUp {
    state: WarmUpState,
}

/// What a user of the warmed pipelines gets.
pub enum Warm<T> {
    /// Still compiling: wait.
    Compiling,
    Ready(T),
    /// Taken already, or the warm-up failed: compile them.
    Unavailable,
}

impl ComputeWarmUp {
    /// The RIFE pipelines, once.
    pub fn take_rife(&mut self) -> Warm<RifePipelines> {
        self.poll();
        match &mut self.state {
            WarmUpState::NotStarted | WarmUpState::Compiling(_) => Warm::Compiling,
            WarmUpState::Ready(warmed) => warmed.rife.take().map_or(Warm::Unavailable, Warm::Ready),
            WarmUpState::Failed => Warm::Unavailable,
        }
    }

    /// Takes the warm-up's result once its thread ended.
    fn poll(&mut self) {
        let WarmUpState::Compiling(thread) = &self.state else { return };
        if !thread.is_finished() {
            return;
        }
        let WarmUpState::Compiling(thread) = std::mem::take(&mut self.state) else { unreachable!() };
        self.state = match thread.join() {
            Ok(WarmPipelines { rife, upscale, block_encoder }) => {
                WarmUpState::Ready(Warmed { rife: Some(rife), upscale, block_encoder })
            }
            Err(_) => {
                error!("The GPU compute warm-up panicked: the pipelines compile when first used.");
                WarmUpState::Failed
            }
        };
    }
}

/// Logs a compile of the warm-up as it ends.
pub fn report_compile(compile: PipelineCompile) {
    if compile.done {
        debug!("GPU compute warm-up {}/{}: {} in {:.1} ms.", compile.index + 1, compile.total, compile.name,
            compile.elapsed.as_secs_f64() * MILLIS_PER_SEC);
    }
}

/// Starts the warm-up on the render device, once.
pub fn sys_start_compute_warm_up(
    mut warm_up: ResMut<ComputeWarmUp>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let WarmUpState::NotStarted = warm_up.state else { return };
    let device = render_device.wgpu_device().clone();
    let queue: &wgpu::Queue = &render_queue;
    let queue = queue.clone();
    let spawned = std::thread::Builder::new().name(THREAD_NAME.to_string()).spawn(move || {
        let started = std::time::Instant::now();
        let warmed = uot_gpu_compute::warm_up(&device, &queue, report_compile);
        info!("The GPU compute pipelines are compiled, in {:.1} s.", started.elapsed().as_secs_f32());
        warmed
    });
    warm_up.state = match spawned {
        Ok(thread) => WarmUpState::Compiling(thread),
        Err(error) => {
            error!("The GPU compute warm-up did not start: {error}");
            WarmUpState::Failed
        }
    };
}
