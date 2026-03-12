use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

// How often to refresh the sysinfo data. Read at this interval from sysinfo,
// refreshing every frame would be expensive and unnecessary.
const SYSINFO_REFRESH_INTERVAL_SEC: f32 = 1.0;

/// Holds the cached sysinfo `System` instance and a cooldown timer for polling.
/// We reuse the same `System` instead of re-creating it each frame — sysinfo reads
/// /proc/ files under the hood and creating a System is not free.
#[derive(Resource)]
pub struct ProcessMetrics {
    /// The sysinfo handle; instantiated once and kept alive for the application lifetime.
    sys: System,
    /// Timer to throttle the update frequency.
    poll_timer: Timer,
    /// Latest CPU usage for this process (percentage 0..100).
    pub cpu_usage: f32,
    /// Latest RAM usage for this process (MiB).
    pub mem_usage_mib: f32,
}

impl Default for ProcessMetrics {
    fn default() -> Self {
        // Only enable the CPU and memory components we care about to avoid overhead.
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Self {
            sys,
            poll_timer: Timer::from_seconds(SYSINFO_REFRESH_INTERVAL_SEC, TimerMode::Repeating),
            cpu_usage: 0.0,
            mem_usage_mib: 0.0,
        }
    }
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
pub struct OverlayPerformanceText;

pub fn setup_overlay_performance(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font: Handle<Font> = asset_server.load("fonts/fira/FiraMono-Medium.ttf");

    let root_id = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            right: Val::Px(20.0),
            top: Val::Px(20.0),
            ..default()
        })
        .id();

    let bg_id = commands
        .spawn((
            Node {
                padding: UiRect::all(Val::Px(7.0)),
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(0.65)),
        ))
        .with_children(|builder| {
            builder.spawn((
                Text::new("FPS: --\nCPU: --%\nRAM: --MB"),
                TextFont {
                    font,
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                OverlayPerformanceText,
            ));
        })
        .id();

    commands.entity(root_id).add_child(bg_id);
}

/// Polls the sysinfo library to get up-to-date CPU and RAM usage for the current process.
/// This runs at most once per `SYSINFO_REFRESH_INTERVAL_SEC` to avoid hammering /proc/.
pub fn sys_refresh_process_metrics(time: Res<Time>, mut metrics: ResMut<ProcessMetrics>) {
    metrics.poll_timer.tick(time.delta());
    if !metrics.poll_timer.just_finished() {
        return;
    }

    // Refresh only CPU and RAM; other subsystems are not needed.
    metrics
        .sys
        .refresh_specifics(RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
            .with_memory(MemoryRefreshKind::nothing().with_ram()));

    // Global CPU usage is the average across all CPU cores.
    metrics.cpu_usage = metrics.sys.global_cpu_usage();

    // `used_memory` returns bytes; convert to MiB for readability.
    const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;
    metrics.mem_usage_mib = metrics.sys.used_memory() as f32 / BYTES_PER_MIB;
}

/// Updates the on-screen text widget with the latest FPS, CPU, and RAM values.
pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    metrics: Res<ProcessMetrics>,
    mut text_query: Query<&mut Text, With<OverlayPerformanceText>>,
) {
    if let Ok(mut text) = text_query.single_mut() {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.1}", val))
            .unwrap_or_else(|| "--".to_string());

        *text = Text::new(format!(
            "FPS: {}\nCPU: {:.1}%\nRAM: {:.0}MB",
            fps, metrics.cpu_usage, metrics.mem_usage_mib
        ));
    }
}
