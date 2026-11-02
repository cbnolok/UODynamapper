//! Part `s6` of the `shrink` pass, LOSSY: how far a real video codec gets. AV1 4:4:4 per (action, direction) through
//! ffmpeg (libaom to encode, libdav1d to decode), the colours only: coverage or shape and the hue flags stay in lossless
//! planes, which are counted. Measure only: this is a bound, and it draws no palette colours.

use super::halved::{self, KIND_NO_SEQUENCE, SheetRow, Sprite};
use super::*;
use uocf::animation_sequence::AnimationSequence;

const FFMPEG: &str = "ffmpeg";
const OUTPUT_DIR: &str = "/home/claudio/rust-target/uodynamapper-scratch/anim-codec/av1";
const BODIES_PER_CLIENT: usize = 12;
const CRF_LEVELS: [u32; 5] = [10, 20, 30, 40, 50];
const CPU_USED: u32 = 6;
const FRAME_RATE: u32 = 10;
const MIN_SIDE: i32 = 16;
const CHANNELS: usize = 3;
const IVF_HEADER_BYTES: usize = 32;
const IVF_FRAME_HEADER_BYTES: usize = 12;
/// Every n-th frame is scored.
const SCORE_EVERY: usize = 2;
const P5: f64 = 0.05;
const MEDIAN: f64 = 0.5;
const P95: f64 = 0.95;
const CHROMA_MIDDLE: f32 = 128.0;
const KR: f32 = 0.299;
const KG: f32 = 0.587;
const KB: f32 = 0.114;
/// Bodies whose streams are decoded one at a time for the timing.
const TIMED_BODIES: usize = 3;
const STARTUP_RUNS: usize = 10;
const WORST_SHEETS_PER_LEVEL: usize = 2;
const SHEET_README: &str = "AV1 4:4:4 bound, LOSSY. Each sheet is a worst scorer. Columns, left to right: the frame before (real), the real frame, then CRF 10, 20, 30, 40, 50. The second row is the difference from the real frame, gain 4. Over grey 128.\n";

struct Stream {
    frames: Vec<usize>,
    left: i32,
    top: i32,
    w: usize,
    h: usize,
}

fn streams_of(planes: &Planes) -> Vec<Stream> {
    units(&planes.boxes, Unit::ActionDirection, planes.client.pixel_bytes())
        .into_iter()
        .map(|frames| {
            let (mut left, mut top, mut right, mut bottom) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for &f in &frames {
                let frame = &planes.boxes[f];
                left = left.min(frame.left);
                top = top.min(frame.top);
                right = right.max(frame.left + frame.w as i32);
                bottom = bottom.max(frame.top + frame.h as i32);
            }
            let side = |extent: i32| (extent.max(MIN_SIDE) + 1) & !1;
            Stream { frames, left, top, w: side(right - left) as usize, h: side(bottom - top) as usize }
        })
        .collect()
}

/// A frame's colours on the stream's canvas, with the transparent pixels filled from the left or from above, so that the
/// encoder spends nothing on edges the mask hides.
fn canvas_rgb(planes: &Planes, frame: &FrameBox, stream: &Stream) -> Vec<[u8; CHANNELS]> {
    let palette = &planes.palettes[frame.palette];
    let mut rgb = vec![[0u8; CHANNELS]; stream.w * stream.h];
    let mut known = vec![false; stream.w * stream.h];
    let (ox, oy) = ((frame.left - stream.left) as usize, (frame.top - stream.top) as usize);
    for y in 0..frame.h {
        for x in 0..frame.w {
            let i = frame.start + y * frame.w + x;
            if planes.is_opaque(i) {
                let [r, g, b, _] = palette[usize::from(planes.index[i])];
                let at = (y + oy) * stream.w + x + ox;
                rgb[at] = [r, g, b];
                known[at] = true;
            }
        }
    }
    for y in 0..stream.h {
        for x in 0..stream.w {
            let at = y * stream.w + x;
            if !known[at] {
                rgb[at] = if x > 0 {
                    rgb[at - 1]
                } else if y > 0 {
                    rgb[at - stream.w]
                } else {
                    [FIRST_PREDICTION; CHANNELS]
                };
            }
        }
    }
    rgb
}

