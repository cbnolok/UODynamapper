//! The `shrink` pass of the mobile animation storage benchmark: how far the official animations can shrink, and what
//! that costs in decode time (results: `docs/agent_notes/proposals/2026-11-01-mobile-anim-codec.md`).
//!
//! The parts, chosen with `MOBILE_ANIM_CODEC_SHRINK_PARTS` (comma-separated; all but `a2` when unset):
//! - `a1`: exact duplicates over the whole set (frame, direction, action, body), and bits per opaque pixel;
//! - `a2`: one solid Zstd stream of everything with a 2 GiB window (`/usr/bin/zstd`), the cross-body upper bound;
//! - `a2p`: the practical forms of cross-body reuse: a trained dictionary, and the largest bodies as a raw reference;
//! - `w`: the candidates named by `MOBILE_ANIM_CODEC_SHRINK_ONLY` over the whole set, a body at a time;
//! - `b`: the stream candidates ([`candidates`]) on a sample: Zstd levels, palette orders, predictors and a static
//!   rANS proof of concept with 2D contexts, stream units, coverage coders; `MOBILE_ANIM_CODEC_SHRINK_ONLY` filters
//!   them by name (comma-separated substrings);
//! - `c`: general-purpose bounds: xz, brotli (their command-line tools, as they are bounds only), WebP lossless and
//!   JPEG XL modular lossless per frame and per direction strip;
//! - `d`: the lossy coverage bound (16 levels reduced to 8 and 4); `e`: the lossy near-lossless index tier;
//! - `f`: the winners on upscaled content, and the cost of upscaling when an action is loaded instead;
//! - `g`: the 20 largest bodies, the stream units and whole-body decode with worker threads.
//!
//! `MOBILE_ANIM_CODEC_SHRINK_STRIDE=n` samples every n-th body by pixel count (10 when unset; 1 is the whole set).
//! `MOBILE_ANIM_CODEC_SHRINK_TIME=1` also times the candidates of `b` on one thread, after encoding them in parallel.
//! `k1`..`k7` (`k`, all of them) and `r`: what knowledge of the data buys, see the `knowledge` module.
//! `k8`: keep about half of the frames and rebuild the others (lossy), see the `halved` module.
//! Every lossless candidate is decoded and compared with its source: a mismatch fails the test.

use super::*;

use jpegxl_rs::encode::{EncoderFrame, EncoderSpeed};
use jpegxl_rs::{decoder_builder, encoder_builder};
use libwebp_sys as webp;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

mod crowd;
mod derive;
mod floor;
mod halved;
mod knowledge;
mod video;

const PARTS_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_PARTS";
const DEFAULT_PARTS: &str = "a1,b,c,d,e,f,g";
const STRIDE_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_STRIDE";
const DEFAULT_STRIDE: usize = 10;
const ONLY_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_ONLY";
const TIME_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_TIME";
/// Bodies per client of part `f`; timing goes one body at a time, so it takes fewer.
const UPSCALE_BODIES_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_UPSCALE_BODIES";
/// Client data file naming a body's kind.
const MOBTYPES_FILE: &str = "mobtypes.txt";

const LEVEL_19: i32 = 19;
const LEVEL_22: i32 = 22;
/// The window of the level 22 variants (128 MiB, the decoder's default limit).
const LARGE_WINDOW_LOG: u32 = 27;
/// The solid stream's window, `zstd --long=31` (2 GiB).
const SOLID_WINDOW_LOG: u32 = 31;
const COMMAND_ZSTD: &str = "zstd";
const COMMAND_XZ: &str = "xz";
const COMMAND_BROTLI: &str = "brotli";
const BROTLI_LARGE_WINDOW_BITS: u32 = 30;
const TEXT_CHUNK_BODIES: usize = 20;

// rANS: 32-bit state, byte-wise renormalisation, 12-bit probabilities.
const RANS_SCALE_BITS: u32 = 12;
const RANS_SCALE: u32 = 1 << RANS_SCALE_BITS;
const RANS_SLOT_MASK: u32 = RANS_SCALE - 1;
const RANS_LOWER_BOUND: u32 = 1 << 23;
const RANS_BYTE_BITS: u32 = 8;
const RANS_STATE_BYTES: usize = 4;
const BYTE_SYMBOLS: usize = 256;
const BINARY_SYMBOLS: usize = 2;
/// Coverage values 0..=16.
const COVERAGE_SYMBOLS: usize = 17;
/// Coverage in the coder's order: the two common values first, for its linear symbol search.
const COVERAGE_SYMBOL_OF: [u8; COVERAGE_SYMBOLS] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 1];
const COVERAGE_OF_SYMBOL: [u8; COVERAGE_SYMBOLS] = [0, 16, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// Largest alphabet decoded by a linear search, without a slot table.
const LINEAR_SEARCH_SYMBOLS: usize = COVERAGE_SYMBOLS;

/// Gradient buckets of the index coder, then one context for a single known neighbour and one for none.
const GRADIENT_BUCKET_TOPS: [u16; 7] = [0, 1, 2, 4, 8, 16, 32];
const GRADIENT_BUCKETS: usize = GRADIENT_BUCKET_TOPS.len() + 1;
const CONTEXT_ONE_NEIGHBOUR: usize = GRADIENT_BUCKETS;
const CONTEXT_NO_NEIGHBOUR: usize = GRADIENT_BUCKETS + 1;
const INDEX_CONTEXTS: usize = GRADIENT_BUCKETS + 2;
/// The previous frame's agreement with the prediction, as extra index contexts: none, equal, near, far.
const PREVIOUS_BUCKETS: usize = 4;
const PREVIOUS_NONE: usize = 0;
const PREVIOUS_EQUAL: usize = 1;
const PREVIOUS_NEAR: usize = 2;
const PREVIOUS_FAR: usize = 3;
const PREVIOUS_NEAR_LIMIT: u8 = 4;
/// West, north, north-west, north-east, west-west, north-north.
const MASK_CONTEXTS: usize = 1 << 6;
/// Coverage of west and north in five classes, of north-east in three.
const COVERAGE_LEVEL_CLASSES: usize = 5;
const COVERAGE_KIND_CLASSES: usize = 3;
const COVERAGE_CONTEXTS: usize = COVERAGE_LEVEL_CLASSES * COVERAGE_LEVEL_CLASSES * COVERAGE_KIND_CLASSES;
const COVERAGE_LOW_TOP: u8 = 5;
const COVERAGE_MID_TOP: u8 = 10;
const COVERAGE_HIGH_TOP: u8 = 15;
/// The value predicted for a frame's first opaque pixel.
const FIRST_PREDICTION: u8 = 128;
const MAX_CHANNEL: i32 = 255;

const LUMA_WEIGHTS: [u32; 3] = [299, 587, 114];
const LENGTH_PREFIX_BYTES: usize = 4;

/// Raw bytes below which an action is merged with the next one ([`Unit::Grouped`]).
const GROUP_MINIMUMS: [usize; 3] = [32 * 1024, 64 * 1024, 256 * 1024];
const WORKER_THREADS: usize = 4;
const LARGEST_BODIES: usize = 20;
const WEBP_MAX_SIDE: usize = 16_383;
const WEBP_METHOD: i32 = 6;
const WEBP_QUALITY: f32 = 100.0;
const JXL_EFFORTS: [(EncoderSpeed, &str); 2] = [(EncoderSpeed::Squirrel, "effort 7"), (EncoderSpeed::Tortoise, "effort 9")];
const BOUND_SAMPLE_PER_CLIENT: usize = 25;
const JXL_EFFORT9_SAMPLE_PER_CLIENT: usize = 12;
const DICTIONARY_BYTES: [usize; 2] = [112 * 1024, 1024 * 1024];
/// Samples given to the dictionary trainer per body.
const DICTIONARY_SAMPLES_PER_BODY: usize = 64;
/// Every n-th chunk of a body goes to the trainer.
const DICTIONARY_SAMPLE_EVERY: usize = 2;
/// Bodies used together as a reference for the others.
const REFERENCE_BODIES: usize = 5;
const BYTES_PER_KIB: usize = 1024;
/// Classic palette entries on disk: RGB555.
const CC_PALETTE_ENTRY_BYTES: usize = 2;
/// Near-lossless thresholds (per channel, 8-bit), by client: Classic colours move in steps of 8.
const NEAR_LOSSLESS_CC: [u8; 2] = [8, 16];
const NEAR_LOSSLESS_EC: [u8; 3] = [2, 4, 8];
const COVERAGE_LEVEL_STEPS: [(u8, &str); 2] = [(2, "8 levels"), (4, "4 levels")];

fn load_average() -> String {
    std::fs::read_to_string(LOADAVG_PATH).unwrap_or_default().trim().to_string()
}

// ---------------------------------------------------------------------------------------------------------------
// The planes

#[derive(Clone)]
struct FrameBox {
    w: usize,
    h: usize,
    left: i32,
    top: i32,
    action: u16,
    direction: u32,
    palette: usize,
    /// Pixel offset in the planes.
    start: usize,
}

impl FrameBox {
    fn pixels(&self) -> usize {
        self.w * self.h
    }

    fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.pixels()
    }
}

/// One body as separate planes; the index under a transparent pixel is 0.
#[derive(Clone)]
struct Planes {
    label: String,
    client: Client,
    boxes: Vec<FrameBox>,
    index: Vec<u8>,
    /// Enhanced only: 0..=16; empty for Classic.
    coverage: Vec<u8>,
    /// R, G, B, keep-colour flag.
    palettes: Vec<Vec<[u8; EC_PALETTE_ENTRY_BYTES]>>,
    /// Today's packer (BC7 + Zstd 7) for this body, 0 when not measured.
    bc7_zstd: u64,
    /// Non-zero indices under transparent pixels that were cleared.
    cleared: u64,
}

impl Planes {
    fn from_body(body: &Body) -> Planes {
        let mut boxes = Vec::with_capacity(body.frame_count());
        let pixels = body.pixel_count() as usize;
        let mut index = Vec::with_capacity(pixels);
        let mut coverage = Vec::with_capacity(if body.client == Client::Ec { pixels } else { 0 });
        let mut cleared = 0;
        for (direction, frames) in body.directions.iter().enumerate() {
            for frame in frames {
                boxes.push(FrameBox {
                    w: frame.width as usize,
                    h: frame.height as usize,
                    left: frame.left,
                    top: frame.top,
                    action: frame.action,
                    direction: direction as u32,
                    palette: frame.palette,
                    start: index.len(),
                });
                match body.client {
                    Client::Cc => index.extend_from_slice(&frame.pixels),
                    Client::Ec => {
                        for pixel in frame.pixels.chunks_exact(EC_PIXEL_BYTES) {
                            let transparent = pixel[1] == COVERAGE_TRANSPARENT;
                            cleared += u64::from(transparent && pixel[0] != 0);
                            index.push(if transparent { 0 } else { pixel[0] });
                            coverage.push(pixel[1]);
                        }
                    }
                }
            }
        }
        let palettes = (0..body.palettes.len())
            .map(|palette| match body.client {
                Client::Cc => palette_lut(body, palette).into_iter().map(|[r, g, b, _]| [r, g, b, 0]).collect(),
                // Padded, as a short palette's opaque pixels can hold indices past its end (they draw as black).
                Client::Ec => {
                    let mut colours: Vec<[u8; EC_PALETTE_ENTRY_BYTES]> = body.palettes[palette]
                        .chunks_exact(EC_PALETTE_ENTRY_BYTES)
                        .map(|c| [c[0], c[1], c[2], c[PALETTE_KEEP_COLOUR_BYTE]])
                        .collect();
                    colours.resize(PALETTE_COLOURS, [0; EC_PALETTE_ENTRY_BYTES]);
                    colours
                }
            })
            .collect();
        Planes { label: body.label.clone(), client: body.client, boxes, index, coverage, palettes, bc7_zstd: 0, cleared }
    }

    fn is_opaque(&self, i: usize) -> bool {
        match self.client {
            Client::Cc => self.index[i] != 0,
            Client::Ec => self.coverage[i] != COVERAGE_TRANSPARENT,
        }
    }

    fn opaque_pixels(&self) -> u64 {
        (0..self.index.len()).filter(|&i| self.is_opaque(i)).count() as u64
    }

    /// Output bytes of the resident format: R8, or RG8 for the Enhanced.
    fn resident_bytes(&self) -> u64 {
        (self.index.len() + self.coverage.len()) as u64
    }

    /// The colour of every opaque pixel in order, hashed: palette orders must not change it.
    fn colour_signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for frame in &self.boxes {
            let palette = &self.palettes[frame.palette];
            for i in frame.range() {
                if self.is_opaque(i) {
                    palette[usize::from(self.index[i])].hash(&mut hasher);
                    if self.client == Client::Ec {
                        self.coverage[i].hash(&mut hasher);
                    }
                }
            }
        }
        hasher.finish()
    }
}

fn load_planes(sources: &Sources, source: &BodySource, measure_bc7: bool) -> Planes {
    let body = load_body(sources, source);
    let mut planes = Planes::from_body(&body);
    if measure_bc7 {
        planes.bc7_zstd = compress(&bc7_plane(&body, Bc7EncoderBackend::AnalyticalWide), PACKER_ZSTD_LEVEL, false).len() as u64;
    }
    planes
}

// ---------------------------------------------------------------------------------------------------------------
// Palette orders

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Order {
    /// As shipped.
    Original,
    Luma,
    /// A greedy nearest-neighbour path through RGB, from the darkest used entry.
    Path,
    /// The same path through all the entries, used or not: the order depends on the palette alone, so palettes shared
    /// by bodies stay shared.
    PathAll,
    /// A greedy path through how often entries touch in the image (modified Zeng).
    Cooccurrence,
    /// A path through the palette with the most opaque pixels, whose slots the other palettes of the body follow: each
    /// entry takes the free slot of the nearest colour, so an index means about the same colour across actions.
    Master,
}

fn luma(colour: &[u8; EC_PALETTE_ENTRY_BYTES]) -> u32 {
    (0..LUMA_WEIGHTS.len()).map(|c| LUMA_WEIGHTS[c] * u32::from(colour[c])).sum()
}

fn colour_distance(a: &[u8; EC_PALETTE_ENTRY_BYTES], b: &[u8; EC_PALETTE_ENTRY_BYTES]) -> i32 {
    (0..LUMA_WEIGHTS.len()).map(|c| (i32::from(a[c]) - i32::from(b[c])).pow(2)).sum()
}

/// The used entries in the new order, for one palette.
fn ordered_entries(palette: &[[u8; EC_PALETTE_ENTRY_BYTES]], used: &[usize], touching: &[u32], order: Order) -> Vec<usize> {
    let by_luma = || {
        let mut entries = used.to_vec();
        entries.sort_by_key(|&e| (luma(&palette[e]), palette[e], e));
        entries
    };
    match order {
        Order::Original => used.to_vec(),
        Order::Luma => by_luma(),
        Order::Path | Order::PathAll | Order::Master => {
            let mut rest = by_luma();
            if rest.is_empty() {
                return rest;
            }
            let mut path = vec![rest.remove(0)];
            while !rest.is_empty() {
                let last = *path.last().unwrap();
                let (next, _) = rest
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &e)| colour_distance(&palette[last], &palette[e]))
                    .unwrap();
                path.push(rest.remove(next));
            }
            path
        }
        Order::Cooccurrence => {
            let mut rest = by_luma();
            if rest.is_empty() {
                return rest;
            }
            let weight = |a: usize, b: usize| touching[a * BYTE_SYMBOLS + b] + touching[b * BYTE_SYMBOLS + a];
            // Start from the most often touching entry, grow the path at either end.
            let start = (0..rest.len())
                .max_by_key(|&i| rest.iter().map(|&other| u64::from(weight(rest[i], other))).sum::<u64>())
                .unwrap();
            let mut path = std::collections::VecDeque::from([rest.remove(start)]);
            while !rest.is_empty() {
                let (head, tail) = (*path.front().unwrap(), *path.back().unwrap());
                let (next, _, at_head) = rest
                    .iter()
                    .enumerate()
                    .map(|(i, &e)| {
                        let (to_head, to_tail) = (weight(e, head), weight(e, tail));
                        (i, to_head.max(to_tail), to_head > to_tail)
                    })
                    .max_by_key(|&(i, score, _)| (score, std::cmp::Reverse(i)))
                    .unwrap();
                let entry = rest.remove(next);
                if at_head {
                    path.push_front(entry);
                } else {
                    path.push_back(entry);
                }
            }
            path.into_iter().collect()
        }
    }
}

/// The planes with every palette reordered, and the indices remapped. Hue flags stay with their entry; Classic keeps
/// entry 0 (transparent) in place; unused entries go last.
fn reordered(planes: &Planes, order: Order) -> Planes {
    if order == Order::Original {
        return planes.clone();
    }
    if order == Order::Master {
        return master_reordered(planes);
    }
    let mut out = planes.clone();
    for palette in 0..planes.palettes.len() {
        let entries = planes.palettes[palette].len();
        let mut used = vec![false; BYTE_SYMBOLS];
        let mut touching = vec![0u32; BYTE_SYMBOLS * BYTE_SYMBOLS];
        for frame in planes.boxes.iter().filter(|f| f.palette == palette) {
            for y in 0..frame.h {
                for x in 0..frame.w {
                    let i = frame.start + y * frame.w + x;
                    if !planes.is_opaque(i) {
                        continue;
                    }
                    let value = usize::from(planes.index[i]);
                    used[value] = true;
                    if x > 0 && planes.is_opaque(i - 1) {
                        touching[usize::from(planes.index[i - 1]) * BYTE_SYMBOLS + value] += 1;
                    }
                    if y > 0 && planes.is_opaque(i - frame.w) {
                        touching[usize::from(planes.index[i - frame.w]) * BYTE_SYMBOLS + value] += 1;
                    }
                }
            }
        }
        let reserved = usize::from(planes.client == Client::Cc);
        let used_entries: Vec<usize> = (reserved..entries).filter(|&e| used[e] || order == Order::PathAll).collect();
        let mut new_order: Vec<usize> = (0..reserved).collect();
        new_order.extend(ordered_entries(&planes.palettes[palette], &used_entries, &touching, order));
        new_order.extend((reserved..entries).filter(|&e| !used[e] && order != Order::PathAll));
        debug_assert_eq!(new_order.len(), entries);
        let mut new_of_old = [0u8; BYTE_SYMBOLS];
        for (new, &old) in new_order.iter().enumerate() {
            new_of_old[old] = new as u8;
        }
        out.palettes[palette] = new_order.iter().map(|&old| planes.palettes[palette][old]).collect();
        for frame in planes.boxes.iter().filter(|f| f.palette == palette) {
            for i in frame.range() {
                if planes.is_opaque(i) {
                    out.index[i] = new_of_old[usize::from(planes.index[i])];
                }
            }
        }
    }
    assert_eq!(planes.colour_signature(), out.colour_signature(), "{}: the palette order changed a colour", planes.label);
    out
}

