//! Part `k` of the `shrink` pass: what knowledge of the data buys, past treating frames as generic images. The items,
//! as parts `k1`..`k7` (`k` is all of them):
//! 1. what the client can never draw: bodies in several sources, and what the def files reach;
//! 2. rank coding through the palette's real colours: a pixel as its rank in the list of entries by RGB distance from
//!    a reference entry;
//! 3. a reference action: streams compressed against the same direction's stand frames;
//! 4. the frame order inside a body stream;
//! 5. better reference frames: how many pixels match exactly under each;
//! 6. coverage with contexts from its structure;
//! 7. equipment layers: their share of the cost, and the shape coded with the base body's as context.
//!
//! Part `r` (only on request): where rank coding's decode time goes, stage by stage, on one thread.
//!
//! Every coder here is decoded and compared with its source: a mismatch fails the test.

use super::*;

/// Bodies of each stand-action range in `anim.mul` (Bodyconv.def's header): monsters, animals, then people.
const FIRST_ANIMAL_BODY: u32 = 200;
const FIRST_PEOPLE_BODY: u32 = 400;
/// Stand actions by kind of body. An assumption from the groups' usual layout (not read from a client): a body
/// without that action takes its first one.
const STAND_ACTION_MONSTER: u16 = 1;
const STAND_ACTION_ANIMAL: u16 = 2;
const STAND_ACTION_PEOPLE: u16 = 4;
const KIND_MONSTER: &str = "MONSTER";
const KIND_SEA_MONSTER: &str = "SEA_MONSTER";
const KIND_ANIMAL: &str = "ANIMAL";
pub(super) const KIND_HUMAN: &str = "HUMAN";
pub(super) const KIND_EQUIPMENT: &str = "EQUIPMENT";
const BODY_DEF_FILE: &str = "Body.def";
/// The `mobtypes.txt` flag `BodyUsesUopFrames` tests.
const MOBTYPES_UOP_FLAG: u32 = 0x10000;
const MOBTYPES_LAST_BODY: u32 = 0x7FF;
const MOBTYPES_KINDS: [&str; 5] = [KIND_MONSTER, KIND_SEA_MONSTER, KIND_ANIMAL, KIND_HUMAN, KIND_EQUIPMENT];
const HEX_RADIX: u32 = 16;
const BODYCONV_DEF_FILE: &str = "Bodyconv.def";
/// `<body> <anim2> <anim3> <anim4> <anim5>`: the client drops a line with fewer numbers.
const BODYCONV_NUMBERS: usize = 5;
/// Enhanced bodies with frames but no creature placeholder: the client cannot show them (`uosa_animation.md`).
const EC_BODIES_NO_PLACEHOLDER: [u32; 2] = [344, 827];
/// Enhanced body whose single-frame actions ship ten frames, of which the client shows one (`uosa_animation.md`).
const EC_BODY_EXCESS_FRAMES: u32 = 679;
const EC_BODY_EXCESS_FACTOR: f64 = 0.9;

const COLOUR_CHANNELS: usize = 3;
const NEAREST_CELL_BITS: u32 = 5;
/// [`RankMode::Coarse`]: a 16x16x16 lookup, a tenth of the table and fewer cells to fill.
const NEAREST_COARSE_CELL_BITS: u32 = 4;
const NEAREST_EMPTY: u16 = u16::MAX;
/// Sums of three channels and two gradients, back to the scale of the index gradient buckets.
const RGB_GRADIENT_DIVISOR: u32 = 4;
const COLOUR_NEAR_LIMIT: u8 = 8;
const TRANSPARENT_COLOUR: u32 = u32::MAX;
/// Earlier frames of a direction compared with each frame (the action's own are always included).
const EARLIER_FRAMES_LIMIT: usize = 64;
/// Horizontal offsets tried for a mirrored frame: `x' = offset - x`.
const MIRROR_OFFSETS: std::ops::RangeInclusive<i32> = -3..=3;
const EDGE_DISTANCE_CLASSES: usize = 4;
const LUMA_CLASSES: usize = 4;
const LUMA_MAX: u32 = 255_000;
const PREVIOUS_COVERAGE_CLASSES: usize = 3;
const PARTIAL_COVERAGE_SYMBOLS: usize = COVERAGE_SYMBOLS - 1;
const BASE_BODIES: [u32; 2] = [400, 401];
const WITH_BASE_MASK_CONTEXTS: usize = MASK_CONTEXTS * 2;

