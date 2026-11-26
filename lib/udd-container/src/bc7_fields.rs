//! The BC7 field split: a reversible layout of a BC7 block stream that the container stores ahead of an entropy
//! coder (codecs [`crate::Codec::Bc7FieldsZstd`] and [`crate::Codec::Bc7FieldsLz4Hc`]), and from which a reader joins
//! only the blocks it needs ([`FieldsPage::extract`]) instead of the whole stream.
//!
//! The split stream of a `raw_len`-byte input of `blocks = raw_len / 16` whole blocks and a `raw_len % 16`-byte tail:
//! 1. the mode map: one byte per block, its BC7 mode (0 to 7), or [`RESERVED_MODE`] (8) for the reserved encoding (no
//!    mode bit set in the first byte);
//! 2. for each mode 0 to 7, the records of its blocks in block order: the bits between the mode bits and the index
//!    bits (partition, rotation, index selector, endpoints, p-bits; [`prefix_bits`]), little-endian, padded with zero
//!    bits to [`prefix_bytes`];
//! 3. for each mode 0 to 7, the index bits of its blocks in block order ([`INDEX_BITS`]), padded to [`index_bytes`];
//! 4. the reserved blocks whole, in block order;
//! 5. the tail as it is.
//!
//! Every byte string round-trips. The inverse checks the map's values, the stream length its counts give and the
//! padding bits, and rejects what [`forward`] cannot have written. Measured on the mobile animation pages
//! (`docs/agent_notes/bc7_anim_supercompression.md`): 4.0 to 4.4 % smaller than the plain blocks at Zstd 9, 8.0 to
//! 8.8 % at Zstd 19.

use std::fmt;

/// Bytes of a BC7 block.
pub const BLOCK_BYTES: usize = 16;
const BLOCK_BITS: u32 = 128;
/// The BC7 modes, 0 to 7.
const BC7_MODES: usize = 8;
/// Modes 0 to 7, and the reserved encoding (no mode bit set in the first byte), kept as raw blocks.
pub const MODE_SLOTS: usize = BC7_MODES + 1;
/// The mode map's value of a block of the reserved encoding.
pub const RESERVED_MODE: u8 = BC7_MODES as u8;
const BITS_PER_BYTE: u32 = 8;

/// Bits of the index fields of each BC7 mode, which end every block: what is left once the mode, partition, rotation,
/// index selector, endpoints and p-bits are read (Microsoft's BC7 format reference, checked against the field sums in
/// the tests).
pub const INDEX_BITS: [u32; BC7_MODES] = [45, 46, 29, 30, 78, 62, 63, 30];

/// Bits of the fields between the mode bits and the indices of a block of `mode`.
pub const fn prefix_bits(mode: usize) -> u32 {
    BLOCK_BITS - INDEX_BITS[mode] - (mode as u32 + 1)
}

/// Bytes of one field record of `mode` in the split stream.
pub const fn prefix_bytes(mode: usize) -> usize {
    prefix_bits(mode).div_ceil(BITS_PER_BYTE) as usize
}

/// Bytes of one index record of `mode` in the split stream.
pub const fn index_bytes(mode: usize) -> usize {
    INDEX_BITS[mode].div_ceil(BITS_PER_BYTE) as usize
}

/// The most bytes a block takes in the split stream: its map entry and its two records (a reserved block takes its 16
/// bytes).
pub const MAX_SPLIT_BYTES_PER_BLOCK: usize = 1 + {
    let mut most = BLOCK_BYTES;
    let mut mode = 0;
    while mode < BC7_MODES {
        let record = prefix_bytes(mode) + index_bytes(mode);
        if record > most {
            most = record;
        }
        mode += 1;
    }
    most
};

/// The longest split stream of a `raw_len`-byte input: what a decoder allocates before the map tells it the length.
pub const fn split_len_bound(raw_len: usize) -> usize {
    raw_len / BLOCK_BYTES * MAX_SPLIT_BYTES_PER_BLOCK + raw_len % BLOCK_BYTES
}

/// The mode of a BC7 block: the position of the lowest set bit of its first byte, [`RESERVED_MODE`] for none.
pub fn block_mode(block: &[u8]) -> u8 {
    let first = block[0];
    if first == 0 { RESERVED_MODE } else { first.trailing_zeros() as u8 }
}

