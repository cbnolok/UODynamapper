//! The RIFE in-betweens of the selected clip, as Dynamapper's RIFE modes show them: the clip's frames (×1), or each
//! followed by one (×2) or three (×4) frames RIFE makes toward the next, the last toward the first, played in the
//! clip's cycle. They are computed on the GPU (`uot_gpu_compute::in_betweens`, the nearest final warp as in game) on a
//! thread of their own, kept per clip and factor, and dropped half-way when another clip or factor is picked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use color_eyre::eyre;
use eframe::egui;
use udd_assets::animation::mobile_strides::MobileClip;
use uot_gpu_compute::in_betweens::InBetweenMaker;
use uot_gpu_compute::rife::RIFE_COMPILES;
use uot_gpu_compute::strides::StrideFrame;

/// The frames shown for each of a clip's: its own alone, and with RIFE's in-betweens (Dynamapper's RIFE x2 and x4).
const FACTORS: [u32; 3] = [1, 2, 4];
/// The time a frame shows without an fps: Classic MUL bodies, as Dynamapper's `CLASSIC_FRAME_SECS` (render/mobiles/
/// clips.rs) and the Classic client's 80 ms.
const CLASSIC_FRAME_SECS: f32 = 0.08;
/// Sequences kept, the oldest dropped first.
const MAX_KEPT_SEQUENCES: usize = 24;
const DEFAULT_ZOOM: f32 = 2.0;
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 8.0;
const PROGRESS_BAR_WIDTH: f32 = 360.0;
const RGBA_BYTES: usize = 4;
const FRAME_UV: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
const MIRRORED_FRAME_UV: egui::Rect = egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0));

/// Reads the selected clip, on the job's thread.
pub type ClipReader = Box<dyn FnOnce() -> eyre::Result<Option<MobileClip>> + Send>;

/// A clip of the open package (its animation record) at a factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SequenceKey {
    animation: usize,
    factor: u32,
}

/// A sequence's frames, each on the canvas of the clip with the anchors on one point.
struct Composed {
    canvas: [usize; 2],
    images: Vec<Vec<u8>>,
    frame_secs: f32,
    mirrored: bool,
    clip_frames: usize,
    compute_secs: f32,
    /// The adapter that made the in-betweens; none at ×1.
    adapter: Option<String>,
}

struct Sequence {
    key: SequenceKey,
    textures: Vec<egui::TextureHandle>,
    composed: Composed,
}

/// What a job does now, and how far it is (0 to 1).
#[derive(Clone, Default)]
struct Progress {
    activity: String,
    fraction: f32,
}

struct Job {
    key: SequenceKey,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    thread: Option<JoinHandle<eyre::Result<Option<Composed>>>>,
}

/// A job dropped before it ended is cancelled.
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The in-between preview of the open mobile animation package.
pub struct InBetweenPreview {
    factor: u32,
    /// The GPU device and RIFE, made by the first job that needs them and kept for the next.
    maker: Arc<Mutex<Option<InBetweenMaker>>>,
    sequences: Vec<Sequence>,
    job: Option<Job>,
    failure: Option<(SequenceKey, String)>,
    zoom: f32,
    /// The animation shown last and the height its preview took, kept while another factor of it is made so the
    /// panel under the preview stays where it was.
    shown: Option<(usize, f32)>,
}

impl Default for InBetweenPreview {
    fn default() -> Self {
        Self { factor: FACTORS[0], maker: Default::default(), sequences: Vec::new(), job: None, failure: None,
            zoom: DEFAULT_ZOOM, shown: None }
    }
}

impl InBetweenPreview {
    /// Forgets the clips of the package closed; the GPU stays.
    pub fn reset(&mut self) {
        self.sequences.clear();
        self.job = None;
        self.failure = None;
        self.shown = None;
    }