/// The direction of each frame within its action. `FrameBox::direction` is the position of the (action, direction)
/// pair in the body, which no other action or body shares. A direction missing from an action shifts the later ones.
pub(super) fn directions_in_action(boxes: &[FrameBox]) -> Vec<u32> {
    let mut seen = HashMap::<u16, (u32, u32)>::new();
    boxes
        .iter()
        .map(|frame| {
            let state = seen.entry(frame.action).or_insert((frame.direction, 0));
            if state.0 != frame.direction {
                *state = (frame.direction, state.1 + 1);
            }
            state.1
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// 1. What the client can never draw

/// The numbers of each Bodyconv.def line, a -1 column being none: (body, the file index per column).
pub(super) fn bodyconv_rows(cc_dir: &Path) -> Vec<(u32, Vec<(u8, u32)>)> {
    let Some(path) = find_ignoring_case(cc_dir, BODYCONV_DEF_FILE) else { return Vec::new() };
    let Ok(text) = std::fs::read(&path) else { return Vec::new() };
    let mut rows = Vec::new();
    for line in String::from_utf8_lossy(&text).lines() {
        let line = line.split('#').next().unwrap_or("");
        let numbers: Vec<i32> = line.split_whitespace().map_while(|word| word.parse().ok()).collect();
        if numbers.len() < BODYCONV_NUMBERS {
            continue;
        }
        let targets = numbers[1..BODYCONV_NUMBERS]
            .iter()
            .enumerate()
            .filter(|(_, target)| **target >= 0)
            .map(|(column, &target)| (column as u8 + 1, target as u32))
            .collect();
        rows.push((numbers[0] as u32, targets));
    }
    rows
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Place {
    Mul(usize),
    Uop(usize),
}

struct ActionRow {
    action: u16,
    direction: u32,
    raw: u64,
    /// Over the frames' places, sizes and colours.
    colours: u64,
    compressed: f64,
}

struct SourceRow {
    place: Place,
    body: u32,
    actions: Vec<ActionRow>,
    compressed: u64,
}

fn frame_hash(planes: &Planes, frame: &FrameBox, hasher: &mut DefaultHasher) {
    (frame.w, frame.h, frame.left, frame.top).hash(hasher);
    let palette = &planes.palettes[frame.palette];
    for i in frame.range() {
        if planes.is_opaque(i) {
            palette[usize::from(planes.index[i])][..COLOUR_CHANNELS].hash(hasher);
        } else {
            COVERAGE_TRANSPARENT.hash(hasher);
        }
    }
}

fn source_row(sources: &Sources, source: &BodySource) -> Option<SourceRow> {
    let (place, body) = match source {
        BodySource::CcMul { file, body, .. } => (Place::Mul(*file), u32::from(*body)),
        BodySource::CcUop { file, body, .. } => (Place::Uop(*file), *body),
        BodySource::Ec { .. } => return None,
    };
    let planes = Planes::from_body(&load_body(sources, source));
    let compressed = compress(&planes.index, LEVEL_19, false).len() as u64;
    let total = planes.index.len().max(1) as f64;
    let mut groups = BTreeMap::<(u16, u32), (u64, DefaultHasher)>::new();
    for (frame, &direction) in planes.boxes.iter().zip(&directions_in_action(&planes.boxes)) {
        let (raw, hasher) = groups.entry((frame.action, direction)).or_insert_with(|| (0, DefaultHasher::new()));
        *raw += frame.pixels() as u64;
        frame_hash(&planes, frame, hasher);
    }
    let actions = groups
        .into_iter()
        .map(|((action, direction), (raw, hasher))| ActionRow {
            action,
            direction,
            raw,
            colours: hasher.finish(),
            compressed: compressed as f64 * raw as f64 / total,
        })
        .collect();
    Some(SourceRow { place, body, actions, compressed })
}

fn part_k1(sources: &Sources, bodies: &[BodySource], cc_dir: &Path) {
    let started = Instant::now();
    let rows: Vec<SourceRow> = bodies.par_iter().filter_map(|source| source_row(sources, source)).collect();
    let total: u64 = rows.iter().map(|r| r.compressed).sum();
    println!("\n### k1: Classic sources, index plane zstd 19 per body, {:.1} MB over {} body batches ({:.0} s)\n", mb(total), rows.len(), started.elapsed().as_secs_f64());
    let mut per_place = BTreeMap::<Place, (usize, u64)>::new();
    for row in &rows {
        let entry = per_place.entry(row.place).or_default();
        entry.0 += 1;
        entry.1 += row.compressed;
    }
    for (place, (count, bytes)) in &per_place {
        println!("  {place:?}: {count} bodies, {:.1} MB", mb(*bytes));
    }

    let conv = bodyconv_rows(cc_dir);
    let mut reached = HashSet::<(usize, u32)>::new();
    let mut conv_bodies = HashSet::<u32>::new();
    for (body, targets) in &conv {
        if !targets.is_empty() {
            conv_bodies.insert(*body);
        }
        reached.extend(targets.iter().map(|&(file, target)| (usize::from(file), target)));
    }
    let remapped: HashSet<u32> = find_ignoring_case(cc_dir, BODY_DEF_FILE)
        .and_then(|path| uocf::classic::body_def::BodyDef::load(path).ok())
        .map(|def| def.iter().map(|(&body, _)| u32::from(body)).collect())
        .unwrap_or_default();
    println!("  Bodyconv.def: {} rows with a target, {} distinct targets; Body.def: {} bodies", conv_bodies.len(), reached.len(), remapped.len());

    // anim.mul bodies the def files take away.
    let anim_mul = Place::Mul(0);
    let sum = |filter: &dyn Fn(&SourceRow) -> bool| -> (usize, u64) {
        let of: Vec<&SourceRow> = rows.iter().filter(|r| filter(r)).collect();
        (of.len(), of.iter().map(|r| r.compressed).sum())
    };
    let line = |name: &str, (count, bytes): (usize, u64)| println!("  {name}: {count} bodies, {:.1} MB", mb(bytes));
    line("anim.mul bodies with a Bodyconv.def target (shadowed when that expansion is enabled)", sum(&|r| r.place == anim_mul && conv_bodies.contains(&r.body)));
    line(
        "anim.mul bodies Body.def remaps, without a Bodyconv.def target (the client does not draw their own art)",
        sum(&|r| r.place == anim_mul && remapped.contains(&r.body) && !conv_bodies.contains(&r.body)),
    );
    line(
        "anim2+.mul bodies no Bodyconv.def column reaches",
        sum(&|r| matches!(r.place, Place::Mul(file) if file > 0 && !reached.contains(&(file, r.body)))),
    );

    // The same body in a MUL and in a UOP: the client picks by body (see the rule below).
    let mut uop_actions = HashMap::<(u32, u16, u32), (u64, f64, Place)>::new();
    let mut uop_duplicates = (0u64, 0f64);
    for row in rows.iter().filter(|r| matches!(r.place, Place::Uop(_))) {
        for action in &row.actions {
            if uop_actions.insert((row.body, action.action, action.direction), (action.colours, action.compressed, row.place)).is_some() {
                uop_duplicates.0 += action.raw;
                uop_duplicates.1 += action.compressed;
            }
        }
    }
    println!("  the same (body, action, direction) in two AnimationFrame*.uop files: {:.1} MB raw, {:.1} MB compressed", mb(uop_duplicates.0), uop_duplicates.1 / BYTES_PER_MB);
    let conv_target: HashMap<(usize, u32), u32> =
        conv.iter().flat_map(|(body, targets)| targets.iter().map(move |&(file, target)| ((usize::from(file), target), *body))).collect();
    // (units, MUL-side compressed, identical units, identical compressed, UOP-side compressed of the same units)
    let mut overlap: BTreeMap<&str, (u64, f64, u64, f64, f64)> = BTreeMap::new();
    let mut overlap_bodies = BTreeMap::<&str, HashSet<(usize, u32)>>::new();
    let mut units_seen = 0u64;
    for row in &rows {
        let Place::Mul(file) = row.place else { continue };
        let candidates: Vec<(&str, u32)> = if file == 0 {
            vec![("anim.mul against the UOP body of the same id", row.body)]
        } else if let Some(&body) = conv_target.get(&(file, row.body)) {
            vec![("anim2+.mul against the UOP of the body whose Bodyconv.def row targets it", body), ("anim2+.mul against the UOP body of the same number (id spaces differ, listed only as a check)", row.body)]
        } else {
            vec![("anim2+.mul against the UOP body of the same number (id spaces differ, listed only as a check)", row.body)]
        };
        for action in &row.actions {
            units_seen += 1;
            for &(name, body) in &candidates {
                if let Some(&(colours, uop_compressed, _)) = uop_actions.get(&(body, action.action, action.direction)) {
                    let entry = overlap.entry(name).or_default();
                    entry.0 += 1;
                    entry.1 += action.compressed;
                    entry.4 += uop_compressed;
                    overlap_bodies.entry(name).or_default().insert((file, row.body));
                    if colours == action.colours {
                        entry.2 += 1;
                        entry.3 += action.compressed;
                    }
                }
            }
        }
    }
    println!("\n  MUL (action, direction) units: {units_seen}. Overlap with a UOP entry, before the client's per-body rule below:\n");
    println!("| overlap | MUL bodies | units | MUL side MB | UOP side MB | identical units | identical MUL MB |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for (name, (units, compressed, same, same_compressed, uop)) in &overlap {
        println!(
            "| {name} | {} | {units} | {:.1} | {:.1} | {same} | {:.1} |",
            overlap_bodies[name].len(),
            compressed / BYTES_PER_MB,
            uop / BYTES_PER_MB,
            same_compressed / BYTES_PER_MB
        );
    }
    // UOP bodies per their MUL presence, whole-body numbers to size either answer.
    let in_mul: HashSet<u32> = rows.iter().filter(|r| r.place == anim_mul).map(|r| r.body).collect();
    line("UOP bodies with no anim.mul body of the same id", sum(&|r| matches!(r.place, Place::Uop(_)) && !in_mul.contains(&r.body)));
    line("UOP bodies with an anim.mul body of the same id", sum(&|r| matches!(r.place, Place::Uop(_)) && in_mul.contains(&r.body)));
    let uop_bodies: HashSet<u32> = rows.iter().filter(|r| matches!(r.place, Place::Uop(_))).map(|r| r.body).collect();
    line("anim.mul bodies with a UOP body of the same id", sum(&|r| r.place == anim_mul && uop_bodies.contains(&r.body)));
    report_source_rule(&rows, &conv, cc_dir);
    // Enhanced, from the client's rule in the frames study.
    let enhanced: Vec<(u32, u64, u64)> = bodies
        .par_iter()
        .filter_map(|source| match source {
            BodySource::Ec { body, .. } if EC_BODIES_NO_PLACEHOLDER.contains(body) || *body == EC_BODY_EXCESS_FRAMES => {
                let planes = Planes::from_body(&load_body(sources, source));
                Some((*body, compress(&planes.index, LEVEL_19, false).len() as u64, compress(&planes.coverage, LEVEL_19, false).len() as u64))
            }
            _ => None,
        })
        .collect();
    for (body, index, coverage) in enhanced {
        let factor = if body == EC_BODY_EXCESS_FRAMES { EC_BODY_EXCESS_FACTOR } else { 1.0 };
        println!("  Enhanced body {body}: {:.2} MB index + {:.2} MB coverage, {:.0} % of it never shown", mb(index), mb(coverage), factor * PERCENT);
    }
}

/// The ids of `mobtypes.txt` the client draws from `AnimationFrame*.uop`, parsed as `LoadMobTypesFile` does: text after
/// `#` is cut, a line is `%d %s %x`, the id is in range and the type known, a later line replaces an earlier one.
fn uop_flagged_bodies(cc_dir: &Path) -> HashSet<u32> {
    let mut flags = HashMap::<u32, u32>::new();
    let text = find_ignoring_case(cc_dir, MOBTYPES_FILE).and_then(|path| std::fs::read(path).ok()).unwrap_or_default();
    for line in String::from_utf8_lossy(&text).lines() {
        let mut words = line.split('#').next().unwrap_or("").split_whitespace();
        let (Some(id), Some(kind), Some(value)) = (words.next(), words.next(), words.next()) else { continue };
        let (Ok(id), Ok(value)) = (id.parse::<u32>(), u32::from_str_radix(value, HEX_RADIX)) else { continue };
        if (1..=MOBTYPES_LAST_BODY).contains(&id) && MOBTYPES_KINDS.contains(&kind) {
            flags.insert(id, value);
        }
    }
    flags.into_iter().filter(|(_, value)| value & MOBTYPES_UOP_FLAG != 0).map(|(id, _)| id).collect()
}

/// Body.def's targets: `<body> {<new body>, ...} <hue>`, one chosen at random by the client.
fn body_def_targets(cc_dir: &Path) -> HashMap<u32, Vec<u32>> {
    let mut targets = HashMap::new();
    let text = find_ignoring_case(cc_dir, BODY_DEF_FILE).and_then(|path| std::fs::read(path).ok()).unwrap_or_default();
    for line in String::from_utf8_lossy(&text).lines() {
        let line = line.split('#').next().unwrap_or("");
        let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else { continue };
        let Ok(body) = line[..open].trim().parse::<u32>() else { continue };
        let list: Vec<u32> = line[open + 1..close].split(',').filter_map(|word| word.trim().parse().ok()).collect();
        targets.insert(body, list);
    }
    targets
}

/// What the client's rule leaves undrawn: `BodyUsesUopFrames` (0x005978a0) sends a body flagged in `mobtypes.txt` to the
/// UOP entry of its id and every other body through Body.def and Bodyconv.def to a MUL, with no fallback either way.
fn report_source_rule(rows: &[SourceRow], conv: &[(u32, Vec<(u8, u32)>)], cc_dir: &Path) {
    let flagged = uop_flagged_bodies(cc_dir);
    let remaps = body_def_targets(cc_dir);
    let conv: HashMap<u32, &Vec<(u8, u32)>> = conv.iter().map(|(body, targets)| (*body, targets)).collect();
    // (file, body) a MUL read can reach, under some expansion setting and with every expansion enabled.
    let (mut any_setting, mut all_enabled) = (HashSet::<(usize, u32)>::new(), HashSet::<(usize, u32)>::new());
    for body in 0..=u32::from(u16::MAX) {
        if flagged.contains(&body) {
            continue;
        }
        let row = conv.get(&body).filter(|targets| !targets.is_empty() && body <= MOBTYPES_LAST_BODY);
        let remapped: &[u32] = remaps.get(&body).map_or(&[], Vec::as_slice);
        // With a Bodyconv.def row for an enabled expansion, Body.def does not apply.
        let enabled: Vec<u32> = if row.is_some() || remapped.is_empty() { vec![body] } else { remapped.to_vec() };
        for &resolved in &enabled {
            match conv.get(&resolved).filter(|targets| !targets.is_empty() && resolved <= MOBTYPES_LAST_BODY) {
                Some(targets) => all_enabled.insert((usize::from(targets[0].0), targets[0].1)),
                None => all_enabled.insert((0, resolved)),
            };
        }
        let mut any: Vec<u32> = remapped.to_vec();
        if remapped.is_empty() || row.is_some() {
            any.push(body);
        }
        for &resolved in &any {
            any_setting.insert((0, resolved));
            if let Some(targets) = conv.get(&resolved).filter(|_| resolved <= MOBTYPES_LAST_BODY) {
                any_setting.extend(targets.iter().map(|&(file, target)| (usize::from(file), target)));
            }
        }
    }
    println!("\n  The client's rule: {} bodies are flagged 0x10000 in mobtypes.txt and use UOP; all others use a MUL.\n", flagged.len());
    println!("| source | bodies | MB | not reached by a mobile's own body, under any expansion setting: bodies | MB | with every expansion enabled: bodies | MB |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    let mut places: Vec<Place> = rows.iter().map(|r| r.place).collect::<HashSet<_>>().into_iter().collect();
    places.sort();
    let (mut lost_any, mut lost_enabled) = (0u64, 0u64);
    for place in places {
        let of: Vec<&SourceRow> = rows.iter().filter(|r| r.place == place).collect();
        let total: u64 = of.iter().map(|r| r.compressed).sum();
        let lost = |reach: &HashSet<(usize, u32)>| -> (usize, u64) {
            let gone: Vec<&&SourceRow> = of
                .iter()
                .filter(|r| match place {
                    Place::Mul(file) => !reach.contains(&(file, r.body)),
                    Place::Uop(_) => !flagged.contains(&r.body),
                })
                .collect();
            (gone.len(), gone.iter().map(|r| r.compressed).sum())
        };
        let (any, enabled) = (lost(&any_setting), lost(&all_enabled));
        lost_any += any.1;
        lost_enabled += enabled.1;
        println!("| {place:?} | {} | {:.1} | {} | {:.1} | {} | {:.1} |", of.len(), mb(total), any.0, mb(any.1), enabled.0, mb(enabled.1));
    }
    let total: u64 = rows.iter().map(|r| r.compressed).sum();
    println!("\n  Not reached by a mobile's own body (other requests are not traced): {:.1} MB of {:.1} MB under any expansion setting; {:.1} MB with every expansion enabled.", mb(lost_any), mb(total), mb(lost_enabled));
    let in_uop: HashSet<u32> = rows.iter().filter(|r| matches!(r.place, Place::Uop(_))).map(|r| r.body).collect();
    let empty = flagged.iter().filter(|body| !in_uop.contains(body)).count();
    println!("  Flagged bodies with no UOP entry, which draw nothing: {empty}.");
}

// ---------------------------------------------------------------------------------------------------------------
// 2. Rank coding

/// The entries by RGB distance from each entry, and the nearest entry of a colour, built as they are first needed.
pub(super) struct RankTables<'a> {
    colours: &'a [[u8; EC_PALETTE_ENTRY_BYTES]],
    first: usize,
    list: Vec<u8>,
    rank: Vec<u8>,
    filled: [bool; BYTE_SYMBOLS],
    nearest: Vec<u16>,
    cell_bits: u32,
    want_rank: bool,
    scratch: Vec<(i32, u16)>,
}

impl<'a> RankTables<'a> {
    pub(super) fn new(colours: &'a [[u8; EC_PALETTE_ENTRY_BYTES]], first: usize, want_rank: bool, cell_bits: u32) -> Self {
        RankTables {
            colours,
            first,
            list: vec![0; BYTE_SYMBOLS * BYTE_SYMBOLS],
            rank: if want_rank { vec![0; BYTE_SYMBOLS * BYTE_SYMBOLS] } else { Vec::new() },
            filled: [false; BYTE_SYMBOLS],
            nearest: Vec::new(),
            cell_bits,
            want_rank,
            scratch: Vec::with_capacity(BYTE_SYMBOLS),
        }
    }

    #[inline(always)]
    fn row(&mut self, reference: usize) {
        if self.filled[reference] {
            return;
        }
        self.filled[reference] = true;
        self.scratch.clear();
        let from = self.colours[reference];
        for entry in self.first..self.colours.len() {
            self.scratch.push((colour_distance(&from, &self.colours[entry]), entry as u16));
        }
        self.scratch.sort_unstable();
        for (rank, &(_, entry)) in self.scratch.iter().enumerate() {
            self.list[reference * BYTE_SYMBOLS + rank] = entry as u8;
            if self.want_rank {
                self.rank[reference * BYTE_SYMBOLS + usize::from(entry)] = rank as u8;
            }
        }
    }

    #[inline(always)]
    pub(super) fn rank_of(&mut self, reference: usize, entry: u8) -> u8 {
        self.row(reference);
        self.rank[reference * BYTE_SYMBOLS + usize::from(entry)]
    }

    #[inline(always)]
    fn entry_at(&mut self, reference: usize, rank: u8) -> u8 {
        self.row(reference);
        self.list[reference * BYTE_SYMBOLS + usize::from(rank)]
    }

    /// The entry nearest the centre of a colour's cell.
    #[inline(always)]
    fn nearest(&mut self, colour: [u8; COLOUR_CHANNELS]) -> usize {
        if self.nearest.is_empty() {
            self.nearest = vec![NEAREST_EMPTY; 1 << (self.cell_bits * COLOUR_CHANNELS as u32)];
        }
        let shift = u8::BITS - self.cell_bits;
        let cell = colour.iter().fold(0usize, |cell, &c| (cell << self.cell_bits) | usize::from(c >> shift));
        if self.nearest[cell] == NEAREST_EMPTY {
            let half = 1u8 << (shift - 1);
            let centre = colour.map(|c| ((c >> shift) << shift) | half);
            let point = [centre[0], centre[1], centre[2], 0];
            let best = (self.first..self.colours.len()).min_by_key(|&e| (colour_distance(&point, &self.colours[e]), e)).unwrap_or(self.first);
            self.nearest[cell] = best as u16;
        }
        usize::from(self.nearest[cell])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RankMode {
    /// (b) The reference is the entry nearest a per-channel MED in RGB.
    Spatial,
    /// (c) The previous frame's colour joins the context.
    PreviousContext,
    /// (c) The previous frame's colour is the reference when known.
    PreviousReference,
    /// As [`RankMode::Spatial`] with a coarser colour lookup, the cheaper decode path of part `r`.
    Coarse,
}

impl RankMode {
    fn cell_bits(self) -> u32 {
        if self == RankMode::Coarse { NEAREST_COARSE_CELL_BITS } else { NEAREST_CELL_BITS }
    }
}

const RANK_MODES: [(RankMode, &str); 3] = [
    (RankMode::Spatial, "b: rank from MED colour, rANS"),
    (RankMode::PreviousContext, "c: as b, previous frame's colour as context, rANS"),
    (RankMode::PreviousReference, "c: rank from the previous frame's colour when known, rANS"),
];

pub(super) fn first_entry(client: Client) -> usize {
    usize::from(client == Client::Cc)
}

fn tables_of<'p, 'a>(tables: &'p mut [Option<RankTables<'a>>], palettes: &'a [Vec<[u8; EC_PALETTE_ENTRY_BYTES]>], palette: usize, first: usize, want_rank: bool, cell_bits: u32) -> &'p mut RankTables<'a> {
    tables[palette].get_or_insert_with(|| RankTables::new(&palettes[palette], first, want_rank, cell_bits))
}

/// (a) The plane with each opaque pixel as its rank from the left pixel; raw after a transparent one.
fn left_rank_plane(planes: &Planes, frames: &[usize], index: &[u8], mask: &[u8]) -> Vec<u8> {
    let first = first_entry(planes.client);
    let mut tables: Vec<Option<RankTables>> = (0..planes.palettes.len()).map(|_| None).collect();
    let mut out = vec![0u8; index.len()];
    let mut start = 0usize;
    for &f in frames {
        let frame = &planes.boxes[f];
        let tables = tables_of(&mut tables, &planes.palettes, frame.palette, first, true, NEAREST_CELL_BITS);
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                out[i] = if x > 0 && mask[i - 1] != 0 {
                    tables.rank_of(usize::from(index[i - 1]), index[i]) + first as u8
                } else {
                    index[i]
                };
            }
        }
        start += frame.pixels();
    }
    out
}

/// Rebuilds [`left_rank_plane`]; Classic's mask comes from the plane itself, the Enhanced one is the coverage.
fn decode_left_rank(boxes: &[FrameBox], palettes: &[Vec<[u8; EC_PALETTE_ENTRY_BYTES]>], client: Client, frames: &[usize], plane: &[u8], coverage: &[u8]) -> Vec<u8> {
    let first = first_entry(client);
    let mut tables: Vec<Option<RankTables>> = (0..palettes.len()).map(|_| None).collect();
    let mut index = vec![0u8; plane.len()];
    let opaque = |i: usize| if client == Client::Cc { plane[i] != 0 } else { coverage[i] != COVERAGE_TRANSPARENT };
    let mut start = 0usize;
    for &f in frames {
        let frame = &boxes[f];
        let tables = tables_of(&mut tables, palettes, frame.palette, first, false, NEAREST_CELL_BITS);
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if !opaque(i) {
                    continue;
                }
                index[i] = if x > 0 && opaque(i - 1) {
                    tables.entry_at(usize::from(index[i - 1]), plane[i] - first as u8)
                } else {
                    plane[i]
                };
            }
        }
        start += frame.pixels();
    }
    index
}

#[inline(always)]
fn rgb(colour: &[u8; EC_PALETTE_ENTRY_BYTES]) -> [u8; COLOUR_CHANNELS] {
    [colour[0], colour[1], colour[2]]
}

/// The prediction of a pixel in RGB, per channel, from the opaque neighbours, with its context.
#[inline(always)]
fn colour_prediction(index: &[u8], mask: &[u8], colours: &[[u8; EC_PALETTE_ENTRY_BYTES]], w: usize, i: usize, x: usize, has_up: bool, last: [u8; COLOUR_CHANNELS]) -> ([u8; COLOUR_CHANNELS], usize) {
    let left = x > 0 && mask[i - 1] != 0;
    let up = has_up && mask[i - w] != 0;
    let colour = |at: usize| rgb(&colours[usize::from(index[at])]);
    match (left, up) {
        (true, true) => {
            let (a, b) = (colour(i - 1), colour(i - w));
            if mask[i - w - 1] != 0 {
                let c = colour(i - w - 1);
                let mut gradient = 0u32;
                let mut prediction = [0u8; COLOUR_CHANNELS];
                for channel in 0..COLOUR_CHANNELS {
                    gradient += u32::from(a[channel].abs_diff(c[channel])) + u32::from(b[channel].abs_diff(c[channel]));
                    prediction[channel] = median_edge(a[channel], b[channel], c[channel]);
                }
                (prediction, gradient_bucket((gradient / RGB_GRADIENT_DIVISOR) as u16))
            } else {
                let mut gradient = 0u32;
                let mut prediction = [0u8; COLOUR_CHANNELS];
                for channel in 0..COLOUR_CHANNELS {
                    gradient += 2 * u32::from(a[channel].abs_diff(b[channel]));
                    prediction[channel] = ((u16::from(a[channel]) + u16::from(b[channel])) / 2) as u8;
                }
                (prediction, gradient_bucket((gradient / RGB_GRADIENT_DIVISOR) as u16))
            }
        }
        (true, false) => (colour(i - 1), CONTEXT_ONE_NEIGHBOUR),
        (false, true) => (colour(i - w), CONTEXT_ONE_NEIGHBOUR),
        (false, false) => (last, CONTEXT_NO_NEIGHBOUR),
    }
}

/// The earlier frame of the same direction in a stream, palette changes allowed.
fn aligned_previous(boxes: &[FrameBox], frames: &[usize]) -> Vec<Option<(PreviousFrame, usize)>> {
    let mut starts = Vec::with_capacity(frames.len());
    let mut offset = 0usize;
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
            Some((
                PreviousFrame { start: starts[n - 1], w: before.w, h: before.h, dx: before.left - frame.left, dy: before.top - frame.top },
                before.palette,
            ))
        })
        .collect()
}

