//! The fast entropy path of the `shrink` pass (parts `s2` and `s3`): keep what prediction gains, without a per-pixel
//! entropy coder or per-pixel palette work. Residuals go into a few streams chosen by the local gradient, and each
//! stream is decoded in bulk before the pixels are rebuilt: Zstd (huff0 and matches) or a table-driven tANS.
//!
//! Motion-compensated prediction adds a stored vector per block, and the moved previous frame as the predicted value.
//! Measure only; every variant round-trips through the checks of part `b`.

use super::halved::{self, KIND_NO_SEQUENCE};
use super::*;
use std::collections::BTreeSet;
use uocf::animation_sequence::AnimationSequence;

const LZ4_HC_LEVEL: i32 = 12;

/// Stream of each index context: every context alone, or the gradient buckets in fours.
const STREAM_OF_CONTEXT_FINE: [u8; INDEX_CONTEXTS] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const STREAM_OF_CONTEXT_COARSE: [u8; INDEX_CONTEXTS] = [0, 0, 1, 1, 2, 2, 3, 3, 3, 3];
const STREAMS_FINE: usize = INDEX_CONTEXTS;
const STREAMS_COARSE: usize = 4;
/// Streams of the pixels the moved frame predicts, by its distance from the spatial prediction.
const MOTION_STREAMS: usize = 4;
const MOTION_BUCKET_TOPS: [u8; MOTION_STREAMS - 1] = [0, 4, 16];
const MAX_STREAMS: usize = STREAMS_FINE + MOTION_STREAMS;

const MOTION_BLOCK: usize = 16;
const MOTION_RADIUS: i32 = 16;
/// A block's vector byte when the spatial prediction is used; the others are `vector + MOTION_RADIUS`.
const MOTION_INTRA: u8 = 255;
/// What a vector is assumed to cost when choosing a mode, in bits.
const MOTION_VECTOR_BITS: u32 = 12;
/// The search looks at every n-th pixel of a block, then checks the winner on all of them.
const MOTION_SEARCH_STEP: usize = 2;

const TANS_BITS: u32 = RANS_SCALE_BITS;
const TANS_SIZE: usize = 1 << TANS_BITS;
const TANS_STATE_BYTES: usize = 2;
const BIT_BUFFER_REFILL_BELOW: u32 = 16;
const BIT_BUFFER_FULL_ABOVE: u32 = 56;

const TIMED_ACTIONS_NOTE: &str = "sizes only: no decode number here, part b times the same streams";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SplitCoder {
    Zstd19,
    Tans,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct SplitSpec {
    predictor: Predictor,
    fine: bool,
    coder: SplitCoder,
    motion: bool,
}

pub(super) const fn split(predictor: Predictor, fine: bool, coder: SplitCoder, motion: bool) -> SplitSpec {
    SplitSpec { predictor, fine, coder, motion }
}

impl SplitSpec {
    fn spatial_streams(self) -> usize {
        if self.fine { STREAMS_FINE } else { STREAMS_COARSE }
    }

    fn streams(self) -> usize {
        self.spatial_streams() + if self.motion { MOTION_STREAMS } else { 0 }
    }

    fn map(self) -> &'static [u8; INDEX_CONTEXTS] {
        if self.fine { &STREAM_OF_CONTEXT_FINE } else { &STREAM_OF_CONTEXT_COARSE }
    }
}

pub(super) fn lz4_compress(data: &[u8]) -> Vec<u8> {
    lz4::block::compress(data, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(LZ4_HC_LEVEL)), false).expect("LZ4 HC")
}

pub(super) fn lz4_decompress(data: &[u8], size: usize) -> Vec<u8> {
    lz4::block::decompress(data, Some(size as i32)).expect("LZ4 decode")
}

// ---------------------------------------------------------------------------------------------------------------
// tANS: a 12-bit table per stream, shared by the streams of a body

#[derive(Clone, Copy, Default)]
struct TansEntry {
    next: u16,
    symbol: u8,
    bits: u8,
}

#[derive(Default)]
pub(super) struct Tans {
    decode: Vec<TansEntry>,
    /// Encoder only.
    encode: Vec<u16>,
    freqs: Vec<u16>,
    cums: Vec<u16>,
}

impl Tans {
    pub(super) fn from_freqs(freqs: Vec<u16>, for_encoding: bool) -> Tans {
        if freqs.iter().all(|&f| f == 0) {
            return Tans::default();
        }
        let mut cums = vec![0u16; BYTE_SYMBOLS];
        for s in 1..BYTE_SYMBOLS {
            cums[s] = cums[s - 1] + freqs[s - 1];
        }
        // The usual spread: an odd step visits every position once.
        let step = (TANS_SIZE >> 1) + (TANS_SIZE >> 3) + 3;
        let mut spread = vec![0u8; TANS_SIZE];
        let mut position = 0usize;
        for (s, &f) in freqs.iter().enumerate() {
            for _ in 0..f {
                spread[position] = s as u8;
                position = (position + step) & (TANS_SIZE - 1);
            }
        }
        let mut next_count: Vec<u32> = freqs.iter().map(|&f| u32::from(f)).collect();
        let mut decode = vec![TansEntry::default(); TANS_SIZE];
        let mut encode = if for_encoding { vec![0u16; TANS_SIZE] } else { Vec::new() };
        let mut seen = [0u16; BYTE_SYMBOLS];
        for (u, &symbol) in spread.iter().enumerate() {
            let s = usize::from(symbol);
            let x = next_count[s];
            next_count[s] += 1;
            let bits = TANS_BITS - (u32::BITS - 1 - x.leading_zeros());
            decode[u] = TansEntry { next: ((x << bits) - TANS_SIZE as u32) as u16, symbol, bits: bits as u8 };
            if for_encoding {
                encode[usize::from(cums[s] + seen[s])] = (TANS_SIZE + u) as u16;
            }
            seen[s] += 1;
        }
        Tans { decode, encode, freqs, cums }
    }

