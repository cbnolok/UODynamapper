//! Part `s4` of the `shrink` pass: why the lossless floor is 5.5 to 5.7 bits per opaque pixel. On surfaces that do not
//! move between two frames, how much do colours change, does the change look like quantisation noise or dither, and how
//! much of the index plane's entropy does it account for. Part `s4q`, LOSSY: what removing it would save.

use super::halved::{self, KIND_IDLE, Sprite};
use super::knowledge::{self, RankTables, first_entry};
use super::*;
use uocf::animation_sequence::AnimationSequence;

const RANK_CELL_BITS: u32 = 5;
const WINDOW_RADIUS: i32 = 2;
const WINDOW_PIXELS: u64 = 25;
/// A pixel is on a static surface when its 5x5 window is solid in both frames, with this mean largest channel change.
const STATIC_MEAN_CHANGE: u64 = 8;
const CHANGE_TOPS: [u8; 8] = [0, 1, 2, 4, 8, 16, 32, 255];
const RANK_TOPS: [u8; 5] = [1, 2, 4, 8, 255];
/// Largest channel difference for a one-pixel ripple to count as a dither candidate.
const RIPPLE_NEAR: u8 = 16;
const SPACING_BUCKETS: usize = 256;
const CLASS_IDLE: usize = 0;
const CLASS_OTHER: usize = 1;
const LUMA_RED: i32 = 77;
const LUMA_GREEN: i32 = 150;
const LUMA_BLUE: i32 = 29;

const QUALITY_BODIES_PER_CLIENT: usize = 25;
const QUALITY_THRESHOLDS: [i32; 4] = [2, 4, 8, 16];
/// Every n-th changed frame is scored.
const QUALITY_SCORE_EVERY: usize = 3;
const P5: f64 = 0.05;
const QUALITY_ZSTD: Candidate = candidate("z19 action", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Action);
const QUALITY_SPLIT: Candidate = candidate("tANS action", Order::Original, IndexCoding::Split(crowd::split(Predictor::Med, true, crowd::SplitCoder::Tans, false)), COVERAGE_ZSTD, Unit::Action);

struct Tally {
    pairs: u64,
    common: u64,
    opaque: u64,
    still: u64,
    same_index: u64,
    same_colour: u64,
    change: [u64; CHANGE_TOPS.len()],
    rank: [u64; RANK_TOPS.len()],
    horizontal: Correlation,
    vertical: Correlation,
    temporal: Correlation,
    interior: u64,
    ripples: u64,
    checkers: u64,
    /// By context and residual, in luma order.
    med_still: Vec<u32>,
    med_rest: Vec<u32>,
    from_previous: Vec<u32>,
}

impl Default for Tally {
    fn default() -> Self {
        Tally {
            pairs: 0,
            common: 0,
            opaque: 0,
            still: 0,
            same_index: 0,
            same_colour: 0,
            change: [0; CHANGE_TOPS.len()],
            rank: [0; RANK_TOPS.len()],
            horizontal: Correlation::default(),
            vertical: Correlation::default(),
            temporal: Correlation::default(),
            interior: 0,
            ripples: 0,
            checkers: 0,
            med_still: vec![0; INDEX_CONTEXTS * BYTE_SYMBOLS],
            med_rest: vec![0; INDEX_CONTEXTS * BYTE_SYMBOLS],
            from_previous: vec![0; BYTE_SYMBOLS],
        }
    }
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.pairs += other.pairs;
        self.common += other.common;
        self.opaque += other.opaque;
        self.still += other.still;
        self.same_index += other.same_index;
        self.same_colour += other.same_colour;
        self.change.iter_mut().zip(&other.change).for_each(|(a, b)| *a += b);
        self.rank.iter_mut().zip(&other.rank).for_each(|(a, b)| *a += b);
        self.horizontal.add(&other.horizontal);
        self.vertical.add(&other.vertical);
        self.temporal.add(&other.temporal);
        self.interior += other.interior;
        self.ripples += other.ripples;
        self.checkers += other.checkers;
        self.med_still.iter_mut().zip(&other.med_still).for_each(|(a, b)| *a += b);
        self.med_rest.iter_mut().zip(&other.med_rest).for_each(|(a, b)| *a += b);
        self.from_previous.iter_mut().zip(&other.from_previous).for_each(|(a, b)| *a += b);
    }
}

