//! Part `v` of the `shrink` pass: what the GPU holds under each layout, against real atlas packing. Sizes only, so it needs
//! no quiet machine, and nothing here decides the format.
//!
//! Every byte the GPU would hold counts: pixels, per-frame and per-row tables, tile maps and palettes. The whole set, the
//! classes of `classes` and the four crowds come from one pass over every body. The allocators then work on the frame sizes
//! that pass kept: today's packer, a dynamic guillotine (`guillotiere`, which the crate already depends on) and shelf
//! allocator, and churn with LRU eviction.

use super::classes::{classes_of, median_area, BodyRule, Class, CLASSES};
use super::*;
use guillotiere::{size2, AllocId, AtlasAllocator};
use std::cmp::Reverse;
use std::sync::{OnceLock, RwLock};

/// Sides of the dicing tiles.
const TILE_SIDES: [usize; 3] = [8, 16, 32];
const TILE_KINDS: usize = TILE_SIDES.len();
/// One pixel each side of a tile, or of an atlas rectangle.
const GUTTER: i32 = 1;
const GUTTER_SIDES: i32 = 2;
const BLOCK_SIDE: usize = 4;
const BC7_BYTES: u64 = 16;
const BC1_BYTES: u64 = 8;
const BC4_BYTES: u64 = 8;
const PIXELS_PER_BLOCK: u64 = 16;
/// The low-end mix's thresholds on a body's median cropped frame area: a body above one takes a block format.
pub(super) const MIX_TOPS: [u64; 3] = [2_500, 10_000, 40_000];
const MIX_BANDS: usize = MIX_TOPS.len() + 1;
/// ASTC's bigger blocks: 16 bytes whatever the block's side.
const ASTC_BYTES: u64 = 16;
const ASTC_SIDE_MID: usize = 6;
const ASTC_SIDE_BIG: usize = 8;
/// 256 RGBA8 colours: a row of a palette texture, the keep-colour flag in alpha.
const PALETTE_ROW_BYTES: u64 = 1024;
/// A frame's entry in every layout: place or base, size, anchor, page and flags. The layouts differ by what they add to it.
const FRAME_ENTRY_BYTES: u64 = 16;
/// A row entry that fits a byte for first and length and 16 bits for its offset, and the one that always works.
const ROW_ENTRY_NARROW: u64 = 4;
const ROW_ENTRY_WIDE: u64 = 8;
const NARROW_OFFSET_TOP: u64 = 1 << u16::BITS;
const SMALLEST_OFFSET_TOP: u64 = 1 << u8::BITS;
const NARROW_SIDE_TOP: usize = u8::MAX as usize;
const ENTRY_WIDTHS: [u64; 4] = [3, 4, 6, 8];
/// A bitmap word of painted-only: 32 pixels' bits and the painted pixels before them, one RG32UI texel.
const WORD_PIXELS: usize = 32;
const WORD_BYTES: u64 = 8;
/// A row's entry when runs are searched: its first run and its run count.
const RUN_ROW_ENTRY: u64 = 4;
const NARROW_TILE_ID_TOP: u64 = u16::MAX as u64;
const NARROW_TILE_ID: u64 = 2;
const WIDE_TILE_ID: u64 = 4;
const COVERAGE_LEVELS: usize = 17;
const COVERAGE_LEVEL_TOP: u32 = 16;
const FULL_BYTE: u32 = 255;
/// How far from a level's own value the first BC4 endpoint search goes; a block no pair there gets right searches them all.
const BC4_GRID_REACH: i32 = 2;
const BC4_PALETTE: usize = 8;
const BITS_PER_ROW_BYTE: usize = 8;
/// The buckets of frame area for what padding to 4 x 4 costs.
const AREA_TOPS: [u64; 5] = [64, 256, 1024, 4096, 16384];
const AREA_BUCKETS: usize = AREA_TOPS.len() + 1;
/// The crowds, in the order of `crowds_of`.
const CROWD_NAMES: [&str; 4] = ["creatures", "dressed NPCs", "uniform", "worst"];
const CROWD_COUNT: usize = CROWD_NAMES.len();
const CREATURES: usize = 0;
const TOWN: usize = 1;
const UNITS_PER_CROWD: usize = 2;
const FIVE: usize = 0;
const FACED: usize = 1;
const SCOPES: usize = CROWD_COUNT * UNITS_PER_CROWD;

const PAGE_SIDES: [i32; 2] = [2048, 4096];
/// Pages kept open when the whole set goes in: a frame that fits none of them opens another.
const OPEN_PAGES: usize = 8;
/// Today's packer: `DEFAULT_ATLAS_GUTTER`, and its page sizes (`atlas_page_buckets`).
const TODAY_GUTTER: u32 = 4;
const TODAY_BUCKETS: [(u32, u32); 7] = [(512, 512), (1024, 512), (512, 1024), (1024, 1024), (2048, 1024), (1024, 2048), (2048, 2048)];
const TODAY_ALIGN: u32 = 4;
const SHELF_HEIGHT_STEP: i32 = 4;
/// A shelf takes a rectangle up to this share of its height wasted, unless nothing else fits.
const SHELF_WASTE_DIVISOR: i32 = 4;
/// A block's first guess of width over the square root of its area.
const BLOCK_WIDTH_SLACK: f64 = 1.1;
const CHURN_ROUNDS: usize = 8;
/// A cache over the largest crowd's fresh pack: none, and a quarter.
const CHURN_BUDGETS: [f64; 2] = [1.0, 1.25];
const CHURN_SEED_STEP: u64 = 0x9E37_79B9_7F4A_7C15;
const PROGRESS_BODIES: usize = 500;

/// What one frame needs in the atlas.
#[derive(Clone, Copy)]
struct FrameSize {
    w: u16,
    h: u16,
    hue: u8,
}

/// What a group of frames holds under each layout.
#[derive(Clone, Default)]
struct Held {
    frames: u64,
    pixels: u64,
    painted: u64,
    spans: u64,
    rows: u64,
    row_smallest: u64,
    row_aligned: u64,
    words: u64,
    runs: u64,
    blocks: u64,
    blocks_some_transparent: u64,
    blocks_empty: u64,
    hue_frames: [u64; 3],
    masked_pixels: u64,
    masked_blocks: u64,
    masked_bits: u64,
    tiles: [u64; TILE_KINDS],
    tile_entries: [u64; TILE_KINDS],
    tile_map_plain: [u64; TILE_KINDS],
    tiles_in_stream: [u64; TILE_KINDS],
    tile_map_stream: [u64; TILE_KINDS],
    entry_widths: [u64; ENTRY_WIDTHS.len()],
    area_frames: [u64; AREA_BUCKETS],
    area_pixels: [u64; AREA_BUCKETS],
    area_blocks: [u64; AREA_BUCKETS],
    area_blocks_mid: [u64; AREA_BUCKETS],
    area_blocks_big: [u64; AREA_BUCKETS],
}

impl Held {
    fn add(&mut self, other: &Held) {
        macro_rules! sum {
            ($($field:ident),*) => { $(self.$field += other.$field;)* };
        }
        macro_rules! sum_each {
            ($($field:ident),*) => { $(self.$field.iter_mut().zip(&other.$field).for_each(|(a, b)| *a += *b);)* };
        }
        sum!(frames, pixels, painted, spans, rows, row_smallest, row_aligned, words, runs, blocks, blocks_some_transparent, blocks_empty, masked_pixels, masked_blocks, masked_bits);
        sum_each!(hue_frames, tiles, tile_entries, tile_map_plain, tiles_in_stream, tile_map_stream, entry_widths, area_frames, area_pixels, area_blocks, area_blocks_mid, area_blocks_big);
    }
}

struct StreamHeld {
    action: u16,
    direction: u32,
    palette: u64,
    held: Held,
    sizes: Vec<FrameSize>,
    /// Where its first frame is in the client's frames, as today's packer sorts them; set when the body is flattened.
    position: usize,
}

/// One body's share of a crowd's load unit.
struct Wanted {
    scope: usize,
    streams: Vec<usize>,
    tiles: [u64; TILE_KINDS],
}

#[derive(Clone, Copy, Default)]
struct Bc4Errors {
    blocks: u64,
    inexact_blocks: u64,
    pixels: u64,
    changed: u64,
    level_sum: u64,
    level_worst: u64,
    byte_sum: u64,
    byte_worst: u64,
    flips: u64,
}

impl Bc4Errors {
    fn add(&mut self, other: &Bc4Errors) {
        self.blocks += other.blocks;
        self.inexact_blocks += other.inexact_blocks;
        self.pixels += other.pixels;
        self.changed += other.changed;
        self.level_sum += other.level_sum;
        self.level_worst = self.level_worst.max(other.level_worst);
        self.byte_sum += other.byte_sum;
        self.byte_worst = self.byte_worst.max(other.byte_worst);
        self.flips += other.flips;
    }
}

struct BodyHeld {
    body: usize,
    class: Class,
    median: u64,
    streams: Vec<StreamHeld>,
    tiles: [u64; TILE_KINDS],
    wanted: Vec<Wanted>,
    /// A crowd and the stream one of its NPCs faces, once per NPC.
    on_screen: Vec<(usize, usize)>,
    bc4: Bc4Errors,
    most_runs: u64,
}

/// What a scope of streams adds up to.
#[derive(Clone, Default)]
struct Scope {
    held: Held,
    streams: u64,
    palettes: HashSet<u64>,
    body_tiles: [u64; TILE_KINDS],
    body_map: [u64; TILE_KINDS],
}

fn tile_map_bytes(entries: u64, ids: u64) -> u64 {
    entries * if ids <= NARROW_TILE_ID_TOP { NARROW_TILE_ID } else { WIDE_TILE_ID }
}

impl Scope {
    fn add_stream(&mut self, stream: &StreamHeld) {
        self.held.add(&stream.held);
        self.streams += 1;
        self.palettes.insert(stream.palette);
    }

