use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
};
use parking_lot::Mutex;

use crate::{
    configs::settings::{SectWorldMapDiagnostics, Settings},
    console_logger::{self, LogAbout, LogSev},
    core::app_states::AppState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldmapTimedSystem {
    ChunkSync,
    ChunkDraw,
}

impl WorldmapTimedSystem {
    fn label(self) -> &'static str {
        match self {
            Self::ChunkSync => "chunk_sync",
            Self::ChunkDraw => "chunk_draw",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldmapCrossAppTimedSystem {
    AtlasStage,
    AtlasExtract,
    AtlasUpload,
    TextureArrayExtract,
    TextureArrayUpload,
    RenderExtractCommands,
    RenderQueue,
    RenderPhaseSort,
    RenderPrepare,
    RenderExecute,
    RenderCleanup,
}

impl WorldmapCrossAppTimedSystem {
    fn label(self) -> &'static str {
        match self {
            Self::AtlasStage => "atlas_stage",
            Self::AtlasExtract => "atlas_extract",
            Self::AtlasUpload => "atlas_upload",
            Self::TextureArrayExtract => "texarray_extract",
            Self::TextureArrayUpload => "texarray_upload",
            Self::RenderExtractCommands => "render_extract_cmd",
            Self::RenderQueue => "render_queue",
            Self::RenderPhaseSort => "render_phase_sort",
            Self::RenderPrepare => "render_prepare",
            Self::RenderExecute => "render_execute",
            Self::RenderCleanup => "render_cleanup",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RunningSystemTimer {
    pub last_us: u64,
    pub max_us: u64,
    pub avg_us: f64,
    pub samples: u64,
}

impl RunningSystemTimer {
    fn record(&mut self, elapsed: Duration) {
        let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.last_us = elapsed_us;
        self.max_us = self.max_us.max(elapsed_us);
        self.samples = self.samples.saturating_add(1);

        let sample_count = self.samples as f64;
        self.avg_us += (elapsed_us as f64 - self.avg_us) / sample_count;
    }

    fn format_summary(&self) -> String {
        format!(
            "last {:.2} ms | avg {:.2} ms | max {:.2} ms",
            self.last_us as f64 / 1000.0,
            self.avg_us / 1000.0,
            self.max_us as f64 / 1000.0,
        )
    }
}

#[derive(Resource, Default)]
pub struct WorldmapSystemDiagnostics {
    pub chunk_sync: RunningSystemTimer,
    pub chunk_draw: RunningSystemTimer,
}

impl WorldmapSystemDiagnostics {
    pub fn record(&mut self, kind: WorldmapTimedSystem, elapsed: Duration) {
        match kind {
            WorldmapTimedSystem::ChunkSync => self.chunk_sync.record(elapsed),
            WorldmapTimedSystem::ChunkDraw => self.chunk_draw.record(elapsed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldmapCrossAppDiagnosticsSnapshot {
    pub atlas_stage: RunningSystemTimer,
    pub atlas_extract: RunningSystemTimer,
    pub atlas_upload: RunningSystemTimer,
    pub texture_array_extract: RunningSystemTimer,
    pub texture_array_upload: RunningSystemTimer,
    pub render_extract_commands: RunningSystemTimer,
    pub render_queue: RunningSystemTimer,
    pub render_phase_sort: RunningSystemTimer,
    pub render_prepare: RunningSystemTimer,
    pub render_execute: RunningSystemTimer,
    pub render_cleanup: RunningSystemTimer,
}

#[derive(Default)]
struct WorldmapCrossAppDiagnosticsState {
    atlas_stage: RunningSystemTimer,
    atlas_extract: RunningSystemTimer,
    atlas_upload: RunningSystemTimer,
    texture_array_extract: RunningSystemTimer,
    texture_array_upload: RunningSystemTimer,
    render_extract_commands: RunningSystemTimer,
    render_queue: RunningSystemTimer,
    render_phase_sort: RunningSystemTimer,
    render_prepare: RunningSystemTimer,
    render_execute: RunningSystemTimer,
    render_cleanup: RunningSystemTimer,
}

#[derive(Resource, Clone, Default)]
pub struct WorldmapCrossAppDiagnostics {
    shared: Arc<Mutex<WorldmapCrossAppDiagnosticsState>>,
}

impl WorldmapCrossAppDiagnostics {
    pub fn record(&self, kind: WorldmapCrossAppTimedSystem, elapsed: Duration) {
        let mut diagnostics = self.shared.lock();
        match kind {
            WorldmapCrossAppTimedSystem::AtlasStage => diagnostics.atlas_stage.record(elapsed),
            WorldmapCrossAppTimedSystem::AtlasExtract => diagnostics.atlas_extract.record(elapsed),
            WorldmapCrossAppTimedSystem::AtlasUpload => diagnostics.atlas_upload.record(elapsed),
            WorldmapCrossAppTimedSystem::TextureArrayExtract => {
                diagnostics.texture_array_extract.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::TextureArrayUpload => {
                diagnostics.texture_array_upload.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::RenderExtractCommands => {
                diagnostics.render_extract_commands.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::RenderQueue => diagnostics.render_queue.record(elapsed),
            WorldmapCrossAppTimedSystem::RenderPhaseSort => {
                diagnostics.render_phase_sort.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::RenderPrepare => {
                diagnostics.render_prepare.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::RenderExecute => {
                diagnostics.render_execute.record(elapsed);
            }
            WorldmapCrossAppTimedSystem::RenderCleanup => {
                diagnostics.render_cleanup.record(elapsed);
            }
        }
    }

    pub fn snapshot(&self) -> WorldmapCrossAppDiagnosticsSnapshot {
        let diagnostics = self.shared.lock();
        WorldmapCrossAppDiagnosticsSnapshot {
            atlas_stage: diagnostics.atlas_stage,
            atlas_extract: diagnostics.atlas_extract,
            atlas_upload: diagnostics.atlas_upload,
            texture_array_extract: diagnostics.texture_array_extract,
            texture_array_upload: diagnostics.texture_array_upload,
            render_extract_commands: diagnostics.render_extract_commands,
            render_queue: diagnostics.render_queue,
            render_phase_sort: diagnostics.render_phase_sort,
            render_prepare: diagnostics.render_prepare,
            render_execute: diagnostics.render_execute,
            render_cleanup: diagnostics.render_cleanup,
        }
    }
}

#[derive(Resource, Default)]
struct RenderScheduleTimerState {
    extract_commands_start: Option<Instant>,
    queue_start: Option<Instant>,
    phase_sort_start: Option<Instant>,
    prepare_start: Option<Instant>,
    render_start: Option<Instant>,
    cleanup_start: Option<Instant>,
}

fn sys_mark_render_extract_commands_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.extract_commands_start = Some(Instant::now());
}

fn sys_mark_render_extract_commands_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.extract_commands_start.take() {
        diagnostics.record(
            WorldmapCrossAppTimedSystem::RenderExtractCommands,
            start.elapsed(),
        );
    }
}

fn sys_mark_render_queue_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.queue_start = Some(Instant::now());
}

fn sys_mark_render_queue_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.queue_start.take() {
        diagnostics.record(WorldmapCrossAppTimedSystem::RenderQueue, start.elapsed());
    }
}

fn sys_mark_render_phase_sort_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.phase_sort_start = Some(Instant::now());
}

fn sys_mark_render_phase_sort_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.phase_sort_start.take() {
        diagnostics.record(
            WorldmapCrossAppTimedSystem::RenderPhaseSort,
            start.elapsed(),
        );
    }
}

fn sys_mark_render_prepare_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.prepare_start = Some(Instant::now());
}

fn sys_mark_render_prepare_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.prepare_start.take() {
        diagnostics.record(WorldmapCrossAppTimedSystem::RenderPrepare, start.elapsed());
    }
}

fn sys_mark_render_execute_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.render_start = Some(Instant::now());
}

fn sys_mark_render_execute_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.render_start.take() {
        diagnostics.record(WorldmapCrossAppTimedSystem::RenderExecute, start.elapsed());
    }
}

fn sys_mark_render_cleanup_start(mut state: ResMut<RenderScheduleTimerState>) {
    state.cleanup_start = Some(Instant::now());
}

fn sys_mark_render_cleanup_end(
    mut state: ResMut<RenderScheduleTimerState>,
    diagnostics: Res<WorldmapCrossAppDiagnostics>,
) {
    if let Some(start) = state.cleanup_start.take() {
        diagnostics.record(WorldmapCrossAppTimedSystem::RenderCleanup, start.elapsed());
    }
}

pub struct ScopedWorldmapSystemTimer<'a> {
    diagnostics: &'a mut WorldmapSystemDiagnostics,
    kind: WorldmapTimedSystem,
    start: Instant,
}

impl Drop for ScopedWorldmapSystemTimer<'_> {
    fn drop(&mut self) {
        self.diagnostics.record(self.kind, self.start.elapsed());
    }
}

pub fn scoped_worldmap_timer<'a>(
    diagnostics: &'a mut WorldmapSystemDiagnostics,
    kind: WorldmapTimedSystem,
) -> ScopedWorldmapSystemTimer<'a> {
    ScopedWorldmapSystemTimer {
        diagnostics,
        kind,
        start: Instant::now(),
    }
}

