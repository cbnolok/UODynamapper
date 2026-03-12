use crate::{core::system_sets::StartupSysSet, prelude::*};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin, SystemInformationDiagnosticsPlugin};
use bevy::prelude::*;

pub struct PerformanceOverlayPlugin;

impl Plugin for PerformanceOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            setup_overlay_performance.in_set(StartupSysSet::SetupSceneStage2),
        )
        .add_systems(
            Update,
            update_performance_text.run_if(in_state(AppState::InGame)),
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

pub fn update_performance_text(
    diagnostics: Res<DiagnosticsStore>,
    mut text_query: Query<&mut Text, With<OverlayPerformanceText>>,
) {
    if let Ok(mut text) = text_query.single_mut() {
        let fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.1}", val))
            .unwrap_or_else(|| "--".to_string());

        let cpu = diagnostics
            .get(&SystemInformationDiagnosticsPlugin::SYSTEM_CPU_USAGE)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.1}", val))
            .unwrap_or_else(|| "--".to_string());

        let mem = diagnostics
            .get(&SystemInformationDiagnosticsPlugin::SYSTEM_MEM_USAGE)
            .and_then(|diag| diag.smoothed())
            .map(|val| format!("{:.0}", val))
            .unwrap_or_else(|| "--".to_string());

        *text = Text::new(format!("FPS: {}\nCPU: {}%\nRAM: {}MB", fps, cpu, mem));
    }
}