#[inline(always)]
fn previous_colour_bucket(before: Option<[u8; COLOUR_CHANNELS]>, prediction: [u8; COLOUR_CHANNELS]) -> usize {
    let Some(before) = before else { return PREVIOUS_NONE };
    let error = (0..COLOUR_CHANNELS).map(|c| before[c].abs_diff(prediction[c])).max().unwrap_or(0);
    match error {
        0 => PREVIOUS_EQUAL,
        e if e <= COLOUR_NEAR_LIMIT => PREVIOUS_NEAR,
        _ => PREVIOUS_FAR,
    }
}

pub(super) fn rank_contexts(mode: RankMode) -> usize {
    match mode {
        RankMode::Spatial | RankMode::Coarse => INDEX_CONTEXTS,
        _ => INDEX_CONTEXTS * PREVIOUS_BUCKETS,
    }
}

/// The reference entry and context of a pixel, shared by the coder and the decoder.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn rank_step(
    mode: RankMode,
    tables: &mut RankTables,
    palettes: &[Vec<[u8; EC_PALETTE_ENTRY_BYTES]>],
    previous: Option<(PreviousFrame, usize)>,
    index: &[u8],
    mask: &[u8],
    colours: &[[u8; EC_PALETTE_ENTRY_BYTES]],
    (w, i, x, y): (usize, usize, usize, usize),
    last: [u8; COLOUR_CHANNELS],
    palette: usize,
) -> (usize, usize) {
    let (prediction, context) = colour_prediction(index, mask, colours, w, i, x, y > 0, last);
    if matches!(mode, RankMode::Spatial | RankMode::Coarse) {
        return (tables.nearest(prediction), context);
    }
    let before = previous.and_then(|(p, before_palette)| p.pixel(index, mask, x, y).map(|value| (value, before_palette)));
    let before_colour = before.map(|(value, before_palette)| rgb(&palettes[before_palette][usize::from(value)]));
    let context = context * PREVIOUS_BUCKETS + previous_colour_bucket(before_colour, prediction);
    let reference = match (mode, before) {
        (RankMode::PreviousReference, Some((value, before_palette))) if before_palette == palette => usize::from(value),
        (RankMode::PreviousReference, Some(_)) => tables.nearest(before_colour.unwrap_or(prediction)),
        _ => tables.nearest(prediction),
    };
    (reference, context)
}

