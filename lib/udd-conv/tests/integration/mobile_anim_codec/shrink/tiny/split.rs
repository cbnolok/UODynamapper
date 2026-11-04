//! Part `t9` of the `shrink` pass: frames under a fifth painted, and a frame's box cut at its best run of fully empty
//! rows or columns into up to two tight boxes. The atlas holds the two boxes, which is what could save VRAM.

use super::*;

/// A frame with fewer painted pixels than this share of its box counts as sparse.
const SPARSE_PERCENT: u64 = 20;
const WHOLE_PERCENT: u64 = 100;
/// A cut must save at least an 8 x 8 tile of atlas, or the frame keeps its one box.
const CUT_SHORTEST_SAVING: u64 = 64;
/// A pixel of atlas gutter on each side of a box.
const GUTTER_PIXELS: u32 = 2;
/// Enhanced bodies whose rows hold separate islands, from the requirements document.
const ISLAND_BODIES: [u32; 4] = [1055, 1056, 1057, 1058];
const CUT_ONE_BOX: u32 = 0;
const CUT_TWO_BOXES: u32 = 1;

/// A box inside a frame's cropped box.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Cut {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Cut {
    fn area(self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }

    fn gutter_area(self) -> u64 {
        if self.area() == 0 {
            0
        } else {
            u64::from(self.w + GUTTER_PIXELS) * u64::from(self.h + GUTTER_PIXELS)
        }
    }
}

/// A frame's boxes; the second is empty when the frame keeps one.
pub(super) type FrameCuts = (Cut, Cut);

