use crate::core::render::scene::camera::UiCameraResource;
use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::color::Srgba;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::text::LineHeight;
use sysinfo::{ProcessesToUpdate, System};

// How often to refresh the sysinfo data. Read at this interval from sysinfo,
// refreshing every frame would be expensive and unnecessary.
const SYSINFO_REFRESH_INTERVAL_SEC: f32 = 1.0;

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
    /// Latest CPU usage (percentage 0..100).
    pub cpu_usage: f32,
    /// Latest RAM usage (MiB).
    pub mem_usage_mib: f32,
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
pub struct OverlayPerformanceContainer;

#[derive(Component)]
pub struct OverlayPerformanceText;

pub fn setup_overlay_performance(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<Settings>,
    ui_camera: Res<UiCameraResource>,
) {
    println!("DEBUG: setup_overlay_performance running");
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
            ZIndex(100),
            OverlayPerformanceContainer,
            UiTargetCamera(ui_camera.0.unwrap()),
        ))
        .with_children(|builder| {
            builder.spawn((
                Text::new("FPS: Init..."),
                TextFont {
                    font,
                    font_size: 14.0,
                    ..default()
                },
                LineHeight::Px(14.0),
                TextColor(Srgba::hex("00FF00").unwrap().into()), // Retro green
                Node::default(), // Required for Bevy UI to recognize and render the text
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

    let pid = metrics.pid;
    metrics
        .sys
        .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

    if let Some(process) = metrics.sys.process(pid) {
        let cpu = process.cpu_usage();
        const BYTES_PER_MIB: f32 = 1024.0 * 1024.0;
        let mem = process.memory() as f32 / BYTES_PER_MIB;

        // sysinfo process cpu usage is [0..100 * num_cpus].
        // Normalize it by dividing by core count for a "standard" 0..100% total system load.
        let core_count = metrics.sys.cpus().len() as f32;
        metrics.cpu_usage = if core_count > 0.0 {
            cpu / core_count
        } else {
            cpu
        };
        metrics.mem_usage_mib = mem;
    }
}

/// Updates the on-screen text widget with latest metrics.
pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    settings: Res<Settings>,
    metrics: Res<ProcessMetrics>,
    entities: &bevy::ecs::entity::Entities,
    land_chunks: Query<&crate::core::render::scene::world::land::LCMesh>,
    mut text_query: Query<&mut Text, With<OverlayPerformanceText>>,
    mut node_query: Query<
        &mut Node,
        (
            With<OverlayPerformanceContainer>,
            Without<OverlayPerformanceText>,
        ),
    >,
) {
    // Real-time visibility toggle from settings
    if let Some(mut node) = node_query.single_mut().ok() {
        let target_display = if settings.app.performance.show_overlay {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != target_display {
            println!(
                "DEBUG: performance overlay display change to {:?}",
                target_display
            );
            node.display = target_display;
        }
    } else {
        println!("DEBUG: performance overlay node NOT FOUND");
    }

    if let Some(mut text) = text_query.single_mut().ok() {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.0}", val))
            .unwrap_or_else(|| "--".to_string());

        let entity_count = entities.len();
        // Since land_chunks query is filtered, we still need to count,
        // but this set is much smaller than all entities.
        let chunk_count = land_chunks.iter().count();

        text.0 = format!(
            "FPS: {}\nCPU: {:.1}% | RAM: {:.1} MiB\nCHKs: {} | ENTs: {}",
            fps, metrics.cpu_usage, metrics.mem_usage_mib, chunk_count, entity_count
        );
    }
}
