//! Part `k8` of the `shrink` pass, LOSSY: keep about half of a body's frames and rebuild the others from their kept
//! neighbours. Measure only; the kept frames round-trip exactly under the coders of part `b`.
//!
//! A frame is kept by a per-action mask of its number within the direction, so the numbers of frame effects and skipped
//! frames stay valid. Actions of [`UNTOUCHED_FRAMES`] frames or fewer are left alone.

use super::*;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::collections::BTreeSet;
use uocf::animation_sequence::{AnimationSequence, MOVE_RUN, MOVE_STAND, MOVE_WALK};
use uocf::enhanced::mobile_body::MobileBodyMap;
use uocf::uop_container::hash::hash_file_name_single;

const OUTPUT_DIR: &str = "/home/claudio/rust-target/uodynamapper-scratch/anim-codec/halved";
const SEQUENCE_FILE: &str = "AnimationSequence.uop";
const TILEART_FILE: &str = "tileart.uop";
const MAX_SEQUENCE_BODY: u32 = 65_535;

const UNTOUCHED_FRAMES: usize = 4;
const BLOCK: i32 = 8;
const SEARCH_RADIUS: i32 = 16;
const LOAD_SAMPLE_STEP: usize = 2;
/// Per unit of vector length, in the units of the sum of absolute differences.
const LOAD_VECTOR_COST: i32 = 2;
const STORED_VECTOR_COST: i32 = 4;
const SOURCE_PREVIOUS: u8 = 0;
const SOURCE_NEXT: u8 = 1;
const SOURCE_EMPTY: u8 = 2;
const GREY: u8 = 128;
const P5: f64 = 0.05;
const SMALL_FRAME_PIXELS: f64 = 4096.0;
const MEDIUM_FRAME_PIXELS: f64 = 16384.0;

/// `0xE2` animation types as ClassicUO's enum names them (an assumption: not read from the client).
const TYPED_ATTACK: i8 = 0;
const TYPED_DIE: i8 = 3;
const TYPED_IMPACT: i8 = 4;
const TYPED_FIDGET: i8 = 5;
const TYPED_SPELL: i8 = 11;
pub(super) const KIND_IDLE: &str = "idle";
const KIND_WALK: &str = "walk";
const KIND_RUN: &str = "run";
const KIND_ATTACK: &str = "attack";
const KIND_DIE: &str = "die";
const KIND_IMPACT: &str = "impact";
const KIND_FIDGET: &str = "fidget";
const KIND_SPELL: &str = "spell";
const KIND_OTHER_TYPED: &str = "other typed";
const KIND_TRANSITION: &str = "transition";
const KIND_OTHER: &str = "other";
pub(super) const KIND_NO_SEQUENCE: &str = "no sequence";

const SELECTION_NAMES: [&str; 2] = ["uniform", "worst interpolators kept"];
const METHOD_WARP_LOAD: usize = 2;
const METHOD_NAMES: [&str; 4] = ["hold previous", "crossfade (off palette)", "block warp at load", "block warp, stored vectors"];

const TIMED_BODIES: usize = 12;
const TIMED_ACTIONS_PER_BODY: usize = 10;
const TOP_FPS_ROWS: usize = 14;
const TOP_PAIR_ROWS: usize = 14;

const SHEET_BODIES: usize = 5;
const SHEET_ROWS: usize = 3;
const SHEET_KINDS: [&str; 3] = [KIND_WALK, KIND_ATTACK, KIND_IDLE];
const SHEET_COLUMNS: usize = 7;
const SHEET_LARGE_SIDE: i32 = 110;
const SHEET_SCALE_SMALL: i32 = 4;
const SHEET_SCALE_LARGE: i32 = 2;
const SHEET_GAP: i32 = 6;
const SHEET_DIFFERENCE_GAIN: i32 = 4;
const SHEET_GAP_COLOUR: [u8; 3] = [32, 32, 48];

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const PNG_BIT_DEPTH: u8 = 8;
const PNG_COLOUR_RGB: u8 = 2;
const CRC_POLYNOMIAL: u32 = 0xEDB8_8320;

// ---------------------------------------------------------------------------------------------------------------
// Sprites: a frame in anchor-relative coordinates

#[derive(Clone)]
pub(super) struct Sprite {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Area {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
}

impl Area {
    fn union(self, other: Area) -> Area {
        let left = self.left.min(other.left);
        let top = self.top.min(other.top);
        let right = (self.left + self.w).max(other.left + other.w);
        let bottom = (self.top + self.h).max(other.top + other.h);
        Area { left, top, w: right - left, h: bottom - top }
    }

    fn expanded(self, by: i32) -> Area {
        Area { left: self.left - by, top: self.top - by, w: self.w + 2 * by, h: self.h + 2 * by }
    }
}

impl Sprite {
    pub(super) fn with_rgba(frame: &FrameBox, rgba: Vec<u8>) -> Sprite {
        Sprite { left: frame.left, top: frame.top, w: frame.w as i32, h: frame.h as i32, rgba }
    }

    pub(super) fn of(planes: &Planes, frame: &FrameBox) -> Sprite {
        Sprite { left: frame.left, top: frame.top, w: frame.w as i32, h: frame.h as i32, rgba: frame_rgba(planes, frame, &planes.index, &planes.coverage) }
    }

    fn area(&self) -> Area {
        Area { left: self.left, top: self.top, w: self.w, h: self.h }
    }

    #[inline(always)]
    pub(super) fn at(&self, x: i32, y: i32) -> [u8; RGBA_CHANNELS] {
        let (px, py) = (x - self.left, y - self.top);
        if px < 0 || py < 0 || px >= self.w || py >= self.h {
            return [0; RGBA_CHANNELS];
        }
        let i = (py * self.w + px) as usize * RGBA_CHANNELS;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// The sprite on `area`, transparent outside its box.
    fn on(&self, area: Area) -> Vec<u8> {
        let mut out = Vec::with_capacity((area.w * area.h) as usize * RGBA_CHANNELS);
        for y in 0..area.h {
            for x in 0..area.w {
                out.extend_from_slice(&self.at(area.left + x, area.top + y));
            }
        }
        out
    }
}

#[inline(always)]
fn over_grey(pixel: [u8; RGBA_CHANNELS]) -> [u8; 3] {
    let a = i32::from(pixel[3]);
    let channel = |c: u8| ((i32::from(c) * a + i32::from(GREY) * (i32::from(OPAQUE) - a)) / i32::from(OPAQUE)) as u8;
    [channel(pixel[0]), channel(pixel[1]), channel(pixel[2])]
}

#[inline(always)]
fn luma_of(rgb: [u8; 3]) -> u8 {
    ((u32::from(rgb[0]) * 77 + u32::from(rgb[1]) * 150 + u32::from(rgb[2]) * 29) >> u8::BITS) as u8
}

/// A sprite over grey, on an area.
struct Composite {
    area: Area,
    rgb: Vec<[u8; 3]>,
}

impl Composite {
    fn of(sprite: &Sprite, area: Area) -> Composite {
        let mut rgb = Vec::with_capacity((area.w * area.h) as usize);
        for y in 0..area.h {
            for x in 0..area.w {
                rgb.push(over_grey(sprite.at(area.left + x, area.top + y)));
            }
        }
        Composite { area, rgb }
    }

