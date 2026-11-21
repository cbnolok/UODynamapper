//! Shaders and pipelines compiling, so a cold driver cache explains itself: a pipeline's first compile on a GPU can
//! take seconds (RADV on the Radeon 880M with Mesa's shader cache empty: 38.6 s for the BC7 compressor, 25 ms once
//! cached), and until then Bevy skips the draws that need it. Two sources feed one book: Bevy's render-world
//! `PipelineCache`, synced each frame ([`sys_sync_pipeline_compiles`]), and the app's own wgpu compiles outside it,
//! each held by a [`ShaderCompileGuard`] while it runs ([`begin_shader_compile`]), or reported by
//! [`shader_compile_started`] and [`shader_compile_ended`] from any thread. The compiles of a burst count as
//! one batch, and its overlay (`ui::overlays::shader_compiles`) reads [`shader_compile_view`].

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipeline, CachedPipelineState, PipelineCache, PipelineDescriptor};
use bevy::render::{Render, RenderApp, RenderSystems};
use parking_lot::Mutex;
use crate::prelude::*;

#[cfg(test)]
mod tests;

/// A compile that runs this long is slow: its time shows while it runs, and it is logged when it ends. A Bevy
/// pipeline's time counts from its queueing: Bevy's async compute pool (one thread here, `app.rs`) compiles the
/// queued pipelines one after another, so a pipeline's time includes those compiled before it.
pub const SLOW_COMPILE: Duration = Duration::from_secs(1);
/// A batch shows once one of its compiles has run this long, so a warm cache's quick compiles never flash it.
pub const SHOW_AFTER: Duration = Duration::from_millis(300);
/// The batch stays open this long after its last compile ends, so the compiles queued a frame or two later join its
/// count instead of starting another.
pub const BATCH_LINGER: Duration = Duration::from_millis(500);
/// The names listed at most; the others are counted.
pub const MAX_LISTED: usize = 3;
/// The name of a Bevy pipeline with no label and no shader path.
const UNNAMED_PIPELINE: &str = "unnamed pipeline";

pub struct ShaderCompilesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ShaderCompilesPlugin);

impl Plugin for ShaderCompilesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.add_systems(Render, sys_sync_pipeline_compiles.in_set(RenderSystems::Cleanup));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompileKey {
    /// A Bevy pipeline cache entry, by its index.
    Pipeline(usize),
    /// One of the app's own compiles.
    Own(u64),
}

struct Compile {
    key: CompileKey,
    name: String,
    started: Instant,
}

struct Batch {
    done: usize,
    failed: usize,
    /// Once shown, the batch stays shown until it closes.
    shown: bool,
    last_end: Option<Instant>,
}

/// A compile that ended.
#[derive(Debug, PartialEq)]
pub struct Finished {
    pub name: String,
    pub elapsed: Duration,
    pub failed: bool,
}

/// A compile running, as listed.
#[derive(Debug, PartialEq)]
pub struct Listed {
    pub name: String,
    pub elapsed: Duration,
}

/// The batch as the overlay shows it.
#[derive(Debug, PartialEq)]
pub struct CompileView {
    pub done: usize,
    /// The compiles done and those running.
    pub total: usize,
    pub failed: usize,
    /// The compiles running longest, first, at most [`MAX_LISTED`].
    pub listed: Vec<Listed>,
    /// The compiles running and not listed.
    pub more: usize,
}

/// The compiles running and the batch they belong to.
#[derive(Default)]
pub struct CompileBook {
    /// In the order they began.
    compiling: Vec<Compile>,
    batch: Option<Batch>,
    next_own: u64,
}

impl CompileBook {
    /// Closes the batch once nothing has compiled for [`BATCH_LINGER`].
    fn close_if_idle(&mut self, now: Instant) {
        let idle = self.compiling.is_empty() && self.batch.as_ref()
            .is_some_and(|batch| batch.last_end.is_none_or(|end| now.saturating_duration_since(end) >= BATCH_LINGER));
        if idle {
            self.batch = None;
        }
    }

    fn begin(&mut self, key: CompileKey, name: String, now: Instant) {
        self.close_if_idle(now);
        self.batch.get_or_insert(Batch { done: 0, failed: 0, shown: false, last_end: None });
        self.compiling.push(Compile { key, name, started: now });
    }

    fn begin_own(&mut self, name: String, now: Instant) -> u64 {
        let id = self.next_own;
        self.next_own += 1;
        self.begin(CompileKey::Own(id), name, now);
        id
    }

    fn end(&mut self, key: CompileKey, failed: bool, now: Instant) -> Option<Finished> {
        let index = self.compiling.iter().position(|compile| compile.key == key)?;
        let compile = self.compiling.remove(index);
        let elapsed = now.saturating_duration_since(compile.started);
        if let Some(batch) = self.batch.as_mut() {
            batch.done += 1;
            batch.failed += failed as usize;
            // A compile that ran long shows its batch even when it ended between two views.
            batch.shown |= elapsed >= SHOW_AFTER;
            batch.last_end = Some(now);
        }
        Some(Finished { name: compile.name, elapsed, failed })
    }

