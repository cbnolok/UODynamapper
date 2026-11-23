//! BC1 for the Classic creatures, the large ones (dragons, wyrms, daemons, ...) and all, against the packages' BC7
//! UltraFast and lossless (docs/agent_notes/atlas_page_compression.md, "BC1 for the Classic creatures").
//!
//! `<selection>` is a comma separated list of bodies, or `sample:N`: every frame of N pages spread over the package.
//! `<inputs>` is a comma separated list from `lossless` (first, the reference), `bc7` (GPU UltraFast, as packed),
//! `bc1-high` and `bc1-best` (CPU, the dds crate's High and Unreasonable) and `bc1-gpu` (uot-gpu-compute).
//!
//! Usage:
//! - `classic_big survey <lossless uddp dir> <BC7 uddp dir> <tsv>`: every body's frames, their median and largest
//!   side and texels, and its share of the two packages' pages on disk (each page's stored size parted over its frames
//!   by their block-padded texels); each body a row of `tsv`, the shares by median side printed.
//! - `classic_big quality <lossless uddp dir> <selection> <SPAN frames per body> <inputs> <tsv>`: the pages holding
//!   the selected frames encoded as each input, each page whole as a packer does: their size at Zstd 19 and raw, and on
//!   the selected frames the opaque PSNR, cutoff flips, ghosts and edge taps; SPAN 2x of some frames per body from each
//!   input, raw and after the game's runtime BC7 encode, against the lossless chain; and Classic hues on every frame.
//! - `classic_big rife <lossless uddp dir> <selection> <animations per body> <pairs> <inputs>`: the RIFE in-betweens
//!   from each input against those from lossless, raw and after the game's runtime encode.
//! - `classic_big dump <lossless uddp dir> <out dir> <inputs> <name=body:action:direction> ...`: whole clips of each
//!   input at 1x, after SPAN 2x and the game's encode, and with RIFE x2 in-betweens (encoded as the game does), as raw
//!   RGBA, for classic_big_sheets.py.
//! - `classic_big runtime-time <lossless uddp dir> <selection> <frames>`: the GPU kernel time of the runtime encode,
//!   BC7 UltraFast against BC1, on SPAN 2x canvases of the selected frames.
//!
//! `CLASSIC_BIG_RUNTIME=bc1-gpu` makes the game's runtime encode of the SPAN frames and in-betweens BC1 (the GPU
//! kernel) instead of BC7 UltraFast, in `quality`, `rife` and `dump`.
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::Instant;
use udd_assets::animation::mobile_anim_cc::{page_entry_path, MobileAnimCcFrameRecord, MISSING_PAGE_INDEX};
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::{HuesPackage, MobileAnimCcPackage};
use udd_container::UddpReader;
use udd_container::xxh64_virtual_path;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::frame_upscales::FrameUpscaler;
use uot_gpu_compute::in_betweens::InBetweenMaker;
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_gpu_compute::strides::StrideFrame;
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, ImageExtent};

/// The environment variable naming the game's runtime encode (`runtime`).
const RUNTIME_ENVIRONMENT: &str = "CLASSIC_BIG_RUNTIME";
/// The game's limit on the upscaled clips (dynamapper upscales.rs `UPSCALE_MAX_CANVAS_PIXELS` and
/// `UPSCALE_CANVAS_MULTIPLE`, uot-gpu-compute's `UPSCALE_BORDER`).
const UPSCALE_MAX_CANVAS_PIXELS: u32 = 260 * 260;
const UPSCALE_CANVAS_MULTIPLE: u32 = 8;
const UPSCALE_BORDER: u32 = 2;
/// `runtime-time`: the timed runs after the warm-up, and the bytes of its two timestamps.
const TIMING_RUNS: usize = 5;
const TIMESTAMP_BYTES: u64 = 16;
const PEAK_SQUARED: f64 = 255.0 * 255.0;
const MIB: f64 = 1048576.0;
/// The renderer's alpha cutoff (statics_draw.rs `alpha_cutoff: 0.5`) on 8-bit alpha.
const CUTOFF: u8 = 128;
const RGBA_BYTES: usize = 4;
const COLOR_CHANNELS: usize = 3;
/// The block side of BC1 and BC7.
const BLOCK_EDGE: u32 = 4;
const BC7_BLOCK_BYTES: u64 = 16;
const BC1_BLOCK_BYTES: u64 = 8;
const RGBA8_TEXEL_BYTES: u64 = 4;
/// The packages' Zstd level.
const ZSTD_LEVEL: i32 = 19;
/// The largest frame side the upscaler takes (dynamapper's upscales; every Classic frame is under it).
const MAX_FRAME_SIDE: u32 = 512;
/// The quality the game encodes its SPAN and RIFE frames at (bc7_encoding.rs `MOBILE_BC7_QUALITY`).
const GAME_QUALITY: Bc7Quality = Bc7Quality::UltraFast;
/// RIFE's frames per pair: the first and one in-between, half-way (the game's 2x).
const RIFE_FACTOR: u32 = 2;
/// The frame atlas (dynamapper frame_atlas.rs): its page side, its pages for the clips, and those for the upscaled
/// sequences while SPAN is on.
const ATLAS_PAGE_SIZE: u64 = 1024;
const ATLAS_PAGES: u64 = 8;
const ATLAS_UPSCALED_PAGES: u64 = 16;
/// The stored directions of a Classic animation (the other three are drawn mirrored).
const STORED_DIRECTIONS: u8 = 5;
const PERCENT: f64 = 100.0;
/// The Classic frames' channels are 5 bits widened as `c << 3` (uocf `rgba8888_word_from_bgra5551`).
const CLASSIC_CHANNEL_SHIFT: u32 = 3;
/// The Rec. 709 luma weights the renderer's static hue keys its strip with (hue.wesl `apply_static_hue`).
const LUMA_WEIGHTS: [f64; 3] = [0.2125, 0.7154, 0.0721];
const HUE_TABLE_ENTRIES: usize = 256;
/// The hues of the hue measurements: dark and bright, saturated and grey-ish, among the common server hues.
const HUES: [u16; 6] = [0x0021, 0x0035, 0x0480, 0x0455, 0x0497, 0x04F2];
const MEDIAN: f64 = 0.5;
const LOW_PERCENTILE: f64 = 0.1;

/// The movement and idle actions a creature's clips take in a scene, by the body's layout (mobile_anim_cc.rs: the
/// monster stand 1 and idle 8, walk 0; the animal walk 0, run 1, stand 2; attacks 4 and 5 for the monster, 8 for the
/// animal). Missing actions count nothing.
const SCENE_ACTIONS: [u16; 6] = [0, 1, 2, 4, 5, 8];

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Input {
    Lossless,
    /// The packages' BC7, the GPU's UltraFast of the whole page.
    Bc7,
    /// BC1 through `dds` on the CPU, at High (the gumps' encoder) and at its best setting.
    Bc1High,
    Bc1Best,
    /// BC1 on the GPU (uot-gpu-compute bc1.wgsl).
    Bc1Gpu,
}

const INPUTS: [Input; 5] = [Input::Lossless, Input::Bc7, Input::Bc1High, Input::Bc1Best, Input::Bc1Gpu];