pub(super) fn rank_symbols(planes: &Planes, frames: &[usize], index: &[u8], mask: &[u8], mode: RankMode) -> Vec<(u16, u8)> {
    let first = first_entry(planes.client);
    let previous = aligned_previous(&planes.boxes, frames);
    let mut tables: Vec<Option<RankTables>> = (0..planes.palettes.len()).map(|_| None).collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    for (n, &f) in frames.iter().enumerate() {
        let frame = &planes.boxes[f];
        let colours = &planes.palettes[frame.palette];
        let tables = tables_of(&mut tables, &planes.palettes, frame.palette, first, true, mode.cell_bits());
        let mut last = [FIRST_PREDICTION; COLOUR_CHANNELS];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (reference, context) = rank_step(mode, tables, &planes.palettes, previous[n], index, mask, colours, (frame.w, i, x, y), last, frame.palette);
                out.push((context as u16, tables.rank_of(reference, index[i])));
                last = rgb(&colours[usize::from(index[i])]);
            }
        }
        start += frame.pixels();
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_rank(stream: &[u8], model: &Model, boxes: &[FrameBox], palettes: &[Vec<[u8; EC_PALETTE_ENTRY_BYTES]>], client: Client, frames: &[usize], mask: &[u8], mode: RankMode) -> Vec<u8> {
    let mut tables: Vec<Option<RankTables>> = (0..palettes.len()).map(|_| None).collect();
    decode_rank_with(&mut tables, stream, model, boxes, palettes, client, frames, mask, mode)
}

#[allow(clippy::too_many_arguments)]
fn decode_rank_with<'a>(tables: &mut [Option<RankTables<'a>>], stream: &[u8], model: &Model, boxes: &[FrameBox], palettes: &'a [Vec<[u8; EC_PALETTE_ENTRY_BYTES]>], client: Client, frames: &[usize], mask: &[u8], mode: RankMode) -> Vec<u8> {
    let first = first_entry(client);
    let previous = aligned_previous(boxes, frames);
    let mut index = vec![0u8; mask.len()];
    let mut decoder = RansDecoder::new(stream);
    let mut start = 0usize;
    for (n, &f) in frames.iter().enumerate() {
        let frame = &boxes[f];
        let colours = &palettes[frame.palette];
        let tables = tables_of(tables, palettes, frame.palette, first, false, mode.cell_bits());
        let mut last = [FIRST_PREDICTION; COLOUR_CHANNELS];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (reference, context) = rank_step(mode, tables, palettes, previous[n], &index, mask, colours, (frame.w, i, x, y), last, frame.palette);
                index[i] = tables.entry_at(reference, decoder.get(model, context));
                last = rgb(&colours[usize::from(index[i])]);
            }
        }
        start += frame.pixels();
    }
    index
}

