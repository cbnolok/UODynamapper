use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::color::Srgba;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::text::LineHeight;
use std::fs;
use sysinfo::{ProcessesToUpdate, System};
use uocf::geo::land_texture_2d::LandTextureSize;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory6,
    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
    DXGI_QUERY_VIDEO_MEMORY_INFO,
};

// How often to refresh the sysinfo data. Read at this interval from sysinfo,
// refreshing every frame would be expensive and unnecessary.
const SYSINFO_REFRESH_INTERVAL_SEC: f32 = 1.0;
const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;

// Keep these in sync with texture/atlas initialization in terrain cache startup.
const TILE_ATLAS_TEXELS: u32 = 2048;
const TILE_ATLAS_MAX_LAYERS: u32 = 16;

/// Holds the cached sysinfo `System` instance and a cooldown timer for polling.
/// We reuse the same `System` instead of re-creating it each frame — sysinfo reads
/// /proc/ files under the hood and creating a System is not free.
#[derive(Resource)]
pub struct ProcessMetrics {
    /// The sysinfo handle.
    sys: System,
    /// PID of the current process.
    pid: sysinfo::Pid,
    /// Timer to throttle the update frequency.
    poll_timer: Timer,
    /// Latest CPU usage normalized to total machine capacity (0..100%).
    pub cpu_usage_total: f32,
    /// Latest process CPU usage in "single core equivalents" (can exceed 100 on multicore).
    pub cpu_usage_one_core: f32,
    /// Latest RAM usage (MiB).
    pub mem_usage_mib: f32,
    /// Estimated terrain texture-array VRAM usage (MiB), based on selected compression.
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
        let mut sys = System::new();
        sys.refresh_all();
        let pid = sysinfo::get_current_pid().expect("Failed to get current check PID");
        Self {
            sys,
            pid,
            poll_timer: Timer::from_seconds(SYSINFO_REFRESH_INTERVAL_SEC, TimerMode::Repeating),
            cpu_usage_total: 0.0,
            cpu_usage_one_core: 0.0,
            mem_usage_mib: 0.0,
            estimated_texture_vram_mib: 0.0,
            estimated_atlas_vram_mib: 0.0,
            process_vram_tracked_mib: 0.0,
            core_count: 0,
        }
    }
}

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
    let entries = fs::read_dir(fdinfo_dir).ok()?;

    let mut total_kib: f32 = 0.0;
    let mut found_any_counter = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(contents) = fs::read_to_string(path) else {
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

pub struct PerformanceOverlayPlugin;

impl Plugin for PerformanceOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProcessMetrics>()
            .add_systems(
                Startup,
                setup_overlay_performance.in_set(StartupSysSet::SetupSceneStage2),
            )
            .add_systems(
                Update,
                (sys_refresh_process_metrics, update_performance_text)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
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
            let scale = settings.app.window.overlay_scale;
            builder.spawn((
                Text::new("FPS: Init..."),
                TextFont {
                    font,
                    font_size: 12.0 * scale,
                    ..default()
                },
                LineHeight::Px(12.0 * scale),
                TextColor(Srgba::hex("00FF00").unwrap().into()), // Retro green
                TextLayout::default(),
                Node::default(),
                OverlayPerformanceText,
            ));
        });
}

/// Polls the sysinfo library to get up-to-date CPU and RAM usage for the current process.
pub fn sys_refresh_process_metrics(
    time: Res<Time>,
    settings: Res<Settings>,
    mut metrics: ResMut<ProcessMetrics>,
) {
    metrics.poll_timer.tick(time.delta());
    if !metrics.poll_timer.just_finished() {
        return;
    }

    let pid = metrics.pid;
    metrics
        .sys
        .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

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

        let lossy = settings.core.graphics.lossy_texture_compression;
        let small_bytes = crate::core::texture_cache::land::texture_array::bytes_per_layer(
            LandTextureSize::Small,
            lossy,
        )
            * crate::core::texture_cache::land::texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS
                as usize;
        let big_bytes = crate::core::texture_cache::land::texture_array::bytes_per_layer(
            LandTextureSize::Big,
            lossy,
        )
            * crate::core::texture_cache::land::texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS
                as usize;
        // Rg16Uint = 4 bytes/texel.
        let atlas_bytes = (TILE_ATLAS_TEXELS as usize
            * TILE_ATLAS_TEXELS as usize
            * TILE_ATLAS_MAX_LAYERS as usize)
            * 4usize;

        metrics.estimated_texture_vram_mib = (small_bytes + big_bytes) as f32 / BYTES_PER_MIB;
        metrics.estimated_atlas_vram_mib = atlas_bytes as f32 / BYTES_PER_MIB;

        // Cross-platform tracked process VRAM: app-owned persistent GPU allocations.
        // Includes terrain texture arrays + tile metadata atlas.
        // Prefer native OS/driver accounting when available.
        let tracked_total_mib =
            metrics.estimated_texture_vram_mib + metrics.estimated_atlas_vram_mib;
        metrics.process_vram_tracked_mib =
            query_process_vram_mib_native(pid).unwrap_or(tracked_total_mib);
    }
}

