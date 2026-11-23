//! The walk pace of a recording, to check a stride change in game: the ground's scroll and the walker's cycle rate.
//! video-pace <video> [--half N] [--halves M] [--shifts <tsv>]: the recording is decoded through an `ffmpeg` pipe of
//!   raw RGB frames and split into `halves` side-by-side views (a before-and-after composition has two), the walker
//!   at the centre of each. Per pair of frames, the ground's scroll is the sub-pixel block match of the background
//!   around the walker; the moving stretches are the runs of scrolling frames, and for each the tool prints the
//!   ground speed and the walker's cycle rate, the period of the autocorrelation of its saturated silhouette. A port
//!   of the gait study's `video_pace.py`, with its numbers; `--shifts` writes the per-pair scroll as TSV.
use clap::Args;
use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};

/// How far the ground match looks, in px either way.
const SEARCH: usize = 9;
const SEARCH_SPAN: usize = 2 * SEARCH + 1;
/// The walker's box, left out of the ground match and holding its silhouette: px from the view's centre.
const WALKER_LEFT: isize = -70;
const WALKER_TOP: isize = -120;
const WALKER_RIGHT: isize = 80;
const WALKER_BOTTOM: isize = 90;
/// The rows at the view's top holding the label, left out of the ground match.
const LABEL_ROWS: usize = 60;
/// One background pixel in this many is matched.
const SAMPLE_EVERY: usize = 7;
/// The scroll, in px a frame, above which the ground moves.
const MOVING_PX: f64 = 0.8;
/// The frames a moving stretch must exceed.
const MIN_SEGMENT: usize = 30;
/// Recordings repeat frames (the app draws under the recording's rate): still runs shorter than this inside a move
/// are repeats.
const MAX_REPEATS: usize = 8;
/// The cycle periods looked for, in frames.
const MIN_PERIOD: usize = 6;
const MAX_PERIOD: usize = 150;
/// The share of the highest autocorrelation the first peak taken must reach.
const PEAK_SHARE: f64 = 0.5;
/// The frames left out at a stretch's start and end, where the walk starts and stops.
const TRIM_START: usize = 15;
const TRIM_END: usize = 10;
/// The silhouette's pixels: saturated, hue between yellow and green (skin, cloth), not the red bricks or grey stone.
const SATURATION: f32 = 40.0;
const HUE_MIN: f32 = 30.0;
const HUE_MAX: f32 = 150.0;
const FULL_TURN_DEGREES: f32 = 360.0;
const RGB: usize = 3;
/// The frames decoded before their views and scroll are worked out in parallel.
const CHUNK_FRAMES: usize = 64;

#[derive(Args, Debug)]
pub struct Arguments {
    /// The recording.
    video: PathBuf,
    /// The view measured, from the left.
    #[arg(long, default_value_t = 0)]
    half: usize,
    /// The views side by side in the recording.
    #[arg(long, default_value_t = 2)]
    halves: usize,
    /// Write the scroll of every pair of frames here, as TSV.
    #[arg(long)]
    shifts: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Rect {
    fn contains(self, x: usize, y: usize) -> bool {
        (self.left..self.right).contains(&x) && (self.top..self.bottom).contains(&y)
    }

    fn width(self) -> usize {
        self.right - self.left
    }
}

/// One view's size, the label rows and the walker's box.
#[derive(Clone, Debug)]
struct Layout {
    width: usize,
    height: usize,
    label_rows: usize,
    walker: Rect,
}

impl Layout {
    fn new(width: usize, height: usize, label_rows: usize, walker: Rect) -> eyre::Result<Self> {
        let margin = SEARCH + 1;
        if label_rows < SEARCH || width <= 2 * margin || height <= label_rows + margin {
            eyre::bail!("a {width}x{height} view with {label_rows} label rows leaves no ground to match");
        }
        if walker.left >= walker.right || walker.top >= walker.bottom || walker.right > width || walker.bottom > height
        {
            eyre::bail!("the walker's box {walker:?} is not inside the {width}x{height} view");
        }
        Ok(Self { width, height, label_rows, walker })
    }

