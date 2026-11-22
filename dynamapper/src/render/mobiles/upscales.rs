//! The SPAN upscales of the mobiles' frames (`MobileUpscale::SpanX2`): while the view magnifies the art enough
//! (`MobileUpscaleGate`), the sequence a mobile shows at 1x, its clip or the clip's RIFE sequence (rife_in_betweens.rs),
//! is upscaled 2x by the SPAN network (uot-gpu-compute's upscale) frame by frame on the GPU, and the mobile shows the
//! 2x frames at the same size in the world once all are made. The frames never leave the GPU: the render world reads
//! each from the frame atlas into a SPAN workspace and writes its 2x canvas back into the atlas (render.rs), encoded to
//! BC7 for the BC7 atlas.
//!
//! An upscaled sequence is an atlas entry of its own in the atlas's upscaled pool (`AtlasPool::Upscaled`), so it never
//! empties the pages of the 1x frames. Each frame keeps the transparent border the network sees around it
//! (`UPSCALE_BORDER`): its placement grows by it, so the halo the network may draw past the frame's edge shows. A
//! mobile switches between its 1x and 2x frames only as its sequence starts over (`UpscaleSwitch`), so a cycle never
//! mixes the two, except that it drops 2x frames the atlas emptied at once.
//!
//! The main world takes the atlas room and lists the jobs each frame, those of the sequences drawn first, nearest the
//! player first, then the largest on screen, then those started of sequences drawn lately (`UPSCALE_RESUME_SECS`);
//! the render world works through them within what the in-betweens leave of the mobiles' compute budget
//! (compute_budget.rs): the in-betweens go first, as the upscales upscale the sequences they make, and a RIFE mode's
//! clip upscales only once its sequence is made. A job not listed any more is dropped, and starts over when listed
//! again.

pub mod render;

use std::collections::HashMap;
use std::sync::Arc;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uot_gpu_compute::upscale::{upscale_canvas_size, UPSCALE_BORDER, UPSCALE_OUTPUT_SCALE};
use crate::render::view::camera::RenderZoom;
use crate::settings::Settings;
use super::clips::{AnimSource, AtlasEntry, Clip, ClipFrame, ClipKey, MobileClips};
use super::frame_atlas::{AtlasPool, AtlasRect, MobileFrameAtlas, MOBILE_ATLAS_UPSCALED_PAGES};
use super::placement::FramePlacement;
use super::rife_in_betweens::{InBetweenKey, MobileInBetweens};

/// The magnification (screen pixels per art pixel) the upscales start at, and the one they stop below: apart, so a
/// zoom around one does not switch back and forth.
pub const UPSCALE_ON_MAGNIFICATION: f32 = 1.5;
pub const UPSCALE_OFF_MAGNIFICATION: f32 = 1.35;
/// A sequence takes atlas room while drawn this recently, and its job runs first.
const UPSCALE_QUEUE_KEEP_SECS: f64 = 2.0;
/// A job already started keeps running, after those of the sequences drawn, while its sequence was drawn this
/// recently: a mobile that stops and walks on finds its walk's frames made, not started over.
const UPSCALE_RESUME_SECS: f64 = 10.0;
/// A job's canvas, its largest frame with the border, is rounded up to this many pixels, so the jobs of a body's
/// actions share a workspace.
pub const UPSCALE_CANVAS_MULTIPLE: u32 = 8;
/// The largest canvas upscaled, in pixels: 260 by 260 takes about 90 MB of workspace and 30 ms on the Radeon 880M. A
/// sequence with a larger frame plays at 1x.
pub const UPSCALE_MAX_CANVAS_PIXELS: u32 = 260 * 260;
/// The distance to the player, in tiles, under which two mobiles count as equally near and the larger one on screen
/// goes first.
const UPSCALE_DISTANCE_STEP_TILES: f32 = 1.0;

/// Whether the mobiles' frames are upscaled while the view magnifies them. Kept in the app-written world rendering
/// settings as `world_rendering.mobile_upscale`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileUpscale {
    /// The frames as stored.
    #[default]
    Off,
    /// The SPAN network's 2x frames.
    SpanX2,
}

