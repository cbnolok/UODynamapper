//! The size-class, clip and crowd studies: bodies grouped by their largest frame, a body's frames laid out on pages of
//! their own as the packer would (`repack.rs`), the decode of a body's clips per method, and a crowd appearing under
//! the loader the user chose (`docs/agent_notes/uddp_anim_format_requirements.md`, "The loader the user chose"), with
//! and without a cache of decoded pages.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use clap::{Args, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use udd_assets::animation::mobile_anim_cc::classic_stored_direction;
use udd_assets::animation::move_actions::{GROUP_PEACE, MOVE_RUN, MOVE_STAND, MOVE_WALK};

use crate::backends::{self, Backend};
use crate::corpus::{Anim, BLOCK_SIDE, FrameInfo, Package};
use crate::repack::{self, PlannedPage, SlotFrame, SourcePage};
use crate::transforms::{BLOCK_BYTES, BlockRect, Transform, fields_extract};
use crate::{GAME_DECODE_THREADS, MS_PER_S, Method, PERCENT, StoredPage, Target, load, mib, percentile, store};

/// World directions of a mobile (0 north, clockwise).
const DIRECTIONS: u8 = 8;
/// The movement set a mobile loads on appearance, unmounted: stand, walk, run.
const MOVEMENTS: [u8; 3] = [MOVE_STAND, MOVE_WALK, MOVE_RUN];
/// The bodies of the `people` crowd: the Classic and Enhanced male and female human.
const PEOPLE_BODIES: [u32; 2] = [400, 401];
/// A cold page's read from the NVMe, per stored MiB (measured for the first study: a random direct read of a page-sized
/// span takes 0.4 to 0.7 ms).
const COLD_READ_MS_PER_MIB: f64 = 0.8;
/// The level the built pages are held at between the methods' passes (memory only, not measured).
const HOLD_ZSTD_LEVEL: i32 = 1;
/// Threads that build pages from their sources at once: each holds the decoded source pages of its page.
const BUILD_THREADS: usize = 8;
/// Suffix of a crowd method whose clips join only their own blocks of a `fields` page (`fields_extract`).
const PER_CLIP_SUFFIX: &str = "@clip";

/// Size classes by the largest side of a body's stored frames: class `i` holds the bodies whose side is at most
/// `bounds[i]` pixels (and above the previous bound); one class more holds the rest.
#[derive(Debug, Clone)]
pub struct Classes {
    bounds: Vec<u32>,
}

impl Classes {
    pub(crate) fn new(mut bounds: Vec<u32>) -> Self {
        bounds.sort_unstable();
        bounds.dedup();
        Self { bounds }
    }

    pub(crate) fn of(&self, side: u32) -> usize {
        self.bounds.iter().position(|&bound| side <= bound).unwrap_or(self.bounds.len())
    }

    fn count(&self) -> usize {
        self.bounds.len() + 1
    }

    pub(crate) fn name(&self, class: usize) -> String {
        match (class.checked_sub(1).map(|previous| self.bounds[previous]), self.bounds.get(class)) {
            (None, Some(bound)) => format!("≤ {bound} px"),
            (Some(low), Some(bound)) => format!("{}–{bound} px", low + 1),
            (Some(low), None) => format!("> {low} px"),
            (None, None) => "all".into(),
        }
    }
}

/// Each body's largest stored frame side: the most of its placed frames' widths and heights.
pub(crate) fn body_sides(package: &Package) -> BTreeMap<u32, u32> {
    let mut sides = BTreeMap::new();
    for frame in package.frames.iter().filter(|frame| frame.placed()) {
        let side = sides.entry(frame.owner_body).or_insert(0);
        *side = (*side).max(u32::from(frame.width.max(frame.height)));
    }
    sides
}

/// The blocks a frame's slot takes on a page: its content padded to blocks, and the gutter on each side.
fn slot_blocks(frame: &FrameInfo, gutter: u16) -> u64 {
    let gutter = u64::from(u32::from(gutter).div_ceil(BLOCK_SIDE));
    let side = |extent: u16| u64::from(u32::from(extent).div_ceil(BLOCK_SIDE)) + 2 * gutter;
    side(frame.width) * side(frame.height)
}

/// The placements of `frames`: frames on the same page at the same corner (Enhanced records of one source frame) are
/// one placement, ordered by the least of their keys; each with the frames that share it.
fn placements(package: &Package, frames: impl Iterator<Item = usize>) -> Vec<(SlotFrame, Vec<usize>)> {
    let mut by_corner: BTreeMap<(u32, u16, u16), Vec<usize>> = BTreeMap::new();
    for frame in frames {
        let info = &package.frames[frame];
        by_corner.entry((info.page, info.x, info.y)).or_default().push(frame);
    }
    by_corner.into_values().map(|members| {
        let first = *members.iter().min_by_key(|&&frame| package.frames[frame].order).expect("a member");
        let info = &package.frames[first];
        (SlotFrame { frame: first, key: info.order, width: info.width, height: info.height }, members)
    }).collect()
}

fn frame_blocks(frame: &FrameInfo) -> BlockRect {
    repack::content_blocks(u32::from(frame.x), u32::from(frame.y), frame.width, frame.height)
}

/// The blocks of `rects` cut from a whole page, as the reader does after the page's decode
/// (`udd_assets::common::extract_atlas_subrect_stored`).
fn cut(page: &[u8], blocks_wide: usize, rects: &[BlockRect], out: &mut Vec<u8>) {
    out.clear();
    for rect in rects {
        for row in rect.y..rect.y + rect.height {
            let start = (row * blocks_wide + rect.x) * BLOCK_BYTES;
            out.extend_from_slice(&page[start..start + rect.width * BLOCK_BYTES]);
        }
    }
}

