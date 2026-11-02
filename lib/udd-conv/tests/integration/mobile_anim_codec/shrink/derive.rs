//! Part `s5` of the `shrink` pass: Classic's `AnimationFrame*.uop` against the Enhanced client's, the same (body, action,
//! direction, frame). Do the shapes match, do the colours match after reducing the Enhanced to RGB555, and what would
//! storing one as a derivation of the other save when both packages are installed. Measure only.

use super::knowledge::directions_in_action;
use super::*;

const CHANNEL_555_SHIFT: u32 = 3;
const OFFSET_SEARCH: i32 = 4;
/// Every n-th shared frame looks for a better alignment than the anchor's.
const OFFSET_SAMPLE_EVERY: usize = 20;
const COVERAGE_LEVELS: usize = COVERAGE_SYMBOLS;

type FrameKey = (u16, u32, usize);

#[derive(Default, Clone)]
struct Match {
    bodies: u64,
    frames_cc: u64,
    frames_ec: u64,
    shared: u64,
    same_box: u64,
    same_shape: u64,
    inter: u64,
    union: u64,
    colour_equal: u64,
    colour_near: u64,
    index_equal: u64,
    channel_error: u64,
    offset_tested: u64,
    offset_better: u64,
    iou_anchor: f64,
    iou_best: f64,
    /// Share of the Classic pixels drawn, per Enhanced coverage level.
    drawn_by_coverage: [u64; COVERAGE_LEVELS],
    pixels_by_coverage: [u64; COVERAGE_LEVELS],
    cc_plain: u64,
    cc_derived: u64,
    ec_plain: u64,
    ec_derived: u64,
}

impl Match {
    fn add(&mut self, other: &Match) {
        self.bodies += other.bodies;
        self.frames_cc += other.frames_cc;
        self.frames_ec += other.frames_ec;
        self.shared += other.shared;
        self.same_box += other.same_box;
        self.same_shape += other.same_shape;
        self.inter += other.inter;
        self.union += other.union;
        self.colour_equal += other.colour_equal;
        self.colour_near += other.colour_near;
        self.index_equal += other.index_equal;
        self.channel_error += other.channel_error;
        self.offset_tested += other.offset_tested;
        self.offset_better += other.offset_better;
        self.iou_anchor += other.iou_anchor;
        self.iou_best += other.iou_best;
        self.drawn_by_coverage.iter_mut().zip(&other.drawn_by_coverage).for_each(|(a, b)| *a += b);
        self.pixels_by_coverage.iter_mut().zip(&other.pixels_by_coverage).for_each(|(a, b)| *a += b);
        self.cc_plain += other.cc_plain;
        self.cc_derived += other.cc_derived;
        self.ec_plain += other.ec_plain;
        self.ec_derived += other.ec_derived;
    }
}

fn frames_by_key(planes: &Planes) -> HashMap<FrameKey, usize> {
    let directions = directions_in_action(&planes.boxes);
    let mut counts = HashMap::<(u16, u32), usize>::new();
    let mut out = HashMap::new();
    for (n, frame) in planes.boxes.iter().enumerate() {
        let count = counts.entry((frame.action, directions[n])).or_default();
        out.insert((frame.action, directions[n], *count), n);
        *count += 1;
    }
    out
}

fn intersection_over_union(cc: &Planes, a: &FrameBox, ec: &Planes, b: &FrameBox, ox: i32, oy: i32) -> f64 {
    let (left, top) = (a.left.min(b.left + ox), a.top.min(b.top + oy));
    let (right, bottom) = ((a.left + a.w as i32).max(b.left + ox + b.w as i32), (a.top + a.h as i32).max(b.top + oy + b.h as i32));
    let (mut inter, mut union) = (0u64, 0u64);
    for y in top..bottom {
        for x in left..right {
            let p = opaque_at(cc, a, x, y);
            let q = opaque_at(ec, b, x - ox, y - oy);
            inter += u64::from(p && q);
            union += u64::from(p || q);
        }
    }
    inter as f64 / union.max(1) as f64
}