impl MobileUpscale {
    pub const ALL: [Self; 2] = [Self::Off, Self::SpanX2];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::SpanX2 => "SPAN 2x",
        }
    }

    /// The atlases' upscaled pages.
    fn atlas_pages(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::SpanX2 => MOBILE_ATLAS_UPSCALED_PAGES,
        }
    }
}

/// Whether the view magnifies the art enough to upscale it: on at `UPSCALE_ON_MAGNIFICATION`, off again below
/// `UPSCALE_OFF_MAGNIFICATION`.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct MobileUpscaleGate {
    pub open: bool,
    /// The screen pixels per art pixel (`view_magnification`).
    pub magnification: f32,
}

impl MobileUpscaleGate {
    fn update(&mut self, magnification: f32) {
        self.magnification = magnification;
        self.open = if self.open {
            magnification >= UPSCALE_OFF_MAGNIFICATION
        } else {
            magnification >= UPSCALE_ON_MAGNIFICATION
        };
    }
}

/// The screen pixels per art pixel: a zoom of 1 draws an art pixel on a logical pixel.
pub fn view_magnification(scale_factor: f32, zoom: f32) -> f32 {
    scale_factor / zoom
}

/// Opens or closes the gate by the view's magnification.
pub fn sys_update_mobile_upscale_gate(
    zoom: Res<RenderZoom>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut gate: ResMut<MobileUpscaleGate>,
) {
    let Ok(window) = window_q.single() else { return };
    let mut next = *gate;
    next.update(view_magnification(window.scale_factor(), zoom.0));
    if next.open != gate.open {
        debug!("Mobile upscales: the view's magnification {:.2} {} them.", next.magnification,
            if next.open { "starts" } else { "stops" });
    }
    if next != *gate {
        *gate = next;
    }
}

/// One frame of a job: a frame of the 1x sequence, and where its 2x canvas goes, from the canvas's top left
/// (`UPSCALE_BORDER` before the frame's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpscaleFrame {
    pub source: AtlasRect,
    pub target: AtlasRect,
}

/// The upscaling of a sequence, as the render world runs it: its frames in order on one canvas size.
#[derive(Clone, Debug, PartialEq)]
pub struct UpscaleJob {
    pub id: u64,
    /// The 2x alpha snaps to 0 or 1, as the Classic frames' is.
    pub snap_alpha: bool,
    /// The workspace's canvas: the largest frame with its border, rounded up to `UPSCALE_CANVAS_MULTIPLE`.
    pub canvas: UVec2,
    pub frames: Vec<UpscaleFrame>,
}

enum UpscaleState {
    /// Drawn, without room in the atlas yet.
    Wanted,
    /// The 2x frames have their room, and the render world fills them as `job`; none while not queued.
    Generating { sequence: Clip, job: Option<u64> },
    Ready(Clip),
    /// The sequence does not fit the upscaled pages, or its canvas would be larger than `UPSCALE_MAX_CANVAS_PIXELS`.
    Missing,
}

struct UpscaleEntry {
    state: UpscaleState,
    /// The clip drawn last showing the sequence.
    source: ClipKey,
    first_drawn: f64,
    last_drawn: f64,
    /// When the mobile last drawn with it was nearest the player, and how large on screen (`UpscalePriority`).
    priority: UpscalePriority,
}

impl UpscaleEntry {
    fn queued(&self, now: f64) -> bool {
        now - self.last_drawn <= UPSCALE_QUEUE_KEEP_SECS
    }

    fn resumable(&self, now: f64) -> bool {
        now - self.last_drawn <= UPSCALE_RESUME_SECS
    }
}

/// Which sequence upscales first: the nearest the player, then the largest on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UpscalePriority {
    /// The distance to the player, in tiles.
    pub distance: f32,
    /// The screen pixels its frames cover.
    pub area: f32,
}

