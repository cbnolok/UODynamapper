//! The render world's half of the RIFE in-betweens: the network, loaded on the background pool the first time a
//! RIFE mode is on, a workspace per padded frame size, and each frame the queued jobs worked through in order within
//! [`RIFE_FRAME_BUDGET_MS`] of estimated GPU time, dispatch by dispatch, in one command buffer submitted before the
//! frame's draws. A pair of a job packs its two frames from the atlas into the workspace, copies the first into the
//! sequence, encodes them, and for each t runs the blocks, the blend and the store of the in-between into the sequence
//! (atlas.wgsl), through a staging buffer and a copy, since the sRGB atlas takes no storage writes. A workspace stays
//! with the job in the middle of a pair until the pair ends; a job finished goes back to the main world.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::Extract;
use bytemuck::{Pod, Zeroable};
use parking_lot::Mutex;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use super::super::clips::AtlasEntry;
use super::super::frame_atlas::{AtlasRect, MobileFrameAtlas};
use super::super::network_shader_module;
use super::super::rife::{RifeDispatch, RifeModel, RifeWorkspace, RIFE_BLOCKS, RIFE_SIZE_MULTIPLE, RIFE_TIMESTEP_SLOTS};
use super::{InBetweenJob, InBetweenPair, MobileInBetweens};

/// The GPU time the in-betweens may take per frame, estimated (`dispatch_ms`, scaled as timed). A dispatch longer than
/// the budget runs alone in its frame. On the Radeon 880M the timed work stays within 0.1 ms of it.
pub const RIFE_FRAME_BUDGET_MS: f64 = 2.0;
/// A dispatch's estimated cost: a fixed part, and its convolution's multiply-adds. Fitted to rife::tests::benchmark on
/// the Radeon 880M (one t: 1.31 ms at 64x64, 0.079 GMAC, 121 dispatches; 14.97 ms at 576x512, 5.66 GMAC, 97).
const DISPATCH_MS: f64 = 0.009;
const MS_PER_GIGA_MULTIPLY_ADD: f64 = 2.5;
/// An atlas step's estimated cost per million pixels it runs on, besides the fixed part.
const ATLAS_STEP_MS_PER_MEGAPIXEL: f64 = 0.5;
/// Where the GPU has timestamp queries, a frame's work now and then is timed and the estimates scaled to match: each
/// timing moves the scale this much of the way, within these bounds.
const COST_SCALE_WEIGHT: f64 = 0.25;
const COST_SCALE_BOUNDS: (f64, f64) = (0.25, 16.0);
const TIMESTAMPS: u32 = 2;
const TIMESTAMP_BYTES: u64 = TIMESTAMPS as u64 * std::mem::size_of::<u64>() as u64;
const NANOS_PER_MS: f64 = 1e6;
const GIGA: f64 = 1e9;
const MEGA: f64 = 1e6;
/// A workspace no job used for this long is freed (1.6 MiB at 64x64, 89 MiB at 576x512).
const WORKSPACE_IDLE_RELEASE: Duration = Duration::from_secs(20);
const ATLAS_SHADER: &str = include_str!("../../../../assets/shaders/rife/atlas.wgsl");
/// Matches THREADS in atlas.wgsl.
const ATLAS_THREADS: u32 = 64;
const STAGING_TEXEL_BYTES: u32 = 4;

/// `AtlasStep` in atlas.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct AtlasStep {
    width: u32,
    height: u32,
    card_width: u32,
    card_height: u32,
    first_layer: u32,
    first_x: u32,
    first_y: u32,
    first_width: u32,
    first_height: u32,
    first_offset_x: u32,
    first_offset_y: u32,
    next_layer: u32,
    next_x: u32,
    next_y: u32,
    next_width: u32,
    next_height: u32,
    next_offset_x: u32,
    next_offset_y: u32,
    pitch: u32,
    snap_alpha: u32,
}

/// The network and the atlas steps' pipelines.
struct RifeGpu {
    model: RifeModel,
    atlas_layout: wgpu::BindGroupLayout,
    pack: wgpu::ComputePipeline,
    copy_frame: wgpu::ComputePipeline,
    store: wgpu::ComputePipeline,
}

