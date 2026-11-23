//! The speckle of the Enhanced animation frames (docs/agent_notes/ec_frame_quantization.md): where it comes from, how
//! widespread it is, and what the candidate filters do to it, to BC7, to SPAN 2x and to the RIFE in-betweens.
//!
//! Usage (client folders are read only):
//! - `survey <client dir> <actions per body> <tsv>`: per body, the palette and the speckle measures of the first
//!   `actions per body` actions' frames (0 for all).
//! - `palette <client dir> <body> <action>`: one entry's palette: its size, the colours its frames use, the channel
//!   depth, how far the outliers lie from their neighbours in palette steps, and the share of dither-like 3x3 windows
//!   next to two re-quantized controls (nearest and Floyd-Steinberg).
//! - `pair <client dir> <other client dir> <body> <action>`: the same entry in two clients (EC and KR): boxes and
//!   texels compared.
//! - `frames <client dir> <package dir> <out dir> <name=body:action:stored direction:frame> ...`: per frame, raw RGBA
//!   (`<name>_<variant>_<w>x<h>.rgba`) of the source, the package's frame placed on the source's box, each filter, and
//!   SPAN 2x of the source and each filter encoded BC7 UltraFast as the game does; their measures in `frames.tsv`.
//! - `measure <client dir> <bodies> <actions per body> <SPAN frames per action> <tsv>`: per body and filter, the speckle
//!   and fidelity at 1x, BC7 UltraFast of the frames packed into pages (PSNR to the filtered frames, Zstd 9 bytes), and
//!   the speckle of SPAN 2x and of RIFE x2 in-betweens of the first stored direction.
//! - `clip <client dir> <out dir> <filters> <name=body:action:stored direction> ...`: whole clips for the motion
//!   videos: each frame of each filter at 1x, SPAN 2x (UltraFast) and the RIFE x2 sequence, raw RGBA, listed in
//!   `clip.tsv` with their anchors.
//! - `timing <client dir> <bodies> <actions per body>`: CPU time of each filter per megapixel on one thread.
//! - `source-body <package dir> <server bodies>`: the `AnimationFrame` bodies the package shows for server bodies.

mod filters;
mod metrics;
mod source;

use std::collections::BTreeSet;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use rayon::prelude::*;
use udd_assets::mobile_anim_ec::MobileAnimEcPackage;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::frame_upscales::FrameUpscaler;
use uot_gpu_compute::in_betweens::{InBetweenMaker, SequenceFrame};
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_gpu_compute::strides::StrideFrame;
use uot_texture_codecs::bc7::{ImageExtent, decode_bc7_to_rgba8888};

use filters::Filter;
use metrics::{Fidelity, Speckle, psnr, speckle};
use source::{Frame, Palette, RGBA_BYTES, Sources};

/// The encode the game gives the SPAN and RIFE frames, and the packages their pages.
const GAME_QUALITY: Bc7Quality = Bc7Quality::UltraFast;
/// The upscaler's largest frame side, as the game's canvas cap (260 with the border).
const MAX_SPAN_SIDE: u32 = 256;
/// The pages measure packs frames into (the packages' 1024 x 1024 atlas) with the packer's 2-texel gutter.
const PAGE_SIDE: usize = 1024;
const GUTTER: usize = 2;
const ZSTD_LEVEL: i32 = 9;
const RIFE_FACTOR: u32 = 2;
const MEGA: f64 = 1.0e6;

fn speckle_of(frame_rgba: &[u8], width: usize, height: usize) -> Speckle {
    speckle(frame_rgba, width, height)
}

fn survey(dir: &str, actions_per_body: usize, tsv: &str) {
    let sources = Sources::open(dir);
    let rows: Vec<String> = (0..source::MAX_BODY).into_par_iter().filter_map(|body| {
        let mut taken = 0;
        let (mut total, mut colours, mut frames, mut texels) = (Speckle::default(), Vec::new(), 0usize, 0usize);
        for action in 0..source::MAX_ACTION {
            if actions_per_body != 0 && taken >= actions_per_body { break; }
            let Some(animation) = sources.entry(body, action) else { continue };
            taken += 1;
            colours.push(animation.colours.len());
            for frame in source::all_frames(&animation) {
                frames += 1;
                texels += frame.solid.iter().filter(|&&s| s).count();
                total.add(&speckle_of(&frame.rgba, frame.width, frame.height));
            }
        }
        (taken > 0).then(|| {
            colours.sort_unstable();
            format!("{body}\t{taken}\t{frames}\t{texels}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.4}\t{}\t{}",
                colours[colours.len() / 2], total.interior, total.outliers, total.per_mille(), total.dark_share(),
                total.mean_high_frequency(), total.lag_correlation(), total.clipped, total.clipped_outliers)
        })
    }).collect();
    let mut out = std::fs::File::create(tsv).unwrap();
    writeln!(out, "body\tactions\tframes\tsolid\tpalette_median\tinterior\toutliers\toutliers_per_mille\tdark_share\t\
        high_frequency\tlag1\tclipped\tclipped_outliers").unwrap();
    for row in &rows { writeln!(out, "{row}").unwrap(); }
    println!("{} bodies", rows.len());
}