pub struct ScopedWorldmapCrossAppTimer<'a> {
    diagnostics: &'a WorldmapCrossAppDiagnostics,
    kind: WorldmapCrossAppTimedSystem,
    start: Instant,
}

impl Drop for ScopedWorldmapCrossAppTimer<'_> {
    fn drop(&mut self) {
        self.diagnostics.record(self.kind, self.start.elapsed());
    }
}

pub fn scoped_worldmap_cross_app_timer<'a>(
    diagnostics: &'a WorldmapCrossAppDiagnostics,
    kind: WorldmapCrossAppTimedSystem,
) -> ScopedWorldmapCrossAppTimer<'a> {
    ScopedWorldmapCrossAppTimer {
        diagnostics,
        kind,
        start: Instant::now(),
    }
}

#[derive(Resource, Default)]
pub struct WorldmapRuntimeDiagnostics {
    pub map_id: u32,
    pub visible_chunk_target: usize,
    pub desired_scale: Option<u32>,
    pub committed_scale: Option<u32>,
    pub transition_target_scale: Option<u32>,
    pub live_chunks: u32,
    pub pending_spawns: usize,
    pub pending_despawns: usize,
}

#[derive(Resource)]
struct DiagnosticDumpState {
    timer: Timer,
    interval_sec: f32,
}