    /// A recording's view, the walker's box around its centre.
    fn recording(width: usize, height: usize) -> eyre::Result<Self> {
        let around = |centre: usize, offset: isize| {
            usize::try_from(centre as isize + offset)
                .map_err(|_| eyre::eyre!("a {width}x{height} view is too small for the walker's box"))
        };
        let (centre_x, centre_y) = (width / 2, height / 2);
        let walker = Rect {
            left: around(centre_x, WALKER_LEFT)?,
            top: around(centre_y, WALKER_TOP)?,
            right: around(centre_x, WALKER_RIGHT)?,
            bottom: around(centre_y, WALKER_BOTTOM)?,
        };
        Self::new(width, height, LABEL_ROWS, walker)
    }

    /// The background pixels matched, in row order: below the label, outside the walker's box, and far enough from
    /// the other edges for every offset searched.
    fn samples(&self) -> Vec<usize> {
        let margin = SEARCH + 1;
        (self.label_rows..self.height - margin)
            .flat_map(|y| (margin..self.width - margin).map(move |x| (x, y)))
            .filter(|&(x, y)| !self.walker.contains(x, y))
            .step_by(SAMPLE_EVERY)
            .map(|(x, y)| y * self.width + x)
            .collect()
    }
}

/// What one frame keeps: its grey over the view and the walker's silhouette over its box.
struct View {
    grey: Vec<f32>,
    silhouette: Vec<bool>,
}

fn view(rgb: &[u8], layout: &Layout) -> View {
    let grey = rgb
        .chunks_exact(RGB)
        .map(|pixel| pixel.iter().map(|&channel| f32::from(channel)).sum::<f32>() / RGB as f32)
        .collect();
    View { grey, silhouette: silhouette(rgb, layout) }
}

fn silhouette(rgb: &[u8], layout: &Layout) -> Vec<bool> {
    let walker = layout.walker;
    let mut mask = Vec::with_capacity(walker.width() * (walker.bottom - walker.top));
    for y in walker.top..walker.bottom {
        let row = &rgb[(y * layout.width + walker.left) * RGB..][..walker.width() * RGB];
        mask.extend(row.chunks_exact(RGB).map(|pixel| {
            let [red, green, blue] = [pixel[0], pixel[1], pixel[2]].map(f32::from);
            let spread = red.max(green).max(blue) - red.min(green).min(blue);
            let hue = (3f32.sqrt() * (green - blue))
                .atan2(2.0 * red - green - blue)
                .to_degrees()
                .rem_euclid(FULL_TURN_DEGREES);
            spread > SATURATION && hue > HUE_MIN && hue < HUE_MAX
        }));
    }
    mask
}

/// The ground's scroll between two frames: `later(x + dx, y + dy)` matches `earlier(x, y)` on the background, with
/// the mean absolute grey difference left at the best offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shift {
    dx: f64,
    dy: f64,
    error: f64,
}

impl Shift {
    fn speed(self) -> f64 {
        self.dx.hypot(self.dy)
    }
}

/// The vertex of the parabola through `errors[index]` and its neighbours, where it opens upwards.
fn sub_pixel(errors: &[f64], index: usize) -> f64 {
    if index == 0 || index + 1 >= errors.len() {
        return index as f64;
    }
    let curvature = errors[index - 1] - 2.0 * errors[index] + errors[index + 1];
    let offset = if curvature > 0.0 { 0.5 * (errors[index - 1] - errors[index + 1]) / curvature } else { 0.0 };
    index as f64 + offset
}

