//! Part `tk` of the `shrink` pass: do equipment and small, medium and large NPCs want different coders? Every lossless
//! coder of the ranking and of part `t`, per (action, direction), by class of body. Needs a quiet machine.
//!
//! The classes come from each client's own `tileart.uop`, see `BodyRule`; part `tk0` prints that rule against `mobtypes.txt`.
//!
//! Each class is spread over its bodies by pixels up to a pixel budget, so small bodies are all there and large ones are
//! thinned just enough. Coding and timing take turns in batches, so a batch's planes and streams are gone before the
//! next, and the timing runs on one thread with the others idle.

use super::*;
use std::collections::BTreeMap as Map;
use uocf::enhanced::mobile_body::{MobileBodyMap, MOBILE_DEFINITION_BASE};

/// A body's class by its median cropped frame area, in pixels: about 50 x 50, and about 100 x 100.
const NPC_SMALL_TOP: u64 = 2_500;
const NPC_MEDIUM_TOP: u64 = 10_000;
/// How much of a class this part codes at most; `CLASS_PIXELS_ENV` overrides it, for a test of the part itself.
const CLASS_PIXELS: u64 = 400_000_000;
const CLASS_PIXELS_ENV: &str = "MOBILE_ANIM_CODEC_TK_CLASS_PIXELS";
/// Coded and timed together before the next batch: planes, residuals and every coder's streams must fit.
const BATCH_PIXELS: u64 = 300_000_000;
const MEDIAN_DIVISOR: usize = 2;
/// Bits in a byte, for bits per painted pixel.
const BITS_PER_BYTE: f64 = 8.0;
/// Assignments measured for real at most, cheapest on disk first, after the prediction lets them through.
const REAL_CHECKS: usize = 60;
const NEAR_MISSES_SHOWN: usize = 6;
const ASSIGNED_CLASSES: usize = 4;
const LUMA_PALETTES: usize = 1;
const ORIGINAL_PALETTES: usize = 0;
const PALETTE_ORDERS: usize = 2;

const TILEART_FILE: &str = "tileart.uop";
/// Bodies named in a list of largest disagreements, and in a list of bodies with no placeholder.
const LIST_ROWS: usize = 25;

/// What a client's creature placeholders say of a body, looked up directly at `MOBILE_DEFINITION_BASE + body`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Verdict {
    /// No race, and its body type is a wearer's map.
    Equipment,
    /// It sets race.
    Person,
    /// Any other placeholder.
    Creature,
    /// No placeholder at its id: `mobtypes.txt` decides.
    NoPlaceholder,
    /// An `anim2+.mul` body no `Bodyconv.def` line reaches, so it has no id to look up: `mobtypes.txt` can't say either.
    NoServerBody,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Verdict::Equipment => "equipment (no race, a wearer's map)",
            Verdict::Person => "NPC, sets race",
            Verdict::Creature => "NPC, other map",
            Verdict::NoPlaceholder => "no placeholder",
            Verdict::NoServerBody => "no Bodyconv.def line",
        }
    }
}

/// One client's placeholders. A wearer's map is the body type of any placeholder that sets race (key 9).
struct TileRule {
    map: MobileBodyMap,
    wearers: BTreeSet<u32>,
}

impl TileRule {
    fn load(dir: &Path) -> TileRule {
        let path = find_ignoring_case(dir, TILEART_FILE).expect("the client's tileart.uop");
        let map = MobileBodyMap::load(path).expect("the client's tileart.uop parses");
        let wearers = map.definitions.values().filter(|d| d.race.is_some()).map(|d| d.sequence_id).collect();
        TileRule { map, wearers }
    }

    fn verdict(&self, id: u32) -> Verdict {
        match self.map.definition(id) {
            None => Verdict::NoPlaceholder,
            Some(tile) if tile.race.is_some() => Verdict::Person,
            Some(tile) if self.wearers.contains(&tile.sequence_id) => Verdict::Equipment,
            Some(_) => Verdict::Creature,
        }
    }
}

/// The placeholder rule, with `mobtypes.txt` for what has no placeholder.
pub(super) struct BodyRule<'a> {
    kinds: &'a BodyKinds,
    classic: TileRule,
    enhanced: TileRule,
}

