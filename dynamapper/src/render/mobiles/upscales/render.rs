//! The render world's half of the SPAN upscales: the network, loaded on the background pool the first time the
//! upscales are on, a workspace per canvas size and atlas texture (a workspace binds the atlas it reads when made),
//! and each frame the queued jobs worked through in order within what the in-betweens left of the mobiles' compute
//! budget (compute_budget.rs), dispatch by dispatch, in one command buffer submitted before the frame's draws. A frame
//! of a job is set on a slot of its workspace, prepared from the atlas onto the canvas, run through the network, and
//! finished into the workspace's 2x output, whose top left part, the frame and its border at 2x, is copied into its
//! place in the atlas: as it is into the RGBA8 atlas, and through the scratch rows of the BC7 encoding it shares with
//! the in-betweens (bc7_encoding.rs) into the BC7 one, all of a submission's frames encoded in one pass after their
//! steps. A workspace stays with the job in the middle of a frame until the frame ends; a job finished goes back to
//! the main world.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::TextureId;
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::Extract;
use parking_lot::Mutex;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use crate::render::shader_compiles::begin_shader_compile;
use uot_gpu_compute::block_encode::record_copy_to_layer;
use uot_gpu_compute::upscale::{UpscaleDispatch, UpscaleModel, UpscalePipelines, UpscaleWorkspace,
    UPSCALE_FRAME_SLOTS};
use uot_gpu_compute::{ConvUnits, SourceRect};
use super::super::bc7_encoding::{bc7_encode_ms, scratch_blocks_offset, Bc7Encoding, MobileBc7Encoder,
    BC7_SCRATCH_SIDE, MOBILE_BLOCK_FORMAT};
use super::super::clips::AtlasEntry;
use super::super::compute_budget::{steps_within_budget, CostScale, GpuClock, MobileComputeBudget,
    MOBILE_COMPUTE_FRAME_BUDGET_MS};
use super::super::compute_warm_up::{ComputeWarmUp, Warm};
use super::super::frame_atlas::{AtlasFormat, AtlasRect, MobileFrameAtlas};
use super::{MobileUpscales, UpscaleFrame, UpscaleJob};

/// A dispatch's estimated cost: a fixed part, and its convolution's multiply-adds. Fitted to uot-gpu-compute's
/// upscale::tests::benchmark on the Radeon 880M with the convolutions on the shader cores (a frame, 25 dispatches:
/// 1.63 ms at 48x40, 8.1 ms at 128x128, 28 to 31 ms at 256x256, about 0.43 million multiply-adds per canvas pixel);
/// on its matrix units a frame takes 0.5 to 0.7 of that, which the timed cost scale learns.
const UPSCALE_DISPATCH_MS: f64 = 0.027;
const UPSCALE_MS_PER_GIGA_MULTIPLY_ADD: f64 = 1.0;
/// The prepare's and the finish's estimated cost per million pixels of the canvas, besides the fixed part.
const PIXEL_STEP_MS_PER_MEGAPIXEL: f64 = 0.5;
const GIGA: f64 = 1e9;
const MEGA: f64 = 1e6;
/// A workspace no job used for this long is freed (about 1360 bytes per canvas pixel: 5.6 MB at 64x64, 89 MB at
/// 256x256).
const WORKSPACE_IDLE_RELEASE: Duration = Duration::from_secs(20);
/// The frames a workspace starts per submission. Each takes a slot of its own, whose parameters the queue writes
/// before the submission runs; a frame started in an earlier submission may still hold one more.
const FRAMES_STARTED_PER_SUBMISSION: usize = UPSCALE_FRAME_SLOTS - 1;

/// The network on `pipelines` warmed ahead, or on pipelines compiled now for `units`.
fn load_network(device: &wgpu::Device, units: ConvUnits, pipelines: Option<UpscalePipelines>)
    -> Result<UpscaleModel, String> {
    match pipelines {
        Some(pipelines) => UpscaleModel::with_pipelines(device, pipelines),
        None => {
            let _compile = begin_shader_compile("SPAN upscales: network");
            UpscaleModel::new(device, units)
        }
    }
}