fn ground_shift(earlier: &[f32], later: &[f32], samples: &[usize], width: usize) -> Shift {
    let errors: Vec<f64> = (0..SEARCH_SPAN * SEARCH_SPAN)
        .into_par_iter()
        .map(|cell| {
            let dy = (cell / SEARCH_SPAN) as isize - SEARCH as isize;
            let dx = (cell % SEARCH_SPAN) as isize - SEARCH as isize;
            let offset = dy * width as isize + dx;
            let total: f64 = samples
                .iter()
                .map(|&index| f64::from((later[index.wrapping_add_signed(offset)] - earlier[index]).abs()))
                .sum();
            total / samples.len() as f64
        })
        .collect();
    // The first of equal minima, as the script's argmin takes.
    let best = (1..errors.len()).fold(0, |best, cell| if errors[cell] < errors[best] { cell } else { best });
    let (row, column) = (best / SEARCH_SPAN, best % SEARCH_SPAN);
    let along_row = &errors[row * SEARCH_SPAN..][..SEARCH_SPAN];
    let along_column: Vec<f64> = (0..SEARCH_SPAN).map(|y| errors[y * SEARCH_SPAN + column]).collect();
    Shift {
        dx: sub_pixel(along_row, column) - SEARCH as f64,
        dy: sub_pixel(&along_column, row) - SEARCH as f64,
        error: errors[best],
    }
}

/// The dominant period in frames of a run of silhouettes, and its autocorrelation, by the autocorrelation of the
/// silhouettes less their mean; `None` when the run is too short for the shortest period or does not change.
fn period(silhouettes: &[&[bool]]) -> Option<(f64, f64)> {
    let frames = silhouettes.len();
    let lags = frames / 2;
    if lags <= MIN_PERIOD {
        return None;
    }
    let pixels = silhouettes[0].len();
    let mut mean = vec![0.0f64; pixels];
    for silhouette in silhouettes {
        for (sum, &inside) in mean.iter_mut().zip(silhouette.iter()) {
            *sum += f64::from(u8::from(inside));
        }
    }
    mean.iter_mut().for_each(|sum| *sum /= frames as f64);
    let centred: Vec<Vec<f64>> = silhouettes
        .par_iter()
        .map(|silhouette| {
            silhouette.iter().zip(&mean).map(|(&inside, mean)| f64::from(u8::from(inside)) - mean).collect()
        })
        .collect();
    let mut correlation: Vec<f64> = (0..lags)
        .into_par_iter()
        .map(|lag| {
            let total: f64 = (0..frames - lag)
                .map(|frame| centred[frame].iter().zip(&centred[frame + lag]).map(|(a, b)| a * b).sum::<f64>())
                .sum();
            total / (frames - lag) as f64
        })
        .collect();
    let zero_lag = correlation[0];
    if zero_lag <= 0.0 {
        return None;
    }
    correlation.iter_mut().for_each(|value| *value /= zero_lag);
    let window = &correlation[MIN_PERIOD..MAX_PERIOD.min(lags)];
    let top = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // The first local maximum past MIN_PERIOD reaching PEAK_SHARE of the highest: beats between the app's frames and
    // the recording's make multiples of the period peak higher than the period itself.
    let peak = (MIN_PERIOD..MAX_PERIOD.min(lags - 1))
        .find(|&lag| {
            let value = correlation[lag];
            value >= correlation[lag - 1]
                && value >= correlation[lag + 1]
                && value > 0.0
                && value >= PEAK_SHARE * top
        })
        .unwrap_or_else(|| {
            MIN_PERIOD + (1..window.len()).fold(0, |best, lag| if window[lag] > window[best] { lag } else { best })
        });
    if peak > 0 && peak + 1 < lags {
        let curvature = correlation[peak - 1] - 2.0 * correlation[peak] + correlation[peak + 1];
        if curvature < 0.0 {
            let offset = 0.5 * (correlation[peak - 1] - correlation[peak + 1]) / curvature;
            return Some((peak as f64 + offset, correlation[peak]));
        }
    }
    Some((peak as f64, correlation[peak]))
}