#[derive(Default, Clone, Copy)]
struct Correlation {
    ab: f64,
    aa: f64,
    bb: f64,
}

impl Correlation {
    #[inline(always)]
    fn push(&mut self, a: i32, b: i32) {
        self.ab += f64::from(a) * f64::from(b);
        self.aa += f64::from(a) * f64::from(a);
        self.bb += f64::from(b) * f64::from(b);
    }

    fn add(&mut self, other: &Correlation) {
        self.ab += other.ab;
        self.aa += other.aa;
        self.bb += other.bb;
    }

    fn value(&self) -> f64 {
        self.ab / (self.aa * self.bb).sqrt().max(1.0)
    }
}

/// Total bits of symbols under their own empirical distribution, per context of `alphabet` symbols.
fn entropy_bits(hist: &[u32], alphabet: usize) -> f64 {
    hist.chunks_exact(alphabet)
        .map(|symbols| {
            let total: f64 = symbols.iter().map(|&c| f64::from(c)).sum();
            symbols.iter().filter(|&&c| c > 0).map(|&c| -f64::from(c) * (f64::from(c) / total).log2()).sum::<f64>()
        })
        .sum()
}

#[inline(always)]
fn luma_of(colour: &[u8; EC_PALETTE_ENTRY_BYTES]) -> i32 {
    (i32::from(colour[0]) * LUMA_RED + i32::from(colour[1]) * LUMA_GREEN + i32::from(colour[2]) * LUMA_BLUE) >> u8::BITS
}

#[inline(always)]
fn largest_change(a: &[u8; EC_PALETTE_ENTRY_BYTES], b: &[u8; EC_PALETTE_ENTRY_BYTES]) -> u8 {
    a[0].abs_diff(b[0]).max(a[1].abs_diff(b[1])).max(a[2].abs_diff(b[2]))
}

struct PaletteFacts {
    actions: u64,
    frames: u64,
    palettes_in_actions: u64,
    distinct_in_actions: u64,
    used_entries: Vec<u32>,
    /// Largest channel distance from each used entry to its nearest other used entry.
    spacing: [u64; SPACING_BUCKETS],
}

fn palette_facts(planes: &Planes) -> PaletteFacts {
    let mut facts = PaletteFacts { actions: 0, frames: planes.boxes.len() as u64, palettes_in_actions: 0, distinct_in_actions: 0, used_entries: Vec::new(), spacing: [0; SPACING_BUCKETS] };
    let mut per_action = BTreeMap::<u16, HashSet<usize>>::new();
    let mut used = vec![[false; BYTE_SYMBOLS]; planes.palettes.len()];
    for frame in &planes.boxes {
        per_action.entry(frame.action).or_default().insert(frame.palette);
        for i in frame.range() {
            if planes.is_opaque(i) {
                used[frame.palette][usize::from(planes.index[i])] = true;
            }
        }
    }
    for palettes in per_action.values() {
        facts.actions += 1;
        facts.palettes_in_actions += palettes.len() as u64;
        let distinct: HashSet<&Vec<[u8; EC_PALETTE_ENTRY_BYTES]>> = palettes.iter().map(|&p| &planes.palettes[p]).collect();
        facts.distinct_in_actions += distinct.len() as u64;
    }
    for (colours, used) in planes.palettes.iter().zip(&used) {
        let entries: Vec<usize> = (0..BYTE_SYMBOLS).filter(|&e| used[e]).collect();
        if entries.is_empty() {
            continue;
        }
        facts.used_entries.push(entries.len() as u32);
        for &e in &entries {
            let nearest = entries.iter().filter(|&&o| o != e && colours[o][..3] != colours[e][..3]).map(|&o| largest_change(&colours[e], &colours[o])).min();
            if let Some(nearest) = nearest {
                facts.spacing[usize::from(nearest)] += 1;
            }
        }
    }
    facts
}

/// Summed-area lookups over one frame.
struct Summed {
    w: i32,
    table: Vec<u64>,
}