fn part_k2(sample: &[Planes]) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let opaque: u64 = bodies.iter().map(|p| p.opaque_pixels()).sum();
        println!("\n### k2: {} rank coding, index plane only, {} bodies, {:.1} Mpx opaque\n", client.label(), bodies.len(), opaque as f64 / PIXELS_PER_MEGAPIXEL);
        println!("| coder | unit | MB | bits per opaque px |");
        println!("|---|---|---:|---:|");
        let row = |name: &str, unit: &str, bytes: u64| {
            println!("| {name} | {unit} | {:.2} | {:.2} |", mb(bytes), bytes as f64 * f64::from(u8::BITS) / opaque.max(1) as f64);
        };
        for (unit, name) in [(Unit::Body, "body"), (Unit::Action, "action")] {
            for ranked in [false, true] {
                let bytes: u64 = bodies
                    .par_iter()
                    .map(|planes| {
                        super::units(&planes.boxes, unit, planes.client.pixel_bytes())
                            .iter()
                            .map(|frames| {
                                let (index, mask) = unit_planes(planes, frames);
                                let plane = if ranked { left_rank_plane(planes, frames, &index, &mask) } else { index.clone() };
                                let bytes = compress(&plane, LEVEL_19, false);
                                let plain = decompress(&bytes, plane.len());
                                let decoded = if ranked { decode_left_rank(&planes.boxes, &planes.palettes, client, frames, &plain, &mask) } else { plain };
                                assert!(decoded == index, "{}: left rank does not round-trip", planes.label);
                                bytes.len() as u64
                            })
                            .sum::<u64>()
                    })
                    .sum();
                row(if ranked { "a: rank from the left pixel, zstd 19" } else { "plain index plane, zstd 19" }, name, bytes);
            }
            for (mode, coder) in RANK_MODES {
                let candidate = candidate(coder, Order::Original, IndexCoding::Rank(mode, Scope::Body), COVERAGE_RANS, unit);
                let bytes: u64 = bodies
                    .par_iter()
                    .map(|planes| {
                        let encoded = encode_candidate(planes, &candidate);
                        verify_round_trip(planes, &encoded, &candidate);
                        encoded.index_bytes()
                    })
                    .sum();
                row(coder, name, bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 3. Reference action

pub(super) fn stand_action(planes: &Planes, kinds: &HashMap<u32, String>) -> u16 {
    let number: Option<u32> = planes.label.rsplit(' ').next().and_then(|id| id.parse().ok());
    let kind = number.and_then(|n| kinds.get(&n)).and_then(|kind| kind.split(':').next());
    let wanted = match (kind, number) {
        (Some(KIND_MONSTER | KIND_SEA_MONSTER), _) => STAND_ACTION_MONSTER,
        (Some(KIND_ANIMAL), _) => STAND_ACTION_ANIMAL,
        (Some(KIND_HUMAN | KIND_EQUIPMENT), _) => STAND_ACTION_PEOPLE,
        (_, Some(n)) if n < FIRST_ANIMAL_BODY => STAND_ACTION_MONSTER,
        (_, Some(n)) if n < FIRST_PEOPLE_BODY => STAND_ACTION_ANIMAL,
        _ => STAND_ACTION_PEOPLE,
    };
    let first = planes.boxes.iter().map(|f| f.action).min().unwrap_or(0);
    if planes.boxes.iter().any(|f| f.action == wanted) { wanted } else { first }
}

fn compress_with_prefix(data: &[u8], prefix: &[u8]) -> Vec<u8> {
    let mut compressor = zstd::bulk::Compressor::with_dictionary(LEVEL_19, prefix).expect("zstd compressor");
    compressor.compress(data).expect("zstd compress")
}

#[derive(Default, Clone, Copy)]
struct ReferenceSizes {
    body: u64,
    action: u64,
    action_direction: u64,
    with_reference: u64,
    stand: u64,
}

impl std::ops::AddAssign for ReferenceSizes {
    fn add_assign(&mut self, other: Self) {
        self.body += other.body;
        self.action += other.action;
        self.action_direction += other.action_direction;
        self.with_reference += other.with_reference;
        self.stand += other.stand;
    }
}

/// One plane's sizes in each unit.
fn reference_sizes(planes: &Planes, plane: &[u8], stand: u16) -> ReferenceSizes {
    let mut sizes = ReferenceSizes { body: compress(plane, LEVEL_19, false).len() as u64, ..ReferenceSizes::default() };
    let collect = |frames: &[usize]| -> Vec<u8> { frames.iter().flat_map(|&f| plane[planes.boxes[f].range()].iter().copied()).collect() };
    for frames in super::units(&planes.boxes, Unit::Action, 1) {
        sizes.action += compress(&collect(&frames), LEVEL_19, false).len() as u64;
    }
    // The stand frames of each direction, as the prefix of the direction's other streams.
    let directions = directions_in_action(&planes.boxes);
    let mut prefixes = BTreeMap::<u32, Vec<u8>>::new();
    for (frame, &direction) in planes.boxes.iter().zip(&directions) {
        if frame.action == stand {
            prefixes.entry(direction).or_default().extend_from_slice(&plane[frame.range()]);
        }
    }
    for frames in super::units(&planes.boxes, Unit::ActionDirection, 1) {
        let data = collect(&frames);
        let plain = compress(&data, LEVEL_19, false);
        sizes.action_direction += plain.len() as u64;
        let prefix = prefixes.get(&directions[frames[0]]).filter(|_| planes.boxes[frames[0]].action != stand);
        match prefix {
            Some(prefix) => {
                let bytes = compress_with_prefix(&data, prefix);
                let mut decompressor = zstd::bulk::Decompressor::with_dictionary(prefix).expect("zstd decompressor");
                assert!(decompressor.decompress(&bytes, data.len()).expect("zstd decompress") == data, "{}: reference action does not round-trip", planes.label);
                sizes.with_reference += bytes.len() as u64;
            }
            None => {
                sizes.with_reference += plain.len() as u64;
                sizes.stand += plain.len() as u64;
            }
        }
    }
    sizes
}

fn part_k3(sample: &[Planes], kinds: &HashMap<u32, String>) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        println!("\n### k3: {} reference action: zstd 19 with the same direction's stand frames as a prefix; {} bodies\n", client.label(), bodies.len());
        println!("| palette slots | plane | whole body MB | action MB | action+direction MB | with reference MB (stand streams MB) |");
        println!("|---|---|---:|---:|---:|---|");
        for (order, name) in [(Order::Original, "as shipped"), (Order::Master, "remapped to a body master palette")] {
            let rows: Vec<(ReferenceSizes, ReferenceSizes)> = bodies
                .par_iter()
                .map(|planes| {
                    let ordered = reordered(planes, order);
                    let stand = stand_action(planes, kinds);
                    let index = reference_sizes(&ordered, &ordered.index, stand);
                    let coverage = if ordered.coverage.is_empty() {
                        ReferenceSizes::default()
                    } else {
                        reference_sizes(&ordered, &ordered.coverage, stand)
                    };
                    (index, coverage)
                })
                .collect();
            let mut index = ReferenceSizes::default();
            let mut coverage = ReferenceSizes::default();
            for (i, c) in rows {
                index += i;
                coverage += c;
            }
            let print = |plane: &str, sizes: &ReferenceSizes| {
                println!(
                    "| {name} | {plane} | {:.2} | {:.2} | {:.2} | {:.2} ({:.2}) |",
                    mb(sizes.body),
                    mb(sizes.action),
                    mb(sizes.action_direction),
                    mb(sizes.with_reference),
                    mb(sizes.stand)
                );
            };
            print("index", &index);
            if client == Client::Ec {
                print("coverage", &coverage);
            }
        }
        println!("\nShared rANS tables on the same bodies (index plane + coverage):\n");
        println!("| order | unit | index MB | coverage MB |");
        println!("|---|---|---:|---:|");
        for (order, order_name) in [(Order::Path, "path per palette"), (Order::Luma, "luma per palette"), (Order::Master, "master palette")] {
            for (unit, unit_name) in [(Unit::Body, "body"), (Unit::Action, "action"), (Unit::ActionDirection, "action+direction")] {
                let candidate = candidate("rANS", order, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, unit);
                let sizes: (u64, u64) = bodies
                    .par_iter()
                    .map(|planes| {
                        let ordered = reordered(planes, order);
                        let encoded = encode_candidate(&ordered, &candidate);
                        verify_round_trip(&ordered, &encoded, &candidate);
                        (encoded.index_bytes(), encoded.coverage_bytes())
                    })
                    .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
                println!("| {order_name} | {unit_name} | {:.2} | {:.2} |", mb(sizes.0), mb(sizes.1));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 4. Frame order

/// The planes with the frames re-laid by direction, then action, then frame.
fn frames_by_direction(planes: &Planes) -> Planes {
    let directions = directions_in_action(&planes.boxes);
    let mut order: Vec<usize> = (0..planes.boxes.len()).collect();
    order.sort_by_key(|&f| (directions[f], planes.boxes[f].action, f));
    let mut out = Planes { boxes: Vec::with_capacity(order.len()), index: Vec::with_capacity(planes.index.len()), coverage: Vec::with_capacity(planes.coverage.len()), ..planes.clone() };
    for f in order {
        let frame = &planes.boxes[f];
        let start = out.index.len();
        out.index.extend_from_slice(&planes.index[frame.range()]);
        if !planes.coverage.is_empty() {
            out.coverage.extend_from_slice(&planes.coverage[frame.range()]);
        }
        out.boxes.push(FrameBox { start, direction: directions[f], ..frame.clone() });
    }
    out
}

fn part_k4(sample: &[Planes]) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        println!("\n### k4: {} frame order inside a whole-body stream; {} bodies\n", client.label(), bodies.len());
        println!("| coder | action first, then direction (today's order) MB | direction first, then action MB |");
        println!("|---|---:|---:|");
        let coders = [
            candidate("zstd 19, index + coverage", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
            candidate("luma, MED, rANS", Order::Luma, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, Unit::Body),
            candidate("luma, previous frame context, rANS", Order::Luma, IndexCoding::RansPrevious(Predictor::Med, Scope::Body, PreviousRole::Context), COVERAGE_RANS, Unit::Body),
        ];
        for candidate in &coders {
            let sizes: (u64, u64) = bodies
                .par_iter()
                .map(|planes| {
                    let ordered = reordered(planes, candidate.order);
                    let by_direction = frames_by_direction(&ordered);
                    let (a, b) = (encode_candidate(&ordered, candidate), encode_candidate(&by_direction, candidate));
                    verify_round_trip(&by_direction, &b, candidate);
                    (a.total(), b.total())
                })
                .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
            println!("| {} | {:.2} | {:.2} |", candidate.name, mb(sizes.0), mb(sizes.1));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 5. Better reference frames

#[derive(Default, Clone, Copy)]
struct MatchCounts {
    opaque: u64,
    left: u64,
    previous: u64,
    previous_known: u64,
    earlier: u64,
    /// Per direction: (opaque, matches, with the best offset).
    mirror: [(u64, u64); STORED_DIRECTIONS as usize],
}

fn colour_frames(planes: &Planes) -> Vec<Vec<u32>> {
    planes
        .boxes
        .iter()
        .map(|frame| {
            let palette = &planes.palettes[frame.palette];
            frame
                .range()
                .map(|i| {
                    if planes.is_opaque(i) {
                        let [r, g, b, _] = palette[usize::from(planes.index[i])];
                        u32::from_le_bytes([r, g, b, 0])
                    } else {
                        TRANSPARENT_COLOUR
                    }
                })
                .collect()
        })
        .collect()
}

fn aligned_matches(frame: &FrameBox, colours: &[u32], other: &FrameBox, other_colours: &[u32]) -> (u64, u64) {
    let (dx, dy) = (frame.left - other.left, frame.top - other.top);
    let (mut matched, mut known) = (0u64, 0u64);
    let y0 = (-dy).max(0);
    let y1 = (other.h as i32 - dy).min(frame.h as i32);
    let x0 = (-dx).max(0);
    let x1 = (other.w as i32 - dx).min(frame.w as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let here = colours[y as usize * frame.w + x as usize];
            let there = other_colours[(y + dy) as usize * other.w + (x + dx) as usize];
            if here != TRANSPARENT_COLOUR && there != TRANSPARENT_COLOUR {
                known += 1;
                matched += u64::from(here == there);
            }
        }
    }
    (matched, known)
}

fn match_counts(planes: &Planes) -> MatchCounts {
    let colours = colour_frames(planes);
    let mut counts = MatchCounts::default();
    let directions = directions_in_action(&planes.boxes);
    let mut by_direction = BTreeMap::<u32, Vec<usize>>::new();
    for (f, &direction) in directions.iter().enumerate() {
        by_direction.entry(direction).or_default().push(f);
    }
    for frames in by_direction.values() {
        for (n, &f) in frames.iter().enumerate() {
            let frame = &planes.boxes[f];
            let here = &colours[f];
            let opaque = here.iter().filter(|&&c| c != TRANSPARENT_COLOUR).count() as u64;
            counts.opaque += opaque;
            for y in 0..frame.h {
                for x in 1..frame.w {
                    let i = y * frame.w + x;
                    counts.left += u64::from(here[i] != TRANSPARENT_COLOUR && here[i] == here[i - 1]);
                }
            }
            if n > 0 {
                let before = frames[n - 1];
                let (matched, known) = aligned_matches(frame, here, &planes.boxes[before], &colours[before]);
                counts.previous += matched;
                counts.previous_known += known;
            }
            let window = n.saturating_sub(EARLIER_FRAMES_LIMIT);
            let best = (window..n)
                .chain((0..window).filter(|&m| planes.boxes[frames[m]].action == frame.action))
                .map(|m| aligned_matches(frame, here, &planes.boxes[frames[m]], &colours[frames[m]]).0)
                .max()
                .unwrap_or(0);
            counts.earlier += best;
            // This frame against itself mirrored about the anchor, at the best offset.
            let direction = (directions[f] as usize).min(usize::from(STORED_DIRECTIONS) - 1);
            let mut best_mirror = 0u64;
            for offset in MIRROR_OFFSETS {
                let mut matched = 0u64;
                for y in 0..frame.h {
                    for x in 0..frame.w {
                        let c = here[y * frame.w + x];
                        if c == TRANSPARENT_COLOUR {
                            continue;
                        }
                        let anchored = x as i32 + frame.left;
                        let mirrored = offset - anchored - frame.left;
                        if mirrored >= 0 && (mirrored as usize) < frame.w {
                            matched += u64::from(here[y * frame.w + mirrored as usize] == c);
                        }
                    }
                }
                best_mirror = best_mirror.max(matched);
            }
            counts.mirror[direction].0 += opaque;
            counts.mirror[direction].1 += best_mirror;
        }
    }
    counts
}

fn part_k5(sample: &[Planes]) {
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let total = bodies
            .par_iter()
            .map(|planes| match_counts(planes))
            .reduce(MatchCounts::default, |mut a, b| {
                a.opaque += b.opaque;
                a.left += b.left;
                a.previous += b.previous;
                a.previous_known += b.previous_known;
                a.earlier += b.earlier;
                for d in 0..a.mirror.len() {
                    a.mirror[d].0 += b.mirror[d].0;
                    a.mirror[d].1 += b.mirror[d].1;
                }
                a
            });
        let share = |n: u64| n as f64 * PERCENT / total.opaque.max(1) as f64;
        println!("\n### k5: {} exact colour matches, % of {:.1} Mpx opaque; {} bodies", client.label(), total.opaque as f64 / PIXELS_PER_MEGAPIXEL, bodies.len());
        println!("  left pixel: {:.1} %", share(total.left));
        println!(
            "  previous frame of the direction, anchor-aligned: {:.1} % ({:.1} % have an opaque pixel there; {:.1} % of those match)",
            share(total.previous),
            share(total.previous_known),
            total.previous as f64 * PERCENT / total.previous_known.max(1) as f64
        );
        println!("  best of the earlier frames of the direction (at most {EARLIER_FRAMES_LIMIT} back, and all of the action): {:.1} %", share(total.earlier));
        for (direction, (opaque, matched)) in total.mirror.iter().enumerate() {
            println!("  stored direction {direction}, the frame against its own mirror image, best offset: {:.1} %", *matched as f64 * PERCENT / (*opaque).max(1) as f64);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 6. Coverage with its structure

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CoverageContexts {
    edge: bool,
    previous: bool,
    colour: bool,
}

const COVERAGE_VARIANTS: [(CoverageContexts, &str); 5] = [
    (CoverageContexts { edge: false, previous: false, colour: false }, "shape, then neighbours"),
    (CoverageContexts { edge: true, previous: false, colour: false }, "+ distance to the edge"),
    (CoverageContexts { edge: false, previous: true, colour: false }, "+ previous frame's coverage"),
    (CoverageContexts { edge: true, previous: true, colour: false }, "+ edge and previous frame"),
    (CoverageContexts { edge: true, previous: true, colour: true }, "+ edge, previous frame and the colour under it"),
];

fn coverage_context_count(which: CoverageContexts) -> usize {
    COVERAGE_CONTEXTS
        * if which.edge { EDGE_DISTANCE_CLASSES } else { 1 }
        * if which.previous { PREVIOUS_COVERAGE_CLASSES } else { 1 }
        * if which.colour { LUMA_CLASSES } else { 1 }
}

/// The city-block distance of each pixel of a frame to the nearest transparent or outside pixel, clamped.
fn edge_distances(mask: &[u8], w: usize, h: usize) -> Vec<u8> {
    let limit = EDGE_DISTANCE_CLASSES as u8;
    let mut distance = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] == 0 {
                continue;
            }
            let left = if x > 0 { distance[i - 1] } else { 0 };
            let up = if y > 0 { distance[i - w] } else { 0 };
            distance[i] = (left.min(up) + 1).min(limit);
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if mask[i] == 0 {
                continue;
            }
            let right = if x + 1 < w { distance[i + 1] } else { 0 };
            let down = if y + 1 < h { distance[i + w] } else { 0 };
            distance[i] = distance[i].min(right.min(down) + 1);
        }
    }
    distance
}

fn previous_coverage_class(value: Option<u8>) -> usize {
    match value {
        None | Some(COVERAGE_TRANSPARENT) => 0,
        Some(COVERAGE_SOLID) => 2,
        Some(_) => 1,
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn rich_coverage_context(which: CoverageContexts, coverage: &[u8], distance: &[u8], colours: &[[u8; EC_PALETTE_ENTRY_BYTES]], index: u8, before: Option<u8>, w: usize, i: usize, x: usize, y: usize) -> usize {
    let mut context = coverage_context(coverage, w, i, x, y);
    if which.edge {
        context = context * EDGE_DISTANCE_CLASSES + usize::from(distance[i]) - 1;
    }
    if which.previous {
        context = context * PREVIOUS_COVERAGE_CLASSES + previous_coverage_class(before);
    }
    if which.colour {
        let light = (luma(&colours[usize::from(index)]) as usize * LUMA_CLASSES) / (LUMA_MAX as usize + 1);
        context = context * LUMA_CLASSES + light.min(LUMA_CLASSES - 1);
    }
    context
}

/// `before` of a frame: the previous frame's coverage, anchor-aligned.
#[inline(always)]
fn coverage_before(plane: &[u8], previous: Option<PreviousFrame>, x: usize, y: usize) -> Option<u8> {
    let previous = previous?;
    let (px, py) = (x as i32 - previous.dx, y as i32 - previous.dy);
    if px < 0 || py < 0 || px >= previous.w as i32 || py >= previous.h as i32 {
        return None;
    }
    Some(plane[previous.start + py as usize * previous.w + px as usize])
}

/// The shape and the partial values with the chosen contexts; the index plane is known first.
fn encode_rich_coverage(planes: &Planes, which: CoverageContexts) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let frames: Vec<usize> = (0..planes.boxes.len()).collect();
    let previous = aligned_previous(&planes.boxes, &frames);
    let mut shape = Vec::with_capacity(planes.coverage.len());
    let mut values = Vec::new();
    for (n, frame) in planes.boxes.iter().enumerate() {
        let local = &planes.coverage[frame.range()];
        let mask: Vec<u8> = local.iter().map(|&c| u8::from(c != COVERAGE_TRANSPARENT)).collect();
        let distance = edge_distances(&mask, frame.w, frame.h);
        let colours = &planes.palettes[frame.palette];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                shape.push((mask_context(&mask, frame.w, i, x, y) as u16, mask[i]));
                if mask[i] == 0 {
                    continue;
                }
                let before = coverage_before(&planes.coverage, previous[n].map(|p| p.0), x, y);
                let context = rich_coverage_context(which, local, &distance, colours, planes.index[frame.start + i], before, frame.w, i, x, y);
                values.push((context as u16, local[i] - 1));
            }
        }
    }
    let (shape_tables, shape_streams) = rans_streams(std::slice::from_ref(&shape), BINARY_SYMBOLS, MASK_CONTEXTS, Scope::Body);
    let (value_tables, value_streams) = rans_streams(std::slice::from_ref(&values), PARTIAL_COVERAGE_SYMBOLS, coverage_context_count(which), Scope::Body);
    (shape_tables, shape_streams.into_iter().next().unwrap().1, value_tables, value_streams.into_iter().next().unwrap().1)
}

fn decode_rich_coverage(planes: &Planes, which: CoverageContexts, parts: &(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)) -> Vec<u8> {
    let shape_model = Model::parse(&parts.0, BINARY_SYMBOLS, MASK_CONTEXTS);
    let value_model = Model::parse(&parts.2, PARTIAL_COVERAGE_SYMBOLS, coverage_context_count(which));
    let frames: Vec<usize> = (0..planes.boxes.len()).collect();
    let previous = aligned_previous(&planes.boxes, &frames);
    let mut coverage = vec![0u8; planes.coverage.len()];
    let mut shape = RansDecoder::new(&parts.1);
    let mut values = RansDecoder::new(&parts.3);
    let mut mask = Vec::new();
    for (n, frame) in planes.boxes.iter().enumerate() {
        mask.clear();
        mask.resize(frame.pixels(), 0u8);
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                mask[i] = shape.get(&shape_model, mask_context(&mask, frame.w, i, x, y));
            }
        }
        let distance = edge_distances(&mask, frame.w, frame.h);
        let colours = &planes.palettes[frame.palette];
        let (done, local) = coverage.split_at_mut(frame.start);
        let local = &mut local[..frame.pixels()];
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                let before = coverage_before(done, previous[n].map(|p| p.0), x, y);
                let context = rich_coverage_context(which, local, &distance, colours, planes.index[frame.start + i], before, frame.w, i, x, y);
                local[i] = values.get(&value_model, context) + 1;
            }
        }
    }
    coverage
}