/// The moving stretches, `start..end` in pairs of frames: runs of scroll above MOVING_PX longer than MIN_SEGMENT,
/// joined over still runs shorter than MAX_REPEATS.
fn segments(speeds: &[f64]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for (index, _) in speeds.iter().enumerate().filter(|&(_, &speed)| speed > MOVING_PX) {
        run = match run {
            Some((start, last)) if index - last <= MAX_REPEATS => Some((start, index)),
            Some((start, last)) => {
                if last - start > MIN_SEGMENT {
                    found.push((start, last + 1));
                }
                Some((index, index))
            }
            None => Some((index, index)),
        };
    }
    if let Some((start, last)) = run
        && last - start > MIN_SEGMENT
    {
        found.push((start, last + 1));
    }
    found
}

/// The scroll of every pair of frames and every frame's silhouette, worked out CHUNK_FRAMES at a time.
fn analyse(
    frames: impl Iterator<Item = eyre::Result<Vec<u8>>>,
    layout: &Layout,
) -> eyre::Result<(Vec<Shift>, Vec<Vec<bool>>)> {
    let samples = layout.samples();
    let (mut shifts, mut silhouettes) = (Vec::new(), Vec::new());
    let mut previous: Option<Vec<f32>> = None;
    let mut frames = frames.peekable();
    while frames.peek().is_some() {
        let chunk = frames.by_ref().take(CHUNK_FRAMES).collect::<eyre::Result<Vec<_>>>()?;
        let views: Vec<View> = chunk.par_iter().map(|rgb| view(rgb, layout)).collect();
        let greys: Vec<&[f32]> =
            previous.iter().map(Vec::as_slice).chain(views.iter().map(|view| &view.grey[..])).collect();
        shifts.par_extend(greys.par_windows(2).map(|pair| ground_shift(pair[0], pair[1], &samples, layout.width)));
        let mut views = views;
        previous = views.last_mut().map(|view| std::mem::take(&mut view.grey));
        silhouettes.extend(views.into_iter().map(|view| view.silhouette));
    }
    Ok((shifts, silhouettes))
}

/// A moving stretch: its pairs of frames, the mean scroll over its trimmed core and the walker's period there.
#[derive(Debug)]
struct Stretch {
    start: usize,
    end: usize,
    dx: f64,
    dy: f64,
    period: Option<(f64, f64)>,
}

fn stretches(shifts: &[Shift], silhouettes: &[Vec<bool>]) -> Vec<Stretch> {
    let speeds: Vec<f64> = shifts.iter().map(|shift| shift.speed()).collect();
    segments(&speeds)
        .into_iter()
        .map(|(start, end)| {
            let core = start + TRIM_START..end - TRIM_END;
            let mean = |pick: fn(&Shift) -> f64| shifts[core.clone()].iter().map(pick).sum::<f64>() / core.len() as f64;
            let run: Vec<&[bool]> = silhouettes[core.clone()].iter().map(Vec::as_slice).collect();
            Stretch { start, end, dx: mean(|shift| shift.dx), dy: mean(|shift| shift.dy), period: period(&run) }
        })
        .collect()
}

/// The recording's width, height and frame rate, from `ffprobe`.
fn probe(video: &Path) -> eyre::Result<(usize, usize, f64)> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate"])
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(video)
        .output()
        .wrap_err("running ffprobe")?;
    if !output.status.success() {
        eyre::bail!("ffprobe failed on {}: {}", video.display(), String::from_utf8_lossy(&output.stderr).trim());
    }
    let text = String::from_utf8(output.stdout)?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .ok_or_else(|| eyre::eyre!("ffprobe gave no {name} for {}", video.display()))
    };
    let rate = field("r_frame_rate")?;
    let fps = match rate.split_once('/') {
        Some((numerator, denominator)) => numerator.parse::<f64>()? / denominator.parse::<f64>()?,
        None => rate.parse()?,
    };
    Ok((field("width")?.parse()?, field("height")?.parse()?, fps))
}

