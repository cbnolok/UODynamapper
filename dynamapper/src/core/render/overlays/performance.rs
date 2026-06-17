use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::color::Srgba;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::text::LineHeight;
//use bevy::time::common_conditions::on_timer;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use uocf::classic::land_texture::LandTextureSize;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory6,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
    DXGI_QUERY_VIDEO_MEMORY_INFO,
};

const FONT_SIZE: f32 = 11.0;

// How often to refresh the sysinfo data. Read at this interval from sysinfo,
// refreshing every frame would be expensive and unnecessary.
const METRICS_REFRESH_INTERVAL_SEC: f32 = 1.0 / 4.0;
const TEXT_REFRESH_INTERVAL_SEC: f32 = 1.0;
const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;

use crate::core::texture_cache::land::texture_array as tex_consts;

/// Holds the cached sysinfo `System` instance and a cooldown timer for polling.
/// We reuse the same `System` instead of re-creating it each frame — sysinfo reads
/// /proc/ files under the hood and creating a System is not free.
#[derive(Resource)]
pub struct ProcessMetrics {
    /// The sysinfo handle.
    sys: System,
    /// PID of the current process.
    pid: sysinfo::Pid,
    /// Latest CPU usage normalized to total machine capacity (0..100%).
    pub cpu_usage_total: f32,
    /// Latest process CPU usage in "single core equivalents" (can exceed 100 on multicore).
    pub cpu_usage_one_core: f32,
    /// Latest RAM usage (MiB).
    pub mem_usage_mib: f32,
    /// Estimated terrain texture-array VRAM usage (MiB).
    pub estimated_texture_vram_mib: f32,
    /// Estimated tile-meta-atlas VRAM usage (MiB).
    pub estimated_atlas_vram_mib: f32,
    /// Cross-platform tracked process VRAM usage (MiB), from app-owned GPU allocations.
    pub process_vram_tracked_mib: f32,
    /// Number of logical CPU cores.
    pub core_count: usize,
}

impl Default for ProcessMetrics {
    fn default() -> Self {
        let sys = sysinfo::System::new_all();
        let pid = sysinfo::get_current_pid().expect("Failed to get PID");

        Self {
            sys,
            pid,
            cpu_usage_total: 0.0,
            cpu_usage_one_core: 0.0,
            mem_usage_mib: 0.0,
            core_count: 0,
            estimated_texture_vram_mib: 0.0,
            estimated_atlas_vram_mib: 0.0,
            process_vram_tracked_mib: 0.0,
        }
    }
}

