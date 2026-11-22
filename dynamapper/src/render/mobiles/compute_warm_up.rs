//! The GPU compute pipelines of `uot_gpu_compute`, compiled ahead of use on the render device, each group on a thread
//! of its own (at most three, one per group, each started once): a cold driver cache takes seconds to tens of
//! seconds, which would hold a worker of the background pool (whose jobs are never interrupted) all that time. RIFE's
//! and the SPAN upscaler's compile at startup. The block encoder (its BC7 UltraFast pipeline, about 6.6 s cold on the
//! Radeon 880M) compiles only while a loaded animation package is BC7, the one atlas whose in-betweens it
//! encodes: from the first frame such a package is loaded, never on the frame path. RADV compiles the threads'
//! pipelines side by side without slowing any, so none waits for another. The RIFE in-betweens take the warmed
//! pipelines when first turned on, and wait for them while they compile, so nothing compiles twice; so does the BC7
//! encoder of the frames the GPU makes for a BC7 frame atlas, and the SPAN upscales (upscales.rs).
//!
//! Each compile shows in the shader-compile overlay while it runs (`render::shader_compiles`), and is logged as it
//! ends.

use std::thread::JoinHandle;
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderQueue};
use bevy::render::Extract;
use uot_gpu_compute::block_encode::{MobileBlockEncoder, BLOCK_ENCODER_COMPILES};
use uot_gpu_compute::rife::{rife_compiles, RifePipelines};
use uot_gpu_compute::upscale::{upscale_compiles, UpscalePipelines};
use uot_gpu_compute::warm_up::CompileReport;
use uot_gpu_compute::{ConvUnits, PipelineCompile};
use crate::render::shader_compiles::{shader_compile_ended, shader_compile_started};
use super::frame_atlas::AtlasFormat;
use super::MobileAnimPackages;

/// The warm-up's threads, one per group of pipelines.
const RIFE_THREAD_NAME: &str = "rife warm-up";
const UPSCALE_THREAD_NAME: &str = "span warm-up";
const BLOCK_ENCODER_THREAD_NAME: &str = "bc7 encoder warm-up";
const MILLIS_PER_SEC: f64 = 1000.0;

/// A compile on a thread of its own, and its result until taken.
enum Compile<T> {
    NotStarted,
    Running(JoinHandle<T>),
    Ready(T),
    /// The thread did not start or panicked (logged): users compile their own pipelines.
    Failed,
}

impl<T: Send + 'static> Compile<T> {
    /// Starts `make` on the thread `name`.
    fn start(&mut self, name: &str, make: impl FnOnce() -> T + Send + 'static) {
        *self = match std::thread::Builder::new().name(name.to_string()).spawn(make) {
            Ok(thread) => Self::Running(thread),
            Err(error) => {
                error!("The {name} did not start: {error}");
                Self::Failed
            }
        };
    }

    /// Takes the result once the thread ended.
    fn poll(&mut self, name: &str) {
        let Self::Running(thread) = self else { return };
        if !thread.is_finished() {
            return;
        }
        let Self::Running(thread) = std::mem::replace(self, Self::Failed) else { unreachable!() };
        match thread.join() {
            Ok(made) => *self = Self::Ready(made),
            Err(_) => error!("The {name} panicked: its pipelines compile when first used."),
        }
    }
}

/// The render world's warm-up.
#[derive(Resource)]
pub struct ComputeWarmUp {
    rife: Compile<Option<RifePipelines>>,
    upscale: Compile<Option<UpscalePipelines>>,
    block_encoder: Compile<Option<MobileBlockEncoder>>,
    /// Whether a loaded animation package is BC7, as extracted.
    block_encoder_wanted: bool,
}

impl Default for ComputeWarmUp {
    fn default() -> Self {
        Self {
            rife: Compile::NotStarted, upscale: Compile::NotStarted, block_encoder: Compile::NotStarted,
            block_encoder_wanted: false,
        }
    }
}

/// What a user of the warmed pipelines gets.
pub enum Warm<T> {
    /// Still compiling: wait.
    Compiling,
    Ready(T),
    /// Taken already, not compiled here, or the warm-up failed: compile them.
    Unavailable,
}

