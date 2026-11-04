//! Parts `q`, `qc`, `qd` and `qm` of the `shrink` pass: lossy block formats for low-end devices, chosen at pack time. Measure
//! only: runtime BC7 stays off, and nothing here decides the format.
//!
//! - `q`: BC7, BC7 with RDO, BC7 with the colour under transparent pixels filled in, BC1, BC1 with a block-reuse pass and BC3,
//!   per (action, direction), on the sample: disk, quality on composited frames, hue fidelity, coverage. Sizes and quality, so
//!   no quiet machine.
//! - `qc`: decode of those rows in the creature and dressed NPC crowds (needs a quiet machine).
//! - `qd`: writes a sample's streams and the creature crowd's to `MOBILE_ANIM_CODEC_DUMP_DIR`, for encoders that aren't in the
//!   workspace. Extracted art: it stays in the scratch directory.
//! - `qm`: reads what those encoders decoded, from the same directory, and measures it as `q` measures its own rows. Then the
//!   low-end mix: a body above a threshold on its median frame area takes a block format, the rest stay lossless; and with
//!   `MOBILE_ANIM_CODEC_PREVIEW_DIR` set, side-by-side crops to look at. Extracted art again, so they stay in the scratch
//!   directory.
//!
//! A stream's blocks are encoded as a strip one block wide, in the stream's order. So what RDO looks back at is what zstd
//! sees, it stays inside the stream, and a pixel outside a frame's box is transparent black in its partial blocks.

use super::classes::{classes_of, median_area, BodyRule, Class, CLASSES};
use super::vram::{hue_mode_of, hueable_of, HUE_MASKED, MIX_TOPS};
use super::*;
use dds::{ColorFormat, CompressionQuality, DecodeOptions, EncodeOptions, Format, ImageView, ImageViewMut, Size};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::PathBuf;
use udd_conv::bc7::analytical::{pack_bc7_rgba, FLAG_PBIT_OPT, FLAG_PBIT_OPT_M6, FLAG_USE_2SUBSETS, FLAG_USE_3SUBSETS, FLAG_USE_DUAL_PLANE, FLAG_USE_TRIVIAL_M6};
use udd_conv::bc7::{decode_bc7_to_rgba8888, encode_to_bc7_with_rdo_options, Bc7RdoOptions, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS};
use uocf::classic::hues::{load_hues, HueEntry};
use uocf::enhanced::hues::{EcHueTable, EcHueTableRef, EcHueTables};

const BLOCK_SIDE: usize = 4;
const BLOCK_PIXELS: usize = BLOCK_SIDE * BLOCK_SIDE;
const BLOCK_RGBA: usize = BLOCK_PIXELS * RGBA_CHANNELS;
const BC1_BLOCK: usize = 8;
const BC_WIDE_BLOCK: usize = 16;
const COLOUR_BYTES: usize = 4;
const RGB: usize = 3;
const OPAQUE_BYTE: u8 = 255;
const HALF_BYTE: u8 = 128;
const LEVELS_TOP: f64 = 16.0;
const FULL_BYTE: f64 = 255.0;
const P5: f64 = 0.05;
const P50: f64 = 0.5;
/// A score this high or better is what the user asks of a lossy row.
const SCORE_LINE: f64 = 80.0;
const DECIBELS: f64 = 10.0;
/// What a changed hued channel counts as visible from.
const HUED_VISIBLE: u8 = 8;
const CLASSIC_KEY_SHIFT: u8 = 3;
const RGB5_MASK: u8 = 0xF8;

/// The strengths of RDO and of the BC1 reuse pass, in BC7 RDO's units: mean squared error against bits times lambda.
const RDO_LAMBDAS: [f32; 3] = [0.5, 1.0, 2.0];
const REUSE_LOOKBACK: usize = 64;
const LITERAL_BITS: f32 = 8.0;
const MATCH_BITS: f32 = 16.0;
const HALF_BLOCK_BYTES: f32 = 4.0;
const WHOLE_BC1_BYTES: f32 = 8.0;
const SELECTOR_BITS: u32 = 2;
const SELECTOR_MASK: u32 = 3;
const P3_TRANSPARENT: u32 = 3;

/// Hues to apply: dyes spread over the tables, skipping any a client lacks.
const HUE_SAMPLES: [u16; 6] = [2, 38, 89, 1109, 1153, 2125];
const HUES_FILE_EC: &str = "Hues.uop";
const HUES_FILE_CC: &str = "hues.mul";

const QC_LAMBDA_ENV: &str = "MOBILE_ANIM_CODEC_QC_LAMBDA";
const QC_DEFAULT_LAMBDA: f32 = 1.0;
const DUMP_DIR_ENV: &str = "MOBILE_ANIM_CODEC_DUMP_DIR";
const STRIPS_SUFFIX: &str = "strips";
const CROWD_SUFFIX: &str = "crowd";
const GPU_SUFFIX: &str = "gpu";
const CROWD_DRESSED_SUFFIX: &str = "crowd-dressed";
const DECODED_MARK: &str = ".decoded.";
const SIZES_MARK: &str = ".sizes.";
const LOSSLESS_NAME: &str = "lossless";
const BC1_NAME: &str = "bc1";
const BC7_NAME: &str = "bc7";
const BC7_BLEED_NAME: &str = "bc7-bleed";
const ASTC_MID_NAME: &str = "astc-6";
const ASTC_BIG_NAME: &str = "astc-8";
const SIZE_RECORD_BYTES: usize = 24;
const PREVIEW_DIR_ENV: &str = "MOBILE_ANIM_CODEC_PREVIEW_DIR";
/// The mix's rows: bodies a format is scored on, and which bodies to name as the worst.
const WORST_BODIES_NAMED: usize = 3;
/// Frames too small to say anything aren't picked for a preview.
const PREVIEW_MIN_AREA: usize = 400;
const PREVIEW_GAP: usize = 8;
/// A frame's zoom in a preview, by its width: wide frames stay at their own size.
const PREVIEW_ZOOMS: [(usize, usize); 2] = [(96, 4), (220, 2)];

#[derive(Clone, Copy, PartialEq)]
enum Coder {
    Bc7,
    Rdo(f32),
    /// The same encoder, with the colour under transparent pixels filled in first.
    Bleed,
    BleedRdo(f32),
    /// And every mode the encoder has, not only the ones the converter turns on.
    BleedAll,
    Bc1,
    Reuse(f32),
    Bc3,
    Bc3Bleed,
}

impl Coder {
    fn name(self) -> String {
        match self {
            Coder::Bc7 => "BC7".to_string(),
            Coder::Rdo(lambda) => format!("BC7, RDO lambda {lambda}"),
            Coder::Bleed => "BC7, colour under transparent pixels".to_string(),
            Coder::BleedRdo(lambda) => format!("BC7, colour under transparent pixels, RDO lambda {lambda}"),
            Coder::BleedAll => "BC7, colour under transparent pixels, all modes".to_string(),
            Coder::Bc1 => "BC1".to_string(),
            Coder::Reuse(lambda) => format!("BC1, block reuse lambda {lambda}"),
            Coder::Bc3 => "BC3 (BC1 colour + BC4 coverage)".to_string(),
            Coder::Bc3Bleed => "BC3, colour under transparent pixels".to_string(),
        }
    }

    fn block_bytes(self) -> usize {
        match self {
            Coder::Bc1 | Coder::Reuse(_) => BC1_BLOCK,
            Coder::Bc7 | Coder::Rdo(_) | Coder::Bleed | Coder::BleedRdo(_) | Coder::BleedAll | Coder::Bc3 | Coder::Bc3Bleed => BC_WIDE_BLOCK,
        }
    }
}