impl Default for DiagnosticDumpState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(2.0, TimerMode::Repeating),
            interval_sec: 2.0,
        }
    }
}

pub fn add_diagnostics_plugins(app: &mut App, config: &SectWorldMapDiagnostics) {
    let cross_app_diagnostics = WorldmapCrossAppDiagnostics::default();
    app.insert_resource(cross_app_diagnostics.clone());
    if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render_app.insert_resource(cross_app_diagnostics.clone());
        render_app.init_resource::<RenderScheduleTimerState>();
        render_app.add_systems(
            bevy::render::Render,
            (
                sys_mark_render_extract_commands_start
                    .before(bevy::render::RenderSystems::ExtractCommands),
                sys_mark_render_extract_commands_end
                    .after(bevy::render::RenderSystems::ExtractCommands)
                    .before(bevy::render::RenderSystems::PrepareAssets),
                sys_mark_render_queue_start.before(bevy::render::RenderSystems::Queue),
                sys_mark_render_queue_end
                    .after(bevy::render::RenderSystems::Queue)
                    .before(bevy::render::RenderSystems::PhaseSort),
                sys_mark_render_phase_sort_start.before(bevy::render::RenderSystems::PhaseSort),
                sys_mark_render_phase_sort_end
                    .after(bevy::render::RenderSystems::PhaseSort)
                    .before(bevy::render::RenderSystems::Prepare),
                sys_mark_render_prepare_start.before(bevy::render::RenderSystems::Prepare),
                sys_mark_render_prepare_end
                    .after(bevy::render::RenderSystems::Prepare)
                    .before(bevy::render::RenderSystems::Render),
                sys_mark_render_execute_start.before(bevy::render::RenderSystems::Render),
                sys_mark_render_execute_end
                    .after(bevy::render::RenderSystems::Render)
                    .before(bevy::render::RenderSystems::Cleanup),
                sys_mark_render_cleanup_start.before(bevy::render::RenderSystems::Cleanup),
                sys_mark_render_cleanup_end
                    .after(bevy::render::RenderSystems::Cleanup)
                    .before(bevy::render::RenderSystems::PostCleanup),
            ),
        );
    }

    {
        let mut registry = crate::util_lib::tracked_plugin::plugin_registry()
            .lock()
            .expect("plugin registry poisoned");
        registry.record("FrameTimeDiagnosticsPlugin", "Core");
        if config.enable_render_diagnostics {
            registry.record("RenderDiagnosticsPlugin", "Core");
        }
    }
    app.add_plugins((bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),));

    if config.enable_render_diagnostics {
        app.add_plugins((
            // `SystemInformationDiagnosticsPlugin` is intentionally NOT used here:
            // it fails to initialize with dynamic_linking enabled (emits a 'not supported'
            // warning and returns no data). We read CPU/RAM directly via `sysinfo` instead
            // (see `core/render/overlays/performance.rs`).

            // GPU pipeline statistics (vertex/fragment invocations, clipper primitives).
            // On Vulkan/DX12 also provides GPU elapsed time per pass.
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
    }

    app.init_resource::<WorldmapSystemDiagnostics>()
        .init_resource::<WorldmapRuntimeDiagnostics>()
        .init_resource::<DiagnosticDumpState>()
        .add_systems(
            Update,
            sys_dump_configurable_diagnostics.run_if(in_state(AppState::InGame)),
        );
}

/// Plugin that logs the active GPU preprocessing mode during app build.
/// Runs in `finish()` — after `RenderPlugin` has initialised `GpuPreprocessingSupport`
/// in the render sub-app — so the resource is guaranteed to exist and the check
/// happens exactly once with zero runtime cost.
pub struct LogGpuPreprocessingModePlugin;

impl Plugin for LogGpuPreprocessingModePlugin {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        use bevy::render::batching::gpu_preprocessing::{
            GpuPreprocessingMode, GpuPreprocessingSupport,
        };

        let Some(render_app) = app.get_sub_app(bevy::render::RenderApp) else {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Performance,
                "[GPU Batching] RenderApp not available — cannot query GpuPreprocessingMode",
            );
            return;
        };

        let Some(support) = render_app.world().get_resource::<GpuPreprocessingSupport>() else {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Performance,
                "[GPU Batching] GpuPreprocessingSupport not yet initialised at finish()",
            );
            return;
        };

        let mode_str = match support.max_supported_mode {
            GpuPreprocessingMode::None => "None (CPU-only, WebGL2 / no compute)",
            GpuPreprocessingMode::PreprocessingOnly => {
                "PreprocessingOnly (GPU uniforms, CPU draw calls)"
            }
            GpuPreprocessingMode::Culling => {
                "Culling (full GPU frustum culling + multi_draw_indirect)"
            }
        };
        console_logger::one(
            LogSev::Warn,
            LogAbout::Performance,
            &format!("[GPU Batching] GpuPreprocessingMode = {mode_str}"),
        );
    }
}

