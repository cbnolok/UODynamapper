//! The visual comparison videos of `lossy --ladder-dir`, per body of the `--clip` clips:
//! - `<prefix>-<body>-ladder.mp4`: every version (the source frames, the plain encode, each variant) side by side, in
//!   three segments: 4x and 2x nearest with |version − source| amplified under each panel, then 1x. Each segment
//!   plays every clip of the body in turn.
//! - `<prefix>-<body>-blink.mp4`: held frames, each panel flipping between the plain encode and one variant (the
//!   first panel between the source and the plain encode), at 2x and at 4x or the largest scale that shows the whole
//!   body.
//!
//! The panels of a body share one window relative to the anchor per scale, centred on the union of its frames and as
//! large as the panel holds, so a large body is cropped at 2x and 4x. Labels are drawn by ffmpeg's `drawtext` from
//! text files (no escaping); the videos are H.264 4:4:4, so 1x pixels keep their colour, at CRF 8 or the first
//! coarser quality that keeps the file under 29 MiB.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use color_eyre::eyre::{self, WrapErr};

/// The canvas, the title bar above the panels, and each panel's label strip and margin, in output pixels.
const CANVAS_WIDTH: usize = 1920;
const CANVAS_HEIGHT: usize = 1080;
const TITLE_HEIGHT: usize = 32;
const LABEL_HEIGHT: usize = 62;
const MARGIN: usize = 4;
/// The ladder's segments, the most magnified first.
const LADDER_SCALES: [usize; 3] = [4, 2, 1];
/// The blink's segments: 2x, then the larger of 4x and the largest scale that shows the whole body.
const BLINK_SCALE: usize = 2;
const BLINK_DETAIL_SCALE: usize = 4;
/// The ladder's panels at this scale and above have |version − source| drawn under them, amplified this much.
const DIFF_MIN_SCALE: usize = 2;
const DIFF_GAIN: u32 = 8;
const FPS: u32 = 8;
/// Each clip plays at least this long in each ladder segment (whole loops).
const CLIP_SECONDS: usize = 2;
/// Blink: frames held per clip, the frames between flips (0.5 s) and the flips each held frame shows.
const BLINK_FRAMES_PER_CLIP: usize = 2;
const BLINK_FLIP_FRAMES: usize = 4;
const BLINK_HOLD_FLIPS: usize = 6;
const BACKGROUND: [u8; 3] = [96, 96, 96];
const DIFF_BACKGROUND: [u8; 3] = [0, 0, 0];
const SEPARATOR: [u8; 3] = [24, 24, 24];
/// The square beside a blink panel's label: which of its two versions shows.
const MARK_SIDE: usize = 12;
const MARK_FIRST: [u8; 3] = [70, 130, 255];
const MARK_SECOND: [u8; 3] = [255, 140, 40];
const TITLE_FONT_SIZE: usize = 20;
const NAME_FONT_SIZE: usize = 17;
const NOTE_FONT_SIZE: usize = 14;
const TEXT_COLOUR: &str = "white";
const NOTE_COLOUR: &str = "0xffd27a";
/// The encode qualities tried in turn until a video fits `MAX_VIDEO_BYTES` (the last one is kept regardless).
const VIDEO_CRFS: [u32; 5] = [8, 12, 16, 20, 24];
const MAX_VIDEO_BYTES: u64 = 29 << 20;
const RGB: usize = 3;
const RGBA_BYTES: usize = 4;
const ALPHA: usize = 3;
const OPAQUE: u32 = 255;

/// A clip frame and its versions' pixels (source, plain, then each variant), RGBA.
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub anchor: (i32, i32),
    pub versions: &'a [Vec<u8>],
}

pub struct Options<'a> {
    pub dir: &'a Path,
    pub prefix: &'a str,
    /// The encodes' names: the plain encode, then each variant.
    pub names: &'a [String],
    /// A line under a version's name (`source`, `plain` or a variant).
    pub notes: &'a HashMap<String, String>,
    pub body_names: &'a HashMap<String, String>,
    pub font: &'a Path,
}

#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

