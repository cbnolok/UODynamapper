pub mod app_states;
mod bevy_log_filter;
pub mod constants;
pub mod controls;
mod diagnostics;
pub mod maps;
pub mod multis;
pub mod render;
pub mod statics;
pub mod system_sets;
mod texture_cache;
mod uo_files_loader;

use crate::{
    configs::{settings, ExternalDataPlugin},
    console_logger::{self, LogAbout, LogSev},
    core::{
        app_states::*,
        bevy_log_filter::init_bevy_logging,
        diagnostics::{add_diagnostics_plugins, LogGpuPreprocessingModePlugin},
        render::scene::camera::UO_TILE_PIXEL_SIZE,
    },
    impl_tracked_plugin,
    util_lib::tracked_plugin::{
        log_plugin_build, set_plugin_log_toggles, sys_log_plugin_registry_tree, TrackedPlugin,
    },
};
use bevy::{
    ecs::schedule::ScheduleLabel,
    //ecs::schedule::ExecutorKind,
    pbr::wireframe::{WireframeConfig, WireframePlugin},
    prelude::*,
    render::settings::{RenderCreation, WgpuFeatures, WgpuSettings},
    window::{PresentMode, WindowResolution},
    winit::{UpdateMode, WinitSettings},
};
use std::{path::Path, process::ExitCode, time::Duration};
use system_sets::*;

fn custom_winit_settings(reduce_unfocused_fps: bool) -> WinitSettings {
    // Use Continuous mode: render every frame unconditionally.
    // Reactive mode only schedules frames when OS events arrive (mouse/keyboard),
    // which caps FPS at the event rate and causes visual stutter during scrolling.
    let mut settings = WinitSettings::game();
    if reduce_unfocused_fps {
        settings.unfocused_mode = UpdateMode::Reactive {
            wait: Duration::from_secs_f32(1.0 / 10.0), // 10 FPS when unfocused
            react_to_device_events: true,
            react_to_user_events: true,
            react_to_window_events: true,
        };
    }
    settings
}

pub fn present_mode_for_vsync(vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    }
}

pub fn framepace_limiter_for_options(
    frame_limit_enabled: bool,
    target_fps: u32,
    vsync: bool,
) -> bevy_framepace::Limiter {
    if frame_limit_enabled && !vsync {
        bevy_framepace::Limiter::from_framerate(target_fps as f64)
    } else {
        bevy_framepace::Limiter::Off
    }
}

pub fn framepace_limiter_for_settings(settings: &settings::Settings) -> bevy_framepace::Limiter {
    framepace_limiter_for_options(
        settings.app.performance.frame_limit_enabled,
        settings.app.performance.target_fps,
        settings.graphics.vsync,
    )
}

fn custom_threadpool_settings() -> TaskPoolPlugin {
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
        },
    }
}