fn coders() -> Vec<Coder> {
    let mut out = vec![Coder::Bc7];
    out.extend(RDO_LAMBDAS.iter().map(|&lambda| Coder::Rdo(lambda)));
    out.push(Coder::Bleed);
    out.extend(RDO_LAMBDAS.iter().map(|&lambda| Coder::BleedRdo(lambda)));
    out.push(Coder::BleedAll);
    out.push(Coder::Bc1);
    out.extend(RDO_LAMBDAS.iter().map(|&lambda| Coder::Reuse(lambda)));
    out.push(Coder::Bc3);
    out.push(Coder::Bc3Bleed);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Encoders and decoders

fn dds_options() -> EncodeOptions {
    let mut options = EncodeOptions::default();
    options.quality = CompressionQuality::High;
    options.parallel = false;
    options
}

fn strip_size(blocks: usize) -> Size {
    Size::new(BLOCK_SIDE as u32, (blocks * BLOCK_SIDE) as u32)
}

fn dds_encode(data: &[u8], blocks: usize, colour: ColorFormat, format: Format) -> Vec<u8> {
    let view = ImageView::new(data, strip_size(blocks), colour).expect("a strip");
    let mut out = Vec::new();
    dds::encode(&mut out, view, format, None, &dds_options()).expect("dds encodes the strip");
    out
}

fn dds_decode(data: &[u8], blocks: usize, colour: ColorFormat, format: Format) -> Vec<u8> {
    let mut out = vec![0u8; blocks * BLOCK_PIXELS * colour.bytes_per_pixel() as usize];
    let view = ImageViewMut::new(&mut out, strip_size(blocks), colour).expect("a strip");
    dds::decode(&mut &data[..], view, format, &DecodeOptions::default()).expect("dds decodes the strip");
    out
}

fn strip_extent(blocks: usize) -> ImageExtent {
    ImageExtent::new(BLOCK_SIDE as u32, (blocks * BLOCK_SIDE) as u32).expect("a strip's extent")
}

/// A strip with each block's transparent pixels holding the coverage-weighted mean colour of its painted ones. They don't show,
/// and the converter's transparent black pulls the encoder's colour line to black in every block on an edge.
fn bleed_strip(strip: &[u8]) -> Vec<u8> {
    let mut out = strip.to_vec();
    for block in out.chunks_exact_mut(BLOCK_RGBA) {
        let mut sum = [0u64; RGB];
        let mut weight = 0u64;
        for pixel in block.chunks_exact(RGBA_CHANNELS) {
            let alpha = u64::from(pixel[ALPHA]);
            weight += alpha;
            for c in 0..RGB {
                sum[c] += u64::from(pixel[c]) * alpha;
            }
        }
        if weight == 0 {
            continue;
        }
        let colour = sum.map(|s| ((s + weight / 2) / weight) as u8);
        for pixel in block.chunks_exact_mut(RGBA_CHANNELS) {
            if pixel[ALPHA] == 0 {
                pixel[..RGB].copy_from_slice(&colour);
            }
        }
    }
    out
}

/// The encoder's own block entry with every search flag on; the converter's path leaves the two- and three-subset modes off.
fn encode_all_modes(strip: &[u8], blocks: usize) -> Vec<u8> {
    let flags = FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6 | FLAG_USE_2SUBSETS | FLAG_USE_3SUBSETS | FLAG_PBIT_OPT;
    let mut out = vec![0u8; blocks * BC_WIDE_BLOCK];
    for (block, source) in out.chunks_exact_mut(BC_WIDE_BLOCK).zip(strip.chunks_exact(BLOCK_RGBA)) {
        let pixels: [[u8; RGBA_CHANNELS]; BLOCK_PIXELS] = std::array::from_fn(|i| source[i * RGBA_CHANNELS..(i + 1) * RGBA_CHANNELS].try_into().expect("a pixel"));
        pack_bc7_rgba(block.try_into().expect("a block"), &pixels, flags);
    }
    out
}

fn encode_strip(coder: Coder, strip: &[u8], blocks: usize) -> Vec<u8> {
    match coder {
        Coder::Bleed => encode_strip(Coder::Bc7, &bleed_strip(strip), blocks),
        Coder::BleedRdo(lambda) => encode_strip(Coder::Rdo(lambda), &bleed_strip(strip), blocks),
        Coder::BleedAll => encode_all_modes(&bleed_strip(strip), blocks),
        Coder::Bc7 => encode_bc7(strip, BLOCK_SIDE as u32, (blocks * BLOCK_SIDE) as u32, Bc7EncoderBackend::AnalyticalWide),
        Coder::Rdo(lambda) => {
            let options = Bc7RdoOptions::sparse_atlas(lambda, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS);
            encode_to_bc7_with_rdo_options(strip, strip_extent(blocks), RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide, &options).expect("BC7 with RDO").into_blocks()
        }
        Coder::Bc1 => dds_encode(strip, blocks, ColorFormat::RGBA_U8, Format::BC1_UNORM),
        Coder::Reuse(lambda) => {
            let mut data = dds_encode(strip, blocks, ColorFormat::RGBA_U8, Format::BC1_UNORM);
            reuse_blocks(&mut data, strip, lambda);
            data
        }
        Coder::Bc3 => dds_encode(strip, blocks, ColorFormat::RGBA_U8, Format::BC3_UNORM),
        Coder::Bc3Bleed => dds_encode(&bleed_strip(strip), blocks, ColorFormat::RGBA_U8, Format::BC3_UNORM),
    }
}

fn decode_strip(coder: Coder, data: &[u8], blocks: usize) -> Vec<u8> {
    match coder {
        Coder::Bc7 | Coder::Rdo(_) | Coder::Bleed | Coder::BleedRdo(_) | Coder::BleedAll => decode_bc7_to_rgba8888(data, strip_extent(blocks)).expect("BC7 decode"),
        Coder::Bc1 | Coder::Reuse(_) => dds_decode(data, blocks, ColorFormat::RGBA_U8, Format::BC1_UNORM),
        Coder::Bc3 | Coder::Bc3Bleed => dds_decode(data, blocks, ColorFormat::RGBA_U8, Format::BC3_UNORM),
    }
}

/// The colours of a BC1 block; in a three-colour block the fourth is transparent.
fn bc1_colours(block: &[u8]) -> ([[i32; RGB]; COLOUR_BYTES], bool) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let expand = |c: u16| -> [i32; RGB] {
        let (r, g, b) = (i32::from((c >> 11) & 31), i32::from((c >> 5) & 63), i32::from(c & 31));
        [(r * 255 + 15) / 31, (g * 255 + 31) / 63, (b * 255 + 15) / 31]
    };
    let (a, b) = (expand(c0), expand(c1));
    if c0 > c1 {
        let two: [i32; RGB] = std::array::from_fn(|i| (2 * a[i] + b[i] + 1) / 3);
        let three: [i32; RGB] = std::array::from_fn(|i| (a[i] + 2 * b[i] + 1) / 3);
        ([a, b, two, three], false)
    } else {
        let two: [i32; RGB] = std::array::from_fn(|i| (a[i] + b[i] + 1) / 2);
        ([a, b, two, [0; RGB]], true)
    }
}

/// Mean squared error per channel on the painted pixels; none if the block's transparency wouldn't survive.
fn bc1_error(block: &[u8], pixels: &[u8]) -> Option<f32> {
    let (colours, three) = bc1_colours(block);
    let selectors = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let (mut sum, mut painted) = (0i64, 0i64);
    for (n, pixel) in pixels.chunks_exact(RGBA_CHANNELS).enumerate() {
        let selector = (selectors >> (n as u32 * SELECTOR_BITS)) & SELECTOR_MASK;
        let is_painted = pixel[ALPHA] >= HALF_BYTE;
        let shows = !(three && selector == P3_TRANSPARENT);
        if is_painted != shows {
            return None;
        }
        if is_painted {
            let colour = colours[selector as usize];
            for c in 0..RGB {
                let off = i64::from(pixel[c]) - i64::from(colour[c]);
                sum += off * off;
            }
            painted += 1;
        }
    }
    Some(if painted == 0 { 0.0 } else { sum as f32 / (painted * RGB as i64) as f32 })
}

/// The selectors that suit a block's pixels best under these endpoints; none if they can't show its transparency.
fn best_selectors(endpoints: &[u8], pixels: &[u8]) -> Option<[u8; BC1_BLOCK]> {
    let mut block = [0u8; BC1_BLOCK];
    block[..COLOUR_BYTES].copy_from_slice(endpoints);
    let (colours, three) = bc1_colours(&block);
    let usable = if three { P3_TRANSPARENT as usize } else { COLOUR_BYTES };
    let mut selectors = 0u32;
    for (n, pixel) in pixels.chunks_exact(RGBA_CHANNELS).enumerate() {
        let selector = if pixel[ALPHA] >= HALF_BYTE {
            let distance = |colour: &[i32; RGB]| (0..RGB).map(|c| (i32::from(pixel[c]) - colour[c]).pow(2)).sum::<i32>();
            (0..usable).min_by_key(|&s| distance(&colours[s])).expect("a colour") as u32
        } else if three {
            P3_TRANSPARENT
        } else {
            return None;
        };
        selectors |= selector << (n as u32 * SELECTOR_BITS);
    }
    block[COLOUR_BYTES..].copy_from_slice(&selectors.to_le_bytes());
    Some(block)
}