/// A window of the frames relative to the anchor, in source pixels.
#[derive(Clone, Copy)]
struct Window {
    x: i32,
    y: i32,
    width: usize,
    height: usize,
}

/// A text drawn on the frames `first..=last`, on the frames of one blink phase only when `phase` says so.
struct Label {
    x: usize,
    y: usize,
    size: usize,
    colour: &'static str,
    text: String,
    frames: (usize, usize),
    phase: Option<usize>,
}

struct Video {
    canvases: Vec<Vec<u8>>,
    labels: Vec<Label>,
}

/// The union of the frames' boxes relative to the anchor: left, top, right, bottom.
fn union(frames: &[&Frame]) -> (i32, i32, i32, i32) {
    let mut bounds = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for frame in frames {
        bounds.0 = bounds.0.min(-frame.anchor.0);
        bounds.1 = bounds.1.min(-frame.anchor.1);
        bounds.2 = bounds.2.max(frame.width as i32 - frame.anchor.0);
        bounds.3 = bounds.3.max(frame.height as i32 - frame.anchor.1);
    }
    bounds
}

/// The largest window `area` holds at `scale`, centred on `bounds` and inside them.
fn window(bounds: (i32, i32, i32, i32), area: (usize, usize), scale: usize) -> Window {
    let fit = |low: i32, high: i32, room: usize| {
        let extent = ((high - low) as usize).min(room / scale);
        let start = (low + high) / 2 - extent as i32 / 2;
        (start.clamp(low, high - extent as i32), extent)
    };
    let (x, width) = fit(bounds.0, bounds.2, area.0);
    let (y, height) = fit(bounds.1, bounds.3, area.1);
    Window { x, y, width, height }
}

/// `cells` panels in a grid whose panels are about as wide as the canvas's aspect, filled row by row.
fn grid(cells: usize) -> Vec<Rect> {
    let aspect = CANVAS_WIDTH as f64 / (CANVAS_HEIGHT - TITLE_HEIGHT) as f64;
    let columns = ((cells as f64 * aspect).sqrt().ceil() as usize).clamp(1, cells);
    let rows = cells.div_ceil(columns);
    let (width, height) = (CANVAS_WIDTH / columns, (CANVAS_HEIGHT - TITLE_HEIGHT) / rows);
    (0..cells).map(|cell| Rect { x: cell % columns * width, y: TITLE_HEIGHT + cell / columns * height, width, height })
        .collect()
}

/// A panel's picture area under its label strip.
fn content(cell: Rect) -> Rect {
    Rect { x: cell.x + MARGIN, y: cell.y + LABEL_HEIGHT, width: cell.width - 2 * MARGIN,
        height: cell.height - LABEL_HEIGHT - MARGIN }
}

fn fill(canvas: &mut [u8], rect: Rect, colour: [u8; 3]) {
    for y in rect.y..rect.y + rect.height {
        for pixel in canvas[(y * CANVAS_WIDTH + rect.x) * RGB..(y * CANVAS_WIDTH + rect.x + rect.width) * RGB]
            .chunks_exact_mut(RGB) {
            pixel.copy_from_slice(&colour);
        }
    }
}

/// A frame's pixel over the background, or `None` outside the frame.
fn over(frame: &Frame, version: usize, x: i32, y: i32) -> Option<[u8; 3]> {
    let (fx, fy) = (x + frame.anchor.0, y + frame.anchor.1);
    if fx < 0 || fy < 0 || fx >= frame.width as i32 || fy >= frame.height as i32 {
        return None;
    }
    let pixel = &frame.versions[version][(fy as usize * frame.width as usize + fx as usize) * RGBA_BYTES..][..RGBA_BYTES];
    let alpha = u32::from(pixel[ALPHA]);
    Some(std::array::from_fn(|channel| {
        ((u32::from(pixel[channel]) * alpha + u32::from(BACKGROUND[channel]) * (OPAQUE - alpha)) / OPAQUE) as u8
    }))
}