    #[inline(always)]
    fn offset(&self, x: i32, y: i32) -> usize {
        ((y - self.area.top) * self.area.w + (x - self.area.left)) as usize
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Rebuilding a frame

fn crossfade(a: &Sprite, b: &Sprite, weight: f32) -> Sprite {
    let area = a.area().union(b.area());
    let mut rgba = Vec::with_capacity((area.w * area.h) as usize * RGBA_CHANNELS);
    for y in 0..area.h {
        for x in 0..area.w {
            let (p, q) = (a.at(area.left + x, area.top + y), b.at(area.left + x, area.top + y));
            let (pa, qa) = (f32::from(p[3]) * (1.0 - weight), f32::from(q[3]) * weight);
            let alpha = pa + qa;
            if alpha <= 0.0 {
                rgba.extend_from_slice(&[0; RGBA_CHANNELS]);
                continue;
            }
            for c in 0..3 {
                rgba.push(((f32::from(p[c]) * pa + f32::from(q[c]) * qa) / alpha).round() as u8);
            }
            rgba.push(alpha.round() as u8);
        }
    }
    Sprite { left: area.left, top: area.top, w: area.w, h: area.h, rgba }
}

fn median_of_nine(mut values: [i32; 9]) -> i32 {
    values.sort_unstable();
    values[4]
}

#[inline(always)]
fn split_vector(v: i32, weight: f32) -> (i32, i32) {
    let toward_previous = (v as f32 * weight).round() as i32;
    (toward_previous, v - toward_previous)
}

/// Symmetric block matching between the neighbours (needs nothing stored), then each pixel is copied from the nearer
/// neighbour with its coverage, so the result stays on that neighbour's palette.
fn warp_at_load(a: &Sprite, b: &Sprite, weight: f32) -> Sprite {
    let area = a.area().union(b.area());
    let region = area.expanded(SEARCH_RADIUS + BLOCK);
    let luma = |sprite: &Sprite| -> Vec<u8> {
        let mut out = Vec::with_capacity((region.w * region.h) as usize);
        for y in 0..region.h {
            for x in 0..region.w {
                out.push(luma_of(over_grey(sprite.at(region.left + x, region.top + y))));
            }
        }
        out
    };
    let (la, lb) = (luma(a), luma(b));
    let offset = |x: i32, y: i32| ((y - region.top) * region.w + (x - region.left)) as usize;
    let (columns, rows) = ((area.w + BLOCK - 1) / BLOCK, (area.h + BLOCK - 1) / BLOCK);
    let mut vectors = vec![(0i32, 0i32); (columns * rows) as usize];
    let mut samples = Vec::with_capacity(BLOCK as usize * BLOCK as usize);
    for row in 0..rows {
        for column in 0..columns {
            let (x0, y0) = (area.left + column * BLOCK, area.top + row * BLOCK);
            samples.clear();
            for y in (y0..y0 + BLOCK).step_by(LOAD_SAMPLE_STEP) {
                for x in (x0..x0 + BLOCK).step_by(LOAD_SAMPLE_STEP) {
                    samples.push(offset(x, y));
                }
            }
            let cost_of = |vx: i32, vy: i32, limit: i32| -> i32 {
                let (ax, bx) = split_vector(vx, weight);
                let (ay, by) = split_vector(vy, weight);
                let (shift_a, shift_b) = ((-ay * region.w - ax) as isize, (by * region.w + bx) as isize);
                let mut cost = 0i32;
                for &at in &samples {
                    let p = la[(at as isize + shift_a) as usize];
                    let q = lb[(at as isize + shift_b) as usize];
                    cost += i32::from(p.abs_diff(q));
                    if cost >= limit {
                        break;
                    }
                }
                cost
            };
            let mut best = (cost_of(0, 0, i32::MAX), 0, 0);
            if best.0 > 0 {
                for vy in -SEARCH_RADIUS..=SEARCH_RADIUS {
                    for vx in -SEARCH_RADIUS..=SEARCH_RADIUS {
                        let penalty = LOAD_VECTOR_COST * (vx.abs() + vy.abs());
                        if penalty >= best.0 {
                            continue;
                        }
                        let cost = cost_of(vx, vy, best.0 - penalty) + penalty;
                        if cost < best.0 {
                            best = (cost, vx, vy);
                        }
                    }
                }
            }
            vectors[(row * columns + column) as usize] = (best.1, best.2);
        }
    }
    // A 3x3 median removes the lone wrong vectors.
    let get = |column: i32, row: i32| vectors[(row.clamp(0, rows - 1) * columns + column.clamp(0, columns - 1)) as usize];
    let mut smooth = vectors.clone();
    for row in 0..rows {
        for column in 0..columns {
            let mut xs = [0i32; 9];
            let mut ys = [0i32; 9];
            let mut n = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let v = get(column + dx, row + dy);
                    xs[n] = v.0;
                    ys[n] = v.1;
                    n += 1;
                }
            }
            smooth[(row * columns + column) as usize] = (median_of_nine(xs), median_of_nine(ys));
        }
    }
    let mut rgba = Vec::with_capacity((area.w * area.h) as usize * RGBA_CHANNELS);
    let nearer_previous = weight <= 0.5;
    for y in 0..area.h {
        let row = y / BLOCK;
        for x in 0..area.w {
            let (vx, vy) = smooth[(row * columns + x / BLOCK) as usize];
            let (ax, bx) = split_vector(vx, weight);
            let (ay, by) = split_vector(vy, weight);
            let pixel = if nearer_previous { a.at(area.left + x - ax, area.top + y - ay) } else { b.at(area.left + x + bx, area.top + y + by) };
            rgba.extend_from_slice(&pixel);
        }
    }
    Sprite { left: area.left, top: area.top, w: area.w, h: area.h, rgba }
}

/// What a stored rebuild needs: the frame's box and, per block, a neighbour and a vector.
#[derive(Default)]
struct StoredFrame {
    area: Option<(i16, i16, i16, i16)>,
    sources: Vec<u8>,
    xs: Vec<u8>,
    ys: Vec<u8>,
}

fn stored_search(a: &Sprite, b: &Sprite, real: &Sprite) -> StoredFrame {
    let area = real.area();
    let region = area.expanded(SEARCH_RADIUS + BLOCK);
    let (ca, cb) = (Composite::of(a, region), Composite::of(b, region));
    let target = Composite::of(real, area);
    let (columns, rows) = ((area.w + BLOCK - 1) / BLOCK, (area.h + BLOCK - 1) / BLOCK);
    let mut stored = StoredFrame { area: Some((area.left as i16, area.top as i16, area.w as i16, area.h as i16)), ..StoredFrame::default() };
    for row in 0..rows {
        for column in 0..columns {
            let (x0, y0) = (area.left + column * BLOCK, area.top + row * BLOCK);
            let (x1, y1) = ((x0 + BLOCK).min(area.left + area.w), (y0 + BLOCK).min(area.top + area.h));
            let mut empty = true;
            'scan: for y in y0..y1 {
                for x in x0..x1 {
                    if real.at(x, y)[3] != 0 {
                        empty = false;
                        break 'scan;
                    }
                }
            }
            if empty {
                stored.sources.push(SOURCE_EMPTY);
                continue;
            }
            let mut best = (i32::MAX, SOURCE_PREVIOUS, 0i32, 0i32);
            for (source, candidate) in [(SOURCE_PREVIOUS, &ca), (SOURCE_NEXT, &cb)] {
                for vy in -SEARCH_RADIUS..=SEARCH_RADIUS {
                    for vx in -SEARCH_RADIUS..=SEARCH_RADIUS {
                        let mut cost = STORED_VECTOR_COST * (vx.abs() + vy.abs());
                        'block: for y in y0..y1 {
                            let from = candidate.offset(x0 + vx, y + vy);
                            let wanted = target.offset(x0, y);
                            for (p, q) in candidate.rgb[from..from + (x1 - x0) as usize].iter().zip(&target.rgb[wanted..wanted + (x1 - x0) as usize]) {
                                cost += i32::from(p[0].abs_diff(q[0])) + i32::from(p[1].abs_diff(q[1])) + i32::from(p[2].abs_diff(q[2]));
                            }
                            if cost >= best.0 {
                                break 'block;
                            }
                        }
                        if cost < best.0 {
                            best = (cost, source, vx, vy);
                        }
                    }
                }
            }
            stored.sources.push(best.1);
            stored.xs.push(best.2 as i8 as u8);
            stored.ys.push(best.3 as i8 as u8);
        }
    }
    stored
}