fn part_k6(sample: &[Planes], time: bool) {
    let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == Client::Ec).collect();
    if bodies.is_empty() {
        return;
    }
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let current = candidate("rANS", Order::Original, IndexCoding::Zstd(LEVEL_19), COVERAGE_RANS, Unit::Body);
    let base: u64 = bodies.par_iter().map(|planes| encode_candidate(planes, &current).coverage_bytes()).sum();
    let plain: u64 = bodies.par_iter().map(|planes| compress(&planes.coverage, LEVEL_19, false).len() as u64).sum();
    println!("\n### k6: Enhanced coverage, {} bodies: zstd 19 {:.2} MB, current rANS {:.2} MB\n", bodies.len(), mb(plain), mb(base));
    println!("| contexts | shape MB | values MB | tables MB | total MB | x current rANS | decode MB/s, one thread |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for (which, name) in COVERAGE_VARIANTS {
        let encoded: Vec<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)> = bodies
            .par_iter()
            .map(|planes| {
                let parts = encode_rich_coverage(planes, which);
                assert!(decode_rich_coverage(planes, which, &parts) == planes.coverage, "{}: rich coverage does not round-trip", planes.label);
                parts
            })
            .collect();
        let (mut shape, mut values, mut tables) = (0u64, 0u64, 0u64);
        for parts in &encoded {
            shape += parts.1.len() as u64;
            values += parts.3.len() as u64;
            tables += (parts.0.len() + parts.2.len()) as u64;
        }
        let total = shape + values + tables;
        let speed = if time {
            let (mut seconds, mut output) = (0f64, 0u64);
            for (planes, parts) in bodies.iter().zip(&encoded) {
                seconds += single.install(|| fastest(|| decode_rich_coverage(planes, which, parts).len())).as_secs_f64();
                output += planes.coverage.len() as u64;
            }
            format!("{:.0}", mb(output) / seconds.max(f64::MIN_POSITIVE))
        } else {
            "-".to_string()
        };
        println!("| {name} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} | {speed} |", mb(shape), mb(values), mb(tables), mb(total), total as f64 / base.max(1) as f64);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 7. Equipment layers