/// A split stream that [`forward`] cannot have written, or a request outside its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldsError {
    /// The stream is not as long as its mode map's counts and the raw length give.
    Length { expected: usize, actual: usize },
    /// A mode map entry that names no mode.
    ModeMap { block: usize, value: u8 },
    /// Padding bits set in a record of the block.
    PaddingBits { block: usize },
    /// A rectangle that leaves the page's grid of blocks, or a grid that does not hold the page.
    Rect { x: usize, y: usize, width: usize, height: usize },
}

impl fmt::Display for FieldsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => write!(f, "BC7 field split stream of {actual} bytes, expected {expected}"),
            Self::ModeMap { block, value } => write!(f, "BC7 field split mode map entry {value} of block {block} is no mode"),
            Self::PaddingBits { block } => write!(f, "padding bits set in the BC7 field split records of block {block}"),
            Self::Rect { x, y, width, height } => {
                write!(f, "block rectangle {width}x{height} at {x},{y} is not within the page's grid")
            }
        }
    }
}

impl std::error::Error for FieldsError {}

fn check_len(expected: usize, actual: usize) -> Result<(), FieldsError> {
    if expected == actual { Ok(()) } else { Err(FieldsError::Length { expected, actual }) }
}

fn block_bits(block: &[u8]) -> u128 {
    u128::from_le_bytes(block.try_into().expect("a 16-byte block"))
}

fn low_bits(value: u128, bits: u32) -> u128 {
    if bits >= BLOCK_BITS { value } else { value & ((1u128 << bits) - 1) }
}

/// Entries [`count_modes`] compares at once: a 256-bit vector of bytes.
const COUNT_LANES: usize = 32;

/// Counts of each mode slot in `map`; the first entry that is no mode is an error (`first_block` is the position of
/// `map[0]` in the page).
#[inline]
fn count_modes(map: &[u8], first_block: usize) -> Result<[usize; MODE_SLOTS], FieldsError> {
    let mut counts = [0usize; MODE_SLOTS];
    // Byte counters, one per mode and lane, over groups of `COUNT_LANES` entries: compares and adds the compiler
    // vectorises. Counting each mode with `filter().count()` took 4.5 ns an entry (1.2 ms a page, half the whole
    // inverse), and a scalar histogram is a chain of dependent increments.
    for run in map.chunks(COUNT_LANES * usize::from(u8::MAX)) {
        let mut lanes = [[0u8; COUNT_LANES]; MODE_SLOTS];
        let mut groups = run.chunks_exact(COUNT_LANES);
        for group in &mut groups {
            for (mode, lanes) in lanes.iter_mut().enumerate() {
                for (lane, &value) in lanes.iter_mut().zip(group) {
                    *lane += u8::from(value == mode as u8);
                }
            }
        }
        for (count, lanes) in counts.iter_mut().zip(&lanes) {
            *count += lanes.iter().map(|&lane| usize::from(lane)).sum::<usize>();
        }
        for &value in groups.remainder() {
            if let Some(count) = counts.get_mut(usize::from(value)) {
                *count += 1;
            }
        }
    }
    if counts.iter().sum::<usize>() != map.len() {
        let block = map.iter().position(|&value| value as usize >= MODE_SLOTS).expect("an entry outside the modes");
        return Err(FieldsError::ModeMap { block: first_block + block, value: map[block] });
    }
    Ok(counts)
}

/// Counts of each mode slot in a mode map, rejecting entries that are no mode.
pub fn mode_counts(map: &[u8]) -> Result<[usize; MODE_SLOTS], FieldsError> {
    count_modes(map, 0)
}

/// Where each part of a split stream starts, after the map.
#[derive(Debug, Clone, Copy)]
struct StreamLayout {
    prefix_starts: [usize; BC7_MODES],
    index_starts: [usize; BC7_MODES],
    reserved_start: usize,
    tail_start: usize,
}

impl StreamLayout {
    /// The layout of the stream after a map of `blocks` entries with `counts`, its length checked against `len`.
    fn of(blocks: usize, counts: &[usize; MODE_SLOTS], tail: usize, len: usize) -> Result<Self, FieldsError> {
        let mut at = blocks;
        let mut prefix_starts = [0usize; BC7_MODES];
        for (mode, start) in prefix_starts.iter_mut().enumerate() {
            *start = at;
            at += counts[mode] * prefix_bytes(mode);
        }
        let mut index_starts = [0usize; BC7_MODES];
        for (mode, start) in index_starts.iter_mut().enumerate() {
            *start = at;
            at += counts[mode] * index_bytes(mode);
        }
        let reserved_start = at;
        let tail_start = reserved_start + counts[RESERVED_MODE as usize] * BLOCK_BYTES;
        check_len(tail_start + tail, len)?;
        Ok(Self { prefix_starts, index_starts, reserved_start, tail_start })
    }
}

