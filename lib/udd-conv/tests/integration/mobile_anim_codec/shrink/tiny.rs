//! Part `t` of the `shrink` pass: what tiny and large streams want, and how close a shape-first residual decode gets to
//! plain Zstd. Everything is per (action, direction), the recommended load unit. Measure only.
//!
//! - `t0`: the whole set, a body at a time: who is equipment and who a creature (`mobtypes.txt`; Enhanced ships none, so
//!   it uses Classic's table by id; `anim2+.mul` go through `Bodyconv.def`), and exact duplicate streams.
//! - `t1`: sizes on the sample, every coder decoded and compared with its source: the stream-size buckets, frame flags
//!   off, per-body dictionaries, the per-stream choice, table-driven coverage, row spans, painted pixels only and the
//!   closing rows.
//! - `tt`: single-thread decode of the same streams (needs a quiet machine).
//! - `tc`: these coders in the creature and dressed NPC crowds; `tu`: in the uniform and worst crowds (need a quiet
//!   machine). `MOBILE_ANIM_CODEC_TINY_ONLY` narrows `tt`, `tc` and `tu` to exact scheme names, separated by `;`.
//! - `ts`: the candidates of part `s1` and BC7 in the creature and dressed NPC crowds (needs a quiet machine).
//! - `t9`: frames under a fifth painted, and split boxes, see `split`.
//! - `tk0`: the `tileart.uop` placeholder rule for equipment against `mobtypes.txt`, per source, see `classes`.
//! - `tk`: every lossless coder by class of body, and which assignment keeps the crowds inside the line, see `classes`.
//! - `v`: what the GPU holds under each layout, and atlas waste under real allocators, see `vram`.
//! - `q`, `qc`, `qd`, `qm`: lossy block formats for low-end devices, see `lossy`.
//!
//! Every table is built from the body being coded: nothing here ships statistics, so custom art round-trips the same.

use super::crowd::{self, Job, Pair, Tans};
use super::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;
use zstd::bulk::{Compressor, Decompressor};
use zstd::dict::DecoderDictionary;

mod classes;
mod lossy;
mod split;
mod vram;
pub(super) use classes::{part_tk, part_tk0};
pub(super) use lossy::{part_q, part_qc, part_qd, part_qm};
pub(super) use vram::part_v;
pub(super) use split::part_t9;

const KIB: usize = 1024;
/// Stream buckets by pixels (one index byte each), as the census of the requirements document counts streams.
const BUCKET_TOPS: [u64; 4] = [4 * KIB as u64, 16 * KIB as u64, 64 * KIB as u64, 256 * KIB as u64];
const BUCKETS: usize = BUCKET_TOPS.len() + 1;
const BUCKET_NAMES: [&str; BUCKETS] = ["up to 4 KiB", "up to 16 KiB", "up to 64 KiB", "up to 256 KiB", "larger"];

const KIND_UNLISTED: &str = "not listed";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Group {
    Equipment,
    Creature,
    Unlisted,
}

const GROUPS: [Group; 3] = [Group::Equipment, Group::Creature, Group::Unlisted];

impl Group {
    fn of(kind: &str) -> Group {
        match kind {
            knowledge::KIND_EQUIPMENT => Group::Equipment,
            KIND_UNLISTED => Group::Unlisted,
            _ => Group::Creature,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Group::Equipment => "equipment",
            Group::Creature => "creatures",
            Group::Unlisted => "not listed",
        }
    }

    fn at(self) -> usize {
        GROUPS.iter().position(|&g| g == self).unwrap()
    }
}

/// A body's `mobtypes.txt` type. The ids of `anim2+.mul` are that file's own, so they go through `Bodyconv.def`.
struct BodyKinds {
    by_id: HashMap<u32, String>,
    conv: HashMap<(usize, u32), u32>,
}

impl BodyKinds {
    fn load(cc_dir: &Path) -> BodyKinds {
        let mut conv = HashMap::new();
        for (body, targets) in knowledge::bodyconv_rows(cc_dir) {
            for (file, target) in targets {
                conv.entry((usize::from(file), target)).or_insert(body);
            }
        }
        BodyKinds { by_id: body_kinds(cc_dir), conv }
    }

    /// The id the client knows the body by; none for an `anim2+.mul` body no `Bodyconv.def` line reaches.
    fn id(&self, source: &BodySource) -> Option<u32> {
        match source {
            BodySource::CcMul { file: 0, body, .. } => Some(u32::from(*body)),
            BodySource::CcMul { file, body, .. } => self.conv.get(&(*file, u32::from(*body))).copied(),
            BodySource::CcUop { body, .. } | BodySource::Ec { body, .. } => Some(*body),
        }
    }

    fn kind(&self, source: &BodySource) -> &str {
        self.id(source).and_then(|id| self.by_id.get(&id)).and_then(|kind| kind.split(':').next()).unwrap_or(KIND_UNLISTED)
    }

    fn group(&self, source: &BodySource) -> Group {
        Group::of(self.kind(source))
    }
}

fn bucket_of(pixels: u64) -> usize {
    BUCKET_TOPS.iter().position(|&top| pixels <= top).unwrap_or(BUCKETS - 1)
}

// ---------------------------------------------------------------------------------------------------------------
// The streams of a body

struct StreamLayout {
    action: u16,
    sizes: Vec<(usize, usize)>,
    previous: Vec<Option<PreviousFrame>>,
    pixels: usize,
    palettes: Vec<usize>,
}

/// A body's streams, in each palette order a coder can want. Empty planes for the orders not asked for.
struct StreamPlanes {
    client: Client,
    layout: Vec<StreamLayout>,
    original: Vec<Vec<u8>>,
    luma: Vec<Vec<u8>>,
    master: Vec<Vec<u8>>,
    mask: Vec<Vec<u8>>,
    rows: Vec<Vec<Row>>,
}

impl StreamPlanes {
    fn of(original: &Planes, luma: Option<&Planes>, master: Option<&Planes>, unit: Unit) -> StreamPlanes {
        let groups = units(&original.boxes, unit, original.client.pixel_bytes());
        let mut out = StreamPlanes { client: original.client, layout: Vec::new(), original: Vec::new(), luma: Vec::new(), master: Vec::new(), mask: Vec::new(), rows: Vec::new() };
        for frames in &groups {
            let (index, mask) = unit_planes(original, frames);
            let palettes: BTreeSet<usize> = frames.iter().map(|&f| original.boxes[f].palette).collect();
            out.layout.push(StreamLayout {
                action: original.boxes[frames[0]].action,
                sizes: frames.iter().map(|&f| (original.boxes[f].w, original.boxes[f].h)).collect(),
                previous: previous_any_palette(&original.boxes, frames),
                pixels: index.len(),
                palettes: palettes.into_iter().collect(),
            });
            out.rows.push(rows_of(&mask, &out.layout[out.layout.len() - 1].sizes));
            out.original.push(index);
            out.mask.push(mask);
            out.luma.push(luma.map(|p| unit_planes(p, frames).0).unwrap_or_default());
            out.master.push(master.map(|p| unit_planes(p, frames).0).unwrap_or_default());
        }
        out
    }

    fn streams(&self) -> usize {
        self.layout.len()
    }
}

/// The frame before each frame of a direction, whatever its palette: master order lines the palettes up.
fn previous_any_palette(boxes: &[FrameBox], frames: &[usize]) -> Vec<Option<PreviousFrame>> {
    let same: Vec<FrameBox> = frames.iter().map(|&f| FrameBox { palette: 0, ..boxes[f].clone() }).collect();
    previous_frames(&same, &(0..same.len()).collect::<Vec<usize>>())
}

/// The bytes a stream decodes to, as every MB/s of this benchmark counts them.
fn resident_of(client: Client, pixels: usize) -> usize {
    pixels * client.pixel_bytes()
}

// ---------------------------------------------------------------------------------------------------------------
// Coders

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum IndexForm {
    /// Zstd 19 on the original plane, a new context per stream, as every earlier row.
    PlainFresh,
    /// The same streams, one context per thread.
    Plain,
    Lz4,
    /// No content size, checksum or dictionary id, and the magic number not counted.
    Bare,
    /// A dictionary of up to this many bytes, trained on the body's own streams; flags as [`IndexForm::Bare`].
    Dict(usize),
    /// The best of no dictionary and [`DICT_SIZES`], per body.
    DictBest,
    /// Luma order, left residual, the per-pixel `predict` of the earlier rows.
    LeftPredict,
    /// The same streams, decoded span by span.
    Left,
    /// Luma order, up residual: a vector add under opaque pixels.
    Up,
    /// Whichever is smaller per stream, plain or left residual; `with_up` adds the up residual.
    Choice { with_up: bool },
    /// Scanline runs, values and lengths in tANS, tables per body.
    Rle,
    /// Master palette order, each frame XOR the previous one, Zstd 19.
    Xor,
    /// Only each row's first to last painted pixel, holes as slot 0, and the rows' starts and ends in the same frame
    /// unless the coverage plane already carries them.
    Spans { with_side: bool, lz4: bool },
    /// [`IndexForm::Spans`] with the rows in the frame, a dictionary trained on the body's own such streams.
    SpansDict(usize),
    /// The best of no dictionary and [`DICT_SIZES`] for those, per body.
    SpansDictBest,
    /// Only the painted pixels.
    Painted { shape: ShapeForm, lz4: bool },
    /// A frame's box cut at empty rows or columns into up to two tight boxes, each a rectangle; their geometry in the
    /// same frame unless the coverage plane already carries it.
    Boxes { with_side: bool, lz4: bool },
}

/// Where a stream of painted pixels finds its exact shape.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ShapeForm {
    /// Enhanced: which coverage values aren't zero.
    InCoverage,
    /// Each row's skips and lengths, before the pixels in one frame.
    RunsInFrame,
    /// The same in a frame of their own.
    RunsApart,
    /// Packed bits in a frame of their own.
    Bits,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum CoverageForm {
    ZstdFresh,
    Zstd,
    Lz4,
    /// Runs of transparent, solid and partial pixels and the partial values, in tANS with tables per body.
    Runs,
    /// With a fourth kind of run, a copy of the row above.
    RunsUp,
    /// [`CoverageForm::Runs`], Zstd 19 on the runs.
    RunsZstd,
    /// [`CoverageForm::RunsUp`], Zstd 19 on the runs.
    RunsUpZstd,
    /// Every pixel in tANS, one table per neighbour context, tables per body.
    Contexts,
    /// [`CoverageForm::RunsZstd`] with the partial values in tANS by neighbour context.
    RunsContexts,
    /// Only each row's first to last painted pixel, with the rows' starts and ends.
    Spans { lz4: bool },
    /// The frames' up to two boxes, with their geometry.
    Boxes { lz4: bool },
}

const ZSTD_MAGIC_BYTES: usize = 4;
const DICT_SIZES: [usize; 5] = [KIB, 4 * KIB, 16 * KIB, 64 * KIB, 256 * KIB];
/// Fewer sample bytes than this many dictionaries' worth train nothing useful.
const DICT_MIN_SAMPLE_RATIO: usize = 4;
const DICT_CHOICE_BYTES: u64 = 1;
const STREAM_CHOICE_BYTES: u64 = 1;
const LANES: usize = 16;
const RLE_RUN_MORE: u8 = 255;

/// A coverage run token: the kind in the top two bits, the length less one in the others.
const RUN_KIND_SHIFT: u32 = 6;
const RUN_LONGEST: usize = 1 << RUN_KIND_SHIFT;
const RUN_LENGTH_MASK: u8 = (RUN_LONGEST - 1) as u8;
const RUN_TRANSPARENT: u8 = 0;
const RUN_SOLID: u8 = 1;
const RUN_PARTIAL: u8 = 2;
const RUN_COPY_UP: u8 = 3;
const COPY_UP_SHORTEST: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Coding {
    #[default]
    ZstdFresh,
    Zstd,
    Lz4,
    ZstdDict,
    LeftPredict,
    Left,
    Up,
    Rle,
    Xor,
    Spans { with_side: bool, lz4: bool, dict: bool },
    Painted { shape: ShapeForm, lz4: bool },
    Boxes { with_side: bool, lz4: bool },
}

/// Shared, because several schemes store the same bytes and differ only in how they decode them.
type Bytes = Arc<Vec<u8>>;

#[derive(Default, Clone)]
struct IndexStream {
    coding: Coding,
    data: Bytes,
    /// Classic's shape bits for the residual codings; Rle's lengths.
    extra: Bytes,
    /// The residuals, or Rle's values.
    symbols: usize,
    /// Rle's length bytes.
    extra_symbols: usize,
    not_counted: usize,
    /// Which side stream a [`Coding::Spans`] picked.
    side_mode: u8,
}

impl IndexStream {
    fn stored(&self) -> u64 {
        (self.data.len() + self.extra.len() - self.not_counted) as u64
    }
}

#[derive(Default, Clone)]
struct IndexBody {
    streams: Vec<IndexStream>,
    /// tANS frequencies.
    tables: Vec<u8>,
    dictionary: Vec<u8>,
    /// What a per-stream or per-body choice adds.
    choice_bytes: u64,
}

impl IndexBody {
    fn shared(&self) -> u64 {
        (self.tables.len() + self.dictionary.len()) as u64 + self.choice_bytes
    }
}

#[derive(Default, Clone)]
struct CoverageStream {
    runs: Bytes,
    partial: Bytes,
    run_count: usize,
    partial_count: usize,
}

impl CoverageStream {
    fn stored(&self) -> u64 {
        (self.runs.len() + self.partial.len()) as u64
    }
}

#[derive(Default, Clone)]
struct CoverageBody {
    streams: Vec<CoverageStream>,
    tables: Vec<u8>,
}

fn compress_bare(compressor: &mut Compressor, data: &[u8]) -> Vec<u8> {
    compressor.compress(data).expect("zstd compress")
}

fn bare_compressor(dictionary: &[u8]) -> Compressor<'static> {
    let mut compressor = Compressor::with_dictionary(LEVEL_19, dictionary).expect("zstd compressor");
    for parameter in [CParameter::ContentSizeFlag(false), CParameter::ChecksumFlag(false), CParameter::DictIdFlag(false)] {
        compressor.set_parameter(parameter).expect("zstd flags");
    }
    compressor
}

fn train_dictionary(samples: &[&[u8]], size: usize) -> Vec<u8> {
    let total: usize = samples.iter().map(|s| s.len()).sum();
    if total < size * DICT_MIN_SAMPLE_RATIO {
        return Vec::new();
    }
    zstd::dict::from_samples(samples, size).unwrap_or_default()
}

/// Each body's own dictionaries: on its plain streams, and on its streams in the row span layout.
struct Dictionaries {
    plain: Vec<Vec<u8>>,
    spans: Vec<Vec<u8>>,
    /// What the body's whole set of streams picked, when only some of them are coded: a crowd's few streams would seldom
    /// repay a dictionary, and the format would choose over the whole body.
    picks: Option<WholePicks>,
}

/// The dictionary size a whole body's streams pick, or none; a form that isn't asked for stays none.
#[derive(Clone, Copy)]
struct WholePicks {
    plain: Option<usize>,
    spans: Option<usize>,
}

impl Dictionaries {
    fn none() -> Dictionaries {
        Dictionaries { plain: vec![Vec::new(); DICT_SIZES.len()], spans: vec![Vec::new(); DICT_SIZES.len()], picks: None }
    }

    fn at(size: usize) -> usize {
        DICT_SIZES.iter().position(|&s| s == size).expect("a dictionary size")
    }
}

fn dictionaries_of(planes: &StreamPlanes) -> Dictionaries {
    let train = |samples: &[&[u8]]| DICT_SIZES.iter().map(|&size| train_dictionary(samples, size)).collect();
    let plain: Vec<&[u8]> = planes.original.iter().map(Vec::as_slice).collect();
    let contents: Vec<Vec<u8>> = (0..planes.streams()).map(|n| span_content(&planes.original[n], &planes.rows[n], &planes.layout[n], true, false)).collect();
    let spans: Vec<&[u8]> = contents.iter().map(Vec::as_slice).collect();
    Dictionaries { plain: train(&plain), spans: train(&spans), picks: None }
}

// ---------------------------------------------------------------------------------------------------------------
// Row spans

/// A row's first painted pixel and the one after its last; (0, 0) when it has none.
type Row = (u32, u32);

const SIDE_RAW: u8 = 0;
const SIDE_PREVIOUS_ROW: u8 = 1;
const SIDE_PREVIOUS_FRAME: u8 = 2;
const SIDE_MODES: [u8; 3] = [SIDE_RAW, SIDE_PREVIOUS_ROW, SIDE_PREVIOUS_FRAME];
const SIDE_MODE_NAMES: [&str; 3] = ["raw", "against the previous row", "against the same row of the previous frame"];

fn rows_of(mask: &[u8], sizes: &[(usize, usize)]) -> Vec<Row> {
    let mut rows = Vec::with_capacity(sizes.iter().map(|size| size.1).sum());
    let mut start = 0usize;
    for &(w, h) in sizes {
        for y in 0..h {
            let row = &mask[start + y * w..start + (y + 1) * w];
            rows.push(match row.iter().position(|&m| m != 0) {
                Some(first) => (first as u32, (w - row.iter().rev().position(|&m| m != 0).expect("a painted pixel")) as u32),
                None => (0, 0),
            });
        }
        start += w * h;
    }
    rows
}

fn zigzag(value: i32) -> u32 {
    ((value << 1) ^ (value >> (i32::BITS - 1))) as u32
}

fn unzigzag(value: u32) -> i32 {
    (value >> 1) as i32 ^ -((value & 1) as i32)
}

/// What a delta side stream expects of a row. Only rows already known are read, so coding and decoding agree.
#[inline(always)]
fn predicted_row(mode: u8, rows: &[Row], bases: (usize, usize), previous: Option<PreviousFrame>, y: usize) -> Row {
    let (base, previous_base) = bases;
    if mode == SIDE_PREVIOUS_FRAME {
        if let Some(previous) = previous {
            let above = y as i32 - previous.dy;
            if above >= 0 && (above as usize) < previous.h {
                let row = rows[previous_base + above as usize];
                if row.1 > row.0 {
                    let moved = |x: u32| (x as i32 + previous.dx).max(0) as u32;
                    return (moved(row.0), moved(row.1));
                }
            }
        }
    }
    if y > 0 {
        rows[base + y - 1]
    } else {
        (0, 0)
    }
}

fn side_bytes(mode: u8, rows: &[Row], layout: &StreamLayout) -> Vec<u8> {
    let mut out = vec![mode];
    let (mut base, mut previous_base) = (0usize, 0usize);
    for (frame, &(_, h)) in layout.sizes.iter().enumerate() {
        for y in 0..h {
            let row = rows[base + y];
            if mode == SIDE_RAW {
                put_varint(&mut out, row.0);
                put_varint(&mut out, row.1 - row.0);
            } else {
                let guess = predicted_row(mode, rows, (base, previous_base), layout.previous[frame], y);
                put_varint(&mut out, zigzag(row.0 as i32 - guess.0 as i32));
                put_varint(&mut out, zigzag(row.1 as i32 - guess.1 as i32));
            }
        }
        previous_base = base;
        base += h;
    }
    out
}

/// Rebuilds the rows; the bytes the side stream took.
#[inline(always)]
fn parse_side(data: &[u8], layout: &StreamLayout, rows: &mut Vec<Row>) -> usize {
    rows.clear();
    let mode = data[0];
    let mut at = 1usize;
    let (mut base, mut previous_base) = (0usize, 0usize);
    for (frame, &(_, h)) in layout.sizes.iter().enumerate() {
        for y in 0..h {
            if mode == SIDE_RAW {
                let start = get_varint(data, &mut at);
                rows.push((start, start + get_varint(data, &mut at)));
            } else {
                let guess = predicted_row(mode, rows, (base, previous_base), layout.previous[frame], y);
                let start = (guess.0 as i32 + unzigzag(get_varint(data, &mut at))) as u32;
                let stop = (guess.1 as i32 + unzigzag(get_varint(data, &mut at))) as u32;
                rows.push((start, stop));
            }
        }
        previous_base = base;
        base += h;
    }
    at
}

fn squeeze(lz4: bool, bytes: &[u8]) -> Bytes {
    match (bytes.is_empty(), lz4) {
        (true, _) => Bytes::default(),
        (false, true) => crowd::lz4_compress(bytes).into(),
        (false, false) => compress_level(bytes, LEVEL_19, false).into(),
    }
}