impl Input {
    fn name(self) -> &'static str {
        match self {
            Input::Lossless => "lossless",
            Input::Bc7 => "bc7",
            Input::Bc1High => "bc1-high",
            Input::Bc1Best => "bc1-best",
            Input::Bc1Gpu => "bc1-gpu",
        }
    }

    /// The bytes the page's blocks (or texels) take before Zstd.
    fn bytes(self, width: u32, height: u32) -> u64 {
        let blocks = u64::from(width.div_ceil(BLOCK_EDGE)) * u64::from(height.div_ceil(BLOCK_EDGE));
        match self {
            Input::Lossless => u64::from(width) * u64::from(height) * RGBA8_TEXEL_BYTES,
            Input::Bc7 => blocks * BC7_BLOCK_BYTES,
            Input::Bc1High | Input::Bc1Best | Input::Bc1Gpu => blocks * BC1_BLOCK_BYTES,
        }
    }
}

/// A lossless page, its used part.
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

fn dds_bc1(rgba: &[u8], width: u32, height: u32, quality: dds::CompressionQuality) -> Vec<u8> {
    let image = dds::ImageView::new(rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8).unwrap();
    let mut options = dds::EncodeOptions::default();
    options.quality = quality;
    options.dithering = dds::Dithering::None;
    let mut blocks = Vec::new();
    dds::encode(&mut blocks, image, dds::Format::BC1_UNORM, None, &options).unwrap();
    blocks
}

fn decode_bc1(blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; width as usize * height as usize * RGBA_BYTES];
    let image = dds::ImageViewMut::new(&mut rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8).unwrap();
    dds::decode(&mut Cursor::new(blocks), image, dds::Format::BC1_UNORM, &dds::DecodeOptions::default()).unwrap();
    rgba
}

/// The page encoded as `input` (its blocks, or its texels for lossless) and decoded again.
fn encode(gpu: &PageEncoderGpu, rgba: &[u8], width: u32, height: u32, input: Input) -> (Vec<u8>, Vec<u8>) {
    let extent = ImageExtent::new(width, height).unwrap();
    match input {
        Input::Lossless => (rgba.to_vec(), rgba.to_vec()),
        Input::Bc7 => {
            let blocks = gpu.encode(rgba, width, height, GAME_QUALITY).unwrap().concat();
            let decoded = decode_bc7_to_rgba8888(&blocks, extent).unwrap();
            (blocks, decoded)
        }
        Input::Bc1High | Input::Bc1Best => {
            let quality = if input == Input::Bc1High { dds::CompressionQuality::High } else {
                dds::CompressionQuality::Unreasonable
            };
            let blocks = dds_bc1(rgba, width, height, quality);
            let decoded = decode_bc1(&blocks, width, height);
            (blocks, decoded)
        }
        Input::Bc1Gpu => {
            let blocks = gpu.encode_bc1(rgba, width, height).unwrap().concat();
            let decoded = decode_bc1(&blocks, width, height);
            (blocks, decoded)
        }
    }
}

fn zstd_size(bytes: &[u8]) -> u64 {
    zstd::bulk::compress(bytes, ZSTD_LEVEL).unwrap().len() as u64
}

fn cut(page_rgba: &[u8], page_width: u32, frame: &MobileAnimCcFrameRecord) -> Vec<u8> {
    let (x, y, w) = (frame.x as usize, frame.y as usize, frame.width as usize);
    (y..y + frame.height as usize).flat_map(|row| {
        let at = (row * page_width as usize + x) * RGBA_BYTES;
        page_rgba[at..at + w * RGBA_BYTES].to_vec()
    }).collect()
}

/// A frame's texels padded to whole blocks, as the BC7 frame atlas takes them.
fn padded_texels(width: u32, height: u32) -> u64 {
    u64::from(width.next_multiple_of(BLOCK_EDGE)) * u64::from(height.next_multiple_of(BLOCK_EDGE))
}

fn frame_ok(frame: &MobileAnimCcFrameRecord) -> bool {
    frame.page_index != MISSING_PAGE_INDEX && frame.width > 0 && frame.height > 0
}

/// The stored size of each page of a package, by page index.
fn page_sizes(dir: &str) -> (MobileAnimCcPackage, HashMap<u32, u64>) {
    let path = format!("{dir}/mobile_anim_cc.uddp");
    let package = MobileAnimCcPackage::load(&path).unwrap();
    let reader = UddpReader::load(&path).unwrap();
    let sizes = package.pages().iter().map(|page| {
        let hash = xxh64_virtual_path(&page_entry_path(page.page_index, page.pixel_format));
        (page.page_index, u64::from(reader.find_by_path_hash(hash).unwrap().stored_size))
    }).collect();
    (package, sizes)
}

/// What a body holds: its frames, their largest side, texels (padded to blocks), and its part of each package's
/// pages on disk.
#[derive(Default, Clone)]
struct BodyStats {
    animations: usize,
    frames: usize,
    largest_side: u32,
    sides: Vec<u32>,
    texels: u64,
    padded: u64,
    disk: [f64; 2],
    /// The block-padded texels of its scene clips (`SCENE_ACTIONS`, every stored direction), and of the largest of
    /// them.
    scene_padded: u64,
    largest_clip_padded: u64,
    /// The block-padded texels of its clips the game upscales (`upscalable`).
    upscalable_padded: u64,
}

/// Whether the game upscales a clip with frames of these sizes (dynamapper upscales.rs `canvas_of`): its largest
/// frame with the border, rounded up, within `UPSCALE_MAX_CANVAS_PIXELS`; else the clip plays at 1x.
fn upscalable(sizes: &[(u32, u32)]) -> bool {
    let (width, height) = sizes.iter().fold((1, 1), |(w, h), &(width, height)| {
        (w.max(width + 2 * UPSCALE_BORDER), h.max(height + 2 * UPSCALE_BORDER))
    });
    width.next_multiple_of(UPSCALE_CANVAS_MULTIPLE) * height.next_multiple_of(UPSCALE_CANVAS_MULTIPLE)
        <= UPSCALE_MAX_CANVAS_PIXELS
}

/// Each body's frames parted over the package's pages by their padded texels.
fn attribute(package: &MobileAnimCcPackage, sizes: &HashMap<u32, u64>, slot: usize,
    stats: &mut BTreeMap<u16, BodyStats>) {
    let mut page_padded: HashMap<u32, u64> = HashMap::new();
    for frame in package.frames().iter().filter(|frame| frame_ok(frame)) {
        *page_padded.entry(frame.page_index).or_default() += padded_texels(u32::from(frame.width),
            u32::from(frame.height));
    }
    for animation in package.animations() {
        let body = stats.entry(animation.body_id).or_default();
        for frame in package.animation_frames(animation).iter().filter(|frame| frame_ok(frame)) {
            let padded = padded_texels(u32::from(frame.width), u32::from(frame.height));
            body.disk[slot] += sizes[&frame.page_index] as f64 * padded as f64 / page_padded[&frame.page_index] as f64;
        }
    }
}

