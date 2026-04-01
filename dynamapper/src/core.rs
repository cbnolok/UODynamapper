pub mod app_states;
mod bevy_log_filter;
mod diagnostics;
pub mod constants;
pub mod controls;
pub mod maps;
pub mod render;
pub mod system_sets;
mod texture_cache;
mod uo_files_loader;

use crate::{
    console_logger::{self, LogAbout, LogSev},
    core::{
        app_states::*,
        bevy_log_filter::custom_bevy_log_config,
        diagnostics::{add_diagnostics_plugins, sys_log_gpu_preprocessing_mode}, render::scene::camera::UO_TILE_PIXEL_SIZE,
    },
    external_data::{ExternalDataPlugin, settings},
    util_lib::tracked_plugin::log_plugin_registry_tree,
};
use bevy::{
    //ecs::schedule::ExecutorKind,
    pbr::wireframe::{WireframeConfig, WireframePlugin},
    prelude::*,
    render::{
        settings::{RenderCreation, WgpuFeatures, WgpuSettings},
        RenderApp, RenderStartup,
    },
    window::{PresentMode, WindowResolution},
    winit::{UpdateMode, WinitSettings},
};
use std::{process::ExitCode, time::Duration};
use system_sets::*;

fn custom_winit_settings(reduce_unfocused_fps: bool) -> WinitSettings {
    // Use Continuous mode: render every frame unconditionally.
    // Reactive mode only schedules frames when OS events arrive (mouse/keyboard),
    // which caps FPS at the event rate and causes visual stutter during scrolling.
    let mut settings = WinitSettings::game();
    if reduce_unfocused_fps {
        /* settings.unfocused_mode = UpdateMode::ReactiveLowPower {
            max_wait: Duration::from_millis(250), // Refresh at least ~4 times a second even if idle
        };
        */
        settings.unfocused_mode = UpdateMode::Reactive {
            wait: Duration::from_millis(250),
            react_to_device_events: true,
            react_to_user_events: true,
            react_to_window_events: true,
        };
    }
    settings
}

fn custom_threadpool_settings() -> TaskPoolPlugin {
    /*
    TaskPoolPlugin {
            task_pool_options: TaskPoolOptions {
                // Minimum threads for system computation (already limited by the feature)
                compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: 1,
                    max_threads: 3,
                    percent: 0.0,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                // Limit the pool for asset loading (I/O)
                io: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: 1,
                    max_threads: 1,
                    percent: 0.0,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                // Limit the pool for asynchronous computation
                async_compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                    min_threads: 1,
                    max_threads: 1,
                    percent: 0.0,
                    on_thread_spawn: None,
                    on_thread_destroy: None,
                },
                ..default()
            }
        }
        */
    TaskPoolPlugin::default()
}