/// [`Order::Master`].
fn master_reordered(planes: &Planes) -> Planes {
    let reserved = usize::from(planes.client == Client::Cc);
    let mut counts = vec![[0u64; BYTE_SYMBOLS]; planes.palettes.len()];
    for frame in &planes.boxes {
        for i in frame.range() {
            if planes.is_opaque(i) {
                counts[frame.palette][usize::from(planes.index[i])] += 1;
            }
        }
    }
    let total = |palette: usize| counts[palette].iter().sum::<u64>();
    let Some(master) = (0..planes.palettes.len()).max_by_key(|&palette| total(palette)) else { return planes.clone() };
    let used = |palette: usize| -> Vec<usize> { (reserved..PALETTE_COLOURS).filter(|&e| counts[palette][e] > 0).collect() };
    let path = ordered_entries(&planes.palettes[master], &used(master), &[], Order::Master);
    // The master's slots, in the path's order: its colour per slot.
    let mut slots: Vec<[u8; EC_PALETTE_ENTRY_BYTES]> = path.iter().map(|&e| planes.palettes[master][e]).collect();
    slots.resize(PALETTE_COLOURS - reserved, [0; EC_PALETTE_ENTRY_BYTES]);
    let mut out = planes.clone();
    for palette in 0..planes.palettes.len() {
        let mut new_of_old = [0u8; BYTE_SYMBOLS];
        let mut free = vec![true; slots.len()];
        let mut taken = vec![false; BYTE_SYMBOLS];
        let entries = used(palette);
        // The closest pairs first; a slot beyond the master's colours is free for the last entries.
        let mut pairs: Vec<(i32, usize, usize)> = Vec::with_capacity(entries.len() * slots.len());
        for &entry in &entries {
            for (slot, colour) in slots.iter().enumerate().take(path.len()) {
                pairs.push((colour_distance(&planes.palettes[palette][entry], colour), entry, slot));
            }
        }
        pairs.sort_unstable();
        for (_, entry, slot) in pairs {
            if !taken[entry] && free[slot] {
                taken[entry] = true;
                free[slot] = false;
                new_of_old[entry] = (slot + reserved) as u8;
            }
        }
        let mut open = (0..slots.len()).filter(|&slot| free[slot]);
        for entry in reserved..PALETTE_COLOURS {
            if !taken[entry] {
                let slot = open.next().expect("as many slots as entries");
                new_of_old[entry] = (slot + reserved) as u8;
            }
        }
        for entry in 0..reserved {
            new_of_old[entry] = entry as u8;
        }
        let mut colours = planes.palettes[palette].clone();
        for (old, &new) in new_of_old.iter().enumerate() {
            colours[usize::from(new)] = planes.palettes[palette][old];
        }
        out.palettes[palette] = colours;
        for frame in planes.boxes.iter().filter(|f| f.palette == palette) {
            for i in frame.range() {
                if planes.is_opaque(i) {
                    out.index[i] = new_of_old[usize::from(planes.index[i])];
                }
            }
        }
    }
    assert_eq!(planes.colour_signature(), out.colour_signature(), "{}: the master palette changed a colour", planes.label);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// rANS with static tables

struct Model {
    alphabet: usize,
    /// Per context: `alphabet` frequencies summing to [`RANS_SCALE`] (all 0 for an unused context).
    freqs: Vec<u16>,
    /// Per context: `alphabet` cumulative starts.
    cums: Vec<u16>,
    /// Per context: symbol of each slot; empty for small alphabets, which search linearly.
    slots: Vec<u8>,
}

fn normalized(hist: &[u32]) -> Vec<u16> {
    let total: u64 = hist.iter().map(|&h| u64::from(h)).sum();
    let mut freqs = vec![0u16; hist.len()];
    if total == 0 {
        return freqs;
    }
    let mut sum = 0i64;
    for (freq, &h) in freqs.iter_mut().zip(hist) {
        if h > 0 {
            *freq = (u64::from(h) * u64::from(RANS_SCALE) / total).max(1) as u16;
            sum += i64::from(*freq);
        }
    }
    let mut excess = sum - i64::from(RANS_SCALE);
    if excess < 0 {
        let largest = (0..freqs.len()).max_by_key(|&s| hist[s]).unwrap();
        freqs[largest] += (-excess) as u16;
        excess = 0;
    }
    while excess > 0 {
        let largest = (0..freqs.len()).max_by_key(|&s| freqs[s]).unwrap();
        let taken = excess.min(i64::from(freqs[largest]) - 1).max(0).min(i64::from(freqs[largest]) / 2 + 1);
        freqs[largest] -= taken as u16;
        excess -= taken;
    }
    freqs
}

impl Model {
    fn from_freqs(alphabet: usize, freqs: Vec<u16>) -> Model {
        let contexts = freqs.len() / alphabet;
        let mut cums = vec![0u16; freqs.len()];
        let with_slots = alphabet > LINEAR_SEARCH_SYMBOLS;
        let mut slots = if with_slots { vec![0u8; contexts * RANS_SCALE as usize] } else { Vec::new() };
        for context in 0..contexts {
            let mut cum = 0u32;
            for symbol in 0..alphabet {
                let freq = u32::from(freqs[context * alphabet + symbol]);
                cums[context * alphabet + symbol] = cum as u16;
                if with_slots && freq > 0 {
                    slots[context * RANS_SCALE as usize + cum as usize..][..freq as usize].fill(symbol as u8);
                }
                cum += freq;
            }
        }
        Model { alphabet, freqs, cums, slots }
    }

    fn from_hist(alphabet: usize, hist: &[u32]) -> Model {
        let freqs: Vec<u16> = hist.chunks_exact(alphabet).flat_map(normalized).collect();
        Model::from_freqs(alphabet, freqs)
    }

    /// The tables, smaller of plain and Zstd: per context the count of used symbols, then gap and frequency pairs.
    fn serialized(&self) -> Vec<u8> {
        let mut raw = Vec::new();
        for context in self.freqs.chunks_exact(self.alphabet) {
            put_varint(&mut raw, context.iter().filter(|&&f| f > 0).count() as u32);
            let mut last = 0usize;
            for (symbol, &freq) in context.iter().enumerate() {
                if freq > 0 {
                    put_varint(&mut raw, (symbol - last) as u32);
                    put_varint(&mut raw, u32::from(freq));
                    last = symbol;
                }
            }
        }
        let packed = compress(&raw, BEST_ZSTD_LEVEL, false);
        let mut out = Vec::with_capacity(raw.len().min(packed.len()) + LENGTH_PREFIX_BYTES);
        out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        if packed.len() < raw.len() {
            out.extend_from_slice(&packed);
        } else {
            out.extend_from_slice(&raw);
        }
        out
    }

    fn parse(bytes: &[u8], alphabet: usize, contexts: usize) -> Model {
        let raw_len = u32::from_le_bytes(bytes[..LENGTH_PREFIX_BYTES].try_into().unwrap()) as usize;
        let body = &bytes[LENGTH_PREFIX_BYTES..];
        let unpacked;
        let raw: &[u8] = if body.len() == raw_len {
            body
        } else {
            unpacked = decompress(body, raw_len);
            &unpacked
        };
        let mut freqs = vec![0u16; contexts * alphabet];
        let mut at = 0usize;
        for context in 0..contexts {
            let used = get_varint(raw, &mut at);
            let mut symbol = 0usize;
            for _ in 0..used {
                symbol += get_varint(raw, &mut at) as usize;
                freqs[context * alphabet + symbol] = get_varint(raw, &mut at) as u16;
            }
        }
        Model::from_freqs(alphabet, freqs)
    }
}

const VARINT_PAYLOAD_BITS: u32 = 7;
const VARINT_MORE: u8 = 0x80;

fn put_varint(out: &mut Vec<u8>, mut value: u32) {
    while value >= u32::from(VARINT_MORE) {
        out.push(value as u8 | VARINT_MORE);
        value >>= VARINT_PAYLOAD_BITS;
    }
    out.push(value as u8);
}

fn get_varint(bytes: &[u8], at: &mut usize) -> u32 {
    let (mut value, mut shift) = (0u32, 0u32);
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u32::from(byte & !VARINT_MORE) << shift;
        if byte & VARINT_MORE == 0 {
            return value;
        }
        shift += VARINT_PAYLOAD_BITS;
    }
}

/// `(context, symbol)` in coding order to a stream decoded in that order.
fn rans_encode(symbols: &[(u16, u8)], model: &Model) -> Vec<u8> {
    let mut state = RANS_LOWER_BOUND;
    let mut reversed = Vec::with_capacity(symbols.len() / 2 + RANS_STATE_BYTES);
    for &(context, symbol) in symbols.iter().rev() {
        let at = usize::from(context) * model.alphabet + usize::from(symbol);
        let freq = u32::from(model.freqs[at]);
        debug_assert!(freq > 0);
        let limit = ((RANS_LOWER_BOUND >> RANS_SCALE_BITS) << RANS_BYTE_BITS) * freq;
        while state >= limit {
            reversed.push(state as u8);
            state >>= RANS_BYTE_BITS;
        }
        state = ((state / freq) << RANS_SCALE_BITS) + state % freq + u32::from(model.cums[at]);
    }
    let mut out = state.to_le_bytes().to_vec();
    out.extend(reversed.iter().rev());
    out
}

struct RansDecoder<'a> {
    state: u32,
    data: &'a [u8],
    at: usize,
}