impl Summed {
    fn new(w: usize, h: usize, value: impl Fn(usize) -> u64) -> Summed {
        let mut table = vec![0u64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0u64;
            for x in 0..w {
                row += value(y * w + x);
                table[(y + 1) * (w + 1) + x + 1] = table[y * (w + 1) + x + 1] + row;
            }
        }
        Summed { w: w as i32 + 1, table }
    }

    #[inline(always)]
    fn window(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> u64 {
        let at = |x: i32, y: i32| self.table[(y * self.w + x) as usize];
        at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)
    }
}

fn measure_body(planes: &Planes, idle_actions: &HashSet<u16>) -> [Tally; 2] {
    let ordered = reordered(planes, Order::Luma);
    let mut tallies = [Tally::default(), Tally::default()];
    let is_solid = |i: usize| match ordered.client {
        Client::Cc => ordered.index[i] != 0,
        Client::Ec => ordered.coverage[i] == COVERAGE_SOLID,
    };
    let all: Vec<usize> = (0..ordered.boxes.len()).collect();
    let (index, mask) = unit_planes(&ordered, &all);
    let mut ranks: HashMap<usize, RankTables> = HashMap::new();
    let mut action_of_ranks = u16::MAX;
    // The signed luma changes of the frame before, for the temporal correlation.
    let mut last_changes: Option<(usize, Vec<i32>, Vec<bool>)> = None;
    for (n, frame) in ordered.boxes.iter().enumerate() {
        let tally = &mut tallies[if idle_actions.contains(&frame.action) { CLASS_IDLE } else { CLASS_OTHER }];
        let colours = &ordered.palettes[frame.palette];
        let (w, h) = (frame.w as i32, frame.h as i32);
        let at = |x: i32, y: i32| frame.start + (y * w + x) as usize;
        let before = (n > 0 && ordered.boxes[n - 1].direction == frame.direction).then(|| &ordered.boxes[n - 1]);
        let mut changes = vec![0i32; frame.pixels()];
        let mut largest = vec![0u8; frame.pixels()];
        let mut common = vec![false; frame.pixels()];
        if let Some(before) = before {
            tally.pairs += 1;
            let (dx, dy) = (before.left - frame.left, before.top - frame.top);
            let colours_before = &ordered.palettes[before.palette];
            for y in 0..h {
                for x in 0..w {
                    let (px, py) = (x - dx, y - dy);
                    if px < 0 || py < 0 || px >= before.w as i32 || py >= before.h as i32 {
                        continue;
                    }
                    let (i, j) = (at(x, y), before.start + (py as usize) * before.w + px as usize);
                    if is_solid(i) && is_solid(j) {
                        let (a, b) = (&colours[usize::from(ordered.index[i])], &colours_before[usize::from(ordered.index[j])]);
                        let local = (y * w + x) as usize;
                        common[local] = true;
                        largest[local] = largest_change(a, b);
                        changes[local] = luma_of(a) - luma_of(b);
                    }
                }
            }
        }
        let solid_windows = Summed::new(frame.w, frame.h, |i| u64::from(common[i]));
        let change_windows = Summed::new(frame.w, frame.h, |i| u64::from(largest[i]));
        let mut still = vec![false; frame.pixels()];
        for y in WINDOW_RADIUS..h - WINDOW_RADIUS {
            for x in WINDOW_RADIUS..w - WINDOW_RADIUS {
                let (x0, y0, x1, y1) = (x - WINDOW_RADIUS, y - WINDOW_RADIUS, x + WINDOW_RADIUS + 1, y + WINDOW_RADIUS + 1);
                if solid_windows.window(x0, y0, x1, y1) == WINDOW_PIXELS && change_windows.window(x0, y0, x1, y1) <= WINDOW_PIXELS * STATIC_MEAN_CHANGE {
                    still[(y * w + x) as usize] = true;
                }
            }
        }
        if frame.action != action_of_ranks {
            ranks.clear();
            action_of_ranks = frame.action;
        }
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            for x in 0..w {
                let i = at(x, y);
                if mask[i] == 0 {
                    continue;
                }
                tally.opaque += 1;
                let (med, context) = predict(&index, &mask, frame.w, i, x as usize, y > 0, Predictor::Med, last);
                last = index[i];
                let local = (y * w + x) as usize;
                let symbol = context * BYTE_SYMBOLS + usize::from(index[i].wrapping_sub(med));
                tally.common += u64::from(common[local]);
                if !still[local] {
                    tally.med_rest[symbol] += 1;
                    continue;
                }
                tally.med_still[symbol] += 1;
                tally.still += 1;
                tally.change[CHANGE_TOPS.iter().position(|&top| largest[local] <= top).unwrap()] += 1;
                let before = before.unwrap();
                let j = before.start + ((y - (before.top - frame.top)) as usize) * before.w + (x - (before.left - frame.left)) as usize;
                if before.palette == frame.palette {
                    tally.from_previous[usize::from(index[i].wrapping_sub(index[j]))] += 1;
                    if index[i] == index[j] {
                        tally.same_index += 1;
                    } else {
                        let tables = ranks.entry(frame.palette).or_insert_with(|| RankTables::new(colours, first_entry(ordered.client), true, RANK_CELL_BITS));
                        let rank = tables.rank_of(usize::from(index[j]), index[i]);
                        tally.rank[RANK_TOPS.iter().position(|&top| rank <= top).unwrap()] += 1;
                    }
                }
                tally.same_colour += u64::from(largest[local] == 0);
                if x + 1 < w && still[local + 1] {
                    tally.horizontal.push(changes[local], changes[local + 1]);
                }
                if y + 1 < h && still[local + frame.w] {
                    tally.vertical.push(changes[local], changes[local + frame.w]);
                }
                if let Some((previous_frame, previous_changes, previous_still)) = &last_changes {
                    if *previous_frame == n - 1 {
                        let earlier = &ordered.boxes[n - 1];
                        let (px, py) = (x - (earlier.left - frame.left), y - (earlier.top - frame.top));
                        if px >= 0 && py >= 0 && px < earlier.w as i32 && py < earlier.h as i32 {
                            let p = (py as usize) * earlier.w + px as usize;
                            if previous_still[p] {
                                tally.temporal.push(changes[local], previous_changes[p]);
                            }
                        }
                    }
                }
            }
        }
        // Dither patterns inside the solid part of the frame.
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = at(x, y);
                if !(is_solid(i) && is_solid(i - 1) && is_solid(i + 1) && is_solid(at(x, y - 1)) && is_solid(at(x, y + 1)) && is_solid(at(x + 1, y + 1))) {
                    continue;
                }
                tally.interior += 1;
                let (left, right, centre) = (ordered.index[i - 1], ordered.index[i + 1], ordered.index[i]);
                if left == right && left != centre && largest_change(&colours[usize::from(left)], &colours[usize::from(centre)]) <= RIPPLE_NEAR {
                    tally.ripples += 1;
                }
                let (below, diagonal) = (ordered.index[at(x, y + 1)], ordered.index[at(x + 1, y + 1)]);
                if centre == diagonal && right == below && centre != right {
                    tally.checkers += 1;
                }
            }
        }
        last_changes = Some((n, changes, still));
    }
    tallies
}

