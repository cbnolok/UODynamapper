//! Storage candidates for mobile animations, measured on every body of a Classic and an Enhanced client (results:
//! `docs/agent_notes/proposals/2026-11-01-mobile-anim-codec.md`).
//!
//! The unit is one body batch: every action, direction and frame of a body from one source (a Classic `anim*.mul`
//! or `AnimationFrame*.uop`, or all the Enhanced `AnimationFrame*.uop`). Frames are trimmed to their non-transparent
//! box and stored back to back per direction, in frame order. Classic pixels are palette indices (one byte, 0
//! transparent: no shipped pixel uses index 0), Enhanced pixels an index and a coverage byte (0..=16 sixteenths).
//! Frame metadata (size, anchor, palette) is left out of every payload: it is the same for all of them.
//!
//! 1. Sizes of every body for each candidate, from all bodies in parallel: BC7 + Zstd as today's packer stores it
//!    (frame by frame, padded to whole blocks, at the container's Zstd level; the packer's atlas gutters are not
//!    counted), RGBA8 + Zstd, the indexed pixels per frame and per batch at several levels (also with long-distance
//!    matching), an 8x8 block copy ("same as the previous frame of the direction at the same anchor-relative
//!    place") and a difference from the previous frame. Also the palettes, deduplicated.
//! 2. Load cost of a sample of bodies on one thread: Zstd plus reconstruction, and BC7 encoding at runtime from the
//!    indexed pixels with both encoder front-ends, no RDO.
//! 3. The Enhanced palettes and coverage, and whether a body fits 8 bits per pixel.
//! 4. Upscaling at 2x before conversion on a sample of bodies: palette-preserving filters kept indexed, colour-making
//!    ones requantized (with SSIMULACRA2 against the unquantized upscale), stored as RGBA or as BC7.
//! 5. Motion between neighbouring frames: how often an 8x8 block is an exact integer shift of the previous frame.
//!
//! Nothing is written to disk. Run with `--release`:
//! `UO_CC_CLIENT_DIR=<Classic client> UO_EC_CLIENT_DIR=<Enhanced client> cargo test --release -p udd-conv --test
//! integration mobile_anim_codec:: -- --ignored --nocapture`; `MOBILE_ANIM_CODEC_STRIDE=n` measures every n-th body only,
//! `MOBILE_ANIM_CODEC_PASSES=timing` (or `sizes`, `upscale`, `shrink`, comma-separated) runs only those passes. The timing pass
//! prints the CPU state (turbo boost, `/sys/devices/system/cpu/cpufreq/boost`, and the CPUs' energy preferences) and
//! discards its timings when the state changed during the pass.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use image_postprocess::upscaling::{epx::apply_epx, hqx::apply_hqx, mmpx::apply_mmpx, xbrz::apply_xbrz};
use rayon::prelude::*;
use ssimulacra2::{compute_frame_ssimulacra2, ColorPrimaries, Rgb, TransferCharacteristic};
use udd_conv::bc7::{encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};
use uocf::classic::anim::{classic_animation_identity_from_source_index, MAX_ANIM_FILES};
use uocf::classic::anim_frames::mul::{ClassicMulEntry, INDEX_ENTRY_BYTES};
use uocf::classic::anim_frames::uop::{frame_box, ClassicUopEntry};
use uocf::classic::anim_frames::{ClassicFrame, ClassicPalette};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::enhanced::animationframe::encode::alpha_of_coverage;
use uocf::enhanced::animationframe::{
    index_frames, AnimationFrame, RgbaFrame, COVERAGE_SOLID, COVERAGE_TRANSPARENT, HUE_MASK_HUEABLE,
    HUE_MASK_KEEP_COLOUR, MAX_ACTION_ID_FALLBACK, MAX_BODY_ID, PALETTE_KEEP_COLOUR_BYTE,
};
use uocf::uop_container::package::{LoadMode, UopPackage};
use zstd::zstd_safe::CParameter;

mod shrink;

const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
/// When set to `n`, only every `n`-th body batch is measured (a quick run).
const BODY_STRIDE_ENV: &str = "MOBILE_ANIM_CODEC_STRIDE";
/// Comma-separated passes to run, of [`ALL_PASSES`]; [`DEFAULT_PASSES`] when unset.
const PASSES_ENV: &str = "MOBILE_ANIM_CODEC_PASSES";
const SIZES_PASS: &str = "sizes";
const TIMING_PASS: &str = "timing";
/// Upscaling and motion.
const UPSCALE_PASS: &str = "upscale";
/// Only on request, as it takes hours: see the `shrink` module.
const SHRINK_PASS: &str = "shrink";
const DEFAULT_PASSES: [&str; 3] = [SIZES_PASS, TIMING_PASS, UPSCALE_PASS];
const ALL_PASSES: [&str; 4] = [SIZES_PASS, TIMING_PASS, UPSCALE_PASS, SHRINK_PASS];
/// CPU turbo boost (`1` on, `0` off) and each CPU's energy preference, read at the start and at the end of the
/// timing pass.
const BOOST_PATH: &str = "/sys/devices/system/cpu/cpufreq/boost";
const CPU_DIR: &str = "/sys/devices/system/cpu";
const EPP_FILE: &str = "cpufreq/energy_performance_preference";
/// The load averages, printed with the CPU state (other work on the machine slows the one-thread timings).
const LOADAVG_PATH: &str = "/proc/loadavg";
/// `AnimationFrame1.uop` .. `AnimationFrame6.uop`.
const ANIMATION_FRAME_FILE_COUNT: u32 = 6;
/// Classic `AnimationFrame*.uop` entries are found by name: bodies below this.
const CC_UOP_BODY_LIMIT: u32 = 4096;
/// Directions stored per action (3..7 of the eight; the others are mirrored).
const STORED_DIRECTIONS: u16 = 5;
/// The container's Zstd level (`CompressionFlag::ZstdNoDict`), which today's packer uses.
const PACKER_ZSTD_LEVEL: i32 = 7;
const BATCH_ZSTD_LEVELS: [i32; 3] = [3, 9, 19];
const FAST_ZSTD_LEVEL: i32 = 9;
const BEST_ZSTD_LEVEL: i32 = 19;
/// Window of the long-distance matching variant (128 MiB, the decoder's default limit).
const LDM_WINDOW_LOG: u32 = 27;
const COPY_BLOCK: i32 = 8;
const RGBA_CHANNELS: usize = 4;
const ALPHA: usize = 3;
const OPAQUE: u8 = u8::MAX;
const CC_PIXEL_BYTES: usize = 1;
const EC_PIXEL_BYTES: usize = 2;
const MAX_PIXEL_BYTES: usize = EC_PIXEL_BYTES;
/// Per stored frame: width, height, left, top, palette (u16 each) and a u32 payload offset.
const FRAME_METADATA_BYTES: u64 = 14;
const EC_PALETTE_ENTRY_BYTES: usize = 4;
const PALETTE_COLOURS: usize = 256;
/// Bodies of each client whose load cost is timed, evenly spread over the bodies sorted by size.
const TIMING_SAMPLE_PER_CLIENT: usize = 200;
/// Timed bodies prepared (compressed in parallel) at a time.
const TIMING_CHUNK: usize = 20;
/// Timed runs per measurement; the fastest is kept.
const TIMING_RUNS: usize = 3;
/// Bodies of each client upscaled and searched for motion, evenly spread over the bodies sorted by size.
const UPSCALE_SAMPLE_PER_CLIENT: usize = 25;
const UPSCALE: u32 = 2;
/// Frames of a body in its SSIMULACRA2 mosaic, evenly spread.
const SSIM_FRAMES_PER_BODY: usize = 64;
const SSIM_MOSAIC_WIDTH: u32 = 1024;
const SSIM_MOSAIC_MIN_SIDE: u32 = 64;
const SSIM_MOSAIC_GAP: u32 = 4;
const SSIM_BACKGROUND: [u8; 3] = [128, 128, 128];
/// Largest block shift searched, per axis.
const MOTION_RADIUS: i32 = 16;
const PROGRESS_EVERY: usize = 500;
const BYTES_PER_MB: f64 = 1_000_000.0;
const BYTES_PER_KB: f64 = 1_000.0;
const MICROS_PER_SECOND: f64 = 1_000_000.0;
const PIXELS_PER_MEGAPIXEL: f64 = 1_000_000.0;
const PERCENT: f64 = 100.0;
const MEDIAN: f64 = 0.5;
const P95: f64 = 0.95;
const P05: f64 = 0.05;
/// 5-bit channel to 8 bits.
const RGB555_SHIFT: u32 = 3;
const RGB555_REPLICATE_SHIFT: u32 = 2;
const RGB555_CHANNEL_MASK: u16 = 0x1F;
const RGB555_RED_SHIFT: u32 = 10;
const RGB555_GREEN_SHIFT: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Client {
    Cc,
    Ec,
}

impl Client {
    fn label(self) -> &'static str {
        match self {
            Client::Cc => "Classic",
            Client::Ec => "Enhanced",
        }
    }

    fn pixel_bytes(self) -> usize {
        match self {
            Client::Cc => CC_PIXEL_BYTES,
            Client::Ec => EC_PIXEL_BYTES,
        }
    }
}

/// One trimmed frame.
#[derive(Clone)]
struct Frame {
    width: u32,
    height: u32,
    /// Anchor-relative place of pixel (0, 0).
    left: i32,
    top: i32,
    /// Classic: the index, 0 transparent. Enhanced: index and coverage per pixel.
    pixels: Vec<u8>,
    /// Straight-alpha RGBA8 as the client draws it unhued.
    rgba: Vec<u8>,
    /// Index into the body's palettes.
    palette: usize,
    action: u16,
}

impl Frame {
    fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// The pixel at anchor-relative `(x, y)`, `None` outside the frame (transparent).
    fn pixel_at(&self, bytes: usize, x: i32, y: i32) -> Option<&[u8]> {
        let fx = x - self.left;
        let fy = y - self.top;
        if fx < 0 || fy < 0 || fx >= self.width as i32 || fy >= self.height as i32 {
            return None;
        }
        let i = (fy as usize * self.width as usize + fx as usize) * bytes;
        Some(&self.pixels[i..i + bytes])
    }

    /// Anchor-aligned 8x8 blocks the frame touches, as block coordinates.
    fn blocks(&self) -> impl Iterator<Item = (i32, i32)> {
        let x0 = self.left.div_euclid(COPY_BLOCK);
        let y0 = self.top.div_euclid(COPY_BLOCK);
        let x1 = (self.left + self.width as i32 - 1).div_euclid(COPY_BLOCK);
        let y1 = (self.top + self.height as i32 - 1).div_euclid(COPY_BLOCK);
        (y0..=y1).flat_map(move |by| (x0..=x1).map(move |bx| (bx, by)))
    }
}