/// Fills `buffer` with the next frame; false at the end of the stream.
fn read_frame(stdout: &mut ChildStdout, buffer: &mut [u8]) -> eyre::Result<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        match stdout.read(&mut buffer[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => eyre::bail!("the stream ended inside a frame"),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

pub fn run(args: Arguments) -> eyre::Result<()> {
    if args.halves == 0 || args.half >= args.halves {
        eyre::bail!("view {} of {} does not exist", args.half, args.halves);
    }
    let (width, height, fps) = probe(&args.video)?;
    let view_width = width / args.halves;
    let layout = Layout::recording(view_width, height)?;
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&args.video)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .wrap_err("running ffmpeg")?;
    let mut stdout = child.stdout.take().ok_or_else(|| eyre::eyre!("ffmpeg gave no output"))?;
    let mut frame = vec![0u8; width * height * RGB];
    let row_bytes = view_width * RGB;
    let frames = std::iter::from_fn(|| match read_frame(&mut stdout, &mut frame) {
        Ok(false) => None,
        Ok(true) => Some(Ok(frame
            .chunks_exact(width * RGB)
            .flat_map(|row| &row[args.half * row_bytes..][..row_bytes])
            .copied()
            .collect())),
        Err(error) => Some(Err(error)),
    });
    let (shifts, silhouettes) = analyse(frames, &layout)?;
    let status = child.wait()?;
    if !status.success() {
        eyre::bail!("ffmpeg failed on {}: {status}", args.video.display());
    }
    if let Some(path) = &args.shifts {
        let mut out = BufWriter::new(File::create(path).wrap_err_with(|| format!("creating {}", path.display()))?);
        writeln!(out, "pair\tdx\tdy\terror")?;
        for (pair, shift) in shifts.iter().enumerate() {
            writeln!(out, "{pair}\t{:.4}\t{:.4}\t{:.4}", shift.dx, shift.dy, shift.error)?;
        }
        out.flush()?;
    }
    let frame_count = silhouettes.len();
    println!("{frame_count} frames at {fps} fps, view {} of {} ({view_width}x{height})", args.half, args.halves);
    for stretch in stretches(&shifts, &silhouettes) {
        let speed = stretch.dx.hypot(stretch.dy);
        let pace = match stretch.period {
            Some((period, correlation)) => format!(
                "silhouette period {period:.1} frames = {:.2} cycles/s (autocorrelation {correlation:.2})",
                fps / period
            ),
            None => "silhouette period: the stretch is too short or does not change".to_owned(),
        };
        println!(
            "segment frames {}-{} ({:.2} s): ground {speed:.2} px/frame = {:.1} px/s (dx {:+.2} dy {:+.2}), {pace}",
            stretch.start,
            stretch.end,
            (stretch.end - stretch.start) as f64 / fps,
            speed * fps,
            stretch.dx,
            stretch.dy,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 160;
    const HEIGHT: usize = 120;
    const TEST_LABEL_ROWS: usize = 12;
    const WALKER: Rect = Rect { left: 60, top: 40, right: 100, bottom: 90 };
    const STILL_FRAMES: usize = 20;
    const MOVING_FRAMES: usize = 80;
    const STEP: (i64, i64) = (3, -2);
    const CYCLE: usize = 10;
    const WALKER_GREEN: [u8; RGB] = [60, 200, 60];
    const NOISE_LOW: u32 = 40;
    const NOISE_RANGE: u32 = 176;
    const MIN_BAR: usize = 4;
    const BAR_GROWTH: usize = 3;
    const BAR_TOP: usize = 50;
    const BAR_BOTTOM: usize = 80;
    const SHIFT_TOLERANCE: f64 = 0.15;
    const PERIOD_TOLERANCE: f64 = 0.3;

    /// Grey ground noise at a world position.
    fn noise(x: i64, y: i64) -> u8 {
        let mut hash = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(0xC2B2_AE3D);
        hash ^= hash >> 13;
        (NOISE_LOW + hash % NOISE_RANGE) as u8
    }

    /// A clip standing, walking STEP a frame and standing again, the walker a green bar whose width cycles in CYCLE
    /// frames.
    fn synthetic_clip() -> Vec<Vec<u8>> {
        let total = 2 * STILL_FRAMES + MOVING_FRAMES;
        let mut offset = (0i64, 0i64);
        (0..total)
            .map(|frame| {
                if frame > STILL_FRAMES && frame <= STILL_FRAMES + MOVING_FRAMES {
                    offset = (offset.0 + STEP.0, offset.1 + STEP.1);
                }
                let bar = MIN_BAR + BAR_GROWTH * (frame % CYCLE);
                let mut rgb = Vec::with_capacity(WIDTH * HEIGHT * RGB);
                for y in 0..HEIGHT {
                    for x in 0..WIDTH {
                        if (BAR_TOP..BAR_BOTTOM).contains(&y) && (WALKER.left..WALKER.left + bar).contains(&x) {
                            rgb.extend(WALKER_GREEN);
                        } else {
                            rgb.extend([noise(x as i64 - offset.0, y as i64 - offset.1); RGB]);
                        }
                    }
                }
                rgb
            })
            .collect()
    }

    #[test]
    fn the_parabola_vertex_is_found_between_samples() {
        const VERTEX: f64 = 2.3;
        let errors: Vec<f64> = (0..5).map(|index| (index as f64 - VERTEX).powi(2)).collect();
        assert!((sub_pixel(&errors, 2) - VERTEX).abs() < 1e-9);
        assert_eq!(sub_pixel(&errors, 0), 0.0);
    }

    #[test]
    fn the_recording_layout_keeps_the_scripts_box() {
        let layout = Layout::recording(640, 480).unwrap();
        assert_eq!(layout.walker, Rect { left: 250, top: 120, right: 400, bottom: 330 });
        assert!(Layout::recording(100, 100).is_err());
    }

    #[test]
    fn a_synthetic_walk_gives_its_scroll_stretch_and_cycle() {
        let layout = Layout::new(WIDTH, HEIGHT, TEST_LABEL_ROWS, WALKER).unwrap();
        let (shifts, silhouettes) = analyse(synthetic_clip().into_iter().map(Ok), &layout).unwrap();
        assert_eq!(silhouettes.len(), 2 * STILL_FRAMES + MOVING_FRAMES);
        assert_eq!(shifts.len(), silhouettes.len() - 1);
        for shift in &shifts[STILL_FRAMES..STILL_FRAMES + MOVING_FRAMES] {
            assert!((shift.dx - STEP.0 as f64).abs() < SHIFT_TOLERANCE, "{shift:?}");
            assert!((shift.dy - STEP.1 as f64).abs() < SHIFT_TOLERANCE, "{shift:?}");
        }
        assert!(shifts[..STILL_FRAMES].iter().all(|shift| shift.speed() < SHIFT_TOLERANCE));
        let found = stretches(&shifts, &silhouettes);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start, found[0].end), (STILL_FRAMES, STILL_FRAMES + MOVING_FRAMES));
        let (period, correlation) = found[0].period.unwrap();
        assert!((period - CYCLE as f64).abs() < PERIOD_TOLERANCE, "period {period}");
        assert!(correlation > PEAK_SHARE);
    }

    #[test]
    fn a_short_or_still_run_has_no_period() {
        let still = vec![false; 4];
        assert_eq!(period(&[&still[..]; 2 * MIN_PERIOD]), None);
        assert_eq!(period(&[&still[..]; 4 * MIN_PERIOD]), None);
    }

    #[test]
    fn short_stills_join_a_stretch_and_long_ones_split_it() {
        let mut speeds = vec![0.0; 200];
        speeds[10..50].fill(2.0);
        speeds[50 + MAX_REPEATS - 1..90].fill(2.0);
        speeds[90 + MAX_REPEATS + 2..140].fill(2.0);
        assert_eq!(segments(&speeds), vec![(10, 90), (100, 140)]);
    }
}