fn survey(lossless_dir: &str, bc7_dir: &str, tsv: &str) {
    let (lossless, lossless_sizes) = page_sizes(lossless_dir);
    let (bc7, bc7_sizes) = page_sizes(bc7_dir);
    println!("lossless: {} pages, {:.1} MiB of pages ({:?}); BC7: {} pages, {:.1} MiB ({:?})", lossless.pages().len(),
        lossless_sizes.values().sum::<u64>() as f64 / MIB, lossless.pixel_format(), bc7.pages().len(),
        bc7_sizes.values().sum::<u64>() as f64 / MIB, bc7.pixel_format());
    let mut stats: BTreeMap<u16, BodyStats> = BTreeMap::new();
    for animation in lossless.animations() {
        let body = stats.entry(animation.body_id).or_default();
        let frames: Vec<&MobileAnimCcFrameRecord> = lossless.animation_frames(animation).iter()
            .filter(|frame| frame_ok(frame)).collect();
        if frames.is_empty() {
            continue;
        }
        body.animations += 1;
        let mut clip_padded = 0;
        for frame in &frames {
            let (width, height) = (u32::from(frame.width), u32::from(frame.height));
            body.frames += 1;
            body.largest_side = body.largest_side.max(width.max(height));
            body.sides.push(width.max(height));
            body.texels += u64::from(width) * u64::from(height);
            let padded = padded_texels(width, height);
            body.padded += padded;
            clip_padded += padded;
        }
        body.largest_clip_padded = body.largest_clip_padded.max(clip_padded);
        let sizes: Vec<(u32, u32)> = frames.iter().map(|f| (u32::from(f.width), u32::from(f.height))).collect();
        if upscalable(&sizes) {
            body.upscalable_padded += clip_padded;
        }
        if SCENE_ACTIONS.contains(&animation.action_id) && animation.direction < STORED_DIRECTIONS {
            body.scene_padded += clip_padded;
        }
    }
    attribute(&lossless, &lossless_sizes, 0, &mut stats);
    attribute(&bc7, &bc7_sizes, 1, &mut stats);
    let total = |f: &dyn Fn(&BodyStats) -> f64| stats.values().map(f).sum::<f64>();
    let (all_frames, all_texels, all_disk0, all_disk1) = (total(&|b| b.frames as f64), total(&|b| b.padded as f64),
        total(&|b| b.disk[0]), total(&|b| b.disk[1]));
    let mut out = String::from("body\tanimations\tframes\tlargest_side\tmedian_side\ttexels\tpadded\t\
        lossless_disk_bytes\tbc7_disk_bytes\tscene_padded\tlargest_clip_padded\tupscalable_padded\n");
    for (body, s) in stats.iter_mut().filter(|(_, s)| s.frames > 0) {
        s.sides.sort_unstable();
        out += &format!("{body}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.0}\t{:.0}\t{}\t{}\t{}\n", s.animations, s.frames,
            s.largest_side, s.sides[s.sides.len() / 2], s.texels, s.padded, s.disk[0], s.disk[1], s.scene_padded,
            s.largest_clip_padded, s.upscalable_padded);
    }
    std::fs::write(tsv, out).unwrap();
    println!("{} bodies with frames, {all_frames} frames, {:.1} Mtexels padded, disk lossless {:.1} MiB, BC7 {:.1} MiB",
        stats.values().filter(|s| s.frames > 0).count(), all_texels / 1e6, all_disk0 / MIB, all_disk1 / MIB);
    // The bodies by median frame side, cumulative shares.
    println!("of the padded texels, {:.1} % in clips the game upscales",
        PERCENT * stats.values().map(|b| b.upscalable_padded as f64).sum::<f64>() / all_texels);
    println!("| median side from | bodies | frames | padded texels | lossless disk | BC7 disk | of them upscaled |");
    for threshold in [0u32, 64, 96, 128, 160, 192, 256] {
        let chosen: Vec<&BodyStats> = stats.values()
            .filter(|s| s.frames > 0 && s.sides[s.sides.len() / 2] >= threshold).collect();
        let sum = |f: &dyn Fn(&BodyStats) -> f64| chosen.iter().map(|s| f(s)).sum::<f64>();
        println!("| {threshold} px | {} | {:.1} % | {:.1} % | {:.1} % ({:.1} MiB) | {:.1} % ({:.1} MiB) | {:.1} % |",
            chosen.len(), PERCENT * sum(&|b| b.frames as f64) / all_frames,
            PERCENT * sum(&|b| b.padded as f64) / all_texels, PERCENT * sum(&|b| b.disk[0]) / all_disk0,
            sum(&|b| b.disk[0]) / MIB, PERCENT * sum(&|b| b.disk[1]) / all_disk1, sum(&|b| b.disk[1]) / MIB,
            PERCENT * sum(&|b| b.upscalable_padded as f64) / sum(&|b| b.padded as f64));
    }
    // The atlas: its bytes per format, and how many scene clip sets of a body it holds.
    let page_texels = ATLAS_PAGE_SIZE * ATLAS_PAGE_SIZE;
    println!("frame atlas: {ATLAS_PAGES} pages of {ATLAS_PAGE_SIZE}^2 + {ATLAS_UPSCALED_PAGES} upscaled: RGBA8 {} MiB, \
        BC7 {} MiB, BC1 {} MiB", (ATLAS_PAGES + ATLAS_UPSCALED_PAGES) * page_texels * RGBA8_TEXEL_BYTES >> 20,
        (ATLAS_PAGES + ATLAS_UPSCALED_PAGES) * page_texels >> 20,
        (ATLAS_PAGES + ATLAS_UPSCALED_PAGES) * page_texels / 2 >> 20);
}


/// RGB error over the reference's opaque texels, cutoff flips, ghosts (texels transparent in the reference decoded
/// with some alpha) and the edge taps: every 2x2 window holding opaque and transparent texels of the reference,
/// blended premultiplied (sum of alpha x RGB over sum of alpha) as the KR client filter taps, its RGB against the
/// reference's tap.
#[derive(Default, Clone, Copy)]
struct Error { squared: f64, count: f64, flips: usize, ghosts: usize, edge_squared: f64, edge_count: f64 }

/// The 2x2 window's offsets.
const WINDOW: [(usize, usize); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];

impl Error {
    fn of(reference: &[u8], other: &[u8], width: usize) -> Self {
        let mut e = Error::default();
        for (r, o) in reference.chunks_exact(RGBA_BYTES).zip(other.chunks_exact(RGBA_BYTES)) {
            if r[3] >= CUTOFF {
                for c in 0..COLOR_CHANNELS { e.squared += (f64::from(r[c]) - f64::from(o[c])).powi(2); }
                e.count += COLOR_CHANNELS as f64;
            } else if o[3] > 0 && r[3] == 0 {
                e.ghosts += 1;
            }
            if (r[3] >= CUTOFF) != (o[3] >= CUTOFF) { e.flips += 1; }
        }
        let height = reference.len() / RGBA_BYTES / width.max(1);
        let tap = |rgba: &[u8], x: usize, y: usize| {
            let (mut sum, mut alpha) = ([0f64; COLOR_CHANNELS], 0f64);
            for (dx, dy) in WINDOW {
                let t = &rgba[((y + dy) * width + x + dx) * RGBA_BYTES..][..RGBA_BYTES];
                let a = f64::from(t[3]);
                for c in 0..COLOR_CHANNELS { sum[c] += a * f64::from(t[c]); }
                alpha += a;
            }
            if alpha > 0.0 { sum.map(|v| v / alpha) } else { [0.0; COLOR_CHANNELS] }
        };
        for y in 0..height.saturating_sub(1) {
            for x in 0..width.saturating_sub(1) {
                let opaque = WINDOW.iter()
                    .filter(|(dx, dy)| reference[((y + dy) * width + x + dx) * RGBA_BYTES + 3] >= CUTOFF).count();
                if opaque == 0 || opaque == WINDOW.len() {
                    continue;
                }
                let (r, o) = (tap(reference, x, y), tap(other, x, y));
                for c in 0..COLOR_CHANNELS { e.edge_squared += (r[c] - o[c]).powi(2); }
                e.edge_count += COLOR_CHANNELS as f64;
            }
        }
        e
    }

