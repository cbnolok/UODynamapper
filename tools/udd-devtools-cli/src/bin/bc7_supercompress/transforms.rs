//! Reversible byte transforms of a BC7 block stream, applied before the entropy coder and undone after it.
//!
//! Every transform takes any byte string: the whole 16-byte blocks are transformed and a tail shorter than a block is
//! kept as it is at the end. Blocks of the reserved mode (first byte 0) are kept whole, so any input round-trips byte
//! for byte. The inverse checks the lengths, the mode map and the padding bits, and rejects what a forward pass cannot
//! have written.

use std::borrow::Cow;
use std::fmt;

pub const BLOCK_BYTES: usize = 16;
const BLOCK_BITS: u32 = 128;
/// Modes 0 to 7, and the reserved encoding (no mode bit set in the first byte), kept as raw blocks.
pub const MODE_SLOTS: usize = 9;
pub const RESERVED_MODE: u8 = 8;
const BITS_PER_BYTE: u32 = 8;

/// Bits of the index fields of each BC7 mode, which end every block: what is left once the mode, partition, rotation,
/// index selector, endpoints and p-bits are read (Microsoft's BC7 format reference, checked against the
/// field sums in the tests).
const INDEX_BITS: [u32; 8] = [45, 46, 29, 30, 78, 62, 63, 30];
/// Bits of the fields between the mode bits and the indices.
const fn prefix_bits(mode: usize) -> u32 {
    BLOCK_BITS - INDEX_BITS[mode] - (mode as u32 + 1)
}
const fn prefix_bytes(mode: usize) -> usize {
    prefix_bits(mode).div_ceil(BITS_PER_BYTE) as usize
}
const fn index_bytes(mode: usize) -> usize {
    INDEX_BITS[mode].div_ceil(BITS_PER_BYTE) as usize
}

/// The mode of a BC7 block: the position of the lowest set bit of its first byte, [`RESERVED_MODE`] for none.
pub fn block_mode(block: &[u8]) -> u8 {
    let first = block[0];
    if first == 0 { RESERVED_MODE } else { first.trailing_zeros() as u8 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    None,
    /// Byte planes: each block read as `BLOCK_BYTES / element_bytes` elements, all first elements, then all second
    /// elements and so on, over tiles of `tile_blocks` blocks (the whole stream for `None`). Element 1 is the byte
    /// shuffle `out[lane * n + block] = in[16 * block + lane]`.
    Split { element_bytes: usize, tile_blocks: Option<usize> },
    /// A mode map (one byte per block), then the blocks grouped by mode in their order.
    ModeSort,
    /// A mode map, then per mode the blocks' fields between the mode bits and the indices, byte-padded, then per mode
    /// their indices, byte-padded, then the reserved blocks. `shuffle` byte-transposes each mode's run of records.
    Fields { shuffle: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformError {
    Length { expected: usize, actual: usize },
    ModeMap { block: usize, value: u8 },
    PaddingBits { block: usize },
    BadTile,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => write!(f, "transformed length {actual}, expected {expected}"),
            Self::ModeMap { block, value } => write!(f, "mode map entry {value} of block {block} is no mode"),
            Self::PaddingBits { block } => write!(f, "padding bits set in the fields of block {block}"),
            Self::BadTile => write!(f, "split element must divide the block and tiles must hold a block"),
        }
    }
}

impl std::error::Error for TransformError {}

impl Transform {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::None,
            "modesort" => Self::ModeSort,
            "fields" => Self::Fields { shuffle: false },
            "fields-shuf" => Self::Fields { shuffle: true },
            _ => {
                let rest = text.strip_prefix("split")?;
                let (element, tile) = match rest.split_once('t') {
                    Some((element, tile)) => (element, Some(tile.parse().ok()?)),
                    None => (rest, None),
                };
                let element_bytes = element.parse().ok()?;
                if element_bytes == 0 || BLOCK_BYTES % element_bytes != 0 || tile == Some(0) {
                    return None;
                }
                Self::Split { element_bytes, tile_blocks: tile }
            }
        })
    }

    pub fn name(self) -> String {
        match self {
            Self::None => "none".into(),
            Self::ModeSort => "modesort".into(),
            Self::Fields { shuffle: false } => "fields".into(),
            Self::Fields { shuffle: true } => "fields-shuf".into(),
            Self::Split { element_bytes, tile_blocks: None } => format!("split{element_bytes}"),
            Self::Split { element_bytes, tile_blocks: Some(tile) } => format!("split{element_bytes}t{tile}"),
        }
    }

    /// Whether the transformed stream is as long as the input, so the container's raw size is enough to undo it.
    pub fn keeps_length(self) -> bool {
        matches!(self, Self::None | Self::Split { .. })
    }

    pub fn forward(self, input: &[u8], out: &mut Vec<u8>) -> Result<(), TransformError> {
        out.clear();
        match self {
            Self::None => out.extend_from_slice(input),
            Self::Split { element_bytes, tile_blocks } => split_forward(input, element_bytes, tile_blocks, out)?,
            Self::ModeSort => mode_sort_forward(input, out),
            Self::Fields { shuffle } => fields_forward(input, shuffle, out),
        }
        Ok(())
    }

    /// Undoes [`Self::forward`] of a `raw_len`-byte input.
    pub fn inverse(self, input: &[u8], raw_len: usize, out: &mut Vec<u8>) -> Result<(), TransformError> {
        out.clear();
        match self {
            Self::None => {
                check_len(raw_len, input.len())?;
                out.extend_from_slice(input);
            }
            Self::Split { element_bytes, tile_blocks } => {
                check_len(raw_len, input.len())?;
                split_inverse(input, element_bytes, tile_blocks, out)?;
            }
            Self::ModeSort => mode_sort_inverse(input, raw_len, out)?,
            Self::Fields { shuffle } => fields_inverse(input, raw_len, shuffle, out)?,
        }
        Ok(())
    }
}