impl<'a> BodyRule<'a> {
    pub(super) fn load(kinds: &'a BodyKinds, cc_dir: &Path) -> BodyRule<'a> {
        let ec_dir = std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client directory");
        BodyRule { kinds, classic: TileRule::load(cc_dir), enhanced: TileRule::load(Path::new(&ec_dir)) }
    }

    fn tiles(&self, source: &BodySource) -> &TileRule {
        match source {
            BodySource::Ec { .. } => &self.enhanced,
            BodySource::CcMul { .. } | BodySource::CcUop { .. } => &self.classic,
        }
    }

    fn verdict(&self, source: &BodySource) -> Verdict {
        self.kinds.id(source).map_or(Verdict::NoServerBody, |id| self.tiles(source).verdict(id))
    }

    pub(super) fn group(&self, source: &BodySource) -> Group {
        match self.verdict(source) {
            Verdict::Equipment => Group::Equipment,
            Verdict::Person | Verdict::Creature => Group::Creature,
            Verdict::NoPlaceholder | Verdict::NoServerBody => self.kinds.group(source),
        }
    }

    pub(super) fn is_person(&self, source: &BodySource) -> bool {
        match self.verdict(source) {
            Verdict::Person => true,
            Verdict::Equipment | Verdict::Creature => false,
            Verdict::NoPlaceholder | Verdict::NoServerBody => self.kinds.kind(source) == knowledge::KIND_HUMAN,
        }
    }
}

#[derive(Clone)]
struct RuleRow {
    source: &'static str,
    name: String,
    verdict: Verdict,
    body_type: Option<u32>,
    race: Option<u32>,
    kind: String,
    /// The rule's group, or `mobtypes.txt`'s where the rule has nothing.
    group: Group,
    frames: u64,
    pixels: u64,
}

const RULE_SOURCES: [&str; 3] = ["Enhanced", "Classic UOP", "Classic MUL"];

fn rule_row(rule: &BodyRule, source: &BodySource, planes: &Planes) -> RuleRow {
    let id = rule.kinds.id(source);
    let tile = id.and_then(|id| rule.tiles(source).map.definition(id));
    let (label, name) = match source {
        BodySource::Ec { body, .. } => (RULE_SOURCES[0], format!("{body}")),
        BodySource::CcUop { file, body, .. } => (RULE_SOURCES[1], format!("{body} (file {file})")),
        BodySource::CcMul { file, body, .. } => (RULE_SOURCES[2], match id {
            Some(id) if *file != 0 => format!("{id} (anim file {file}, its body {body})"),
            _ => format!("{body} (anim file {file})"),
        }),
    };
    RuleRow {
        source: label,
        name,
        verdict: rule.verdict(source),
        body_type: tile.map(|t| t.sequence_id),
        race: tile.and_then(|t| t.race),
        kind: rule.kinds.kind(source).to_string(),
        group: rule.group(source),
        frames: planes.boxes.len() as u64,
        pixels: planes.boxes.iter().map(|f| f.pixels() as u64).sum(),
    }
}

fn print_body_list(rows: &[&RuleRow], frames: u64, pixels: u64) {
    println!("\n| body | rule | body type | race | mobtypes.txt | frames | % frames | Mpx | % pixels |\n|---|---|---:|---:|---|---:|---:|---:|---:|");
    for row in rows {
        let number = |value: Option<u32>| value.map_or_else(|| "-".to_string(), |n| n.to_string());
        println!(
            "| {} | {} | {} | {} | {} | {} | {:.2} | {:.2} | {:.2} |",
            row.name,
            row.verdict.label(),
            number(row.body_type),
            number(row.race),
            row.kind,
            row.frames,
            row.frames as f64 * PERCENT / frames.max(1) as f64,
            row.pixels as f64 / PIXELS_PER_MEGAPIXEL,
            row.pixels as f64 * PERCENT / pixels.max(1) as f64,
        );
    }
}

fn print_rule_source(label: &str, of: &[RuleRow]) {
    let frames: u64 = of.iter().map(|r| r.frames).sum();
    let pixels: u64 = of.iter().map(|r| r.pixels).sum();
    println!("\n#### {label}: {} bodies, {frames} frames, {:.1} Mpx in cropped boxes; rule class against mobtypes.txt kind\n", of.len(), pixels as f64 / PIXELS_PER_MEGAPIXEL);
    println!("| rule class | mobtypes.txt kind | bodies | frames | % frames | Mpx | % pixels |\n|---|---|---:|---:|---:|---:|---:|");
    let mut cells: Map<(Verdict, &str), (u64, u64, u64)> = Map::new();
    for row in of {
        let cell = cells.entry((row.verdict, row.kind.as_str())).or_default();
        *cell = (cell.0 + 1, cell.1 + row.frames, cell.2 + row.pixels);
    }
    for ((verdict, kind), (count, f, p)) in &cells {
        println!("| {} | {kind} | {count} | {f} | {:.2} | {:.2} | {:.2} |", verdict.label(), *f as f64 * PERCENT / frames.max(1) as f64, *p as f64 / PIXELS_PER_MEGAPIXEL, *p as f64 * PERCENT / pixels.max(1) as f64);
    }
    println!("\n| equipment by the wearer's map it plays | bodies | frames | % frames | Mpx | % pixels |\n|---|---:|---:|---:|---:|---:|");
    let mut maps: Map<u32, (u64, u64, u64)> = Map::new();
    for row in of.iter().filter(|r| r.verdict == Verdict::Equipment) {
        let cell = maps.entry(row.body_type.unwrap_or_default()).or_default();
        *cell = (cell.0 + 1, cell.1 + row.frames, cell.2 + row.pixels);
    }
    for (map, (count, f, p)) in &maps {
        println!("| {map} | {count} | {f} | {:.2} | {:.2} | {:.2} |", *f as f64 * PERCENT / frames.max(1) as f64, *p as f64 / PIXELS_PER_MEGAPIXEL, *p as f64 * PERCENT / pixels.max(1) as f64);
    }
    println!("\n| the rule's group, with mobtypes.txt where it has nothing | bodies | % frames | % pixels |\n|---|---:|---:|---:|");
    for group in GROUPS {
        let in_group = || of.iter().filter(|r| r.group == group);
        println!("| {} | {} | {:.2} | {:.2} |", group.label(), in_group().count(), in_group().map(|r| r.frames).sum::<u64>() as f64 * PERCENT / frames.max(1) as f64, in_group().map(|r| r.pixels).sum::<u64>() as f64 * PERCENT / pixels.max(1) as f64);
    }
    let mut none: Vec<&RuleRow> = of.iter().filter(|r| matches!(r.verdict, Verdict::NoPlaceholder | Verdict::NoServerBody)).collect();
    none.sort_by_key(|r| std::cmp::Reverse(r.pixels));
    println!("\nNo placeholder: {} bodies, {} frames, {:.2} Mpx; of them {} with no id to look up. Largest, up to {LIST_ROWS}:", none.len(), none.iter().map(|r| r.frames).sum::<u64>(), none.iter().map(|r| r.pixels).sum::<u64>() as f64 / PIXELS_PER_MEGAPIXEL, none.iter().filter(|r| r.verdict == Verdict::NoServerBody).count());
    print_body_list(&none[..none.len().min(LIST_ROWS)], frames, pixels);
    let mut apart: Vec<&RuleRow> = of.iter().filter(|r| r.group != Group::of(&r.kind)).collect();
    apart.sort_by_key(|r| std::cmp::Reverse(r.pixels));
    println!("\nThe two disagree on equipment or NPC, or mobtypes.txt has no line: {} bodies, {} frames, {:.2} Mpx. Largest, up to {LIST_ROWS}:", apart.len(), apart.iter().map(|r| r.frames).sum::<u64>(), apart.iter().map(|r| r.pixels).sum::<u64>() as f64 / PIXELS_PER_MEGAPIXEL);
    print_body_list(&apart[..apart.len().min(LIST_ROWS)], frames, pixels);
    let listed_apart: Vec<&RuleRow> = apart.iter().copied().filter(|r| r.kind != KIND_UNLISTED).collect();
    println!("\nOf those, where mobtypes.txt has a line: {} bodies, {} frames, {:.2} Mpx. Largest, up to {LIST_ROWS}:", listed_apart.len(), listed_apart.iter().map(|r| r.frames).sum::<u64>(), listed_apart.iter().map(|r| r.pixels).sum::<u64>() as f64 / PIXELS_PER_MEGAPIXEL);
    print_body_list(&listed_apart[..listed_apart.len().min(LIST_ROWS)], frames, pixels);
    let mut person: Vec<&RuleRow> = of.iter().filter(|r| r.verdict == Verdict::Person && r.kind != knowledge::KIND_HUMAN || r.verdict == Verdict::Creature && r.kind == knowledge::KIND_HUMAN).collect();
    person.sort_by_key(|r| std::cmp::Reverse(r.pixels));
    println!("\nInside NPCs, race against HUMAN: {} bodies apart. Largest, up to {LIST_ROWS}:", person.len());
    print_body_list(&person[..person.len().min(LIST_ROWS)], frames, pixels);
}

fn print_tiles(label: &str, tiles: &TileRule) {
    let setting = tiles.map.definitions.values().filter(|d| d.race.is_some()).count();
    let plain = tiles.map.definitions.values().filter(|d| d.race.is_none() && tiles.wearers.contains(&d.sequence_id)).count();
    println!("{label} {TILEART_FILE}: {} placeholders at {MOBILE_DEFINITION_BASE} + body, {setting} set race, whose body types are the wearer's maps {:?}; {plain} have no race and play one of those", tiles.map.definitions.len(), tiles.wearers);
    let setters: Vec<String> = tiles.map.definitions.values().filter(|d| d.race.is_some()).map(|d| format!("{} (body type {}, race {}, gender {})", d.body, d.sequence_id, d.race.unwrap_or_default(), d.gender.unwrap_or_default())).collect();
    println!("  they set race: {}", setters.join("; "));
    for map in &tiles.wearers {
        let players: Vec<u32> = tiles.map.definitions.values().filter(|d| d.race.is_none() && d.sequence_id == *map).map(|d| d.body).collect();
        println!("  no race, body type {map}: {} placeholders{}", players.len(), if players.len() <= LIST_ROWS { format!(": {players:?}") } else { String::new() });
    }
}

pub(in super::super) fn part_tk0(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    println!("\n### tk0: the placeholder rule, run as one rule, on every body; pixels are cropped boxes");
    print_tiles("Enhanced", &rule.enhanced);
    print_tiles("Classic", &rule.classic);
    let rows: Vec<RuleRow> = present.par_iter().map(|s| rule_row(&rule, &bodies[s.2], &Planes::from_body(&load_body(sources, &bodies[s.2])))).collect();
    for label in RULE_SOURCES {
        let of: Vec<RuleRow> = rows.iter().filter(|r| r.source == label).cloned().collect();
        print_rule_source(label, &of);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Class {
    Equipment,
    Small,
    Medium,
    Large,
    Unlisted,
}

pub(super) const CLASSES: [Class; 5] = [Class::Equipment, Class::Small, Class::Medium, Class::Large, Class::Unlisted];
/// The classes an assignment chooses a coder for; an unlisted body follows the class of its size.
const ASSIGNED: [Class; ASSIGNED_CLASSES] = [Class::Equipment, Class::Small, Class::Medium, Class::Large];

impl Class {
    pub(super) fn at(self) -> usize {
        self as usize
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Class::Equipment => "equipment",
            Class::Small => "NPC small",
            Class::Medium => "NPC medium",
            Class::Large => "NPC large",
            Class::Unlisted => "not listed",
        }
    }
}

struct BodyInfo {
    body: usize,
    class: Class,
    /// Equipment, or the size class of anything else.
    assigned: Class,
    streams: u64,
    pixels: u64,
    painted: u64,
}

/// A body's class, and for what the rule can't place, the size class it is assigned as.
/// A body's median cropped frame area, which its size class comes from.
pub(super) fn median_area(planes: &Planes) -> u64 {
    let mut areas: Vec<u64> = planes.boxes.iter().map(|f| f.pixels() as u64).filter(|&area| area > 0).collect();
    areas.sort_unstable();
    areas.get(areas.len() / MEDIAN_DIVISOR).copied().unwrap_or(0)
}

pub(super) fn classes_of(planes: &Planes, rule: &BodyRule, source: &BodySource) -> (Class, Class) {
    let size = match median_area(planes) {
        area if area <= NPC_SMALL_TOP => Class::Small,
        area if area <= NPC_MEDIUM_TOP => Class::Medium,
        _ => Class::Large,
    };
    match rule.group(source) {
        Group::Equipment => (Class::Equipment, Class::Equipment),
        Group::Creature => (size, size),
        Group::Unlisted => (Class::Unlisted, size),
    }
}

fn survey(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], rule: &BodyRule, client: Client) -> Vec<BodyInfo> {
    let of: Vec<usize> = present.iter().filter(|s| s.0 == client).map(|s| s.2).collect();
    of.par_iter()
        .map(|&i| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            let (class, assigned) = classes_of(&planes, rule, &bodies[i]);
            let painted = match client {
                Client::Cc => planes.index.iter().filter(|&&value| value != 0).count(),
                Client::Ec => planes.coverage.iter().filter(|&&value| value != 0).count(),
            } as u64;
            let streams = units(&planes.boxes, Unit::ActionDirection, client.pixel_bytes()).len() as u64;
            BodyInfo { body: i, class, assigned, streams, pixels: planes.boxes.iter().map(|f| f.pixels() as u64).sum(), painted }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Coder {
    Old(Candidate),
    New(Scheme),
}

impl Coder {
    fn name(&self) -> &'static str {
        match self {
            Coder::Old(candidate) => candidate.name,
            Coder::New(scheme) => scheme.name,
        }
    }

    fn palettes(&self) -> usize {
        let luma = match self {
            Coder::Old(candidate) => candidate.order == Order::Luma,
            Coder::New(scheme) => matches!(scheme.index, IndexForm::Left | IndexForm::Up),
        };
        if luma { LUMA_PALETTES } else { ORIGINAL_PALETTES }
    }
}

const PLAIN_NAME: &str = "zstd 19";

const NEW_COMMON: [Scheme; 6] = [
    scheme(PLAIN_NAME, IndexForm::Plain, CoverageForm::Zstd),
    scheme("LZ4 HC", IndexForm::Lz4, CoverageForm::Lz4),
    scheme("luma, left, tight", IndexForm::Left, CoverageForm::Zstd),
    scheme("luma, up, tight", IndexForm::Up, CoverageForm::Zstd),
    scheme("a row spans, zstd 19", SPANS_ZSTD, CoverageForm::Zstd),
    scheme("a row spans, LZ4 HC", SPANS_LZ4, CoverageForm::Lz4),
];

const NEW_CLASSIC: [Scheme; 5] = [
    scheme("a row spans, best dictionary per body", IndexForm::SpansDictBest, CoverageForm::Zstd),
    scheme("b painted, runs apart, zstd 19", CC_PAINTED[1], CoverageForm::Zstd),
    scheme("b painted, runs in the frame, LZ4 HC", CC_PAINTED[3], CoverageForm::Zstd),
    kept("split boxes, zstd 19", BOXES_SIDE_ZSTD, CoverageForm::Zstd),
    kept("split boxes, LZ4 HC", BOXES_SIDE_LZ4, CoverageForm::Zstd),
];

const NEW_ENHANCED: [Scheme; 13] = [
    scheme("luma, left, tight, coverage row spans", IndexForm::Left, COVERAGE_SPANS_ZSTD),
    scheme("luma, left, tight, coverage every pixel tANS by context", IndexForm::Left, CoverageForm::Contexts),
    scheme("zstd 19, coverage every pixel tANS by context", IndexForm::Plain, CoverageForm::Contexts),
    scheme("b painted, zstd 19", PAINTED_EC_ZSTD, CoverageForm::Zstd),
    scheme("b painted, coverage row spans, zstd 19", PAINTED_EC_ZSTD, COVERAGE_SPANS_ZSTD),
    scheme("b painted, coverage every pixel tANS by context", PAINTED_EC_ZSTD, CoverageForm::Contexts),
    scheme("b painted, coverage runs zstd 19, partial tANS by context", PAINTED_EC_ZSTD, CoverageForm::RunsContexts),
    scheme("b painted, LZ4 HC", PAINTED_EC_LZ4, CoverageForm::Lz4),
    scheme("b painted, coverage row spans, LZ4 HC", PAINTED_EC_LZ4, COVERAGE_SPANS_LZ4),
    scheme("c both planes in row spans, zstd 19", SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD),
    scheme("c both planes in row spans, LZ4 HC", SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4),
    kept("split boxes, zstd 19", BOXES_ALONE_ZSTD, COVERAGE_BOXES_ZSTD),
    kept("split boxes, LZ4 HC", BOXES_ALONE_LZ4, COVERAGE_BOXES_LZ4),
];

fn coders_of(client: Client) -> Vec<Coder> {
    let mut coders: Vec<Coder> = NEW_COMMON.iter().map(|&s| Coder::New(s)).collect();
    match client {
        Client::Cc => coders.extend(NEW_CLASSIC.iter().map(|&s| Coder::New(s))),
        Client::Ec => coders.extend(NEW_ENHANCED.iter().map(|&s| Coder::New(s))),
    }
    // The ranking's other rows; its zstd 19 and LZ4 HC are the first two above.
    coders.extend(crowd::CROWD.iter().filter(|c| c.order == Order::Luma || matches!(c.index, IndexCoding::Rank(..))).map(|c| Coder::Old(crowd::with_unit(c, Unit::ActionDirection))));
    coders
}

enum CodedBody {
    Old(Candidate, Encoded),
    New(Coded),
}

/// A body ready to decode under every coder.
struct Ready {
    planes: Planes,
    luma: Planes,
    streams: StreamPlanes,
    coded: Vec<CodedBody>,
}

fn prepare(whole: Planes, actions: Option<&BTreeSet<u16>>, coders: &[Coder]) -> Ready {
    let needs_dictionaries = coders.iter().any(|c| matches!(c, Coder::New(s) if matches!(s.index, IndexForm::SpansDictBest)));
    let dictionaries = if needs_dictionaries { dictionaries_of(&StreamPlanes::of(&whole, None, None, Unit::ActionDirection)) } else { Dictionaries::none() };
    let planes = match actions {
        Some(wanted) => crowd::restricted(&whole, wanted),
        None => whole,
    };
    let luma = reordered(&planes, Order::Luma);
    let streams = StreamPlanes::of(&planes, Some(&luma), None, Unit::ActionDirection);
    let mut atoms = Atoms::default();
    let coded = coders
        .iter()
        .map(|coder| match coder {
            Coder::New(scheme) => CodedBody::New(coded_of(scheme, &streams, &dictionaries, &mut atoms)),
            Coder::Old(candidate) => {
                let ordered = if candidate.order == Order::Luma { &luma } else { &planes };
                CodedBody::Old(*candidate, crowd::encode_checked(ordered, candidate))
            }
        })
        .collect();
    Ready { planes, luma, streams, coded }
}

impl Ready {
    fn ordered(&self, candidate: &Candidate) -> &Planes {
        if candidate.order == Order::Luma { &self.luma } else { &self.planes }
    }
}

#[derive(Default)]
struct CoderCost {
    shared: u64,
    tables: f64,
    bytes: Vec<u32>,
    seconds: Vec<f32>,
}

struct BodyCost {
    class: Class,
    assigned: Class,
    pixels: Vec<u64>,
    painted: u64,
    coders: Vec<CoderCost>,
    /// A palette's hash in the original and in luma order.
    palettes: [Vec<u64>; PALETTE_ORDERS],
    stream_palettes: Vec<Vec<usize>>,
}

/// One thread, tables built once; each stream's fastest of the timing runs.
fn cost_of(ready: &Ready, coded: &CodedBody) -> CoderCost {
    match coded {
        CodedBody::New(code) => {
            let tables = fastest(|| code.tables(None)).as_secs_f64();
            let mut built = code.tables(None);
            let layout = &ready.streams.layout;
            let seconds = SCRATCH.with(|scratch| {
                let o = &mut *scratch.borrow_mut();
                (0..layout.len()).map(|s| fastest(|| code.decode(layout, s, &mut built, o, code.expand)).as_secs_f32()).collect()
            });
            CoderCost { shared: code.shared(), tables, bytes: (0..layout.len()).map(|s| code.stored_of(s) as u32).collect(), seconds }
        }
        CodedBody::Old(candidate, encoded) => {
            let planes = ready.ordered(candidate);
            let tables = fastest(|| shared_models(encoded, candidate, planes.client)).as_secs_f64();
            let built = shared_models(encoded, candidate, planes.client);
            let seconds = encoded.units.iter().map(|unit| fastest(|| decode_unit(unit, &planes.boxes, &planes.palettes, candidate, planes.client, &built)).as_secs_f32()).collect();
            let bytes = encoded.units.iter().map(|u| (u.index.len() + u.coverage.len() + u.index_tables.len() + u.coverage_tables.len()) as u32).collect();
            CoderCost { shared: (encoded.index_tables.len() + encoded.coverage_tables.len()) as u64, tables, bytes, seconds }
        }
    }
}

fn body_cost(ready: &Ready, info: &BodyInfo) -> BodyCost {
    BodyCost {
        class: info.class,
        assigned: info.assigned,
        pixels: ready.streams.layout.iter().map(|l| l.pixels as u64).collect(),
        painted: info.painted,
        coders: ready.coded.iter().map(|coded| cost_of(ready, coded)).collect(),
        palettes: [palette_hashes(&ready.planes), palette_hashes(&ready.luma)],
        stream_palettes: ready.streams.layout.iter().map(|l| l.palettes.clone()).collect(),
    }
}

/// What a way of picking a coder costs on the measured bodies, and scaled to the whole set by class.
#[derive(Default, Clone, Copy)]
struct Way {
    measured: u64,
    scaled: f64,
    palettes: f64,
    seconds: f64,
    resident: u64,
    largest: f64,
}

struct Measured<'a> {
    client: Client,
    costs: &'a [BodyCost],
    /// Whole class pixels over measured class pixels.
    scale: [f64; CLASSES.len()],
}

impl Measured<'_> {
    /// The palette table when `order_of` says which order each stream's palettes are in: a palette counts once, at the
    /// scale of the first class that holds it, so sampled classes count for their whole set.
    fn palette_table(&self, order_of: &dyn Fn(&BodyCost, usize) -> usize) -> f64 {
        let mut seen = HashSet::<u64>::new();
        let mut palettes = 0.0f64;
        for class in CLASSES {
            let mut new = 0u64;
            for body in self.costs.iter().filter(|b| b.class == class) {
                for (s, used) in body.stream_palettes.iter().enumerate() {
                    let order = order_of(body, s);
                    new += used.iter().filter(|&&p| seen.insert(body.palettes[order][p])).count() as u64;
                }
            }
            palettes += new as f64 * self.scale[class.at()];
        }
        let entry_bytes = match self.client {
            Client::Cc => CC_PALETTE_ENTRY_BYTES,
            Client::Ec => EC_PALETTE_ENTRY_BYTES,
        };
        palettes * (PALETTE_COLOURS * entry_bytes) as f64
    }

