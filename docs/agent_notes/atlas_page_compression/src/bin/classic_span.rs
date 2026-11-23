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
//!
//! The inputs: the lossless frames, BC7 UltraFast and VeryFast, UltraFast of the page with the transparent texels of
//! its edge blocks given their nearest opaque colour first (`dilated`, classic_formats' `prefill`), and BC1.
use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::MobileAnimCcPackage;
use udd_assets::animation::mobile_anim_cc::MobileAnimCcFrameRecord;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::frame_upscales::FrameUpscaler;
use uot_gpu_compute::in_betweens::InBetweenMaker;
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

/// SPAN 2x of a frame (its whole canvas, the halo included), Classic alpha snapped.
fn span(upscaler: &mut FrameUpscaler, frame: &MobileAnimCcFrameRecord, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let stride = StrideFrame { width: frame.width, height: frame.height, anchor_x: i32::from(frame.center_x),
        anchor_y: i32::from(frame.center_y), rgba };
    let out = upscaler.upscale(&stride, true).unwrap();
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
            let strides: Vec<StrideFrame> = frames.iter().zip(&cut_frames).map(|(frame, rgba)| StrideFrame {
                width: frame.width, height: frame.height, anchor_x: i32::from(frame.center_x),
                anchor_y: i32::from(frame.center_y), rgba }).collect();
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
        _ => panic!("classic_span frames|psnr|rife ..."),
    }
}