fn stored_rebuild(a: &Sprite, b: &Sprite, stored: &StoredFrame) -> Sprite {
    let (left, top, w, h) = stored.area.expect("a searched frame");
    let area = Area { left: i32::from(left), top: i32::from(top), w: i32::from(w), h: i32::from(h) };
    let (columns, rows) = ((area.w + BLOCK - 1) / BLOCK, (area.h + BLOCK - 1) / BLOCK);
    let mut rgba = vec![0u8; (area.w * area.h) as usize * RGBA_CHANNELS];
    let mut next_vector = 0usize;
    for row in 0..rows {
        for column in 0..columns {
            let source = stored.sources[(row * columns + column) as usize];
            if source == SOURCE_EMPTY {
                continue;
            }
            let (vx, vy) = (i32::from(stored.xs[next_vector] as i8), i32::from(stored.ys[next_vector] as i8));
            next_vector += 1;
            let from = if source == SOURCE_PREVIOUS { a } else { b };
            for y in row * BLOCK..((row + 1) * BLOCK).min(area.h) {
                for x in column * BLOCK..((column + 1) * BLOCK).min(area.w) {
                    let pixel = from.at(area.left + x + vx, area.top + y + vy);
                    rgba[(y * area.w + x) as usize * RGBA_CHANNELS..][..RGBA_CHANNELS].copy_from_slice(&pixel);
                }
            }
        }
    }
    Sprite { left: area.left, top: area.top, w: area.w, h: area.h, rgba }
}

/// SSIMULACRA2 of a rebuild against the real frame, on the union of their boxes over grey.
pub(super) fn score(real: &Sprite, rebuilt: &Sprite) -> f64 {
    let area = real.area().union(rebuilt.area());
    let (p, q) = (real.on(area), rebuilt.on(area));
    if p == q {
        return SSIMULACRA2_IDENTICAL;
    }
    let image = |rgba: &[u8]| mosaic(&[(area.w as u32, area.h as u32, rgba)]);
    compute_frame_ssimulacra2(image(&p), image(&q)).expect("SSIMULACRA2")
}

const SSIMULACRA2_IDENTICAL: f64 = 100.0;

fn squared_error(real: &Sprite, rebuilt: &Sprite) -> u64 {
    let area = real.area().union(rebuilt.area());
    let mut total = 0u64;
    for y in 0..area.h {
        for x in 0..area.w {
            let (p, q) = (over_grey(real.at(area.left + x, area.top + y)), over_grey(rebuilt.at(area.left + x, area.top + y)));
            for c in 0..3 {
                let d = u64::from(p[c].abs_diff(q[c]));
                total += d * d;
            }
        }
    }
    total
}

// ---------------------------------------------------------------------------------------------------------------
// Which frames are kept

/// The frames of an action, per direction, in number order.
struct ActionFrames {
    action: u16,
    directions: Vec<Vec<usize>>,
}

impl ActionFrames {
    fn frames_per_direction(&self) -> usize {
        self.directions.iter().map(Vec::len).max().unwrap_or(0)
    }
}

fn actions_of(planes: &Planes) -> Vec<ActionFrames> {
    let directions = knowledge::directions_in_action(&planes.boxes);
    let mut map = BTreeMap::<u16, BTreeMap<u32, Vec<usize>>>::new();
    for (f, frame) in planes.boxes.iter().enumerate() {
        map.entry(frame.action).or_default().entry(directions[f]).or_default().push(f);
    }
    map.into_iter().map(|(action, by_direction)| ActionFrames { action, directions: by_direction.into_values().collect() }).collect()
}

fn uniform_mask(count: usize, protected: &BTreeSet<u32>) -> Vec<bool> {
    (0..count).map(|n| count <= UNTOUCHED_FRAMES || n % 2 == 0 || n == count - 1 || protected.contains(&(n as u32))).collect()
}