    /// Picks a coder per stream, with `id_bytes` stored per stream.
    fn way(&self, coders: &[Coder], id_bytes: u64, pick: &dyn Fn(&BodyCost, usize) -> usize) -> Way {
        let mut way = Way::default();
        for body in self.costs {
            let scale = self.scale[body.class.at()];
            let mut used = vec![false; coders.len()];
            let mut bytes = 0u64;
            for s in 0..body.pixels.len() {
                let c = pick(body, s);
                used[c] = true;
                let cost = &body.coders[c];
                bytes += u64::from(cost.bytes[s]) + id_bytes;
                way.seconds += f64::from(cost.seconds[s]);
                way.resident += body.pixels[s] * self.client.pixel_bytes() as u64;
                way.largest = way.largest.max(f64::from(cost.seconds[s]) + cost.tables);
            }
            for (c, _) in used.iter().enumerate().filter(|(_, in_use)| **in_use) {
                bytes += body.coders[c].shared;
                way.seconds += body.coders[c].tables;
            }
            way.measured += bytes;
            way.scaled += bytes as f64 * scale;
        }
        way.palettes = self.palette_table(&|body, s| coders[pick(body, s)].palettes());
        way
    }
}

fn print_way(name: &str, picked: &str, way: &Way, plain: &Way) {
    let whole = way.scaled + way.palettes;
    println!(
        "| {name} | {picked} | {:.1} | {:.1} | {:.1} | {:.3} | {:.0} | {:.2} |",
        mb(way.measured),
        mb(way.palettes as u64),
        mb(whole as u64),
        whole / (plain.scaled + plain.palettes),
        way.resident as f64 / BYTES_PER_MB / way.seconds,
        ms(way.largest),
    );
}

