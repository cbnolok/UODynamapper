//! The render world's half of the RIFE in-betweens: the network, loaded on the background pool the first time a
//! RIFE mode is on, a workspace per padded frame size, and each frame the queued jobs worked through in order within
//! the mobiles' compute budget of estimated GPU time (compute_budget.rs; the SPAN upscales take what is left of it),
//! dispatch by dispatch, in one command buffer submitted before the frame's draws. A pair of a job packs its two frames
//! from the atlas into the workspace, copies the first into the sequence, encodes them, and for each t runs the blocks,
//! the blend and the store of the in-between into the sequence (uot-gpu-compute's rife::frames), through a staging
//! buffer and a copy, since the sRGB atlas takes no storage writes. A workspace stays with the job in the middle of a
//! pair until the pair ends; a job finished goes back to the main world.
//!
//! A clip of the BC7 atlas (frame_atlas.rs) has its in-betweens in that atlas: its first frames are copied as they are,
//! block by block, through a bounce buffer (a copy within one texture layer is not allowed), and each in-between goes
//! from the staging rows to rows of its own in the scratch texture of the BC7 encoding it shares with the upscales
//! (bc7_encoding.rs), cleared to transparent around it. After the frame's steps the encoder encodes them all in one
//! pass, its one `compress` of the submission, and their blocks are copied into the atlas. A frame takes the BC7 steps that fit
//! the scratch rows and the bounce buffer, and leaves the rest to the next.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::Extract;
use parking_lot::Mutex;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use crate::render::shader_compiles::begin_shader_compile;
use super::super::clips::AtlasEntry;
use super::super::bc7_encoding::{bc7_encode_ms, scratch_blocks_offset, Bc7Encoding, MobileBc7Encoder, BC7_BOUNCE_BYTES,
    BC7_SCRATCH_SIDE, MOBILE_BLOCK_FORMAT};
use super::super::compute_budget::{steps_within_budget, CostScale, GpuClock, MobileComputeBudget,
    MOBILE_COMPUTE_FRAME_BUDGET_MS};
use super::super::frame_atlas::{AtlasFormat, AtlasRect, MobileFrameAtlas};
use uot_gpu_compute::block_encode::{record_copy_to_layer, BLOCK_SIDE};
use uot_gpu_compute::rife::frames::{rife_frame_step_stride, rife_output_bytes_per_row, CardFrame, RifeFrameStep};
use uot_gpu_compute::rife::{
    RifeDispatch, RifeModel, RifePipelines, RifeWorkspace, RIFE_BLOCKS, RIFE_SIZE_MULTIPLE, RIFE_TIMESTEP_SLOTS,
};
use uot_gpu_compute::SourceRect;
use super::super::compute_warm_up::{ComputeWarmUp, Warm};
use super::{InBetweenJob, InBetweenPair, MobileInBetweens};

/// A dispatch's estimated cost: a fixed part, and its convolution's multiply-adds. Fitted to uot-gpu-compute's
/// rife::tests::benchmark on the Radeon 880M (one t: 1.31 ms at 64x64, 0.079 GMAC, 121 dispatches; 14.97 ms at
/// 576x512, 5.66 GMAC, 97).
const DISPATCH_MS: f64 = 0.009;
const MS_PER_GIGA_MULTIPLY_ADD: f64 = 2.5;
/// An atlas step's estimated cost per million pixels it runs on, besides the fixed part.
const ATLAS_STEP_MS_PER_MEGAPIXEL: f64 = 0.5;
/// The bytes of a BC7 block.
const BC7_BLOCK_BYTES: u32 = 16;
const GIGA: f64 = 1e9;
const MEGA: f64 = 1e6;
/// A workspace no job used for this long is freed (1.6 MiB at 64x64, 89 MiB at 576x512).
const WORKSPACE_IDLE_RELEASE: Duration = Duration::from_secs(20);

/// The network on `pipelines` warmed ahead, or on pipelines compiled now.
fn load_network(device: &wgpu::Device, pipelines: Option<RifePipelines>) -> Result<RifeModel, String> {
    match pipelines {
        Some(pipelines) => RifeModel::with_pipelines(device, pipelines),
        None => {
            let _compile = begin_shader_compile("RIFE in-betweens: flow network");
            RifeModel::new(device)
        }
    }
}