impl UpscalePriority {
    /// Orders the sooner first.
    fn order(&self, other: &Self) -> std::cmp::Ordering {
        let step = |distance: f32| (distance / UPSCALE_DISTANCE_STEP_TILES).floor();
        step(self.distance).total_cmp(&step(other.distance)).then(other.area.total_cmp(&self.area))
    }
}

/// The 2x sequences asked for, the jobs listed for the render world this frame, and the jobs it finished since.
#[derive(Resource, Default)]
pub struct MobileUpscales {
    entries: HashMap<InBetweenKey, UpscaleEntry>,
    next_job: u64,
    jobs: Vec<UpscaleJob>,
    /// Whether the upscales are on: the render world loads the network then.
    active: bool,
    finished: Arc<Mutex<Vec<u64>>>,
}

impl MobileUpscales {
    /// Asks for the 2x frames of the sequence of `key`, shown by clip `source`, drawn at `now` with `priority`.
    pub fn want(&mut self, key: InBetweenKey, source: ClipKey, priority: UpscalePriority, now: f64) {
        let entry = self.entries.entry(key).or_insert(UpscaleEntry {
            state: UpscaleState::Wanted, source, first_drawn: now, last_drawn: now, priority,
        });
        if !entry.queued(now) {
            entry.first_drawn = now;
        }
        entry.last_drawn = now;
        entry.source = source;
        entry.priority = priority;
    }

    /// The 2x frames of the sequence of `key`, once all are made.
    pub fn ready(&self, key: &InBetweenKey) -> Option<&Clip> {
        match &self.entries.get(key)?.state {
            UpscaleState::Ready(sequence) => Some(sequence),
            _ => None,
        }
    }

    /// Forgets the 2x sequences of `evicted`, entries of pages the atlas emptied.
    pub fn forget(&mut self, evicted: &[AtlasEntry]) {
        for entry in evicted {
            if let AtlasEntry::Upscaled(key) = entry {
                self.entries.remove(key);
            }
        }
    }