#[inline(always)]
fn offset_in(frame: &FrameBox, x: i32, y: i32) -> Option<usize> {
    let (px, py) = (x - frame.left, y - frame.top);
    (px >= 0 && py >= 0 && px < frame.w as i32 && py < frame.h as i32).then(|| frame.start + py as usize * frame.w + px as usize)
}

#[inline(always)]
fn opaque_at(planes: &Planes, frame: &FrameBox, x: i32, y: i32) -> bool {
    offset_in(frame, x, y).is_some_and(|i| planes.is_opaque(i))
}

#[inline(always)]
fn to_555(colour: &[u8; EC_PALETTE_ENTRY_BYTES]) -> [u8; 3] {
    [colour[0] >> CHANNEL_555_SHIFT, colour[1] >> CHANNEL_555_SHIFT, colour[2] >> CHANNEL_555_SHIFT]
}

fn compare_body(cc: &Planes, ec: &Planes) -> Match {
    let mut m = Match { bodies: 1, frames_cc: cc.boxes.len() as u64, frames_ec: ec.boxes.len() as u64, ..Match::default() };
    let ec_frames = frames_by_key(ec);
    let cc_frames = frames_by_key(cc);
    let mut keys: Vec<(FrameKey, usize)> = cc_frames.iter().map(|(k, &v)| (*k, v)).collect();
    keys.sort_unstable();
    let mut tested = 0usize;
    for (key, n) in &keys {
        let Some(&e) = ec_frames.get(key) else { continue };
        let (a, b) = (&cc.boxes[*n], &ec.boxes[e]);
        m.shared += 1;
        m.same_box += u64::from(a.w == b.w && a.h == b.h && a.left == b.left && a.top == b.top);
        let (cc_colours, ec_colours) = (&cc.palettes[a.palette], &ec.palettes[b.palette]);
        let (left, top) = (a.left.min(b.left), a.top.min(b.top));
        let (right, bottom) = ((a.left + a.w as i32).max(b.left + b.w as i32), (a.top + a.h as i32).max(b.top + b.h as i32));
        let mut same = true;
        for y in top..bottom {
            for x in left..right {
                let (i, j) = (offset_in(a, x, y), offset_in(b, x, y));
                let p = i.is_some_and(|i| cc.is_opaque(i));
                let q = j.is_some_and(|j| ec.is_opaque(j));
                if let Some(j) = j {
                    let level = usize::from(ec.coverage[j]);
                    m.pixels_by_coverage[level] += 1;
                    m.drawn_by_coverage[level] += u64::from(p);
                }
                m.union += u64::from(p || q);
                same &= p == q;
                if !(p && q) {
                    continue;
                }
                m.inter += 1;
                let (i, j) = (i.unwrap(), j.unwrap());
                let (s, t) = (to_555(&cc_colours[usize::from(cc.index[i])]), to_555(&ec_colours[usize::from(ec.index[j])]));
                let largest = (0..3).map(|c| s[c].abs_diff(t[c])).max().unwrap();
                m.colour_equal += u64::from(largest == 0);
                m.colour_near += u64::from(largest <= 1);
                m.channel_error += u64::from(largest);
                m.index_equal += u64::from(cc.index[i] == ec.index[j]);
            }
        }
        m.same_shape += u64::from(same);
        tested += 1;
        if tested % OFFSET_SAMPLE_EVERY == 0 {
            let anchor = intersection_over_union(cc, a, ec, b, 0, 0);
            let mut best = anchor;
            for oy in -OFFSET_SEARCH..=OFFSET_SEARCH {
                for ox in -OFFSET_SEARCH..=OFFSET_SEARCH {
                    best = best.max(intersection_over_union(cc, a, ec, b, ox, oy));
                }
            }
            m.offset_tested += 1;
            m.offset_better += u64::from(best > anchor);
            m.iou_anchor += anchor;
            m.iou_best += best;
        }
    }
    let (cc_plain, cc_derived) = derived_size(cc, ec, &cc_frames, &ec_frames);
    let (ec_plain, ec_derived) = derived_size(ec, cc, &ec_frames, &cc_frames);
    m.cc_plain = cc_plain;
    m.cc_derived = cc_derived;
    m.ec_plain = ec_plain;
    m.ec_derived = ec_derived;
    m
}