fn neighbours(mask: &[bool], position: usize, length: usize) -> (usize, usize) {
    let previous = (0..position).rev().find(|&n| mask[n]).unwrap_or(0);
    let next = (position + 1..length).find(|&n| mask[n]).unwrap_or(previous);
    (previous, next)
}

fn weight_between(previous: usize, position: usize, next: usize) -> f32 {
    if next == previous {
        0.0
    } else {
        (position - previous) as f32 / (next - previous) as f32
    }
}

/// As many frames as `target` remain: repeatedly drops the frame a crossfade of its kept neighbours rebuilds best.
fn worst_interpolators_mask(action: &ActionFrames, sprites: &HashMap<usize, Sprite>, target: usize, protected: &BTreeSet<u32>) -> Vec<bool> {
    let count = action.frames_per_direction();
    let mut mask = vec![true; count];
    if count <= UNTOUCHED_FRAMES {
        return mask;
    }
    let mut cache = HashMap::<(usize, usize, usize), u64>::new();
    let mut kept = count;
    while kept > target {
        let mut best: Option<(u64, usize)> = None;
        for position in 1..count - 1 {
            if !mask[position] || protected.contains(&(position as u32)) {
                continue;
            }
            let (previous, next) = neighbours(&mask, position, count);
            let error = *cache.entry((previous, position, next)).or_insert_with(|| {
                action
                    .directions
                    .iter()
                    .filter(|frames| next < frames.len())
                    .map(|frames| {
                        let rebuilt = crossfade(&sprites[&frames[previous]], &sprites[&frames[next]], weight_between(previous, position, next));
                        squared_error(&sprites[&frames[position]], &rebuilt)
                    })
                    .sum()
            });
            if best.is_none_or(|(smallest, _)| error < smallest) {
                best = Some((error, position));
            }
        }
        let Some((_, position)) = best else { break };
        mask[position] = false;
        kept -= 1;
    }
    mask
}

/// A copy of a body with only the frames the masks keep.
fn kept_planes(planes: &Planes, actions: &[ActionFrames], masks: &[Vec<bool>]) -> Planes {
    let mut keep = vec![false; planes.boxes.len()];
    for (action, mask) in actions.iter().zip(masks) {
        for frames in &action.directions {
            for (position, &f) in frames.iter().enumerate() {
                keep[f] = mask[position];
            }
        }
    }
    let mut out = Planes { boxes: Vec::new(), index: Vec::new(), coverage: Vec::new(), ..planes.clone() };
    for (f, frame) in planes.boxes.iter().enumerate() {
        if keep[f] {
            let start = out.index.len();
            out.index.extend_from_slice(&planes.index[frame.range()]);
            if !planes.coverage.is_empty() {
                out.coverage.extend_from_slice(&planes.coverage[frame.range()]);
            }
            out.boxes.push(FrameBox { start, ..frame.clone() });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Sequences: protected frames, action kinds and rates

pub(super) fn load_sequences(client_dir: &Path) -> BTreeMap<u32, AnimationSequence> {
    let Some(path) = find_ignoring_case(client_dir, SEQUENCE_FILE) else { return BTreeMap::new() };
    let mut package = UopPackage::load_with_mode(&path, LoadMode::Lazy).expect("the sequence package");
    let mut sequences = BTreeMap::new();
    let mut failed = 0usize;
    for body in 0..=MAX_SEQUENCE_BODY {
        let hash = hash_file_name_single(&AnimationSequence::entry_path(body));
        let Ok(Some(data)) = package.unpack_file_by_hash_cached(hash) else { continue };
        match AnimationSequence::parse(&data) {
            Ok(sequence) => {
                sequences.insert(body, sequence);
            }
            Err(_) => failed += 1,
        }
    }
    println!("{}: {} sequences parsed, {failed} failed", path.display(), sequences.len());
    sequences
}

/// The sequence each frames body plays by, from the tile art placeholders; else its own.
pub(super) fn sequence_of_frames_body(sequences: &BTreeMap<u32, AnimationSequence>, client_dir: &Path) -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    if let Some(body_map) = find_ignoring_case(client_dir, TILEART_FILE).and_then(|path| MobileBodyMap::load(path).ok()) {
        for definition in body_map.definitions.values() {
            let frames = definition.requested_frames_body();
            if sequences.contains_key(&definition.sequence_id) {
                if definition.body == frames {
                    map.insert(frames, definition.sequence_id);
                } else {
                    map.entry(frames).or_insert(definition.sequence_id);
                }
            }
        }
    }
    for &body in sequences.keys() {
        map.entry(body).or_insert(body);
    }
    map
}

pub(super) fn kinds_of(sequence: &AnimationSequence) -> HashMap<u32, &'static str> {
    let mut kinds = HashMap::new();
    let mut add = |action: u32, kind: &'static str| {
        if let Some(resolved) = sequence.resolve_action(action, false) {
            kinds.entry(resolved).or_insert(kind);
        }
    };
    for group in sequence.groups.values() {
        for (&movement, moves) in &group.moves {
            let kind = match movement {
                MOVE_STAND => KIND_IDLE,
                MOVE_WALK => KIND_WALK,
                MOVE_RUN => KIND_RUN,
                _ => KIND_OTHER,
            };
            add(moves.action, kind);
            add(moves.alternate, kind);
        }
    }
    for group in sequence.groups.values() {
        for (&(anim_type, _), actions) in &group.typed {
            let kind = match anim_type {
                TYPED_ATTACK => KIND_ATTACK,
                TYPED_DIE => KIND_DIE,
                TYPED_IMPACT => KIND_IMPACT,
                TYPED_FIDGET => KIND_FIDGET,
                TYPED_SPELL => KIND_SPELL,
                _ => KIND_OTHER_TYPED,
            };
            for &action in actions {
                add(action, kind);
            }
        }
    }
    for group in sequence.groups.values() {
        for &action in group.transitions.values() {
            add(action, KIND_TRANSITION);
        }
    }
    kinds
}

pub(super) fn body_number(planes: &Planes) -> Option<u32> {
    planes.label.rsplit(' ').next().and_then(|id| id.parse().ok())
}