fn best_side(rows: &[Row], layout: &StreamLayout, lz4: bool) -> Vec<u8> {
    SIDE_MODES.iter().map(|&mode| side_bytes(mode, rows, layout)).min_by_key(|side| squeeze(lz4, side).len()).expect("a side stream")
}

/// The best side stream if wanted, then each row's first to last painted pixel.
fn span_content(plane: &[u8], rows: &[Row], layout: &StreamLayout, with_side: bool, lz4: bool) -> Vec<u8> {
    let mut content = if with_side { best_side(rows, layout, lz4) } else { Vec::new() };
    let (mut row, mut start) = (0usize, 0usize);
    for &(w, h) in &layout.sizes {
        for y in 0..h {
            let (first, stop) = rows[row];
            row += 1;
            content.extend_from_slice(&plane[start + y * w + first as usize..start + y * w + stop as usize]);
        }
        start += w * h;
    }
    content
}

#[inline(always)]
fn expand_rows(pixels: &[u8], rows: &[Row], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut next, mut row, mut start) = (0usize, 0usize, 0usize);
    for &(w, h) in sizes {
        for y in 0..h {
            let (first, stop) = rows[row];
            row += 1;
            let length = (stop - first) as usize;
            let at = start + y * w + first as usize;
            out[at..at + length].copy_from_slice(&pixels[next..next + length]);
            next += length;
        }
        start += w * h;
    }
}

/// Each row's skips and lengths; a row ends when they add up to its width.
fn shape_runs(mask: &[u8], sizes: &[(usize, usize)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for &(w, h) in sizes {
        for y in 0..h {
            let row = &mask[start + y * w..start + (y + 1) * w];
            let mut x = 0usize;
            while x < w {
                let skip = row[x..].iter().take_while(|&&m| m == 0).count();
                put_varint(&mut out, skip as u32);
                x += skip;
                if x == w {
                    break;
                }
                let length = row[x..].iter().take_while(|&&m| m != 0).count();
                put_varint(&mut out, length as u32);
                x += length;
            }
        }
        start += w * h;
    }
    out
}

fn painted_stream(planes: &StreamPlanes, n: usize, shape: ShapeForm, lz4: bool) -> IndexStream {
    let mask = &planes.mask[n];
    let pixels: Vec<u8> = planes.original[n].iter().zip(mask).filter(|pair| *pair.1 != 0).map(|pair| *pair.0).collect();
    let sizes = &planes.layout[n].sizes;
    let mut stream = IndexStream { coding: Coding::Painted { shape, lz4 }, ..IndexStream::default() };
    match shape {
        ShapeForm::InCoverage => {
            stream.symbols = pixels.len();
            stream.data = squeeze(lz4, &pixels);
        }
        ShapeForm::RunsInFrame => {
            let runs = shape_runs(mask, sizes);
            let mut content = Vec::with_capacity(LENGTH_PREFIX_BYTES + runs.len() + pixels.len());
            put_varint(&mut content, runs.len() as u32);
            content.extend_from_slice(&runs);
            content.extend_from_slice(&pixels);
            stream.symbols = content.len();
            stream.data = squeeze(lz4, &content);
        }
        ShapeForm::RunsApart | ShapeForm::Bits => {
            let side = if shape == ShapeForm::Bits { packed_bits(mask.iter().map(|&m| m != 0)) } else { shape_runs(mask, sizes) };
            stream.symbols = pixels.len();
            stream.data = squeeze(lz4, &pixels);
            stream.extra_symbols = side.len();
            stream.extra = squeeze(lz4, &side);
        }
    }
    stream
}

#[inline(always)]
fn expand_painted<S: Shape>(shape: &S, pixels: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut next, mut start) = (0usize, 0usize);
    for &(w, h) in sizes {
        for y in 0..h {
            let row = start + y * w;
            let end = row + w;
            let mut x = shape.next_opaque(row, end);
            while x < end {
                let stop = shape.next_clear(x, end);
                let length = stop - x;
                out[x..stop].copy_from_slice(&pixels[next..next + length]);
                next += length;
                x = shape.next_opaque(stop, end);
            }
        }
        start += w * h;
    }
}

#[inline(always)]
fn expand_by_runs(runs: &[u8], pixels: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut at, mut next, mut start) = (0usize, 0usize, 0usize);
    for &(w, h) in sizes {
        for y in 0..h {
            let row = start + y * w;
            let mut x = 0usize;
            while x < w {
                x += get_varint(runs, &mut at) as usize;
                if x >= w {
                    break;
                }
                let length = get_varint(runs, &mut at) as usize;
                out[row + x..row + x + length].copy_from_slice(&pixels[next..next + length]);
                next += length;
                x += length;
            }
        }
        start += w * h;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// tANS by neighbour context, with a table small enough to build for each body

const CONTEXT_TANS_BITS: u32 = 10;
const CONTEXT_TANS_SIZE: usize = 1 << CONTEXT_TANS_BITS;
const CONTEXT_STATE_BYTES: usize = 2;
const CONTEXT_REFILL_BELOW: u32 = 16;
const CONTEXT_BUFFER_FULL_ABOVE: u32 = 56;

#[derive(Clone, Copy, Default)]
struct ContextEntry {
    next: u16,
    symbol: u8,
    bits: u8,
}

/// `COVERAGE_CONTEXTS` tables with one state between them, so the decoder may switch table at every pixel.
struct ContextCoder {
    decode: Vec<ContextEntry>,
    encode: Vec<u16>,
    freqs: Vec<[u16; COVERAGE_SYMBOLS]>,
    cums: Vec<[u16; COVERAGE_SYMBOLS]>,
}

fn scaled_freqs(hist: &[u32; COVERAGE_SYMBOLS]) -> [u16; COVERAGE_SYMBOLS] {
    let total: u64 = hist.iter().map(|&h| u64::from(h)).sum();
    let mut freqs = [0u16; COVERAGE_SYMBOLS];
    if total == 0 {
        return freqs;
    }
    for (freq, &h) in freqs.iter_mut().zip(hist) {
        if h > 0 {
            *freq = (u64::from(h) * CONTEXT_TANS_SIZE as u64 / total).max(1) as u16;
        }
    }
    let mut sum: usize = freqs.iter().map(|&f| usize::from(f)).sum();
    while sum != CONTEXT_TANS_SIZE {
        let largest = (0..COVERAGE_SYMBOLS).max_by_key(|&s| freqs[s]).expect("a symbol");
        if sum < CONTEXT_TANS_SIZE {
            freqs[largest] += (CONTEXT_TANS_SIZE - sum) as u16;
            sum = CONTEXT_TANS_SIZE;
        } else {
            let taken = (sum - CONTEXT_TANS_SIZE).min(usize::from(freqs[largest]) - 1);
            freqs[largest] -= taken as u16;
            sum -= taken;
        }
    }
    freqs
}

impl ContextCoder {
    fn from_freqs(freqs: Vec<[u16; COVERAGE_SYMBOLS]>, for_encoding: bool) -> ContextCoder {
        let step = (CONTEXT_TANS_SIZE >> 1) + (CONTEXT_TANS_SIZE >> 3) + 3;
        let mut decode = vec![ContextEntry::default(); freqs.len() * CONTEXT_TANS_SIZE];
        let mut encode = if for_encoding { vec![0u16; freqs.len() * CONTEXT_TANS_SIZE] } else { Vec::new() };
        let mut cums = vec![[0u16; COVERAGE_SYMBOLS]; freqs.len()];
        for (context, table) in freqs.iter().enumerate() {
            if table.iter().all(|&f| f == 0) {
                continue;
            }
            let mut spread = [0u8; CONTEXT_TANS_SIZE];
            let mut next_count = [0u32; COVERAGE_SYMBOLS];
            let (mut position, mut cum) = (0usize, 0u16);
            for (s, &f) in table.iter().enumerate() {
                cums[context][s] = cum;
                cum += f;
                next_count[s] = u32::from(f);
                for _ in 0..f {
                    spread[position] = s as u8;
                    position = (position + step) & (CONTEXT_TANS_SIZE - 1);
                }
            }
            let mut seen = [0u16; COVERAGE_SYMBOLS];
            for (u, &symbol) in spread.iter().enumerate() {
                let s = usize::from(symbol);
                let x = next_count[s];
                next_count[s] += 1;
                let bits = CONTEXT_TANS_BITS - (u32::BITS - 1 - x.leading_zeros());
                decode[context * CONTEXT_TANS_SIZE + u] = ContextEntry { next: ((x << bits) - CONTEXT_TANS_SIZE as u32) as u16, symbol, bits: bits as u8 };
                if for_encoding {
                    encode[context * CONTEXT_TANS_SIZE + usize::from(cums[context][s] + seen[s])] = (CONTEXT_TANS_SIZE + u) as u16;
                }
                seen[s] += 1;
            }
        }
        ContextCoder { decode, encode, freqs, cums }
    }

    /// The tables from a body's (context, symbol) pairs, and their stored form.
    fn from_sequences(sequences: &[Vec<(u8, u8)>]) -> (Vec<u8>, ContextCoder) {
        let mut hists = vec![[0u32; COVERAGE_SYMBOLS]; COVERAGE_CONTEXTS];
        sequences.iter().flatten().for_each(|&(context, symbol)| hists[usize::from(context)][usize::from(symbol)] += 1);
        let freqs: Vec<[u16; COVERAGE_SYMBOLS]> = hists.iter().map(scaled_freqs).collect();
        let mut raw = Vec::new();
        freqs.iter().flatten().for_each(|&f| put_varint(&mut raw, u32::from(f)));
        (compress_level(&raw, LEVEL_19, false), ContextCoder::from_freqs(freqs, true))
    }

    fn parse(bytes: &[u8]) -> ContextCoder {
        let raw = decompress(bytes, COVERAGE_CONTEXTS * COVERAGE_SYMBOLS * LENGTH_PREFIX_BYTES);
        let mut at = 0usize;
        let freqs = (0..COVERAGE_CONTEXTS)
            .map(|_| {
                let mut table = [0u16; COVERAGE_SYMBOLS];
                table.iter_mut().for_each(|f| *f = get_varint(&raw, &mut at) as u16);
                table
            })
            .collect();
        ContextCoder::from_freqs(freqs, false)
    }

    fn encode(&self, sequence: &[(u8, u8)]) -> Vec<u8> {
        if sequence.is_empty() {
            return Vec::new();
        }
        let mut state = CONTEXT_TANS_SIZE as u32;
        let mut pushed = Vec::with_capacity(sequence.len());
        for &(context, symbol) in sequence.iter().rev() {
            let (context, s) = (usize::from(context), usize::from(symbol));
            let freq = u32::from(self.freqs[context][s]);
            let mut bits = CONTEXT_TANS_BITS - (u32::BITS - 1 - freq.leading_zeros());
            if (state >> bits) < freq {
                bits -= 1;
            }
            pushed.push(((state & ((1 << bits) - 1)) as u16, bits as u8));
            state = u32::from(self.encode[context * CONTEXT_TANS_SIZE + usize::from(self.cums[context][s]) + (state >> bits) as usize - freq as usize]);
        }
        let mut out = ((state as usize - CONTEXT_TANS_SIZE) as u16).to_le_bytes().to_vec();
        let (mut buffer, mut have) = (0u64, 0u32);
        for &(value, bits) in pushed.iter().rev() {
            buffer |= u64::from(value) << have;
            have += u32::from(bits);
            while have >= u8::BITS {
                out.push(buffer as u8);
                buffer >>= u8::BITS;
                have -= u8::BITS;
            }
        }
        if have > 0 {
            out.push(buffer as u8);
        }
        out
    }
}

struct ContextReader<'a> {
    data: &'a [u8],
    at: usize,
    buffer: u64,
    have: u32,
    state: usize,
}

impl<'a> ContextReader<'a> {
    fn new(data: &'a [u8]) -> ContextReader<'a> {
        let state = if data.len() >= CONTEXT_STATE_BYTES { usize::from(u16::from_le_bytes([data[0], data[1]])) } else { 0 };
        ContextReader { data, at: CONTEXT_STATE_BYTES, buffer: 0, have: 0, state }
    }

    #[inline(always)]
    fn next(&mut self, table: &[ContextEntry], context: usize) -> u8 {
        if self.have < CONTEXT_REFILL_BELOW {
            while self.have <= CONTEXT_BUFFER_FULL_ABOVE && self.at < self.data.len() {
                self.buffer |= u64::from(self.data[self.at]) << self.have;
                self.at += 1;
                self.have += u8::BITS;
            }
        }
        let entry = table[context * CONTEXT_TANS_SIZE + self.state];
        let bits = u32::from(entry.bits);
        self.state = usize::from(entry.next) + (self.buffer & ((1u64 << bits) - 1)) as usize;
        self.buffer >>= bits;
        self.have = self.have.saturating_sub(bits);
        entry.symbol
    }
}

fn context_pairs(coverage: &[u8], sizes: &[(usize, usize)], partial_only: bool) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for &(w, h) in sizes {
        for y in 0..h {
            for x in 0..w {
                let i = start + y * w + x;
                if partial_only && (coverage[i] == COVERAGE_TRANSPARENT || coverage[i] == COVERAGE_SOLID) {
                    continue;
                }
                out.push((coverage_context(coverage, w, i, x, y) as u8, coverage[i]));
            }
        }
        start += w * h;
    }
    out
}

fn decode_contexts(coder: &ContextCoder, data: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let mut reader = ContextReader::new(data);
    let mut start = 0usize;
    for &(w, h) in sizes {
        for y in 0..h {
            for x in 0..w {
                let i = start + y * w + x;
                out[i] = reader.next(&coder.decode, coverage_context(out, w, i, x, y));
            }
        }
        start += w * h;
    }
}

fn decode_runs_contexts(tokens: &[u8], coder: &ContextCoder, data: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let mut reader = ContextReader::new(data);
    let (mut token, mut start) = (0usize, 0usize);
    for &(w, h) in sizes {
        let end = start + w * h;
        let mut at = start;
        while at < end {
            let byte = tokens[token];
            token += 1;
            let length = usize::from(byte & RUN_LENGTH_MASK) + 1;
            match byte >> RUN_KIND_SHIFT {
                RUN_TRANSPARENT => {}
                RUN_SOLID => out[at..at + length].fill(COVERAGE_SOLID),
                _ => {
                    let (mut x, mut y) = ((at - start) % w, (at - start) / w);
                    for i in at..at + length {
                        out[i] = reader.next(&coder.decode, coverage_context(out, w, i, x, y));
                        x += 1;
                        if x == w {
                            x = 0;
                            y += 1;
                        }
                    }
                }
            }
            at += length;
        }
        start = end;
    }
}

fn residuals_of(sizes: &[(usize, usize)], index: &[u8], mask: &[u8], predictor: Predictor) -> Vec<u8> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for &(w, h) in sizes {
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            for x in 0..w {
                let i = start + y * w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (prediction, _) = predict(index, mask, w, i, x, y > 0, predictor, last);
                out.push(index[i].wrapping_sub(prediction));
                last = index[i];
            }
        }
        start += w * h;
    }
    out
}

fn residual_stream(coding: Coding, predictor: Predictor, client: Client, layout: &StreamLayout, luma: &[u8], mask: &[u8]) -> IndexStream {
    let residuals = residuals_of(&layout.sizes, luma, mask, predictor);
    let extra = match client {
        Client::Cc => compress_level(&packed_bits(mask.iter().map(|&m| m != 0)), LEVEL_19, false),
        Client::Ec => Vec::new(),
    };
    IndexStream { coding, symbols: residuals.len(), data: compress_level(&residuals, LEVEL_19, false).into(), extra: extra.into(), ..IndexStream::default() }
}

fn plain_stream(coding: Coding, index: &[u8]) -> IndexStream {
    IndexStream { coding, data: compress_level(index, LEVEL_19, false).into(), ..IndexStream::default() }
}

/// Bytes of each symbol's serialized frequency stay out of the fast path: varints in Zstd, as part `s3` stores them.
fn tans_tables(hists: &[Vec<u32>]) -> (Vec<u8>, Vec<Tans>) {
    let mut raw = Vec::new();
    let tables = hists
        .iter()
        .map(|hist| {
            let freqs = normalized(hist);
            freqs.iter().for_each(|&f| put_varint(&mut raw, u32::from(f)));
            Tans::from_freqs(freqs, true)
        })
        .collect();
    (compress_level(&raw, LEVEL_19, false), tables)
}

fn parse_tans(bytes: &[u8], count: usize) -> Vec<Tans> {
    let raw = decompress(bytes, count * BYTE_SYMBOLS * LENGTH_PREFIX_BYTES);
    let mut at = 0usize;
    (0..count)
        .map(|_| {
            let freqs: Vec<u16> = (0..BYTE_SYMBOLS).map(|_| get_varint(&raw, &mut at) as u16).collect();
            Tans::from_freqs(freqs, false)
        })
        .collect()
}

fn histogram_of(symbols: &[u8], hist: &mut [u32]) {
    symbols.iter().for_each(|&s| hist[usize::from(s)] += 1);
}

fn rle_symbols(plane: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut values, mut runs) = (Vec::new(), Vec::new());
    let mut at = 0usize;
    while at < plane.len() {
        let value = plane[at];
        let length = plane[at..].iter().take_while(|&&v| v == value).count();
        values.push(value);
        let mut left = length - 1;
        while left >= usize::from(RLE_RUN_MORE) {
            runs.push(RLE_RUN_MORE);
            left -= usize::from(RLE_RUN_MORE);
        }
        runs.push(left as u8);
        at += length;
    }
    (values, runs)
}

/// Each frame of the plane XOR the frame before it, over the rectangle they share. Frames last to first when coding, and
/// first to last when decoding: the frame before must be the original either way.
fn xor_previous(plane: &mut [u8], layout: &StreamLayout, decoding: bool) {
    let mut starts = Vec::with_capacity(layout.sizes.len());
    let mut start = 0usize;
    for &(w, h) in &layout.sizes {
        starts.push(start);
        start += w * h;
    }
    let one = |plane: &mut [u8], n: usize| {
        let Some(previous) = layout.previous[n] else { return };
        let (w, h) = layout.sizes[n];
        let (before, frame) = plane.split_at_mut(starts[n]);
        let (x0, x1) = (previous.dx.max(0), (previous.w as i32 + previous.dx).min(w as i32));
        let (y0, y1) = (previous.dy.max(0), (previous.h as i32 + previous.dy).min(h as i32));
        if x0 >= x1 {
            return;
        }
        for y in y0..y1 {
            let from = previous.start + (y - previous.dy) as usize * previous.w + (x0 - previous.dx) as usize;
            let to = y as usize * w + x0 as usize;
            let length = (x1 - x0) as usize;
            frame[to..to + length].iter_mut().zip(&before[from..from + length]).for_each(|(value, other)| *value ^= other);
        }
    };
    if decoding {
        (1..layout.sizes.len()).for_each(|n| one(plane, n));
    } else {
        (1..layout.sizes.len()).rev().for_each(|n| one(plane, n));
    }
}