struct Body {
    label: String,
    client: Client,
    /// Classic: 256 RGB555 words; Enhanced: the colours, 4 bytes each.
    palettes: Vec<Vec<u8>>,
    directions: Vec<Vec<Frame>>,
}

impl Body {
    fn bytes(&self) -> usize {
        self.client.pixel_bytes()
    }

    fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.directions.iter().flatten()
    }

    fn frame_count(&self) -> usize {
        self.directions.iter().map(Vec::len).sum()
    }

    fn pixel_count(&self) -> u64 {
        self.frames().map(|frame| frame.pixel_count() as u64).sum()
    }
}

/// Where a body's frames are read from.
#[derive(Clone)]
enum BodySource {
    CcMul { file: usize, body: u16, entries: Vec<u32> },
    CcUop { file: usize, body: u32, actions: Vec<u16> },
    Ec { body: u32, entries: Vec<(usize, u64)> },
}

struct MulPair {
    name: String,
    idx: Vec<u8>,
    mul: Vec<u8>,
}

struct Sources {
    cc_muls: Vec<Option<MulPair>>,
    cc_uops: Vec<(String, UopPackage)>,
    ec_uops: Vec<(String, UopPackage)>,
}

fn find_ignoring_case(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

fn animation_frame_packages(dir: &Path) -> Vec<(String, UopPackage)> {
    (1..=ANIMATION_FRAME_FILE_COUNT)
        .filter_map(|n| {
            let name = format!("AnimationFrame{n}.uop");
            let path = find_ignoring_case(dir, &name)?;
            let package = UopPackage::load_with_mode(&path, LoadMode::Lazy).expect("load AnimationFrame package");
            Some((name, package))
        })
        .collect()
}

fn load_sources(cc_dir: &Path, ec_dir: &Path) -> Sources {
    let cc_muls = (0..MAX_ANIM_FILES)
        .map(|file| {
            let suffix = if file == 0 { String::new() } else { (file + 1).to_string() };
            let idx = find_ignoring_case(cc_dir, &format!("anim{suffix}.idx"))?;
            let mul = find_ignoring_case(cc_dir, &format!("anim{suffix}.mul"))?;
            Some(MulPair {
                name: format!("anim{suffix}.mul"),
                idx: std::fs::read(idx).expect("read anim idx"),
                mul: std::fs::read(mul).expect("read anim mul"),
            })
        })
        .collect();
    Sources { cc_muls, cc_uops: animation_frame_packages(cc_dir), ec_uops: animation_frame_packages(ec_dir) }
}

/// The `(lookup, length)` of a MUL index entry, `None` when empty or past the data.
fn mul_range(pair: &MulPair, index: u32) -> Option<(usize, usize)> {
    let start = index as usize * INDEX_ENTRY_BYTES;
    let field = |n: usize| {
        u32::from_le_bytes(pair.idx[start + n * 4..start + n * 4 + 4].try_into().unwrap())
    };
    let (lookup, length) = (field(0), field(1));
    if lookup == u32::MAX || length == 0 || lookup as usize + length as usize > pair.mul.len() {
        return None;
    }
    Some((lookup as usize, length as usize))
}

/// Every body of both clients, with counts of the UOP entries found by name and of all entries.
fn enumerate_bodies(sources: &Sources) -> Vec<BodySource> {
    let mut bodies = Vec::new();
    for (file, pair) in sources.cc_muls.iter().enumerate() {
        let Some(pair) = pair else { continue };
        let mut by_body = BTreeMap::<u16, Vec<u32>>::new();
        for index in 0..(pair.idx.len() / INDEX_ENTRY_BYTES) as u32 {
            if mul_range(pair, index).is_some() {
                let identity = classic_animation_identity_from_source_index(file as u8, index);
                by_body.entry(identity.body_id).or_default().push(index);
            }
        }
        bodies.extend(by_body.into_iter().map(|(body, entries)| BodySource::CcMul { file, body, entries }));
    }
    for (file, (name, package)) in sources.cc_uops.iter().enumerate() {
        let present = package.iter_files().filter(|f| f.has_size()).count();
        let mut found = 0;
        for body in 0..CC_UOP_BODY_LIMIT {
            let actions: Vec<u16> = (0..MAX_ACTION_ID_FALLBACK)
                .filter(|&action| {
                    package
                        .get_file_by_hash(AnimationFrameCc::animationframe_hash(body, action))
                        .is_some_and(|f| f.has_size())
                })
                .collect();
            found += actions.len();
            if !actions.is_empty() {
                bodies.push(BodySource::CcUop { file, body, actions });
            }
        }
        println!("Classic {name}: {found} of {present} entries found by name");
    }
    let mut ec = BTreeMap::<u32, Vec<(usize, u64)>>::new();
    for (file, (name, package)) in sources.ec_uops.iter().enumerate() {
        let present = package.iter_files().filter(|f| f.has_size()).count();
        let mut found = 0;
        for body in 0..MAX_BODY_ID {
            for action in 0..MAX_ACTION_ID_FALLBACK {
                let hash = AnimationFrame::animationframe_hash(body, action);
                if package.get_file_by_hash(hash).is_some_and(|f| f.has_size()) {
                    found += 1;
                    ec.entry(body).or_default().push((file, hash));
                }
            }
        }
        println!("Enhanced {name}: {found} of {present} entries found by name");
    }
    bodies.extend(ec.into_iter().map(|(body, entries)| BodySource::Ec { body, entries }));
    bodies
}

/// The non-transparent box of a frame, cropped; `None` when every pixel is transparent.
fn trimmed(width: u32, height: u32, left: i32, top: i32, rgba: &[u8], pixels: &[u8], bytes: usize, palette: usize) -> Option<Frame> {
    let (w, h) = (width as usize, height as usize);
    let opaque = |x: usize, y: usize| rgba[(y * w + x) * RGBA_CHANNELS + ALPHA] != 0;
    let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| opaque(x, y))).collect();
    let (&y0, &y1) = (rows.first()?, rows.last()?);
    let x0 = (0..w).find(|&x| (y0..=y1).any(|y| opaque(x, y)))?;
    let x1 = (0..w).rev().find(|&x| (y0..=y1).any(|y| opaque(x, y)))?;
    let (tw, th) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut out_rgba = Vec::with_capacity(tw * th * RGBA_CHANNELS);
    let mut out_pixels = Vec::with_capacity(tw * th * bytes);
    for y in y0..=y1 {
        out_rgba.extend_from_slice(&rgba[(y * w + x0) * RGBA_CHANNELS..(y * w + x1 + 1) * RGBA_CHANNELS]);
        out_pixels.extend_from_slice(&pixels[(y * w + x0) * bytes..(y * w + x1 + 1) * bytes]);
    }
    Some(Frame {
        width: tw as u32,
        height: th as u32,
        left: left + x0 as i32,
        top: top + y0 as i32,
        pixels: out_pixels,
        rgba: out_rgba,
        palette,
        action: 0,
    })
}

fn classic_frame(frame: &ClassicFrame, palette: &ClassicPalette, palette_id: usize) -> Option<Frame> {
    if frame.is_empty() {
        return None;
    }
    let (x0, y0, _, _) = frame_box(frame).ok()?;
    let rgba = frame.to_rgba(palette).data;
    let pixels: Vec<u8> = frame.pixels.iter().map(|pixel| pixel.unwrap_or(0)).collect();
    trimmed(u32::from(frame.width), u32::from(frame.height), i32::from(x0), i32::from(y0), &rgba, &pixels, CC_PIXEL_BYTES, palette_id)
}