// TODO: Add Android, iOS, iPadOS etc.
// TODO: double check and ensure that this is legit and works also for MacOS and similar, since they do not have perfect posix compliance.
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "macos"
))]
fn query_process_vram_mib_linux_drm(pid: sysinfo::Pid) -> Option<f32> {
    // Linux kernel exposes per-process DRM memory stats under:
    //   /proc/<pid>/fdinfo/<fd>
    // We sum VRAM-like counters across all DRM fds for this process.
    // Typical keys:
    //   drm-memory-vram: <KiB> kB
    //   drm-memory-local: <KiB> kB
    // TODO: can we extract anything else from here? GTT/system memory? More precise than other ways?
    let fdinfo_dir = format!("/proc/{}/fdinfo", pid);
    let entries = std::fs::read_dir(fdinfo_dir).ok()?;

    let mut total_kib: f32 = 0.0;
    let mut found_any_counter = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };

        for line in contents.lines() {
            let line = line.trim();
            // AMD: drm-memory-vram
            // Intel (Xe/i915 variants): drm-memory-local
            let is_vram_key =
                line.starts_with("drm-memory-vram:") || line.starts_with("drm-memory-local:");
            if !is_vram_key {
                continue;
            }

            // Format is generally: "key: <number> kB"
            let Some((_, rhs)) = line.split_once(':') else {
                continue;
            };
            let mut parts = rhs.split_whitespace();
            let Some(num_str) = parts.next() else {
                continue;
            };
            if let Ok(v_kib) = num_str.parse::<f32>() {
                total_kib += v_kib;
                found_any_counter = true;
            }
        }
    }

    if found_any_counter {
        Some(total_kib / 1024.0)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
fn query_process_vram_mib_windows_dxgi(_pid: sysinfo::Pid) -> Option<f32> {
    // DXGI reports memory usage for the current process on the queried adapter.
    // We read LOCAL segment usage (dedicated VRAM / local memory).
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory1().ok()?;
        let mut adapter_index: u32 = 0;
        let mut best_mib = None::<f32>;

        loop {
            let adapter: IDXGIAdapter1 = match factory
                .EnumAdapterByGpuPreference(adapter_index, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
            {
                Ok(a) => a,
                Err(_) => break,
            };
            adapter_index += 1;

            use windows::core::Interface; // bring trait in scope in order to use cast().
            let adapter3: IDXGIAdapter3 = match adapter.cast() {
                Ok(a3) => a3,
                Err(_) => continue,
            };

            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if adapter3
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
                .is_ok()
            {
                let mib = info.CurrentUsage as f32 / BYTES_PER_MIB;
                best_mib = Some(best_mib.map_or(mib, |v| v.max(mib)));
            }
        }

        best_mib
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn query_process_vram_mib_windows_dxgi(_pid: sysinfo::Pid) -> Option<f32> {
    None
}

fn query_process_vram_mib_native(pid: sysinfo::Pid) -> Option<f32> {
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "macos"
    ))]
    {
        if let Some(v) = query_process_vram_mib_linux_drm(pid) {
            return Some(v);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(v) = query_process_vram_mib_windows_dxgi(pid) {
            return Some(v);
        }
    }

    None
}

// ----

use bevy::time::common_conditions::on_real_timer;
use std::time::Duration;

pub struct PerformanceOverlayPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(PerformanceOverlayPlugin);

impl Plugin for PerformanceOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProcessMetrics>()
            .add_systems(
                Startup,
                setup_overlay_performance.in_set(StartupSysSet::SetupSceneStage2),
            )
            .add_systems(
                Update,
                sys_refresh_process_metrics
                    .run_if(in_state(AppState::InGame))
                    .run_if(on_real_timer(Duration::from_secs_f32(METRICS_REFRESH_INTERVAL_SEC))),
            )
            .add_systems(
                Update,
                update_performance_text
                    .run_if(in_state(AppState::InGame))
                    .run_if(on_real_timer(Duration::from_secs_f32(TEXT_REFRESH_INTERVAL_SEC))),
            );
    }
}

#[derive(Component)]
pub struct OverlayPerformanceContainer;

#[derive(Component)]
pub struct OverlayPerformanceText;

pub fn setup_overlay_performance(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<Settings>,
) {
    log_system_add_startup::<PerformanceOverlayPlugin>(StartupSysSet::SetupSceneStage2, fname!());
    let font: Handle<Font> = asset_server.load("fonts/fira/FiraMono-Medium.ttf");
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::all(Val::Px(10.0)),
                display: if settings.app.performance.show_overlay {
                    Display::Flex
                } else {
                    Display::None
                },
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GlobalZIndex(100),
            OverlayPerformanceContainer,
        ))
        .with_children(|builder| {
            let scale = settings.app.window.performance_overlay_scale;
            builder.spawn((
                Text::new("FPS: Init..."),
                TextFont {
                    font,
                    font_size: FONT_SIZE * scale,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Srgba::hex("00FF00").unwrap().into()), // Retro green
                TextLayout::default(),
                Node::default(),
                OverlayPerformanceText,
            ));
        });
}

