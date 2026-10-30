//! The preview thread: owns the package, the decoded resources, the scene and its simulation, and answers build
//! and render requests so the UI never simulates or rasterizes. Requests queue; a newer build or render replaces
//! the older ones still queued (the UI only ever wants the latest frame). A panic while building or rendering
//! (a corrupt package the reader did not catch) is reported as an error, and the thread keeps serving.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

use particle_sim::raster::RasterTexture;
use udd_assets::EffectsEcPackage;

use super::preview::{
    PreviewPart, PreviewScene, PreviewSimulation, PreviewTarget, ResourceCache, SimulationSettings, ViewSettings,
    WorldBounds, fit_bounds, preview_view, render_canvas,
};

/// Name of the preview thread.
const WORKER_THREAD_NAME: &str = "effects-preview";

/// What the UI asks.
pub enum WorkerRequest {
    /// Resolve and prepare `target`; later renders draw it. `generation` tags the answers.
    Build { generation: u64, target: PreviewTarget, settings: SimulationSettings },
    /// Draw the current scene at `time_ms` (simulated from 0 when it lies before the current time).
    Render { time_ms: u32, view: ViewSettings },
    /// The atlas hued NIF parts are drawn through (`None`: drawn plain).
    SetHueAtlas(Option<Arc<RasterTexture>>),
}

/// A scene ready to render.
#[derive(Debug, Clone)]
pub struct BuiltScene {
    pub generation: u64,
    pub target: PreviewTarget,
    pub parts: Vec<PreviewPart>,
    pub notes: Vec<String>,
    pub playing_parts: usize,
    pub has_hued_parts: bool,
    pub bounds: WorldBounds,
}

/// A drawn frame.
#[derive(Debug, Clone)]
pub struct RenderedFrame {
    pub generation: u64,
    pub time_ms: u32,
    pub width: usize,
    pub height: usize,
    /// Straight RGBA8, row-major.
    pub rgba: Vec<u8>,
    pub particles: usize,
    /// Every part stopped and has no particles left.
    pub drained: bool,
}

pub enum WorkerResponse {
    Built(BuiltScene),
    Frame(RenderedFrame),
    Failed { generation: u64, error: String },
}

/// The handle the UI keeps; dropping it stops the thread.
pub struct PreviewWorker {
    requests: Option<Sender<WorkerRequest>>,
    responses: Receiver<WorkerResponse>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewWorker {
    /// Starts the thread; `notify` runs after every response (the UI asks for a repaint).
    pub fn spawn(package: Arc<EffectsEcPackage>, notify: impl Fn() + Send + 'static) -> Self {
        let (request_tx, request_rx) = channel();
        let (response_tx, response_rx) = channel();
        let thread = std::thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                let mut state = WorkerState { package, cache: ResourceCache::default(), current: None, hue_atlas: None };
                while let Some(batch) = next_batch(&request_rx) {
                    for response in state.serve(batch) {
                        if response_tx.send(response).is_err() {
                            return;
                        }
                        notify();
                    }
                }
            })
            .ok();
        Self { requests: Some(request_tx), responses: response_rx, thread }
    }

    /// Queues a request; false when the thread is gone.
    pub fn send(&self, request: WorkerRequest) -> bool {
        self.requests.as_ref().is_some_and(|requests| requests.send(request).is_ok())
    }

    /// Every response received so far.
    pub fn poll(&self) -> Vec<WorkerResponse> {
        self.responses.try_iter().collect()
    }

    /// Waits for the next response (tests and offline rendering).
    pub fn wait(&self) -> Option<WorkerResponse> {
        self.responses.recv().ok()
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|thread| !thread.is_finished())
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        // Closing the channel ends the loop after the current request.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The requests to serve now: the latest build and the latest render queued after it, hue atlas changes in
/// order. `None` when the UI is gone.
struct Batch {
    hue_atlas: Option<Option<Arc<RasterTexture>>>,
    build: Option<(u64, PreviewTarget, SimulationSettings)>,
    render: Option<(u32, ViewSettings)>,
}

fn next_batch(requests: &Receiver<WorkerRequest>) -> Option<Batch> {
    let mut batch = Batch { hue_atlas: None, build: None, render: None };
    let absorb = |request: WorkerRequest, batch: &mut Batch| match request {
        WorkerRequest::SetHueAtlas(atlas) => batch.hue_atlas = Some(atlas),
        WorkerRequest::Build { generation, target, settings } => {
            batch.build = Some((generation, target, settings));
            batch.render = None;
        }
        WorkerRequest::Render { time_ms, view } => batch.render = Some((time_ms, view)),
    };
    absorb(requests.recv().ok()?, &mut batch);
    loop {
        match requests.try_recv() {
            Ok(request) => absorb(request, &mut batch),
            Err(TryRecvError::Empty) => return Some(batch),
            Err(TryRecvError::Disconnected) => return None,
        }
    }
}

