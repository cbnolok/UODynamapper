//! Offscreen web-map export mode, enabled by `DYNAMAPPER_EXPORT_JOB=<absolute job.toml>`.
//!
//! The renderer reads the job that `webmap-export-cli run` writes, renders every frame into an offscreen
//! image with the fixed export preset, and hands each one to the tool through the spool directory: the raw
//! RGBA8 pixels first, then the sidecar, each written as `*.tmp` and renamed. The contract is
//! `docs/agent_notes/proposals/2026-11-01-web-map-export.md`; the tool's side is `tools/webmap-export-cli`.
//! No interactive state is touched: settings are overridden in memory only and never saved.
use crate::configs::shader_presets::{UniformState, CLASSIC_PRESET_GROUP, PAINTERLY_PRESET_GROUP};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::app_states::AppState;
use crate::core::controls::teleport::teleport_player;
use crate::core::render::scene::{
    camera::{ORTHO_SIZE_FACTOR, ORTHO_WIDTH_SCALE_FACTOR, PlayerCamera},
    chunks_in_tile_rect,
    player::Player,
    world::{
        WorldGeoData,
        art::statics_collect::StaticArtCollectDebugState,
        land::{LCMesh, LandUploadTelemetry, draw_mesh::ChunkScale, mesh_material::LandShaderModePresets, tile_atlas::TileAtlas},
    },
    ExportChunkFootprint, RecomputeVisibleChunksEvent,
};
use crate::core::texture_cache::art::{GroundArtPageAtlas, SpriteArtPageAtlas};
use crate::prelude::*;
use bevy::prelude::*;
use bevy::{
    camera::{ImageRenderTarget, RenderTarget, ScalingMode},
    ecs::system::SystemParam,
    image::Image,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{PipelineCache, TextureFormat, TextureUsages},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use color_eyre::eyre::{self, WrapErr};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use udd_container::UddpReader;

pub const EXPORT_JOB_ENV: &str = "DYNAMAPPER_EXPORT_JOB";
const JOB_FORMAT_VERSION: u32 = 1;
const SIDECAR_FORMAT_VERSION: u32 = 1;
/// The projection this renderer draws at zoom 1: 44-pixel diamonds and 4 pixels per Z step.
const TILE_HALF_WIDTH_PX: u32 = 22;
const Z_STEP_PX: u32 = 4;
/// How far below z = 0 a tile can be drawn, in Z steps (UO Z reaches -128).
const LOWEST_Z_STEPS: u32 = 128;
/// The texture size wgpu guarantees on every backend; the capture target, pad included, must fit it.
const MAX_FRAME_SIZE_PX: u32 = 8192;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const TEMPORARY_SUFFIX: &str = ".tmp";
const PIXELS_EXTENSION: &str = "rgba";
const SIDECAR_EXTENSION: &str = "toml";
/// The export draws at exactly one screen pixel per client pixel.
const EXPORT_ZOOM: f32 = 1.0;
/// Fixed daylight: the period of the style preset the export uses.
const EXPORT_LIGHTING_PERIOD: &str = "afternoon";
/// Consecutive frames the readiness signals must hold, unchanged, before a capture.
const READY_STABLE_FRAMES: u32 = 3;
const FRAME_READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a ready frame may take to give two identical captures.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
/// How far the measured check-tile vertex may sit from a whole pixel before the placement is an error.
const PLACEMENT_TOLERANCE_PX: f32 = 0.05;
/// Extra target pixels on every side of the frame. The camera always centres a whole tile's top vertex on the
/// target, exactly as the interactive view does, so every frame rasterises the same camera-relative geometry; the
/// frame is then cut out around it. The nearest tile vertex lies within one half-width of the frame centre on
/// each axis, so a full tile width covers the cut on every side.
const CAPTURE_PAD_PX: u32 = 2 * TILE_HALF_WIDTH_PX;
const EXPORT_FAILURE_EXIT_CODE: u8 = 1;

/// `job.toml`, read strictly. Mirrors `webmap_export_cli::formats::Job`.
#[derive(Clone, Debug, Deserialize, Resource)]
#[serde(deny_unknown_fields)]
pub struct ExportJob {
    pub format_version: u32,
    pub facet: u32,
    pub map_width_tiles: u32,
    pub map_height_tiles: u32,
    pub texture_source: ClientTextureSource,
    pub tile_half_width_px: u32,
    pub z_step_px: u32,
    pub z_top_px: u32,
    pub frame_size_px: u32,
    pub frame_margin_px: u32,
    pub spool_dir: PathBuf,
    pub max_spool_frames: u32,
    pub frames: Vec<ExportJobFrame>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportJobFrame {
    pub id: String,
    /// Top-left corner of the rendered frame, in base-level pixels; may be negative.
    pub left_px: i64,
    pub top_px: i64,
}

/// `<spool>/<id>.toml`. Mirrors `webmap_export_cli::formats::FrameSidecar`.
#[derive(Debug, Serialize)]
struct FrameSidecar<'a> {
    format_version: u32,
    id: &'a str,
    left_px: i64,
    top_px: i64,
    width_px: u32,
    height_px: u32,
    check_tile: [u32; 2],
    check_tile_px: [i64; 2],
    render_ms: u64,
    packages: &'a [PackageRevision],
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct PackageRevision {
    name: String,
    hash64: String,
}

/// Reads the job named by [`EXPORT_JOB_ENV`], if any.
pub fn load_requested_job() -> eyre::Result<Option<ExportJob>> {
    let Some(path) = std::env::var_os(EXPORT_JOB_ENV) else { return Ok(None) };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        eyre::bail!("{EXPORT_JOB_ENV} must be an absolute path, not {}", path.display());
    }
    let text = fs::read_to_string(&path).wrap_err_with(|| format!("read {}", path.display()))?;
    let job: ExportJob = uocf::utils::config_toml::parse_strict(&path.display().to_string(), &text)?;
    job.validate().wrap_err_with(|| format!("{}", path.display()))?;
    Ok(Some(job))
}

impl ExportJob {
    fn validate(&self) -> eyre::Result<()> {
        if self.format_version != JOB_FORMAT_VERSION {
            eyre::bail!("format_version {} is not {JOB_FORMAT_VERSION}", self.format_version);
        }
        if (self.tile_half_width_px, self.z_step_px) != (TILE_HALF_WIDTH_PX, Z_STEP_PX) {
            eyre::bail!(
                "the job's projection ({} px half width, {} px per Z) is not the renderer's ({TILE_HALF_WIDTH_PX}, {Z_STEP_PX})",
                self.tile_half_width_px,
                self.z_step_px
            );
        }
        if u8::try_from(self.facet).is_err() {
            eyre::bail!("facet {} is out of range", self.facet);
        }
        let max_frame_size_px = MAX_FRAME_SIZE_PX - 2 * CAPTURE_PAD_PX;
        if self.frame_size_px % 2 != 0 || self.frame_size_px > max_frame_size_px || self.frame_size_px <= 2 * self.frame_margin_px {
            eyre::bail!("frame_size_px {} (margin {}) is not an even size up to {max_frame_size_px} wider than both margins",
                self.frame_size_px, self.frame_margin_px);
        }
        if self.map_width_tiles == 0 || self.map_height_tiles == 0 {
            eyre::bail!("the map size is empty");
        }
        if !self.spool_dir.is_absolute() || !self.spool_dir.is_dir() {
            eyre::bail!("spool_dir {} is not an existing absolute directory", self.spool_dir.display());
        }
        if self.max_spool_frames == 0 {
            eyre::bail!("max_spool_frames must be at least 1");
        }
        if self.frames.is_empty() {
            eyre::bail!("the job has no frames");
        }
        for frame in &self.frames {
            let plain = !frame.id.is_empty()
                && !frame.id.starts_with('.')
                && frame.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !plain {
                eyre::bail!("frame id {:?} is not a plain file name", frame.id);
            }
        }
        Ok(())
    }

    fn lighting_preset_key(&self) -> String {
        // EC exports keep the painterly look they had before KR-like became the faithful client look.
        let style = match self.texture_source {
            ClientTextureSource::Cc => CLASSIC_PRESET_GROUP,
            ClientTextureSource::Ec => PAINTERLY_PRESET_GROUP,
        };
        format!("{style}.{EXPORT_LIGHTING_PERIOD}")
    }
}

/// The export preset, applied in memory to the loaded settings; export mode never saves them.
pub fn apply_settings_overrides(settings: &mut Settings, job: &ExportJob) {
    settings.core.world.hide_player = true;
    settings.world_rendering.enable_statics = true;
    settings.world_rendering.enable_static_lights = false;
    settings.world_rendering.enable_static_art_local_lights = false;
    settings.world_rendering.show_hidden_multi_parts = false;
    settings.world_rendering.diagnostics.highlight_hovered_static_object = false;
    settings.graphics.vsync = false;
    settings.graphics.reduce_unfocused_fps = false;
    settings.graphics.anti_aliasing = AntiAliasingMode::Off;
    settings.graphics.art_texture_source = job.texture_source;
    settings.graphics.land_texture_source = job.texture_source;
    settings.app.window.free_camera = false;
    settings.app.window.perspective_camera = false;
    settings.app.performance.show_overlay = false;
    settings.app.performance.frame_limit_enabled = false;
    settings.app.debug.map_render_wireframe = false;
    settings.session_state.window.zoom = EXPORT_ZOOM;
    let placement = FramePlacement::new(job, &job.frames[0]);
    settings.session_state.world.last_p = placement.player_position(job);
}

/// Where one frame puts the camera, and which tiles it needs.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FramePlacement {
    /// The tile whose top vertex the camera looks at, at the capture target's centre: the one nearest the frame
    /// centre, so the camera always sits on a whole tile.
    look_at_tile: (i64, i64),
    /// The capture target pixel of the frame's top-left corner, where the frame is cut out.
    crop_origin: (u32, u32),
    /// Tiles whose sprites can reach the frame, half-open and clamped to the map.
    footprint: ExportChunkFootprint,
    /// A tile inside the map near the frame centre; the player stands there and the sidecar reports it.
    anchor_tile: (u32, u32),
}

