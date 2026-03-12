use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::color::Srgba;
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

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(0.7)),
            ZIndex(100),
        ))
        .with_children(|builder| {
            builder.spawn((
                Text::new(""),
                TextFont {
                    font,
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Srgba::hex("00FF00").unwrap().into()), // Retro green
                OverlayPerformanceText,
            ));
        });
}

/// Polls the sysinfo library to get up-to-date CPU and RAM usage for the current process.
pub fn sys_refresh_process_metrics(time: Res<Time>, mut metrics: ResMut<ProcessMetrics>) {
    metrics.poll_timer.tick(time.delta());
    if !metrics.poll_timer.just_finished() {
        return;
    }

    metrics.sys.refresh_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );

    metrics.cpu_usage = metrics.sys.global_cpu_usage();
    const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;
    metrics.mem_usage_mib = metrics.sys.used_memory() as f32 / BYTES_PER_MIB;
}

/// Updates the on-screen text widget with latest metrics.
pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    _settings: Res<Settings>,
    metrics: Res<ProcessMetrics>,
    entities: Query<Entity>,
    land_chunks: Query<&crate::core::render::scene::world::land::LCMesh>,
    mut text_query: Query<&mut Text, With<OverlayPerformanceText>>,
) {
    if let Ok(mut text) = text_query.get_single_mut() {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.0}", val))
            .unwrap_or_else(|| "--".to_string());

        let entity_count = entities.iter().count();
        let chunk_count = land_chunks.iter().count();

        text.0 = format!(
            "FPS: {}   CPU: {:.0}%   RAM: {} MiB   ENTs: {}   CHKs: {}",
            fps, metrics.cpu_usage, metrics.mem_usage_mib, entity_count, chunk_count
        );
    }
}