impl<'a> RansDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        let state = u32::from_le_bytes(data[..RANS_STATE_BYTES].try_into().unwrap());
        RansDecoder { state, data, at: RANS_STATE_BYTES }
    }

    #[inline(always)]
    fn get(&mut self, model: &Model, context: usize) -> u8 {
        let slot = self.state & RANS_SLOT_MASK;
        let base = context * model.alphabet;
        let symbol = if model.slots.is_empty() {
            let mut symbol = model.alphabet - 1;
            for s in 1..model.alphabet {
                // A used symbol starts at or before the slot; the first start past the slot ends the search.
                if u32::from(model.cums[base + s]) > slot {
                    symbol = s - 1;
                    break;
                }
            }
            // Trailing unused symbols share the end of the range: step back to the one that covers the slot.
            while model.freqs[base + symbol] == 0 {
                symbol -= 1;
            }
            symbol
        } else {
            usize::from(model.slots[context * RANS_SCALE as usize + slot as usize])
        };
        let at = base + symbol;
        self.state = u32::from(model.freqs[at]) * (self.state >> RANS_SCALE_BITS) + slot - u32::from(model.cums[at]);
        while self.state < RANS_LOWER_BOUND {
            self.state = (self.state << RANS_BYTE_BITS) | u32::from(self.data[self.at]);
            self.at += 1;
        }
        symbol as u8
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Predictors and contexts

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Predictor {
    Left,
    Up,
    Med,
}

fn median_edge(a: u8, b: u8, c: u8) -> u8 {
    let (low, high) = (a.min(b), a.max(b));
    if c >= high {
        low
    } else if c <= low {
        high
    } else {
        (i32::from(a) + i32::from(b) - i32::from(c)).clamp(0, MAX_CHANNEL) as u8
    }
}

fn gradient_bucket(gradient: u16) -> usize {
    GRADIENT_BUCKET_TOPS.iter().position(|&top| gradient <= top).unwrap_or(GRADIENT_BUCKETS - 1)
}

/// The prediction of pixel `i` from opaque neighbours (a transparent one is not used), and its context.
#[inline(always)]
fn predict(pixels: &[u8], mask: &[u8], w: usize, i: usize, x: usize, has_up: bool, predictor: Predictor, last: u8) -> (u8, usize) {
    let left = x > 0 && mask[i - 1] != 0;
    let up = has_up && mask[i - w] != 0;
    match (left, up) {
        (true, true) => {
            let (a, b) = (pixels[i - 1], pixels[i - w]);
            if mask[i - w - 1] != 0 {
                let c = pixels[i - w - 1];
                let gradient = u16::from(a.abs_diff(c)) + u16::from(b.abs_diff(c));
                let prediction = match predictor {
                    Predictor::Left => a,
                    Predictor::Up => b,
                    Predictor::Med => median_edge(a, b, c),
                };
                (prediction, gradient_bucket(gradient))
            } else {
                let prediction = match predictor {
                    Predictor::Left => a,
                    Predictor::Up => b,
                    Predictor::Med => ((u16::from(a) + u16::from(b)) / 2) as u8,
                };
                (prediction, gradient_bucket(2 * u16::from(a.abs_diff(b))))
            }
        }
        (true, false) => (pixels[i - 1], CONTEXT_ONE_NEIGHBOUR),
        (false, true) => (pixels[i - w], CONTEXT_ONE_NEIGHBOUR),
        (false, false) => (last, CONTEXT_NO_NEIGHBOUR),
    }
}

/// The agreement of the previous frame's pixel at the same anchor-relative place with `prediction`.
#[inline(always)]
fn previous_bucket(previous: Option<u8>, prediction: u8) -> usize {
    match previous {
        None => PREVIOUS_NONE,
        Some(value) if value == prediction => PREVIOUS_EQUAL,
        Some(value) if value.abs_diff(prediction) <= PREVIOUS_NEAR_LIMIT => PREVIOUS_NEAR,
        Some(_) => PREVIOUS_FAR,
    }
}

/// The earlier frame of the same direction in a unit, as offsets into the unit's planes.
#[derive(Clone, Copy)]
struct PreviousFrame {
    start: usize,
    w: usize,
    h: usize,
    /// `previous.left - frame.left`, `previous.top - frame.top`.
    dx: i32,
    dy: i32,
}

impl PreviousFrame {
    #[inline(always)]
    fn pixel(&self, pixels: &[u8], mask: &[u8], x: usize, y: usize) -> Option<u8> {
        let px = x as i32 - self.dx;
        let py = y as i32 - self.dy;
        if px < 0 || py < 0 || px >= self.w as i32 || py >= self.h as i32 {
            return None;
        }
        let i = self.start + py as usize * self.w + px as usize;
        (mask[i] != 0).then(|| pixels[i])
    }
}

#[inline(always)]
fn mask_context(mask: &[u8], w: usize, i: usize, x: usize, y: usize) -> usize {
    let bit = |known: bool, offset: usize| usize::from(known && mask[i - offset] != 0);
    bit(x > 0, 1)
        | bit(y > 0, w) << 1
        | bit(x > 0 && y > 0, w + 1) << 2
        | bit(y > 0 && x + 1 < w, w.wrapping_sub(1)) << 3
        | bit(x > 1, 2) << 4
        | bit(y > 1, 2 * w) << 5
}

fn coverage_level_class(coverage: u8) -> usize {
    match coverage {
        0 => 0,
        c if c <= COVERAGE_LOW_TOP => 1,
        c if c <= COVERAGE_MID_TOP => 2,
        c if c <= COVERAGE_HIGH_TOP => 3,
        _ => 4,
    }
}

fn coverage_kind_class(coverage: u8) -> usize {
    match coverage {
        COVERAGE_TRANSPARENT => 0,
        COVERAGE_SOLID => 2,
        _ => 1,
    }
}

#[inline(always)]
fn coverage_context(coverage: &[u8], w: usize, i: usize, x: usize, y: usize) -> usize {
    let west = if x > 0 { coverage[i - 1] } else { 0 };
    let north = if y > 0 { coverage[i - w] } else { 0 };
    let north_east = if y > 0 && x + 1 < w { coverage[i + 1 - w] } else { 0 };
    (coverage_level_class(west) * COVERAGE_LEVEL_CLASSES + coverage_level_class(north)) * COVERAGE_KIND_CLASSES + coverage_kind_class(north_east)
}

// ---------------------------------------------------------------------------------------------------------------
// Candidates

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Unit {
    Body,
    Action,
    /// One stream per (action, direction).
    ActionDirection,
    /// Actions in order, merged until a stream holds at least this many raw bytes.
    Grouped(usize),
}

/// Where a rANS stream's tables live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    /// One set for the body, shared by its streams.
    Body,
    /// Each stream carries its own.
    Stream,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum IndexCoding {
    /// The whole index plane, transparent pixels included. The Classic plane carries the shape.
    Zstd(i32),
    /// Level 22 with a 128 MiB window.
    ZstdLarge,
    /// Prediction residuals of the opaque pixels, Zstd; Classic adds the shape, a bitmask.
    Residual(Predictor, i32),
    /// The same residuals, rANS with gradient contexts; Classic adds the shape, rANS with neighbour contexts.
    Rans(Predictor, Scope),
    /// As [`IndexCoding::Rans`], with the previous frame of the direction at the same anchor-relative place.
    RansPrevious(Predictor, Scope, PreviousRole),
    /// Each pixel's rank in the palette's entries by RGB distance from a reference, rANS; see `knowledge`.
    Rank(knowledge::RankMode, Scope),
    /// The whole index plane, LZ4 HC. The Classic plane carries the shape.
    Lz4Hc,
    /// Prediction residuals of the opaque pixels in context-split streams; Classic adds the shape, a bitmask. See `crowd`.
    Split(crowd::SplitSpec),
}

/// What the previous frame's pixel does in [`IndexCoding::RansPrevious`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviousRole {
    /// Its agreement with the spatial prediction joins the context.
    Context,
    /// It is the prediction when known; its agreement with the spatial one joins the context.
    Prediction,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum CoverageCoding {
    /// The plane.
    Zstd(i32),
    /// Shape bitmask, solid bitmask of the opaque pixels, partial values: three streams.
    Split(i32),
    /// rANS with neighbour contexts.
    Rans(Scope),
    /// The plane, LZ4 HC.
    Lz4Hc,
}

#[derive(Clone, Copy)]
struct Candidate {
    name: &'static str,
    order: Order,
    index: IndexCoding,
    coverage: CoverageCoding,
    unit: Unit,
}

const fn candidate(name: &'static str, order: Order, index: IndexCoding, coverage: CoverageCoding, unit: Unit) -> Candidate {
    Candidate { name, order, index, coverage, unit }
}

const COVERAGE_ZSTD: CoverageCoding = CoverageCoding::Zstd(LEVEL_19);
const COVERAGE_RANS: CoverageCoding = CoverageCoding::Rans(Scope::Body);

/// Every stream candidate of part `b`.
const CANDIDATES: [Candidate; 68] = {
    use IndexCoding::{Rans, RansPrevious, Residual, Zstd, ZstdLarge};
    use Order::{Cooccurrence, Luma, Original, Path, PathAll};
    use crowd::SplitCoder::{Tans, Zstd19};
    use crowd::split;
    use PreviousRole::{Context, Prediction};
    use Predictor::{Left, Med, Up};
    [
        // The baseline, and level 22 with a large window (A3).
        candidate("base: original order, zstd 19, body", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("base: original order, zstd 19, action", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Action),
        candidate("a3: zstd 22 window 27, body", Original, ZstdLarge, CoverageCoding::Zstd(LEVEL_22), Unit::Body),
        candidate("a3: zstd 22 window 27, action", Original, ZstdLarge, CoverageCoding::Zstd(LEVEL_22), Unit::Action),
        // Palette orders alone (B4).
        candidate("b4: luma, zstd 19, body", Luma, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b4: path, zstd 19, body", Path, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b4: cooccurrence, zstd 19, body", Cooccurrence, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        // Residuals + Zstd (B5).
        candidate("b5: original, MED residual, zstd 19", Original, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: luma, left residual, zstd 19", Luma, Residual(Left, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: luma, up residual, zstd 19", Luma, Residual(Up, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: luma, MED residual, zstd 19", Luma, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: path, left residual, zstd 19", Path, Residual(Left, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: path, up residual, zstd 19", Path, Residual(Up, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: path, MED residual, zstd 19", Path, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("b5: cooccurrence, MED residual, zstd 19", Cooccurrence, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        // Residuals + rANS with 2D contexts (B6).
        candidate("b6: original, MED, rANS body tables", Original, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: luma, MED, rANS body tables", Luma, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: path, MED, rANS body tables", Path, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: cooccurrence, MED, rANS body tables", Cooccurrence, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: path over all entries, MED, rANS body tables", PathAll, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: luma, left, rANS body tables", Luma, Rans(Left, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("b6: luma, MED, rANS per-action tables, action streams", Luma, Rans(Med, Scope::Stream), CoverageCoding::Rans(Scope::Stream), Unit::Action),
        candidate("b6: luma, MED, rANS body tables, action streams", Luma, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Action),
        candidate("b6: path, MED, rANS body tables, action streams", Path, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Action),
        candidate("b6: path over all entries, MED, rANS body tables, action streams", PathAll, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Action),
        // The previous frame as extra context (B7).
        candidate("b7: luma, MED + previous frame context, rANS body tables", Luma, RansPrevious(Med, Scope::Body, Context), COVERAGE_RANS, Unit::Body),
        candidate("b7: luma, previous frame predicts, rANS body tables", Luma, RansPrevious(Med, Scope::Body, Prediction), COVERAGE_RANS, Unit::Body),
        candidate("b7: luma, previous frame predicts, action streams", Luma, RansPrevious(Med, Scope::Body, Prediction), COVERAGE_RANS, Unit::Action),
        candidate("b7: path, MED + previous frame context, rANS body tables", Path, RansPrevious(Med, Scope::Body, Context), COVERAGE_RANS, Unit::Body),
        candidate("b7: path, MED + previous frame, action streams", Path, RansPrevious(Med, Scope::Body, Context), COVERAGE_RANS, Unit::Action),
        // The coverage plane (D10); the index coding is the baseline's.
        candidate("d10: coverage split, zstd 19", Original, Zstd(LEVEL_19), CoverageCoding::Split(LEVEL_19), Unit::Body),
        candidate("d10: coverage rANS body tables", Original, Zstd(LEVEL_19), COVERAGE_RANS, Unit::Body),
        candidate("d10: coverage rANS body tables, action streams", Original, Zstd(LEVEL_19), COVERAGE_RANS, Unit::Action),
        // Stream units (G15).
        candidate("g15: original, zstd 19, action+direction", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::ActionDirection),
        candidate("g15: original, zstd 19, group 32 KiB", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Grouped(GROUP_MINIMUMS[0])),
        candidate("g15: original, zstd 19, group 64 KiB", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Grouped(GROUP_MINIMUMS[1])),
        candidate("g15: original, zstd 19, group 256 KiB", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Grouped(GROUP_MINIMUMS[2])),
        candidate("g15: path, MED, rANS body tables, action+direction", Path, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::ActionDirection),
        candidate("g15: path, MED, rANS per-stream tables, action+direction", Path, Rans(Med, Scope::Stream), CoverageCoding::Rans(Scope::Stream), Unit::ActionDirection),
        candidate("g15: path, MED, rANS body tables, group 64 KiB", Path, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Grouped(GROUP_MINIMUMS[1])),
        candidate("g15: path, MED, rANS body tables, group 256 KiB", Path, Rans(Med, Scope::Body), COVERAGE_RANS, Unit::Grouped(GROUP_MINIMUMS[2])),
        // Slots following a body master palette (part k, item 3).
        candidate("k3: master palette, zstd 19, body", Order::Master, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        // Rank coding through the palette's colours (part k, item 2).
        candidate("k2: rank from MED colour, rANS body tables, action streams", Original, IndexCoding::Rank(knowledge::RankMode::Spatial, Scope::Body), COVERAGE_RANS, Unit::Action),
        candidate("k2: rank + previous frame context, rANS body tables, action streams", Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Action),
        candidate("k2: rank with a 16x16x16 colour lookup, rANS body tables, action streams", Original, IndexCoding::Rank(knowledge::RankMode::Coarse, Scope::Body), COVERAGE_RANS, Unit::Action),
        // The fast entropy path (part s2): no per-pixel entropy coder, no per-pixel palette work.
        candidate("s2: slot plane, LZ4 HC, body", Original, IndexCoding::Lz4Hc, CoverageCoding::Lz4Hc, Unit::Body),
        candidate("s2: slot plane, LZ4 HC, action", Original, IndexCoding::Lz4Hc, CoverageCoding::Lz4Hc, Unit::Action),
        candidate("s2: luma, MED residual, zstd 19, action", Luma, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, left residual, zstd 19, action", Luma, Residual(Left, LEVEL_19), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 4 streams, zstd 19, action", Luma, IndexCoding::Split(split(Med, false, Zstd19, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 10 streams, zstd 19, action", Luma, IndexCoding::Split(split(Med, true, Zstd19, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 10 streams, zstd 19, body", Luma, IndexCoding::Split(split(Med, true, Zstd19, false)), COVERAGE_ZSTD, Unit::Body),
        candidate("s2: luma, left, 10 streams, zstd 19, action", Luma, IndexCoding::Split(split(Left, true, Zstd19, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: slot, MED, 10 streams, zstd 19, action", Original, IndexCoding::Split(split(Med, true, Zstd19, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 4 streams, tANS, action", Luma, IndexCoding::Split(split(Med, false, Tans, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 10 streams, tANS, action", Luma, IndexCoding::Split(split(Med, true, Tans, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: luma, MED, 10 streams, tANS, body", Luma, IndexCoding::Split(split(Med, true, Tans, false)), COVERAGE_ZSTD, Unit::Body),
        candidate("s2: luma, MED, 10 streams, tANS, action+direction", Luma, IndexCoding::Split(split(Med, true, Tans, false)), COVERAGE_ZSTD, Unit::ActionDirection),
        candidate("s2: luma, left, 10 streams, tANS, action", Luma, IndexCoding::Split(split(Left, true, Tans, false)), COVERAGE_ZSTD, Unit::Action),
        candidate("s2: slot, MED, 10 streams, tANS, action", Original, IndexCoding::Split(split(Med, true, Tans, false)), COVERAGE_ZSTD, Unit::Action),
        // Motion-compensated prediction (part s3), through the same streams.
        candidate("s3: luma, MED + block motion, zstd 19, action", Luma, IndexCoding::Split(split(Med, true, Zstd19, true)), COVERAGE_ZSTD, Unit::Action),
        candidate("s3: luma, MED + block motion, tANS, action", Luma, IndexCoding::Split(split(Med, true, Tans, true)), COVERAGE_ZSTD, Unit::Action),
        // The other load units of part s1's candidates, for their disk cost.
        candidate("s1: slot plane, LZ4 HC, action+direction", Original, IndexCoding::Lz4Hc, CoverageCoding::Lz4Hc, Unit::ActionDirection),
        candidate("s1: luma, MED residual, zstd 19, action+direction", Luma, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::ActionDirection),
        candidate("s1: luma, MED, 10 streams, zstd 19, action+direction", Luma, IndexCoding::Split(split(Med, true, Zstd19, false)), COVERAGE_ZSTD, Unit::ActionDirection),
        candidate("s1: luma, MED + block motion, tANS, body", Luma, IndexCoding::Split(split(Med, true, Tans, true)), COVERAGE_ZSTD, Unit::Body),
        candidate("s1: luma, MED + block motion, tANS, action+direction", Luma, IndexCoding::Split(split(Med, true, Tans, true)), COVERAGE_ZSTD, Unit::ActionDirection),
        candidate("s1: rank + previous, action+direction", Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::ActionDirection),
    ]
};

/// The frames of each stream, in order.
fn units(boxes: &[FrameBox], unit: Unit, pixel_bytes: usize) -> Vec<Vec<usize>> {
    let by_key = |key: &dyn Fn(&FrameBox) -> u64| -> Vec<Vec<usize>> {
        let mut groups = BTreeMap::<u64, Vec<usize>>::new();
        for (i, frame) in boxes.iter().enumerate() {
            groups.entry(key(frame)).or_default().push(i);
        }
        groups.into_values().collect()
    };
    match unit {
        Unit::Body => vec![(0..boxes.len()).collect()],
        Unit::Action => by_key(&|f| u64::from(f.action)),
        Unit::ActionDirection => by_key(&|f| (u64::from(f.action) << u32::BITS) | u64::from(f.direction)),
        Unit::Grouped(minimum) => {
            let mut groups: Vec<Vec<usize>> = Vec::new();
            let mut open = 0usize;
            for action in by_key(&|f| u64::from(f.action)) {
                let raw: usize = action.iter().map(|&i| boxes[i].pixels() * pixel_bytes).sum();
                if groups.is_empty() || open >= minimum {
                    groups.push(action);
                    open = raw;
                } else {
                    groups.last_mut().unwrap().extend(action);
                    open += raw;
                }
            }
            groups
        }
    }
}

/// The index of the frame before each frame of a stream in the same direction, if it is in the stream.
fn previous_frames(boxes: &[FrameBox], frames: &[usize]) -> Vec<Option<PreviousFrame>> {
    let mut offset = 0usize;
    let mut starts = Vec::with_capacity(frames.len());
    for &f in frames {
        starts.push(offset);
        offset += boxes[f].pixels();
    }
    (0..frames.len())
        .map(|n| {
            if n == 0 || boxes[frames[n - 1]].direction != boxes[frames[n]].direction {
                return None;
            }
            let (before, frame) = (&boxes[frames[n - 1]], &boxes[frames[n]]);
            (before.palette == frame.palette).then_some(PreviousFrame {
                start: starts[n - 1],
                w: before.w,
                h: before.h,
                dx: before.left - frame.left,
                dy: before.top - frame.top,
            })
        })
        .collect()
}

#[derive(Default)]
struct EncodedUnit {
    frames: Vec<usize>,
    index: Vec<u8>,
    coverage: Vec<u8>,
    /// Stream-scope tables.
    index_tables: Vec<u8>,
    coverage_tables: Vec<u8>,
}

#[derive(Default)]
struct Encoded {
    index_tables: Vec<u8>,
    coverage_tables: Vec<u8>,
    units: Vec<EncodedUnit>,
}

impl Encoded {
    fn index_bytes(&self) -> u64 {
        self.index_tables.len() as u64 + self.units.iter().map(|u| (u.index.len() + u.index_tables.len()) as u64).sum::<u64>()
    }

    fn coverage_bytes(&self) -> u64 {
        self.coverage_tables.len() as u64 + self.units.iter().map(|u| (u.coverage.len() + u.coverage_tables.len()) as u64).sum::<u64>()
    }

    fn total(&self) -> u64 {
        self.index_bytes() + self.coverage_bytes()
    }
}

fn compress_level(data: &[u8], level: i32, large_window: bool) -> Vec<u8> {
    let mut compressor = zstd::bulk::Compressor::new(level).expect("zstd compressor");
    if large_window {
        compressor.set_parameter(CParameter::WindowLog(LARGE_WINDOW_LOG)).expect("zstd window");
    }
    compressor.compress(data).expect("zstd compress")
}

fn packed_bits(bits: impl Iterator<Item = bool>) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, bit) in bits.enumerate() {
        if n % u8::BITS as usize == 0 {
            out.push(0u8);
        }
        if bit {
            *out.last_mut().unwrap() |= 1 << (n % u8::BITS as usize);
        }
    }
    out
}

fn packed_bytes(count: usize) -> usize {
    count.div_ceil(u8::BITS as usize)
}

fn put_streams(streams: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for stream in streams {
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        out.extend_from_slice(stream);
    }
    out
}

fn get_stream<'a>(bytes: &'a [u8], at: &mut usize) -> &'a [u8] {
    let len = u32::from_le_bytes(bytes[*at..*at + LENGTH_PREFIX_BYTES].try_into().unwrap()) as usize;
    *at += LENGTH_PREFIX_BYTES;
    let stream = &bytes[*at..*at + len];
    *at += len;
    stream
}

/// Symbols of each stream to bytes, with the tables at `scope`: (shared tables, per stream (tables, bytes)).
fn rans_streams(streams: &[Vec<(u16, u8)>], alphabet: usize, contexts: usize, scope: Scope) -> (Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>) {
    let tally = |symbols: &[(u16, u8)], hist: &mut [u32]| {
        for &(context, symbol) in symbols {
            hist[usize::from(context) * alphabet + usize::from(symbol)] += 1;
        }
    };
    match scope {
        Scope::Body => {
            let mut hist = vec![0u32; contexts * alphabet];
            streams.iter().for_each(|symbols| tally(symbols, &mut hist));
            let model = Model::from_hist(alphabet, &hist);
            (model.serialized(), streams.iter().map(|symbols| (Vec::new(), rans_encode(symbols, &model))).collect())
        }
        Scope::Stream => (
            Vec::new(),
            streams
                .iter()
                .map(|symbols| {
                    let mut hist = vec![0u32; contexts * alphabet];
                    tally(symbols, &mut hist);
                    let model = Model::from_hist(alphabet, &hist);
                    (model.serialized(), rans_encode(symbols, &model))
                })
                .collect(),
        ),
    }
}

fn previous_role(coding: IndexCoding) -> Option<PreviousRole> {
    match coding {
        IndexCoding::RansPrevious(_, _, role) => Some(role),
        _ => None,
    }
}

fn index_contexts(coding: IndexCoding) -> usize {
    match coding {
        IndexCoding::RansPrevious(..) => INDEX_CONTEXTS * PREVIOUS_BUCKETS,
        IndexCoding::Rank(mode, _) => knowledge::rank_contexts(mode),
        _ => INDEX_CONTEXTS,
    }
}

/// The unit's index plane and mask (coverage for the Enhanced; 0/1 for Classic), frames back to back.
fn unit_planes(planes: &Planes, frames: &[usize]) -> (Vec<u8>, Vec<u8>) {
    let mut index = Vec::new();
    let mut mask = Vec::new();
    for &f in frames {
        let range = planes.boxes[f].range();
        index.extend_from_slice(&planes.index[range.clone()]);
        match planes.client {
            Client::Cc => mask.extend(planes.index[range].iter().map(|&i| u8::from(i != 0))),
            Client::Ec => mask.extend_from_slice(&planes.coverage[range]),
        }
    }
    (index, mask)
}

/// Prediction residuals of the unit's opaque pixels, with contexts when `previous` asks for them.
fn index_symbols(planes: &Planes, frames: &[usize], index: &[u8], mask: &[u8], predictor: Predictor, role: Option<PreviousRole>) -> Vec<(u16, u8)> {
    let mut out = Vec::new();
    let previous = previous_frames(&planes.boxes, frames);
    let mut start = 0usize;
    for (n, &f) in frames.iter().enumerate() {
        let frame = &planes.boxes[f];
        let mut last = FIRST_PREDICTION;
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (mut prediction, mut context) = predict(index, mask, frame.w, i, x, y > 0, predictor, last);
                if let Some(role) = role {
                    let before = previous[n].and_then(|p| p.pixel(index, mask, x, y));
                    context = context * PREVIOUS_BUCKETS + previous_bucket(before, prediction);
                    if role == PreviousRole::Prediction {
                        prediction = before.unwrap_or(prediction);
                    }
                }
                out.push((context as u16, index[i].wrapping_sub(prediction)));
                last = index[i];
            }
        }
        start += frame.pixels();
    }
    out
}

fn mask_symbols(boxes: &[FrameBox], frames: &[usize], mask: &[u8]) -> Vec<(u16, u8)> {
    let mut out = Vec::with_capacity(mask.len());
    let mut start = 0usize;
    for &f in frames {
        let frame = &boxes[f];
        let local = &mask[start..start + frame.pixels()];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                out.push((mask_context(local, frame.w, i, x, y) as u16, u8::from(local[i] != 0)));
            }
        }
        start += frame.pixels();
    }
    out
}

fn coverage_symbols(boxes: &[FrameBox], frames: &[usize], coverage: &[u8]) -> Vec<(u16, u8)> {
    let mut out = Vec::with_capacity(coverage.len());
    let mut start = 0usize;
    for &f in frames {
        let frame = &boxes[f];
        let local = &coverage[start..start + frame.pixels()];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                out.push((coverage_context(local, frame.w, i, x, y) as u16, COVERAGE_SYMBOL_OF[usize::from(local[i])]));
            }
        }
        start += frame.pixels();
    }
    out
}

fn encode_candidate(planes: &Planes, candidate: &Candidate) -> Encoded {
    let groups = units(&planes.boxes, candidate.unit, planes.client.pixel_bytes());
    let parts: Vec<(Vec<u8>, Vec<u8>)> = groups.iter().map(|frames| unit_planes(planes, frames)).collect();
    let mut encoded = Encoded {
        units: groups.iter().map(|frames| EncodedUnit { frames: frames.clone(), ..EncodedUnit::default() }).collect(),
        ..Encoded::default()
    };

    // Index streams.
    match candidate.index {
        IndexCoding::Zstd(level) => {
            for (unit, (index, _)) in encoded.units.iter_mut().zip(&parts) {
                unit.index = compress_level(index, level, false);
            }
        }
        IndexCoding::ZstdLarge => {
            for (unit, (index, _)) in encoded.units.iter_mut().zip(&parts) {
                unit.index = compress_level(index, LEVEL_22, true);
            }
        }
        IndexCoding::Residual(predictor, level) => {
            for (n, (unit, (index, mask))) in encoded.units.iter_mut().zip(&parts).enumerate() {
                let symbols = index_symbols(planes, &groups[n], index, mask, predictor, None);
                let residuals: Vec<u8> = symbols.iter().map(|s| s.1).collect();
                unit.index = compress_level(&residuals, level, false);
            }
        }
        IndexCoding::Rans(predictor, scope) | IndexCoding::RansPrevious(predictor, scope, _) => {
            let streams: Vec<Vec<(u16, u8)>> = parts
                .iter()
                .enumerate()
                .map(|(n, (index, mask))| index_symbols(planes, &groups[n], index, mask, predictor, previous_role(candidate.index)))
                .collect();
            let (shared, out) = rans_streams(&streams, BYTE_SYMBOLS, index_contexts(candidate.index), scope);
            encoded.index_tables = shared;
            for (unit, (tables, bytes)) in encoded.units.iter_mut().zip(out) {
                unit.index_tables = tables;
                unit.index = bytes;
            }
        }
        IndexCoding::Rank(mode, scope) => {
            let streams: Vec<Vec<(u16, u8)>> =
                parts.iter().enumerate().map(|(n, (index, mask))| knowledge::rank_symbols(planes, &groups[n], index, mask, mode)).collect();
            let (shared, out) = rans_streams(&streams, BYTE_SYMBOLS, index_contexts(candidate.index), scope);
            encoded.index_tables = shared;
            for (unit, (tables, bytes)) in encoded.units.iter_mut().zip(out) {
                unit.index_tables = tables;
                unit.index = bytes;
            }
        }
        IndexCoding::Lz4Hc => {
            for (unit, (index, _)) in encoded.units.iter_mut().zip(&parts) {
                unit.index = crowd::lz4_compress(index);
            }
        }
        IndexCoding::Split(spec) => {
            encoded.index_tables = crowd::encode_split(planes, &groups, &parts, spec, &mut encoded.units);
        }
    }

    // Shape and coverage streams.
    match planes.client {
        Client::Cc => match candidate.index {
            IndexCoding::Zstd(_) | IndexCoding::ZstdLarge | IndexCoding::Lz4Hc => {}
            IndexCoding::Residual(_, level) => {
                for (unit, (_, mask)) in encoded.units.iter_mut().zip(&parts) {
                    unit.coverage = compress_level(&packed_bits(mask.iter().map(|&m| m != 0)), level, false);
                }
            }
            IndexCoding::Split(_) => {
                for (unit, (_, mask)) in encoded.units.iter_mut().zip(&parts) {
                    unit.coverage = compress_level(&packed_bits(mask.iter().map(|&m| m != 0)), LEVEL_19, false);
                }
            }
            IndexCoding::Rans(_, scope) | IndexCoding::RansPrevious(_, scope, _) | IndexCoding::Rank(_, scope) => {
                let streams: Vec<Vec<(u16, u8)>> =
                    groups.iter().zip(&parts).map(|(frames, (_, mask))| mask_symbols(&planes.boxes, frames, mask)).collect();
                let (shared, out) = rans_streams(&streams, BINARY_SYMBOLS, MASK_CONTEXTS, scope);
                encoded.coverage_tables = shared;
                for (unit, (tables, bytes)) in encoded.units.iter_mut().zip(out) {
                    unit.coverage_tables = tables;
                    unit.coverage = bytes;
                }
            }
        },
        Client::Ec => match candidate.coverage {
            CoverageCoding::Zstd(level) => {
                for (unit, (_, coverage)) in encoded.units.iter_mut().zip(&parts) {
                    unit.coverage = compress_level(coverage, level, level == LEVEL_22);
                }
            }
            CoverageCoding::Split(level) => {
                for (unit, (_, coverage)) in encoded.units.iter_mut().zip(&parts) {
                    let shape = packed_bits(coverage.iter().map(|&c| c != COVERAGE_TRANSPARENT));
                    let solid = packed_bits(coverage.iter().filter(|&&c| c != COVERAGE_TRANSPARENT).map(|&c| c == COVERAGE_SOLID));
                    let partial: Vec<u8> = coverage.iter().copied().filter(|&c| c != COVERAGE_TRANSPARENT && c != COVERAGE_SOLID).collect();
                    unit.coverage = put_streams(&[
                        &compress_level(&shape, level, false),
                        &compress_level(&solid, level, false),
                        &compress_level(&partial, level, false),
                    ]);
                }
            }
            CoverageCoding::Lz4Hc => {
                for (unit, (_, coverage)) in encoded.units.iter_mut().zip(&parts) {
                    unit.coverage = crowd::lz4_compress(coverage);
                }
            }
            CoverageCoding::Rans(scope) => {
                let streams: Vec<Vec<(u16, u8)>> =
                    groups.iter().zip(&parts).map(|(frames, (_, coverage))| coverage_symbols(&planes.boxes, frames, coverage)).collect();
                let (shared, out) = rans_streams(&streams, COVERAGE_SYMBOLS, COVERAGE_CONTEXTS, scope);
                encoded.coverage_tables = shared;
                for (unit, (tables, bytes)) in encoded.units.iter_mut().zip(out) {
                    unit.coverage_tables = tables;
                    unit.coverage = bytes;
                }
            }
        },
    }
    encoded
}

/// The models shared by a body's streams, parsed once per body.
#[derive(Default)]
struct SharedModels {
    index: Option<Model>,
    coverage: Option<Model>,
    split: Vec<crowd::Tans>,
}

fn shared_models(encoded: &Encoded, candidate: &Candidate, client: Client) -> SharedModels {
    let mut models = SharedModels::default();
    if let IndexCoding::Split(spec) = candidate.index {
        models.split = crowd::parse_tables(&encoded.index_tables, spec);
    } else if !encoded.index_tables.is_empty() {
        models.index = Some(Model::parse(&encoded.index_tables, BYTE_SYMBOLS, index_contexts(candidate.index)));
    }
    if !encoded.coverage_tables.is_empty() {
        models.coverage = Some(match client {
            Client::Cc => Model::parse(&encoded.coverage_tables, BINARY_SYMBOLS, MASK_CONTEXTS),
            Client::Ec => Model::parse(&encoded.coverage_tables, COVERAGE_SYMBOLS, COVERAGE_CONTEXTS),
        });
    }
    models
}

/// Rebuilds a stream's index plane and its coverage plane (empty for Classic).
fn decode_unit(unit: &EncodedUnit, boxes: &[FrameBox], palettes: &[Vec<[u8; EC_PALETTE_ENTRY_BYTES]>], candidate: &Candidate, client: Client, shared: &SharedModels) -> (Vec<u8>, Vec<u8>) {
    let total: usize = unit.frames.iter().map(|&f| boxes[f].pixels()).sum();
    // Classic: the 0/1 shape, or nothing when the index plane carries it. Enhanced: the coverage.
    let mut mask = Vec::new();
    match client {
        Client::Cc => match candidate.index {
            IndexCoding::Zstd(_) | IndexCoding::ZstdLarge | IndexCoding::Lz4Hc => {}
            IndexCoding::Residual(..) | IndexCoding::Split(_) => {
                let bits = decompress(&unit.coverage, packed_bytes(total));
                mask = (0..total).map(|n| (bits[n / u8::BITS as usize] >> (n % u8::BITS as usize)) & 1).collect();
            }
            IndexCoding::Rans(..) | IndexCoding::RansPrevious(..) | IndexCoding::Rank(..) => {
                let own;
                let model = if unit.coverage_tables.is_empty() {
                    shared.coverage.as_ref().unwrap()
                } else {
                    own = Model::parse(&unit.coverage_tables, BINARY_SYMBOLS, MASK_CONTEXTS);
                    &own
                };
                mask = vec![0u8; total];
                let mut decoder = RansDecoder::new(&unit.coverage);
                let mut start = 0usize;
                for &f in &unit.frames {
                    let frame = &boxes[f];
                    let local = &mut mask[start..start + frame.pixels()];
                    for y in 0..frame.h {
                        for x in 0..frame.w {
                            let i = y * frame.w + x;
                            local[i] = decoder.get(model, mask_context(local, frame.w, i, x, y));
                        }
                    }
                    start += frame.pixels();
                }
            }
        },
        Client::Ec => match candidate.coverage {
            CoverageCoding::Zstd(_) => mask = decompress(&unit.coverage, total),
            CoverageCoding::Lz4Hc => mask = crowd::lz4_decompress(&unit.coverage, total),
            CoverageCoding::Split(_) => {
                let mut at = 0usize;
                let shape = decompress(get_stream(&unit.coverage, &mut at), packed_bytes(total));
                let solid = decompress(get_stream(&unit.coverage, &mut at), packed_bytes(total));
                let partial = decompress(get_stream(&unit.coverage, &mut at), total);
                mask = vec![0u8; total];
                let (mut opaque, mut next) = (0usize, 0usize);
                for (n, pixel) in mask.iter_mut().enumerate() {
                    if (shape[n / u8::BITS as usize] >> (n % u8::BITS as usize)) & 1 == 0 {
                        continue;
                    }
                    if (solid[opaque / u8::BITS as usize] >> (opaque % u8::BITS as usize)) & 1 != 0 {
                        *pixel = COVERAGE_SOLID;
                    } else {
                        *pixel = partial[next];
                        next += 1;
                    }
                    opaque += 1;
                }
            }
            CoverageCoding::Rans(_) => {
                let own;
                let model = if unit.coverage_tables.is_empty() {
                    shared.coverage.as_ref().unwrap()
                } else {
                    own = Model::parse(&unit.coverage_tables, COVERAGE_SYMBOLS, COVERAGE_CONTEXTS);
                    &own
                };
                mask = vec![0u8; total];
                let mut decoder = RansDecoder::new(&unit.coverage);
                let mut start = 0usize;
                for &f in &unit.frames {
                    let frame = &boxes[f];
                    let local = &mut mask[start..start + frame.pixels()];
                    for y in 0..frame.h {
                        for x in 0..frame.w {
                            let i = y * frame.w + x;
                            let context = coverage_context(local, frame.w, i, x, y);
                            local[i] = COVERAGE_OF_SYMBOL[usize::from(decoder.get(model, context))];
                        }
                    }
                    start += frame.pixels();
                }
            }
        },
    }

    let index = match candidate.index {
        IndexCoding::Zstd(_) | IndexCoding::ZstdLarge => decompress(&unit.index, total),
        IndexCoding::Lz4Hc => crowd::lz4_decompress(&unit.index, total),
        IndexCoding::Split(spec) => crowd::decode_split(unit, boxes, &mask, spec, &shared.split, total),
        IndexCoding::Residual(predictor, _) => {
            let opaque = mask.iter().filter(|&&m| m != 0).count();
            let residuals = decompress(&unit.index, opaque);
            let mut index = vec![0u8; total];
            let (mut start, mut next) = (0usize, 0usize);
            for &f in &unit.frames {
                let frame = &boxes[f];
                let mut last = FIRST_PREDICTION;
                for y in 0..frame.h {
                    for x in 0..frame.w {
                        let i = start + y * frame.w + x;
                        if mask[i] == 0 {
                            continue;
                        }
                        let (prediction, _) = predict(&index, &mask, frame.w, i, x, y > 0, predictor, last);
                        index[i] = prediction.wrapping_add(residuals[next]);
                        next += 1;
                        last = index[i];
                    }
                }
                start += frame.pixels();
            }
            index
        }
        IndexCoding::Rans(predictor, _) | IndexCoding::RansPrevious(predictor, _, _) => {
            let role = previous_role(candidate.index);
            let own;
            let model = if unit.index_tables.is_empty() {
                shared.index.as_ref().unwrap()
            } else {
                own = Model::parse(&unit.index_tables, BYTE_SYMBOLS, index_contexts(candidate.index));
                &own
            };
            let previous = if role.is_some() { previous_frames(boxes, &unit.frames) } else { Vec::new() };
            let mut index = vec![0u8; total];
            let mut decoder = RansDecoder::new(&unit.index);
            let mut start = 0usize;
            for (n, &f) in unit.frames.iter().enumerate() {
                let frame = &boxes[f];
                let mut last = FIRST_PREDICTION;
                for y in 0..frame.h {
                    for x in 0..frame.w {
                        let i = start + y * frame.w + x;
                        if mask[i] == 0 {
                            continue;
                        }
                        let (mut prediction, mut context) = predict(&index, &mask, frame.w, i, x, y > 0, predictor, last);
                        if let Some(role) = role {
                            let before = previous[n].and_then(|p| p.pixel(&index, &mask, x, y));
                            context = context * PREVIOUS_BUCKETS + previous_bucket(before, prediction);
                            if role == PreviousRole::Prediction {
                                prediction = before.unwrap_or(prediction);
                            }
                        }
                        index[i] = prediction.wrapping_add(decoder.get(model, context));
                        last = index[i];
                    }
                }
                start += frame.pixels();
            }
            index
        }
        IndexCoding::Rank(mode, _) => {
            let own;
            let model = if unit.index_tables.is_empty() {
                shared.index.as_ref().unwrap()
            } else {
                own = Model::parse(&unit.index_tables, BYTE_SYMBOLS, index_contexts(candidate.index));
                &own
            };
            knowledge::decode_rank(&unit.index, model, boxes, palettes, client, &unit.frames, &mask, mode)
        }
    };
    let coverage = if client == Client::Ec { mask } else { Vec::new() };
    (index, coverage)
}

/// Decodes every stream and compares it with the source, panicking on any difference.
fn verify_round_trip(planes: &Planes, encoded: &Encoded, candidate: &Candidate) {
    let shared = shared_models(encoded, candidate, planes.client);
    for unit in &encoded.units {
        let (index, coverage) = decode_unit(unit, &planes.boxes, &planes.palettes, candidate, planes.client, &shared);
        let (want_index, want_mask) = unit_planes(planes, &unit.frames);
        assert!(index == want_index, "{} / {}: the index plane does not round-trip", planes.label, candidate.name);
        if planes.client == Client::Ec {
            assert!(coverage == want_mask, "{} / {}: the coverage plane does not round-trip", planes.label, candidate.name);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part b: sizes and timings of the candidates

#[derive(Default, Clone)]
struct DecodeTiming {
    /// Whole-body decode per body.
    bodies: Vec<f64>,
    /// Tables plus one stream, for every stream: the time from asking for an action to its first drawable frame.
    first: Vec<f64>,
    /// With [`WORKER_THREADS`] threads, whole body.
    workers: Vec<f64>,
    output_bytes: u64,
    total_seconds: f64,
}

/// One candidate over a sample: sizes per client, and its decode on one thread.
struct CandidateResult {
    name: &'static str,
    index: BTreeMap<Client, u64>,
    coverage: BTreeMap<Client, u64>,
    timing: BTreeMap<Client, DecodeTiming>,
}

fn time_candidate(sample: &[(&Planes, Encoded)], candidate: &Candidate, single: &rayon::ThreadPool, workers: &rayon::ThreadPool) -> BTreeMap<Client, DecodeTiming> {
    let mut timings = BTreeMap::<Client, DecodeTiming>::new();
    for (planes, encoded) in sample {
        let timing = timings.entry(planes.client).or_default();
        let (client, boxes) = (planes.client, &planes.boxes);
        let whole = single.install(|| {
            fastest(|| {
                let shared = shared_models(encoded, candidate, client);
                encoded.units.iter().map(|unit| decode_unit(unit, boxes, &planes.palettes, candidate, client, &shared).0.len()).sum::<usize>()
            })
        });
        let tables = single.install(|| fastest(|| shared_models(encoded, candidate, client).index.is_some()));
        let shared = shared_models(encoded, candidate, client);
        for unit in &encoded.units {
            let stream = single.install(|| fastest(|| decode_unit(unit, boxes, &planes.palettes, candidate, client, &shared).0.len()));
            timing.first.push((tables + stream).as_secs_f64());
        }
        let parallel = workers.install(|| {
            fastest(|| {
                let shared = shared_models(encoded, candidate, client);
                encoded.units.par_iter().map(|unit| decode_unit(unit, boxes, &planes.palettes, candidate, client, &shared).0.len()).sum::<usize>()
            })
        });
        timing.bodies.push(whole.as_secs_f64());
        timing.workers.push(parallel.as_secs_f64());
        timing.total_seconds += whole.as_secs_f64();
        timing.output_bytes += planes.resident_bytes();
    }
    timings
}

fn sample_of(present: &[(Client, u64, usize)], client: Client, count: usize) -> Vec<usize> {
    let of_client: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client).collect();
    spread(&of_client, count, |s| s.1).into_iter().map(|s| s.2).collect()
}

fn ms(seconds: f64) -> f64 {
    seconds * BYTES_PER_KB
}

fn part_b(all: &[Planes], only: &[String], time: bool) {
    let chosen: Vec<&Candidate> =
        CANDIDATES.iter().filter(|c| only.is_empty() || only.iter().any(|filter| c.name.contains(filter.as_str()))).collect();
    let orders: Vec<Order> = {
        let mut orders = Vec::new();
        for c in &chosen {
            if !orders.contains(&c.order) {
                orders.push(c.order);
            }
        }
        orders
    };
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let workers = rayon::ThreadPoolBuilder::new().num_threads(WORKER_THREADS).build().expect("worker pool");
    let state = cpu_state();
    println!("\nCPU {state} at the start of part b; load average {}", load_average());
    // One palette order in memory at a time, and each candidate's streams only while it is measured.
    let mut results = Vec::new();
    for order in orders {
        let reorder_started = Instant::now();
        let planes: Vec<Planes> = all.par_iter().map(|planes| reordered(planes, order)).collect();
        println!("palette order {order:?} built and checked in {:.0} s", reorder_started.elapsed().as_secs_f64());
        for (position, candidate) in chosen.iter().enumerate().filter(|(_, c)| c.order == order) {
            let started = Instant::now();
            let encoded: Vec<Encoded> = planes
                .par_iter()
                .map(|planes| {
                    let encoded = encode_candidate(planes, candidate);
                    verify_round_trip(planes, &encoded, candidate);
                    encoded
                })
                .collect();
            let mut result = CandidateResult { name: candidate.name, index: BTreeMap::new(), coverage: BTreeMap::new(), timing: BTreeMap::new() };
            for (planes, encoded) in planes.iter().zip(&encoded) {
                *result.index.entry(planes.client).or_default() += encoded.index_bytes();
                *result.coverage.entry(planes.client).or_default() += encoded.coverage_bytes();
            }
            let encode_seconds = started.elapsed().as_secs_f64();
            if time {
                let load_before = load_average();
                let sample: Vec<(&Planes, Encoded)> = planes.iter().zip(encoded).collect();
                result.timing = time_candidate(&sample, candidate, &single, &workers);
                println!("{}: encoded and verified in {encode_seconds:.0} s, timed in {:.0} s (load {load_before})", candidate.name, started.elapsed().as_secs_f64() - encode_seconds);
            } else {
                println!("{}: encoded and verified in {encode_seconds:.0} s", candidate.name);
            }
            results.push((position, result));
        }
    }
    results.sort_by_key(|r| r.0);
    let mut results: Vec<CandidateResult> = results.into_iter().map(|r| r.1).collect();
    let state_end = cpu_state();
    println!("Load average at the end of part b {}", load_average());
    if state_end != state {
        println!("\nThe CPU state changed during part b ({state} at the start, {state_end} at the end): timings discarded");
        results.iter_mut().for_each(|r| r.timing.clear());
    }
    print_candidates(all, &results, &state);
}

fn print_candidates(all: &[Planes], results: &[CandidateResult], state: &str) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = all.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let bc7: u64 = bodies.iter().map(|p| p.bc7_zstd).sum();
        let opaque: u64 = bodies.iter().map(|p| p.opaque_pixels()).sum();
        let pixels: u64 = bodies.iter().map(|p| p.index.len() as u64).sum();
        println!(
            "\n### {} candidates: {} bodies, {:.1} Mpx ({:.1} Mpx opaque); today's BC7 + zstd 7 on this sample {:.1} MB; CPU {state}\n",
            client.label(),
            bodies.len(),
            pixels as f64 / PIXELS_PER_MEGAPIXEL,
            opaque as f64 / PIXELS_PER_MEGAPIXEL,
            mb(bc7)
        );
        println!("| candidate | index MB | bits per opaque px | shape/coverage MB | total MB | x BC7 | decode MB/s | body median / p95 / max ms | one stream with its tables, median / p95 / max ms | 4 threads median / p95 / max ms |");
        println!("|---|---:|---:|---:|---:|---:|---:|---|---|---|");
        for result in results {
            let index = result.index.get(&client).copied().unwrap_or(0);
            let coverage = result.coverage.get(&client).copied().unwrap_or(0);
            let timing = result.timing.get(&client);
            let triple = |values: &[f64]| {
                if values.is_empty() {
                    return "-".to_string();
                }
                let sorted = sorted(values.to_vec());
                format!("{:.2} / {:.2} / {:.2}", ms(percentile(&sorted, MEDIAN)), ms(percentile(&sorted, P95)), ms(*sorted.last().unwrap()))
            };
            println!(
                "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} | {} | {} | {} | {} |",
                result.name,
                mb(index),
                index as f64 * f64::from(u8::BITS) / opaque.max(1) as f64,
                mb(coverage),
                mb(index + coverage),
                (index + coverage) as f64 / bc7.max(1) as f64,
                timing.map_or("-".to_string(), |t| format!("{:.0}", mb(t.output_bytes) / t.total_seconds.max(f64::MIN_POSITIVE))),
                timing.map_or("-".to_string(), |t| triple(&t.bodies)),
                timing.map_or("-".to_string(), |t| triple(&t.first)),
                timing.map_or("-".to_string(), |t| triple(&t.workers)),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part a: redundancy over the whole set

#[derive(Default, Clone)]
struct LevelStats {
    total: u64,
    /// Bytes of the copies after the first with the same palette colours: within the body, across bodies of a
    /// source file, across source files.
    same_palette: [u64; 3],
    /// The same for copies with other colours (recolours).
    recolour: [u64; 3],
    /// The compressed estimate of all copies: each copy's share of its body's index + coverage zstd 19 size.
    estimated_compressed: f64,
}

/// The copies of one item in the whole set: (key, palette hash, raw bytes, body number, source number, compressed).
type Item = (u64, u64, u64, u32, u32, f64);

fn scope_of(first: (u32, u32), copy: (u32, u32)) -> usize {
    if first.0 == copy.0 {
        0
    } else if first.1 == copy.1 {
        1
    } else {
        2
    }
}

fn duplicates(items: &[Item]) -> LevelStats {
    let mut stats = LevelStats::default();
    let mut seen = HashMap::<u64, (u32, u32, HashSet<u64>)>::new();
    for &(key, palette, bytes, body, source, compressed) in items {
        stats.total += bytes;
        match seen.entry(key) {
            std::collections::hash_map::Entry::Vacant(place) => {
                place.insert((body, source, HashSet::from([palette])));
            }
            std::collections::hash_map::Entry::Occupied(mut place) => {
                let (first_body, first_source, palettes) = place.get_mut();
                let scope = scope_of((*first_body, *first_source), (body, source));
                if palettes.contains(&palette) {
                    stats.same_palette[scope] += bytes;
                } else {
                    stats.recolour[scope] += bytes;
                    palettes.insert(palette);
                }
                stats.estimated_compressed += compressed;
            }
        }
    }
    stats
}

fn source_of(label: &str) -> &str {
    label.split(" body ").next().unwrap_or(label)
}

fn part_a1(sources: &Sources, bodies: &[BodySource]) {
    struct FrameRow {
        key: u64,
        palette: u64,
        bytes: u64,
        action: u16,
        direction: u32,
    }
    struct BodyRow {
        client: Client,
        label: String,
        frames: Vec<FrameRow>,
        compressed_index: u64,
        compressed_coverage: u64,
        opaque: u64,
        cleared: u64,
        pixels: u64,
    }
    let started = Instant::now();
    let rows: Vec<BodyRow> = bodies
        .par_iter()
        .map(|source| {
            let planes = Planes::from_body(&load_body(sources, source));
            let frames = planes
                .boxes
                .iter()
                .map(|frame| {
                    let mut hasher = DefaultHasher::new();
                    (frame.w, frame.h).hash(&mut hasher);
                    planes.index[frame.range()].hash(&mut hasher);
                    if planes.client == Client::Ec {
                        planes.coverage[frame.range()].hash(&mut hasher);
                    }
                    let mut colours = DefaultHasher::new();
                    planes.palettes[frame.palette].hash(&mut colours);
                    FrameRow {
                        key: hasher.finish(),
                        palette: colours.finish(),
                        bytes: frame.pixels() as u64,
                        action: frame.action,
                        direction: frame.direction,
                    }
                })
                .collect();
            BodyRow {
                client: planes.client,
                label: planes.label.clone(),
                frames,
                compressed_index: compress(&planes.index, BEST_ZSTD_LEVEL, false).len() as u64,
                compressed_coverage: if planes.coverage.is_empty() { 0 } else { compress(&planes.coverage, BEST_ZSTD_LEVEL, false).len() as u64 },
                opaque: planes.opaque_pixels(),
                cleared: planes.cleared,
                pixels: planes.index.len() as u64,
            }
        })
        .collect();
    println!("part a1: every body hashed and compressed in {:.0} s", started.elapsed().as_secs_f64());

    for client in [Client::Cc, Client::Ec] {
        let rows: Vec<&BodyRow> = rows.iter().filter(|r| r.client == client && r.pixels > 0).collect();
        let opaque: u64 = rows.iter().map(|r| r.opaque).sum();
        let compressed: u64 = rows.iter().map(|r| r.compressed_index).sum();
        let coverage: u64 = rows.iter().map(|r| r.compressed_coverage).sum();
        let pixels: u64 = rows.iter().map(|r| r.pixels).sum();
        println!(
            "\n### {}: {:.1} Mpx, {:.1} Mpx opaque ({:.1} %); index plane zstd 19 per body {:.1} MB = {:.2} bits per opaque pixel; coverage {:.1} MB; non-zero indices under transparent pixels {}",
            client.label(),
            pixels as f64 / PIXELS_PER_MEGAPIXEL,
            opaque as f64 / PIXELS_PER_MEGAPIXEL,
            opaque as f64 * PERCENT / pixels.max(1) as f64,
            mb(compressed),
            compressed as f64 * f64::from(u8::BITS) / opaque.max(1) as f64,
            mb(coverage),
            rows.iter().map(|r| r.cleared).sum::<u64>(),
        );
        // Sequence keys per level.
        let source_ids: HashMap<&str, u32> = {
            let mut names: Vec<&str> = rows.iter().map(|r| source_of(&r.label)).collect();
            names.sort_unstable();
            names.dedup();
            names.into_iter().enumerate().map(|(n, name)| (name, n as u32)).collect()
        };
        let mut levels: [(&str, Vec<Item>); 4] = [("frame", Vec::new()), ("direction", Vec::new()), ("action", Vec::new()), ("body", Vec::new())];
        for (number, row) in rows.iter().enumerate() {
            let source = source_ids[source_of(&row.label)];
            let body_compressed = (row.compressed_index + row.compressed_coverage) as f64;
            let total_bytes: u64 = row.frames.iter().map(|f| f.bytes).sum();
            let share = |bytes: u64| body_compressed * bytes as f64 / total_bytes.max(1) as f64;
            let group = |frames: &[&FrameRow]| -> Item {
                let mut key = DefaultHasher::new();
                let mut palette = DefaultHasher::new();
                for frame in frames {
                    frame.key.hash(&mut key);
                    frame.palette.hash(&mut palette);
                }
                let bytes = frames.iter().map(|f| f.bytes).sum();
                (key.finish(), palette.finish(), bytes, number as u32, source, share(bytes))
            };
            for frame in &row.frames {
                levels[0].1.push((frame.key, frame.palette, frame.bytes, number as u32, source, share(frame.bytes)));
            }
            let mut by_direction = BTreeMap::<u32, Vec<&FrameRow>>::new();
            let mut by_action = BTreeMap::<u16, Vec<&FrameRow>>::new();
            for frame in &row.frames {
                by_direction.entry(frame.direction).or_default().push(frame);
                by_action.entry(frame.action).or_default().push(frame);
            }
            levels[1].1.extend(by_direction.values().map(|frames| group(frames)));
            levels[2].1.extend(by_action.values().map(|frames| group(frames)));
            let all: Vec<&FrameRow> = row.frames.iter().collect();
            levels[3].1.push(group(&all));
        }
        println!("\n| level | items | raw MB | same pixels and colours: in the body / across bodies of a file / across files MB | same pixels, other colours: in the body / across bodies / across files MB | removable raw % | compressed estimate MB |");
        println!("|---|---:|---:|---|---|---:|---:|");
        for (name, items) in &levels {
            let stats = duplicates(items);
            let removable: u64 = stats.same_palette.iter().chain(&stats.recolour).sum();
            println!(
                "| {name} | {} | {:.1} | {:.1} / {:.1} / {:.1} | {:.1} / {:.1} / {:.1} | {:.2} | {:.1} |",
                items.len(),
                mb(stats.total),
                mb(stats.same_palette[0]),
                mb(stats.same_palette[1]),
                mb(stats.same_palette[2]),
                mb(stats.recolour[0]),
                mb(stats.recolour[1]),
                mb(stats.recolour[2]),
                removable as f64 * PERCENT / stats.total.max(1) as f64,
                stats.estimated_compressed / BYTES_PER_MB,
            );
        }
    }
}

/// Runs a filter command over `input`, returning its output.
fn piped(command: &str, args: &[String], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|error| panic!("{command}: {error}"));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            stdin.write_all(input).expect("write to the filter");
        });
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).expect("read the filter");
        assert!(child.wait().expect("wait for the filter").success(), "{command} failed");
        output
    })
}

/// One solid stream of every index plane of a client: the upper bound of cross-body redundancy.
fn part_a2(sources: &Sources, bodies: &[BodySource]) {
    for (client, level) in [(Client::Cc, LEVEL_19), (Client::Ec, LEVEL_19), (Client::Cc, LEVEL_22)] {
        let started = Instant::now();
        let mut args = vec![format!("-{level}"), format!("--long={SOLID_WINDOW_LOG}"), "-T1".to_string(), "-c".to_string(), "-q".to_string()];
        if level > LEVEL_19 {
            args.push("--ultra".to_string());
        }
        let mut child = Command::new(COMMAND_ZSTD)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run zstd");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let counter = std::thread::spawn(move || {
            let mut total = 0u64;
            let mut buffer = vec![0u8; 1 << 20];
            loop {
                match stdout.read(&mut buffer).expect("read zstd") {
                    0 => return total,
                    n => total += n as u64,
                }
            }
        });
        let mut raw = 0u64;
        let of_client: Vec<&BodySource> = bodies
            .iter()
            .filter(|b| match client {
                Client::Cc => !matches!(b, BodySource::Ec { .. }),
                Client::Ec => matches!(b, BodySource::Ec { .. }),
            })
            .collect();
        for chunk in of_client.chunks(TEXT_CHUNK_BODIES) {
            let planes: Vec<Vec<u8>> = chunk.par_iter().map(|&source| Planes::from_body(&load_body(sources, source)).index).collect();
            for plane in planes {
                raw += plane.len() as u64;
                stdin.write_all(&plane).expect("write to zstd");
            }
        }
        drop(stdin);
        let compressed = counter.join().expect("counter");
        assert!(child.wait().expect("wait for zstd").success());
        println!(
            "Solid stream, {} index planes, zstd {level} --long={SOLID_WINDOW_LOG}: {:.1} MB raw -> {:.1} MB ({:.0} s)",
            client.label(),
            mb(raw),
            mb(compressed),
            started.elapsed().as_secs_f64()
        );
    }
}

/// Practical cross-body reuse on a sample, index planes only: a dictionary trained on it, and its largest bodies as a
/// raw reference for the others (what a family compressed against its reference saves, at best).
fn part_a2p(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)]) {
    for client in [Client::Cc, Client::Ec] {
        let sample = sample_of(present, client, present.iter().filter(|s| s.0 == client).count().div_ceil(DEFAULT_STRIDE));
        let planes: Vec<Vec<u8>> = sample.par_iter().map(|&i| Planes::from_body(&load_body(sources, &bodies[i])).index).collect();
        let base: u64 = planes.par_iter().map(|p| compress(p, LEVEL_19, false).len() as u64).sum();
        println!("\n{}: index planes of {} bodies, zstd 19 per body {:.1} MB", client.label(), planes.len(), mb(base));
        for dictionary_bytes in DICTIONARY_BYTES {
            let samples: Vec<&[u8]> = planes
                .iter()
                .flat_map(|plane| {
                    let step = plane.len().div_ceil(DICTIONARY_SAMPLES_PER_BODY).max(1);
                    plane.chunks(step).step_by(DICTIONARY_SAMPLE_EVERY)
                })
                .collect();
            let sizes: Vec<usize> = samples.iter().map(|s| s.len()).collect();
            let joined: Vec<u8> = samples.concat();
            let started = Instant::now();
            let dictionary = zstd::dict::from_continuous(&joined, &sizes, dictionary_bytes).expect("train a dictionary");
            let with: u64 = planes
                .par_iter()
                .map(|plane| {
                    let mut compressor = zstd::bulk::Compressor::with_dictionary(LEVEL_19, &dictionary).expect("zstd compressor");
                    compressor.compress(plane).expect("zstd compress").len() as u64
                })
                .sum();
            println!(
                "  trained dictionary of {} KiB (trained in {:.0} s): {:.1} MB, x{:.3}",
                dictionary.len() / BYTES_PER_KIB,
                started.elapsed().as_secs_f64(),
                mb(with + dictionary.len() as u64),
                (with + dictionary.len() as u64) as f64 / base.max(1) as f64
            );
        }
        // The largest body as a reference for the others: what a family would save at best.
        let mut order: Vec<&Vec<u8>> = planes.iter().collect();
        order.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let references = &order[..REFERENCE_BODIES.min(order.len())];
        let reference: Vec<u8> = references.iter().flat_map(|p| p.iter().copied()).collect();
        let rest = &order[references.len()..];
        let rest_base: u64 = rest.par_iter().map(|p| compress(p, LEVEL_19, false).len() as u64).sum();
        let with: u64 = rest
            .par_iter()
            .map(|plane| {
                let mut compressor = zstd::bulk::Compressor::with_dictionary(LEVEL_19, &reference).expect("zstd compressor");
                compressor.set_parameter(CParameter::WindowLog(LARGE_WINDOW_LOG)).expect("zstd window");
                compressor.compress(plane).expect("zstd compress").len() as u64
            })
            .sum();
        println!(
            "  the {} largest bodies ({:.1} MB raw) as a raw reference for the other {}: {:.1} MB -> {:.1} MB, x{:.3}",
            references.len(),
            mb(reference.len() as u64),
            rest.len(),
            mb(rest_base),
            mb(with),
            with as f64 / rest_base.max(1) as f64
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part c: general-purpose bounds

struct Strip {
    w: usize,
    h: usize,
    pixels: Vec<u8>,
}

/// The direction's frames stacked top to bottom, left-aligned, transparent padding; split at `max_height`.
fn direction_strips(planes: &Planes, max_height: usize) -> Vec<(Strip, Vec<usize>)> {
    let mut by_direction = BTreeMap::<u32, Vec<usize>>::new();
    for (i, frame) in planes.boxes.iter().enumerate() {
        by_direction.entry(frame.direction).or_default().push(i);
    }
    let mut strips = Vec::new();
    for frames in by_direction.values() {
        let mut open: Vec<usize> = Vec::new();
        let mut height = 0usize;
        let mut flush = |open: &mut Vec<usize>| {
            if open.is_empty() {
                return;
            }
            let w = open.iter().map(|&f| planes.boxes[f].w).max().unwrap();
            let h: usize = open.iter().map(|&f| planes.boxes[f].h).sum();
            let mut pixels = vec![0u8; w * h];
            let mut y0 = 0usize;
            for &f in open.iter() {
                let frame = &planes.boxes[f];
                let source = &planes.index[frame.range()];
                for y in 0..frame.h {
                    pixels[(y0 + y) * w..][..frame.w].copy_from_slice(&source[y * frame.w..][..frame.w]);
                }
                y0 += frame.h;
            }
            strips.push((Strip { w, h, pixels }, std::mem::take(open)));
        };
        for &f in frames {
            if height + planes.boxes[f].h > max_height {
                flush(&mut open);
                height = 0;
            }
            open.push(f);
            height += planes.boxes[f].h;
        }
        flush(&mut open);
    }
    strips
}

fn jxl_encode(pixels: &[u8], w: usize, h: usize, channels: u32, speed: EncoderSpeed) -> Vec<u8> {
    let mut encoder = encoder_builder()
        .has_alpha(channels == RGBA_CHANNELS as u32)
        .lossless(true)
        .uses_original_profile(true)
        .speed(speed)
        .color_encoding(if channels == 1 { jpegxl_rs::encode::ColorEncoding::SrgbLuma } else { jpegxl_rs::encode::ColorEncoding::Srgb })
        .build()
        .expect("JPEG XL encoder");
    let frame = EncoderFrame::new(pixels).num_channels(channels);
    encoder.encode_frame::<u8, u8>(&frame, w as u32, h as u32).expect("JPEG XL encode").data
}

fn jxl_decode(data: &[u8]) -> Vec<u8> {
    let decoder = decoder_builder().build().expect("JPEG XL decoder");
    decoder.decode_with::<u8>(data).expect("JPEG XL decode").1
}

/// WebP lossless of RGBA, `exact`: pixels under alpha 0 are kept.
fn webp_encode(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut config = webp::WebPConfig::new().expect("libwebp config");
    config.lossless = 1;
    config.quality = WEBP_QUALITY;
    config.method = WEBP_METHOD;
    config.exact = 1;
    let mut picture = webp::WebPPicture::new().expect("libwebp picture");
    picture.use_argb = 1;
    picture.width = w as i32;
    picture.height = h as i32;
    let mut writer = std::mem::MaybeUninit::<webp::WebPMemoryWriter>::uninit();
    // SAFETY: the buffers outlive the calls; the writer is initialised before use and cleared after the copy.
    unsafe {
        webp::WebPMemoryWriterInit(writer.as_mut_ptr());
        let mut writer = writer.assume_init();
        assert!(webp::WebPPictureImportRGBA(&mut picture, rgba.as_ptr(), (w * RGBA_CHANNELS) as i32) != 0, "WebP import");
        picture.writer = Some(webp::WebPMemoryWrite);
        picture.custom_ptr = (&mut writer as *mut webp::WebPMemoryWriter).cast();
        assert!(webp::WebPEncode(&config, &mut picture) != 0, "WebP encode, error {:?}", picture.error_code);
        let out = std::slice::from_raw_parts(writer.mem, writer.size).to_vec();
        webp::WebPMemoryWriterClear(&mut writer);
        webp::WebPPictureFree(&mut picture);
        out
    }
}

fn webp_decode(data: &[u8]) -> Vec<u8> {
    let (mut w, mut h) = (0i32, 0i32);
    // SAFETY: on success libwebp returns w * h RGBA pixels, freed with WebPFree after the copy.
    unsafe {
        let pixels = webp::WebPDecodeRGBA(data.as_ptr(), data.len(), &mut w, &mut h);
        assert!(!pixels.is_null(), "WebP decode");
        let out = std::slice::from_raw_parts(pixels, w as usize * h as usize * RGBA_CHANNELS).to_vec();
        webp::WebPFree(pixels.cast());
        out
    }
}

/// An index plane as RGBA: the value in green, so nothing else varies.
fn green_rgba(pixels: &[u8]) -> Vec<u8> {
    pixels.iter().flat_map(|&v| [0, v, 0, OPAQUE]).collect()
}

const GREEN: usize = 1;

fn images_of(planes: &Planes, strips: bool, max_height: usize) -> Vec<Strip> {
    if strips {
        direction_strips(planes, max_height).into_iter().map(|(strip, _)| strip).collect()
    } else {
        planes.boxes.iter().map(|f| Strip { w: f.w, h: f.h, pixels: planes.index[f.range()].to_vec() }).collect()
    }
}

/// Encodes the images in parallel, checks them, times their decode on one thread.
fn bound_images(name: &str, sample: &[&Planes], strips: bool, max_height: usize, encode: &(dyn Fn(&Strip) -> Vec<u8> + Sync), decode: &(dyn Fn(&[u8], &Strip) -> Vec<u8> + Sync), single: &rayon::ThreadPool) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let started = Instant::now();
        let encoded: Vec<(Vec<Strip>, Vec<Vec<u8>>)> = bodies
            .par_iter()
            .map(|planes| {
                let images = images_of(planes, strips, max_height);
                let data: Vec<Vec<u8>> = images.par_iter().map(|image| encode(image)).collect();
                for (image, bytes) in images.iter().zip(&data) {
                    assert!(decode(bytes, image) == image.pixels, "{} / {name}: does not round-trip", planes.label);
                }
                (images, data)
            })
            .collect();
        let encode_seconds = started.elapsed().as_secs_f64();
        let bytes: u64 = encoded.iter().flat_map(|(_, data)| data.iter().map(|d| d.len() as u64)).sum();
        let zstd_base: u64 = bodies.iter().map(|p| compress(&p.index, BEST_ZSTD_LEVEL, false).len() as u64).sum();
        let opaque: u64 = bodies.iter().map(|p| p.opaque_pixels()).sum();
        let raw: u64 = bodies.iter().map(|p| p.index.len() as u64).sum();
        let mut body_times = Vec::new();
        single.install(|| {
            for (images, data) in &encoded {
                let run = fastest(|| images.iter().zip(data).map(|(image, bytes)| decode(bytes, image).len()).sum::<usize>());
                body_times.push(run.as_secs_f64());
            }
        });
        let total: f64 = body_times.iter().sum();
        println!(
            "| {} | {name} | {} | {:.2} | {:.2} | {:.3} | {:.0} | {:.0} s |",
            client.label(),
            bodies.len(),
            mb(bytes),
            bytes as f64 * f64::from(u8::BITS) / opaque.max(1) as f64,
            bytes as f64 / zstd_base.max(1) as f64,
            mb(raw) / total.max(f64::MIN_POSITIVE),
            encode_seconds,
        );
    }
}

/// Index-plane bounds with general compressors over each body, one process per body.
fn bound_tool(name: &str, sample: &[&Planes], command: &str, encode_args: &[String], decode_args: &[String], single: &rayon::ThreadPool) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let started = Instant::now();
        let encoded: Vec<Vec<u8>> = bodies.par_iter().map(|p| piped(command, encode_args, &p.index)).collect();
        let encode_seconds = started.elapsed().as_secs_f64();
        for (planes, bytes) in bodies.iter().zip(&encoded) {
            assert!(piped(command, decode_args, bytes) == planes.index, "{} / {name}: does not round-trip", planes.label);
        }
        let bytes: u64 = encoded.iter().map(|e| e.len() as u64).sum();
        let zstd_base: u64 = bodies.iter().map(|p| compress(&p.index, BEST_ZSTD_LEVEL, false).len() as u64).sum();
        let opaque: u64 = bodies.iter().map(|p| p.opaque_pixels()).sum();
        let raw: u64 = bodies.iter().map(|p| p.index.len() as u64).sum();
        let total: f64 = single.install(|| encoded.iter().map(|e| fastest(|| piped(command, decode_args, e).len()).as_secs_f64()).sum());
        println!(
            "| {} | {name} | {} | {:.2} | {:.2} | {:.3} | {:.0} | {:.0} s |",
            client.label(),
            bodies.len(),
            mb(bytes),
            bytes as f64 * f64::from(u8::BITS) / opaque.max(1) as f64,
            bytes as f64 / zstd_base.max(1) as f64,
            mb(raw) / total.max(f64::MIN_POSITIVE),
            encode_seconds,
        );
    }
}