    fn add(&mut self, o: Error) {
        self.squared += o.squared;
        self.count += o.count;
        self.flips += o.flips;
        self.ghosts += o.ghosts;
        self.edge_squared += o.edge_squared;
        self.edge_count += o.edge_count;
    }

    fn db(squared: f64, count: f64) -> f64 {
        if squared == 0.0 { f64::INFINITY } else { 10.0 * (PEAK_SQUARED * count.max(1.0) / squared).log10() }
    }

    fn psnr(&self) -> f64 { Self::db(self.squared, self.count) }

    fn edge_psnr(&self) -> f64 { Self::db(self.edge_squared, self.edge_count) }

    fn mse(&self) -> f64 { self.squared / self.count.max(1.0) }
}

fn db_text(value: f64) -> String {
    if value.is_infinite() { "lossless".into() } else { format!("{value:.2} dB") }
}

/// The median, tenth percentile and lowest PSNR of single frames (those without opaque texels left out).
fn spread(errors: &[Error]) -> String {
    let mut values: Vec<f64> = errors.iter().filter(|e| e.count > 0.0).map(Error::psnr).collect();
    if values.is_empty() {
        return "-".into();
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() as f64 * q) as usize).min(values.len() - 1)];
    format!("{:.1}, {:.1}, {:.1}", at(MEDIAN), at(LOW_PERCENTILE), values[0])
}

/// The largest 5-bit channel value.
const FIVE_BIT_MAX: u32 = (1 << 5) - 1;

/// A channel's 5-bit value as the client keys a hue: the colours are widened `c << 3`, so a decoded colour rounds
/// to the nearest step.
fn five_bits(value: u8) -> u8 {
    ((u32::from(value) + (1 << (CLASSIC_CHANNEL_SHIFT - 1))) >> CLASSIC_CHANNEL_SHIFT).min(FIVE_BIT_MAX) as u8
}

/// The client's grey test of partial hue: the 5-bit channels equal.
fn grey(texel: &[u8]) -> bool {
    let (r, g, b) = (five_bits(texel[0]), five_bits(texel[1]), five_bits(texel[2]));
    r == g && g == b
}

/// An 8-bit grey test with a tolerance, as an approximate test of a block-coded body would take.
fn grey_within(texel: &[u8], tolerance: u8) -> bool {
    let (max, min) = (texel[..COLOR_CHANNELS].iter().max().unwrap(), texel[..COLOR_CHANNELS].iter().min().unwrap());
    max - min <= tolerance
}

/// The 8-bit grey tolerances measured besides the exact 5-bit test.
const GREY_TOLERANCES: [u8; 2] = [8, 16];

/// How a decoded frame takes Classic hues against the lossless one, over the texels opaque in the lossless frame.
#[derive(Default, Clone, Copy)]
struct HueStats {
    opaque: usize,
    grey: usize,
    /// Grey texels the decoded frame fails the 5-bit test on, and other texels it passes it on.
    grey_lost: usize,
    grey_gained: usize,
    /// The same with the 8-bit tolerances.
    tolerant_lost: [usize; GREY_TOLERANCES.len()],
    tolerant_gained: [usize; GREY_TOLERANCES.len()],
    /// Texels whose red key (5 bits) changes, and the sum of the steps.
    key_changed: usize,
    key_steps: usize,
    /// Squared RGB error of the hued frame against the lossless frame hued, summed over [`HUES`]: full hue keyed by
    /// red as the client keys it, partial hue (grey texels hued, the rest as they are), and full hue keyed by luma as
    /// the renderer's static path keys its strip; and the unhued error over the same texels.
    full: f64,
    partial: f64,
    luma: f64,
    plain: f64,
    samples: f64,
}

impl HueStats {
    fn of(lossless: &[u8], decoded: &[u8], tables: &[[[u8; 4]; HUE_TABLE_ENTRIES]]) -> Self {
        let mut s = HueStats::default();
        let key = |texel: &[u8]| usize::from(five_bits(texel[0])) << CLASSIC_CHANNEL_SHIFT;
        let luma = |texel: &[u8]| ((0..COLOR_CHANNELS).map(|c| LUMA_WEIGHTS[c] * f64::from(texel[c])).sum::<f64>()
            .round() as usize).min(HUE_TABLE_ENTRIES - 1);
        let squared = |a: &[u8], b: &[u8]| (0..COLOR_CHANNELS)
            .map(|c| (f64::from(a[c]) - f64::from(b[c])).powi(2)).sum::<f64>();
        for (l, d) in lossless.chunks_exact(RGBA_BYTES).zip(decoded.chunks_exact(RGBA_BYTES)) {
            if l[3] < CUTOFF {
                continue;
            }
            s.opaque += 1;
            let (l_grey, d_grey) = (grey(l), grey(d));
            s.grey += usize::from(l_grey);
            s.grey_lost += usize::from(l_grey && !d_grey);
            s.grey_gained += usize::from(!l_grey && d_grey);
            for (k, &tolerance) in GREY_TOLERANCES.iter().enumerate() {
                let d_tolerant = grey_within(d, tolerance);
                s.tolerant_lost[k] += usize::from(l_grey && !d_tolerant);
                s.tolerant_gained[k] += usize::from(!l_grey && d_tolerant);
            }
            let (l_key, d_key) = (five_bits(l[0]), five_bits(d[0]));
            s.key_changed += usize::from(l_key != d_key);
            s.key_steps += usize::from(l_key.abs_diff(d_key));
            for table in tables {
                s.full += squared(&table[key(l)], &table[key(d)]);
                let l_partial: &[u8] = if l_grey { &table[key(l)] } else { l };
                let d_partial: &[u8] = if d_grey { &table[key(d)] } else { d };
                s.partial += squared(l_partial, d_partial);
                s.luma += squared(&table[luma(l)], &table[luma(d)]);
                s.plain += squared(l, d);
                s.samples += COLOR_CHANNELS as f64;
            }
        }
        s
    }

    fn add(&mut self, o: HueStats) {
        self.opaque += o.opaque;
        self.grey += o.grey;
        self.grey_lost += o.grey_lost;
        self.grey_gained += o.grey_gained;
        for k in 0..GREY_TOLERANCES.len() {
            self.tolerant_lost[k] += o.tolerant_lost[k];
            self.tolerant_gained[k] += o.tolerant_gained[k];
        }
        self.key_changed += o.key_changed;
        self.key_steps += o.key_steps;
        self.full += o.full;
        self.partial += o.partial;
        self.luma += o.luma;
        self.plain += o.plain;
        self.samples += o.samples;
    }
}

