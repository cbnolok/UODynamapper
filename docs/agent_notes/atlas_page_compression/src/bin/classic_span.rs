//! BC7 on the Classic animation frames at 1x and after SPAN 2x (docs/agent_notes/atlas_page_compression.md, "BC7
//! frames as they show, at 1x and after SPAN"). The 1x frames stay in the frame atlas as the package stores them, so
//! with BC7 packages SPAN (and RIFE) read BC7-decoded frames; SPAN's 2x output goes into the BC7 atlas encoded
//! UltraFast (render/mobiles/bc7_encoding.rs) whatever the package. Every encode here is of a whole lossless page, as
//! the packer's, and a frame is cut from the decoded page, so it shares its edge blocks with its neighbours as in the
//! package.
//!
//! Usage:
//! - `classic_span frames <lossless uddp dir> <out dir> <name=body:action:direction:frame> ...`: each frame's crops as
//!   raw RGBA, `<name>_<variant>_<width>x<height>.rgba`: `lossless`, `ultrafast`, `veryfast` and `bc1` at 1x, and
//!   `span-lossless` and `span-ultrafast`, SPAN 2x of the lossless and the UltraFast frame, each encoded UltraFast as
//!   the game does (`-raw` before that encode).
//! - `classic_span psnr <lossless uddp dir> <pages> <frames>`: the error of each input format at 1x and after SPAN,
//!   over `frames` frames spread over every frame of `pages` pages sampled as `bc7_gpu` samples them.
//! - `classic_span rife <lossless uddp dir> <animations> <pairs>`: the RIFE in-betweens (factor 2, Classic alpha
//!   snapped, as the game makes them) of the first `pairs` frame pairs of `animations` animations spread over the
//!   package, from each input format against those from the lossless frames, raw and encoded as the game encodes them.
//! - `classic_span rife-rank <lossless uddp dir> <kernel name> <animations> <pairs> <tsv>`: the same in-betweens from
//!   the BC7 UltraFast package of the kernel this build has, two runtime fixes of it and two noise controls, each in
//!   `tsv` with its flips, flow and the error parted into flow and colour; prints the summary and the worst ones
//!   (atlas_page_compression.md, "RIFE in-betweens from the BC7 frames, looked at").
//! - `classic_span rife-dump <lossless uddp dir> <kernel name> <out dir> <#record | body:action:direction> ...`: whole
//!   clips from the lossless and the BC7 frames, with each pair's in-between and flow, for classic_rife_sheets.py.
//!
//! Every in-between anchors its two frames as the game does, on (`center_x`, `height + center_y`).
//!
//! The inputs of `psnr` and `rife`: the lossless frames, BC7 UltraFast and VeryFast, UltraFast of the page with the
//! transparent texels of its edge blocks given their nearest opaque colour first (`dilated`, classic_formats'
//! `prefill`), and BC1.
use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::MobileAnimCcPackage;
use udd_assets::animation::mobile_anim_cc::MobileAnimCcFrameRecord;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::frame_upscales::FrameUpscaler;
use uot_gpu_compute::in_betweens::InBetweenMaker;
use uot_gpu_compute::rife::{RifeModel, RifeWorkspace, RIFE_ALPHA_CHANNEL, RIFE_FLOW_CHANNELS, RIFE_IMAGE_CHANNELS,
    RIFE_SIZE_MULTIPLE};
use uot_gpu_compute::ConvUnits;
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_gpu_compute::strides::StrideFrame;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, ImageExtent};

const PEAK_SQUARED: f64 = 255.0 * 255.0;
const OPAQUE: u8 = u8::MAX;
/// The renderer's alpha cutoff (statics_draw.rs `alpha_cutoff: 0.5`) on 8-bit alpha.
const CUTOFF: u8 = 128;
const RGBA_BYTES: usize = 4;
/// The largest frame side the upscaler takes (the Classic frames' are under it).
const MAX_FRAME_SIDE: u32 = 512;
/// The input formats of the 1x frames: the lossless package's, and the encodes of a lossy package.
const INPUTS: [Input; 5] = [Input::Lossless, Input::Bc7(Bc7Quality::UltraFast), Input::Bc7(Bc7Quality::VeryFast),
    Input::Bc7Dilated(Bc7Quality::UltraFast), Input::Bc1];
/// The quality the game encodes the SPAN frames at (bc7_encoding.rs `MOBILE_BC7_QUALITY`).
const GAME_QUALITY: Bc7Quality = Bc7Quality::UltraFast;
const PERCENT: usize = 100;
const MEDIAN_PERCENT: usize = 50;
/// The low percentile of the frames' PSNR reported.
const LOW_PERCENT: usize = 10;
/// The BC7 block side.
const BLOCK_EDGE: usize = 4;
/// The Classic client's time per frame of a MUL body's animation (dynamapper's clips.rs `CLASSIC_FRAME_SECS`).
const CLASSIC_FRAME_SECS: f32 = 0.08;
/// RIFE's frames per pair: the first and one in-between, half-way (the game's 2x).
const RIFE_FACTOR: u32 = 2;

#[derive(Clone, Copy, PartialEq)]
enum Input {
    Lossless,
    Bc7(Bc7Quality),
    /// BC7 of the page dilated: its edge blocks' transparent texels given their nearest opaque colour.
    Bc7Dilated(Bc7Quality),
    Bc1,
}

impl Input {
    fn name(self) -> String {
        match self {
            Input::Lossless => "lossless".into(),
            Input::Bc7(quality) => format!("{quality:?}").to_lowercase(),
            Input::Bc7Dilated(quality) => format!("{quality:?}-dilated").to_lowercase(),
            Input::Bc1 => "bc1".into(),
        }
    }
}

/// A lossless page and its texels.
struct Page { width: u32, height: u32, rgba: Vec<u8> }

fn load_page(package: &MobileAnimCcPackage, page_index: u32) -> Page {
    let record = package.pages().iter().find(|page| page.page_index == page_index).unwrap();
    assert_eq!(record.pixel_format, PagePixelFormat::Rgba8888, "a lossless source (the RGBA8888 package)");
    let rgba = package.read_page_rgba(page_index).unwrap();
    let (width, height) = (record.used_width as usize, record.used_height as usize);
    let rgba = if rgba.len() == width * height * RGBA_BYTES { rgba } else {
        rgba.chunks_exact(record.atlas_width as usize * RGBA_BYTES).take(height)
            .flat_map(|row| row[..width * RGBA_BYTES].to_vec()).collect()
    };
    Page { width: record.used_width, height: record.used_height, rgba }
}

/// The page encoded in `input` and decoded again.
fn encode_decode(gpu: &PageEncoderGpu, rgba: &[u8], width: u32, height: u32, input: Input) -> Vec<u8> {
    let extent = ImageExtent::new(width, height).unwrap();
    match input {
        Input::Lossless => rgba.to_vec(),
        Input::Bc7(quality) => decode_bc7_to_rgba8888(&gpu.encode(rgba, width, height, quality).unwrap().concat(),
            extent).unwrap(),
        Input::Bc7Dilated(quality) => decode_bc7_to_rgba8888(&gpu.encode(&dilate(rgba, width, height), width, height,
            quality).unwrap().concat(), extent).unwrap(),
        Input::Bc1 => {
            let blocks = gpu.encode_bc1(rgba, width, height).unwrap().concat();
            decode_bc1(&blocks, width, height)
        }
    }
}