fn by_size<'a>(planes: &[&'a Planes], count: usize) -> Vec<&'a Planes> {
    let mut out = Vec::new();
    for client in [Client::Cc, Client::Ec] {
        let of: Vec<&Planes> = planes.iter().copied().filter(|p| p.client == client).collect();
        out.extend(spread(&of, count, |p| p.index.len() as u64).into_iter().copied());
    }
    out
}

fn part_c(sample: &[Planes], order: Order) {
    println!("\n### General-purpose bounds on the index plane; CPU {}; load average {}\n", cpu_state(), load_average());
    println!("| client | coder | bodies | MB | bits per opaque px | x zstd 19 per body | decode MB/s, one thread | encode time |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let strings = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
    let original: Vec<&Planes> = sample.iter().collect();
    bound_tool("xz -9e, body", &original, COMMAND_XZ, &strings(&["-9e", "-T1", "-c"]), &strings(&["-d", "-c"]), &single);
    let lgwin = format!("--large_window={BROTLI_LARGE_WINDOW_BITS}");
    bound_tool("brotli 11, large window, body", &original, COMMAND_BROTLI, &strings(&["-q", "11", lgwin.as_str(), "-c"]), &strings(&["-d", lgwin.as_str(), "-c"]), &single);

    let sorted: Vec<Planes> = sample.par_iter().map(|p| reordered(p, order)).collect();
    let sorted_refs: Vec<&Planes> = sorted.iter().collect();
    let webp_encode_index = |image: &Strip| webp_encode(&green_rgba(&image.pixels), image.w, image.h);
    let webp_decode_index = |data: &[u8], _: &Strip| webp_decode(data).iter().skip(GREEN).step_by(RGBA_CHANNELS).copied().collect::<Vec<u8>>();
    for (name, planes) in [("original", &original), ("sorted", &sorted_refs)] {
        for (strips, unit) in [(false, "per frame"), (true, "per direction strip")] {
            bound_images(&format!("WebP lossless m6, {name}, {unit}"), planes, strips, WEBP_MAX_SIDE, &webp_encode_index, &webp_decode_index, &single);
        }
    }
    for (speed, effort) in JXL_EFFORTS {
        let limit = if matches!(speed, EncoderSpeed::Tortoise) { JXL_EFFORT9_SAMPLE_PER_CLIENT } else { BOUND_SAMPLE_PER_CLIENT };
        jxl_rows(&original, &sorted_refs, speed, effort, limit, &single);
    }
}

fn jxl_rows(original: &[&Planes], sorted: &[&Planes], speed: EncoderSpeed, effort: &str, limit: usize, single: &rayon::ThreadPool) {
    let encode = move |image: &Strip| jxl_encode(&image.pixels, image.w, image.h, 1, speed);
    let decode = |data: &[u8], _: &Strip| jxl_decode(data);
    for (name, planes) in [("original", original), ("sorted", sorted)] {
        let chosen = by_size(planes, limit);
        for (strips, unit) in [(false, "per frame"), (true, "per direction strip")] {
            bound_images(&format!("JPEG XL lossless {effort}, {name}, {unit}, {limit} bodies"), &chosen, strips, usize::MAX, &encode, &decode, single);
        }
    }
}

/// JPEG XL effort 7 on the bodies of the effort 9 rows, so the two compare directly.
fn part_c7(sample: &[Planes], order: Order) {
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let sorted: Vec<Planes> = sample.par_iter().map(|p| reordered(p, order)).collect();
    let (original, sorted_refs): (Vec<&Planes>, Vec<&Planes>) = (sample.iter().collect(), sorted.iter().collect());
    jxl_rows(&original, &sorted_refs, JXL_EFFORTS[0].0, JXL_EFFORTS[0].1, JXL_EFFORT9_SAMPLE_PER_CLIENT, &single);
}

// ---------------------------------------------------------------------------------------------------------------
// Part w: the winners over the whole set, one body at a time so the set does not have to fit in memory

/// Colours of one frame as straight-alpha RGBA.
fn frame_rgba(planes: &Planes, frame: &FrameBox, index: &[u8], coverage: &[u8]) -> Vec<u8> {
    let palette = &planes.palettes[frame.palette];
    let mut out = Vec::with_capacity(frame.pixels() * RGBA_CHANNELS);
    for i in frame.range() {
        let [r, g, b, _] = palette[usize::from(index[i])];
        match planes.client {
            Client::Cc if index[i] == 0 => out.extend_from_slice(&[0; RGBA_CHANNELS]),
            Client::Cc => out.extend_from_slice(&[r, g, b, OPAQUE]),
            Client::Ec if coverage[i] == COVERAGE_TRANSPARENT => out.extend_from_slice(&[0; RGBA_CHANNELS]),
            Client::Ec => out.extend_from_slice(&[r, g, b, alpha_of_coverage(coverage[i])]),
        }
    }
    out
}

fn part_w(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], only: &[String]) {
    let chosen: Vec<&Candidate> = CANDIDATES.iter().filter(|c| only.iter().any(|filter| c.name.contains(filter.as_str()))).collect();
    let started = Instant::now();
    let done = AtomicUsize::new(0);
    // Per body: (client, per candidate (index, coverage), per order (palette hashes)).
    #[derive(Default)]
    struct Row {
        sizes: Vec<(u64, u64)>,
        palettes: HashMap<Order, HashSet<u64>>,
    }
    let orders: Vec<Order> = {
        let mut orders = vec![Order::Original];
        for c in &chosen {
            if !orders.contains(&c.order) {
                orders.push(c.order);
            }
        }
        orders
    };
    let rows: Vec<(Client, Row)> = present
        .par_iter()
        .map(|&(client, _, i)| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            let mut row = Row { sizes: vec![(0, 0); chosen.len()], ..Row::default() };
            for &order in &orders {
                let ordered = reordered(&planes, order);
                for (n, candidate) in chosen.iter().enumerate().filter(|(_, c)| c.order == order) {
                    let encoded = encode_candidate(&ordered, candidate);
                    verify_round_trip(&ordered, &encoded, candidate);
                    row.sizes[n] = (encoded.index_bytes(), encoded.coverage_bytes());
                }
                let hashes = ordered.palettes.iter().map(|colours| {
                    let mut hasher = DefaultHasher::new();
                    colours.hash(&mut hasher);
                    hasher.finish()
                });
                row.palettes.insert(order, hashes.collect());
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % PROGRESS_EVERY == 0 {
                println!("  {n}/{} bodies ({:.0} s)", present.len(), started.elapsed().as_secs_f64());
            }
            (client, row)
        })
        .collect();
    println!("\nWhole set, every body encoded and verified in {:.0} s", started.elapsed().as_secs_f64());
    for client in [Client::Cc, Client::Ec] {
        let of: Vec<&Row> = rows.iter().filter(|r| r.0 == client).map(|r| &r.1).collect();
        let entry_bytes = match client {
            Client::Cc => CC_PALETTE_ENTRY_BYTES,
            Client::Ec => EC_PALETTE_ENTRY_BYTES,
        };
        println!("\n### {}: whole set, {} bodies\n", client.label(), of.len());
        println!("| candidate | index MB | shape/coverage MB | total MB |");
        println!("|---|---:|---:|---:|");
        for (n, candidate) in chosen.iter().enumerate() {
            let index: u64 = of.iter().map(|r| r.sizes[n].0).sum();
            let coverage: u64 = of.iter().map(|r| r.sizes[n].1).sum();
            println!("| {} | {:.1} | {:.1} | {:.1} |", candidate.name, mb(index), mb(coverage), mb(index + coverage));
        }
        for &order in &orders {
            let distinct: HashSet<u64> = of.iter().flat_map(|r| r.palettes[&order].iter().copied()).collect();
            println!(
                "Distinct palettes over the set, order {order:?}: {} = {:.1} MB raw",
                distinct.len(),
                mb((distinct.len() * PALETTE_COLOURS * entry_bytes) as u64)
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part d: the coverage plane, reduced (lossy)

fn reduced_coverage(coverage: u8, step: u8) -> u8 {
    if coverage == COVERAGE_TRANSPARENT || coverage == COVERAGE_SOLID {
        return coverage;
    }
    // To the nearest multiple of the step, never to fully transparent.
    (((coverage + step / 2) / step) * step).clamp(step, COVERAGE_SOLID)
}

fn all_frames(planes: &Planes, index: &[u8], coverage: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    planes.boxes.iter().map(|f| (f.w as u32, f.h as u32, frame_rgba(planes, f, index, coverage))).collect()
}

fn ssimulacra2_of(reference: &[(u32, u32, Vec<u8>)], test: &[(u32, u32, Vec<u8>)]) -> f64 {
    let view = |frames: &'_ [(u32, u32, Vec<u8>)]| frames.iter().map(|(w, h, rgba)| (*w, *h, rgba.clone())).collect::<Vec<_>>();
    let (reference, test) = (view(reference), view(test));
    let as_refs = |frames: &'_ [(u32, u32, Vec<u8>)]| -> Rgb {
        let refs: Vec<(u32, u32, &[u8])> = frames.iter().map(|(w, h, rgba)| (*w, *h, rgba.as_slice())).collect();
        mosaic(&refs)
    };
    compute_frame_ssimulacra2(as_refs(&reference), as_refs(&test)).expect("SSIMULACRA2")
}

fn part_d(sample: &[Planes]) {
    let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == Client::Ec).collect();
    println!("\n### Enhanced coverage, reduced (LOSSY) on {} bodies: zstd 19 and rANS on the same plane, per body\n", bodies.len());
    println!("| levels | zstd 19 MB | rANS MB | SSIMULACRA2 median / min |");
    println!("|---|---:|---:|---|");
    let measure = |step: u8| -> (u64, u64, Vec<f64>) {
        let rows: Vec<(u64, u64, f64)> = bodies
            .par_iter()
            .map(|planes| {
                let coverage: Vec<u8> = planes.coverage.iter().map(|&c| reduced_coverage(c, step)).collect();
                let mut lossy = (*planes).clone();
                lossy.coverage = coverage;
                let plane = compress(&lossy.coverage, BEST_ZSTD_LEVEL, false).len() as u64;
                let candidate = candidate("coverage rANS", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_RANS, Unit::Body);
                let encoded = encode_candidate(&lossy, &candidate);
                verify_round_trip(&lossy, &encoded, &candidate);
                let score = if step == 1 {
                    f64::INFINITY
                } else {
                    ssimulacra2_of(&all_frames(planes, &planes.index, &planes.coverage), &all_frames(&lossy, &lossy.index, &lossy.coverage))
                };
                (plane, encoded.coverage_bytes(), score)
            })
            .collect();
        (rows.iter().map(|r| r.0).sum(), rows.iter().map(|r| r.1).sum(), rows.iter().map(|r| r.2).collect())
    };
    let (plane, rans, _) = measure(1);
    println!("| 16 (lossless) | {:.2} | {:.2} | - |", mb(plane), mb(rans));
    for (step, name) in COVERAGE_LEVEL_STEPS {
        let (plane, rans, scores) = measure(step);
        let scores = sorted(scores);
        println!("| {name} | {:.2} | {:.2} | {:.2} / {:.2} |", mb(plane), mb(rans), percentile(&scores, MEDIAN), scores[0]);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part e: near-lossless index (lossy)

/// Replaces a pixel's index with the left, upper or previous frame's when that colour is within `threshold` of the
/// original on every channel and has the same keep-colour flag. Returns the plane and the largest channel error.
fn near_lossless(planes: &Planes, threshold: u8) -> (Vec<u8>, u8) {
    let mut out = planes.index.clone();
    let mut worst = 0u8;
    let previous = {
        let all: Vec<usize> = (0..planes.boxes.len()).collect();
        previous_frames(&planes.boxes, &all)
    };
    for (n, frame) in planes.boxes.iter().enumerate() {
        let palette = &planes.palettes[frame.palette];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = frame.start + y * frame.w + x;
                if !planes.is_opaque(i) {
                    continue;
                }
                let original = palette[usize::from(planes.index[i])];
                let left = (x > 0 && planes.is_opaque(i - 1)).then(|| out[i - 1]);
                let up = (y > 0 && planes.is_opaque(i - frame.w)).then(|| out[i - frame.w]);
                let before = previous[n].and_then(|p| {
                    let (px, py) = (x as i32 - p.dx, y as i32 - p.dy);
                    if px < 0 || py < 0 || px >= p.w as i32 || py >= p.h as i32 {
                        return None;
                    }
                    let at = p.start + py as usize * p.w + px as usize;
                    planes.is_opaque(at).then(|| out[at])
                });
                let mut try_candidate = |candidate: Option<u8>| {
                    let Some(value) = candidate else { return false };
                    let colour = palette[usize::from(value)];
                    if colour[PALETTE_KEEP_COLOUR_BYTE] != original[PALETTE_KEEP_COLOUR_BYTE] {
                        return false;
                    }
                    let error = (0..LUMA_WEIGHTS.len()).map(|c| colour[c].abs_diff(original[c])).max().unwrap();
                    if error > threshold {
                        return false;
                    }
                    out[i] = value;
                    worst = worst.max(error);
                    true
                };
                let _ = try_candidate(left) || try_candidate(up) || try_candidate(before);
            }
        }
    }
    (out, worst)
}

fn part_e(sample: &[Planes]) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let thresholds: &[u8] = if client == Client::Cc { &NEAR_LOSSLESS_CC } else { &NEAR_LOSSLESS_EC };
        println!("\n### {} near-lossless index (LOSSY), {} bodies: index plane only\n", client.label(), bodies.len());
        println!("| threshold | zstd 19 MB | path, MED, rANS MB | largest channel error | SSIMULACRA2 median / min |");
        println!("|---|---:|---:|---:|---|");
        let rans = candidate("rANS", Order::Path, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, Unit::Body);
        let lossless: (u64, u64) = bodies
            .par_iter()
            .map(|planes| {
                let ordered = reordered(planes, Order::Path);
                (compress(&planes.index, BEST_ZSTD_LEVEL, false).len() as u64, encode_candidate(&ordered, &rans).index_bytes())
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        println!("| 0 (lossless) | {:.2} | {:.2} | 0 | - |", mb(lossless.0), mb(lossless.1));
        for &threshold in thresholds {
            let rows: Vec<(u64, u64, u8, f64)> = bodies
                .par_iter()
                .map(|planes| {
                    let (index, worst) = near_lossless(planes, threshold);
                    let mut lossy = (*planes).clone();
                    lossy.index = index;
                    let score = ssimulacra2_of(&all_frames(planes, &planes.index, &planes.coverage), &all_frames(&lossy, &lossy.index, &lossy.coverage));
                    let plane = compress(&lossy.index, BEST_ZSTD_LEVEL, false).len() as u64;
                    let ordered = reordered(&lossy, Order::Path);
                    let encoded = encode_candidate(&ordered, &rans);
                    verify_round_trip(&ordered, &encoded, &rans);
                    (plane, encoded.index_bytes(), worst, score)
                })
                .collect();
            let scores = sorted(rows.iter().map(|r| r.3).collect::<Vec<f64>>());
            println!(
                "| {threshold} | {:.2} | {:.2} | {} | {:.2} / {:.2} |",
                mb(rows.iter().map(|r| r.0).sum()),
                mb(rows.iter().map(|r| r.1).sum()),
                rows.iter().map(|r| r.2).max().unwrap_or(0),
                percentile(&scores, MEDIAN),
                scores[0],
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Part f: upscaled content

/// Index coders of the indexed upscales.
const UPSCALE_CANDIDATES: [Candidate; 5] = [
    candidate("zstd 19, body", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
    candidate("zstd 19, action", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Action),
    candidate("luma, MED, rANS, body", Order::Luma, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, Unit::Body),
    candidate("luma, MED, rANS, action", Order::Luma, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, Unit::Action),
    candidate("rank + previous frame context, rANS, action", Order::Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Action),
];

/// Planes from requantized frames; the Enhanced plane layout whatever the source, as there is partial coverage.
fn requantized_planes(body: &Body, filter: Upscaler) -> (Planes, Vec<(u32, u32, Vec<u8>)>) {
    let scaled = upscale_body(body, filter);
    let mut boxes = Vec::new();
    let mut inputs = Vec::new();
    let mut rgba = Vec::new();
    let mut start = 0usize;
    for (direction, (frames, scaled)) in body.directions.iter().zip(&scaled).enumerate() {
        for (frame, s) in frames.iter().zip(scaled) {
            boxes.push(FrameBox {
                w: s.width as usize,
                h: s.height as usize,
                left: s.left,
                top: s.top,
                action: frame.action,
                direction: direction as u32,
                palette: 0,
                start,
            });
            start += (s.width * s.height) as usize;
            inputs.push(RgbaFrame {
                width: s.width as u16,
                height: s.height as u16,
                center_x: 0,
                center_y: 0,
                rgba: s.rgba.clone(),
                hue_mask: scaled_hue_mask(body, frame, s),
            });
            rgba.push((s.width, s.height, s.rgba.clone()));
        }
    }
    let (palette, indexed, _) = index_frames(&inputs, None).expect("requantize");
    let mut colours: Vec<[u8; EC_PALETTE_ENTRY_BYTES]> = palette.to_vec();
    colours.resize(PALETTE_COLOURS, [0; EC_PALETTE_ENTRY_BYTES]);
    let mut index = Vec::with_capacity(start);
    let mut coverage = Vec::with_capacity(start);
    for frame in &indexed {
        for (&i, &c) in frame.indices.iter().zip(&frame.coverage) {
            index.push(if c == COVERAGE_TRANSPARENT { 0 } else { i });
            coverage.push(c);
        }
    }
    (Planes { label: body.label.clone(), client: Client::Ec, boxes, index, coverage, palettes: vec![colours], bc7_zstd: 0, cleared: 0 }, rgba)
}

const CHROMA_OFFSET: u8 = 128;

/// YCoCg-R in wrapping 8-bit arithmetic, the chroma centred on [`CHROMA_OFFSET`]; reversible whatever the values.
fn ycocg(r: u8, g: u8, b: u8) -> [u8; 3] {
    let co = r.wrapping_sub(b);
    let t = b.wrapping_add(((co as i8) >> 1) as u8);
    let cg = g.wrapping_sub(t);
    let y = t.wrapping_add(((cg as i8) >> 1) as u8);
    [y, co.wrapping_add(CHROMA_OFFSET), cg.wrapping_add(CHROMA_OFFSET)]
}

fn rgb_of_ycocg(y: u8, co: u8, cg: u8) -> [u8; 3] {
    let (co, cg) = (co.wrapping_sub(CHROMA_OFFSET), cg.wrapping_sub(CHROMA_OFFSET));
    let t = y.wrapping_sub(((cg as i8) >> 1) as u8);
    let g = cg.wrapping_add(t);
    let b = t.wrapping_sub(((co as i8) >> 1) as u8);
    [co.wrapping_add(b), g, b]
}

const COLOUR_PLANES: usize = 3;
/// All pixels of a frame count as known for the alpha plane: its transparency is what it codes.
const ALPHA_ALL_KNOWN: u8 = 1;

/// RGBA frames coded losslessly as alpha + YCoCg-R, each plane MED + rANS with gradient contexts; RGB under alpha 0
/// is normalised to 0 first and counts as equal.
struct ColourStream {
    tables: Vec<Vec<u8>>,
    streams: Vec<Vec<u8>>,
}

fn normalised(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(RGBA_CHANNELS).flat_map(|p| if p[ALPHA] == 0 { [0; RGBA_CHANNELS] } else { [p[0], p[1], p[2], p[ALPHA]] }).collect()
}

fn encode_colour(frames: &[(u32, u32, Vec<u8>)]) -> ColourStream {
    let mut symbols: Vec<Vec<(u16, u8)>> = vec![Vec::new(); RGBA_CHANNELS];
    for (w, h, rgba) in frames {
        let (w, h) = (*w as usize, *h as usize);
        let alpha: Vec<u8> = rgba.chunks_exact(RGBA_CHANNELS).map(|p| p[ALPHA]).collect();
        let known = vec![ALPHA_ALL_KNOWN; alpha.len()];
        let mut planes = vec![vec![0u8; alpha.len()]; COLOUR_PLANES];
        for (i, p) in rgba.chunks_exact(RGBA_CHANNELS).enumerate() {
            let [y, co, cg] = ycocg(p[0], p[1], p[2]);
            planes[0][i] = y;
            planes[1][i] = co;
            planes[2][i] = cg;
        }
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (prediction, context) = predict(&alpha, &known, w, i, x, y > 0, Predictor::Med, last);
                symbols[COLOUR_PLANES].push((context as u16, alpha[i].wrapping_sub(prediction)));
                last = alpha[i];
            }
        }
        for (plane, out) in planes.iter().zip(symbols.iter_mut()) {
            let mut last = FIRST_PREDICTION;
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if alpha[i] == 0 {
                        continue;
                    }
                    let (prediction, context) = predict(plane, &alpha, w, i, x, y > 0, Predictor::Med, last);
                    out.push((context as u16, plane[i].wrapping_sub(prediction)));
                    last = plane[i];
                }
            }
        }
    }
    let mut stream = ColourStream { tables: Vec::new(), streams: Vec::new() };
    for plane in &symbols {
        let mut hist = vec![0u32; INDEX_CONTEXTS * BYTE_SYMBOLS];
        for &(context, symbol) in plane {
            hist[usize::from(context) * BYTE_SYMBOLS + usize::from(symbol)] += 1;
        }
        let model = Model::from_hist(BYTE_SYMBOLS, &hist);
        stream.tables.push(model.serialized());
        stream.streams.push(rans_encode(plane, &model));
    }
    stream
}

fn decode_colour(stream: &ColourStream, sizes: &[(u32, u32)]) -> Vec<Vec<u8>> {
    let models: Vec<Model> = stream.tables.iter().map(|t| Model::parse(t, BYTE_SYMBOLS, INDEX_CONTEXTS)).collect();
    let mut decoders: Vec<RansDecoder> = stream.streams.iter().map(|s| RansDecoder::new(s)).collect();
    let mut out = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        let (w, h) = (w as usize, h as usize);
        let total = w * h;
        let known = vec![ALPHA_ALL_KNOWN; total];
        let mut alpha = vec![0u8; total];
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let (prediction, context) = predict(&alpha, &known, w, i, x, y > 0, Predictor::Med, last);
                alpha[i] = prediction.wrapping_add(decoders[COLOUR_PLANES].get(&models[COLOUR_PLANES], context));
                last = alpha[i];
            }
        }
        let mut planes = vec![vec![0u8; total]; COLOUR_PLANES];
        for (n, plane) in planes.iter_mut().enumerate() {
            let mut last = FIRST_PREDICTION;
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if alpha[i] == 0 {
                        continue;
                    }
                    let (prediction, context) = predict(plane, &alpha, w, i, x, y > 0, Predictor::Med, last);
                    plane[i] = prediction.wrapping_add(decoders[n].get(&models[n], context));
                    last = plane[i];
                }
            }
        }
        let mut rgba = Vec::with_capacity(total * RGBA_CHANNELS);
        for i in 0..total {
            if alpha[i] == 0 {
                rgba.extend_from_slice(&[0; RGBA_CHANNELS]);
            } else {
                let [r, g, b] = rgb_of_ycocg(planes[0][i], planes[1][i], planes[2][i]);
                rgba.extend_from_slice(&[r, g, b, alpha[i]]);
            }
        }
        out.push(rgba);
    }
    out
}