fn idle_of(planes: &Planes, source: &BodySource, sequence: Option<&AnimationSequence>, kinds: &HashMap<u32, String>) -> HashSet<u16> {
    match (source, sequence) {
        (BodySource::CcMul { .. }, _) | (_, None) => HashSet::from([knowledge::stand_action(planes, kinds)]),
        (_, Some(sequence)) => halved::kinds_of(sequence).into_iter().filter(|&(_, kind)| kind == KIND_IDLE).map(|(action, _)| action as u16).collect(),
    }
}

pub(super) fn part_s4(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let ec_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client folder"));
    println!("\n### s4: what the lossless floor is made of, every {stride}-th body; CPU {}", cpu_state());
    let kinds = body_kinds(cc_dir);
    let ec_sequences = halved::load_sequences(&ec_dir);
    let cc_sequences = halved::load_sequences(cc_dir);
    let ec_map = halved::sequence_of_frames_body(&ec_sequences, &ec_dir);
    let cc_map = halved::sequence_of_frames_body(&cc_sequences, cc_dir);
    for client in [Client::Cc, Client::Ec] {
        let of: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client).collect();
        let sample: Vec<usize> = spread(&of, of.len().div_ceil(stride), |s| s.1).into_iter().map(|s| s.2).collect();
        let results: Vec<([Tally; 2], PaletteFacts)> = sample
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                let number = halved::body_number(&planes);
                let sequence = match &bodies[i] {
                    BodySource::CcMul { .. } => None,
                    BodySource::CcUop { .. } => number.and_then(|n| cc_map.get(&n)).and_then(|id| cc_sequences.get(id)),
                    BodySource::Ec { .. } => number.and_then(|n| ec_map.get(&n)).and_then(|id| ec_sequences.get(id)),
                };
                let idle = idle_of(&planes, &bodies[i], sequence, &kinds);
                (measure_body(&planes, &idle), palette_facts(&planes))
            })
            .collect();
        let mut tallies = [Tally::default(), Tally::default()];
        for (body, _) in &results {
            tallies[CLASS_IDLE].add(&body[CLASS_IDLE]);
            tallies[CLASS_OTHER].add(&body[CLASS_OTHER]);
        }
        print_palette_facts(client, &results);
        println!("\n| {} | idle | other actions |\n|---|---:|---:|", client.label());
        let row = |name: &str, value: &dyn Fn(&Tally) -> String| println!("| {name} | {} | {} |", value(&tallies[CLASS_IDLE]), value(&tallies[CLASS_OTHER]));
        let percent = |part: u64, of: u64| format!("{:.1} %", part as f64 / of.max(1) as f64 * PERCENT);
        row("opaque Mpx", &|t| format!("{:.1}", t.opaque as f64 / PIXELS_PER_MEGAPIXEL));
        row("solid in both frames, of opaque", &|t| percent(t.common, t.opaque));
        row("on a static surface, of opaque", &|t| percent(t.still, t.opaque));
        row("static: same index", &|t| percent(t.same_index, t.still));
        row("static: same colour", &|t| percent(t.same_colour, t.still));
        for (n, top) in CHANGE_TOPS.iter().enumerate() {
            row(&format!("static: largest channel change up to {top}"), &|t| percent(t.change[n], t.still));
        }
        for (n, top) in RANK_TOPS.iter().enumerate() {
            row(&format!("static and changed: new colour within the {top} nearest palette entries"), &|t| percent(t.rank[n], t.rank.iter().sum()));
        }
        row("change correlation with the next pixel (real surface change: positive; noise: about 0)", &|t| format!("{:.3}", t.horizontal.value()));
        row("change correlation with the pixel below", &|t| format!("{:.3}", t.vertical.value()));
        row("change correlation with the same pixel's previous change (frame noise: about -0.5)", &|t| format!("{:.3}", t.temporal.value()));
        row("one-pixel ripples per 1000 solid pixels", &|t| format!("{:.1}", t.ripples as f64 / t.interior.max(1) as f64 * BYTES_PER_KB));
        row("2x2 checkers per 1000 solid pixels", &|t| format!("{:.1}", t.checkers as f64 / t.interior.max(1) as f64 * BYTES_PER_KB));
        let bits = |t: &Tally| (entropy_bits(&t.med_still, BYTE_SYMBOLS), entropy_bits(&t.med_rest, BYTE_SYMBOLS));
        row("bits per pixel, MED with contexts, static pixels", &|t| format!("{:.2}", bits(t).0 / t.still.max(1) as f64));
        row("bits per pixel, MED with contexts, the others", &|t| format!("{:.2}", bits(t).1 / (t.opaque - t.still).max(1) as f64));
        row("bits per pixel, from the previous frame's index, static pixels", &|t| format!("{:.2}", entropy_bits(&t.from_previous, BYTE_SYMBOLS) / t.from_previous.iter().map(|&c| u64::from(c)).sum::<u64>().max(1) as f64));
        row("share of the index bits spent on static pixels", &|t| {
            let (still, rest) = bits(t);
            format!("{:.1} %", still / (still + rest).max(1.0) * PERCENT)
        });
    }
}

