pub mod app_states;
pub mod constants;
pub mod controls;
pub mod maps;
pub mod render;
pub mod system_sets;
mod texture_cache;
mod uo_files_loader;

use crate::{
    console_logger::{self, LogAbout, LogSev},
    core::app_states::*,
    external_data::{settings, ExternalDataPlugin},
};
use bevy::{
    //ecs::schedule::ExecutorKind,
    pbr::wireframe::{WireframeConfig, WireframePlugin},
    prelude::*,
    render::{
        settings::{RenderCreation, WgpuFeatures, WgpuSettings},
        RenderApp, RenderStartup,
    },
    window::WindowResolution,
    winit::{UpdateMode, WinitSettings},
};
use std::{process::ExitCode, time::Duration};
use system_sets::*;

// Low level GPU diagnostics, used during development
const DUMP_DIAGNOSTICS_INTERVAL_SEC: u64 = 0; // 0 = don't do it. If needed, 2 seconds is good.

/// Replaces Bevy's default fmt layer with a compact one that uses HH:MM:SS
/// timestamps instead of the verbose ISO-8601 default.
/// This is wired into LogPlugin::fmt_layer (not custom_layer), which means it
/// fully replaces the default formatter rather than being added on top of it.
/*
fn bevy_logging_fmt_layer(_app: &mut App) -> Option<bevy::log::BoxedFmtLayer> {
    Some(Box::new(
        fmt::layer()
            //.with_span_events(FmtSpan::NONE)
            .with_ansi(true)
            .with_level(true)
            .with_target(true)
            // Compact HH:MM:SS format — avoids the verbose 2026-03-18T09:50:34.068944Z default.
            .with_timer(fmt::time::ChronoLocal::new("%H:%M:%S".into()))
            .compact(),
    ))
}
*/

/// Additive layer (runs alongside the fmt layer, can't replace it).
/// Intercepts tracing events whose target starts with "uocf" — emitted by the
/// `uocf` crate via the standard `log` facade → `tracing-log` bridge — and
/// re-emits them through `console_logger::one` for consistent formatting.

fn bevy_logging_custom_layer(_app: &mut App) -> Option<bevy::log::BoxedLayer> {
    use crate::console_logger::{LogAbout, LogSev};
    use tracing::{Level, Subscriber};
    use tracing_subscriber::{registry::LookupSpan, Layer};

    struct InterceptLogLayer;

    impl<S> Layer<S> for InterceptLogLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let target = event.metadata().target();
            // Only intercept events from our crates or Bevy itself.
            let about = if target.starts_with("uocf") {
                LogAbout::UoFiles
            } else if target.starts_with("bevy") {
                LogAbout::Bevy
            } else {
                return;
            };

            let sev = match *event.metadata().level() {
                Level::ERROR => LogSev::Error,
                Level::WARN => LogSev::Warn,
                Level::DEBUG => LogSev::Debug,
                Level::TRACE => LogSev::DebugVerbose,
                _ => LogSev::Info,
            };

            struct MsgVisitor(String);
            impl tracing::field::Visit for MsgVisitor {
                fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
                    if f.name() == "message" {
                        self.0 = v.to_string();
                    }
                }
                fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                    if f.name() == "message" {
                        self.0 = format!("{v:?}");
                    }
                }
            }
            let mut vis = MsgVisitor(String::new());
            event.record(&mut vis);

            crate::console_logger::one(Some(false), sev, about, &vis.0);
        }
    }

    Some(InterceptLogLayer.boxed())
}