#[derive(Default, Clone)]
struct UpscaleTally {
    bytes: u64,
    output: u64,
    seconds: f64,
}

impl UpscaleTally {
    fn add(&mut self, bytes: u64, output: u64, seconds: f64) {
        self.bytes += bytes;
        self.output += output;
        self.seconds += seconds;
    }
}

/// The size and one-thread decode of one indexed upscale with the coders of [`UPSCALE_CANDIDATES`].
fn upscale_indexed(planes: &Planes, time: bool, single: &rayon::ThreadPool) -> Vec<(u64, f64)> {
    UPSCALE_CANDIDATES
        .iter()
        .map(|candidate| {
            let ordered = reordered(planes, candidate.order);
            let encoded = encode_candidate(&ordered, candidate);
            verify_round_trip(&ordered, &encoded, candidate);
            let seconds = if time {
                single
                    .install(|| {
                        fastest(|| {
                            let shared = shared_models(&encoded, candidate, ordered.client);
                            encoded.units.iter().map(|u| decode_unit(u, &ordered.boxes, &ordered.palettes, candidate, ordered.client, &shared).0.len()).sum::<usize>()
                        })
                    })
                    .as_secs_f64()
            } else {
                0.0
            };
            (encoded.total(), seconds)
        })
        .collect()
}

