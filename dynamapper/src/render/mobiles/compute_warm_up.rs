//! The GPU compute pipelines of `uot_gpu_compute` (RIFE, the SPAN upscaler, the BC1/BC7 encoders), compiled once at
//! startup on the render device by [`uot_gpu_compute::warm_up`], on a thread of their own: a cold driver cache takes
//! tens of seconds, which would hold a worker of the background pool (whose jobs are never interrupted) all that time.
//! The RIFE in-betweens take the warmed pipelines when first turned on, and wait for them while they compile, so
//! nothing compiles twice; so does the BC7 encoder of the in-betweens of a BC7 frame atlas. SPAN is not drawn with
//! yet: its pipelines wait here.
//!
//! Each compile shows in the shader-compile overlay while it runs (`render::shader_compiles`), and is logged as it
//! ends.

use std::thread::JoinHandle;
use bevy::prelude::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use uot_gpu_compute::block_encode::MobileBlockEncoder;
use uot_gpu_compute::rife::RifePipelines;
use uot_gpu_compute::upscale::UpscalePipelines;
use uot_gpu_compute::{PipelineCompile, WarmPipelines};
use crate::render::shader_compiles::{shader_compile_ended, shader_compile_started};

const THREAD_NAME: &str = "gpu compute warm-up";
const MILLIS_PER_SEC: f64 = 1000.0;

/// The pipelines warmed, until their users take them.
struct Warmed {
    rife: Option<RifePipelines>,
    #[expect(dead_code, reason = "the SPAN upscaler is not wired to the mobiles yet; its pipelines wait for it")]
    upscale: UpscalePipelines,
    block_encoder: Option<MobileBlockEncoder>,
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

    /// The block encoder, once.
    pub fn take_block_encoder(&mut self) -> Warm<MobileBlockEncoder> {
        self.poll();
        match &mut self.state {
            WarmUpState::NotStarted | WarmUpState::Compiling(_) => Warm::Compiling,
            WarmUpState::Ready(warmed) => warmed.block_encoder.take().map_or(Warm::Unavailable, Warm::Ready),
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
                WarmUpState::Ready(Warmed { rife: Some(rife), upscale, block_encoder: Some(block_encoder) })
            }
            Err(_) => {
                error!("The GPU compute warm-up panicked: the pipelines compile when first used.");
                WarmUpState::Failed
            }
        };
    }
}

/// Shows each compile of the warm-up in the shader-compile overlay while it runs, and logs it as it ends. The
/// warm-up compiles one pipeline at a time.
fn compile_reporter() -> impl FnMut(PipelineCompile) {
    let mut running = None;
    move |compile| {
        if !compile.done {
            running = Some(shader_compile_started(compile.name));
            return;
        }
        if let Some(id) = running.take() {
            shader_compile_ended(id);
        }
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
        let warmed = uot_gpu_compute::warm_up(&device, &queue, compile_reporter());
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