const WAY_HEADER: &str = "| way | coders | measured bodies MB | palette table, whole set MB | whole set estimate MB | x zstd 19 | decode MB/s, one thread | largest stream ms |\n|---|---|---:|---:|---:|---:|---:|---:|";

/// The jobs of one crowd check under each coder: `[coder][pair]`.
struct Check<'a> {
    name: String,
    /// `ASSIGNED` position of each pair's body.
    class_at: Vec<usize>,
    jobs: Vec<Vec<Job<'a>>>,
    /// Each pair's fastest seconds on one thread: `[coder][pair]`.
    seconds: Vec<Vec<f64>>,
}

/// Jobs in queue order on `WORKER_THREADS` workers, each taking the next as it frees.
fn simulated(seconds: impl Iterator<Item = f64>) -> f64 {
    let mut free = [0.0f64; WORKER_THREADS];
    for job in seconds {
        let (next, _) = free.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).expect("a worker");
        free[next] += job;
    }
    free.iter().copied().fold(0.0, f64::max)
}

fn mixed<'a>(check: &'a Check<'a>, assignment: &[usize; ASSIGNED_CLASSES]) -> Vec<Job<'a>> {
    check
        .class_at
        .iter()
        .enumerate()
        .map(|(pair, &class)| {
            let job = &check.jobs[assignment[class]][pair];
            Job { run: Box::new(move || (job.run)()), stored: 0, rgba: 0 }
        })
        .collect()
}