fn kind_of(planes: &Planes, kinds: &HashMap<u32, String>) -> String {
    planes
        .label
        .rsplit(' ')
        .next()
        .and_then(|id| id.parse::<u32>().ok())
        .and_then(|n| kinds.get(&n))
        .and_then(|kind| kind.split(':').next())
        .unwrap_or("not listed")
        .to_string()
}

/// The shape of each frame of a body, by (action, direction, position in the action).
fn shapes_of(planes: &Planes) -> HashMap<(u16, u32, usize), (FrameBox, Vec<u8>)> {
    let mut positions = HashMap::<(u16, u32), usize>::new();
    let mut shapes = HashMap::new();
    for (frame, &direction) in planes.boxes.iter().zip(&directions_in_action(&planes.boxes)) {
        let position = positions.entry((frame.action, direction)).or_default();
        let mask: Vec<u8> = frame.range().map(|i| u8::from(planes.is_opaque(i))).collect();
        shapes.insert((frame.action, direction, *position), (frame.clone(), mask));
        *position += 1;
    }
    shapes
}

/// The shape of a body coded with a base body's aligned shape as context: its bytes, and the share of its opaque pixels
/// inside the base's silhouette.
fn shape_with_base(planes: &Planes, base: &HashMap<(u16, u32, usize), (FrameBox, Vec<u8>)>) -> (u64, u64, u64) {
    let mut positions = HashMap::<(u16, u32), usize>::new();
    let mut symbols = Vec::new();
    let (mut opaque, mut inside) = (0u64, 0u64);
    for (frame, &direction) in planes.boxes.iter().zip(&directions_in_action(&planes.boxes)) {
        let position = positions.entry((frame.action, direction)).or_default();
        let reference = base.get(&(frame.action, direction, *position));
        *position += 1;
        let mask: Vec<u8> = frame.range().map(|i| u8::from(planes.is_opaque(i))).collect();
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = y * frame.w + x;
                let in_base = reference.is_some_and(|(other, shape)| {
                    let (px, py) = (x as i32 + frame.left - other.left, y as i32 + frame.top - other.top);
                    px >= 0 && py >= 0 && px < other.w as i32 && py < other.h as i32 && shape[py as usize * other.w + px as usize] != 0
                });
                opaque += u64::from(mask[i]);
                inside += u64::from(mask[i] != 0 && in_base);
                let context = mask_context(&mask, frame.w, i, x, y) * 2 + usize::from(in_base);
                symbols.push((context as u16, mask[i]));
            }
        }
    }
    let (tables, streams) = rans_streams(std::slice::from_ref(&symbols), BINARY_SYMBOLS, WITH_BASE_MASK_CONTEXTS, Scope::Body);
    let bytes = streams.into_iter().next().unwrap().1;
    // A decode of the symbols must give the mask back: the contexts depend only on what comes earlier or on the base.
    let model = Model::parse(&tables, BINARY_SYMBOLS, WITH_BASE_MASK_CONTEXTS);
    let mut decoder = RansDecoder::new(&bytes);
    for &(context, symbol) in &symbols {
        assert_eq!(decoder.get(&model, usize::from(context)), symbol, "{}: shape with base does not round-trip", planes.label);
    }
    ((tables.len() + bytes.len()) as u64, opaque, inside)
}