/// The page with every transparent texel of a block holding an opaque one given the colour of its nearest opaque
/// texel (4-neighbour distance over the page); its alpha stays, and the wholly transparent blocks stay as they were
/// (classic_formats' `prefill`).
fn dilate(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    let opaque = |i: usize| rgba[i * RGBA_BYTES + 3] >= CUTOFF;
    let mut nearest: Vec<u32> = vec![u32::MAX; width * height];
    let mut queue = VecDeque::new();
    for i in (0..width * height).filter(|&i| opaque(i)) {
        nearest[i] = i as u32;
        queue.push_back(i);
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % width, i / width);
        let mut visit = |j: usize| if nearest[j] == u32::MAX { nearest[j] = nearest[i]; queue.push_back(j); };
        if x > 0 { visit(i - 1); }
        if x + 1 < width { visit(i + 1); }
        if y > 0 { visit(i - width); }
        if y + 1 < height { visit(i + width); }
    }
    let blocks_wide = width.div_ceil(BLOCK_EDGE);
    let block = |i: usize| (i / width / BLOCK_EDGE) * blocks_wide + (i % width) / BLOCK_EDGE;
    let mut mixed = vec![false; blocks_wide * height.div_ceil(BLOCK_EDGE)];
    for i in (0..width * height).filter(|&i| opaque(i)) {
        mixed[block(i)] = true;
    }
    let mut dilated = rgba.to_vec();
    for i in (0..width * height).filter(|&i| !opaque(i) && mixed[block(i)] && nearest[i] != u32::MAX) {
        let from = nearest[i] as usize * RGBA_BYTES;
        dilated.copy_within(from..from + 3, i * RGBA_BYTES);
    }
    dilated
}

/// BC1 blocks (block rows top down) back to RGBA, through the CPU decoder `classic_formats` uses.
fn decode_bc1(blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; width as usize * height as usize * RGBA_BYTES];
    let image = dds::ImageViewMut::new(&mut rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8).unwrap();
    dds::decode(&mut Cursor::new(blocks), image, dds::Format::BC1_UNORM, &dds::DecodeOptions::default()).unwrap();
    rgba
}

fn cut(page_rgba: &[u8], page_width: u32, frame: &MobileAnimCcFrameRecord) -> Vec<u8> {
    let (x, y, w) = (frame.x as usize, frame.y as usize, frame.width as usize);
    (y..y + frame.height as usize).flat_map(|row| {
        let at = (row * page_width as usize + x) * RGBA_BYTES;
        page_rgba[at..at + w * RGBA_BYTES].to_vec()
    }).collect()
}

/// A frame with its anchor as the game places it: its centre x from the left and its centre y from the bottom
/// (mobile_anim_cc.rs `record_clip_in`). The RIFE in-betweens of this bench before 2026-11-23's second look took the
/// centre y from the top, which put two frames of different heights that far apart on their card.
fn stride_frame<'a>(frame: &MobileAnimCcFrameRecord, rgba: &'a [u8]) -> StrideFrame<'a> {
    StrideFrame { width: frame.width, height: frame.height, anchor_x: i32::from(frame.center_x),
        anchor_y: i32::from(frame.height) + i32::from(frame.center_y), rgba }
}

/// SPAN 2x of a frame (its whole canvas, the halo included), Classic alpha snapped.
fn span(upscaler: &mut FrameUpscaler, frame: &MobileAnimCcFrameRecord, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let out = upscaler.upscale(&stride_frame(frame, rgba), true).unwrap();
    (out.width, out.height, out.rgba)
}

/// Squared RGB error over the reference's opaque texels, their count (texels x 3), and the texels whose alpha
/// lands on the other side of the cutoff.
#[derive(Default, Clone, Copy)]
struct Error { squared: f64, count: f64, flips: usize, texels: usize }

impl Error {
    fn of(reference: &[u8], other: &[u8]) -> Self {
        let mut e = Error { texels: reference.len() / RGBA_BYTES, ..Error::default() };
        for (r, o) in reference.chunks_exact(RGBA_BYTES).zip(other.chunks_exact(RGBA_BYTES)) {
            if r[3] == OPAQUE {
                for c in 0..3 { e.squared += (f64::from(r[c]) - f64::from(o[c])).powi(2); }
                e.count += 3.0;
            }
            if (r[3] >= CUTOFF) != (o[3] >= CUTOFF) { e.flips += 1; }
        }
        e
    }

    fn add(&mut self, other: Error) {
        self.squared += other.squared;
        self.count += other.count;
        self.flips += other.flips;
        self.texels += other.texels;
    }

    fn mse(&self) -> f64 {
        self.squared / self.count.max(1.0)
    }

    fn psnr(&self) -> String {
        if self.squared == 0.0 { "lossless".into() } else { format!("{:.2} dB", 10.0 * (PEAK_SQUARED / self.mse()).log10()) }
    }
}

fn write(dir: &str, name: &str, variant: &str, width: u32, height: u32, rgba: &[u8]) {
    std::fs::write(format!("{dir}/{name}_{variant}_{width}x{height}.rgba"), rgba).unwrap();
}

/// A frame named `body:action:direction:frame`.
fn find_frame<'a>(package: &'a MobileAnimCcPackage, spec: &str) -> &'a MobileAnimCcFrameRecord {
    let fields: Vec<u32> = spec.split(':').map(|field| field.parse().unwrap()).collect();
    let animation = package.animation(fields[0] as u16, fields[1] as u16, fields[2] as u8)
        .unwrap_or_else(|| panic!("no animation {spec}"));
    &package.animation_frames(animation)[fields[3] as usize]
}

fn frames(dir: &str, out: &str, specs: &[String]) {
    std::fs::create_dir_all(out).unwrap();
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    println!("GPU: {}", gpu.adapter());
    for named in specs {
        let (name, spec) = named.split_once('=').expect("name=body:action:direction:frame");
        let frame = find_frame(&package, spec);
        let page = load_page(&package, frame.page_index);
        let mut decoded = HashMap::new();
        for input in INPUTS {
            let rgba = cut(&encode_decode(&gpu, &page.rgba, page.width, page.height, input), page.width, frame);
            write(out, name, &input.name(), u32::from(frame.width), u32::from(frame.height), &rgba);
            decoded.insert(input.name(), rgba);
        }
        let lossless = &decoded["lossless"];
        let line: Vec<String> = INPUTS[1..].iter().map(|input| format!("{} {}", input.name(),
            Error::of(lossless, &decoded[&input.name()]).psnr())).collect();
        for input in [Input::Lossless, Input::Bc7(GAME_QUALITY)] {
            let (width, height, raw) = span(&mut upscaler, frame, &decoded[&input.name()]);
            write(out, name, &format!("span-{}-raw", input.name()), width, height, &raw);
            let encoded = encode_decode(&gpu, &raw, width, height, Input::Bc7(GAME_QUALITY));
            write(out, name, &format!("span-{}", input.name()), width, height, &encoded);
        }
        println!("{name} ({spec}, {}x{} on page {}): {}", frame.width, frame.height, frame.page_index, line.join(", "));
    }
}

fn pick(total: usize, count: usize) -> Vec<usize> {
    let count = count.min(total);
    (0..count).map(|k| k * total / count).collect()
}