    pub(super) fn encode(&self, symbols: &[u8]) -> Vec<u8> {
        let mut state = TANS_SIZE as u32;
        let mut pushed = Vec::with_capacity(symbols.len());
        for &symbol in symbols.iter().rev() {
            let s = usize::from(symbol);
            let freq = u32::from(self.freqs[s]);
            let mut bits = TANS_BITS - (u32::BITS - 1 - freq.leading_zeros());
            if (state >> bits) < freq {
                bits -= 1;
            }
            pushed.push(((state & ((1 << bits) - 1)) as u16, bits as u8));
            state = u32::from(self.encode[usize::from(self.cums[s]) + (state >> bits) as usize - freq as usize]);
        }
        let mut out = ((state as usize - TANS_SIZE) as u16).to_le_bytes().to_vec();
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

    pub(super) fn decode_into(&self, data: &[u8], out: &mut [u8]) {
        let mut state = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let mut at = TANS_STATE_BYTES;
        let (mut buffer, mut have) = (0u64, 0u32);
        for symbol in out.iter_mut() {
            if have < BIT_BUFFER_REFILL_BELOW {
                while have <= BIT_BUFFER_FULL_ABOVE && at < data.len() {
                    buffer |= u64::from(data[at]) << have;
                    at += 1;
                    have += u8::BITS;
                }
            }
            let entry = self.decode[state];
            *symbol = entry.symbol;
            let bits = u32::from(entry.bits);
            state = usize::from(entry.next) + (buffer & ((1u64 << bits) - 1)) as usize;
            buffer >>= bits;
            have = have.saturating_sub(bits);
        }
    }
}

/// Decoding tables of a body from its serialized frequencies.
pub(super) fn parse_tables(bytes: &[u8], spec: SplitSpec) -> Vec<Tans> {
    if spec.coder != SplitCoder::Tans {
        return Vec::new();
    }
    let raw = decompress(bytes, spec.streams() * BYTE_SYMBOLS * LENGTH_PREFIX_BYTES);
    let mut at = 0usize;
    (0..spec.streams())
        .map(|_| {
            let freqs: Vec<u16> = (0..BYTE_SYMBOLS).map(|_| get_varint(&raw, &mut at) as u16).collect();
            Tans::from_freqs(freqs, false)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The walk over a stream's pixels, the same for coding and decoding

#[inline(always)]
fn motion_bucket(distance: u8) -> usize {
    MOTION_BUCKET_TOPS.iter().position(|&top| distance <= top).unwrap_or(MOTION_STREAMS - 1)
}

fn block_has_opaque(mask: &[u8], frame: &FrameBox, block_x: usize, block_y: usize) -> bool {
    let (x1, y1) = ((block_x + MOTION_BLOCK).min(frame.w), (block_y + MOTION_BLOCK).min(frame.h));
    (block_y..y1).any(|y| mask[y * frame.w + block_x..y * frame.w + x1].iter().any(|&m| m != 0))
}

/// The moved previous frame of every block of a frame, from the vector stream.
fn read_blocks(frame: &FrameBox, mask: &[u8], vectors: &[u8], at: &mut usize, previous: PreviousFrame, blocks: &mut Vec<Option<PreviousFrame>>) {
    blocks.clear();
    for block_y in (0..frame.h).step_by(MOTION_BLOCK) {
        for block_x in (0..frame.w).step_by(MOTION_BLOCK) {
            if !block_has_opaque(mask, frame, block_x, block_y) {
                blocks.push(None);
                continue;
            }
            let mode = vectors[*at];
            *at += 1;
            if mode == MOTION_INTRA {
                blocks.push(None);
            } else {
                let (vx, vy) = (i32::from(mode) - MOTION_RADIUS, i32::from(vectors[*at]) - MOTION_RADIUS);
                *at += 1;
                blocks.push(Some(PreviousFrame { dx: previous.dx - vx, dy: previous.dy - vy, ..previous }));
            }
        }
    }
}

#[inline(always)]
fn walk<F: FnMut(&mut [u8], usize, usize, u8)>(boxes: &[FrameBox], frames: &[usize], index: &mut [u8], mask: &[u8], spec: SplitSpec, vectors: &[u8], mut code: F) {
    let stream_of = spec.map();
    let spatial = spec.spatial_streams();
    let previous = if spec.motion { previous_frames(boxes, frames) } else { Vec::new() };
    let mut blocks = Vec::new();
    let (mut at_vector, mut start) = (0usize, 0usize);
    for (n, &f) in frames.iter().enumerate() {
        let frame = &boxes[f];
        let moved = spec.motion && previous[n].is_some();
        if moved {
            read_blocks(frame, &mask[start..start + frame.pixels()], vectors, &mut at_vector, previous[n].unwrap(), &mut blocks);
        }
        let columns = frame.w.div_ceil(MOTION_BLOCK);
        let mut last = FIRST_PREDICTION;
        for y in 0..frame.h {
            for x in 0..frame.w {
                let i = start + y * frame.w + x;
                if mask[i] == 0 {
                    continue;
                }
                let (med, context) = predict(index, mask, frame.w, i, x, y > 0, spec.predictor, last);
                let (mut prediction, mut stream) = (med, usize::from(stream_of[context]));
                if moved {
                    if let Some(shifted) = blocks[(y / MOTION_BLOCK) * columns + x / MOTION_BLOCK] {
                        if let Some(value) = shifted.pixel(index, mask, x, y) {
                            prediction = value;
                            stream = spatial + motion_bucket(value.abs_diff(med));
                        }
                    }
                }
                code(index, stream, i, prediction);
                last = index[i];
            }
        }
        start += frame.pixels();
    }
}

#[inline(always)]
fn residual_bits(residual: u8) -> u32 {
    let magnitude = u32::from(residual.min(residual.wrapping_neg()));
    1 + 2 * (u32::BITS - magnitude.leading_zeros())
}

/// A vector per block with an opaque pixel, for the frames with a previous frame; see [`read_blocks`].
fn search_vectors(boxes: &[FrameBox], frames: &[usize], index: &[u8], mask: &[u8], spec: SplitSpec) -> Vec<u8> {
    let previous = previous_frames(boxes, frames);
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut pixels: Vec<(usize, usize, usize, u32)> = Vec::with_capacity(MOTION_BLOCK * MOTION_BLOCK);
    for (n, &f) in frames.iter().enumerate() {
        let frame = &boxes[f];
        let Some(before) = previous[n] else {
            start += frame.pixels();
            continue;
        };
        for block_y in (0..frame.h).step_by(MOTION_BLOCK) {
            for block_x in (0..frame.w).step_by(MOTION_BLOCK) {
                pixels.clear();
                for y in block_y..(block_y + MOTION_BLOCK).min(frame.h) {
                    for x in block_x..(block_x + MOTION_BLOCK).min(frame.w) {
                        let i = start + y * frame.w + x;
                        if mask[i] != 0 {
                            let (med, _) = predict(index, mask, frame.w, i, x, y > 0, spec.predictor, FIRST_PREDICTION);
                            pixels.push((x, y, i, residual_bits(index[i].wrapping_sub(med))));
                        }
                    }
                }
                if pixels.is_empty() {
                    continue;
                }
                let intra: u32 = pixels.iter().map(|p| p.3).sum();
                let cost_of = |vx: i32, vy: i32, step: usize, limit: u32| -> u32 {
                    let shifted = PreviousFrame { dx: before.dx - vx, dy: before.dy - vy, ..before };
                    let mut cost = 0;
                    for &(x, y, i, spatial_bits) in pixels.iter().step_by(step) {
                        cost += shifted.pixel(index, mask, x, y).map_or(spatial_bits, |value| residual_bits(index[i].wrapping_sub(value)));
                        if cost > limit {
                            return u32::MAX;
                        }
                    }
                    cost
                };
                let mut best = (0, 0);
                let mut best_cost = cost_of(0, 0, MOTION_SEARCH_STEP, u32::MAX - 1);
                for vy in -MOTION_RADIUS..=MOTION_RADIUS {
                    for vx in -MOTION_RADIUS..=MOTION_RADIUS {
                        let cost = cost_of(vx, vy, MOTION_SEARCH_STEP, best_cost);
                        if cost < best_cost {
                            (best, best_cost) = ((vx, vy), cost);
                        }
                    }
                }
                let full = cost_of(best.0, best.1, 1, u32::MAX - 1);
                if full + MOTION_VECTOR_BITS < intra {
                    out.push((best.0 + MOTION_RADIUS) as u8);
                    out.push((best.1 + MOTION_RADIUS) as u8);
                } else {
                    out.push(MOTION_INTRA);
                }
            }
        }
        start += frame.pixels();
    }
    out
}

/// Fills the units' index streams; returns the serialized tables shared by the body's streams, if any.
pub(super) fn encode_split(planes: &Planes, groups: &[Vec<usize>], parts: &[(Vec<u8>, Vec<u8>)], spec: SplitSpec, units: &mut [EncodedUnit]) -> Vec<u8> {
    let streams = spec.streams();
    let mut coded: Vec<(Vec<u8>, Vec<Vec<u8>>)> = Vec::with_capacity(groups.len());
    for (frames, (index, mask)) in groups.iter().zip(parts) {
        let vectors = if spec.motion { search_vectors(&planes.boxes, frames, index, mask, spec) } else { Vec::new() };
        let mut residuals = vec![Vec::new(); streams];
        let mut plane = index.clone();
        walk(&planes.boxes, frames, &mut plane, mask, spec, &vectors, |plane, stream, i, prediction| {
            residuals[stream].push(plane[i].wrapping_sub(prediction));
        });
        coded.push((vectors, residuals));
    }
    let mut tables = Vec::new();
    let mut tans = Vec::new();
    if spec.coder == SplitCoder::Tans {
        let mut raw = Vec::new();
        for stream in 0..streams {
            let mut hist = vec![0u32; BYTE_SYMBOLS];
            for (_, residuals) in &coded {
                for &r in &residuals[stream] {
                    hist[usize::from(r)] += 1;
                }
            }
            let freqs = normalized(&hist);
            freqs.iter().for_each(|&f| put_varint(&mut raw, u32::from(f)));
            tans.push(Tans::from_freqs(freqs, true));
        }
        tables = compress_level(&raw, LEVEL_19, false);
    }
    for (unit, (vectors, residuals)) in units.iter_mut().zip(&coded) {
        let mut out = Vec::new();
        if spec.motion {
            let packed = compress_level(vectors, LEVEL_19, false);
            put_varint(&mut out, vectors.len() as u32);
            put_varint(&mut out, packed.len() as u32);
            out.extend_from_slice(&packed);
        }
        for (stream, symbols) in residuals.iter().enumerate() {
            put_varint(&mut out, symbols.len() as u32);
            if symbols.is_empty() {
                continue;
            }
            let packed = match spec.coder {
                SplitCoder::Zstd19 => compress_level(symbols, LEVEL_19, false),
                SplitCoder::Tans => tans[stream].encode(symbols),
            };
            put_varint(&mut out, packed.len() as u32);
            out.extend_from_slice(&packed);
        }
        unit.index = out;
    }
    tables
}

/// The vector bytes of a stream, compressed, for reporting.
fn vector_bytes(unit: &EncodedUnit) -> (u64, u64) {
    let mut at = 0usize;
    let raw = get_varint(&unit.index, &mut at);
    let packed = get_varint(&unit.index, &mut at);
    (u64::from(raw), u64::from(packed))
}

pub(super) fn decode_split(unit: &EncodedUnit, boxes: &[FrameBox], mask: &[u8], spec: SplitSpec, tables: &[Tans], total: usize) -> Vec<u8> {
    let mut at = 0usize;
    let mut vectors = Vec::new();
    if spec.motion {
        let raw = get_varint(&unit.index, &mut at) as usize;
        let packed = get_varint(&unit.index, &mut at) as usize;
        if raw > 0 {
            vectors = decompress(&unit.index[at..at + packed], raw);
        }
        at += packed;
    }
    let mut streams: [Vec<u8>; MAX_STREAMS] = Default::default();
    for (stream, out) in streams.iter_mut().enumerate().take(spec.streams()) {
        let count = get_varint(&unit.index, &mut at) as usize;
        if count == 0 {
            continue;
        }
        let packed = get_varint(&unit.index, &mut at) as usize;
        let data = &unit.index[at..at + packed];
        at += packed;
        *out = match spec.coder {
            SplitCoder::Zstd19 => decompress(data, count),
            SplitCoder::Tans => {
                let mut symbols = vec![0u8; count];
                tables[stream].decode_into(data, &mut symbols);
                symbols
            }
        };
    }
    let mut cursors = [0usize; MAX_STREAMS];
    let mut index = vec![0u8; total];
    walk(boxes, &unit.frames, &mut index, mask, spec, &vectors, |plane, stream, i, prediction| {
        plane[i] = prediction.wrapping_add(streams[stream][cursors[stream]]);
        cursors[stream] += 1;
    });
    index
}

// ---------------------------------------------------------------------------------------------------------------
// Part s3: the gain per action kind

const S3_PLAIN: Candidate = candidate("s3 row: luma, MED, 10 streams, tANS", Order::Luma, IndexCoding::Split(split(Predictor::Med, true, SplitCoder::Tans, false)), COVERAGE_ZSTD, Unit::Action);
const S3_MOTION: Candidate = candidate("s3 row: luma, MED + block motion, tANS", Order::Luma, IndexCoding::Split(split(Predictor::Med, true, SplitCoder::Tans, true)), COVERAGE_ZSTD, Unit::Action);

#[derive(Default, Clone, Copy)]
struct KindRow {
    plain: u64,
    moved: u64,
    vectors_packed: u64,
    inter_blocks: u64,
    blocks: u64,
    pixels: u64,
}

fn kind_rows(planes: &Planes, kinds: &HashMap<u32, &'static str>) -> HashMap<&'static str, KindRow> {
    let ordered = reordered(planes, Order::Luma);
    let plain = encode_candidate(&ordered, &S3_PLAIN);
    let moved = encode_candidate(&ordered, &S3_MOTION);
    verify_round_trip(&ordered, &moved, &S3_MOTION);
    let mut rows = HashMap::<&'static str, KindRow>::new();
    for (a, b) in plain.units.iter().zip(&moved.units) {
        let action = ordered.boxes[a.frames[0]].action;
        let kind = kinds.get(&u32::from(action)).copied().unwrap_or(KIND_NO_SEQUENCE);
        let row = rows.entry(kind).or_default();
        let (raw, packed) = vector_bytes(b);
        row.plain += a.index.len() as u64;
        row.moved += b.index.len() as u64;
        row.vectors_packed += packed;
        // An inter block takes two vector bytes, an intra one takes one.
        let blocks: u64 = a.frames.iter().map(|&f| (ordered.boxes[f].w.div_ceil(MOTION_BLOCK) * ordered.boxes[f].h.div_ceil(MOTION_BLOCK)) as u64).sum();
        let intra = bytes_marked_intra(b);
        row.inter_blocks += (raw - intra) / 2;
        row.blocks += blocks;
        row.pixels += a.frames.iter().map(|&f| ordered.boxes[f].pixels() as u64).sum::<u64>();
    }
    rows
}

fn bytes_marked_intra(unit: &EncodedUnit) -> u64 {
    let mut at = 0usize;
    let raw = get_varint(&unit.index, &mut at) as usize;
    let packed = get_varint(&unit.index, &mut at) as usize;
    if raw == 0 {
        return 0;
    }
    let vectors = decompress(&unit.index[at..at + packed], raw);
    let mut at = 0usize;
    let mut intra = 0u64;
    while at < vectors.len() {
        if vectors[at] == MOTION_INTRA {
            intra += 1;
            at += 1;
        } else {
            at += 2;
        }
    }
    intra
}

pub(super) fn part_s3(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], cc_dir: &Path, stride: usize) {
    let ec_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("the Enhanced client folder"));
    println!("\n### s3: motion-compensated prediction, lossless, every {stride}-th body ({TIMED_ACTIONS_NOTE}); CPU {}", cpu_state());
    let ec_sequences = halved::load_sequences(&ec_dir);
    let cc_sequences = halved::load_sequences(cc_dir);
    let ec_map = halved::sequence_of_frames_body(&ec_sequences, &ec_dir);
    let cc_map = halved::sequence_of_frames_body(&cc_sequences, cc_dir);
    for client in [Client::Cc, Client::Ec] {
        let of: Vec<(Client, u64, usize)> = present.iter().copied().filter(|s| s.0 == client).collect();
        let sample: Vec<usize> = spread(&of, of.len().div_ceil(stride), |s| s.1).into_iter().map(|s| s.2).collect();
        let rows: Vec<HashMap<&'static str, KindRow>> = sample
            .par_iter()
            .map(|&i| {
                let planes = Planes::from_body(&load_body(sources, &bodies[i]));
                let sequence: Option<&AnimationSequence> = match &bodies[i] {
                    BodySource::CcMul { .. } => None,
                    BodySource::CcUop { .. } => halved::body_number(&planes).and_then(|n| cc_map.get(&n)).and_then(|id| cc_sequences.get(id)),
                    BodySource::Ec { .. } => halved::body_number(&planes).and_then(|n| ec_map.get(&n)).and_then(|id| ec_sequences.get(id)),
                };
                let kinds = sequence.map(halved::kinds_of).unwrap_or_default();
                kind_rows(&planes, &kinds)
            })
            .collect();
        let mut total = BTreeMap::<&'static str, KindRow>::new();
        for body in &rows {
            for (kind, row) in body {
                let sum = total.entry(kind).or_default();
                sum.plain += row.plain;
                sum.moved += row.moved;
                sum.vectors_packed += row.vectors_packed;
                sum.inter_blocks += row.inter_blocks;
                sum.blocks += row.blocks;
                sum.pixels += row.pixels;
            }
        }
        println!("\n#### {}, {} bodies, action streams, index only (MB)\n", client.label(), sample.len());
        println!("| action kind | Mpx | no motion | with motion | of which vectors | change | blocks with a vector, % of all blocks |");
        println!("|---|---:|---:|---:|---:|---:|---:|");
        let mut all = KindRow::default();
        for (kind, row) in &total {
            print_kind(kind, row);
            all.plain += row.plain;
            all.moved += row.moved;
            all.vectors_packed += row.vectors_packed;
            all.inter_blocks += row.inter_blocks;
            all.blocks += row.blocks;
            all.pixels += row.pixels;
        }
        print_kind("all", &all);
    }
}

fn print_kind(kind: &str, row: &KindRow) {
    println!(
        "| {kind} | {:.1} | {:.2} | {:.2} | {:.2} | {:+.1} % | {:.1} |",
        row.pixels as f64 / PIXELS_PER_MEGAPIXEL,
        mb(row.plain),
        mb(row.moved),
        mb(row.vectors_packed),
        (row.moved as f64 / row.plain.max(1) as f64 - 1.0) * PERCENT,
        row.inter_blocks as f64 / row.blocks.max(1) as f64 * PERCENT,
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Part s1: the crowd, hundreds of distinct (body, action) pairs loaded for one screen

pub(super) const CROWD_PAIRS: usize = 300;
/// A smaller crowd, to try the part out.
const CROWD_PAIRS_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_CROWD_PAIRS";
/// `cc-uniform,cc-worst,ec-uniform,ec-worst`: which scenarios run; all by default.
const CROWD_SCENARIOS_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_CROWD_SCENARIOS";
const CROWD_NO_BC7_ENV: &str = "MOBILE_ANIM_CODEC_SHRINK_CROWD_NO_BC7";
/// The user's target: this many new actions drawable on `WORKER_THREADS` threads is fast enough.
pub(super) const CROWD_TARGET_MS: f64 = 250.0;
const CROWD_SEED: u64 = 0x2026_1102_C0DE_5EED;
/// Multiplier spreading a pair's position over its directions, for the direction it happens to face.
const FACING_HASH: u64 = 2_654_435_761;
pub(super) const PALETTE_TEXTURE_BYTES: u64 = (PALETTE_COLOURS * RGBA_CHANNELS) as u64;
const BC7_BLOCK_SIDE: usize = 4;
pub(super) const P95: f64 = 0.95;
pub(super) const P50: f64 = 0.5;

const CROWD: [Candidate; 7] = {
    use IndexCoding::{Rank, Residual, Zstd};
    use Order::{Luma, Original};
    use Predictor::Med;
    use SplitCoder::{Tans, Zstd19};
    [
        candidate("slot plane, zstd 19", Original, Zstd(LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("slot plane, LZ4 HC", Original, IndexCoding::Lz4Hc, CoverageCoding::Lz4Hc, Unit::Body),
        candidate("luma, MED, zstd 19", Luma, Residual(Med, LEVEL_19), COVERAGE_ZSTD, Unit::Body),
        candidate("luma, MED, 10 streams, zstd 19", Luma, IndexCoding::Split(split(Med, true, Zstd19, false)), COVERAGE_ZSTD, Unit::Body),
        candidate("luma, MED, 10 streams, tANS", Luma, IndexCoding::Split(split(Med, true, Tans, false)), COVERAGE_ZSTD, Unit::Body),
        candidate("luma, MED + motion, tANS", Luma, IndexCoding::Split(split(Med, true, Tans, true)), COVERAGE_ZSTD, Unit::Body),
        candidate("rank + previous, rANS (reference)", Original, Rank(knowledge::RankMode::PreviousContext, Scope::Body), COVERAGE_RANS, Unit::Body),
    ]
};

#[derive(Clone, Copy)]
pub(super) struct Pair {
    body: usize,
    action: u16,
    pixels: u64,
}

impl Pair {
    pub(super) fn body(&self) -> usize {
        self.body
    }

    pub(super) fn action(&self) -> u16 {
        self.action
    }

    pub(super) fn pixels(&self) -> u64 {
        self.pixels
    }
}

pub(super) fn all_pairs(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], client: Client) -> Vec<Pair> {
    let of: Vec<usize> = present.iter().filter(|s| s.0 == client).map(|s| s.2).collect();
    let rows: Vec<Vec<Pair>> = of
        .par_iter()
        .map(|&i| {
            let planes = Planes::from_body(&load_body(sources, &bodies[i]));
            let mut actions = BTreeMap::<u16, u64>::new();
            planes.boxes.iter().for_each(|f| *actions.entry(f.action).or_default() += f.pixels() as u64);
            actions.into_iter().map(|(action, pixels)| Pair { body: i, action, pixels }).collect()
        })
        .collect();
    rows.into_iter().flatten().collect()
}

/// Uniform over the real pairs (partial Fisher-Yates), so their sizes follow the real distribution.
pub(super) fn pick_uniform(pairs: &[Pair], count: usize) -> Vec<Pair> {
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    let mut state = CROWD_SEED;
    for n in 0..count.min(order.len()) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let other = n + (state % (order.len() - n) as u64) as usize;
        order.swap(n, other);
    }
    order[..count.min(order.len())].iter().map(|&n| pairs[n]).collect()
}

pub(super) fn pick_largest(pairs: &[Pair], count: usize) -> Vec<Pair> {
    let mut sorted = pairs.to_vec();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.pixels));
    sorted.truncate(count);
    sorted
}

pub(super) struct Job<'a> {
    pub(super) run: Box<dyn Fn() -> usize + Sync + 'a>,
    pub(super) stored: u64,
    pub(super) rgba: u64,
}

/// The queue on `threads` threads, the fastest of the timing runs: wall seconds, and each job's fastest seconds.
fn run_jobs(jobs: &[Job], threads: usize) -> (f64, Vec<f64>) {
    let mut best_wall = f64::MAX;
    let mut best: Vec<f64> = vec![f64::MAX; jobs.len()];
    for _ in 0..TIMING_RUNS {
        let next = AtomicUsize::new(0);
        let started = Instant::now();
        let measured: Vec<Vec<(usize, f64)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut mine = Vec::new();
                        loop {
                            let n = next.fetch_add(1, Ordering::Relaxed);
                            if n >= jobs.len() {
                                return mine;
                            }
                            let begun = Instant::now();
                            black_box((jobs[n].run)());
                            mine.push((n, begun.elapsed().as_secs_f64()));
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a crowd worker")).collect()
        });
        best_wall = best_wall.min(started.elapsed().as_secs_f64());
        for (n, seconds) in measured.into_iter().flatten() {
            best[n] = best[n].min(seconds);
        }
    }
    (best_wall, best)
}

pub(super) fn quantile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(((sorted.len() - 1) as f64 * q) as usize).copied().unwrap_or(0.0)
}