fn palette_bytes(palette: &ClassicPalette) -> Vec<u8> {
    palette.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// The direction of a frame of an entry whose highest frame number is `highest`: numbers split in
/// [`STORED_DIRECTIONS`] equal spans when they can, else one direction.
fn direction_of(frame_number: u16, highest: u16) -> u16 {
    if highest >= STORED_DIRECTIONS && highest % STORED_DIRECTIONS == 0 {
        frame_number.saturating_sub(1) / (highest / STORED_DIRECTIONS)
    } else {
        0
    }
}

fn load_body(sources: &Sources, source: &BodySource) -> Body {
    match source {
        BodySource::CcMul { file, body, entries } => {
            let pair = sources.cc_muls[*file].as_ref().unwrap();
            let mut palettes = Vec::new();
            let mut directions = Vec::new();
            for &index in entries {
                let (lookup, length) = mul_range(pair, index).unwrap();
                let Ok(entry) = ClassicMulEntry::parse(&pair.mul[lookup..lookup + length]) else { continue };
                let palette_id = palettes.len();
                palettes.push(palette_bytes(&entry.palette));
                let frames: Vec<Frame> = entry
                    .frames
                    .iter()
                    .flatten()
                    .filter_map(|frame| classic_frame(frame, &entry.palette, palette_id))
                    .map(|frame| Frame { action: classic_animation_identity_from_source_index(*file as u8, index).action_id, ..frame })
                    .collect();
                if !frames.is_empty() {
                    directions.push(frames);
                }
            }
            Body { label: format!("{} body {body}", pair.name), client: Client::Cc, palettes, directions }
        }
        BodySource::CcUop { file, body, actions } => {
            let (name, package) = &sources.cc_uops[*file];
            let mut palettes = Vec::new();
            let mut directions = Vec::new();
            for &action in actions {
                let hash = AnimationFrameCc::animationframe_hash(*body, action);
                let Ok(Some(payload)) = package.unpack_file_by_hash(hash) else { continue };
                let Ok(entry) = ClassicUopEntry::parse(&payload) else { continue };
                let palette_id = palettes.len();
                palettes.push(palette_bytes(&entry.palette));
                let highest = entry.frames.iter().map(|f| f.frame_number).max().unwrap_or(0);
                let mut by_direction = BTreeMap::<u16, Vec<(u16, Frame)>>::new();
                for frame in &entry.frames {
                    if let Some(trimmed) = classic_frame(&frame.frame, &entry.palette, palette_id) {
                        let trimmed = Frame { action, ..trimmed };
                        by_direction.entry(direction_of(frame.frame_number, highest)).or_default().push((frame.frame_number, trimmed));
                    }
                }
                directions.extend(by_direction.into_values().map(|mut frames| {
                    frames.sort_by_key(|(number, _)| *number);
                    frames.into_iter().map(|(_, frame)| frame).collect::<Vec<_>>()
                }));
            }
            Body { label: format!("{name} body {body}"), client: Client::Cc, palettes, directions }
        }
        BodySource::Ec { body, entries } => {
            let mut palettes = Vec::new();
            let mut directions = Vec::new();
            for &(file, hash) in entries {
                let package = &sources.ec_uops[file].1;
                let Ok(Some(payload)) = package.unpack_file_by_hash(hash) else { continue };
                let Ok(animation) = AnimationFrame::load(&payload) else { continue };
                let palette_id = palettes.len();
                palettes.push(animation.colours.iter().flatten().copied().collect::<Vec<u8>>());
                let highest = animation.frames.iter().map(|f| f.frame_number).max().unwrap_or(0);
                let mut by_direction = BTreeMap::<u16, Vec<(u16, Frame)>>::new();
                for entry in &animation.frames {
                    let (Ok(indexed), Ok(decoded)) = (animation.decode_frame_indexed(entry), animation.decode_frame(entry)) else {
                        continue;
                    };
                    if indexed.width == 0 || indexed.height == 0 {
                        continue;
                    }
                    let pixels: Vec<u8> = indexed.indices.iter().zip(&indexed.coverage).flat_map(|(&i, &c)| [i, c]).collect();
                    let frame = trimmed(
                        u32::from(indexed.width),
                        u32::from(indexed.height),
                        -i32::from(indexed.center_x),
                        -i32::from(indexed.center_y),
                        &decoded.data,
                        &pixels,
                        EC_PIXEL_BYTES,
                        palette_id,
                    );
                    if let Some(frame) = frame {
                        let frame = Frame { action: entry.action, ..frame };
                        by_direction.entry(direction_of(entry.frame_number, highest)).or_default().push((entry.frame_number, frame));
                    }
                }
                directions.extend(by_direction.into_values().map(|mut frames| {
                    frames.sort_by_key(|(number, _)| *number);
                    frames.into_iter().map(|(_, frame)| frame).collect::<Vec<_>>()
                }));
            }
            Body { label: format!("EC body {body}"), client: Client::Ec, palettes, directions }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Payloads

fn compress(data: &[u8], level: i32, long_distance: bool) -> Vec<u8> {
    let mut compressor = zstd::bulk::Compressor::new(level).expect("zstd compressor");
    if long_distance {
        compressor.set_parameter(CParameter::EnableLongDistanceMatching(true)).expect("zstd LDM");
        compressor.set_parameter(CParameter::WindowLog(LDM_WINDOW_LOG)).expect("zstd window");
    }
    compressor.compress(data).expect("zstd compress")
}

fn decompress(data: &[u8], capacity: usize) -> Vec<u8> {
    zstd::bulk::decompress(data, capacity).expect("zstd decompress")
}

fn batch_plane(body: &Body) -> Vec<u8> {
    body.frames().flat_map(|frame| frame.pixels.iter().copied()).collect()
}

/// Enhanced pixels as two planes: every index, then every coverage.
fn planar(plane: &[u8]) -> Vec<u8> {
    plane.iter().step_by(EC_PIXEL_BYTES).chain(plane.iter().skip(1).step_by(EC_PIXEL_BYTES)).copied().collect()
}

fn rgba_plane(body: &Body) -> Vec<u8> {
    body.frames().flat_map(|frame| frame.rgba.iter().copied()).collect()
}

fn encode_bc7(rgba: &[u8], width: u32, height: u32, backend: Bc7EncoderBackend) -> Vec<u8> {
    let extent = ImageExtent::new(width, height).expect("extent");
    encode_to_bc7(rgba, extent, RawImageFormat::Rgba8888, backend).expect("BC7 encode").into_blocks()
}

fn bc7_plane(body: &Body, backend: Bc7EncoderBackend) -> Vec<u8> {
    body.frames().flat_map(|frame| encode_bc7(&frame.rgba, frame.width, frame.height, backend)).collect()
}

fn block_matches(current: &Frame, previous: &Frame, bytes: usize, block: (i32, i32), shift: (i32, i32)) -> bool {
    const ZERO: [u8; MAX_PIXEL_BYTES] = [0; MAX_PIXEL_BYTES];
    let zero = &ZERO[..bytes];
    let (x0, y0) = (block.0 * COPY_BLOCK, block.1 * COPY_BLOCK);
    (y0..y0 + COPY_BLOCK).all(|y| {
        (x0..x0 + COPY_BLOCK).all(|x| {
            current.pixel_at(bytes, x, y).unwrap_or(zero) == previous.pixel_at(bytes, x + shift.0, y + shift.1).unwrap_or(zero)
        })
    })
}

/// Whether the non-transparent pixels of the block of `current` equal those of `previous` shifted by `shift`
/// (the transparent ones are not compared).
fn block_matches_opaque(current: &Frame, previous: &Frame, bytes: usize, block: (i32, i32), shift: (i32, i32)) -> bool {
    let (x0, y0) = (block.0 * COPY_BLOCK, block.1 * COPY_BLOCK);
    (y0..y0 + COPY_BLOCK).all(|y| {
        (x0..x0 + COPY_BLOCK).all(|x| match current.pixel_at(bytes, x, y) {
            Some(pixel) if pixel.iter().any(|&b| b != 0) => previous.pixel_at(bytes, x + shift.0, y + shift.1) == Some(pixel),
            _ => true,
        })
    })
}

/// The pixel range of block `block` inside `frame`, as frame rows `y` and columns `x0..x1`.
fn block_rows(frame: &Frame, block: (i32, i32)) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let bx0 = (block.0 * COPY_BLOCK - frame.left).max(0) as usize;
    let bx1 = ((block.0 + 1) * COPY_BLOCK - frame.left).min(frame.width as i32) as usize;
    let by0 = (block.1 * COPY_BLOCK - frame.top).max(0) as usize;
    let by1 = ((block.1 + 1) * COPY_BLOCK - frame.top).min(frame.height as i32) as usize;
    (by0..by1).map(move |y| (y, bx0, bx1))
}

/// Block copy: the first frame of a direction as is; each next one with the blocks equal to the previous frame at
/// the same anchor-relative place zeroed, then every frame's block flags (one bit per block, in order).
fn block_copy_payload(body: &Body) -> Vec<u8> {
    let bytes = body.bytes();
    let mut planes = Vec::new();
    let mut flags = Vec::new();
    let mut bit = 0usize;
    for direction in &body.directions {
        for (n, frame) in direction.iter().enumerate() {
            if n == 0 {
                planes.extend_from_slice(&frame.pixels);
                continue;
            }
            let previous = &direction[n - 1];
            let mut literal = frame.pixels.clone();
            for block in frame.blocks() {
                if bit % u8::BITS as usize == 0 {
                    flags.push(0u8);
                }
                if block_matches(frame, previous, bytes, block, (0, 0)) {
                    *flags.last_mut().unwrap() |= 1 << (bit % u8::BITS as usize);
                    for (y, x0, x1) in block_rows(frame, block) {
                        let row = y * frame.width as usize;
                        literal[(row + x0) * bytes..(row + x1) * bytes].fill(0);
                    }
                }
                bit += 1;
            }
            planes.extend_from_slice(&literal);
        }
    }
    planes.extend_from_slice(&flags);
    planes
}

/// Rebuilds the frames of a [`block_copy_payload`] (the body gives the frame boxes).
fn decode_block_copy(body: &Body, payload: &[u8]) -> Vec<u8> {
    let bytes = body.bytes();
    let plane_bytes = body.pixel_count() as usize * bytes;
    let (planes, flags) = payload.split_at(plane_bytes);
    let mut out = Vec::with_capacity(plane_bytes);
    let mut offset = 0usize;
    let mut bit = 0usize;
    for direction in &body.directions {
        let mut previous_start = 0usize;
        for (n, frame) in direction.iter().enumerate() {
            let start = out.len();
            let len = frame.pixel_count() * bytes;
            out.extend_from_slice(&planes[offset..offset + len]);
            offset += len;
            if n > 0 {
                let previous = &direction[n - 1];
                for block in frame.blocks() {
                    let copied = flags[bit / u8::BITS as usize] & (1 << (bit % u8::BITS as usize)) != 0;
                    bit += 1;
                    if !copied {
                        continue;
                    }
                    for (y, x0, x1) in block_rows(frame, block) {
                        for x in x0..x1 {
                            let (ax, ay) = (frame.left + x as i32, frame.top + y as i32);
                            let (px, py) = (ax - previous.left, ay - previous.top);
                            if px < 0 || py < 0 || px >= previous.width as i32 || py >= previous.height as i32 {
                                continue;
                            }
                            let source = previous_start + (py as usize * previous.width as usize + px as usize) * bytes;
                            let target = start + (y * frame.width as usize + x) * bytes;
                            out.copy_within(source..source + bytes, target);
                        }
                    }
                }
            }
            previous_start = start;
        }
    }
    out
}

/// Difference: each frame after the first of its direction minus the previous frame at the same anchor-relative
/// place (transparent outside it), byte by byte.
fn difference_payload(body: &Body) -> Vec<u8> {
    const ZERO: [u8; MAX_PIXEL_BYTES] = [0; MAX_PIXEL_BYTES];
    let bytes = body.bytes();
    let mut out = Vec::new();
    for direction in &body.directions {
        for (n, frame) in direction.iter().enumerate() {
            if n == 0 {
                out.extend_from_slice(&frame.pixels);
                continue;
            }
            let previous = &direction[n - 1];
            for y in 0..frame.height as i32 {
                for x in 0..frame.width as i32 {
                    let i = (y as usize * frame.width as usize + x as usize) * bytes;
                    let before = previous.pixel_at(bytes, frame.left + x, frame.top + y).unwrap_or(&ZERO[..bytes]);
                    out.extend(frame.pixels[i..i + bytes].iter().zip(before).map(|(a, b)| a.wrapping_sub(*b)));
                }
            }
        }
    }
    out
}

fn decode_difference(body: &Body, payload: &[u8]) -> Vec<u8> {
    let bytes = body.bytes();
    let mut out = payload.to_vec();
    let mut start = 0usize;
    for direction in &body.directions {
        let mut previous_start = 0usize;
        for (n, frame) in direction.iter().enumerate() {
            if n > 0 {
                let previous = &direction[n - 1];
                for y in 0..frame.height as i32 {
                    let py = frame.top + y - previous.top;
                    if py < 0 || py >= previous.height as i32 {
                        continue;
                    }
                    for x in 0..frame.width as i32 {
                        let px = frame.left + x - previous.left;
                        if px < 0 || px >= previous.width as i32 {
                            continue;
                        }
                        let target = start + (y as usize * frame.width as usize + x as usize) * bytes;
                        let source = previous_start + (py as usize * previous.width as usize + px as usize) * bytes;
                        for k in 0..bytes {
                            out[target + k] = out[target + k].wrapping_add(out[source + k]);
                        }
                    }
                }
            }
            previous_start = start;
            start += frame.pixel_count() * bytes;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Statistics

const VARIANTS: [&str; 17] = [
    "BC7 + zstd 7 (today's packer)",
    "RGBA8 + zstd 7",
    "RGBA8 + zstd 19",
    "indexed, per frame + zstd 9",
    "indexed, per frame + zstd 19",
    "indexed, batch + zstd 3",
    "indexed, batch + zstd 9",
    "indexed, batch + zstd 19",
    "indexed, batch + zstd 19 + LDM",
    "indexed, batch planar + zstd 19 (EC)",
    "indexed + 8x8 block copy + zstd 9",
    "indexed + 8x8 block copy + zstd 19",
    "indexed + difference + zstd 9",
    "indexed + difference + zstd 19",
    "EC index plane only + zstd 19 (coverage dropped: lower bound)",
    "indexed, per-action batch + zstd 19",
    "BC7, per-action batch + zstd 7",
];

#[derive(Default, Clone)]
struct EcStats {
    entry_palette_sizes: Vec<usize>,
    /// Distinct colours (RGB and flag) of all the body's palettes.
    body_colours: usize,
    coded_pixels: u64,
    partial_pixels: u64,
    /// Most distinct (index, coverage) pairs of one entry's frames.
    max_entry_pairs: usize,
    /// Distinct (RGB, flag, coverage) of the whole body.
    body_pairs: usize,
}

struct BodyStats {
    label: String,
    client: Client,
    source: usize,
    frames: u64,
    pixels: u64,
    bc7_bytes: u64,
    sizes: Vec<u64>,
    palettes: Vec<(u64, usize)>,
    ec: Option<EcStats>,
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn ec_stats(body: &Body) -> EcStats {
    let mut stats = EcStats { entry_palette_sizes: body.palettes.iter().map(|p| p.len() / EC_PALETTE_ENTRY_BYTES).collect(), ..EcStats::default() };
    stats.body_colours = body.palettes.iter().flat_map(|p| p.chunks_exact(EC_PALETTE_ENTRY_BYTES)).collect::<HashSet<_>>().len();
    let mut entry_pairs: HashMap<usize, HashSet<[u8; 2]>> = HashMap::new();
    let mut body_pairs: HashSet<[u8; 5]> = HashSet::new();
    for frame in body.frames() {
        let palette = &body.palettes[frame.palette];
        let pairs = entry_pairs.entry(frame.palette).or_default();
        for pixel in frame.pixels.chunks_exact(EC_PIXEL_BYTES) {
            let (index, coverage) = (pixel[0], pixel[1]);
            if coverage == COVERAGE_TRANSPARENT {
                continue;
            }
            stats.coded_pixels += 1;
            if coverage != COVERAGE_SOLID {
                stats.partial_pixels += 1;
            }
            pairs.insert([index, coverage]);
            let colour = &palette[usize::from(index) * EC_PALETTE_ENTRY_BYTES..][..EC_PALETTE_ENTRY_BYTES];
            body_pairs.insert([colour[0], colour[1], colour[2], colour[PALETTE_KEEP_COLOUR_BYTE], coverage]);
        }
    }
    stats.max_entry_pairs = entry_pairs.values().map(HashSet::len).max().unwrap_or(0);
    stats.body_pairs = body_pairs.len();
    stats
}

/// The frames of each action (in direction and frame order) with their index in [`Body::frames`].
fn action_groups(body: &Body) -> BTreeMap<u16, Vec<(usize, &Frame)>> {
    let mut groups = BTreeMap::<u16, Vec<(usize, &Frame)>>::new();
    for (i, frame) in body.frames().enumerate() {
        groups.entry(frame.action).or_default().push((i, frame));
    }
    groups
}

/// Per action: its indexed pixels and its BC7 blocks (`bc7_frames` in [`Body::frames`] order), back to back.
fn action_planes(body: &Body, bc7_frames: &[Vec<u8>]) -> Vec<(u16, Vec<u8>, Vec<u8>)> {
    action_groups(body)
        .into_iter()
        .map(|(action, frames)| {
            let pixels = frames.iter().flat_map(|(_, f)| f.pixels.iter().copied()).collect();
            let bc7 = frames.iter().flat_map(|&(i, _)| bc7_frames[i].iter().copied()).collect();
            (action, pixels, bc7)
        })
        .collect()
}

fn bc7_frames(body: &Body) -> Vec<Vec<u8>> {
    body.frames().map(|frame| encode_bc7(&frame.rgba, frame.width, frame.height, Bc7EncoderBackend::AnalyticalWide)).collect()
}

fn measure_body(body: &Body, source: usize) -> BodyStats {
    let plane = batch_plane(body);
    let rgba = rgba_plane(body);
    let bc7_frames = bc7_frames(body);
    let bc7: Vec<u8> = bc7_frames.concat();
    let per_frame = |level| body.frames().map(|frame| compress(&frame.pixels, level, false).len() as u64).sum::<u64>();
    let planar_plane = if body.client == Client::Ec { planar(&plane) } else { plane.clone() };
    let block_copy = block_copy_payload(body);
    let difference = difference_payload(body);
    let mut sizes = vec![
        compress(&bc7, PACKER_ZSTD_LEVEL, false).len() as u64,
        compress(&rgba, PACKER_ZSTD_LEVEL, false).len() as u64,
        compress(&rgba, BEST_ZSTD_LEVEL, false).len() as u64,
        per_frame(FAST_ZSTD_LEVEL),
        per_frame(BEST_ZSTD_LEVEL),
    ];
    sizes.extend(BATCH_ZSTD_LEVELS.iter().map(|&level| compress(&plane, level, false).len() as u64));
    sizes.push(compress(&plane, BEST_ZSTD_LEVEL, true).len() as u64);
    sizes.push(compress(&planar_plane, BEST_ZSTD_LEVEL, false).len() as u64);
    for payload in [&block_copy, &difference] {
        sizes.push(compress(payload, FAST_ZSTD_LEVEL, false).len() as u64);
        sizes.push(compress(payload, BEST_ZSTD_LEVEL, false).len() as u64);
    }
    let index_only: Vec<u8> = if body.client == Client::Ec { plane.iter().step_by(EC_PIXEL_BYTES).copied().collect() } else { plane.clone() };
    sizes.push(compress(&index_only, BEST_ZSTD_LEVEL, false).len() as u64);
    let actions = action_planes(body, &bc7_frames);
    sizes.push(actions.iter().map(|(_, pixels, _)| compress(pixels, BEST_ZSTD_LEVEL, false).len() as u64).sum());
    sizes.push(actions.iter().map(|(_, _, bc7)| compress(bc7, PACKER_ZSTD_LEVEL, false).len() as u64).sum());
    debug_assert_eq!(sizes.len(), VARIANTS.len());
    BodyStats {
        label: body.label.clone(),
        client: body.client,
        source,
        frames: body.frame_count() as u64,
        pixels: body.pixel_count(),
        bc7_bytes: bc7.len() as u64,
        sizes,
        palettes: body.palettes.iter().map(|p| (hash_bytes(p), p.len())).collect(),
        ec: (body.client == Client::Ec).then(|| ec_stats(body)),
    }
}

fn percentile<T: Copy + PartialOrd>(sorted: &[T], p: f64) -> T {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn sorted<T: Copy + PartialOrd>(mut values: Vec<T>) -> Vec<T> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_MB
}

fn kb(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_KB
}

/// `count` items evenly spread over `items` sorted by `key`.
fn spread<'a, T>(items: &'a [T], count: usize, key: impl Fn(&T) -> u64) -> Vec<&'a T> {
    let mut order: Vec<&T> = items.iter().collect();
    order.sort_by_key(|item| key(item));
    if order.len() <= count {
        return order;
    }
    (0..count).map(|i| order[(i * 2 + 1) * order.len() / (count * 2)]).collect()
}

fn print_size_tables(stats: &[BodyStats]) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&BodyStats> = stats.iter().filter(|s| s.client == client && s.pixels > 0).collect();
        let frames: u64 = bodies.iter().map(|s| s.frames).sum();
        let pixels: u64 = bodies.iter().map(|s| s.pixels).sum();
        println!(
            "\n### {} sizes: {} bodies, {frames} frames, {:.1} Mpx trimmed; frame metadata {:.1} MB ({FRAME_METADATA_BYTES} B per frame)\n",
            client.label(),
            bodies.len(),
            pixels as f64 / PIXELS_PER_MEGAPIXEL,
            mb(frames * FRAME_METADATA_BYTES)
        );
        println!("| variant | total MB | vs BC7 | per body median KB | p95 KB | max KB |");
        println!("|---|---:|---:|---:|---:|---:|");
        let bc7_total: u64 = bodies.iter().map(|s| s.sizes[0]).sum();
        for (v, name) in VARIANTS.iter().enumerate() {
            let per_body = sorted(bodies.iter().map(|s| s.sizes[v]).collect());
            let total: u64 = per_body.iter().sum();
            println!(
                "| {name} | {:.1} | {:.2} | {:.1} | {:.1} | {:.1} |",
                mb(total),
                total as f64 / bc7_total as f64,
                kb(percentile(&per_body, MEDIAN)),
                kb(percentile(&per_body, P95)),
                kb(*per_body.last().unwrap())
            );
        }
        let all: Vec<(u64, usize)> = bodies.iter().flat_map(|s| s.palettes.iter().copied()).collect();
        let unique: HashMap<u64, usize> = all.iter().copied().collect();
        let body_unique = sorted(bodies.iter().map(|s| s.palettes.iter().map(|p| p.0).collect::<HashSet<_>>().len() as u64).collect());
        println!(
            "\nPalettes: {} stored ({:.2} MB), {} distinct ({:.3} MB, {:.3} MB as RGBA8 256-entry rows); distinct per body median {}, p95 {}, max {}",
            all.len(),
            mb(all.iter().map(|p| p.1 as u64).sum()),
            unique.len(),
            mb(unique.values().map(|&len| len as u64).sum()),
            mb((unique.len() * PALETTE_COLOURS * RGBA_CHANNELS) as u64),
            percentile(&body_unique, MEDIAN),
            percentile(&body_unique, P95),
            body_unique.last().unwrap()
        );
        println!("\nResident bytes per body (pixels only):\n");
        println!("| format | total MB | median KB | p95 KB | max KB |");
        println!("|---|---:|---:|---:|---:|");
        let formats: [(&str, Box<dyn Fn(&BodyStats) -> u64>); 4] = [
            ("R8", Box::new(|s| s.pixels)),
            ("RG8", Box::new(|s| s.pixels * EC_PIXEL_BYTES as u64)),
            ("RGBA8", Box::new(|s| s.pixels * RGBA_CHANNELS as u64)),
            ("BC7", Box::new(|s| s.bc7_bytes)),
        ];
        for (name, bytes) in formats {
            let per_body = sorted(bodies.iter().map(|s| bytes(s)).collect());
            println!(
                "| {name} | {:.1} | {:.1} | {:.1} | {:.1} |",
                mb(per_body.iter().sum()),
                kb(percentile(&per_body, MEDIAN)),
                kb(percentile(&per_body, P95)),
                kb(*per_body.last().unwrap())
            );
        }
        let largest = bodies.iter().max_by_key(|s| s.pixels).unwrap();
        println!("\nLargest body: {} ({} frames, {:.1} Mpx)", largest.label, largest.frames, largest.pixels as f64 / PIXELS_PER_MEGAPIXEL);
    }
}

fn print_ec_tables(stats: &[BodyStats]) {
    let ec: Vec<&EcStats> = stats.iter().filter_map(|s| s.ec.as_ref()).filter(|e| e.coded_pixels > 0).collect();
    let entries = sorted(ec.iter().flat_map(|e| e.entry_palette_sizes.iter().map(|&n| n as u64)).collect());
    let body_colours = sorted(ec.iter().map(|e| e.body_colours as u64).collect());
    let coded: u64 = ec.iter().map(|e| e.coded_pixels).sum();
    let partial: u64 = ec.iter().map(|e| e.partial_pixels).sum();
    let partial_share = sorted(ec.iter().map(|e| e.partial_pixels as f64 / e.coded_pixels as f64).collect());
    println!("\n### Enhanced palettes and coverage ({} bodies, {} entries)\n", ec.len(), entries.len());
    println!(
        "Palette colours per entry: median {}, p95 {}, max {}; entries with 256: {:.1}%",
        percentile(&entries, MEDIAN),
        percentile(&entries, P95),
        entries.last().unwrap(),
        PERCENT * entries.iter().filter(|&&n| n as usize == PALETTE_COLOURS).count() as f64 / entries.len() as f64
    );
    println!(
        "Distinct colours of a whole body: median {}, p95 {}, max {}; bodies within 256: {:.1}%",
        percentile(&body_colours, MEDIAN),
        percentile(&body_colours, P95),
        body_colours.last().unwrap(),
        PERCENT * body_colours.iter().filter(|&&n| n as usize <= PALETTE_COLOURS).count() as f64 / body_colours.len() as f64
    );
    println!(
        "Partial-coverage pixels: {:.2}% of {} coded pixels; per body median {:.2}%, p95 {:.2}%, max {:.2}%",
        PERCENT * partial as f64 / coded as f64,
        coded,
        PERCENT * percentile(&partial_share, MEDIAN),
        PERCENT * percentile(&partial_share, P95),
        PERCENT * partial_share.last().unwrap()
    );
    let usable = PALETTE_COLOURS - 1;
    let entry_fit = ec.iter().filter(|e| e.max_entry_pairs <= usable).count();
    let body_fit = ec.iter().filter(|e| e.body_pairs <= usable).count();
    let body_pairs = sorted(ec.iter().map(|e| e.body_pairs as u64).collect());
    println!(
        "8 bits per pixel with (index, coverage) pairs as the palette, index 0 transparent: every entry of the body fits {usable} pairs in {:.1}% of bodies; one palette for the whole body fits in {:.1}% (distinct pairs per body median {}, p95 {}, max {})",
        PERCENT * entry_fit as f64 / ec.len() as f64,
        PERCENT * body_fit as f64 / ec.len() as f64,
        percentile(&body_pairs, MEDIAN),
        percentile(&body_pairs, P95),
        body_pairs.last().unwrap()
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Load cost

fn fastest<T>(mut run: impl FnMut() -> T) -> Duration {
    (0..TIMING_RUNS)
        .map(|_| {
            let start = Instant::now();
            black_box(run());
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// RGBA of every palette entry (and, for the Enhanced, every coverage), for the runtime expansion.
fn palette_lut(body: &Body, palette: usize) -> Vec<[u8; RGBA_CHANNELS]> {
    let bytes = &body.palettes[palette];
    match body.client {
        Client::Cc => bytes
            .chunks_exact(2)
            .enumerate()
            .map(|(index, word)| {
                let word = u16::from_le_bytes([word[0], word[1]]);
                let channel = |shift: u32| {
                    let c = (word >> shift) & RGB555_CHANNEL_MASK;
                    ((c << RGB555_SHIFT) | (c >> RGB555_REPLICATE_SHIFT)) as u8
                };
                let alpha = if index == 0 { 0 } else { OPAQUE };
                [channel(RGB555_RED_SHIFT), channel(RGB555_GREEN_SHIFT), channel(0), alpha]
            })
            .collect(),
        Client::Ec => bytes.chunks_exact(EC_PALETTE_ENTRY_BYTES).map(|c| [c[0], c[1], c[2], OPAQUE]).collect(),
    }
}

/// The indexed pixels of one frame to RGBA8 through its palette.
fn expand(body: &Body, frame: &Frame, pixels: &[u8], lut: &[[u8; RGBA_CHANNELS]], out: &mut Vec<u8>) {
    out.clear();
    match body.client {
        Client::Cc => out.extend(pixels.iter().flat_map(|&i| lut[usize::from(i)])),
        Client::Ec => out.extend(pixels.chunks_exact(EC_PIXEL_BYTES).flat_map(|p| {
            let [r, g, b, _] = lut.get(usize::from(p[0])).copied().unwrap_or_default();
            if p[1] == COVERAGE_TRANSPARENT { [0; RGBA_CHANNELS] } else { [r, g, b, alpha_of_coverage(p[1])] }
        })),
    }
    debug_assert_eq!(out.len(), frame.pixel_count() * RGBA_CHANNELS);
}

const TIMED: [&str; 9] = [
    "BC7 + zstd 7 (zstd only)",
    "RGBA8 + zstd 7",
    "indexed, per frame + zstd 9",
    "indexed, batch + zstd 9",
    "indexed, batch + zstd 19",
    "indexed, batch + zstd 19 + LDM",
    "indexed + block copy + zstd 19 (zstd + rebuild)",
    "indexed + difference + zstd 19 (zstd + rebuild)",
    "indexed batch -> RGBA8 expand (no zstd)",
];
const BC7_BACKENDS: [(Bc7EncoderBackend, &str); 2] =
    [(Bc7EncoderBackend::Analytical, "Analytical"), (Bc7EncoderBackend::AnalyticalWide, "AnalyticalWide")];

/// The compressed payloads of one timed body, made in parallel before the one-thread timing.
struct Prepared {
    bc7: (Vec<u8>, usize),
    rgba: (Vec<u8>, usize),
    frames: Vec<(Vec<u8>, usize)>,
    batches: Vec<Vec<u8>>,
    block_copy: (Vec<u8>, usize),
    difference: (Vec<u8>, usize),
    plane_len: usize,
    luts: Vec<Vec<[u8; RGBA_CHANNELS]>>,
    /// Per action: indexed zstd 19 and its length, BC7 zstd 7 and its length.
    actions: Vec<(u16, (Vec<u8>, usize), (Vec<u8>, usize))>,
}

fn prepare_body(body: &Body) -> Prepared {
    let plane = batch_plane(body);
    let rgba = rgba_plane(body);
    let bc7_frames = bc7_frames(body);
    let bc7: Vec<u8> = bc7_frames.concat();
    let actions = action_planes(body, &bc7_frames)
        .into_iter()
        .map(|(action, pixels, bc7)| {
            (
                action,
                (compress(&pixels, BEST_ZSTD_LEVEL, false), pixels.len()),
                (compress(&bc7, PACKER_ZSTD_LEVEL, false), bc7.len()),
            )
        })
        .collect();
    let block_copy = block_copy_payload(body);
    let block_z = compress(&block_copy, BEST_ZSTD_LEVEL, false);
    assert!(decode_block_copy(body, &decompress(&block_z, block_copy.len())) == plane, "{}: block copy does not rebuild the frames", body.label);
    let difference = difference_payload(body);
    assert!(decode_difference(body, &difference) == plane, "{}: the difference does not rebuild the frames", body.label);
    Prepared {
        bc7: (compress(&bc7, PACKER_ZSTD_LEVEL, false), bc7.len()),
        rgba: (compress(&rgba, PACKER_ZSTD_LEVEL, false), rgba.len()),
        frames: body.frames().map(|f| (compress(&f.pixels, FAST_ZSTD_LEVEL, false), f.pixels.len())).collect(),
        batches: [(FAST_ZSTD_LEVEL, false), (BEST_ZSTD_LEVEL, false), (BEST_ZSTD_LEVEL, true)]
            .into_iter()
            .map(|(level, ldm)| compress(&plane, level, ldm))
            .collect(),
        block_copy: (block_z, block_copy.len()),
        difference: (compress(&difference, BEST_ZSTD_LEVEL, false), difference.len()),
        plane_len: plane.len(),
        luts: (0..body.palettes.len()).map(|p| palette_lut(body, p)).collect(),
        actions,
    }
}

struct Timing {
    client: Client,
    /// Per [`TIMED`]: time and decoded bytes.
    decode: Vec<(Duration, u64)>,
    /// Per [`BC7_BACKENDS`]: expand plus encode.
    bc7: Vec<Duration>,
    pixels: u64,
    /// Compressed bytes of the whole body (indexed zstd 19, BC7 zstd 7).
    body_bytes: (u64, u64),
    /// Per action: compressed bytes and decode time, indexed zstd 19 and BC7 zstd 7; lowest action first.
    actions: Vec<((u64, Duration), (u64, Duration))>,
}

fn time_body(body: &Body, prepared: &Prepared) -> Timing {
    let plane_len = prepared.plane_len;
    let mut decode = Vec::new();
    let (bc7_z, bc7_len) = &prepared.bc7;
    decode.push((fastest(|| decompress(bc7_z, *bc7_len)), *bc7_len as u64));
    let (rgba_z, rgba_len) = &prepared.rgba;
    decode.push((fastest(|| decompress(rgba_z, *rgba_len)), *rgba_len as u64));
    decode.push((fastest(|| prepared.frames.iter().map(|(z, len)| decompress(z, *len).len()).sum::<usize>()), plane_len as u64));
    for z in &prepared.batches {
        decode.push((fastest(|| decompress(z, plane_len)), plane_len as u64));
    }
    let (block_z, block_len) = &prepared.block_copy;
    decode.push((fastest(|| decode_block_copy(body, &decompress(block_z, *block_len))), plane_len as u64));
    let (difference_z, difference_len) = &prepared.difference;
    decode.push((fastest(|| decode_difference(body, &decompress(difference_z, *difference_len))), plane_len as u64));
    let actions = prepared
        .actions
        .iter()
        .map(|(_, (indexed_z, indexed_len), (bc7_z, bc7_len))| {
            (
                (indexed_z.len() as u64, fastest(|| decompress(indexed_z, *indexed_len))),
                (bc7_z.len() as u64, fastest(|| decompress(bc7_z, *bc7_len))),
            )
        })
        .collect();
    let body_bytes = (prepared.batches[1].len() as u64, prepared.bc7.0.len() as u64);
    let luts = &prepared.luts;
    let mut scratch = Vec::new();
    decode.push((
        fastest(|| {
            for frame in body.frames() {
                expand(body, frame, &frame.pixels, &luts[frame.palette], &mut scratch);
                black_box(&scratch);
            }
        }),
        *rgba_len as u64,
    ));
    let bc7 = BC7_BACKENDS
        .iter()
        .map(|&(backend, _)| {
            let start = Instant::now();
            for frame in body.frames() {
                expand(body, frame, &frame.pixels, &luts[frame.palette], &mut scratch);
                black_box(encode_bc7(&scratch, frame.width, frame.height, backend));
            }
            start.elapsed()
        })
        .collect();
    Timing { client: body.client, decode, bc7, pixels: body.pixel_count(), body_bytes, actions }
}

/// The CPU boost state and the energy preferences of the CPUs, as a label: `boost off, EPP power`.
fn cpu_state() -> String {
    let boost = match std::fs::read_to_string(BOOST_PATH).as_deref().map(str::trim) {
        Ok("1") => "boost on",
        Ok("0") => "boost off",
        _ => "boost unknown",
    };
    let preferences: std::collections::BTreeSet<String> = std::fs::read_dir(CPU_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_prefix("cpu").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path().join(EPP_FILE)).ok())
        .map(|preference| preference.trim().to_string())
        .collect();
    let preferences: Vec<String> = preferences.into_iter().collect();
    match preferences.len() {
        0 => format!("{boost}, EPP unknown"),
        _ => format!("{boost}, EPP {}", preferences.join("/")),
    }
}

/// Times the load of the `sample` bodies on one thread ([`time_body`], payloads made in parallel beforehand); the
/// timings are discarded when the CPU boost state changed meanwhile.
fn time_sample(sources: &Sources, bodies: &[BodySource], sample: &[usize]) {
    let state = cpu_state();
    let load = || std::fs::read_to_string(LOADAVG_PATH).unwrap_or_default().trim().to_string();
    println!("\nCPU {state} at the start of the timing pass; load average {}", load());
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let mut timings = Vec::new();
    for chunk in sample.chunks(TIMING_CHUNK) {
        let prepared: Vec<(Body, Prepared)> = chunk
            .par_iter()
            .map(|&i| {
                let body = load_body(sources, &bodies[i]);
                let prepared = prepare_body(&body);
                (body, prepared)
            })
            .collect();
        timings.extend(single.install(|| prepared.iter().map(|(body, prepared)| time_body(body, prepared)).collect::<Vec<_>>()));
    }
    println!("Load average at the end of the timing pass {}", load());
    let state_end = cpu_state();
    if state_end != state {
        println!("\nThe CPU state changed during the timing pass ({state} at the start, {state_end} at the end): timings discarded");
        return;
    }
    print_timing_tables(&timings, &state);
}

fn print_timing_tables(timings: &[Timing], state: &str) {
    for client in [Client::Cc, Client::Ec] {
        let rows: Vec<&Timing> = timings.iter().filter(|t| t.client == client).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n### {} load cost, one thread, {} bodies, CPU {state}\n", client.label(), rows.len());
        println!("| step | per body median µs | p95 µs | max µs | MB/s of output |");
        println!("|---|---:|---:|---:|---:|");
        for (i, name) in TIMED.iter().enumerate() {
            let micros = sorted(rows.iter().map(|t| t.decode[i].0.as_secs_f64() * MICROS_PER_SECOND).collect());
            let bytes: u64 = rows.iter().map(|t| t.decode[i].1).sum();
            let seconds: f64 = rows.iter().map(|t| t.decode[i].0.as_secs_f64()).sum();
            println!(
                "| {name} | {:.0} | {:.0} | {:.0} | {:.0} |",
                percentile(&micros, MEDIAN),
                percentile(&micros, P95),
                micros.last().unwrap(),
                bytes as f64 / BYTES_PER_MB / seconds
            );
        }
        let pixels: u64 = rows.iter().map(|t| t.pixels).sum();
        for (b, (_, name)) in BC7_BACKENDS.iter().enumerate() {
            let micros = sorted(rows.iter().map(|t| t.bc7[b].as_secs_f64() * MICROS_PER_SECOND).collect());
            let seconds: f64 = rows.iter().map(|t| t.bc7[b].as_secs_f64()).sum();
            println!(
                "| runtime BC7 ({name}): expand + encode | {:.0} | {:.0} | {:.0} | {:.1} Mpx/s |",
                percentile(&micros, MEDIAN),
                percentile(&micros, P95),
                micros.last().unwrap(),
                pixels as f64 / PIXELS_PER_MEGAPIXEL / seconds
            );
        }
        println!("\nLoad unit, {} bodies, CPU {state}: the first frame is drawable once the batch holding it is decoded (the body's lowest action for a per-action batch).\n", rows.len());
        println!("| unit | stored as | compressed MB | per batch decode median µs | p95 µs | first frame drawable median µs | p95 µs | max µs |");
        println!("|---|---|---:|---:|---:|---:|---:|---:|");
        // (label, body decode index in TIMED, body bytes, per-action selector)
        let units: [(&str, usize, fn(&Timing) -> u64, fn(&((u64, Duration), (u64, Duration))) -> (u64, Duration)); 2] = [
            ("indexed zstd 19", 4, |t| t.body_bytes.0, |a| a.0),
            ("BC7 zstd 7", 0, |t| t.body_bytes.1, |a| a.1),
        ];
        let micros = |d: Duration| d.as_secs_f64() * MICROS_PER_SECOND;
        for (name, timed, body_bytes, select) in units {
            let body_decode = sorted(rows.iter().map(|t| micros(t.decode[timed].0)).collect());
            println!(
                "| body | {name} | {:.1} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
                mb(rows.iter().map(|t| body_bytes(t)).sum()),
                percentile(&body_decode, MEDIAN),
                percentile(&body_decode, P95),
                percentile(&body_decode, MEDIAN),
                percentile(&body_decode, P95),
                body_decode.last().unwrap()
            );
            let action_decode = sorted(rows.iter().flat_map(|t| t.actions.iter().map(|a| micros(select(a).1))).collect());
            let first = sorted(rows.iter().filter_map(|t| t.actions.first()).map(|a| micros(select(a).1)).collect());
            println!(
                "| action | {name} | {:.1} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
                mb(rows.iter().flat_map(|t| t.actions.iter().map(|a| select(a).0)).sum()),
                percentile(&action_decode, MEDIAN),
                percentile(&action_decode, P95),
                percentile(&first, MEDIAN),
                percentile(&first, P95),
                first.last().unwrap()
            );
        }
        let batch19 = rows.iter().map(|t| t.decode[4].0.as_secs_f64()).sum::<f64>();
        let wide = rows.iter().map(|t| t.bc7[1].as_secs_f64()).sum::<f64>();
        println!("\nRuntime BC7 (AnalyticalWide) costs {:.0}x the batch zstd 19 decode.", wide / batch19);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Upscaling and motion

fn nearest(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (width * scale, height * scale);
    let mut out = Vec::with_capacity((w * h) as usize * RGBA_CHANNELS);
    for y in 0..h {
        for x in 0..w {
            let i = ((y / scale) * width + x / scale) as usize * RGBA_CHANNELS;
            out.extend_from_slice(&rgba[i..i + RGBA_CHANNELS]);
        }
    }
    (w, h, out)
}

type Upscaler = fn(u32, u32, &[u8], u32) -> (u32, u32, Vec<u8>);
const PALETTE_FILTERS: [(&str, Upscaler); 3] = [("nearest", nearest), ("EPX (Scale2x)", apply_epx), ("MMPX", apply_mmpx)];
const COLOUR_FILTERS: [(&str, Upscaler); 2] = [("xBRZ", apply_xbrz), ("hqx", apply_hqx)];

/// One upscaled frame: its RGBA and the anchor-relative box.
struct Scaled {
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    rgba: Vec<u8>,
}

fn upscale_body(body: &Body, filter: Upscaler) -> Vec<Vec<Scaled>> {
    body.directions
        .iter()
        .map(|direction| {
            direction
                .iter()
                .map(|frame| {
                    let (width, height, rgba) = filter(frame.width, frame.height, &frame.rgba, UPSCALE);
                    Scaled { width, height, left: frame.left * UPSCALE as i32, top: frame.top * UPSCALE as i32, rgba }
                })
                .collect()
        })
        .collect()
}

/// A body made of the upscaled frames, its pixels mapped back to the source pixels of the same RGBA (palette
/// filters); returns it and the count of pixels with an RGBA no source pixel of that palette has.
fn reindexed(body: &Body, scaled: &[Vec<Scaled>]) -> (Body, u64) {
    let bytes = body.bytes();
    let mut maps: HashMap<usize, HashMap<[u8; RGBA_CHANNELS], [u8; MAX_PIXEL_BYTES]>> = HashMap::new();
    for frame in body.frames() {
        let map = maps.entry(frame.palette).or_default();
        for (rgba, pixel) in frame.rgba.chunks_exact(RGBA_CHANNELS).zip(frame.pixels.chunks_exact(bytes)) {
            let mut key = [0; MAX_PIXEL_BYTES];
            key[..bytes].copy_from_slice(pixel);
            map.entry(rgba.try_into().unwrap()).or_insert(key);
        }
    }
    let mut unmapped = 0;
    let directions = body
        .directions
        .iter()
        .zip(scaled)
        .map(|(direction, scaled)| {
            direction
                .iter()
                .zip(scaled)
                .map(|(frame, s)| {
                    let map = &maps[&frame.palette];
                    let pixels = s
                        .rgba
                        .chunks_exact(RGBA_CHANNELS)
                        .flat_map(|rgba| {
                            let pixel = if rgba[ALPHA] == 0 {
                                [0; MAX_PIXEL_BYTES]
                            } else {
                                map.get(rgba).copied().unwrap_or_else(|| {
                                    unmapped += 1;
                                    [0; MAX_PIXEL_BYTES]
                                })
                            };
                            pixel.into_iter().take(bytes)
                        })
                        .collect();
                    Frame {
                        width: s.width,
                        height: s.height,
                        left: s.left,
                        top: s.top,
                        pixels,
                        rgba: s.rgba.clone(),
                        palette: frame.palette,
                        action: frame.action,
                    }
                })
                .collect()
        })
        .collect();
    (Body { label: body.label.clone(), client: body.client, palettes: body.palettes.clone(), directions }, unmapped)
}

/// The frames of a body as one image over [`SSIM_BACKGROUND`], at most [`SSIM_FRAMES_PER_BODY`] of them.
fn mosaic(frames: &[(u32, u32, &[u8])]) -> Rgb {
    let step = frames.len().div_ceil(SSIM_FRAMES_PER_BODY).max(1);
    let chosen: Vec<&(u32, u32, &[u8])> = frames.iter().step_by(step).collect();
    let mut places = Vec::new();
    let (mut x, mut y, mut row_height, mut width) = (0u32, 0u32, 0u32, SSIM_MOSAIC_MIN_SIDE);
    for &&(w, h, _) in &chosen {
        if x > 0 && x + w > SSIM_MOSAIC_WIDTH {
            x = 0;
            y += row_height + SSIM_MOSAIC_GAP;
            row_height = 0;
        }
        places.push((x, y));
        x += w + SSIM_MOSAIC_GAP;
        row_height = row_height.max(h);
        width = width.max(x);
    }
    let height = (y + row_height).max(SSIM_MOSAIC_MIN_SIDE);
    let background = SSIM_BACKGROUND.map(|c| f32::from(c) / f32::from(OPAQUE));
    let mut data = vec![background; (width * height) as usize];
    for (&&(w, h, rgba), &(px, py)) in chosen.iter().zip(&places) {
        for fy in 0..h {
            for fx in 0..w {
                let p = &rgba[(fy * w + fx) as usize * RGBA_CHANNELS..][..RGBA_CHANNELS];
                let a = f32::from(p[ALPHA]) / f32::from(OPAQUE);
                let target = &mut data[((py + fy) * width + px + fx) as usize];
                for c in 0..3 {
                    target[c] = background[c] * (1.0 - a) + f32::from(p[c]) / f32::from(OPAQUE) * a;
                }
            }
        }
    }
    Rgb::new(data, width as usize, height as usize, TransferCharacteristic::SRGB, ColorPrimaries::BT709).expect("mosaic")
}

/// The source hue mask of a frame, upscaled by nearest neighbour (Enhanced: the palette's keep-colour flag).
fn scaled_hue_mask(body: &Body, frame: &Frame, scaled: &Scaled) -> Option<Vec<u8>> {
    if body.client == Client::Cc {
        return None;
    }
    let palette = &body.palettes[frame.palette];
    Some(
        (0..scaled.height)
            .flat_map(|y| (0..scaled.width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let i = ((y / UPSCALE) * frame.width + x / UPSCALE) as usize * EC_PIXEL_BYTES;
                let index = usize::from(frame.pixels[i]);
                let keep = palette.get(index * EC_PALETTE_ENTRY_BYTES + PALETTE_KEEP_COLOUR_BYTE).copied().unwrap_or(0);
                if keep == 0 { HUE_MASK_HUEABLE } else { HUE_MASK_KEEP_COLOUR }
            })
            .collect(),
    )
}

struct UpscaleResult {
    client: Client,
    label: String,
    /// Indexed batch zstd 19 at 1x, BC7 + zstd 7 at 1x, RGBA8 + zstd 19 at 1x.
    base: [u64; 3],
    /// (filter, indexed batch zstd 19, block copy zstd 19, unmapped pixels).
    palette: Vec<(&'static str, u64, u64, u64)>,
    /// (filter, requantized RG8 batch zstd 19 + palette, SSIMULACRA2, RGBA8 zstd 19, BC7 zstd 7, partial share).
    colour: Vec<(&'static str, u64, f64, u64, u64, f64)>,
}

fn upscale_measure(body: &Body) -> UpscaleResult {
    let base = [
        compress(&batch_plane(body), BEST_ZSTD_LEVEL, false).len() as u64,
        compress(&bc7_plane(body, Bc7EncoderBackend::AnalyticalWide), PACKER_ZSTD_LEVEL, false).len() as u64,
        compress(&rgba_plane(body), BEST_ZSTD_LEVEL, false).len() as u64,
    ];
    let palette = PALETTE_FILTERS
        .iter()
        .map(|&(name, filter)| {
            let (scaled, unmapped) = reindexed(body, &upscale_body(body, filter));
            (
                name,
                compress(&batch_plane(&scaled), BEST_ZSTD_LEVEL, false).len() as u64,
                compress(&block_copy_payload(&scaled), BEST_ZSTD_LEVEL, false).len() as u64,
                unmapped,
            )
        })
        .collect();
    let colour = COLOUR_FILTERS
        .iter()
        .map(|&(name, filter)| {
            let scaled = upscale_body(body, filter);
            let pairs: Vec<(&Frame, &Scaled)> = body.frames().zip(scaled.iter().flatten()).collect();
            let inputs: Vec<RgbaFrame> = pairs
                .iter()
                .map(|(frame, s)| RgbaFrame {
                    width: s.width as u16,
                    height: s.height as u16,
                    center_x: 0,
                    center_y: 0,
                    rgba: s.rgba.clone(),
                    hue_mask: scaled_hue_mask(body, frame, s),
                })
                .collect();
            let (palette, indexed, _) = index_frames(&inputs, None).expect("requantize");
            let mut plane = Vec::new();
            let mut quantized = Vec::new();
            let (mut coded, mut partial) = (0u64, 0u64);
            for frame in &indexed {
                let mut rgba = Vec::with_capacity(frame.indices.len() * RGBA_CHANNELS);
                for (&index, &coverage) in frame.indices.iter().zip(&frame.coverage) {
                    plane.extend_from_slice(&[index, coverage]);
                    if coverage == COVERAGE_TRANSPARENT {
                        rgba.extend_from_slice(&[0; RGBA_CHANNELS]);
                        continue;
                    }
                    coded += 1;
                    partial += u64::from(coverage != COVERAGE_SOLID);
                    let [r, g, b, _] = palette[usize::from(index)];
                    rgba.extend_from_slice(&[r, g, b, alpha_of_coverage(coverage)]);
                }
                quantized.push(rgba);
            }
            let reference: Vec<(u32, u32, &[u8])> = pairs.iter().map(|(_, s)| (s.width, s.height, s.rgba.as_slice())).collect();
            let test: Vec<(u32, u32, &[u8])> =
                pairs.iter().zip(&quantized).map(|((_, s), q)| (s.width, s.height, q.as_slice())).collect();
            let score = compute_frame_ssimulacra2(mosaic(&reference), mosaic(&test)).expect("SSIMULACRA2");
            let rgba: Vec<u8> = pairs.iter().flat_map(|(_, s)| s.rgba.iter().copied()).collect();
            let bc7: Vec<u8> =
                pairs.iter().flat_map(|(_, s)| encode_bc7(&s.rgba, s.width, s.height, Bc7EncoderBackend::AnalyticalWide)).collect();
            (
                name,
                compress(&plane, BEST_ZSTD_LEVEL, false).len() as u64 + (palette.len() * EC_PALETTE_ENTRY_BYTES) as u64,
                score,
                compress(&rgba, BEST_ZSTD_LEVEL, false).len() as u64,
                compress(&bc7, PACKER_ZSTD_LEVEL, false).len() as u64,
                partial as f64 / coded.max(1) as f64,
            )
        })
        .collect();
    UpscaleResult { client: body.client, label: body.label.clone(), base, palette, colour }
}

fn print_upscale_tables(results: &[UpscaleResult]) {
    for client in [Client::Cc, Client::Ec] {
        let rows: Vec<&UpscaleResult> = results.iter().filter(|r| r.client == client).collect();
        if rows.is_empty() {
            continue;
        }
        let base: Vec<u64> = (0..3).map(|i| rows.iter().map(|r| r.base[i]).sum()).collect();
        println!(
            "\n### {} 2x upscale, {} bodies (1x: indexed batch zstd 19 {:.1} MB, BC7 + zstd 7 {:.1} MB, RGBA8 + zstd 19 {:.1} MB)\n",
            client.label(),
            rows.len(),
            mb(base[0]),
            mb(base[1]),
            mb(base[2])
        );
        println!("| filter | stored as | MB | x 1x indexed | x 1x same format | quality |");
        println!("|---|---|---:|---:|---:|---|");
        for (f, (name, _)) in PALETTE_FILTERS.iter().enumerate() {
            let batch: u64 = rows.iter().map(|r| r.palette[f].1).sum();
            let block: u64 = rows.iter().map(|r| r.palette[f].2).sum();
            let unmapped: u64 = rows.iter().map(|r| r.palette[f].3).sum();
            println!("| {name} | indexed, batch zstd 19 | {:.1} | {:.2} | {:.2} | exact ({unmapped} pixels off the palette) |", mb(batch), batch as f64 / base[0] as f64, batch as f64 / base[0] as f64);
            println!("| {name} | indexed + block copy zstd 19 | {:.1} | {:.2} | | |", mb(block), block as f64 / base[0] as f64);
        }
        for (f, (name, _)) in COLOUR_FILTERS.iter().enumerate() {
            let quantized: u64 = rows.iter().map(|r| r.colour[f].1).sum();
            let scores = sorted(rows.iter().map(|r| r.colour[f].2).collect());
            let partial = sorted(rows.iter().map(|r| r.colour[f].5).collect());
            let rgba: u64 = rows.iter().map(|r| r.colour[f].3).sum();
            let bc7: u64 = rows.iter().map(|r| r.colour[f].4).sum();
            println!(
                "| {name} | (i) requantized RG8, batch zstd 19 | {:.1} | {:.2} | | SSIMULACRA2 median {:.1}, p5 {:.1}, min {:.1}; partial coverage median {:.1}% |",
                mb(quantized),
                quantized as f64 / base[0] as f64,
                percentile(&scores, MEDIAN),
                percentile(&scores, P05),
                scores[0],
                PERCENT * percentile(&partial, MEDIAN)
            );
            println!("| {name} | (ii) RGBA8 zstd 19 | {:.1} | {:.2} | {:.2} | lossless |", mb(rgba), rgba as f64 / base[0] as f64, rgba as f64 / base[2] as f64);
            println!("| {name} | (iii) BC7 zstd 7 | {:.1} | {:.2} | {:.2} | BC7 |", mb(bc7), bc7 as f64 / base[0] as f64, bc7 as f64 / base[1] as f64);
        }
        let worst = rows.iter().min_by(|a, b| a.colour[0].2.partial_cmp(&b.colour[0].2).unwrap()).unwrap();
        println!("\nLowest xBRZ SSIMULACRA2: {} ({:.1})", worst.label, worst.colour[0].2);
    }
}

#[derive(Default)]
struct Motion {
    blocks: u64,
    zero: u64,
    shifted: u64,
    /// Euclidean length of each non-zero exact shift.
    lengths: Vec<f64>,
    /// Count per Chebyshev length 1..=MOTION_RADIUS.
    chebyshev: Vec<u64>,
    /// The same with only the block's non-transparent pixels compared.
    loose_zero: u64,
    loose_shifted: u64,
    loose_lengths: Vec<f64>,
}

fn motion(body: &Body) -> Motion {
    let bytes = body.bytes();
    let mut shifts: Vec<(i32, i32)> = (-MOTION_RADIUS..=MOTION_RADIUS)
        .flat_map(|dy| (-MOTION_RADIUS..=MOTION_RADIUS).map(move |dx| (dx, dy)))
        .filter(|&s| s != (0, 0))
        .collect();
    shifts.sort_by_key(|&(dx, dy)| (dx * dx + dy * dy, dy, dx));
    let parts: Vec<Motion> = body
        .directions
        .par_iter()
        .map(|direction| {
            let mut m = Motion { chebyshev: vec![0; MOTION_RADIUS as usize + 1], ..Motion::default() };
            for pair in direction.windows(2) {
                let (previous, current) = (&pair[0], &pair[1]);
                for block in current.blocks() {
                    let empty = block_rows(current, block).all(|(y, x0, x1)| {
                        let row = y * current.width as usize;
                        current.pixels[(row + x0) * bytes..(row + x1) * bytes].iter().all(|&b| b == 0)
                    });
                    if empty {
                        continue;
                    }
                    m.blocks += 1;
                    if block_matches_opaque(current, previous, bytes, block, (0, 0)) {
                        m.loose_zero += 1;
                    } else if let Some(&(dx, dy)) =
                        shifts.iter().find(|&&shift| block_matches_opaque(current, previous, bytes, block, shift))
                    {
                        m.loose_shifted += 1;
                        m.loose_lengths.push(f64::from(dx * dx + dy * dy).sqrt());
                    }
                    if block_matches(current, previous, bytes, block, (0, 0)) {
                        m.zero += 1;
                        continue;
                    }
                    if let Some(&(dx, dy)) = shifts.iter().find(|&&shift| block_matches(current, previous, bytes, block, shift)) {
                        m.shifted += 1;
                        m.lengths.push(f64::from(dx * dx + dy * dy).sqrt());
                        m.chebyshev[dx.abs().max(dy.abs()) as usize] += 1;
                    }
                }
            }
            m
        })
        .collect();
    let mut total = Motion { chebyshev: vec![0; MOTION_RADIUS as usize + 1], ..Motion::default() };
    for part in parts {
        total.blocks += part.blocks;
        total.zero += part.zero;
        total.shifted += part.shifted;
        total.lengths.extend(part.lengths);
        total.loose_zero += part.loose_zero;
        total.loose_shifted += part.loose_shifted;
        total.loose_lengths.extend(part.loose_lengths);
        for (a, b) in total.chebyshev.iter_mut().zip(part.chebyshev) {
            *a += b;
        }
    }
    total
}

fn print_motion(client: Client, results: &[Motion]) {
    let blocks: u64 = results.iter().map(|m| m.blocks).sum();
    let zero: u64 = results.iter().map(|m| m.zero).sum();
    let shifted: u64 = results.iter().map(|m| m.shifted).sum();
    let lengths = sorted(results.iter().flat_map(|m| m.lengths.iter().copied()).collect());
    let mut chebyshev = vec![0u64; MOTION_RADIUS as usize + 1];
    for m in results {
        for (a, b) in chebyshev.iter_mut().zip(&m.chebyshev) {
            *a += b;
        }
    }
    let share = |n: u64| PERCENT * n as f64 / blocks.max(1) as f64;
    println!(
        "\n{} motion ({} bodies, {blocks} non-empty 8x8 blocks after the first frame of a direction): same place {:.1}%, exact shift within ±{MOTION_RADIUS} {:.1}%, no exact match {:.1}%; shift length median {:.1}, p95 {:.1}; Chebyshev 1: {:.1}%, 2: {:.1}%, 3-4: {:.1}%, 5-8: {:.1}%, 9-16: {:.1}% of the shifted blocks",
        client.label(),
        results.len(),
        share(zero),
        share(shifted),
        share(blocks - zero - shifted),
        if lengths.is_empty() { 0.0 } else { percentile(&lengths, MEDIAN) },
        if lengths.is_empty() { 0.0 } else { percentile(&lengths, P95) },
        PERCENT * chebyshev[1] as f64 / shifted.max(1) as f64,
        PERCENT * chebyshev[2] as f64 / shifted.max(1) as f64,
        PERCENT * chebyshev[3..=4].iter().sum::<u64>() as f64 / shifted.max(1) as f64,
        PERCENT * chebyshev[5..=8].iter().sum::<u64>() as f64 / shifted.max(1) as f64,
        PERCENT * chebyshev[9..].iter().sum::<u64>() as f64 / shifted.max(1) as f64,
    );
    let loose_zero: u64 = results.iter().map(|m| m.loose_zero).sum();
    let loose_shifted: u64 = results.iter().map(|m| m.loose_shifted).sum();
    let loose_lengths = sorted(results.iter().flat_map(|m| m.loose_lengths.iter().copied()).collect());
    println!(
        "{} motion, non-transparent pixels only: same place {:.1}%, exact shift {:.1}%, no match {:.1}%; shift length median {:.1}, p95 {:.1}",
        client.label(),
        share(loose_zero),
        share(loose_shifted),
        share(blocks - loose_zero - loose_shifted),
        if loose_lengths.is_empty() { 0.0 } else { percentile(&loose_lengths, MEDIAN) },
        if loose_lengths.is_empty() { 0.0 } else { percentile(&loose_lengths, P95) },
    );
}

#[test]
#[ignore = "needs a Classic and an Enhanced client: set UO_CC_CLIENT_DIR and UO_EC_CLIENT_DIR, run with --release"]
fn mobile_animation_storage_candidates() {
    let (Some(cc_dir), Some(ec_dir)) = (std::env::var_os(CC_CLIENT_DIR_ENV), std::env::var_os(EC_CLIENT_DIR_ENV)) else {
        println!("set {CC_CLIENT_DIR_ENV} and {EC_CLIENT_DIR_ENV}; skipped");
        return;
    };
    let started = Instant::now();
    let sources = load_sources(Path::new(&cc_dir), Path::new(&ec_dir));
    let stride = std::env::var(BODY_STRIDE_ENV).ok().map(|n| n.parse::<usize>().expect("stride")).unwrap_or(1);
    let bodies: Vec<BodySource> = enumerate_bodies(&sources).into_iter().step_by(stride).collect();
    println!("{} body batches", bodies.len());

    let passes: Vec<String> = std::env::var(PASSES_ENV)
        .map(|list| list.split(',').map(|pass| pass.trim().to_string()).collect())
        .unwrap_or_else(|_| DEFAULT_PASSES.map(String::from).to_vec());
    for pass in &passes {
        assert!(ALL_PASSES.contains(&pass.as_str()), "{PASSES_ENV}: unknown pass {pass}, expected one of {ALL_PASSES:?}");
    }
    let runs = |pass: &str| passes.iter().any(|p| p == pass);

    // (client, trimmed pixels, body) of every body, to pick the samples by size.
    let done = AtomicUsize::new(0);
    let sizes: Vec<(Client, u64, usize)> = if runs(SIZES_PASS) {
        let stats: Vec<BodyStats> = bodies
            .par_iter()
            .enumerate()
            .map(|(i, source)| {
                let body = load_body(&sources, source);
                let stats = measure_body(&body, i);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % PROGRESS_EVERY == 0 {
                    println!("  {n}/{} bodies measured ({:.0} s)", bodies.len(), started.elapsed().as_secs_f64());
                }
                stats
            })
            .collect();
        println!("sizes measured in {:.0} s", started.elapsed().as_secs_f64());
        print_size_tables(&stats);
        print_ec_tables(&stats);
        stats.iter().map(|s| (s.client, s.pixels, s.source)).collect()
    } else {
        bodies.par_iter().enumerate().map(|(i, source)| {
            let body = load_body(&sources, source);
            (body.client, body.pixel_count(), i)
        }).collect()
    };
    let present: Vec<(Client, u64, usize)> = sizes.into_iter().filter(|s| s.1 > 0).collect();
    let pick = |client: Client, count: usize| -> Vec<usize> {
        let of_client: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client).collect();
        spread(&of_client, count, |s| s.1).into_iter().map(|s| s.2).collect()
    };
    if runs(TIMING_PASS) {
        time_sample(&sources, &bodies, &[Client::Cc, Client::Ec].map(|c| pick(c, TIMING_SAMPLE_PER_CLIENT)).concat());
    }
    if runs(SHRINK_PASS) {
        shrink::run(&sources, &bodies, &present, Path::new(&cc_dir));
    }
    if !runs(UPSCALE_PASS) {
        println!("\ndone in {:.0} s", started.elapsed().as_secs_f64());
        return;
    }

    let upscale_sample: Vec<usize> =
        [Client::Cc, Client::Ec].into_iter().flat_map(|c| pick(c, UPSCALE_SAMPLE_PER_CLIENT)).collect();
    let sample_bodies: Vec<Body> = upscale_sample.par_iter().map(|&i| load_body(&sources, &bodies[i])).collect();
    println!("\nUpscale and motion sample:");
    for body in &sample_bodies {
        println!("  {} ({} frames, {:.2} Mpx)", body.label, body.frame_count(), body.pixel_count() as f64 / PIXELS_PER_MEGAPIXEL);
    }
    let upscaled: Vec<UpscaleResult> = sample_bodies.par_iter().map(upscale_measure).collect();
    print_upscale_tables(&upscaled);
    for client in [Client::Cc, Client::Ec] {
        let results: Vec<Motion> = sample_bodies.iter().filter(|b| b.client == client).map(motion).collect();
        print_motion(client, &results);
    }
    println!("\ndone in {:.0} s", started.elapsed().as_secs_f64());
}