/// Draws `window` of the frame's `version` into `rect` at `scale`, centred; with `diff`, |version − source| times
/// [`DIFF_GAIN`] on black instead.
fn draw(canvas: &mut [u8], rect: Rect, frame: &Frame, version: usize, window: Window, scale: usize, diff: bool) {
    let background = if diff { DIFF_BACKGROUND } else { BACKGROUND };
    fill(canvas, rect, background);
    let (origin_x, origin_y) = (rect.x + (rect.width - window.width * scale) / 2,
        rect.y + (rect.height - window.height * scale) / 2);
    for wy in 0..window.height {
        for wx in 0..window.width {
            let (x, y) = (window.x + wx as i32, window.y + wy as i32);
            let Some(shown) = over(frame, version, x, y) else { continue };
            let colour = if diff {
                let source = over(frame, 0, x, y).expect("same frame");
                std::array::from_fn(|channel| {
                    (u32::from(shown[channel].abs_diff(source[channel])) * DIFF_GAIN).min(OPAQUE) as u8
                })
            } else {
                shown
            };
            for sy in 0..scale {
                let row = (origin_y + wy * scale + sy) * CANVAS_WIDTH + origin_x + wx * scale;
                for pixel in canvas[row * RGB..(row + scale) * RGB].chunks_exact_mut(RGB) {
                    pixel.copy_from_slice(&colour);
                }
            }
        }
    }
}

fn blank() -> Vec<u8> {
    SEPARATOR.repeat(CANVAS_WIDTH * CANVAS_HEIGHT)
}

/// The version names, the source first.
fn version_names(names: &[String]) -> Vec<String> {
    std::iter::once("source".to_string()).chain(names.iter().cloned()).collect()
}

fn ladder(title: &str, clips: &[(String, Vec<&Frame>)], options: &Options) -> Video {
    let names = version_names(options.names);
    let cells = grid(names.len());
    let all: Vec<&Frame> = clips.iter().flat_map(|(_, frames)| frames.iter().copied()).collect();
    let bounds = union(&all);
    let mut video = Video { canvases: Vec::new(), labels: Vec::new() };
    for (cell, name) in cells.iter().zip(&names) {
        video.labels.push(Label { x: cell.x + MARGIN, y: cell.y + MARGIN, size: NAME_FONT_SIZE, colour: TEXT_COLOUR,
            text: name.clone(), frames: (0, usize::MAX), phase: None });
        if let Some(note) = options.notes.get(name) {
            video.labels.push(Label { x: cell.x + MARGIN, y: cell.y + MARGIN + NAME_FONT_SIZE + MARGIN,
                size: NOTE_FONT_SIZE, colour: NOTE_COLOUR, text: note.clone(), frames: (0, usize::MAX), phase: None });
        }
    }
    for scale in LADDER_SCALES {
        let diff = scale >= DIFF_MIN_SCALE;
        for (clip, frames) in clips {
            let area = content(cells[0]);
            let height = if diff { (area.height - MARGIN) / 2 } else { area.height };
            let window = window(bounds, (area.width, height), scale);
            let loops = (CLIP_SECONDS * FPS as usize).div_ceil(frames.len()).max(1);
            let first = video.canvases.len();
            for _ in 0..loops {
                for frame in frames {
                    let mut canvas = blank();
                    for (version, cell) in cells.iter().enumerate() {
                        let area = content(*cell);
                        if diff {
                            let top = Rect { height: (area.height - MARGIN) / 2, ..area };
                            let bottom = Rect { y: top.y + top.height + MARGIN, ..top };
                            draw(&mut canvas, top, frame, version, window, scale, false);
                            draw(&mut canvas, bottom, frame, version, window, scale, true);
                        } else {
                            draw(&mut canvas, area, frame, version, window, scale, false);
                        }
                    }
                    video.canvases.push(canvas);
                }
            }
            let mut text = format!("{title} · {clip} · {scale}x nearest");
            if diff {
                text.push_str(&format!(" · under each: |version − source| × {DIFF_GAIN} per channel (black = exact)"));
            }
            if window.width < (bounds.2 - bounds.0) as usize || window.height < (bounds.3 - bounds.1) as usize {
                text.push_str(&format!(" · cropped to {}×{} px", window.width, window.height));
            }
            video.labels.push(Label { x: MARGIN, y: MARGIN, size: TITLE_FONT_SIZE, colour: TEXT_COLOUR, text,
                frames: (first, video.canvases.len() - 1), phase: None });
        }
    }
    video
}