pub(super) fn print_jobs(name: &str, unit: &str, read: f64, jobs: &[Job]) {
    let (one, per_job) = run_jobs(jobs, 1);
    let (four, _) = run_jobs(jobs, WORKER_THREADS);
    let worst = per_job.iter().copied().fold(0.0, f64::max);
    println!(
        "| {name} | {unit} | {read:.1} | {:.1} | {:.1} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
        ms(one),
        ms(four),
        if ms(four) <= CROWD_TARGET_MS { "yes" } else { "no" },
        ms(worst),
        ms(quantile(&per_job, P95)),
        mb(jobs.iter().map(|j| j.stored).sum()),
        mb(jobs.iter().map(|j| j.rgba).sum()),
    );
}

fn print_turn(name: &str, jobs: &[Job]) {
    let (_, per_job) = run_jobs(jobs, 1);
    println!(
        "  {name}: turning, the other directions of one NPC, one thread: median {:.2} ms, p95 {:.1} ms, max {:.1} ms; {:.1} MB stored if all turn",
        ms(quantile(&per_job, P50)),
        ms(quantile(&per_job, P95)),
        ms(per_job.iter().copied().fold(0.0, f64::max)),
        mb(jobs.iter().map(|j| j.stored).sum()),
    );
}

const fn with_unit(candidate: &Candidate, unit: Unit) -> Candidate {
    Candidate { name: candidate.name, order: candidate.order, index: candidate.index, coverage: candidate.coverage, unit }
}