    /// Follows the pipeline cache: `waiting` are the entries not ready yet, `name` names a new one and `failed` tells
    /// whether one that left the waiting ones failed. Returns the compiles that ended.
    fn sync_pipelines(&mut self, waiting: &HashSet<usize>, mut name: impl FnMut(usize) -> String,
        mut failed: impl FnMut(usize) -> bool, now: Instant) -> Vec<Finished> {
        let ended = self.compiling.iter().filter_map(|compile| match compile.key {
            CompileKey::Pipeline(id) if !waiting.contains(&id) => Some(id),
            _ => None,
        }).collect::<Vec<_>>();
        let finished = ended.into_iter()
            .filter_map(|id| self.end(CompileKey::Pipeline(id), failed(id), now)).collect();
        let known = self.compiling.iter().filter_map(|compile| match compile.key {
            CompileKey::Pipeline(id) => Some(id),
            CompileKey::Own(_) => None,
        }).collect::<HashSet<_>>();
        let mut new = waiting.iter().copied().filter(|id| !known.contains(id)).collect::<Vec<_>>();
        new.sort_unstable();
        for id in new {
            self.begin(CompileKey::Pipeline(id), name(id), now);
        }
        finished
    }

    fn has_pipelines(&self) -> bool {
        self.compiling.iter().any(|compile| matches!(compile.key, CompileKey::Pipeline(_)))
    }

    /// The batch to show, if any: from when one of its compiles has run [`SHOW_AFTER`] until it closes.
    fn view(&mut self, now: Instant) -> Option<CompileView> {
        self.close_if_idle(now);
        let batch = self.batch.as_mut()?;
        let mut running = self.compiling.iter()
            .map(|compile| (compile, now.saturating_duration_since(compile.started))).collect::<Vec<_>>();
        batch.shown |= running.iter().any(|(_, elapsed)| *elapsed >= SHOW_AFTER);
        if !batch.shown {
            return None;
        }
        running.sort_by_key(|(_, elapsed)| std::cmp::Reverse(*elapsed));
        let more = running.len().saturating_sub(MAX_LISTED);
        Some(CompileView {
            done: batch.done,
            total: batch.done + running.len(),
            failed: batch.failed,
            listed: running.into_iter().take(MAX_LISTED)
                .map(|(compile, elapsed)| Listed { name: compile.name.clone(), elapsed }).collect(),
            more,
        })
    }
}

fn book() -> &'static Mutex<CompileBook> {
    static BOOK: OnceLock<Mutex<CompileBook>> = OnceLock::new();
    BOOK.get_or_init(Default::default)
}

fn log_finished(finished: &Finished) {
    if !finished.failed && finished.elapsed >= SLOW_COMPILE {
        info!("Shader compile: {} ready after {:.1} s.", finished.name, finished.elapsed.as_secs_f64());
    }
}

/// One of the app's own compiles, outside Bevy's pipeline cache: plain data, so a compile reported from any thread
/// (a background task, a warm-up's callback) needs no Bevy types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderCompileId(u64);

/// Shows `name` compiling until [`shader_compile_ended`] ends it.
pub fn shader_compile_started(name: impl Into<String>) -> ShaderCompileId {
    ShaderCompileId(book().lock().begin_own(name.into(), Instant::now()))
}

/// Ends a compile [`shader_compile_started`] began, logging it when slow. Ending it again does nothing.
pub fn shader_compile_ended(id: ShaderCompileId) {
    let finished = book().lock().end(CompileKey::Own(id.0), false, Instant::now());
    if let Some(finished) = finished {
        log_finished(&finished);
    }
}

/// One of the app's own compiles, shown until the guard drops.
#[must_use = "the compile ends when the guard drops"]
pub struct ShaderCompileGuard(ShaderCompileId);

/// Shows `name` compiling until the guard returned drops. Hold it around the pipeline creation.
pub fn begin_shader_compile(name: impl Into<String>) -> ShaderCompileGuard {
    ShaderCompileGuard(shader_compile_started(name))
}

impl Drop for ShaderCompileGuard {
    fn drop(&mut self) {
        shader_compile_ended(self.0);
    }
}

/// The batch compiling, once it shows.
pub fn shader_compile_view() -> Option<CompileView> {
    book().lock().view(Instant::now())
}

/// A Bevy pipeline's label, with the file of its fragment, mesh or compute shader.
fn pipeline_name(pipeline: &CachedPipeline) -> String {
    let (label, shader) = match &pipeline.descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(descriptor) => (&descriptor.label,
            descriptor.fragment.as_ref().map_or(&descriptor.vertex.shader, |fragment| &fragment.shader)),
        PipelineDescriptor::ComputePipelineDescriptor(descriptor) => (&descriptor.label, &descriptor.shader),
        PipelineDescriptor::MeshPipelineDescriptor(descriptor) => (&descriptor.label, &descriptor.mesh.shader),
    };
    let file = shader.path().and_then(|path| path.path().file_name()).map(|file| file.to_string_lossy());
    match (label, file) {
        (Some(label), Some(file)) => format!("{label} ({file})"),
        (Some(label), None) => label.to_string(),
        (None, Some(file)) => file.into_owned(),
        (None, None) => UNNAMED_PIPELINE.to_string(),
    }
}

/// Brings the book in step with the pipeline cache, after the frame's `process_queue`.
fn sys_sync_pipeline_compiles(cache: Res<PipelineCache>) {
    let waiting = cache.waiting_pipelines().collect::<HashSet<_>>();
    let mut book = book().lock();
    if waiting.is_empty() && !book.has_pipelines() {
        return;
    }
    let pipelines = cache.pipelines().collect::<Vec<_>>();
    let finished = book.sync_pipelines(&waiting,
        |id| pipelines.get(id).map_or_else(|| UNNAMED_PIPELINE.to_string(), |pipeline| pipeline_name(pipeline)),
        |id| pipelines.get(id).is_some_and(|pipeline| matches!(pipeline.state, CachedPipelineState::Err(_))),
        Instant::now());
    drop(book);
    finished.iter().for_each(log_finished);
}