fn psnr(dir: &str, page_count: usize, frame_count: usize) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    println!("GPU: {}", gpu.adapter());
    let pages: Vec<u32> = pick(package.pages().len(), page_count).into_iter()
        .map(|i| package.pages()[i].page_index).collect();
    let candidates: Vec<&MobileAnimCcFrameRecord> = package.frames().iter()
        .filter(|frame| pages.contains(&frame.page_index) && frame.width > 0 && frame.height > 0
            && u32::from(frame.width.max(frame.height)) <= MAX_FRAME_SIDE).collect();
    let chosen: Vec<&MobileAnimCcFrameRecord> = pick(candidates.len(), frame_count).into_iter()
        .map(|i| candidates[i]).collect();
    // Sums per input: at 1x against the lossless frame; SPAN of it against SPAN of the lossless frame, raw and as
    // the game encodes it; and the game's SPAN frame against the one it makes from the lossless package.
    let mut at_1x = vec![Error::default(); INPUTS.len()];
    let mut span_raw = vec![Error::default(); INPUTS.len()];
    let mut span_game = vec![Error::default(); INPUTS.len()];
    let mut span_game_vs_game = vec![Error::default(); INPUTS.len()];
    // Each frame's own, for the spread over the frames.
    let mut frames_1x: Vec<Vec<Error>> = vec![Vec::new(); INPUTS.len()];
    let mut frames_game: Vec<Vec<Error>> = vec![Vec::new(); INPUTS.len()];
    for &page_index in &pages {
        let page = load_page(&package, page_index);
        let decoded: Vec<Vec<u8>> = INPUTS.iter()
            .map(|&input| encode_decode(&gpu, &page.rgba, page.width, page.height, input)).collect();
        for frame in chosen.iter().filter(|frame| frame.page_index == page_index) {
            let mut reference_raw = Vec::new();
            let mut reference_game = Vec::new();
            for (k, page_rgba) in decoded.iter().enumerate() {
                let rgba = cut(page_rgba, page.width, frame);
                let (width, height, raw) = span(&mut upscaler, frame, &rgba);
                let game = encode_decode(&gpu, &raw, width, height, Input::Bc7(GAME_QUALITY));
                if k == 0 {
                    reference_raw = raw.clone();
                    reference_game = game.clone();
                    at_1x[k].add(Error::of(&rgba, &rgba));
                } else {
                    let error = Error::of(&cut(&decoded[0], page.width, frame), &rgba);
                    at_1x[k].add(error);
                    frames_1x[k].push(error);
                }
                span_raw[k].add(Error::of(&reference_raw, &raw));
                let game_error = Error::of(&reference_raw, &game);
                span_game[k].add(game_error);
                frames_game[k].push(game_error);
                span_game_vs_game[k].add(Error::of(&reference_game, &game));
            }
        }
    }
    println!("{} frames from {} pages", chosen.len(), pages.len());
    println!("| 1x input | 1x vs lossless | flips | SPAN 2x vs SPAN of lossless | flips | x MSE of 1x | \
        game's 2x (BC7 {GAME_QUALITY:?}) vs SPAN of lossless | game's 2x vs the lossless package's | flips |");
    for (k, input) in INPUTS.iter().enumerate() {
        let ratio = if at_1x[k].squared == 0.0 { "-".to_string() } else {
            format!("{:.2}", span_raw[k].mse() / at_1x[k].mse())
        };
        println!("| {} | {} | {} | {} | {} | {ratio} | {} | {} | {} |", input.name(), at_1x[k].psnr(), at_1x[k].flips,
            span_raw[k].psnr(), span_raw[k].flips, span_game[k].psnr(), span_game_vs_game[k].psnr(),
            span_game_vs_game[k].flips);
    }
    println!("| Per frame | 1x vs lossless: median, p10, worst | game's 2x vs SPAN of lossless: median, p10, worst |");
    for (k, input) in INPUTS.iter().enumerate() {
        println!("| {} | {} | {} |", input.name(), spread(&frames_1x[k]), spread(&frames_game[k]));
    }
}

/// The median, tenth percentile and lowest of the frames' PSNR (frames without opaque texels left out).
fn spread(frames: &[Error]) -> String {
    let mut values: Vec<f64> = frames.iter().filter(|e| e.count > 0.0)
        .map(|e| if e.squared == 0.0 { f64::INFINITY } else { 10.0 * (PEAK_SQUARED / e.mse()).log10() }).collect();
    if values.is_empty() {
        return "-".into();
    }
    values.sort_by(f64::total_cmp);
    let at = |q: usize| values[values.len() * q / PERCENT];
    format!("{:.1}, {:.1}, {:.1} dB", at(MEDIAN_PERCENT), at(LOW_PERCENT), values[0])
}

/// The RIFE in-betweens of animations' first frame pairs from each input against those from the lossless frames.
fn rife(dir: &str, animation_count: usize, pair_count: usize) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut maker = InBetweenMaker::new(|_| {}).unwrap();
    println!("GPU: {} (RIFE on {})", gpu.adapter(), maker.adapter());
    let candidates: Vec<usize> = package.animations().iter().enumerate()
        .filter(|(_, animation)| animation.frame_count >= 2).map(|(index, _)| index).collect();
    let chosen: Vec<usize> = pick(candidates.len(), animation_count).into_iter().map(|i| candidates[i]).collect();
    // Each page's width and decoded texels per input, made as the animations ask for them.
    let mut pages: HashMap<u32, (u32, Vec<Vec<u8>>)> = HashMap::new();
    let cancel = AtomicBool::new(false);
    let mut raw = vec![Error::default(); INPUTS.len()];
    let mut game = vec![Error::default(); INPUTS.len()];
    let mut game_vs_game = vec![Error::default(); INPUTS.len()];
    let mut frames_game: Vec<Vec<Error>> = vec![Vec::new(); INPUTS.len()];
    let mut in_betweens = 0;
    for &animation_index in &chosen {
        let frames = package.animation_frames(&package.animations()[animation_index]);
        let frames = &frames[..frames.len().min(pair_count + 1)];
        if frames.iter().any(|frame| frame.width == 0 || frame.height == 0) {
            continue;
        }
        for frame in frames {
            pages.entry(frame.page_index).or_insert_with(|| {
                let page = load_page(&package, frame.page_index);
                (page.width, INPUTS.iter().map(|&input| encode_decode(&gpu, &page.rgba, page.width, page.height, input))
                    .collect())
            });
        }
        // The in-betweens of the pairs (0, 1), (1, 2), ..., without the one from the last frame back to the first.
        let mut reference: Vec<(u32, u32, Vec<u8>, Vec<u8>)> = Vec::new();
        for k in 0..INPUTS.len() {
            let cut_frames: Vec<Vec<u8>> = frames.iter().map(|frame| {
                let (page_width, decoded) = &pages[&frame.page_index];
                cut(&decoded[k], *page_width, frame)
            }).collect();
            let strides: Vec<StrideFrame> = frames.iter().zip(&cut_frames)
                .map(|(frame, rgba)| stride_frame(frame, rgba)).collect();
            let sequence = maker.sequence(&strides, RIFE_FACTOR, true, &cancel, |_, _| {}).unwrap().unwrap();
            let made: Vec<_> = sequence.iter().skip(1).step_by(RIFE_FACTOR as usize).take(frames.len() - 1).collect();
            for (pair, in_between) in made.iter().enumerate() {
                let encoded = encode_decode(&gpu, &in_between.rgba, in_between.width, in_between.height,
                    Input::Bc7(GAME_QUALITY));
                if k == 0 {
                    game[k].add(Error::of(&in_between.rgba, &encoded));
                    reference.push((in_between.width, in_between.height, in_between.rgba.clone(), encoded));
                    in_betweens += 1;
                    continue;
                }
                let (width, height, reference_raw, reference_game) = &reference[pair];
                assert_eq!((*width, *height), (in_between.width, in_between.height), "the same card");
                raw[k].add(Error::of(reference_raw, &in_between.rgba));
                let error = Error::of(reference_raw, &encoded);
                game[k].add(error);
                frames_game[k].push(error);
                game_vs_game[k].add(Error::of(reference_game, &encoded));
            }
        }
    }
    println!("{in_betweens} in-betweens of {} animations, {} pages; the game's encode of those from lossless: {}",
        chosen.len(), pages.len(), game[0].psnr());
    println!("| 1x input | RIFE in-between vs from lossless | flips | game's (BC7 {GAME_QUALITY:?}) vs from lossless | \
        game's vs the lossless package's | flips | game's per in-between: median, p10, worst |");
    for (k, input) in INPUTS.iter().enumerate().skip(1) {
        println!("| {} | {} | {} | {} | {} | {} | {} |", input.name(), raw[k].psnr(), raw[k].flips, game[k].psnr(),
            game_vs_game[k].psnr(), game_vs_game[k].flips, spread(&frames_game[k]));
    }
}