#[derive(Args)]
pub struct ClassesArgs {
    #[command(flatten)]
    target: Target,
    /// Upper bounds of the classes' largest frame side, in pixels.
    #[arg(long, value_delimiter = ',', required = true)]
    bounds: Vec<u32>,
    /// Bodies to show with their class.
    #[arg(long, value_delimiter = ',')]
    bodies: Vec<u32>,
    /// Replays the packer on every frame and compares the layout with the package's, and every n-th page built from
    /// it with the stored page byte for byte (0: no replay).
    #[arg(long, default_value_t = 0)]
    check_every: usize,
}

pub fn classes(args: &ClassesArgs) -> eyre::Result<()> {
    let package = Package::open(&args.target.package, args.target.kind)?;
    let classes = Classes::new(args.bounds.clone());
    let sides = body_sides(&package);
    let position: HashMap<u32, usize> = package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();

    // Each page's stored bytes, shared among its frames by their slots' blocks.
    let mut page_blocks = vec![0u64; package.pages.len()];
    for frame in package.frames.iter().filter(|frame| frame.placed()) {
        page_blocks[position[&frame.page]] += slot_blocks(frame, package.gutter);
    }
    #[derive(Default, Clone)]
    struct Tally {
        bodies: usize,
        frames: usize,
        clips: usize,
        slot_blocks: u64,
        stored: f64,
        pages: BTreeSet<usize>,
    }
    let mut tallies = vec![Tally::default(); classes.count()];
    let mut by_body: BTreeMap<u32, Tally> = BTreeMap::new();
    for frame in package.frames.iter().filter(|frame| frame.placed()) {
        let at = position[&frame.page];
        let blocks = slot_blocks(frame, package.gutter);
        let share = f64::from(package.pages[at].entry.stored_size) * blocks as f64 / page_blocks[at] as f64;
        for tally in [&mut tallies[classes.of(sides[&frame.owner_body])], by_body.entry(frame.owner_body).or_default()] {
            tally.frames += 1;
            tally.slot_blocks += blocks;
            tally.stored += share;
            tally.pages.insert(at);
        }
    }
    for clip in &package.clips {
        if let Some(&side) = sides.get(&clip.body) {
            tallies[classes.of(side)].clips += 1;
            by_body.entry(clip.body).or_default().clips += 1;
        }
    }
    for &side in sides.values() {
        tallies[classes.of(side)].bodies += 1;
    }

    let mut weighted: Vec<(u32, u64)> = by_body.iter().map(|(body, tally)| (sides[body], tally.slot_blocks)).collect();
    weighted.sort_unstable();
    let total_blocks: u64 = weighted.iter().map(|&(_, blocks)| blocks).sum();
    let by_count: Vec<f64> = weighted.iter().map(|&(side, _)| f64::from(side)).collect();
    print!("largest frame side by body, deciles:");
    for decile in 1..=10 {
        print!(" {:.0}", percentile(&by_count, f64::from(decile) * 10.0));
    }
    print!("\nlargest frame side by slot blocks (the side below which that share of the pages' frame blocks lies):");
    let mut running = 0u64;
    let mut next = 1;
    for &(side, blocks) in &weighted {
        running += blocks;
        while next <= 10 && running * 10 >= total_blocks * next {
            print!(" {next}0 %: {side}");
            next += 1;
        }
    }
    println!();

    let stored_total: f64 = package.pages.iter().map(|page| f64::from(page.entry.stored_size)).sum();
    println!("\n| Class | Bodies | Frames | Clips | Slot MiB (raw) | Stored MiB (shared by slot) | Share of the pages | \
        Pages touched |\n|---|---:|---:|---:|---:|---:|---:|---:|");
    for (class, tally) in tallies.iter().enumerate() {
        println!("| {} | {} | {} | {} | {:.1} | {:.1} | {:.1} % | {} |", classes.name(class), tally.bodies,
            tally.frames, tally.clips, mib(tally.slot_blocks * BLOCK_BYTES as u64), tally.stored / crate::BYTES_PER_MIB,
            tally.stored * PERCENT / stored_total, tally.pages.len());
    }
    if !args.bodies.is_empty() {
        println!("\n| Body | Largest side | Class | Frames | Clips | Stored MiB (shared) | Pages touched |\n\
            |---:|---:|---|---:|---:|---:|---:|");
        for body in &args.bodies {
            let Some(tally) = by_body.get(body) else {
                println!("| {body} | | not in the package | | | | |");
                continue;
            };
            println!("| {body} | {} | {} | {} | {} | {:.2} | {} |", sides[body], classes.name(classes.of(sides[body])),
                tally.frames, tally.clips, tally.stored / crate::BYTES_PER_MIB, tally.pages.len());
        }
    }
    if args.check_every > 0 {
        check_replay(&package, args.check_every)?;
    }
    Ok(())
}