/// Appends the split stream of `input` to `out`, cleared first.
pub fn forward(input: &[u8], out: &mut Vec<u8>) {
    out.clear();
    let body = input.len() / BLOCK_BYTES * BLOCK_BYTES;
    let map: Vec<u8> = input[..body].chunks_exact(BLOCK_BYTES).map(block_mode).collect();
    let counts = mode_counts(&map).expect("modes come from block_mode");
    let mut prefixes: [Vec<u8>; BC7_MODES] = Default::default();
    let mut indices: [Vec<u8>; BC7_MODES] = Default::default();
    let mut reserved = Vec::new();
    for mode in 0..BC7_MODES {
        prefixes[mode].reserve(counts[mode] * prefix_bytes(mode));
        indices[mode].reserve(counts[mode] * index_bytes(mode));
    }
    for (block, &mode) in input[..body].chunks_exact(BLOCK_BYTES).zip(&map) {
        if mode == RESERVED_MODE {
            reserved.extend_from_slice(block);
            continue;
        }
        let mode = mode as usize;
        let bits = block_bits(block);
        let mode_bits = mode as u32 + 1;
        let prefix = low_bits(bits >> mode_bits, prefix_bits(mode));
        let index = bits >> (mode_bits + prefix_bits(mode));
        prefixes[mode].extend_from_slice(&prefix.to_le_bytes()[..prefix_bytes(mode)]);
        indices[mode].extend_from_slice(&index.to_le_bytes()[..index_bytes(mode)]);
    }
    out.reserve(split_len_bound(input.len()));
    out.extend_from_slice(&map);
    for stream in prefixes.iter().chain(&indices) {
        out.extend_from_slice(stream);
    }
    out.extend_from_slice(&reserved);
    out.extend_from_slice(&input[body..]);
}

/// The `raw_len` bytes whose split stream is `input`, appended to `out`, cleared first.
pub fn inverse(input: &[u8], raw_len: usize, out: &mut Vec<u8>) -> Result<(), FieldsError> {
    out.clear();
    let blocks = raw_len / BLOCK_BYTES;
    let tail = raw_len - blocks * BLOCK_BYTES;
    if input.len() < blocks {
        return Err(FieldsError::Length { expected: blocks, actual: input.len() });
    }
    let map = &input[..blocks];
    let counts = mode_counts(map)?;
    let layout = StreamLayout::of(blocks, &counts, tail, input.len())?;
    let streams = RecordStreams::of(input, &layout);

    out.resize(raw_len, 0);
    let (body_out, tail_out) = out.split_at_mut(blocks * BLOCK_BYTES);
    tail_out.copy_from_slice(&input[layout.tail_start..]);
    let mut next = [0usize; MODE_SLOTS];
    for (block, (target, &mode)) in body_out.chunks_exact_mut(BLOCK_BYTES).zip(map).enumerate() {
        let slot = mode as usize;
        let n = next[slot];
        next[slot] += 1;
        if !streams.join(mode, n, target) {
            out.clear();
            return Err(FieldsError::PaddingBits { block });
        }
    }
    Ok(())
}

