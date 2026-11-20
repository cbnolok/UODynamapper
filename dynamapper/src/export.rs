//! Offscreen web-map export mode, enabled by `DYNAMAPPER_EXPORT_JOB=<absolute job.toml>`.
//!
//! The renderer reads the job that `webmap-export-cli run` writes, renders every frame into an offscreen
//! image with the fixed export preset, and hands each one to the tool through the spool directory: the raw
//! RGBA8 pixels first, then the sidecar, each written as `*.tmp` and renamed. The contract is
//! `docs/agent_notes/proposals/2026-11-01-web-map-export.md`; the tool's side is `tools/webmap-export-cli`.
//! No interactive state is touched: settings are overridden in memory only and never saved.
//!
//! The job (`job`) places each frame (`placement`); a frame is captured once ready (`readiness`, `capture`) and written
//! to the spool with the packages drawn from (`spool`). This file runs the frames through those steps.

mod capture;
mod job;
mod placement;
mod readiness;
mod spool;

pub use job::{EXPORT_JOB_ENV, ExportJob, ExportJobFrame, apply_settings_overrides, load_requested_job};

use crate::render::uniform_state::UniformState;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::app::app_states::AppState;
use crate::render::pipeline_readiness::count_waiting_render_pipelines;
use crate::render::scene::RecomputeVisibleChunksEvent;
use crate::world::player::Player;
use crate::world::maps::WorldGeoData;
use crate::render::view::camera::PlayerCamera;
use crate::render::terrain::mesh_material::LandShaderModePresets;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::{
    camera::RenderTarget,
    ecs::system::SystemParam,
    image::Image,
    render::{Render, RenderApp, RenderSystems},
};
use color_eyre::eyre;
use std::{path::Path, time::{Duration, Instant}};
use capture::{ExportCapture, frame_pixels, measured_vertex, place_frame, request_capture, setup_export_camera};
use placement::FramePlacement;
use readiness::{ExportReadiness, ReadySignature, ready_signature};
use spool::{PackageRevision, count_sidecars, package_revisions, write_frame};

/// Consecutive frames the readiness signals must hold, unchanged, before a capture.
const READY_STABLE_FRAMES: u32 = 3;
const FRAME_READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a ready frame may take to give two identical captures.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
const EXPORT_FAILURE_EXIT_CODE: u8 = 1;

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
        count_waiting_render_pipelines(app);
        // Animated materials read `globals.time`; the export draws them all at one fixed time.
        app.sub_app_mut(RenderApp)
            .add_systems(Render, sys_freeze_render_time.in_set(RenderSystems::PrepareAssets));
    }
}

fn sys_apply_export_uniforms(mut uniforms: ResMut<UniformState>) {
    uniforms.effects.enable_water_animation = 0;
    uniforms.lighting.enable_fog = 0;
    uniforms.dirty = true;
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