/// Per target palette, the nearest entry of each colour of the reference, lazily.
fn nearest_entry(colours: &[[u8; EC_PALETTE_ENTRY_BYTES]], first: usize, colour: &[u8; EC_PALETTE_ENTRY_BYTES], cache: &mut HashMap<[u8; 3], u8>) -> u8 {
    let key = [colour[0], colour[1], colour[2]];
    *cache.entry(key).or_insert_with(|| {
        (first..colours.len().min(BYTE_SYMBOLS)).min_by_key(|&e| colour_distance(&colours[e], colour)).unwrap_or(first) as u8
    })
}

/// The target's index plane (Enhanced: also coverage) as plain Zstd 19, whole body, in luma order; and coded against the
/// reference at the same place: the difference from the nearest palette entry to the reference's colour, and the shape
/// as a difference from the reference's shape.
fn derived_size(target: &Planes, reference: &Planes, target_frames: &HashMap<FrameKey, usize>, reference_frames: &HashMap<FrameKey, usize>) -> (u64, u64) {
    let ordered = reordered(target, Order::Luma);
    let plain = compress_level(&ordered.index, LEVEL_19, false).len() as u64;
    let first = knowledge::first_entry(target.client);
    let mut caches = HashMap::<usize, HashMap<[u8; 3], u8>>::new();
    let mut residual = ordered.index.clone();
    // Classic's shape is in its index plane (0): the shape differences are kept apart so the plane stays comparable.
    let mut shape = vec![0u8; ordered.index.len()];
    let mut coded_keys: Vec<(&FrameKey, &usize)> = target_frames.iter().collect();
    coded_keys.sort_unstable();
    for (key, &n) in coded_keys {
        let Some(&r) = reference_frames.get(key) else { continue };
        let (a, b) = (&ordered.boxes[n], &reference.boxes[r]);
        let colours = &ordered.palettes[a.palette];
        let source = &reference.palettes[b.palette];
        let cache = caches.entry(a.palette).or_default();
        for y in 0..a.h as i32 {
            for x in 0..a.w as i32 {
                let i = a.start + y as usize * a.w + x as usize;
                let at = offset_in(b, a.left + x, a.top + y);
                let there = at.filter(|&j| reference.is_opaque(j));
                let here = ordered.is_opaque(i);
                shape[i] = u8::from(here != there.is_some());
                residual[i] = match (here, there) {
                    (true, Some(j)) => ordered.index[i].wrapping_sub(nearest_entry(colours, first, &source[usize::from(reference.index[j])], cache)),
                    (true, None) => ordered.index[i],
                    (false, _) => 0,
                };
            }
        }
    }
    let mut derived = compress_level(&residual, LEVEL_19, false).len() as u64;
    // The Enhanced shape stays in its coverage plane, which this doesn't touch.
    if target.client == Client::Cc {
        derived += compress_level(&packed_bits(shape.iter().map(|&s| s != 0)), LEVEL_19, false).len() as u64;
    }
    (plain, derived)
}