impl RifeGpu {
    fn new(device: &wgpu::Device) -> Result<Self, String> {
        let model = RifeModel::new(device)?;
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None,
            },
        };
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rife atlas step"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<AtlasStep>() as u64),
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
                    },
                },
                storage(2, false),
                storage(3, true),
                storage(4, false),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(&atlas_layout)], immediate_size: 0,
        });
        // SAFETY: atlas.wgsl's one loop runs to a compile-time constant by a positive step.
        let module = unsafe { network_shader_module(device, "rife atlas steps", ATLAS_SHADER.to_string()) };
        let pipeline = |entry_point| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry_point), layout: Some(&layout), module: &module, entry_point: Some(entry_point),
            compilation_options: Default::default(), cache: None,
        });
        Ok(Self { pack: pipeline("pack"), copy_frame: pipeline("copy_frame"), store: pipeline("store"), model,
            atlas_layout })
    }
}

enum GpuState {
    Unloaded,
    Loading(Task<Result<RifeGpu, String>>),
    Ready(Arc<RifeGpu>),
    /// Logged; the clips play plain.
    Failed,
}

/// A workspace and the staging rows of its frame size.
struct Workspace {
    rife: RifeWorkspace,
    staging: wgpu::Buffer,
    last_used: Instant,
}

impl Workspace {
    fn new(gpu: &RifeGpu, device: &wgpu::Device, size: UVec2) -> Result<Self, String> {
        let rife = RifeWorkspace::new(&gpu.model, device, size.x, size.y)?;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rife atlas staging"), size: staging_pitch_bytes(size.x) as u64 * size.y as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        Ok(Self { rife, staging, last_used: Instant::now() })
    }
}

/// Times a frame's work on the GPU, one frame at a time: two timestamps around it, read back once the GPU wrote them.
struct GpuClock {
    queries: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// The estimate of the frame timed, until its timestamps are read.
    timing: Option<f64>,
    /// Whether the readback mapped, once it did.
    mapped: Arc<Mutex<Option<bool>>>,
}

impl GpuClock {
    fn new(device: &wgpu::Device) -> Self {
        let buffer = |label, usage| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size: TIMESTAMP_BYTES, usage, mapped_at_creation: false,
        });
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("mobile in-betweens"), ty: wgpu::QueryType::Timestamp, count: TIMESTAMPS,
            }),
            resolved: buffer("mobile in-betweens timestamps", wgpu::BufferUsages::QUERY_RESOLVE
                | wgpu::BufferUsages::COPY_SRC),
            readback: buffer("mobile in-betweens timestamps read", wgpu::BufferUsages::MAP_READ
                | wgpu::BufferUsages::COPY_DST),
            timing: None,
            mapped: Arc::new(Mutex::new(None)),
        }
    }

    /// The GPU time of the frame timed, in ms, once read, with its estimate.
    fn read(&mut self, period_ns: f32) -> Option<(f64, f64)> {
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

    /// An empty pass writing timestamp `index`.
    fn stamp(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("mobile in-betweens timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries, beginning_of_pass_write_index: Some(index), end_of_pass_write_index: None,
            }),
        });
    }

    /// Resolves the timestamps, after the work: the readback maps once the GPU wrote them.
    fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..TIMESTAMPS, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, TIMESTAMP_BYTES);
    }

    fn map(&mut self, estimate: f64) {
        self.timing = Some(estimate);
        let mapped = self.mapped.clone();
        self.readback.slice(..).map_async(wgpu::MapMode::Read, move |result| *mapped.lock() = Some(result.is_ok()));
    }
}

enum WorkspaceState {
    Creating(Task<Result<Workspace, String>>),
    Ready(Workspace),
    Failed,
}

/// Where a job is: the pair, and the step of the pair next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct JobProgress {
    pair: usize,
    step: usize,
}

/// The render world's in-betweens: the jobs listed this frame, the network, the workspaces and the jobs' progress.
#[derive(Resource)]
pub struct RenderInBetweens {
    image: Option<Handle<Image>>,
    active: bool,
    jobs: Vec<InBetweenJob>,
    finished: Option<Arc<Mutex<Vec<u64>>>>,
    gpu: GpuState,
    workspaces: HashMap<UVec2, WorkspaceState>,
    progress: HashMap<u64, JobProgress>,
    /// The job each workspace is in the middle of a pair of.
    holders: HashMap<UVec2, u64>,
    /// Jobs finished, until the main world stops listing them.
    completed: HashSet<u64>,
    step_uniforms: Option<wgpu::Buffer>,
    /// None without timestamp queries.
    clock: Option<GpuClock>,
    /// The GPU time per estimated ms, as timed.
    cost_scale: f64,
}