fn blink(title: &str, clips: &[(String, Vec<&Frame>)], options: &Options) -> Video {
    let names = version_names(options.names);
    // Panel 0 flips the source and the plain encode, panel k the plain encode and variant k.
    let pairs: Vec<(usize, usize)> = std::iter::once((0, 1)).chain((2..names.len()).map(|variant| (1, variant))).collect();
    let cells = grid(pairs.len());
    let all: Vec<&Frame> = clips.iter().flat_map(|(_, frames)| frames.iter().copied()).collect();
    let bounds = union(&all);
    let mut video = Video { canvases: Vec::new(), labels: Vec::new() };
    for (cell, &(first, second)) in cells.iter().zip(&pairs) {
        let x = cell.x + MARGIN;
        video.labels.push(Label { x, y: cell.y + MARGIN, size: NAME_FONT_SIZE, colour: TEXT_COLOUR,
            text: format!("{} ↔ {}", names[first], names[second]), frames: (0, usize::MAX), phase: None });
        if let Some(note) = options.notes.get(&names[second]) {
            video.labels.push(Label { x, y: cell.y + MARGIN + NAME_FONT_SIZE + MARGIN, size: NOTE_FONT_SIZE,
                colour: NOTE_COLOUR, text: note.clone(), frames: (0, usize::MAX), phase: None });
        }
        for (phase, version) in [first, second].into_iter().enumerate() {
            video.labels.push(Label { x: x + MARK_SIDE + MARGIN, y: cell.y + LABEL_HEIGHT - NOTE_FONT_SIZE - 2 * MARGIN,
                size: NOTE_FONT_SIZE, colour: TEXT_COLOUR, text: format!("showing: {}", names[version]),
                frames: (0, usize::MAX), phase: Some(phase) });
        }
    }
    let hold = BLINK_HOLD_FLIPS * BLINK_FLIP_FRAMES;
    let area = content(cells[0]);
    let whole = (area.width / (bounds.2 - bounds.0) as usize).min(area.height / (bounds.3 - bounds.1) as usize);
    for scale in [BLINK_SCALE, BLINK_DETAIL_SCALE.max(whole)] {
        for (clip, frames) in clips {
            let window = window(bounds, (content(cells[0]).width, content(cells[0]).height), scale);
            let first_canvas = video.canvases.len();
            for pick in 0..BLINK_FRAMES_PER_CLIP {
                let frame = frames[frames.len() * (pick + 1) / (BLINK_FRAMES_PER_CLIP + 1)];
                for at in 0..hold {
                    let phase = at / BLINK_FLIP_FRAMES % 2;
                    let mut canvas = blank();
                    for (cell, &(first, second)) in cells.iter().zip(&pairs) {
                        let version = if phase == 0 { first } else { second };
                        draw(&mut canvas, content(*cell), frame, version, window, scale, false);
                        let mark = Rect { x: cell.x + MARGIN, y: cell.y + LABEL_HEIGHT - MARK_SIDE - 2 * MARGIN,
                            width: MARK_SIDE, height: MARK_SIDE };
                        fill(&mut canvas, mark, if phase == 0 { MARK_FIRST } else { MARK_SECOND });
                    }
                    video.canvases.push(canvas);
                }
            }
            let mut text = format!("{title} · {clip} · blink every 0.5 s · {scale}x nearest");
            if window.width < (bounds.2 - bounds.0) as usize || window.height < (bounds.3 - bounds.1) as usize {
                text.push_str(&format!(" · cropped to {}×{} px", window.width, window.height));
            }
            video.labels.push(Label { x: MARGIN, y: MARGIN, size: TITLE_FONT_SIZE, colour: TEXT_COLOUR, text,
                frames: (first_canvas, video.canvases.len() - 1), phase: None });
        }
    }
    video
}