impl ComputeWarmUp {
    /// The RIFE pipelines, once.
    pub fn take_rife(&mut self) -> Warm<RifePipelines> {
        self.rife.poll(RIFE_THREAD_NAME);
        match &mut self.rife {
            Compile::NotStarted | Compile::Running(_) => Warm::Compiling,
            Compile::Ready(pipelines) => pipelines.take().map_or(Warm::Unavailable, Warm::Ready),
            Compile::Failed => Warm::Unavailable,
        }
    }

    /// The SPAN pipelines, once.
    pub fn take_upscale(&mut self) -> Warm<UpscalePipelines> {
        self.upscale.poll(UPSCALE_THREAD_NAME);
        match &mut self.upscale {
            Compile::NotStarted | Compile::Running(_) => Warm::Compiling,
            Compile::Ready(pipelines) => pipelines.take().map_or(Warm::Unavailable, Warm::Ready),
            Compile::Failed => Warm::Unavailable,
        }
    }

    /// The block encoder, once. It waits for its compile while a BC7 package is loaded, and is unavailable without
    /// one.
    pub fn take_block_encoder(&mut self) -> Warm<MobileBlockEncoder> {
        self.block_encoder.poll(BLOCK_ENCODER_THREAD_NAME);
        match &mut self.block_encoder {
            Compile::NotStarted if self.block_encoder_wanted => Warm::Compiling,
            Compile::Running(_) => Warm::Compiling,
            Compile::Ready(encoder) => encoder.take().map_or(Warm::Unavailable, Warm::Ready),
            Compile::NotStarted | Compile::Failed => Warm::Unavailable,
        }
    }
}

/// Shows each compile of a warm-up thread in the shader-compile overlay while it runs, and logs it as it ends. A
/// thread compiles one pipeline at a time.
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

/// Notes whether a loaded animation package is BC7, which wants the block encoder.
pub fn sys_extract_block_encoder_wanted(
    packages: Extract<Option<Res<MobileAnimPackages>>>,
    mut warm_up: ResMut<ComputeWarmUp>,
) {
    warm_up.block_encoder_wanted = packages.as_ref().is_some_and(|packages| packages.0.has_format(AtlasFormat::Bc7));
}

/// Starts RIFE's and SPAN's warm-ups on the render device, once, and the block encoder's, once, as soon as it is
/// wanted.
pub fn sys_start_compute_warm_up(
    mut warm_up: ResMut<ComputeWarmUp>,
    render_adapter: Res<RenderAdapter>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let warm_up = &mut *warm_up;
    let device = render_device.wgpu_device();
    if let Compile::NotStarted = warm_up.rife {
        let device = device.clone();
        let units = ConvUnits::of(&render_adapter, &device);
        warm_up.rife.start(RIFE_THREAD_NAME, move || {
            let started = std::time::Instant::now();
            let pipelines = RifePipelines::new(&device, units,
                &mut CompileReport::new(rife_compiles(units), compile_reporter()));
            info!("The RIFE pipelines are compiled for the {units:?}, in {:.1} s.", started.elapsed().as_secs_f32());
            Some(pipelines)
        });
    }
    if let Compile::NotStarted = warm_up.upscale {
        let device = device.clone();
        let units = ConvUnits::of(&render_adapter, &device);
        warm_up.upscale.start(UPSCALE_THREAD_NAME, move || {
            let started = std::time::Instant::now();
            let pipelines = UpscalePipelines::new(&device, units,
                &mut CompileReport::new(upscale_compiles(units), compile_reporter()));
            info!("The SPAN upscaler pipelines are compiled for the {units:?}, in {:.1} s.",
                started.elapsed().as_secs_f32());
            Some(pipelines)
        });
    }
    if let Compile::NotStarted = warm_up.block_encoder
        && warm_up.block_encoder_wanted {
        let device = device.clone();
        let queue: &wgpu::Queue = &render_queue;
        let queue = queue.clone();
        warm_up.block_encoder.start(BLOCK_ENCODER_THREAD_NAME, move || {
            let started = std::time::Instant::now();
            let encoder = MobileBlockEncoder::compile(&device, &queue,
                &mut CompileReport::new(BLOCK_ENCODER_COMPILES, compile_reporter()));
            info!("The BC7 block encoder is compiled, in {:.1} s.", started.elapsed().as_secs_f32());
            Some(encoder)
        });
    }
}