/// Replays the packer on every placed frame and compares the layout and every `every`-th page with the package's.
fn check_replay(package: &Package, every: usize) -> eyre::Result<()> {
    let start = Instant::now();
    let units = placements(package, (0..package.frames.len()).filter(|&frame| package.frames[frame].placed()));
    let slots: Vec<SlotFrame> = units.iter().map(|(slot, _)| *slot).collect();
    let pages = repack::plan(&slots, package.gutter).map_err(|error| eyre::eyre!(error))?;
    eprintln!("replayed {} frames onto {} pages in {:.1} s", slots.len(), pages.len(), start.elapsed().as_secs_f64());
    let mut misplaced = 0usize;
    let mut extents = 0usize;
    for (at, page) in pages.iter().enumerate() {
        let stored = package.pages.get(at);
        if stored.is_none_or(|stored| (stored.used_width, stored.used_height) != (page.used_width, page.used_height)) {
            extents += 1;
        }
        for placed in &page.frames {
            let frame = &package.frames[placed.frame];
            if stored.is_none_or(|stored| stored.index != frame.page)
                || (u32::from(frame.x), u32::from(frame.y)) != (placed.x, placed.y) {
                misplaced += 1;
            }
        }
    }
    println!("\nreplay: {} pages against {} stored; {} pages of another used extent; {misplaced} of {} placements \
        elsewhere", pages.len(), package.pages.len(), extents, slots.len());
    // Whether the stored pages follow the replay's order: each page's placements all after the previous page's.
    let mut key_range = vec![(u64::MAX, 0u64); package.pages.len()];
    let position: HashMap<u32, usize> = package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();
    for (slot, _) in &units {
        let range = &mut key_range[position[&package.frames[slot.frame].page]];
        *range = (range.0.min(slot.key), range.1.max(slot.key));
    }
    let ordered = key_range.windows(2).filter(|pair| pair[0].1 < pair[1].0).count();
    println!("stored pages in the replay's order: {ordered} of {} page boundaries", key_range.len().saturating_sub(1));
    let background = repack::most_common_block(&package.raw(&package.pages[0])?);
    let comparable = pages.len().min(package.pages.len());
    let checked: Vec<(usize, bool, bool)> = pages[..comparable].par_iter().enumerate().step_by(every).map(|(at, page)| {
        let built = build_page(package, page, &background)?;
        let stored = package.raw(&package.pages[at])?;
        Ok((at, built.len() == stored.len(), built == stored))
    }).collect::<eyre::Result<_>>()?;
    let same = checked.iter().filter(|(_, _, same)| *same).count();
    let same_length = checked.iter().filter(|(_, length, _)| *length).count();
    println!("built pages: {same} of {} byte for byte the stored page ({same_length} of the stored length)",
        checked.len());
    for (at, length, same) in checked.iter().filter(|(_, _, same)| !same).take(5) {
        println!("  page {at}: length {}, bytes {}", if *length { "same" } else { "differs" },
            if *same { "same" } else { "differ" });
    }
    Ok(())
}

/// The blocks of a planned page, from the stored pages its frames come from.
fn build_page(package: &Package, page: &PlannedPage, background: &[u8; BLOCK_BYTES]) -> eyre::Result<Vec<u8>> {
    let sources: BTreeSet<u32> = page.frames.iter().map(|placed| package.frames[placed.frame].page).collect();
    let mut loaded: HashMap<u32, (Vec<u8>, usize)> = HashMap::new();
    for index in sources {
        let stored = package.page(index).ok_or_else(|| eyre::eyre!("no page {index}"))?;
        let raw = package.raw(stored)?;
        let blocks_wide = stored.blocks_wide(raw.len());
        loaded.insert(index, (raw, blocks_wide));
    }
    repack::build(page, &package.frames, package.gutter, background,
        |index| loaded.get(&index).map(|(blocks, blocks_wide)| SourcePage { blocks, blocks_wide: *blocks_wide }))
        .map_err(|error| eyre::eyre!(error))
}

/// What a page costs to read under a method: its stored bytes, its decode (the coder, then the transform's whole-page
/// inverse), and for each clip on it the cut of its blocks from the whole page and their join from the split stream
/// (`fields` only; the cut otherwise). Times in ms, the fastest of the runs.
#[derive(Debug, Clone, Default)]
struct PageCost {
    raw: usize,
    bytes: usize,
    decompress: f64,
    inverse: f64,
    cut: Vec<f64>,
    extract: Vec<f64>,
}

fn splits_fields(transform: Transform) -> bool {
    transform == Transform::Fields { shuffle: false }
}

fn coded(method: Method) -> eyre::Result<(Transform, Backend)> {
    match method {
        Method::Coded { transform, backend } if !backend.needs_dictionary() => Ok((transform, backend)),
        _ => eyre::bail!("{}: the studies take a transform and a backend without a dictionary", method.name()),
    }
}

/// Stores `raw` by `method` and checks the round trip.
fn encode_page(raw: &[u8], blocks_wide: usize, method: Method) -> eyre::Result<StoredPage> {
    let (transform, backend) = coded(method)?;
    let stored = store(transform, backend, raw, blocks_wide, None, &mut Vec::new())?;
    let (back, _, _) = load(&stored, None)?;
    eyre::ensure!(back == raw, "{} does not round-trip", method.name());
    Ok(stored)
}

/// Times the decode of `stored` and the reads of `clips` (each clip's block rectangles on the page).
fn time_page(stored: &StoredPage, clips: &[Vec<BlockRect>], reps: usize) -> eyre::Result<PageCost> {
    let reps = reps.max(1);
    let mut cost = PageCost { raw: stored.raw_len, bytes: stored.bytes, decompress: f64::MAX, inverse: f64::MAX,
        ..PageCost::default() };
    let mut page = Vec::new();
    for _ in 0..reps {
        let (decoded, decompress, inverse) = load(stored, None)?;
        cost.decompress = cost.decompress.min(decompress.as_secs_f64() * MS_PER_S);
        cost.inverse = cost.inverse.min(inverse.as_secs_f64() * MS_PER_S);
        page = decoded;
    }
    let mut out = Vec::new();
    let best = |work: &mut dyn FnMut() -> eyre::Result<()>| -> eyre::Result<f64> {
        let mut best = f64::MAX;
        for _ in 0..reps {
            let start = Instant::now();
            work()?;
            best = best.min(start.elapsed().as_secs_f64() * MS_PER_S);
        }
        Ok(best)
    };
    for rects in clips {
        cost.cut.push(best(&mut || {
            cut(&page, stored.blocks_wide, rects, &mut out);
            Ok(())
        })?);
    }
    if splits_fields(stored.transform) {
        let mut stream = Vec::new();
        backends::decode_into(&stored.encoded, stored.transformed_len, None, &mut stream)
            .map_err(|error| eyre::eyre!("{error}"))?;
        let mut joined = Vec::new();
        for rects in clips {
            cost.extract.push(best(&mut || {
                fields_extract(&stream, stored.raw_len, stored.blocks_wide, rects, &mut joined)
                    .map_err(|error| eyre::eyre!("{error}"))
            })?);
            cut(&page, stored.blocks_wide, rects, &mut out);
            eyre::ensure!(joined == out, "the clip's blocks joined from the split stream differ from the page's");
        }
    } else {
        cost.extract = cost.cut.clone();
    }
    Ok(cost)
}