/// A small entropy-reduction pass over a stream's BC1 blocks, as `bc7enc_rdo` does for BC7: a block may take the whole of an
/// earlier block, its selectors or its endpoints, within the lookback, if error + lambda x bits falls. An estimate of what an
/// LZ match costs decides; zstd's real size is what's reported.
fn reuse_blocks(data: &mut [u8], strip: &[u8], lambda: f32) {
    let blocks = data.len() / BC1_BLOCK;
    for n in 0..blocks {
        let pixels = &strip[n * BLOCK_RGBA..(n + 1) * BLOCK_RGBA];
        if pixels.chunks_exact(RGBA_CHANNELS).all(|p| p[ALPHA] < HALF_BYTE) {
            continue;
        }
        let at = n * BC1_BLOCK;
        let mut best: [u8; BC1_BLOCK] = data[at..at + BC1_BLOCK].try_into().expect("a block");
        let Some(error) = bc1_error(&best, pixels) else { continue };
        let mut best_cost = error + LITERAL_BITS * WHOLE_BC1_BYTES * lambda;
        let half_cost = (LITERAL_BITS * HALF_BLOCK_BYTES + MATCH_BITS) * lambda;
        let own = best;
        for back in 1..=REUSE_LOOKBACK.min(n) {
            let other: [u8; BC1_BLOCK] = data[at - back * BC1_BLOCK..at - (back - 1) * BC1_BLOCK].try_into().expect("a block");
            if let Some(error) = bc1_error(&other, pixels) {
                let cost = error + MATCH_BITS * lambda;
                if cost < best_cost {
                    (best, best_cost) = (other, cost);
                }
            }
            let mut mixed = own;
            mixed[COLOUR_BYTES..].copy_from_slice(&other[COLOUR_BYTES..]);
            if let Some(error) = bc1_error(&mixed, pixels) {
                let cost = error + half_cost;
                if cost < best_cost {
                    (best, best_cost) = (mixed, cost);
                }
            }
            if let Some(candidate) = best_selectors(&other[..COLOUR_BYTES], pixels) {
                if let Some(error) = bc1_error(&candidate, pixels) {
                    let cost = error + half_cost;
                    if cost < best_cost {
                        (best, best_cost) = (candidate, cost);
                    }
                }
            }
        }
        data[at..at + BC1_BLOCK].copy_from_slice(&best);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Streams as strips

struct Strip {
    frames: Vec<usize>,
    blocks: usize,
    rgba: Vec<u8>,
}

fn blocks_of(frame: &FrameBox) -> usize {
    frame.w.div_ceil(BLOCK_SIDE) * frame.h.div_ceil(BLOCK_SIDE)
}

/// A frame's blocks in raster order, a transparent pixel always (0, 0, 0, 0).
fn append_blocks(frame: &FrameBox, rgba: &[u8], out: &mut Vec<u8>) {
    let (w, h) = (frame.w, frame.h);
    let zero = [0u8; RGBA_CHANNELS];
    for by in 0..h.div_ceil(BLOCK_SIDE) {
        for bx in 0..w.div_ceil(BLOCK_SIDE) {
            for y in by * BLOCK_SIDE..(by + 1) * BLOCK_SIDE {
                for x in bx * BLOCK_SIDE..(bx + 1) * BLOCK_SIDE {
                    let pixel = if x < w && y < h { &rgba[(y * w + x) * RGBA_CHANNELS..(y * w + x + 1) * RGBA_CHANNELS] } else { &zero[..] };
                    out.extend_from_slice(if pixel[ALPHA] == 0 { &zero[..] } else { pixel });
                }
            }
        }
    }
}

fn strips_of(body: &Body, planes: &Planes) -> Vec<Strip> {
    let frames: Vec<&Frame> = body.directions.iter().flatten().collect();
    units(&planes.boxes, Unit::ActionDirection, RGBA_CHANNELS)
        .into_iter()
        .map(|group| {
            let blocks: usize = group.iter().map(|&n| blocks_of(&planes.boxes[n])).sum();
            let mut rgba = Vec::with_capacity(blocks * BLOCK_RGBA);
            for &n in &group {
                append_blocks(&planes.boxes[n], &frames[n].rgba, &mut rgba);
            }
            Strip { frames: group, blocks, rgba }
        })
        .collect()
}

/// A decoded strip back into its frames.
fn frames_of_strip(decoded: &[u8], frames: &[usize], boxes: &[FrameBox], out: &mut [Vec<u8>]) {
    let mut block = 0usize;
    for &n in frames {
        let frame = &boxes[n];
        let (w, h) = (frame.w, frame.h);
        let mut rgba = vec![0u8; w * h * RGBA_CHANNELS];
        for by in 0..h.div_ceil(BLOCK_SIDE) {
            for bx in 0..w.div_ceil(BLOCK_SIDE) {
                let source = &decoded[block * BLOCK_RGBA..(block + 1) * BLOCK_RGBA];
                for y in 0..BLOCK_SIDE.min(h - by * BLOCK_SIDE) {
                    let columns = BLOCK_SIDE.min(w - bx * BLOCK_SIDE);
                    let target = ((by * BLOCK_SIDE + y) * w + bx * BLOCK_SIDE) * RGBA_CHANNELS;
                    let from = y * BLOCK_SIDE * RGBA_CHANNELS;
                    rgba[target..target + columns * RGBA_CHANNELS].copy_from_slice(&source[from..from + columns * RGBA_CHANNELS]);
                }
                block += 1;
            }
        }
        out[n] = rgba;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What a row is worth

#[derive(Clone, Default)]
struct Tally {
    seconds: f64,
    raw: u64,
    zstd_packer: u64,
    zstd_best: u64,
    pixels: u64,
    painted: u64,
    channel_sum: u64,
    channel_squared: u64,
    channel_samples: u64,
    channel_worst: u8,
    vanished: u64,
    appeared: u64,
    edges: u64,
    edge_sum: f64,
    edge_worst: f64,
    edge_changed: u64,
    solid: u64,
    solid_changed: u64,
    hue_pixels: u64,
    key_changed: u64,
    key_sum: u64,
    key_worst: u8,
    hued_samples: u64,
    hued_sum: u64,
    hued_worst: u8,
    hued_visible: u64,
    partial_samples: u64,
    partial_sum: u64,
    partial_worst: u8,
    grey: u64,
    grey_broken: u64,
    grey_broken_5: u64,
    colour: u64,
    colour_grey: u64,
    colour_grey_5: u64,
    scores: Vec<f64>,
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        macro_rules! sum {
            ($($field:ident),*) => { $(self.$field += other.$field;)* };
        }
        macro_rules! worst {
            ($($field:ident),*) => { $(self.$field = self.$field.max(other.$field);)* };
        }
        sum!(seconds, raw, zstd_packer, zstd_best, pixels, painted, channel_sum, channel_squared, channel_samples, vanished, appeared, edges, edge_sum, edge_changed, solid, solid_changed, hue_pixels, key_changed, key_sum, hued_samples, hued_sum, hued_visible, partial_samples, partial_sum, grey, grey_broken, grey_broken_5, colour, colour_grey, colour_grey_5);
        worst!(channel_worst, key_worst, hued_worst, partial_worst);
        self.edge_worst = self.edge_worst.max(other.edge_worst);
        self.scores.extend_from_slice(&other.scores);
    }
}

/// The real hues, for the error after one is applied.
struct Hues {
    enhanced: Vec<EcHueTable>,
    classic: Vec<HueEntry>,
}

impl Hues {
    fn load(cc_dir: &Path) -> Hues {
        let ec_dir = std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client directory");
        let path = find_ignoring_case(Path::new(&ec_dir), HUES_FILE_EC).expect("the Enhanced hues");
        let tables = EcHueTables::load(path).expect("the Enhanced hues parse");
        let enhanced = HUE_SAMPLES
            .iter()
            .filter_map(|&hue| match tables.hue_table(hue) {
                EcHueTableRef::Table(table) => Some(table.clone()),
                EcHueTableRef::None | EcHueTableRef::Shadow => None,
            })
            .collect();
        let path = find_ignoring_case(cc_dir, HUES_FILE_CC).expect("the Classic hues");
        let all = load_hues(&path).expect("the Classic hues parse");
        let classic = HUE_SAMPLES.iter().filter_map(|&hue| all.get(usize::from(hue)).cloned()).collect();
        Hues { enhanced, classic }
    }
}

fn is_grey(pixel: &[u8]) -> bool {
    pixel[0] == pixel[1] && pixel[1] == pixel[2]
}

fn is_grey_5(pixel: &[u8]) -> bool {
    pixel[0] & RGB5_MASK == pixel[1] & RGB5_MASK && pixel[1] & RGB5_MASK == pixel[2] & RGB5_MASK
}

fn packed(pixel: &[u8]) -> u32 {
    (u32::from(OPAQUE_BYTE) << 24) | (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2])
}

fn add_hued(sum: &mut u64, worst: &mut u8, visible: Option<&mut u64>, a: [u8; RGB], b: [u8; RGB]) {
    let mut seen = false;
    for c in 0..RGB {
        let off = a[c].abs_diff(b[c]);
        *sum += u64::from(off);
        *worst = (*worst).max(off);
        seen |= off >= HUED_VISIBLE;
    }
    if let Some(count) = visible {
        *count += u64::from(seen);
    }
}

fn channels_of(colour: u32) -> [u8; RGB] {
    [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8]
}

fn tally_frame(planes: &Planes, frame: &FrameBox, truth: &[u8], decoded: &[u8], hues: &Hues, hueable: &[[bool; PALETTE_COLOURS]], tally: &mut Tally) {
    let enhanced = planes.client == Client::Ec;
    let tables = if enhanced { hues.enhanced.len() } else { hues.classic.len() } as u64;
    tally.pixels += frame.pixels() as u64;
    for n in 0..frame.pixels() {
        let t = &truth[n * RGBA_CHANNELS..(n + 1) * RGBA_CHANNELS];
        let d = &decoded[n * RGBA_CHANNELS..(n + 1) * RGBA_CHANNELS];
        let at = frame.start + n;
        if enhanced {
            let level = planes.coverage[at];
            if level == COVERAGE_SOLID {
                tally.solid += 1;
                tally.solid_changed += u64::from(d[ALPHA] != OPAQUE_BYTE);
            } else if level != COVERAGE_TRANSPARENT {
                let levels = f64::from(t[ALPHA].abs_diff(d[ALPHA])) * LEVELS_TOP / FULL_BYTE;
                tally.edges += 1;
                tally.edge_sum += levels;
                tally.edge_worst = tally.edge_worst.max(levels);
                tally.edge_changed += u64::from(levels >= 1.0);
            }
        }
        if t[ALPHA] == 0 {
            tally.appeared += u64::from(d[ALPHA] != 0);
            continue;
        }
        tally.painted += 1;
        if d[ALPHA] == 0 {
            tally.vanished += 1;
            continue;
        }
        for c in 0..RGB {
            let off = t[c].abs_diff(d[c]);
            tally.channel_sum += u64::from(off);
            tally.channel_squared += u64::from(off) * u64::from(off);
            tally.channel_worst = tally.channel_worst.max(off);
        }
        tally.channel_samples += RGB as u64;
        if enhanced {
            if hueable[frame.palette][usize::from(planes.index[at])] {
                let moved = t[0].abs_diff(d[0]);
                tally.hue_pixels += 1;
                tally.key_changed += u64::from(moved != 0);
                tally.key_sum += u64::from(moved);
                tally.key_worst = tally.key_worst.max(moved);
                tally.hued_samples += tables * RGB as u64;
                if moved != 0 {
                    for table in &hues.enhanced {
                        let (a, b) = (table.entries[usize::from(t[0])], table.entries[usize::from(d[0])]);
                        add_hued(&mut tally.hued_sum, &mut tally.hued_worst, Some(&mut tally.hued_visible), [a[0], a[1], a[2]], [b[0], b[1], b[2]]);
                    }
                }
            }
        } else {
            let (key_t, key_d) = (t[0] >> CLASSIC_KEY_SHIFT, d[0] >> CLASSIC_KEY_SHIFT);
            let moved = key_t.abs_diff(key_d);
            tally.hue_pixels += 1;
            tally.key_changed += u64::from(moved != 0);
            tally.key_sum += u64::from(moved);
            tally.key_worst = tally.key_worst.max(moved);
            tally.hued_samples += tables * RGB as u64;
            tally.partial_samples += tables * RGB as u64;
            if is_grey(t) {
                tally.grey += 1;
                tally.grey_broken += u64::from(!is_grey(d));
                tally.grey_broken_5 += u64::from(!is_grey_5(d));
            } else {
                tally.colour += 1;
                tally.colour_grey += u64::from(is_grey(d));
                tally.colour_grey_5 += u64::from(is_grey_5(d));
            }
            if t[..RGB] != d[..RGB] {
                let (colour_t, colour_d) = (packed(t), packed(d));
                for entry in &hues.classic {
                    add_hued(&mut tally.hued_sum, &mut tally.hued_worst, Some(&mut tally.hued_visible), channels_of(entry.apply_to_color32(colour_t, false)), channels_of(entry.apply_to_color32(colour_d, false)));
                    add_hued(&mut tally.partial_sum, &mut tally.partial_worst, None, channels_of(entry.apply_to_color32(colour_t, true)), channels_of(entry.apply_to_color32(colour_d, true)));
                }
            }
        }
    }
}

fn tally_body(planes: &Planes, truth: &[&Frame], decoded: &[Vec<u8>], hues: &Hues, tally: &mut Tally) {
    let hueable = hueable_of(planes);
    for ((frame, t), d) in planes.boxes.iter().zip(truth).zip(decoded) {
        tally_frame(planes, frame, &t.rgba, d, hues, &hueable, tally);
    }
    let reference: Vec<(u32, u32, &[u8])> = truth.iter().map(|f| (f.width, f.height, f.rgba.as_slice())).collect();
    let test: Vec<(u32, u32, &[u8])> = truth.iter().zip(decoded).map(|(f, d)| (f.width, f.height, d.as_slice())).collect();
    if !reference.is_empty() {
        tally.scores.push(compute_frame_ssimulacra2(mosaic(&reference), mosaic(&test)).expect("SSIMULACRA2"));
    }
}

/// A hue mask in BC4 against the true flags: the painted pixels of masked frames, and those that come back wrong.
fn mask_errors(planes: &Planes) -> (u64, u64) {
    let hueable = hueable_of(planes);
    let mask: &[u8] = if planes.client == Client::Ec { &planes.coverage } else { &planes.index };
    let (mut pixels, mut wrong) = (0u64, 0u64);
    for frame in &planes.boxes {
        if hue_mode_of(planes, frame, mask, &hueable) != HUE_MASKED {
            continue;
        }
        let flags: Vec<u8> = frame.range().map(|at| if mask[at] != 0 && !hueable[frame.palette][usize::from(planes.index[at])] { OPAQUE_BYTE } else { 0 }).collect();
        let blocks = blocks_of(frame);
        let mut strip = Vec::with_capacity(blocks * BLOCK_PIXELS);
        for by in 0..frame.h.div_ceil(BLOCK_SIDE) {
            for bx in 0..frame.w.div_ceil(BLOCK_SIDE) {
                for y in by * BLOCK_SIDE..(by + 1) * BLOCK_SIDE {
                    for x in bx * BLOCK_SIDE..(bx + 1) * BLOCK_SIDE {
                        strip.push(if x < frame.w && y < frame.h { flags[y * frame.w + x] } else { 0 });
                    }
                }
            }
        }
        let data = dds_encode(&strip, blocks, ColorFormat::GRAYSCALE_U8, Format::BC4_UNORM);
        let back = dds_decode(&data, blocks, ColorFormat::GRAYSCALE_U8, Format::BC4_UNORM);
        for (value, truth) in back.iter().zip(&strip) {
            wrong += u64::from((*value >= HALF_BYTE) != (*truth >= HALF_BYTE));
        }
        pixels += mask[frame.range()].iter().filter(|&&c| c != 0).count() as u64;
    }
    (pixels, wrong)
}

// ---------------------------------------------------------------------------------------------------------------
// Part q

struct BodyRow {
    class: Class,
    id: String,
    median: u64,
    /// The plain planes at zstd 19, per stream; zero in part `q`.
    lossless: u64,
    tallies: Vec<Tally>,
    mask_pixels: u64,
    mask_wrong: u64,
}

fn measure_body(body: &Body, planes: &Planes, coders: &[Coder], hues: &Hues) -> (Vec<Tally>, u64, u64) {
    let strips = strips_of(body, planes);
    let truth: Vec<&Frame> = body.directions.iter().flatten().collect();
    let mut tallies = Vec::with_capacity(coders.len());
    for &coder in coders {
        let mut tally = Tally::default();
        let mut decoded: Vec<Vec<u8>> = vec![Vec::new(); planes.boxes.len()];
        for strip in &strips {
            let started = Instant::now();
            let data = encode_strip(coder, &strip.rgba, strip.blocks);
            tally.seconds += started.elapsed().as_secs_f64();
            assert_eq!(data.len(), strip.blocks * coder.block_bytes(), "{}: the blocks of {}", planes.label, coder.name());
            tally.raw += data.len() as u64;
            tally.zstd_packer += compress(&data, PACKER_ZSTD_LEVEL, false).len() as u64;
            tally.zstd_best += compress(&data, BEST_ZSTD_LEVEL, false).len() as u64;
            frames_of_strip(&decode_strip(coder, &data, strip.blocks), &strip.frames, &planes.boxes, &mut decoded);
        }
        tally_body(planes, &truth, &decoded, hues, &mut tally);
        tallies.push(tally);
    }
    let (mask_pixels, mask_wrong) = mask_errors(planes);
    (tallies, mask_pixels, mask_wrong)
}

fn quantile_of(sorted: &[f64], at: f64) -> f64 {
    if sorted.is_empty() { 0.0 } else { sorted[(((sorted.len() - 1) as f64) * at) as usize] }
}

fn scores_of(tally: &Tally) -> (f64, f64, f64) {
    let mut scores = tally.scores.clone();
    scores.sort_by(f64::total_cmp);
    (quantile_of(&scores, P50), quantile_of(&scores, P5), scores.first().copied().unwrap_or(0.0))
}

fn share(part: u64, of: u64) -> f64 {
    part as f64 * PERCENT / of.max(1) as f64
}

/// PSNR over the RGB of painted pixels that stayed painted, in dB: what published codec figures are given in.
fn psnr_of(tally: &Tally) -> f64 {
    let squared = tally.channel_squared as f64 / tally.channel_samples.max(1) as f64;
    if squared == 0.0 {
        return f64::INFINITY;
    }
    DECIBELS * (FULL_BYTE * FULL_BYTE / squared).log10()
}

fn mean(sum: u64, of: u64) -> f64 {
    sum as f64 / of.max(1) as f64
}

fn print_rows(client: Client, names: &[String], scale: f64, rows: &[Tally]) {
    println!("\n#### {}: disk, the whole set by this sample's share of the pixels (x {scale:.3}); quality on the body mosaics\n\n| row | block MB | zstd {PACKER_ZSTD_LEVEL} MB | zstd {BEST_ZSTD_LEVEL} MB | encode, worker s a Mpx | SSIMULACRA2 median | p5 | min | bodies at {SCORE_LINE} or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    for (name, tally) in names.iter().zip(rows) {
        let (median, p5, min) = scores_of(tally);
        let good = tally.scores.iter().filter(|&&s| s >= SCORE_LINE).count();
        println!(
            "| {name} | {:.1} | {:.1} | {:.1} | {:.3} | {median:.2} | {p5:.2} | {min:.2} | {good} of {} | {:.3} | {} | {:.2} | {:.3} % | {:.3} % |",
            mb(tally.raw) * scale,
            mb(tally.zstd_packer) * scale,
            mb(tally.zstd_best) * scale,
            tally.seconds / (tally.pixels.max(1) as f64 / PIXELS_PER_MEGAPIXEL),
            tally.scores.len(),
            mean(tally.channel_sum, tally.channel_samples),
            tally.channel_worst,
            psnr_of(tally),
            share(tally.vanished, tally.painted),
            share(tally.appeared, tally.pixels - tally.painted),
        );
    }
}

fn print_hue(client: Client, names: &[String], rows: &[Tally], tables: usize) {
    match client {
        Client::Ec => {
            println!("\n#### Enhanced: hue: the key is a hueable pixel's red byte; the error is after {tables} real tables, with a lossless mask\n\n| row | key changed | key moved, mean | worst | hued channel error, mean | worst | hued pixels off by {HUED_VISIBLE} or more |\n|---|---:|---:|---:|---:|---:|---:|");
            for (name, t) in names.iter().zip(rows) {
                println!("| {name} | {:.2} % | {:.3} | {} | {:.3} | {} | {:.3} % |", share(t.key_changed, t.hue_pixels), mean(t.key_sum, t.hue_pixels), t.key_worst, mean(t.hued_sum, t.hued_samples), t.hued_worst, share(t.hued_visible, t.hue_pixels * tables as u64));
            }
        }
        Client::Cc => {
            println!("\n#### Classic: hue: the key is the red byte's top 5 bits; a full hue takes every pixel, a partial one greys; error after {tables} real tables\n\n| row | key changed | moved, mean | worst | full hue channel error, mean | worst | hued pixels off by {HUED_VISIBLE} or more | partial hue error, mean | worst | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
            for (name, t) in names.iter().zip(rows) {
                println!("| {name} | {:.2} % | {:.3} | {} | {:.3} | {} | {:.3} % | {:.3} | {} | {:.2} % | {:.2} % | {:.3} % | {:.3} % |", share(t.key_changed, t.hue_pixels), mean(t.key_sum, t.hue_pixels), t.key_worst, mean(t.hued_sum, t.hued_samples), t.hued_worst, share(t.hued_visible, t.hue_pixels * tables as u64), mean(t.partial_sum, t.partial_samples), t.partial_worst, share(t.grey_broken, t.grey), share(t.grey_broken_5, t.grey), share(t.colour_grey, t.colour), share(t.colour_grey_5, t.colour));
            }
        }
    }
}

fn print_edges(names: &[String], rows: &[Tally]) {
    println!("\n#### Enhanced: coverage: what the block format's alpha does to the true levels, 0 to 16\n\n| row | edge pixels' error, mean levels | worst | edge pixels off by a level or more | solid pixels no longer solid |\n|---|---:|---:|---:|---:|");
    for (name, t) in names.iter().zip(rows) {
        println!("| {name} | {:.3} | {:.1} | {:.2} % | {:.3} % |", t.edge_sum / t.edges.max(1) as f64, t.edge_worst, share(t.edge_changed, t.edges), share(t.solid_changed, t.solid));
    }
}

fn print_classes(client: Client, names: &[String], scale: f64, by_class: &[Vec<Tally>]) {
    let classes: Vec<&Class> = CLASSES.iter().filter(|c| by_class[c.at()].first().is_some_and(|t| t.pixels > 0)).collect();
    let header: Vec<String> = classes.iter().map(|c| format!("{} MB | SSIMULACRA2 median / min, RGB PSNR dB", c.label())).collect();
    println!("\n#### {}: by class: zstd {BEST_ZSTD_LEVEL} on a stream, by this sample's share of the set (x {scale:.3})\n\n| row | {} |\n|---|{}", client.label(), header.join(" | "), "---:|---|".repeat(classes.len()));
    for (r, name) in names.iter().enumerate() {
        let cells: Vec<String> = classes
            .iter()
            .map(|c| {
                let tally = &by_class[c.at()][r];
                let (median, _, min) = scores_of(tally);
                format!("{:.1} | {median:.2} / {min:.2}, {:.2}", mb(tally.zstd_best) * scale, psnr_of(tally))
            })
            .collect();
        println!("| {name} | {} |", cells.join(" | "));
    }
}

fn print_verdict(client: Client, names: &[String], scale: f64, rows: &[Tally]) {
    let any: Vec<String> = names
        .iter()
        .zip(rows)
        .filter(|row| scores_of(row.1).0 >= SCORE_LINE)
        .map(|(name, tally)| format!("{name} {:.1} MB", mb(tally.zstd_best) * scale))
        .collect();
    println!("\n{}: rows with a median at {SCORE_LINE} or better: {}", client.label(), any.join("; "));
}

fn sample_scale(present: &[(Client, u64, usize)], client: Client, sample: &[usize]) -> f64 {
    let all: u64 = present.iter().filter(|s| s.0 == client).map(|s| s.1).sum();
    let chosen: BTreeSet<usize> = sample.iter().copied().collect();
    let part: u64 = present.iter().filter(|s| s.0 == client && chosen.contains(&s.2)).map(|s| s.1).sum();
    all as f64 / part.max(1) as f64
}

fn print_all(client: Client, names: &[String], scale: f64, rows: &[BodyRow], tables: usize) {
    let mut totals = vec![Tally::default(); names.len()];
    let mut by_class: Vec<Vec<Tally>> = vec![vec![Tally::default(); names.len()]; CLASSES.len()];
    let (mut mask_pixels, mut mask_wrong) = (0u64, 0u64);
    for row in rows {
        for (r, tally) in row.tallies.iter().enumerate() {
            totals[r].add(tally);
            by_class[row.class.at()][r].add(tally);
        }
        mask_pixels += row.mask_pixels;
        mask_wrong += row.mask_wrong;
    }
    print_rows(client, names, scale, &totals);
    print_classes(client, names, scale, &by_class);
    print_hue(client, names, &totals, tables);
    if client == Client::Ec {
        print_edges(names, &totals);
    }
    if mask_pixels > 0 {
        println!("\n{}: a hue mask through BC4 (dds, High) and back, threshold at a half: {mask_wrong} of {mask_pixels} painted pixels of masked frames take the wrong flag. A one-bit mask has none by construction.", client.label());
    }
    print_verdict(client, names, scale, &totals);
}

pub(in super::super) fn part_q(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    let hues = Hues::load(cc_dir);
    let coders = coders();
    let names: Vec<String> = coders.iter().map(|c| c.name()).collect();
    println!("\n### q: lossy block formats on every {stride}-th body by pixels, per (action, direction); classes by the placeholder rule; encode times are a loaded machine's worker seconds\n\nBC1 and BC3: the `dds` crate, quality High, `parallel` off, no dithering, alpha threshold a half (punch-through from RGBA). BC7: this repo's analytical wide encoder; RDO: this repo's, lookback {DEFAULT_BC7_RDO_LOOKBACK_BLOCKS} blocks, over a strip of the stream's blocks. BC1 reuse: a pass of this part, lookback {REUSE_LOOKBACK} blocks, inside the stream.");
    for client in [Client::Cc, Client::Ec] {
        let started = Instant::now();
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let scale = sample_scale(present, client, &sample);
        let done = AtomicUsize::new(0);
        let rows: Vec<BodyRow> = sample
            .par_iter()
            .map(|&i| {
                let body = load_body(sources, &bodies[i]);
                let planes = Planes::from_body(&body);
                let (class, _) = classes_of(&planes, &rule, &bodies[i]);
                let (tallies, mask_pixels, mask_wrong) = measure_body(&body, &planes, &coders, &hues);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                println!("  {} body {n}/{} done by {:.0} s", client.label(), sample.len(), started.elapsed().as_secs_f64());
                BodyRow { class, id: planes.label.clone(), median: median_area(&planes), lossless: 0, tallies, mask_pixels, mask_wrong }
            })
            .collect();
        let tables = if client == Client::Ec { hues.enhanced.len() } else { hues.classic.len() };
        print_all(client, &names, scale, &rows, tables);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part qc

/// A body's streams for a crowd: the frames' blocks through each row, and the plain planes.
struct CrowdStream {
    action: u16,
    rows: Vec<(u64, Vec<u8>)>,
    rgba: u64,
}

fn crowd_rows(label: &str, pairs: &[Pair], sources: &Sources, bodies: &[BodySource], coders: &[Coder], names: &[String]) {
    describe_crowd(label, pairs);
    print_crowd_header();
    let distinct: Vec<usize> = pairs.iter().map(|p| p.body()).collect::<BTreeSet<usize>>().into_iter().collect();
    let position: HashMap<usize, usize> = distinct.iter().enumerate().map(|(n, &b)| (b, n)).collect();
    let mut wanted: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); distinct.len()];
    pairs.iter().for_each(|p| {
        wanted[position[&p.body()]].insert(p.action());
    });
    // The plain row first, then the block rows; each is a body's wanted streams, already coded.
    let coded: Vec<Vec<CrowdStream>> = distinct
        .par_iter()
        .zip(&wanted)
        .map(|(&i, actions)| {
            let body = load_body(sources, &bodies[i]);
            let planes = Planes::from_body(&body);
            let strips = strips_of(&body, &planes);
            strips
                .iter()
                .filter(|strip| actions.contains(&planes.boxes[strip.frames[0]].action))
                .map(|strip| {
                    let mut plain = Vec::new();
                    for &n in &strip.frames {
                        let range = planes.boxes[n].range();
                        plain.extend_from_slice(&planes.index[range.clone()]);
                        if planes.client == Client::Ec {
                            plain.extend_from_slice(&planes.coverage[range]);
                        }
                    }
                    let mut rows = vec![(plain.len() as u64, compress(&plain, BEST_ZSTD_LEVEL, false))];
                    for &coder in coders {
                        let data = encode_strip(coder, &strip.rgba, strip.blocks);
                        rows.push((data.len() as u64, compress(&data, BEST_ZSTD_LEVEL, false)));
                    }
                    let rgba = strip.frames.iter().map(|&n| (planes.boxes[n].pixels() * RGBA_CHANNELS) as u64).sum();
                    CrowdStream { action: planes.boxes[strip.frames[0]].action, rows, rgba }
                })
                .collect()
        })
        .collect();
    let all_names: Vec<String> = std::iter::once(format!("plain planes, zstd {BEST_ZSTD_LEVEL}")).chain(names.iter().map(|n| format!("{n}, zstd {BEST_ZSTD_LEVEL}"))).collect();
    for (r, name) in all_names.iter().enumerate() {
        for (faced, unit) in [(false, "action+direction"), (true, "facing direction only")] {
            let mut read = 0u64;
            let jobs: Vec<Job> = pairs
                .iter()
                .enumerate()
                .map(|(n_pair, pair)| {
                    let streams: Vec<&CrowdStream> = coded[position[&pair.body()]].iter().filter(|s| s.action == pair.action()).collect();
                    let chosen: Vec<&CrowdStream> = if faced { vec![streams[crowd::facing_unit(n_pair, &(0..streams.len()).collect::<Vec<usize>>())]] } else { streams };
                    read += chosen.iter().map(|s| s.rows[r].1.len() as u64).sum::<u64>();
                    let stored = chosen.iter().map(|s| s.rows[r].0).sum();
                    let rgba = chosen.iter().map(|s| s.rgba).sum();
                    let parts: Vec<(usize, &[u8])> = chosen.iter().map(|s| (s.rows[r].0 as usize, s.rows[r].1.as_slice())).collect();
                    Job { run: Box::new(move || parts.iter().map(|&(raw, data)| decompress(data, raw).len()).sum()), stored, rgba }
                })
                .collect();
            crowd::print_jobs(name, unit, mb(read), &jobs);
        }
    }
}

pub(in super::super) fn part_qc(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    let lambda = std::env::var(QC_LAMBDA_ENV).ok().and_then(|text| text.parse().ok()).unwrap_or(QC_DEFAULT_LAMBDA);
    let coders = [Coder::Bc7, Coder::Rdo(lambda), Coder::Bleed, Coder::BleedRdo(lambda), Coder::Bc1, Coder::Reuse(lambda), Coder::Bc3];
    let names: Vec<String> = coders.iter().map(|c| c.name()).collect();
    println!("\n### qc: block formats in the crowds, by the placeholder rule; decode is zstd only, the GPU reads the blocks; CPU {}; load average {}", cpu_state(), load_average());
    for client in [Client::Cc, Client::Ec] {
        let pairs = crowd::all_pairs(sources, bodies, present, client);
        let npcs: Vec<Pair> = pairs.iter().copied().filter(|p| rule.group(&bodies[p.body()]) == Group::Creature).collect();
        let creatures = crowd::pick_uniform(&npcs, crowd::CROWD_PAIRS);
        let town = town_of(&pairs, &|body| rule.is_person(&bodies[body]), &|body| rule.group(&bodies[body]) == Group::Equipment);
        crowd_rows(&format!("{}, creatures", client.label()), &creatures, sources, bodies, &coders, &names);
        crowd_rows(&format!("{}, dressed NPCs", client.label()), &town, sources, bodies, &coders, &names);
        println!("\nCPU {}; load average {}", cpu_state(), load_average());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Parts qd and qm

fn dump_dir() -> PathBuf {
    PathBuf::from(std::env::var_os(DUMP_DIR_ENV).expect("the dump directory"))
}

fn client_tag(client: Client) -> &'static str {
    match client {
        Client::Cc => "cc",
        Client::Ec => "ec",
    }
}

fn write_u32(out: &mut impl Write, value: usize) {
    out.write_all(&(value as u32).to_le_bytes()).expect("the dump is written");
}

/// A record: (the body's place in the dump, block count, frame count, then each frame's width and height), then the RGBA.
fn write_strip(out: &mut impl Write, body: usize, class: usize, strip: &Strip, boxes: &[FrameBox]) {
    write_u32(out, body);
    write_u32(out, class);
    write_u32(out, strip.blocks);
    write_u32(out, strip.frames.len());
    for &n in &strip.frames {
        write_u32(out, boxes[n].w);
        write_u32(out, boxes[n].h);
    }
    out.write_all(&strip.rgba).expect("the dump is written");
}

/// A stream's plain planes, a frame after a frame, as the lossless rows store them.
fn plain_planes(planes: &Planes, strip: &Strip) -> Vec<u8> {
    let mut plain = Vec::new();
    for &n in &strip.frames {
        let range = planes.boxes[n].range();
        plain.extend_from_slice(&planes.index[range.clone()]);
        if planes.client == Client::Ec {
            plain.extend_from_slice(&planes.coverage[range]);
        }
    }
    plain
}

/// A crowd stream for the timed rows outside the workspace: its strip, its body's median area, the lossless row's bytes
/// and its BC1 and BC7 blocks.
fn write_crowd_stream(out: &mut impl Write, pair: usize, class: usize, median: u64, planes: &Planes, strip: &Strip) {
    write_strip(out, pair, class, strip, &planes.boxes);
    write_u32(out, median as usize);
    let plain = plain_planes(planes, strip);
    let stored = compress(&plain, BEST_ZSTD_LEVEL, false);
    write_u32(out, plain.len());
    write_u32(out, stored.len());
    out.write_all(&stored).expect("the dump is written");
    out.write_all(&encode_strip(Coder::Bc1, &strip.rgba, strip.blocks)).expect("the dump is written");
    out.write_all(&encode_strip(Coder::Bc7, &strip.rgba, strip.blocks)).expect("the dump is written");
}

/// One crowd's pairs, each with all its streams and the faced one marked.
fn write_crowd(path: &Path, pairs: &[Pair], sources: &Sources, bodies: &[BodySource], rule: &BodyRule) {
    let mut out = BufWriter::new(std::fs::File::create(path).expect("the dump file"));
    write_u32(&mut out, pairs.len());
    for (n_pair, pair) in pairs.iter().enumerate() {
        let body = load_body(sources, &bodies[pair.body()]);
        let planes = Planes::from_body(&body);
        let class = classes_of(&planes, rule, &bodies[pair.body()]).0.at();
        let median = median_area(&planes);
        let strips: Vec<Strip> = strips_of(&body, &planes).into_iter().filter(|strip| planes.boxes[strip.frames[0]].action == pair.action()).collect();
        let faced = crowd::facing_unit(n_pair, &(0..strips.len()).collect::<Vec<usize>>());
        write_u32(&mut out, strips.len());
        write_u32(&mut out, faced);
        for strip in &strips {
            write_crowd_stream(&mut out, n_pair, class, median, &planes, strip);
        }
    }
}

fn write_sizes(out: &mut impl Write, raw: u64, packer: u64, stored: u64) {
    for value in [raw, packer, stored] {
        out.write_all(&value.to_le_bytes()).expect("the dump is written");
    }
}

fn read_sizes(reader: &mut impl Read) -> (u64, u64, u64) {
    let mut bytes = [0u8; SIZE_RECORD_BYTES];
    reader.read_exact(&mut bytes).expect("the sizes file holds this stream");
    let at = |n: usize| u64::from_le_bytes(bytes[n * 8..(n + 1) * 8].try_into().expect("eight bytes"));
    (at(0), at(1), at(2))
}

/// One creature-crowd NPC's faced stream for the GPU draw: its frames as index, coverage, BC1 and BC7, and its palette.
fn write_gpu_stream(out: &mut impl Write, planes: &Planes, strip: &Strip) {
    let palette = planes.boxes[strip.frames[0]].palette;
    write_u32(out, strip.frames.len());
    for colour in &planes.palettes[palette] {
        out.write_all(colour).expect("the dump is written");
    }
    let padding = PALETTE_COLOURS.saturating_sub(planes.palettes[palette].len());
    out.write_all(&vec![0u8; padding * EC_PALETTE_ENTRY_BYTES]).expect("the dump is written");
    let bc7 = encode_strip(Coder::Bc7, &strip.rgba, strip.blocks);
    let bc1 = encode_strip(Coder::Bc1, &strip.rgba, strip.blocks);
    let (mut seven, mut one) = (0usize, 0usize);
    for &n in &strip.frames {
        let frame = &planes.boxes[n];
        let blocks = blocks_of(frame);
        write_u32(out, frame.w);
        write_u32(out, frame.h);
        out.write_all(&planes.index[frame.range()]).expect("the dump is written");
        if planes.client == Client::Ec {
            out.write_all(&planes.coverage[frame.range()]).expect("the dump is written");
        }
        out.write_all(&bc1[one..one + blocks * BC1_BLOCK]).expect("the dump is written");
        out.write_all(&bc7[seven..seven + blocks * BC_WIDE_BLOCK]).expect("the dump is written");
        one += blocks * BC1_BLOCK;
        seven += blocks * BC_WIDE_BLOCK;
    }
}

pub(in super::super) fn part_qd(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    let dir = dump_dir();
    std::fs::create_dir_all(&dir).expect("the dump directory");
    for client in [Client::Cc, Client::Ec] {
        let tag = client_tag(client);
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let mut out = BufWriter::new(std::fs::File::create(dir.join(format!("{tag}.{STRIPS_SUFFIX}"))).expect("the dump file"));
        let file = |name: &str, mark: &str| BufWriter::new(std::fs::File::create(dir.join(format!("{tag}{mark}{name}"))).expect("the dump file"));
        let mut lossless_out = file(LOSSLESS_NAME, SIZES_MARK);
        let block_rows = [(Coder::Bc1, BC1_NAME), (Coder::Bc7, BC7_NAME), (Coder::Bleed, BC7_BLEED_NAME)];
        let mut decoded_out: Vec<_> = block_rows.iter().map(|row| file(row.1, DECODED_MARK)).collect();
        let mut sizes_out: Vec<_> = block_rows.iter().map(|row| file(row.1, SIZES_MARK)).collect();
        let mut streams = 0usize;
        for (place, &i) in sample.iter().enumerate() {
            let body = load_body(sources, &bodies[i]);
            let planes = Planes::from_body(&body);
            let class = classes_of(&planes, &rule, &bodies[i]).0.at();
            let strips = strips_of(&body, &planes);
            let coded: Vec<_> = strips
                .par_iter()
                .map(|strip| {
                    let plain = plain_planes(&planes, strip);
                    let lossless = (plain.len() as u64, compress(&plain, PACKER_ZSTD_LEVEL, false).len() as u64, compress(&plain, BEST_ZSTD_LEVEL, false).len() as u64);
                    let rows: Vec<_> = block_rows
                        .iter()
                        .map(|row| {
                            let data = encode_strip(row.0, &strip.rgba, strip.blocks);
                            ((data.len() as u64, compress(&data, PACKER_ZSTD_LEVEL, false).len() as u64, compress(&data, BEST_ZSTD_LEVEL, false).len() as u64), decode_strip(row.0, &data, strip.blocks))
                        })
                        .collect();
                    (lossless, rows)
                })
                .collect();
            for (strip, (lossless, rows)) in strips.iter().zip(&coded) {
                write_strip(&mut out, place, class, strip, &planes.boxes);
                write_sizes(&mut lossless_out, lossless.0, lossless.1, lossless.2);
                for (n, (sizes, decoded)) in rows.iter().enumerate() {
                    write_sizes(&mut sizes_out[n], sizes.0, sizes.1, sizes.2);
                    decoded_out[n].write_all(decoded).expect("the dump is written");
                }
                streams += 1;
            }
        }
        println!("{}: {} bodies, {streams} streams dumped; every {stride}-th body by pixels", client.label(), sample.len());

        // The creature crowd, a pair at a time: all its streams, the faced one marked, for decode rows and the GPU.
        let pairs = crowd::all_pairs(sources, bodies, present, client);
        let npcs: Vec<Pair> = pairs.iter().copied().filter(|p| rule.group(&bodies[p.body()]) == Group::Creature).collect();
        let creatures = crowd::pick_uniform(&npcs, crowd::CROWD_PAIRS);
        let town = town_of(&pairs, &|body| rule.is_person(&bodies[body]), &|body| rule.group(&bodies[body]) == Group::Equipment);
        write_crowd(&dir.join(format!("{tag}.{CROWD_SUFFIX}")), &creatures, sources, bodies, &rule);
        write_crowd(&dir.join(format!("{tag}.{CROWD_DRESSED_SUFFIX}")), &town, sources, bodies, &rule);
        let mut gpu_out = BufWriter::new(std::fs::File::create(dir.join(format!("{tag}.{GPU_SUFFIX}"))).expect("the dump file"));
        write_u32(&mut gpu_out, creatures.len());
        for (n_pair, pair) in creatures.iter().enumerate() {
            let body = load_body(sources, &bodies[pair.body()]);
            let planes = Planes::from_body(&body);
            let strips: Vec<Strip> = strips_of(&body, &planes).into_iter().filter(|strip| planes.boxes[strip.frames[0]].action == pair.action()).collect();
            let faced = crowd::facing_unit(n_pair, &(0..strips.len()).collect::<Vec<usize>>());
            write_gpu_stream(&mut gpu_out, &planes, &strips[faced]);
        }
        println!("{}: the crowds dumped: {} creature pairs, {} dressed NPC pairs", client.label(), creatures.len(), town.len());
    }
}

/// Every `<tag>.decoded.<name>` in the dump directory, sorted.
fn decoded_files(dir: &Path, tag: &str) -> Vec<(String, PathBuf)> {
    let prefix = format!("{tag}{DECODED_MARK}");
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .expect("the dump directory")
        .filter_map(|entry| {
            let path = entry.expect("a dump file").path();
            let name = path.file_name()?.to_str()?.strip_prefix(prefix.as_str())?.to_string();
            Some((name, path))
        })
        .collect();
    out.sort();
    out
}

/// What the mix's rows need of a sample body's place in the decoded files.
#[derive(Clone, Copy)]
struct Place {
    source: usize,
    offset: u64,
}

/// The rows of the mix: above a threshold, a format's name; the rest lossless or a second format.
struct MixRow {
    name: String,
    top: u64,
    above: usize,
    below: Option<usize>,
}

fn mix_rows(client: Client, names: &[String]) -> Vec<MixRow> {
    let at = |name: &str| names.iter().position(|n| n == name);
    let mut wanted: Vec<&str> = vec![BC1_NAME, ASTC_MID_NAME, ASTC_BIG_NAME];
    if client == Client::Ec {
        wanted.splice(1..1, [BC7_NAME, BC7_BLEED_NAME]);
    }
    let mut out = Vec::new();
    for top in std::iter::once(0).chain(MIX_TOPS) {
        for name in &wanted {
            if let Some(above) = at(name) {
                out.push(MixRow { name: name.to_string(), top, above, below: None });
            }
        }
        for rest in [BC7_NAME, BC7_BLEED_NAME] {
            if let (Client::Ec, Some(bc1), Some(bc7), true) = (client, at(BC1_NAME), at(rest), top > 0) {
                out.push(MixRow { name: format!("{BC1_NAME}, the rest {rest}"), top, above: bc1, below: Some(bc7) });
            }
        }
    }
    out
}

fn print_mix_rows(client: Client, names: &[String], scale: f64, rows: &[BodyRow]) {
    let lossless_all: u64 = rows.iter().map(|r| r.lossless).sum();
    println!("\n#### {}: the low-end mix: a body above the threshold takes the format, the rest stay lossless (plain planes, zstd {BEST_ZSTD_LEVEL}, per stream). Disk is the whole set by this sample's share of the pixels (x {scale:.3}), {:.1} MB all lossless, palettes not counted. Scores and hue are the block-coded bodies only\n\n| above | format | bodies | Mpx coded | disk MB | x all lossless | SSIMULACRA2 median | p5 | min | at {SCORE_LINE} or better | RGB PSNR dB | channel error | painted pixels gone | hue key changed | key moved | hued error | hued worst | edge levels, mean | worst bodies |\n|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|", client.label(), mb(lossless_all) * scale);
    for row in mix_rows(client, names) {
        let mut tally = Tally::default();
        let mut disk = 0u64;
        let mut coded = 0usize;
        let mut scored: Vec<(f64, &BodyRow)> = Vec::new();
        for body in rows {
            if body.median > row.top {
                let own = &body.tallies[row.above];
                tally.add(own);
                disk += own.zstd_best;
                coded += 1;
                scored.extend(own.scores.iter().map(|&score| (score, body)));
            } else if let Some(below) = row.below {
                let own = &body.tallies[below];
                tally.add(own);
                disk += own.zstd_best;
                coded += 1;
                scored.extend(own.scores.iter().map(|&score| (score, body)));
            } else {
                disk += body.lossless;
            }
        }
        if coded == 0 {
            println!("| {} | {} | 0 | | {:.1} | 1.000 | no body in this sample | | | | | | | | | | | | |", row.top, row.name, mb(disk) * scale);
            continue;
        }
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        let worst: Vec<String> = scored.iter().take(WORST_BODIES_NAMED).map(|(score, body)| format!("{} ({}, median {} px): {score:.1}", body.id, body.class.label(), body.median)).collect();
        let (median, p5, min) = scores_of(&tally);
        let good = tally.scores.iter().filter(|&&s| s >= SCORE_LINE).count();
        println!(
            "| {} | {} | {coded} | {:.1} | {:.1} | {:.3} | {median:.2} | {p5:.2} | {min:.2} | {good} of {} | {:.2} | {:.3} | {:.3} % | {:.2} % | {:.3} | {:.3} | {} | {:.3} | {} |",
            row.top,
            row.name,
            tally.pixels as f64 / PIXELS_PER_MEGAPIXEL,
            mb(disk) * scale,
            disk as f64 / lossless_all.max(1) as f64,
            tally.scores.len(),
            psnr_of(&tally),
            mean(tally.channel_sum, tally.channel_samples),
            share(tally.vanished, tally.painted),
            share(tally.key_changed, tally.hue_pixels),
            mean(tally.key_sum, tally.hue_pixels),
            mean(tally.hued_sum, tally.hued_samples),
            tally.hued_worst,
            tally.edge_sum / tally.edges.max(1) as f64,
            worst.join("; "),
        );
    }
}

fn composite_error(truth: &[u8], decoded: &[u8]) -> f64 {
    let background = SSIM_BACKGROUND.map(f64::from);
    let mut sum = 0.0f64;
    for (t, d) in truth.chunks_exact(RGBA_CHANNELS).zip(decoded.chunks_exact(RGBA_CHANNELS)) {
        let (ta, da) = (f64::from(t[ALPHA]) / FULL_BYTE, f64::from(d[ALPHA]) / FULL_BYTE);
        for c in 0..RGB {
            let off = background[c] * (1.0 - ta) + f64::from(t[c]) * ta - background[c] * (1.0 - da) - f64::from(d[c]) * da;
            sum += off * off;
        }
    }
    sum / (truth.len() / RGBA_CHANNELS).max(1) as f64
}

fn write_preview(path: &Path, frame: &Frame, cells: &[&[u8]]) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let zoom = PREVIEW_ZOOMS.iter().find(|row| w <= row.0).map_or(1, |row| row.1);
    let columns = cells.len() + 1;
    let (sheet_w, sheet_h) = (columns * w * zoom + (columns - 1) * PREVIEW_GAP, h * zoom);
    let mut rgb = vec![0u8; sheet_w * sheet_h * RGB];
    for (c, rgba) in std::iter::once(frame.rgba.as_slice()).chain(cells.iter().copied()).enumerate() {
        let left = c * (w * zoom + PREVIEW_GAP);
        for y in 0..sheet_h {
            for x in 0..w * zoom {
                let p = &rgba[(y / zoom * w + x / zoom) * RGBA_CHANNELS..][..RGBA_CHANNELS];
                let a = f32::from(p[ALPHA]) / FULL_BYTE as f32;
                for k in 0..RGB {
                    rgb[(y * sheet_w + left + x) * RGB + k] = (f32::from(SSIM_BACKGROUND[k]) * (1.0 - a) + f32::from(p[k]) * a).round() as u8;
                }
            }
        }
    }
    super::super::halved::write_png(path, sheet_w, sheet_h, &rgb);
}

/// Side-by-side crops: lossless, BC1, ASTC 6 x 6, ASTC 8 x 8, on the bodies that score worst and median, and within each its
/// worst and median frame; `index.txt` says what each file is.
#[allow(clippy::too_many_arguments)]
fn previews(client: Client, tag: &str, names: &[String], files: &[(String, PathBuf)], rows: &[BodyRow], places: &[Place], sources: &Sources, bodies: &[BodySource], out_dir: &Path, index: &mut impl Write) {
    let at = |name: &str| names.iter().position(|n| n == name);
    let (Some(bc1), Some(mid), Some(big)) = (at(BC1_NAME), at(ASTC_MID_NAME), at(ASTC_BIG_NAME)) else {
        println!("{}: no previews: BC1, ASTC 6 x 6 and ASTC 8 x 8 aren't all decoded", client.label());
        return;
    };
    let shown = [bc1, mid, big];
    let score_of = |row: &BodyRow, r: usize| row.tallies[r].scores.iter().copied().fold(f64::MAX, f64::min);
    let mut chosen: Vec<(String, usize, usize)> = Vec::new();
    for class in [Class::Large, Class::Equipment] {
        let mut of: Vec<usize> = (0..rows.len()).filter(|&n| rows[n].class == class).collect();
        if of.is_empty() {
            continue;
        }
        let mut pick = |what: &str, r: usize| {
            of.sort_by(|&a, &b| score_of(&rows[a], r).total_cmp(&score_of(&rows[b], r)));
            let label = format!("{}-{what}", class.label().replace(' ', "-"));
            chosen.push((format!("{label}-worst"), of[0], r));
            chosen.push((format!("{label}-median"), of[of.len() / 2], r));
        };
        pick("by-bc1", bc1);
        pick("by-astc8", big);
    }
    for (label, place, r) in chosen {
        let row = &rows[place];
        let body = load_body(sources, &bodies[places[place].source]);
        let planes = Planes::from_body(&body);
        let truth: Vec<&Frame> = body.directions.iter().flatten().collect();
        let strips = strips_of(&body, &planes);
        let decoded: Vec<Vec<Vec<u8>>> = shown
            .iter()
            .map(|&s| {
                let mut reader = BufReader::new(std::fs::File::open(&files[s].1).expect("a decoded file"));
                reader.seek(std::io::SeekFrom::Start(places[place].offset)).expect("a seek");
                let mut frames: Vec<Vec<u8>> = vec![Vec::new(); planes.boxes.len()];
                for strip in &strips {
                    let mut bytes = vec![0u8; strip.blocks * BLOCK_RGBA];
                    reader.read_exact(&mut bytes).expect("the decoded file holds this stream");
                    frames_of_strip(&bytes, &strip.frames, &planes.boxes, &mut frames);
                }
                frames
            })
            .collect();
        // The frames by their error under the row that picked this body.
        let row_cell = shown.iter().position(|&s| s == r).expect("a shown row");
        let mut ranked: Vec<(f64, usize)> = (0..truth.len()).filter(|&n| planes.boxes[n].pixels() >= PREVIEW_MIN_AREA).map(|n| (composite_error(&truth[n].rgba, &decoded[row_cell][n]), n)).collect();
        if ranked.is_empty() {
            continue;
        }
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        let id: String = row.id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        for (frame_label, n) in [("worst-frame", ranked[ranked.len() - 1].1), ("median-frame", ranked[ranked.len() / 2].1)] {
            let cells: Vec<&[u8]> = decoded.iter().map(|frames| frames[n].as_slice()).collect();
            let name = format!("{tag}_{label}_{id}_{frame_label}.png");
            write_preview(&out_dir.join(&name), truth[n], &cells);
            let scores: Vec<String> = cells
                .iter()
                .zip(&shown)
                .map(|(cell, &s)| {
                    let reference = [(truth[n].width, truth[n].height, truth[n].rgba.as_slice())];
                    let test = [(truth[n].width, truth[n].height, *cell)];
                    let score = compute_frame_ssimulacra2(mosaic(&reference), mosaic(&test)).expect("SSIMULACRA2");
                    format!("{} frame {score:.1}, body {:.1}", names[s], score_of(row, s))
                })
                .collect();
            let frame = &planes.boxes[n];
            writeln!(index, "{name}: {}, {}, median {} px; action {}, direction {}, {} x {}; columns: lossless, {}, {}, {}; {}", row.id, row.class.label(), row.median, frame.action, frame.direction, frame.w, frame.h, names[bc1], names[mid], names[big], scores.join("; ")).expect("the index");
        }
    }
}

pub(in super::super) fn part_qm(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    let hues = Hues::load(cc_dir);
    let dir = dump_dir();
    let preview_dir = std::env::var_os(PREVIEW_DIR_ENV).map(PathBuf::from);
    println!("\n### qm: what the encoders outside the workspace decoded, measured as part q measures, on every {stride}-th body by pixels; sizes per (action, direction), from the encoders; stride {stride}; classes by the placeholder rule");
    for client in [Client::Cc, Client::Ec] {
        let tag = client_tag(client);
        let files = decoded_files(&dir, tag);
        if files.is_empty() {
            continue;
        }
        let names: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let scale = sample_scale(present, client, &sample);
        let mut readers: Vec<BufReader<std::fs::File>> = files.iter().map(|f| BufReader::new(std::fs::File::open(&f.1).expect("a decoded file"))).collect();
        let mut sizes: Vec<Option<BufReader<std::fs::File>>> = names.iter().map(|name| std::fs::File::open(dir.join(format!("{tag}{SIZES_MARK}{name}"))).ok().map(BufReader::new)).collect();
        let mut lossless_sizes = BufReader::new(std::fs::File::open(dir.join(format!("{tag}{SIZES_MARK}{LOSSLESS_NAME}"))).expect("the lossless sizes"));
        let mut rows = Vec::with_capacity(sample.len());
        let mut places = Vec::with_capacity(sample.len());
        let mut offset = 0u64;
        for (done, &i) in sample.iter().enumerate() {
            let body = load_body(sources, &bodies[i]);
            let planes = Planes::from_body(&body);
            let (class, _) = classes_of(&planes, &rule, &bodies[i]);
            let truth: Vec<&Frame> = body.directions.iter().flatten().collect();
            let strips = strips_of(&body, &planes);
            places.push(Place { source: i, offset });
            offset += strips.iter().map(|s| (s.blocks * BLOCK_RGBA) as u64).sum::<u64>();
            let per_file: Vec<Vec<Vec<u8>>> = readers
                .iter_mut()
                .map(|reader| {
                    let mut decoded: Vec<Vec<u8>> = vec![Vec::new(); planes.boxes.len()];
                    for strip in &strips {
                        let mut bytes = vec![0u8; strip.blocks * BLOCK_RGBA];
                        reader.read_exact(&mut bytes).expect("the decoded file holds this stream");
                        frames_of_strip(&bytes, &strip.frames, &planes.boxes, &mut decoded);
                    }
                    decoded
                })
                .collect();
            let mut tallies: Vec<Tally> = per_file
                .par_iter()
                .map(|decoded| {
                    let mut tally = Tally::default();
                    tally_body(&planes, &truth, decoded, &hues, &mut tally);
                    tally
                })
                .collect();
            let mut lossless = 0u64;
            for _ in &strips {
                lossless += read_sizes(&mut lossless_sizes).2;
                for (tally, reader) in tallies.iter_mut().zip(&mut sizes) {
                    if let Some(reader) = reader {
                        let (raw, packer, stored) = read_sizes(reader);
                        tally.raw += raw;
                        tally.zstd_packer += packer;
                        tally.zstd_best += stored;
                    }
                }
            }
            println!("  {} body {}/{} scored", client.label(), done + 1, sample.len());
            rows.push(BodyRow { class, id: planes.label.clone(), median: median_area(&planes), lossless, tallies, mask_pixels: 0, mask_wrong: 0 });
        }
        let tables = if client == Client::Ec { hues.enhanced.len() } else { hues.classic.len() };
        print_all(client, &names, scale, &rows, tables);
        print_mix_rows(client, &names, scale, &rows);
        if let Some(out_dir) = &preview_dir {
            std::fs::create_dir_all(out_dir).expect("the preview directory");
            let mut index = BufWriter::new(std::fs::File::options().create(true).append(true).open(out_dir.join("index.txt")).expect("the preview index"));
            previews(client, tag, &names, &files, &rows, &places, sources, bodies, out_dir, &mut index);
        }
    }
}