fn check_len(expected: usize, actual: usize) -> Result<(), TransformError> {
    if expected == actual { Ok(()) } else { Err(TransformError::Length { expected, actual }) }
}

/// Transposes `records` records of `record_len` bytes: byte `lane` of record `i` goes to `lane * records + i`.
fn transpose_into(input: &[u8], record_len: usize, out: &mut [u8]) {
    let records = input.len() / record_len;
    for (record, bytes) in input.chunks_exact(record_len).enumerate() {
        for (lane, &byte) in bytes.iter().enumerate() {
            out[lane * records + record] = byte;
        }
    }
}

fn untranspose_into(input: &[u8], record_len: usize, out: &mut [u8]) {
    let records = input.len() / record_len;
    for (record, bytes) in out.chunks_exact_mut(record_len).enumerate() {
        for (lane, byte) in bytes.iter_mut().enumerate() {
            *byte = input[lane * records + record];
        }
    }
}

/// Byte planes of `element_bytes`-byte elements over one tile of whole blocks.
fn split_tile(input: &[u8], element_bytes: usize, out: &mut [u8], forward: bool) {
    let blocks = input.len() / BLOCK_BYTES;
    let lanes = BLOCK_BYTES / element_bytes;
    for block in 0..blocks {
        for lane in 0..lanes {
            let packed = block * BLOCK_BYTES + lane * element_bytes;
            let planar = (lane * blocks + block) * element_bytes;
            let (from, to) = if forward { (packed, planar) } else { (planar, packed) };
            out[to..to + element_bytes].copy_from_slice(&input[from..from + element_bytes]);
        }
    }
}

fn split_pass(
    input: &[u8],
    element_bytes: usize,
    tile_blocks: Option<usize>,
    out: &mut Vec<u8>,
    forward: bool,
) -> Result<(), TransformError> {
    if element_bytes == 0 || BLOCK_BYTES % element_bytes != 0 || tile_blocks == Some(0) {
        return Err(TransformError::BadTile);
    }
    let body = input.len() / BLOCK_BYTES * BLOCK_BYTES;
    out.resize(input.len(), 0);
    let tile_bytes = tile_blocks.map_or(body.max(BLOCK_BYTES), |tile| tile * BLOCK_BYTES);
    for (from, to) in input[..body].chunks(tile_bytes).zip(out[..body].chunks_mut(tile_bytes)) {
        split_tile(from, element_bytes, to, forward);
    }
    out[body..].copy_from_slice(&input[body..]);
    Ok(())
}

fn split_forward(input: &[u8], element_bytes: usize, tile_blocks: Option<usize>, out: &mut Vec<u8>)
    -> Result<(), TransformError> {
    split_pass(input, element_bytes, tile_blocks, out, true)
}