fn part_f(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], time: bool) {
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    println!("\n### Upscaled content at {UPSCALE}x; CPU {}; load average {}", cpu_state(), load_average());
    for client in [Client::Cc, Client::Ec] {
        let bodies_wanted = std::env::var(UPSCALE_BODIES_ENV).ok().map_or(UPSCALE_SAMPLE_PER_CLIENT, |n| n.parse().expect("a body count"));
        let sample = sample_of(present, client, bodies_wanted);
        let mut bc7 = 0u64;
        let mut mmpx = vec![UpscaleTally::default(); UPSCALE_CANDIDATES.len()];
        let mut xbrz = vec![UpscaleTally::default(); UPSCALE_CANDIDATES.len()];
        let mut colour = UpscaleTally::default();
        let mut raw_rgba = 0u64;
        let mut mmpx_unmapped = 0u64;
        let mut webp_frame = UpscaleTally::default();
        let mut webp_strip = UpscaleTally::default();
        let mut jxl_frame = UpscaleTally::default();
        // Parallel over bodies when only sizes are wanted; one body at a time when timing, as timing needs a quiet machine.
        let measure = |&i: &usize| {
            let body = load_body(sources, &bodies[i]);
            let (mapped, unmapped) = reindexed(&body, &upscale_body(&body, apply_mmpx));
            let mmpx_planes = Planes::from_body(&mapped);
            let mmpx_rows = upscale_indexed(&mmpx_planes, time, &single);
            let mmpx_output = mmpx_planes.resident_bytes();
            drop(mmpx_planes);
            let (quantized, rgba) = requantized_planes(&body, apply_xbrz);
            let xbrz_rows = upscale_indexed(&quantized, time, &single);
            let xbrz_output = quantized.resident_bytes();
            drop(quantized);
            let bc7: Vec<u8> = rgba.iter().flat_map(|(w, h, data)| encode_bc7(data, *w, *h, Bc7EncoderBackend::AnalyticalWide)).collect();
            let bc7 = compress(&bc7, PACKER_ZSTD_LEVEL, false).len() as u64;
            let frames: Vec<(u32, u32, Vec<u8>)> = rgba.into_iter().map(|(w, h, data)| (w, h, normalised(&data))).collect();
            let raw: u64 = frames.iter().map(|f| f.2.len() as u64).sum();
            let stream = encode_colour(&frames);
            let sizes: Vec<(u32, u32)> = frames.iter().map(|f| (f.0, f.1)).collect();
            let decoded = decode_colour(&stream, &sizes);
            assert!(decoded.iter().zip(&frames).all(|(a, b)| *a == b.2), "{}: RGBA colour stream does not round-trip", body.label);
            drop(decoded);
            let colour_bytes: u64 = stream.tables.iter().chain(&stream.streams).map(|s| s.len() as u64).sum();
            let colour_seconds = if time { single.install(|| fastest(|| decode_colour(&stream, &sizes).len())).as_secs_f64() } else { 0.0 };
            // WebP and JPEG XL lossless, frame by frame; WebP also as one strip per direction.
            let webps: Vec<Vec<u8>> = frames.par_iter().map(|(w, h, data)| webp_encode(data, *w as usize, *h as usize)).collect();
            let webp_seconds = if time { single.install(|| fastest(|| webps.iter().map(|d| webp_decode(d).len()).sum::<usize>())).as_secs_f64() } else { 0.0 };
            assert!(webps.iter().zip(&frames).all(|(d, f)| webp_decode(d) == f.2), "{}: WebP does not round-trip", body.label);
            let webp_bytes: u64 = webps.iter().map(|d| d.len() as u64).sum();
            drop(webps);
            let jxls: Vec<Vec<u8>> = frames.par_iter().map(|(w, h, data)| jxl_encode(data, *w as usize, *h as usize, RGBA_CHANNELS as u32, EncoderSpeed::Squirrel)).collect();
            let jxl_seconds = if time { single.install(|| fastest(|| jxls.iter().map(|d| jxl_decode(d).len()).sum::<usize>())).as_secs_f64() } else { 0.0 };
            assert!(jxls.iter().zip(&frames).all(|(d, f)| jxl_decode(d) == f.2), "{}: JPEG XL does not round-trip", body.label);
            let jxl_bytes: u64 = jxls.iter().map(|d| d.len() as u64).sum();
            drop(jxls);
            let strip = rgba_strips(&frames);
            let strips: Vec<Vec<u8>> = strip.par_iter().map(|(w, h, data)| webp_encode(data, *w as usize, *h as usize)).collect();
            assert!(strips.iter().zip(&strip).all(|(d, f)| webp_decode(d) == f.2), "{}: WebP strip does not round-trip", body.label);
            let strip_seconds = if time { single.install(|| fastest(|| strips.iter().map(|d| webp_decode(d).len()).sum::<usize>())).as_secs_f64() } else { 0.0 };
            let strip_bytes: u64 = strips.iter().map(|d| d.len() as u64).sum();
            println!("  {} done: {:.1} Mpx upscaled", body.label, raw as f64 / RGBA_CHANNELS as f64 / PIXELS_PER_MEGAPIXEL);
            (bc7, unmapped, mmpx_rows, mmpx_output, xbrz_rows, xbrz_output, raw, colour_bytes, colour_seconds, webp_bytes, webp_seconds, jxl_bytes, jxl_seconds, strip_bytes, strip_seconds)
        };
        let rows: Vec<_> = if time { sample.iter().map(measure).collect() } else { sample.par_iter().map(measure).collect() };
        for row in rows {
            let (b, unmapped, mmpx_rows, mmpx_output, xbrz_rows, xbrz_output, raw, colour_bytes, colour_seconds, webp_bytes, webp_seconds, jxl_bytes, jxl_seconds, strip_bytes, strip_seconds) = row;
            bc7 += b;
            mmpx_unmapped += unmapped;
            raw_rgba += raw;
            for (tally, (bytes, seconds)) in mmpx.iter_mut().zip(mmpx_rows) {
                tally.add(bytes, mmpx_output, seconds);
            }
            for (tally, (bytes, seconds)) in xbrz.iter_mut().zip(xbrz_rows) {
                tally.add(bytes, xbrz_output, seconds);
            }
            colour.add(colour_bytes, raw, colour_seconds);
            webp_frame.add(webp_bytes, raw, webp_seconds);
            webp_strip.add(strip_bytes, raw, strip_seconds);
            jxl_frame.add(jxl_bytes, raw, jxl_seconds);
        }
        println!("\n#### {} upscale sample: {} bodies, {:.0} Mpx at {UPSCALE}x; xBRZ BC7 + zstd 7 {:.1} MB; MMPX pixels without a source colour {mmpx_unmapped}\n", client.label(), sample.len(), raw_rgba as f64 / RGBA_CHANNELS as f64 / PIXELS_PER_MEGAPIXEL, mb(bc7));
        println!("| content and coder | MB | x BC7 at {UPSCALE}x | decode MB/s, one thread | lossless |");
        println!("|---|---:|---:|---:|---|");
        let row = |name: String, tally: &UpscaleTally, lossless: &str| {
            let speed = if tally.seconds > 0.0 { format!("{:.0}", mb(tally.output) / tally.seconds) } else { "-".to_string() };
            println!("| {name} | {:.1} | {:.3} | {speed} | {lossless} |", mb(tally.bytes), tally.bytes as f64 / bc7.max(1) as f64);
        };
        for (candidate, tally) in UPSCALE_CANDIDATES.iter().zip(&mmpx) {
            row(format!("MMPX indexed, {}", candidate.name), tally, "yes (of the upscale)");
        }
        for (candidate, tally) in UPSCALE_CANDIDATES.iter().zip(&xbrz) {
            row(format!("xBRZ requantized, {}", candidate.name), tally, "no (quantized)");
        }
        row("xBRZ RGBA8, YCoCg-R + alpha, MED, rANS".to_string(), &colour, "yes");
        row("xBRZ RGBA8, WebP lossless m6, per frame".to_string(), &webp_frame, "yes");
        row("xBRZ RGBA8, WebP lossless m6, per direction strip".to_string(), &webp_strip, "yes");
        row("xBRZ RGBA8, JPEG XL lossless effort 7, per frame".to_string(), &jxl_frame, "yes");
    }
    part_f13(sources, bodies, present);
}