fn checks_of<'a>(label: &str, pairs: &[Pair], ready: &'a [Ready], position: &HashMap<usize, usize>, infos: &HashMap<usize, &BodyInfo>, coders: usize) -> Vec<Check<'a>> {
    let wanted_of: Vec<Vec<usize>> = pairs
        .iter()
        .map(|p| {
            let layout = &ready[position[&p.body()]].streams.layout;
            (0..layout.len()).filter(|&s| layout[s].action == p.action()).collect()
        })
        .collect();
    let class_at: Vec<usize> = pairs.iter().map(|p| ASSIGNED.iter().position(|&class| class == infos[&p.body()].assigned).expect("an assigned class")).collect();
    [false, true]
        .into_iter()
        .map(|faced| {
            let jobs: Vec<Vec<Job<'a>>> = (0..coders)
                .map(|c| {
                    pairs
                        .iter()
                        .enumerate()
                        .map(|(n, p)| {
                            let body = &ready[position[&p.body()]];
                            let wanted: Vec<usize> = if faced { vec![crowd::facing_unit(n, &wanted_of[n])] } else { wanted_of[n].clone() };
                            let run: Box<dyn Fn() -> usize + Sync + 'a> = match &body.coded[c] {
                                CodedBody::New(code) => Box::new(move || code.decode_all(&body.streams.layout, &wanted, None)),
                                CodedBody::Old(candidate, encoded) => Box::new(move || crowd::decode_units(body.ordered(candidate), encoded, candidate, &wanted)),
                            };
                            Job { run, stored: 0, rgba: 0 }
                        })
                        .collect()
                })
                .collect();
            let seconds = jobs.iter().map(|list| crowd::run_jobs(list, 1).1).collect();
            Check { name: format!("{label}, {}", if faced { "faced direction" } else { "five directions" }), class_at: class_at.clone(), jobs, seconds }
        })
        .collect()
}