fn unit_stored(planes: &Planes, frames: &[usize]) -> (u64, u64) {
    let pixels: u64 = frames.iter().map(|&f| planes.boxes[f].pixels() as u64).sum();
    let palettes: HashSet<usize> = frames.iter().map(|&f| planes.boxes[f].palette).collect();
    (pixels * planes.client.pixel_bytes() as u64 + palettes.len() as u64 * PALETTE_TEXTURE_BYTES, pixels * RGBA_CHANNELS as u64)
}

/// The BC7 blocks of a frame's pixel count (`w` by `h`, rounded up to blocks).
fn bc7_bytes(frame: &FrameBox) -> usize {
    frame.w.div_ceil(BC7_BLOCK_SIDE) * frame.h.div_ceil(BC7_BLOCK_SIDE) * BC7_BLOCK_BYTES
}

const BC7_BLOCK_BYTES: usize = 16;

/// The units of one pair: all of its streams, or the facing one and the others.
fn units_of_action(boxes: &[FrameBox], encoded: &Encoded, action: u16) -> Vec<usize> {
    (0..encoded.units.len()).filter(|&u| boxes[encoded.units[u].frames[0]].action == action).collect()
}

/// A copy with only the frames of `actions`, keeping the palettes: action-sized load units only need those to be encoded.
pub(super) fn restricted(planes: &Planes, actions: &BTreeSet<u16>) -> Planes {
    let mut boxes = Vec::new();
    let (mut index, mut coverage) = (Vec::new(), Vec::new());
    for frame in planes.boxes.iter().filter(|f| actions.contains(&f.action)) {
        let range = frame.start..frame.start + frame.pixels();
        boxes.push(FrameBox { start: index.len(), ..*frame });
        index.extend_from_slice(&planes.index[range.clone()]);
        if !planes.coverage.is_empty() {
            coverage.extend_from_slice(&planes.coverage[range]);
        }
    }
    Planes { label: planes.label.clone(), client: planes.client, boxes, index, coverage, palettes: planes.palettes.clone(), bc7_zstd: 0, cleared: 0 }
}