impl Default for RenderInBetweens {
    fn default() -> Self {
        Self { image: None, active: false, jobs: Vec::new(), finished: None, gpu: GpuState::Unloaded,
            workspaces: HashMap::new(), progress: HashMap::new(), holders: HashMap::new(), completed: HashSet::new(),
            step_uniforms: None, clock: None, cost_scale: 1.0 }
    }
}

pub fn sys_extract_mobile_in_betweens(
    in_betweens: Extract<Option<Res<MobileInBetweens>>>,
    atlas: Extract<Option<Res<MobileFrameAtlas<AtlasEntry>>>>,
    mut render: ResMut<RenderInBetweens>,
) {
    let (Some(in_betweens), Some(atlas)) = (in_betweens.as_ref(), atlas.as_ref()) else { return };
    render.image = Some(atlas.image().clone());
    render.active = in_betweens.active;
    render.jobs.clone_from(&in_betweens.jobs);
    render.finished = Some(in_betweens.finished.clone());
}

/// A step of a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairStep {
    /// Both frames into the workspace's input.
    Pack,
    /// The first frame into its place in the sequence.
    CopyFirst,
    Encode(usize),
    Block { slot: usize, block: usize, index: usize },
    Merge(usize),
    /// The in-between just blended into its place in the sequence: its index among the pair's in-betweens.
    Store(usize),
}

/// The timestep slot of a pair's in-between `generated` (0-based) at `factor`: its t, (generated + 1) / factor, among
/// the slots' 1/4, 1/2 and 3/4.
fn timestep_slot(generated: usize, factor: u32) -> usize {
    (generated + 1) * (RIFE_TIMESTEP_SLOTS + 1) / factor as usize - 1
}

/// A pair's steps, in order.
fn pair_steps(workspace: &RifeWorkspace, factor: u32) -> Vec<PairStep> {
    let mut steps = vec![PairStep::Pack, PairStep::CopyFirst];
    steps.extend((0..workspace.encode().len()).map(PairStep::Encode));
    for generated in 0..factor as usize - 1 {
        let slot = timestep_slot(generated, factor);
        for block in 0..RIFE_BLOCKS {
            steps.extend((0..workspace.block(slot, block).len()).map(|index| PairStep::Block { slot, block, index }));
        }
        steps.extend((0..workspace.merge().len()).map(PairStep::Merge));
        steps.push(PairStep::Store(generated));
    }
    steps
}

fn rife_dispatch(workspace: &RifeWorkspace, step: PairStep) -> Option<&RifeDispatch> {
    match step {
        PairStep::Encode(index) => workspace.encode().get(index),
        PairStep::Block { slot, block, index } => workspace.block(slot, block).get(index),
        PairStep::Merge(index) => workspace.merge().get(index),
        PairStep::Pack | PairStep::CopyFirst | PairStep::Store(_) => None,
    }
}

/// A step's estimated GPU time, in ms.
fn step_ms(workspace: &RifeWorkspace, step: PairStep, pair: &InBetweenPair) -> f64 {
    let pixels = match step {
        PairStep::Pack => workspace.size().0 * workspace.size().1,
        PairStep::CopyFirst => pair.first.width * pair.first.height,
        PairStep::Store(_) => pair.card.x * pair.card.y,
        _ => 0,
    };
    let multiply_adds = rife_dispatch(workspace, step).map_or(0, RifeDispatch::multiply_adds);
    dispatch_ms(multiply_adds, pixels)
}

fn dispatch_ms(multiply_adds: u64, atlas_pixels: u32) -> f64 {
    DISPATCH_MS + multiply_adds as f64 / GIGA * MS_PER_GIGA_MULTIPLY_ADD
        + atlas_pixels as f64 / MEGA * ATLAS_STEP_MS_PER_MEGAPIXEL
}

/// How many of `costs` (ms, in order) run within what is left of the budget: as many as fit, and at least one when
/// nothing ran yet this frame.
fn steps_within_budget(costs: impl IntoIterator<Item = f64>, spent_ms: f64) -> (usize, f64) {
    let mut spent = spent_ms;
    let mut count = 0;
    for cost in costs {
        if spent + cost > RIFE_FRAME_BUDGET_MS && spent > 0.0 {
            break;
        }
        spent += cost;
        count += 1;
    }
    (count, spent)
}