struct Current {
    generation: u64,
    scene: PreviewScene,
    simulation: PreviewSimulation,
    bounds: WorldBounds,
}

struct WorkerState {
    package: Arc<EffectsEcPackage>,
    cache: ResourceCache,
    current: Option<Current>,
    hue_atlas: Option<Arc<RasterTexture>>,
}

impl WorkerState {
    fn serve(&mut self, batch: Batch) -> Vec<WorkerResponse> {
        let mut responses = Vec::new();
        if let Some(atlas) = batch.hue_atlas {
            self.hue_atlas = atlas;
        }
        if let Some((generation, target, settings)) = batch.build {
            self.current = None;
            let package = Arc::clone(&self.package);
            let cache = &mut self.cache;
            let built = catch_unwind(AssertUnwindSafe(|| {
                let scene = PreviewScene::build(&package, cache, target, &settings);
                let bounds = fit_bounds(&scene);
                let simulation = PreviewSimulation::new(&scene);
                Current { generation, scene, simulation, bounds }
            }));
            match built {
                Ok(current) => {
                    responses.push(WorkerResponse::Built(BuiltScene {
                        generation,
                        target,
                        parts: current.scene.parts.clone(),
                        notes: current.scene.notes.clone(),
                        playing_parts: current.scene.playing_part_count(),
                        has_hued_parts: current.simulation.has_hued_parts(),
                        bounds: current.bounds,
                    }));
                    self.current = Some(current);
                }
                Err(panic) => {
                    // A half-filled cache entry is not possible (entries are inserted whole), but drop it anyway.
                    self.cache = ResourceCache::default();
                    responses.push(WorkerResponse::Failed { generation, error: panic_message(&panic) });
                }
            }
        }
        if let (Some((time_ms, view)), Some(current)) = (batch.render, self.current.as_mut()) {
            let generation = current.generation;
            let atlas = self.hue_atlas.as_deref();
            let rendered = catch_unwind(AssertUnwindSafe(|| render(current, time_ms, &view, atlas)));
            responses.push(match rendered {
                Ok(frame) => WorkerResponse::Frame(frame),
                Err(panic) => {
                    self.current = None;
                    WorkerResponse::Failed { generation, error: panic_message(&panic) }
                }
            });
        }
        responses
    }
}

fn render(current: &mut Current, time_ms: u32, view: &ViewSettings, atlas: Option<&RasterTexture>) -> RenderedFrame {
    if time_ms < current.simulation.time_ms() {
        current.simulation = PreviewSimulation::new(&current.scene);
    }
    current.simulation.advance_to(time_ms);
    let ortho = preview_view(current.bounds, view.zoom, view.resolution_px);
    let canvas = render_canvas(&current.scene, &current.simulation, &ortho, view, atlas);
    RenderedFrame {
        generation: current.generation,
        time_ms: current.simulation.time_ms(),
        width: ortho.width,
        height: ortho.height,
        rgba: canvas.to_rgba8(),
        particles: current.simulation.particle_count(),
        drained: current.simulation.is_drained(),
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    let message = panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned());
    format!("the preview failed: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effects::model::RootKey;
    use crate::logic::effects::test_package::{self, TestPackage, ids};

    const RENDER_TIME_MS: u32 = 500;

    #[test]
    fn builds_then_renders_the_latest_request() {
        let (package, _) = test_package::open(TestPackage::standard());
        let worker = PreviewWorker::spawn(package, || {});
        let target = PreviewTarget::Root(RootKey::Collection(ids::PLAIN));
        assert!(worker.send(WorkerRequest::Build { generation: 7, target, settings: SimulationSettings::default() }));
        let view = ViewSettings::default();
        assert!(worker.send(WorkerRequest::Render { time_ms: RENDER_TIME_MS, view }));
        let mut built = None;
        let mut frame = None;
        while frame.is_none() {
            match worker.wait().expect("worker answers") {
                WorkerResponse::Built(scene) => built = Some(scene),
                WorkerResponse::Frame(rendered) => frame = Some(rendered),
                WorkerResponse::Failed { error, .. } => panic!("{error}"),
            }
        }
        let (built, frame) = (built.unwrap(), frame.unwrap());
        assert_eq!((built.generation, built.playing_parts), (7, 1));
        assert_eq!((frame.generation, frame.width), (7, view.resolution_px));
        assert_eq!(frame.rgba.len(), frame.width * frame.height * 4);
        assert!(frame.particles > 0);

        // Scrubbing back re-simulates from 0.
        assert!(worker.send(WorkerRequest::Render { time_ms: 0, view }));
        match worker.wait() {
            Some(WorkerResponse::Frame(rendered)) => assert_eq!(rendered.time_ms, 0),
            _ => panic!("a frame"),
        }
        assert!(worker.is_running());
    }
}