    /// The preview of animation record `animation`, its clip read by `reader` when it is not kept; Classic frames'
    /// in-betweens with `snap_alpha`.
    pub fn ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, animation: usize, snap_alpha: bool,
        reader: impl FnOnce() -> ClipReader) {
        ui.horizontal(|ui| {
            ui.strong("In-betweens");
            for factor in FACTORS {
                ui.selectable_value(&mut self.factor, factor, format!("×{factor}")).on_hover_text(match factor {
                    1 => "The clip's own frames".to_string(),
                    _ => format!("Each frame followed by {} made by RIFE toward the next (nearest final warp, as \
                        Dynamapper's RIFE x{factor})", factor - 1),
                });
            }
        });
        let key = SequenceKey { animation, factor: self.factor };
        if self.job.as_ref().is_some_and(|job| job.key != key) {
            self.job = None;
        }
        self.poll(ctx);
        let top = ui.cursor().top();
        if let Some(sequence) = self.sequences.iter().find(|sequence| sequence.key == key) {
            show_sequence(ctx, ui, sequence, &mut self.zoom);
            self.shown = Some((animation, ui.cursor().top() - top));
            return;
        }
        if let Some((_, error)) = self.failure.as_ref().filter(|(failed, _)| *failed == key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        if self.job.is_none() {
            self.start(ctx, key, snap_alpha, reader());
        }
        if let Some(job) = &self.job {
            let progress = job.progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.add(egui::ProgressBar::new(progress.fraction).desired_width(PROGRESS_BAR_WIDTH)
                    .text(progress.activity));
            });
        }
        if let Some((_, height)) = self.shown.filter(|(shown, _)| *shown == animation) {
            ui.add_space((height - (ui.cursor().top() - top)).max(0.0));
        }
    }

    /// Takes the result of the job once it ended.
    fn poll(&mut self, ctx: &egui::Context) {
        if !self.job.as_ref().and_then(|job| job.thread.as_ref()).is_some_and(JoinHandle::is_finished) {
            return;
        }
        let Some(mut job) = self.job.take() else { return };
        let Some(thread) = job.thread.take() else { return };
        let result = thread.join().unwrap_or_else(|_| Err(eyre::eyre!("the in-between work panicked")));
        match result {
            Ok(Some(composed)) => {
                let textures = composed.images.iter().enumerate().map(|(index, image)| {
                    ctx.load_texture(format!("in_between_{}_{}_{index}", job.key.animation, job.key.factor),
                        egui::ColorImage::from_rgba_unmultiplied(composed.canvas, image), egui::TextureOptions::NEAREST)
                }).collect();
                if self.sequences.len() >= MAX_KEPT_SEQUENCES {
                    self.sequences.remove(0);
                }
                self.sequences.push(Sequence { key: job.key, textures, composed });
            }
            Ok(None) => {}
            Err(error) => {
                log::error!("In-betweens of animation {} at x{}: {error:#}", job.key.animation, job.key.factor);
                self.failure = Some((job.key, format!("{error:#}")));
            }
        }
    }

    fn start(&mut self, ctx: &egui::Context, key: SequenceKey, snap_alpha: bool, reader: ClipReader) {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(Progress { activity: "reading the clip".to_string(), fraction: 0.0 }));
        let maker = self.maker.clone();
        let thread = {
            let (cancel, progress, repaint) = (cancel.clone(), progress.clone(), ctx.clone());
            std::thread::spawn(move || {
                let report = |activity: String, fraction: f32| {
                    *progress.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Progress { activity, fraction };
                    repaint.request_repaint();
                };
                let result = make_sequence(key.factor, snap_alpha, reader, &maker, &cancel, &report);
                repaint.request_repaint();
                result
            })
        };
        self.job = Some(Job { key, cancel, progress, thread: Some(thread) });
    }
}

/// Reads the clip and makes its sequence at `factor`, composed on the clip's canvas; `None` when cancelled.
fn make_sequence(factor: u32, snap_alpha: bool, reader: ClipReader, maker: &Mutex<Option<InBetweenMaker>>,
    cancel: &AtomicBool, report: &dyn Fn(String, f32)) -> eyre::Result<Option<Composed>> {
    let started = Instant::now();
    let clip = reader()?.ok_or_else(|| eyre::eyre!("The clip has no frames to show."))?;
    let frames: Vec<StrideFrame> = clip.frames.iter().map(|frame| StrideFrame {
        width: frame.width, height: frame.height, anchor_x: frame.anchor_x, anchor_y: frame.anchor_y,
        rgba: &frame.rgba,
    }).collect();
    let mut adapter = None;
    let placed: Vec<PlacedFrame> = if factor == 1 {
        frames.iter().map(|frame| PlacedFrame {
            width: usize::from(frame.width), height: usize::from(frame.height), anchor_x: frame.anchor_x,
            anchor_y: frame.anchor_y, rgba: frame.rgba.to_vec(),
        }).collect()
    } else {
        let mut maker = maker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if maker.is_none() {
            report(format!("compiling RIFE (0 of {RIFE_COMPILES})"), 0.0);
            *maker = Some(InBetweenMaker::new(|compile| if compile.done {
                report(format!("compiling RIFE ({} of {})", compile.index + 1, compile.total),
                    (compile.index + 1) as f32 / compile.total as f32);
            })?);
        }
        let maker = maker.as_mut().expect("made above");
        adapter = Some(maker.adapter().to_string());
        report("in-betweens".to_string(), 0.0);
        let Some(sequence) = maker.sequence(&frames, factor, snap_alpha, cancel, |done, total| {
            report(format!("in-betweens: pair {done} of {total}"), done as f32 / total as f32);
        })? else {
            return Ok(None);
        };
        sequence.into_iter().map(|frame| PlacedFrame {
            width: frame.width as usize, height: frame.height as usize, anchor_x: frame.anchor_x,
            anchor_y: frame.anchor_y, rgba: frame.rgba,
        }).collect()
    };
    let (canvas, images) = compose(&placed);
    Ok(Some(Composed {
        canvas, images, frame_secs: if clip.fps > 0.0 { clip.fps.recip() } else { CLASSIC_FRAME_SECS },
        mirrored: clip.mirrored, clip_frames: clip.frames.len(), compute_secs: started.elapsed().as_secs_f32(), adapter,
    }))
}