/// The bytes of a staging row of a frame `width` wide: a copy into a texture takes rows of a multiple of 256 bytes.
fn staging_pitch_bytes(width: u32) -> u32 {
    (width * STAGING_TEXEL_BYTES).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

fn padded_size(size: UVec2) -> UVec2 {
    UVec2::new(size.x.max(1).next_multiple_of(RIFE_SIZE_MULTIPLE), size.y.max(1).next_multiple_of(RIFE_SIZE_MULTIPLE))
}

/// What a frame records, in order.
enum Recorded {
    Rife(RifeDispatch),
    /// An atlas step of a workspace, its uniform the `step`th of the frame.
    Atlas { workspace: UVec2, pipeline: wgpu::ComputePipeline, step: usize, workgroups: u32 },
    /// A workspace's staging rows into the atlas.
    Copy { workspace: UVec2, target: AtlasRect },
}

/// Runs the queued jobs within the frame's budget.
pub fn sys_render_mobile_in_betweens(
    mut state: ResMut<RenderInBetweens>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let state = &mut *state;
    let device = render_device.wgpu_device();
    if let GpuState::Unloaded = state.gpu && state.active {
        let device = device.clone();
        state.gpu = GpuState::Loading(background_pool().spawn(Priority::Visible, "rife network load",
            move || RifeGpu::new(&device)));
    }
    if let GpuState::Loading(task) = &mut state.gpu {
        state.gpu = match task.poll() {
            TaskPoll::Pending => return,
            TaskPoll::Ready(Ok(gpu)) => {
                info!("The RIFE network is loaded: {:.1} MB of weights.", gpu.model.weight_bytes() as f64 / MEGA);
                GpuState::Ready(Arc::new(gpu))
            }
            TaskPoll::Ready(Err(error)) => {
                error!("The RIFE network did not load, so the mobiles' clips play without its in-betweens: {error}");
                GpuState::Failed
            }
            TaskPoll::Failed => GpuState::Failed,
        };
    }
    let GpuState::Ready(gpu) = &state.gpu else { return };
    let gpu = gpu.clone();
    if state.clock.is_none() && device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        state.clock = Some(GpuClock::new(device));
    }
    let period_ns = render_queue.get_timestamp_period();
    if let Some((measured, estimate)) = state.clock.as_mut().and_then(|clock| clock.read(period_ns)) {
        let (low, high) = COST_SCALE_BOUNDS;
        state.cost_scale += (measured / estimate - state.cost_scale) * COST_SCALE_WEIGHT;
        state.cost_scale = state.cost_scale.clamp(low, high);
        debug!("Mobile in-betweens: {measured:.2} ms on the GPU for {estimate:.2} ms estimated; scale {:.2}.",
            state.cost_scale);
    }
    let cost_scale = state.cost_scale;

    // Forget what the jobs no longer listed left, and the workspaces idle for long.
    let listed = state.jobs.iter().map(|job| job.id).collect::<HashSet<_>>();
    state.progress.retain(|id, _| listed.contains(id));
    state.completed.retain(|id| listed.contains(id));
    state.holders.retain(|_, id| listed.contains(id));
    let holders = &state.holders;
    state.workspaces.retain(|size, workspace| match workspace {
        WorkspaceState::Ready(workspace) =>
            holders.contains_key(size) || workspace.last_used.elapsed() < WORKSPACE_IDLE_RELEASE,
        WorkspaceState::Creating(_) => true,
        WorkspaceState::Failed => listed.is_empty(),
    });

    let Some(atlas) = state.image.as_ref().and_then(|image| gpu_images.get(image)) else { return };
    let mut recorded = Vec::new();
    let mut steps = Vec::new();
    let mut spent = 0.0;
    let mut finished = Vec::new();
    let RenderInBetweens { jobs, workspaces, progress, holders, completed, .. } = &mut *state;
    for job in jobs.iter() {
        if completed.contains(&job.id) || spent >= RIFE_FRAME_BUDGET_MS {
            continue;
        }
        let size = padded_size(job.size);
        if holders.get(&size).is_some_and(|holder| *holder != job.id) {
            continue;
        }
        let workspace = workspaces.entry(size).or_insert_with(|| {
            let (gpu, device) = (gpu.clone(), device.clone());
            WorkspaceState::Creating(background_pool().spawn(Priority::Visible, "rife workspace",
                move || Workspace::new(&gpu, &device, size)))
        });
        if let WorkspaceState::Creating(task) = workspace {
            *workspace = match task.poll() {
                TaskPoll::Pending => continue,
                TaskPoll::Ready(Ok(ready)) => WorkspaceState::Ready(ready),
                TaskPoll::Ready(Err(error)) => {
                    error!("No RIFE workspace of {}x{}: {error}", size.x, size.y);
                    WorkspaceState::Failed
                }
                TaskPoll::Failed => WorkspaceState::Failed,
            };
        }
        let WorkspaceState::Ready(workspace) = workspace else { continue };
        workspace.last_used = Instant::now();
        let at = progress.entry(job.id).or_default();
        while at.pair < job.pairs.len() && spent < RIFE_FRAME_BUDGET_MS {
            let pair = &job.pairs[at.pair];
            let plan = pair_steps(&workspace.rife, job.factor);
            let (count, now_spent) = steps_within_budget(
                plan[at.step..].iter().map(|&step| step_ms(&workspace.rife, step, pair) * cost_scale), spent);
            spent = now_spent;
            for &step in &plan[at.step..at.step + count] {
                record_step(&mut recorded, &mut steps, &gpu, workspace, size, job, pair, step);
            }
            at.step += count;
            if at.step < plan.len() {
                if at.step > 0 {
                    holders.insert(size, job.id);
                }
                break;
            }
            holders.remove(&size);
            *at = JobProgress { pair: at.pair + 1, step: 0 };
        }
        if at.pair == job.pairs.len() {
            completed.insert(job.id);
            finished.push(job.id);
        }
    }
    if recorded.is_empty() {
        return;
    }

    let stride = (std::mem::size_of::<AtlasStep>() as u64)
        .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
    let needed = stride * steps.len().max(1) as u64;
    if state.step_uniforms.as_ref().is_none_or(|buffer| buffer.size() < needed) {
        state.step_uniforms = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rife atlas steps"), size: needed.next_power_of_two(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        }));
    }
    let uniforms = state.step_uniforms.as_ref().expect("made above");
    let mut bytes = vec![0u8; needed as usize];
    for (index, step) in steps.iter().enumerate() {
        let at = index * stride as usize;
        bytes[at..at + std::mem::size_of::<AtlasStep>()].copy_from_slice(bytemuck::bytes_of(step));
    }
    render_queue.write_buffer(uniforms, 0, &bytes);
    let bind_groups = state.workspaces.iter().filter_map(|(size, workspace)| {
        let WorkspaceState::Ready(workspace) = workspace else { return None };
        Some((*size, device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife atlas step"), layout: &gpu.atlas_layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniforms, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<AtlasStep>() as u64),
                }) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.texture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: workspace.rife.input().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: workspace.rife.merged().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: workspace.staging.as_entire_binding() },
            ],
        })))
    }).collect::<HashMap<_, _>>();

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mobile in-betweens") });
    let timed = state.clock.as_ref().filter(|clock| clock.timing.is_none());
    if let Some(clock) = timed {
        clock.stamp(&mut encoder, 0);
    }
    let mut next = recorded.iter().peekable();
    while next.peek().is_some() {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mobile in-betweens"), timestamp_writes: None,
            });
            while let Some(item) = next.next_if(|item| !matches!(item, Recorded::Copy { .. })) {
                match item {
                    Recorded::Rife(dispatch) => dispatch.record(&mut pass),
                    Recorded::Atlas { workspace, pipeline, step, workgroups } => {
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &bind_groups[workspace], &[(*step as u64 * stride) as u32]);
                        pass.dispatch_workgroups(*workgroups, 1, 1);
                    }
                    Recorded::Copy { .. } => unreachable!(),
                }
            }
        }
        if let Some(Recorded::Copy { workspace, target }) = next.next() {
            let WorkspaceState::Ready(workspace) = &state.workspaces[workspace] else { unreachable!() };
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &workspace.staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0, bytes_per_row: Some(staging_pitch_bytes(target.width)), rows_per_image: None,
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas.texture, mip_level: 0,
                    origin: wgpu::Origin3d { x: target.x, y: target.y, z: target.layer },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d { width: target.width, height: target.height, depth_or_array_layers: 1 },
            );
        }
    }
    if let Some(clock) = timed {
        clock.stamp(&mut encoder, 1);
        clock.resolve(&mut encoder);
    }
    let timed = timed.is_some();
    render_queue.submit([encoder.finish()]);
    if timed && let Some(clock) = state.clock.as_mut() {
        clock.map(spent / cost_scale);
    }
    if let Some(shared) = &state.finished
        && !finished.is_empty() {
        shared.lock().extend(finished);
    }
}

