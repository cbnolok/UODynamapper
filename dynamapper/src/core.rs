pub mod app_states;
pub mod constants;
pub mod controls;
pub mod maps;
pub mod render;
pub mod system_sets;
mod texture_cache;
mod uo_files_loader;

use crate::{
    core::app_states::*,
    logger::{self, *},
    settings,
};
use bevy::{
    //ecs::schedule::ExecutorKind,
    log::{BoxedLayer, LogPlugin},
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
use crate::core::render::scene::world::land::draw_chunk_mesh::setup_land_mesh;
use tracing_subscriber::fmt;

#[allow(unused)]
fn bevy_logging_custom_layer(_app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(
        fmt::layer()
            .with_ansi(true) // colored output like Bevy default
            .with_level(true)
            .with_target(true)
            // Use chrono for timestamp, format with NO milliseconds
            .with_timer(fmt::time::ChronoLocal::new("%H:%M:%S".into()))
            // compact() looks a lot like Bevy default
            // (use .pretty() for multiline pretty logs)
            .pretty(),
    ))
}

/*
// Work in progress for a new Bevy version?
#[allow(unused)]
fn bevy_logging_fmt_layer(_app: &mut App) -> Option<BoxedFmtLayer> {
    Some(Box::new(
        fmt::Layer::default()
            .without_time()
            .with_writer(std::io::stderr),
    ))
}
*/

fn custom_bevy_log_config() -> LogPlugin {
    LogPlugin {
        //custom_layer: bevy_logging_custom_layer,
        ..Default::default()
    }
}

fn custom_winit_settings() -> WinitSettings {
    WinitSettings {
        focused_mode: UpdateMode::reactive(Duration::from_secs_f64(1.0 / 60.0)), // 60.0 Hz
        unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 30.0)), // 30.0 Hz
    }
}

fn custom_threadpool_settings() -> TaskPoolPlugin {
    TaskPoolPlugin {
        //task_pool_options: TaskPoolOptions::with_num_threads(3),
        ..default()
    }
}

fn custom_window_plugin_settings(size: (f32, f32)) -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "UODynamapper".to_string(),
            resizable: true,
            // Force 1:1 aspect for virtual rendering (game world)
            // UO requires 'virtual' 44×44 diamonds, so...
            resolution: WindowResolution::new(size.0, size.1), //(1320.0, 924.0), // (44*30)x(44*21), etc
            resize_constraints: WindowResizeConstraints {
                min_width: 44.0 * 10.0,
                min_height: 44.0 * 10.0,
                ..Default::default()
            },
            // Let window freely resize, but camera+scene SYSTEMS keep virtual grid and diamonds fixed.
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn custom_wireframe_config(enabled: bool) -> WireframeConfig {
    // Wireframes can be configured with this resource. This can be changed at runtime.
    WireframeConfig {
        // The global wireframe config enables drawing of wireframes on every mesh,
        // except those with `NoWireframe`. Meshes with `Wireframe` will always have a wireframe,
        // regardless of the global configuration.
        global: enabled,
        // Controls the default color of all wireframes. Used as the default color for global wireframes.
        // Can be changed per mesh using the `WireframeColor` component.
        default_color: Color::srgb_from_array(
            bevy::color::palettes::css::BLACK.to_f32_array_no_alpha(),
        ), //.with_alpha(0.2), // alpha is unsupported, even if we change it
    }
}

fn custom_render_plugin_settings() -> RenderPlugin {
    RenderPlugin {
        render_creation: RenderCreation::Automatic(WgpuSettings {
            features: WgpuFeatures::POLYGON_MODE_LINE, // Required for wireframe
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn run_bevy_app() -> ExitCode {
    let cwd = std::env::current_dir().unwrap();
    let assets_folder = cwd.join(constants::ASSET_FOLDER);

    // Current working directory.
    println!("CWD: {cwd:?}");
    // Other debug info.
    //println!("DEFAULT ASSET DIR: {:?}", bevy::asset::AssetPlugin::default().file_path);
    println!("Assets folder: {assets_folder:?}");

    let settings_data = settings::load_from_file();
    logger::one(
        None,
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file to retrieve app building data.",
    );

    let window_size: (f32, f32) = (settings_data.window.width, settings_data.window.height);
    let wireframe_enabled: bool = settings_data.debug.map_render_wireframe;

    let mut app = App::new();
    let result = app
        .insert_resource(custom_winit_settings())
        .add_plugins(
            DefaultPlugins
                .build()
                .set(custom_bevy_log_config())
                .set(custom_window_plugin_settings(window_size))
                .set(custom_threadpool_settings())
                .set(custom_render_plugin_settings())
                .set(ImagePlugin::default_linear())
                .set(AssetPlugin {
                    //watch_for_changes_override: true,
                    file_path: assets_folder.to_str().unwrap().to_string(),
                    ..default()
                }),
        )
        .add_plugins(WireframePlugin::default()) // Needed enable wireframe rendering
        .insert_resource(custom_wireframe_config(wireframe_enabled))
        //.edit_schedule(Update, |schedule| {
        //  schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        //})
        .add_plugins(FramepacePlugin) // caps at 60 FPS by default
        //.use(bevy_framepace::FramepaceSettings::default().with_framerate(30.0))
        .add_plugins((
            controls::ControlsPlugin {
                registered_by: "Core",
            },
            render::RenderPlugin {
                registered_by: "Core",
            },
            settings::SettingsPlugin {
                registered_by: "Core",
            },
            texture_cache::TextureCachePlugin {
                registered_by: "Core",
            },
            uo_files_loader::UOFilesPlugin {
                registered_by: "Core",
            },
        ))
        .init_state::<AppState>()
        .insert_state(AppState::StartupSetup)
        .configure_sets(
            Startup,
            (
                StartupSysSet::LoadStartupUOFiles.after(StartupSysSet::First),
                StartupSysSet::SetupSceneStage1.after(StartupSysSet::LoadStartupUOFiles),
                StartupSysSet::SetupSceneStage2.after(StartupSysSet::SetupSceneStage1),
                StartupSysSet::Done.after(StartupSysSet::SetupSceneStage2),
            ),
        )
        .configure_sets(
            Update,
            MovementSysSet::UpdateCamera.after(MovementSysSet::MovementActions),
        )
        .add_systems(
            PreStartup,
            advance_state_after_init_core.in_set(StartupSysSet::First),
        )
        .add_systems(
            Startup,
            (setup_land_mesh, advance_state_after_scene_setup_stage_2.after(StartupSysSet::SetupSceneStage2)),
        )
        .run();

    match result {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(value) => ExitCode::from(value.get()),
    }
}

fn advance_state_after_init_core() {
    log_appstate_change("StartupSetup");
}

fn advance_state_after_scene_setup_stage_2(mut next_state: ResMut<NextState<AppState>>) {
    log_appstate_change("InGame");
    next_state.set(AppState::InGame);
}