/// A frame of a sequence with its anchor from its top-left corner.
struct PlacedFrame {
    width: usize,
    height: usize,
    anchor_x: i32,
    anchor_y: i32,
    rgba: Vec<u8>,
}

/// Each frame on one canvas holding them all with their anchors on one point: the canvas size and the images.
fn compose(frames: &[PlacedFrame]) -> ([usize; 2], Vec<Vec<u8>>) {
    let left = frames.iter().map(|frame| frame.anchor_x).max().unwrap_or(0);
    let up = frames.iter().map(|frame| frame.anchor_y).max().unwrap_or(0);
    let right = frames.iter().map(|frame| frame.width as i32 - frame.anchor_x).max().unwrap_or(0);
    let down = frames.iter().map(|frame| frame.height as i32 - frame.anchor_y).max().unwrap_or(0);
    let canvas = [(left + right).max(1) as usize, (up + down).max(1) as usize];
    let images = frames.iter().map(|frame| {
        let mut image = vec![0u8; canvas[0] * canvas[1] * RGBA_BYTES];
        let (x0, y0) = ((left - frame.anchor_x) as usize, (up - frame.anchor_y) as usize);
        let row_bytes = frame.width * RGBA_BYTES;
        for (y, row) in frame.rgba.chunks_exact(row_bytes).enumerate() {
            let at = ((y0 + y) * canvas[0] + x0) * RGBA_BYTES;
            image[at..at + row_bytes].copy_from_slice(row);
        }
        image
    }).collect();
    (canvas, images)
}

/// The sequence playing in the clip's cycle, each frame `frame_secs / factor`; Shift or Ctrl and the wheel zoom.
fn show_sequence(ctx: &egui::Context, ui: &mut egui::Ui, sequence: &Sequence, zoom: &mut f32) {
    let composed = &sequence.composed;
    let count = sequence.textures.len();
    if count == 0 {
        return;
    }
    let shown_secs = composed.frame_secs / sequence.key.factor as f32;
    let info = ui.label(format!(
        "{count} frames ({} of the clip) · {:.1} ms each · cycle {:.3} s · made in {:.2} s",
        composed.clip_frames, shown_secs * 1e3, shown_secs * count as f32, composed.compute_secs
    ));
    if let Some(adapter) = &composed.adapter {
        info.on_hover_text(format!("In-betweens made on {adapter}"));
    }
    let now = ctx.input(|input| input.time);
    let step = now / f64::from(shown_secs);
    let index = step as usize % count;
    ctx.request_repaint_after_secs(((step.floor() + 1.0 - step) * f64::from(shown_secs)) as f32);
    let size = egui::vec2(composed.canvas[0] as f32, composed.canvas[1] as f32) * *zoom;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if response.hovered() {
        *zoom = (*zoom * ui.input(|input| input.zoom_delta())).clamp(MIN_ZOOM, MAX_ZOOM);
    }
    ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    let uv = if composed.mirrored { MIRRORED_FRAME_UV } else { FRAME_UV };
    ui.painter().image(sequence.textures[index].id(), rect, uv, egui::Color32::WHITE);
    response.on_hover_text(format!("Frame {} of {count}; Shift or Ctrl and the wheel zoom", index + 1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composed_frames_share_a_canvas_with_their_anchors_on_one_point() {
        const OPAQUE: [u8; RGBA_BYTES] = [1, 2, 3, u8::MAX];
        let frames = [
            PlacedFrame { width: 2, height: 3, anchor_x: 0, anchor_y: 3, rgba: OPAQUE.repeat(6) },
            PlacedFrame { width: 3, height: 1, anchor_x: 2, anchor_y: 1, rgba: OPAQUE.repeat(3) },
        ];
        let (canvas, images) = compose(&frames);
        // Left of the anchor 2, right of it 2; above it 3, below nothing.
        assert_eq!(canvas, [4, 3]);
        let alpha = |image: &Vec<u8>| -> Vec<u8> { image.chunks_exact(RGBA_BYTES).map(|texel| texel[3]).collect() };
        let (o, t) = (u8::MAX, 0);
        assert_eq!(alpha(&images[0]), [t, t, o, o, t, t, o, o, t, t, o, o]);
        assert_eq!(alpha(&images[1]), [t, t, t, t, t, t, t, t, o, o, o, t]);
    }
}