fn encode_index(form: IndexForm, planes: &StreamPlanes, dictionaries: &Dictionaries) -> IndexBody {
    let client = planes.client;
    let count = planes.streams();
    let residuals = |coding: Coding, predictor: Predictor| -> Vec<IndexStream> { (0..count).map(|n| residual_stream(coding, predictor, client, &planes.layout[n], &planes.luma[n], &planes.mask[n])).collect() };
    let mut body = IndexBody::default();
    match form {
        IndexForm::Plain => body.streams = planes.original.iter().map(|p| plain_stream(Coding::Zstd, p)).collect(),
        IndexForm::Lz4 => body.streams = planes.original.iter().map(|p| IndexStream { coding: Coding::Lz4, data: crowd::lz4_compress(p).into(), ..IndexStream::default() }).collect(),
        IndexForm::Bare => {
            let mut compressor = bare_compressor(&[]);
            body.streams = planes
                .original
                .iter()
                .map(|p| IndexStream { coding: Coding::Zstd, data: compress_bare(&mut compressor, p).into(), not_counted: ZSTD_MAGIC_BYTES, ..IndexStream::default() })
                .collect();
        }
        IndexForm::Dict(size) => {
            let dictionary = dictionaries.plain[Dictionaries::at(size)].clone();
            let mut compressor = bare_compressor(&dictionary);
            let coding = if dictionary.is_empty() { Coding::Zstd } else { Coding::ZstdDict };
            body.streams = planes
                .original
                .iter()
                .map(|p| IndexStream { coding, data: compress_bare(&mut compressor, p).into(), not_counted: ZSTD_MAGIC_BYTES, ..IndexStream::default() })
                .collect();
            body.dictionary = dictionary;
        }
        IndexForm::Left => body.streams = residuals(Coding::Left, Predictor::Left),
        IndexForm::Up => body.streams = residuals(Coding::Up, Predictor::Up),
        IndexForm::Rle => {
            let symbols: Vec<(Vec<u8>, Vec<u8>)> = planes.original.iter().map(|p| rle_symbols(p)).collect();
            let mut hists = vec![vec![0u32; BYTE_SYMBOLS]; INDEX_TABLES];
            for (values, runs) in &symbols {
                histogram_of(values, &mut hists[0]);
                histogram_of(runs, &mut hists[1]);
            }
            let (tables, tans) = tans_tables(&hists);
            body.tables = tables;
            body.streams = symbols
                .iter()
                .map(|(values, runs)| IndexStream {
                    coding: Coding::Rle,
                    data: if values.is_empty() { Bytes::default() } else { tans[0].encode(values).into() },
                    extra: if runs.is_empty() { Bytes::default() } else { tans[1].encode(runs).into() },
                    symbols: values.len(),
                    extra_symbols: runs.len(),
                    ..IndexStream::default()
                })
                .collect();
        }
        IndexForm::Xor => {
            body.streams = (0..count)
                .map(|n| {
                    let mut plane = planes.master[n].clone();
                    xor_previous(&mut plane, &planes.layout[n], false);
                    plain_stream(Coding::Xor, &plane)
                })
                .collect();
        }
        IndexForm::Spans { with_side, lz4 } => {
            body.streams = (0..count)
                .map(|n| {
                    let content = span_content(&planes.original[n], &planes.rows[n], &planes.layout[n], with_side, lz4);
                    let side_mode = if with_side { content[0] } else { SIDE_RAW };
                    IndexStream { coding: Coding::Spans { with_side, lz4, dict: false }, symbols: content.len(), data: squeeze(lz4, &content), side_mode, ..IndexStream::default() }
                })
                .collect();
        }
        IndexForm::SpansDict(size) => {
            let dictionary = dictionaries.spans[Dictionaries::at(size)].clone();
            let mut compressor = bare_compressor(&dictionary);
            let dict = !dictionary.is_empty();
            body.streams = (0..count)
                .map(|n| {
                    let content = span_content(&planes.original[n], &planes.rows[n], &planes.layout[n], true, false);
                    IndexStream { coding: Coding::Spans { with_side: true, lz4: false, dict }, symbols: content.len(), side_mode: content[0], data: compress_bare(&mut compressor, &content).into(), not_counted: ZSTD_MAGIC_BYTES, ..IndexStream::default() }
                })
                .collect();
            body.dictionary = dictionary;
        }
        IndexForm::Painted { shape, lz4 } => body.streams = (0..count).map(|n| painted_stream(planes, n, shape, lz4)).collect(),
        IndexForm::Boxes { with_side, lz4 } => {
            body.streams = (0..count)
                .map(|n| {
                    let sizes = &planes.layout[n].sizes;
                    let content = split::box_content(&planes.original[n], &split::frame_cuts(&planes.mask[n], sizes), sizes, with_side);
                    IndexStream { coding: Coding::Boxes { with_side, lz4 }, symbols: content.len(), data: squeeze(lz4, &content), ..IndexStream::default() }
                })
                .collect();
        }
        IndexForm::PlainFresh | IndexForm::DictBest | IndexForm::SpansDictBest | IndexForm::LeftPredict | IndexForm::Choice { .. } => unreachable!("{form:?} is made from the others"),
    }
    body
}

/// What a body's schemes have coded so far: the same streams appear in many schemes, and Zstd 19 is what takes the time.
#[derive(Default)]
struct Atoms {
    index: HashMap<IndexForm, IndexBody>,
    coverage: HashMap<CoverageForm, CoverageBody>,
}

impl Atoms {
    fn recoded(&mut self, form: IndexForm, coding: Coding, planes: &StreamPlanes, dictionaries: &Dictionaries) -> IndexBody {
        let mut body = self.index_of(form, planes, dictionaries);
        body.streams.iter_mut().for_each(|stream| stream.coding = coding);
        body
    }

    /// The dictionary size that makes these streams smallest with the dictionary counted, or none.
    fn best_size(&mut self, none: IndexForm, with: fn(usize) -> IndexForm, planes: &StreamPlanes, dictionaries: &Dictionaries) -> Option<usize> {
        let total = |b: &IndexBody| b.streams.iter().map(IndexStream::stored).sum::<u64>() + b.shared();
        let mut best = (None, total(&self.index_of(none, planes, dictionaries)));
        for size in DICT_SIZES {
            let other = total(&self.index_of(with(size), planes, dictionaries));
            if other < best.1 {
                best = (Some(size), other);
            }
        }
        best.0
    }

    fn index_of(&mut self, form: IndexForm, planes: &StreamPlanes, dictionaries: &Dictionaries) -> IndexBody {
        if let Some(body) = self.index.get(&form) {
            return body.clone();
        }
        let body = match form {
            IndexForm::PlainFresh => self.recoded(IndexForm::Plain, Coding::ZstdFresh, planes, dictionaries),
            IndexForm::LeftPredict => self.recoded(IndexForm::Left, Coding::LeftPredict, planes, dictionaries),
            IndexForm::DictBest => {
                let size = match dictionaries.picks {
                    Some(picks) => picks.plain,
                    None => self.best_size(IndexForm::Bare, IndexForm::Dict, planes, dictionaries),
                };
                let mut best = self.index_of(size.map_or(IndexForm::Bare, IndexForm::Dict), planes, dictionaries);
                best.choice_bytes = DICT_CHOICE_BYTES;
                best
            }
            IndexForm::SpansDictBest => {
                let size = match dictionaries.picks {
                    Some(picks) => picks.spans,
                    None => self.best_size(SPANS_ZSTD, IndexForm::SpansDict, planes, dictionaries),
                };
                let mut best = self.index_of(size.map_or(SPANS_ZSTD, IndexForm::SpansDict), planes, dictionaries);
                best.choice_bytes = DICT_CHOICE_BYTES;
                best
            }
            IndexForm::Choice { with_up } => {
                let mut options = vec![self.index_of(IndexForm::Plain, planes, dictionaries), self.index_of(IndexForm::Left, planes, dictionaries)];
                if with_up {
                    options.push(self.index_of(IndexForm::Up, planes, dictionaries));
                }
                let streams = (0..planes.streams())
                    .map(|n| options.iter().map(|option| &option.streams[n]).min_by_key(|stream| stream.stored()).expect("an option").clone())
                    .collect();
                IndexBody { streams, choice_bytes: STREAM_CHOICE_BYTES * planes.streams() as u64, ..IndexBody::default() }
            }
            _ => encode_index(form, planes, dictionaries),
        };
        self.index.insert(form, body.clone());
        body
    }

    fn coverage_of(&mut self, form: CoverageForm, planes: &StreamPlanes) -> CoverageBody {
        // The same bytes; only the decoder's context differs.
        let form = if form == CoverageForm::ZstdFresh { CoverageForm::Zstd } else { form };
        self.coverage.entry(form).or_insert_with(|| encode_coverage(form, planes)).clone()
    }
}

/// The runs of a stream's coverage, and its partial values in order.
fn coverage_runs(coverage: &[u8], sizes: &[(usize, usize)], copy_up: bool) -> (Vec<u8>, Vec<u8>) {
    let (mut tokens, mut partial) = (Vec::new(), Vec::new());
    let mut start = 0usize;
    for &(w, h) in sizes {
        let end = start + w * h;
        let mut at = start;
        while at < end {
            let value = coverage[at];
            let kind = match value {
                COVERAGE_TRANSPARENT => RUN_TRANSPARENT,
                COVERAGE_SOLID => RUN_SOLID,
                _ => RUN_PARTIAL,
            };
            let same = coverage[at..end]
                .iter()
                .take(RUN_LONGEST)
                .take_while(|&&c| match kind {
                    RUN_TRANSPARENT => c == COVERAGE_TRANSPARENT,
                    RUN_SOLID => c == COVERAGE_SOLID,
                    _ => c != COVERAGE_TRANSPARENT && c != COVERAGE_SOLID,
                })
                .count();
            // Never longer than a row, so the copy doesn't overlap itself.
            let copied = if copy_up && at >= start + w {
                coverage[at..end].iter().zip(&coverage[at - w..]).take(RUN_LONGEST.min(w)).take_while(|(a, b)| a == b).count()
            } else {
                0
            };
            if copied >= COPY_UP_SHORTEST && (copied > same || (copied == same && kind == RUN_PARTIAL)) {
                tokens.push((RUN_COPY_UP << RUN_KIND_SHIFT) | (copied - 1) as u8);
                at += copied;
            } else {
                tokens.push((kind << RUN_KIND_SHIFT) | (same - 1) as u8);
                if kind == RUN_PARTIAL {
                    partial.extend_from_slice(&coverage[at..at + same]);
                }
                at += same;
            }
        }
        start = end;
    }
    (tokens, partial)
}

fn expand_runs(tokens: &[u8], partial: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut token, mut next, mut start) = (0usize, 0usize, 0usize);
    for &(w, h) in sizes {
        let end = start + w * h;
        let mut at = start;
        while at < end {
            let byte = tokens[token];
            token += 1;
            let length = usize::from(byte & RUN_LENGTH_MASK) + 1;
            match byte >> RUN_KIND_SHIFT {
                RUN_TRANSPARENT => {}
                RUN_SOLID => out[at..at + length].fill(COVERAGE_SOLID),
                RUN_PARTIAL => {
                    out[at..at + length].copy_from_slice(&partial[next..next + length]);
                    next += length;
                }
                _ => out.copy_within(at - w..at - w + length, at),
            }
            at += length;
        }
        start = end;
    }
}

fn encode_coverage(form: CoverageForm, planes: &StreamPlanes) -> CoverageBody {
    let mut body = CoverageBody::default();
    if planes.client != Client::Ec {
        return body;
    }
    match form {
        CoverageForm::ZstdFresh | CoverageForm::Zstd => {
            body.streams = planes.mask.iter().map(|m| CoverageStream { runs: compress_level(m, LEVEL_19, false).into(), ..CoverageStream::default() }).collect();
        }
        CoverageForm::Lz4 => {
            body.streams = planes.mask.iter().map(|m| CoverageStream { runs: crowd::lz4_compress(m).into(), ..CoverageStream::default() }).collect();
        }
        CoverageForm::Spans { lz4 } => {
            body.streams = (0..planes.streams())
                .map(|n| {
                    let content = span_content(&planes.mask[n], &planes.rows[n], &planes.layout[n], true, lz4);
                    CoverageStream { runs: squeeze(lz4, &content), run_count: content.len(), ..CoverageStream::default() }
                })
                .collect();
        }
        CoverageForm::Boxes { lz4 } => {
            body.streams = (0..planes.streams())
                .map(|n| {
                    let sizes = &planes.layout[n].sizes;
                    let content = split::box_content(&planes.mask[n], &split::frame_cuts(&planes.mask[n], sizes), sizes, true);
                    CoverageStream { runs: squeeze(lz4, &content), run_count: content.len(), ..CoverageStream::default() }
                })
                .collect();
        }
        CoverageForm::Contexts | CoverageForm::RunsContexts => {
            let every = form == CoverageForm::Contexts;
            let sequences: Vec<Vec<(u8, u8)>> = planes.mask.iter().zip(&planes.layout).map(|(m, l)| context_pairs(m, &l.sizes, !every)).collect();
            let (tables, coder) = ContextCoder::from_sequences(&sequences);
            body.tables = tables;
            body.streams = (0..planes.streams())
                .map(|n| {
                    let coded = coder.encode(&sequences[n]);
                    if every {
                        CoverageStream { runs: coded.into(), ..CoverageStream::default() }
                    } else {
                        let tokens = coverage_runs(&planes.mask[n], &planes.layout[n].sizes, false).0;
                        CoverageStream { runs: squeeze(false, &tokens), run_count: tokens.len(), partial: coded.into(), partial_count: sequences[n].len() }
                    }
                })
                .collect();
        }
        CoverageForm::Runs | CoverageForm::RunsUp | CoverageForm::RunsZstd | CoverageForm::RunsUpZstd => {
            let copy_up = matches!(form, CoverageForm::RunsUp | CoverageForm::RunsUpZstd);
            let zstd_runs = matches!(form, CoverageForm::RunsZstd | CoverageForm::RunsUpZstd);
            let symbols: Vec<(Vec<u8>, Vec<u8>)> = planes.mask.iter().zip(&planes.layout).map(|(m, l)| coverage_runs(m, &l.sizes, copy_up)).collect();
            let mut hists = vec![vec![0u32; BYTE_SYMBOLS]; COVERAGE_TABLES];
            for (tokens, partial) in &symbols {
                histogram_of(tokens, &mut hists[0]);
                histogram_of(partial, &mut hists[1]);
            }
            let (tables, tans) = tans_tables(&hists);
            body.tables = tables;
            body.streams = symbols
                .iter()
                .map(|(tokens, partial)| CoverageStream {
                    runs: match (tokens.is_empty(), zstd_runs) {
                        (true, _) => Bytes::default(),
                        (false, true) => compress_level(tokens, LEVEL_19, false).into(),
                        (false, false) => tans[0].encode(tokens).into(),
                    },
                    partial: if partial.is_empty() { Bytes::default() } else { tans[1].encode(partial).into() },
                    run_count: tokens.len(),
                    partial_count: partial.len(),
                })
                .collect();
        }
    }
    body
}

// ---------------------------------------------------------------------------------------------------------------
// The tight decode

/// Which pixels are opaque, as a coverage plane or as packed bits.
trait Shape {
    fn opaque(&self, i: usize) -> bool;
    /// The first opaque pixel of `from..end`, or `end`.
    fn next_opaque(&self, from: usize, end: usize) -> usize;
    /// The first transparent pixel of `from..end`, or `end`.
    fn next_clear(&self, from: usize, end: usize) -> usize;
}

const WORD_BYTES: usize = 8;
const WORD_LOW_BITS: u64 = 0x0101_0101_0101_0101;
const WORD_HIGH_BITS: u64 = 0x8080_8080_8080_8080;

struct BytesShape<'a>(&'a [u8]);

impl Shape for BytesShape<'_> {
    #[inline(always)]
    fn opaque(&self, i: usize) -> bool {
        self.0[i] != 0
    }

    #[inline(always)]
    fn next_opaque(&self, from: usize, end: usize) -> usize {
        let mut at = from;
        while at + WORD_BYTES <= end {
            let word = u64::from_le_bytes(self.0[at..at + WORD_BYTES].try_into().unwrap());
            if word != 0 {
                return at + (word.trailing_zeros() / u8::BITS) as usize;
            }
            at += WORD_BYTES;
        }
        while at < end && self.0[at] == 0 {
            at += 1;
        }
        at
    }

    #[inline(always)]
    fn next_clear(&self, from: usize, end: usize) -> usize {
        let mut at = from;
        while at + WORD_BYTES <= end {
            let word = u64::from_le_bytes(self.0[at..at + WORD_BYTES].try_into().unwrap());
            // The lowest set bit marks the first zero byte.
            let zeros = word.wrapping_sub(WORD_LOW_BITS) & !word & WORD_HIGH_BITS;
            if zeros != 0 {
                return at + (zeros.trailing_zeros() / u8::BITS) as usize;
            }
            at += WORD_BYTES;
        }
        while at < end && self.0[at] != 0 {
            at += 1;
        }
        at
    }
}

/// Packed bits, followed by [`WORD_BYTES`] of padding.
struct BitsShape<'a>(&'a [u8]);

impl BitsShape<'_> {
    #[inline(always)]
    fn stop_at(&self, from: usize, end: usize, invert: u64) -> usize {
        let mut at = from;
        while at < end {
            let byte = at / u8::BITS as usize;
            let skipped = (at % u8::BITS as usize) as u32;
            let word = u64::from_le_bytes(self.0[byte..byte + WORD_BYTES].try_into().unwrap());
            let stops = (word ^ invert) >> skipped;
            if stops != 0 {
                return (at + stops.trailing_zeros() as usize).min(end);
            }
            at += (u64::BITS - skipped) as usize;
        }
        end
    }
}

impl Shape for BitsShape<'_> {
    #[inline(always)]
    fn opaque(&self, i: usize) -> bool {
        (self.0[i / u8::BITS as usize] >> (i % u8::BITS as usize)) & 1 != 0
    }

    #[inline(always)]
    fn next_opaque(&self, from: usize, end: usize) -> usize {
        self.stop_at(from, end, 0)
    }

    #[inline(always)]
    fn next_clear(&self, from: usize, end: usize) -> usize {
        self.stop_at(from, end, u64::MAX)
    }
}

/// `out[n] = seed + residuals[0] + .. + residuals[n]`, in vectors of [`LANES`] where the span allows.
#[inline(always)]
fn prefix_sum(residuals: &[u8], seed: u8, out: &mut [u8]) {
    let count = out.len();
    assert!(residuals.len() >= count, "residuals of a span");
    let mut at = 0usize;
    let mut previous = seed;
    #[cfg(target_arch = "x86_64")]
    if count >= LANES {
        use std::arch::x86_64::*;
        const LAST_LANE_EVERYWHERE: i32 = 0xFF;
        // SAFETY: SSE2 is in the x86_64 baseline, and `at + LANES <= count` keeps every access inside both slices.
        unsafe {
            let mut carry = _mm_set1_epi8(seed as i8);
            while at + LANES <= count {
                let mut v = _mm_loadu_si128(residuals.as_ptr().add(at).cast());
                v = _mm_add_epi8(v, _mm_slli_si128::<1>(v));
                v = _mm_add_epi8(v, _mm_slli_si128::<2>(v));
                v = _mm_add_epi8(v, _mm_slli_si128::<4>(v));
                v = _mm_add_epi8(v, _mm_slli_si128::<8>(v));
                v = _mm_add_epi8(v, carry);
                _mm_storeu_si128(out.as_mut_ptr().add(at).cast(), v);
                let high = _mm_unpackhi_epi8(v, v);
                carry = _mm_shuffle_epi32::<LAST_LANE_EVERYWHERE>(_mm_unpackhi_epi16(high, high));
                at += LANES;
            }
        }
        previous = out[at - 1];
    }
    for (value, residual) in out[at..].iter_mut().zip(&residuals[at..]) {
        previous = previous.wrapping_add(*residual);
        *value = previous;
    }
}

/// An opaque span with the up predictor: a vector add under opaque pixels, otherwise from the left.
#[inline(always)]
fn span_up<S: Shape>(shape: &S, residuals: &[u8], out: &mut [u8], w: usize, has_up: bool, span: (usize, usize), last: u8) {
    let (first, stop) = span;
    let mut at = first;
    let mut next = 0usize;
    while at < stop {
        if has_up && shape.opaque(at - w) {
            let run_end = shape.next_clear(at - w, stop - w) + w;
            let length = run_end - at;
            let (before, after) = out.split_at_mut(at);
            let up = &before[at - w..at - w + length];
            after[..length].iter_mut().zip(up).zip(&residuals[next..next + length]).for_each(|((value, above), residual)| *value = above.wrapping_add(*residual));
            next += length;
            at = run_end;
        } else {
            let run_end = if has_up { shape.next_opaque(at - w, stop - w) + w } else { stop };
            let length = run_end - at;
            let seed = if at > first { out[at - 1] } else { last };
            prefix_sum(&residuals[next..next + length], seed, &mut out[at..run_end]);
            next += length;
            at = run_end;
        }
    }
}