/// The inputs of `rife-rank` and `rife-dump`: the lossless frames; the package's BC7 UltraFast, of the kernel this
/// build has (`<kernel>` names it); two runtime fixes of those frames before RIFE, which cost nothing on disk: their
/// alpha snapped at the cutoff, and their RGB rounded to the 1555 colours the Classic frames hold; and, as controls of
/// how far RIFE follows a small change of its input, the lossless frames with uniform noise on their opaque texels'
/// RGB.
#[derive(Clone, Copy, PartialEq)]
enum RifeInput {
    Lossless,
    Package,
    PackageSnapped,
    PackageRounded,
    Noise(u8),
}

/// ±2 of 255 on each channel, less than a 1555 step (8).
const SMALL_NOISE: u8 = 2;
/// ±10 of 255: an RMS of 10/√3 = 5.8, 32.9 dB, as large as the UltraFast frames' error (33.1 dB) but without its
/// structure.
const BC7_LIKE_NOISE: u8 = 10;
const RANK_INPUTS: [RifeInput; 6] = [RifeInput::Lossless, RifeInput::Package, RifeInput::PackageSnapped,
    RifeInput::PackageRounded, RifeInput::Noise(SMALL_NOISE), RifeInput::Noise(BC7_LIKE_NOISE)];
/// The Classic frames' channels are 5 bits widened as `c << 3` (uocf `rgba8888_word_from_bgra5551`).
const CLASSIC_CHANNEL_SHIFT: u32 = 3;
const CLASSIC_CHANNEL_MAX: u32 = (1 << 5) - 1;
const DUMP_INPUTS: [RifeInput; 2] = [RifeInput::Lossless, RifeInput::Package];
/// The pages kept decoded at once (each with its lossless and BC7 texels), and the RIFE workspaces kept.
const PAGE_CACHE: usize = 6;
/// The grey the network sees behind the frames (in_betweens.rs `BACKDROP_GREY`, frames.wgsl's `BACKGROUND_GREY`).
const BACKDROP_GREY: f32 = 0.5;
/// The coverage from which a Classic in-between is opaque (frames.wgsl's `SNAP_ALPHA`).
const CLASSIC_ALPHA_SNAP: f32 = 0.5;
/// RIFE's image planes: RGB over grey, premultiplied RGB, alpha.
const COLOR_CHANNELS: usize = 3;
const PREMULTIPLIED_CHANNEL: u32 = 3;
const MERGED_READ_CHANNELS: u32 = RIFE_IMAGE_CHANNELS - PREMULTIPLIED_CHANNEL;
const MAX_CHANNEL: f32 = 255.0;
const F32_BYTES: u64 = 4;
/// The in-betweens listed worst first.
const WORST_LISTED: usize = 15;
/// The second list of the worst in-betweens leaves out those with fewer opaque texels (a 20x20 sprite): on a few
/// texels one flip makes the PSNR.
const MIN_LISTED_OPAQUE: usize = 400;
/// A flow vector counts as moved past this many pixels from the reference's.
const FLOW_MOVED_PIXELS: f32 = 1.0;

impl RifeInput {
    fn name(self, kernel: &str) -> String {
        match self {
            RifeInput::Lossless => "lossless".into(),
            RifeInput::Package => kernel.into(),
            RifeInput::PackageSnapped => format!("{kernel}-snapped"),
            RifeInput::PackageRounded => format!("{kernel}-rounded"),
            RifeInput::Noise(amplitude) => format!("noise{amplitude}"),
        }
    }

    /// The frame this input gives RIFE, from the lossless and the package's frame; `seed` names the frame.
    fn frame(self, lossless: &[u8], package: &[u8], seed: u64) -> Vec<u8> {
        match self {
            RifeInput::Lossless => lossless.to_vec(),
            RifeInput::Package => package.to_vec(),
            RifeInput::PackageSnapped => package.chunks_exact(RGBA_BYTES).flat_map(|texel| {
                if texel[3] >= CUTOFF { [texel[0], texel[1], texel[2], OPAQUE] } else { [0; RGBA_BYTES] }
            }).collect(),
            RifeInput::PackageRounded => package.chunks_exact(RGBA_BYTES).flat_map(|texel| {
                let round = |value: u8| {
                    let step = 1u32 << CLASSIC_CHANNEL_SHIFT;
                    (((u32::from(value) + step / 2) >> CLASSIC_CHANNEL_SHIFT).min(CLASSIC_CHANNEL_MAX)
                        << CLASSIC_CHANNEL_SHIFT) as u8
                };
                [round(texel[0]), round(texel[1]), round(texel[2]), texel[3]]
            }).collect(),
            RifeInput::Noise(amplitude) => {
                let span = 2 * u64::from(amplitude) + 1;
                let mut noisy = lossless.to_vec();
                for (i, texel) in noisy.chunks_exact_mut(RGBA_BYTES).enumerate().filter(|(_, t)| t[3] == OPAQUE) {
                    for c in 0..COLOR_CHANNELS {
                        let offset = (splitmix(seed ^ (seed << 32) ^ (i * COLOR_CHANNELS + c) as u64) % span) as i32
                            - i32::from(amplitude);
                        texel[c] = (i32::from(texel[c]) + offset).clamp(0, i32::from(OPAQUE)) as u8;
                    }
                }
                noisy
            }
        }
    }
}