fn split_inverse(input: &[u8], element_bytes: usize, tile_blocks: Option<usize>, out: &mut Vec<u8>)
    -> Result<(), TransformError> {
    split_pass(input, element_bytes, tile_blocks, out, false)
}

fn mode_map(blocks: &[u8]) -> Vec<u8> {
    blocks.chunks_exact(BLOCK_BYTES).map(block_mode).collect()
}

/// Counts of each mode slot in a mode map, rejecting entries that are no mode.
fn mode_counts(map: &[u8]) -> Result<[usize; MODE_SLOTS], TransformError> {
    let mut counts = [0usize; MODE_SLOTS];
    for (block, &value) in map.iter().enumerate() {
        let slot = counts.get_mut(value as usize).ok_or(TransformError::ModeMap { block, value })?;
        *slot += 1;
    }
    Ok(counts)
}

fn mode_sort_forward(input: &[u8], out: &mut Vec<u8>) {
    let body = input.len() / BLOCK_BYTES * BLOCK_BYTES;
    let map = mode_map(&input[..body]);
    out.reserve(input.len() + map.len());
    out.extend_from_slice(&map);
    for mode in 0..MODE_SLOTS as u8 {
        for (block, _) in input[..body].chunks_exact(BLOCK_BYTES).zip(&map).filter(|(_, m)| **m == mode) {
            out.extend_from_slice(block);
        }
    }
    out.extend_from_slice(&input[body..]);
}

fn mode_sort_inverse(input: &[u8], raw_len: usize, out: &mut Vec<u8>) -> Result<(), TransformError> {
    let blocks = raw_len / BLOCK_BYTES;
    check_len(raw_len + blocks, input.len())?;
    let (map, rest) = input.split_at(blocks);
    let counts = mode_counts(map)?;
    let mut starts = [0usize; MODE_SLOTS];
    let mut start = 0;
    for (slot, count) in starts.iter_mut().zip(counts) {
        *slot = start;
        start += count * BLOCK_BYTES;
    }
    out.reserve(raw_len);
    for (block, &mode) in map.iter().enumerate() {
        let at = starts[mode as usize];
        let stored = &rest[at..at + BLOCK_BYTES];
        // The map must agree with the block it points at, or a corrupt map would scramble silently.
        if block_mode(stored) != mode {
            return Err(TransformError::ModeMap { block, value: mode });
        }
        out.extend_from_slice(stored);
        starts[mode as usize] += BLOCK_BYTES;
    }
    out.extend_from_slice(&rest[blocks * BLOCK_BYTES..]);
    Ok(())
}

fn block_bits(block: &[u8]) -> u128 {
    u128::from_le_bytes(block.try_into().expect("a 16-byte block"))
}

fn low_bits(value: u128, bits: u32) -> u128 {
    if bits >= BLOCK_BITS { value } else { value & ((1u128 << bits) - 1) }
}

/// Bytes of one mode's field records (fields, then indices) for `count` blocks.
fn field_stream_lengths(counts: &[usize; MODE_SLOTS]) -> ([usize; 8], [usize; 8], usize) {
    let mut prefix = [0usize; 8];
    let mut index = [0usize; 8];
    for mode in 0..8 {
        prefix[mode] = counts[mode] * prefix_bytes(mode);
        index[mode] = counts[mode] * index_bytes(mode);
    }
    (prefix, index, counts[RESERVED_MODE as usize] * BLOCK_BYTES)
}