impl FramePlacement {
    fn new(job: &ExportJob, frame: &ExportJobFrame) -> Self {
        let half = i64::from(job.tile_half_width_px);
        let size = i64::from(job.frame_size_px);
        let height = i64::from(job.map_height_tiles);
        let z_top = i64::from(job.z_top_px);
        let (centre_x, centre_y) = (frame.left_px + size / 2, frame.top_px + size / 2);
        // s = x − y and t = x + y of the frame centre's z = 0 ground point.
        let s = centre_x as f64 / half as f64 - height as f64;
        let t = (centre_y - z_top) as f64 / half as f64;
        let look_at_tile = (((s + t) / 2.0).round() as i64, ((t - s) / 2.0).round() as i64);
        let (vertex_x, vertex_y) = project(job, look_at_tile.0, look_at_tile.1);
        let target_centre = size / 2 + i64::from(CAPTURE_PAD_PX);
        let crop_origin = (
            (target_centre - (vertex_x - frame.left_px)) as u32,
            (target_centre - (vertex_y - frame.top_px)) as u32,
        );

        // Any sprite reaching the frame has its tile within the frame's z = 0 footprint widened by the deepest
        // Z drop plus the tallest rise, on every side (wide sprites spread sideways too).
        let margin = (i64::from(job.z_top_px) + i64::from(job.z_step_px * LOWEST_Z_STEPS)).div_euclid(half) + 1;
        let s0 = frame.left_px.div_euclid(half) - height - margin;
        let s1 = (frame.left_px + size).div_euclid(half) - height + 1 + margin;
        let t0 = (frame.top_px - z_top).div_euclid(half) - margin;
        let t1 = (frame.top_px + size - z_top).div_euclid(half) + 1 + margin;
        let clamp_x = |value: i64| value.clamp(0, i64::from(job.map_width_tiles)) as u32;
        let clamp_y = |value: i64| value.clamp(0, i64::from(job.map_height_tiles)) as u32;
        let footprint = ExportChunkFootprint {
            tile_x0: clamp_x((s0 + t0).div_euclid(2)),
            tile_x1: clamp_x((s1 + t1).div_euclid(2) + 1),
            tile_y0: clamp_y((t0 - s1).div_euclid(2)),
            tile_y1: clamp_y((t1 - s0).div_euclid(2) + 1),
        };
        let anchor_tile = (
            look_at_tile.0.clamp(0, i64::from(job.map_width_tiles) - 1) as u32,
            look_at_tile.1.clamp(0, i64::from(job.map_height_tiles) - 1) as u32,
        );
        Self { look_at_tile, crop_origin, footprint, anchor_tile }
    }