/// Records `step` of `pair`: a network dispatch, or an atlas step and, for a copy or a store, its copy into the
/// atlas.
#[allow(clippy::too_many_arguments)]
fn record_step(recorded: &mut Vec<Recorded>, steps: &mut Vec<AtlasStep>, gpu: &RifeGpu, workspace: &Workspace,
    size: UVec2, job: &InBetweenJob, pair: &InBetweenPair, step: PairStep) {
    if let Some(dispatch) = rife_dispatch(&workspace.rife, step) {
        recorded.push(Recorded::Rife(dispatch.clone()));
        return;
    }
    let frame =
        |rect: AtlasRect, offset: UVec2| (rect.layer, rect.x, rect.y, rect.width, rect.height, offset.x, offset.y);
    let (first_layer, first_x, first_y, first_width, first_height, first_offset_x, first_offset_y) =
        frame(pair.first, pair.first_offset);
    let (next_layer, next_x, next_y, next_width, next_height, next_offset_x, next_offset_y) =
        frame(pair.next, pair.next_offset);
    let (target, card, pipeline) = match step {
        PairStep::Pack => (None, pair.card, &gpu.pack),
        PairStep::CopyFirst =>
            (Some(pair.targets[0]), UVec2::new(pair.first.width, pair.first.height), &gpu.copy_frame),
        PairStep::Store(generated) => (Some(pair.targets[generated + 1]), pair.card, &gpu.store),
        _ => unreachable!("network steps are dispatches"),
    };
    let mut atlas_step = AtlasStep {
        width: size.x, height: size.y, card_width: card.x, card_height: card.y,
        first_layer, first_x, first_y, first_width, first_height, first_offset_x, first_offset_y,
        next_layer, next_x, next_y, next_width, next_height, next_offset_x, next_offset_y,
        pitch: staging_pitch_bytes(card.x) / STAGING_TEXEL_BYTES, snap_alpha: job.snap_alpha as u32,
    };
    if step == PairStep::CopyFirst {
        // The frame alone, at the card's corner.
        (atlas_step.first_offset_x, atlas_step.first_offset_y) = (0, 0);
    }
    let threads = if step == PairStep::Pack { size.x * size.y } else { card.x * card.y };
    recorded.push(Recorded::Atlas { workspace: size, pipeline: pipeline.clone(), step: steps.len(),
        workgroups: threads.div_ceil(ATLAS_THREADS) });
    steps.push(atlas_step);
    if let Some(target) = target {
        recorded.push(Recorded::Copy { workspace: size, target });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_in_between_takes_the_slot_of_its_t() {
        // x2: the middle; x4: the quarters, as the slots start (rife.rs).
        assert_eq!(timestep_slot(0, 2), 1);
        assert_eq!((0..3).map(|generated| timestep_slot(generated, 4)).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn the_budget_takes_what_fits_and_at_least_one_step() {
        let half = RIFE_FRAME_BUDGET_MS / 2.0;
        assert_eq!(steps_within_budget([half, half, half], 0.0).0, 2);
        assert_eq!(steps_within_budget([half, half], half).0, 1);
        // A step over the budget runs alone in a frame that ran nothing yet, and waits in one that did.
        let over = RIFE_FRAME_BUDGET_MS * 3.0;
        assert_eq!(steps_within_budget([over, half], 0.0), (1, over));
        assert_eq!(steps_within_budget([over], half).0, 0);
    }

    #[test]
    fn staging_rows_and_workspaces_are_aligned() {
        assert_eq!(staging_pitch_bytes(1), wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        assert_eq!(staging_pitch_bytes(65), 2 * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        assert_eq!(padded_size(UVec2::new(65, 64)), UVec2::new(128, 64));
    }

    #[test]
    fn the_shader_reads_an_atlas_step_as_wgsl_lays_it_out() {
        // Twenty u32.
        const WGSL_ATLAS_STEP_SIZE: usize = 80;
        assert_eq!(std::mem::size_of::<AtlasStep>(), WGSL_ATLAS_STEP_SIZE);
    }
}