/// The frames a run measures: those of some bodies, or every frame of pages sampled over the package.
enum Selection { Bodies(Vec<u16>), Sample(usize) }

impl Selection {
    fn parse(text: &str) -> Self {
        match text.strip_prefix("sample:") {
            Some(count) => Selection::Sample(count.parse().unwrap()),
            None => Selection::Bodies(text.split(',').map(|body| body.parse().unwrap()).collect()),
        }
    }

    /// The measured frames with their body, by page.
    fn frames<'a>(&self, package: &'a MobileAnimCcPackage) -> BTreeMap<u32, Vec<(u16, &'a MobileAnimCcFrameRecord)>> {
        let sampled: HashSet<u32> = match self {
            Selection::Sample(count) => {
                let total = package.pages().len();
                (0..*count).map(|k| package.pages()[k * total / count].page_index).collect()
            }
            Selection::Bodies(_) => HashSet::new(),
        };
        let mut pages: BTreeMap<u32, Vec<(u16, &MobileAnimCcFrameRecord)>> = BTreeMap::new();
        for animation in package.animations() {
            if let Selection::Bodies(bodies) = self && !bodies.contains(&animation.body_id) {
                continue;
            }
            for frame in package.animation_frames(animation).iter().filter(|frame| frame_ok(frame)) {
                if matches!(self, Selection::Sample(_)) && !sampled.contains(&frame.page_index) {
                    continue;
                }
                pages.entry(frame.page_index).or_default().push((animation.body_id, frame));
            }
        }
        pages
    }
}

fn hue_tables(dir: &str) -> Vec<[[u8; 4]; HUE_TABLE_ENTRIES]> {
    let hues = HuesPackage::load(format!("{dir}/hues.uddp")).unwrap();
    HUES.iter().map(|&hue| hues.hue_table(hue).unwrap_or_else(|| panic!("no hue {hue:#x}")).entries).collect()
}

/// A frame with its anchor as the game places it (mobile_anim_cc.rs `record_clip_in`).
fn stride_frame<'a>(frame: &MobileAnimCcFrameRecord, rgba: &'a [u8]) -> StrideFrame<'a> {
    StrideFrame { width: frame.width, height: frame.height, anchor_x: i32::from(frame.center_x),
        anchor_y: i32::from(frame.height) + i32::from(frame.center_y), rgba }
}

/// The format of the game's runtime encode: BC7 UltraFast as the renderer does (`MOBILE_BLOCK_FORMAT`), or with
/// `RUNTIME_ENVIRONMENT=bc1-gpu` the GPU BC1 kernel the renderer would take (`MobileBlockFormat::Bc1`).
fn runtime() -> Input {
    static RUNTIME: OnceLock<Input> = OnceLock::new();
    *RUNTIME.get_or_init(|| match std::env::var(RUNTIME_ENVIRONMENT).as_deref() {
        Err(_) | Ok("bc7") => Input::Bc7,
        Ok("bc1-gpu") => Input::Bc1Gpu,
        Ok(other) => panic!("{RUNTIME_ENVIRONMENT}={other}: bc7 or bc1-gpu"),
    })
}

/// The game's runtime encode of a frame it made (a SPAN 2x frame, an in-between), decoded.
fn game_encode(gpu: &PageEncoderGpu, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    encode(gpu, rgba, width, height, runtime()).1
}

fn parse_inputs(text: &str) -> Vec<Input> {
    let inputs: Vec<Input> = text.split(',').map(|name| *INPUTS.iter().find(|i| i.name() == name)
        .unwrap_or_else(|| panic!("unknown input {name}"))).collect();
    assert_eq!(inputs[0], Input::Lossless, "lossless first, the reference");
    inputs
}