fn custom_bevy_log_config() -> bevy::log::LogPlugin {
    bevy::log::LogPlugin {
        // Suppress benign calloop warnings on Linux (e.g. "Received an event for non-existence source")
        filter: "calloop=error,bevy_framepace=warn"
            .into(),
        // Return a no-op fmt layer that writes to /dev/null.
        // Returning None would make Bevy fall back to its default stderr formatter,
        // causing double logging alongside our InterceptLogLayer.
        fmt_layer: |_| {
            Some(Box::new(
                tracing_subscriber::fmt::Layer::default().with_writer(std::io::sink),
            ))
        },
        // Add the InterceptLogLayer on top (intercepts log events from Bevy or Uocf crates).
        custom_layer: bevy_logging_custom_layer,
        ..Default::default()
    }
}

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
            resolution: WindowResolution::new(size.0 as u32, size.1 as u32), //(1320.0, 924.0), // (44*30)x(44*21), etc
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
    // TODO/FIXME: PRIORITIZE THIS: should we query WgpuSettings::features and then adding POLYGON_MODE_LINE?
    //  Are we renouncing to features that would be extremely beneficial to us, like PARTIALLY_BOUND_BINDING_ARRAY or other native ones
    //      useful on some platforms like Android or with Metal API?
    // Also, since Vulkan appears to support way more features than OpenGL,
    //  how can we tell Bevy or Wgpu to prioritize Vulkan, if supported, over OpenGL?
    bevy::render::RenderPlugin {
        render_creation: RenderCreation::Automatic(WgpuSettings {
            features: WgpuFeatures::POLYGON_MODE_LINE, // Required for wireframe
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn add_diagnostics_plugins(app: &mut App) {
    app.add_plugins((
        bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
        // `SystemInformationDiagnosticsPlugin` is intentionally NOT used here:
        // it fails to initialize with dynamic_linking enabled (emits a 'not supported'
        // warning and returns no data). We read CPU/RAM directly via `sysinfo` instead
        // (see `core/render/overlays/performance.rs`).

        // GPU pipeline statistics (vertex/fragment invocations, clipper primitives).
        // On Vulkan/DX12 also provides GPU elapsed time per pass.
        // NOTE: no public DiagnosticPath constants in 0.18 — paths are dynamic strings
        // like "render/{span_name}/fragment_shader_invocations".
        // Use LogDiagnosticsPlugin below to discover the exact span names.
        bevy::render::diagnostic::RenderDiagnosticsPlugin,
    ));

    if DUMP_DIAGNOSTICS_INTERVAL_SEC != 0 {
        app.add_plugins((
            // Temporary: dump all render/diagnostics to console every 2 seconds.
            // filter=None means log ALL diagnostics (FPS + render stats).
            // Once we know the exact span names, narrow to just the render ones.
            bevy::diagnostic::LogDiagnosticsPlugin {
                filter: None,
                wait_duration: std::time::Duration::from_secs(DUMP_DIAGNOSTICS_INTERVAL_SEC),
                ..default()
            },
        ));
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
    console_logger::system(&format!("Setting custom Assets folder: {assets_folder:?}"));

    let settings_data = settings::load_from_files();
    console_logger::one(
        None,
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file to retrieve app building data.",
    );

    let window_size: (f32, f32) = (
        settings_data.app.window.width,
        settings_data.app.window.height,
    );
    let wireframe_enabled: bool = settings_data.app.debug.map_render_wireframe;

    let mut app = App::new();
    app.insert_resource(custom_winit_settings(
        settings_data.core.graphics.reduce_unfocused_fps,
    ))
    .add_plugins(
        DefaultPlugins
            .build()
            //.disable::<LogPlugin>() // This removes every Bevy logs, instead of just disabling default to avoid double-logging or formatting issues
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
    .add_plugins(WireframePanicFixPlugin) // Fix for bevy_pbr 0.18.1 Node3d::PostProcessing panic
    .add_plugins(WireframePlugin::default()) // Needed enable wireframe rendering
    .insert_resource(custom_wireframe_config(wireframe_enabled))
    //.edit_schedule(Update, |schedule| {
    //  schedule.set_executor_kind(ExecutorKind::SingleThreaded);
    //})
    .add_plugins(bevy_framepace::FramepacePlugin)
    .insert_resource(bevy_framepace::FramepaceSettings {
        limiter: if settings_data.app.performance.frame_limit_enabled {
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
        MovementSysSet::UpdateCamera.after(MovementSysSet::MovementActions),
    )
    .add_systems(
        PreStartup,
        advance_state_after_init_core.in_set(StartupSysSet::First),
    )
    .add_systems(
        Startup,
        advance_state_after_scene_setup_stage_2.after(StartupSysSet::SetupSceneStage2),
    );

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

/// One-shot system (render world) that logs the active GPU preprocessing mode.
/// Helps verify whether Bevy is using full GPU culling + indirect draw.
fn sys_log_gpu_preprocessing_mode(
    support: Res<bevy::render::batching::gpu_preprocessing::GpuPreprocessingSupport>,
) {
    use bevy::render::batching::gpu_preprocessing::GpuPreprocessingMode;
    let mode_str = match support.max_supported_mode {
        GpuPreprocessingMode::None => "None (CPU-only, WebGL2 / no compute)",
        GpuPreprocessingMode::PreprocessingOnly => {
            "PreprocessingOnly (GPU uniforms, CPU draw calls)"
        }
        GpuPreprocessingMode::Culling => "Culling (full GPU frustum culling + multi_draw_indirect)",
    };
    bevy::log::info!("[GPU Batching] GpuPreprocessingMode = {mode_str}");
}

fn advance_state_after_init_core() {
    log_appstate_change("StartupSetup");
}

fn advance_state_after_scene_setup_stage_2(mut next_state: ResMut<NextState<AppState>>) {
    log_appstate_change("InGame");
    next_state.set(AppState::InGame);
}