enum GpuState {
    Unloaded,
    Loading(Task<Result<UpscaleModel, String>>),
    Ready(Arc<UpscaleModel>),
    /// Logged; the clips play at 1x.
    Failed,
}

/// A workspace and the slot its next frame takes.
struct Workspace {
    span: UpscaleWorkspace,
    next_slot: usize,
    last_used: Instant,
}

/// A workspace's canvas, and the atlas texture it reads.
type WorkspaceKey = (UVec2, TextureId);

enum WorkspaceState {
    Creating(Task<Result<Workspace, String>>),
    Ready(Workspace),
    Failed,
}

/// A step of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameStep {
    /// The frame onto the canvas.
    Prepare,
    Network(usize),
    /// The 2x canvas into the output, and into the atlas.
    Finish,
}

/// A frame's steps, in order.
fn frame_steps(workspace: &UpscaleWorkspace) -> Vec<FrameStep> {
    let mut steps = vec![FrameStep::Prepare];
    steps.extend((0..workspace.network().len()).map(FrameStep::Network));
    steps.push(FrameStep::Finish);
    steps
}

/// A step's estimated GPU time, in ms; with its BC7 encode, for a frame of the BC7 atlas.
fn step_ms(workspace: &UpscaleWorkspace, step: FrameStep, frame: &UpscaleFrame) -> f64 {
    let (width, height) = workspace.size();
    let canvas = width * height;
    match step {
        FrameStep::Prepare => dispatch_ms(0, canvas),
        FrameStep::Network(index) => dispatch_ms(workspace.network()[index].multiply_adds(), 0),
        FrameStep::Finish => {
            let encode = match frame.target.format {
                AtlasFormat::Rgba8 => 0.0,
                AtlasFormat::Bc7 => {
                    let (width, height) = frame.target.padded_size();
                    bc7_encode_ms(width * height)
                }
            };
            dispatch_ms(0, workspace.output_size().0 * workspace.output_size().1) + encode
        }
    }
}

fn dispatch_ms(multiply_adds: u64, pixels: u32) -> f64 {
    UPSCALE_DISPATCH_MS + multiply_adds as f64 / GIGA * UPSCALE_MS_PER_GIGA_MULTIPLY_ADD
        + pixels as f64 / MEGA * PIXEL_STEP_MS_PER_MEGAPIXEL
}

/// Where a job is: the frame, the step of the frame next, and the slot the frame took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct JobProgress {
    frame: usize,
    step: usize,
    slot: usize,
}

/// What a frame records, in order.
enum Recorded {
    Dispatch(UpscaleDispatch),
    /// A workspace's output into the RGBA8 atlas.
    Copy { workspace: WorkspaceKey, target: AtlasRect },
    /// A workspace's output into the scratch texture at row `y`, for its BC7 encode into `target`.
    Scratch { workspace: WorkspaceKey, target: AtlasRect, y: u32 },
}

/// The render world's upscales: the jobs listed this frame, the network, the workspaces and the jobs' progress.
#[derive(Resource)]
pub struct RenderUpscales {
    images: Vec<(AtlasFormat, Handle<Image>)>,
    active: bool,
    jobs: Vec<UpscaleJob>,
    finished: Option<Arc<Mutex<Vec<u64>>>>,
    gpu: GpuState,
    workspaces: HashMap<WorkspaceKey, WorkspaceState>,
    progress: HashMap<u64, JobProgress>,
    /// The job each workspace is in the middle of a frame of.
    holders: HashMap<WorkspaceKey, u64>,
    /// Jobs finished, until the main world stops listing them.
    completed: HashSet<u64>,
    /// None without timestamp queries.
    clock: Option<GpuClock>,
    cost_scale: CostScale,
}

impl Default for RenderUpscales {
    fn default() -> Self {
        Self { images: Vec::new(), active: false, jobs: Vec::new(), finished: None, gpu: GpuState::Unloaded,
            workspaces: HashMap::new(), progress: HashMap::new(), holders: HashMap::new(), completed: HashSet::new(),
            clock: None, cost_scale: CostScale::default() }
    }
}