fn class_sums(costs: &[BodyCost], c: usize, class: Class) -> (u64, f64, u64, f64, u64) {
    let (mut bytes, mut seconds, mut resident, mut largest, mut painted) = (0u64, 0.0f64, 0u64, 0.0f64, 0u64);
    for body in costs.iter().filter(|b| b.class == class) {
        let cost = &body.coders[c];
        bytes += cost.shared + cost.bytes.iter().map(|&b| u64::from(b)).sum::<u64>();
        seconds += cost.tables + cost.seconds.iter().map(|&s| f64::from(s)).sum::<f64>();
        resident += body.pixels.iter().sum::<u64>();
        largest = cost.seconds.iter().fold(largest, |top, &s| top.max(f64::from(s) + cost.tables));
        painted += body.painted;
    }
    (bytes, seconds, resident, largest, painted)
}

fn print_class_tables(client: Client, coders: &[Coder], costs: &[BodyCost], plain: usize) {
    for class in CLASSES {
        let (base, ..) = class_sums(costs, plain, class);
        if base == 0 {
            continue;
        }
        println!("\n#### {}: {}; palettes not counted\n\n| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |\n|---|---:|---:|---:|---:|---:|", client.label(), class.label());
        let mut rows: Vec<(usize, u64)> = (0..coders.len()).map(|c| (c, class_sums(costs, c, class).0)).collect();
        rows.sort_by_key(|row| row.1);
        for (c, bytes) in rows {
            let (_, seconds, resident, largest, painted) = class_sums(costs, c, class);
            println!(
                "| {} | {:.2} | {:.3} | {:.2} | {:.0} | {:.2} |",
                coders[c].name(),
                mb(bytes),
                bytes as f64 / base as f64,
                bytes as f64 * BITS_PER_BYTE / painted.max(1) as f64,
                (resident * client.pixel_bytes() as u64) as f64 / BYTES_PER_MB / seconds,
                ms(largest),
            );
        }
    }
}

fn names_of(coders: &[Coder], assignment: &[usize]) -> String {
    ASSIGNED.iter().zip(assignment).map(|(class, &c)| format!("{}: {}", class.label(), coders[c].name())).collect::<Vec<String>>().join("; ")
}