    /// Forgets every 2x sequence, when the atlas is remade.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.jobs.clear();
    }

    /// Takes the jobs the render world finished, makes room for the sequences drawn, and lists the jobs to run, the
    /// soonest first (`UpscalePriority`), each with its 1x sequence where it is now (`base`). Returns the entries of the
    /// pages emptied for the room.
    fn schedule<'c>(&mut self, atlas: &mut MobileFrameAtlas<AtlasEntry>,
        base: impl Fn(&InBetweenKey, ClipKey) -> Option<&'c Clip>, active: bool, now: f64) -> Vec<AtlasEntry> {
        self.active = active;
        let finished = std::mem::take(&mut *self.finished.lock());
        for (key, entry) in self.entries.iter_mut() {
            if let UpscaleState::Generating { job: Some(job), .. } = &entry.state && finished.contains(job) {
                let UpscaleState::Generating { sequence, .. } =
                    std::mem::replace(&mut entry.state, UpscaleState::Missing) else { unreachable!() };
                debug!("Mobile upscales: {:?} at 2x in {:.2} s.", key, now - entry.first_drawn);
                entry.state = UpscaleState::Ready(sequence);
            }
        }
        self.entries.retain(|_, entry| entry.queued(now) || !matches!(entry.state, UpscaleState::Wanted));
        if !active {
            self.jobs.clear();
            return Vec::new();
        }

        let mut wanted = self.entries.iter_mut()
            .filter(|(_, entry)| matches!(entry.state, UpscaleState::Wanted) && entry.queued(now))
            .collect::<Vec<_>>();
        wanted.sort_by(|(_, a), (_, b)| a.priority.order(&b.priority));
        let mut evicted = Vec::new();
        for (key, entry) in wanted {
            let Some(clip) = base(key, entry.source) else { continue };
            let Some(format) = clip.frames.first().map(|frame| frame.rect.format) else { continue };
            if canvas_of(clip).is_none() {
                let largest = clip.frames.iter().map(|frame| (frame.rect.width, frame.rect.height))
                    .max_by_key(|(width, height)| width * height).unwrap_or_default();
                warn!("Mobile clip {:?} has a frame of {}x{}, too large to upscale, so it plays at 1x.", entry.source,
                    largest.0, largest.1);
                entry.state = UpscaleState::Missing;
                continue;
            }
            let placements = clip.frames.iter().map(|frame| upscaled_placement(frame.rect, frame.placement))
                .collect::<Vec<_>>();
            let sizes = clip.frames.iter().map(|frame| upscaled_size(frame.rect)).collect::<Vec<_>>();
            // The pages drawn this frame stay: without one free of them, the sequence waits.
            if !atlas.has_room_sparing_in(format, AtlasPool::Upscaled, &sizes, now) {
                continue;
            }
            entry.state = match atlas.reserve_in(AtlasEntry::Upscaled(*key), format, AtlasPool::Upscaled, &sizes, now) {
                Some((rects, page_evicted)) => {
                    evicted.extend(page_evicted);
                    UpscaleState::Generating {
                        sequence: Clip {
                            frames: rects.into_iter().zip(placements)
                                .map(|(rect, placement)| ClipFrame { rect, placement }).collect(),
                            mirrored: clip.mirrored,
                            frame_secs: clip.frame_secs,
                            stored: clip.stored,
                            texel_scale: UPSCALE_OUTPUT_SCALE,
                        },
                        job: None,
                    }
                }
                None => {
                    warn!("The 2x frames of mobile clip {:?} do not fit the frame atlas, so it plays at 1x.",
                        entry.source);
                    UpscaleState::Missing
                }
            };
        }
        self.forget(&evicted);

        // The sequences drawn first, then those started that were drawn lately.
        let mut queued = self.entries.iter_mut()
            .filter(|(_, entry)| matches!(entry.state, UpscaleState::Generating { .. }))
            .collect::<Vec<_>>();
        queued.sort_by(|(_, a), (_, b)| b.queued(now).cmp(&a.queued(now)).then(a.priority.order(&b.priority)));
        self.jobs.clear();
        for (key, entry) in queued {
            let listed = match entry.state {
                UpscaleState::Generating { job, .. } => entry.queued(now) || job.is_some() && entry.resumable(now),
                _ => false,
            };
            let clip = base(key, entry.source).filter(|_| listed);
            let UpscaleState::Generating { sequence, job } = &mut entry.state else { continue };
            let (Some(clip), Some(canvas)) = (clip, clip.and_then(canvas_of)) else {
                *job = None;
                continue;
            };
            let id = *job.get_or_insert_with(|| {
                self.next_job += 1;
                self.next_job
            });
            // Its pages are in use while it is made, though not drawn yet.
            sequence.touch_pages(atlas, now);
            self.jobs.push(UpscaleJob {
                id,
                snap_alpha: key.source == AnimSource::Classic,
                canvas,
                frames: clip.frames.iter().zip(&sequence.frames)
                    .map(|(frame, upscaled)| UpscaleFrame { source: frame.rect, target: upscaled.rect }).collect(),
            });
        }
        evicted
    }
}

/// The placement of a frame's 2x canvas cut to the frame and its border: the frame's grown by `UPSCALE_BORDER` on
/// each side, in the frame's pixels.
fn upscaled_placement(rect: AtlasRect, placement: FramePlacement) -> FramePlacement {
    let border = UPSCALE_BORDER as f32;
    FramePlacement {
        width: (rect.width + 2 * UPSCALE_BORDER) as f32,
        height: (rect.height + 2 * UPSCALE_BORDER) as f32,
        anchor_x: placement.anchor_x + border,
        anchor_y: placement.anchor_y + border,
    }
}

/// The room of a frame's 2x canvas cut to the frame and its border, in texels.
fn upscaled_size(rect: AtlasRect) -> (u32, u32) {
    let (width, height) = upscale_canvas_size(rect.width, rect.height);
    (width * UPSCALE_OUTPUT_SCALE, height * UPSCALE_OUTPUT_SCALE)
}