fn palette(dir: &str, body: u32, action: u16) {
    let sources = Sources::open(dir);
    let animation = sources.entry(body, action).expect("no such entry");
    let frames = source::all_frames(&animation);
    let used: BTreeSet<u8> = frames.iter()
        .flat_map(|f| f.index.iter().zip(&f.solid).filter(|(_, s)| **s).map(|(i, _)| *i)).collect();
    let hueable = animation.colours.iter().filter(|c| c[3] == 0).count();
    println!("body {body} action {action}: {} frames ({} per direction), {} palette colours ({} hueable), {} used by \
        solid texels", frames.len(), source::frame_count(&animation), animation.colours.len(), hueable, used.len());
    for step in [2u32, 4, 8, 16] {
        let multiples = animation.colours.iter().flat_map(|c| c[..3].to_vec())
            .filter(|&v| u32::from(v) % step == 0).count();
        println!("  channel values that are multiples of {step}: {:.1} % (8-bit uniform: {:.1} %)",
            100.0 * multiples as f64 / (3 * animation.colours.len()) as f64, 100.0 / f64::from(step));
    }
    let distinct: Vec<usize> = (0..3).map(|c| animation.colours.iter().map(|x| x[c]).collect::<BTreeSet<_>>().len())
        .collect();
    println!("  distinct values per channel: {distinct:?}");
    let pal = Palette::of(&animation);
    let mut spacing = pal.spacing.clone();
    spacing.sort_by(f32::total_cmp);
    println!("  palette spacing (mean distance to the 3 nearest colours): median {:.1}, p10 {:.1}, p90 {:.1}",
        spacing[spacing.len() / 2], spacing[spacing.len() / 10], spacing[spacing.len() * 9 / 10]);
    let mut total = Speckle::default();
    let mut distinct_per_frame = Vec::new();
    for f in &frames {
        total.add(&speckle_of(&f.rgba, f.width, f.height));
        distinct_per_frame.push(f.index.iter().zip(&f.solid).filter(|(_, s)| **s).map(|(i, _)| *i)
            .collect::<BTreeSet<_>>().len());
    }
    // Each outlier: how far it is from its neighbours' median in palette spacings, and how many palette colours lie
    // nearer to that median than the outlier's own (0: the outlier is the median's nearest colour).
    let (mut steps, mut ranks) = (Vec::new(), Vec::new());
    for f in &frames {
        let inner = metrics::interior(&f.solid, f.width, f.height);
        for i in (0..f.width * f.height).filter(|&i| inner[i]) {
            let Some(m) = metrics::is_outlier(&f.rgba, f.width, i, metrics::OUTLIER_MIN_DISTANCE,
                metrics::OUTLIER_SPREAD_FACTOR) else { continue };
            let own = pal.rgb[usize::from(f.index[i])];
            let d = source::distance(&own, &m);
            steps.push(d / pal.spacing[usize::from(f.index[i])]);
            ranks.push(pal.rgb.iter().filter(|c| source::distance(c, &m) < d).count());
        }
    }
    steps.sort_by(f32::total_cmp);
    ranks.sort_unstable();
    if !steps.is_empty() {
        let at = |share: f32| ((steps.len() - 1) as f32 * share) as usize;
        println!("  outliers from their neighbours' median, in palette spacings: p10 {:.1}, median {:.1}, p90 {:.1}; \
            palette colours nearer that median than the outlier's: p10 {}, median {}, p90 {}", steps[at(0.1)],
            steps[at(0.5)], steps[at(0.9)], ranks[at(0.1)], ranks[at(0.5)], ranks[at(0.9)]);
    }
    // Dither windows in the frames as stored, and in two re-quantizations of them to fewer colours (median cut):
    // nearest colour, and Floyd-Steinberg error diffusion, the controls of what a dithered quantizer leaves.
    let (mut stored, mut nearest, mut diffused) = ((0, 0), (0, 0), (0, 0));
    let add = |total: &mut (usize, usize), part: (usize, usize)| { total.0 += part.0; total.1 += part.1; };
    let mut weights = std::collections::BTreeMap::new();
    for f in &frames {
        for i in (0..f.width * f.height).filter(|&i| f.solid[i]) {
            let at = i * RGBA_BYTES;
            *weights.entry([f.rgba[at], f.rgba[at + 1], f.rgba[at + 2]]).or_insert(0u64) += 1;
        }
    }
    let reduced = uocf::enhanced::animationframe::quantize::median_cut(weights.into_iter().collect(), CONTROL_COLOURS);
    let reduced_palette = Palette::from_rgb(reduced.iter().map(|c| [f32::from(c[0]), f32::from(c[1]), f32::from(c[2])])
        .collect());
    for f in &frames {
        add(&mut stored, dither_windows(f.width, f.height, &f.index, &f.solid, &pal));
        add(&mut nearest, dither_windows(f.width, f.height, &requantize(f, &reduced_palette, false), &f.solid,
            &reduced_palette));
        add(&mut diffused, dither_windows(f.width, f.height, &requantize(f, &reduced_palette, true), &f.solid,
            &reduced_palette));
    }
    let share = |t: (usize, usize)| 100.0 * t.1 as f64 / t.0.max(1) as f64;
    println!("  dither windows (3x3, two palette-adjacent colours, at least {DITHER_TRANSITIONS} of 12 pairs differing): \
        stored {:.2} %; re-quantized to {CONTROL_COLOURS} colours: nearest {:.2} %, Floyd-Steinberg {:.2} %",
        share(stored), share(nearest), share(diffused));
    distinct_per_frame.sort_unstable();
    println!("  colours per frame: median {}, max {}", distinct_per_frame[distinct_per_frame.len() / 2],
        distinct_per_frame.last().unwrap());
    println!("  speckle: {:.2} outliers per 1000 interior texels ({} of {}), {:.0} % darker, high frequency {:.2}, \
        residual lag-1 correlation {:.3}", total.per_mille(), total.outliers, total.interior, 100.0 * total.dark_share(),
        total.mean_high_frequency(), total.lag_correlation());
}