fn print_palette_facts(client: Client, results: &[([Tally; 2], PaletteFacts)]) {
    let (mut actions, mut frames, mut palettes, mut distinct) = (0, 0, 0, 0);
    let mut used = Vec::new();
    let mut spacing = [0u64; SPACING_BUCKETS];
    for (_, facts) in results {
        actions += facts.actions;
        frames += facts.frames;
        palettes += facts.palettes_in_actions;
        distinct += facts.distinct_in_actions;
        used.extend_from_slice(&facts.used_entries);
        spacing.iter_mut().zip(&facts.spacing).for_each(|(a, b)| *a += b);
    }
    used.sort_unstable();
    let total: u64 = spacing.iter().sum();
    let mut running = 0;
    let median_spacing = spacing.iter().position(|&c| {
        running += c;
        running * 2 >= total
    });
    println!(
        "\n{}: {actions} actions, {frames} frames; {:.2} palettes per action, {:.2} distinct by content; palettes per frame {:.3}; used entries per palette, median {}; median distance from a used entry to its nearest other colour {:?} (largest channel)",
        client.label(),
        palettes as f64 / actions.max(1) as f64,
        distinct as f64 / actions.max(1) as f64,
        palettes as f64 / frames.max(1) as f64,
        used.get(used.len() / 2).copied().unwrap_or(0),
        median_spacing,
    );
}