/// Updates the on-screen text widget with latest metrics.
pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    settings: Res<Settings>,
    metrics: Res<ProcessMetrics>,
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
) {
    let current_scale = settings.app.window.overlay_scale;
    let scale_changed = (*last_scale - current_scale).abs() > 0.001;

    // Real-time visibility toggle from settings
    if let Ok(mut node) = node_query.single_mut() {
        let target_display = if settings.app.performance.show_overlay {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != target_display {
            node.display = target_display;
        }
    }

    if let Ok((mut text, mut text_font, mut line_height)) = text_query.single_mut() {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.0}", val))
            .unwrap_or_else(|| "--".to_string());

        let entity_count = entities.len();
        let chunk_count = land_chunk_count.0;
        let tex_mode = if settings.core.graphics.lossy_texture_compression {
            "BC7"
        } else {
            "RGBA8"
        };

        // Read render pipeline statistics from RenderDiagnosticsPlugin.
        // Paths are dynamic strings: "render/{span_name}/{stat}".
        // The span names are logged at startup via LogDiagnosticsPlugin.
        // On Vulkan, these are populated via GPU pipeline query objects.
        let vert_invoc = find_render_stat(&diagnostics, "vertex_shader_invocations");
        let frag_invoc = find_render_stat(&diagnostics, "fragment_shader_invocations");
        let clipper_in = find_render_stat(&diagnostics, "clipper_invocations");
        let clipper_out = find_render_stat(&diagnostics, "clipper_primitives_out");
        let gpu_elapsed = find_render_stat(&diagnostics, "elapsed_gpu");

        let render_stats = format!(
            "GPU elapsed: {} | Clipper in/out: {}/{} | Vert/Frag calls: {}/{}",
            gpu_elapsed, clipper_in, clipper_out, vert_invoc, frag_invoc,
        );

        text.0 = format!(
            "FPS: {}\nCPU(total): {:.1}% | CPU(proc, 1c-eq): {:.1}% | cores: {}\nRAM: {:.1} MiB\nTex VRAM est [{}]: {:.1} MiB | Atlas est: {:.1} MiB\nProcess VRAM tracked: {:.1} MiB\nCHKs: {} | ENTs: {}\n{}",
            fps,
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
            render_stats,
        );

        if scale_changed {
            text_font.font_size = 14.0 * current_scale;
            *line_height = LineHeight::Px(14.0 * current_scale);
            *last_scale = current_scale;
        }
    }
}

/// Sums all render diagnostics whose path ends with `stat_suffix` across all render spans.
/// Returns a formatted string with the total, or "--" if no data is available.
/// This is needed because Bevy 0.18 uses dynamic path strings for render diagnostics
/// (e.g. "render/main_opaque_pass/fragment_shader_invocations") with no public constants.
fn find_render_stat(diagnostics: &DiagnosticsStore, stat_suffix: &str) -> String {
    let mut total: f64 = 0.0;
    let mut found = false;
    for diag in diagnostics.iter() {
        let path = diag.path().as_str();
        if path.starts_with("render/") && path.ends_with(stat_suffix) {
            if let Some(val) = diag.value() {
                total += val;
                found = true;
            }
        }
    }
    if found {
        if stat_suffix.contains("elapsed") {
            format!("{total:.2}ms")
        } else {
            // Large numbers: format with K/M suffix for readability
            if total >= 1_000_000.0 {
                format!("{:.1}M", total / 1_000_000.0)
            } else if total >= 1_000.0 {
                format!("{:.1}K", total / 1_000.0)
            } else {
                format!("{total:.0}")
            }
        }
    } else {
        "--".to_string()
    }
}