enum GpuState {
    Unloaded,
    Loading(Task<Result<RifeModel, String>>),
    Ready(Arc<RifeModel>),
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
    fn new(gpu: &RifeModel, device: &wgpu::Device, size: UVec2) -> Result<Self, String> {
        let rife = RifeWorkspace::new(gpu, device, size.x, size.y)?;
        // COPY_DST to be cleared before an in-between of the BC7 atlas.
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rife atlas staging"), size: rife_output_bytes_per_row(size.x) as u64 * size.y as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self { rife, staging, last_used: Instant::now() })
    }
}

/// What a frame's BC7 steps took of the scratch rows and the bounce buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Bc7Room {
    scratch_rows: u32,
    bounce_bytes: u64,
}

impl Bc7Room {
    /// The room taken before `step` of `pair`, if what it needs is left (its in-between's padded rows, or its first
    /// frame's blocks), and takes that.
    fn take(&mut self, step: PairStep, pair: &InBetweenPair) -> Option<Bc7Room> {
        let at = *self;
        match step {
            PairStep::CopyFirst => {
                let (pitch, rows) = block_copy_layout(pair.first);
                let bytes = u64::from(pitch) * u64::from(rows);
                (at.bounce_bytes + bytes <= BC7_BOUNCE_BYTES).then(|| self.bounce_bytes += bytes)?;
            }
            PairStep::Store(generated) => {
                let rows = pair.targets[generated + 1].padded_size().1;
                (at.scratch_rows + rows <= BC7_SCRATCH_SIDE).then(|| self.scratch_rows += rows)?;
            }
            _ => {}
        }
        Some(at)
    }
}

/// The rows of `rect`'s blocks in a buffer: bytes per row (a multiple of 256, for a copy of several rows) and rows.
fn block_copy_layout(rect: AtlasRect) -> (u32, u32) {
    let (width, height) = rect.padded_size();
    ((width / BLOCK_SIDE * BC7_BLOCK_BYTES).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT), height / BLOCK_SIDE)
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
    images: Vec<(AtlasFormat, Handle<Image>)>,
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
    cost_scale: CostScale,
}

impl Default for RenderInBetweens {
    fn default() -> Self {
        Self { images: Vec::new(), active: false, jobs: Vec::new(), finished: None, gpu: GpuState::Unloaded,
            workspaces: HashMap::new(), progress: HashMap::new(), holders: HashMap::new(), completed: HashSet::new(),
            step_uniforms: None, clock: None, cost_scale: CostScale::default() }
    }
}