/// A layout of the frames on pages: the package's own, or a replay of the packer on chosen frames.
struct Layout {
    /// Each page's blocks per row, and its blocks held at a fast Zstd level when built here.
    pages: Vec<LayoutPage>,
    /// For each frame record, its page in `pages` and its block rectangle there.
    places: Vec<Option<(usize, BlockRect)>>,
}

struct LayoutPage {
    blocks_wide: usize,
    class: usize,
    held: Held,
}

enum Held {
    /// The package's page at this position.
    Stored(usize),
    Built(Vec<u8>),
}

impl Layout {
    fn raw(&self, package: &Package, at: usize) -> eyre::Result<Vec<u8>> {
        match &self.pages[at].held {
            Held::Stored(position) => package.raw(&package.pages[*position]),
            Held::Built(held) => zstd::bulk::decompress(held, (2048 / BLOCK_SIDE as usize).pow(2) * BLOCK_BYTES)
                .wrap_err("unhold a built page"),
        }
    }

    /// The package's pages as they are.
    fn stored(package: &Package, class_of_page: impl Fn(usize) -> usize) -> eyre::Result<Self> {
        let position: HashMap<u32, usize> = package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();
        let pages = package.pages.iter().enumerate().map(|(at, page)| LayoutPage {
            blocks_wide: page.blocks_wide(page.entry.raw_size as usize),
            class: class_of_page(at),
            held: Held::Stored(at),
        }).collect();
        let places = package.frames.iter().map(|frame| frame.placed().then(|| (position[&frame.page], frame_blocks(frame))))
            .collect();
        Ok(Self { pages, places })
    }

    /// Each group's frames (frame record indices) on pages of their own, as the packer would lay them out, the
    /// groups one after another.
    fn built(package: &Package, groups: &[Vec<usize>]) -> eyre::Result<Self> {
        let background = repack::most_common_block(&package.raw(&package.pages[0])?);
        let mut planned: Vec<(usize, PlannedPage)> = Vec::new();
        let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
        for (class, frames) in groups.iter().enumerate() {
            let units = placements(package, frames.iter().copied());
            let slots: Vec<SlotFrame> = units.iter().map(|(slot, _)| *slot).collect();
            members.extend(units.into_iter().map(|(slot, frames)| (slot.frame, frames)));
            let start = Instant::now();
            let pages = repack::plan(&slots, package.gutter).map_err(|error| eyre::eyre!(error))?;
            if slots.len() > 1000 {
                eprintln!("group {class}: {} placements onto {} pages in {:.1} s", slots.len(), pages.len(),
                    start.elapsed().as_secs_f64());
            }
            planned.extend(pages.into_iter().map(|page| (class, page)));
        }
        let mut places = vec![None; package.frames.len()];
        for (at, (_, page)) in planned.iter().enumerate() {
            for placed in &page.frames {
                let frame = &package.frames[placed.frame];
                let rect = repack::content_blocks(placed.x, placed.y, frame.width, frame.height);
                for &member in &members[&placed.frame] {
                    places[member] = Some((at, rect));
                }
            }
        }
        let pool = rayon::ThreadPoolBuilder::new().num_threads(BUILD_THREADS).build()?;
        let pages = pool.install(|| planned.par_iter().map(|(class, page)| {
            let blocks = build_page(package, page, &background)?;
            Ok(LayoutPage {
                blocks_wide: page.blocks_wide(),
                class: *class,
                held: Held::Built(zstd::bulk::compress(&blocks, HOLD_ZSTD_LEVEL)?),
            })
        }).collect::<eyre::Result<Vec<_>>>())?;
        Ok(Self { pages, places })
    }

    /// For each page, the clips (indices into `package.clips`) with frames on it and their rectangles there; for each
    /// clip, its pages and its entry in that page's list.
    fn clips(&self, package: &Package) -> (Vec<Vec<(usize, Vec<BlockRect>)>>, Vec<Vec<(usize, usize)>>) {
        let mut page_clips: Vec<Vec<(usize, Vec<BlockRect>)>> = vec![Vec::new(); self.pages.len()];
        let mut clip_pages = vec![Vec::new(); package.clips.len()];
        for (clip, info) in package.clips.iter().enumerate() {
            let mut by_page: BTreeMap<usize, Vec<BlockRect>> = BTreeMap::new();
            let frames = info.frame_start as usize..(info.frame_start + info.frame_count) as usize;
            for frame in frames {
                if let Some(Some((page, rect))) = self.places.get(frame) {
                    by_page.entry(*page).or_default().push(*rect);
                }
            }
            for (page, rects) in by_page {
                clip_pages[clip].push((page, page_clips[page].len()));
                page_clips[page].push((clip, rects));
            }
        }
        (page_clips, clip_pages)
    }
}