fn part_k7(sources: &Sources, bodies: &[BodySource], sample: &[Planes], kinds: &HashMap<u32, String>) {
    for client in [Client::Cc, Client::Ec] {
        let of: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if of.is_empty() {
            continue;
        }
        let current = candidate("rANS", Order::Luma, IndexCoding::Rans(Predictor::Med, Scope::Body), COVERAGE_RANS, Unit::Body);
        let rows: Vec<(String, u64, u64, u64)> = of
            .par_iter()
            .map(|planes| {
                let ordered = reordered(planes, Order::Luma);
                let encoded = encode_candidate(&ordered, &current);
                (kind_of(planes, kinds), encoded.index_bytes(), encoded.coverage_bytes(), planes.index.len() as u64)
            })
            .collect();
        let total: u64 = rows.iter().map(|r| r.1 + r.2).sum();
        let mut per_kind = BTreeMap::<String, (usize, u64, u64, u64)>::new();
        for row in rows {
            let entry = per_kind.entry(row.0).or_default();
            entry.0 += 1;
            entry.1 += row.1;
            entry.2 += row.2;
            entry.3 += row.3;
        }
        println!("\n### k7: {} cost by kind of body (mobtypes.txt), luma, MED, rANS; {} bodies, {:.2} MB\n", client.label(), of.len(), mb(total));
        println!("| kind | bodies | Mpx | index MB | shape or coverage MB | shape or coverage, % of the total |");
        println!("|---|---:|---:|---:|---:|---:|");
        for (kind, (count, index, coverage, pixels)) in &per_kind {
            println!("| {kind} | {count} | {:.1} | {:.2} | {:.2} | {:.1} |", *pixels as f64 / PIXELS_PER_MEGAPIXEL, mb(*index), mb(*coverage), *coverage as f64 * PERCENT / total.max(1) as f64);
        }
        // Shape with the base body's silhouette as context, on the equipment bodies.
        let base_source = |number: u32| {
            bodies.iter().find(|source| match (source, client) {
                (BodySource::CcMul { file: 0, body, .. }, Client::Cc) => u32::from(*body) == number,
                (BodySource::Ec { body, .. }, Client::Ec) => *body == number,
                _ => false,
            })
        };
        let bases: Vec<HashMap<(u16, u32, usize), (FrameBox, Vec<u8>)>> =
            BASE_BODIES.iter().filter_map(|&number| base_source(number)).map(|source| shapes_of(&Planes::from_body(&load_body(sources, source)))).collect();
        if bases.is_empty() {
            println!("No base body among the sources: shape context not measured");
            continue;
        }
        let equipment: Vec<&&Planes> = of.iter().filter(|p| kind_of(p, kinds) == KIND_EQUIPMENT).collect();
        let results: Vec<(u64, u64, u64, u64)> = equipment
            .par_iter()
            .map(|planes| {
                // The plain shape coder on this body, whichever the plane layout.
                let frames: Vec<usize> = (0..planes.boxes.len()).collect();
                let (_, mask) = unit_planes(planes, &frames);
                let mask01: Vec<u8> = mask.iter().map(|&m| u8::from(m != 0)).collect();
                let symbols = mask_symbols(&planes.boxes, &frames, &mask01);
                let (tables, streams) = rans_streams(std::slice::from_ref(&symbols), BINARY_SYMBOLS, MASK_CONTEXTS, Scope::Body);
                let plain_bytes = (tables.len() + streams[0].1.len()) as u64;
                let best = bases.iter().map(|base| shape_with_base(planes, base)).min_by_key(|r| r.0).unwrap();
                (plain_bytes, best.0, best.1, best.2)
            })
            .collect();
        let (mut shape, mut with_base, mut opaque, mut inside) = (0u64, 0u64, 0u64, 0u64);
        for r in results {
            shape += r.0;
            with_base += r.1;
            opaque += r.2;
            inside += r.3;
        }
        println!(
            "\nEquipment, {} bodies: shape {:.2} MB, with the base body's shape as context {:.2} MB (x{:.3}); {:.1} % of their opaque pixels lie inside the base's silhouette",
            equipment.len(),
            mb(shape),
            mb(with_base),
            with_base as f64 / shape.max(1) as f64,
            inside as f64 * PERCENT / opaque.max(1) as f64
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------

/// Decodes every stream of a body, with tables from outside.
fn decode_all_with<'a>(tables: &mut [Vec<Option<RankTables<'a>>>], encoded: &Encoded, masks: &[Vec<u8>], model: &Model, planes: &'a Planes, mode: RankMode) -> usize {
    encoded
        .units
        .iter()
        .zip(masks)
        .zip(tables.iter_mut())
        .map(|((unit, mask), tables)| decode_rank_with(tables, &unit.index, model, &planes.boxes, &planes.palettes, planes.client, &unit.frames, mask, mode).len())
        .sum()
}

/// Where rank coding's decode time goes, on one thread, summed over the sample.
fn part_r(sample: &[Planes]) {
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool");
    let state = cpu_state();
    let rows: [(&str, RankMode, Unit); 4] = [
        ("rank + previous frame context", RankMode::PreviousContext, Unit::Action),
        ("rank + previous frame context", RankMode::PreviousContext, Unit::Body),
        ("rank from MED colour", RankMode::Spatial, Unit::Action),
        ("rank from MED colour, 16x16x16 lookup", RankMode::Coarse, Unit::Action),
    ];
    println!("\n### r: rank coding's decode by stage, one thread; CPU {state}; load average {}", load_average());
    for client in [Client::Cc, Client::Ec] {
        let bodies: Vec<&Planes> = sample.iter().filter(|p| p.client == client).collect();
        if bodies.is_empty() {
            continue;
        }
        let resident: u64 = bodies.iter().map(|p| p.resident_bytes()).sum();
        let opaque: u64 = bodies.iter().map(|p| p.opaque_pixels()).sum();
        println!("\n#### {}: {} bodies, {:.1} MB resident, {:.1} Mpx opaque\n", client.label(), bodies.len(), mb(resident), opaque as f64 / PIXELS_PER_MEGAPIXEL);
        println!("| coder | stream | total MB | index MB | parse tables ms | shape/coverage ms | palette tables ms | pixel loop ms | whole decode ms | decode MB/s | opaque Mpx/s in the pixel loop |");
        println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
        for (name, mode, unit) in rows {
            let candidate = candidate(name, Order::Original, IndexCoding::Rank(mode, Scope::Body), COVERAGE_RANS, unit);
            let encoded: Vec<Encoded> = bodies
                .par_iter()
                .map(|planes| {
                    let encoded = encode_candidate(planes, &candidate);
                    verify_round_trip(planes, &encoded, &candidate);
                    encoded
                })
                .collect();
            let (mut parse, mut whole, mut cold, mut warm) = (0f64, 0f64, 0f64, 0f64);
            for (planes, encoded) in bodies.iter().zip(&encoded) {
                let boxes = &planes.boxes;
                let masks: Vec<Vec<u8>> = encoded.units.iter().map(|u| unit_planes(planes, &u.frames).1).collect();
                single.install(|| {
                    parse += fastest(|| shared_models(encoded, &candidate, client).index.is_some()).as_secs_f64();
                    let shared = shared_models(encoded, &candidate, client);
                    let model = shared.index.as_ref().expect("body tables");
                    whole += fastest(|| encoded.units.iter().map(|u| decode_unit(u, boxes, &planes.palettes, &candidate, client, &shared).0.len()).sum::<usize>()).as_secs_f64();
                    cold += fastest(|| encoded.units.iter().zip(&masks).map(|(u, mask)| decode_rank(&u.index, model, boxes, &planes.palettes, client, &u.frames, mask, mode).len()).sum::<usize>()).as_secs_f64();
                    // Tables filled by a first decode stay for the timed ones, one set per stream as in a real decode.
                    let mut tables: Vec<Vec<Option<RankTables>>> = encoded.units.iter().map(|_| (0..planes.palettes.len()).map(|_| None).collect()).collect();
                    decode_all_with(&mut tables, encoded, &masks, model, planes, mode);
                    warm += fastest(|| decode_all_with(&mut tables, encoded, &masks, model, planes, mode)).as_secs_f64();
                });
            }
            let total: u64 = encoded.iter().map(Encoded::total).sum();
            let index: u64 = encoded.iter().map(Encoded::index_bytes).sum();
            println!(
                "| {name} | {unit:?} | {:.2} | {:.2} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.1} | {:.1} |",
                mb(total),
                mb(index),
                ms(parse),
                ms((whole - cold).max(0.0)),
                ms((cold - warm).max(0.0)),
                ms(warm),
                ms(parse + whole),
                mb(resident) / (parse + whole),
                opaque as f64 / PIXELS_PER_MEGAPIXEL / warm,
            );
        }
    }
    let end = cpu_state();
    println!("\nLoad average at the end of part r {}", load_average());
    if end != state {
        println!("The CPU state changed during part r ({state} at the start, {end} at the end): discard its timings");
    }
}

pub(super) fn run(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, parts: &[String], stride: usize, time: bool) {
    let runs = |part: &str| parts.iter().any(|p| p == part || p == "k");
    let runs_r = parts.iter().any(|p| p == "r");
    if !runs_r && !["k1", "k2", "k3", "k4", "k5", "k6", "k7"].iter().any(|part| runs(part)) {
        return;
    }
    println!("\nPart k, every {stride}-th body; CPU {}; load average {}", cpu_state(), load_average());
    if runs("k1") {
        part_k1(sources, bodies, cc_dir);
    }
    if !runs_r && !["k2", "k3", "k4", "k5", "k6", "k7"].iter().any(|part| runs(part)) {
        return;
    }
    let kinds = body_kinds(cc_dir);
    let indices: Vec<usize> = [Client::Cc, Client::Ec]
        .into_iter()
        .flat_map(|client| {
            let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
            sample_of(present, client, count)
        })
        .collect();
    let sample: Vec<Planes> = indices.par_iter().map(|&i| Planes::from_body(&load_body(sources, &bodies[i]))).collect();
    if runs_r {
        part_r(&sample);
    }
    if runs("k2") {
        part_k2(&sample);
    }
    if runs("k3") {
        part_k3(&sample, &kinds);
    }
    if runs("k4") {
        part_k4(&sample);
    }
    if runs("k5") {
        part_k5(&sample);
    }
    if runs("k6") {
        part_k6(&sample, time);
    }
    if runs("k7") {
        part_k7(sources, bodies, &sample, &kinds);
    }
}