    fn player_position(&self, job: &ExportJob) -> UOVec4 {
        UOVec4::new(self.anchor_tile.0 as u16, self.anchor_tile.1 as u16, 0, job.facet as u8)
    }
}

/// The base-level pixel of tile `(x, y)`'s top vertex at z = 0.
fn project(job: &ExportJob, x: i64, y: i64) -> (i64, i64) {
    let half = i64::from(job.tile_half_width_px);
    ((x - y + i64::from(job.map_height_tiles)) * half, (x + y) * half + i64::from(job.z_top_px))
}

pub struct ExportJobPlugin {
    pub registered_by: &'static str,
    pub job: Option<ExportJob>,
}
impl_tracked_plugin!(ExportJobPlugin);

impl Plugin for ExportJobPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        let Some(job) = self.job.clone() else { return };
        console_logger::one(
            LogSev::Info,
            LogAbout::Renderer,
            &format!("Export job: {} frames into {}", job.frames.len(), job.spool_dir.display()),
        );
        app.world_mut()
            .get_resource_mut::<LandShaderModePresets>()
            .expect("ExportJobPlugin must be added after the shader presets are loaded")
            .default_preset = job.lighting_preset_key();
        app.insert_resource(job)
            .init_resource::<ExportRuntime>()
            .init_resource::<ExportCapture>()
            .add_systems(PostStartup, sys_apply_export_uniforms)
            .add_systems(Update, sys_run_export.run_if(in_state(AppState::InGame)));
        let waiting_pipelines = WaitingRenderPipelines::default();
        app.insert_resource(waiting_pipelines.clone());
        // Animated materials read `globals.time`; the export draws them all at one fixed time.
        app.sub_app_mut(RenderApp)
            .insert_resource(waiting_pipelines)
            .add_systems(Render, sys_freeze_render_time.in_set(RenderSystems::PrepareAssets))
            .add_systems(Render, sys_count_waiting_pipelines.in_set(RenderSystems::Cleanup));
    }
}

fn sys_apply_export_uniforms(mut uniforms: ResMut<UniformState>) {
    uniforms.effects.enable_water_animation = 0;
    uniforms.lighting.enable_fog = 0;
    uniforms.dirty = true;
}