fn report_rates(label: &str, sequences: &BTreeMap<u32, AnimationSequence>) {
    let mut fps = BTreeMap::<u32, u64>::new();
    let mut pairs = BTreeMap::<(u32, u32), u64>::new();
    let mut by_kind = BTreeMap::<&str, Vec<f64>>::new();
    let mut durations = Vec::new();
    for sequence in sequences.values() {
        let kinds = kinds_of(sequence);
        for (&action, record) in sequence.actions.iter().filter(|(_, record)| record.alias.is_none()) {
            let tenths = (f64::from(record.fps) * 10.0).round() as u32;
            *fps.entry(tenths).or_default() += 1;
            *pairs.entry((record.frame_count, tenths)).or_default() += 1;
            by_kind.entry(kinds.get(&action).copied().unwrap_or(KIND_OTHER)).or_default().push(f64::from(record.fps));
            if record.fps > 0.0 {
                durations.push(f64::from(record.frame_count) / f64::from(record.fps));
            }
        }
    }
    let total: u64 = fps.values().sum();
    println!("\n### {label}: fps of the {total} plain actions of {} sequences\n", sequences.len());
    let mut rows: Vec<(u32, u64)> = fps.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!("| fps | actions | % |\n|---:|---:|---:|");
    for (tenths, count) in rows.iter().take(TOP_FPS_ROWS) {
        println!("| {:.1} | {count} | {:.1} |", f64::from(*tenths) / 10.0, *count as f64 * PERCENT / total.max(1) as f64);
    }
    let mut rows: Vec<((u32, u32), u64)> = pairs.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!("\n| frames per direction | fps | actions | one cycle s |\n|---:|---:|---:|---:|");
    for ((frames, tenths), count) in rows.iter().take(TOP_PAIR_ROWS) {
        println!("| {frames} | {:.1} | {count} | {:.2} |", f64::from(*tenths) / 10.0, f64::from(*frames) * 10.0 / f64::from((*tenths).max(1)));
    }
    let durations = sorted(durations);
    if !durations.is_empty() {
        println!("\nCycle duration: median {:.2} s, p95 {:.2} s, max {:.2} s", percentile(&durations, MEDIAN), percentile(&durations, P95), durations.last().unwrap());
    }
    println!("\n| kind | actions | median fps | p5 fps | p95 fps |\n|---|---:|---:|---:|---:|");
    for (kind, values) in by_kind {
        let values = sorted(values);
        println!("| {kind} | {} | {:.1} | {:.1} | {:.1} |", values.len(), percentile(&values, MEDIAN), percentile(&values, P5), percentile(&values, P95));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// One body

#[derive(Clone)]
struct ScoreRow {
    selection: usize,
    method: usize,
    kind: &'static str,
    size_class: &'static str,
    score: f32,
    /// (action, direction, position)
    place: (u16, usize, usize),
}

#[derive(Default, Clone, Copy)]
struct BodySizes {
    frames: u64,
    pixels: u64,
    zstd: u64,
    rank: u64,
    vectors: u64,
    masks: u64,
}

struct BodyResult {
    label: String,
    number: Option<u32>,
    full: BodySizes,
    selected: [BodySizes; 2],
    untouched_frames: u64,
    protected_frames: u64,
    scores: Vec<ScoreRow>,
}

fn size_class_of(planes: &Planes) -> &'static str {
    let mean = planes.index.len() as f64 / planes.boxes.len().max(1) as f64;
    if mean < SMALL_FRAME_PIXELS {
        "small frames"
    } else if mean < MEDIUM_FRAME_PIXELS {
        "medium frames"
    } else {
        "large frames"
    }
}

fn coded_sizes(planes: &Planes) -> (u64, u64) {
    let zstd = candidate("zstd 19, body", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body);
    let rank = candidate("rank", Order::Original, IndexCoding::Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Action);
    let (zstd_encoded, rank_encoded) = rayon::join(|| encode_candidate(planes, &zstd), || encode_candidate(planes, &rank));
    verify_round_trip(planes, &zstd_encoded, &zstd);
    verify_round_trip(planes, &rank_encoded, &rank);
    (zstd_encoded.total(), rank_encoded.total())
}

fn protected_of(sequence: Option<&AnimationSequence>, action: u16) -> BTreeSet<u32> {
    sequence.and_then(|s| s.actions.get(&u32::from(action))).map(|record| record.frame_effects.keys().copied().collect()).unwrap_or_default()
}

fn masks_of(action: &ActionFrames, sprites: &HashMap<usize, Sprite>, protected: &BTreeSet<u32>) -> [Vec<bool>; 2] {
    let uniform = uniform_mask(action.frames_per_direction(), protected);
    let target = uniform.iter().filter(|&&keep| keep).count();
    let worst = worst_interpolators_mask(action, sprites, target, protected);
    [uniform, worst]
}

fn sprites_of(planes: &Planes, action: &ActionFrames) -> HashMap<usize, Sprite> {
    action.directions.iter().flatten().map(|&f| (f, Sprite::of(planes, &planes.boxes[f]))).collect()
}

fn measure_body(planes: &Planes, sequence: Option<&AnimationSequence>) -> BodyResult {
    let kinds = sequence.map(kinds_of).unwrap_or_default();
    let size_class = size_class_of(planes);
    let actions = actions_of(planes);
    let mut masks: [Vec<Vec<bool>>; 2] = [Vec::with_capacity(actions.len()), Vec::with_capacity(actions.len())];
    let mut stored: [StoredFrame; 2] = Default::default();
    let mut boxes: [Vec<u8>; 2] = Default::default();
    let mut scores = Vec::new();
    let (mut untouched_frames, mut protected_frames) = (0u64, 0u64);
    for action in &actions {
        let sprites = sprites_of(planes, action);
        let protected = protected_of(sequence, action.action);
        let kind = match sequence {
            None => KIND_NO_SEQUENCE,
            Some(_) => kinds.get(&u32::from(action.action)).copied().unwrap_or(KIND_OTHER),
        };
        let count = action.frames_per_direction();
        if count <= UNTOUCHED_FRAMES {
            untouched_frames += action.directions.iter().map(|d| d.len() as u64).sum::<u64>();
        } else {
            protected_frames += action.directions.iter().map(|d| protected.iter().filter(|&&n| (n as usize) < d.len() && n % 2 == 1 && n as usize != count - 1).count() as u64).sum::<u64>();
        }
        let selected = masks_of(action, &sprites, &protected);
        for (selection, mask) in selected.iter().enumerate() {
            for (direction, frames) in action.directions.iter().enumerate() {
                let rows: Vec<(usize, usize, usize)> = (0..frames.len())
                    .filter(|&position| !mask[position])
                    .map(|position| {
                        let (previous, next) = neighbours(mask, position, frames.len());
                        (position, previous, next)
                    })
                    .collect();
                let rebuilt: Vec<(usize, [f64; METHOD_NAMES.len()], StoredFrame)> = rows
                    .par_iter()
                    .map(|&(position, previous, next)| {
                        let (a, b, real) = (&sprites[&frames[previous]], &sprites[&frames[next]], &sprites[&frames[position]]);
                        let weight = weight_between(previous, position, next);
                        let searched = stored_search(a, b, real);
                        let values = [score(real, a), score(real, &crossfade(a, b, weight)), score(real, &warp_at_load(a, b, weight)), score(real, &stored_rebuild(a, b, &searched))];
                        (position, values, searched)
                    })
                    .collect();
                for (position, values, searched) in rebuilt {
                    for (method, &value) in values.iter().enumerate() {
                        scores.push(ScoreRow { selection, method, kind, size_class, score: value as f32, place: (action.action, direction, position) });
                    }
                    if let Some((left, top, w, h)) = searched.area {
                        for field in [left, top, w, h] {
                            boxes[selection].extend_from_slice(&field.to_le_bytes());
                        }
                    }
                    stored[selection].sources.extend_from_slice(&searched.sources);
                    stored[selection].xs.extend_from_slice(&searched.xs);
                    stored[selection].ys.extend_from_slice(&searched.ys);
                }
            }
        }
        let [uniform, worst] = selected;
        masks[0].push(uniform);
        masks[1].push(worst);
    }
    let (zstd, rank) = coded_sizes(planes);
    let full = BodySizes { frames: planes.boxes.len() as u64, pixels: planes.index.len() as u64, zstd, rank, vectors: 0, masks: 0 };
    let mut selected = [BodySizes::default(); 2];
    for (selection, out) in selected.iter_mut().enumerate() {
        let kept = kept_planes(planes, &actions, &masks[selection]);
        let (zstd, rank) = coded_sizes(&kept);
        let vectors = [&stored[selection].sources, &stored[selection].xs, &stored[selection].ys, &boxes[selection]]
            .iter()
            .map(|plane| compress(plane, LEVEL_19, false).len() as u64)
            .sum();
        let masks_bytes = masks[selection].iter().map(|m| m.len().div_ceil(u8::BITS as usize) as u64).sum();
        *out = BodySizes { frames: kept.boxes.len() as u64, pixels: kept.index.len() as u64, zstd, rank, vectors, masks: masks_bytes };
    }
    BodyResult { label: planes.label.clone(), number: body_number(planes), full, selected, untouched_frames, protected_frames, scores }
}

// ---------------------------------------------------------------------------------------------------------------
// Reports

fn report_sizes(label: &str, results: &[BodyResult]) {
    let total = |pick: &dyn Fn(&BodyResult) -> BodySizes| {
        let mut sum = BodySizes::default();
        for result in results {
            let s = pick(result);
            sum.frames += s.frames;
            sum.pixels += s.pixels;
            sum.zstd += s.zstd;
            sum.rank += s.rank;
            sum.vectors += s.vectors;
            sum.masks += s.masks;
        }
        sum
    };
    let full = total(&|r| r.full);
    let untouched: u64 = results.iter().map(|r| r.untouched_frames).sum();
    let protected: u64 = results.iter().map(|r| r.protected_frames).sum();
    println!("\n### k8 (LOSSY) {label}: {} bodies, {} frames, {untouched} in actions of {UNTOUCHED_FRAMES} frames or fewer; frame effects protect {protected} frames uniform would drop\n", results.len(), full.frames);
    println!("| selection | kept frames % | kept pixels % | zstd 19 body MB (x full) | rank + previous, action streams MB (x full) | stored vectors MB | keep masks MB |");
    println!("|---|---:|---:|---|---|---:|---:|");
    println!("| full set | 100 | 100 | {:.2} (1.000) | {:.2} (1.000) | - | - |", mb(full.zstd), mb(full.rank));
    for selection in 0..SELECTION_NAMES.len() {
        let s = total(&|r| r.selected[selection]);
        println!(
            "| {} | {:.1} | {:.1} | {:.2} ({:.3}) | {:.2} ({:.3}) | {:.2} | {:.3} |",
            SELECTION_NAMES[selection],
            s.frames as f64 * PERCENT / full.frames.max(1) as f64,
            s.pixels as f64 * PERCENT / full.pixels.max(1) as f64,
            mb(s.zstd),
            s.zstd as f64 / full.zstd.max(1) as f64,
            mb(s.rank),
            s.rank as f64 / full.rank.max(1) as f64,
            mb(s.vectors),
            mb(s.masks)
        );
    }
}

fn report_scores(label: &str, results: &[BodyResult]) {
    println!("\n### k8 (LOSSY) {label}: SSIMULACRA2 of each rebuild against the real dropped frame, over grey 128\n");
    println!("| selection | group | method | frames | median | p5 | min |");
    println!("|---|---|---|---:|---:|---:|---:|");
    let print = |selection: usize, group: &str, filter: &dyn Fn(&ScoreRow) -> bool| {
        for method in 0..METHOD_NAMES.len() {
            let values: Vec<f64> =
                results.iter().flat_map(|r| &r.scores).filter(|row| row.selection == selection && row.method == method && filter(row)).map(|row| f64::from(row.score)).collect();
            if values.is_empty() {
                continue;
            }
            let values = sorted(values);
            println!(
                "| {} | {group} | {} | {} | {:.1} | {:.1} | {:.1} |",
                SELECTION_NAMES[selection],
                METHOD_NAMES[method],
                values.len(),
                percentile(&values, MEDIAN),
                percentile(&values, P5),
                values[0]
            );
        }
    };
    for selection in 0..SELECTION_NAMES.len() {
        print(selection, "all", &|_| true);
        let kinds: BTreeSet<&str> = results.iter().flat_map(|r| &r.scores).map(|row| row.kind).collect();
        for kind in kinds {
            print(selection, kind, &|row| row.kind == kind);
        }
        for class in ["small frames", "medium frames", "large frames"] {
            print(selection, class, &|row| row.size_class == class);
        }
    }
}

fn report_cpu(label: &str, sample: &[&Planes]) {
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    println!("\n### k8 {label}: CPU per dropped frame and per action (5 directions), one thread; CPU {}; load average {}\n", cpu_state(), load_average());
    let mut per_frame: [Vec<f64>; 3] = Default::default();
    let mut per_action: [Vec<f64>; 3] = Default::default();
    single.install(|| {
        for planes in sample {
            let actions = actions_of(planes);
            for action in actions.iter().filter(|a| a.frames_per_direction() > UNTOUCHED_FRAMES).take(TIMED_ACTIONS_PER_BODY) {
                let sprites = sprites_of(planes, action);
                let mask = uniform_mask(action.frames_per_direction(), &BTreeSet::new());
                let mut totals = [0f64; 3];
                for frames in &action.directions {
                    for position in (0..frames.len()).filter(|&p| !mask[p]) {
                        let (previous, next) = neighbours(&mask, position, frames.len());
                        let (a, b, real) = (&sprites[&frames[previous]], &sprites[&frames[next]], &sprites[&frames[position]]);
                        let weight = weight_between(previous, position, next);
                        let searched = stored_search(a, b, real);
                        let timed: [f64; 3] = [
                            { let started = Instant::now(); black_box(crossfade(a, b, weight)); started.elapsed().as_secs_f64() },
                            { let started = Instant::now(); black_box(warp_at_load(a, b, weight)); started.elapsed().as_secs_f64() },
                            { let started = Instant::now(); black_box(stored_rebuild(a, b, &searched)); started.elapsed().as_secs_f64() },
                        ];
                        for (method, seconds) in timed.iter().enumerate() {
                            per_frame[method].push(*seconds);
                            totals[method] += seconds;
                        }
                    }
                }
                for (method, total) in totals.iter().enumerate() {
                    per_action[method].push(*total);
                }
            }
        }
    });
    println!("| method | per frame median / p95 / max ms | per action median / p95 / max ms |\n|---|---|---|");
    for (method, name) in ["crossfade", "block warp at load (search and copy)", "block warp, stored vectors (copy)"].iter().enumerate() {
        let frame = sorted(per_frame[method].clone());
        let action = sorted(per_action[method].clone());
        if frame.is_empty() {
            continue;
        }
        println!(
            "| {name} | {:.2} / {:.2} / {:.2} | {:.1} / {:.1} / {:.1} |",
            ms(percentile(&frame, MEDIAN)),
            ms(percentile(&frame, P95)),
            ms(*frame.last().unwrap()),
            ms(percentile(&action, MEDIAN)),
            ms(percentile(&action, P95)),
            ms(*action.last().unwrap())
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Contact sheets

fn crc32(chunks: &[&[u8]]) -> u32 {
    let mut crc = u32::MAX;
    for chunk in chunks {
        for &byte in *chunk {
            crc ^= u32::from(byte);
            for _ in 0..u8::BITS {
                crc = if crc & 1 != 0 { (crc >> 1) ^ CRC_POLYNOMIAL } else { crc >> 1 };
            }
        }
    }
    !crc
}

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) {
    let mut encoder = ZlibEncoder::new(Vec::with_capacity(rgb.len() / 2), Compression::default());
    for row in rgb.chunks_exact(w * 3) {
        encoder.write_all(&[0]).expect("zlib");
        encoder.write_all(row).expect("zlib");
    }
    let data = encoder.finish().expect("zlib");
    let mut header = Vec::new();
    header.extend_from_slice(&(w as u32).to_be_bytes());
    header.extend_from_slice(&(h as u32).to_be_bytes());
    header.extend_from_slice(&[PNG_BIT_DEPTH, PNG_COLOUR_RGB, 0, 0, 0]);
    let mut out = PNG_SIGNATURE.to_vec();
    for (name, body) in [(b"IHDR", header.as_slice()), (b"IDAT", data.as_slice()), (b"IEND", &[][..])] {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(body);
        out.extend_from_slice(&crc32(&[name, body]).to_be_bytes());
    }
    std::fs::write(path, out).expect("write a contact sheet");
}

/// One dropped frame's cells: the previous kept frame, the real one, the next kept one, the four rebuilds. The row below
/// holds each cell's difference from the real frame, gained.
pub(super) struct SheetRow {
    pub(super) cells: [Sprite; SHEET_COLUMNS],
}

fn sheet_row(sprites: &HashMap<usize, Sprite>, frames: &[usize], mask: &[bool], position: usize) -> SheetRow {
    let (previous, next) = neighbours(mask, position, frames.len());
    let (a, b, real) = (&sprites[&frames[previous]], &sprites[&frames[next]], &sprites[&frames[position]]);
    let weight = weight_between(previous, position, next);
    let searched = stored_search(a, b, real);
    SheetRow { cells: [a.clone(), real.clone(), b.clone(), a.clone(), crossfade(a, b, weight), warp_at_load(a, b, weight), stored_rebuild(a, b, &searched)] }
}

const REAL_COLUMN: usize = 1;

pub(super) fn write_sheet(path: &Path, rows: &[SheetRow]) {
    if rows.is_empty() {
        return;
    }
    let mut area = rows[0].cells[0].area();
    for row in rows {
        for cell in &row.cells {
            area = area.union(cell.area());
        }
    }
    let scale = if area.w.max(area.h) > SHEET_LARGE_SIDE { SHEET_SCALE_LARGE } else { SHEET_SCALE_SMALL };
    let (cell_w, cell_h) = (area.w * scale, area.h * scale);
    let columns = SHEET_COLUMNS as i32;
    let (w, h) = (columns * (cell_w + SHEET_GAP) + SHEET_GAP, rows.len() as i32 * 2 * (cell_h + SHEET_GAP) + SHEET_GAP);
    let mut rgb = vec![0u8; (w * h) as usize * 3];
    for pixel in rgb.chunks_exact_mut(3) {
        pixel.copy_from_slice(&SHEET_GAP_COLOUR);
    }
    for (r, row) in rows.iter().enumerate() {
        let real = &row.cells[REAL_COLUMN];
        for (c, cell) in row.cells.iter().enumerate() {
            let x0 = SHEET_GAP + c as i32 * (cell_w + SHEET_GAP);
            let y0 = SHEET_GAP + r as i32 * 2 * (cell_h + SHEET_GAP);
            for y in 0..area.h {
                for x in 0..area.w {
                    let p = over_grey(cell.at(area.left + x, area.top + y));
                    let q = over_grey(real.at(area.left + x, area.top + y));
                    let difference = [0, 1, 2].map(|i| (i32::from(p[i].abs_diff(q[i])) * SHEET_DIFFERENCE_GAIN).min(i32::from(OPAQUE)) as u8);
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let picture = (((y0 + y * scale + sy) * w + x0 + x * scale + sx) * 3) as usize;
                            rgb[picture..picture + 3].copy_from_slice(&p);
                            let below = (((y0 + cell_h + SHEET_GAP + y * scale + sy) * w + x0 + x * scale + sx) * 3) as usize;
                            rgb[below..below + 3].copy_from_slice(&difference);
                        }
                    }
                }
            }
        }
    }
    write_png(path, w as usize, h as usize, &rgb);
}

pub(super) fn slug(label: &str) -> String {
    label.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn write_kind_sheets(planes: &Planes, sequence: Option<&AnimationSequence>) {
    let kinds = sequence.map(kinds_of).unwrap_or_default();
    let actions = actions_of(planes);
    for kind in SHEET_KINDS {
        let Some(action) = actions.iter().find(|a| kinds.get(&u32::from(a.action)) == Some(&kind) && a.frames_per_direction() > UNTOUCHED_FRAMES) else { continue };
        let sprites = sprites_of(planes, action);
        let mask = uniform_mask(action.frames_per_direction(), &protected_of(sequence, action.action));
        let frames = &action.directions[action.directions.len() / 2];
        let rows: Vec<SheetRow> =
            (0..frames.len()).filter(|&p| !mask[p]).take(SHEET_ROWS).map(|position| sheet_row(&sprites, frames, &mask, position)).collect();
        write_sheet(&Path::new(OUTPUT_DIR).join(format!("{}_{kind}_action{}.png", slug(&planes.label), action.action)), &rows);
    }
}

fn write_worst_sheet(planes: &Planes, row: &ScoreRow, sequence: Option<&AnimationSequence>) {
    let actions = actions_of(planes);
    let Some(action) = actions.iter().find(|a| a.action == row.place.0) else { return };
    let sprites = sprites_of(planes, action);
    let protected = protected_of(sequence, action.action);
    let mask = &masks_of(action, &sprites, &protected)[row.selection];
    let frames = &action.directions[row.place.1];
    let name = format!("worst_{}_{}_action{}_score{:.0}.png", slug(METHOD_NAMES[row.method]), slug(&planes.label), row.place.0, row.score);
    write_sheet(&Path::new(OUTPUT_DIR).join(name), &[sheet_row(&sprites, frames, mask, row.place.2)]);
}

const SHEET_README: &str = "Contact sheets of the halved-frames measurement (LOSSY, a tier to judge, not a default).\n\
Each dropped frame takes two rows. Columns, left to right:\n\
1 previous kept frame, 2 the real dropped frame, 3 next kept frame, 4 hold previous, 5 crossfade (off palette),\n\
6 block warp computed at load, 7 block warp with stored vectors.\n\
The row below shows each cell's difference from the real frame, gain 4, so column 2 is black.\n\
Frames over grey 128, nearest-neighbour upscale.\n\
<body>_<kind>_action<N>.png: a walk, an attack and an idle; the middle stored direction; uniform selection.\n\
worst_<method>_<body>_action<N>_score<S>.png: the lowest SSIMULACRA2 of that method in the sample.\n";

// ---------------------------------------------------------------------------------------------------------------

pub(super) fn run(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let ec_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client folder"));
    std::fs::create_dir_all(OUTPUT_DIR).expect("the contact sheet folder");
    std::fs::write(Path::new(OUTPUT_DIR).join("README.txt"), SHEET_README).expect("the contact sheet notes");
    println!("\n### k8 (LOSSY): halved frames, every {stride}-th body; CPU {}; load average {}", cpu_state(), load_average());
    let ec_sequences = load_sequences(&ec_dir);
    let cc_sequences = load_sequences(cc_dir);
    report_rates("Enhanced", &ec_sequences);
    report_rates("Classic", &cc_sequences);
    let ec_map = sequence_of_frames_body(&ec_sequences, &ec_dir);
    let cc_map = sequence_of_frames_body(&cc_sequences, cc_dir);

    let groups: [(&str, Vec<(Client, u64, usize)>, &BTreeMap<u32, AnimationSequence>, &HashMap<u32, u32>); 2] = [
        ("Enhanced", present.iter().copied().filter(|s| s.0 == Client::Ec).collect(), &ec_sequences, &ec_map),
        ("Classic UOP", present.iter().copied().filter(|s| matches!(bodies[s.2], BodySource::CcUop { .. })).collect(), &cc_sequences, &cc_map),
    ];
    for (label, of, sequences, map) in groups {
        let sample: Vec<usize> = spread(&of, of.len().div_ceil(stride), |s| s.1).into_iter().map(|s| s.2).collect();
        let sequence_for = |planes: &Planes| body_number(planes).and_then(|n| map.get(&n)).and_then(|id| sequences.get(id));
        let started = Instant::now();
        let results: Vec<BodyResult> = sample
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                let result = measure_body(&planes, sequence_for(&planes));
                println!("  {} measured", result.label);
                result
            })
            .collect();
        println!("{label}: {} bodies measured in {:.0} s", results.len(), started.elapsed().as_secs_f64());
        let with_sequence = results.iter().filter(|r| r.number.and_then(|n| map.get(&n)).is_some()).count();
        println!("{with_sequence} of them have a sequence through the tile art or their own id");
        report_sizes(label, &results);
        report_scores(label, &results);

        // Contact sheets: bodies spread over the sample, and the worst scorer of each method.
        let chosen: Vec<usize> = (0..SHEET_BODIES).map(|n| sample[n * (sample.len() - 1) / (SHEET_BODIES - 1).max(1)]).collect();
        chosen.par_iter().for_each(|&i| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            write_kind_sheets(&planes, sequence_for(&planes));
        });
        for method in 0..METHOD_NAMES.len() {
            let worst = results
                .iter()
                .enumerate()
                .flat_map(|(n, r)| r.scores.iter().map(move |row| (n, row)))
                .filter(|(_, row)| row.method == method)
                .min_by(|a, b| a.1.score.total_cmp(&b.1.score));
            if let Some((n, row)) = worst {
                let planes = Planes::from_body(&load_body(sources, &bodies[sample[n]]));
                println!("  worst of {}: {} action {} direction {} frame {}, {:.1} ({})", METHOD_NAMES[method], planes.label, row.place.0, row.place.1, row.place.2, row.score, row.kind);
                write_worst_sheet(&planes, row, sequence_for(&planes));
                if method == METHOD_WARP_LOAD {
                    write_kind_sheets(&planes, sequence_for(&planes));
                }
            }
        }

        // CPU cost, one thread, nothing else running.
        let timed: Vec<usize> = (0..TIMED_BODIES).map(|n| sample[n * (sample.len() - 1) / (TIMED_BODIES - 1).max(1)]).collect();
        let timed: Vec<Planes> = timed.into_iter().map(|i| Planes::from_body(&load_body(sources, &bodies[i]))).collect();
        report_cpu(label, &timed.iter().collect::<Vec<_>>());
    }
}