fn splitmix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The card of a pair: both frames with their anchors on one point (in_betweens.rs `PairCard`, the game's
/// `union_placement`).
#[derive(Clone, Copy, PartialEq, Debug)]
struct PairCard { width: u32, height: u32, anchor_x: i32, anchor_y: i32, first: (u32, u32), next: (u32, u32) }

impl PairCard {
    fn of(first: &StrideFrame, next: &StrideFrame) -> Self {
        let left = first.anchor_x.max(next.anchor_x);
        let right = (i32::from(first.width) - first.anchor_x).max(i32::from(next.width) - next.anchor_x);
        let up = first.anchor_y.max(next.anchor_y);
        let down = (i32::from(first.height) - first.anchor_y).max(i32::from(next.height) - next.anchor_y);
        let place = |frame: &StrideFrame| ((left - frame.anchor_x) as u32, (up - frame.anchor_y) as u32);
        Self { width: (left + right) as u32, height: (up + down) as u32, anchor_x: left, anchor_y: up,
            first: place(first), next: place(next) }
    }
}

/// An in-between on its card: straight RGBA with Classic alpha snapped, its coverage before the snap, and RIFE's last
/// flow (t->0 xy, t->1 xy, in pixels), each plane card-sized.
struct PairOut { card: PairCard, rgba: Vec<u8>, coverage: Vec<f32>, flow: Vec<f32> }

/// RIFE on a device of the bench's own, one pair at a time, as in_betweens.rs `InBetweenMaker` makes them (the same
/// model, packing and store), reading its flow back too.
struct Rife {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: String,
    model: RifeModel,
    workspaces: Vec<((u32, u32), RifeWorkspace)>,
}

impl Rife {
    /// The device as uot-gpu-compute's `own_device_and_units` makes it.
    fn new() -> Self {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
            }).await.unwrap();
            let info = adapter.get_info();
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: adapter.features() & ConvUnits::MATRIX_FEATURES,
                required_limits: adapter.limits(),
                // SAFETY: as own_device_and_units: the networks use the cooperative matrices only where
                // ConvUnits::of finds them supported.
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                ..Default::default()
            }).await.unwrap();
            let units = ConvUnits::of(&adapter, &device);
            let model = RifeModel::new(&device, units).unwrap();
            Rife { device, queue, adapter: format!("{} ({:?}, {units:?})", info.name, info.backend), model,
                workspaces: Vec::new() }
        })
    }

    fn workspace(&mut self, width: u32, height: u32) -> usize {
        let size = (width.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE,
            height.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE);
        if let Some(index) = self.workspaces.iter().position(|(kept, _)| *kept == size) {
            return index;
        }
        if self.workspaces.len() >= PAGE_CACHE {
            self.workspaces.remove(0);
        }
        self.workspaces.push((size, RifeWorkspace::new(&self.model, &self.device, size.0, size.1).unwrap()));
        self.workspaces.len() - 1
    }

    /// The in-between half-way from `first` to `next`.
    fn pair(&mut self, first: &StrideFrame, next: &StrideFrame) -> PairOut {
        self.pair_blending(first, next, None)
    }

    /// The in-between half-way from `first` to `next`; with `blended`, the blend warps and mixes those two frames
    /// instead, with the flow and mask the network found for `first` and `next` (same sizes and anchors): it parts
    /// what a different flow does from what different colours do.
    fn pair_blending(&mut self, first: &StrideFrame, next: &StrideFrame,
        blended: Option<(&StrideFrame, &StrideFrame)>) -> PairOut {
        let card = PairCard::of(first, next);
        let index = self.workspace(card.width, card.height);
        let workspace = &self.workspaces[index].1;
        let (width, height) = workspace.size();
        self.queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&pack_pair(first, next, &card, width,
            height)));
        workspace.set_timestep(&self.queue, 0, 1.0 / RIFE_FACTOR as f32);
        let plane_bytes = u64::from(width * height) * F32_BYTES;
        let merged_bytes = u64::from(MERGED_READ_CHANNELS) * plane_bytes;
        let flow_bytes = u64::from(RIFE_FLOW_CHANNELS) * plane_bytes;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("in-between and flow"), size: merged_bytes + flow_bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            workspace.encode().iter().chain(workspace.interpolate(0)).for_each(|dispatch| dispatch.record(&mut pass));
        }
        if let Some((first, next)) = blended {
            assert_eq!(PairCard::of(first, next), card, "frames of the same sizes and anchors");
            self.queue.submit([encoder.finish()]);
            // Written ahead of the next submission, after the one above.
            self.queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&pack_pair(first, next, &card, width,
                height)));
            encoder = self.device.create_command_encoder(&Default::default());
            let mut pass = encoder.begin_compute_pass(&Default::default());
            workspace.merge().iter().for_each(|dispatch| dispatch.record(&mut pass));
        }
        encoder.copy_buffer_to_buffer(workspace.merged(), u64::from(PREMULTIPLIED_CHANNEL) * plane_bytes, &readback,
            0, merged_bytes);
        encoder.copy_buffer_to_buffer(workspace.state(), 0, &readback, merged_bytes, flow_bytes);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        let values: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
        readback.unmap();
        let plane = (width * height) as usize;
        let (merged, flow) = values.split_at(MERGED_READ_CHANNELS as usize * plane);
        let flow: Vec<f32> = (0..RIFE_FLOW_CHANNELS as usize).flat_map(|p| (0..card.height as usize).flat_map(move |y| {
            let at = p * plane + y * width as usize;
            flow[at..at + card.width as usize].to_vec()
        })).collect();
        let (rgba, coverage) = unpack_card(merged, width, &card);
        PairOut { card, rgba, coverage, flow }
    }
}

/// The network's input for one pair (in_betweens.rs `pack_pair`): each frame's RGB over grey, premultiplied RGB and
/// alpha at its place on the card; grey and transparent elsewhere.
fn pack_pair(first: &StrideFrame, next: &StrideFrame, card: &PairCard, width: u32, height: u32) -> Vec<f32> {
    let plane = (width * height) as usize;
    let image_floats = RIFE_IMAGE_CHANNELS as usize * plane;
    let mut images = vec![0f32; 2 * image_floats];
    let placed = [(first, card.first), (next, card.next)];
    for (image, (frame, (x0, y0))) in images.chunks_exact_mut(image_floats).zip(placed) {
        image[..COLOR_CHANNELS * plane].fill(BACKDROP_GREY);
        let frame_width = usize::from(frame.width);
        for y in 0..usize::from(frame.height) {
            for x in 0..frame_width {
                let texel = &frame.rgba[(y * frame_width + x) * RGBA_BYTES..][..RGBA_BYTES];
                let alpha = f32::from(texel[COLOR_CHANNELS]) / MAX_CHANNEL;
                let at = (y0 as usize + y) * width as usize + x0 as usize + x;
                for channel in 0..COLOR_CHANNELS {
                    let premultiplied = f32::from(texel[channel]) / MAX_CHANNEL * alpha;
                    image[channel * plane + at] = premultiplied + BACKDROP_GREY * (1.0 - alpha);
                    image[(PREMULTIPLIED_CHANNEL as usize + channel) * plane + at] = premultiplied;
                }
                image[RIFE_ALPHA_CHANNEL as usize * plane + at] = alpha;
            }
        }
    }
    images
}

/// The in-between's card from its premultiplied RGB and alpha planes, `width` wide, Classic alpha snapped
/// (in_betweens.rs `unpack_card`), and its coverage.
fn unpack_card(planes: &[f32], width: u32, card: &PairCard) -> (Vec<u8>, Vec<f32>) {
    let plane = planes.len() / MERGED_READ_CHANNELS as usize;
    let mut rgba = vec![0u8; (card.width * card.height) as usize * RGBA_BYTES];
    let mut coverages = vec![0f32; (card.width * card.height) as usize];
    let to_byte = |value: f32| (value.clamp(0.0, 1.0) * MAX_CHANNEL).round() as u8;
    for y in 0..card.height as usize {
        for x in 0..card.width as usize {
            let at = y * width as usize + x;
            let coverage = planes[COLOR_CHANNELS * plane + at].clamp(0.0, 1.0);
            coverages[y * card.width as usize + x] = coverage;
            let texel = &mut rgba[(y * card.width as usize + x) * RGBA_BYTES..][..RGBA_BYTES];
            if coverage >= CLASSIC_ALPHA_SNAP {
                for channel in 0..COLOR_CHANNELS {
                    texel[channel] = to_byte(planes[channel * plane + at] / coverage);
                }
                texel[COLOR_CHANNELS] = OPAQUE;
            }
        }
    }
    (rgba, coverages)
}