#[derive(Args)]
pub struct BodiesArgs {
    #[command(flatten)]
    target: Target,
    #[arg(long, value_delimiter = ',', required = true)]
    bodies: Vec<u32>,
    /// Comma-separated methods (`<transform>+<backend>`, no dictionary).
    #[arg(long, value_delimiter = ',', required = true)]
    methods: Vec<String>,
    /// Upper bounds of the size classes, to name each body's.
    #[arg(long, value_delimiter = ',', required = true)]
    bounds: Vec<u32>,
    /// Decodes of each page per method; the fastest counts.
    #[arg(long, default_value_t = 5)]
    reps: usize,
}

/// Each body's frames on pages of their own, every clip of the body read from them on one thread.
pub fn bodies(args: &BodiesArgs) -> eyre::Result<()> {
    let package = Package::open(&args.target.package, args.target.kind)?;
    let methods: Vec<Method> = args.methods.iter().map(|name| Method::parse(name)).collect::<eyre::Result<_>>()?;
    let classes = Classes::new(args.bounds.clone());
    let sides = body_sides(&package);
    println!("| Body | Class | Pages | Clips | Method | Raw MiB | Stored MiB | Raw / stored | vs first | Decode MB/s \
        | Clip ms p50 / p95, whole page | Clip set ms, whole page | Clip ms p50 / p95, per clip | Clip set ms, per clip |\n\
        |---:|---|---:|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for &body in &args.bodies {
        let Some(&side) = sides.get(&body) else {
            println!("| {body} | not in the package |||||||||||||");
            continue;
        };
        let frames: Vec<usize> = package.frames.iter().enumerate()
            .filter(|(_, frame)| frame.placed() && frame.owner_body == body).map(|(frame, _)| frame).collect();
        let layout = Layout::built(&package, &[frames])?;
        let (page_clips, clip_pages) = layout.clips(&package);
        let clips: Vec<usize> = (0..package.clips.len()).filter(|&clip| !clip_pages[clip].is_empty()).collect();
        let raws: Vec<Vec<u8>> = (0..layout.pages.len()).map(|at| layout.raw(&package, at)).collect::<eyre::Result<_>>()?;
        let mut first_bytes = None;
        for &method in &methods {
            let stored: Vec<StoredPage> = raws.par_iter().zip(&layout.pages)
                .map(|(raw, page)| encode_page(raw, page.blocks_wide, method)).collect::<eyre::Result<_>>()?;
            let costs: Vec<PageCost> = stored.iter().zip(&page_clips).map(|(stored, clips)| {
                let rects: Vec<Vec<BlockRect>> = clips.iter().map(|(_, rects)| rects.clone()).collect();
                time_page(stored, &rects, args.reps)
            }).collect::<eyre::Result<_>>()?;
            let raw: usize = costs.iter().map(|cost| cost.raw).sum();
            let bytes: usize = costs.iter().map(|cost| cost.bytes).sum();
            let decode: f64 = costs.iter().map(|cost| cost.decompress + cost.inverse).sum();
            let first = *first_bytes.get_or_insert(bytes);
            let clip_ms = |per_clip: bool| -> (f64, f64, f64) {
                let mut times: Vec<f64> = clips.iter().map(|&clip| clip_pages[clip].iter().map(|&(page, entry)| {
                    let cost = &costs[page];
                    if per_clip { cost.decompress + cost.extract[entry] } else {
                        cost.decompress + cost.inverse + cost.cut[entry] }
                }).sum()).collect();
                let total = times.iter().sum();
                times.sort_by(f64::total_cmp);
                (percentile(&times, 50.0), percentile(&times, 95.0), total)
            };
            let (whole_50, whole_95, whole_total) = clip_ms(false);
            let per_clip = if let Method::Coded { transform, .. } = method && splits_fields(transform) {
                let (p50, p95, total) = clip_ms(true);
                (format!("{p50:.2} / {p95:.2}"), format!("{total:.0}"))
            } else {
                (String::new(), String::new())
            };
            println!("| {body} | {} ({side} px) | {} | {} | {} | {:.2} | {:.3} | {:.2} | {:+.2} % | {:.0} | {whole_50:.2} \
                / {whole_95:.2} | {whole_total:.0} | {} | {} |", classes.name(classes.of(side)), layout.pages.len(),
                clips.len(), method.name(), mib(raw as u64), mib(bytes as u64), raw as f64 / bytes as f64,
                (bytes as f64 / first as f64 - 1.0) * PERCENT, raw as f64 / crate::BYTES_PER_MB / (decode / MS_PER_S),
                per_clip.0, per_clip.1);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LayoutKind {
    /// The package's own pages.
    Stored,
    /// Each size class's bodies on pages of their own.
    Classes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mix {
    /// Bodies drawn uniformly from those with a movement set.
    Random,
    /// Humans only (bodies 400 and 401).
    People,
}

#[derive(Args)]
pub struct CrowdArgs {
    #[command(flatten)]
    target: Target,
    #[arg(long, value_enum, default_value = "stored")]
    layout: LayoutKind,
    /// Upper bounds of the size classes (for the `classes` layout and the per-class tables).
    #[arg(long, value_delimiter = ',', required = true)]
    bounds: Vec<u32>,
    /// `name=spec`: one method for every page, or one per size class separated by '/'; a `fields` method ending in
    /// `@clip` joins only the clip's blocks instead of inverting the whole page. Repeat for each configuration.
    #[arg(long = "config", required = true)]
    configs: Vec<String>,
    /// Decoded pages the reader keeps (0: none, as today).
    #[arg(long, value_delimiter = ',', default_value = "0,8,16,32")]
    lru: Vec<usize>,
    /// Crowds simulated; the medians are shown.
    #[arg(long, default_value_t = 20)]
    crowds: usize,
    /// Mobiles appearing at once: 34 give about 100 clips for their faced directions.
    #[arg(long, default_value_t = 34)]
    mobiles: usize,
    /// Comma-separated crowd mixes.
    #[arg(long, value_enum, value_delimiter = ',', default_value = "random")]
    mix: Vec<Mix>,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Decodes of each page per method; the fastest counts.
    #[arg(long, default_value_t = 3)]
    reps: usize,
    /// With the stored layout: each page's blocks from `<dir>/<page position>.zst` instead of the package (a variant
    /// `lossy --store` wrote), the package giving the records.
    #[arg(long)]
    pages: Option<std::path::PathBuf>,
}

/// A configuration: for each size class, the method its pages are stored with and whether a clip joins only its own
/// blocks.
struct Config {
    name: String,
    per_class: Vec<(usize, bool)>,
}

/// The clip of a mobile's (body, action, direction), as the game's reader resolves it, as an index into
/// `package.clips`.
fn resolve_clip(package: &Package, by_frames: &HashMap<(u32, u32), usize>, body: u32, action: u16, direction: u8)
    -> Option<usize> {
    let (start, count) = match &package.anim {
        Anim::Cc(anim) => {
            let (stored, _) = classic_stored_direction(direction)?;
            let record = anim.server_body_animation(u16::try_from(body).ok()?, action, stored)?;
            (record.frame_start, u32::from(record.frame_count))
        }
        Anim::Ec(anim) => {
            let resolved = anim.resolve_animation(body, action, direction, false)?;
            (resolved.record.frame_start, u32::from(resolved.record.frame_count))
        }
    };
    by_frames.get(&(start, count)).copied()
}

fn movement_action(package: &Package, body: u32, movement: u8) -> Option<u16> {
    match &package.anim {
        Anim::Cc(anim) => anim.movement_action(u16::try_from(body).ok()?, movement, false),
        Anim::Ec(anim) => anim.move_action(body, GROUP_PEACE, movement),
    }
}

/// A clip asked for: its index in `package.clips`.
struct Request {
    clip: usize,
}

/// A crowd's requests in the loader's order, and for each mobile its faced clips' requests.
struct Crowd {
    requests: Vec<Request>,
    faced: Vec<Vec<usize>>,
}

/// A small deterministic generator (SplitMix64), so a crowd can be drawn again.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// The loader's order for `mobiles` (body, faced direction) appearing at once: every mobile's movement set facing its
/// direction first, then the next direction clockwise of every mobile, and so on; a clip already asked for is not
/// asked again (`MobileClips` keys by body, action and direction).
fn crowd(package: &Package, by_frames: &HashMap<(u32, u32), usize>, mobiles: &[(u32, u8)]) -> Crowd {
    let mut asked: HashMap<(u32, u16, u8), usize> = HashMap::new();
    let mut requests = Vec::new();
    let mut faced = vec![Vec::new(); mobiles.len()];
    for turn in 0..DIRECTIONS {
        for (mobile, &(body, facing)) in mobiles.iter().enumerate() {
            let direction = (facing + turn) % DIRECTIONS;
            for movement in MOVEMENTS {
                let Some(action) = movement_action(package, body, movement) else { continue };
                let Some(clip) = resolve_clip(package, by_frames, body, action, direction) else { continue };
                let request = *asked.entry((body, action, direction)).or_insert_with(|| {
                    requests.push(Request { clip });
                    requests.len() - 1
                });
                if turn == 0 {
                    faced[mobile].push(request);
                }
            }
        }
    }
    Crowd { requests, faced }
}

/// What a page costs a clip under a configuration.
struct Costs<'a> {
    pages: Vec<(&'a PageCost, bool)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Outcome {
    decodes: usize,
    busy_ms: f64,
    first_drawable_ms: f64,
    mobile_p50_ms: f64,
    all_ms: f64,
}

/// The crowd's clips on the game's decode workers, first come first served: each clip decodes every page it touches
/// unless the reader's cache of `lru` decoded pages holds it (a page another worker is decoding is waited for), then
/// reads its blocks. `cold`: a page's first read in the crowd comes from the disk.
fn simulate(crowd: &Crowd, clip_pages: &[Vec<(usize, usize)>], costs: &Costs, lru: usize, cold: bool) -> Outcome {
    let mut free = [0.0f64; GAME_DECODE_THREADS];
    let mut cache: HashMap<usize, (f64, u64)> = HashMap::new();
    let mut read = vec![false; costs.pages.len()];
    let mut tick = 0u64;
    let mut outcome = Outcome::default();
    let mut finish = Vec::with_capacity(crowd.requests.len());
    for request in &crowd.requests {
        let worker = (0..GAME_DECODE_THREADS).min_by(|&a, &b| free[a].total_cmp(&free[b])).expect("workers");
        let mut now = free[worker];
        for &(page, entry) in &clip_pages[request.clip] {
            let (cost, per_clip) = costs.pages[page];
            tick += 1;
            if let Some((ready, used)) = cache.get_mut(&page) {
                now = now.max(*ready);
                *used = tick;
            } else {
                let mut decode = cost.decompress + if per_clip { 0.0 } else { cost.inverse };
                if cold && !read[page] {
                    read[page] = true;
                    decode += cost.bytes as f64 / crate::BYTES_PER_MIB * COLD_READ_MS_PER_MIB;
                }
                outcome.decodes += 1;
                outcome.busy_ms += decode;
                now += decode;
                if lru > 0 {
                    if cache.len() >= lru {
                        let oldest = *cache.iter().min_by_key(|(_, (_, used))| *used).expect("a full cache").0;
                        cache.remove(&oldest);
                    }
                    cache.insert(page, (now, tick));
                }
            }
            let read_blocks = if per_clip { cost.extract[entry] } else { cost.cut[entry] };
            outcome.busy_ms += read_blocks;
            now += read_blocks;
        }
        free[worker] = now;
        finish.push(now);
    }
    let mut mobiles: Vec<f64> = crowd.faced.iter().filter(|faced| !faced.is_empty())
        .map(|faced| faced.iter().map(|&request| finish[request]).fold(0.0, f64::max)).collect();
    mobiles.sort_by(f64::total_cmp);
    outcome.first_drawable_ms = mobiles.last().copied().unwrap_or(0.0);
    outcome.mobile_p50_ms = percentile(&mobiles, 50.0);
    outcome.all_ms = finish.iter().copied().fold(0.0, f64::max);
    outcome
}

pub fn crowd_study(args: &CrowdArgs) -> eyre::Result<()> {
    let package = Package::open(&args.target.package, args.target.kind)?;
    let classes = Classes::new(args.bounds.clone());
    let sides = body_sides(&package);
    let class_of_body = |body: u32| sides.get(&body).map_or(0, |&side| classes.of(side));

    // The methods the configurations name, each measured once on every page.
    let mut methods: Vec<Method> = Vec::new();
    let mut configs = Vec::new();
    for text in &args.configs {
        let (name, spec) = text.split_once('=').ok_or_else(|| eyre::eyre!("config {text}: no '='"))?;
        let parts: Vec<&str> = spec.split('/').collect();
        eyre::ensure!(parts.len() == 1 || parts.len() == classes.count(),
            "config {name}: {} methods for {} classes", parts.len(), classes.count());
        let mut per_class = Vec::new();
        for class in 0..classes.count() {
            let part = parts[class.min(parts.len() - 1)];
            let (method_name, per_clip) = match part.strip_suffix(PER_CLIP_SUFFIX) {
                Some(method_name) => (method_name, true),
                None => (part, false),
            };
            let method = Method::parse(method_name)?;
            let (transform, _) = coded(method)?;
            eyre::ensure!(!per_clip || splits_fields(transform), "{part}: only a fields method joins per clip");
            let at = methods.iter().position(|known| known.name() == method.name()).unwrap_or_else(|| {
                methods.push(method);
                methods.len() - 1
            });
            per_class.push((at, per_clip));
        }
        eyre::ensure!(args.layout == LayoutKind::Classes || per_class.iter().all(|&choice| choice == per_class[0]),
            "config {name}: per-class methods need the classes layout");
        configs.push(Config { name: name.to_string(), per_class });
    }

    let start = Instant::now();
    let layout = match args.layout {
        LayoutKind::Stored => {
            // A stored page's class: the class of most of its slot blocks.
            let position: HashMap<u32, usize> =
                package.pages.iter().enumerate().map(|(at, page)| (page.index, at)).collect();
            let mut blocks = vec![vec![0u64; classes.count()]; package.pages.len()];
            for frame in package.frames.iter().filter(|frame| frame.placed()) {
                blocks[position[&frame.page]][class_of_body(frame.owner_body)] += slot_blocks(frame, package.gutter);
            }
            let mut layout =
                Layout::stored(&package, |at| (0..classes.count()).max_by_key(|&class| blocks[at][class]).unwrap_or(0))?;
            if let Some(dir) = &args.pages {
                for (at, page) in layout.pages.iter_mut().enumerate() {
                    let path = dir.join(format!("{at}.zst"));
                    page.held = Held::Built(std::fs::read(&path).wrap_err_with(|| format!("read {}", path.display()))?);
                }
            }
            layout
        }
        LayoutKind::Classes => {
            let mut groups = vec![Vec::new(); classes.count()];
            for (frame, info) in package.frames.iter().enumerate().filter(|(_, frame)| frame.placed()) {
                groups[class_of_body(info.owner_body)].push(frame);
            }
            Layout::built(&package, &groups)?
        }
    };
    eprintln!("layout: {} pages in {:.1} s", layout.pages.len(), start.elapsed().as_secs_f64());
    let (page_clips, clip_pages) = layout.clips(&package);

    // Every page through every method: encoded on the whole machine, timed on the game's worker count.
    let timing_pool = rayon::ThreadPoolBuilder::new().num_threads(GAME_DECODE_THREADS).build()?;
    let mut measured: Vec<Vec<PageCost>> = Vec::new();
    for &method in &methods {
        let start = Instant::now();
        let stored: Vec<StoredPage> = (0..layout.pages.len()).into_par_iter().map(|at| {
            encode_page(&layout.raw(&package, at)?, layout.pages[at].blocks_wide, method)
        }).collect::<eyre::Result<_>>()?;
        let encoded = start.elapsed();
        let costs: Vec<PageCost> = timing_pool.install(|| stored.par_iter().zip(&page_clips).map(|(stored, clips)| {
            let rects: Vec<Vec<BlockRect>> = clips.iter().map(|(_, rects)| rects.clone()).collect();
            time_page(stored, &rects, args.reps)
        }).collect::<eyre::Result<_>>())?;
        eprintln!("{}: encoded in {:.1} s, timed in {:.1} s", method.name(), encoded.as_secs_f64(),
            (start.elapsed() - encoded).as_secs_f64());
        measured.push(costs);
    }

    println!("layout {:?}: {} pages", args.layout, layout.pages.len());
    println!("\n| Class | Method | Pages | Raw MiB | Stored MiB | Raw / stored | Decode MB/s (4 workers busy) | Page ms \
        p50 / p95 | Inverse ms p50 |\n|---|---|---:|---:|---:|---:|---:|---:|---:|");
    for class in 0..classes.count() {
        for (at, method) in methods.iter().enumerate() {
            let costs: Vec<&PageCost> = measured[at].iter().zip(&layout.pages)
                .filter(|(_, page)| page.class == class).map(|(cost, _)| cost).collect();
            if costs.is_empty() {
                continue;
            }
            let raw: usize = costs.iter().map(|cost| cost.raw).sum();
            let bytes: usize = costs.iter().map(|cost| cost.bytes).sum();
            let decode: f64 = costs.iter().map(|cost| cost.decompress + cost.inverse).sum();
            let mut page_ms: Vec<f64> = costs.iter().map(|cost| cost.decompress + cost.inverse).collect();
            page_ms.sort_by(f64::total_cmp);
            let mut inverse: Vec<f64> = costs.iter().map(|cost| cost.inverse).collect();
            inverse.sort_by(f64::total_cmp);
            println!("| {} | {} | {} | {:.1} | {:.2} | {:.2} | {:.0} | {:.2} / {:.2} | {:.2} |", classes.name(class),
                method.name(), costs.len(), mib(raw as u64), mib(bytes as u64), raw as f64 / bytes as f64,
                raw as f64 / crate::BYTES_PER_MB / (decode / MS_PER_S), percentile(&page_ms, 50.0),
                percentile(&page_ms, 95.0), percentile(&inverse, 50.0));
        }
    }

    // The crowds, the same for every configuration.
    let by_frames: HashMap<(u32, u32), usize> = package.clips.iter().enumerate()
        .map(|(clip, info)| ((info.frame_start, info.frame_count), clip)).collect();
    for &mix in &args.mix {
        crowds_of(args, mix, &package, &sides, &by_frames, &layout, &clip_pages, &configs, &measured)?;
    }
    Ok(())
}

/// The crowds of `mix`, the same for every configuration, and each configuration's outcomes.
#[allow(clippy::too_many_arguments)]
fn crowds_of(
    args: &CrowdArgs,
    mix: Mix,
    package: &Package,
    sides: &BTreeMap<u32, u32>,
    by_frames: &HashMap<(u32, u32), usize>,
    layout: &Layout,
    clip_pages: &[Vec<(usize, usize)>],
    configs: &[Config],
    measured: &[Vec<PageCost>],
) -> eyre::Result<()> {
    let candidates: Vec<u32> = match mix {
        Mix::People => PEOPLE_BODIES.to_vec(),
        Mix::Random => sides.keys().copied().filter(|&body| {
            movement_action(package, body, MOVE_STAND)
                .and_then(|action| resolve_clip(package, by_frames, body, action, 0)).is_some()
        }).collect(),
    };
    eyre::ensure!(!candidates.is_empty(), "no body with a movement set");
    let mut rng = SplitMix(args.seed);
    let crowds: Vec<Crowd> = (0..args.crowds).map(|_| {
        let mobiles: Vec<(u32, u8)> = (0..args.mobiles).map(|_| {
            (candidates[rng.below(candidates.len())], rng.below(usize::from(DIRECTIONS)) as u8)
        }).collect();
        crowd(package, by_frames, &mobiles)
    }).collect();
    let faced_clips: Vec<f64> = crowds.iter().map(|crowd| {
        let faced: BTreeSet<usize> = crowd.faced.iter().flatten().copied().collect();
        faced.len() as f64
    }).collect();
    let all_clips: Vec<f64> = crowds.iter().map(|crowd| crowd.requests.len() as f64).collect();
    let median = |mut values: Vec<f64>| -> f64 {
        values.sort_by(f64::total_cmp);
        percentile(&values, 50.0)
    };
    println!("\n{} crowds of {} mobiles ({:?}, from {} bodies): {:.0} faced clips and {:.0} clips in all at the median",
        args.crowds, args.mobiles, mix, candidates.len(), median(faced_clips), median(all_clips));

    println!("\n| Config | Pages MiB | Cache (pages) | Page decodes | Decode ms (all workers) | First drawable, warm / \
        cold ms | Mobile p50 drawable, warm ms | All preloaded, warm / cold ms |\n|---|---:|---:|---:|---:|---:|---:|---:|");
    for config in configs {
        let costs = Costs {
            pages: layout.pages.iter().enumerate().map(|(at, page)| {
                let (method, per_clip) = config.per_class[page.class];
                (&measured[method][at], per_clip)
            }).collect(),
        };
        let bytes: usize = costs.pages.iter().map(|(cost, _)| cost.bytes).sum();
        for &lru in &args.lru {
            let warm: Vec<Outcome> = crowds.iter().map(|crowd| simulate(crowd, clip_pages, &costs, lru, false)).collect();
            let cold: Vec<Outcome> = crowds.iter().map(|crowd| simulate(crowd, clip_pages, &costs, lru, true)).collect();
            let of = |outcomes: &[Outcome], pick: fn(&Outcome) -> f64| median(outcomes.iter().map(pick).collect());
            println!("| {} | {:.1} | {lru} | {:.0} | {:.0} | {:.1} / {:.1} | {:.1} | {:.0} / {:.0} |", config.name,
                mib(bytes as u64), of(&warm, |outcome| outcome.decodes as f64), of(&warm, |outcome| outcome.busy_ms),
                of(&warm, |outcome| outcome.first_drawable_ms), of(&cold, |outcome| outcome.first_drawable_ms),
                of(&warm, |outcome| outcome.mobile_p50_ms), of(&warm, |outcome| outcome.all_ms),
                of(&cold, |outcome| outcome.all_ms));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_split_at_their_bounds() {
        let classes = Classes::new(vec![256, 96]);
        assert_eq!((classes.of(1), classes.of(96), classes.of(97), classes.of(256), classes.of(257)), (0, 0, 1, 1, 2));
        assert_eq!(classes.count(), 3);
        assert_eq!(classes.name(0), "≤ 96 px");
        assert_eq!(classes.name(1), "97–256 px");
        assert_eq!(classes.name(2), "> 256 px");
    }
}