// ---------------------------------------------------------------------------------------------------------------
// s4q, LOSSY: the same surfaces made to repeat

/// Each pixel takes the previous frame's index at the same anchor-relative place when the colour is within `threshold`,
/// with the same hue flag and the same greyness. With `merge`, entries within the threshold first become one, within a
/// palette, so a frame's own near-duplicates go too.
fn stabilised(planes: &Planes, threshold: i32, merge: bool, repeat: bool) -> Planes {
    let mut out = planes.clone();
    let near = |a: &[u8; EC_PALETTE_ENTRY_BYTES], b: &[u8; EC_PALETTE_ENTRY_BYTES]| {
        i32::from(largest_change(a, b)) <= threshold && a[PALETTE_FLAG_COLUMN] == b[PALETTE_FLAG_COLUMN] && (a[0] == a[1] && a[1] == a[2]) == (b[0] == b[1] && b[1] == b[2])
    };
    if merge {
        let first = first_entry(planes.client);
        let remap: Vec<Vec<u8>> = planes
            .palettes
            .iter()
            .map(|colours| {
                let mut map: Vec<u8> = (0..BYTE_SYMBOLS).map(|e| e as u8).collect();
                let mut kept: Vec<usize> = Vec::new();
                for e in first..colours.len().min(BYTE_SYMBOLS) {
                    if let Some(&k) = kept.iter().find(|&&k| near(&colours[k], &colours[e])) {
                        map[e] = k as u8;
                    } else {
                        kept.push(e);
                    }
                }
                map
            })
            .collect();
        for frame in &planes.boxes {
            for i in frame.range() {
                if planes.is_opaque(i) {
                    out.index[i] = remap[frame.palette][usize::from(planes.index[i])];
                }
            }
        }
    }
    if repeat {
        for n in 1..planes.boxes.len() {
            let (before, frame) = (&planes.boxes[n - 1], &planes.boxes[n]);
            if before.direction != frame.direction || before.palette != frame.palette {
                continue;
            }
            let colours = &planes.palettes[frame.palette];
            let (dx, dy) = (before.left - frame.left, before.top - frame.top);
            for y in 0..frame.h as i32 {
                for x in 0..frame.w as i32 {
                    let i = frame.start + y as usize * frame.w + x as usize;
                    let (px, py) = (x - dx, y - dy);
                    if !planes.is_opaque(i) || px < 0 || py < 0 || px >= before.w as i32 || py >= before.h as i32 {
                        continue;
                    }
                    let j = before.start + py as usize * before.w + px as usize;
                    if planes.is_opaque(j) && near(&colours[usize::from(out.index[j])], &colours[usize::from(planes.index[i])]) {
                        out.index[i] = out.index[j];
                    }
                }
            }
        }
    }
    out
}

struct QualityRow {
    zstd: u64,
    split: u64,
    changed: u64,
    opaque: u64,
    worst_channel: u8,
    scores: Vec<f64>,
}