fn rgba_strips(frames: &[(u32, u32, Vec<u8>)]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut strips = Vec::new();
    let mut open: Vec<&(u32, u32, Vec<u8>)> = Vec::new();
    let mut flush = |open: &mut Vec<&(u32, u32, Vec<u8>)>| {
        if open.is_empty() {
            return;
        }
        let w = open.iter().map(|f| f.0).max().unwrap();
        let h: u32 = open.iter().map(|f| f.1).sum();
        let mut data = vec![0u8; (w * h) as usize * RGBA_CHANNELS];
        let mut y0 = 0usize;
        for &&(fw, fh, ref pixels) in open.iter() {
            let row = fw as usize * RGBA_CHANNELS;
            for y in 0..fh as usize {
                data[(y0 + y) * w as usize * RGBA_CHANNELS..][..row].copy_from_slice(&pixels[y * row..][..row]);
            }
            y0 += fh as usize;
        }
        strips.push((w, h, data));
        open.clear();
    };
    let mut height = 0usize;
    for frame in frames {
        if height + frame.1 as usize > WEBP_MAX_SIDE {
            flush(&mut open);
            height = 0;
        }
        open.push(frame);
        height += frame.1 as usize;
    }
    flush(&mut open);
    strips
}

/// Upscaling at load time instead of storing: each filter's speed on one thread, and what it adds per action.
fn part_f13(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)]) {
    const FILTERS: [(&str, Upscaler); 4] = [("MMPX", apply_mmpx), ("EPX (Scale2x)", apply_epx), ("xBRZ", apply_xbrz), ("hqx", apply_hqx)];
    println!("\n### Upscaling when an action is loaded, at {UPSCALE}x, one thread; CPU {}; load average {}\n", cpu_state(), load_average());
    println!("| client | filter | Mpx/s of input | action median / p95 / max ms | whole body max ms |");
    println!("|---|---|---:|---|---:|");
    let state = cpu_state();
    let mut lines = Vec::new();
    for client in [Client::Cc, Client::Ec] {
        let sample: Vec<Body> = sample_of(present, client, UPSCALE_SAMPLE_PER_CLIENT).par_iter().map(|&i| load_body(sources, &bodies[i])).collect();
        for (name, filter) in FILTERS {
            let (mut pixels, mut seconds) = (0u64, 0f64);
            let mut actions = Vec::new();
            let mut whole_max = 0f64;
            for body in &sample {
                let mut per_action = BTreeMap::<u16, f64>::new();
                for frame in body.frames() {
                    let started = Instant::now();
                    black_box(filter(frame.width, frame.height, &frame.rgba, UPSCALE));
                    let elapsed = started.elapsed().as_secs_f64();
                    *per_action.entry(frame.action).or_default() += elapsed;
                    seconds += elapsed;
                    pixels += frame.pixel_count() as u64;
                }
                whole_max = whole_max.max(per_action.values().sum());
                actions.extend(per_action.into_values());
            }
            let actions = sorted(actions);
            lines.push(format!(
                "| {} | {name} | {:.1} | {:.1} / {:.1} / {:.1} | {:.0} |",
                client.label(),
                pixels as f64 / PIXELS_PER_MEGAPIXEL / seconds.max(f64::MIN_POSITIVE),
                ms(percentile(&actions, MEDIAN)),
                ms(percentile(&actions, P95)),
                ms(*actions.last().unwrap_or(&0.0)),
                ms(whole_max),
            ));
        }
    }
    for line in &lines {
        println!("{line}");
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during the filter timing ({state} at the start, {end} at the end): discard the table above");
    }
    println!("Load average after the filter timing {}", load_average());
}