pub fn sys_extract_mobile_upscales(
    upscales: Extract<Option<Res<MobileUpscales>>>,
    atlas: Extract<Option<Res<MobileFrameAtlas<AtlasEntry>>>>,
    mut render: ResMut<RenderUpscales>,
) {
    let (Some(upscales), Some(atlas)) = (upscales.as_ref(), atlas.as_ref()) else { return };
    if render.images.len() != atlas.images().count() {
        render.images = atlas.images().map(|(format, image)| (format, image.clone())).collect();
    }
    render.active = upscales.active;
    render.jobs.clone_from(&upscales.jobs);
    render.finished = Some(upscales.finished.clone());
}

/// Runs the queued jobs within what is left of the frame's budget.
pub fn sys_render_mobile_upscales(
    mut state: ResMut<RenderUpscales>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_adapter: Res<RenderAdapter>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut warm_up: ResMut<ComputeWarmUp>,
    mut budget: ResMut<MobileComputeBudget>,
    mut bc7: ResMut<MobileBc7Encoder>,
) {
    let state = &mut *state;
    let device = render_device.wgpu_device();
    if let GpuState::Unloaded = state.gpu && state.active {
        let pipelines = match warm_up.take_upscale() {
            Warm::Compiling => return,
            Warm::Ready(pipelines) => Some(pipelines),
            Warm::Unavailable => None,
        };
        let device = device.clone();
        let units = ConvUnits::of(&render_adapter, &device);
        state.gpu = GpuState::Loading(background_pool().spawn(Priority::Visible, "span network load",
            move || load_network(&device, units, pipelines)));
    }
    if let GpuState::Loading(task) = &mut state.gpu {
        state.gpu = match task.poll() {
            TaskPoll::Pending => return,
            TaskPoll::Ready(Ok(gpu)) => {
                info!("The SPAN network is loaded: {:.1} MB of weights.", gpu.weight_bytes() as f64 / MEGA);
                GpuState::Ready(Arc::new(gpu))
            }
            TaskPoll::Ready(Err(error)) => {
                error!("The SPAN network did not load, so the mobiles' frames play at 1x: {error}");
                GpuState::Failed
            }
            TaskPoll::Failed => GpuState::Failed,
        };
    }
    let GpuState::Ready(gpu) = &state.gpu else { return };
    let gpu = gpu.clone();
    if state.clock.is_none() {
        state.clock = GpuClock::new(device, "mobile upscales");
    }
    if let Some(clock) = state.clock.as_mut() {
        clock.update_scale(render_queue.get_timestamp_period(), &mut state.cost_scale);
    }
    let cost_scale = state.cost_scale.get();
    let bc7_listed = state.active && state.jobs.iter().any(|job| job_format(job) == AtlasFormat::Bc7);
    bc7.poll(bc7_listed, &mut warm_up, device, &render_queue);

    let atlases = state.images.iter()
        .filter_map(|(format, image)| Some((*format, gpu_images.get(image)?)))
        .collect::<Vec<_>>();
    let atlas_of = |format| atlases.iter().find(|(atlas, _)| *atlas == format).map(|(_, image)| *image);
    let textures = atlases.iter().map(|(_, image)| image.texture.id()).collect::<HashSet<_>>();

    // Forget what the jobs no longer listed left, the workspaces idle for long, and those of an atlas remade.
    let listed = state.jobs.iter().map(|job| job.id).collect::<HashSet<_>>();
    state.progress.retain(|id, _| listed.contains(id));
    state.completed.retain(|id| listed.contains(id));
    state.holders.retain(|key, id| listed.contains(id) && textures.contains(&key.1));
    let holders = &state.holders;
    state.workspaces.retain(|key, workspace| textures.contains(&key.1) && match workspace {
        WorkspaceState::Ready(workspace) =>
            holders.contains_key(key) || workspace.last_used.elapsed() < WORKSPACE_IDLE_RELEASE,
        WorkspaceState::Creating(_) => true,
        WorkspaceState::Failed => !listed.is_empty(),
    });

    let mut recorded = Vec::new();
    let mut spent = budget.spent_ms();
    let started = spent;
    let mut finished = Vec::new();
    let mut scratch_rows = 0;
    let mut bc7_full = false;
    let mut frames_started = HashMap::<WorkspaceKey, usize>::new();
    let RenderUpscales { jobs, workspaces, progress, holders, completed, .. } = &mut *state;
    let bc7_ready = bc7.ready().is_some();
    for job in jobs.iter() {
        if completed.contains(&job.id) || spent >= MOBILE_COMPUTE_FRAME_BUDGET_MS || bc7_full {
            continue;
        }
        let format = job_format(job);
        let Some(atlas) = atlas_of(format) else { continue };
        if format == AtlasFormat::Bc7 && !bc7_ready {
            continue;
        }
        let key = (job.canvas, atlas.texture.id());
        if holders.get(&key).is_some_and(|holder| *holder != job.id) {
            continue;
        }
        let workspace = workspaces.entry(key).or_insert_with(|| {
            let (gpu, device, source) = (gpu.clone(), device.clone(), (*atlas.texture_view).clone());
            let canvas = job.canvas;
            WorkspaceState::Creating(background_pool().spawn(Priority::Visible, "span workspace", move || {
                let span = UpscaleWorkspace::new(&gpu, &device, &source, canvas.x, canvas.y)?;
                Ok(Workspace { span, next_slot: 0, last_used: Instant::now() })
            }))
        });
        if let WorkspaceState::Creating(task) = workspace {
            *workspace = match task.poll() {
                TaskPoll::Pending => continue,
                TaskPoll::Ready(Ok(ready)) => WorkspaceState::Ready(ready),
                TaskPoll::Ready(Err(error)) => {
                    error!("No SPAN workspace of {}x{}: {error}", job.canvas.x, job.canvas.y);
                    WorkspaceState::Failed
                }
                TaskPoll::Failed => WorkspaceState::Failed,
            };
        }
        let WorkspaceState::Ready(workspace) = workspace else { continue };
        workspace.last_used = Instant::now();
        let plan = frame_steps(&workspace.span);
        let at = progress.entry(job.id).or_default();
        while at.frame < job.frames.len() && spent < MOBILE_COMPUTE_FRAME_BUDGET_MS && !bc7_full {
            let frame = &job.frames[at.frame];
            let started_here = frames_started.entry(key).or_default();
            if at.step == 0 && *started_here >= FRAMES_STARTED_PER_SUBMISSION {
                break;
            }
            let costs = plan[at.step..].iter().map(|&step| step_ms(&workspace.span, step, frame) * cost_scale)
                .collect::<Vec<_>>();
            let (mut count, _) = steps_within_budget(costs.iter().copied(), spent);
            for (index, &step) in plan[at.step..at.step + count].iter().enumerate() {
                match step {
                    FrameStep::Prepare => {
                        at.slot = workspace.next_slot;
                        workspace.next_slot = (workspace.next_slot + 1) % UPSCALE_FRAME_SLOTS;
                        *started_here += 1;
                        if let Err(error) = workspace.span.set_frame(&render_queue, at.slot, source_rect(frame.source),
                            job.snap_alpha) {
                            error!("A mobile frame was not upscaled: {error}");
                        }
                        recorded.extend(workspace.span.prepare(at.slot).iter().cloned().map(Recorded::Dispatch));
                    }
                    FrameStep::Network(dispatch) =>
                        recorded.push(Recorded::Dispatch(workspace.span.network()[dispatch].clone())),
                    FrameStep::Finish => {
                        let target = frame.target;
                        let y = scratch_rows;
                        if format == AtlasFormat::Bc7 {
                            // Out of scratch rows: the rest waits for the next frame.
                            let rows = target.padded_size().1;
                            if scratch_rows + rows > BC7_SCRATCH_SIDE {
                                count = index;
                                bc7_full = true;
                                break;
                            }
                            scratch_rows += rows;
                        }
                        recorded.extend(workspace.span.finish(at.slot).iter().cloned().map(Recorded::Dispatch));
                        recorded.push(match format {
                            AtlasFormat::Rgba8 => Recorded::Copy { workspace: key, target },
                            AtlasFormat::Bc7 => Recorded::Scratch { workspace: key, target, y },
                        });
                    }
                }
            }
            spent += costs[..count].iter().sum::<f64>();
            at.step += count;
            if at.step < plan.len() {
                if at.step > 0 {
                    holders.insert(key, job.id);
                }
                break;
            }
            holders.remove(&key);
            *at = JobProgress { frame: at.frame + 1, step: 0, slot: at.slot };
        }
        if at.frame == job.frames.len() {
            completed.insert(job.id);
            finished.push(job.id);
        }
    }
    budget.spend(spent - started);
    if recorded.is_empty() {
        return;
    }

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mobile upscales") });
    let timed = state.clock.as_ref().filter(|clock| clock.idle());
    if let Some(clock) = timed {
        clock.stamp(&mut encoder, 0);
    }
    let output = |key: &WorkspaceKey| {
        let WorkspaceState::Ready(workspace) = &state.workspaces[key] else { unreachable!("a step ran on it") };
        (workspace.span.output(), workspace.span.output_bytes_per_row())
    };
    let mut next = recorded.iter().peekable();
    let mut encodes = Vec::new();
    while next.peek().is_some() {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mobile upscales"), timestamp_writes: None,
            });
            while let Some(Recorded::Dispatch(dispatch)) = next.next_if(|item| matches!(item, Recorded::Dispatch(_))) {
                dispatch.record(&mut pass);
            }
        }
        // The output's top left part, the frame and its border at 2x: into the RGBA8 atlas, or whole blocks of it
        // into the scratch rows.
        let (workspace, texture, origin, (width, height)): (_, &wgpu::Texture, _, _) = match next.next() {
            None | Some(Recorded::Dispatch(_)) => continue,
            Some(Recorded::Copy { workspace, target }) => {
                let atlas = atlas_of(AtlasFormat::Rgba8).expect("a step ran on it");
                (workspace, &atlas.texture, wgpu::Origin3d { x: target.x, y: target.y, z: target.layer },
                    (target.width, target.height))
            }
            Some(Recorded::Scratch { workspace, target, y }) => {
                encodes.push((*target, *y));
                let bc7_encoding: &Bc7Encoding = bc7.ready().expect("a BC7 step ran");
                (workspace, &bc7_encoding.scratch, wgpu::Origin3d { x: 0, y: *y, z: 0 }, target.padded_size())
            }
        };
        let (buffer, bytes_per_row) = output(workspace);
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: None },
            },
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
    if !encodes.is_empty() {
        // The frames of the BC7 atlas, encoded together: the encoder's one compress of the submission.
        let atlas = atlas_of(AtlasFormat::Bc7).expect("a step ran on it");
        let Bc7Encoding { encoder: block_encoder, scratch_view, blocks, .. } =
            bc7.ready_mut().expect("a BC7 step ran");
        for &(target, y) in &encodes {
            block_encoder.add_frame_at(MOBILE_BLOCK_FORMAT, scratch_view, y, target.width, target.height, blocks,
                scratch_blocks_offset(y));
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mobile upscales BC7"), timestamp_writes: None,
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

/// The atlas of a job's frames, and of its 2x frames.
fn job_format(job: &UpscaleJob) -> AtlasFormat {
    job.frames.first().map_or(AtlasFormat::Rgba8, |frame| frame.source.format)
}

/// An atlas rectangle as the compute crate takes a frame.
fn source_rect(rect: AtlasRect) -> SourceRect {
    SourceRect { layer: rect.layer, x: rect.x, y: rect.y, width: rect.width, height: rect.height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_costs_about_what_the_benchmark_timed() {
        // The model's fit at 128x128 (a 132x132 canvas, 25 dispatches): 8.1 ms on the Radeon 880M.
        const CANVAS_PIXELS: f64 = 132.0 * 132.0;
        const MULTIPLY_ADDS_PER_PIXEL: f64 = 0.425e6;
        const DISPATCHES: f64 = 25.0;
        const TIMED_MS: f64 = 8.1;
        let estimate = DISPATCHES * UPSCALE_DISPATCH_MS
            + CANVAS_PIXELS * MULTIPLY_ADDS_PER_PIXEL / GIGA * UPSCALE_MS_PER_GIGA_MULTIPLY_ADD;
        assert!((estimate - TIMED_MS).abs() < TIMED_MS * 0.1, "{estimate}");
    }
}