/// What a load reads from disk: the body's tables once per job and the wanted streams.
fn read_bytes(encoded: &Encoded, wanted: &[usize]) -> u64 {
    (encoded.index_tables.len() + encoded.coverage_tables.len()) as u64
        + wanted.iter().map(|&u| &encoded.units[u]).map(|u| (u.index.len() + u.coverage.len() + u.index_tables.len() + u.coverage_tables.len()) as u64).sum::<u64>()
}

pub(super) fn facing_unit(position: usize, wanted: &[usize]) -> usize {
    wanted[((position as u64).wrapping_mul(FACING_HASH) >> u8::BITS) as usize % wanted.len()]
}

/// BC7 as today's packer makes it, without atlas pages: zstd 7 on the blocks. Its frames are freed with this call.
fn bc7_rows(loaded: &[Planes], distinct: &[usize], pairs: &[Pair], position: &HashMap<usize, usize>, wanted_actions: &[BTreeSet<u16>], sources: &Sources, bodies: &[BodySource], stream_rows_only: bool) {
    let blocks: Vec<Vec<Vec<u8>>> = distinct
        .par_iter()
        .zip(loaded)
        .map(|(&i, planes)| {
            let plane = bc7_plane(&load_body(sources, &bodies[i]), Bc7EncoderBackend::AnalyticalWide);
            let mut frames = Vec::with_capacity(planes.boxes.len());
            let mut at = 0usize;
            for frame in &planes.boxes {
                let bytes = bc7_bytes(frame);
                frames.push(plane[at..at + bytes].to_vec());
                at += bytes;
            }
            assert_eq!(at, plane.len(), "{}: the BC7 plane's frames", planes.label);
            frames
        })
        .collect();
    let rgba_of = |n: usize, frames: &[usize]| frames.iter().map(|&f| loaded[n].boxes[f].pixels() as u64 * RGBA_CHANNELS as u64).sum::<u64>();
    let raw_of = |n: usize, frames: &[usize]| frames.iter().map(|&f| blocks[n][f].len() as u64).sum::<u64>();
    let bc7_units = |unit: Unit| -> Vec<Vec<(Vec<usize>, Vec<u8>)>> {
        loaded
            .par_iter()
            .zip(&blocks)
            .enumerate()
            .map(|(n, (planes, frames))| {
                units(&planes.boxes, unit, BC7_BLOCK_BYTES)
                    .into_iter()
                    .filter(|group| unit == Unit::Body || wanted_actions[n].contains(&planes.boxes[group[0]].action))
                    .map(|group| {
                        let raw: Vec<u8> = group.iter().flat_map(|&f| frames[f].iter().copied()).collect();
                        let stored = compress(&raw, PACKER_ZSTD_LEVEL, false);
                        (group, stored)
                    })
                    .collect()
            })
            .collect()
    };
    if !stream_rows_only {
        let by_unit = bc7_units(Unit::Body);
        let read = by_unit.iter().flatten().map(|u| u.1.len() as u64).sum::<u64>();
        let jobs: Vec<Job> = (0..distinct.len())
            .map(|n| {
                let (group, data) = &by_unit[n][0];
                let raw = raw_of(n, group);
                Job { run: Box::new(move || decompress(data, raw as usize).len()), stored: raw, rgba: rgba_of(n, group) }
            })
            .collect();
        print_jobs("BC7, zstd 7", "body", mb(read), &jobs);
    }
    for (unit, name) in [(Unit::Action, "action"), (Unit::ActionDirection, "action+direction")] {
        if stream_rows_only && unit != Unit::ActionDirection {
            continue;
        }
        let by_unit = bc7_units(unit);
        let mut read = 0u64;
        let jobs: Vec<Job> = pairs
            .iter()
            .map(|p| {
                let n = position[&p.body];
                let parts: Vec<&(Vec<usize>, Vec<u8>)> = by_unit[n].iter().filter(|u| loaded[n].boxes[u.0[0]].action == p.action).collect();
                read += parts.iter().map(|u| u.1.len() as u64).sum::<u64>();
                let stored = parts.iter().map(|u| raw_of(n, &u.0)).sum();
                let rgba = parts.iter().map(|u| rgba_of(n, &u.0)).sum();
                let sizes: Vec<(usize, &[u8])> = parts.iter().map(|u| (raw_of(n, &u.0) as usize, u.1.as_slice())).collect();
                Job { run: Box::new(move || sizes.iter().map(|&(raw, data)| decompress(data, raw).len()).sum()), stored, rgba }
            })
            .collect();
        print_jobs("BC7, zstd 7", name, mb(read), &jobs);
        if stream_rows_only {
            let mut read = 0u64;
            let faced: Vec<Job> = pairs
                .iter()
                .enumerate()
                .map(|(n_pair, p)| {
                    let n = position[&p.body];
                    let parts: Vec<&(Vec<usize>, Vec<u8>)> = by_unit[n].iter().filter(|u| loaded[n].boxes[u.0[0]].action == p.action).collect();
                    let chosen: usize = facing_unit(n_pair, &(0..parts.len()).collect::<Vec<usize>>());
                    let part = parts[chosen];
                    read += part.1.len() as u64;
                    let raw = raw_of(n, &part.0);
                    Job { run: Box::new(move || decompress(&part.1, raw as usize).len()), stored: raw, rgba: rgba_of(n, &part.0) }
                })
                .collect();
            print_jobs("BC7, zstd 7", "facing direction only", mb(read), &faced);
        }
    }
}