fn quality(dir: &str, selection: &str, span_per_body: usize, inputs: &str, tsv: &str) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    let tables = hue_tables(dir);
    println!("GPU: {}; runtime encode {}", gpu.adapter(), runtime().name());
    let inputs = parse_inputs(inputs);
    let n = inputs.len();
    let selection = Selection::parse(selection);
    let pages = selection.frames(&package);
    let frame_total: usize = pages.values().map(Vec::len).sum();
    // SPAN on every n-th frame of a body, so that each body gives about `span_per_body`.
    let mut body_frames: HashMap<u16, usize> = HashMap::new();
    for (body, _) in pages.values().flatten() {
        *body_frames.entry(*body).or_default() += 1;
    }
    let mut body_seen: HashMap<u16, usize> = HashMap::new();
    let (mut sizes, mut raw_sizes) = (vec![0u64; n], vec![0u64; n]);
    let mut at_1x = vec![Error::default(); n];
    let mut frames_1x: Vec<Vec<Error>> = vec![Vec::new(); n];
    let mut per_body: BTreeMap<u16, Vec<Error>> = BTreeMap::new();
    let mut hue = vec![HueStats::default(); n];
    // SPAN: raw against SPAN of lossless; after the game's encode against SPAN of lossless, and against the game's
    // chain from lossless.
    let mut span_1x = vec![Error::default(); n];
    let mut span_raw = vec![Error::default(); n];
    let mut span_game = vec![Error::default(); n];
    let mut span_game_vs_game = vec![Error::default(); n];
    let mut span_frames: Vec<Vec<Error>> = vec![Vec::new(); n];
    let mut span_count = 0;
    let mut rows = String::from("page\tbody\tanimation\tframe\twidth\theight\tinput\tpsnr\tedge_psnr\tflips\tghosts\n");
    for (&page_index, frames) in &pages {
        let page = load_page(&package, page_index);
        // The CPU encodes side by side, the GPU's on this thread.
        let cpu: Vec<Option<(Vec<u8>, Vec<u8>)>> = inputs.par_iter().map(|&input| match input {
            Input::Bc7 | Input::Bc1Gpu => None,
            _ => Some(encode(&gpu, &page.rgba, page.width, page.height, input)),
        }).collect();
        let encoded: Vec<(Vec<u8>, Vec<u8>)> = cpu.into_iter().zip(&inputs).map(|(done, &input)| done
            .unwrap_or_else(|| encode(&gpu, &page.rgba, page.width, page.height, input))).collect();
        let compressed: Vec<u64> = encoded.par_iter().map(|(blocks, _)| zstd_size(blocks)).collect();
        for k in 0..n {
            sizes[k] += compressed[k];
            raw_sizes[k] += inputs[k].bytes(page.width, page.height);
        }
        for &(body, frame) in frames {
            let lossless = cut(&page.rgba, page.width, frame);
            let width = usize::from(frame.width);
            let seen = body_seen.entry(body).or_default();
            let every = body_frames[&body].div_ceil(span_per_body.max(1)).max(1);
            let do_span = span_per_body > 0 && *seen % every == 0
                && u32::from(frame.width.max(frame.height)) <= MAX_FRAME_SIDE;
            *seen += 1;
            let mut span_reference: Option<(Vec<u8>, Vec<u8>)> = None;
            for (k, input) in inputs.iter().enumerate() {
                let decoded = cut(&encoded[k].1, page.width, frame);
                let error = Error::of(&lossless, &decoded, width);
                if k > 0 {
                    at_1x[k].add(error);
                    frames_1x[k].push(error);
                    per_body.entry(body).or_insert_with(|| vec![Error::default(); n])[k].add(error);
                    hue[k].add(HueStats::of(&lossless, &decoded, &tables));
                    rows += &format!("{page_index}\t{body}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{}\t{}\n",
                        frame.animation_index, frame.frame_index, frame.width, frame.height, input.name(),
                        error.psnr(), error.edge_psnr(), error.flips, error.ghosts);
                }
                if !do_span {
                    continue;
                }
                let out = upscaler.upscale(&stride_frame(frame, &decoded), true).unwrap();
                let game = game_encode(&gpu, &out.rgba, out.width, out.height);
                let w = out.width as usize;
                let Some((reference_raw, reference_game)) = &span_reference else {
                    span_game[k].add(Error::of(&out.rgba, &game, w));
                    span_reference = Some((out.rgba, game));
                    span_count += 1;
                    continue;
                };
                span_1x[k].add(error);
                span_raw[k].add(Error::of(reference_raw, &out.rgba, w));
                let game_error = Error::of(reference_raw, &game, w);
                span_game[k].add(game_error);
                span_frames[k].push(game_error);
                span_game_vs_game[k].add(Error::of(reference_game, &game, w));
            }
        }
    }
    std::fs::write(tsv, rows).unwrap();
    println!("{frame_total} frames on {} pages; SPAN on {span_count} of them", pages.len());
    println!("| Input | Pages, Zstd {ZSTD_LEVEL} | of lossless | Raw (VRAM) | Opaque PSNR | Edge taps | Flips | Ghosts | \
        Per frame: median, p10, worst |");
    for (k, input) in inputs.iter().enumerate() {
        println!("| {} | {:.2} MiB | {:.0} % | {:.2} MiB | {} | {} | {} | {} | {} |", input.name(),
            sizes[k] as f64 / MIB, PERCENT * sizes[k] as f64 / sizes[0] as f64, raw_sizes[k] as f64 / MIB,
            db_text(at_1x[k].psnr()), db_text(at_1x[k].edge_psnr()), at_1x[k].flips, at_1x[k].ghosts,
            spread(&frames_1x[k]));
    }
    println!("| Input | 1x of these frames | SPAN 2x vs SPAN of lossless (edge, flips) | x MSE of 1x | game's runtime \
        encode of it vs SPAN of lossless (edge, flips) | vs the lossless chain's | per frame: median, p10, worst |");
    for (k, input) in inputs.iter().enumerate() {
        let ratio = if k == 0 { "-".to_string() } else { format!("{:.2}", span_raw[k].mse() / span_1x[k].mse()) };
        println!("| {} | {} | {} ({}, {}) | {ratio} | {} ({}, {}) | {} | {} |", input.name(), db_text(span_1x[k].psnr()),
            db_text(span_raw[k].psnr()), db_text(span_raw[k].edge_psnr()), span_raw[k].flips,
            db_text(span_game[k].psnr()), db_text(span_game[k].edge_psnr()), span_game[k].flips,
            db_text(span_game_vs_game[k].psnr()), spread(&span_frames[k]));
    }
    if let Selection::Bodies(bodies) = &selection {
        println!("| Body | frames | {} (opaque / edge dB) |", inputs[1..].iter().map(|i| i.name()).collect::<Vec<_>>()
            .join(" | "));
        for body in bodies {
            let Some(errors) = per_body.get(body) else { continue };
            println!("| {body} | {} | {} |", body_frames[body], errors[1..].iter()
                .map(|e| format!("{:.2} / {:.2}", e.psnr(), e.edge_psnr())).collect::<Vec<_>>().join(" | "));
        }
    }
    println!("Hues {HUES:x?}, over the opaque texels: | input | grey (5-bit test) | lost / gained by the 5-bit test | \
        lost / gained at 8-bit tolerances {GREY_TOLERANCES:?} | red key changed (mean steps) | unhued | full hue by \
        red | partial hue | full hue by luma |");
    for (k, input) in inputs.iter().enumerate().skip(1) {
        let h = &hue[k];
        let share = |count: usize, of: usize| PERCENT * count as f64 / of.max(1) as f64;
        let tolerant: Vec<String> = (0..GREY_TOLERANCES.len()).map(|t| format!("{:.2} / {:.2} %",
            share(h.tolerant_lost[t], h.grey), share(h.tolerant_gained[t], h.opaque - h.grey))).collect();
        println!("| {} | {:.2} % | {:.2} / {:.2} % | {} | {:.2} % ({:.2}) | {} | {} | {} | {} |", input.name(),
            share(h.grey, h.opaque), share(h.grey_lost, h.grey), share(h.grey_gained, h.opaque - h.grey),
            tolerant.join(" | "), share(h.key_changed, h.opaque), h.key_steps as f64 / h.key_changed.max(1) as f64,
            db_text(Error::db(h.plain, h.samples)), db_text(Error::db(h.full, h.samples)),
            db_text(Error::db(h.partial, h.samples)), db_text(Error::db(h.luma, h.samples)));
    }
}

/// The pages kept decoded at once by `rife` and `dump`.
const PAGE_CACHE: usize = 48;

/// Decoded pages of each input, made as asked for and kept.
struct Decoded<'a> { package: &'a MobileAnimCcPackage, gpu: &'a PageEncoderGpu, inputs: Vec<Input>,
    pages: HashMap<u32, (u32, Vec<Vec<u8>>)> }

impl Decoded<'_> {
    fn frame(&mut self, frame: &MobileAnimCcFrameRecord, k: usize) -> Vec<u8> {
        if self.pages.len() >= PAGE_CACHE && !self.pages.contains_key(&frame.page_index) {
            self.pages.clear();
        }
        let (package, gpu, inputs) = (self.package, self.gpu, &self.inputs);
        let (width, decoded) = self.pages.entry(frame.page_index).or_insert_with(|| {
            let page = load_page(package, frame.page_index);
            (page.width, inputs.iter().map(|&input| encode(gpu, &page.rgba, page.width, page.height, input).1).collect())
        });
        cut(&decoded[k], *width, frame)
    }
}

/// The animations of a selection with two frames or more, none past the upscaler's side: `count` per body, or
/// `count` spread over the package.
fn rife_animations(package: &MobileAnimCcPackage, selection: &Selection, count: usize) -> Vec<usize> {
    let usable = |index: &usize| {
        let animation = &package.animations()[*index];
        animation.frame_count >= 2 && package.animation_frames(animation).iter()
            .all(|frame| frame_ok(frame) && u32::from(frame.width.max(frame.height)) <= MAX_FRAME_SIDE)
    };
    let pick = |candidates: Vec<usize>| -> Vec<usize> {
        let n = count.min(candidates.len());
        (0..n).map(|k| candidates[k * candidates.len() / n]).collect()
    };
    match selection {
        Selection::Sample(_) => pick((0..package.animations().len()).filter(usable).collect()),
        Selection::Bodies(bodies) => bodies.iter().flat_map(|&body| pick((0..package.animations().len())
            .filter(|&i| package.animations()[i].body_id == body).filter(usable).collect())).collect(),
    }
}