/// Rebuilds a stream's index plane (`out` holds zeros) from the shape and the residuals, span by span.
#[inline(always)]
fn decode_residuals<S: Shape>(up: bool, shape: &S, residuals: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut next, mut start) = (0usize, 0usize);
    for &(w, h) in sizes {
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            let row = start + y * w;
            let end = row + w;
            let mut x = shape.next_opaque(row, end);
            while x < end {
                let stop = shape.next_clear(x, end);
                let length = stop - x;
                if up {
                    span_up(shape, &residuals[next..next + length], out, w, y > 0, (x, stop), last);
                } else {
                    let seed = if y > 0 && shape.opaque(x - w) { out[x - w] } else { last };
                    prefix_sum(&residuals[next..next + length], seed, &mut out[x..stop]);
                }
                next += length;
                last = out[stop - 1];
                x = shape.next_opaque(stop, end);
            }
        }
        start += w * h;
    }
}

/// The earlier rows' decode: a mask of bytes and `predict` per pixel.
fn decode_predicted(mask: &[u8], residuals: &[u8], sizes: &[(usize, usize)], out: &mut [u8]) {
    let (mut next, mut start) = (0usize, 0usize);
    for &(w, h) in sizes {
        let mut last = FIRST_PREDICTION;
        for y in 0..h {
            for x in 0..w {
                let i = start + y * w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (prediction, _) = predict(out, mask, w, i, x, y > 0, Predictor::Left, last);
                out[i] = prediction.wrapping_add(residuals[next]);
                last = out[i];
                next += 1;
            }
        }
        start += w * h;
    }
}

fn expand_rle(values: &[u8], runs: &[u8], out: &mut [u8]) {
    let (mut at, mut run) = (0usize, 0usize);
    for &value in values {
        let mut length = 1usize;
        loop {
            let byte = runs[run];
            run += 1;
            length += usize::from(byte);
            if byte != RLE_RUN_MORE {
                break;
            }
        }
        out[at..at + length].fill(value);
        at += length;
    }
}

struct Scratch {
    context: Decompressor<'static>,
    index: Vec<u8>,
    coverage: Vec<u8>,
    symbols: Vec<u8>,
    extra: Vec<u8>,
    rows: Vec<Row>,
    cuts: Vec<split::FrameCuts>,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch { context: Decompressor::new().expect("zstd context"), index: Vec::new(), coverage: Vec::new(), symbols: Vec::new(), extra: Vec::new(), rows: Vec::new(), cuts: Vec::new() }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

#[inline(always)]
fn unzstd(context: &mut Decompressor<'_>, data: &[u8], capacity: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(capacity);
    context.decompress_to_buffer(data, out).expect("zstd decompress");
}

#[inline(always)]
fn unpack(lz4: bool, context: &mut Decompressor<'_>, data: &[u8], size: usize, out: &mut Vec<u8>) {
    if size == 0 {
        out.clear();
    } else if lz4 {
        *out = crowd::lz4_decompress(data, size);
    } else {
        unzstd(context, data, size, out);
    }
}

#[inline(always)]
fn tans_into(table: &Tans, data: &[u8], count: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(count, 0);
    if count > 0 {
        table.decode_into(data, out);
    }
}

/// What a load builds once per job: the body's tANS tables and its dictionary.
struct BodyTables<'a> {
    index: Vec<Tans>,
    coverage: Vec<Tans>,
    contexts: Option<ContextCoder>,
    context: Option<Decompressor<'a>>,
}

const INDEX_TABLES: usize = 2;
const COVERAGE_TABLES: usize = 2;

fn coverage_stream_decode(form: CoverageForm, stream: &CoverageStream, layout: &StreamLayout, tables: &BodyTables, o: &mut Scratch, expand: bool) {
    let Scratch { context, coverage, symbols, extra, rows, cuts, .. } = o;
    match form {
        CoverageForm::ZstdFresh => *coverage = decompress(&stream.runs, layout.pixels),
        CoverageForm::Zstd => unzstd(context, &stream.runs, layout.pixels, coverage),
        CoverageForm::Lz4 => *coverage = crowd::lz4_decompress(&stream.runs, layout.pixels),
        CoverageForm::Runs | CoverageForm::RunsUp | CoverageForm::RunsZstd | CoverageForm::RunsUpZstd => {
            if matches!(form, CoverageForm::Runs | CoverageForm::RunsUp) {
                tans_into(&tables.coverage[0], &stream.runs, stream.run_count, symbols);
            } else {
                unzstd(context, &stream.runs, stream.run_count, symbols);
            }
            tans_into(&tables.coverage[1], &stream.partial, stream.partial_count, extra);
            coverage.clear();
            coverage.resize(layout.pixels, COVERAGE_TRANSPARENT);
            expand_runs(symbols, extra, &layout.sizes, coverage);
        }
        CoverageForm::Contexts => {
            coverage.clear();
            coverage.resize(layout.pixels, COVERAGE_TRANSPARENT);
            decode_contexts(tables.contexts.as_ref().expect("the tables"), &stream.runs, &layout.sizes, coverage);
        }
        CoverageForm::RunsContexts => {
            unpack(false, context, &stream.runs, stream.run_count, symbols);
            coverage.clear();
            coverage.resize(layout.pixels, COVERAGE_TRANSPARENT);
            decode_runs_contexts(symbols, tables.contexts.as_ref().expect("the tables"), &stream.partial, &layout.sizes, coverage);
        }
        CoverageForm::Spans { lz4 } => {
            unpack(lz4, context, &stream.runs, stream.run_count, symbols);
            let at = parse_side(symbols, layout, rows);
            if expand {
                coverage.clear();
                coverage.resize(layout.pixels, COVERAGE_TRANSPARENT);
                expand_rows(&symbols[at..], rows, &layout.sizes, coverage);
            }
        }
        CoverageForm::Boxes { lz4 } => {
            unpack(lz4, context, &stream.runs, stream.run_count, symbols);
            let at = split::parse_cuts(symbols, &layout.sizes, cuts);
            if expand {
                coverage.clear();
                coverage.resize(layout.pixels, COVERAGE_TRANSPARENT);
                split::expand_cuts(&symbols[at..], cuts, &layout.sizes, coverage);
            }
        }
    }
}

fn index_stream_decode(client: Client, stream: &IndexStream, layout: &StreamLayout, tables: &mut BodyTables, o: &mut Scratch, expand: bool) {
    let Scratch { context, index, coverage, symbols, extra, rows, cuts } = o;
    match stream.coding {
        Coding::ZstdFresh => *index = decompress(&stream.data, layout.pixels),
        Coding::Zstd => unzstd(context, &stream.data, layout.pixels, index),
        Coding::Lz4 => *index = crowd::lz4_decompress(&stream.data, layout.pixels),
        Coding::ZstdDict => unzstd(tables.context.as_mut().expect("the dictionary"), &stream.data, layout.pixels, index),
        Coding::Left | Coding::Up => {
            unzstd(context, &stream.data, stream.symbols, symbols);
            index.clear();
            index.resize(layout.pixels, 0);
            let up = stream.coding == Coding::Up;
            match client {
                Client::Cc => {
                    unzstd(context, &stream.extra, packed_bytes(layout.pixels) + WORD_BYTES, extra);
                    extra.resize(extra.len() + WORD_BYTES, 0);
                    decode_residuals(up, &BitsShape(extra), symbols, &layout.sizes, index);
                }
                Client::Ec => decode_residuals(up, &BytesShape(coverage), symbols, &layout.sizes, index),
            }
        }
        Coding::LeftPredict => {
            unzstd(context, &stream.data, stream.symbols, symbols);
            index.clear();
            index.resize(layout.pixels, 0);
            match client {
                Client::Cc => {
                    unzstd(context, &stream.extra, packed_bytes(layout.pixels), extra);
                    let mask: Vec<u8> = (0..layout.pixels).map(|n| (extra[n / u8::BITS as usize] >> (n % u8::BITS as usize)) & 1).collect();
                    decode_predicted(&mask, symbols, &layout.sizes, index);
                }
                Client::Ec => decode_predicted(coverage, symbols, &layout.sizes, index),
            }
        }
        Coding::Rle => {
            tans_into(&tables.index[0], &stream.data, stream.symbols, symbols);
            tans_into(&tables.index[1], &stream.extra, stream.extra_symbols, extra);
            index.clear();
            index.resize(layout.pixels, 0);
            expand_rle(symbols, extra, index);
        }
        Coding::Xor => {
            unzstd(context, &stream.data, layout.pixels, index);
            xor_previous(index, layout, true);
        }
        Coding::Spans { with_side, lz4, dict } => {
            if dict {
                unzstd(tables.context.as_mut().expect("the dictionary"), &stream.data, stream.symbols, symbols);
            } else {
                unpack(lz4, context, &stream.data, stream.symbols, symbols);
            }
            let at = if with_side { parse_side(symbols, layout, rows) } else { 0 };
            if expand {
                index.clear();
                index.resize(layout.pixels, 0);
                expand_rows(&symbols[at..], rows, &layout.sizes, index);
            }
        }
        Coding::Boxes { with_side, lz4 } => {
            unpack(lz4, context, &stream.data, stream.symbols, symbols);
            let at = if with_side { split::parse_cuts(symbols, &layout.sizes, cuts) } else { 0 };
            if expand {
                index.clear();
                index.resize(layout.pixels, 0);
                split::expand_cuts(&symbols[at..], cuts, &layout.sizes, index);
            }
        }
        Coding::Painted { shape, lz4 } => {
            unpack(lz4, context, &stream.data, stream.symbols, symbols);
            if matches!(shape, ShapeForm::RunsApart | ShapeForm::Bits) {
                unpack(lz4, context, &stream.extra, stream.extra_symbols, extra);
            }
            if expand {
                index.clear();
                index.resize(layout.pixels, 0);
                match shape {
                    ShapeForm::InCoverage => expand_painted(&BytesShape(coverage), symbols, &layout.sizes, index),
                    ShapeForm::Bits => {
                        extra.resize(extra.len() + WORD_BYTES, 0);
                        expand_painted(&BitsShape(extra), symbols, &layout.sizes, index);
                    }
                    ShapeForm::RunsApart => expand_by_runs(extra, symbols, &layout.sizes, index),
                    ShapeForm::RunsInFrame => {
                        let mut at = 0usize;
                        let runs = get_varint(symbols, &mut at) as usize;
                        expand_by_runs(&symbols[at..at + runs], &symbols[at + runs..], &layout.sizes, index);
                    }
                }
            }
        }
    }
}

/// Everything of one scheme for one body.
struct Coded {
    client: Client,
    index_form: IndexForm,
    coverage_form: CoverageForm,
    index: IndexBody,
    coverage: CoverageBody,
    /// Decodes write the rectangle the atlas needs; otherwise they stop at spans a renderer could keep.
    expand: bool,
}

