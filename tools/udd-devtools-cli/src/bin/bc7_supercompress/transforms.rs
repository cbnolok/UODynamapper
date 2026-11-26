//! Reversible byte transforms of a BC7 block stream, applied before the entropy coder and undone after it.
//!
//! Every transform takes any byte string: the whole 16-byte blocks are transformed and a tail shorter than a block is
//! kept as it is at the end. Blocks of the reserved mode (first byte 0) are kept whole, so any input round-trips byte
//! for byte. The inverse checks the lengths, the mode map and the padding bits, and rejects what a forward pass cannot
//! have written.

use std::borrow::Cow;
use std::fmt;

use udd_container::bc7_fields::{self, FieldsError};
pub use udd_container::bc7_fields::BlockRect;

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

/// The fields between the mode bits and the indices of each BC7 mode, in their order in the block (Microsoft's BC7
/// format reference): the header (partition, rotation, index selector), then the endpoint values channel by channel
/// (red of every endpoint, green, blue, then alpha when the mode has it), then the p-bits.
#[derive(Debug, Clone, Copy)]
struct ModeLayout {
    subsets: u32,
    colour_bits: u32,
    alpha_bits: u32,
    pbits: u32,
    header_bits: u32,
}

impl ModeLayout {
    const fn endpoints(self) -> usize {
        2 * self.subsets as usize
    }
    /// Endpoint values: one per channel and endpoint.
    const fn values(self) -> usize {
        self.endpoints() * if self.alpha_bits > 0 { 4 } else { 3 }
    }
    const fn value_bits(self, value: usize) -> u32 {
        if value < 3 * self.endpoints() { self.colour_bits } else { self.alpha_bits }
    }
    /// Bit offset of endpoint value `value` within the prefix (the fields after the mode bits).
    const fn value_offset(self, value: usize) -> u32 {
        let colour = 3 * self.endpoints();
        if value < colour {
            self.header_bits + value as u32 * self.colour_bits
        } else {
            self.header_bits + colour as u32 * self.colour_bits + (value - colour) as u32 * self.alpha_bits
        }
    }
    const fn pbit_offset(self) -> u32 {
        self.value_offset(self.values())
    }
    /// The header and p-bits, the prefix bits that are no endpoint value.
    const fn side_bits(self) -> u32 {
        self.header_bits + self.pbits
    }
}

/// Subsets, colour and alpha bits per endpoint value, p-bits, header bits (partition, rotation, index selector).
const MODE_LAYOUTS: [ModeLayout; 8] = [
    ModeLayout { subsets: 3, colour_bits: 4, alpha_bits: 0, pbits: 6, header_bits: 4 },
    ModeLayout { subsets: 2, colour_bits: 6, alpha_bits: 0, pbits: 2, header_bits: 6 },
    ModeLayout { subsets: 3, colour_bits: 5, alpha_bits: 0, pbits: 0, header_bits: 6 },
    ModeLayout { subsets: 2, colour_bits: 7, alpha_bits: 0, pbits: 4, header_bits: 6 },
    ModeLayout { subsets: 1, colour_bits: 5, alpha_bits: 6, pbits: 0, header_bits: 3 },
    ModeLayout { subsets: 1, colour_bits: 7, alpha_bits: 8, pbits: 0, header_bits: 2 },
    ModeLayout { subsets: 1, colour_bits: 7, alpha_bits: 7, pbits: 2, header_bits: 0 },
    ModeLayout { subsets: 2, colour_bits: 5, alpha_bits: 5, pbits: 4, header_bits: 6 },
];
/// The most endpoint values of a mode (modes 0 and 2: three subsets of red, green and blue).
const MAX_VALUES: usize = 18;
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
    /// The `Fields` layout with each block's endpoint values replaced by their residuals against a neighbour of the
    /// same mode (modulo the field's width), in `layout`; `xor_indices` also XORs each index record with that of the
    /// predicting neighbour. Undone in raster order, each block from the blocks above and before it.
    Predict { predictor: Predictor, layout: ResidualLayout, xor_indices: bool },
}

/// The block whose endpoint values predict a block's: always one of the same mode, earlier in raster order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predictor {
    /// No prediction: the values as they are (to measure a layout alone).
    None,
    /// The previous block of the same mode (its previous record in the `Fields` layout).
    Previous,
    /// The block above when it has the same mode, else the previous one of the same mode.
    Up,
    /// LOCO-I's median edge detector over the blocks to the left, above and above-left when all three have the same
    /// mode, else the block above, the one to the left or the previous one of the same mode.
    Median,
}