/// The RIFE in-betweens of the selected animations from each input, raw and after the game's runtime encode, against
/// those from lossless; and, as the game upscales a RIFE sequence (upscales.rs), SPAN 2x of each encoded in-between,
/// encoded again, against SPAN 2x of the raw in-between from lossless.
fn rife(dir: &str, selection: &str, count: usize, pair_count: usize, inputs: &str) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut maker = InBetweenMaker::new(|_| {}).unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    println!("GPU: {} (RIFE on {}); runtime encode {}", gpu.adapter(), maker.adapter(), runtime().name());
    let inputs = parse_inputs(inputs);
    let n = inputs.len();
    let selection = Selection::parse(selection);
    let mut chosen = rife_animations(&package, &selection, count);
    // By page, so that the decoded pages are reused.
    chosen.sort_by_key(|&index| package.animation_frames(&package.animations()[index])[0].page_index);
    let mut decoded = Decoded { package: &package, gpu: &gpu, inputs: inputs.clone(), pages: HashMap::new() };
    let cancel = AtomicBool::new(false);
    let mut raw = vec![Error::default(); n];
    let mut game = vec![Error::default(); n];
    let mut game_vs_game = vec![Error::default(); n];
    let mut per: Vec<Vec<Error>> = vec![Vec::new(); n];
    let mut chained = vec![Error::default(); n];
    let mut in_betweens = 0;
    for &animation_index in &chosen {
        let frames = package.animation_frames(&package.animations()[animation_index]);
        let frames = &frames[..frames.len().min(pair_count + 1)];
        // Per in-between from lossless: raw, encoded, width, and SPAN 2x of the raw one.
        let mut reference: Vec<(Vec<u8>, Vec<u8>, usize, Option<Vec<u8>>)> = Vec::new();
        for k in 0..n {
            let cut_frames: Vec<Vec<u8>> = frames.iter().map(|frame| decoded.frame(frame, k)).collect();
            let strides: Vec<StrideFrame> = frames.iter().zip(&cut_frames)
                .map(|(frame, rgba)| stride_frame(frame, rgba)).collect();
            let sequence = maker.sequence(&strides, RIFE_FACTOR, true, &cancel, |_, _| {}).unwrap().unwrap();
            // The in-betweens of the pairs (0, 1), (1, 2), ..., without the one from the last frame back to the first.
            let made: Vec<_> = sequence.iter().skip(1).step_by(RIFE_FACTOR as usize).take(frames.len() - 1).collect();
            for (pair, in_between) in made.iter().enumerate() {
                let w = in_between.width as usize;
                let encoded = game_encode(&gpu, &in_between.rgba, in_between.width, in_between.height);
                let fits = in_between.width.max(in_between.height) <= MAX_FRAME_SIDE;
                let mut span = |rgba: &[u8]| fits.then(|| {
                    let frame = StrideFrame { width: in_between.width as u16, height: in_between.height as u16,
                        anchor_x: in_between.anchor_x, anchor_y: in_between.anchor_y, rgba };
                    upscaler.upscale(&frame, true).unwrap()
                });
                let up = span(&encoded);
                if k == 0 {
                    game[0].add(Error::of(&in_between.rgba, &encoded, w));
                    let up_reference = span(&in_between.rgba).map(|up| up.rgba);
                    if let (Some(up), Some(up_reference)) = (&up, &up_reference) {
                        let game_up = game_encode(&gpu, &up.rgba, up.width, up.height);
                        chained[0].add(Error::of(up_reference, &game_up, up.width as usize));
                    }
                    reference.push((in_between.rgba.clone(), encoded, w, up_reference));
                    in_betweens += 1;
                    continue;
                }
                let (reference_raw, reference_game, width, up_reference) = &reference[pair];
                assert_eq!(*width, w, "the same card");
                raw[k].add(Error::of(reference_raw, &in_between.rgba, w));
                let error = Error::of(reference_raw, &encoded, w);
                game[k].add(error);
                per[k].push(error);
                game_vs_game[k].add(Error::of(reference_game, &encoded, w));
                if let (Some(up), Some(up_reference)) = (&up, up_reference) {
                    let game_up = game_encode(&gpu, &up.rgba, up.width, up.height);
                    chained[k].add(Error::of(up_reference, &game_up, up.width as usize));
                }
            }
        }
    }
    println!("{in_betweens} in-betweens of {} animations; the game's encode of those from lossless: {} (edge {}); \
        SPAN 2x of that, encoded, vs SPAN 2x of the raw: {} (edge {})", chosen.len(), db_text(game[0].psnr()),
        db_text(game[0].edge_psnr()), db_text(chained[0].psnr()), db_text(chained[0].edge_psnr()));
    println!("| Input | RIFE in-between vs from lossless (edge, flips) | game's runtime encode of it vs from lossless \
        (edge, flips) | vs the lossless chain's | per in-between: median, p10, worst | then SPAN 2x and the runtime \
        encode vs SPAN 2x of the raw one from lossless (edge, flips) |");
    for (k, input) in inputs.iter().enumerate().skip(1) {
        println!("| {} | {} ({}, {}) | {} ({}, {}) | {} | {} | {} ({}, {}) |", input.name(), db_text(raw[k].psnr()),
            db_text(raw[k].edge_psnr()), raw[k].flips, db_text(game[k].psnr()), db_text(game[k].edge_psnr()),
            game[k].flips, db_text(game_vs_game[k].psnr()), spread(&per[k]), db_text(chained[k].psnr()),
            db_text(chained[k].edge_psnr()), chained[k].flips);
    }
}

