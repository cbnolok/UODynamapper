pub mod app_states;
pub mod constants;
mod maps;
mod render;
pub mod system_sets;
mod texture_cache;
mod uo_files_loader;

use crate::prelude::*;
use bevy::{
    //ecs::schedule::ExecutorKind,
    pbr::wireframe::{WireframeConfig, WireframePlugin},
    prelude::*,
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuFeatures, WgpuSettings},
    },
    window::WindowResolution,
    winit::{UpdateMode, WinitSettings},
};
use bevy_framepace::FramepacePlugin;
use std::{process::ExitCode, time::Duration};
use system_sets::*;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub fn run_bevy_app() -> ExitCode {
    // Install the custom log subscriber (must come BEFORE Bevy app launch!)
    //  to change the default Bevy log format.
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(true) // colored output like Bevy default
                .with_level(true)
                .with_target(true)
                // Use chrono for timestamp, format with NO milliseconds
                .with_timer(fmt::time::ChronoLocal::new("%Y-%m-%d %H:%M:%s".into()))
                .compact(), // Looks a lot like Bevy default (use .pretty() for multiline pretty logs)
        )
        .with(EnvFilter::from_default_env())
        .init();

    log_appstate_change("LoadStartupFiles");

    let result = App::new()
    .insert_resource(WinitSettings {
        focused_mode: UpdateMode::reactive(Duration::from_secs_f64(1.0 / 244.0)),
        unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 60.0)), /* 60Hz, */
    })
    .add_plugins(
        DefaultPlugins
        .build()
        .disable::<bevy::log::LogPlugin>()
        .set(ImagePlugin::default_linear())
        .set(
            WindowPlugin {
                primary_window: Some(Window {
                    title: "UODynamapper".to_string(),
                    resizable: true,
                    // Force 1:1 aspect for virtual rendering (game world)
                    // UO requires 'virtual' 44×44 diamonds, so...
                    resolution: WindowResolution::new(1320.0, 924.0), // (44*30)x(44*21), etc
                    resize_constraints: WindowResizeConstraints {
                        min_width: 44.0 * 10.0,
                        min_height: 44.0 * 10.0,
                        ..Default::default()
                    },
                    // Let window freely resize, but camera+scene SYSTEMS keep virtual grid and diamonds fixed.
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .set(RenderPlugin {
            render_creation: RenderCreation::Automatic(WgpuSettings {
                features: WgpuFeatures::POLYGON_MODE_LINE, // Required for wireframe
                ..Default::default()
            }),
            ..Default::default()
        },
    ))
    //.add_plugins(WireframePlugin::default()) // Needed enable wireframe rendering
    // Wireframes can be configured with this resource. This can be changed at runtime.
    .insert_resource(WireframeConfig {
        // The global wireframe config enables drawing of wireframes on every mesh,
        // except those with `NoWireframe`. Meshes with `Wireframe` will always have a wireframe,
        // regardless of the global configuration.
        global: true,
        // Controls the default color of all wireframes. Used as the default color for global wireframes.
        // Can be changed per mesh using the `WireframeColor` component.
        default_color: Color::srgb_from_array(bevy::color::palettes::css::WHITE.to_f32_array_no_alpha()),
    })
    /*
    .edit_schedule(Update, |schedule| {
    schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    })
    */
    .add_plugins(FramepacePlugin) // caps at 60 FPS by default
    //.use(bevy_framepace::FramepaceSettings::default().with_framerate(30.0))
    .init_state::<AppState>()
    .insert_state(AppState::LoadStartupFiles)
    .add_plugins((
        render::RenderPlugin { registered_by: "Core" },
        texture_cache::TextureCachePlugin { registered_by: "Core" },
        uo_files_loader::UoFilesPlugin { registered_by: "Core" },
    ))
    .configure_sets(Startup, (StartupSysSet::SetupScene,))
    .add_systems(
        OnEnter(AppState::SetupScene),
        advance_state_after_scene_setup.after(StartupSysSet::SetupScene),
    )
    .run();

    match result {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(value) => ExitCode::from(value.get()),
    }
}

fn advance_state_after_scene_setup(mut next_state: ResMut<NextState<AppState>>) {
    log_appstate_change("InGame");
    next_state.set(AppState::InGame);
}