/// Colours of the re-quantization controls.
const CONTROL_COLOURS: usize = 64;
/// A dither window: two colours at most `DITHER_ADJACENT` palette spacings apart, intermixed so that at least
/// `DITHER_TRANSITIONS` of the window's 12 four-adjacent pairs differ (a band edge across a 3x3 window gives 3 or 4).
const DITHER_ADJACENT: f32 = 1.5;
const DITHER_TRANSITIONS: usize = 6;
/// Floyd-Steinberg weights, in sixteenths: right, below left, below, below right.
const FLOYD_STEINBERG: [(isize, isize, f32); 4] = [(1, 0, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0),
    (1, 1, 1.0 / 16.0)];

/// Interior 3x3 windows (all solid), and those that look dithered.
fn dither_windows(width: usize, height: usize, index: &[u8], solid: &[bool], palette: &Palette) -> (usize, usize) {
    let (mut windows, mut dithered) = (0, 0);
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let at = |dx: usize, dy: usize| (y + dy - 1) * width + x + dx - 1;
            if !(0..3).all(|dy| (0..3).all(|dx| solid[at(dx, dy)])) { continue; }
            windows += 1;
            let colours: BTreeSet<u8> = (0..3).flat_map(|dy| (0..3).map(move |dx| (dx, dy)))
                .map(|(dx, dy)| index[at(dx, dy)]).collect();
            if colours.len() != 2 { continue; }
            let pair: Vec<usize> = colours.iter().map(|&c| usize::from(c)).collect();
            let spacing = palette.spacing[pair[0]].max(palette.spacing[pair[1]]);
            if source::distance(&palette.rgb[pair[0]], &palette.rgb[pair[1]]) > DITHER_ADJACENT * spacing { continue; }
            let mut transitions = 0;
            for dy in 0..3 {
                for dx in 0..3 {
                    if dx < 2 && index[at(dx, dy)] != index[at(dx + 1, dy)] { transitions += 1; }
                    if dy < 2 && index[at(dx, dy)] != index[at(dx, dy + 1)] { transitions += 1; }
                }
            }
            if transitions >= DITHER_TRANSITIONS { dithered += 1; }
        }
    }
    (windows, dithered)
}