/// `runtime-time <lossless uddp dir> <selection> <frames>`: the GPU kernel time of the game's runtime encode, BC7
/// UltraFast against BC1, on SPAN 2x canvases of up to `frames` selected frames, all recorded in one compute pass as
/// the renderer batches them (timestamp queries; best of `TIMING_RUNS` after a warm-up run).
fn runtime_time(dir: &str, selection: &str, count: usize) {
    use uot_gpu_compute::block_encode::{MobileBlockEncoder, MobileBlockFormat};
    use wgpu::util::DeviceExt;
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    let selection = Selection::parse(selection);
    let all: Vec<&MobileAnimCcFrameRecord> = selection.frames(&package).into_values().flatten()
        .map(|(_, frame)| frame).filter(|frame| u32::from(frame.width.max(frame.height)) <= MAX_FRAME_SIDE).collect();
    let picked: Vec<_> = (0..count.min(all.len())).map(|k| all[k * all.len() / count.min(all.len())]).collect();
    let mut canvases = Vec::new();
    let mut pages: HashMap<u32, (Vec<u8>, u32)> = HashMap::new();
    for frame in picked {
        let (rgba, width) = pages.entry(frame.page_index).or_insert_with(|| {
            let page = load_page(&package, frame.page_index);
            (page.rgba, page.width)
        });
        let lossless = cut(rgba, *width, frame);
        canvases.push(upscaler.upscale(&stride_frame(frame, &lossless), true).unwrap());
        if pages.len() > PAGE_CACHE { pages.clear(); }
    }
    let texels: f64 = canvases.iter().map(|c| f64::from(c.width) * f64::from(c.height)).sum();
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.unwrap();
        adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TIMESTAMP_QUERY, required_limits: adapter.limits(), ..Default::default()
        }).await.unwrap()
    });
    let period = f64::from(queue.get_timestamp_period());
    let views: Vec<wgpu::TextureView> = canvases.iter().map(|canvas| device.create_texture_with_data(&queue,
        &wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: canvas.width, height: canvas.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }, wgpu::util::TextureDataOrder::LayerMajor, &canvas.rgba).create_view(&Default::default())).collect();
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None, ty: wgpu::QueryType::Timestamp, count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: TIMESTAMP_BYTES, mapped_at_creation: false,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    });
    let stamps = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: TIMESTAMP_BYTES, mapped_at_creation: false,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    });
    let mut encoder = MobileBlockEncoder::lazy(&device);
    println!("{} SPAN 2x canvases, {:.1} Mtexels", canvases.len(), texels / 1e6);
    for format in [MobileBlockFormat::Bc7(GAME_QUALITY), MobileBlockFormat::Bc1] {
        let offsets: Vec<u64> = canvases.iter().scan(0, |offset, canvas| {
            let at = *offset;
            *offset += format.encoded_bytes(canvas.width, canvas.height);
            Some(at)
        }).collect();
        let bytes = offsets.last().unwrap() + format.encoded_bytes(canvases.last().unwrap().width,
            canvases.last().unwrap().height);
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: bytes, mapped_at_creation: false, usage: wgpu::BufferUsages::STORAGE,
        });
        let mut best = f64::MAX;
        let wall = Instant::now();
        for run in 0..=TIMING_RUNS {
            let mut commands = device.create_command_encoder(&Default::default());
            {
                let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None, timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: &queries, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1),
                    }),
                });
                for ((canvas, view), offset) in canvases.iter().zip(&views).zip(&offsets) {
                    encoder.add_frame(format, view, canvas.width, canvas.height, &output, *offset);
                }
                encoder.record(&mut pass);
            }
            commands.resolve_query_set(&queries, 0..2, &resolve, 0);
            commands.copy_buffer_to_buffer(&resolve, 0, &stamps, 0, TIMESTAMP_BYTES);
            queue.submit([commands.finish()]);
            let slice = stamps.slice(..);
            slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let ticks: Vec<u64> = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
            stamps.unmap();
            let secs = (ticks[1] - ticks[0]) as f64 * period * 1e-9;
            if run > 0 { best = best.min(secs); }
        }
        println!("| {} | {:.2} ms | {:.3} ms | {:.0} Mtexels/s | (wall of {} runs and the compile {:.2} s) |",
            format.compile_name(), best * 1e3, best * 1e3 / canvases.len() as f64, texels / 1e6 / best,
            TIMING_RUNS + 1, wall.elapsed().as_secs_f64());
    }
}

fn write_frame(dir: &std::path::Path, index: usize, width: u32, height: u32, anchor_x: i32, anchor_y: i32,
    rgba: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{index:03}_{width}x{height}_{anchor_x}_{anchor_y}.rgba")), rgba).unwrap();
}

/// Whole clips of each input: `1x/`, `span/` (SPAN 2x after the game's BC7 encode) and `rife/` (the clip with its
/// RIFE x2 in-betweens, those after the game's encode), each frame `<index>_<w>x<h>_<anchor x>_<anchor y>.rgba`.
fn dump(dir: &str, out: &str, inputs: &str, specs: &[String]) {
    let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
    let gpu = PageEncoderGpu::new().unwrap();
    let mut upscaler = FrameUpscaler::new(MAX_FRAME_SIDE).unwrap();
    let mut maker = InBetweenMaker::new(|_| {}).unwrap();
    let inputs = parse_inputs(inputs);
    let mut decoded = Decoded { package: &package, gpu: &gpu, inputs: inputs.clone(), pages: HashMap::new() };
    let cancel = AtomicBool::new(false);
    for named in specs {
        let (name, spec) = named.split_once('=').expect("name=body:action:direction");
        let fields: Vec<u16> = spec.split(':').map(|field| field.parse().unwrap()).collect();
        let animation = package.animation(fields[0], fields[1], fields[2] as u8)
            .unwrap_or_else(|| panic!("no animation {spec}"));
        let frames = package.animation_frames(animation);
        for (k, input) in inputs.iter().enumerate() {
            let base = std::path::Path::new(out).join(name).join(input.name());
            let cut_frames: Vec<Vec<u8>> = frames.iter().map(|frame| decoded.frame(frame, k)).collect();
            let strides: Vec<StrideFrame> = frames.iter().zip(&cut_frames)
                .map(|(frame, rgba)| stride_frame(frame, rgba)).collect();
            for (index, stride) in strides.iter().enumerate() {
                write_frame(&base.join("1x"), index, u32::from(stride.width), u32::from(stride.height),
                    stride.anchor_x, stride.anchor_y, stride.rgba);
                let up = upscaler.upscale(stride, true).unwrap();
                let game = game_encode(&gpu, &up.rgba, up.width, up.height);
                write_frame(&base.join("span"), index, up.width, up.height, up.anchor_x, up.anchor_y, &game);
            }
            let sequence = maker.sequence(&strides, RIFE_FACTOR, true, &cancel, |_, _| {}).unwrap().unwrap();
            for (index, frame) in sequence.iter().enumerate() {
                let rgba = if index % RIFE_FACTOR as usize == 0 { frame.rgba.clone() } else {
                    game_encode(&gpu, &frame.rgba, frame.width, frame.height)
                };
                write_frame(&base.join("rife"), index, frame.width, frame.height, frame.anchor_x, frame.anchor_y,
                    &rgba);
            }
        }
        println!("{name}: {spec}, {} frames", frames.len());
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("survey") => {
            let (lossless, bc7, tsv) = (args.next().unwrap(), args.next().unwrap(), args.next().unwrap());
            survey(&lossless, &bc7, &tsv);
        }
        Some("quality") => {
            let (dir, selection) = (args.next().unwrap(), args.next().unwrap());
            let span = args.next().unwrap().parse().unwrap();
            let (inputs, tsv) = (args.next().unwrap(), args.next().unwrap());
            quality(&dir, &selection, span, &inputs, &tsv);
        }
        Some("rife") => {
            let (dir, selection) = (args.next().unwrap(), args.next().unwrap());
            let (count, pairs) = (args.next().unwrap().parse().unwrap(), args.next().unwrap().parse().unwrap());
            rife(&dir, &selection, count, pairs, &args.next().unwrap());
        }
        Some("dump") => {
            let (dir, out, inputs) = (args.next().unwrap(), args.next().unwrap(), args.next().unwrap());
            dump(&dir, &out, &inputs, &args.collect::<Vec<_>>());
        }
        Some("runtime-time") => {
            let (dir, selection) = (args.next().unwrap(), args.next().unwrap());
            runtime_time(&dir, &selection, args.next().unwrap().parse().unwrap());
        }
        _ => panic!("classic_big survey|quality|rife|dump|runtime-time ..."),
    }
}