// ---------------------------------------------------------------------------------------------------------------
// Part g: the largest bodies

fn body_kinds(cc_dir: &Path) -> HashMap<u32, String> {
    let mut kinds = HashMap::new();
    let Some(path) = find_ignoring_case(cc_dir, MOBTYPES_FILE) else { return kinds };
    let Ok(text) = std::fs::read(&path) else { return kinds };
    for line in String::from_utf8_lossy(&text).lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let (fields, comment) = line.split_once('#').unwrap_or((line, ""));
        let mut fields = fields.split_whitespace();
        if let (Some(id), Some(kind)) = (fields.next().and_then(|id| id.parse().ok()), fields.next()) {
            let comment = comment.trim();
            kinds.insert(id, if comment.is_empty() { kind.to_string() } else { format!("{kind}: {comment}") });
        }
    }
    kinds
}

const LARGEST_CANDIDATES: [(Candidate, Candidate, Candidate); 2] = [
    (
        candidate("zstd 19", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("zstd 19", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Action),
        candidate("zstd 19", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::ActionDirection),
    ),
    (
        candidate("rank + previous frame context, rANS", Order::Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Body),
        candidate("rank + previous frame context, rANS", Order::Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Action),
        candidate("rank + previous frame context, rANS", Order::Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::ActionDirection),
    ),
];

fn part_g(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let kinds = body_kinds(cc_dir);
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let workers = rayon::ThreadPoolBuilder::new().num_threads(WORKER_THREADS).build().expect("worker pool");
    let state = cpu_state();
    println!("\n### The {LARGEST_BODIES} largest bodies of each client; CPU {state}; load average {}", load_average());
    for client in [Client::Cc, Client::Ec] {
        let mut of: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client).collect();
        of.sort_by_key(|s| std::cmp::Reverse(s.1));
        of.truncate(LARGEST_BODIES);
        let planes: Vec<Planes> = of.par_iter().map(|s| load_planes(sources, &bodies[s.2], false)).collect();
        for (body_candidate, action_candidate, direction_candidate) in &LARGEST_CANDIDATES {
            println!("\n#### {}, {}\n", client.label(), body_candidate.name);
            println!("| body | kind | Mpx | frames | actions | resident MB | MB body / action / action+direction | whole body ms | 4 threads, action streams ms | largest action ms | largest action+direction ms |");
            println!("|---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|");
            for planes in &planes {
                let ordered = reordered(planes, body_candidate.order);
                let encoded: Vec<Encoded> = [body_candidate, action_candidate, direction_candidate]
                    .par_iter()
                    .map(|&candidate| {
                        let encoded = encode_candidate(&ordered, candidate);
                        verify_round_trip(&ordered, &encoded, candidate);
                        encoded
                    })
                    .collect();
                let boxes = &ordered.boxes;
                let whole = single.install(|| {
                    fastest(|| {
                        let shared = shared_models(&encoded[0], body_candidate, client);
                        encoded[0].units.iter().map(|u| decode_unit(u, boxes, &ordered.palettes, body_candidate, client, &shared).0.len()).sum::<usize>()
                    })
                });
                let parallel = workers.install(|| {
                    fastest(|| {
                        let shared = shared_models(&encoded[1], action_candidate, client);
                        encoded[1].units.par_iter().map(|u| decode_unit(u, boxes, &ordered.palettes, action_candidate, client, &shared).0.len()).sum::<usize>()
                    })
                });
                let largest = |encoded: &Encoded, candidate: &Candidate| {
                    let unit = encoded.units.iter().max_by_key(|u| u.frames.iter().map(|&f| boxes[f].pixels()).sum::<usize>()).unwrap();
                    single.install(|| {
                        fastest(|| {
                            let shared = shared_models(encoded, candidate, client);
                            decode_unit(unit, boxes, &ordered.palettes, candidate, client, &shared).0.len()
                        })
                    })
                };
                let number: Option<u32> = planes.label.rsplit(' ').next().and_then(|id| id.parse().ok());
                let kind = number.and_then(|n| kinds.get(&n)).map_or("-", String::as_str);
                let actions: HashSet<u16> = planes.boxes.iter().map(|f| f.action).collect();
                println!(
                    "| {} | {kind} | {:.1} | {} | {} | {:.1} | {:.2} / {:.2} / {:.2} | {:.1} | {:.1} | {:.1} | {:.1} |",
                    planes.label,
                    planes.index.len() as f64 / PIXELS_PER_MEGAPIXEL,
                    planes.boxes.len(),
                    actions.len(),
                    mb(planes.resident_bytes()),
                    mb(encoded[0].total()),
                    mb(encoded[1].total()),
                    mb(encoded[2].total()),
                    ms(whole.as_secs_f64()),
                    ms(parallel.as_secs_f64()),
                    ms(largest(&encoded[1], action_candidate).as_secs_f64()),
                    ms(largest(&encoded[2], direction_candidate).as_secs_f64()),
                );
            }
        }
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part g ({state} at the start, {end} at the end): discard its timings");
    }
    println!("Load average at the end of part g {}", load_average());
}

// ---------------------------------------------------------------------------------------------------------------
// The pass

pub(super) fn run(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let parts: Vec<String> = std::env::var(PARTS_ENV).unwrap_or_else(|_| DEFAULT_PARTS.to_string()).split(',').map(|p| p.trim().to_string()).collect();
    let runs = |part: &str| parts.iter().any(|p| p == part);
    let stride = std::env::var(STRIDE_ENV).ok().map(|n| n.parse::<usize>().expect("stride")).unwrap_or(DEFAULT_STRIDE).max(1);
    let only: Vec<String> = std::env::var(ONLY_ENV).map(|list| list.split(',').map(|p| p.trim().to_string()).collect()).unwrap_or_default();
    let time = std::env::var_os(TIME_ENV).is_some();
    println!("\nShrink pass: parts {parts:?}, every {stride}-th body by pixel count; CPU {}; load average {}", cpu_state(), load_average());

    if runs("a1") {
        part_a1(sources, bodies);
    }
    if runs("a2") {
        part_a2(sources, bodies);
    }
    if runs("a2p") {
        part_a2p(sources, bodies, present);
    }
    if runs("w") {
        part_w(sources, bodies, present, &only);
    }
    let sample_indices: Vec<usize> = [Client::Cc, Client::Ec]
        .into_iter()
        .flat_map(|client| {
            let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
            sample_of(present, client, count)
        })
        .collect();
    if ["b", "c", "c7", "d", "e"].iter().any(|part| runs(part)) {
        let started = Instant::now();
        let measure_bc7 = runs("b");
        let sample: Vec<Planes> = sample_indices.par_iter().map(|&i| load_planes(sources, &bodies[i], measure_bc7)).collect();
        println!("sample of {} bodies loaded in {:.0} s", sample.len(), started.elapsed().as_secs_f64());
        if runs("b") {
            part_b(&sample, &only, time);
        }
        let bound_sample = || -> Vec<Planes> {
            [Client::Cc, Client::Ec]
                .into_iter()
                .flat_map(|client| {
                    let of: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
                    spread(&of, BOUND_SAMPLE_PER_CLIENT, |p| p.index.len() as u64).into_iter().map(|p| (*p).clone()).collect::<Vec<Planes>>()
                })
                .collect()
        };
        if runs("c") {
            part_c(&bound_sample(), Order::Path);
        }
        if runs("c7") {
            part_c7(&bound_sample(), Order::Path);
        }
        if runs("d") {
            part_d(&bound_sample());
        }
        if runs("e") {
            part_e(&bound_sample());
        }
    }
    if runs("f") {
        part_f(sources, bodies, present, time);
    }
    if runs("g") {
        part_g(sources, bodies, present, cc_dir);
    }
    knowledge::run(sources, bodies, present, cc_dir, &parts, stride, time);
    if runs("k8") {
        halved::run(sources, bodies, present, cc_dir, stride);
    }
    if runs("s1") {
        crowd::part_s1(sources, bodies, present, &only);
    }
    if runs("s6") {
        video::part_s6(sources, bodies, present, cc_dir, time);
    }
    if runs("s3") {
        crowd::part_s3(sources, bodies, present, cc_dir, stride);
    }
    if runs("s4") {
        floor::part_s4(sources, bodies, present, cc_dir, stride);
    }
    if runs("s4q") {
        floor::part_s4q(sources, bodies, present);
    }
    if runs("s5") {
        derive::part_s5(sources, bodies);
    }
}