/// A frame's solid texels as indices of `palette`: the nearest colour, or Floyd-Steinberg over the solid texels.
fn requantize(frame: &Frame, palette: &Palette, diffuse: bool) -> Vec<u8> {
    let (w, h) = (frame.width, frame.height);
    let mut work: Vec<[f32; 3]> = (0..w * h).map(|i| metrics::rgb(&frame.rgba, i)).collect();
    let mut out = vec![0u8; w * h];
    for i in (0..w * h).filter(|&i| frame.solid[i]) {
        let c = work[i];
        let best = (0..palette.rgb.len()).min_by(|&a, &b| source::distance(&palette.rgb[a], &c)
            .total_cmp(&source::distance(&palette.rgb[b], &c))).unwrap();
        out[i] = best as u8;
        if !diffuse { continue; }
        let error = [c[0] - palette.rgb[best][0], c[1] - palette.rgb[best][1], c[2] - palette.rgb[best][2]];
        let (x, y) = ((i % w) as isize, (i / w) as isize);
        for &(dx, dy, weight) in &FLOYD_STEINBERG {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || nx as usize >= w || ny as usize >= h { continue; }
            let j = ny as usize * w + nx as usize;
            if !frame.solid[j] { continue; }
            for k in 0..3 { work[j][k] += weight * error[k]; }
        }
    }
    out
}

fn pair(dir: &str, other: &str, body: u32, action: u16) {
    let (a, b) = (Sources::open(dir), Sources::open(other));
    let (ea, eb) = (a.entry(body, action).expect("not in the first"), b.entry(body, action).expect("not in the second"));
    println!("palettes {} and {}, frames {} and {}", ea.colours.len(), eb.colours.len(), ea.frames.len(), eb.frames.len());
    let (mut same_box, mut same_pixels, mut compared) = (0, 0, 0);
    let (mut squared, mut count) = (0.0, 0.0);
    for fa in &ea.frames {
        let Some(fb) = eb.frames.iter().find(|f| f.frame_number == fa.frame_number && f.action == fa.action) else {
            continue;
        };
        compared += 1;
        let boxes = |f: &uocf::enhanced::animationframe::FrameEntry| (f.init_coords_x, f.init_coords_y, f.end_coords_x,
            f.end_coords_y);
        if boxes(fa) != boxes(fb) { continue; }
        same_box += 1;
        let (da, db) = (Frame::decode(&ea, fa), Frame::decode(&eb, fb));
        if da.rgba == db.rgba { same_pixels += 1; }
        let (s, c) = metrics::opaque_error(&da.rgba, &db.rgba);
        squared += s;
        count += c;
    }
    println!("{compared} frames in both, {same_box} with the same box, {same_pixels} identical; PSNR of the second to \
        the first over the first's opaque texels {:.2} dB", psnr(squared, count));
}

/// `body:action:stored direction[:frame]`.
fn parse_spec(spec: &str) -> (u32, u16, usize, usize) {
    let f: Vec<usize> = spec.split(':').map(|x| x.parse().unwrap()).collect();
    (f[0] as u32, f[1] as u16, f[2], f.get(3).copied().unwrap_or(0))
}

fn write_rgba(dir: &str, name: &str, variant: &str, width: usize, height: usize, rgba: &[u8]) {
    std::fs::write(format!("{dir}/{name}_{variant}_{width}x{height}.rgba"), rgba).unwrap();
}

fn encode_decode(gpu: &PageEncoderGpu, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let blocks = gpu.encode(rgba, width, height, GAME_QUALITY).unwrap().concat();
    decode_bc7_to_rgba8888(&blocks, ImageExtent::new(width, height).unwrap()).unwrap()
}

fn stride_frame<'a>(frame: &Frame, rgba: &'a [u8]) -> StrideFrame<'a> {
    StrideFrame { width: frame.width as u16, height: frame.height as u16, anchor_x: frame.anchor_x,
        anchor_y: frame.height as i32 + frame.anchor_y, rgba }
}