fn to_yuv(rgb: &[[u8; CHANNELS]], out: &mut Vec<u8>) {
    let channel = |c: u8| f32::from(c);
    out.extend(rgb.iter().map(|p| (KR * channel(p[0]) + KG * channel(p[1]) + KB * channel(p[2])).round() as u8));
    out.extend(rgb.iter().map(|p| {
        let y = KR * channel(p[0]) + KG * channel(p[1]) + KB * channel(p[2]);
        ((channel(p[2]) - y) / (2.0 * (1.0 - KB)) + CHROMA_MIDDLE).round().clamp(0.0, 255.0) as u8
    }));
    out.extend(rgb.iter().map(|p| {
        let y = KR * channel(p[0]) + KG * channel(p[1]) + KB * channel(p[2]);
        ((channel(p[0]) - y) / (2.0 * (1.0 - KR)) + CHROMA_MIDDLE).round().clamp(0.0, 255.0) as u8
    }));
}

#[inline(always)]
fn from_yuv(y: u8, u: u8, v: u8) -> [u8; CHANNELS] {
    let (y, cb, cr) = (f32::from(y), f32::from(u) - CHROMA_MIDDLE, f32::from(v) - CHROMA_MIDDLE);
    let r = y + 2.0 * (1.0 - KR) * cr;
    let b = y + 2.0 * (1.0 - KB) * cb;
    let g = (y - KR * r - KB * b) / KG;
    [r.round().clamp(0.0, 255.0) as u8, g.round().clamp(0.0, 255.0) as u8, b.round().clamp(0.0, 255.0) as u8]
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn encode_stream(yuv: &[u8], w: usize, h: usize, crf: u32) -> Vec<u8> {
    let size = format!("{w}x{h}");
    let (rate, crf, cpu) = (FRAME_RATE.to_string(), crf.to_string(), CPU_USED.to_string());
    let args = strings(&[
        "-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "yuv444p", "-color_range", "pc", "-s", &size, "-r", &rate, "-i", "-",
        "-c:v", "libaom-av1", "-cpu-used", &cpu, "-crf", &crf, "-b:v", "0", "-threads", "1", "-row-mt", "0", "-g", "9999", "-pix_fmt", "yuv444p",
        "-color_range", "pc", "-f", "ivf", "-",
    ]);
    piped(FFMPEG, &args, yuv)
}

fn decode_stream(ivf: &[u8]) -> Vec<u8> {
    let args = strings(&[
        "-hide_banner", "-loglevel", "error", "-threads", "1", "-c:v", "libdav1d", "-f", "ivf", "-i", "-", "-fps_mode", "passthrough", "-f", "rawvideo",
        "-pix_fmt", "yuv444p", "-color_range", "pc", "-",
    ]);
    piped(FFMPEG, &args, ivf)
}

fn encoded_stream(planes: &Planes, stream: &Stream, crf: u32) -> Vec<u8> {
    let mut yuv = Vec::with_capacity(stream.w * stream.h * CHANNELS * stream.frames.len());
    for &f in &stream.frames {
        to_yuv(&canvas_rgb(planes, &planes.boxes[f], stream), &mut yuv);
    }
    encode_stream(&yuv, stream.w, stream.h, crf)
}

/// The frame's sprite with the decoded colours and the real alpha.
fn rebuilt_sprite(planes: &Planes, frame: &FrameBox, stream: &Stream, position: usize, decoded: &[u8]) -> Sprite {
    let pixels = stream.w * stream.h;
    let start = position * pixels * CHANNELS;
    let (ys, us, vs) = (&decoded[start..start + pixels], &decoded[start + pixels..start + 2 * pixels], &decoded[start + 2 * pixels..start + 3 * pixels]);
    let (ox, oy) = ((frame.left - stream.left) as usize, (frame.top - stream.top) as usize);
    let mut rgba = frame_rgba(planes, frame, &planes.index, &planes.coverage);
    for y in 0..frame.h {
        for x in 0..frame.w {
            let out = (y * frame.w + x) * RGBA_CHANNELS;
            if rgba[out + RGBA_CHANNELS - 1] != 0 {
                let at = (y + oy) * stream.w + x + ox;
                rgba[out..out + CHANNELS].copy_from_slice(&from_yuv(ys[at], us[at], vs[at]));
            }
        }
    }
    Sprite::with_rgba(frame, rgba)
}

#[derive(Default, Clone)]
struct LevelTally {
    video: u64,
    opaque: u64,
    exact: u64,
    on_palette: u64,
    error: u64,
    worst: u8,
    grey: u64,
    grey_kept: u64,
    flagged: u64,
    flagged_exact: u64,
    scores: Vec<(f64, &'static str, u16, u32, usize)>,
}

impl LevelTally {
    fn add(&mut self, other: LevelTally) {
        self.video += other.video;
        self.opaque += other.opaque;
        self.exact += other.exact;
        self.on_palette += other.on_palette;
        self.error += other.error;
        self.worst = self.worst.max(other.worst);
        self.grey += other.grey;
        self.grey_kept += other.grey_kept;
        self.flagged += other.flagged;
        self.flagged_exact += other.flagged_exact;
        self.scores.extend(other.scores);
    }
}

/// The lossless planes of a stream: coverage or shape, and the hue flag per opaque pixel (Enhanced).
fn lossless_bytes(planes: &Planes, stream: &Stream) -> (u64, u64) {
    let (index, mask) = unit_planes(planes, &stream.frames);
    let shape = match planes.client {
        Client::Cc => compress_level(&packed_bits(mask.iter().map(|&m| m != 0)), LEVEL_19, false).len() as u64,
        Client::Ec => compress_level(&mask, LEVEL_19, false).len() as u64,
    };
    if planes.client == Client::Cc {
        return (shape, 0);
    }
    let mut flags = Vec::new();
    let mut at = 0usize;
    for &f in &stream.frames {
        let frame = &planes.boxes[f];
        let colours = &planes.palettes[frame.palette];
        for _ in 0..frame.pixels() {
            if mask[at] != 0 {
                flags.push(colours[usize::from(index[at])][PALETTE_KEEP_COLOUR_BYTE] != 0);
            }
            at += 1;
        }
    }
    (shape, compress_level(&packed_bits(flags.into_iter()), LEVEL_19, false).len() as u64)
}

fn measure_stream(planes: &Planes, stream: &Stream, crf: u32, kinds: &HashMap<u32, &'static str>) -> LevelTally {
    let ivf = encoded_stream(planes, stream, crf);
    let decoded = decode_stream(&ivf);
    let pixels = stream.w * stream.h;
    assert_eq!(decoded.len(), pixels * CHANNELS * stream.frames.len(), "{}: decoded frames", planes.label);
    let first = &planes.boxes[stream.frames[0]];
    let kind = kinds.get(&u32::from(first.action)).copied().unwrap_or(KIND_NO_SEQUENCE);
    let mut tally = LevelTally { video: (ivf.len() - IVF_HEADER_BYTES - IVF_FRAME_HEADER_BYTES * stream.frames.len()) as u64, ..LevelTally::default() };
    for (position, &f) in stream.frames.iter().enumerate() {
        let frame = &planes.boxes[f];
        let colours = &planes.palettes[frame.palette];
        let palette_set: HashSet<[u8; CHANNELS]> = colours.iter().map(|c| [c[0], c[1], c[2]]).collect();
        let real = Sprite::of(planes, frame);
        let rebuilt = rebuilt_sprite(planes, frame, stream, position, &decoded);
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = frame.start + y * frame.w + x;
                if !planes.is_opaque(i) {
                    continue;
                }
                let want = colours[usize::from(planes.index[i])];
                let got = rebuilt.at(frame.left + x as i32, frame.top + y as i32);
                let largest = (0..CHANNELS).map(|c| want[c].abs_diff(got[c])).max().unwrap();
                let got_rgb = [got[0], got[1], got[2]];
                tally.opaque += 1;
                tally.exact += u64::from(largest == 0);
                tally.on_palette += u64::from(palette_set.contains(&got_rgb));
                tally.error += u64::from(largest);
                tally.worst = tally.worst.max(largest);
                if want[0] == want[1] && want[1] == want[2] {
                    tally.grey += 1;
                    tally.grey_kept += u64::from(got[0] == got[1] && got[1] == got[2]);
                }
                if planes.client == Client::Ec && want[PALETTE_KEEP_COLOUR_BYTE] != 0 {
                    tally.flagged += 1;
                    tally.flagged_exact += u64::from(largest == 0);
                }
            }
        }
        if position % SCORE_EVERY == 0 {
            tally.scores.push((halved::score(&real, &rebuilt), kind, first.action, first.direction, position));
        }
    }
    tally
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted.get(((sorted.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(0.0)
}

fn sheet_row(planes: &Planes, action: u16, direction: u32, position: usize) -> Option<SheetRow> {
    let streams = streams_of(planes);
    let stream = streams.iter().find(|s| planes.boxes[s.frames[0]].action == action && planes.boxes[s.frames[0]].direction == direction)?;
    let frame = &planes.boxes[stream.frames[position]];
    let before = &planes.boxes[stream.frames[position.saturating_sub(1)]];
    let rebuilt: Vec<Sprite> = CRF_LEVELS
        .iter()
        .map(|&crf| rebuilt_sprite(planes, frame, stream, position, &decode_stream(&encoded_stream(planes, stream, crf))))
        .collect();
    let mut cells = vec![Sprite::of(planes, before), Sprite::of(planes, frame)];
    cells.extend(rebuilt);
    Some(SheetRow { cells: cells.try_into().ok()? })
}

pub(super) fn part_s6(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, time: bool) {
    let ec_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client folder"));
    std::fs::create_dir_all(OUTPUT_DIR).expect("the sheet folder");
    std::fs::write(Path::new(OUTPUT_DIR).join("README.txt"), SHEET_README).expect("the sheet notes");
    let state = cpu_state();
    println!("\n### s6 (LOSSY): AV1 4:4:4, libaom cpu-used {CPU_USED}, per (action, direction); CPU {state}; load average {}", load_average());
    let ec_sequences = halved::load_sequences(&ec_dir);
    let cc_sequences = halved::load_sequences(cc_dir);
    let ec_map = halved::sequence_of_frames_body(&ec_sequences, &ec_dir);
    let cc_map = halved::sequence_of_frames_body(&cc_sequences, cc_dir);
    for client in [Client::Cc, Client::Ec] {
        let sample = sample_of(present, client, BODIES_PER_CLIENT);
        let planes: Vec<Planes> = sample.par_iter().map(|&i| load_planes(sources, &bodies[i], true)).collect();
        let kinds: Vec<HashMap<u32, &'static str>> = planes
            .iter()
            .zip(&sample)
            .map(|(p, &i)| {
                let number = halved::body_number(p);
                let sequence: Option<&AnimationSequence> = match &bodies[i] {
                    BodySource::CcMul { .. } => None,
                    BodySource::CcUop { .. } => number.and_then(|n| cc_map.get(&n)).and_then(|id| cc_sequences.get(id)),
                    BodySource::Ec { .. } => number.and_then(|n| ec_map.get(&n)).and_then(|id| ec_sequences.get(id)),
                };
                sequence.map(halved::kinds_of).unwrap_or_default()
            })
            .collect();
        let streams: Vec<Vec<Stream>> = planes.iter().map(streams_of).collect();
        let canvas: u64 = streams.iter().flatten().map(|s| (s.w * s.h * s.frames.len()) as u64).sum();
        let trimmed: u64 = planes.iter().map(|p| p.index.len() as u64).sum();
        let (shape, flags) = planes.iter().zip(&streams).fold((0u64, 0u64), |sum, (p, list)| {
            let parts: Vec<(u64, u64)> = list.par_iter().map(|s| lossless_bytes(p, s)).collect();
            (sum.0 + parts.iter().map(|x| x.0).sum::<u64>(), sum.1 + parts.iter().map(|x| x.1).sum::<u64>())
        });
        let base = candidate("z19 action", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Action);
        let z19: u64 = planes.par_iter().map(|p| encode_candidate(p, &base).total()).sum();
        let bc7: u64 = planes.iter().map(|p| p.bc7_zstd).sum();
        let palettes: u64 = planes.iter().map(|p| p.palettes.len() as u64 * (PALETTE_COLOURS * EC_PALETTE_ENTRY_BYTES) as u64).sum();
        println!(
            "\n#### {}, {} bodies spread by size; {:.1} Mpx trimmed, {:.1} Mpx on the canvases (× {:.2}); shape or coverage {:.2} MB, hue flags {:.2} MB, zstd 19 per action {:.2} MB, BC7 z7 {:.2} MB\n",
            client.label(),
            planes.len(),
            trimmed as f64 / PIXELS_PER_MEGAPIXEL,
            canvas as f64 / PIXELS_PER_MEGAPIXEL,
            canvas as f64 / trimmed.max(1) as f64,
            mb(shape),
            mb(flags),
            mb(z19),
            mb(bc7),
        );
        println!("Palettes are not stored by this candidate (raw {:.2} MB for the others).\n", mb(palettes));
        let mut levels: Vec<(u32, Vec<LevelTally>)> = Vec::new();
        for crf in CRF_LEVELS {
            let started = Instant::now();
            let per_body: Vec<LevelTally> = planes
                .iter()
                .zip(&streams)
                .zip(&kinds)
                .map(|((p, list), kinds)| {
                    let parts: Vec<LevelTally> = list.par_iter().map(|s| measure_stream(p, s, crf, kinds)).collect();
                    let mut sum = LevelTally::default();
                    parts.into_iter().for_each(|t| sum.add(t));
                    sum
                })
                .collect();
            println!("CRF {crf}: encoded, decoded and scored in {:.0} s", started.elapsed().as_secs_f64());
            levels.push((crf, per_body));
        }
        println!("\n| CRF | video MB | with shape and flags MB | × zstd 19 action | × BC7 z7 | SSIMULACRA2 median | p5 | min | pixels with the exact colour | on the palette | mean largest channel error | worst | grey pixels that stay grey | hue-flagged pixels exact |");
        println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        for (crf, per_body) in &levels {
            let mut all = LevelTally::default();
            per_body.iter().cloned().for_each(|t| all.add(t));
            let mut scores: Vec<f64> = all.scores.iter().map(|s| s.0).collect();
            scores.sort_by(f64::total_cmp);
            let total = all.video + shape + flags;
            let percent = |part: u64, of: u64| part as f64 / of.max(1) as f64 * PERCENT;
            println!(
                "| {crf} | {:.2} | {:.2} | {:.3} | {:.3} | {:.1} | {:.1} | {:.1} | {:.1} % | {:.1} % | {:.2} | {} | {:.1} % | {:.1} % |",
                mb(all.video),
                mb(total),
                total as f64 / z19.max(1) as f64,
                total as f64 / bc7.max(1) as f64,
                quantile(&scores, MEDIAN),
                quantile(&scores, P5),
                scores.first().copied().unwrap_or(0.0),
                percent(all.exact, all.opaque),
                percent(all.on_palette, all.opaque),
                all.error as f64 / all.opaque.max(1) as f64,
                all.worst,
                percent(all.grey_kept, all.grey),
                percent(all.flagged_exact, all.flagged),
            );
        }
        println!("\n| CRF | action kind | frames scored | median | p5 | min |\n|---:|---|---:|---:|---:|---:|");
        for (crf, per_body) in &levels {
            let mut by_kind = BTreeMap::<&'static str, Vec<f64>>::new();
            for tally in per_body {
                for score in &tally.scores {
                    by_kind.entry(score.1).or_default().push(score.0);
                }
            }
            for (kind, scores) in &mut by_kind {
                scores.sort_by(f64::total_cmp);
                println!("| {crf} | {kind} | {} | {:.1} | {:.1} | {:.1} |", scores.len(), quantile(scores, MEDIAN), quantile(scores, P5), scores[0]);
            }
        }
        // The worst sheets.
        for (crf, per_body) in &levels {
            let mut worst: Vec<(f64, usize, u16, u32, usize)> =
                per_body.iter().enumerate().flat_map(|(n, t)| t.scores.iter().map(move |s| (s.0, n, s.2, s.3, s.4))).collect();
            worst.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(score, n, action, direction, position) in worst.iter().take(WORST_SHEETS_PER_LEVEL) {
                println!("  worst at CRF {crf}: {} action {action} stream {direction} frame {position}, {score:.1}", planes[n].label);
                if let Some(row) = sheet_row(&planes[n], action, direction, position) {
                    let name = format!("worst_crf{crf}_{}_action{action}_{direction}_{position}.png", halved::slug(&planes[n].label));
                    halved::write_sheet(&Path::new(OUTPUT_DIR).join(name), &[row]);
                }
            }
        }
        if time {
            time_decode(&planes, &streams);
        }
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part s6 ({state} at the start, {end} at the end): discard its timings");
    }
}

/// One stream at a time on one thread, through ffmpeg: the process start is measured on a tiny stream and taken off.
fn time_decode(planes: &[Planes], streams: &[Vec<Stream>]) {
    println!("\nDecode through ffmpeg and libdav1d, one thread, one stream at a time; load average {}", load_average());
    let tiny = encode_stream(&vec![FIRST_PREDICTION; MIN_SIDE as usize * MIN_SIDE as usize * CHANNELS], MIN_SIDE as usize, MIN_SIDE as usize, CRF_LEVELS[0]);
    let mut starts: Vec<f64> = (0..STARTUP_RUNS)
        .map(|_| {
            let started = Instant::now();
            black_box(decode_stream(&tiny));
            started.elapsed().as_secs_f64()
        })
        .collect();
    starts.sort_by(f64::total_cmp);
    let startup = quantile(&starts, MEDIAN);
    println!("A one-frame stream takes {:.1} ms, taken off each stream below.\n", ms(startup));
    println!("| CRF | Mpx/s on the canvas | action decode median ms | p95 | max |\n|---:|---:|---:|---:|---:|");
    let chosen: Vec<usize> = (0..TIMED_BODIES).map(|n| planes.len() * (2 * n + 1) / (2 * TIMED_BODIES)).collect();
    for crf in CRF_LEVELS {
        let (mut seconds, mut pixels) = (0.0f64, 0u64);
        let mut actions = Vec::new();
        for &n in &chosen {
            let encoded: Vec<Vec<u8>> = streams[n].par_iter().map(|s| encoded_stream(&planes[n], s, crf)).collect();
            let mut by_action = BTreeMap::<u16, f64>::new();
            for (stream, ivf) in streams[n].iter().zip(&encoded) {
                let started = Instant::now();
                black_box(decode_stream(ivf));
                let spent = (started.elapsed().as_secs_f64() - startup).max(0.0);
                seconds += spent;
                pixels += (stream.w * stream.h * stream.frames.len()) as u64;
                *by_action.entry(planes[n].boxes[stream.frames[0]].action).or_default() += spent;
            }
            actions.extend(by_action.into_values());
        }
        actions.sort_by(f64::total_cmp);
        println!(
            "| {crf} | {:.1} | {:.1} | {:.1} | {:.1} |",
            pixels as f64 / PIXELS_PER_MEGAPIXEL / seconds.max(f64::MIN_POSITIVE),
            ms(quantile(&actions, MEDIAN)),
            ms(quantile(&actions, P95)),
            ms(actions.last().copied().unwrap_or(0.0)),
        );
    }
}