fn custom_window_plugin_settings(size: (f32, f32), vsync: bool) -> WindowPlugin {
    let present_mode = present_mode_for_vsync(vsync);
    WindowPlugin {
        primary_window: Some(Window {
            present_mode,
            composite_alpha_mode: bevy::window::CompositeAlphaMode::Opaque,
            desired_maximum_frame_latency: Some(std::num::NonZeroU32::new(1).unwrap()),
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
struct WireframePanicFixPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WireframePanicFixPlugin);
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
        log_plugin_build(self);
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

fn custom_render_plugin_settings(enable_wireframe: bool) -> bevy::render::RenderPlugin {
    // Features field is a must-enable set, not a “disable everything else” set.
    #[allow(unused_mut)]
    let mut features = WgpuFeatures::empty();
    if enable_wireframe {
        features |= WgpuFeatures::POLYGON_MODE_LINE;
    }
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

fn file_watcher_fix_suggestion() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "Linux fix: increase the inotify max_user_instances limit with: su -c \"echo 256 > /proc/sys/fs/inotify/max_user_instances\""
    }

    #[cfg(target_os = "macos")]
    {
        return "macOS fix: close applications using many file watchers or raise the open-file limit with ulimit -n before starting UODynamapper.";
    }

    #[cfg(target_os = "windows")]
    {
        return "Windows fix: close applications using many file watchers, then restart UODynamapper. If sync or antivirus software is watching the assets directory, exclude it.";
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        return "Fix: close applications using many file watchers or raise the host OS file watcher/open-file limit before starting UODynamapper.";
    }
}

fn resilient_asset_source_builder(file_path: &Path) -> bevy::asset::io::AssetSourceBuilder {
    let file_path = file_path.to_string_lossy().to_string();
    let watched_file_path = file_path.clone();

    bevy::asset::io::AssetSourceBuilder::platform_default(&file_path, None).with_watcher(
        move |sender| {
            let path = bevy::asset::io::file::FileAssetReader::get_base_path()
                .join(watched_file_path.clone());

            if !path.exists() {
                bevy::log::warn!(
                    "Skipping asset file watcher because path {:?} does not exist.",
                    path
                );
                return None;
            }

            match bevy::asset::io::file::FileWatcher::new(
                path.clone(),
                sender,
                Duration::from_millis(300),
            ) {
                Ok(watcher) => {
                    Some(Box::new(watcher) as Box<dyn bevy::asset::io::AssetWatcher>)
                }
                Err(err) => {
                    bevy::log::error!(
                        "Failed to create asset file watcher for path {:?}: {:?}. Asset hot-reloading is disabled for this run. {}",
                        path,
                        err,
                        file_watcher_fix_suggestion()
                    );
                    None
                }
            }
        },
    )
}

pub fn run_bevy_app() -> ExitCode {
    let cwd = std::env::current_dir().unwrap();
    let assets_folder = constants::valid_asset_dir();

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
    // Configure plugin log toggles (controls flat per-plugin lines and tree dump)
    set_plugin_log_toggles(
        settings_data.logging.emit_flat_plugin_build,
        settings_data.logging.emit_tree_plugin_build,
    );
    console_logger::one(
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file to retrieve app building data.",
    );

    let window_size: (f32, f32) = (
        settings_data.session_state.window.width,
        settings_data.session_state.window.height,
    );
    let wireframe_enabled: bool = settings_data.app.debug.map_render_wireframe;

    let vsync_enabled = settings_data.graphics.vsync;

    init_bevy_logging();

    let mut app = App::new();
    let mut asset_source_builders = bevy::asset::io::AssetSourceBuilders::default();
    asset_source_builders.insert(
        bevy::asset::io::AssetSourceId::Default,
        resilient_asset_source_builder(&assets_folder),
    );
    app.insert_resource(asset_source_builders);

    app.insert_resource(custom_winit_settings(
        settings_data.graphics.reduce_unfocused_fps,
    ))
    .add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::log::LogPlugin>()
            //.disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
            //.disable::<bevy::core_pipeline::experimental::mip_generation::MipGenerationPlugin>()
            //.disable::<bevy::core_pipeline::oit::OrderIndependentTransparencyPlugin>()
            //.disable::<bevy::core_pipeline::upscaling::UpscalingPlugin>()
            .set(custom_window_plugin_settings(window_size, vsync_enabled))
            .set(custom_threadpool_settings())
            .set(custom_render_plugin_settings(wireframe_enabled))
            .set(ImagePlugin::default_nearest())
            .set(AssetPlugin {
                watch_for_changes_override: Some(true),
                file_path: assets_folder.to_str().unwrap().to_string(),
                ..default()
            }),
    );

    for sched in [
        Update.intern(),
        PreUpdate.intern(),
        PostUpdate.intern(),
        FixedUpdate.intern(),
        FixedPreUpdate.intern(),
        FixedPostUpdate.intern(),
    ] {
        app.edit_schedule(sched, |schedule| {
            schedule.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
        });
    }

    if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
        use bevy::render;
        for sched in [render::Render.intern(), render::ExtractSchedule.intern()] {
            render_app.edit_schedule(sched, |schedule| {
                schedule.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
            });
        }
    }

    {
        let mut registry = crate::util_lib::tracked_plugin::plugin_registry()
            .lock()
            .expect("plugin registry poisoned");
        registry.record("DefaultPlugins", "Core");
        if wireframe_enabled {
            registry.record("WireframePlugin", "Core");
        }
        registry.record("FramepacePlugin", "Core");
        registry.record("EguiPlugin", "Core");
    }

    app.insert_resource(custom_wireframe_config(wireframe_enabled));
    if wireframe_enabled {
        app.add_plugins(WireframePanicFixPlugin {
            registered_by: "Core",
        }) // Fix for bevy_pbr 0.18.1 Node3d::PostProcessing panic
        .add_plugins(WireframePlugin::default()); // Needed to enable wireframe rendering
    }

    app.add_plugins(bevy_framepace::FramepacePlugin)
    .insert_resource(bevy_framepace::FramepaceSettings {
        limiter: framepace_limiter_for_settings(&settings_data),
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
        sys_advance_state_after_init_core.in_set(StartupSysSet::First),
    )
    .add_systems(
        Startup,
        sys_advance_state_after_scene_setup_stage_2.after(StartupSysSet::SetupSceneStage2),
    );

    app.add_systems(
        Startup,
        sys_log_plugin_registry_tree.in_set(StartupSysSet::Done),
    );

    // One-shot startup log: report whether Bevy uses GPU preprocessing / indirect draw.
    app.add_plugins(LogGpuPreprocessingModePlugin);

    // Manually add complex plugins or plugin tuples.
    add_diagnostics_plugins(&mut app, &settings_data.world_rendering.diagnostics);

    let result = app.run();

    match result {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(value) => ExitCode::from(value.get()),
    }
}

fn sys_advance_state_after_init_core() {
    log_appstate_change("StartupSetup");
}

fn sys_advance_state_after_scene_setup_stage_2(mut next_state: ResMut<NextState<AppState>>) {
    log_appstate_change("InGame");
    next_state.set(AppState::InGame);
}