pub fn sys_extract_mobile_in_betweens(
    in_betweens: Extract<Option<Res<MobileInBetweens>>>,
    atlas: Extract<Option<Res<MobileFrameAtlas<AtlasEntry>>>>,
    mut render: ResMut<RenderInBetweens>,
) {
    let (Some(in_betweens), Some(atlas)) = (in_betweens.as_ref(), atlas.as_ref()) else { return };
    if render.images.len() != atlas.images().count() {
        render.images = atlas.images().map(|(format, image)| (format, image.clone())).collect();
    }
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

/// A step's estimated GPU time, in ms; with its BC7 encode, for an in-between of the BC7 atlas.
fn step_ms(workspace: &RifeWorkspace, step: PairStep, pair: &InBetweenPair) -> f64 {
    let pixels = match step {
        PairStep::Pack => workspace.size().0 * workspace.size().1,
        PairStep::CopyFirst => pair.first.width * pair.first.height,
        PairStep::Store(_) => pair.card.x * pair.card.y,
        _ => 0,
    };
    let multiply_adds = rife_dispatch(workspace, step).map_or(0, RifeDispatch::multiply_adds);
    let encode = match step {
        PairStep::Store(generated) if pair.targets[generated + 1].format == AtlasFormat::Bc7 => {
            let (width, height) = pair.targets[generated + 1].padded_size();
            bc7_encode_ms(width * height)
        }
        _ => 0.0,
    };
    dispatch_ms(multiply_adds, pixels) + encode
}

fn dispatch_ms(multiply_adds: u64, atlas_pixels: u32) -> f64 {
    DISPATCH_MS + multiply_adds as f64 / GIGA * MS_PER_GIGA_MULTIPLY_ADD
        + atlas_pixels as f64 / MEGA * ATLAS_STEP_MS_PER_MEGAPIXEL
}

fn padded_size(size: UVec2) -> UVec2 {
    UVec2::new(size.x.max(1).next_multiple_of(RIFE_SIZE_MULTIPLE), size.y.max(1).next_multiple_of(RIFE_SIZE_MULTIPLE))
}

/// What a frame records, in order.
enum Recorded {
    Rife(RifeDispatch),
    /// A frame step of a workspace on the atlas of `format`, its uniform the `uniform`th of the frame.
    Atlas { workspace: UVec2, format: AtlasFormat, step: RifeFrameStep, uniform: usize },
    /// A workspace's staging rows into the RGBA8 atlas.
    Copy { workspace: UVec2, target: AtlasRect },
    /// A workspace's staging rows cleared to transparent, as far as an in-between padded to blocks reaches.
    ClearStaging { workspace: UVec2, target: AtlasRect },
    /// A workspace's staging rows into the scratch texture at row `y`, for their BC7 encode into `target`.
    Scratch { workspace: UVec2, target: AtlasRect, y: u32 },
    /// A first frame's blocks into its place in the sequence, through the bounce buffer from byte `offset`.
    Blocks { source: AtlasRect, target: AtlasRect, offset: u64 },
}

impl Recorded {
    /// Whether it records in a compute pass, rather than as a command between passes.
    fn in_pass(&self) -> bool {
        matches!(self, Self::Rife(_) | Self::Atlas { .. })
    }
}

/// Runs the queued jobs within the frame's budget.
pub fn sys_render_mobile_in_betweens(
    mut state: ResMut<RenderInBetweens>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut warm_up: ResMut<ComputeWarmUp>,
    mut budget: ResMut<MobileComputeBudget>,
    mut bc7: ResMut<MobileBc7Encoder>,
) {
    let state = &mut *state;
    let device = render_device.wgpu_device();
    if let GpuState::Unloaded = state.gpu && state.active {
        let pipelines = match warm_up.take_rife() {
            Warm::Compiling => return,
            Warm::Ready(pipelines) => Some(pipelines),
            Warm::Unavailable => None,
        };
        let device = device.clone();
        state.gpu = GpuState::Loading(background_pool().spawn(Priority::Visible, "rife network load",
            move || load_network(&device, pipelines)));
    }
    if let GpuState::Loading(task) = &mut state.gpu {
        state.gpu = match task.poll() {
            TaskPoll::Pending => return,
            TaskPoll::Ready(Ok(gpu)) => {
                info!("The RIFE network is loaded: {:.1} MB of weights.", gpu.weight_bytes() as f64 / MEGA);
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
    if state.clock.is_none() {
        state.clock = GpuClock::new(device, "mobile in-betweens");
    }
    if let Some(clock) = state.clock.as_mut() {
        clock.update_scale(render_queue.get_timestamp_period(), &mut state.cost_scale);
    }
    let cost_scale = state.cost_scale.get();
    let bc7_listed = state.active && state.jobs.iter().any(|job| job_format(job) == AtlasFormat::Bc7);
    bc7.poll(bc7_listed, &mut warm_up, device, &render_queue);

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

    let atlases = state.images.iter()
        .filter_map(|(format, image)| Some((*format, gpu_images.get(image)?)))
        .collect::<Vec<_>>();
    let atlas_of = |format| atlases.iter().find(|(atlas, _)| *atlas == format).map(|(_, image)| *image);
    let mut recorded = Vec::new();
    let mut steps = Vec::new();
    let mut spent = budget.spent_ms();
    let started = spent;
    let mut finished = Vec::new();
    let mut bc7_room = Bc7Room::default();
    let mut bc7_full = false;
    let RenderInBetweens { jobs, workspaces, progress, holders, completed, .. } = &mut *state;
    let bc7_ready = bc7.ready().is_some();
    for job in jobs.iter() {
        if completed.contains(&job.id) || spent >= MOBILE_COMPUTE_FRAME_BUDGET_MS || bc7_full {
            continue;
        }
        let format = job_format(job);
        if atlas_of(format).is_none() || (format == AtlasFormat::Bc7 && !bc7_ready) {
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
        while at.pair < job.pairs.len() && spent < MOBILE_COMPUTE_FRAME_BUDGET_MS && !bc7_full {
            let pair = &job.pairs[at.pair];
            let plan = pair_steps(&workspace.rife, job.factor);
            let costs = plan[at.step..].iter().map(|&step| step_ms(&workspace.rife, step, pair) * cost_scale)
                .collect::<Vec<_>>();
            let (mut count, _) = steps_within_budget(costs.iter().copied(), spent);
            for (index, &step) in plan[at.step..at.step + count].iter().enumerate() {
                let room = match format {
                    AtlasFormat::Rgba8 => Some(Bc7Room::default()),
                    AtlasFormat::Bc7 => bc7_room.take(step, pair),
                };
                // Out of scratch rows or bounce bytes: the rest waits for the next frame.
                let Some(room) = room else {
                    count = index;
                    bc7_full = true;
                    break;
                };
                record_step(&mut recorded, &mut steps, workspace, size, format, job, pair, step, room);
            }
            spent += costs[..count].iter().sum::<f64>();
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
    budget.spend(spent - started);
    if recorded.is_empty() {
        return;
    }

    let stride = rife_frame_step_stride(device);
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
        bytes[at..at + step.uniform().len()].copy_from_slice(step.uniform());
    }
    render_queue.write_buffer(uniforms, 0, &bytes);
    let used = recorded.iter().filter_map(|item| match item {
        Recorded::Atlas { workspace, format, .. } => Some((*workspace, *format)),
        _ => None,
    }).collect::<HashSet<_>>();
    let bind_groups = used.into_iter().map(|(size, format)| {
        let WorkspaceState::Ready(workspace) = &state.workspaces[&size] else { unreachable!("a step ran on it") };
        let atlas = atlas_of(format).expect("a step ran on it");
        ((size, format),
            gpu.frame_bind_group(device, &workspace.rife, &workspace.staging, uniforms, &atlas.texture_view))
    }).collect::<HashMap<_, _>>();

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mobile in-betweens") });
    let timed = state.clock.as_ref().filter(|clock| clock.idle());
    if let Some(clock) = timed {
        clock.stamp(&mut encoder, 0);
    }
    let mut next = recorded.iter().peekable();
    let mut encodes = Vec::new();
    while next.peek().is_some() {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mobile in-betweens"), timestamp_writes: None,
            });
            while let Some(item) = next.next_if(|item| item.in_pass()) {
                match item {
                    Recorded::Rife(dispatch) => dispatch.record(&mut pass),
                    Recorded::Atlas { workspace, format, step, uniform } => step.record(&gpu, &mut pass,
                        &bind_groups[&(*workspace, *format)], (*uniform as u64 * stride) as u32),
                    _ => unreachable!("taken while in a pass"),
                }
            }
        }
        let staging = |workspace| {
            let WorkspaceState::Ready(workspace) = &state.workspaces[workspace] else { unreachable!("a step ran on it") };
            &workspace.staging
        };
        let bc7_encoding = || -> &Bc7Encoding { bc7.ready().expect("a BC7 step ran") };
        match next.next() {
            None => {}
            Some(Recorded::Copy { workspace, target }) => {
                let atlas = atlas_of(AtlasFormat::Rgba8).expect("a step ran on it");
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: staging(workspace),
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0, bytes_per_row: Some(rife_output_bytes_per_row(target.width)),
                            rows_per_image: None,
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
            Some(Recorded::ClearStaging { workspace, target }) => {
                let (_, height) = target.padded_size();
                encoder.clear_buffer(staging(workspace), 0,
                    Some(u64::from(rife_output_bytes_per_row(target.width)) * u64::from(height)));
            }
            Some(Recorded::Scratch { workspace, target, y }) => {
                let (width, height) = target.padded_size();
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: staging(workspace),
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0, bytes_per_row: Some(rife_output_bytes_per_row(target.width)),
                            rows_per_image: None,
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &bc7_encoding().scratch, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: *y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                );
                encodes.push((*target, *y));
            }
            Some(Recorded::Blocks { source, target, offset }) => {
                let atlas = atlas_of(AtlasFormat::Bc7).expect("a step ran on it");
                let bounce = &bc7_encoding().bounce;
                let (bytes_per_row, _) = block_copy_layout(*source);
                let (width, height) = source.padded_size();
                let layout =
                    wgpu::TexelCopyBufferLayout { offset: *offset, bytes_per_row: Some(bytes_per_row), rows_per_image: None };
                let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
                let at = |rect: &AtlasRect| wgpu::TexelCopyTextureInfo {
                    texture: &atlas.texture, mip_level: 0, origin: wgpu::Origin3d { x: rect.x, y: rect.y, z: rect.layer },
                    aspect: wgpu::TextureAspect::All,
                };
                encoder.copy_texture_to_buffer(at(source), wgpu::TexelCopyBufferInfo { buffer: bounce, layout }, extent);
                encoder.copy_buffer_to_texture(wgpu::TexelCopyBufferInfo { buffer: bounce, layout }, at(target), extent);
            }
            Some(Recorded::Rife(_) | Recorded::Atlas { .. }) => unreachable!("recorded in a pass"),
        }
    }
    if !encodes.is_empty() {
        // The in-betweens of the BC7 atlas, encoded together: the encoder's one compress of the submission.
        let atlas = atlas_of(AtlasFormat::Bc7).expect("a step ran on it");
        let Bc7Encoding { encoder: block_encoder, scratch_view, blocks, .. } =
            bc7.ready_mut().expect("a BC7 step ran");
        for &(target, y) in &encodes {
            block_encoder.add_frame_at(MOBILE_BLOCK_FORMAT, scratch_view, y, target.width, target.height, blocks,
                scratch_blocks_offset(y));
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mobile in-betweens BC7"), timestamp_writes: None,
            });
            block_encoder.record(&mut pass);
        }
        for &(target, y) in &encodes {
            record_copy_to_layer(&mut encoder, MOBILE_BLOCK_FORMAT, blocks, scratch_blocks_offset(y), target.width,
                target.height, &atlas.texture, target.layer, target.x, target.y);
        }
    }
    if let Some(clock) = timed {
        clock.stamp(&mut encoder, 1);
        clock.resolve(&mut encoder);
    }
    let timed = timed.is_some();
    render_queue.submit([encoder.finish()]);
    if timed && let Some(clock) = state.clock.as_mut() {
        clock.map((spent - started) / cost_scale);
    }
    if let Some(shared) = &state.finished
        && !finished.is_empty() {
        shared.lock().extend(finished);
    }
}

/// The atlas of a job's frames, and of its in-betweens.
fn job_format(job: &InBetweenJob) -> AtlasFormat {
    job.pairs.first().map_or(AtlasFormat::Rgba8, |pair| pair.first.format)
}

/// Records `step` of `pair`: a network dispatch, or an atlas step and, for a copy or a store, its copy into the
/// atlas of `format`. On the BC7 atlas a first frame's blocks are copied as they are, and an in-between goes to the
/// scratch rows `room` names, to be encoded.
#[allow(clippy::too_many_arguments)]
fn record_step(recorded: &mut Vec<Recorded>, steps: &mut Vec<RifeFrameStep>, workspace: &Workspace, size: UVec2,
    format: AtlasFormat, job: &InBetweenJob, pair: &InBetweenPair, step: PairStep, room: Bc7Room) {
    if let Some(dispatch) = rife_dispatch(&workspace.rife, step) {
        recorded.push(Recorded::Rife(dispatch.clone()));
        return;
    }
    if format == AtlasFormat::Bc7 && step == PairStep::CopyFirst {
        debug_assert_eq!(pair.first.padded_size(), pair.targets[0].padded_size(), "a first frame keeps its size");
        recorded.push(Recorded::Blocks { source: pair.first, target: pair.targets[0], offset: room.bounce_bytes });
        return;
    }
    let card_frame = |rect: AtlasRect, offset: UVec2| CardFrame { source: source_rect(rect), offset: offset.into() };
    let size_pair = size.into();
    let (target, frame_step) = match step {
        PairStep::Pack => (None, RifeFrameStep::pack(size_pair, card_frame(pair.first, pair.first_offset),
            card_frame(pair.next, pair.next_offset))),
        PairStep::CopyFirst => (Some(pair.targets[0]), RifeFrameStep::copy_frame(size_pair, source_rect(pair.first))),
        PairStep::Store(generated) =>
            (Some(pair.targets[generated + 1]), RifeFrameStep::store(size_pair, pair.card.into(), job.snap_alpha)),
        _ => unreachable!("network steps are dispatches"),
    };
    let bc7_target = target.filter(|_| format == AtlasFormat::Bc7);
    if let Some(target) = bc7_target {
        recorded.push(Recorded::ClearStaging { workspace: size, target });
    }
    recorded.push(Recorded::Atlas { workspace: size, format, step: frame_step, uniform: steps.len() });
    steps.push(frame_step);
    match (target, bc7_target) {
        (_, Some(target)) => recorded.push(Recorded::Scratch { workspace: size, target, y: room.scratch_rows }),
        (Some(target), None) => recorded.push(Recorded::Copy { workspace: size, target }),
        (None, _) => {}
    }
}

/// An atlas rectangle as the compute crate takes a frame.
fn source_rect(rect: AtlasRect) -> SourceRect {
    SourceRect { layer: rect.layer, x: rect.x, y: rect.y, width: rect.width, height: rect.height }
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
    fn workspaces_are_padded() {
        assert_eq!(padded_size(UVec2::new(65, 64)), UVec2::new(128, 64));
    }

    /// A BC7 pair: a first frame of 13 by 6 (4 by 2 blocks) and in-betweens of 10 by 9 (12 rows padded).
    fn bc7_pair() -> InBetweenPair {
        let rect = |width, height| AtlasRect { format: AtlasFormat::Bc7, layer: 1, x: 8, y: 4, width, height };
        InBetweenPair { first: rect(13, 6), next: rect(13, 6), card: UVec2::new(10, 9),
            first_offset: UVec2::ZERO, next_offset: UVec2::ZERO, targets: vec![rect(13, 6), rect(10, 9), rect(10, 9)] }
    }

    #[test]
    fn bc7_steps_take_scratch_rows_and_bounce_bytes_until_none_are_left() {
        let pair = bc7_pair();
        // A first frame's blocks: rows of 4 blocks, in 256 bytes for a copy of several rows.
        assert_eq!(block_copy_layout(pair.first), (wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 2));
        let mut room = Bc7Room::default();
        assert_eq!(room.take(PairStep::Pack, &pair), Some(Bc7Room::default()));
        assert_eq!(room.take(PairStep::CopyFirst, &pair), Some(Bc7Room::default()));
        assert_eq!(room.take(PairStep::Store(0), &pair).unwrap().bounce_bytes, 2 * 256);
        assert_eq!(room.take(PairStep::Store(1), &pair).unwrap().scratch_rows, 12);
        assert_eq!(room, Bc7Room { scratch_rows: 24, bounce_bytes: 2 * 256 });
        // The in-betweens' rows stay within the scratch texture, and their blocks apart.
        let mut full = Bc7Room { scratch_rows: BC7_SCRATCH_SIDE - 8, bounce_bytes: BC7_BOUNCE_BYTES - 256 };
        assert_eq!(full.take(PairStep::Store(0), &pair), None);
        assert_eq!(full.take(PairStep::CopyFirst, &pair), None);
        assert_eq!(full, Bc7Room { scratch_rows: BC7_SCRATCH_SIDE - 8, bounce_bytes: BC7_BOUNCE_BYTES - 256 });
        assert_eq!(scratch_blocks_offset(12) - scratch_blocks_offset(0),
            MOBILE_BLOCK_FORMAT.encoded_bytes(BC7_SCRATCH_SIDE, 12));
        assert!(scratch_blocks_offset(BC7_SCRATCH_SIDE) <= MOBILE_BLOCK_FORMAT.encoded_bytes(BC7_SCRATCH_SIDE,
            BC7_SCRATCH_SIDE));
    }
}