/// Encodes `video` to `file`, its labels as `drawtext` filters reading text files in a folder beside it.
fn encode(video: &Video, file: &Path, font: &Path) -> eyre::Result<()> {
    let texts: PathBuf = file.with_extension("labels");
    std::fs::create_dir_all(&texts)?;
    let mut filters = Vec::new();
    for (at, label) in video.labels.iter().enumerate() {
        let text = texts.join(format!("{at}.txt"));
        std::fs::write(&text, &label.text)?;
        let last = label.frames.1.min(video.canvases.len() - 1);
        let mut enable = format!("between(n,{},{last})", label.frames.0);
        if let Some(phase) = label.phase {
            enable.push_str(&format!("*eq(mod(floor(n/{BLINK_FLIP_FRAMES}),2),{phase})"));
        }
        filters.push(format!("drawtext=fontfile={}:textfile={}:expansion=none:x={}:y={}:fontsize={}:fontcolor={}:\
            enable='{enable}'", font.display(), text.display(), label.x, label.y, label.size, label.colour));
    }
    let script = texts.join("filters.txt");
    std::fs::write(&script, filters.join(",\n"))?;
    let mut crf = VIDEO_CRFS[0];
    for (at, &quality) in VIDEO_CRFS.iter().enumerate() {
        crf = quality;
        let mut child = Command::new("ffmpeg").args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
            "-s", &format!("{CANVAS_WIDTH}x{CANVAS_HEIGHT}"), "-r", &FPS.to_string(), "-i", "-", "-filter_script:v"])
            .arg(&script).args(["-c:v", "libx264", "-preset", "slow", "-crf", &crf.to_string(), "-pix_fmt", "yuv444p"])
            .arg(file).stdin(Stdio::piped()).spawn().wrap_err("start ffmpeg")?;
        let mut stdin = child.stdin.take().expect("piped");
        for canvas in &video.canvases {
            stdin.write_all(canvas)?;
        }
        drop(stdin);
        eyre::ensure!(child.wait()?.success(), "ffmpeg failed for {}", file.display());
        if std::fs::metadata(file)?.len() <= MAX_VIDEO_BYTES || at + 1 == VIDEO_CRFS.len() {
            break;
        }
    }
    std::fs::remove_dir_all(&texts)?;
    println!("{}: {} frames, CRF {crf}, {:.1} MiB", file.display(), video.canvases.len(),
        std::fs::metadata(file)?.len() as f64 / f64::from(1u32 << 20));
    Ok(())
}

/// The ladder and blink videos of each body of `clips` (`body:action:direction` names, in order).
pub fn render(clips: &[(String, Vec<Frame>)], options: &Options) -> eyre::Result<()> {
    std::fs::create_dir_all(options.dir)?;
    let mut bodies: Vec<(String, Vec<(String, Vec<&Frame>)>)> = Vec::new();
    for (name, frames) in clips {
        if frames.is_empty() {
            eprintln!("clip {name}: no frames");
            continue;
        }
        let (body, clip) = name.split_once(':').unwrap_or((name, ""));
        let entry = match bodies.iter_mut().position(|(kept, _)| kept == body) {
            Some(at) => &mut bodies[at].1,
            None => {
                bodies.push((body.to_string(), Vec::new()));
                &mut bodies.last_mut().expect("pushed").1
            }
        };
        entry.push((clip.replace(':', " direction "), frames.iter().collect()));
    }
    for (body, clips) in &bodies {
        let name = options.body_names.get(body).cloned().unwrap_or_default();
        let stem = if name.is_empty() { format!("{}-{body}", options.prefix) } else { format!("{}-{body}-{name}", options.prefix) };
        let title = format!("{} {name} (body {body})", options.prefix);
        encode(&ladder(&title, clips, options), &options.dir.join(format!("{stem}-ladder.mp4")), options.font)?;
        encode(&blink(&title, clips, options), &options.dir.join(format!("{stem}-blink.mp4")), options.font)?;
    }
    Ok(())
}