fn fields_forward(input: &[u8], shuffle: bool, out: &mut Vec<u8>) {
    let body = input.len() / BLOCK_BYTES * BLOCK_BYTES;
    let map = mode_map(&input[..body]);
    let counts = mode_counts(&map).expect("modes come from block_mode");
    let mut prefixes: [Vec<u8>; 8] = Default::default();
    let mut indices: [Vec<u8>; 8] = Default::default();
    let mut reserved = Vec::new();
    for mode in 0..8 {
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
    out.reserve(map.len() + body + 2 * map.len());
    out.extend_from_slice(&map);
    for (streams, record_len) in [(&prefixes, prefix_bytes as fn(usize) -> usize), (&indices, index_bytes)] {
        for (mode, stream) in streams.iter().enumerate() {
            let start = out.len();
            out.resize(start + stream.len(), 0);
            if shuffle {
                transpose_into(stream, record_len(mode), &mut out[start..]);
            } else {
                out[start..].copy_from_slice(stream);
            }
        }
    }
    out.extend_from_slice(&reserved);
    out.extend_from_slice(&input[body..]);
}

fn fields_inverse(input: &[u8], raw_len: usize, shuffle: bool, out: &mut Vec<u8>) -> Result<(), TransformError> {
    let blocks = raw_len / BLOCK_BYTES;
    let tail = raw_len - blocks * BLOCK_BYTES;
    if input.len() < blocks {
        return Err(TransformError::Length { expected: blocks, actual: input.len() });
    }
    let (map, rest) = input.split_at(blocks);
    let counts = mode_counts(map)?;
    let (prefix_lengths, index_lengths, reserved_length) = field_stream_lengths(&counts);
    let expected = prefix_lengths.iter().sum::<usize>() + index_lengths.iter().sum::<usize>() + reserved_length + tail;
    check_len(blocks + expected, input.len())?;

    // Each mode's two streams, unshuffled into scratch when they were shuffled.
    let mut prefix_streams: [Cow<[u8]>; 8] = Default::default();
    let mut index_streams: [Cow<[u8]>; 8] = Default::default();
    let mut at = 0;
    for (streams, lengths, record_len) in [
        (&mut prefix_streams, &prefix_lengths, prefix_bytes as fn(usize) -> usize),
        (&mut index_streams, &index_lengths, index_bytes),
    ] {
        for mode in 0..8 {
            let stored = &rest[at..at + lengths[mode]];
            at += lengths[mode];
            streams[mode] = if shuffle {
                let mut plain = vec![0u8; stored.len()];
                untranspose_into(stored, record_len(mode), &mut plain);
                Cow::Owned(plain)
            } else {
                Cow::Borrowed(stored)
            };
        }
    }
    let reserved = &rest[at..at + reserved_length];
    let tail_bytes = &rest[at + reserved_length..];

    let start = out.len();
    out.resize(start + raw_len, 0);
    let (body_out, tail_out) = out[start..].split_at_mut(blocks * BLOCK_BYTES);
    tail_out.copy_from_slice(tail_bytes);
    let mut next = [0usize; MODE_SLOTS];
    let mut unpadded = None;
    for (block, (target, &mode)) in body_out.chunks_exact_mut(BLOCK_BYTES).zip(map).enumerate() {
        let slot = mode as usize;
        let n = next[slot];
        next[slot] += 1;
        let (prefixes, indices) = match index_streams.get(slot) {
            Some(index_stream) => (&prefix_streams[slot][..], &index_stream[..]),
            None => {
                target.copy_from_slice(&reserved[n * BLOCK_BYTES..(n + 1) * BLOCK_BYTES]);
                continue;
            }
        };
        // One instance per mode, so the record lengths and shifts are constants. A page's modes come in runs (most
        // blocks are the one transparent block), so this branch predicts better than a table-driven join, which
        // measured 50 % slower.
        let joined = match mode {
            0 => join_block::<0>(prefixes, indices, n, target),
            1 => join_block::<1>(prefixes, indices, n, target),
            2 => join_block::<2>(prefixes, indices, n, target),
            3 => join_block::<3>(prefixes, indices, n, target),
            4 => join_block::<4>(prefixes, indices, n, target),
            5 => join_block::<5>(prefixes, indices, n, target),
            6 => join_block::<6>(prefixes, indices, n, target),
            _ => join_block::<7>(prefixes, indices, n, target),
        };
        if !joined {
            unpadded = Some(block);
            break;
        }
    }
    if let Some(block) = unpadded {
        out.truncate(start);
        return Err(TransformError::PaddingBits { block });
    }
    Ok(())
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
    }

    /// A deterministic generator, so a failing case can be rerun.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u8 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 56) as u8
        }
    }

    /// Blocks of every mode and of the reserved encoding, with random fields, then `tail` stray bytes.
    fn sample(blocks: usize, tail: usize, seed: u64) -> Vec<u8> {
        let mut rng = Lcg(seed);
        let mut data = Vec::new();
        for block in 0..blocks {
            let mut bytes = [0u8; BLOCK_BYTES];
            bytes.iter_mut().for_each(|byte| *byte = rng.next());
            let mode = block % MODE_SLOTS;
            bytes[0] = if mode == RESERVED_MODE as usize { 0 } else { (bytes[0] | 1) << mode };
            data.extend_from_slice(&bytes);
        }
        data.extend((0..tail).map(|_| rng.next()));
        data
    }

    fn all_transforms() -> Vec<Transform> {
        let mut transforms = vec![Transform::None, Transform::ModeSort, Transform::Fields { shuffle: false },
            Transform::Fields { shuffle: true }];
        for element_bytes in [1, 2, 4, 8, 16] {
            for tile_blocks in [None, Some(1), Some(3), Some(32), Some(256)] {
                transforms.push(Transform::Split { element_bytes, tile_blocks });
            }
        }
        transforms
    }

    #[test]
    fn every_transform_round_trips_every_mode_and_tail() {
        for transform in all_transforms() {
            for (blocks, tail) in [(0, 0), (0, 7), (1, 0), (9, 15), (100, 1), (1000, 0)] {
                let input = sample(blocks, tail, (blocks * 31 + tail) as u64);
                let (mut forward, mut back) = (Vec::new(), Vec::new());
                transform.forward(&input, &mut forward).unwrap();
                if transform.keeps_length() {
                    assert_eq!(forward.len(), input.len());
                }
                transform.inverse(&forward, input.len(), &mut back).unwrap();
                assert_eq!(back, input, "{} with {blocks} blocks and {tail} tail bytes", transform.name());
            }
        }
    }

    #[test]
    fn names_parse_back() {
        for transform in all_transforms() {
            assert_eq!(Transform::parse(&transform.name()), Some(transform));
        }
        for bad in ["split3", "split0", "split1t0", "splitx", "shuffle"] {
            assert_eq!(Transform::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn truncated_or_padded_streams_are_rejected() {
        let input = sample(50, 3, 7);
        for transform in all_transforms() {
            let (mut forward, mut back) = (Vec::new(), Vec::new());
            transform.forward(&input, &mut forward).unwrap();
            let mut short = forward.clone();
            short.pop();
            assert!(transform.inverse(&short, input.len(), &mut back).is_err(), "{}", transform.name());
            let mut long = forward.clone();
            long.push(0);
            assert!(transform.inverse(&long, input.len(), &mut back).is_err(), "{}", transform.name());
            assert!(transform.inverse(&forward, input.len() + 1, &mut back).is_err(), "{}", transform.name());
        }
    }

    #[test]
    fn a_corrupt_mode_map_is_rejected() {
        let input = sample(20, 0, 3);
        for transform in [Transform::ModeSort, Transform::Fields { shuffle: false }, Transform::Fields { shuffle: true }] {
            let (mut forward, mut back) = (Vec::new(), Vec::new());
            transform.forward(&input, &mut forward).unwrap();
            let mut bad = forward.clone();
            bad[2] = MODE_SLOTS as u8;
            assert_eq!(transform.inverse(&bad, input.len(), &mut back),
                Err(TransformError::ModeMap { block: 2, value: MODE_SLOTS as u8 }));
            // A map whose counts no longer match the stream: one block moved to another mode.
            let mut recounted = forward.clone();
            recounted[0] = if recounted[0] == 6 { 5 } else { 6 };
            assert!(transform.inverse(&recounted, input.len(), &mut back).is_err(), "{}", transform.name());
        }
    }

    #[test]
    fn set_padding_bits_are_rejected() {
        // Mode 6 only: its fields take 58 of 64 bits and its indices 63 of 64.
        let input: Vec<u8> = sample(MODE_SLOTS * 4, 0, 11).chunks_exact(BLOCK_BYTES)
            .filter(|block| block_mode(block) == 6).flatten().copied().collect();
        let blocks = input.len() / BLOCK_BYTES;
        let (mut forward, mut back) = (Vec::new(), Vec::new());
        Transform::Fields { shuffle: false }.forward(&input, &mut forward).unwrap();
        let last_prefix_byte = blocks + prefix_bytes(6) - 1;
        forward[last_prefix_byte] |= 0x80;
        assert_eq!(Transform::Fields { shuffle: false }.inverse(&forward, input.len(), &mut back),
            Err(TransformError::PaddingBits { block: 0 }));
    }
}