    /// One body's dedupe: its distinct tiles, through a map as wide as those need.
    fn add_body_tiles(&mut self, tiles: &[u64; TILE_KINDS], entries: &[u64; TILE_KINDS]) {
        for t in 0..TILE_KINDS {
            self.body_tiles[t] += tiles[t];
            self.body_map[t] += tile_map_bytes(entries[t], tiles[t]);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// BC4 on the coverage levels

/// The best a BC4 block can do for a set of levels: what each level decodes to, and whether all round back to themselves.
#[derive(Clone, Copy)]
struct Bc4Fit {
    decoded: [u8; COVERAGE_LEVELS],
    exact: bool,
}

fn level_byte(level: usize) -> u32 {
    (level as u32 * FULL_BYTE + COVERAGE_LEVEL_TOP / 2) / COVERAGE_LEVEL_TOP
}

fn byte_level(value: u32) -> usize {
    ((value * COVERAGE_LEVEL_TOP + FULL_BYTE / 2) / FULL_BYTE) as usize
}

fn bc4_palette(e0: u32, e1: u32) -> [u32; BC4_PALETTE] {
    if e0 > e1 {
        [e0, e1, (6 * e0 + e1 + 3) / 7, (5 * e0 + 2 * e1 + 3) / 7, (4 * e0 + 3 * e1 + 3) / 7, (3 * e0 + 4 * e1 + 3) / 7, (2 * e0 + 5 * e1 + 3) / 7, (e0 + 6 * e1 + 3) / 7]
    } else {
        [e0, e1, (4 * e0 + e1 + 2) / 5, (3 * e0 + 2 * e1 + 2) / 5, (2 * e0 + 3 * e1 + 2) / 5, (e0 + 4 * e1 + 2) / 5, 0, FULL_BYTE]
    }
}

/// (levels that don't round back, squared error) of a palette on a set of levels, and what each decodes to.
fn bc4_try(levels: &[usize], palette: &[u32; BC4_PALETTE], decoded: &mut [u8; COVERAGE_LEVELS]) -> (u32, u64) {
    let (mut wrong, mut error) = (0u32, 0u64);
    for &level in levels {
        let target = level_byte(level) as i64;
        let value = palette.iter().copied().min_by_key(|&v| (v as i64 - target).abs()).expect("a palette");
        decoded[level] = value as u8;
        wrong += u32::from(byte_level(value) != level);
        error += ((value as i64 - target) * (value as i64 - target)) as u64;
    }
    (wrong, error)
}

fn bc4_search(levels: &[usize], candidates: &[u32], best: &mut (u32, u64, Bc4Fit)) {
    let mut decoded = [0u8; COVERAGE_LEVELS];
    for &e0 in candidates {
        for &e1 in candidates {
            let palette = bc4_palette(e0, e1);
            let (wrong, error) = bc4_try(levels, &palette, &mut decoded);
            if (wrong, error) < (best.0, best.1) {
                *best = (wrong, error, Bc4Fit { decoded, exact: wrong == 0 });
            }
        }
    }
}

fn bc4_fit_of(mask: u32) -> Bc4Fit {
    static FITS: OnceLock<RwLock<HashMap<u32, Bc4Fit>>> = OnceLock::new();
    let fits = FITS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(fit) = fits.read().expect("the BC4 fits").get(&mask) {
        return *fit;
    }
    let levels: Vec<usize> = (0..COVERAGE_LEVELS).filter(|&l| mask & (1 << l) != 0).collect();
    let mut grid: Vec<u32> = (0..COVERAGE_LEVELS)
        .flat_map(|l| (-BC4_GRID_REACH..=BC4_GRID_REACH).map(move |d| (level_byte(l) as i32 + d).clamp(0, FULL_BYTE as i32) as u32))
        .collect();
    grid.sort_unstable();
    grid.dedup();
    let mut best = (u32::MAX, u64::MAX, Bc4Fit { decoded: [0; COVERAGE_LEVELS], exact: false });
    bc4_search(&levels, &grid, &mut best);
    if !best.2.exact {
        let all: Vec<u32> = (0..=FULL_BYTE).collect();
        bc4_search(&levels, &all, &mut best);
    }
    fits.write().expect("the BC4 fits").insert(mask, best.2);
    best.2
}

// ---------------------------------------------------------------------------------------------------------------
// One pass over a body

struct Reading<'a> {
    planes: &'a Planes,
    /// Enhanced's coverage, Classic's index: not 0 is painted.
    mask: &'a [u8],
    hueable: Vec<[bool; PALETTE_COLOURS]>,
}

/// Which pixels take the hue: Enhanced's keep-colour flag; on Classic the grey colours, as a partial hue goes by them.
pub(super) fn hueable_of(planes: &Planes) -> Vec<[bool; PALETTE_COLOURS]> {
    planes
        .palettes
        .iter()
        .map(|palette| {
            let mut out = [false; PALETTE_COLOURS];
            for (n, colour) in palette.iter().enumerate().take(PALETTE_COLOURS) {
                out[n] = match planes.client {
                    Client::Ec => colour[PALETTE_KEEP_COLOUR_BYTE] == 0,
                    Client::Cc => colour[0] == colour[1] && colour[1] == colour[2],
                };
            }
            out
        })
        .collect()
}

fn hash_of_palette(palette: &[[u8; EC_PALETTE_ENTRY_BYTES]]) -> u64 {
    let mut hasher = DefaultHasher::new();
    palette.hash(&mut hasher);
    hasher.finish()
}

fn smallest_entry(w: usize, total: u64) -> u64 {
    let side = if w <= NARROW_SIDE_TOP { 1 } else { 2 };
    let offset = if total <= SMALLEST_OFFSET_TOP {
        1
    } else if total <= NARROW_OFFSET_TOP {
        2
    } else {
        4
    };
    2 * side + offset
}

fn aligned_entry(w: usize, total: u64) -> u64 {
    if w <= NARROW_SIDE_TOP && total <= NARROW_OFFSET_TOP { ROW_ENTRY_NARROW } else { ROW_ENTRY_WIDE }
}

fn count_rows(reading: &Reading, frame: &FrameBox, held: &mut Held, most_runs: &mut u64) {
    let (w, h) = (frame.w, frame.h);
    let (mut spans, mut painted, mut words, mut runs) = (0u64, 0u64, 0u64, 0u64);
    for y in 0..h {
        let row = &reading.mask[frame.start + y * w..frame.start + (y + 1) * w];
        let Some(first) = row.iter().position(|&c| c != 0) else { continue };
        let last = row.iter().rposition(|&c| c != 0).expect("a painted pixel");
        let length = last - first + 1;
        spans += length as u64;
        words += length.div_ceil(WORD_PIXELS) as u64;
        let mut row_runs = 0u64;
        let mut inside = false;
        for &c in &row[first..=last] {
            let is_painted = c != 0;
            painted += u64::from(is_painted);
            row_runs += u64::from(is_painted && !inside);
            inside = is_painted;
        }
        runs += row_runs;
        *most_runs = (*most_runs).max(row_runs);
    }
    let smallest = smallest_entry(w, spans);
    held.spans += spans;
    held.painted += painted;
    held.words += words;
    held.runs += runs;
    held.rows += h as u64;
    held.row_smallest += h as u64 * smallest;
    held.row_aligned += h as u64 * aligned_entry(w, spans);
    held.entry_widths[ENTRY_WIDTHS.iter().position(|&width| width == smallest).expect("an entry width")] += 1;
}

fn count_blocks(reading: &Reading, frame: &FrameBox, held: &mut Held, bc4: &mut Bc4Errors) -> u64 {
    let (w, h) = (frame.w, frame.h);
    let (columns, rows) = (w.div_ceil(BLOCK_SIDE), h.div_ceil(BLOCK_SIDE));
    let enhanced = reading.planes.client == Client::Ec;
    for by in 0..rows {
        for bx in 0..columns {
            let (mut some_transparent, mut some_painted) = (false, false);
            let mut levels = 0u32;
            for y in by * BLOCK_SIDE..((by + 1) * BLOCK_SIDE).min(h) {
                for x in bx * BLOCK_SIDE..((bx + 1) * BLOCK_SIDE).min(w) {
                    let value = reading.mask[frame.start + y * w + x];
                    some_transparent |= value == 0;
                    some_painted |= value != 0;
                    if enhanced {
                        levels |= 1 << value;
                    }
                }
            }
            held.blocks_some_transparent += u64::from(some_transparent);
            held.blocks_empty += u64::from(!some_painted);
            if enhanced {
                bc4.blocks += 1;
                // Two levels are the endpoints, so only more can go wrong.
                if levels.count_ones() > 2 {
                    let fit = bc4_fit_of(levels);
                    bc4.inexact_blocks += u64::from(!fit.exact);
                    for y in by * BLOCK_SIDE..((by + 1) * BLOCK_SIDE).min(h) {
                        for x in bx * BLOCK_SIDE..((bx + 1) * BLOCK_SIDE).min(w) {
                            let level = usize::from(reading.mask[frame.start + y * w + x]);
                            let decoded = u32::from(fit.decoded[level]);
                            let back = byte_level(decoded);
                            let (off, byte_off) = (back.abs_diff(level) as u64, decoded.abs_diff(level_byte(level)) as u64);
                            bc4.changed += u64::from(off != 0);
                            bc4.level_sum += off;
                            bc4.level_worst = bc4.level_worst.max(off);
                            bc4.byte_sum += byte_off;
                            bc4.byte_worst = bc4.byte_worst.max(byte_off);
                            bc4.flips += u64::from((back == 0) != (level == 0));
                        }
                    }
                }
                bc4.pixels += ((((by + 1) * BLOCK_SIDE).min(h) - by * BLOCK_SIDE) * (((bx + 1) * BLOCK_SIDE).min(w) - bx * BLOCK_SIDE)) as u64;
            }
        }
    }
    (columns * rows) as u64
}

/// NONE, ALL or MASKED among a frame's painted pixels, as the packer's `frame_window_hue_mode` decides.
pub(super) fn hue_mode_of(planes: &Planes, frame: &FrameBox, mask: &[u8], hueable: &[[bool; PALETTE_COLOURS]]) -> u8 {
    let takes = &hueable[frame.palette];
    let (mut some_hue, mut some_kept) = (false, false);
    for i in frame.range() {
        if mask[i] != 0 {
            let hue = takes[usize::from(planes.index[i])];
            some_hue |= hue;
            some_kept |= !hue;
            if some_hue && some_kept {
                break;
            }
        }
    }
    match (some_hue, some_kept) {
        (true, true) => HUE_MASKED,
        (true, false) => HUE_ALL,
        _ => HUE_NONE,
    }
}

fn count_hue(reading: &Reading, frame: &FrameBox, blocks: u64, held: &mut Held) -> u8 {
    let mode = hue_mode_of(reading.planes, frame, reading.mask, &reading.hueable);
    held.hue_frames[usize::from(mode)] += 1;
    if mode == HUE_MASKED {
        held.masked_pixels += frame.pixels() as u64;
        held.masked_blocks += blocks;
        held.masked_bits += (frame.w.div_ceil(BITS_PER_ROW_BYTE) * frame.h) as u64;
    }
    mode
}

/// The tile hashes of a frame; a clipped tile is padded with transparent pixels, so it can match a whole one.
fn count_tiles(reading: &Reading, frame: &FrameBox, held: &mut Held, hashes: &mut [Vec<u64>; TILE_KINDS], buffer: &mut Vec<u8>) {
    let (w, h) = (frame.w, frame.h);
    let planes = reading.planes;
    let bytes = planes.client.pixel_bytes();
    for (t, &side) in TILE_SIDES.iter().enumerate() {
        let (columns, rows) = (w.div_ceil(side), h.div_ceil(side));
        held.tile_entries[t] += (columns * rows) as u64;
        for ty in 0..rows {
            for tx in 0..columns {
                buffer.clear();
                buffer.resize(side * side * bytes, 0);
                let mut any = false;
                for y in ty * side..((ty + 1) * side).min(h) {
                    let start = frame.start + y * w + tx * side;
                    let length = side.min(w - tx * side);
                    any |= reading.mask[start..start + length].iter().any(|&c| c != 0);
                    let at = (y - ty * side) * side;
                    buffer[at..at + length].copy_from_slice(&planes.index[start..start + length]);
                    if bytes == EC_PIXEL_BYTES {
                        let second = side * side + at;
                        buffer[second..second + length].copy_from_slice(&planes.coverage[start..start + length]);
                    }
                }
                if any {
                    let mut hasher = DefaultHasher::new();
                    hasher.write(buffer);
                    hashes[t].push(hasher.finish());
                }
            }
        }
    }
}

const HUE_NONE: u8 = 0;
const HUE_ALL: u8 = 1;
pub(super) const HUE_MASKED: u8 = 2;

fn area_bucket(pixels: u64) -> usize {
    AREA_TOPS.iter().position(|&top| pixels <= top).unwrap_or(AREA_BUCKETS - 1)
}

/// A place in one crowd: which crowd, the NPC's position in it, and its action.
#[derive(Clone, Copy)]
struct Occurrence {
    crowd: usize,
    position: usize,
    action: u16,
}

fn count_body(planes: &Planes, body: usize, class: Class, occurrences: &[Occurrence]) -> BodyHeld {
    let mask: &[u8] = match planes.client {
        Client::Cc => &planes.index,
        Client::Ec => &planes.coverage,
    };
    let reading = Reading { planes, mask, hueable: hueable_of(planes) };
    let palette_hashes: Vec<u64> = planes.palettes.iter().map(|p| hash_of_palette(p)).collect();
    let mut groups = BTreeMap::<(u16, u32), Vec<usize>>::new();
    planes.boxes.iter().enumerate().for_each(|(n, f)| groups.entry((f.action, f.direction)).or_default().push(n));

    let mut bc4 = Bc4Errors::default();
    let mut most_runs = 0u64;
    let mut buffer = Vec::new();
    let mut streams = Vec::with_capacity(groups.len());
    let mut all_hashes: Vec<[Vec<u64>; TILE_KINDS]> = Vec::with_capacity(groups.len());
    for (&(action, direction), frames) in &groups {
        let mut held = Held::default();
        let mut sizes = Vec::with_capacity(frames.len());
        let mut hashes: [Vec<u64>; TILE_KINDS] = Default::default();
        for &n in frames {
            let frame = &planes.boxes[n];
            let pixels = frame.pixels() as u64;
            held.frames += 1;
            held.pixels += pixels;
            count_rows(&reading, frame, &mut held, &mut most_runs);
            let blocks = count_blocks(&reading, frame, &mut held, &mut bc4);
            held.blocks += blocks;
            let bucket = area_bucket(pixels);
            held.area_frames[bucket] += 1;
            held.area_pixels[bucket] += pixels;
            held.area_blocks[bucket] += blocks;
            held.area_blocks_mid[bucket] += (frame.w.div_ceil(ASTC_SIDE_MID) * frame.h.div_ceil(ASTC_SIDE_MID)) as u64;
            held.area_blocks_big[bucket] += (frame.w.div_ceil(ASTC_SIDE_BIG) * frame.h.div_ceil(ASTC_SIDE_BIG)) as u64;
            let hue = count_hue(&reading, frame, blocks, &mut held);
            count_tiles(&reading, frame, &mut held, &mut hashes, &mut buffer);
            sizes.push(FrameSize { w: frame.w as u16, h: frame.h as u16, hue });
        }
        for t in 0..TILE_KINDS {
            let tiles = hashes[t].len() as u64;
            let distinct = hashes[t].iter().copied().collect::<HashSet<u64>>().len() as u64;
            held.tiles[t] = tiles;
            held.tile_map_plain[t] = tile_map_bytes(held.tile_entries[t], tiles);
            held.tiles_in_stream[t] = distinct;
            held.tile_map_stream[t] = tile_map_bytes(held.tile_entries[t], distinct);
        }
        let palette = palette_hashes[planes.boxes[frames[0]].palette];
        streams.push(StreamHeld { action, direction, palette, held, sizes, position: 0 });
        all_hashes.push(hashes);
    }

    let mut tiles = [0u64; TILE_KINDS];
    for (t, count) in tiles.iter_mut().enumerate() {
        *count = all_hashes.iter().flat_map(|h| h[t].iter().copied()).collect::<HashSet<u64>>().len() as u64;
    }

    // The crowds' shares of this body.
    let mut sets = BTreeMap::<usize, BTreeSet<usize>>::new();
    let mut on_screen = Vec::new();
    for occurrence in occurrences {
        let of_action: Vec<usize> = streams.iter().enumerate().filter(|(_, s)| s.action == occurrence.action).map(|(n, _)| n).collect();
        if of_action.is_empty() {
            continue;
        }
        let faced = crowd::facing_unit(occurrence.position, &of_action);
        sets.entry(occurrence.crowd * UNITS_PER_CROWD + FIVE).or_default().extend(of_action.iter().copied());
        sets.entry(occurrence.crowd * UNITS_PER_CROWD + FACED).or_default().insert(faced);
        on_screen.push((occurrence.crowd, faced));
    }
    let wanted = sets
        .into_iter()
        .map(|(scope, set)| {
            let mut distinct = [0u64; TILE_KINDS];
            for (t, count) in distinct.iter_mut().enumerate() {
                *count = set.iter().flat_map(|&s| all_hashes[s][t].iter().copied()).collect::<HashSet<u64>>().len() as u64;
            }
            Wanted { scope, streams: set.into_iter().collect(), tiles: distinct }
        })
        .collect();
    BodyHeld { body, class, median: median_area(planes), streams, tiles, wanted, on_screen, bc4, most_runs }
}

// ---------------------------------------------------------------------------------------------------------------
// The layouts

struct Layout {
    name: String,
    pixels: u64,
    tables: u64,
    palettes: u64,
    fetches: &'static str,
    sampler: &'static str,
}

impl Layout {
    fn total(&self) -> u64 {
        self.pixels + self.tables + self.palettes
    }
}

const SAMPLER_YES: &str = "yes";
const SAMPLER_INDEX: &str = "nearest only, a palette lookup after it";
const SAMPLER_NO: &str = "no: explicit fetches in the shader";
const SAMPLER_TILES: &str = "tiles only, after a tile map fetch; a gutter is what lets them filter";

fn layouts(client: Client, scope: &Scope) -> Vec<Layout> {
    let held = &scope.held;
    let bytes = client.pixel_bytes() as u64;
    let frames = held.frames * FRAME_ENTRY_BYTES;
    let palettes = scope.palettes.len() as u64 * PALETTE_ROW_BYTES;
    let mut out = vec![
        Layout { name: format!("rectangle, {}", if client == Client::Cc { "R8" } else { "RG8" }), pixels: held.pixels * bytes, tables: frames, palettes, fetches: "1, then the palette", sampler: SAMPLER_INDEX },
        Layout { name: "BC7 rectangle, padded to 4 x 4 (lossy, no hue by index)".to_string(), pixels: held.blocks * BC7_BYTES, tables: frames, palettes: 0, fetches: "1", sampler: SAMPLER_YES },
        Layout { name: "BC1 rectangle, padded to 4 x 4 (lossy, one-bit alpha)".to_string(), pixels: held.blocks * BC1_BYTES, tables: frames, palettes: 0, fetches: "1", sampler: SAMPLER_YES },
    ];
    if client == Client::Ec {
        out.push(Layout { name: "R8 index + BC4 coverage".to_string(), pixels: held.pixels + held.blocks * BC4_BYTES, tables: frames, palettes, fetches: "2, then the palette", sampler: SAMPLER_INDEX });
    }
    out.push(Layout { name: "row spans, linear memory".to_string(), pixels: held.spans * bytes, tables: frames + held.row_aligned, palettes, fetches: "2, then the palette", sampler: SAMPLER_NO });
    out.push(Layout { name: "painted only, row entry + 32-pixel word + pixel".to_string(), pixels: held.painted * bytes, tables: frames + held.row_aligned + held.words * WORD_BYTES, palettes, fetches: "3, then the palette", sampler: SAMPLER_NO });
    out.push(Layout { name: "painted only, runs searched in the row".to_string(), pixels: held.painted * bytes, tables: frames + held.rows * RUN_ROW_ENTRY + held.runs * ROW_ENTRY_NARROW, palettes, fetches: "row + log2(runs) + pixel, then the palette", sampler: SAMPLER_NO });
    for (t, &side) in TILE_SIDES.iter().enumerate() {
        let plain = (held.tiles[t], held.tile_map_plain[t], "no dedupe");
        let stream = (held.tiles_in_stream[t], held.tile_map_stream[t], "dedupe in a stream");
        let body = (scope.body_tiles[t], scope.body_map[t], "dedupe in a body");
        for (tiles, map, how) in [plain, stream, body] {
            for gutter in [0, GUTTER_SIDES as u64] {
                let tile_side = side as u64 + gutter;
                out.push(Layout {
                    name: format!("dice {side} x {side}, {how}, {}", if gutter == 0 { "no gutter" } else { "1 px gutter" }),
                    pixels: tiles * tile_side * tile_side * bytes,
                    tables: frames + map,
                    palettes,
                    fetches: "tile map + tile, then the palette",
                    sampler: SAMPLER_TILES,
                });
            }
        }
    }
    out
}

fn ratio(value: u64, base: u64) -> f64 {
    value as f64 / base.max(1) as f64
}

const RECTANGLE_ROW: usize = 0;
const BC7_ROW: usize = 1;

fn print_layouts(client: Client, whole: &Scope) {
    println!("\n#### {}: what the GPU holds, the whole set\n\n| layout | pixels MB | tables MB | palettes MB | total MB | x rectangle | x BC7 | fetches | hardware sampler |\n|---|---:|---:|---:|---:|---:|---:|---|---|", client.label());
    let rows = layouts(client, whole);
    let (rectangle, bc7) = (rows[RECTANGLE_ROW].total(), rows[BC7_ROW].total());
    for row in &rows {
        println!("| {} | {:.1} | {:.1} | {:.2} | {:.1} | {:.3} | {:.3} | {} | {} |", row.name, mb(row.pixels), mb(row.tables), mb(row.palettes), mb(row.total()), ratio(row.total(), rectangle), ratio(row.total(), bc7), row.fetches, row.sampler);
    }
}

fn print_scopes(title: &str, client: Client, scopes: &[(String, &Scope)]) {
    let names: Vec<&str> = scopes.iter().map(|s| s.0.as_str()).collect();
    println!("\n#### {}: {title}: total MB (x rectangle)\n\n| layout | {} |\n|---|{}", client.label(), names.join(" | "), "---:|".repeat(names.len()));
    let tables: Vec<Vec<Layout>> = scopes.iter().map(|s| layouts(client, s.1)).collect();
    for r in 0..tables[0].len() {
        let cells: Vec<String> = tables.iter().map(|rows| format!("{:.1} ({:.3})", mb(rows[r].total()), ratio(rows[r].total(), rows[RECTANGLE_ROW].total()))).collect();
        println!("| {} | {} |", tables[0][r].name, cells.join(" | "));
    }
}

fn print_blocks(client: Client, scopes: &[(String, &Scope)]) {
    println!("\n#### {}: 4 x 4 blocks: those with any transparent pixel drop BC1 to three colours\n\n| scope | blocks M | with a transparent pixel | wholly transparent | among blocks with paint |\n|---|---:|---:|---:|---:|", client.label());
    for (name, scope) in scopes {
        let held = &scope.held;
        let some = held.blocks_some_transparent as f64;
        let empty = held.blocks_empty as f64;
        let share = |value: f64, of: f64| value * PERCENT / of.max(1.0);
        println!("| {name} | {:.1} | {:.1} % | {:.1} % | {:.1} % |", held.blocks as f64 / PIXELS_PER_MEGAPIXEL, share(some, held.blocks as f64), share(empty, held.blocks as f64), share(some - empty, held.blocks as f64 - empty));
    }
}

/// What rounding a frame up to whole blocks costs, by frame area: BC1, BC7 and ASTC 4 x 4 round to 4, ASTC to 6 or 8.
fn print_padding(client: Client, scopes: &[(String, &Scope)]) {
    let (mid, big) = (ASTC_SIDE_MID as u64, ASTC_SIDE_BIG as u64);
    println!("\n#### {}: what padding to whole blocks costs, by frame area: x pixels is block pixels over true pixels; bytes a pixel are a frame's own blocks with no gutter, over its true pixels\n\n| scope | frame area up to | frames | pixels Mpx | x at 4 x 4 | x at {mid} x {mid} | x at {big} x {big} | BC1 bytes a pixel | BC7 or ASTC 4 x 4 | ASTC {mid} x {mid} | ASTC {big} x {big} | {mid} x {mid} over BC1 | {big} x {big} over BC1 |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    for (name, scope) in scopes {
        let held = &scope.held;
        let row = |top: &str, frames: u64, pixels: u64, blocks: u64, blocks_mid: u64, blocks_big: u64| {
            println!(
                "| {name} | {top} | {frames} | {:.1} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
                pixels as f64 / PIXELS_PER_MEGAPIXEL,
                ratio(blocks * PIXELS_PER_BLOCK, pixels),
                ratio(blocks_mid * mid * mid, pixels),
                ratio(blocks_big * big * big, pixels),
                ratio(blocks * BC1_BYTES, pixels),
                ratio(blocks * BC7_BYTES, pixels),
                ratio(blocks_mid * ASTC_BYTES, pixels),
                ratio(blocks_big * ASTC_BYTES, pixels),
                ratio(blocks_mid * ASTC_BYTES, blocks * BC1_BYTES),
                ratio(blocks_big * ASTC_BYTES, blocks * BC1_BYTES),
            );
        };
        for n in 0..=AREA_TOPS.len() {
            if held.area_frames[n] > 0 {
                let top = AREA_TOPS.get(n).map_or_else(|| "larger".to_string(), u64::to_string);
                row(&top, held.area_frames[n], held.area_pixels[n], held.area_blocks[n], held.area_blocks_mid[n], held.area_blocks_big[n]);
            }
        }
        row("all", held.area_frames.iter().sum(), held.area_pixels.iter().sum(), held.area_blocks.iter().sum(), held.area_blocks_mid.iter().sum(), held.area_blocks_big.iter().sum());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The low-end mix: a body takes a block format above a threshold on its median frame area, lossless below it

#[derive(Clone, Copy, PartialEq)]
enum MixBlock {
    Bc1,
    Bc7,
    AstcMid,
    AstcBig,
}

impl MixBlock {
    fn name(self) -> &'static str {
        match self {
            MixBlock::Bc1 => "BC1",
            MixBlock::Bc7 => "BC7",
            MixBlock::AstcMid => "ASTC 6 x 6",
            MixBlock::AstcBig => "ASTC 8 x 8",
        }
    }

    fn bytes(self, held: &Held) -> u64 {
        match self {
            MixBlock::Bc1 => held.blocks * BC1_BYTES,
            MixBlock::Bc7 => held.blocks * BC7_BYTES,
            MixBlock::AstcMid => held.area_blocks_mid.iter().sum::<u64>() * ASTC_BYTES,
            MixBlock::AstcBig => held.area_blocks_big.iter().sum::<u64>() * ASTC_BYTES,
        }
    }
}

struct MixLayout {
    name: String,
    /// By band of median area, what a body there takes; none is lossless.
    bands: [Option<MixBlock>; MIX_BANDS],
}

fn mix_band(median: u64) -> usize {
    MIX_TOPS.iter().position(|&top| median <= top).unwrap_or(MIX_TOPS.len())
}

fn mix_layouts(client: Client) -> Vec<MixLayout> {
    let formats: &[MixBlock] = match client {
        Client::Cc => &[MixBlock::Bc1, MixBlock::AstcMid, MixBlock::AstcBig],
        Client::Ec => &[MixBlock::Bc1, MixBlock::Bc7, MixBlock::AstcMid, MixBlock::AstcBig],
    };
    let mut out = vec![MixLayout { name: "all lossless".to_string(), bands: [None; MIX_BANDS] }];
    for &format in formats {
        out.push(MixLayout { name: format!("all {}", format.name()), bands: [Some(format); MIX_BANDS] });
    }
    for (n, top) in MIX_TOPS.iter().enumerate() {
        for &format in formats {
            let mut bands = [None; MIX_BANDS];
            bands[n + 1..].fill(Some(format));
            out.push(MixLayout { name: format!("above {top}: {}", format.name()), bands });
        }
    }
    if client == Client::Ec {
        for (n, top) in MIX_TOPS.iter().enumerate() {
            let mut bands = [Some(MixBlock::Bc7); MIX_BANDS];
            bands[n + 1..].fill(Some(MixBlock::Bc1));
            out.push(MixLayout { name: format!("above {top}: BC1, the rest BC7"), bands });
        }
    }
    out
}

/// A scope's streams by class of body and band of median area.
type MixCells = Vec<[Scope; MIX_BANDS]>;

fn new_mix_cells() -> MixCells {
    (0..CLASSES.len()).map(|_| std::array::from_fn(|_| Scope::default())).collect()
}

fn mix_bytes(client: Client, cells: &MixCells, layout: &MixLayout) -> (u64, u64, u64, u64, u64) {
    let (mut lossless, mut block, mut masks, mut block_pixels, mut pixels) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut palettes = HashSet::<u64>::new();
    for class in cells {
        for (band, scope) in class.iter().enumerate() {
            let held = &scope.held;
            pixels += held.pixels;
            match layout.bands[band] {
                None => {
                    lossless += held.pixels * client.pixel_bytes() as u64 + held.frames * FRAME_ENTRY_BYTES;
                    palettes.extend(scope.palettes.iter().copied());
                }
                Some(format) => {
                    block += format.bytes(held) + held.frames * FRAME_ENTRY_BYTES;
                    masks += held.masked_pixels;
                    block_pixels += held.pixels;
                }
            }
        }
    }
    (lossless + palettes.len() as u64 * PALETTE_ROW_BYTES, block, masks, block_pixels, pixels)
}

fn print_mix_population(client: Client, counted: &[BodyHeld]) {
    let mut bodies = vec![[0u64; MIX_BANDS]; CLASSES.len()];
    let mut pixels = vec![[0u64; MIX_BANDS]; CLASSES.len()];
    for body in counted {
        let band = mix_band(body.median);
        bodies[body.class.at()][band] += 1;
        pixels[body.class.at()][band] += body.streams.iter().map(|s| s.held.pixels).sum::<u64>();
    }
    println!("\n#### {}: the whole set by a body's median cropped frame area: bodies, and Mpx\n\n| class | up to {} | up to {} | up to {} | above |\n|---|---:|---:|---:|---:|", client.label(), MIX_TOPS[0], MIX_TOPS[1], MIX_TOPS[2]);
    for class in CLASSES.iter().filter(|c| bodies[c.at()].iter().sum::<u64>() > 0) {
        let cells: Vec<String> = (0..MIX_BANDS).map(|band| format!("{}, {:.1}", bodies[class.at()][band], pixels[class.at()][band] as f64 / PIXELS_PER_MEGAPIXEL)).collect();
        println!("| {} | {} |", class.label(), cells.join(" | "));
    }
}

fn print_mix(client: Client, scopes: &[(String, &MixCells)]) {
    println!("\n#### {}: the low-end mix: what the GPU holds. A body above the threshold takes the block format, padded to its blocks; the rest stay R8 / RG8 with palettes. Frame entries in both. Masks are R8 over a block-coded body's masked frames; a block format can't hue by index\n\n| scope | layout | lossless bodies MB | block bodies MB | total MB | x all lossless | x all BC7 | block bodies' share of pixels | R8 masks MB | total with masks MB |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    let layouts = mix_layouts(client);
    let all_bc7 = MixLayout { name: String::new(), bands: [Some(MixBlock::Bc7); MIX_BANDS] };
    for (name, cells) in scopes {
        let (base_lossless, _, _, _, _) = mix_bytes(client, cells, &layouts[0]);
        let (_, base_bc7, _, _, _) = mix_bytes(client, cells, &all_bc7);
        for layout in &layouts {
            let (lossless, block, masks, block_pixels, pixels) = mix_bytes(client, cells, layout);
            println!(
                "| {name} | {} | {:.1} | {:.1} | {:.1} | {:.3} | {:.3} | {:.1} % | {:.1} | {:.1} |",
                layout.name,
                mb(lossless),
                mb(block),
                mb(lossless + block),
                ratio(lossless + block, base_lossless),
                ratio(lossless + block, base_bc7),
                block_pixels as f64 * PERCENT / pixels.max(1) as f64,
                mb(masks),
                mb(lossless + block + masks),
            );
        }
    }
}

/// How much of a scope comes from each class, and from the bodies above the largest threshold.
fn print_mix_shares(client: Client, scopes: &[(String, &MixCells)]) {
    println!("\n#### {}: each class's share of what a scope holds: its MB, then its share of the scope\n\n| scope | class | streams | Mpx | R8 / RG8 | BC1 | BC7 | ASTC 6 x 6 | ASTC 8 x 8 |\n|---|---|---:|---:|---:|---:|---:|---:|---:|", client.label());
    let bytes_of = |held: &Held| -> [u64; 5] {
        [held.pixels * client.pixel_bytes() as u64, MixBlock::Bc1.bytes(held), MixBlock::Bc7.bytes(held), MixBlock::AstcMid.bytes(held), MixBlock::AstcBig.bytes(held)]
    };
    for (name, cells) in scopes {
        let mut total = Held::default();
        cells.iter().flatten().for_each(|scope| total.add(&scope.held));
        let whole = bytes_of(&total);
        let line = |label: &str, held: &Held, streams: u64| {
            let bytes = bytes_of(held);
            let columns: Vec<String> = bytes.iter().zip(&whole).map(|(b, w)| format!("{:.1}, {:.1} %", mb(*b), *b as f64 * PERCENT / (*w).max(1) as f64)).collect();
            println!("| {name} | {label} | {streams} | {:.1} | {} |", held.pixels as f64 / PIXELS_PER_MEGAPIXEL, columns.join(" | "));
        };
        for class in CLASSES.iter() {
            let mut held = Held::default();
            let mut streams = 0u64;
            for scope in &cells[class.at()] {
                held.add(&scope.held);
                streams += scope.streams;
            }
            if held.frames > 0 {
                line(class.label(), &held, streams);
            }
        }
        let mut above = Held::default();
        let mut streams = 0u64;
        for class in cells.iter() {
            above.add(&class[MIX_BANDS - 1].held);
            streams += class[MIX_BANDS - 1].streams;
        }
        if above.frames > 0 {
            line(&format!("of all these, above {}", MIX_TOPS[MIX_TOPS.len() - 1]), &above, streams);
        }
    }
}

fn print_row_costs(client: Client, whole: &Scope, most_runs: u64, runs_by_log: &[u64]) {
    let held = &whole.held;
    let widths: Vec<String> = ENTRY_WIDTHS.iter().zip(&held.entry_widths).map(|(width, count)| format!("{width} bytes {:.1} %", *count as f64 * PERCENT / held.frames.max(1) as f64)).collect();
    println!("\n{}: row tables: {:.1} rows a frame on average; the smallest entry that works per frame (first, length, offset in whole bytes): {}; {:.1} bytes a frame. At 4 or 8 bytes, which a shader reads in one fetch: {:.1} bytes a frame, {:.1} MB; 8 bytes everywhere would be {:.1} MB.", client.label(), ratio(held.rows, held.frames), widths.join(", "), ratio(held.row_smallest, held.frames), ratio(held.row_aligned, held.frames), mb(held.row_aligned), mb(held.rows * ROW_ENTRY_WIDE));
    let logs: Vec<String> = runs_by_log.iter().enumerate().filter(|row| *row.1 > 0).map(|(log, count)| format!("{} fetches: {} bodies", log + 2, count)).collect();
    println!("{}: painted runs in a row: at most {most_runs}; {:.2} runs a row. With the runs searched, the worst row of each body takes: {}.", client.label(), ratio(held.runs, held.rows), logs.join(", "));
}

fn print_bc4(errors: &Bc4Errors) {
    println!("\nEnhanced, R8 index + BC4 coverage, the best endpoints for each block's set of levels: {} of {} blocks don't round back to their levels ({:.4} %); {:.4} % of pixels change level, {:.5} levels on average and at worst {} of 16; {:.3} of 255 from the level's own byte on average and at worst {} (before a shader rounds it back); {} pixels change between transparent and painted.", errors.inexact_blocks, errors.blocks, errors.inexact_blocks as f64 * PERCENT / errors.blocks.max(1) as f64, errors.changed as f64 * PERCENT / errors.pixels.max(1) as f64, errors.level_sum as f64 / errors.pixels.max(1) as f64, errors.level_worst, errors.byte_sum as f64 / errors.pixels.max(1) as f64, errors.byte_worst, errors.flips);
}

fn print_hue(client: Client, scopes: &[(String, &Scope)]) {
    let what = if client == Client::Cc { "under a partial hue, grey colours take it: no such mask is packed today" } else { "the palette's keep-colour flag" };
    println!("\n#### {}: hue masks ({what}): BC7 and BC1 can't hue by index, so a masked frame needs one\n\n| scope | none | all | masked | R8 mask MB | BC4 mask MB | one-bit mask MB | BC7 MB | + R8 | + BC4 | + bits | BC1 MB | + R8 | + BC4 | + bits | rectangle MB |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    for (name, scope) in scopes {
        let held = &scope.held;
        let share = |mode: u8| held.hue_frames[usize::from(mode)] as f64 * PERCENT / held.frames.max(1) as f64;
        let (r8, bc4, bits) = (held.masked_pixels, held.masked_blocks * BC4_BYTES, held.masked_bits);
        let (bc7, bc1) = (held.blocks * BC7_BYTES, held.blocks * BC1_BYTES);
        println!(
            "| {name} | {:.1} % | {:.1} % | {:.1} % | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
            share(HUE_NONE), share(HUE_ALL), share(HUE_MASKED), mb(r8), mb(bc4), mb(bits), mb(bc7), mb(bc7 + r8), mb(bc7 + bc4), mb(bc7 + bits), mb(bc1), mb(bc1 + r8), mb(bc1 + bc4), mb(bc1 + bits), mb(held.pixels * client.pixel_bytes() as u64)
        );
    }
}

/// The load units, with the on-screen frames as the mean frame of each NPC's faced stream.
fn print_load_units(client: Client, counted: &[BodyHeld]) {
    println!("\n#### {}: what each load unit holds, MB; palettes and tables included, each layout's own\n\n| crowd | load unit | streams | frames | rectangle | BC7 | BC1 | row spans | painted only |\n|---|---|---:|---:|---:|---:|---:|---:|---:|", client.label());
    let bytes = client.pixel_bytes() as u64;
    for crowd in 0..CROWD_COUNT {
        let mut rows: Vec<(&str, f64, f64, Scope)> = Vec::new();
        // All actions of the bodies in the crowd.
        let mut all = Scope::default();
        let mut streams = 0.0;
        for body in counted.iter().filter(|b| b.wanted.iter().any(|w| w.scope == crowd * UNITS_PER_CROWD + FIVE)) {
            body.streams.iter().for_each(|s| all.add_stream(s));
            streams += body.streams.len() as f64;
        }
        rows.push(("all actions of each body", streams, all.held.frames as f64, all));
        for (unit, name) in [(FIVE, "one action, five directions"), (FACED, "one action, the faced direction")] {
            let mut scope = Scope::default();
            let mut count = 0.0;
            for body in counted {
                for wanted in body.wanted.iter().filter(|w| w.scope == crowd * UNITS_PER_CROWD + unit) {
                    wanted.streams.iter().for_each(|&s| scope.add_stream(&body.streams[s]));
                    count += wanted.streams.len() as f64;
                }
            }
            rows.push((name, count, scope.held.frames as f64, scope));
        }
        for (name, streams, frames, scope) in &rows {
            let held = &scope.held;
            let frame_table = held.frames * FRAME_ENTRY_BYTES;
            let palettes = scope.palettes.len() as u64 * PALETTE_ROW_BYTES;
            println!(
                "| {} | {name} | {streams:.0} | {frames:.0} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
                CROWD_NAMES[crowd],
                mb(held.pixels * bytes + frame_table + palettes),
                mb(held.blocks * BC7_BYTES + frame_table),
                mb(held.blocks * BC1_BYTES + frame_table),
                mb(held.spans * bytes + frame_table + held.row_aligned + palettes),
                mb(held.painted * bytes + frame_table + held.row_aligned + held.words * WORD_BYTES + palettes),
            );
        }
        // Only the frames on screen: one NPC, one frame, a stream's mean.
        let (mut npcs, mut rectangle, mut bc7, mut bc1, mut spans, mut painted) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let mut palettes = HashSet::new();
        for body in counted {
            for &(_, s) in body.on_screen.iter().filter(|row| row.0 == crowd) {
                let stream = &body.streams[s];
                let held = &stream.held;
                let scale = 1.0 / held.frames.max(1) as f64;
                let frame_table = FRAME_ENTRY_BYTES as f64;
                npcs += 1.0;
                palettes.insert(stream.palette);
                rectangle += (held.pixels * bytes) as f64 * scale + frame_table;
                bc7 += (held.blocks * BC7_BYTES) as f64 * scale + frame_table;
                bc1 += (held.blocks * BC1_BYTES) as f64 * scale + frame_table;
                spans += (held.spans * bytes + held.row_aligned) as f64 * scale + frame_table;
                painted += (held.painted * bytes + held.row_aligned + held.words * WORD_BYTES) as f64 * scale + frame_table;
            }
        }
        let palette_bytes = (palettes.len() as u64 * PALETTE_ROW_BYTES) as f64;
        let in_mb = |value: f64| value / BYTES_PER_MB;
        println!("| {} | only the frames on screen, a stream's mean frame | {npcs:.0} | {npcs:.0} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |", CROWD_NAMES[crowd], in_mb(rectangle + palette_bytes), in_mb(bc7), in_mb(bc1), in_mb(spans + palette_bytes), in_mb(painted + palette_bytes));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Allocators

trait PageAllocator {
    type Id: Copy;
    fn new_page(side: i32) -> Self;
    fn place(&mut self, w: i32, h: i32) -> Option<Self::Id>;
    fn free(&mut self, id: Self::Id);
}

struct Guillotine(AtlasAllocator);

impl PageAllocator for Guillotine {
    type Id = AllocId;

    fn new_page(side: i32) -> Self {
        Guillotine(AtlasAllocator::new(size2(side, side)))
    }

    fn place(&mut self, w: i32, h: i32) -> Option<AllocId> {
        self.0.allocate(size2(w, h)).map(|allocation| allocation.id)
    }

    fn free(&mut self, id: AllocId) {
        self.0.deallocate(id);
    }
}

struct Shelf {
    y: i32,
    height: i32,
    /// Free runs of x, sorted: (x, width).
    free: Vec<(i32, i32)>,
}

/// Shelves with a free list in each: a whole free shelf merges with free neighbours and can take other heights.
struct ShelfPage {
    side: i32,
    shelves: Vec<Shelf>,
    bottom: i32,
}

#[derive(Clone, Copy)]
struct ShelfId {
    y: i32,
    x: i32,
    w: i32,
}

impl ShelfPage {
    fn is_free(&self, shelf: &Shelf) -> bool {
        shelf.free.len() == 1 && shelf.free[0].1 == self.side
    }

    fn take(shelf: &mut Shelf, w: i32) -> Option<ShelfId> {
        let at = shelf.free.iter().position(|run| run.1 >= w)?;
        let (x, width) = shelf.free[at];
        if width == w {
            shelf.free.remove(at);
        } else {
            shelf.free[at] = (x + w, width - w);
        }
        Some(ShelfId { y: shelf.y, x, w })
    }
}

impl PageAllocator for ShelfPage {
    type Id = ShelfId;

    fn new_page(side: i32) -> Self {
        ShelfPage { side, shelves: Vec::new(), bottom: 0 }
    }

    fn place(&mut self, w: i32, h: i32) -> Option<ShelfId> {
        let height = (h + SHELF_HEIGHT_STEP - 1) / SHELF_HEIGHT_STEP * SHELF_HEIGHT_STEP;
        let tolerated = height + height / SHELF_WASTE_DIVISOR;
        let mut best: Option<(i32, usize)> = None;
        for (n, shelf) in self.shelves.iter().enumerate() {
            if shelf.height >= height && shelf.height <= tolerated && shelf.free.iter().any(|run| run.1 >= w) && best.is_none_or(|b| shelf.height < b.0) {
                best = Some((shelf.height, n));
            }
        }
        if let Some((_, n)) = best {
            return Self::take(&mut self.shelves[n], w);
        }
        if self.bottom + height <= self.side {
            self.shelves.push(Shelf { y: self.bottom, height, free: vec![(0, self.side)] });
            self.bottom += height;
            return Self::take(self.shelves.last_mut().expect("a new shelf"), w);
        }
        // A wholly free shelf can be cut to this height.
        if let Some(n) = (0..self.shelves.len()).find(|&n| self.is_free(&self.shelves[n]) && self.shelves[n].height >= height) {
            let rest = self.shelves[n].height - height;
            self.shelves[n].height = height;
            if rest > 0 {
                let y = self.shelves[n].y + height;
                self.shelves.insert(n + 1, Shelf { y, height: rest, free: vec![(0, self.side)] });
            }
            return Self::take(&mut self.shelves[n], w);
        }
        // Nothing fits as it should: any taller shelf, whatever it wastes.
        let n = (0..self.shelves.len()).filter(|&n| self.shelves[n].height >= height && self.shelves[n].free.iter().any(|run| run.1 >= w)).min_by_key(|&n| self.shelves[n].height)?;
        Self::take(&mut self.shelves[n], w)
    }

    fn free(&mut self, id: ShelfId) {
        let n = self.shelves.iter().position(|shelf| shelf.y == id.y).expect("the allocation's shelf");
        let shelf = &mut self.shelves[n];
        let at = shelf.free.partition_point(|run| run.0 < id.x);
        shelf.free.insert(at, (id.x, id.w));
        if at + 1 < shelf.free.len() && shelf.free[at].0 + shelf.free[at].1 == shelf.free[at + 1].0 {
            shelf.free[at].1 += shelf.free[at + 1].1;
            shelf.free.remove(at + 1);
        }
        if at > 0 && shelf.free[at - 1].0 + shelf.free[at - 1].1 == shelf.free[at].0 {
            shelf.free[at - 1].1 += shelf.free[at].1;
            shelf.free.remove(at);
        }
        // Free shelves merge; the last one gives its height back.
        let mut n = n;
        if n + 1 < self.shelves.len() && self.is_free(&self.shelves[n]) && self.is_free(&self.shelves[n + 1]) {
            self.shelves[n].height += self.shelves[n + 1].height;
            self.shelves.remove(n + 1);
        }
        if n > 0 && self.is_free(&self.shelves[n]) && self.is_free(&self.shelves[n - 1]) {
            self.shelves[n - 1].height += self.shelves[n].height;
            self.shelves.remove(n);
            n -= 1;
        }
        if n + 1 == self.shelves.len() && self.is_free(&self.shelves[n]) {
            self.bottom = self.shelves[n].y;
            self.shelves.pop();
        }
    }
}

/// A rectangle for the atlas: its size with gutters, and the frame pixels in it.
#[derive(Clone, Copy)]
struct Rect {
    w: i32,
    h: i32,
    pixels: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum RectMode {
    Frames,
    StreamBlock,
}

impl RectMode {
    fn label(self) -> &'static str {
        match self {
            RectMode::Frames => "each frame",
            RectMode::StreamBlock => "each stream as one block",
        }
    }
}

fn rects_of_frames(sizes: &[FrameSize]) -> Vec<Rect> {
    sizes.iter().map(|s| Rect { w: i32::from(s.w) + GUTTER * GUTTER_SIDES, h: i32::from(s.h) + GUTTER * GUTTER_SIDES, pixels: u64::from(s.w) * u64::from(s.h) }).collect()
}

/// A stream's frames on shelves inside one block, tallest first; a stream too large for a page takes more than one.
fn rects_of_stream(sizes: &[FrameSize], page: i32) -> Vec<Rect> {
    let mut frames = rects_of_frames(sizes);
    frames.sort_by_key(|r| Reverse(r.h));
    let area: f64 = frames.iter().map(|r| f64::from(r.w) * f64::from(r.h)).sum();
    let widest = frames.iter().map(|r| r.w).max().unwrap_or(0);
    let target = ((area.sqrt() * BLOCK_WIDTH_SLACK) as i32).max(widest).min(page);
    let mut out = Vec::new();
    let (mut x, mut y, mut row, mut width, mut pixels) = (0, 0, 0, 0, 0u64);
    for frame in frames {
        if x > 0 && x + frame.w > target {
            y += row;
            x = 0;
            row = 0;
        }
        if y + frame.h > page {
            out.push(Rect { w: width, h: y + row, pixels });
            (x, y, row, width, pixels) = (0, 0, 0, 0, 0);
        }
        x += frame.w;
        row = row.max(frame.h);
        width = width.max(x);
        pixels += frame.pixels;
    }
    if pixels > 0 {
        out.push(Rect { w: width, h: y + row, pixels });
    }
    out
}

fn rects_of(mode: RectMode, sizes: &[FrameSize], page: i32) -> Vec<Rect> {
    match mode {
        RectMode::Frames => rects_of_frames(sizes),
        RectMode::StreamBlock => rects_of_stream(sizes, page),
    }
}

/// A fresh pack: how many pages, and the area its rectangles take.
#[derive(Clone, Copy, Default)]
struct Pack {
    pages: u64,
    rectangle_area: u64,
    pixels: u64,
}

fn fresh_pack<A: PageAllocator>(rects: impl Iterator<Item = Rect>, side: i32, window: usize) -> Pack {
    let mut pages: Vec<A> = Vec::new();
    let mut pack = Pack::default();
    for rect in rects {
        pack.rectangle_area += i64::from(rect.w).unsigned_abs() * i64::from(rect.h).unsigned_abs();
        pack.pixels += rect.pixels;
        let open = pages.len().saturating_sub(window);
        if !pages[open..].iter_mut().any(|page| page.place(rect.w, rect.h).is_some()) {
            let mut page = A::new_page(side);
            assert!(page.place(rect.w, rect.h).is_some(), "a rectangle of {} x {} on an empty page of {side}", rect.w, rect.h);
            pages.push(page);
            pack.pages += 1;
        }
    }
    pack
}

fn page_area(pages: u64, side: i32) -> u64 {
    pages * i64::from(side).unsigned_abs() * i64::from(side).unsigned_abs()
}

fn print_pack(name: &str, allocator: &str, mode: RectMode, side: i32, pack: &Pack, bytes: u64) {
    let area = page_area(pack.pages, side);
    println!(
        "| {name} | {allocator} | {side} | {} | {} | {:.1} | {:.1} | {:.1} % | {:.1} % | {:.1} % | {:.1} |",
        mode.label(),
        pack.pages,
        area as f64 / PIXELS_PER_MEGAPIXEL,
        pack.pixels as f64 / PIXELS_PER_MEGAPIXEL,
        pack.pixels as f64 * PERCENT / area.max(1) as f64,
        (pack.rectangle_area - pack.pixels) as f64 * PERCENT / area.max(1) as f64,
        area.saturating_sub(pack.rectangle_area) as f64 * PERCENT / area.max(1) as f64,
        mb(area * bytes),
    );
}

const PACK_HEADER: &str = "| what | allocator | page side | rectangles | pages | page Mpx | frame Mpx | frames' share | gutters and blocks' gaps | empty | page MB, rectangle format |\n|---|---|---:|---|---:|---:|---:|---:|---:|---:|---:|";

/// One crowd as streams: the sizes of each stream it loads.
fn streams_of_scope<'a>(counted: &'a [BodyHeld], scope: usize) -> Vec<&'a [FrameSize]> {
    let mut out = Vec::new();
    for body in counted {
        for wanted in body.wanted.iter().filter(|w| w.scope == scope) {
            out.extend(wanted.streams.iter().map(|&s| body.streams[s].sizes.as_slice()));
        }
    }
    out
}

fn print_dynamic(client: Client, counted: &[BodyHeld]) {
    let bytes = client.pixel_bytes() as u64;
    let all: Vec<&[FrameSize]> = counted.iter().flat_map(|b| b.streams.iter().map(|s| s.sizes.as_slice())).collect();
    println!("\n#### {}: dynamic allocators, a fresh pack, a 1 px gutter on each side of each rectangle; the whole set keeps {OPEN_PAGES} pages open, a crowd all of them\n\n{PACK_HEADER}", client.label());
    let mut jobs: Vec<(String, Vec<&[FrameSize]>, usize)> = vec![("whole set".to_string(), all, OPEN_PAGES)];
    for crowd in [CREATURES, TOWN] {
        jobs.push((format!("{}, five directions", CROWD_NAMES[crowd]), streams_of_scope(counted, crowd * UNITS_PER_CROWD + FIVE), usize::MAX));
        jobs.push((format!("{}, faced only", CROWD_NAMES[crowd]), streams_of_scope(counted, crowd * UNITS_PER_CROWD + FACED), usize::MAX));
    }
    for (name, streams, window) in &jobs {
        let mut rows = Vec::new();
        for side in PAGE_SIDES {
            for mode in [RectMode::Frames, RectMode::StreamBlock] {
                rows.push((side, mode));
            }
        }
        let packs: Vec<(Pack, Pack)> = rows
            .par_iter()
            .map(|&(side, mode)| {
                let rects = || streams.iter().flat_map(move |sizes| rects_of(mode, sizes, side));
                (fresh_pack::<Guillotine>(rects(), side, *window), fresh_pack::<ShelfPage>(rects(), side, *window))
            })
            .collect();
        for (&(side, mode), (guillotine, shelf)) in rows.iter().zip(&packs) {
            print_pack(name, "guillotine", mode, side, guillotine, bytes);
            print_pack(name, "shelf", mode, side, shelf, bytes);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Today's packer

struct TodayPage {
    used: u64,
    bucket: u64,
    /// Where its frames end, as a position in the sorted frames.
    end: usize,
    masked: bool,
}

fn align_today(value: u32) -> u32 {
    value.div_ceil(TODAY_ALIGN) * TODAY_ALIGN
}

fn today_extent(side: u16) -> u32 {
    align_today(u32::from(side)) + 2 * align_today(TODAY_GUTTER)
}

/// The largest used extents when all of these go on a page, biggest first as the packer places them; none if they don't fit.
fn today_place(frames: &[FrameSize], bucket: (u32, u32)) -> Option<(u32, u32)> {
    let mut rects: Vec<(u32, u32, usize)> = frames.iter().enumerate().map(|(n, f)| (today_extent(f.w), today_extent(f.h), n)).collect();
    rects.sort_by_key(|r| (Reverse(r.0 * r.1), r.2));
    let mut allocator = AtlasAllocator::new(size2(bucket.0 as i32, bucket.1 as i32));
    let (mut used_w, mut used_h) = (0u32, 0u32);
    for (w, h, _) in rects {
        let allocation = allocator.allocate(size2(w as i32, h as i32))?;
        used_w = used_w.max(allocation.rectangle.min.x as u32 + w);
        used_h = used_h.max(allocation.rectangle.min.y as u32 + h);
    }
    Some((used_w, used_h))
}

/// The longest prefix a bucket takes: it can't exceed the bucket's area, and fitting is taken as monotonic, as the packer does.
fn today_prefix(frames: &[FrameSize], bucket: (u32, u32)) -> (usize, u64) {
    let area = u64::from(bucket.0) * u64::from(bucket.1);
    let (mut top, mut taken) = (0usize, 0u64);
    for frame in frames {
        let (w, h) = (today_extent(frame.w), today_extent(frame.h));
        let cost = u64::from(w) * u64::from(h);
        if w > bucket.0 || h > bucket.1 || taken + cost > area {
            break;
        }
        taken += cost;
        top += 1;
    }
    if top == 0 {
        return (0, 0);
    }
    let (mut low, mut high) = (0usize, top);
    if today_place(&frames[..top], bucket).is_some() {
        low = top;
    }
    while high - low > 1 {
        let middle = (low + high) / 2;
        if today_place(&frames[..middle], bucket).is_some() {
            low = middle;
        } else {
            high = middle;
        }
    }
    let alloc: u64 = frames[..low].iter().map(|f| u64::from(today_extent(f.w)) * u64::from(today_extent(f.h))).sum();
    (low, alloc)
}

/// `pack_planned_frames_into_package` without pixels: of each page size's longest prefix, the one filling its page best.
fn todays_packer(frames: &[FrameSize]) -> Vec<TodayPage> {
    let mut pages = Vec::new();
    let mut at = 0usize;
    while at < frames.len() {
        let rest = &frames[at..];
        let picks: Vec<((u32, u32), usize, u64)> = TODAY_BUCKETS.par_iter().map(|&bucket| {
            let (prefix, alloc) = today_prefix(rest, bucket);
            (bucket, prefix, alloc)
        }).collect();
        let mut best: Option<((u32, u32), usize, u64)> = None;
        for pick in picks.into_iter().filter(|pick| pick.1 > 0) {
            let area = |bucket: (u32, u32)| u64::from(bucket.0) * u64::from(bucket.1);
            let replace = best.is_none_or(|b| {
                pick.2 * area(b.0) > b.2 * area(pick.0) || (pick.2 * area(b.0) == b.2 * area(pick.0) && (pick.1 > b.1 || (pick.1 == b.1 && area(pick.0) < area(b.0))))
            });
            if replace {
                best = Some(pick);
            }
        }
        let (bucket, prefix, _) = best.expect("a frame fits the largest page");
        let (used_w, used_h) = today_place(&rest[..prefix], bucket).expect("the prefix fits");
        pages.push(TodayPage {
            used: u64::from(used_w) * u64::from(used_h),
            bucket: u64::from(bucket.0) * u64::from(bucket.1),
            end: at + prefix,
            masked: rest[..prefix].iter().any(|f| f.hue == HUE_MASKED),
        });
        at += prefix;
    }
    pages
}

fn print_today(client: Client, counted: &mut [BodyHeld], whole: &Scope) {
    let mut frames = Vec::new();
    for body in counted.iter_mut() {
        for stream in &mut body.streams {
            stream.position = frames.len();
            frames.extend_from_slice(&stream.sizes);
        }
    }
    let started = Instant::now();
    let pages = todays_packer(&frames);
    let held = &whole.held;
    let used: u64 = pages.iter().map(|p| p.used).sum();
    let bucket: u64 = pages.iter().map(|p| p.bucket).sum();
    let mask: u64 = pages.iter().filter(|p| p.masked).map(|p| p.used).sum();
    let padded = held.blocks * PIXELS_PER_BLOCK;
    println!("\n#### {}: today's packer, simulated without pixels ({:.0} s): bucket pages, a {TODAY_GUTTER} px gutter, frames aligned to 4\n", client.label(), started.elapsed().as_secs_f64());
    println!("{} pages; BC7 pages at 1 byte a used pixel {:.1} MB; atlas page area {:.1} Mpx; pages with a hue mask {} of R8 {:.1} MB; together {:.1} MB", pages.len(), mb(used), bucket as f64 / PIXELS_PER_MEGAPIXEL, pages.iter().filter(|p| p.masked).count(), mb(mask), mb(used + mask));
    println!("Frame boxes {:.1} Mpx ({:.1} % of used); padded to 4 x 4 {:.1} Mpx ({:.1} %); painted {:.1} Mpx ({:.1} %, so {:.1} % of the used page holds no painted pixel)", held.pixels as f64 / PIXELS_PER_MEGAPIXEL, held.pixels as f64 * PERCENT / used.max(1) as f64, padded as f64 / PIXELS_PER_MEGAPIXEL, padded as f64 * PERCENT / used.max(1) as f64, held.painted as f64 / PIXELS_PER_MEGAPIXEL, held.painted as f64 * PERCENT / used.max(1) as f64, PERCENT - held.painted as f64 * PERCENT / used.max(1) as f64);

    // Whole pages held for a crowd: a frame anywhere on a page holds all of it.
    println!("\n| crowd | load unit | frames' own BC7 MB | pages touched | whole pages BC7 MB | x frames' own | with their R8 masks MB |\n|---|---|---:|---:|---:|---:|---:|");
    let page_of = |position: usize| pages.partition_point(|p| p.end <= position);
    for crowd in 0..CROWD_COUNT {
        for (unit, name) in [(FIVE, "five directions"), (FACED, "faced only")] {
            let mut touched = BTreeSet::new();
            let mut own = 0u64;
            for body in counted.iter() {
                for wanted in body.wanted.iter().filter(|w| w.scope == crowd * UNITS_PER_CROWD + unit) {
                    for &s in &wanted.streams {
                        let stream = &body.streams[s];
                        own += stream.held.blocks * BC7_BYTES;
                        for n in 0..stream.sizes.len() {
                            touched.insert(page_of(stream.position + n));
                        }
                    }
                }
            }
            let whole_pages: u64 = touched.iter().map(|&p| pages[p].used).sum();
            let masks: u64 = touched.iter().filter(|&&p| pages[p].masked).map(|&p| pages[p].used).sum();
            println!("| {} | {name} | {:.1} | {} | {:.1} | {:.2} | {:.1} |", CROWD_NAMES[crowd], mb(own), touched.len(), mb(whole_pages), ratio(whole_pages, own), mb(whole_pages + masks));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Churn

/// A stream a round loads: its identity and its rectangles for one mode and page.
struct RoundStream {
    id: (usize, u32),
    rects: Vec<Rect>,
}

struct Resident<Id> {
    used_in: usize,
    places: Vec<(usize, Id)>,
    pixels: u64,
    area: u64,
}

#[derive(Clone, Copy, Default)]
struct ChurnResult {
    budget: u64,
    peak_pages: u64,
    over_budget_rounds: u64,
    evicted: u64,
    /// At the round with the most pages over the fresh pack: its pages, live pixels, the fresh pack's pages.
    at_peak_pages: u64,
    at_peak_pixels: u64,
    at_peak_fresh: u64,
    /// The worst round's pages over its fresh pack.
    worst_over_fresh: f64,
}

fn churn<A: PageAllocator>(rounds: &[Vec<RoundStream>], side: i32, budget: u64) -> ChurnResult {
    let mut pages: Vec<A> = Vec::new();
    let mut residents = HashMap::<(usize, u32), Resident<A::Id>>::new();
    let mut result = ChurnResult { budget, ..ChurnResult::default() };
    for (round, streams) in rounds.iter().enumerate() {
        let pinned: HashSet<(usize, u32)> = streams.iter().map(|s| s.id).collect();
        for stream in streams {
            if let Some(resident) = residents.get_mut(&stream.id) {
                resident.used_in = round;
            }
        }
        for stream in streams.iter().filter(|s| !residents.contains_key(&s.id)).collect::<Vec<_>>() {
            let mut places = Vec::with_capacity(stream.rects.len());
            for rect in &stream.rects {
                loop {
                    if let Some(place) = pages.iter_mut().enumerate().find_map(|(n, page)| page.place(rect.w, rect.h).map(|id| (n, id))) {
                        places.push(place);
                        break;
                    }
                    if (pages.len() as u64) < budget {
                        pages.push(A::new_page(side));
                        continue;
                    }
                    // The least recently used stream this round doesn't need; failing that, a page over the budget.
                    let oldest = residents.iter().filter(|(id, _)| !pinned.contains(id)).min_by_key(|(id, r)| (r.used_in, **id)).map(|(id, _)| *id);
                    if let Some(id) = oldest {
                        let resident = residents.remove(&id).expect("a resident");
                        for (n, place) in resident.places {
                            pages[n].free(place);
                        }
                        result.evicted += 1;
                    } else {
                        pages.push(A::new_page(side));
                    }
                }
            }
            let (pixels, area) = stream.rects.iter().fold((0, 0), |sum, r| (sum.0 + r.pixels, sum.1 + (r.w as u64) * (r.h as u64)));
            residents.insert(stream.id, Resident { used_in: round, places, pixels, area });
        }
        let count = pages.len() as u64;
        result.peak_pages = result.peak_pages.max(count);
        result.over_budget_rounds += u64::from(count > budget);
        let live: u64 = residents.iter().filter(|(id, _)| pinned.contains(id)).map(|(_, r)| r.pixels).sum();
        let fresh = fresh_pack::<A>(streams.iter().flat_map(|s| s.rects.iter().copied()), side, usize::MAX).pages;
        let over = count as f64 / fresh.max(1) as f64;
        if over > result.worst_over_fresh {
            result.worst_over_fresh = over;
            result.at_peak_pages = count;
            result.at_peak_pixels = live;
            result.at_peak_fresh = fresh;
        }
    }
    let _cached: u64 = residents.values().map(|r| r.area).sum();
    result
}

/// A crowd drawn with its own seed, as `pick_uniform` draws one.
fn drawn(pairs: &[Pair], count: usize, seed: u64) -> Vec<Pair> {
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    let mut state = seed;
    for n in 0..count.min(order.len()) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let other = n + (state % (order.len() - n) as u64) as usize;
        order.swap(n, other);
    }
    order[..count.min(order.len())].iter().map(|&n| pairs[n]).collect()
}

fn print_churn(client: Client, counted: &[BodyHeld], pairs: &[Pair], rule: &BodyRule, bodies: &[BodySource]) {
    let bytes = client.pixel_bytes() as u64;
    let by_body: HashMap<usize, &BodyHeld> = counted.iter().map(|b| (b.body, b)).collect();
    let npcs: Vec<Pair> = pairs.iter().copied().filter(|p| rule.group(&bodies[p.body()]) == Group::Creature).collect();
    // Dungeon, town, dungeon...: odd rounds replace a creature crowd by dressed NPCs, and back.
    let crowds: Vec<Vec<Pair>> = (0..CHURN_ROUNDS)
        .map(|round| {
            let seed = crowd::CROWD_SEED.wrapping_add(CHURN_SEED_STEP.wrapping_mul(round as u64 + 1));
            if round % 2 == 0 {
                drawn(&npcs, crowd::CROWD_PAIRS, seed)
            } else {
                let shuffled = drawn(pairs, pairs.len(), seed);
                distinct_pairs(&town_of(&shuffled, &|body| rule.is_person(&bodies[body]), &|body| rule.group(&bodies[body]) == Group::Equipment))
            }
        })
        .collect();
    println!("\n#### {}: churn: {CHURN_ROUNDS} rounds, creature crowds and dressed NPCs in turn, five directions, 1 px gutter; LRU evicts what the round doesn't need, and a page is added only when nothing can be evicted\n\n| allocator | page side | rectangles | budget x largest fresh pack | budget pages | peak pages | rounds over budget | streams evicted | worst round: pages | its fresh pack | its live Mpx | live's share of its pages | waste that is fragmentation and cache | peak MB, rectangle format |\n|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    let mut configs = Vec::new();
    for side in PAGE_SIDES {
        for mode in [RectMode::Frames, RectMode::StreamBlock] {
            for factor in CHURN_BUDGETS {
                for shelf in [false, true] {
                    configs.push((side, mode, factor, shelf));
                }
            }
        }
    }
    let results: Vec<ChurnResult> = configs
        .par_iter()
        .map(|&(side, mode, factor, shelf)| {
            let rounds: Vec<Vec<RoundStream>> = crowds
                .iter()
                .map(|crowd| {
                    let mut seen = HashSet::new();
                    let mut out = Vec::new();
                    for pair in crowd {
                        for stream in by_body[&pair.body()].streams.iter().filter(|s| s.action == pair.action()) {
                            if seen.insert((pair.body(), stream.direction)) {
                                out.push(RoundStream { id: (pair.body(), stream.direction), rects: rects_of(mode, &stream.sizes, side) });
                            }
                        }
                    }
                    out
                })
                .collect();
            let fresh_of = |round: &Vec<RoundStream>| {
                let rects = || round.iter().flat_map(|s| s.rects.iter().copied());
                if shelf { fresh_pack::<ShelfPage>(rects(), side, usize::MAX).pages } else { fresh_pack::<Guillotine>(rects(), side, usize::MAX).pages }
            };
            let largest = rounds.iter().map(fresh_of).max().unwrap_or(0);
            let budget = (largest as f64 * factor).ceil() as u64;
            if shelf { churn::<ShelfPage>(&rounds, side, budget) } else { churn::<Guillotine>(&rounds, side, budget) }
        })
        .collect();
    for (&(side, mode, factor, shelf), result) in configs.iter().zip(&results) {
        let area = page_area(result.at_peak_pages, side);
        let fresh = page_area(result.at_peak_fresh, side);
        let live_share = result.at_peak_pixels as f64 * PERCENT / area.max(1) as f64;
        let waste = area.saturating_sub(result.at_peak_pixels);
        let beyond_fresh = area.saturating_sub(fresh);
        println!(
            "| {} | {side} | {} | {factor:.2} | {} | {} | {} | {} | {} | {} | {:.1} | {live_share:.1} % | {:.1} % | {:.1} |",
            if shelf { "shelf" } else { "guillotine" },
            mode.label(),
            result.budget,
            result.peak_pages,
            result.over_budget_rounds,
            result.evicted,
            result.at_peak_pages,
            result.at_peak_fresh,
            result.at_peak_pixels as f64 / PIXELS_PER_MEGAPIXEL,
            beyond_fresh as f64 * PERCENT / waste.max(1) as f64,
            mb(page_area(result.peak_pages, side) * bytes),
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------

/// The crowds of part `tk`, by the placeholder rule, and the uniform and worst ones of part `tu`.
fn crowds_of(pairs: &[Pair], rule: &BodyRule, bodies: &[BodySource]) -> [Vec<Pair>; CROWD_COUNT] {
    let npcs: Vec<Pair> = pairs.iter().copied().filter(|p| rule.group(&bodies[p.body()]) == Group::Creature).collect();
    [
        crowd::pick_uniform(&npcs, crowd::CROWD_PAIRS),
        town_of(pairs, &|body| rule.is_person(&bodies[body]), &|body| rule.group(&bodies[body]) == Group::Equipment),
        crowd::pick_uniform(pairs, crowd::CROWD_PAIRS),
        crowd::pick_largest(pairs, crowd::CROWD_PAIRS),
    ]
}

fn client_part(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], rule: &BodyRule, client: Client) {
    let started = Instant::now();
    let pairs = crowd::all_pairs(sources, bodies, present, client);
    let crowds = crowds_of(&pairs, rule, bodies);
    let mut occurrences = HashMap::<usize, Vec<Occurrence>>::new();
    for (crowd, list) in crowds.iter().enumerate() {
        for (position, pair) in list.iter().enumerate() {
            occurrences.entry(pair.body()).or_default().push(Occurrence { crowd, position, action: pair.action() });
        }
    }
    println!("\n### {}: the crowds drawn by {:.0} s", client.label(), started.elapsed().as_secs_f64());

    let mut of: Vec<usize> = present.iter().filter(|s| s.0 == client).map(|s| s.2).collect();
    // Today's packer takes frames in body order.
    of.sort_unstable();
    let done = AtomicUsize::new(0);
    let none = Vec::new();
    let mut counted: Vec<BodyHeld> = of
        .par_iter()
        .map(|&i| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            let (class, _) = classes_of(&planes, rule, &bodies[i]);
            let held = count_body(&planes, i, class, occurrences.get(&i).unwrap_or(&none));
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % PROGRESS_BODIES == 0 {
                println!("  {n}/{} bodies counted ({:.0} s)", of.len(), started.elapsed().as_secs_f64());
            }
            held
        })
        .collect();
    println!("  {} bodies counted by {:.0} s", counted.len(), started.elapsed().as_secs_f64());

    let mut whole = Scope::default();
    let mut classes: Vec<Scope> = vec![Scope::default(); CLASSES.len()];
    let mut scopes: Vec<Scope> = vec![Scope::default(); SCOPES];
    let mut bc4 = Bc4Errors::default();
    let mut most_runs = 0u64;
    let mut runs_by_log = vec![0u64; u64::BITS as usize];
    let mut mix_whole = new_mix_cells();
    let mut mix_scopes: Vec<MixCells> = (0..SCOPES).map(|_| new_mix_cells()).collect();
    for body in &counted {
        let band = mix_band(body.median);
        for stream in &body.streams {
            mix_whole[body.class.at()][band].add_stream(stream);
        }
        for wanted in &body.wanted {
            for &s in &wanted.streams {
                mix_scopes[wanted.scope][body.class.at()][band].add_stream(&body.streams[s]);
            }
        }
        let mut entries = [0u64; TILE_KINDS];
        for stream in &body.streams {
            whole.add_stream(stream);
            classes[body.class.at()].add_stream(stream);
            for t in 0..TILE_KINDS {
                entries[t] += stream.held.tile_entries[t];
            }
        }
        whole.add_body_tiles(&body.tiles, &entries);
        classes[body.class.at()].add_body_tiles(&body.tiles, &entries);
        for wanted in &body.wanted {
            let mut entries = [0u64; TILE_KINDS];
            for &s in &wanted.streams {
                scopes[wanted.scope].add_stream(&body.streams[s]);
                for t in 0..TILE_KINDS {
                    entries[t] += body.streams[s].held.tile_entries[t];
                }
            }
            scopes[wanted.scope].add_body_tiles(&wanted.tiles, &entries);
        }
        bc4.add(&body.bc4);
        most_runs = most_runs.max(body.most_runs);
        let log = u64::BITS - body.most_runs.saturating_sub(1).leading_zeros();
        runs_by_log[log as usize] += 1;
    }

    let class_rows: Vec<(String, &Scope)> = CLASSES.iter().map(|c| (c.label().to_string(), &classes[c.at()])).filter(|row| row.1.held.frames > 0).collect();
    let mut crowd_rows: Vec<(String, &Scope)> = Vec::new();
    for (crowd, name) in CROWD_NAMES.iter().enumerate() {
        crowd_rows.push((format!("{name}, five"), &scopes[crowd * UNITS_PER_CROWD + FIVE]));
        crowd_rows.push((format!("{name}, faced"), &scopes[crowd * UNITS_PER_CROWD + FACED]));
    }
    let mut whole_rows: Vec<(String, &Scope)> = vec![("whole set".to_string(), &whole)];
    whole_rows.extend(class_rows.iter().cloned());
    let mut all_rows = whole_rows.clone();
    all_rows.extend(crowd_rows.iter().cloned());

    println!("\nA frame's entry counts {FRAME_ENTRY_BYTES} bytes in every layout, a palette {PALETTE_ROW_BYTES} bytes, each distinct palette once in its scope. A crowd counts each distinct stream once. BC7 and BC1 hold no palette; their masks are in the hue table.");
    print_layouts(client, &whole);
    print_scopes("the classes", client, &class_rows);
    print_scopes("the crowds", client, &crowd_rows);
    print_row_costs(client, &whole, most_runs, &runs_by_log);
    if client == Client::Ec {
        print_bc4(&bc4);
    }
    print_blocks(client, &all_rows);
    let padding_rows: Vec<(String, &Scope)> = std::iter::once(("whole set".to_string(), &whole)).chain(class_rows.iter().map(|row| (row.0.clone(), row.1))).collect();
    print_padding(client, &padding_rows);
    print_hue(client, &all_rows);
    let mut mix_rows: Vec<(String, &MixCells)> = vec![("whole set".to_string(), &mix_whole)];
    for (crowd, name) in CROWD_NAMES.iter().enumerate() {
        mix_rows.push((format!("{name}, five"), &mix_scopes[crowd * UNITS_PER_CROWD + FIVE]));
        mix_rows.push((format!("{name}, faced"), &mix_scopes[crowd * UNITS_PER_CROWD + FACED]));
    }
    print_mix_population(client, &counted);
    print_mix(client, &mix_rows);
    print_mix_shares(client, &mix_rows);
    print_load_units(client, &counted);
    print_today(client, &mut counted, &whole);
    print_dynamic(client, &counted);
    print_churn(client, &counted, &pairs, rule, bodies);
    println!("\n  {} done by {:.0} s", client.label(), started.elapsed().as_secs_f64());
}

pub(in super::super) fn part_v(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    println!("\n### v: what the GPU holds; sizes only; classes and crowds by the placeholder rule");
    for client in [Client::Cc, Client::Ec] {
        client_part(sources, bodies, present, &rule, client);
    }
}