/// SPAN 2x of a frame, encoded and decoded as the game encodes it; `None` past the canvas cap.
fn span_game(upscaler: &mut FrameUpscaler, gpu: &PageEncoderGpu, frame: &Frame, rgba: &[u8])
    -> Option<(usize, usize, Vec<u8>)> {
    if frame.width as u32 > MAX_SPAN_SIDE || frame.height as u32 > MAX_SPAN_SIDE { return None; }
    let out = upscaler.upscale(&stride_frame(frame, rgba), false).unwrap();
    let decoded = encode_decode(gpu, &out.rgba, out.width, out.height);
    Some((out.width as usize, out.height as usize, decoded))
}

/// The package frame of a source frame, placed on the source frame's box (the package trims transparent rows and
/// columns).
fn package_frame(package: &MobileAnimEcPackage, body: u32, action: u16, frame_number: u16, source_frame: &Frame)
    -> Option<Vec<u8>> {
    let animations = package.animations();
    let record = package.frames().iter().find(|f| f.source_action == action && f.source_frame_number == frame_number
        && f.page_index != udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX
        && animations[f.animation_index as usize].source_body_id == body)?;
    let rgba = package.read_frame_rgba(record).ok()?;
    let left = source_frame.anchor_x - i32::from(record.center_x);
    let top = source_frame.anchor_y - i32::from(record.center_y);
    let mut placed = vec![0u8; source_frame.rgba.len()];
    for y in 0..usize::from(record.height) {
        for x in 0..usize::from(record.width) {
            let (sx, sy) = (left + x as i32, top + y as i32);
            if sx < 0 || sy < 0 || sx as usize >= source_frame.width || sy as usize >= source_frame.height { continue; }
            let to = (sy as usize * source_frame.width + sx as usize) * RGBA_BYTES;
            let from = (y * usize::from(record.width) + x) * RGBA_BYTES;
            placed[to..to + RGBA_BYTES].copy_from_slice(&rgba[from..from + RGBA_BYTES]);
        }
    }
    Some(placed)
}

fn frames(dir: &str, package_dir: &str, out: &str, specs: &[String]) {
    std::fs::create_dir_all(out).unwrap();
    let sources = Sources::open(dir);
    let package = MobileAnimEcPackage::load(format!("{package_dir}/mobile_anim_ec.uddp")).unwrap();
    println!("package pages: {:?}", package.pixel_format());
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_SPAN_SIDE).unwrap();
    println!("GPU: {}", gpu.adapter());
    let mut tsv = std::fs::File::create(format!("{out}/frames.tsv")).unwrap();
    writeln!(tsv, "name\tvariant\twidth\theight\toutliers_per_mille\tdark_share\thigh_frequency\tlag1\tpsnr_to_source\t\
        low_pass_psnr\tedge_ratio").unwrap();
    for named in specs {
        let (name, spec) = named.split_once('=').expect("name=body:action:direction:frame");
        let (body, action, direction, index) = parse_spec(spec);
        let animation = sources.entry(body, action).expect("no entry");
        let palette = Palette::of(&animation);
        let entry = source::find_entry(&animation, direction, index).expect("no frame");
        let frame = Frame::decode(&animation, &entry);
        let (w, h) = (frame.width, frame.height);
        let mut row = |variant: &str, width: usize, height: usize, rgba: &[u8], reference: Option<&[u8]>| {
            let s = speckle_of(rgba, width, height);
            let (full, low, edge) = reference.map(|r| {
                let f = Fidelity::of(r, rgba, width, height);
                (format!("{:.2}", f.full_psnr()), format!("{:.2}", f.low_pass_psnr()), format!("{:.3}", f.edge_ratio()))
            }).unwrap_or_default();
            writeln!(tsv, "{name}\t{variant}\t{width}\t{height}\t{:.3}\t{:.3}\t{:.3}\t{:.4}\t{full}\t{low}\t{edge}",
                s.per_mille(), s.dark_share(), s.mean_high_frequency(), s.lag_correlation()).unwrap();
        };
        if let Some(packaged) = package_frame(&package, body, action, entry.frame_number, &frame) {
            write_rgba(out, name, "package", w, h, &packaged);
            row("package", w, h, &packaged, Some(&frame.rgba));
        }
        let mut span_source = None;
        for filter in Filter::ALL {
            let rgba = filter.apply(&frame, &palette);
            write_rgba(out, name, filter.name(), w, h, &rgba);
            row(filter.name(), w, h, &rgba, Some(&frame.rgba));
            if let Some((sw, sh, span)) = span_game(&mut upscaler, &gpu, &frame, &rgba) {
                write_rgba(out, name, &format!("span-{}", filter.name()), sw, sh, &span);
                if filter == Filter::Source { span_source = Some(span.clone()); }
                row(&format!("span-{}", filter.name()), sw, sh, &span, span_source.as_deref());
            }
        }
        println!("{name} ({spec}): {w}x{h}");
    }
}