pub(super) fn scenario(label: &str, pairs: &[Pair], sources: &Sources, bodies: &[BodySource], only: &[String], with_bc7: bool, stream_rows_only: bool) {
    let distinct: BTreeSet<usize> = pairs.iter().map(|p| p.body).collect();
    let distinct: Vec<usize> = distinct.into_iter().collect();
    let position: HashMap<usize, usize> = distinct.iter().enumerate().map(|(n, &b)| (b, n)).collect();
    let mut wanted_actions: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); distinct.len()];
    pairs.iter().for_each(|p| {
        wanted_actions[position[&p.body]].insert(p.action);
    });
    let pair_pixels: Vec<f64> = pairs.iter().map(|p| p.pixels as f64 / PIXELS_PER_MEGAPIXEL).collect();
    println!(
        "\n#### {label}: {} pairs of {} bodies, {:.1} Mpx trimmed; a pair's Mpx: median {:.3}, p95 {:.2}, max {:.2}\n",
        pairs.len(),
        distinct.len(),
        pair_pixels.iter().sum::<f64>(),
        quantile(&pair_pixels, P50),
        quantile(&pair_pixels, P95),
        pair_pixels.iter().copied().fold(0.0, f64::max),
    );
    println!("| candidate | load unit | read from disk MB | all drawable, 1 thread ms | {WORKER_THREADS} threads ms | within {CROWD_TARGET_MS} ms on {WORKER_THREADS} threads | worst job ms | job p95 ms | upload and VRAM as stored MB | RGBA8 MB |");
    println!("|---|---|---:|---:|---:|---|---:|---:|---:|---:|");

    let loaded: Vec<Planes> = distinct.par_iter().map(|&i| Planes::from_body(&load_body(sources, &bodies[i]))).collect();
    if with_bc7 {
        bc7_rows(&loaded, &distinct, pairs, &position, &wanted_actions, sources, bodies, stream_rows_only);
    }

    let chosen: Vec<&Candidate> = CROWD.iter().filter(|c| only.is_empty() || only.iter().any(|filter| c.name.contains(filter.as_str()))).collect();
    for &base in &chosen {
        let started = Instant::now();
        let ordered: Vec<Planes> = loaded.par_iter().map(|planes| reordered(planes, base.order)).collect();
        // Whole bodies.
        let mut tables = 0.0;
        if !stream_rows_only {
            let body_candidate = with_unit(base, Unit::Body);
            let encoded: Vec<Encoded> = ordered.par_iter().map(|p| encode_checked(p, &body_candidate)).collect();
            // The tables are built once per job below; a real loader keeps them per body.
            let tables_started = Instant::now();
            encoded.iter().zip(&ordered).for_each(|(e, p)| {
                black_box(shared_models(e, &body_candidate, p.client));
            });
            tables = tables_started.elapsed().as_secs_f64();
            let candidate = &body_candidate;
            let jobs: Vec<Job> = (0..distinct.len())
                .map(|n| {
                    let (planes, encoded) = (&ordered[n], &encoded[n]);
                    let all: Vec<usize> = (0..planes.boxes.len()).collect();
                    let (stored, rgba) = unit_stored(planes, &all);
                    Job { run: Box::new(move || decode_units(planes, encoded, candidate, &[0])), stored, rgba }
                })
                .collect();
            let read: u64 = encoded.iter().map(Encoded::total).sum();
            print_jobs(base.name, "body", mb(read), &jobs);
        }
        let small: Vec<Planes> = ordered.iter().zip(&wanted_actions).map(|(p, actions)| restricted(p, actions)).collect();
        drop(ordered);
        for (unit, unit_name) in [(Unit::Action, "action"), (Unit::ActionDirection, "action+direction")] {
            if stream_rows_only && unit != Unit::ActionDirection {
                continue;
            }
            let candidate = with_unit(base, unit);
            let encoded: Vec<Encoded> = small.par_iter().map(|p| encode_checked(p, &candidate)).collect();
            let candidate = &candidate;
            let mut read = 0u64;
            let plain: Vec<Job> = pairs
                .iter()
                .map(|p| {
                    let n = position[&p.body];
                    let (planes, encoded) = (&small[n], &encoded[n]);
                    let wanted = units_of_action(&planes.boxes, encoded, p.action);
                    read += read_bytes(encoded, &wanted);
                    let frames: Vec<usize> = wanted.iter().flat_map(|&u| encoded.units[u].frames.iter().copied()).collect();
                    let (stored, rgba) = unit_stored(planes, &frames);
                    Job { run: Box::new(move || decode_units(planes, encoded, candidate, &wanted)), stored, rgba }
                })
                .collect();
            print_jobs(base.name, unit_name, mb(read), &plain);
            if unit == Unit::ActionDirection {
                let mut read = 0u64;
                let facing: Vec<Job> = pairs
                    .iter()
                    .enumerate()
                    .map(|(n_pair, p)| {
                        let n = position[&p.body];
                        let (planes, encoded) = (&small[n], &encoded[n]);
                        let chosen = facing_unit(n_pair, &units_of_action(&planes.boxes, encoded, p.action));
                        read += read_bytes(encoded, &[chosen]);
                        let (stored, rgba) = unit_stored(planes, &encoded.units[chosen].frames);
                        Job { run: Box::new(move || decode_units(planes, encoded, candidate, &[chosen])), stored, rgba }
                    })
                    .collect();
                print_jobs(base.name, "facing direction only", mb(read), &facing);
                let turning: Vec<Job> = pairs
                    .iter()
                    .enumerate()
                    .map(|(n_pair, p)| {
                        let n = position[&p.body];
                        let (planes, encoded) = (&small[n], &encoded[n]);
                        let wanted = units_of_action(&planes.boxes, encoded, p.action);
                        let chosen = facing_unit(n_pair, &wanted);
                        let others: Vec<usize> = wanted.into_iter().filter(|&u| u != chosen).collect();
                        let frames: Vec<usize> = others.iter().flat_map(|&u| encoded.units[u].frames.iter().copied()).collect();
                        let (stored, rgba) = unit_stored(planes, &frames);
                        Job { run: Box::new(move || decode_units(planes, encoded, candidate, &others)), stored, rgba }
                    })
                    .collect();
                print_turn(base.name, &turning);
            }
        }
        println!("  {}: all tables once, one thread {:.1} ms; this row set took {:.0} s", base.name, ms(tables), started.elapsed().as_secs_f64());
    }
}