/// The smallest assignment whose crowds pass the line for real; sizes as `Measured::way`.
fn search(measured: &Measured, coders: &[Coder], checks: &[Check], plain: &Way) {
    let client = measured.client;
    // Sums by assigned class and coder: its own bodies, and the unlisted bodies that follow it.
    let mut sums = vec![[0.0f64; ASSIGNED_CLASSES]; coders.len()];
    for body in measured.costs {
        let scale = measured.scale[body.class.at()];
        let at = ASSIGNED.iter().position(|&class| class == body.assigned).expect("an assigned class");
        for (c, sum) in sums.iter_mut().enumerate() {
            let cost = &body.coders[c];
            sum[at] += (cost.shared + cost.bytes.iter().map(|&b| u64::from(b)).sum::<u64>()) as f64 * scale;
        }
    }
    // The palette table depends only on which classes use luma order.
    let patterns: Vec<f64> = (0..1usize << ASSIGNED_CLASSES)
        .map(|pattern| {
            measured.palette_table(&|body, _| {
                let at = ASSIGNED.iter().position(|&class| class == body.assigned).expect("an assigned class");
                (pattern >> at) & 1
            })
        })
        .collect();
    let mut order: Vec<([usize; ASSIGNED_CLASSES], f64)> = Vec::new();
    let n = coders.len();
    for e in 0..n {
        for s in 0..n {
            for m in 0..n {
                for l in 0..n {
                    let a = [e, s, m, l];
                    let pattern: usize = a.iter().enumerate().map(|(at, &c)| (coders[c].palettes() == LUMA_PALETTES) as usize * (1 << at)).sum();
                    let total = a.iter().enumerate().map(|(at, &c)| sums[c][at]).sum::<f64>() + patterns[pattern];
                    order.push((a, total));
                }
            }
        }
    }
    order.sort_by(|x, y| x.1.total_cmp(&y.1));
    println!("\n#### {}: the smallest assignment inside the {} ms line on {WORKER_THREADS} threads, in all {} checks; {} assignments in disk order\n", client.label(), crowd::CROWD_TARGET_MS, checks.len(), order.len());
    println!("| assignment | whole set estimate MB | x zstd 19 | prediction ms, worst check | real {WORKER_THREADS} threads ms, per check | passes |\n|---|---:|---:|---:|---|---|");
    let target = crowd::CROWD_TARGET_MS;
    let mut real_checks = 0usize;
    let mut misses: Vec<String> = Vec::new();
    for (assignment, total) in &order {
        let predicted = checks.iter().map(|check| simulated(check.class_at.iter().enumerate().map(|(pair, &class)| check.seconds[assignment[class]][pair]))).fold(0.0, f64::max);
        if ms(predicted) > target {
            continue;
        }
        real_checks += 1;
        let real: Vec<f64> = checks.iter().map(|check| ms(crowd::run_jobs(&mixed(check, assignment), WORKER_THREADS).0)).collect();
        let passes = real.iter().all(|&t| t <= target);
        let cells: Vec<String> = real.iter().map(|t| format!("{t:.1}")).collect();
        let row = format!("| {} | {:.1} | {:.3} | {:.1} | {} | {} |", names_of(coders, assignment), mb(*total as u64), total / (plain.scaled + plain.palettes), ms(predicted), cells.join(" / "), if passes { "yes" } else { "no" });
        if passes {
            misses.iter().rev().take(NEAR_MISSES_SHOWN).rev().for_each(|miss| println!("{miss}"));
            println!("{row}");
            let way = measured.way(coders, 0, &|body, _| assignment[ASSIGNED.iter().position(|&class| class == body.assigned).expect("an assigned class")]);
            println!("\nChecks, in order: {}. Prediction lets through {real_checks} real checks before this one passed.\n\n{WAY_HEADER}", checks.iter().map(|c| c.name.as_str()).collect::<Vec<&str>>().join("; "));
            print_way("assignment", &names_of(coders, assignment), &way, plain);
            return;
        }
        misses.push(row);
        if real_checks >= REAL_CHECKS {
            break;
        }
    }
    misses.iter().rev().take(NEAR_MISSES_SHOWN).rev().for_each(|miss| println!("{miss}"));
    println!("\nNo assignment passed within {REAL_CHECKS} real checks.");
}