/// Decoded pages, lossless and of the package's BC7 UltraFast, the last [`PAGE_CACHE`] asked for.
struct Pages<'a> {
    package: &'a MobileAnimCcPackage,
    gpu: &'a PageEncoderGpu,
    kept: VecDeque<(u32, u32, Vec<u8>, Vec<u8>)>,
}

impl Pages<'_> {
    /// The frame cut from the lossless page and from its BC7 UltraFast.
    fn frame(&mut self, frame: &MobileAnimCcFrameRecord) -> (Vec<u8>, Vec<u8>) {
        if !self.kept.iter().any(|(index, ..)| *index == frame.page_index) {
            let page = load_page(self.package, frame.page_index);
            let bc7 = encode_decode(self.gpu, &page.rgba, page.width, page.height, Input::Bc7(GAME_QUALITY));
            if self.kept.len() >= PAGE_CACHE {
                self.kept.pop_front();
            }
            self.kept.push_back((frame.page_index, page.width, page.rgba, bc7));
        }
        let (_, width, lossless, bc7) = self.kept.iter().find(|(index, ..)| *index == frame.page_index).unwrap();
        (cut(lossless, *width, frame), cut(bc7, *width, frame))
    }
}

/// How an in-between differs from the reference's, on the same card: the squared RGB error on the texels opaque in
/// both, and on the reference's opaque texels that came out transparent (`lost`, which show the background and count
/// their colour as black, as [`Error::of`] counts them); the texels that came out opaque where the reference is
/// transparent (`gained`); the flips on the reference's silhouette (within one texel of a texel of the other side)
/// and away from it; and the reference's opaque texels.
#[derive(Default, Clone, Copy)]
struct Diff { colour_squared: f64, colour_texels: usize, lost_squared: f64, lost: usize, gained: usize,
    edge_flips: usize, far_flips: usize, opaque: usize }

impl Diff {
    fn of(reference: &[u8], other: &[u8], width: u32) -> Self {
        let width = width as usize;
        let height = reference.len() / RGBA_BYTES / width;
        let opaque = |rgba: &[u8], x: usize, y: usize| rgba[(y * width + x) * RGBA_BYTES + 3] >= CUTOFF;
        let mut d = Diff::default();
        for y in 0..height {
            for x in 0..width {
                let at = (y * width + x) * RGBA_BYTES;
                let (r, o) = (&reference[at..at + RGBA_BYTES], &other[at..at + RGBA_BYTES]);
                let squared: f64 = (0..COLOR_CHANNELS).map(|c| (f64::from(r[c]) - f64::from(o[c])).powi(2)).sum();
                let (r_opaque, o_opaque) = (opaque(reference, x, y), opaque(other, x, y));
                d.opaque += usize::from(r_opaque);
                match (r_opaque, o_opaque) {
                    (true, true) => { d.colour_squared += squared; d.colour_texels += 1; }
                    (true, false) => { d.lost_squared += squared; d.lost += 1; }
                    (false, true) => d.gained += 1,
                    (false, false) => {}
                }
                if r_opaque != o_opaque {
                    let on_edge = (y.saturating_sub(1)..(y + 2).min(height)).any(|ny| (x.saturating_sub(1)
                        ..(x + 2).min(width)).any(|nx| opaque(reference, nx, ny) != r_opaque));
                    if on_edge { d.edge_flips += 1 } else { d.far_flips += 1 }
                }
            }
        }
        d
    }

    fn add(&mut self, other: Diff) {
        self.colour_squared += other.colour_squared;
        self.colour_texels += other.colour_texels;
        self.lost_squared += other.lost_squared;
        self.lost += other.lost;
        self.gained += other.gained;
        self.edge_flips += other.edge_flips;
        self.far_flips += other.far_flips;
        self.opaque += other.opaque;
    }

    fn flips(&self) -> usize {
        self.lost + self.gained
    }

    fn db(squared: f64, samples: f64) -> f64 {
        if squared == 0.0 { f64::INFINITY } else { 10.0 * (PEAK_SQUARED * samples.max(1.0) / squared).log10() }
    }

    /// PSNR over the reference's opaque texels, as [`Error::psnr`].
    fn psnr(&self) -> f64 {
        Self::db(self.colour_squared + self.lost_squared, (COLOR_CHANNELS * self.opaque) as f64)
    }

    /// PSNR of the texels opaque in both alone.
    fn colour_psnr(&self) -> f64 {
        Self::db(self.colour_squared, (COLOR_CHANNELS * self.colour_texels) as f64)
    }

    /// The share of the squared error on the lost texels.
    fn lost_share(&self) -> f64 {
        self.lost_squared / (self.colour_squared + self.lost_squared).max(f64::MIN_POSITIVE)
    }
}

/// How far an in-between's flow lies from the reference's, over the texels either has at least half covered: the mean
/// and largest distance of a flow vector (t->0 and t->1 both), the share of texels with a vector moved past
/// [`FLOW_MOVED_PIXELS`], and the reference's mean flow length.
fn flow_diff(reference: &PairOut, other: &PairOut) -> (f32, f32, f32, f32) {
    let plane = (reference.card.width * reference.card.height) as usize;
    let (mut sum, mut max, mut moved, mut length, mut count) = (0f32, 0f32, 0usize, 0f32, 0usize);
    for i in (0..plane).filter(|&i| reference.coverage[i] >= CLASSIC_ALPHA_SNAP
        || other.coverage[i] >= CLASSIC_ALPHA_SNAP) {
        let mut texel_max = 0f32;
        for vector in 0..2 {
            let (x, y) = ((2 * vector) * plane + i, (2 * vector + 1) * plane + i);
            let distance = (reference.flow[x] - other.flow[x]).hypot(reference.flow[y] - other.flow[y]);
            sum += distance;
            texel_max = texel_max.max(distance);
            length += reference.flow[x].hypot(reference.flow[y]);
        }
        max = max.max(texel_max);
        moved += usize::from(texel_max > FLOW_MOVED_PIXELS);
        count += 1;
    }
    let vectors = (2 * count.max(1)) as f32;
    (sum / vectors, max, moved as f32 / count.max(1) as f32, length / vectors)
}

/// The animations with two frames or more, none empty or past the largest side, `count` of them spread over the
/// package.
fn rife_animations(package: &MobileAnimCcPackage, count: usize) -> Vec<usize> {
    let candidates: Vec<usize> = package.animations().iter().enumerate()
        .filter(|(_, animation)| animation.frame_count >= 2
            && package.animation_frames(animation).iter().all(|frame| frame.width > 0 && frame.height > 0
                && u32::from(frame.width.max(frame.height)) <= MAX_FRAME_SIDE))
        .map(|(index, _)| index).collect();
    pick(candidates.len(), count).into_iter().map(|i| candidates[i]).collect()
}