fn quality_of(planes: &Planes, reference: &Planes) -> QualityRow {
    let zstd = encode_candidate(planes, &QUALITY_ZSTD);
    let split = encode_candidate(planes, &QUALITY_SPLIT);
    verify_round_trip(planes, &split, &QUALITY_SPLIT);
    let mut row = QualityRow { zstd: zstd.index_bytes(), split: split.index_bytes(), changed: 0, opaque: 0, worst_channel: 0, scores: Vec::new() };
    let mut seen_changed = 0usize;
    for frame in &reference.boxes {
        let colours = &reference.palettes[frame.palette];
        let mut frame_changed = false;
        for i in frame.range() {
            if reference.is_opaque(i) {
                row.opaque += 1;
                let change = largest_change(&colours[usize::from(reference.index[i])], &colours[usize::from(planes.index[i])]);
                row.worst_channel = row.worst_channel.max(change);
                if change > 0 {
                    row.changed += 1;
                    frame_changed = true;
                }
            }
        }
        if frame_changed {
            seen_changed += 1;
            if seen_changed % QUALITY_SCORE_EVERY == 0 {
                let (real, rebuilt): (Sprite, Sprite) = (Sprite::of(reference, frame), Sprite::of(planes, frame));
                row.scores.push(halved::score(&real, &rebuilt));
            }
        }
    }
    row
}

pub(super) fn part_s4q(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)]) {
    println!("\n### s4q (LOSSY): static surfaces made to repeat; CPU {}", cpu_state());
    for client in [Client::Cc, Client::Ec] {
        let sample = sample_of(present, client, QUALITY_BODIES_PER_CLIENT);
        // Name, merge entries, repeat from the previous frame, threshold.
        let mut variants: Vec<(String, bool, bool, i32)> = vec![("lossless".to_string(), false, false, 0)];
        for threshold in QUALITY_THRESHOLDS {
            variants.push((format!("repeat, up to {threshold}"), false, true, threshold));
            variants.push((format!("merged palette, up to {threshold}"), true, false, threshold));
            variants.push((format!("merged palette and repeat, up to {threshold}"), true, true, threshold));
        }
        let rows: Vec<Vec<QualityRow>> = sample
            .par_iter()
            .map(|&i| {
                let planes = reordered(&Planes::from_body(&load_body(sources, &bodies[i])), Order::Luma);
                variants.iter().map(|&(_, merge, repeat, threshold)| quality_of(&stabilised(&planes, threshold, merge, repeat), &planes)).collect()
            })
            .collect();
        println!("\n#### {}, {} bodies spread by size; action streams, index only, luma palette order\n", client.label(), sample.len());
        println!("| variant | zstd 19 MB | MED, 10 streams, tANS MB | pixels changed | largest channel error | SSIMULACRA2 median | p5 | min |");
        println!("|---|---:|---:|---:|---:|---:|---:|---:|");
        for (n, (name, ..)) in variants.iter().enumerate() {
            let zstd: u64 = rows.iter().map(|r| r[n].zstd).sum();
            let split: u64 = rows.iter().map(|r| r[n].split).sum();
            let changed: u64 = rows.iter().map(|r| r[n].changed).sum();
            let opaque: u64 = rows.iter().map(|r| r[n].opaque).sum();
            let worst = rows.iter().map(|r| r[n].worst_channel).max().unwrap_or(0);
            let mut scores: Vec<f64> = rows.iter().flat_map(|r| r[n].scores.iter().copied()).collect();
            scores.sort_by(f64::total_cmp);
            let pick = |q: f64| scores.get((scores.len() as f64 * q) as usize).copied().unwrap_or(SSIMULACRA2_NO_CHANGE);
            println!(
                "| {name} | {:.2} | {:.2} | {:.1} % | {worst} | {:.1} | {:.1} | {:.1} |",
                mb(zstd),
                mb(split),
                changed as f64 / opaque.max(1) as f64 * PERCENT,
                pick(MEDIAN),
                pick(P5),
                scores.first().copied().unwrap_or(SSIMULACRA2_NO_CHANGE),
            );
        }
    }
}

const SSIMULACRA2_NO_CHANGE: f64 = 100.0;
const MEDIAN: f64 = 0.5;
const PALETTE_FLAG_COLUMN: usize = 3;