fn client_part(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], rule: &BodyRule, client: Client, stride: usize, class_pixels: u64) {
    let started = Instant::now();
    let survey = survey(sources, bodies, present, rule, client);
    let infos: HashMap<usize, &BodyInfo> = survey.iter().map(|info| (info.body, info)).collect();
    let coders = coders_of(client);
    let plain_at = coders.iter().position(|c| c.name() == PLAIN_NAME).expect("the plain coder");

    // The measured bodies: spread over each class by pixels; equipment as thin as the sample, the others to the budget.
    let mut measured_bodies: Vec<usize> = Vec::new();
    for class in CLASSES {
        let of: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client && infos[&s.2].class == class).collect();
        let pixels: u64 = of.iter().map(|s| infos[&s.2].pixels).sum();
        let by_budget = pixels.div_ceil(class_pixels.max(1)).max(1) as usize;
        let step = if class == Class::Equipment { by_budget.max(stride) } else { by_budget };
        measured_bodies.extend(spread(&of, of.len().div_ceil(step), |s| s.1).into_iter().map(|s| s.2));
    }
    let measured_set: HashSet<usize> = measured_bodies.iter().copied().collect();

    println!("\n### {}: the classes; the whole set, and what this part codes\n\n| class | bodies | streams | Mpx | painted Mpx | bodies measured | streams measured | Mpx measured | painted Mpx measured |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|", client.label());
    let mut scale = [1.0f64; CLASSES.len()];
    for class in CLASSES {
        let sum = |only_measured: bool, pick: fn(&BodyInfo) -> u64| survey.iter().filter(|b| b.class == class && (!only_measured || measured_set.contains(&b.body))).map(pick).sum::<u64>();
        let bodies_of = |only_measured: bool| sum(only_measured, |_| 1);
        let mpx = |value: u64| value as f64 / PIXELS_PER_MEGAPIXEL;
        println!(
            "| {} | {} | {} | {:.1} | {:.1} | {} | {} | {:.1} | {:.1} |",
            class.label(),
            bodies_of(false),
            sum(false, |b| b.streams),
            mpx(sum(false, |b| b.pixels)),
            mpx(sum(false, |b| b.painted)),
            bodies_of(true),
            sum(true, |b| b.streams),
            mpx(sum(true, |b| b.pixels)),
            mpx(sum(true, |b| b.painted)),
        );
        if sum(true, |b| b.pixels) > 0 {
            scale[class.at()] = sum(false, |b| b.pixels) as f64 / sum(true, |b| b.pixels) as f64;
        }
    }

    // Coding and timing take turns.
    let mut batches: Vec<Vec<usize>> = vec![Vec::new()];
    let mut in_batch = 0u64;
    for &body in &measured_bodies {
        if in_batch >= BATCH_PIXELS {
            batches.push(Vec::new());
            in_batch = 0;
        }
        batches.last_mut().expect("a batch").push(body);
        in_batch += infos[&body].pixels;
    }
    let mut costs: Vec<BodyCost> = Vec::new();
    for (n, batch) in batches.iter().enumerate() {
        let ready: Vec<Ready> = batch.par_iter().map(|&i| prepare(Planes::from_body(&load_body(sources, &bodies[i])), None, &coders)).collect();
        let coded_at = started.elapsed().as_secs_f64();
        costs.extend(ready.iter().zip(batch).map(|(r, &i)| body_cost(r, infos[&i])));
        println!("  batch {} of {}: {} bodies; coded by {:.0} s, timed by {:.0} s; load average {}", n + 1, batches.len(), batch.len(), coded_at, started.elapsed().as_secs_f64(), load_average());
    }
    print_class_tables(client, &coders, &costs, plain_at);

    // The crowds.
    let pairs = crowd::all_pairs(sources, bodies, present, client);
    // Drawn by the rule, so they can differ a little from part `tc`'s, which go by `mobtypes.txt`.
    let npcs: Vec<Pair> = pairs.iter().copied().filter(|p| rule.group(&bodies[p.body()]) == Group::Creature).collect();
    let creatures = crowd::pick_uniform(&npcs, crowd::CROWD_PAIRS);
    let town = town_of(&pairs, &|body| rule.is_person(&bodies[body]), &|body| rule.group(&bodies[body]) == Group::Equipment);
    let mut actions = Map::<usize, BTreeSet<u16>>::new();
    creatures.iter().chain(&town).for_each(|p| {
        actions.entry(p.body()).or_default().insert(p.action());
    });
    let distinct: Vec<(usize, BTreeSet<u16>)> = actions.into_iter().collect();
    let position: HashMap<usize, usize> = distinct.iter().enumerate().map(|(n, entry)| (entry.0, n)).collect();
    let ready: Vec<Ready> = distinct.par_iter().map(|(i, wanted)| prepare(Planes::from_body(&load_body(sources, &bodies[*i])), Some(wanted), &coders)).collect();
    println!("\n  the crowds' {} bodies coded by {:.0} s", distinct.len(), started.elapsed().as_secs_f64());
    let mut checks = checks_of("creatures", &creatures, &ready, &position, &infos, coders.len());
    checks.extend(checks_of("dressed NPCs", &town, &ready, &position, &infos, coders.len()));
    println!("\n#### {}: each coder alone in the crowds, {WORKER_THREADS} threads, ms; {}\n\n| coder | {} | all inside {} ms |\n|---|{}---|", client.label(), load_average(), checks.iter().map(|c| c.name.as_str()).collect::<Vec<&str>>().join(" | "), crowd::CROWD_TARGET_MS, "---:|".repeat(checks.len()));
    let mut inside: Vec<usize> = Vec::new();
    for (c, coder) in coders.iter().enumerate() {
        let times: Vec<f64> = checks.iter().map(|check| ms(crowd::run_jobs(&check.jobs[c], WORKER_THREADS).0)).collect();
        let passes = times.iter().all(|&t| t <= crowd::CROWD_TARGET_MS);
        if passes {
            inside.push(c);
        }
        let cells: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
        println!("| {} | {} | {} |", coder.name(), cells.join(" | "), if passes { "yes" } else { "no" });
    }

    // Totals on the same bodies.
    let measured = Measured { client, costs: &costs, scale };
    let plain = measured.way(&coders, 0, &|_, _| plain_at);
    println!("\n#### {}: one coder, the best per class, the best per stream, on the same bodies; equipment and the like scaled by pixels to the whole set\n\n{WAY_HEADER}", client.label());
    print_way("one coder", PLAIN_NAME, &plain, &plain);
    let all: Vec<usize> = (0..coders.len()).collect();
    for (scope, allowed) in [("any coder", &all), ("coders that keep all crowds inside the line alone", &inside)] {
        if allowed.is_empty() {
            continue;
        }
        let single = allowed.iter().map(|&c| (c, measured.way(&coders, 0, &|_, _| c))).min_by(|a, b| (a.1.scaled + a.1.palettes).total_cmp(&(b.1.scaled + b.1.palettes))).expect("a coder");
        print_way(&format!("one coder, {scope}"), coders[single.0].name(), &single.1, &plain);
        let best_of_class: Vec<usize> = CLASSES.iter().map(|&class| allowed.iter().copied().min_by_key(|&c| class_sums(&costs, c, class).0).expect("a coder")).collect();
        let per_class = measured.way(&coders, 0, &|body, _| best_of_class[body.class.at()]);
        let picks: Vec<String> = CLASSES.iter().map(|class| format!("{}: {}", class.label(), coders[best_of_class[class.at()]].name())).collect();
        print_way(&format!("best per class, {scope}"), &picks.join("; "), &per_class, &plain);
        let best_of_stream = |body: &BodyCost, s: usize| allowed.iter().copied().min_by_key(|&c| body.coders[c].bytes[s]).expect("a coder");
        let per_stream = measured.way(&coders, STREAM_CHOICE_BYTES, &best_of_stream);
        print_way(&format!("best per stream, an id byte each, {scope}"), "as the stream prefers", &per_stream, &plain);
        let mut share = vec![0u64; coders.len()];
        for body in &costs {
            for s in 0..body.pixels.len() {
                share[best_of_stream(body, s)] += 1;
            }
        }
        let total: u64 = share.iter().sum();
        let mut top: Vec<(usize, u64)> = share.iter().copied().enumerate().filter(|row| row.1 > 0).collect();
        top.sort_by_key(|row| std::cmp::Reverse(row.1));
        let list: Vec<String> = top.iter().map(|&(c, count)| format!("{} {:.1} %", coders[c].name(), count as f64 * PERCENT / total.max(1) as f64)).collect();
        println!("  streams picking each, {scope}: {}", list.join("; "));
    }
    search(&measured, &coders, &checks, &plain);
    println!("\n{} done in {:.0} s", client.label(), started.elapsed().as_secs_f64());
}

pub(in super::super) fn part_tk(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    let rule = BodyRule::load(&kinds, cc_dir);
    let state = cpu_state();
    let class_pixels = std::env::var(CLASS_PIXELS_ENV).ok().map_or(CLASS_PIXELS, |n| n.parse::<u64>().expect("a pixel budget"));
    println!("\n### tk: coders by class of body; at most {:.0} Mpx per class, equipment at most every {stride}-th body; CPU {state}; load average {}; classes from each client's tileart.uop placeholders, mobtypes.txt for what has none", class_pixels as f64 / PIXELS_PER_MEGAPIXEL, load_average());
    println!("Classes by the median cropped frame area: small up to {NPC_SMALL_TOP} px, medium up to {NPC_MEDIUM_TOP} px, large above; a not listed body is reported apart, and an assignment gives it the coder of its size.");
    for client in [Client::Cc, Client::Ec] {
        client_part(sources, bodies, present, &rule, client, stride, class_pixels);
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part tk ({state} at the start, {end} at the end): discard its timings");
    }
    println!("Load average at the end of part tk {}", load_average());
}
