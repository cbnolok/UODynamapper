//! The world cameras draw nothing until the window's own pipelines are compiled, once, at the start.
//!
//! Bevy compiles the queued pipelines on its async compute pool, one thread here (`app.rs`), one after another in the
//! order it started them. With a cold driver cache (RADV, Radeon 880M, Mesa's cache empty) the world's first pipelines
//! take about 35 s, the land's alone most of it, and those Bevy needs to show anything at all (the blit to the window,
//! which its upscaling prepare even waits for, egui's, the tonemapping, the screenshot) queued in the same frames went
//! behind them: the window stayed blank, the overlay of the compiles included. So the world cameras (every `Camera3d`:
//! the world camera and the light buffer camera) start on no render layer and get theirs back once the render world
//! reports the window's pipelines ready: every view drawing to a target has requested its blit and no pipeline is left
//! compiling. The world's pipelines then queue behind nothing, the window and the overlay draw while they compile and
//! the world fills in as they arrive. With a warm cache the hold ends within the frames the scene takes to load.
//!
//! The held cameras are also kept active, drawing nothing, so the blit of every target they can draw to is compiled in
//! the hold too: a blit requested later, while the world's compiles run, is waited for by Bevy's upscaling prepare, and
//! the light buffer camera's (its image's format, not the window's) froze the app for 30 s when switching to night
//! during a cold start.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::camera::{CameraOutputMode, CameraUpdateSystems};
use bevy::core_pipeline::upscaling::ViewUpscalingPipeline;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::PipelineCache;
use bevy::render::view::ViewTarget;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use crate::prelude::*;

/// Past this the world draws even if the window's pipelines never reported ready (a pipeline whose shader never
/// loads stays waiting): the start is then as it was without the hold.
pub const HOLD_LIMIT: Duration = Duration::from_secs(10);

pub struct WorldDrawHoldPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WorldDrawHoldPlugin);

impl Plugin for WorldDrawHoldPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        let ready = WindowPipelinesReady::default();
        app.insert_resource(WorldDrawHold::default())
            .insert_resource(ready.clone())
            .add_systems(
                PostUpdate,
                sys_hold_world_cameras.after(CameraUpdateSystems).before(VisibilitySystems::CheckVisibility),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.insert_resource(ready)
            .init_resource::<ExtractedWorldDrawHold>()
            .add_systems(ExtractSchedule, sys_extract_world_draw_hold)
            .add_systems(Render, sys_report_window_pipelines.in_set(RenderSystems::Cleanup));
    }
}

/// Whether the world cameras are still held, since when.
#[derive(Resource, Default)]
pub struct WorldDrawHold {
    /// When the hold began, once it did.
    since: Option<Instant>,
    released: bool,
}

/// Set by the render world once the window's pipelines are ready while the hold lasts.
#[derive(Resource, Clone, Default)]
struct WindowPipelinesReady(Arc<AtomicBool>);

/// What a held camera gets back on release.
#[derive(Component)]
struct HeldCamera {
    /// Its own render layers (`None`: it had none, so layer 0).
    layers: Option<RenderLayers>,
    /// Whether it was active when held, or its owner turned it off since: the hold keeps it active. An owner turning it
    /// on during the hold finds it on already and writes nothing, so it must keep its camera in sync every frame (as
    /// the light buffer camera's does) to have it on again after the release.
    active: bool,
}

/// Whether the frame the render world draws was extracted with the world held.
#[derive(Resource, Default)]
pub struct ExtractedWorldDrawHold(bool);

impl ExtractedWorldDrawHold {
    /// The world cameras draw nothing in this frame: their pipelines are not even requested yet.
    pub fn held(&self) -> bool {
        self.0
    }
}

/// Holds every world camera active on no render layer until the window's pipelines are ready or [`HOLD_LIMIT`]
/// passed, then gives each its layers and its activity back. Runs after the cameras' owners update them.
fn sys_hold_world_cameras(
    mut commands: Commands,
    mut hold: ResMut<WorldDrawHold>,
    ready: Res<WindowPipelinesReady>,
    mut new_cameras: Query<(Entity, &mut Camera, Option<&RenderLayers>), (With<Camera3d>, Without<HeldCamera>)>,
    mut held_cameras: Query<(Entity, &mut Camera, &mut HeldCamera)>,
) {
    if hold.released {
        return;
    }
    let now = Instant::now();
    let since = *hold.since.get_or_insert(now);
    let window_ready = ready.0.load(Ordering::Acquire);
    if window_ready || now.saturating_duration_since(since) >= HOLD_LIMIT {
        for (entity, mut camera, held) in &mut held_cameras {
            if camera.is_active != held.active {
                camera.is_active = held.active;
            }
            let mut entity = commands.entity(entity);
            entity.remove::<HeldCamera>();
            match &held.layers {
                Some(layers) => entity.insert(layers.clone()),
                None => entity.remove::<RenderLayers>(),
            };
        }
        hold.released = true;
        let seconds = now.saturating_duration_since(since).as_secs_f64();
        if window_ready {
            info!("The window's pipelines are ready after {seconds:.2} s: the world draws.");
        } else {
            warn!("The window's pipelines are not ready after {seconds:.0} s: the world draws anyway.");
        }
        return;
    }
    for (_, mut camera, mut held) in &mut held_cameras {
        if !camera.is_active {
            held.active = false;
            camera.is_active = true;
        }
    }
    for (entity, mut camera, layers) in &mut new_cameras {
        let held = HeldCamera { layers: layers.cloned(), active: camera.is_active };
        camera.is_active = true;
        commands.entity(entity).insert((held, RenderLayers::none()));
    }
}