fn sys_count_waiting_pipelines(cache: Res<PipelineCache>, waiting: Res<WaitingRenderPipelines>) {
    waiting.0.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

fn sys_freeze_render_time(time: Option<ResMut<Time>>) {
    if let Some(mut time) = time {
        *time = Time::default();
    }
}

#[derive(Default)]
enum ExportPhase {
    #[default]
    Start,
    WaitForSpool,
    Settling { stable_frames: u32, signature: Option<ReadySignature> },
    Capturing,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ReadySignature {
    visible_chunks: usize,
    emitted_instances: usize,
    /// Statics whose art the package does not have; they are drawn as nothing, as in the interactive view.
    absent_art: usize,
    land_revision: u64,
}

#[derive(Resource, Default)]
struct ExportRuntime {
    phase: ExportPhase,
    frame_index: usize,
    target: Option<Handle<Image>>,
    packages: Vec<PackageRevision>,
    placement: Option<FramePlacement>,
    frame_started: Option<Instant>,
    phase_started: Option<Instant>,
    /// The frame's last capture, kept until a second one confirms it.
    previous_capture: Option<Vec<u8>>,
}

/// Render pipelines still compiling, counted in the render world. A new target format needs new pipelines,
/// and Bevy skips draws whose pipeline is not ready yet.
#[derive(Resource, Clone, Default)]
struct WaitingRenderPipelines(Arc<AtomicUsize>);

/// The pixels of the frame being captured, delivered by the screenshot observer.
#[derive(Resource, Default)]
struct ExportCapture(Option<Image>);

#[derive(SystemParam)]
struct ExportScene<'w, 's> {
    camera: Query<'w, 's, (&'static mut Camera, &'static mut Projection, &'static mut Transform,
        &'static mut RenderTarget, &'static GlobalTransform), (With<PlayerCamera>, Without<Player>)>,
    player: Query<'w, 's, (&'static mut Player, &'static mut Transform), Without<PlayerCamera>>,
    world: Res<'w, WorldGeoData>,
    images: ResMut<'w, Assets<Image>>,
    settings: Res<'w, Settings>,
    recompute: MessageWriter<'w, RecomputeVisibleChunksEvent>,
    exit: MessageWriter<'w, AppExit>,
}

#[derive(SystemParam)]
struct ExportReadiness<'w, 's> {
    chunks: Query<'w, 's, (&'static LCMesh, Has<Mesh3d>)>,
    chunk_scale: Res<'w, ChunkScale>,
    tile_atlas: Option<Res<'w, TileAtlas>>,
    land_uploads: Option<Res<'w, LandUploadTelemetry>>,
    art_stats: Option<Res<'w, StaticArtCollectDebugState>>,
    sprite_atlas: Option<Res<'w, SpriteArtPageAtlas>>,
    ground_atlas: Option<Res<'w, GroundArtPageAtlas>>,
    waiting_pipelines: Res<'w, WaitingRenderPipelines>,
}

fn sys_run_export(
    mut commands: Commands,
    job: Res<ExportJob>,
    mut runtime: ResMut<ExportRuntime>,
    mut capture: ResMut<ExportCapture>,
    mut scene: ExportScene,
    readiness: ExportReadiness,
) {
    if let Err(error) = advance_export(&mut commands, &job, &mut runtime, &mut capture, &mut scene, &readiness) {
        console_logger::one(LogSev::Error, LogAbout::Renderer, &format!("Export failed: {error:?}"));
        runtime.phase = ExportPhase::Finished;
        scene.exit.write(AppExit::from_code(EXPORT_FAILURE_EXIT_CODE));
    }
}

fn advance_export(
    commands: &mut Commands,
    job: &ExportJob,
    runtime: &mut ExportRuntime,
    capture: &mut ExportCapture,
    scene: &mut ExportScene,
    readiness: &ExportReadiness,
) -> eyre::Result<()> {
    match runtime.phase {
        ExportPhase::Finished => {}
        ExportPhase::Start => {
            runtime.packages = package_revisions(Path::new(&scene.settings.runtime_assets.udd_path), job)?;
            runtime.target = Some(setup_export_camera(job, scene)?);
            runtime.phase = ExportPhase::WaitForSpool;
        }
        ExportPhase::WaitForSpool => {
            if count_sidecars(&job.spool_dir)? >= job.max_spool_frames as usize {
                return Ok(());
            }
            let frame = &job.frames[runtime.frame_index];
            let placement = FramePlacement::new(job, frame);
            place_frame(job, &placement, scene)?;
            commands.insert_resource(placement.footprint);
            scene.recompute.write(RecomputeVisibleChunksEvent);
            runtime.placement = Some(placement);
            runtime.frame_started = Some(Instant::now());
            runtime.phase_started = runtime.frame_started;
            runtime.phase = ExportPhase::Settling { stable_frames: 0, signature: None };
        }
        ExportPhase::Settling { stable_frames, signature } => {
            let placement = runtime.placement.expect("a placed frame");
            let readiness = ready_signature(job, &placement, scene, readiness);
            let current = readiness.as_ref().ok().copied();
            let stable_frames = if current.is_some() && current == signature { stable_frames + 1 } else { 0 };
            if stable_frames >= READY_STABLE_FRAMES {
                let absent_art = current.map_or(0, |current| current.absent_art);
                if absent_art != 0 {
                    console_logger::one(
                        LogSev::Warn,
                        LogAbout::Renderer,
                        &format!(
                            "Export frame {}: {absent_art} statics have no art in the texture package and are not drawn",
                            job.frames[runtime.frame_index].id
                        ),
                    );
                }
                request_capture(commands, runtime);
                runtime.previous_capture = None;
                runtime.phase = ExportPhase::Capturing;
                runtime.phase_started = Some(Instant::now());
            } else {
                if runtime.phase_started.is_some_and(|started| started.elapsed() > FRAME_READY_TIMEOUT) {
                    let waiting_for = readiness.err().unwrap_or_else(|| "a stable scene".to_owned());
                    eyre::bail!(
                        "frame {} did not become ready within {FRAME_READY_TIMEOUT:?}; still waiting for {waiting_for}",
                        job.frames[runtime.frame_index].id
                    );
                }
                runtime.phase = ExportPhase::Settling { stable_frames, signature: current };
            }
        }
        ExportPhase::Capturing => {
            let Some(image) = capture.0.take() else {
                if runtime.phase_started.is_some_and(|started| started.elapsed() > CAPTURE_TIMEOUT) {
                    eyre::bail!("frame {} gave no two identical captures within {CAPTURE_TIMEOUT:?}", job.frames[runtime.frame_index].id);
                }
                return Ok(());
            };
            let frame = &job.frames[runtime.frame_index];
            let placement = runtime.placement.expect("a placed frame");
            let pixels = frame_pixels(image, job.frame_size_px, placement.crop_origin)?;
            // Only a picture that two consecutive captures agree on is final: anything still streaming,
            // uploading or compiling shows up as a difference.
            if runtime.previous_capture.as_ref() != Some(&pixels) {
                runtime.previous_capture = Some(pixels);
                request_capture(commands, runtime);
                return Ok(());
            }
            runtime.previous_capture = None;
            let check_tile_px = measured_vertex(job, frame, &placement, scene)?;
            let render_ms = runtime.frame_started.map_or(0, |started| started.elapsed().as_millis() as u64);
            write_frame(job, frame, &pixels, placement.anchor_tile, check_tile_px, render_ms, &runtime.packages)?;
            console_logger::one(
                LogSev::Info,
                LogAbout::Renderer,
                &format!("Export frame {} written ({}/{}, {render_ms} ms)", frame.id, runtime.frame_index + 1, job.frames.len()),
            );
            runtime.frame_index += 1;
            if runtime.frame_index == job.frames.len() {
                runtime.phase = ExportPhase::Finished;
                scene.exit.write(AppExit::Success);
            } else {
                runtime.phase = ExportPhase::WaitForSpool;
            }
        }
    }
    Ok(())
}

fn request_capture(commands: &mut Commands, runtime: &ExportRuntime) {
    let target = runtime.target.clone().expect("the export target");
    commands.spawn(Screenshot::image(target)).observe(
        |captured: On<ScreenshotCaptured>, mut capture: ResMut<ExportCapture>| {
            capture.0 = Some(captured.image.clone());
        },
    );
}

/// Points the world camera at an offscreen target, the frame plus its pad, with an opaque black background.
fn setup_export_camera(job: &ExportJob, scene: &mut ExportScene) -> eyre::Result<Handle<Image>> {
    let target_size = capture_size_px(job.frame_size_px);
    let mut image = Image::new_target_texture(target_size, target_size, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = scene.images.add(image);
    let (mut camera, mut projection, _, mut target, _) =
        scene.camera.single_mut().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    camera.clear_color = ClearColorConfig::Custom(Color::BLACK);
    *target = RenderTarget::Image(ImageRenderTarget { handle: handle.clone(), scale_factor: 1.0 });
    let Projection::Orthographic(ortho) = &mut *projection else {
        eyre::bail!("the export needs the orthographic camera");
    };
    // The same pixel scale as the window: `sys_update_camera_projection_to_view` at the target's size.
    let size = target_size as f32;
    ortho.scale = EXPORT_ZOOM;
    ortho.scaling_mode = ScalingMode::Fixed {
        width: size / ORTHO_SIZE_FACTOR,
        height: (size / ORTHO_WIDTH_SCALE_FACTOR) / ORTHO_SIZE_FACTOR,
    };
    Ok(handle)
}

fn place_frame(job: &ExportJob, placement: &FramePlacement, scene: &mut ExportScene) -> eyre::Result<()> {
    let (_, _, mut transform, _, _) =
        scene.camera.single_mut().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    let look_at = Vec3::new(placement.look_at_tile.0 as f32, 0.0, placement.look_at_tile.1 as f32);
    *transform = Transform::from_translation(look_at + PlayerCamera::BASE_OFFSET_FROM_PLAYER).looking_at(look_at, Vec3::Y);
    let (mut player, mut player_transform) =
        scene.player.single_mut().map_err(|error| eyre::eyre!("player: {error}"))?;
    teleport_player(&mut player, &mut player_transform, placement.player_position(job), &scene.world)
        .map_err(|error| eyre::eyre!(error))
}

/// What must hold before a capture: the frame's chunks built, every land and art upload done and the art
/// collection settled on them. While anything is pending, the error says what.
fn ready_signature(
    job: &ExportJob,
    placement: &FramePlacement,
    scene: &ExportScene,
    readiness: &ExportReadiness,
) -> Result<ReadySignature, String> {
    let waiting_pipelines = readiness.waiting_pipelines.0.load(Ordering::Relaxed);
    if waiting_pipelines != 0 {
        return Err(format!("{waiting_pipelines} render pipelines to compile"));
    }
    let (player, _) = scene.player.single().map_err(|error| format!("the player: {error}"))?;
    if player.current_pos.is_none() || player.current_pos != player.prev_rendered_pos {
        return Err("the player to reach the anchor tile".to_owned());
    }
    let map = scene.world.maps.get(&job.facet).ok_or_else(|| format!("map {} to load", job.facet))?;
    let scale = readiness.chunk_scale.0;
    let built: HashSet<(u32, u32)> = readiness
        .chunks
        .iter()
        .filter(|(chunk, has_mesh)| *has_mesh && chunk.parent_map_id == job.facet && chunk.scale == scale)
        .map(|(chunk, _)| (chunk.gx, chunk.gy))
        .collect();
    let required = chunks_in_tile_rect(&placement.footprint, map.width, map.height, scale);
    let missing = required.iter().filter(|chunk| !built.contains(chunk)).count();
    if missing != 0 {
        return Err(format!("{missing} of {} land chunks to be built", required.len()));
    }
    let land_revision = match (&readiness.tile_atlas, &readiness.land_uploads) {
        (Some(atlas), Some(uploads)) => {
            let uploads = uploads.snapshot();
            if atlas.has_pending_uploads()
                || uploads.pending_ops != 0
                || uploads.submitted_metadata_revision != atlas.content_revision
            {
                return Err(format!(
                    "land uploads (atlas pending {}, upload ops {}, metadata revision {} of {})",
                    atlas.has_pending_uploads(),
                    uploads.pending_ops,
                    uploads.submitted_metadata_revision,
                    atlas.content_revision
                ));
            }
            atlas.content_revision
        }
        _ => 0,
    };
    let stats = readiness
        .art_stats
        .as_ref()
        .and_then(|art_stats| art_stats.last)
        .ok_or_else(|| "the first static art collection".to_owned())?;
    let pending_pages = readiness.sprite_atlas.as_ref().map_or(0, |atlas| atlas.0.pending_page_count())
        + readiness.ground_atlas.as_ref().map_or(0, |atlas| atlas.0.pending_page_count());
    // A miss with every requested page resident and none pending is art the package lacks, which stays
    // missing; the stable-signature rule makes sure the count has settled.
    if stats.map_id != job.facet
        || pending_pages != 0
        || stats.unique_requested_pages > stats.atlas_capacity_pages
        || stats.visible_chunks < required.len()
    {
        return Err(format!(
            "static art (map {}, atlas misses {} of {}, pending pages {pending_pages}, resident pages {} of {} requested, \
             capacity {}, collected chunks {} of {})",
            stats.map_id,
            stats.atlas_misses,
            stats.atlas_hits + stats.atlas_misses,
            stats.resident_pages,
            stats.unique_requested_pages,
            stats.atlas_capacity_pages,
            stats.visible_chunks,
            required.len()
        ));
    }
    Ok(ReadySignature {
        visible_chunks: stats.visible_chunks,
        emitted_instances: stats.emitted_instances,
        absent_art: stats.atlas_misses,
        land_revision,
    })
}

/// Where the camera actually put the anchor tile's top vertex, in base-level pixels.
fn measured_vertex(
    job: &ExportJob,
    frame: &ExportJobFrame,
    placement: &FramePlacement,
    scene: &ExportScene,
) -> eyre::Result<[i64; 2]> {
    let (camera, _, _, _, camera_transform) = scene.camera.single().map_err(|error| eyre::eyre!("world camera: {error}"))?;
    let (x, y) = placement.anchor_tile;
    let viewport = camera
        .world_to_viewport(camera_transform, Vec3::new(x as f32, 0.0, y as f32))
        .map_err(|error| eyre::eyre!("project the check tile: {error:?}"))?;
    let rounded = viewport.round();
    if (viewport - rounded).abs().max_element() > PLACEMENT_TOLERANCE_PX {
        eyre::bail!("frame {}: tile {x},{y} landed at {viewport}, off the pixel grid", frame.id);
    }
    let measured = [
        frame.left_px + rounded.x as i64 - i64::from(placement.crop_origin.0),
        frame.top_px + rounded.y as i64 - i64::from(placement.crop_origin.1),
    ];
    let expected = project(job, i64::from(x), i64::from(y));
    if (measured[0], measured[1]) != expected {
        eyre::bail!("frame {}: tile {x},{y} landed at {measured:?}, the projection says {expected:?}", frame.id);
    }
    Ok(measured)
}

fn capture_size_px(frame_size_px: u32) -> u32 {
    frame_size_px + 2 * CAPTURE_PAD_PX
}

/// The frame cut out of the capture, as RGBA8 rows top to bottom.
fn frame_pixels(image: Image, frame_size_px: u32, crop_origin: (u32, u32)) -> eyre::Result<Vec<u8>> {
    let format = image.texture_descriptor.format;
    let size = image.texture_descriptor.size;
    let data = image.data.ok_or_else(|| eyre::eyre!("the capture has no pixel data"))?;
    let target_size = capture_size_px(frame_size_px);
    let expected = target_size as usize * target_size as usize * RGBA_BYTES_PER_PIXEL;
    if (size.width, size.height) != (target_size, target_size) || data.len() != expected {
        eyre::bail!("the capture is {}x{} with {} bytes, not a {target_size}² RGBA8 image", size.width, size.height, data.len());
    }
    let (row_bytes, frame_row_bytes) =
        (target_size as usize * RGBA_BYTES_PER_PIXEL, frame_size_px as usize * RGBA_BYTES_PER_PIXEL);
    let mut data: Vec<u8> = (0..frame_size_px as usize)
        .flat_map(|row| {
            let start = (crop_origin.1 as usize + row) * row_bytes + crop_origin.0 as usize * RGBA_BYTES_PER_PIXEL;
            data[start..start + frame_row_bytes].iter().copied()
        })
        .collect();
    match format {
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => {}
        TextureFormat::Bgra8UnormSrgb | TextureFormat::Bgra8Unorm => {
            data.chunks_exact_mut(RGBA_BYTES_PER_PIXEL).for_each(|pixel| pixel.swap(0, 2));
        }
        other => eyre::bail!("the capture format {other:?} is not 8-bit RGBA"),
    }
    Ok(data)
}

/// The spool's committed frames: sidecars the tool has not consumed yet.
fn count_sidecars(spool_dir: &Path) -> eyre::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(spool_dir).wrap_err_with(|| format!("read {}", spool_dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some(SIDECAR_EXTENSION) {
            count += 1;
        }
    }
    Ok(count)
}

fn write_frame(
    job: &ExportJob,
    frame: &ExportJobFrame,
    pixels: &[u8],
    check_tile: (u32, u32),
    check_tile_px: [i64; 2],
    render_ms: u64,
    packages: &[PackageRevision],
) -> eyre::Result<()> {
    let sidecar = FrameSidecar {
        format_version: SIDECAR_FORMAT_VERSION,
        id: &frame.id,
        left_px: frame.left_px,
        top_px: frame.top_px,
        width_px: job.frame_size_px,
        height_px: job.frame_size_px,
        check_tile: [check_tile.0, check_tile.1],
        check_tile_px,
        render_ms,
        packages,
    };
    let sidecar = toml::to_string(&sidecar).wrap_err("serialize the sidecar")?;
    // The pixels first; the sidecar's rename is the commit marker.
    write_atomically(&job.spool_dir.join(format!("{}.{PIXELS_EXTENSION}", frame.id)), pixels)?;
    write_atomically(&job.spool_dir.join(format!("{}.{SIDECAR_EXTENSION}", frame.id)), sidecar.as_bytes())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(TEMPORARY_SUFFIX);
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).wrap_err_with(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, path).wrap_err_with(|| format!("rename {} into place", temporary.display()))
}

/// Every package the export draws from, with its embedded hash, in a fixed order.
fn package_revisions(udd_path: &Path, job: &ExportJob) -> eyre::Result<Vec<PackageRevision>> {
    const SHARED_PACKAGES: [&str; 2] = ["tilemeta.uddp", "hues.uddp"];
    const CC_PACKAGES: [&str; 2] = ["tex_land_cc.uddp", "tex_art_cc.uddp"];
    // EC art and land fall back to the Classic packages, so an EC export reads both sets.
    const EC_PACKAGES: [&str; 3] = ["tex_land_ec.uddp", "tex_art_ec.uddp", "tex_aux_ec.uddp"];
    let facet_packages = [format!("map{}.uddp", job.facet), format!("statics{}.uddp", job.facet)];
    let textures: &[&str] = match job.texture_source {
        ClientTextureSource::Cc => &CC_PACKAGES,
        ClientTextureSource::Ec => &EC_PACKAGES,
    };
    let fallbacks: &[&str] = match job.texture_source {
        ClientTextureSource::Cc => &[],
        ClientTextureSource::Ec => &CC_PACKAGES,
    };
    let mut revisions = Vec::new();
    for name in facet_packages.iter().map(String::as_str).chain(SHARED_PACKAGES).chain(textures.iter().copied()).chain(fallbacks.iter().copied()) {
        let path = udd_path.join(name);
        if !path.exists() {
            if facet_packages.iter().any(|required| required == name) {
                eyre::bail!("{} is missing", path.display());
            }
            continue;
        }
        let package = UddpReader::load(&path).map_err(|error| eyre::eyre!("open {}: {error}", path.display()))?;
        revisions.push(PackageRevision { name: name.to_string(), hash64: format!("0x{:016x}", package.header().package_hash64) });
    }
    Ok(revisions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRITANNIA_WIDTH: u32 = 7168;
    const BRITANNIA_HEIGHT: u32 = 4096;
    const PILOT_Z_TOP_PX: u32 = 1152;
    const PILOT_FRAME_SIZE_PX: u32 = 4160;
    const PILOT_MARGIN_PX: u32 = 32;

    fn job_text(spool: &Path, extra: &str) -> String {
        format!(
            "format_version = 1\nfacet = 0\nmap_width_tiles = {BRITANNIA_WIDTH}\nmap_height_tiles = {BRITANNIA_HEIGHT}\n\
             texture_source = \"cc\"\ntile_half_width_px = 22\nz_step_px = 4\nz_top_px = {PILOT_Z_TOP_PX}\n\
             frame_size_px = {PILOT_FRAME_SIZE_PX}\nframe_margin_px = {PILOT_MARGIN_PX}\nspool_dir = {:?}\nmax_spool_frames = 2\n{extra}\
             [[frames]]\nid = \"f0021_0000\"\nleft_px = 85984\ntop_px = -32\n",
            spool.display().to_string()
        )
    }

    fn parse(text: &str) -> eyre::Result<ExportJob> {
        let job: ExportJob = uocf::utils::config_toml::parse_strict("job.toml", text)?;
        job.validate()?;
        Ok(job)
    }

    #[test]
    fn job_parses_strictly_and_checks_the_projection() {
        let spool = std::env::temp_dir();
        let job = parse(&job_text(&spool, "")).unwrap();
        assert_eq!(job.frames[0].left_px, 85984);
        assert_eq!(job.lighting_preset_key(), "classic.afternoon");
        assert!(parse(&job_text(&spool, "unknown_key = 1\n")).is_err());
        assert!(parse(&job_text(&spool, "").replace("z_step_px = 4", "z_step_px = 5")).is_err());
        assert!(parse(&job_text(&spool, "").replace("\"f0021_0000\"", "\"../escape\"")).is_err());
        assert!(parse(&job_text(Path::new("relative/spool"), "")).is_err());
    }

    #[test]
    fn placement_puts_a_tile_vertex_near_the_frame_centre_on_whole_pixels() {
        let job = parse(&job_text(&std::env::temp_dir(), "")).unwrap();
        // On the diamond, at its left corner, and wholly outside it (no tile can reach the top-left frame).
        for (left_px, top_px) in [(85984, -32), (4064, 8160), (163808, 98272), (-32, 89088), (-32, -32)] {
            let frame = ExportJobFrame { id: "f".into(), left_px, top_px };
            let placement = FramePlacement::new(&job, &frame);
            let (vx, vy) = project(&job, placement.look_at_tile.0, placement.look_at_tile.1);
            let size = i64::from(job.frame_size_px);
            // The vertex lies within one tile of the centre.
            assert!((vx - (left_px + size / 2)).abs() <= 2 * i64::from(TILE_HALF_WIDTH_PX));
            assert!((vy - (top_px + size / 2)).abs() <= 2 * i64::from(TILE_HALF_WIDTH_PX));
            // The cut lies inside the padded target and puts the vertex at the target centre.
            let (cx, cy) = placement.crop_origin;
            assert!(cx <= 2 * CAPTURE_PAD_PX && cy <= 2 * CAPTURE_PAD_PX, "{:?}", placement.crop_origin);
            let centre = i64::from(capture_size_px(job.frame_size_px) / 2);
            assert_eq!((left_px - i64::from(cx) + centre, top_px - i64::from(cy) + centre), (vx, vy));
            // Every tile whose top vertex projects into the frame is inside the footprint.
            let f = placement.footprint;
            assert!(f.tile_x1 <= BRITANNIA_WIDTH && f.tile_y1 <= BRITANNIA_HEIGHT);
            for x in (0..BRITANNIA_WIDTH as i64).step_by(7) {
                for y in (0..BRITANNIA_HEIGHT as i64).step_by(7) {
                    let (px, py) = project(&job, x, y);
                    if px >= left_px && px < left_px + size && py >= top_px && py < top_px + size {
                        assert!((f.tile_x0 as i64..f.tile_x1 as i64).contains(&x) && (f.tile_y0 as i64..f.tile_y1 as i64).contains(&y),
                            "tile {x},{y} of frame {left_px},{top_px} is outside {f:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn spool_files_are_renamed_into_place_pixels_first() {
        let spool = std::env::temp_dir().join(format!("dynamapper-export-spool-{}", std::process::id()));
        fs::create_dir_all(&spool).unwrap();
        let mut job = parse(&job_text(&spool, "")).unwrap();
        job.frame_size_px = 2;
        let packages = [PackageRevision { name: "map0.uddp".into(), hash64: "0x0123456789abcdef".into() }];
        assert_eq!(count_sidecars(&spool).unwrap(), 0);
        write_frame(&job, &job.frames[0], &[0; 16], (10, 20), [89892, 1812], 5, &packages).unwrap();
        let names: HashSet<String> = fs::read_dir(&spool).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, HashSet::from(["f0021_0000.rgba".to_string(), "f0021_0000.toml".to_string()]));
        assert_eq!(count_sidecars(&spool).unwrap(), 1);
        let sidecar = fs::read_to_string(spool.join("f0021_0000.toml")).unwrap();
        assert!(sidecar.contains("check_tile = [10, 20]") && sidecar.contains("hash64 = \"0x0123456789abcdef\""), "{sidecar}");
        fs::remove_dir_all(&spool).unwrap();
    }
}