fn find_diag_value(diagnostics: &DiagnosticsStore, path: &str) -> Option<f64> {
    diagnostics
        .iter()
        .find(|diag| diag.path().as_str() == path)
        .and_then(|diag| diag.smoothed().or_else(|| diag.value()))
}

fn find_render_stat_total(diagnostics: &DiagnosticsStore, stat_suffix: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut found = false;
    for diag in diagnostics.iter() {
        let path = diag.path().as_str();
        if path.starts_with("render/") && path.ends_with(stat_suffix) {
            if let Some(value) = diag.smoothed().or_else(|| diag.value()) {
                total += value;
                found = true;
            }
        }
    }
    found.then_some(total)
}

fn format_ms(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2} ms"))
        .unwrap_or_else(|| "--".to_string())
}

fn format_count(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "--".to_string();
    };

    if value >= 1_000_000.0 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.2}K", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn format_opt_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn sys_dump_configurable_diagnostics(
    time: Res<Time>,
    settings: Res<Settings>,
    diagnostics: Res<DiagnosticsStore>,
    runtime: Res<WorldmapRuntimeDiagnostics>,
    system_timers: Res<WorldmapSystemDiagnostics>,
    cross_app_timers: Res<WorldmapCrossAppDiagnostics>,
    land_upload_telemetry: Res<crate::core::render::scene::world::land::LandUploadTelemetry>,
    mut dump_state: ResMut<DiagnosticDumpState>,
) {
    let config = &settings.worldmap_rendering.diagnostics;
    let interval_sec = config.dump_interval_sec.max(0.1);
    if (dump_state.interval_sec - interval_sec).abs() > f32::EPSILON {
        dump_state.interval_sec = interval_sec;
        dump_state.timer = Timer::from_seconds(interval_sec, TimerMode::Repeating);
    }

    if !config.dump_to_console {
        dump_state.timer.reset();
        return;
    }

    dump_state.timer.tick(time.delta());
    if !dump_state.timer.just_finished() {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diag| diag.smoothed().or_else(|| diag.value()))
        .map(|fps| format!("{fps:.0}"))
        .unwrap_or_else(|| "--".to_string());
    let frame_time = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|diag| diag.smoothed().or_else(|| diag.value()));

    let upload_snapshot = land_upload_telemetry.snapshot();
    let cross_app_snapshot = cross_app_timers.snapshot();

    let mut lines = Vec::with_capacity(4);
    lines.push(format!(
        "Worldmap diag | fps {fps} | frame {}",
        format_ms(frame_time),
    ));

    if config.log_render_breakdown {
        let main_opaque_gpu =
            find_diag_value(&diagnostics, "render/main_opaque_pass_3d/elapsed_gpu");
        let upscaling_gpu = find_diag_value(&diagnostics, "render/upscaling/elapsed_gpu");
        let total_gpu = find_render_stat_total(&diagnostics, "elapsed_gpu");
        let main_frag = find_diag_value(
            &diagnostics,
            "render/main_opaque_pass_3d/fragment_shader_invocations",
        );
        let upscaling_frag =
            find_diag_value(&diagnostics, "render/upscaling/fragment_shader_invocations");

        lines.push(format!(
            "Render | total gpu {} | main3d {} | upscale {} | frag main3d {} | frag upscale {}",
            format_ms(total_gpu),
            format_ms(main_opaque_gpu),
            format_ms(upscaling_gpu),
            format_count(main_frag),
            format_count(upscaling_frag),
        ));
    }

    if config.log_world_state {
        let upload_backlog = if upload_snapshot.pending_ops > 0 || upload_snapshot.pending_bytes > 0
        {
            format!(
                " | upload backlog {} ops / {} bytes",
                upload_snapshot.pending_ops, upload_snapshot.pending_bytes,
            )
        } else {
            String::new()
        };
        lines.push(format!(
            "World | map {} | live {} | target {} | desired {} | committed {} | transition {} | pending spawns {} | pending despawns {}{}",
            runtime.map_id,
            runtime.live_chunks,
            runtime.visible_chunk_target,
            format_opt_u32(runtime.desired_scale),
            format_opt_u32(runtime.committed_scale),
            format_opt_u32(runtime.transition_target_scale),
            runtime.pending_spawns,
            runtime.pending_despawns,
            upload_backlog,
        ));
    }

    if config.log_system_timers {
        let tracked_main_last_ms =
            (system_timers.chunk_sync.last_us + system_timers.chunk_draw.last_us) as f64 / 1000.0;
        let frame_gap_ms = frame_time.map(|value| (value - tracked_main_last_ms).max(0.0));

        lines.push(format!(
            "CPU | {}: {} | {}: {}",
            WorldmapTimedSystem::ChunkSync.label(),
            system_timers.chunk_sync.format_summary(),
            WorldmapTimedSystem::ChunkDraw.label(),
            system_timers.chunk_draw.format_summary(),
        ));
        lines.push(format!(
            "Aux | {}: {} | {}: {} | {}: {} | {}: {} | {}: {}",
            WorldmapCrossAppTimedSystem::AtlasStage.label(),
            cross_app_snapshot.atlas_stage.format_summary(),
            WorldmapCrossAppTimedSystem::AtlasExtract.label(),
            cross_app_snapshot.atlas_extract.format_summary(),
            WorldmapCrossAppTimedSystem::AtlasUpload.label(),
            cross_app_snapshot.atlas_upload.format_summary(),
            WorldmapCrossAppTimedSystem::TextureArrayExtract.label(),
            cross_app_snapshot.texture_array_extract.format_summary(),
            WorldmapCrossAppTimedSystem::TextureArrayUpload.label(),
            cross_app_snapshot.texture_array_upload.format_summary(),
        ));
        lines.push(format!(
            "RenderCPU | {}: {} | {}: {} | {}: {} | {}: {} | {}: {} | {}: {}",
            WorldmapCrossAppTimedSystem::RenderExtractCommands.label(),
            cross_app_snapshot.render_extract_commands.format_summary(),
            WorldmapCrossAppTimedSystem::RenderQueue.label(),
            cross_app_snapshot.render_queue.format_summary(),
            WorldmapCrossAppTimedSystem::RenderPhaseSort.label(),
            cross_app_snapshot.render_phase_sort.format_summary(),
            WorldmapCrossAppTimedSystem::RenderPrepare.label(),
            cross_app_snapshot.render_prepare.format_summary(),
            WorldmapCrossAppTimedSystem::RenderExecute.label(),
            cross_app_snapshot.render_execute.format_summary(),
            WorldmapCrossAppTimedSystem::RenderCleanup.label(),
            cross_app_snapshot.render_cleanup.format_summary(),
        ));
        lines.push(format!(
            "Frame gap | main-world tracked {:.2} ms | residual {}",
            tracked_main_last_ms,
            format_ms(frame_gap_ms),
        ));
    }

    console_logger::one(
        LogSev::Diagnostics,
        LogAbout::Performance,
        &lines.join("\n"),
    );
}