/// Shelf-packs frames onto pages of `PAGE_SIDE`, each with its gutter; returns the pages and where each frame went.
fn pack(frames: &[(usize, usize, Vec<u8>)]) -> (Vec<Vec<u8>>, Vec<(usize, usize, usize)>) {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(frames[i].1));
    let mut pages = vec![vec![0u8; PAGE_SIDE * PAGE_SIDE * RGBA_BYTES]];
    let mut places = vec![(0, 0, 0); frames.len()];
    let (mut x, mut y, mut shelf) = (0usize, 0usize, 0usize);
    for i in order {
        let (w, h, rgba) = (&frames[i].0, &frames[i].1, &frames[i].2);
        let (pw, ph) = (w + 2 * GUTTER, h + 2 * GUTTER);
        if x + pw > PAGE_SIDE { x = 0; y += shelf; shelf = 0; }
        if y + ph > PAGE_SIDE {
            pages.push(vec![0u8; PAGE_SIDE * PAGE_SIDE * RGBA_BYTES]);
            (x, y, shelf) = (0, 0, 0);
        }
        let page = pages.last_mut().unwrap();
        for row in 0..*h {
            let to = ((y + GUTTER + row) * PAGE_SIDE + x + GUTTER) * RGBA_BYTES;
            page[to..to + w * RGBA_BYTES].copy_from_slice(&rgba[row * w * RGBA_BYTES..(row + 1) * w * RGBA_BYTES]);
        }
        places[i] = (pages.len() - 1, x + GUTTER, y + GUTTER);
        x += pw;
        shelf = shelf.max(ph);
    }
    (pages, places)
}

fn cut(page: &[u8], x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
    (0..h).flat_map(|row| {
        let at = ((y + row) * PAGE_SIDE + x) * RGBA_BYTES;
        page[at..at + w * RGBA_BYTES].to_vec()
    }).collect()
}

/// The actions of a body to measure: the first `count` that exist (0 for all).
fn actions_of(sources: &Sources, body: u32, count: usize) -> Vec<u16> {
    let mut actions = Vec::new();
    for action in 0..source::MAX_ACTION {
        if count != 0 && actions.len() >= count { break; }
        if sources.entry(body, action).is_some() { actions.push(action); }
    }
    actions
}