/// Polls the sysinfo library to get up-to-date CPU and RAM usage for the current process.
pub fn sys_refresh_process_metrics(
    _settings: Res<Settings>,
    mut metrics: ResMut<ProcessMetrics>,
    tex_cache: Res<crate::core::texture_cache::land::cache::LandTextureCache>,
    tile_atlas: Res<crate::core::render::scene::world::land::tile_atlas::TileAtlas>,
) {
    let pid = metrics.pid;
    // 1. Refresh global CPU usage. This is required on Windows to update the system time baseline
    // used to calculate process CPU usage as a delta.
    metrics.sys.refresh_cpu_usage();

    // 2. Refresh specific process using recommended specifics for accuracy and performance.
    metrics.sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true, // remove_dead_processes
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );

    if let Some(process) = metrics.sys.process(pid) {
        let cpu = process.cpu_usage();
        let mem = process.memory() as f32 / BYTES_PER_MIB;

        // sysinfo process cpu usage is [0..100 * num_cpus].
        // Normalize it by dividing by core count for a "standard" 0..100% total system load.
        let core_count = metrics.sys.cpus().len();
        let core_count_f = core_count as f32;
        metrics.cpu_usage_total = if core_count_f > 0.0 {
            cpu / core_count_f
        } else {
            cpu
        };
        metrics.cpu_usage_one_core = cpu;
        metrics.core_count = core_count;
        metrics.mem_usage_mib = mem;

        // Use ACTUAL current layer counts, not theoretical maximums.
        let small_bytes = tex_consts::bytes_per_layer(LandTextureSize::Small)
            * tex_cache.small.active_layers as usize;
        let big_bytes = tex_consts::bytes_per_layer(LandTextureSize::Big)
            * tex_cache.big.active_layers as usize;

        // Tile metadata atlas: Rg16Uint = TILE_ATLAS_BYTES_PER_TEXEL bytes/texel.
        let atlas_layers = tile_atlas.params.max_layers as usize;
        let atlas_bytes = (tex_consts::TILE_ATLAS_PAGE_TEXELS as usize)
            * (tex_consts::TILE_ATLAS_PAGE_TEXELS as usize)
            * atlas_layers
            * (tex_consts::TILE_ATLAS_BYTES_PER_TEXEL as usize);

        metrics.estimated_texture_vram_mib = (small_bytes + big_bytes) as f32 / BYTES_PER_MIB;
        metrics.estimated_atlas_vram_mib = atlas_bytes as f32 / BYTES_PER_MIB;

        // Cross-platform tracked process VRAM: app-owned persistent GPU allocations.
        // Includes terrain texture arrays + tile metadata atlas.
        metrics.process_vram_tracked_mib = query_process_vram_mib_native(pid).unwrap_or(0.0);
    }
}

/// Updates the on-screen text widget with latest metrics.
pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    settings: Res<Settings>,
    metrics: Res<ProcessMetrics>,
    land_upload_telemetry: Res<crate::core::render::scene::world::land::LandUploadTelemetry>,
    entities: &bevy::ecs::entity::Entities,
    land_chunk_count: Res<crate::core::render::scene::LandChunkCount>,
    mut text_query: Query<
        (&mut Text, &mut TextFont, &mut LineHeight),
        With<OverlayPerformanceText>,
    >,
    mut node_query: Query<
        &mut Node,
        (
            With<OverlayPerformanceContainer>,
            Without<OverlayPerformanceText>,
        ),
    >,
    mut last_scale: Local<f32>,
    mut last_text_cached: Local<String>,
) {
    let show_overlay = settings.app.performance.show_overlay;

    // 1. Real-time visibility toggle from settings
    if let Ok(mut node) = node_query.single_mut() {
        let target_display = if show_overlay {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != target_display {
            node.display = target_display;
        }
    }

    // 2. Early exit if not showing
    if !show_overlay {
        return;
    }

    let current_scale = settings.app.window.performance_overlay_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    use std::fmt::Write;
    let mut buffer = String::with_capacity(1024);

    let fps_val = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diag| diag.smoothed());

    let entity_count = entities.len();
    let chunk_count = land_chunk_count.0;
    let upload_snapshot = land_upload_telemetry.snapshot();
    let tex_mode = "RGBA8";

    // Single-pass render diagnostic scanning
    let stats = BatchRenderStats::scan(&diagnostics);

    let _ = write!(
        &mut buffer,
        "FPS: {}\nCPU(total): {:.1}% | CPU(proc, 1c-eq): {:.1}% | cores: {}\nRAM: {:.1} MiB\nTex VRAM est [{}]: {:.1} MiB | Atlas est: {:.1} MiB\nProcess VRAM tracked: {:.1} MiB\nCHKs: {} | ENTs: {}\n",
        fps_val.map_or("--".to_string(), |v| format!("{:.0}", v)),
        metrics.cpu_usage_total,
        metrics.cpu_usage_one_core,
        metrics.core_count,
        metrics.mem_usage_mib,
        tex_mode,
        metrics.estimated_texture_vram_mib,
        metrics.estimated_atlas_vram_mib,
        metrics.process_vram_tracked_mib,
        chunk_count,
        entity_count,
    );

    let _ = writeln!(
        &mut buffer,
        "Land uploads: dirty {} -> queued {} ({}) | submitted {} ({}) | backlog {} ({})",
        upload_snapshot.dirty_block_updates,
        upload_snapshot.queued_ops,
        ByteSizeFormatter(upload_snapshot.queued_bytes),
        upload_snapshot.submitted_ops,
        ByteSizeFormatter(upload_snapshot.submitted_bytes),
        upload_snapshot.pending_ops,
        ByteSizeFormatter(upload_snapshot.pending_bytes),
    );

    if stats.found {
        let _ = write!(
            &mut buffer,
            "GPU elapsed: {:.2}ms | Clipper in/out: {}/{} | Vert/Frag calls: {}/{}",
            stats.gpu_elapsed,
            CountFormatter(stats.clipper_in),
            CountFormatter(stats.clipper_out),
            CountFormatter(stats.vert_invoc),
            CountFormatter(stats.frag_invoc),
        );
    } else {
        let _ = write!(&mut buffer, "GPU stats: --");
    }

    if *last_text_cached == buffer && !scale_changed {
        return;
    }

    if let Ok((mut text, mut text_font, mut line_height)) = text_query.single_mut() {
        if *last_text_cached != buffer {
            text.0 = buffer.clone();
            *last_text_cached = buffer;
        }

        if scale_changed {
            text_font.font_size = FONT_SIZE * current_scale;
            *line_height = LineHeight::Px(FONT_SIZE * current_scale);
            *last_scale = current_scale;
        }
    }
}