/// How the residuals are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidualLayout {
    /// In the fields' own bits: the `Fields` records with each value replaced by its residual.
    Packed,
    /// Per block the header and p-bits in whole bytes, then each residual in a byte, zigzag-coded (small magnitudes
    /// of either sign become small bytes).
    Bytes,
    /// `Bytes`, byte-transposed per mode: the first byte of every record of the mode, then the second, and so on.
    Planes,
}

const PREDICTORS: [(Predictor, &str); 4] =
    [(Predictor::None, "none"), (Predictor::Previous, "prev"), (Predictor::Up, "up"), (Predictor::Median, "med")];
const RESIDUAL_LAYOUTS: [(ResidualLayout, &str); 3] =
    [(ResidualLayout::Packed, "packed"), (ResidualLayout::Bytes, "bytes"), (ResidualLayout::Planes, "planes")];
const XOR_INDICES_SUFFIX: &str = "xi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformError {
    Length { expected: usize, actual: usize },
    ModeMap { block: usize, value: u8 },
    PaddingBits { block: usize },
    BadTile,
    /// A rectangle that leaves the page's grid of blocks, or a grid that does not hold the page.
    Rect { x: usize, y: usize, width: usize, height: usize },
    /// A byte-wide endpoint residual (`pred-*-bytes`, `pred-*-planes`) outside its field's range.
    Residual { block: usize, value: u8 },
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => write!(f, "transformed length {actual}, expected {expected}"),
            Self::ModeMap { block, value } => write!(f, "mode map entry {value} of block {block} is no mode"),
            Self::PaddingBits { block } => write!(f, "padding bits set in the fields of block {block}"),
            Self::BadTile => write!(f, "split element must divide the block and tiles must hold a block"),
            Self::Rect { x, y, width, height } => {
                write!(f, "block rectangle {width}x{height} at {x},{y} is not within the page's grid")
            }
            Self::Residual { block, value } => write!(f, "endpoint residual {value} of block {block} exceeds its field"),
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
            _ if text.starts_with("pred-") => {
                let mut parts = text["pred-".len()..].split('-');
                let predictor = parts.next()?;
                let predictor = PREDICTORS.iter().find(|(_, name)| *name == predictor)?.0;
                let layout = parts.next()?;
                let layout = RESIDUAL_LAYOUTS.iter().find(|(_, name)| *name == layout)?.0;
                let xor_indices = match parts.next() {
                    None => false,
                    Some(XOR_INDICES_SUFFIX) => true,
                    Some(_) => return None,
                };
                if parts.next().is_some() {
                    return None;
                }
                Self::Predict { predictor, layout, xor_indices }
            }
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
            Self::Predict { predictor, layout, xor_indices } => {
                let predictor = PREDICTORS.iter().find(|(p, _)| *p == predictor).expect("every predictor is named").1;
                let layout = RESIDUAL_LAYOUTS.iter().find(|(l, _)| *l == layout).expect("every layout is named").1;
                let suffix = if xor_indices { format!("-{XOR_INDICES_SUFFIX}") } else { String::new() };
                format!("pred-{predictor}-{layout}{suffix}")
            }
        }
    }

    /// Whether the transformed stream is as long as the input, so the container's raw size is enough to undo it.
    pub fn keeps_length(self) -> bool {
        matches!(self, Self::None | Self::Split { .. })
    }

    /// Transforms `input`, a page `blocks_wide` blocks wide (only `Predict` reads the grid).
    pub fn forward(self, input: &[u8], blocks_wide: usize, out: &mut Vec<u8>) -> Result<(), TransformError> {
        out.clear();
        match self {
            Self::None => out.extend_from_slice(input),
            Self::Split { element_bytes, tile_blocks } => split_forward(input, element_bytes, tile_blocks, out)?,
            Self::ModeSort => mode_sort_forward(input, out),
            Self::Fields { shuffle } => fields_forward(input, shuffle, out),
            Self::Predict { predictor, layout, xor_indices } => {
                predict_forward(input, grid_width(blocks_wide)?, Prediction { predictor, layout, xor_indices }, out)
            }
        }
        Ok(())
    }

    /// Undoes [`Self::forward`] of a `raw_len`-byte input `blocks_wide` blocks wide.
    pub fn inverse(self, input: &[u8], raw_len: usize, blocks_wide: usize, out: &mut Vec<u8>)
        -> Result<(), TransformError> {
        out.clear();
        match self {
            Self::Predict { predictor, layout, xor_indices } => predict_inverse(input, raw_len, grid_width(blocks_wide)?,
                Prediction { predictor, layout, xor_indices }, out)?,
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

fn grid_width(blocks_wide: usize) -> Result<usize, TransformError> {
    if blocks_wide == 0 { Err(TransformError::Rect { x: 0, y: 0, width: 0, height: 0 }) } else { Ok(blocks_wide) }
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

impl From<FieldsError> for TransformError {
    fn from(error: FieldsError) -> Self {
        match error {
            FieldsError::Length { expected, actual } => Self::Length { expected, actual },
            FieldsError::ModeMap { block, value } => Self::ModeMap { block, value },
            FieldsError::PaddingBits { block } => Self::PaddingBits { block },
            FieldsError::Rect { x, y, width, height } => Self::Rect { x, y, width, height },
        }
    }
}

/// The field split of the container's codecs 4 and 5 ([`bc7_fields::forward`]); `fields-shuf` then transposes each
/// mode's records into byte planes.
fn fields_forward(input: &[u8], shuffle: bool, out: &mut Vec<u8>) {
    bc7_fields::forward(input, out);
    if shuffle {
        let blocks = input.len() / BLOCK_BYTES;
        let counts = mode_counts(&out[..blocks]).expect("modes come from block_mode");
        let plain = out[blocks..].to_vec();
        shuffle_records(&plain, &counts, &mut out[blocks..], transpose_into);
    }
}

fn fields_inverse(input: &[u8], raw_len: usize, shuffle: bool, out: &mut Vec<u8>) -> Result<(), TransformError> {
    if !shuffle {
        return Ok(bc7_fields::inverse(input, raw_len, out)?);
    }
    let blocks = raw_len / BLOCK_BYTES;
    let tail = raw_len - blocks * BLOCK_BYTES;
    if input.len() < blocks {
        return Err(TransformError::Length { expected: blocks, actual: input.len() });
    }
    let counts = mode_counts(&input[..blocks])?;
    let (prefix_lengths, index_lengths, reserved_length) = field_stream_lengths(&counts);
    let expected = prefix_lengths.iter().sum::<usize>() + index_lengths.iter().sum::<usize>() + reserved_length + tail;
    check_len(blocks + expected, input.len())?;
    let mut plain = input.to_vec();
    shuffle_records(&input[blocks..], &counts, &mut plain[blocks..], untranspose_into);
    Ok(bc7_fields::inverse(&plain, raw_len, out)?)
}

/// Each mode's field and index records of the stream after the map, from `input` into `out` by `pass` (transposed or
/// back); the reserved blocks and the tail are copied.
fn shuffle_records(input: &[u8], counts: &[usize; MODE_SLOTS], out: &mut [u8], pass: fn(&[u8], usize, &mut [u8])) {
    let (prefix_lengths, index_lengths, _) = field_stream_lengths(counts);
    let mut at = 0;
    for (lengths, record_len) in [(&prefix_lengths, prefix_bytes as fn(usize) -> usize), (&index_lengths, index_bytes)] {
        for mode in 0..8 {
            pass(&input[at..at + lengths[mode]], record_len(mode), &mut out[at..at + lengths[mode]]);
            at += lengths[mode];
        }
    }
    out[at..].copy_from_slice(&input[at..]);
}

/// The blocks of `rects`, each row-major, one rectangle after another, from a `fields` stream of a `raw_len`-byte page
/// `blocks_wide` blocks wide, joining only those blocks: what a reader handing on the frames of one clip needs, where
/// [`Transform::inverse`] joins the whole page. The production join ([`bc7_fields::FieldsPage`]): the whole map is
/// checked (its entries and the stream length its counts give); padding bits only in the blocks joined.
pub fn fields_extract(input: &[u8], raw_len: usize, blocks_wide: usize, rects: &[BlockRect], out: &mut Vec<u8>)
    -> Result<(), TransformError> {
    out.clear();
    Ok(bc7_fields::FieldsPage::new(input, raw_len, blocks_wide)?.extract(rects, out)?)
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

#[derive(Debug, Clone, Copy)]
struct Prediction {
    predictor: Predictor,
    layout: ResidualLayout,
    xor_indices: bool,
}

const fn value_mask(bits: u32) -> u32 {
    (1u32 << bits) - 1
}

/// The fields after the mode bits of a block of `mode`.
fn block_prefix(block: &[u8], mode: usize) -> u128 {
    low_bits(block_bits(block) >> (mode as u32 + 1), prefix_bits(mode))
}

fn block_index(block: &[u8], mode: usize) -> u128 {
    block_bits(block) >> (mode as u32 + 1 + prefix_bits(mode))
}

fn endpoint_values(prefix: u128, layout: ModeLayout) -> [u8; MAX_VALUES] {
    let mut values = [0u8; MAX_VALUES];
    for (value, slot) in values.iter_mut().enumerate().take(layout.values()) {
        *slot = ((prefix >> layout.value_offset(value)) as u32 & value_mask(layout.value_bits(value))) as u8;
    }
    values
}

/// `prefix` with its endpoint values replaced by `values`.
fn with_values(mut prefix: u128, layout: ModeLayout, values: &[u8; MAX_VALUES]) -> u128 {
    for (value, &field) in values.iter().enumerate().take(layout.values()) {
        let offset = layout.value_offset(value);
        let mask = u128::from(value_mask(layout.value_bits(value))) << offset;
        prefix = (prefix & !mask) | (u128::from(field) << offset);
    }
    prefix
}

/// The header and p-bits of a prefix, the header in the low bits.
fn side_of(prefix: u128, layout: ModeLayout) -> u128 {
    low_bits(prefix, layout.header_bits) | (low_bits(prefix >> layout.pbit_offset(), layout.pbits) << layout.header_bits)
}

fn prefix_from_side(side: u128, layout: ModeLayout, values: &[u8; MAX_VALUES]) -> u128 {
    let prefix = low_bits(side, layout.header_bits) | ((side >> layout.header_bits) << layout.pbit_offset());
    with_values(prefix, layout, values)
}

const fn side_bytes(mode: usize) -> usize {
    MODE_LAYOUTS[mode].side_bits().div_ceil(BITS_PER_BYTE) as usize
}

/// Bytes of one residual record of `mode` in `layout`.
const fn residual_bytes(mode: usize, layout: ResidualLayout) -> usize {
    match layout {
        ResidualLayout::Packed => prefix_bytes(mode),
        ResidualLayout::Bytes | ResidualLayout::Planes => side_bytes(mode) + MODE_LAYOUTS[mode].values(),
    }
}

/// A `bits`-wide residual (modulo 2^bits) as a zigzag byte: 0, -1, 1, -2, ... become 0, 1, 2, 3, ...
const fn zigzag(residual: u32, bits: u32) -> u8 {
    let half = 1u32 << (bits - 1);
    let byte = if residual < half { residual * 2 } else { ((1u32 << bits) - residual) * 2 - 1 };
    byte as u8
}

const fn unzigzag(byte: u8, bits: u32) -> u32 {
    let byte = byte as u32;
    if byte % 2 == 0 { byte / 2 } else { ((1u32 << bits) - (byte + 1) / 2) & value_mask(bits) }
}

/// The values that predict block `block` of `mode` and the block whose index record predicts its indices, from the
/// earlier blocks of `blocks` (raster order, `blocks_wide` wide); `previous` is the previous block of the mode.
fn prediction_of(
    predictor: Predictor,
    map: &[u8],
    blocks: &[u8],
    block: usize,
    blocks_wide: usize,
    mode: usize,
    previous: Option<usize>,
) -> ([u8; MAX_VALUES], Option<usize>) {
    let layout = MODE_LAYOUTS[mode];
    let same = |at: usize| map[at] as usize == mode;
    let left = (block % blocks_wide != 0 && same(block - 1)).then(|| block - 1);
    let up = (block >= blocks_wide && same(block - blocks_wide)).then(|| block - blocks_wide);
    let up_left = (left.is_some() && up.is_some() && same(block - blocks_wide - 1)).then(|| block - blocks_wide - 1);
    let values_at = |at: usize| endpoint_values(block_prefix(&blocks[at * BLOCK_BYTES..(at + 1) * BLOCK_BYTES], mode),
        layout);
    let from = |source: Option<usize>| (source.map_or([0; MAX_VALUES], values_at), source);
    match predictor {
        Predictor::None => ([0; MAX_VALUES], previous),
        Predictor::Previous => from(previous),
        Predictor::Up => from(up.or(previous)),
        Predictor::Median => match (left, up, up_left) {
            (Some(left), Some(up), Some(up_left)) => {
                let (a, b, c) = (values_at(left), values_at(up), values_at(up_left));
                let mut predicted = [0u8; MAX_VALUES];
                for value in 0..layout.values() {
                    let (a, b, c) = (a[value], b[value], c[value]);
                    predicted[value] = if c >= a.max(b) {
                        a.min(b)
                    } else if c <= a.min(b) {
                        a.max(b)
                    } else {
                        // Between a and b, so in range; the sum may pass 255 (8-bit alpha) on the way.
                        a.wrapping_add(b).wrapping_sub(c)
                    };
                }
                (predicted, Some(up))
            }
            _ => from(up.or(left).or(previous)),
        },
    }
}

fn predict_forward(input: &[u8], blocks_wide: usize, prediction: Prediction, out: &mut Vec<u8>) {
    let body = input.len() / BLOCK_BYTES * BLOCK_BYTES;
    let blocks = &input[..body];
    let map = mode_map(blocks);
    let mut residuals: [Vec<u8>; 8] = Default::default();
    let mut indices: [Vec<u8>; 8] = Default::default();
    let mut reserved = Vec::new();
    let mut previous: [Option<usize>; 8] = [None; 8];
    for (block, &mode) in map.iter().enumerate() {
        let bytes = &blocks[block * BLOCK_BYTES..(block + 1) * BLOCK_BYTES];
        if mode == RESERVED_MODE {
            reserved.extend_from_slice(bytes);
            continue;
        }
        let mode = mode as usize;
        let layout = MODE_LAYOUTS[mode];
        let (predicted, source) =
            prediction_of(prediction.predictor, &map, blocks, block, blocks_wide, mode, previous[mode]);
        previous[mode] = Some(block);
        let prefix = block_prefix(bytes, mode);
        let values = endpoint_values(prefix, layout);
        let mut residual = [0u8; MAX_VALUES];
        for value in 0..layout.values() {
            let bits = layout.value_bits(value);
            let difference = u32::from(values[value]).wrapping_sub(u32::from(predicted[value])) & value_mask(bits);
            residual[value] = match prediction.layout {
                ResidualLayout::Packed => difference as u8,
                ResidualLayout::Bytes | ResidualLayout::Planes => zigzag(difference, bits),
            };
        }
        match prediction.layout {
            ResidualLayout::Packed => residuals[mode]
                .extend_from_slice(&with_values(prefix, layout, &residual).to_le_bytes()[..prefix_bytes(mode)]),
            ResidualLayout::Bytes | ResidualLayout::Planes => {
                residuals[mode].extend_from_slice(&side_of(prefix, layout).to_le_bytes()[..side_bytes(mode)]);
                residuals[mode].extend_from_slice(&residual[..layout.values()]);
            }
        }
        let mut index = block_index(bytes, mode);
        if let (true, Some(source)) = (prediction.xor_indices, source) {
            index ^= block_index(&blocks[source * BLOCK_BYTES..(source + 1) * BLOCK_BYTES], mode);
        }
        indices[mode].extend_from_slice(&index.to_le_bytes()[..index_bytes(mode)]);
    }
    out.reserve(map.len() + input.len() + map.len());
    out.extend_from_slice(&map);
    for (mode, stream) in residuals.iter().enumerate() {
        let start = out.len();
        out.resize(start + stream.len(), 0);
        if prediction.layout == ResidualLayout::Planes {
            transpose_into(stream, residual_bytes(mode, prediction.layout), &mut out[start..]);
        } else {
            out[start..].copy_from_slice(stream);
        }
    }
    for stream in &indices {
        out.extend_from_slice(stream);
    }
    out.extend_from_slice(&reserved);
    out.extend_from_slice(&input[body..]);
}

fn predict_inverse(input: &[u8], raw_len: usize, blocks_wide: usize, prediction: Prediction, out: &mut Vec<u8>)
    -> Result<(), TransformError> {
    let blocks = raw_len / BLOCK_BYTES;
    let tail = raw_len - blocks * BLOCK_BYTES;
    if input.len() < blocks {
        return Err(TransformError::Length { expected: blocks, actual: input.len() });
    }
    let (map, rest) = input.split_at(blocks);
    let counts = mode_counts(map)?;
    let residual_lengths: [usize; 8] = std::array::from_fn(|mode| counts[mode] * residual_bytes(mode, prediction.layout));
    let index_lengths: [usize; 8] = std::array::from_fn(|mode| counts[mode] * index_bytes(mode));
    let reserved_length = counts[RESERVED_MODE as usize] * BLOCK_BYTES;
    let expected = residual_lengths.iter().sum::<usize>() + index_lengths.iter().sum::<usize>() + reserved_length + tail;
    check_len(blocks + expected, input.len())?;
    let mut residual_streams: [Cow<[u8]>; 8] = Default::default();
    let mut index_streams: [&[u8]; 8] = [&[]; 8];
    let mut at = 0;
    for mode in 0..8 {
        let stored = &rest[at..at + residual_lengths[mode]];
        at += residual_lengths[mode];
        residual_streams[mode] = if prediction.layout == ResidualLayout::Planes {
            let mut plain = vec![0u8; stored.len()];
            untranspose_into(stored, residual_bytes(mode, prediction.layout), &mut plain);
            Cow::Owned(plain)
        } else {
            Cow::Borrowed(stored)
        };
    }
    for mode in 0..8 {
        index_streams[mode] = &rest[at..at + index_lengths[mode]];
        at += index_lengths[mode];
    }
    let reserved = &rest[at..at + reserved_length];
    out.resize(raw_len, 0);
    out[blocks * BLOCK_BYTES..].copy_from_slice(&rest[at + reserved_length..]);
    let mut next = [0usize; MODE_SLOTS];
    let mut previous: [Option<usize>; 8] = [None; 8];
    for block in 0..blocks {
        let slot = map[block] as usize;
        let n = next[slot];
        next[slot] += 1;
        let (done, target) = out.split_at_mut(block * BLOCK_BYTES);
        let target = &mut target[..BLOCK_BYTES];
        if slot == RESERVED_MODE as usize {
            target.copy_from_slice(&reserved[n * BLOCK_BYTES..(n + 1) * BLOCK_BYTES]);
            continue;
        }
        let mode = slot;
        let layout = MODE_LAYOUTS[mode];
        let (predicted, source) =
            prediction_of(prediction.predictor, map, done, block, blocks_wide, mode, previous[mode]);
        previous[mode] = Some(block);
        let record_len = residual_bytes(mode, prediction.layout);
        let record = &residual_streams[mode][n * record_len..(n + 1) * record_len];
        let (residual_prefix, residual) = match prediction.layout {
            ResidualLayout::Packed => {
                let prefix = read_record(record, 0, record_len);
                if prefix >> prefix_bits(mode) != 0 {
                    return Err(TransformError::PaddingBits { block });
                }
                (prefix, endpoint_values(prefix, layout))
            }
            ResidualLayout::Bytes | ResidualLayout::Planes => {
                let side = read_record(&record[..side_bytes(mode)], 0, side_bytes(mode));
                if side >> layout.side_bits() != 0 {
                    return Err(TransformError::PaddingBits { block });
                }
                let mut residual = [0u8; MAX_VALUES];
                for value in 0..layout.values() {
                    let bits = layout.value_bits(value);
                    let byte = record[side_bytes(mode) + value];
                    if u32::from(byte) > value_mask(bits) {
                        return Err(TransformError::Residual { block, value: byte });
                    }
                    residual[value] = unzigzag(byte, bits) as u8;
                }
                (prefix_from_side(side, layout, &[0; MAX_VALUES]), residual)
            }
        };
        let mut values = [0u8; MAX_VALUES];
        for value in 0..layout.values() {
            let bits = layout.value_bits(value);
            values[value] =
                (u32::from(residual[value]).wrapping_add(u32::from(predicted[value])) & value_mask(bits)) as u8;
        }
        let prefix = with_values(residual_prefix, layout, &values);
        let mut index = read_record(index_streams[mode], n, index_bytes(mode));
        if index >> INDEX_BITS[mode] != 0 {
            return Err(TransformError::PaddingBits { block });
        }
        if let (true, Some(source)) = (prediction.xor_indices, source) {
            index ^= block_index(&done[source * BLOCK_BYTES..(source + 1) * BLOCK_BYTES], mode);
        }
        let mode_bits = mode as u32 + 1;
        let bits = (index << (mode_bits + prefix_bits(mode))) | (prefix << mode_bits) | (1u128 << mode);
        target.copy_from_slice(&bits.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid width the tests lay their blocks on: not a divisor of most block counts, so rows end mid-run.
    const TEST_BLOCKS_WIDE: usize = 7;

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

    #[test]
    fn mode_layouts_match_the_format_reference() {
        for (mode, &(subsets, colour, alpha, pbits, partition, rotation, selector)) in MODE_FIELDS.iter().enumerate() {
            let layout = MODE_LAYOUTS[mode];
            assert_eq!((layout.subsets, layout.colour_bits, layout.alpha_bits, layout.pbits, layout.header_bits),
                (subsets, colour, alpha, pbits, partition + rotation + selector), "mode {mode}");
            assert_eq!(layout.pbit_offset() + layout.pbits, prefix_bits(mode), "mode {mode}");
            assert!(layout.values() <= MAX_VALUES, "mode {mode}");
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

    /// Blocks in runs of one mode, as pages have them, so that the left, upper and upper-left neighbours often share
    /// a block's mode; every mode and the reserved encoding occur.
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

    fn predictions() -> Vec<Transform> {
        let mut transforms = Vec::new();
        for (predictor, _) in PREDICTORS {
            for (layout, _) in RESIDUAL_LAYOUTS {
                for xor_indices in [false, true] {
                    transforms.push(Transform::Predict { predictor, layout, xor_indices });
                }
            }
        }
        transforms
    }

    fn all_transforms() -> Vec<Transform> {
        let mut transforms = vec![Transform::None, Transform::ModeSort, Transform::Fields { shuffle: false },
            Transform::Fields { shuffle: true }];
        for element_bytes in [1, 2, 4, 8, 16] {
            for tile_blocks in [None, Some(1), Some(3), Some(32), Some(256)] {
                transforms.push(Transform::Split { element_bytes, tile_blocks });
            }
        }
        transforms.extend(predictions());
        transforms
    }

    #[test]
    fn every_transform_round_trips_every_mode_and_tail() {
        for transform in all_transforms() {
            for (blocks, tail) in [(0, 0), (0, 7), (1, 0), (9, 15), (100, 1), (1000, 0)] {
                for input in [sample(blocks, tail, (blocks * 31 + tail) as u64), runs(blocks, tail, blocks as u64)] {
                    for blocks_wide in [1, 3, TEST_BLOCKS_WIDE, 64] {
                        let (mut forward, mut back) = (Vec::new(), Vec::new());
                        transform.forward(&input, blocks_wide, &mut forward).unwrap();
                        if transform.keeps_length() {
                            assert_eq!(forward.len(), input.len());
                        }
                        transform.inverse(&forward, input.len(), blocks_wide, &mut back).unwrap();
                        assert_eq!(back, input, "{} with {blocks} blocks, {tail} tail bytes, {blocks_wide} wide",
                            transform.name());
                    }
                }
            }
        }
    }

    #[test]
    fn residuals_zigzag_both_ways_at_every_width() {
        for bits in 1..=8 {
            let mut seen = std::collections::HashSet::new();
            for residual in 0..1u32 << bits {
                let byte = zigzag(residual, bits);
                assert!(u32::from(byte) <= value_mask(bits));
                assert!(seen.insert(byte), "{bits} bits: {byte} twice");
                assert_eq!(unzigzag(byte, bits), residual, "{bits} bits");
            }
            assert_eq!(zigzag(0, bits), 0);
            if bits > 1 {
                assert_eq!(zigzag(value_mask(bits), bits), 1, "-1 at {bits} bits");
                assert_eq!(zigzag(1, bits), 2, "+1 at {bits} bits");
            }
        }
    }

    #[test]
    fn names_parse_back() {
        for transform in all_transforms() {
            assert_eq!(Transform::parse(&transform.name()), Some(transform));
        }
        for bad in ["split3", "split0", "split1t0", "splitx", "shuffle", "pred", "pred-up", "pred-up-wide",
            "pred-left-bytes", "pred-up-bytes-x", "pred-up-bytes-xi-xi"] {
            assert_eq!(Transform::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn truncated_or_padded_streams_are_rejected() {
        for input in [sample(50, 3, 7), runs(50, 3, 7)] {
            for transform in all_transforms() {
                let (mut forward, mut back) = (Vec::new(), Vec::new());
                transform.forward(&input, TEST_BLOCKS_WIDE, &mut forward).unwrap();
                let mut short = forward.clone();
                short.pop();
                assert!(transform.inverse(&short, input.len(), TEST_BLOCKS_WIDE, &mut back).is_err(), "{}",
                    transform.name());
                let mut long = forward.clone();
                long.push(0);
                assert!(transform.inverse(&long, input.len(), TEST_BLOCKS_WIDE, &mut back).is_err(), "{}",
                    transform.name());
                assert!(transform.inverse(&forward, input.len() + 1, TEST_BLOCKS_WIDE, &mut back).is_err(), "{}",
                    transform.name());
            }
        }
    }

    #[test]
    fn a_corrupt_mode_map_is_rejected() {
        let input = sample(20, 0, 3);
        let mut mapped = vec![Transform::ModeSort, Transform::Fields { shuffle: false }, Transform::Fields { shuffle: true }];
        mapped.extend(predictions());
        for transform in mapped {
            let (mut forward, mut back) = (Vec::new(), Vec::new());
            transform.forward(&input, TEST_BLOCKS_WIDE, &mut forward).unwrap();
            let mut bad = forward.clone();
            bad[2] = MODE_SLOTS as u8;
            assert_eq!(transform.inverse(&bad, input.len(), TEST_BLOCKS_WIDE, &mut back),
                Err(TransformError::ModeMap { block: 2, value: MODE_SLOTS as u8 }), "{}", transform.name());
            // A map whose counts no longer match the stream: one block moved to another mode.
            let mut recounted = forward.clone();
            recounted[0] = if recounted[0] == 6 { 5 } else { 6 };
            assert!(transform.inverse(&recounted, input.len(), TEST_BLOCKS_WIDE, &mut back).is_err(), "{}",
                transform.name());
        }
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
        for transform in [Transform::Fields { shuffle: false },
            Transform::Predict { predictor: Predictor::Up, layout: ResidualLayout::Packed, xor_indices: true }] {
            let (mut forward, mut back) = (Vec::new(), Vec::new());
            transform.forward(&input, TEST_BLOCKS_WIDE, &mut forward).unwrap();
            let last_prefix_byte = blocks + prefix_bytes(6) - 1;
            forward[last_prefix_byte] |= 0x80;
            assert_eq!(transform.inverse(&forward, input.len(), TEST_BLOCKS_WIDE, &mut back),
                Err(TransformError::PaddingBits { block: 0 }), "{}", transform.name());
        }
        // The byte layouts: mode 6's side byte holds two p-bits, then a residual byte per endpoint value.
        for layout in [ResidualLayout::Bytes, ResidualLayout::Planes] {
            let transform = Transform::Predict { predictor: Predictor::Previous, layout, xor_indices: false };
            let (mut forward, mut back) = (Vec::new(), Vec::new());
            transform.forward(&input, TEST_BLOCKS_WIDE, &mut forward).unwrap();
            let mut side = forward.clone();
            side[blocks] |= 0x80;
            assert_eq!(transform.inverse(&side, input.len(), TEST_BLOCKS_WIDE, &mut back),
                Err(TransformError::PaddingBits { block: 0 }), "{}", transform.name());
            // The first residual of block 0: right after the side byte, or after every block's side byte.
            let first_residual = if layout == ResidualLayout::Bytes { blocks + side_bytes(6) } else { blocks + blocks };
            let mut wide = forward.clone();
            wide[first_residual] = 0x80;
            assert_eq!(transform.inverse(&wide, input.len(), TEST_BLOCKS_WIDE, &mut back),
                Err(TransformError::Residual { block: 0, value: 0x80 }), "{}", transform.name());
        }
    }

    /// The blocks of `rects` cut from a whole page, as the game's reader does after the full inverse.
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
            let mut forward = Vec::new();
            Transform::Fields { shuffle: false }.forward(&input, blocks_wide, &mut forward).unwrap();
            let rect_sets = [
                vec![],
                vec![BlockRect { x: 0, y: 0, width: blocks_wide, height: rows }],
                vec![BlockRect { x: 3, y: 2, width: 4, height: 5 }, BlockRect { x: 0, y: 9, width: 13, height: 2 }],
                vec![BlockRect { x: 12, y: 10, width: 1, height: 1 }, BlockRect { x: 5, y: 0, width: 0, height: 3 },
                    BlockRect { x: 1, y: 1, width: 2, height: 2 }],
            ];
            for rects in rect_sets {
                let mut out = Vec::new();
                fields_extract(&forward, input.len(), blocks_wide, &rects, &mut out).unwrap();
                assert_eq!(out, cut(&input, blocks_wide, &rects), "{rects:?}");
            }
        }
    }

    #[test]
    fn extracting_from_a_malformed_stream_fails() {
        let (blocks_wide, rows) = (13, 11);
        let input = runs(blocks_wide * rows, 0, 23);
        let mut forward = Vec::new();
        Transform::Fields { shuffle: false }.forward(&input, blocks_wide, &mut forward).unwrap();
        let rect = [BlockRect { x: 2, y: 3, width: 4, height: 4 }];
        let mut out = Vec::new();
        for bad in [BlockRect { x: 10, y: 0, width: 4, height: 1 }, BlockRect { x: 0, y: 10, width: 1, height: 2 }] {
            assert_eq!(fields_extract(&forward, input.len(), blocks_wide, &[bad], &mut out),
                Err(TransformError::Rect { x: bad.x, y: bad.y, width: bad.width, height: bad.height }));
        }
        // A grid that does not hold the page.
        assert!(fields_extract(&forward, input.len(), blocks_wide + 1, &rect, &mut out).is_err());
        assert!(fields_extract(&forward, input.len(), 0, &rect, &mut out).is_err());
        let mut short = forward.clone();
        short.pop();
        assert!(fields_extract(&short, input.len(), blocks_wide, &rect, &mut out).is_err());
        let mut long = forward.clone();
        long.push(0);
        assert!(fields_extract(&long, input.len(), blocks_wide, &rect, &mut out).is_err());
        // A bad entry far from the rectangle is still found: the whole map is checked.
        let mut bad_map = forward.clone();
        let far = blocks_wide * rows - 1;
        bad_map[far] = MODE_SLOTS as u8;
        assert_eq!(fields_extract(&bad_map, input.len(), blocks_wide, &rect, &mut out),
            Err(TransformError::ModeMap { block: far, value: MODE_SLOTS as u8 }));
        // Padding bits in a block the rectangle joins.
        let mode_6 = mode_6_blocks();
        let mut forward_6 = Vec::new();
        Transform::Fields { shuffle: false }.forward(&mode_6, 1, &mut forward_6).unwrap();
        let blocks = mode_6.len() / BLOCK_BYTES;
        forward_6[blocks + prefix_bytes(6) - 1] |= 0x80;
        assert_eq!(fields_extract(&forward_6, mode_6.len(), 1, &[BlockRect { x: 0, y: 0, width: 1, height: 1 }],
            &mut out), Err(TransformError::PaddingBits { block: 0 }));
    }
}