fn measure(dir: &str, bodies: &[u32], actions_per_body: usize, span_per_action: usize, tsv: &str) {
    let sources = Sources::open(dir);
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_SPAN_SIDE).unwrap();
    let mut rife = InBetweenMaker::new(|_| {}).unwrap();
    println!("GPU: {}", gpu.adapter());
    let mut out = std::fs::File::create(tsv).unwrap();
    writeln!(out, "body\tfilter\tframes\toutliers_per_mille\tdark_share\thigh_frequency\tlag1\tpsnr_to_source\t\
        low_pass_psnr\tedge_ratio\tbc7_psnr\tbc7_zstd_bytes\tspan_frames\tspan_outliers_per_mille\tspan_high_frequency\t\
        span_low_pass_psnr\tspan_edge_ratio\trife_frames\trife_outliers_per_mille\trife_high_frequency").unwrap();
    for &body in bodies {
        let actions = actions_of(&sources, body, actions_per_body);
        let entries: Vec<_> = actions.iter().map(|&a| sources.entry(body, a).unwrap()).collect();
        for filter in Filter::ALL {
            let mut frames_rgba = Vec::new();
            let (mut total, mut fidelity) = (Speckle::default(), Fidelity::default());
            let (mut span_total, mut span_fidelity, mut span_count) = (Speckle::default(), Fidelity::default(), 0);
            let (mut rife_total, mut rife_count) = (Speckle::default(), 0);
            for animation in &entries {
                let palette = Palette::of(animation);
                let frames = source::all_frames(animation);
                let filtered: Vec<Vec<u8>> = frames.par_iter().map(|f| filter.apply(f, &palette)).collect();
                for (frame, rgba) in frames.iter().zip(&filtered) {
                    total.add(&speckle_of(rgba, frame.width, frame.height));
                    fidelity.add(&Fidelity::of(&frame.rgba, rgba, frame.width, frame.height));
                    frames_rgba.push((frame.width, frame.height, rgba.clone()));
                }
                // SPAN of frames spread over the entry, against SPAN of the same source frames.
                let step = (frames.len() / span_per_action.max(1)).max(1);
                for i in (0..frames.len()).step_by(step).take(span_per_action) {
                    let frame = &frames[i];
                    let Some((sw, sh, span)) = span_game(&mut upscaler, &gpu, frame, &filtered[i]) else { continue };
                    let (_, _, reference) = span_game(&mut upscaler, &gpu, frame, &frame.rgba).unwrap();
                    span_total.add(&speckle_of(&span, sw, sh));
                    span_fidelity.add(&Fidelity::of(&reference, &span, sw, sh));
                    span_count += 1;
                }
                // RIFE x2 of the first stored direction: its in-betweens only.
                let clip = source::direction_frames(animation, 0);
                if clip.len() >= 2 {
                    let clip_rgba: Vec<Vec<u8>> = clip.iter().map(|f| filter.apply(f, &palette)).collect();
                    let strides: Vec<StrideFrame> = clip.iter().zip(&clip_rgba).map(|(f, r)| stride_frame(f, r)).collect();
                    let sequence = rife.sequence(&strides, RIFE_FACTOR, false, &AtomicBool::new(false), |_, _| {})
                        .unwrap().unwrap();
                    for between in sequence.iter().skip(1).step_by(RIFE_FACTOR as usize) {
                        rife_total.add(&speckle_of(&between.rgba, between.width as usize, between.height as usize));
                        rife_count += 1;
                    }
                }
            }
            let (pages, places) = pack(&frames_rgba);
            let (mut bc7_squared, mut bc7_count, mut bytes) = (0.0, 0.0, 0usize);
            for (index, page) in pages.iter().enumerate() {
                let blocks = gpu.encode(page, PAGE_SIDE as u32, PAGE_SIDE as u32, GAME_QUALITY).unwrap().concat();
                bytes += zstd::bulk::compress(&blocks, ZSTD_LEVEL).unwrap().len();
                let decoded = decode_bc7_to_rgba8888(&blocks, ImageExtent::new(PAGE_SIDE as u32, PAGE_SIDE as u32)
                    .unwrap()).unwrap();
                for (i, place) in places.iter().enumerate().filter(|(_, p)| p.0 == index) {
                    let (w, h, rgba) = &frames_rgba[i];
                    let (s, c) = metrics::opaque_error(rgba, &cut(&decoded, place.1, place.2, *w, *h));
                    bc7_squared += s;
                    bc7_count += c;
                }
            }
            writeln!(out, "{body}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.4}\t{:.2}\t{:.2}\t{:.3}\t{:.2}\t{bytes}\t{span_count}\t\
                {:.3}\t{:.3}\t{:.2}\t{:.3}\t{rife_count}\t{:.3}\t{:.3}", filter.name(), frames_rgba.len(), total.per_mille(),
                total.dark_share(), total.mean_high_frequency(), total.lag_correlation(), fidelity.full_psnr(),
                fidelity.low_pass_psnr(), fidelity.edge_ratio(), psnr(bc7_squared, bc7_count), span_total.per_mille(),
                span_total.mean_high_frequency(), span_fidelity.low_pass_psnr(), span_fidelity.edge_ratio(),
                rife_total.per_mille(), rife_total.mean_high_frequency()).unwrap();
            out.flush().unwrap();
            println!("body {body} {}: {:.2} per mille, {} pages, {} bytes", filter.name(), total.per_mille(), pages.len(),
                bytes);
        }
    }
}