/// One in-between of one input, as `rife-rank` lists it: against the one from lossless, raw and after the game's
/// encode; the flow's distance ([`flow_diff`]); the two input frames' error against lossless and their ghosts. For the
/// package's frames, the raw in-between's error parted: the lossless frames blended with this input's flow and mask
/// (`flow_only`), and this input's frames with the lossless frames' flow and mask (`colour_only`).
struct Ranked { animation: usize, pair: usize, input: usize, raw: Diff, game: Diff, flow: (f32, f32, f32, f32),
    inputs_1x: Error, ghosts: usize, sizes: [(u16, u16); 2], card: (u32, u32), flow_only: Diff, colour_only: Diff }

/// The in-betweens of the first `pair_count` pairs (the last frame's back to the first included when the clip is that
/// short) of `animation_count` animations, from each of [`RANK_INPUTS`] against those from lossless, each listed in
/// `tsv` with what it differs by, and the worst printed.
fn rife_rank(dir: &str, kernel: &str, animation_count: usize, pair_count: usize, tsv: &str) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut rife = Rife::new();
    println!("GPU: {} (RIFE on {})", gpu.adapter(), rife.adapter);
    let chosen = rife_animations(&package, animation_count);
    let mut pages = Pages { package: &package, gpu: &gpu, kept: VecDeque::new() };
    let mut checked = false;
    let mut ranked: Vec<Ranked> = Vec::new();
    let mut reference_game = Diff::default();
    for &animation_index in &chosen {
        let frames = package.animation_frames(&package.animations()[animation_index]);
        let cut: Vec<(Vec<u8>, Vec<u8>)> = frames.iter().map(|frame| pages.frame(frame)).collect();
        for pair in 0..pair_count.min(frames.len()) {
            let (a, b) = (pair, (pair + 1) % frames.len());
            let mut reference: Option<PairOut> = None;
            for (k, input) in RANK_INPUTS.iter().enumerate() {
                let made: Vec<Vec<u8>> = [a, b].iter().map(|&f| input.frame(&cut[f].0, &cut[f].1,
                    (animation_index as u64) << 16 | f as u64)).collect();
                let first = stride_frame(&frames[a], &made[0]);
                let next = stride_frame(&frames[b], &made[1]);
                let out = rife.pair(&first, &next);
                if !checked {
                    // The bench's RIFE gives the frames uot-gpu-compute's InBetweenMaker gives.
                    let mut maker = InBetweenMaker::new(|_| {}).unwrap();
                    let sequence = maker.sequence(&[first, next], RIFE_FACTOR, true, &AtomicBool::new(false),
                        |_, _| {}).unwrap().unwrap();
                    assert!(sequence[1].rgba == out.rgba, "the bench's in-between differs from InBetweenMaker's");
                    checked = true;
                }
                let game = encode_decode(&gpu, &out.rgba, out.card.width, out.card.height, Input::Bc7(GAME_QUALITY));
                let Some(reference) = &reference else {
                    reference_game.add(Diff::of(&out.rgba, &game, out.card.width));
                    reference = Some(out);
                    continue;
                };
                assert_eq!(reference.card, out.card, "the same card");
                let (mut flow_only, mut colour_only) = (Diff::default(), Diff::default());
                if *input == RifeInput::Package {
                    let lossless = (stride_frame(&frames[a], &cut[a].0), stride_frame(&frames[b], &cut[b].0));
                    let width = out.card.width;
                    flow_only = Diff::of(&reference.rgba, &rife.pair_blending(&first, &next,
                        Some((&lossless.0, &lossless.1))).rgba, width);
                    colour_only = Diff::of(&reference.rgba, &rife.pair_blending(&lossless.0, &lossless.1,
                        Some((&first, &next))).rgba, width);
                }
                let mut inputs_1x = Error::default();
                let mut ghosts = 0;
                for (&f, other) in [a, b].iter().zip(&made) {
                    let lossless = &cut[f].0;
                    inputs_1x.add(Error::of(lossless, other));
                    ghosts += lossless.chunks_exact(RGBA_BYTES).zip(other.chunks_exact(RGBA_BYTES))
                        .filter(|(l, o)| l[3] == 0 && o[3] > 0).count();
                }
                ranked.push(Ranked { animation: animation_index, pair, input: k,
                    raw: Diff::of(&reference.rgba, &out.rgba, out.card.width),
                    game: Diff::of(&reference.rgba, &game, out.card.width), flow: flow_diff(reference, &out),
                    inputs_1x, ghosts,
                    sizes: [(frames[a].width, frames[a].height), (frames[b].width, frames[b].height)],
                    card: (out.card.width, out.card.height), flow_only, colour_only });
            }
        }
    }
    let mut file = String::from("record\tfile\tbody\taction\tdirection\tpair\tframes\tinput\tgame_db\traw_db\t\
        game_colour_db\tflips\tlost\tgained\tedge_flips\tfar_flips\tlost_share\topaque\tflow_mean\tflow_max\t\
        flow_moved\tflow_length\tinputs_db\tinput_ghosts\tsize_a\tsize_b\tcard\tflow_only_db\tcolour_only_db\n");
    for r in &ranked {
        let animation = &package.animations()[r.animation];
        file += &format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{}\t{:.3}\t\
            {:.2}\t{:.3}\t{:.2}\t{:.2}\t{}\t{}x{}\t{}x{}\t{}x{}\t{:.2}\t{:.2}\n", r.animation, animation.file_index,
            animation.body_id, animation.action_id, animation.direction, r.pair,
            animation.frame_count, RANK_INPUTS[r.input].name(kernel), r.game.psnr(), r.raw.psnr(), r.game.colour_psnr(),
            r.game.flips(), r.game.lost, r.game.gained, r.game.edge_flips, r.game.far_flips, r.game.lost_share(),
            r.game.opaque, r.flow.0, r.flow.1, r.flow.2, r.flow.3, Diff::db(r.inputs_1x.squared, r.inputs_1x.count),
            r.ghosts, r.sizes[0].0, r.sizes[0].1, r.sizes[1].0, r.sizes[1].1, r.card.0, r.card.1, r.flow_only.psnr(),
            r.colour_only.psnr());
    }
    std::fs::write(tsv, file).unwrap();
    let count = ranked.len() / (RANK_INPUTS.len() - 1);
    println!("{count} in-betweens of {} animations; the game's encode of those from lossless: {:.2} dB",
        chosen.len(), reference_game.psnr());
    println!("| Input | in-between vs from lossless | game's vs from lossless | flips (on the edge, away) | \
        error on lost texels | game's per in-between: median, p10, worst | flow moved > {FLOW_MOVED_PIXELS} px |");
    for (k, input) in RANK_INPUTS.iter().enumerate().skip(1) {
        let rows: Vec<&Ranked> = ranked.iter().filter(|r| r.input == k).collect();
        let (mut raw, mut game) = (Diff::default(), Diff::default());
        for r in &rows {
            raw.add(r.raw);
            game.add(r.game);
        }
        let mut values: Vec<f64> = rows.iter().map(|r| r.game.psnr()).collect();
        values.sort_by(f64::total_cmp);
        let at = |q: usize| values[values.len() * q / PERCENT];
        let moved = rows.iter().map(|r| r.flow.2).sum::<f32>() / rows.len() as f32;
        println!("| {} | {:.2} dB | {:.2} dB | {} ({}, {}) | {:.0} % | {:.1}, {:.1}, {:.1} dB | {:.1} % |",
            input.name(kernel), raw.psnr(), game.psnr(), game.flips(), game.edge_flips, game.far_flips,
            PERCENT as f64 * game.lost_share(), at(MEDIAN_PERCENT), at(LOW_PERCENT), values[0],
            PERCENT as f32 * moved);
    }
    let mut worst: Vec<&Ranked> = ranked.iter().filter(|r| RANK_INPUTS[r.input] == RifeInput::Package).collect();
    let (mut flow_only, mut colour_only) = (Diff::default(), Diff::default());
    for r in &worst {
        flow_only.add(r.flow_only);
        colour_only.add(r.colour_only);
    }
    println!("{kernel}'s in-betweens parted: the lossless frames with its flow and mask {:.2} dB, its frames with the \
        lossless frames' flow and mask {:.2} dB", flow_only.psnr(), colour_only.psnr());
    worst.sort_by(|a, b| a.game.psnr().total_cmp(&b.game.psnr()));
    for min_opaque in [0, MIN_LISTED_OPAQUE] {
        println!("Worst {WORST_LISTED} from {kernel} of {min_opaque} opaque texels or more:");
        println!("| record (body:action:direction) pair | frames | game's | colour alone | flips (edge, away) | \
            lost share | flow mean, max, moved | flow length | flow only | colour only | inputs 1x | ghosts | sizes | \
            card | opaque |");
        for r in worst.iter().filter(|r| r.game.opaque >= min_opaque).take(WORST_LISTED) {
            let animation = &package.animations()[r.animation];
            println!("| #{} ({}:{}:{}) {} | {} | {:.1} dB | {:.1} dB | {} ({}, {}) | {:.0} % | {:.2}, {:.1}, {:.0} % | \
                {:.1} | {:.1} dB | {:.1} dB | {:.1} dB | {} | {}x{}, {}x{} | {}x{} | {} |", r.animation,
                animation.body_id, animation.action_id, animation.direction, r.pair, animation.frame_count,
                r.game.psnr(), r.game.colour_psnr(), r.game.flips(),
                r.game.edge_flips, r.game.far_flips, PERCENT as f64 * r.game.lost_share(), r.flow.0, r.flow.1,
                PERCENT as f32 * r.flow.2, r.flow.3, r.flow_only.psnr(), r.colour_only.psnr(),
                Diff::db(r.inputs_1x.squared, r.inputs_1x.count), r.ghosts, r.sizes[0].0, r.sizes[0].1, r.sizes[1].0,
                r.sizes[1].1, r.card.0, r.card.1, r.game.opaque);
        }
    }
}

