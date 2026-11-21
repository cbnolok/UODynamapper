//! Render pipelines still compiling, counted in the render world and published to the main world. A new target format
//! or a newly drawn material needs new pipelines, and Bevy skips the draws whose pipeline is not ready yet: the map
//! export waits for none to be left before a capture, and so does the development control channel after a switch.
//! While the world cameras are held at the start (`world_draw_hold`) their pipelines are not even requested: the hold
//! counts as one.

use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems, render_resource::PipelineCache};
use crate::render::world_draw_hold::ExtractedWorldDrawHold;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Render pipelines still compiling, as counted in the render world's last completed frame.
#[derive(Resource, Clone, Default)]
pub struct WaitingRenderPipelines(Arc<AtomicUsize>);

impl WaitingRenderPipelines {
    /// The count of the render world's last completed frame.
    pub fn count(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

/// Publish `WaitingRenderPipelines` in the main world, counted in the render world; the export and the development
/// control channel both read it, and either may install it first.
pub fn count_waiting_render_pipelines(app: &mut App) {
    if app.world().contains_resource::<WaitingRenderPipelines>() {
        return;
    }
    let waiting_pipelines = WaitingRenderPipelines::default();
    app.insert_resource(waiting_pipelines.clone());
    app.sub_app_mut(RenderApp)
        .insert_resource(waiting_pipelines)
        .add_systems(Render, sys_count_waiting_pipelines.in_set(RenderSystems::Cleanup));
}

fn sys_count_waiting_pipelines(
    cache: Res<PipelineCache>,
    hold: Option<Res<ExtractedWorldDrawHold>>,
    waiting: Res<WaitingRenderPipelines>,
) {
    let held = hold.is_some_and(|hold| hold.held());
    waiting.0.store(cache.waiting_pipelines().count() + usize::from(held), Ordering::Relaxed);
}