fn clip(dir: &str, out: &str, filter_names: &str, specs: &[String]) {
    std::fs::create_dir_all(out).unwrap();
    let filters: Vec<Filter> = filter_names.split(',').map(Filter::parse).collect();
    let sources = Sources::open(dir);
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_SPAN_SIDE).unwrap();
    let mut rife = InBetweenMaker::new(|_| {}).unwrap();
    let mut tsv = std::fs::File::create(format!("{out}/clip.tsv")).unwrap();
    writeln!(tsv, "name\tvariant\tindex\twidth\theight\tanchor_x\tanchor_y\tfile").unwrap();
    for named in specs {
        let (name, spec) = named.split_once('=').expect("name=body:action:direction");
        let (body, action, direction, _) = parse_spec(spec);
        let animation = sources.entry(body, action).expect("no entry");
        let palette = Palette::of(&animation);
        let frames = source::direction_frames(&animation, direction);
        for &filter in &filters {
            let filtered: Vec<Vec<u8>> = frames.iter().map(|f| filter.apply(f, &palette)).collect();
            let mut put = |variant: &str, index: usize, f: &SequenceFrame| {
                let file = format!("{name}_{variant}_{index:03}_{}x{}.rgba", f.width, f.height);
                std::fs::write(format!("{out}/{file}"), &f.rgba).unwrap();
                writeln!(tsv, "{name}\t{variant}\t{index}\t{}\t{}\t{}\t{}\t{file}", f.width, f.height, f.anchor_x,
                    f.anchor_y).unwrap();
            };
            for (index, (frame, rgba)) in frames.iter().zip(&filtered).enumerate() {
                let s = stride_frame(frame, rgba);
                put(filter.name(), index, &SequenceFrame { width: frame.width as u32, height: frame.height as u32,
                    anchor_x: s.anchor_x, anchor_y: s.anchor_y, rgba: rgba.clone() });
                if frame.width as u32 <= MAX_SPAN_SIDE && frame.height as u32 <= MAX_SPAN_SIDE {
                    let mut up = upscaler.upscale(&s, false).unwrap();
                    up.rgba = encode_decode(&gpu, &up.rgba, up.width, up.height);
                    put(&format!("span-{}", filter.name()), index, &up);
                }
            }
            let strides: Vec<StrideFrame> = frames.iter().zip(&filtered).map(|(f, r)| stride_frame(f, r)).collect();
            let sequence = rife.sequence(&strides, RIFE_FACTOR, false, &AtomicBool::new(false), |_, _| {}).unwrap()
                .unwrap();
            for (index, f) in sequence.iter().enumerate() {
                let mut f = f.clone();
                if index % RIFE_FACTOR as usize != 0 { f.rgba = encode_decode(&gpu, &f.rgba, f.width, f.height); }
                put(&format!("rife-{}", filter.name()), index, &f);
            }
        }
        println!("{name}: {} frames", frames.len());
    }
}

fn timing(dir: &str, bodies: &[u32], actions_per_body: usize) {
    let sources = Sources::open(dir);
    let mut work = Vec::new();
    for &body in bodies {
        for action in actions_of(&sources, body, actions_per_body) {
            let animation = sources.entry(body, action).unwrap();
            let palette = Palette::of(&animation);
            work.push((palette, source::all_frames(&animation)));
        }
    }
    let texels: usize = work.iter().flat_map(|(_, f)| f.iter().map(|f| f.width * f.height)).sum();
    for filter in Filter::ALL {
        let start = Instant::now();
        for (palette, frames) in &work {
            for frame in frames { std::hint::black_box(filter.apply(frame, palette)); }
        }
        let seconds = start.elapsed().as_secs_f64();
        println!("{}: {:.3} s for {:.2} Mpx, {:.3} s per Mpx", filter.name(), seconds, texels as f64 / MEGA,
            seconds / (texels as f64 / MEGA));
    }
}

fn bodies(list: &str) -> Vec<u32> {
    list.split(',').map(|b| b.parse().unwrap()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("survey") => survey(&args[1], args[2].parse().unwrap(), &args[3]),
        Some("palette") => palette(&args[1], args[2].parse().unwrap(), args[3].parse().unwrap()),
        Some("pair") => pair(&args[1], &args[2], args[3].parse().unwrap(), args[4].parse().unwrap()),
        Some("frames") => frames(&args[1], &args[2], &args[3], &args[4..]),
        Some("measure") => measure(&args[1], &bodies(&args[2]), args[3].parse().unwrap(), args[4].parse().unwrap(),
            &args[5]),
        Some("clip") => clip(&args[1], &args[2], &args[3], &args[4..]),
        Some("source-body") => {
            let package = MobileAnimEcPackage::load(format!("{}/mobile_anim_ec.uddp", args[1])).unwrap();
            for body in bodies(&args[2]) {
                let sources: BTreeSet<u32> = package.animations().iter().filter(|a| a.body_id == body)
                    .map(|a| a.source_body_id).collect();
                println!("server body {body}: AnimationFrame bodies {sources:?}");
            }
        }
        Some("timing") =>timing(&args[1], &bodies(&args[2]), args[3].parse().unwrap()),
        _ => eprintln!("usage: see the crate's doc comment"),
    }
}