impl Coded {
    fn tables<'a>(&'a self, cached: Option<&'a DecoderDictionary<'a>>) -> BodyTables<'a> {
        let index = if self.index.tables.is_empty() { Vec::new() } else { parse_tans(&self.index.tables, INDEX_TABLES) };
        let by_context = matches!(self.coverage_form, CoverageForm::Contexts | CoverageForm::RunsContexts);
        let coverage = if self.coverage.tables.is_empty() || by_context { Vec::new() } else { parse_tans(&self.coverage.tables, COVERAGE_TABLES) };
        let contexts = (by_context && !self.coverage.tables.is_empty()).then(|| ContextCoder::parse(&self.coverage.tables));
        let context = (!self.index.dictionary.is_empty()).then(|| match cached {
            Some(dictionary) => Decompressor::with_prepared_dictionary(dictionary).expect("zstd dictionary"),
            None => Decompressor::with_dictionary(&self.index.dictionary).expect("zstd dictionary"),
        });
        BodyTables { index, coverage, contexts, context }
    }

    /// Decodes into the thread's scratch buffers; the pixels of the stream.
    fn decode(&self, layout: &[StreamLayout], stream: usize, tables: &mut BodyTables, o: &mut Scratch, expand: bool) -> usize {
        if self.client == Client::Ec {
            coverage_stream_decode(self.coverage_form, &self.coverage.streams[stream], &layout[stream], tables, o, expand);
        }
        index_stream_decode(self.client, &self.index.streams[stream], &layout[stream], tables, o, expand);
        layout[stream].pixels
    }

    /// A job: the body's tables once, then the wanted streams.
    fn decode_all<'a>(&'a self, layout: &[StreamLayout], wanted: &[usize], cached: Option<&'a DecoderDictionary<'a>>) -> usize {
        let mut tables = self.tables(cached);
        SCRATCH.with(|scratch| {
            let o = &mut *scratch.borrow_mut();
            wanted.iter().map(|&stream| self.decode(layout, stream, &mut tables, o, self.expand)).sum()
        })
    }

    fn stored_of(&self, stream: usize) -> u64 {
        self.index.streams[stream].stored() + self.coverage.streams.get(stream).map_or(0, CoverageStream::stored)
    }

    fn shared(&self) -> u64 {
        self.index.shared() + self.coverage.tables.len() as u64
    }

    fn total(&self) -> u64 {
        self.shared() + (0..self.index.streams.len()).map(|s| self.stored_of(s)).sum::<u64>()
    }

    fn verify(&self, planes: &StreamPlanes) {
        let mut tables = self.tables(None);
        let mut o = Scratch::new();
        for stream in 0..planes.streams() {
            self.decode(&planes.layout, stream, &mut tables, &mut o, true);
            let expected = match self.index.streams[stream].coding {
                Coding::ZstdFresh | Coding::Zstd | Coding::Lz4 | Coding::ZstdDict | Coding::Rle | Coding::Spans { .. } | Coding::Painted { .. } | Coding::Boxes { .. } => &planes.original[stream],
                Coding::LeftPredict | Coding::Left | Coding::Up => &planes.luma[stream],
                Coding::Xor => &planes.master[stream],
            };
            assert!(o.index == *expected, "{:?}: index plane of stream {stream}", self.index_form);
            if self.client == Client::Ec {
                assert!(o.coverage == planes.mask[stream], "{:?}: coverage of stream {stream}", self.coverage_form);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// t1: sizes on the sample

#[derive(Default, Clone, Copy)]
struct ZstdOverhead {
    /// Magic number and frame header.
    frame: u64,
    block_headers: u64,
    /// Literals section headers and Huffman tree descriptions.
    literal_tables: u64,
    /// Sequences section headers and FSE table descriptions.
    sequence_tables: u64,
}

const ZSTD_DICT_ID_BYTES: [usize; 4] = [0, 1, 2, 4];
const ZSTD_SIZE_BYTES: [usize; 4] = [1, 2, 4, 8];
const ZSTD_BLOCK_HEADER_BYTES: usize = 3;
const ZSTD_SINGLE_SEGMENT: u8 = 0x20;
const ZSTD_CHECKSUM: u8 = 0x04;
const ZSTD_CHECKSUM_BYTES: usize = 4;
const ZSTD_DICT_ID_MASK: u8 = 3;
const ZSTD_SIZE_FLAG_SHIFT: u32 = 6;
const ZSTD_BLOCK_RAW: u32 = 0;
const ZSTD_BLOCK_RLE: u32 = 1;
const LITERALS_RAW: u8 = 0;
const LITERALS_RLE: u8 = 1;
const LITERALS_COMPRESSED: u8 = 2;
const LITERALS_TYPE_MASK: u8 = 3;
const LITERALS_FORMAT_SHIFT: u32 = 2;
const LITERALS_FORMAT_MASK: u8 = 3;
const HUFFMAN_FSE_BELOW: u8 = 128;
const HUFFMAN_DIRECT_OFFSET: usize = 127;
const JUMP_TABLE_BYTES: usize = 6;
const SEQUENCES_TWO_BYTES_FROM: u8 = 128;
const SEQUENCES_THREE_BYTES_AT: u8 = 255;
const SEQUENCE_MODE_RLE: u8 = 1;
const SEQUENCE_MODE_FSE: u8 = 2;
const SEQUENCE_MODE_MASK: u8 = 3;
const SEQUENCE_MODE_SHIFTS: [u32; 3] = [6, 4, 2];
const FSE_LOG_BITS: u32 = 4;
const FSE_LOG_BASE: u32 = 5;
const FSE_ZERO_REPEAT_BITS: u32 = 2;
const FSE_ZERO_REPEAT_MORE: u32 = 3;

fn read_le(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0u64, |value, &byte| (value << u8::BITS) | u64::from(byte))
}

/// The bytes of an FSE table description (RFC 8878, 4.1.1).
fn fse_table_bytes(data: &[u8]) -> usize {
    let mut bit = 0usize;
    let read = |bit: &mut usize, count: u32| -> u32 {
        let value = (0..count as usize).fold(0u32, |value, n| {
            let at = *bit + n;
            let byte = data.get(at / u8::BITS as usize).copied().unwrap_or(0);
            value | (u32::from((byte >> (at % u8::BITS as usize)) & 1) << n)
        });
        *bit += count as usize;
        value
    };
    let log = read(&mut bit, FSE_LOG_BITS) + FSE_LOG_BASE;
    let mut remaining = 1i32 << log;
    while remaining > 0 {
        let bits = u32::BITS - ((remaining + 1) as u32).leading_zeros();
        let mut value = read(&mut bit, bits);
        let lower = (1u32 << (bits - 1)) - 1;
        let threshold = (1u32 << bits) - 1 - (remaining as u32 + 1);
        if value & lower < threshold {
            bit -= 1;
            value &= lower;
        } else if value > lower {
            value -= threshold;
        }
        let probability = value as i32 - 1;
        remaining -= probability.abs();
        if probability == 0 {
            while read(&mut bit, FSE_ZERO_REPEAT_BITS) == FSE_ZERO_REPEAT_MORE {}
        }
    }
    bit.div_ceil(u8::BITS as usize)
}

fn compressed_block_overhead(block: &[u8], out: &mut ZstdOverhead) {
    let first = block[0];
    let kind = first & LITERALS_TYPE_MASK;
    let format = (first >> LITERALS_FORMAT_SHIFT) & LITERALS_FORMAT_MASK;
    let (header, content, tree) = if kind == LITERALS_RAW || kind == LITERALS_RLE {
        let (header, regenerated) = match format {
            0 | 2 => (1, usize::from(first >> 3)),
            1 => (2, (read_le(&block[..2]) >> 4) as usize),
            _ => (3, (read_le(&block[..3]) >> 4) as usize),
        };
        (header, if kind == LITERALS_RAW { regenerated } else { 1 }, 0)
    } else {
        let (header, compressed) = match format {
            0 | 1 => (3, (read_le(&block[..3]) >> 14) as usize),
            2 => (4, (read_le(&block[..4]) >> 18) as usize),
            _ => (5, (read_le(&block[..5]) >> 22) as usize),
        };
        let tree = if kind == LITERALS_COMPRESSED {
            let description = block[header];
            if description < HUFFMAN_FSE_BELOW {
                1 + usize::from(description)
            } else {
                1 + (usize::from(description) - HUFFMAN_DIRECT_OFFSET).div_ceil(2)
            }
        } else {
            0
        };
        (header, compressed, tree + if format == 0 { 0 } else { JUMP_TABLE_BYTES })
    };
    out.literal_tables += (header + tree) as u64;
    let sequences = &block[header + content..];
    let count = sequences[0];
    let header = match count {
        0 => {
            out.sequence_tables += 1;
            return;
        }
        c if c < SEQUENCES_TWO_BYTES_FROM => 1,
        c if c < SEQUENCES_THREE_BYTES_AT => 2,
        _ => 3,
    };
    let modes = sequences[header];
    let mut at = header + 1;
    for shift in SEQUENCE_MODE_SHIFTS {
        match (modes >> shift) & SEQUENCE_MODE_MASK {
            SEQUENCE_MODE_RLE => at += 1,
            SEQUENCE_MODE_FSE => at += fse_table_bytes(&sequences[at..]),
            _ => {}
        }
    }
    out.sequence_tables += at as u64;
}

/// What of a Zstd frame is not entropy-coded payload.
fn zstd_overhead(frame: &[u8]) -> ZstdOverhead {
    let mut out = ZstdOverhead::default();
    let descriptor = frame[ZSTD_MAGIC_BYTES];
    let single = descriptor & ZSTD_SINGLE_SEGMENT != 0;
    let size_flag = usize::from(descriptor >> ZSTD_SIZE_FLAG_SHIFT);
    let size_bytes = if size_flag == 0 && !single { 0 } else { ZSTD_SIZE_BYTES[size_flag] };
    let mut at = ZSTD_MAGIC_BYTES + 1 + usize::from(!single) + ZSTD_DICT_ID_BYTES[usize::from(descriptor & ZSTD_DICT_ID_MASK)] + size_bytes;
    out.frame = at as u64 + if descriptor & ZSTD_CHECKSUM != 0 { ZSTD_CHECKSUM_BYTES as u64 } else { 0 };
    loop {
        let header = read_le(&frame[at..at + ZSTD_BLOCK_HEADER_BYTES]) as u32;
        at += ZSTD_BLOCK_HEADER_BYTES;
        out.block_headers += ZSTD_BLOCK_HEADER_BYTES as u64;
        let size = (header >> 3) as usize;
        match (header >> 1) & 3 {
            ZSTD_BLOCK_RAW => at += size,
            ZSTD_BLOCK_RLE => at += 1,
            _ => {
                compressed_block_overhead(&frame[at..at + size], &mut out);
                at += size;
            }
        }
        if header & 1 != 0 {
            break;
        }
    }
    out
}

const CC_PAINTED: [IndexForm; 5] = [
    IndexForm::Painted { shape: ShapeForm::RunsInFrame, lz4: false },
    IndexForm::Painted { shape: ShapeForm::RunsApart, lz4: false },
    IndexForm::Painted { shape: ShapeForm::Bits, lz4: false },
    IndexForm::Painted { shape: ShapeForm::RunsInFrame, lz4: true },
    IndexForm::Painted { shape: ShapeForm::RunsApart, lz4: true },
];
const EC_PAINTED: [IndexForm; 4] = [
    IndexForm::Painted { shape: ShapeForm::InCoverage, lz4: false },
    IndexForm::Painted { shape: ShapeForm::InCoverage, lz4: true },
    IndexForm::Spans { with_side: false, lz4: false },
    IndexForm::Spans { with_side: false, lz4: true },
];
const SPANS_ZSTD: IndexForm = IndexForm::Spans { with_side: true, lz4: false };
const SPANS_LZ4: IndexForm = IndexForm::Spans { with_side: true, lz4: true };
const SPANS_ALONE_ZSTD: IndexForm = IndexForm::Spans { with_side: false, lz4: false };
const SPANS_ALONE_LZ4: IndexForm = IndexForm::Spans { with_side: false, lz4: true };
const PAINTED_EC_ZSTD: IndexForm = IndexForm::Painted { shape: ShapeForm::InCoverage, lz4: false };
const PAINTED_EC_LZ4: IndexForm = IndexForm::Painted { shape: ShapeForm::InCoverage, lz4: true };
const COVERAGE_SPANS_ZSTD: CoverageForm = CoverageForm::Spans { lz4: false };
const COVERAGE_SPANS_LZ4: CoverageForm = CoverageForm::Spans { lz4: true };
const BOXES_SIDE_ZSTD: IndexForm = IndexForm::Boxes { with_side: true, lz4: false };
const BOXES_SIDE_LZ4: IndexForm = IndexForm::Boxes { with_side: true, lz4: true };
const BOXES_ALONE_ZSTD: IndexForm = IndexForm::Boxes { with_side: false, lz4: false };
const BOXES_ALONE_LZ4: IndexForm = IndexForm::Boxes { with_side: false, lz4: true };
const COVERAGE_BOXES_ZSTD: CoverageForm = CoverageForm::Boxes { lz4: false };
const COVERAGE_BOXES_LZ4: CoverageForm = CoverageForm::Boxes { lz4: true };
const SIZE_COVERAGE_FORMS: [CoverageForm; 10] = [
    CoverageForm::Zstd,
    CoverageForm::Lz4,
    CoverageForm::Runs,
    CoverageForm::RunsUp,
    CoverageForm::RunsZstd,
    CoverageForm::RunsUpZstd,
    CoverageForm::Contexts,
    CoverageForm::RunsContexts,
    COVERAGE_SPANS_ZSTD,
    COVERAGE_SPANS_LZ4,
];

fn size_index_forms(client: Client) -> Vec<IndexForm> {
    let mut forms = vec![IndexForm::Plain, IndexForm::Lz4, IndexForm::Bare, IndexForm::Left, IndexForm::Up, IndexForm::Rle, IndexForm::Xor, SPANS_ZSTD, SPANS_LZ4];
    forms.extend(DICT_SIZES.map(IndexForm::Dict));
    forms.extend(DICT_SIZES.map(IndexForm::SpansDict));
    match client {
        Client::Cc => forms.extend(CC_PAINTED),
        Client::Ec => forms.extend(EC_PAINTED),
    }
    forms
}

/// The coverage a form of the index plane can be checked with: spans without their rows need the coverage's.
fn coverage_with(form: IndexForm) -> CoverageForm {
    match form {
        IndexForm::Spans { with_side: false, lz4 } => CoverageForm::Spans { lz4 },
        IndexForm::Boxes { with_side: false, lz4 } => CoverageForm::Boxes { lz4 },
        _ => CoverageForm::Zstd,
    }
}

#[derive(Default)]
struct FormSizes {
    streams: Vec<u64>,
    /// The second frame of a stream: Classic's shape, or Rle's lengths.
    extra: Vec<u64>,
    shared: u64,
}

#[derive(Default, Clone, Copy)]
struct PatchBound {
    blocks: u64,
    distinct: u64,
    codebook_zstd: u64,
    id_bits: f64,
}

/// A body's exact 4 x 4 patches: the distinct ones in Zstd, and each block's id at its order-0 entropy with no table.
fn patch_bound(planes: &Planes) -> PatchBound {
    const PATCH_SIDE: usize = 4;
    const PATCH_BYTES: usize = PATCH_SIDE * PATCH_SIDE;
    let mut counts = HashMap::<[u8; PATCH_BYTES], u64>::new();
    let mut codebook = Vec::new();
    let mut blocks = 0u64;
    for frame in &planes.boxes {
        for top in (0..frame.h).step_by(PATCH_SIDE) {
            for left in (0..frame.w).step_by(PATCH_SIDE) {
                let mut patch = [0u8; PATCH_BYTES];
                for y in top..(top + PATCH_SIDE).min(frame.h) {
                    let row = frame.start + y * frame.w;
                    let width = PATCH_SIDE.min(frame.w - left);
                    patch[(y - top) * PATCH_SIDE..][..width].copy_from_slice(&planes.index[row + left..row + left + width]);
                }
                blocks += 1;
                let count = counts.entry(patch).or_default();
                if *count == 0 {
                    codebook.extend_from_slice(&patch);
                }
                *count += 1;
            }
        }
    }
    let id_bits = counts.values().map(|&n| -(n as f64) * (n as f64 / blocks as f64).log2()).sum();
    PatchBound { blocks, distinct: counts.len() as u64, codebook_zstd: compress_level(&codebook, LEVEL_19, false).len() as u64, id_bits }
}

fn palette_hashes(planes: &Planes) -> Vec<u64> {
    planes
        .palettes
        .iter()
        .map(|colours| {
            let mut hasher = DefaultHasher::new();
            colours.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

struct BodySizes {
    client: Client,
    group: Group,
    pixels: Vec<u64>,
    palettes_of_stream: Vec<Vec<usize>>,
    overhead: Vec<ZstdOverhead>,
    index: HashMap<IndexForm, FormSizes>,
    coverage: HashMap<CoverageForm, FormSizes>,
    /// Which side stream the row spans picked, and its bytes alone in Zstd 19.
    side_modes: Vec<u8>,
    side_bytes: Vec<u64>,
    patches: PatchBound,
    palettes_original: Vec<u64>,
    palettes_luma: Vec<u64>,
    palettes_master: Vec<u64>,
}

fn body_sizes(planes: &Planes, group: Group) -> BodySizes {
    let luma = reordered(planes, Order::Luma);
    let master = reordered(planes, Order::Master);
    let streams = StreamPlanes::of(planes, Some(&luma), Some(&master), Unit::ActionDirection);
    let dictionaries = dictionaries_of(&streams);
    let mut atoms = Atoms::default();
    let mut index = HashMap::new();
    for form in size_index_forms(planes.client) {
        let coded = coded_of(&scheme("sizes", form, coverage_with(form)), &streams, &dictionaries, &mut atoms);
        let of_stream = |pick: fn(&IndexStream) -> u64| coded.index.streams.iter().map(pick).collect();
        index.insert(form, FormSizes { streams: of_stream(IndexStream::stored), extra: of_stream(|s| s.extra.len() as u64), shared: coded.index.shared() });
    }
    let overhead = atoms.index_of(IndexForm::Plain, &streams, &dictionaries).streams.iter().map(|s| zstd_overhead(&s.data)).collect();
    let side_modes = atoms.index_of(SPANS_ZSTD, &streams, &dictionaries).streams.iter().map(|s| s.side_mode).collect();
    let side_bytes = (0..streams.streams()).map(|n| squeeze(false, &best_side(&streams.rows[n], &streams.layout[n], false)).len() as u64).collect();
    let mut coverage = HashMap::new();
    if planes.client == Client::Ec {
        for form in SIZE_COVERAGE_FORMS {
            let coded = coded_of(&scheme("sizes", IndexForm::Plain, form), &streams, &dictionaries, &mut atoms);
            coverage.insert(form, FormSizes { streams: coded.coverage.streams.iter().map(CoverageStream::stored).collect(), extra: Vec::new(), shared: coded.coverage.tables.len() as u64 });
        }
    }
    BodySizes {
        client: planes.client,
        group,
        pixels: streams.layout.iter().map(|l| l.pixels as u64).collect(),
        palettes_of_stream: streams.layout.iter().map(|l| l.palettes.clone()).collect(),
        overhead,
        index,
        coverage,
        side_modes,
        side_bytes,
        patches: patch_bound(planes),
        palettes_original: palette_hashes(planes),
        palettes_luma: palette_hashes(&luma),
        palettes_master: palette_hashes(&master),
    }
}

#[derive(Default, Clone, Copy)]
struct BucketRow {
    streams: u64,
    pixels: u64,
    plain: u64,
    lz4: u64,
    left: u64,
    overhead: ZstdOverhead,
}

fn print_buckets(of: &[&BodySizes]) {
    println!("| kind | stream size | streams | Mpx | zstd 19 MB | LZ4 HC MB | luma, left, zstd 19 MB | left / zstd 19 | LZ4 / zstd 19 | magic + frame headers MB | block headers MB | Huffman tables MB | FSE tables MB | all of those, share of the zstd index bytes |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut rows = [[BucketRow::default(); BUCKETS]; GROUPS.len()];
    for body in of {
        for stream in 0..body.pixels.len() {
            let row = &mut rows[body.group.at()][bucket_of(body.pixels[stream])];
            let coverage = |form: CoverageForm| body.coverage.get(&form).map_or(0, |sizes| sizes.streams[stream]);
            row.streams += 1;
            row.pixels += body.pixels[stream];
            row.plain += body.index[&IndexForm::Plain].streams[stream] + coverage(CoverageForm::Zstd);
            row.lz4 += body.index[&IndexForm::Lz4].streams[stream] + coverage(CoverageForm::Lz4);
            row.left += body.index[&IndexForm::Left].streams[stream] + coverage(CoverageForm::Zstd);
            let cost = body.overhead[stream];
            row.overhead.frame += cost.frame;
            row.overhead.block_headers += cost.block_headers;
            row.overhead.literal_tables += cost.literal_tables;
            row.overhead.sequence_tables += cost.sequence_tables;
        }
    }
    let line = |kind: &str, name: &str, row: &BucketRow, index_only: u64| {
        let cost = row.overhead;
        let spent = cost.frame + cost.block_headers + cost.literal_tables + cost.sequence_tables;
        println!(
            "| {kind} | {name} | {} | {:.1} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} % |",
            row.streams,
            row.pixels as f64 / PIXELS_PER_MEGAPIXEL,
            mb(row.plain),
            mb(row.lz4),
            mb(row.left),
            row.left as f64 / row.plain.max(1) as f64,
            row.lz4 as f64 / row.plain.max(1) as f64,
            mb(cost.frame),
            mb(cost.block_headers),
            mb(cost.literal_tables),
            mb(cost.sequence_tables),
            spent as f64 * PERCENT / index_only.max(1) as f64,
        );
    };
    let index_bytes = |group: Option<Group>, bucket: Option<usize>| -> u64 {
        of.iter()
            .filter(|b| group.is_none_or(|g| b.group == g))
            .map(|b| (0..b.pixels.len()).filter(|&s| bucket.is_none_or(|n| bucket_of(b.pixels[s]) == n)).map(|s| b.index[&IndexForm::Plain].streams[s]).sum::<u64>())
            .sum()
    };
    let add = |total: &mut BucketRow, row: &BucketRow| {
        total.streams += row.streams;
        total.pixels += row.pixels;
        total.plain += row.plain;
        total.lz4 += row.lz4;
        total.left += row.left;
        total.overhead.frame += row.overhead.frame;
        total.overhead.block_headers += row.overhead.block_headers;
        total.overhead.literal_tables += row.overhead.literal_tables;
        total.overhead.sequence_tables += row.overhead.sequence_tables;
    };
    let mut all = BucketRow::default();
    for group in GROUPS {
        let mut total = BucketRow::default();
        for bucket in 0..BUCKETS {
            let row = &rows[group.at()][bucket];
            if row.streams > 0 {
                line(group.label(), BUCKET_NAMES[bucket], row, index_bytes(Some(group), Some(bucket)));
            }
            add(&mut total, row);
        }
        if total.streams > 0 {
            line(group.label(), "all sizes", &total, index_bytes(Some(group), None));
        }
        add(&mut all, &total);
    }
    for bucket in 0..BUCKETS {
        let mut total = BucketRow::default();
        GROUPS.iter().for_each(|group| add(&mut total, &rows[group.at()][bucket]));
        line("everything", BUCKET_NAMES[bucket], &total, index_bytes(None, Some(bucket)));
    }
    line("everything", "all sizes", &all, index_bytes(None, None));
}

/// A row by kind of body: the stored bytes of each stream chosen by `of_stream`, plus each body's `shared`.
fn group_row(of: &[&BodySizes], name: &str, baseline: u64, of_stream: impl Fn(&BodySizes, usize) -> u64, shared: impl Fn(&BodySizes) -> u64) -> u64 {
    let mut groups = [0u64; GROUPS.len()];
    for body in of {
        groups[body.group.at()] += shared(body) + (0..body.pixels.len()).map(|s| of_stream(body, s)).sum::<u64>();
    }
    let total: u64 = groups.iter().sum();
    let gain = |value: u64, base: u64| if base == 0 { 0.0 } else { (value as f64 / base as f64 - 1.0) * PERCENT };
    println!("| {name} | {:.2} | {:.2} | {:.2} | {:.2} | {:+.1} % |", mb(groups[0]), mb(groups[1]), mb(groups[2]), mb(total), gain(total, baseline));
    total
}

const ROW_HEADER: &str = "| coder | equipment MB | creatures MB | not listed MB | total MB | against the first row |";
const ROW_RULE: &str = "|---|---:|---:|---:|---:|---:|";

fn print_rows(client: Client, of: &[&BodySizes]) {
    let stream = |form: IndexForm, body: &BodySizes, s: usize| body.index[&form].streams[s];
    let shared = |form: IndexForm, body: &BodySizes| body.index[&form].shared;
    let plain_total: u64 = of.iter().map(|b| b.index[&IndexForm::Plain].streams.iter().sum::<u64>()).sum();

    println!("\n#### {}: t3, the per-stream cost: index plane only, dictionaries counted\n\n{ROW_HEADER}\n{ROW_RULE}", client.label());
    group_row(of, "zstd 19 per stream", plain_total, |b, s| stream(IndexForm::Plain, b, s), |_| 0);
    group_row(of, "no size or checksum, magic not counted", plain_total, |b, s| stream(IndexForm::Bare, b, s), |_| 0);
    for size in DICT_SIZES {
        group_row(of, &format!("own dictionary, {} KiB", size / KIB), plain_total, |b, s| stream(IndexForm::Dict(size), b, s), |b| shared(IndexForm::Dict(size), b));
    }
    let best_shared = |b: &BodySizes| -> (u64, IndexForm) {
        let total = |form: IndexForm| b.index[&form].streams.iter().sum::<u64>() + b.index[&form].shared;
        let mut best = IndexForm::Bare;
        for size in DICT_SIZES {
            if total(IndexForm::Dict(size)) < total(best) {
                best = IndexForm::Dict(size);
            }
        }
        (total(best), best)
    };
    let mut chosen = [0u64; DICT_SIZES.len() + 1];
    let mut dictionary_bytes = 0u64;
    for body in of {
        match best_shared(body).1 {
            IndexForm::Dict(size) => {
                chosen[DICT_SIZES.iter().position(|&d| d == size).unwrap() + 1] += 1;
                dictionary_bytes += shared(IndexForm::Dict(size), body);
            }
            _ => chosen[0] += 1,
        }
    }
    group_row(of, "best of those per body, one byte per body", plain_total, |_, _| 0, |b| best_shared(b).0 + DICT_CHOICE_BYTES);
    println!("\nBest per body: none {} bodies; by dictionary size ({DICT_SIZES:?} bytes) {:?}; {:.2} MB of dictionaries.", chosen[0], &chosen[1..], mb(dictionary_bytes));

    println!("\n#### {}: t5, the per-stream choice: index plane and Classic's shape, id counted\n\n{ROW_HEADER}\n{ROW_RULE}", client.label());
    group_row(of, "zstd 19 per stream", plain_total, |b, s| stream(IndexForm::Plain, b, s), |_| 0);
    group_row(of, "luma, left, zstd 19", plain_total, |b, s| stream(IndexForm::Left, b, s), |_| 0);
    group_row(of, "luma, up, zstd 19", plain_total, |b, s| stream(IndexForm::Up, b, s), |_| 0);
    let left_or_plain = |b: &BodySizes, s: usize| stream(IndexForm::Plain, b, s).min(stream(IndexForm::Left, b, s));
    let any = |b: &BodySizes, s: usize| left_or_plain(b, s).min(stream(IndexForm::Up, b, s));
    let ids = |b: &BodySizes| b.pixels.len() as u64 * STREAM_CHOICE_BYTES;
    group_row(of, "plain or left, per stream", plain_total, left_or_plain, ids);
    group_row(of, "plain, left or up, per stream", plain_total, any, ids);
    println!("\n| kind | stream size | streams | picking plain | picking left | their share of the plain bytes |\n|---|---|---:|---:|---:|---:|");
    for group in GROUPS {
        for bucket in 0..=BUCKETS {
            let (mut streams, mut plains, mut lefts, mut left_bytes, mut plain_bytes) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for body in of.iter().filter(|b| b.group == group) {
                for s in (0..body.pixels.len()).filter(|&s| bucket == BUCKETS || bucket_of(body.pixels[s]) == bucket) {
                    let plain = stream(IndexForm::Plain, body, s);
                    streams += 1;
                    plain_bytes += plain;
                    if stream(IndexForm::Left, body, s) < plain {
                        lefts += 1;
                        left_bytes += plain;
                    } else {
                        plains += 1;
                    }
                }
            }
            if streams > 0 {
                let name = if bucket == BUCKETS { "all sizes" } else { BUCKET_NAMES[bucket] };
                println!("| {} | {name} | {streams} | {plains} | {lefts} | {:.1} % |", group.label(), left_bytes as f64 * PERCENT / plain_bytes.max(1) as f64);
            }
        }
    }
    // Palettes: original where only plain streams use one, sorted where a residual stream does.
    let entry_bytes = match client {
        Client::Cc => CC_PALETTE_ENTRY_BYTES,
        Client::Ec => EC_PALETTE_ENTRY_BYTES,
    };
    let table = |set: &HashSet<u64>| mb((set.len() * PALETTE_COLOURS * entry_bytes) as u64);
    let collect = |of_body: &dyn Fn(&BodySizes) -> Vec<u64>| -> HashSet<u64> { of.iter().flat_map(|b| of_body(b)).collect() };
    let original = collect(&|b| b.palettes_original.clone());
    let sorted = collect(&|b| b.palettes_luma.clone());
    let master_set = collect(&|b| b.palettes_master.clone());
    let mixed = collect(&|b| {
        let mut wanted = Vec::new();
        for s in 0..b.pixels.len() {
            let left = stream(IndexForm::Left, b, s) < stream(IndexForm::Plain, b, s);
            wanted.extend(b.palettes_of_stream[s].iter().map(|&p| if left { b.palettes_luma[p] } else { b.palettes_original[p] }));
        }
        wanted
    });
    println!(
        "\nDistinct palettes in the sample: original {:.2} MB, sorted {:.2} MB, master {:.2} MB, the choice's mix (each stream's palette in its own order) {:.2} MB.",
        table(&original),
        table(&sorted),
        table(&master_set),
        table(&mixed)
    );

    if client == Client::Ec {
        println!("\n#### {}: t6, the coverage plane: tables per body counted\n\n{ROW_HEADER}\n{ROW_RULE}", client.label());
        let base: u64 = of.iter().map(|b| b.coverage[&CoverageForm::Zstd].streams.iter().sum::<u64>()).sum();
        let named: [(&str, CoverageForm); 10] = [
            ("zstd 19", CoverageForm::Zstd),
            ("LZ4 HC", CoverageForm::Lz4),
            ("runs + partial values, tANS", CoverageForm::Runs),
            ("runs with copy-up + partial values, tANS", CoverageForm::RunsUp),
            ("runs zstd 19, partial values tANS", CoverageForm::RunsZstd),
            ("runs with copy-up zstd 19, partial values tANS", CoverageForm::RunsUpZstd),
            ("every pixel in tANS by neighbour context", CoverageForm::Contexts),
            ("runs zstd 19, partial values in tANS by neighbour context", CoverageForm::RunsContexts),
            ("row spans with their rows, zstd 19", COVERAGE_SPANS_ZSTD),
            ("row spans with their rows, LZ4 HC", COVERAGE_SPANS_LZ4),
        ];
        for (name, form) in named {
            group_row(of, name, base, |b, s| b.coverage[&form].streams[s], |b| b.coverage[&form].shared);
        }
    }

    println!("\n#### {}: t7, the rows to close: index plane and Classic's shape\n\n{ROW_HEADER}\n{ROW_RULE}", client.label());
    group_row(of, "zstd 19 per stream", plain_total, |b, s| stream(IndexForm::Plain, b, s), |_| 0);
    group_row(of, "(a) scanline runs, values and lengths in tANS, tables per body", plain_total, |b, s| stream(IndexForm::Rle, b, s), |b| shared(IndexForm::Rle, b));
    group_row(of, "(b) bound: own 4 x 4 patches in zstd 19, ids at their entropy, no table", plain_total, |_, _| 0, |b| b.patches.codebook_zstd + (b.patches.id_bits / f64::from(u8::BITS)) as u64);
    group_row(of, "(c) master palette, XOR previous, zstd 19", plain_total, |b, s| b.index[&IndexForm::Xor].streams[s], |_| 0);
    let blocks: u64 = of.iter().map(|b| b.patches.blocks).sum();
    let distinct: u64 = of.iter().map(|b| b.patches.distinct).sum();
    println!("\nPatches: {blocks} blocks, {distinct} distinct within their bodies ({:.1} %).", distinct as f64 * PERCENT / blocks.max(1) as f64);
}

/// A column of the row span tables: the bytes of an index form and a coverage form, against an earlier column.
#[derive(Clone, Copy)]
struct Column {
    name: &'static str,
    base: usize,
    index: IndexForm,
    coverage: CoverageForm,
}

impl Column {
    fn stream(&self, body: &BodySizes, s: usize) -> u64 {
        body.index[&self.index].streams[s] + body.coverage.get(&self.coverage).map_or(0, |sizes| sizes.streams[s])
    }

    fn shared(&self, body: &BodySizes) -> u64 {
        body.index[&self.index].shared + body.coverage.get(&self.coverage).map_or(0, |sizes| sizes.shared)
    }
}

const fn column(name: &'static str, base: usize, index: IndexForm, coverage: CoverageForm) -> Column {
    Column { name, base, index, coverage }
}

fn print_columns_by_bucket(of: &[&BodySizes], columns: &[Column]) {
    let names: Vec<&str> = columns.iter().map(|c| c.name).collect();
    println!("| kind | stream size | streams | Mpx | {} |", names.join(" | "));
    println!("|---|---|---:|---:|{}", "---:|".repeat(columns.len()));
    let line = |kind: &str, name: &str, group: Option<Group>, bucket: Option<usize>| {
        let (mut streams, mut pixels) = (0u64, 0u64);
        let mut bytes = vec![0u64; columns.len()];
        for body in of.iter().filter(|b| group.is_none_or(|g| b.group == g)) {
            for s in (0..body.pixels.len()).filter(|&s| bucket.is_none_or(|n| bucket_of(body.pixels[s]) == n)) {
                streams += 1;
                pixels += body.pixels[s];
                bytes.iter_mut().zip(columns).for_each(|(sum, c)| *sum += c.stream(body, s));
            }
        }
        if streams == 0 {
            return;
        }
        let cells: Vec<String> = columns.iter().enumerate().map(|(n, c)| if c.base == n { format!("{:.3}", mb(bytes[n])) } else { format!("{:.3} ({:.3})", mb(bytes[n]), bytes[n] as f64 / bytes[c.base].max(1) as f64) }).collect();
        println!("| {kind} | {name} | {streams} | {:.1} | {} |", pixels as f64 / PIXELS_PER_MEGAPIXEL, cells.join(" | "));
    };
    for group in GROUPS {
        for bucket in 0..BUCKETS {
            line(group.label(), BUCKET_NAMES[bucket], Some(group), Some(bucket));
        }
        line(group.label(), "all sizes", Some(group), None);
    }
    for bucket in 0..BUCKETS {
        line("everything", BUCKET_NAMES[bucket], None, Some(bucket));
    }
    line("everything", "all sizes", None, None);
}

fn print_spans(client: Client, of: &[&BodySizes]) {
    let cc = client == Client::Cc;
    let (plain_cov, lz4_cov) = (CoverageForm::Zstd, CoverageForm::Lz4);
    let total_of = |form: IndexForm| of.iter().map(|b| b.index[&form].streams.iter().sum::<u64>()).sum::<u64>();
    let smallest = |forms: &[IndexForm]| *forms.iter().min_by_key(|&&form| total_of(form)).expect("a form");
    let painted_zstd = if cc { smallest(&CC_PAINTED[..3]) } else { PAINTED_EC_ZSTD };
    let painted_lz4 = if cc { smallest(&CC_PAINTED[3..]) } else { PAINTED_EC_LZ4 };
    let mut columns = vec![column("zstd 19 rectangle MB", 0, IndexForm::Plain, plain_cov), column("a row spans (over rectangle)", 0, SPANS_ZSTD, plain_cov), column("b painted (over rectangle)", 0, painted_zstd, plain_cov)];
    if !cc {
        columns.push(column("c both planes in spans (over rectangle)", 0, SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD));
        columns.push(column("b, coverage in spans (over rectangle)", 0, painted_zstd, COVERAGE_SPANS_ZSTD));
    }
    let lz4_at = columns.len();
    columns.push(column("LZ4 HC rectangle MB", lz4_at, IndexForm::Lz4, lz4_cov));
    columns.push(column("a row spans (over rectangle)", lz4_at, SPANS_LZ4, lz4_cov));
    columns.push(column("b painted (over rectangle)", lz4_at, painted_lz4, lz4_cov));
    if !cc {
        columns.push(column("c both planes in spans (over rectangle)", lz4_at, SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4));
        columns.push(column("b, coverage in spans (over rectangle)", lz4_at, painted_lz4, COVERAGE_SPANS_LZ4));
    }
    println!("\n#### {}: t8, row spans and painted pixels, by kind and stream size; b here is {painted_zstd:?} and {painted_lz4:?}; coverage counted\n", client.label());
    print_columns_by_bucket(of, &columns);

    println!("\n#### {}: t8, side stream of the row spans in zstd 19\n\n| kind | streams | alone, zstd 19 MB | {} |\n|---|---:|---:|---:|---:|---:|", client.label(), SIDE_MODE_NAMES.join(" | "));
    for group in GROUPS {
        let chosen: Vec<&&BodySizes> = of.iter().filter(|b| b.group == group).collect();
        let streams: usize = chosen.iter().map(|b| b.pixels.len()).sum();
        if streams == 0 {
            continue;
        }
        let bytes: u64 = chosen.iter().map(|b| b.side_bytes.iter().sum::<u64>()).sum();
        let picks: Vec<String> = SIDE_MODES.iter().map(|&mode| chosen.iter().map(|b| b.side_modes.iter().filter(|&&m| m == mode).count()).sum::<usize>().to_string()).collect();
        println!("| {} | {streams} | {:.3} | {} |", group.label(), mb(bytes), picks.join(" | "));
    }

    println!("\n#### {}: t8, every combination, whole streams with coverage and tables\n\n{ROW_HEADER}\n{ROW_RULE}", client.label());
    let rows: Vec<Column> = if cc {
        vec![
            column("zstd 19 rectangle", 0, IndexForm::Plain, plain_cov),
            column("a row spans, zstd 19", 0, SPANS_ZSTD, plain_cov),
            column("b painted, runs in the same frame, zstd 19", 0, CC_PAINTED[0], plain_cov),
            column("b painted, runs in a frame of their own, zstd 19", 0, CC_PAINTED[1], plain_cov),
            column("b painted, bits in a frame of their own, zstd 19", 0, CC_PAINTED[2], plain_cov),
            column("LZ4 HC rectangle", 0, IndexForm::Lz4, plain_cov),
            column("a row spans, LZ4 HC", 0, SPANS_LZ4, plain_cov),
            column("b painted, runs in the same frame, LZ4 HC", 0, CC_PAINTED[3], plain_cov),
            column("b painted, runs in a frame of their own, LZ4 HC", 0, CC_PAINTED[4], plain_cov),
        ]
    } else {
        vec![
            column("zstd 19 rectangle, both planes", 0, IndexForm::Plain, plain_cov),
            column("a index in row spans, zstd 19", 0, SPANS_ZSTD, plain_cov),
            column("b index painted only, zstd 19", 0, PAINTED_EC_ZSTD, plain_cov),
            column("c both planes in row spans, zstd 19", 0, SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD),
            column("b, coverage in row spans, zstd 19", 0, PAINTED_EC_ZSTD, COVERAGE_SPANS_ZSTD),
            column("luma, left, coverage zstd 19", 0, IndexForm::Left, plain_cov),
            column("luma, left, coverage in row spans", 0, IndexForm::Left, COVERAGE_SPANS_ZSTD),
            column("luma, left, coverage runs zstd 19, partial tANS", 0, IndexForm::Left, CoverageForm::RunsZstd),
            column("luma, left, coverage runs zstd 19, partial tANS by context", 0, IndexForm::Left, CoverageForm::RunsContexts),
            column("luma, left, coverage every pixel tANS by context", 0, IndexForm::Left, CoverageForm::Contexts),
            column("b, coverage runs zstd 19, partial tANS by context", 0, PAINTED_EC_ZSTD, CoverageForm::RunsContexts),
            column("b, coverage every pixel tANS by context", 0, PAINTED_EC_ZSTD, CoverageForm::Contexts),
            column("zstd 19, coverage runs zstd 19, partial tANS by context", 0, IndexForm::Plain, CoverageForm::RunsContexts),
            column("zstd 19, coverage every pixel tANS by context", 0, IndexForm::Plain, CoverageForm::Contexts),
            column("LZ4 HC rectangle, both planes", 0, IndexForm::Lz4, lz4_cov),
            column("a index in row spans, LZ4 HC", 0, SPANS_LZ4, lz4_cov),
            column("b index painted only, LZ4 HC", 0, PAINTED_EC_LZ4, lz4_cov),
            column("c both planes in row spans, LZ4 HC", 0, SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4),
            column("b, coverage in row spans, LZ4 HC", 0, PAINTED_EC_LZ4, COVERAGE_SPANS_LZ4),
        ]
    };
    let base: u64 = of.iter().map(|b| rows[0].shared(b) + (0..b.pixels.len()).map(|s| rows[0].stream(b, s)).sum::<u64>()).sum();
    for row in &rows {
        group_row(of, row.name, base, |b, s| row.stream(b, s), |b| row.shared(b));
    }
    // The choices, each with its id per stream.
    let ids = |b: &BodySizes| b.pixels.len() as u64 * STREAM_CHOICE_BYTES;
    let rectangle = |b: &BodySizes, s: usize| rows[0].stream(b, s);
    let spans = |b: &BodySizes, s: usize| Column { index: SPANS_ZSTD, ..rows[0] }.stream(b, s);
    let painted = |b: &BodySizes, s: usize| Column { index: painted_zstd, ..rows[0] }.stream(b, s);
    group_row(of, "rectangle, a or b per stream, zstd 19", base, |b, s| rectangle(b, s).min(spans(b, s)).min(painted(b, s)), ids);
    let lz4_rows = |index: IndexForm, b: &BodySizes, s: usize| column("", 0, index, lz4_cov).stream(b, s);
    group_row(of, "rectangle, a or b per stream, LZ4 HC", base, |b, s| lz4_rows(IndexForm::Lz4, b, s).min(lz4_rows(SPANS_LZ4, b, s)).min(lz4_rows(painted_lz4, b, s)), ids);
    if cc {
        let apart = CC_PAINTED[1];
        group_row(of, "luma, left, shape as runs in a frame of their own: sizes only", base, |b, s| b.index[&IndexForm::Left].streams[s] - b.index[&IndexForm::Left].extra[s] + b.index[&apart].extra[s], |_| 0);
    } else {
        let left = |b: &BodySizes, s: usize| Column { index: IndexForm::Left, ..rows[0] }.stream(b, s);
        group_row(of, "rectangle, b or left per stream, zstd 19", base, |b, s| rectangle(b, s).min(painted(b, s)).min(left(b, s)), ids);
    }
    for size in DICT_SIZES {
        let form = IndexForm::SpansDict(size);
        group_row(of, &format!("a row spans, own dictionary of those, {} KiB", size / KIB), base, |b, s| column("", 0, form, plain_cov).stream(b, s), |b| b.index[&form].shared);
    }
    let best = |b: &BodySizes| -> u64 {
        let total = |form: IndexForm| b.index[&form].streams.iter().sum::<u64>() + b.index[&form].shared;
        DICT_SIZES.iter().map(|&size| total(IndexForm::SpansDict(size))).chain([total(SPANS_ZSTD)]).min().expect("a form") + DICT_CHOICE_BYTES
    };
    let coverage_bytes = |b: &BodySizes| b.coverage.get(&plain_cov).map_or(0, |sizes| sizes.streams.iter().sum::<u64>());
    group_row(of, "a row spans, best dictionary or none per body", base, |_, _| 0, |b| best(b) + coverage_bytes(b));
}

pub(super) fn part_t1(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    println!("\n### t1: sizes, every {stride}-th body by pixel count, per (action, direction); CPU {}; every coder round-trips", cpu_state());
    for client in [Client::Cc, Client::Ec] {
        let started = Instant::now();
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let rows: Vec<BodySizes> = sample
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                body_sizes(&planes, kinds.group(&bodies[i]))
            })
            .collect();
        assert!(rows.iter().all(|r| r.client == client));
        let of: Vec<&BodySizes> = rows.iter().collect();
        println!("\n### {}: {} bodies, encoded and verified in {:.0} s\n\n#### t1: by stream size and kind; shape and coverage counted\n", client.label(), of.len(), started.elapsed().as_secs_f64());
        print_buckets(&of);
        print_rows(client, &of);
        print_spans(client, &of);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// t0: the whole set: kinds and duplicates

type StreamHash = (u64, u64);

struct WholeStream {
    hash: StreamHash,
    luma_hash: StreamHash,
    pixels: u64,
    frames: u32,
    action: u16,
}

struct WholeBody {
    body: usize,
    source: (u8, usize),
    streams: Vec<WholeStream>,
}

const HASH_SEEDS: (u64, u64) = (0x2026_1104_0000_0001, 0x2026_1104_0000_0002);
const SOURCE_NAMES: [&str; 3] = ["Classic MUL", "Classic UOP", "Enhanced"];

fn source_of_body(source: &BodySource) -> (u8, usize) {
    match source {
        BodySource::CcMul { file, .. } => (0, *file),
        BodySource::CcUop { file, .. } => (1, *file),
        BodySource::Ec { .. } => (2, 0),
    }
}

fn stream_hash(sizes: &[(usize, usize)], index: &[u8], mask: &[u8], client: Client) -> StreamHash {
    let one = |seed: u64| {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        sizes.hash(&mut hasher);
        index.hash(&mut hasher);
        if client == Client::Ec {
            mask.hash(&mut hasher);
        }
        hasher.finish()
    };
    (one(HASH_SEEDS.0), one(HASH_SEEDS.1))
}

const DUPLICATE_SCOPES: [&str; 3] = ["within a body", "across bodies of one file", "across files"];

#[derive(Default, Clone, Copy)]
struct KindCensus {
    bodies: u64,
    pairs: u64,
    streams: u64,
    frames: u64,
    pixels: u64,
}

#[derive(Default, Clone, Copy)]
struct ActionCensus {
    bodies: u64,
    pixels: u64,
    frames: u64,
}

fn print_census(rows: &[WholeBody], bodies: &[BodySource], kinds: &BodyKinds) {
    let mut by_kind = BTreeMap::<(u8, String), KindCensus>::new();
    for row in rows {
        let kind = by_kind.entry((row.source.0, kinds.kind(&bodies[row.body]).to_string())).or_default();
        let actions: BTreeSet<u16> = row.streams.iter().map(|s| s.action).collect();
        kind.bodies += 1;
        kind.pairs += actions.len() as u64;
        kind.streams += row.streams.len() as u64;
        kind.frames += row.streams.iter().map(|s| u64::from(s.frames)).sum::<u64>();
        kind.pixels += row.streams.iter().map(|s| s.pixels).sum::<u64>();
    }
    println!("| source | kind | bodies | (body, action) pairs | streams | frames | share of frames | Mpx | share of pixels | Mpx per pair |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for ((source, name), k) in &by_kind {
        let of_source = |value: fn(&KindCensus) -> u64| -> u64 { by_kind.iter().filter(|(key, _)| key.0 == *source).map(|(_, k)| value(k)).sum() };
        println!(
            "| {} | {name} | {} | {} | {} | {} | {:.1} % | {:.1} | {:.1} % | {:.3} |",
            SOURCE_NAMES[usize::from(*source)],
            k.bodies,
            k.pairs,
            k.streams,
            k.frames,
            k.frames as f64 * PERCENT / of_source(|k| k.frames).max(1) as f64,
            k.pixels as f64 / PIXELS_PER_MEGAPIXEL,
            k.pixels as f64 * PERCENT / of_source(|k| k.pixels).max(1) as f64,
            k.pixels as f64 / PIXELS_PER_MEGAPIXEL / k.pairs.max(1) as f64,
        );
    }
    // Which action a town stands in: how many bodies of each kind hold it.
    let mut actions = BTreeMap::<u16, [ActionCensus; 2]>::new();
    for row in rows {
        let slot = match kinds.kind(&bodies[row.body]) {
            knowledge::KIND_HUMAN => 0,
            knowledge::KIND_EQUIPMENT => 1,
            _ => continue,
        };
        let mut of_body = BTreeMap::<u16, (u64, u64)>::new();
        for stream in &row.streams {
            let entry = of_body.entry(stream.action).or_default();
            entry.0 += stream.pixels;
            entry.1 += u64::from(stream.frames);
        }
        for (action, (pixels, frames)) in of_body {
            let entry = &mut actions.entry(action).or_default()[slot];
            entry.bodies += 1;
            entry.pixels += pixels;
            entry.frames += frames;
        }
    }
    println!("\n| action | human bodies | human Mpx per body | human frames per body | equipment bodies | equipment Mpx per body | equipment frames per body |\n|---:|---:|---:|---:|---:|---:|---:|");
    for (action, [human, equipment]) in &actions {
        let per = |value: u64, of: &ActionCensus| value as f64 / of.bodies.max(1) as f64;
        println!(
            "| {action} | {} | {:.3} | {:.1} | {} | {:.3} | {:.1} |",
            human.bodies,
            per(human.pixels, human) / PIXELS_PER_MEGAPIXEL,
            per(human.frames, human),
            equipment.bodies,
            per(equipment.pixels, equipment) / PIXELS_PER_MEGAPIXEL,
            per(equipment.frames, equipment),
        );
    }
}

/// Stream numbers of each body that repeat an earlier stream, in body order, with the scope of the repeat.
fn duplicates_of(rows: &[WholeBody], key: impl Fn(&WholeStream) -> StreamHash) -> HashMap<usize, Vec<(usize, usize)>> {
    let mut first = HashMap::<StreamHash, (usize, (u8, usize))>::new();
    let mut repeats = HashMap::<usize, Vec<(usize, usize)>>::new();
    for row in rows {
        for (n, stream) in row.streams.iter().enumerate() {
            match first.entry(key(stream)) {
                std::collections::hash_map::Entry::Vacant(place) => {
                    place.insert((row.body, row.source));
                }
                std::collections::hash_map::Entry::Occupied(place) => {
                    let (body, source) = *place.get();
                    let scope = if body == row.body {
                        0
                    } else if source == row.source {
                        1
                    } else {
                        2
                    };
                    repeats.entry(row.body).or_default().push((n, scope));
                }
            }
        }
    }
    repeats
}

#[derive(Clone, Copy)]
struct Repeat {
    scope: usize,
    group: Group,
    pixels: u64,
    index: u64,
    coverage: u64,
    /// The same stream in row spans, painted only, and its coverage in row spans.
    spans: u64,
    painted: u64,
    coverage_spans: u64,
}

fn print_duplicates(title: &str, measured: &[Repeat], streams: usize) {
    println!("\n#### {title}\n");
    println!("| kind | scope | repeated streams | Mpx | index saved MB | coverage saved MB | total saved MB | index in row spans MB | index painted MB | coverage in row spans MB |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    let (mut count, mut pixels, mut index, mut coverage) = (0usize, 0u64, 0u64, 0u64);
    let mut layouts = [0u64; 3];
    for group in GROUPS {
        for (scope, name) in DUPLICATE_SCOPES.iter().enumerate() {
            let picked: Vec<&Repeat> = measured.iter().filter(|m| m.scope == scope && m.group == group).collect();
            if picked.is_empty() {
                continue;
            }
            let (p, i, c): (u64, u64, u64) = (picked.iter().map(|m| m.pixels).sum(), picked.iter().map(|m| m.index).sum(), picked.iter().map(|m| m.coverage).sum());
            let of_layout = |pick: fn(&Repeat) -> u64| picked.iter().map(|m| pick(m)).sum::<u64>();
            let here = [of_layout(|m| m.spans), of_layout(|m| m.painted), of_layout(|m| m.coverage_spans)];
            println!("| {} | {name} | {} | {:.2} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |", group.label(), picked.len(), p as f64 / PIXELS_PER_MEGAPIXEL, mb(i), mb(c), mb(i + c), mb(here[0]), mb(here[1]), mb(here[2]));
            layouts.iter_mut().zip(here).for_each(|(sum, value)| *sum += value);
            count += picked.len();
            pixels += p;
            index += i;
            coverage += c;
        }
    }
    println!("| all | all | {count} of {streams} | {:.2} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |", pixels as f64 / PIXELS_PER_MEGAPIXEL, mb(index), mb(coverage), mb(index + coverage), mb(layouts[0]), mb(layouts[1]), mb(layouts[2]));
}

pub(super) fn part_t0(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path) {
    let kinds = BodyKinds::load(cc_dir);
    println!("\n### t0: kinds and exact duplicate streams over the whole set; Enhanced ships no mobtypes.txt, so its ids use Classic's table; CPU {}", cpu_state());
    for client in [Client::Cc, Client::Ec] {
        let started = Instant::now();
        let of: Vec<usize> = present.iter().filter(|s| s.0 == client).map(|s| s.2).collect();
        let rows: Vec<WholeBody> = of
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                let luma = reordered(&planes, Order::Luma);
                let streams = StreamPlanes::of(&planes, Some(&luma), None, Unit::ActionDirection);
                let whole = (0..streams.streams())
                    .map(|n| {
                        let layout = &streams.layout[n];
                        WholeStream {
                            hash: stream_hash(&layout.sizes, &streams.original[n], &streams.mask[n], client),
                            luma_hash: stream_hash(&layout.sizes, &streams.luma[n], &streams.mask[n], client),
                            pixels: layout.pixels as u64,
                            frames: layout.sizes.len() as u32,
                            action: layout.action,
                        }
                    })
                    .collect();
                WholeBody { body: i, source: source_of_body(&bodies[i]), streams: whole }
            })
            .collect();
        println!("\n### {}: {} bodies hashed in {:.0} s\n", client.label(), rows.len(), started.elapsed().as_secs_f64());
        print_census(&rows, bodies, &kinds);
        let (repeats, luma_repeats) = (duplicates_of(&rows, |s| s.hash), duplicates_of(&rows, |s| s.luma_hash));
        let with_repeats: Vec<usize> = repeats.keys().chain(luma_repeats.keys()).copied().collect::<BTreeSet<usize>>().into_iter().collect();
        // The second pass compresses only the repeats, for their real bytes.
        let measured: Vec<(Vec<Repeat>, Vec<Repeat>)> = with_repeats
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                let luma = reordered(&planes, Order::Luma);
                let streams = StreamPlanes::of(&planes, Some(&luma), None, Unit::ActionDirection);
                let group = kinds.group(&bodies[i]);
                let coverage_of = |n: usize| if client == Client::Ec { compress_level(&streams.mask[n], LEVEL_19, false).len() as u64 } else { 0 };
                let painted_shape = if client == Client::Cc { ShapeForm::RunsInFrame } else { ShapeForm::InCoverage };
                let repeat = |n: usize, scope: usize, index: u64| Repeat {
                    scope,
                    group,
                    pixels: streams.layout[n].pixels as u64,
                    index,
                    coverage: coverage_of(n),
                    spans: squeeze(false, &span_content(&streams.original[n], &streams.rows[n], &streams.layout[n], true, false)).len() as u64,
                    painted: painted_stream(&streams, n, painted_shape, false).stored(),
                    coverage_spans: if client == Client::Ec { squeeze(false, &span_content(&streams.mask[n], &streams.rows[n], &streams.layout[n], true, false)).len() as u64 } else { 0 },
                };
                let plain = repeats.get(&i).map_or(&[][..], Vec::as_slice).iter().map(|&(n, scope)| repeat(n, scope, compress_level(&streams.original[n], LEVEL_19, false).len() as u64)).collect();
                let left = luma_repeats.get(&i).map_or(&[][..], Vec::as_slice).iter().map(|&(n, scope)| repeat(n, scope, residual_stream(Coding::Left, Predictor::Left, client, &streams.layout[n], &streams.luma[n], &streams.mask[n]).stored())).collect();
                (plain, left)
            })
            .collect();
        let streams: usize = rows.iter().map(|r| r.streams.len()).sum();
        let (plain, left): (Vec<Vec<Repeat>>, Vec<Vec<Repeat>>) = measured.into_iter().unzip();
        print_duplicates(&format!("{}: exact duplicate (action, direction) streams on the original index plane, zstd 19, pointed at one blob", client.label()), &plain.concat(), streams);
        print_duplicates(&format!("{}: the same on the luma plane, with the left residual's bytes (and Classic's shape)", client.label()), &left.concat(), streams);
        println!("\n{} finished in {:.0} s", client.label(), started.elapsed().as_secs_f64());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// tt: single-thread decode of the sample

#[derive(Clone, Copy)]
struct Scheme {
    name: &'static str,
    index: IndexForm,
    coverage: CoverageForm,
    /// Decode to the rectangle the atlas needs, not only to spans a renderer could keep.
    expand: bool,
}

const fn scheme(name: &'static str, index: IndexForm, coverage: CoverageForm) -> Scheme {
    Scheme { name, index, coverage, expand: true }
}

const fn kept(name: &'static str, index: IndexForm, coverage: CoverageForm) -> Scheme {
    Scheme { name, index, coverage, expand: false }
}

const COMMON_SCHEMES: [Scheme; 21] = [
    scheme("zstd 19, a new context per stream", IndexForm::PlainFresh, CoverageForm::ZstdFresh),
    scheme("zstd 19, a context per thread", IndexForm::Plain, CoverageForm::Zstd),
    scheme("LZ4 HC", IndexForm::Lz4, CoverageForm::Lz4),
    scheme("zstd 19, flags off", IndexForm::Bare, CoverageForm::Zstd),
    scheme("own dictionary, 1 KiB", IndexForm::Dict(DICT_SIZES[0]), CoverageForm::Zstd),
    scheme("own dictionary, 16 KiB", IndexForm::Dict(DICT_SIZES[2]), CoverageForm::Zstd),
    scheme("own dictionary, 256 KiB", IndexForm::Dict(DICT_SIZES[4]), CoverageForm::Zstd),
    scheme("best dictionary per body", IndexForm::DictBest, CoverageForm::Zstd),
    scheme("luma, left, zstd 19, per-pixel predict", IndexForm::LeftPredict, CoverageForm::Zstd),
    scheme("luma, left, zstd 19, tight", IndexForm::Left, CoverageForm::Zstd),
    scheme("luma, up, zstd 19, tight", IndexForm::Up, CoverageForm::Zstd),
    scheme("plain or left per stream, tight", IndexForm::Choice { with_up: false }, CoverageForm::Zstd),
    scheme("plain, left or up per stream, tight", IndexForm::Choice { with_up: true }, CoverageForm::Zstd),
    scheme("(a) scanline runs, tANS", IndexForm::Rle, CoverageForm::Zstd),
    scheme("(c) master palette, XOR previous, zstd 19", IndexForm::Xor, CoverageForm::Zstd),
    scheme("zstd 19, a context per thread (again, for drift)", IndexForm::Plain, CoverageForm::Zstd),
    scheme("a row spans, zstd 19, expanded", SPANS_ZSTD, CoverageForm::Zstd),
    kept("a row spans, zstd 19, kept as spans", SPANS_ZSTD, CoverageForm::Zstd),
    scheme("a row spans, LZ4 HC, expanded", SPANS_LZ4, CoverageForm::Lz4),
    kept("a row spans, LZ4 HC, kept as spans", SPANS_LZ4, CoverageForm::Lz4),
    scheme("a row spans, best dictionary per body, expanded", IndexForm::SpansDictBest, CoverageForm::Zstd),
];

const CLASSIC_SCHEMES: [Scheme; 10] = [
    scheme("split boxes, zstd 19, expanded", BOXES_SIDE_ZSTD, CoverageForm::Zstd),
    kept("split boxes, zstd 19, kept as boxes", BOXES_SIDE_ZSTD, CoverageForm::Zstd),
    scheme("split boxes, LZ4 HC, expanded", BOXES_SIDE_LZ4, CoverageForm::Zstd),
    kept("split boxes, LZ4 HC, kept as boxes", BOXES_SIDE_LZ4, CoverageForm::Zstd),
    scheme("b painted, runs in the frame, zstd 19, expanded", CC_PAINTED[0], CoverageForm::Zstd),
    kept("b painted, runs in the frame, zstd 19, kept as spans", CC_PAINTED[0], CoverageForm::Zstd),
    scheme("b painted, runs apart, zstd 19, expanded", CC_PAINTED[1], CoverageForm::Zstd),
    scheme("b painted, bits apart, zstd 19, expanded", CC_PAINTED[2], CoverageForm::Zstd),
    scheme("b painted, runs in the frame, LZ4 HC, expanded", CC_PAINTED[3], CoverageForm::Zstd),
    kept("b painted, runs in the frame, LZ4 HC, kept as spans", CC_PAINTED[3], CoverageForm::Zstd),
];

const ENHANCED_SCHEMES: [Scheme; 33] = [
    scheme("split boxes, both planes, zstd 19, expanded", BOXES_ALONE_ZSTD, COVERAGE_BOXES_ZSTD),
    kept("split boxes, both planes, zstd 19, kept as boxes", BOXES_ALONE_ZSTD, COVERAGE_BOXES_ZSTD),
    scheme("split boxes, both planes, LZ4 HC, expanded", BOXES_ALONE_LZ4, COVERAGE_BOXES_LZ4),
    kept("split boxes, both planes, LZ4 HC, kept as boxes", BOXES_ALONE_LZ4, COVERAGE_BOXES_LZ4),
    scheme("zstd 19, coverage runs, tANS", IndexForm::Plain, CoverageForm::Runs),
    scheme("zstd 19, coverage runs with copy-up, tANS", IndexForm::Plain, CoverageForm::RunsUp),
    scheme("zstd 19, coverage runs zstd 19, partial tANS", IndexForm::Plain, CoverageForm::RunsZstd),
    scheme("zstd 19, coverage runs with copy-up zstd 19, partial tANS", IndexForm::Plain, CoverageForm::RunsUpZstd),
    scheme("zstd 19, coverage every pixel tANS by context", IndexForm::Plain, CoverageForm::Contexts),
    scheme("zstd 19, coverage runs zstd 19, partial tANS by context", IndexForm::Plain, CoverageForm::RunsContexts),
    scheme("luma, left, tight, coverage runs, tANS", IndexForm::Left, CoverageForm::Runs),
    scheme("luma, left, tight, coverage runs with copy-up, tANS", IndexForm::Left, CoverageForm::RunsUp),
    scheme("luma, left, tight, coverage runs zstd 19, partial tANS", IndexForm::Left, CoverageForm::RunsZstd),
    scheme("luma, left, tight, coverage runs with copy-up zstd 19, partial tANS", IndexForm::Left, CoverageForm::RunsUpZstd),
    scheme("luma, left, tight, coverage every pixel tANS by context", IndexForm::Left, CoverageForm::Contexts),
    scheme("luma, left, tight, coverage runs zstd 19, partial tANS by context", IndexForm::Left, CoverageForm::RunsContexts),
    scheme("luma, left, tight, coverage row spans", IndexForm::Left, COVERAGE_SPANS_ZSTD),
    scheme("b painted, zstd 19, expanded", PAINTED_EC_ZSTD, CoverageForm::Zstd),
    kept("b painted, zstd 19, kept as spans", PAINTED_EC_ZSTD, CoverageForm::Zstd),
    scheme("c both planes in row spans, zstd 19, expanded", SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD),
    kept("c both planes in row spans, zstd 19, kept as spans", SPANS_ALONE_ZSTD, COVERAGE_SPANS_ZSTD),
    scheme("b painted, coverage row spans, zstd 19, expanded", PAINTED_EC_ZSTD, COVERAGE_SPANS_ZSTD),
    kept("b painted, coverage row spans, zstd 19, kept as spans", PAINTED_EC_ZSTD, COVERAGE_SPANS_ZSTD),
    scheme("b painted, coverage runs zstd 19, partial tANS by context, expanded", PAINTED_EC_ZSTD, CoverageForm::RunsContexts),
    scheme("b painted, coverage every pixel tANS by context, expanded", PAINTED_EC_ZSTD, CoverageForm::Contexts),
    scheme("b painted, LZ4 HC, expanded", PAINTED_EC_LZ4, CoverageForm::Lz4),
    kept("b painted, LZ4 HC, kept as spans", PAINTED_EC_LZ4, CoverageForm::Lz4),
    scheme("c both planes in row spans, LZ4 HC, expanded", SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4),
    kept("c both planes in row spans, LZ4 HC, kept as spans", SPANS_ALONE_LZ4, COVERAGE_SPANS_LZ4),
    scheme("b painted, coverage row spans, LZ4 HC, expanded", PAINTED_EC_LZ4, COVERAGE_SPANS_LZ4),
    kept("b painted, coverage row spans, LZ4 HC, kept as spans", PAINTED_EC_LZ4, COVERAGE_SPANS_LZ4),
    scheme("a row spans, coverage runs zstd 19, partial tANS by context, expanded", SPANS_ZSTD, CoverageForm::RunsContexts),
    scheme("a row spans, coverage every pixel tANS by context, expanded", SPANS_ZSTD, CoverageForm::Contexts),
];

const TINY_ONLY_ENV: &str = "MOBILE_ANIM_CODEC_TINY_ONLY";
const TINY_ONLY_SEPARATOR: char = ';';

/// Every scheme of a client, or the ones named exactly by [`TINY_ONLY_ENV`].
fn chosen_schemes(client: Client) -> Vec<Scheme> {
    let mut all: Vec<Scheme> = COMMON_SCHEMES.to_vec();
    match client {
        Client::Cc => all.extend(CLASSIC_SCHEMES),
        Client::Ec => all.extend(ENHANCED_SCHEMES),
    }
    match std::env::var(TINY_ONLY_ENV) {
        Ok(names) => {
            let names: Vec<&str> = names.split(TINY_ONLY_SEPARATOR).map(str::trim).collect();
            all.into_iter().filter(|s| names.contains(&s.name)).collect()
        }
        Err(_) => all,
    }
}

fn coded_of(scheme: &Scheme, planes: &StreamPlanes, dictionaries: &Dictionaries, atoms: &mut Atoms) -> Coded {
    let index = atoms.index_of(scheme.index, planes, dictionaries);
    let coded = Coded { client: planes.client, index_form: scheme.index, coverage_form: scheme.coverage, index, coverage: atoms.coverage_of(scheme.coverage, planes), expand: scheme.expand };
    coded.verify(planes);
    coded
}

struct TimedRow {
    seconds: f64,
    by_bucket: [[f64; BUCKETS]; GROUPS.len()],
    bytes_by_bucket: [[u64; BUCKETS]; GROUPS.len()],
    stream_ms: Vec<f64>,
}

/// One thread: each body whole, the fastest of the timing runs, tables included; then each stream alone.
fn time_scheme(coded: &[Coded], planes: &[StreamPlanes], groups: &[Group]) -> TimedRow {
    let mut row = TimedRow { seconds: 0.0, by_bucket: [[0.0; BUCKETS]; GROUPS.len()], bytes_by_bucket: [[0; BUCKETS]; GROUPS.len()], stream_ms: Vec::new() };
    for ((coded, planes), group) in coded.iter().zip(planes).zip(groups) {
        let wanted: Vec<usize> = (0..planes.streams()).collect();
        row.seconds += fastest(|| coded.decode_all(&planes.layout, &wanted, None)).as_secs_f64();
        let mut tables = coded.tables(None);
        SCRATCH.with(|scratch| {
            let o = &mut *scratch.borrow_mut();
            for stream in 0..planes.streams() {
                let seconds = fastest(|| coded.decode(&planes.layout, stream, &mut tables, o, coded.expand)).as_secs_f64();
                let pixels = planes.layout[stream].pixels;
                let bucket = bucket_of(pixels as u64);
                row.by_bucket[group.at()][bucket] += seconds;
                row.bytes_by_bucket[group.at()][bucket] += resident_of(planes.client, pixels) as u64;
                row.stream_ms.push(ms(seconds));
            }
        });
    }
    row
}

/// Where the tight decode's time goes: Zstd on the residuals, the shape, the span walk.
fn stages_of_left(coded: &[Coded], planes: &[StreamPlanes]) -> (f64, f64, f64) {
    let (mut residuals, mut shape, mut walk) = (0.0f64, 0.0f64, 0.0f64);
    SCRATCH.with(|scratch| {
        let o = &mut *scratch.borrow_mut();
        for (coded, planes) in coded.iter().zip(planes) {
            let tables = coded.tables(None);
            for stream in 0..planes.streams() {
                let index_stream = &coded.index.streams[stream];
                let layout = &planes.layout[stream];
                shape += fastest(|| {
                    if coded.client == Client::Ec {
                        coverage_stream_decode(coded.coverage_form, &coded.coverage.streams[stream], layout, &tables, o, true);
                    } else {
                        unzstd(&mut o.context, &index_stream.extra, packed_bytes(layout.pixels) + WORD_BYTES, &mut o.extra);
                        o.extra.resize(o.extra.len() + WORD_BYTES, 0);
                    }
                })
                .as_secs_f64();
                residuals += fastest(|| unzstd(&mut o.context, &index_stream.data, index_stream.symbols, &mut o.symbols)).as_secs_f64();
                walk += fastest(|| {
                    o.index.clear();
                    o.index.resize(layout.pixels, 0);
                    match coded.client {
                        Client::Cc => decode_residuals(false, &BitsShape(&o.extra), &o.symbols, &layout.sizes, &mut o.index),
                        Client::Ec => decode_residuals(false, &BytesShape(&o.coverage), &o.symbols, &layout.sizes, &mut o.index),
                    }
                })
                .as_secs_f64();
            }
        }
    });
    (residuals, shape, walk)
}

fn print_timed(name: &str, coded: &[Coded], row: &TimedRow, output: u64, unit_name: &str) {
    let total: u64 = coded.iter().map(Coded::total).sum();
    let index: u64 = coded.iter().map(|c| c.index.streams.iter().map(IndexStream::stored).sum::<u64>() + c.index.shared()).sum();
    let mut rates = String::new();
    for group in [Group::Equipment, Group::Creature] {
        let parts: Vec<String> = (0..BUCKETS)
            .map(|b| {
                let (seconds, bytes) = (row.by_bucket[group.at()][b], row.bytes_by_bucket[group.at()][b]);
                if bytes == 0 {
                    "-".to_string()
                } else {
                    format!("{:.0}", bytes as f64 / BYTES_PER_MB / seconds)
                }
            })
            .collect();
        rates.push_str(&format!(" | {}", parts.join(" / ")));
    }
    let mut stream_ms = row.stream_ms.clone();
    stream_ms.sort_by(f64::total_cmp);
    println!(
        "| {name} | {unit_name} | {:.2} | {:.2} | {:.2} | {:.0} | {:.2} / {:.2}{rates} |",
        mb(index),
        mb(total - index),
        mb(total),
        output as f64 / BYTES_PER_MB / row.seconds,
        crowd::quantile(&stream_ms, crowd::P50),
        stream_ms.last().copied().unwrap_or(0.0),
    );
}

fn stream_planes_of(planes: &Planes, need_master: bool, unit: Unit) -> StreamPlanes {
    let luma = reordered(planes, Order::Luma);
    let master = need_master.then(|| reordered(planes, Order::Master));
    StreamPlanes::of(planes, Some(&luma), master.as_ref(), unit)
}

/// The coverage plane on its own: coverage bytes per second, tables built once per body.
fn print_coverage_alone(planes: &[StreamPlanes], dictionaries: &[Dictionaries], atoms: &mut [Atoms]) {
    println!("\n| coverage coder, alone | MB | coverage MB/s, one thread | building the tables, all bodies, ms |\n|---|---:|---:|---:|");
    for form in SIZE_COVERAGE_FORMS {
        let plan = scheme("coverage alone", IndexForm::Plain, form);
        let coded: Vec<Coded> = planes.par_iter().zip(dictionaries).zip(atoms.par_iter_mut()).map(|((p, d), a)| coded_of(&plan, p, d, a)).collect();
        let bytes: u64 = coded.iter().map(|c| c.coverage.streams.iter().map(CoverageStream::stored).sum::<u64>() + c.coverage.tables.len() as u64).sum();
        let pixels: u64 = planes.iter().flat_map(|p| p.layout.iter().map(|l| l.pixels as u64)).sum();
        let build = fastest(|| coded.iter().map(|c| c.tables(None).coverage.len()).sum::<usize>()).as_secs_f64();
        let seconds: f64 = coded
            .iter()
            .zip(planes)
            .map(|(c, p)| {
                let tables = c.tables(None);
                SCRATCH.with(|scratch| {
                    let o = &mut *scratch.borrow_mut();
                    fastest(|| {
                        for stream in 0..p.streams() {
                            coverage_stream_decode(form, &c.coverage.streams[stream], &p.layout[stream], &tables, o, true);
                        }
                    })
                    .as_secs_f64()
                })
            })
            .sum();
        println!("| {form:?} | {:.2} | {:.0} | {:.2} |", mb(bytes), pixels as f64 / BYTES_PER_MB / seconds, ms(build));
    }
}

pub(super) fn part_tt(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let kinds = BodyKinds::load(cc_dir);
    let state = cpu_state();
    println!("\n### tt: one thread, the sample, every {stride}-th body; CPU {state}; load average {}", load_average());
    for client in [Client::Cc, Client::Ec] {
        let count = present.iter().filter(|s| s.0 == client).count().div_ceil(stride);
        let sample = sample_of(present, client, count);
        let planes: Vec<Planes> = sample.par_iter().map(|&i| Planes::from_body(&load_body(sources, &bodies[i]))).collect();
        let groups: Vec<Group> = sample.iter().map(|&i| kinds.group(&bodies[i])).collect();
        let by_stream: Vec<StreamPlanes> = planes.par_iter().map(|p| stream_planes_of(p, true, Unit::ActionDirection)).collect();
        let dictionaries: Vec<Dictionaries> = by_stream.par_iter().map(dictionaries_of).collect();
        let mut atoms: Vec<Atoms> = by_stream.iter().map(|_| Atoms::default()).collect();
        let output: u64 = planes.iter().map(Planes::resident_bytes).sum();
        println!("\n### {}: {} bodies, {:.1} MB resident; load average {}\n", client.label(), planes.len(), mb(output), load_average());
        println!("| coder | unit | index MB | coverage MB | total MB | MB/s | one stream median / max ms | equipment MB/s, by stream size ({}) | creatures MB/s, by stream size |\n|---|---|---:|---:|---:|---:|---:|---:|---:|", BUCKET_NAMES.join(" / "));
        for scheme in chosen_schemes(client) {
            let coded: Vec<Coded> = by_stream.par_iter().zip(&dictionaries).zip(&mut atoms).map(|((p, d), a)| coded_of(&scheme, p, d, a)).collect();
            let row = time_scheme(&coded, &by_stream, &groups);
            print_timed(scheme.name, &coded, &row, output, "action+direction");
            if scheme.index == IndexForm::Left && scheme.coverage == CoverageForm::Zstd {
                let (residuals, shape, walk) = stages_of_left(&coded, &by_stream);
                println!("  stages of the tight decode, one thread, s: Zstd on the residuals {residuals:.3}, shape {shape:.3}, span walk {walk:.3}");
            }
        }
        if client == Client::Ec {
            print_coverage_alone(&by_stream, &dictionaries, &mut atoms);
        }
        // The units the earlier row quoted, for a like-for-like against its 361 and 161 MB/s.
        let by_action: Vec<StreamPlanes> = planes.par_iter().map(|p| stream_planes_of(p, false, Unit::Action)).collect();
        let none = Dictionaries::none();
        let mut atoms: Vec<Atoms> = by_action.iter().map(|_| Atoms::default()).collect();
        for scheme in [
            scheme("zstd 19, a new context per stream", IndexForm::PlainFresh, CoverageForm::ZstdFresh),
            scheme("luma, left, zstd 19, per-pixel predict", IndexForm::LeftPredict, CoverageForm::Zstd),
            scheme("luma, left, zstd 19, tight", IndexForm::Left, CoverageForm::Zstd),
        ] {
            let coded: Vec<Coded> = by_action.par_iter().zip(&mut atoms).map(|(p, a)| coded_of(&scheme, p, &none, a)).collect();
            let row = time_scheme(&coded, &by_action, &groups);
            print_timed(scheme.name, &coded, &row, output, "action");
        }
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part tt ({state} at the start, {end} at the end): discard its timings");
    }
    println!("Load average at the end of part tt {}", load_average());
}

// ---------------------------------------------------------------------------------------------------------------
// tc and ts: the crowds

const TOWN_NPCS: usize = 50;
const TOWN_EQUIPMENT_PER_NPC: usize = 5;
/// Chosen from the census of part `t0`: the action most human and equipment bodies share.
const TOWN_ACTION: u16 = 0;

fn creature_pairs(pairs: &[Pair], kinds: &BodyKinds, bodies: &[BodySource], count: usize) -> Vec<Pair> {
    let of: Vec<Pair> = pairs.iter().copied().filter(|p| kinds.group(&bodies[p.body()]) == Group::Creature).collect();
    crowd::pick_uniform(&of, count)
}

/// `TOWN_NPCS` NPCs of a human body and `TOWN_EQUIPMENT_PER_NPC` equipment bodies, all in one action. Human bodies repeat
/// (there are few), and equipment bodies repeat only when the action has too few of them.
fn town_pairs(pairs: &[Pair], kinds: &BodyKinds, bodies: &[BodySource]) -> Vec<Pair> {
    town_of(pairs, &|body| kinds.kind(&bodies[body]) == knowledge::KIND_HUMAN, &|body| kinds.kind(&bodies[body]) == knowledge::KIND_EQUIPMENT)
}

/// The town, from what says a body is human and what says it is equipment.
fn town_of(pairs: &[Pair], is_human: &dyn Fn(usize) -> bool, is_equipment: &dyn Fn(usize) -> bool) -> Vec<Pair> {
    let of = |wanted: &dyn Fn(usize) -> bool| -> Vec<Pair> {
        let list: Vec<Pair> = pairs.iter().copied().filter(|p| p.action() == TOWN_ACTION && wanted(p.body())).collect();
        crowd::pick_uniform(&list, list.len())
    };
    let (humans, equipment) = (of(is_human), of(is_equipment));
    assert!(!humans.is_empty() && !equipment.is_empty(), "a town needs human and equipment bodies in action {TOWN_ACTION}");
    let mut out = Vec::with_capacity(TOWN_NPCS * (TOWN_EQUIPMENT_PER_NPC + 1));
    for npc in 0..TOWN_NPCS {
        out.push(humans[npc % humans.len()]);
        for piece in 0..TOWN_EQUIPMENT_PER_NPC {
            out.push(equipment[(npc * TOWN_EQUIPMENT_PER_NPC + piece) % equipment.len()]);
        }
    }
    out
}

/// A crowd without repeated pairs: what a loader with a cache decodes.
fn distinct_pairs(pairs: &[Pair]) -> Vec<Pair> {
    let mut seen = HashSet::new();
    pairs.iter().copied().filter(|p| seen.insert((p.body(), p.action()))).collect()
}

fn describe_crowd(label: &str, pairs: &[Pair]) {
    let distinct: BTreeSet<usize> = pairs.iter().map(|p| p.body()).collect();
    let mpx: Vec<f64> = pairs.iter().map(|p| p.pixels() as f64 / PIXELS_PER_MEGAPIXEL).collect();
    println!("\n#### {label}: {} pairs of {} bodies, {:.1} Mpx trimmed; a pair's Mpx: median {:.3}, p95 {:.2}, max {:.2}\n", pairs.len(), distinct.len(), mpx.iter().sum::<f64>(), crowd::quantile(&mpx, crowd::P50), crowd::quantile(&mpx, crowd::P95), mpx.iter().copied().fold(0.0, f64::max));
}

/// One crowd through this part's coders: all five directions and only the faced one, on one and four threads.
fn tiny_crowd(label: &str, pairs: &[Pair], sources: &Sources, bodies: &[BodySource], schemes: &[Scheme]) {
    describe_crowd(label, pairs);
    print_crowd_header();
    let distinct: Vec<usize> = pairs.iter().map(|p| p.body()).collect::<BTreeSet<usize>>().into_iter().collect();
    let position: HashMap<usize, usize> = distinct.iter().enumerate().map(|(n, &b)| (b, n)).collect();
    let mut wanted_actions: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); distinct.len()];
    pairs.iter().for_each(|p| {
        wanted_actions[position[&p.body()]].insert(p.action());
    });
    let need_master = schemes.iter().any(|s| s.index == IndexForm::Xor);
    let need_dictionaries = schemes.iter().any(|s| matches!(s.index, IndexForm::Dict(_) | IndexForm::DictBest | IndexForm::SpansDict(_) | IndexForm::SpansDictBest));
    let need_plain_pick = schemes.iter().any(|s| s.index == IndexForm::DictBest);
    let need_spans_pick = schemes.iter().any(|s| s.index == IndexForm::SpansDictBest);
    // Dictionaries train on the whole body, and the best one is picked over it; only the streams of the wanted actions are coded.
    let prepared: Vec<(StreamPlanes, Dictionaries)> = distinct
        .par_iter()
        .zip(&wanted_actions)
        .map(|(&i, actions)| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            let dictionaries = if need_dictionaries {
                let whole = StreamPlanes::of(&planes, None, None, Unit::ActionDirection);
                let mut dictionaries = dictionaries_of(&whole);
                let mut atoms = Atoms::default();
                let plain = if need_plain_pick { atoms.best_size(IndexForm::Bare, IndexForm::Dict, &whole, &dictionaries) } else { None };
                let spans = if need_spans_pick { atoms.best_size(SPANS_ZSTD, IndexForm::SpansDict, &whole, &dictionaries) } else { None };
                dictionaries.picks = Some(WholePicks { plain, spans });
                dictionaries
            } else {
                Dictionaries::none()
            };
            let luma = reordered(&planes, Order::Luma);
            let master = need_master.then(|| reordered(&planes, Order::Master));
            let (small, small_luma) = (crowd::restricted(&planes, actions), crowd::restricted(&luma, actions));
            let small_master = master.as_ref().map(|m| crowd::restricted(m, actions));
            (StreamPlanes::of(&small, Some(&small_luma), small_master.as_ref(), Unit::ActionDirection), dictionaries)
        })
        .collect();
    let rgba_of = |n: usize, wanted: &[usize]| wanted.iter().map(|&s| prepared[n].0.layout[s].pixels as u64 * RGBA_CHANNELS as u64).sum::<u64>();
    let stored_of = |n: usize, wanted: &[usize]| -> u64 {
        let palettes: BTreeSet<usize> = wanted.iter().flat_map(|&s| prepared[n].0.layout[s].palettes.iter().copied()).collect();
        wanted.iter().map(|&s| resident_of(prepared[n].0.client, prepared[n].0.layout[s].pixels) as u64).sum::<u64>() + palettes.len() as u64 * crowd::PALETTE_TEXTURE_BYTES
    };
    let wanted_of: Vec<Vec<usize>> = pairs.iter().map(|p| {
        let streams = &prepared[position[&p.body()]].0;
        (0..streams.streams()).filter(|&s| streams.layout[s].action == p.action()).collect()
    }).collect();
    // What cutting the frames would save of the atlas.
    let areas: Vec<Vec<(u64, u64)>> = prepared.par_iter().map(|(p, _)| (0..p.streams()).map(|s| split::stream_areas(&p.mask[s], &p.layout[s].sizes)).collect()).collect();
    let area_of = |faced: bool| -> (u64, u64) {
        let mut sums = (0u64, 0u64);
        for (pair, p) in pairs.iter().enumerate() {
            let n = position[&p.body()];
            let wanted: Vec<usize> = if faced { vec![crowd::facing_unit(pair, &wanted_of[pair])] } else { wanted_of[pair].clone() };
            wanted.iter().for_each(|&s| {
                sums.0 += areas[n][s].0;
                sums.1 += areas[n][s].1;
            });
        }
        sums
    };
    for (faced, name) in [(false, "five directions"), (true, "faced direction")] {
        let (whole, cut) = area_of(faced);
        println!("  atlas area, {name}: {:.2} Mpx in one box per frame, {:.2} Mpx with frames cut, {:.1} % saved", whole as f64 / PIXELS_PER_MEGAPIXEL, cut as f64 / PIXELS_PER_MEGAPIXEL, (whole - cut) as f64 * PERCENT / whole.max(1) as f64);
    }
    let mut atoms: Vec<Atoms> = prepared.iter().map(|_| Atoms::default()).collect();
    for scheme in schemes {
        let started = Instant::now();
        let coded: Vec<Coded> = prepared.par_iter().zip(&mut atoms).map(|((planes, dictionaries), a)| coded_of(scheme, planes, dictionaries, a)).collect();
        let cached: Vec<Option<DecoderDictionary<'static>>> = coded.iter().map(|c| (!c.index.dictionary.is_empty()).then(|| DecoderDictionary::copy(&c.index.dictionary))).collect();
        let read_of = |n: usize, wanted: &[usize]| coded[n].shared() + wanted.iter().map(|&s| coded[n].stored_of(s)).sum::<u64>();
        let jobs = |faced: bool, use_cached: bool| -> (Vec<Job>, u64) {
            let mut read = 0u64;
            let list = pairs
                .iter()
                .enumerate()
                .map(|(pair, p)| {
                    let n = position[&p.body()];
                    let wanted: Vec<usize> = if faced { vec![crowd::facing_unit(pair, &wanted_of[pair])] } else { wanted_of[pair].clone() };
                    read += read_of(n, &wanted);
                    let (stored, rgba) = (stored_of(n, &wanted), rgba_of(n, &wanted));
                    let (code, layout, dictionary) = (&coded[n], &prepared[n].0.layout, cached[n].as_ref().filter(|_| use_cached));
                    Job { run: Box::new(move || code.decode_all(layout, &wanted, dictionary)), stored, rgba }
                })
                .collect();
            (list, read)
        };
        let (five, read) = jobs(false, false);
        crowd::print_jobs(scheme.name, "five directions", mb(read), &five);
        drop(five);
        let (faced, read) = jobs(true, false);
        crowd::print_jobs(scheme.name, "faced direction only", mb(read), &faced);
        drop(faced);
        if cached.iter().any(Option::is_some) {
            let (five, read) = jobs(false, true);
            crowd::print_jobs(scheme.name, "five directions, dictionaries already loaded", mb(read), &five);
            drop(five);
            let (faced, read) = jobs(true, true);
            crowd::print_jobs(scheme.name, "faced direction only, dictionaries already loaded", mb(read), &faced);
            drop(faced);
            let loads = fastest(|| coded.iter().filter(|c| !c.index.dictionary.is_empty()).map(|c| DecoderDictionary::copy(&c.index.dictionary)).count());
            println!("  {}: every body's dictionary loaded once, one thread, {:.2} ms for {} bodies", scheme.name, ms(loads.as_secs_f64()), cached.iter().flatten().count());
        }
        println!("  {}: this row set took {:.0} s", scheme.name, started.elapsed().as_secs_f64());
    }
}

const TC_HEADER_RULE: &str = "|---|---|---:|---:|---:|---|---:|---:|---:|---:|";

fn print_crowd_header() {
    println!("| candidate | load unit | read from disk MB | all drawable, 1 thread ms | {WORKER_THREADS} threads ms | within {} ms on {WORKER_THREADS} threads | worst job ms | job p95 ms | VRAM stored MB | RGBA8 MB |\n{TC_HEADER_RULE}", crowd::CROWD_TARGET_MS);
}

pub(super) fn part_tc(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, narrow: bool) {
    let kinds = BodyKinds::load(cc_dir);
    let state = cpu_state();
    println!("\n### {}: this part's coders in the crowds; CPU {state}; load average {}", if narrow { "tc" } else { "tu" }, load_average());
    for client in [Client::Cc, Client::Ec] {
        let pairs = crowd::all_pairs(sources, bodies, present, client);
        let schemes = chosen_schemes(client);
        let lists: Vec<(String, Vec<Pair>)> = if narrow {
            let town = town_pairs(&pairs, &kinds, bodies);
            vec![
                (format!("{}, 300 creature pairs", client.label()), creature_pairs(&pairs, &kinds, bodies, crowd::CROWD_PAIRS)),
                (format!("{}, dressed NPCs, 50 x (human + 5 equipment), action {TOWN_ACTION}", client.label()), town.clone()),
                (format!("{}, the same dressed NPCs, each distinct pair once", client.label()), distinct_pairs(&town)),
            ]
        } else {
            vec![
                (format!("{}, 300 pairs drawn uniformly", client.label()), crowd::pick_uniform(&pairs, crowd::CROWD_PAIRS)),
                (format!("{}, worst case: the 300 largest pairs", client.label()), crowd::pick_largest(&pairs, crowd::CROWD_PAIRS)),
            ]
        };
        for (label, list) in &lists {
            tiny_crowd(label, list, sources, bodies, &schemes);
            println!("  load average after this crowd {}", load_average());
        }
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during this part ({state} at the start, {end} at the end): discard its timings");
    }
}

pub(super) fn part_ts(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, only: &[String]) {
    let kinds = BodyKinds::load(cc_dir);
    let state = cpu_state();
    println!("\n### ts: the candidates of part s1 and BC7 in the creature and dressed NPC crowds; CPU {state}; load average {}", load_average());
    for client in [Client::Cc, Client::Ec] {
        let pairs = crowd::all_pairs(sources, bodies, present, client);
        let town = town_pairs(&pairs, &kinds, bodies);
        crowd::scenario(&format!("{}, 300 creature pairs", client.label()), &creature_pairs(&pairs, &kinds, bodies, crowd::CROWD_PAIRS), sources, bodies, only, true, true);
        crowd::scenario(&format!("{}, dressed NPCs, 50 x (human + 5 equipment), action {TOWN_ACTION}", client.label()), &town, sources, bodies, only, true, true);
        crowd::scenario(&format!("{}, the same dressed NPCs, each distinct pair once", client.label()), &distinct_pairs(&town), sources, bodies, only, true, true);
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part ts ({state} at the start, {end} at the end): discard its timings");
    }
}