fn sys_extract_world_draw_hold(mut extracted: ResMut<ExtractedWorldDrawHold>, hold: Extract<Res<WorldDrawHold>>) {
    extracted.0 = !hold.released;
}

/// The window's pipelines are ready once at least one view draws to a target, every such view has requested its blit
/// (Bevy's upscaling prepare adds the component then, compiled or not) and no pipeline is left compiling, the blits
/// included.
fn window_pipelines_ready(views_have_blit: impl IntoIterator<Item = bool>, waiting_pipelines: usize) -> bool {
    let mut views = 0;
    for has_blit in views_have_blit {
        if !has_blit {
            return false;
        }
        views += 1;
    }
    views != 0 && waiting_pipelines == 0
}

fn sys_report_window_pipelines(
    hold: Res<ExtractedWorldDrawHold>,
    ready: Res<WindowPipelinesReady>,
    cache: Res<PipelineCache>,
    views: Query<(&ExtractedCamera, Has<ViewUpscalingPipeline>), With<ViewTarget>>,
) {
    if !hold.0 || ready.0.load(Ordering::Relaxed) {
        return;
    }
    let drawing = views.iter()
        .filter(|(camera, _)| matches!(camera.output_mode, CameraOutputMode::Write { .. }))
        .map(|(_, has_blit)| has_blit);
    if window_pipelines_ready(drawing, cache.waiting_pipelines().count()) {
        ready.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_ready_with_every_blit_and_nothing_compiling() {
        assert!(window_pipelines_ready([true, true], 0));
        assert!(!window_pipelines_ready([true, false], 0), "a view without its blit");
        assert!(!window_pipelines_ready([true, true], 1), "a pipeline compiling");
        assert!(!window_pipelines_ready([], 0), "no view drawing yet");
    }

    fn hold_app() -> App {
        let mut app = App::new();
        app.insert_resource(WorldDrawHold::default())
            .insert_resource(WindowPipelinesReady::default())
            .add_systems(Update, sys_hold_world_cameras);
        app
    }

    /// The world cameras lose their layers while held and get their own back, none for the camera that had none; the
    /// UI camera (2D) is never held.
    #[test]
    fn world_cameras_draw_nothing_until_the_window_is_ready() {
        const LIGHT_LAYER: usize = 3;
        let mut app = hold_app();
        let world = app.world_mut().spawn(Camera3d::default()).id();
        let inactive = Camera { is_active: false, ..default() };
        let light = app.world_mut().spawn((Camera3d::default(), inactive, RenderLayers::layer(LIGHT_LAYER))).id();
        let ui = app.world_mut().spawn(Camera2d).id();
        app.update();
        app.update();
        assert_eq!(app.world().get::<RenderLayers>(world), Some(&RenderLayers::none()));
        assert_eq!(app.world().get::<RenderLayers>(light), Some(&RenderLayers::none()));
        assert_eq!(app.world().get::<RenderLayers>(ui), None);
        assert!(app.world().get::<Camera>(light).is_some_and(|camera| camera.is_active), "held active");

        app.world().resource::<WindowPipelinesReady>().0.store(true, Ordering::Release);
        app.update();
        assert_eq!(app.world().get::<RenderLayers>(world), None);
        assert_eq!(app.world().get::<RenderLayers>(light), Some(&RenderLayers::layer(LIGHT_LAYER)));
        assert!(app.world().get::<Camera>(light).is_some_and(|camera| !camera.is_active), "inactive again");
        assert!(app.world().get::<Camera>(world).is_some_and(|camera| camera.is_active));
        assert!(app.world().resource::<WorldDrawHold>().released);

        // A camera spawned after the release is not held.
        let late = app.world_mut().spawn(Camera3d::default()).id();
        app.update();
        assert_eq!(app.world().get::<RenderLayers>(late), None);
    }

    #[test]
    fn the_world_draws_after_the_hold_limit_anyway() {
        let mut app = hold_app();
        let world = app.world_mut().spawn(Camera3d::default()).id();
        app.update();
        assert_eq!(app.world().get::<RenderLayers>(world), Some(&RenderLayers::none()));
        let started = app.world().resource::<WorldDrawHold>().since.expect("the hold began");
        app.world_mut().resource_mut::<WorldDrawHold>().since = started.checked_sub(HOLD_LIMIT);
        app.update();
        assert_eq!(app.world().get::<RenderLayers>(world), None);
        assert!(app.world().resource::<WorldDrawHold>().released);
    }
}