fn tight(mask: &[u8], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Cut {
    let (mut left, mut top, mut right, mut bottom) = (x1, y1, x0, y0);
    for y in y0..y1 {
        for x in x0..x1 {
            if mask[y * w + x] != 0 {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if right <= left {
        return Cut::default();
    }
    Cut { x: left as u32, y: top as u32, w: (right - left) as u32, h: (bottom - top) as u32 }
}

/// Each run of empty lines with painted lines on both sides.
fn empty_runs(painted: &[bool]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut at = 0usize;
    while at < painted.len() {
        if painted[at] {
            at += 1;
            continue;
        }
        let stop = at + painted[at..].iter().take_while(|&&p| !p).count();
        if at > 0 && stop < painted.len() {
            runs.push((at, stop));
        }
        at = stop;
    }
    runs
}

fn cut_frame(mask: &[u8], w: usize, h: usize) -> FrameCuts {
    let whole = Cut { x: 0, y: 0, w: w as u32, h: h as u32 };
    let mut best = (whole, Cut::default());
    let mut best_area = whole.area().saturating_sub(CUT_SHORTEST_SAVING);
    let mut consider = |first: Cut, second: Cut| {
        if first.area() > 0 && second.area() > 0 && first.area() + second.area() <= best_area {
            best_area = first.area() + second.area();
            best = (first, second);
        }
    };
    let rows: Vec<bool> = (0..h).map(|y| mask[y * w..(y + 1) * w].iter().any(|&m| m != 0)).collect();
    let columns: Vec<bool> = (0..w).map(|x| (0..h).any(|y| mask[y * w + x] != 0)).collect();
    for (start, stop) in empty_runs(&rows) {
        consider(tight(mask, w, 0, 0, w, start), tight(mask, w, 0, stop, w, h));
    }
    for (start, stop) in empty_runs(&columns) {
        consider(tight(mask, w, 0, 0, start, h), tight(mask, w, stop, 0, w, h));
    }
    best
}

pub(super) fn frame_cuts(mask: &[u8], sizes: &[(usize, usize)]) -> Vec<FrameCuts> {
    let mut start = 0usize;
    sizes
        .iter()
        .map(|&(w, h)| {
            let cuts = cut_frame(&mask[start..start + w * h], w, h);
            start += w * h;
            cuts
        })
        .collect()
}

fn put_cut(out: &mut Vec<u8>, cut: Cut) {
    [cut.x, cut.y, cut.w, cut.h].into_iter().for_each(|value| put_varint(out, value));
}

fn cut_side(cuts: &[FrameCuts], out: &mut Vec<u8>) {
    for &(first, second) in cuts {
        if second.area() == 0 {
            put_varint(out, CUT_ONE_BOX);
        } else {
            put_varint(out, CUT_TWO_BOXES);
            put_cut(out, first);
            put_cut(out, second);
        }
    }
}

/// The boxes' geometry if wanted, then the pixels of every box, row by row.
pub(super) fn box_content(plane: &[u8], cuts: &[FrameCuts], sizes: &[(usize, usize)], with_side: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if with_side {
        cut_side(cuts, &mut out);
    }
    let mut start = 0usize;
    for (&(w, h), &(first, second)) in sizes.iter().zip(cuts) {
        for cut in [first, second] {
            for y in cut.y as usize..(cut.y + cut.h) as usize {
                let row = start + y * w + cut.x as usize;
                out.extend_from_slice(&plane[row..row + cut.w as usize]);
            }
        }
        start += w * h;
    }
    out
}

#[inline(always)]
fn get_cut(data: &[u8], at: &mut usize) -> Cut {
    let x = get_varint(data, at);
    let y = get_varint(data, at);
    let w = get_varint(data, at);
    let h = get_varint(data, at);
    Cut { x, y, w, h }
}

/// Rebuilds the boxes; the bytes their geometry took.
#[inline(always)]
pub(super) fn parse_cuts(data: &[u8], sizes: &[(usize, usize)], cuts: &mut Vec<FrameCuts>) -> usize {
    cuts.clear();
    let mut at = 0usize;
    for &(w, h) in sizes {
        if get_varint(data, &mut at) == CUT_ONE_BOX {
            cuts.push((Cut { x: 0, y: 0, w: w as u32, h: h as u32 }, Cut::default()));
        } else {
            let first = get_cut(data, &mut at);
            let second = get_cut(data, &mut at);
            cuts.push((first, second));
        }
    }
    at
}

#[inline(always)]
pub(super) fn expand_cuts(pixels: &[u8], cuts: &[FrameCuts], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut next, mut start) = (0usize, 0usize);
    for (&(w, h), &(first, second)) in sizes.iter().zip(cuts) {
        for cut in [first, second] {
            let length = cut.w as usize;
            for y in cut.y as usize..(cut.y + cut.h) as usize {
                let row = start + y * w + cut.x as usize;
                out[row..row + length].copy_from_slice(&pixels[next..next + length]);
                next += length;
            }
        }
        start += w * h;
    }
}

/// A stream's atlas area with one box per frame, and with the frames cut.
pub(super) fn stream_areas(mask: &[u8], sizes: &[(usize, usize)]) -> (u64, u64) {
    let whole = sizes.iter().map(|&(w, h)| (w * h) as u64).sum();
    (whole, frame_cuts(mask, sizes).iter().map(|&(first, second)| first.area() + second.area()).sum())
}

fn is_sparse(painted: u64, pixels: u64) -> bool {
    painted * WHOLE_PERCENT < SPARSE_PERCENT * pixels
}

/// The same streams with only the sparse frames, or only the others; no previous frame, since neighbours are gone.
fn frames_only(planes: &StreamPlanes, sparse: bool) -> StreamPlanes {
    let mut out = StreamPlanes { client: planes.client, layout: Vec::new(), original: Vec::new(), luma: Vec::new(), master: Vec::new(), mask: Vec::new(), rows: Vec::new() };
    for n in 0..planes.streams() {
        let (mut index, mut mask, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
        let mut start = 0usize;
        for &(w, h) in &planes.layout[n].sizes {
            let pixels = w * h;
            let frame = &planes.mask[n][start..start + pixels];
            if is_sparse(frame.iter().filter(|&&m| m != 0).count() as u64, pixels as u64) == sparse {
                index.extend_from_slice(&planes.original[n][start..start + pixels]);
                mask.extend_from_slice(frame);
                sizes.push((w, h));
            }
            start += pixels;
        }
        if sizes.is_empty() {
            continue;
        }
        out.rows.push(rows_of(&mask, &sizes));
        out.layout.push(StreamLayout { action: planes.layout[n].action, previous: vec![None; sizes.len()], pixels: index.len(), palettes: planes.layout[n].palettes.clone(), sizes });
        out.original.push(index);
        out.mask.push(mask);
        out.luma.push(Vec::new());
        out.master.push(Vec::new());
    }
    out
}

#[derive(Clone, Copy)]
struct Form {
    name: &'static str,
    index: IndexForm,
    coverage: CoverageForm,
}

const fn form(name: &'static str, index: IndexForm, coverage: CoverageForm) -> Form {
    Form { name, index, coverage }
}

const CC_FORMS: [Form; 8] = [
    form("zstd 19 rectangle", IndexForm::Plain, CoverageForm::Zstd),
    form("a row spans", SPANS_ZSTD, CoverageForm::Zstd),
    form("b painted", CC_PAINTED[1], CoverageForm::Zstd),
    form("split boxes", BOXES_SIDE_ZSTD, CoverageForm::Zstd),
    form("LZ4 HC rectangle", IndexForm::Lz4, CoverageForm::Zstd),
    form("a row spans", SPANS_LZ4, CoverageForm::Zstd),
    form("b painted", CC_PAINTED[3], CoverageForm::Zstd),
    form("split boxes", BOXES_SIDE_LZ4, CoverageForm::Zstd),
];

const EC_FORMS: [Form; 12] = [
    form("zstd 19 rectangle", IndexForm::Plain, CoverageForm::Zstd),
    form("a row spans", SPANS_ZSTD, CoverageForm::Zstd),
    form("b painted", PAINTED_EC_ZSTD, CoverageForm::Zstd),
    form("c both planes in row spans", SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD),
    form("b, coverage in row spans", PAINTED_EC_ZSTD, COVERAGE_SPANS_ZSTD),
    form("split boxes, both planes", BOXES_ALONE_ZSTD, COVERAGE_BOXES_ZSTD),
    form("LZ4 HC rectangle", IndexForm::Lz4, CoverageForm::Lz4),
    form("a row spans", SPANS_LZ4, CoverageForm::Lz4),
    form("b painted", PAINTED_EC_LZ4, CoverageForm::Lz4),
    form("c both planes in row spans", SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4),
    form("b, coverage in row spans", PAINTED_EC_LZ4, COVERAGE_SPANS_LZ4),
    form("split boxes, both planes", BOXES_ALONE_LZ4, COVERAGE_BOXES_LZ4),
];

#[derive(Default, Clone)]
struct FrameSet {
    frames: u64,
    pixels: u64,
    painted: u64,
    span_pixels: u64,
    cut_frames: u64,
    cut_pixels: u64,
    gutter_pixels: u64,
    cut_gutter_pixels: u64,
    geometry: u64,
    bytes: Vec<u64>,
}

impl FrameSet {
    fn add(&mut self, other: &FrameSet) {
        self.frames += other.frames;
        self.pixels += other.pixels;
        self.painted += other.painted;
        self.span_pixels += other.span_pixels;
        self.cut_frames += other.cut_frames;
        self.cut_pixels += other.cut_pixels;
        self.gutter_pixels += other.gutter_pixels;
        self.cut_gutter_pixels += other.cut_gutter_pixels;
        self.geometry += other.geometry;
        if self.bytes.len() < other.bytes.len() {
            self.bytes.resize(other.bytes.len(), 0);
        }
        self.bytes.iter_mut().zip(&other.bytes).for_each(|(sum, value)| *sum += value);
    }
}

fn frame_set(planes: &StreamPlanes, forms: &[Form]) -> FrameSet {
    let (mut atoms, none) = (Atoms::default(), Dictionaries::none());
    let mut set = FrameSet { bytes: forms.iter().map(|f| coded_of(&scheme("t9", f.index, f.coverage), planes, &none, &mut atoms).total()).collect(), ..FrameSet::default() };
    for n in 0..planes.streams() {
        let sizes = &planes.layout[n].sizes;
        let cuts = frame_cuts(&planes.mask[n], sizes);
        let mut side = Vec::new();
        cut_side(&cuts, &mut side);
        set.geometry += squeeze(false, &side).len() as u64;
        set.span_pixels += planes.rows[n].iter().map(|&(start, stop)| u64::from(stop - start)).sum::<u64>();
        set.painted += planes.mask[n].iter().filter(|&&m| m != 0).count() as u64;
        for (&(w, h), &(first, second)) in sizes.iter().zip(&cuts) {
            let whole = Cut { x: 0, y: 0, w: w as u32, h: h as u32 };
            set.frames += 1;
            set.pixels += whole.area();
            set.gutter_pixels += whole.gutter_area();
            set.cut_frames += u64::from(second.area() > 0);
            set.cut_pixels += first.area() + second.area();
            set.cut_gutter_pixels += first.gutter_area() + second.gutter_area();
        }
    }
    set
}

const SET_NAMES: [&str; 3] = ["under 20 % painted, coded apart", "the rest, coded apart", "all frames, one stream"];

struct BodySplit {
    group: Group,
    sets: [FrameSet; 3],
}

fn body_split(planes: &Planes, group: Group, forms: &[Form]) -> BodySplit {
    let whole = StreamPlanes::of(planes, None, None, Unit::ActionDirection);
    BodySplit { group, sets: [frame_set(&frames_only(&whole, true), forms), frame_set(&frames_only(&whole, false), forms), frame_set(&whole, forms)] }
}

fn print_sets(of: &[BodySplit], forms: &[Form], pixel_bytes: u64) {
    let names: Vec<&str> = forms.iter().map(|f| f.name).collect();
    println!("| kind | frames | count | % of frames | % of box area | % of painted | {} |", names.join(" | "));
    println!("|---|---|---:|---:|---:|---:|{}", "---:|".repeat(forms.len()));
    let mut atlas = String::new();
    let line = |kind: &str, group: Option<Group>, atlas: &mut String| {
        let mut sets: [FrameSet; 3] = Default::default();
        for body in of.iter().filter(|b| group.is_none_or(|g| b.group == g)) {
            sets.iter_mut().zip(&body.sets).for_each(|(sum, set)| sum.add(set));
        }
        let all = sets[2].clone();
        if all.frames == 0 {
            return;
        }
        for (name, set) in SET_NAMES.iter().zip(&sets) {
            let bytes: Vec<String> = set.bytes.iter().map(|&b| format!("{:.3}", mb(b))).collect();
            let share = |part: u64, whole: u64| part as f64 * PERCENT / whole.max(1) as f64;
            println!(
                "| {kind} | {name} | {} | {:.1} | {:.1} | {:.1} | {} |",
                set.frames,
                share(set.frames, all.frames),
                share(set.pixels, all.pixels),
                share(set.painted, all.painted),
                bytes.join(" | ")
            );
            let saved = set.pixels - set.cut_pixels;
            atlas.push_str(&format!(
                "| {kind} | {name} | {} | {:.1} | {:.3} | {:.3} | {:.1} | {:.3} | {:.1} | {:.3} | {:.1} | {:.3} |\n",
                set.cut_frames,
                share(set.cut_frames, set.frames),
                set.pixels as f64 / PIXELS_PER_MEGAPIXEL,
                set.cut_pixels as f64 / PIXELS_PER_MEGAPIXEL,
                share(saved, set.pixels),
                mb(saved * pixel_bytes),
                share(set.gutter_pixels - set.cut_gutter_pixels, set.gutter_pixels),
                mb((set.gutter_pixels - set.cut_gutter_pixels) * pixel_bytes),
                share(set.span_pixels, set.pixels),
                mb(set.geometry),
            ));
        }
    };
    for group in GROUPS {
        line(group.label(), Some(group), &mut atlas);
    }
    line("everything", None, &mut atlas);
    println!("\nAtlas area: one box per frame against up to two; no atlas packing waste, and pixel bytes of the client's texture format.\n");
    println!("| kind | frames | cut frames | % cut | box Mpx | after the cut Mpx | saved % | saved VRAM MB | saved % with a 1-pixel gutter | saved VRAM MB with it | row spans % of box | box geometry, zstd 19 MB |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    print!("{atlas}");
}

pub(in super::super) fn part_t9(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    println!("\n### t9: sparse frames and split boxes, every {stride}-th body by pixel count, per (action, direction); CPU {}; every coder round-trips", cpu_state());
    for client in [Client::Cc, Client::Ec] {
        let forms: &[Form] = match client {
            Client::Cc => &CC_FORMS,
            Client::Ec => &EC_FORMS,
        };
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let of: Vec<BodySplit> = sample.par_iter().map(|&i| body_split(&Planes::from_body(&load_body(sources, &bodies[i])), kinds.group(&bodies[i]), forms)).collect();
        println!("\n#### {}: {} bodies; each set is coded on its own, so the two sets lose the matches between them; coverage counted\n", client.label(), of.len());
        print_sets(&of, forms, client.pixel_bytes() as u64);
        if client == Client::Ec {
            let islands: Vec<(u32, usize)> = present
                .iter()
                .filter(|s| s.0 == client)
                .filter_map(|s| match &bodies[s.2] {
                    BodySource::Ec { body, .. } if ISLAND_BODIES.contains(body) => Some((*body, s.2)),
                    _ => None,
                })
                .collect();
            for (body, i) in islands {
                let row = body_split(&Planes::from_body(&load_body(sources, &bodies[i])), kinds.group(&bodies[i]), forms);
                println!("\n##### Enhanced body {body}\n");
                print_sets(std::slice::from_ref(&row), forms, client.pixel_bytes() as u64);
            }
        }
    }
}