/// The canvas of a sequence's frames (`UpscaleJob::canvas`), unless larger than `UPSCALE_MAX_CANVAS_PIXELS`.
fn canvas_of(clip: &Clip) -> Option<UVec2> {
    let canvas = clip.frames.iter().fold(UVec2::ONE, |canvas, frame| {
        let (width, height) = upscale_canvas_size(frame.rect.width, frame.rect.height);
        canvas.max(UVec2::new(width, height))
    });
    let canvas = UVec2::new(canvas.x.next_multiple_of(UPSCALE_CANVAS_MULTIPLE),
        canvas.y.next_multiple_of(UPSCALE_CANVAS_MULTIPLE));
    (canvas.x * canvas.y <= UPSCALE_MAX_CANVAS_PIXELS).then_some(canvas)
}

/// The sequence a clip shows at 1x and upscales: its RIFE sequence in a RIFE mode, else the clip, as one of factor 1.
pub fn upscale_key(key: ClipKey, clip: &Clip, factor: u32) -> InBetweenKey {
    InBetweenKey::new(key, clip, factor)
}

/// Where a mobile layer stands between its 1x and 2x frames: it switches only as its sequence starts over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpscaleSwitch {
    pub tie_level: u32,
    pub key: InBetweenKey,
    pub upscaled: bool,
    /// The frame drawn last.
    pub index: usize,
}

impl UpscaleSwitch {
    /// Whether the layer at frame `index` of the sequence of `key` (of `frames` frames) shows its 2x frames, after
    /// `last`, the layer's switch the frame before: as soon as they are `ready` for a sequence just started or of one
    /// frame, else only as the sequence starts over (its frame index going back) when they are `wanted` and ready,
    /// and never once they are gone.
    pub fn next(last: Option<&Self>, tie_level: u32, key: InBetweenKey, frames: usize, index: usize, wanted: bool,
        ready: bool) -> Self {
        let target = wanted && ready;
        let upscaled = match last.filter(|last| last.key == key && frames > 1) {
            None => target,
            Some(_) if !ready => false,
            Some(last) if index < last.index => target,
            Some(last) => last.upscaled,
        };
        Self { tie_level, key, upscaled, index }
    }
}

/// The switches of a mobile's layers (`UpscaleSwitch`), one per tie level drawn.
#[derive(Clone, Debug, Default)]
pub struct MobileUpscaleSwitches(pub Vec<UpscaleSwitch>);

impl MobileUpscaleSwitches {
    pub fn get(&self, tie_level: u32) -> Option<&UpscaleSwitch> {
        self.0.iter().find(|switch| switch.tie_level == tie_level)
    }
}

/// Takes the jobs finished, makes room for the sequences drawn and lists the jobs for the render world.
pub fn sys_schedule_mobile_upscales(
    time: Res<Time>,
    settings: Res<Settings>,
    mut upscales: ResMut<MobileUpscales>,
    atlas: Option<ResMut<MobileFrameAtlas<AtlasEntry>>>,
    clips: Res<MobileClips>,
    in_betweens: Res<MobileInBetweens>,
) {
    let Some(mut atlas) = atlas else { return };
    let active = settings.world_rendering.mobile_upscale != MobileUpscale::Off && atlas.upscaled_pages() > 0;
    let base = |key: &InBetweenKey, source: ClipKey| -> Option<&Clip> {
        if key.factor > 1 {
            return in_betweens.ready(key);
        }
        clips.resident(source).filter(|clip| upscale_key(source, clip, 1) == *key)
    };
    // Only upscaled entries empty for upscaled ones, and they are forgotten here.
    let _ = upscales.schedule(&mut atlas, base, active, time.elapsed_secs_f64());
}

/// The upscaled pages the setting gives the atlases (`MobileFrameAtlas::set_upscaled_pages`).
pub fn atlas_pages_wanted(settings: &Settings) -> u32 {
    settings.world_rendering.mobile_upscale.atlas_pages()
}

#[cfg(test)]
mod tests;