/// Each mode's record streams of a split stream, and its reserved blocks.
struct RecordStreams<'a> {
    prefixes: [&'a [u8]; BC7_MODES],
    indices: [&'a [u8]; BC7_MODES],
    reserved: &'a [u8],
}

impl<'a> RecordStreams<'a> {
    fn of(input: &'a [u8], layout: &StreamLayout) -> Self {
        let prefix_ends: [usize; BC7_MODES] =
            std::array::from_fn(|mode| layout.prefix_starts.get(mode + 1).copied().unwrap_or(layout.index_starts[0]));
        let index_ends: [usize; BC7_MODES] =
            std::array::from_fn(|mode| layout.index_starts.get(mode + 1).copied().unwrap_or(layout.reserved_start));
        Self {
            prefixes: std::array::from_fn(|mode| &input[layout.prefix_starts[mode]..prefix_ends[mode]]),
            indices: std::array::from_fn(|mode| &input[layout.index_starts[mode]..index_ends[mode]]),
            reserved: &input[layout.reserved_start..layout.tail_start],
        }
    }

    /// Writes block `n` of mode slot `mode` into `target`; false when a record's padding bits are set.
    #[inline(always)]
    fn join(&self, mode: u8, n: usize, target: &mut [u8]) -> bool {
        // One instance per mode, so the record lengths and shifts are constants. A page's modes come in runs (most
        // blocks are the one transparent block), so this branch predicts better than a table-driven join, which
        // measured 50 % slower.
        let slot = mode as usize;
        match mode {
            0 => join_block::<0>(self.prefixes[slot], self.indices[slot], n, target),
            1 => join_block::<1>(self.prefixes[slot], self.indices[slot], n, target),
            2 => join_block::<2>(self.prefixes[slot], self.indices[slot], n, target),
            3 => join_block::<3>(self.prefixes[slot], self.indices[slot], n, target),
            4 => join_block::<4>(self.prefixes[slot], self.indices[slot], n, target),
            5 => join_block::<5>(self.prefixes[slot], self.indices[slot], n, target),
            6 => join_block::<6>(self.prefixes[slot], self.indices[slot], n, target),
            7 => join_block::<7>(self.prefixes[slot], self.indices[slot], n, target),
            _ => {
                target.copy_from_slice(&self.reserved[n * BLOCK_BYTES..(n + 1) * BLOCK_BYTES]);
                true
            }
        }
    }
}

/// Record `n` of `len` bytes of a stream, as a little-endian number: one 16-byte load masked to the record where the
/// stream has 16 bytes left, else a copy.
#[inline(always)]
fn read_record(stream: &[u8], n: usize, len: usize) -> u128 {
    let at = n * len;
    match stream.get(at..at + BLOCK_BYTES) {
        Some(bytes) => low_bits(u128::from_le_bytes(bytes.try_into().expect("16 bytes")), len as u32 * BITS_PER_BYTE),
        None => {
            let mut buffer = [0u8; BLOCK_BYTES];
            buffer[..len].copy_from_slice(&stream[at..at + len]);
            u128::from_le_bytes(buffer)
        }
    }
}

/// Writes block `n` of mode `MODE` from its field and index records into `target`; false when a record's padding bits
/// are set.
#[inline(always)]
fn join_block<const MODE: usize>(prefixes: &[u8], indices: &[u8], n: usize, target: &mut [u8]) -> bool {
    let prefix = read_record(prefixes, n, prefix_bytes(MODE));
    let index = read_record(indices, n, index_bytes(MODE));
    let mode_bits = MODE as u32 + 1;
    if prefix >> prefix_bits(MODE) != 0 || index >> INDEX_BITS[MODE] != 0 {
        return false;
    }
    let bits = (index << (mode_bits + prefix_bits(MODE))) | (prefix << mode_bits) | (1u128 << MODE);
    target.copy_from_slice(&bits.to_le_bytes());
    true
}

/// A rectangle of whole blocks on a page, in blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// A split stream of a page laid out `blocks_wide` blocks wide, checked once, from which any rectangles of blocks
/// join without the rest of the page: what a reader handing on the frames of one clip keeps.
///
/// [`FieldsPage::new`] checks the whole map (its entries and the stream length its counts give) and keeps each mode's
/// count before each row of blocks; a block's records are then found from its row's counts and the map's entries of
/// that row before it. Padding bits are checked in the blocks joined.
pub struct FieldsPage<S = Vec<u8>> {
    stream: S,
    raw_len: usize,
    blocks_wide: usize,
    layout: StreamLayout,
    /// Each mode's count before each row of blocks.
    before_row: Vec<[u32; MODE_SLOTS]>,
}

impl<S: AsRef<[u8]>> FieldsPage<S> {
    /// The split stream `stream` of a `raw_len`-byte page `blocks_wide` blocks wide, checked. The page's whole blocks
    /// must fill whole rows.
    pub fn new(stream: S, raw_len: usize, blocks_wide: usize) -> Result<Self, FieldsError> {
        let input = stream.as_ref();
        let blocks = raw_len / BLOCK_BYTES;
        let tail = raw_len - blocks * BLOCK_BYTES;
        if input.len() < blocks {
            return Err(FieldsError::Length { expected: blocks, actual: input.len() });
        }
        if blocks_wide == 0 || blocks % blocks_wide != 0 {
            return Err(FieldsError::Rect { x: 0, y: 0, width: blocks_wide, height: 0 });
        }
        let rows = blocks / blocks_wide;
        let mut before_row = Vec::with_capacity(rows + 1);
        let mut running = [0usize; MODE_SLOTS];
        before_row.push([0u32; MODE_SLOTS]);
        for (row, entries) in input[..blocks].chunks_exact(blocks_wide).enumerate() {
            let counts = count_modes(entries, row * blocks_wide)?;
            for (total, count) in running.iter_mut().zip(counts) {
                *total += count;
            }
            // A page holds fewer than 2^32 blocks: its raw size is a u32.
            before_row.push(running.map(|count| count as u32));
        }
        let layout = StreamLayout::of(blocks, &running, tail, input.len())?;
        Ok(Self { stream, raw_len, blocks_wide, layout, before_row })
    }

    /// Bytes of the page the stream holds.
    pub fn raw_len(&self) -> usize {
        self.raw_len
    }

    /// Bytes of the split stream.
    pub fn stream_len(&self) -> usize {
        self.stream.as_ref().len()
    }

    /// The page's width in blocks.
    pub fn blocks_wide(&self) -> usize {
        self.blocks_wide
    }

    /// The page's height in blocks.
    pub fn rows(&self) -> usize {
        self.before_row.len() - 1
    }

    /// The blocks of `rects`, each row-major, one rectangle after another, in `out`, cleared first.
    pub fn extract(&self, rects: &[BlockRect], out: &mut Vec<u8>) -> Result<(), FieldsError> {
        out.clear();
        let wanted: usize = rects.iter().map(|rect| rect.width * rect.height).sum();
        out.resize(wanted * BLOCK_BYTES, 0);
        let mut at = 0;
        for rect in rects {
            let bytes = rect.width * rect.height * BLOCK_BYTES;
            if let Err(error) = self.extract_into(*rect, &mut out[at..at + bytes]) {
                out.clear();
                return Err(error);
            }
            at += bytes;
        }
        Ok(())
    }

    /// The blocks of `rect`, row-major, into `out`, which holds exactly as many.
    pub fn extract_into(&self, rect: BlockRect, out: &mut [u8]) -> Result<(), FieldsError> {
        let rows = self.rows();
        let fits = rect.x.checked_add(rect.width).is_some_and(|right| right <= self.blocks_wide)
            && rect.y.checked_add(rect.height).is_some_and(|bottom| bottom <= rows);
        if !fits || out.len() != rect.width * rect.height * BLOCK_BYTES {
            return Err(FieldsError::Rect { x: rect.x, y: rect.y, width: rect.width, height: rect.height });
        }
        let input = self.stream.as_ref();
        let map = &input[..self.raw_len / BLOCK_BYTES];
        let streams = RecordStreams::of(input, &self.layout);
        let mut targets = out.chunks_exact_mut(BLOCK_BYTES);
        for row in rect.y..rect.y + rect.height {
            let row_start = row * self.blocks_wide;
            let mut next = self.before_row[row].map(|count| count as usize);
            // Checked by `new`, so this counts and finds nothing to reject.
            let before = count_modes(&map[row_start..row_start + rect.x], row_start)?;
            for (count, more) in next.iter_mut().zip(before) {
                *count += more;
            }
            let start = row_start + rect.x;
            for (block, &mode) in map[start..start + rect.width].iter().enumerate() {
                let target = targets.next().expect("one target per block of the rectangle");
                let slot = mode as usize;
                let n = next[slot];
                next[slot] += 1;
                if !streams.join(mode, n, target) {
                    return Err(FieldsError::PaddingBits { block: start + block });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Endpoint and p-bit counts of each mode, from the BC7 format reference: (subsets, colour bits, alpha bits,
    /// p-bits per endpoint or shared, partition bits, rotation bits, index selector bits).
    const MODE_FIELDS: [(u32, u32, u32, u32, u32, u32, u32); 8] = [
        (3, 4, 0, 6, 4, 0, 0),
        (2, 6, 0, 2, 6, 0, 0),
        (3, 5, 0, 0, 6, 0, 0),
        (2, 7, 0, 4, 6, 0, 0),
        (1, 5, 6, 0, 0, 2, 1),
        (1, 7, 8, 0, 0, 2, 0),
        (1, 7, 7, 2, 0, 0, 0),
        (2, 5, 5, 4, 6, 0, 0),
    ];

    #[test]
    fn index_bits_complete_each_mode_layout() {
        for (mode, &(subsets, colour, alpha, pbits, partition, rotation, selector)) in MODE_FIELDS.iter().enumerate() {
            let endpoints = subsets * 2 * (3 * colour + alpha);
            let header = mode as u32 + 1 + partition + rotation + selector;
            assert_eq!(header + endpoints + pbits, prefix_bits(mode) + mode as u32 + 1, "mode {mode}");
        }
        // Mode 0's records take 11 + 6 bytes, the most of any mode.
        assert_eq!(MAX_SPLIT_BYTES_PER_BLOCK, 18);
    }

    /// A deterministic generator, so a failing case can be rerun.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u8 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 56) as u8
        }
    }

    fn random_block(rng: &mut Lcg, mode: usize) -> [u8; BLOCK_BYTES] {
        let mut bytes = [0u8; BLOCK_BYTES];
        bytes.iter_mut().for_each(|byte| *byte = rng.next());
        bytes[0] = if mode == RESERVED_MODE as usize { 0 } else { (bytes[0] | 1) << mode };
        bytes
    }

    /// Blocks of every mode and of the reserved encoding, with random fields, then `tail` stray bytes.
    fn sample(blocks: usize, tail: usize, seed: u64) -> Vec<u8> {
        let mut rng = Lcg(seed);
        let mut data = Vec::new();
        for block in 0..blocks {
            data.extend_from_slice(&random_block(&mut rng, block % MODE_SLOTS));
        }
        data.extend((0..tail).map(|_| rng.next()));
        data
    }

    /// Blocks in runs of one mode, as pages have them; every mode and the reserved encoding occur.
    fn runs(blocks: usize, tail: usize, seed: u64) -> Vec<u8> {
        let mut rng = Lcg(seed);
        let mut data = Vec::new();
        let mut mode = 6;
        for _ in 0..blocks {
            if rng.next() < 24 {
                mode = rng.next() as usize % MODE_SLOTS;
            }
            data.extend_from_slice(&random_block(&mut rng, mode));
        }
        data.extend((0..tail).map(|_| rng.next()));
        data
    }

    fn split(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        forward(input, &mut out);
        out
    }

    #[test]
    fn every_mode_and_tail_round_trips() {
        for (blocks, tail) in [(0, 0), (0, 7), (1, 0), (9, 15), (100, 1), (1000, 0)] {
            for input in [sample(blocks, tail, (blocks * 31 + tail) as u64), runs(blocks, tail, blocks as u64)] {
                let stream = split(&input);
                assert!(stream.len() <= split_len_bound(input.len()));
                let mut back = Vec::new();
                inverse(&stream, input.len(), &mut back).unwrap();
                assert_eq!(back, input, "{blocks} blocks, {tail} tail bytes");
            }
        }
    }

    #[test]
    fn truncated_or_extended_streams_are_rejected() {
        for input in [sample(50, 3, 7), runs(50, 3, 7)] {
            let stream = split(&input);
            let mut back = Vec::new();
            assert!(inverse(&stream[..stream.len() - 1], input.len(), &mut back).is_err());
            let mut long = stream.clone();
            long.push(0);
            assert!(inverse(&long, input.len(), &mut back).is_err());
            assert!(inverse(&stream, input.len() + 1, &mut back).is_err());
            assert!(inverse(&[], input.len(), &mut back).is_err());
        }
    }

    #[test]
    fn a_corrupt_mode_map_is_rejected() {
        let input = sample(20, 0, 3);
        let stream = split(&input);
        let mut back = Vec::new();
        let mut bad = stream.clone();
        bad[2] = MODE_SLOTS as u8;
        assert_eq!(inverse(&bad, input.len(), &mut back), Err(FieldsError::ModeMap { block: 2, value: MODE_SLOTS as u8 }));
        // A map whose counts no longer match the stream: one block moved to another mode.
        let mut recounted = stream.clone();
        recounted[0] = if recounted[0] == 6 { 5 } else { 6 };
        assert!(inverse(&recounted, input.len(), &mut back).is_err());
    }

    /// Mode 6 blocks only: its fields take 58 of 64 bits and its indices 63 of 64.
    fn mode_6_blocks() -> Vec<u8> {
        sample(MODE_SLOTS * 4, 0, 11).chunks_exact(BLOCK_BYTES).filter(|block| block_mode(block) == 6).flatten()
            .copied().collect()
    }

    #[test]
    fn set_padding_bits_are_rejected() {
        let input = mode_6_blocks();
        let blocks = input.len() / BLOCK_BYTES;
        let mut stream = split(&input);
        stream[blocks + prefix_bytes(6) - 1] |= 0x80;
        let mut back = Vec::new();
        assert_eq!(inverse(&stream, input.len(), &mut back), Err(FieldsError::PaddingBits { block: 0 }));
        assert!(back.is_empty());
    }

    /// The blocks of `rects` cut from a whole page.
    fn cut(page: &[u8], blocks_wide: usize, rects: &[BlockRect]) -> Vec<u8> {
        let mut out = Vec::new();
        for rect in rects {
            for row in rect.y..rect.y + rect.height {
                let start = (row * blocks_wide + rect.x) * BLOCK_BYTES;
                out.extend_from_slice(&page[start..start + rect.width * BLOCK_BYTES]);
            }
        }
        out
    }

    #[test]
    fn extracting_rectangles_matches_cutting_them_from_the_whole_page() {
        let (blocks_wide, rows) = (13, 11);
        for input in [sample(blocks_wide * rows, 5, 21), runs(blocks_wide * rows, 0, 22)] {
            let page = FieldsPage::new(split(&input), input.len(), blocks_wide).unwrap();
            assert_eq!((page.blocks_wide(), page.rows(), page.raw_len()), (blocks_wide, rows, input.len()));
            let rect_sets = [
                vec![],
                vec![BlockRect { x: 0, y: 0, width: blocks_wide, height: rows }],
                vec![BlockRect { x: 3, y: 2, width: 4, height: 5 }, BlockRect { x: 0, y: 9, width: 13, height: 2 }],
                vec![BlockRect { x: 12, y: 10, width: 1, height: 1 }, BlockRect { x: 5, y: 0, width: 0, height: 3 },
                    BlockRect { x: 1, y: 1, width: 2, height: 2 }],
            ];
            for rects in rect_sets {
                let mut out = Vec::new();
                page.extract(&rects, &mut out).unwrap();
                assert_eq!(out, cut(&input, blocks_wide, &rects), "{rects:?}");
            }
        }
    }

    #[test]
    fn extracting_from_a_malformed_stream_fails() {
        let (blocks_wide, rows) = (13, 11);
        let input = runs(blocks_wide * rows, 0, 23);
        let stream = split(&input);
        let page = FieldsPage::new(stream.as_slice(), input.len(), blocks_wide).unwrap();
        let mut out = Vec::new();
        for bad in [BlockRect { x: 10, y: 0, width: 4, height: 1 }, BlockRect { x: 0, y: 10, width: 1, height: 2 },
            BlockRect { x: usize::MAX, y: 0, width: 2, height: 1 }] {
            assert_eq!(page.extract(&[bad], &mut out),
                Err(FieldsError::Rect { x: bad.x, y: bad.y, width: bad.width, height: bad.height }));
        }
        // A grid that does not hold the page.
        assert!(FieldsPage::new(stream.as_slice(), input.len(), blocks_wide + 1).is_err());
        assert!(FieldsPage::new(stream.as_slice(), input.len(), 0).is_err());
        assert!(FieldsPage::new(&stream[..stream.len() - 1], input.len(), blocks_wide).is_err());
        let mut long = stream.clone();
        long.push(0);
        assert!(FieldsPage::new(long, input.len(), blocks_wide).is_err());
        // A bad entry far from any rectangle is still found: the whole map is checked.
        let mut bad_map = stream.clone();
        let far = blocks_wide * rows - 1;
        bad_map[far] = MODE_SLOTS as u8;
        assert_eq!(FieldsPage::new(bad_map, input.len(), blocks_wide).err(),
            Some(FieldsError::ModeMap { block: far, value: MODE_SLOTS as u8 }));
        // Padding bits in a block the rectangle joins.
        let mode_6 = mode_6_blocks();
        let mut stream_6 = split(&mode_6);
        let blocks = mode_6.len() / BLOCK_BYTES;
        stream_6[blocks + prefix_bytes(6) - 1] |= 0x80;
        let page_6 = FieldsPage::new(stream_6, mode_6.len(), 1).unwrap();
        assert_eq!(page_6.extract(&[BlockRect { x: 0, y: 0, width: 1, height: 1 }], &mut out),
            Err(FieldsError::PaddingBits { block: 0 }));
        assert!(out.is_empty());
    }
}