fn custom_window_plugin_settings(size: (f32, f32), vsync: bool) -> WindowPlugin {
    let present_mode = if vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    WindowPlugin {
        primary_window: Some(Window {
            present_mode,
            title: "UODynamapper".to_string(),
            resizable: true,
            // Force 1:1 aspect for virtual rendering (game world)
            // UO requires 'virtual' 44×44 diamonds, so...
            resolution: WindowResolution::new(size.0 as u32, size.1 as u32), //(1320.0, 924.0), // (44*30)x(44*21), etc
            resize_constraints: WindowResizeConstraints {
                min_width: UO_TILE_PIXEL_SIZE * 10.0,
                min_height: UO_TILE_PIXEL_SIZE * 10.0,
                ..Default::default()
            },
            // Let window freely resize, but camera+scene SYSTEMS keep virtual grid and diamonds fixed.
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Bevy 0.18.1 WireframePlugin panics if Node3d::PostProcessing is missing from the graph.
/// This plugin adds an EmptyNode to satisfy the edge requirement.
struct WireframePanicFixPlugin;
impl Plugin for WireframePanicFixPlugin {
    /* https://taintedcoders.com/bevy/rendering
    Bevy provides an extendable graph-structured rendering system, where input nodes pass data to output nodes.
    These nodes are held inside the RenderGraph, a stateless structure that holds your stateful nodes.
    In a similar way to your App, it has a separate runner that iterates over this graph to actually render things.
    These graphs are made up of Nodes, Edges and Slots.
    - Nodes are responsible for generating draw calls and operating on input and output slots.
    - Edges specify the order of execution for nodes and connect input and output slots together.
    - Slots describe the render resources created or used by the nodes.
    Adding an input node to a render graph allows them to be nested.
    Render Graphs are a way to logically model GPU command construction in a modular way. Graph Nodes pass GPU resources
    like Textures and Buffers (and sometimes Entities) to each other, forming a directed acyclic graph.
    When a Graph Node runs, it uses its graph inputs and the Render World to construct GPU command lists.
     */
    fn build(&self, app: &mut App) {
        use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
        use bevy::render::render_graph::{EmptyNode, RenderGraphExt};
        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        render_app.add_render_graph_node::<EmptyNode>(Core3d, Node3d::PostProcessing);
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

fn custom_render_plugin_settings() -> bevy::render::RenderPlugin {
    // Features field is a must-enable set, not a “disable everything else” set.
    #[allow(unused_mut)]
    let mut features = WgpuFeatures::POLYGON_MODE_LINE; // Enable this to allow wireframes.
    #[cfg(feature = "gpu-profiling")]
    {
        features |= WgpuFeatures::TIMESTAMP_QUERY;
        features |= WgpuFeatures::TIMESTAMP_QUERY_INSIDE_PASSES;
    }

    bevy::render::RenderPlugin {
        render_creation: RenderCreation::Automatic(WgpuSettings {
            //backends: Some(Backends::VULKAN),
            features,
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn run_bevy_app() -> ExitCode {
    let cwd = std::env::current_dir().unwrap();
    let assets_folder = cwd.join(constants::ASSET_FOLDER);

    // Current working directory.
    console_logger::system(&format!("CWD: {cwd:?}"));
    // Other debug info.
    console_logger::system(&format!(
        "Default Assets folder: {:?}",
        bevy::asset::AssetPlugin::default().file_path
    ));
    console_logger::system(&format!(
        "Setting custom Assets folder: {}",
        assets_folder.as_os_str().to_str().unwrap_or("N/A")
    ));

    let settings_data = settings::load_from_files();
    settings::apply_logging_settings(&settings_data.logging);
    console_logger::one(
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file to retrieve app building data.",
    );

    let window_size: (f32, f32) = (
        settings_data.app.window.width,
        settings_data.app.window.height,
    );
    let wireframe_enabled: bool = settings_data.app.debug.map_render_wireframe;

    let vsync_enabled = settings_data.graphics.vsync;

    let mut app = App::new();
    app.insert_resource(custom_winit_settings(
        settings_data.graphics.reduce_unfocused_fps,
    ))
    .add_plugins(
        DefaultPlugins
            .build()
            //.disable::<LogPlugin>() // This removes every Bevy logs, instead of just disabling default to avoid double-logging or formatting issues
            .set(custom_bevy_log_config())
            .set(custom_window_plugin_settings(window_size, vsync_enabled))
            .set(custom_threadpool_settings())
            .set(custom_render_plugin_settings())
            .set(ImagePlugin::default_linear())
            .set(AssetPlugin {
                //watch_for_changes_override: true,
                file_path: assets_folder.to_str().unwrap().to_string(),
                ..default()
            }),
    )
    .add_plugins(WireframePanicFixPlugin) // Fix for bevy_pbr 0.18.1 Node3d::PostProcessing panic
    .add_plugins(WireframePlugin::default()) // Needed enable wireframe rendering
    .insert_resource(custom_wireframe_config(wireframe_enabled))
    //.edit_schedule(Update, |schedule| {
    //  schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    //})
    .add_plugins(bevy_framepace::FramepacePlugin)
    // TODO: we have to enable hot reloading of this setting, not just setting it at startup.
    .insert_resource(bevy_framepace::FramepaceSettings {
        limiter: if settings_data.app.performance.frame_limit_enabled && !vsync_enabled {
            bevy_framepace::Limiter::from_framerate(settings_data.app.performance.target_fps as f64)
        } else {
            bevy_framepace::Limiter::Off
        },
    })
    .insert_resource(bevy_egui::EguiGlobalSettings {
        auto_create_primary_context: false, // We manually spawn PrimaryEguiContext on the UI camera
        ..default()
    })
    .add_plugins(bevy_egui::EguiPlugin::default()) // egui UI layer (used for teleport dialog, etc.)
    .add_plugins((
        ExternalDataPlugin {
            registered_by: "Core",
        },
        controls::ControlsPlugin {
            registered_by: "Core",
        },
        render::RenderPlugin {
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
        (MovementSysSet::UpdateCamera
            .after(MovementSysSet::MovementActions)
            .before(crate::core::system_sets::SceneRenderLandSysSet::ListenSyncRequests),),
    )
    .add_systems(
        PreStartup,
        advance_state_after_init_core.in_set(StartupSysSet::First),
    )
    .add_systems(
        Startup,
        advance_state_after_scene_setup_stage_2.after(StartupSysSet::SetupSceneStage2),
    );

    app.add_systems(Startup, log_plugin_registry_tree.in_set(StartupSysSet::Done));

    // One-shot render-world startup: log whether GPU indirect draw is active.
    // GpuPreprocessingSupport lives only in the render world, not the main world.
    app.sub_app_mut(RenderApp)
        .add_systems(RenderStartup, sys_log_gpu_preprocessing_mode);

    // Manually add complex plugins or plugin tuples.
    add_diagnostics_plugins(&mut app);

    let result = app.run();

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