/// Whole clips for the pictures: for each animation, `#<record>` as `rife-rank` lists it or `body:action:direction` as
/// the package finds it (`MobileAnimCcPackage::animation`), from the lossless frames and from the
/// package's (named `<kernel>`), every frame and every in-between, the last frame's back to the first included, raw
/// and encoded as the game encodes it, and each pair's flow. Under `<out>/<record>/<input>/`:
/// `frame<k>.rgba`, `between<k>.rgba`, `between<k>-game.rgba`, `flow<k>.f32` (four card-sized planes, little-endian),
/// and `meta.tsv` with each one's size and anchor, and the clip's frame time.
fn rife_dump(dir: &str, kernel: &str, out: &str, specs: &[String]) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut rife = Rife::new();
    let mut pages = Pages { package: &package, gpu: &gpu, kept: VecDeque::new() };
    for spec in specs {
        let record = match spec.strip_prefix('#') {
            Some(record) => record.parse().unwrap(),
            None => {
                let fields: Vec<u32> = spec.split(':').map(|field| field.parse().unwrap()).collect();
                let animation = package.animation(fields[0] as u16, fields[1] as u16, fields[2] as u8)
                    .unwrap_or_else(|| panic!("no animation {spec}"));
                package.animations().iter().position(|other| std::ptr::eq(other, animation)).unwrap()
            }
        };
        let animation = &package.animations()[record];
        let frames = package.animation_frames(animation);
        let cut: Vec<(Vec<u8>, Vec<u8>)> = frames.iter().map(|frame| pages.frame(frame)).collect();
        let frame_secs = if animation.fps > 0.0 { animation.fps.recip() } else { CLASSIC_FRAME_SECS };
        for input in DUMP_INPUTS {
            let folder = format!("{out}/{record}/{}", input.name(kernel));
            std::fs::create_dir_all(&folder).unwrap();
            let made: Vec<Vec<u8>> = cut.iter().enumerate()
                .map(|(k, (lossless, bc7))| input.frame(lossless, bc7, k as u64)).collect();
            let mut meta = format!("kind\tindex\twidth\theight\tanchor_x\tanchor_y\n\
                frame_secs\t{frame_secs}\t0\t0\t0\t0\n");
            for (k, frame) in frames.iter().enumerate() {
                let first = stride_frame(frame, &made[k]);
                std::fs::write(format!("{folder}/frame{k}.rgba"), &made[k]).unwrap();
                meta += &format!("frame\t{k}\t{}\t{}\t{}\t{}\n", first.width, first.height, first.anchor_x,
                    first.anchor_y);
                let next = (k + 1) % frames.len();
                let pair = rife.pair(&first, &stride_frame(&frames[next], &made[next]));
                let game = encode_decode(&gpu, &pair.rgba, pair.card.width, pair.card.height, Input::Bc7(GAME_QUALITY));
                std::fs::write(format!("{folder}/between{k}.rgba"), &pair.rgba).unwrap();
                std::fs::write(format!("{folder}/between{k}-game.rgba"), &game).unwrap();
                std::fs::write(format!("{folder}/flow{k}.f32"), bytemuck::cast_slice::<f32, u8>(&pair.flow)).unwrap();
                meta += &format!("between\t{k}\t{}\t{}\t{}\t{}\n", pair.card.width, pair.card.height,
                    pair.card.anchor_x, pair.card.anchor_y);
            }
            std::fs::write(format!("{folder}/meta.tsv"), meta).unwrap();
        }
        println!("{spec}: record {record} ({}:{}:{}, file {}), {} frames", animation.body_id, animation.action_id,
            animation.direction, animation.file_index, frames.len());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("frames") => {
            let (dir, out) = (args.next().unwrap(), args.next().unwrap());
            frames(&dir, &out, &args.collect::<Vec<_>>());
        }
        Some("psnr") => {
            let dir = args.next().unwrap();
            let (pages, count) = (args.next().unwrap().parse().unwrap(), args.next().unwrap().parse().unwrap());
            psnr(&dir, pages, count);
        }
        Some("rife") => {
            let dir = args.next().unwrap();
            let (animations, pairs) = (args.next().unwrap().parse().unwrap(), args.next().unwrap().parse().unwrap());
            rife(&dir, animations, pairs);
        }
        Some("rife-rank") => {
            let (dir, kernel) = (args.next().unwrap(), args.next().unwrap());
            let (animations, pairs) = (args.next().unwrap().parse().unwrap(), args.next().unwrap().parse().unwrap());
            rife_rank(&dir, &kernel, animations, pairs, &args.next().unwrap());
        }
        Some("rife-dump") => {
            let (dir, kernel, out) = (args.next().unwrap(), args.next().unwrap(), args.next().unwrap());
            rife_dump(&dir, &kernel, &out, &args.collect::<Vec<_>>());
        }
        _ => panic!("classic_span frames|psnr|rife|rife-rank|rife-dump ..."),
    }
}
