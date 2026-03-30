#![cfg(feature = "gpu-profiling")]
use bevy::core_pipeline::core_3d::graph::Core3d;
use bevy::prelude::*;
use bevy::render::{
    render_graph::{Node, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
    render_resource::WgpuFeatures,
    renderer::{RenderContext, RenderDevice, RenderQueue},
    Extract, Render, RenderApp, RenderSystems,
};
use wgpu_profiler::{GpuProfiler, GpuProfilerQuery, GpuProfilerSettings};
// Workaround for Node3d if import fails: use the full path or check variants.
use bevy::core_pipeline::core_3d::graph::Node3d;
use parking_lot::Mutex;
use std::ops::DerefMut;
use std::sync::Arc;

/// Resource wrapping the wgpu-profiler instance in the Render World.
/// We use a Mutex because Node::run provides an immutable reference to the World.
#[derive(Resource)]
pub struct GpuProfilerResource(pub Arc<Mutex<GpuProfiler>>);

/// Resource in the Main World to trigger a profiling export.
#[derive(Resource, Default)]
pub struct GpuProfilingTrigger {
    pub request_export: bool,
}

/// Resource in the Render World that receives the export trigger.
#[derive(Resource, Default)]
pub struct RenderGpuProfilingTrigger {
    pub request_export: Arc<Mutex<bool>>,
}

/// Resource in the Render World to track a query across nodes.
#[derive(Resource, Default)]
pub struct ActiveProfilingQuery(pub Arc<Mutex<Option<GpuProfilerQuery>>>);

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct ProfilingBeginLabel;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct ProfilingEndLabel;

pub struct LandProfilingPlugin;

impl Plugin for LandProfilingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GpuProfilingTrigger>()
            .add_systems(Update, sys_trigger_gpu_profiling);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<RenderGpuProfilingTrigger>()
            .init_resource::<ActiveProfilingQuery>()
            .add_systems(ExtractSchedule, sys_extract_profiling_trigger)
            .add_systems(
                Render,
                sys_gpu_profiler_finalize.in_set(RenderSystems::Cleanup),
            );

        // Add nodes to the Render Graph
        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();

        let Some(core_3d) = graph.get_sub_graph_mut(Core3d) else {
            return;
        };

        core_3d.add_node(ProfilingBeginLabel, ProfilingBeginNode);
        core_3d.add_node(ProfilingEndLabel, ProfilingEndNode);

        // Wrap the MainPass
        core_3d.add_node_edge(ProfilingBeginLabel, Node3d::StartMainPass);
        core_3d.add_node_edge(Node3d::EndMainPass, ProfilingEndLabel);
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        let render_device = render_app.world().resource::<RenderDevice>();

        // Ensure the device supports timestamp queries.
        if !render_device
            .features()
            .contains(WgpuFeatures::TIMESTAMP_QUERY)
        {
            bevy::log::warn!(
                "GPU Profiling: TIMESTAMP_QUERY NOT supported by device. Profiling disabled."
            );
            return;
        }

        // Initialize profiler with wgpu 0.25 (wgpu 27.0)
        let mut settings = GpuProfilerSettings::default();
        settings.enable_debug_groups = false;
        settings.max_num_pending_frames = 8; // Increase head-room for slow GPU resolution

        let profiler = GpuProfiler::new(render_device.wgpu_device(), settings)
            .expect("Failed to create GpuProfiler");

        render_app.insert_resource(GpuProfilerResource(Arc::new(Mutex::new(profiler))));
    }
}

fn sys_extract_profiling_trigger(
    trigger: Extract<Res<GpuProfilingTrigger>>,
    render_trigger: Res<RenderGpuProfilingTrigger>,
) {
    if trigger.request_export {
        *render_trigger.request_export.lock() = true;
    }
}

fn sys_trigger_gpu_profiling(
    input: Res<ButtonInput<KeyCode>>,
    mut trigger: ResMut<GpuProfilingTrigger>,
) {
    if input.just_pressed(KeyCode::F10) {
        trigger.request_export = true;
        bevy::log::info!("GPU Profiling: export requested via F10...");
    }
}

pub struct ProfilingBeginNode;
impl Node for ProfilingBeginNode {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        // Only profile if it's a camera view (we don't want to profile every shadow pass etc by default)
        // In this app, we specifically care about the main world rendering.

        let profiler_res = world.get_resource::<GpuProfilerResource>();
        let active_query_res = world.resource::<ActiveProfilingQuery>();

        let Some(profiler_res) = profiler_res else {
            return Ok(());
        };

        let mut profiler = profiler_res.0.lock();
        let mut active_query = active_query_res.0.lock();
        let mut encoder = render_context.command_encoder();

        // Use begin_query for 0.25.0
        // We only start a "Total Frame" query if one isn't already active.
        // This handles multiple views by timing the first one encountered.
        if active_query.is_none() {
            *active_query = Some(profiler.begin_query("Total Frame", encoder.deref_mut()));
        }

        Ok(())
    }
}

pub struct ProfilingEndNode;
impl Node for ProfilingEndNode {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let profiler_res = world.get_resource::<GpuProfilerResource>();
        let active_query_res = world.resource::<ActiveProfilingQuery>();

        let Some(profiler_res) = profiler_res else {
            return Ok(());
        };

        let mut profiler = profiler_res.0.lock();
        let mut active_query = active_query_res.0.lock();
        let mut encoder = render_context.command_encoder();

        // 1. Close the frame query if it belongs to this node's start
        // We only close it here. If multiple views run, only the first view's query is timed.
        if let Some(query) = active_query.take() {
            profiler.end_query(encoder.deref_mut(), query);

            // 2. Resolve queries for the frame - DO IT ONLY ONCE at the end of the query
            profiler.resolve_queries(encoder.deref_mut());
        }

        // 3. Process finished frames and end_frame MUST BE CALLED ONCE per App Frame.
        // We use a global system for this instead of a node to be safe.
        // Or we could check if we are the "last" camera, but that's hard.

        Ok(())
    }
}

/// System to finalize the profiler frame once per frame.
fn sys_gpu_profiler_finalize(
    profiler_res: Option<Res<GpuProfilerResource>>,
    trigger_res: Res<RenderGpuProfilingTrigger>,
    render_queue: Res<RenderQueue>,
) {
    let Some(profiler_res) = profiler_res else {
        return;
    };
    let mut profiler = profiler_res.0.lock();

    // 1. Process finished frames
    let mut trigger = trigger_res.request_export.lock();
    while let Some(results) = profiler.process_finished_frame(render_queue.get_timestamp_period()) {
        if *trigger {
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let filename = format!("gpu_profile_{}.json", timestamp);

            match wgpu_profiler::chrometrace::write_chrometrace(
                std::path::Path::new(&filename),
                &results,
            ) {
                Ok(_) => bevy::log::info!("GPU Profile exported to {}", filename),
                Err(e) => bevy::log::error!("Failed to export GPU profile: {}", e),
            }
            *trigger = false;
        }
    }

    // 2. End frame to prepare for next one.
    // This MUST be called after all resolve_queries are done and submitted.
    let _ = profiler.end_frame();
}