struct ByteSizeFormatter(u64);
impl std::fmt::Display for ByteSizeFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const BYTES_PER_KIB: f64 = 1024.0;
        const BYTES_PER_MIB_F64: f64 = 1024.0 * 1024.0;

        let bytes_f = self.0 as f64;
        if self.0 == 0 {
            write!(f, "0 B")
        } else if bytes_f >= BYTES_PER_MIB_F64 {
            write!(f, "{:.2} MiB", bytes_f / BYTES_PER_MIB_F64)
        } else if bytes_f >= BYTES_PER_KIB {
            write!(f, "{:.1} KiB", bytes_f / BYTES_PER_KIB)
        } else {
            write!(f, "{} B", self.0)
        }
    }
}

struct CountFormatter(f64);
impl std::fmt::Display for CountFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 >= 1_000_000.0 {
            write!(f, "{:.1}M", self.0 / 1_000_000.0)
        } else if self.0 >= 1_000.0 {
            write!(f, "{:.1}K", self.0 / 1_000.0)
        } else {
            write!(f, "{:.0}", self.0)
        }
    }
}

#[derive(Default)]
struct BatchRenderStats {
    gpu_elapsed: f64,
    clipper_in: f64,
    clipper_out: f64,
    vert_invoc: f64,
    frag_invoc: f64,
    found: bool,
}

impl BatchRenderStats {
    fn scan(diagnostics: &DiagnosticsStore) -> Self {
        let mut stats = Self::default();
        for diag in diagnostics.iter() {
            let path = diag.path().as_str();
            if !path.starts_with("render/") {
                continue;
            }
            if let Some(val) = diag.value() {
                if path.ends_with("elapsed_gpu") {
                    stats.gpu_elapsed += val;
                    stats.found = true;
                } else if path.ends_with("clipper_invocations") {
                    stats.clipper_in += val;
                    stats.found = true;
                } else if path.ends_with("clipper_primitives_out") {
                    stats.clipper_out += val;
                    stats.found = true;
                } else if path.ends_with("vertex_shader_invocations") {
                    stats.vert_invoc += val;
                    stats.found = true;
                } else if path.ends_with("fragment_shader_invocations") {
                    stats.frag_invoc += val;
                    stats.found = true;
                }
            }
        }
        stats
    }
}