fn encode_checked(planes: &Planes, candidate: &Candidate) -> Encoded {
    let encoded = encode_candidate(planes, candidate);
    verify_round_trip(planes, &encoded, candidate);
    encoded
}

/// What a load does: the body's tables, then the wanted streams.
fn decode_units(planes: &Planes, encoded: &Encoded, candidate: &Candidate, wanted: &[usize]) -> usize {
    let shared = shared_models(encoded, candidate, planes.client);
    wanted.iter().map(|&u| decode_unit(&encoded.units[u], &planes.boxes, &planes.palettes, candidate, planes.client, &shared).0.len()).sum()
}

pub(super) fn part_s1(sources: &Sources, bodies: &[BodySource], present: &[(Client, u64, usize)], only: &[String]) {
    let state = cpu_state();
    let count = std::env::var(CROWD_PAIRS_ENV).ok().map(|n| n.parse::<usize>().expect("the crowd size")).unwrap_or(CROWD_PAIRS);
    println!("\n### s1: the crowd; CPU {state}; load average {}", load_average());
    // A rerun of one scenario, by name; its BC7 rows can be left out if they already ran.
    let picked = std::env::var(CROWD_SCENARIOS_ENV).ok();
    let wanted = |name: &str| picked.as_deref().is_none_or(|list| list.split(',').any(|scenario| scenario == name));
    let with_bc7 = std::env::var_os(CROWD_NO_BC7_ENV).is_none();
    for (client, short) in [(Client::Cc, "cc"), (Client::Ec, "ec")] {
        let (uniform, worst) = (format!("{short}-uniform"), format!("{short}-worst"));
        if !wanted(&uniform) && !wanted(&worst) {
            continue;
        }
        let pairs = all_pairs(sources, bodies, present, client);
        println!("\n{}: {} (body, action) pairs in the set", client.label(), pairs.len());
        if wanted(&uniform) {
            scenario(&format!("{}, {count} pairs drawn uniformly from the real ones", client.label()), &pick_uniform(&pairs, count), sources, bodies, only, with_bc7, false);
        }
        if wanted(&worst) {
            scenario(&format!("{}, worst case: the {count} largest pairs", client.label()), &pick_largest(&pairs, count), sources, bodies, only, with_bc7, false);
        }
    }
    let end = cpu_state();
    if end != state {
        println!("\nThe CPU state changed during part s1 ({state} at the start, {end} at the end): discard its timings");
    }
    println!("Load average at the end of part s1 {}", load_average());
}