pub(super) fn part_s5(sources: &Sources, bodies: &[BodySource]) {
    println!("\n### s5: Classic AnimationFrame*.uop against the Enhanced, same (body, action, direction, frame); CPU {}", cpu_state());
    let ec: HashMap<u32, usize> = bodies
        .iter()
        .enumerate()
        .filter_map(|(i, body)| match body {
            BodySource::Ec { body, .. } => Some((*body, i)),
            _ => None,
        })
        .collect();
    let pairs: Vec<(usize, usize, u32, usize)> = bodies
        .iter()
        .enumerate()
        .filter_map(|(i, body)| match body {
            BodySource::CcUop { body, file, .. } => ec.get(body).map(|&e| (i, e, *body, *file)),
            _ => None,
        })
        .collect();
    let classic_uop = bodies.iter().filter(|b| matches!(b, BodySource::CcUop { .. })).count();
    println!("{classic_uop} Classic UOP bodies, {} Enhanced bodies, {} in both", ec.len(), pairs.len());
    let rows: Vec<(u32, usize, Match)> = pairs
        .par_iter()
        .map(|&(c, e, body, file)| {
            let cc = Planes::from_body(&load_body(sources, &bodies[c]));
            let en = Planes::from_body(&load_body(sources, &bodies[e]));
            (body, file, compare_body(&cc, &en))
        })
        .collect();
    let mut all = Match::default();
    rows.iter().for_each(|r| all.add(&r.2));
    let percent = |part: u64, of: u64| part as f64 / of.max(1) as f64 * PERCENT;
    println!("Bodies compared: {}; frames Classic UOP {}, Enhanced {}, shared {}", all.bodies, all.frames_cc, all.frames_ec, all.shared);
    println!("Same box and anchor: {:.1} % of shared frames; exactly the same shape: {:.1} %", percent(all.same_box, all.shared), percent(all.same_shape, all.shared));
    println!("Pixels opaque in both, of those opaque in either: {:.1} %", percent(all.inter, all.union));
    println!("On those pixels: same RGB555 {:.1} %, within one 5-bit step on every channel {:.1} %; same index {:.1} %; mean largest error {:.2} steps", percent(all.colour_equal, all.inter), percent(all.colour_near, all.inter), percent(all.index_equal, all.inter), all.channel_error as f64 / all.inter.max(1) as f64);
    println!(
        "Alignment (every {OFFSET_SAMPLE_EVERY}-th shared frame, {} tested, offsets up to {OFFSET_SEARCH} px): mean overlap at the anchor {:.3}, at the best offset {:.3}; better elsewhere in {:.1} %",
        all.offset_tested,
        all.iou_anchor / all.offset_tested.max(1) as f64,
        all.iou_best / all.offset_tested.max(1) as f64,
        percent(all.offset_better, all.offset_tested),
    );
    println!("\nClassic drawn, by Enhanced coverage at the same place:\n\n| coverage | Enhanced pixels (M) | drawn in Classic |\n|---:|---:|---:|");
    for level in 0..COVERAGE_LEVELS {
        println!("| {level} | {:.1} | {:.1} % |", all.pixels_by_coverage[level] as f64 / PIXELS_PER_MEGAPIXEL, percent(all.drawn_by_coverage[level], all.pixels_by_coverage[level]));
    }
    println!(
        "\nSizes, index plane and shape, Zstd 19 per body, luma order, MB:\nClassic UOP alone {:.1}; coded against the Enhanced {:.1} ({:+.1} %)\nEnhanced alone {:.1}; coded against Classic {:.1} ({:+.1} %)",
        mb(all.cc_plain),
        mb(all.cc_derived),
        (all.cc_derived as f64 / all.cc_plain.max(1) as f64 - 1.0) * PERCENT,
        mb(all.ec_plain),
        mb(all.ec_derived),
        (all.ec_derived as f64 / all.ec_plain.max(1) as f64 - 1.0) * PERCENT,
    );
    let mut worst: Vec<&(u32, usize, Match)> = rows.iter().collect();
    worst.sort_by(|a, b| percent(a.2.colour_equal, a.2.inter).total_cmp(&percent(b.2.colour_equal, b.2.inter)));
    println!("\nThe bodies by colour match, a spread:\n\n| body | Classic file | shared frames | same shape | same RGB555 | Classic coded against the Enhanced |\n|---:|---:|---:|---:|---:|---:|");
    for n in 0..BODY_ROWS.min(worst.len()) {
        let (body, file, m) = worst[n * (worst.len() - 1) / (BODY_ROWS - 1).max(1)];
        println!("| {body} | {file} | {} | {:.1} % | {:.1} % | {:+.1} % |", m.shared, percent(m.same_shape, m.shared), percent(m.colour_equal, m.inter), (m.cc_derived as f64 / m.cc_plain.max(1) as f64 - 1.0) * PERCENT);
    }
}

const BODY_ROWS: usize = 12;
