//! Lossy conditioning of a BC7 atlas page of animation frames: a chain of steps that change the encoded blocks so the
//! page codes smaller after them, each bounded against the source pixels and never moving a texel's alpha from the
//! plain encode's (except the library RDO step, whose alpha changes the caller counts). The page decodes as any BC7
//! page; only the bytes of the blocks change. The steps and their measurements are in
//! `docs/agent_notes/bc7_anim_supercompression.md`, "Lossy conditioning"; the mobile animation packers apply a chain
//! after the encode, and the study harness (`udd-bc7-supercompress lossy`) runs every step.
//!
//! The steps work on the modes 4, 5 and 6 the encoder writes for these pages ([`super::block_fields`]), on one page at
//! a time and in raster order of its blocks, so the result depends only on the page, its blocks and its placements.
//! - [`Step::Rdo`]: [`super::rdo`] on one window over the whole page.
//! - [`Step::Reuse`]: a block takes the block of an adjacent frame of its clip at the same place relative to the frame
//!   anchor (or up to `radius` blocks beside it), when that block comes earlier on the page and its error over the
//!   visible texels is at most `rms` levels above the plain encode's; the candidate at the offset the block before it
//!   used wins a tie, so rows of taken blocks stay one match.
//! - [`Step::Zrdo`]: whole-block reuse priced by a model of the coder's sequences: candidates at the last match offset,
//!   to the left, above, in adjacent frames (as `Reuse`) and in a hash of earlier blocks of the same mode and mean; the
//!   cheapest of distortion plus `lambda` times the estimated bits, within `rms`.
//! - [`Step::Fields`]: the same pricing for the BC7 field split (`udd_container::bc7_fields`): a block takes the endpoint
//!   fields of an earlier block of its mode (its indices picked again), or its index bits (its endpoints fitted again),
//!   so one of its two records repeats in its mode's stream.
//! - [`Step::Coarse`]: colour endpoint values rounded to multiples of `1 << bits`, indices picked again.
//! - [`Step::Flat`]: an opaque block within `rms` of its mean becomes one solid mode 6 block with zero indices.
//!
//! Each step is bounded against the plain encode's error (the blocks the conditioner was made with), so a chain does
//! not compound its bound.

use std::collections::HashMap;
use std::fmt;

use super::block_fields::{Block, MODE_4, MODE_6, Rgba, TEXELS};
use super::rdo::{Bc7RdoParams, reduce_entropy_bc7_with_progress};
use super::{BC7_BLOCK_BYTES, BC7_BLOCK_EDGE, ImageExtent, decode_bc7_to_rgba8888};

const RGBA_BYTES: usize = 4;
const ALPHA: usize = 3;
const BLOCK_EDGE: usize = BC7_BLOCK_EDGE as usize;
const BLOCK_BITS: u32 = 128;
/// Index bits of modes 4, 5 and 6.
const INDEX_RECORD_BITS: [u32; 3] = [78, 62, 63];
/// Levels a step may move a texel's alpha from the plain encode's: none, so silhouettes and coverage stay as they are.
const ALPHA_TOLERANCE: u8 = 0;
/// The model of Zstd's costs the coder-aware steps price a block with, in bits: a literal byte of BC7 data (its
/// Huffman code over nearly random bytes), the codes of a new match beyond its offset bits, and a match that only
/// extends the one before it at the same offset (a repeat offset).
const LITERAL_BYTE_BITS: f64 = 7.0;
const MATCH_OVERHEAD_BITS: f64 = 12.0;
const REPEAT_MATCH_BITS: f64 = 2.0;
/// The LZ4 block format: literals are stored raw, a sequence's token is one byte, a match's offset two, a literal run
/// or match longer than the token's 4 bits holds adds a byte per 255, and a match reaches back at most 65 535 bytes.
const LZ4_BYTE_BITS: f64 = 8.0;
const LZ4_TOKEN_BITS: f64 = 8.0;
const LZ4_OFFSET_BITS: f64 = 16.0;
const LZ4_LENGTH_BYTE_SPAN: f64 = 255.0;
const LZ4_WINDOW_BYTES: usize = 65_535;
/// Earlier blocks of one mode and mean kept per hash bucket by `Zrdo`, and the bits of the mean per channel.
const BUCKET_DEPTH: usize = 8;
const BUCKET_MEAN_SHIFT: u32 = 4;
/// A temporal candidate's reach beside the co-located block, in blocks: `Zrdo`'s, and `Reuse`'s default.
pub const DEFAULT_REUSE_RADIUS: i32 = 1;
/// `Fields` takes only the co-located block of an adjacent frame.
const FIELDS_REUSE_RADIUS: i32 = 0;
const BITS_PER_BYTE: u32 = 8;
const MAX_ENDPOINT: f64 = 127.0;
const PBIT_HALF: f64 = 2.0;

/// The coder a coder-aware step prices its matches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coder {
    Zstd,
    Lz4,
}

/// One step of a conditioning chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// The library RDO at `lambda`, its look-back `lookback` blocks, or the whole page when `None`.
    Rdo { lambda: f32, lookback: Option<usize> },
    /// Temporal block reuse within `rms` levels, up to `radius` blocks beside the co-located block.
    Reuse { rms: f64, radius: i32 },
    /// Coder-priced whole-block reuse within `rms` levels.
    Zrdo { lambda: f64, rms: f64, coder: Coder },
    /// Field-split-priced record reuse within `rms` levels.
    Fields { lambda: f64, rms: f64, coder: Coder },
    /// Colour endpoints rounded to multiples of `1 << bits`, within `rms` levels.
    Coarse { bits: u32, rms: f64 },
    /// Solid mode 6 blocks within `rms` levels.
    Flat { rms: f64 },
}

/// A frame's pixels on the page: its allocation (the content padded to blocks and the gutter on each side, in pixels,
/// block-aligned; the blocks it covers are the frame's) and its anchor, in page pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub allocation: (u32, u32, u32, u32),
    pub anchor: (i32, i32),
}

impl Placement {
    /// A frame whose content is `width` × `height` at the block corner `x`, `y`, with `gutter` pixels around it
    /// rounded up to blocks (cut at the page's top and left edges), and its anchor `anchor` pixels from the content
    /// corner.
    pub fn in_gutter(x: u32, y: u32, width: u32, height: u32, gutter: u16, anchor: (i32, i32)) -> Self {
        let lead = u32::from(gutter).next_multiple_of(BC7_BLOCK_EDGE);
        Self {
            allocation: (x.saturating_sub(lead), y.saturating_sub(lead), width.next_multiple_of(BC7_BLOCK_EDGE) + 2 * lead,
                height.next_multiple_of(BC7_BLOCK_EDGE) + 2 * lead),
            anchor: (x as i32 + anchor.0, y as i32 + anchor.1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionError {
    /// The blocks or the pixels do not cover the page.
    Length { blocks: usize, pixels: usize, expected_blocks: usize },
    /// An adjacency list names a placement the page does not have.
    Adjacency { placement: usize, neighbour: usize },
}

impl fmt::Display for ConditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { blocks, pixels, expected_blocks } => write!(f,
                "{blocks} bytes of blocks and {pixels} bytes of pixels for a page of {expected_blocks} blocks"),
            Self::Adjacency { placement, neighbour } => write!(f,
                "placement {placement} lists neighbour {neighbour}, which the page does not have"),
        }
    }
}

impl std::error::Error for ConditionError {}

/// A page ready for the steps: its blocks' source pixels, the plain encode and its decode, and which placement owns
/// each block. `adjacent[p]` lists the placements of the page that come before or after placement `p` in one of its
/// clips, with a tag of the caller's (the study names the clip's kind) handed back for blocks taken from them.
pub struct PageConditioner<'a, T = ()> {
    wide: usize,
    pixels: Vec<[Rgba; TEXELS]>,
    base: Vec<[u8; 16]>,
    base_decoded: Vec<[Rgba; TEXELS]>,
    base_error: Vec<(u32, u32)>,
    owner: Vec<Option<u32>>,
    /// Per placement: the block holding its anchor (relative coordinates are against it).
    anchor_block: Vec<(i32, i32)>,
    adjacent: &'a [Vec<(usize, T)>],
}

/// The pixels in block order: each block's 16 texels, row by row; the texels of a partial block past the page's edge
/// are transparent (no step changes them, as no step moves alpha).
fn block_pixels(rgba: &[u8], width: u32, height: u32) -> Vec<[Rgba; TEXELS]> {
    let (wide, high) = (width.div_ceil(BC7_BLOCK_EDGE) as usize, height.div_ceil(BC7_BLOCK_EDGE) as usize);
    (0..wide * high).map(|block| {
        let (bx, by) = (block % wide, block / wide);
        std::array::from_fn(|texel| {
            let (x, y) = (bx * BLOCK_EDGE + texel % BLOCK_EDGE, by * BLOCK_EDGE + texel / BLOCK_EDGE);
            if x >= width as usize || y >= height as usize {
                return [0; RGBA_BYTES];
            }
            let at = (y * width as usize + x) * RGBA_BYTES;
            rgba[at..at + RGBA_BYTES].try_into().expect("a pixel")
        })
    }).collect()
}

fn decode_block(bytes: &[u8; 16]) -> [Rgba; TEXELS] {
    if let Some(block) = Block::unpack(bytes) {
        return block.decode();
    }
    let extent = ImageExtent::new(BC7_BLOCK_EDGE, BC7_BLOCK_EDGE).expect("a block");
    let rgba = decode_bc7_to_rgba8888(bytes, extent).expect("a block decodes");
    std::array::from_fn(|texel| rgba[texel * RGBA_BYTES..][..RGBA_BYTES].try_into().expect("a pixel"))
}

/// The squared error over the visible texels (source alpha above 0), all four channels, and their count.
fn visible_error(decoded: &[Rgba; TEXELS], pixels: &[Rgba; TEXELS]) -> (u32, u32) {
    let mut error = 0;
    let mut count = 0;
    for (a, b) in decoded.iter().zip(pixels) {
        if b[ALPHA] == 0 {
            continue;
        }
        count += 1;
        error += (0..RGBA_BYTES).map(|channel| (i32::from(a[channel]) - i32::from(b[channel])).unsigned_abs().pow(2))
            .sum::<u32>();
    }
    (error, count)
}

fn rms(error: u32, count: u32) -> f64 {
    if count == 0 { 0.0 } else { (f64::from(error) / f64::from(count * RGBA_BYTES as u32)).sqrt() }
}

/// Whether `candidate` keeps every texel's alpha as `reference` decodes it.
fn same_alpha(candidate: &[Rgba; TEXELS], reference: &[Rgba; TEXELS]) -> bool {
    candidate.iter().zip(reference).all(|(a, b)| a[ALPHA].abs_diff(b[ALPHA]) <= ALPHA_TOLERANCE)
}

impl<'a, T: Copy> PageConditioner<'a, T> {
    /// The page `width` × `height` with its source pixels `rgba` (RGBA8888, rows of `width`), its plain
    /// encode `blocks` and its placements; `adjacent` has one list per placement.
    pub fn new(rgba: &[u8], width: u32, height: u32, blocks: &[u8], placements: &[Placement],
        adjacent: &'a [Vec<(usize, T)>]) -> Result<Self, ConditionError> {
        let (wide, high) = (width.div_ceil(BC7_BLOCK_EDGE) as usize, height.div_ceil(BC7_BLOCK_EDGE) as usize);
        if blocks.len() != wide * high * BC7_BLOCK_BYTES || rgba.len() != (width * height) as usize * RGBA_BYTES {
            return Err(ConditionError::Length { blocks: blocks.len(), pixels: rgba.len(), expected_blocks: wide * high });
        }
        for (placement, list) in adjacent.iter().enumerate() {
            if let Some(&(neighbour, _)) = list.iter().find(|(neighbour, _)| *neighbour >= placements.len()) {
                return Err(ConditionError::Adjacency { placement, neighbour });
            }
        }
        let base: Vec<[u8; 16]> = blocks.chunks_exact(BC7_BLOCK_BYTES).map(|block| block.try_into().expect("a block"))
            .collect();
        let pixels = block_pixels(rgba, width, height);
        let base_decoded: Vec<[Rgba; TEXELS]> = base.iter().map(decode_block).collect();
        let base_error = base_decoded.iter().zip(&pixels).map(|(a, b)| visible_error(a, b)).collect();
        let mut owner = vec![None; base.len()];
        let side = BC7_BLOCK_EDGE as i32;
        let mut anchor_block = Vec::with_capacity(placements.len());
        for (at, placement) in placements.iter().enumerate() {
            let (rx, ry, rw, rh) = placement.allocation;
            for by in ry / BC7_BLOCK_EDGE..((ry + rh) / BC7_BLOCK_EDGE).min(high as u32) {
                for bx in rx / BC7_BLOCK_EDGE..((rx + rw) / BC7_BLOCK_EDGE).min(wide as u32) {
                    owner[by as usize * wide + bx as usize] = Some(at as u32);
                }
            }
            anchor_block.push((placement.anchor.0.div_euclid(side), placement.anchor.1.div_euclid(side)));
        }
        Ok(Self { wide, pixels, base, base_decoded, base_error, owner, anchor_block, adjacent })
    }

    /// The plain encode's blocks, the start of a chain.
    pub fn base_blocks(&self) -> Vec<[u8; 16]> {
        self.base.clone()
    }

    /// Whether `block` has visible texels (source alpha above 0): the blocks the steps may change.
    pub fn visible(&self, block: usize) -> bool {
        self.base_error[block].1 > 0
    }

    /// The placement whose allocation covers `block`.
    pub fn owner(&self, block: usize) -> Option<usize> {
        self.owner[block].map(|placement| placement as usize)
    }

    fn high(&self) -> usize {
        self.base.len() / self.wide
    }

    /// The plain encode with every step of `steps` applied in turn, as bytes.
    pub fn condition(&self, steps: &[Step]) -> Vec<u8> {
        let mut blocks = self.base_blocks();
        for &step in steps {
            self.apply(step, &mut blocks, &mut |_, _| {});
        }
        blocks.iter().flatten().copied().collect()
    }

    /// Applies `step` to `blocks`, calling `taken` with each block a step other than the RDO changes and the tag of
    /// the adjacency it was taken through, when it was; returns the blocks the RDO changed.
    pub fn apply(&self, step: Step, blocks: &mut [[u8; 16]], taken: &mut dyn FnMut(usize, Option<T>)) -> u32 {
        match step {
            Step::Rdo { lambda, lookback } => {
                let lookback = lookback.unwrap_or(blocks.len());
                let params = Bc7RdoParams { lambda, lookback_window_size: lookback * BC7_BLOCK_BYTES,
                    skip_zero_mse_blocks: true, ..Default::default() };
                let pixels: Vec<[u8; 4]> = self.pixels.iter().flatten().copied().collect();
                // One window over the whole page, on the caller's thread (the packer's RDO splits a page into row
                // chunks of at least the look-back each, which changes a few blocks at the chunk edges).
                return reduce_entropy_bc7_with_progress(blocks, &pixels, self.wide, self.high(), &params, |_| {});
            }
            Step::Reuse { rms, radius } => self.reuse(blocks, rms, radius, taken),
            Step::Zrdo { lambda, rms, coder } => self.zrdo(blocks, lambda, rms, coder.model(), taken),
            Step::Fields { lambda, rms, coder } => self.fields(blocks, lambda, rms, coder.model(), taken),
            Step::Coarse { bits, rms } => {
                for block in 0..blocks.len() {
                    let Some(mut fields) = Block::unpack(&blocks[block]).filter(|_| self.visible(block)) else { continue };
                    fields.coarsen_endpoints(bits, &self.pixels[block]);
                    if self.accept(block, &fields.decode(), rms).is_some() {
                        blocks[block] = fields.pack();
                        taken(block, None);
                    }
                }
            }
            Step::Flat { rms } => {
                for block in 0..blocks.len() {
                    if !self.visible(block) {
                        continue;
                    }
                    let Some(solid) = solid_block(&self.pixels[block]) else { continue };
                    if self.accept(block, &solid.decode(), rms).is_some() {
                        blocks[block] = solid.pack();
                        taken(block, None);
                    }
                }
            }
        }
        0
    }

    /// Earlier blocks of the adjacent frames at `block`'s place relative to its anchor, within `radius`, with the
    /// adjacency's tag.
    fn temporal(&self, block: usize, radius: i32, out: &mut Vec<(usize, T)>) {
        out.clear();
        let Some(slot) = self.owner[block] else { return };
        let slot = slot as usize;
        let (ax, ay) = self.anchor_block[slot];
        let (bx, by) = ((block % self.wide) as i32, (block / self.wide) as i32);
        for &(other, tag) in &self.adjacent[slot] {
            let (ox, oy) = self.anchor_block[other];
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (x, y) = (bx - ax + ox + dx, by - ay + oy + dy);
                    if x < 0 || y < 0 || x as usize >= self.wide || y as usize >= self.high() {
                        continue;
                    }
                    let candidate = y as usize * self.wide + x as usize;
                    if candidate < block && self.owner[candidate] == Some(other as u32) {
                        out.push((candidate, tag));
                    }
                }
            }
        }
    }

    fn accept(&self, block: usize, decoded: &[Rgba; TEXELS], limit: f64) -> Option<u32> {
        if !same_alpha(decoded, &self.base_decoded[block]) {
            return None;
        }
        let (error, count) = visible_error(decoded, &self.pixels[block]);
        let (base_error, _) = self.base_error[block];
        (rms(error, count) <= rms(base_error, count) + limit).then_some(error)
    }

    fn reuse(&self, blocks: &mut [[u8; 16]], limit: f64, radius: i32, taken: &mut dyn FnMut(usize, Option<T>)) {
        let mut candidates = Vec::new();
        let mut last_offset = 0usize;
        for block in 0..blocks.len() {
            if !self.visible(block) {
                continue;
            }
            self.temporal(block, radius, &mut candidates);
            let mut best: Option<(bool, u32, usize, T)> = None;
            for &(candidate, tag) in &candidates {
                if blocks[candidate] == blocks[block] {
                    best = Some((true, 0, candidate, tag));
                    break;
                }
                let Some(error) = self.accept(block, &decode_block(&blocks[candidate]), limit) else { continue };
                let repeat = block - candidate == last_offset;
                if best.is_none_or(|(best_repeat, best_error, _, _)| (repeat, std::cmp::Reverse(error))
                    > (best_repeat, std::cmp::Reverse(best_error))) {
                    best = Some((repeat, error, candidate, tag));
                }
            }
            if let Some((_, _, candidate, tag)) = best {
                if blocks[candidate] != blocks[block] {
                    taken(block, Some(tag));
                }
                blocks[block] = blocks[candidate];
                last_offset = block - candidate;
            }
        }
    }

    fn zrdo(&self, blocks: &mut [[u8; 16]], lambda: f64, limit: f64, model: CoderModel,
        taken: &mut dyn FnMut(usize, Option<T>)) {
        let mut buckets: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut temporal = Vec::new();
        let mut last_offset = 0usize;
        let mut previous_matched = false;
        let literal_bits = BC7_BLOCK_BYTES as f64 * model.literal_byte;
        let match_bits = |offset: usize, extends: bool| -> f64 {
            if extends {
                model.extend
            } else {
                model.match_overhead + model.offset_log_weight * ((offset * BC7_BLOCK_BYTES) as f64).log2()
            }
        };
        for block in 0..blocks.len() {
            if self.visible(block) {
                let key = bucket_key(&self.pixels[block], &blocks[block]);
                let mut candidates: Vec<(usize, Option<T>)> = Vec::new();
                if last_offset > 0 && last_offset <= block {
                    candidates.push((block - last_offset, None));
                }
                if block >= 1 {
                    candidates.push((block - 1, None));
                }
                if block >= self.wide {
                    candidates.push((block - self.wide, None));
                }
                self.temporal(block, DEFAULT_REUSE_RADIUS, &mut temporal);
                candidates.extend(temporal.iter().map(|&(candidate, tag)| (candidate, Some(tag))));
                if let Some(bucket) = buckets.get(&key) {
                    candidates.extend(bucket.iter().map(|&candidate| (candidate, None)));
                }
                candidates.retain(|&(candidate, _)| (block - candidate) * BC7_BLOCK_BYTES <= model.window);
                let (own_error, _) = visible_error(&decode_block(&blocks[block]), &self.pixels[block]);
                // The block as it is: a literal, or a match when an earlier block is the same.
                let own_match = candidates.iter().find(|&&(candidate, _)| blocks[candidate] == blocks[block]);
                let own_bits = own_match.map_or(literal_bits, |&(candidate, _)| {
                    let offset = block - candidate;
                    match_bits(offset, previous_matched && offset == last_offset)
                });
                let mut best = (f64::from(own_error) + lambda * own_bits, None::<(usize, Option<T>)>);
                if own_match.is_none() {
                    for &(candidate, tag) in &candidates {
                        let Some(error) = self.accept(block, &decode_block(&blocks[candidate]), limit) else { continue };
                        let offset = block - candidate;
                        let cost = f64::from(error) + lambda * match_bits(offset, previous_matched && offset == last_offset);
                        if cost < best.0 {
                            best = (cost, Some((candidate, tag)));
                        }
                    }
                }
                match (best.1, own_match) {
                    (Some((candidate, tag)), _) => {
                        blocks[block] = blocks[candidate];
                        taken(block, tag);
                        last_offset = block - candidate;
                        previous_matched = true;
                    }
                    (None, Some(&(candidate, _))) => {
                        last_offset = block - candidate;
                        previous_matched = true;
                    }
                    (None, None) => previous_matched = false,
                }
            } else if block > 0 && blocks[block - 1] == blocks[block] {
                // A run of the one transparent encoding: a match at the offset of one block.
                last_offset = 1;
                previous_matched = true;
            } else {
                previous_matched = false;
            }
            let key = bucket_key(&self.pixels[block], &blocks[block]);
            let bucket = buckets.entry(key).or_default();
            if bucket.len() == BUCKET_DEPTH {
                bucket.remove(0);
            }
            bucket.push(block);
        }
    }

    fn fields(&self, blocks: &mut [[u8; 16]], lambda: f64, limit: f64, model: CoderModel,
        taken: &mut dyn FnMut(usize, Option<T>)) {
        // The last block of each mode (and mode 4 index selector) in raster order: the previous record of its stream.
        let mut previous: HashMap<(u8, u8), usize> = HashMap::new();
        let mut temporal = Vec::new();
        for block in 0..blocks.len() {
            let Some(own) = Block::unpack(&blocks[block]) else { continue };
            let stream = (own.mode, own.index_mode);
            if !self.visible(block) {
                previous.insert(stream, block);
                continue;
            }
            let (field_bits, index_bits) = record_bits(&own);
            let mut candidates: Vec<usize> = previous.get(&stream).copied().into_iter().collect();
            if block >= 1 {
                candidates.push(block - 1);
            }
            if block >= self.wide {
                candidates.push(block - self.wide);
            }
            self.temporal(block, FIELDS_REUSE_RADIUS, &mut temporal);
            candidates.extend(temporal.iter().map(|&(candidate, _)| candidate));
            // A record's stream holds at most one record a block, so a block's distance bounds the record's.
            candidates.retain(|&candidate| (block - candidate) * (field_bits.div_ceil(BITS_PER_BYTE) as usize)
                <= model.window);
            let (own_error, _) = visible_error(&own.decode(), &self.pixels[block]);
            let literal = |bits: u32| f64::from(bits.div_ceil(BITS_PER_BYTE)) * model.literal_byte;
            let mut best = (f64::from(own_error) + lambda * (literal(field_bits) + literal(index_bits)), None::<Block>);
            for &candidate in &candidates {
                let Some(other) = Block::unpack(&blocks[candidate]) else { continue };
                if (other.mode, other.index_mode) != stream {
                    continue;
                }
                // Its endpoint fields, the indices picked again.
                let mut endpoints = other;
                endpoints.pick_indices(&self.pixels[block]);
                // Its indices, the endpoints fitted again (modes 4 and 5 keep their own rotation and alpha endpoints,
                // so only a candidate of the same rotation counts).
                let mut indices = own;
                indices.colour_index = other.colour_index;
                indices.alpha_index = other.alpha_index;
                indices.fit_endpoints(&self.pixels[block]);
                let indices_kept = other.rotation == own.rotation && indices.colour_index == other.colour_index
                    && indices.alpha_index == other.alpha_index;
                for (trial, bits) in [(endpoints, model.record_match + literal(index_bits)),
                    (indices, literal(field_bits) + if indices_kept { model.record_match } else { literal(index_bits) })] {
                    let Some(error) = self.accept(block, &trial.decode(), limit) else { continue };
                    let cost = f64::from(error) + lambda * bits;
                    if cost < best.0 {
                        best = (cost, Some(trial));
                    }
                }
            }
            if let Some(trial) = best.1 {
                blocks[block] = trial.pack();
                taken(block, None);
            }
            let mode = Block::unpack(&blocks[block]).map_or(stream, |fields| (fields.mode, fields.index_mode));
            previous.insert(mode, block);
        }
    }
}

/// A coder's costs in bits, as the coder-aware steps price a block or a field-split record.
#[derive(Debug, Clone, Copy)]
struct CoderModel {
    literal_byte: f64,
    /// A new match: its fixed codes, plus this weight times log2 of its offset in bytes.
    match_overhead: f64,
    offset_log_weight: f64,
    /// A block's worth of a match that extends the one before it at the same offset.
    extend: f64,
    /// A field-split record taken whole from an earlier record of its stream.
    record_match: f64,
    /// The farthest a match reaches back, in bytes.
    window: usize,
}

impl Coder {
    fn model(self) -> CoderModel {
        match self {
            Self::Zstd => CoderModel { literal_byte: LITERAL_BYTE_BITS, match_overhead: MATCH_OVERHEAD_BITS,
                offset_log_weight: 1.0, extend: REPEAT_MATCH_BITS, record_match: MATCH_OVERHEAD_BITS, window: usize::MAX },
            // A literal run's length bytes add one per 255 literals; a new match is a token and an offset (the block's
            // 16 bytes fit the token's match length), and extending one adds one length byte per 255 bytes.
            Self::Lz4 => CoderModel { literal_byte: LZ4_BYTE_BITS + LZ4_BYTE_BITS / LZ4_LENGTH_BYTE_SPAN,
                match_overhead: LZ4_TOKEN_BITS + LZ4_OFFSET_BITS, offset_log_weight: 0.0,
                extend: BC7_BLOCK_BYTES as f64 * LZ4_BYTE_BITS / LZ4_LENGTH_BYTE_SPAN,
                record_match: LZ4_TOKEN_BITS + LZ4_OFFSET_BITS, window: LZ4_WINDOW_BYTES },
        }
    }
}

/// The bits of a block's two field-split records: the fields between the mode bits and the indices, and the indices.
fn record_bits(block: &Block) -> (u32, u32) {
    let index = INDEX_RECORD_BITS[usize::from(block.mode - MODE_4)];
    (BLOCK_BITS - index - u32::from(block.mode) - 1, index)
}

fn bucket_key(pixels: &[Rgba; TEXELS], bytes: &[u8; 16]) -> u32 {
    let mut mean = [0u32; RGBA_BYTES];
    for pixel in pixels {
        for channel in 0..RGBA_BYTES {
            mean[channel] += u32::from(pixel[channel]);
        }
    }
    let mode = u32::from(Block::mode_of(bytes).unwrap_or(0));
    mean.iter().fold(mode, |key, &sum| (key << 8) ^ ((sum / TEXELS as u32) >> BUCKET_MEAN_SHIFT))
}

/// An opaque block (every texel's alpha the same) as one mode 6 colour: the mean, its p-bit the one nearer.
fn solid_block(pixels: &[Rgba; TEXELS]) -> Option<Block> {
    let alpha = pixels[0][ALPHA];
    if pixels.iter().any(|pixel| pixel[ALPHA] != alpha) {
        return None;
    }
    let mean: [f64; RGBA_BYTES] = std::array::from_fn(|channel| {
        pixels.iter().map(|pixel| f64::from(pixel[channel])).sum::<f64>() / TEXELS as f64
    });
    let best = (0..2u8).map(|pbit| {
        let values: [u8; RGBA_BYTES] = std::array::from_fn(|channel| {
            (((mean[channel] - f64::from(pbit)) / PBIT_HALF).round()).clamp(0.0, MAX_ENDPOINT) as u8
        });
        let error: f64 = (0..RGBA_BYTES).map(|channel| (f64::from(values[channel] << 1 | pbit) - mean[channel]).powi(2)).sum();
        (error, pbit, values)
    }).min_by(|a, b| a.0.total_cmp(&b.0))?;
    let (_, pbit, values) = best;
    Some(Block { mode: MODE_6, rotation: 0, index_mode: 0, colour: [[values[0], values[1], values[2]]; 2],
        alpha: [values[ALPHA]; 2], pbits: [pbit; 2], colour_index: [0; TEXELS], alpha_index: [0; TEXELS] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bc7::{Bc7EncoderBackend, RawImageFormat, encode_to_bc7};

    const SIDE: u32 = 32;
    const GUTTER: u16 = 4;
    const FRAME_SIDE: u32 = 8;
    const FRAME_GAP: u32 = 16;

    /// Two frames of one clip side by side, the second a slightly changed copy of the first, on a transparent page.
    fn page() -> Vec<u8> {
        let mut rgba = vec![0u8; (SIDE * SIDE) as usize * RGBA_BYTES];
        for frame in 0..2u32 {
            for y in 0..FRAME_SIDE {
                for x in 0..FRAME_SIDE {
                    let at = (((GUTTER as u32 + y) * SIDE + GUTTER as u32 + frame * FRAME_GAP + x) as usize) * RGBA_BYTES;
                    let shade = (x * 24 + y * 7 + frame) as u8;
                    rgba[at..at + RGBA_BYTES].copy_from_slice(&[shade, shade / 2, 200 - shade / 3, u8::MAX]);
                }
            }
        }
        rgba
    }

    fn placements() -> Vec<Placement> {
        (0..2).map(|frame| Placement::in_gutter(u32::from(GUTTER) + frame * FRAME_GAP, u32::from(GUTTER), FRAME_SIDE,
            FRAME_SIDE, GUTTER, (FRAME_SIDE as i32 / 2, FRAME_SIDE as i32))).collect()
    }

    #[test]
    fn steps_keep_alpha_and_the_page_decodes() {
        let rgba = page();
        let extent = ImageExtent::new(SIDE, SIDE).unwrap();
        let blocks = encode_to_bc7(&rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap().into_blocks();
        let adjacent = vec![vec![(1, ())], vec![(0, ())]];
        let conditioner = PageConditioner::new(&rgba, SIDE, SIDE, &blocks, &placements(), &adjacent).unwrap();
        let plain = decode_bc7_to_rgba8888(&blocks, extent).unwrap();
        for steps in [&[Step::Fields { lambda: 30.0, rms: 3.0, coder: Coder::Zstd }][..],
            &[Step::Coarse { bits: 2, rms: 4.0 }, Step::Zrdo { lambda: 100.0, rms: 6.0, coder: Coder::Zstd }],
            &[Step::Flat { rms: 8.0 }, Step::Reuse { rms: 4.0, radius: DEFAULT_REUSE_RADIUS }]] {
            let conditioned = conditioner.condition(steps);
            assert_eq!(conditioned.len(), blocks.len());
            let decoded = decode_bc7_to_rgba8888(&conditioned, extent).unwrap();
            for (a, b) in decoded.chunks_exact(RGBA_BYTES).zip(plain.chunks_exact(RGBA_BYTES)) {
                assert_eq!(a[ALPHA], b[ALPHA], "{steps:?}");
            }
            // The same input gives the same blocks.
            assert_eq!(conditioner.condition(steps), conditioned, "{steps:?}");
        }
        // A page of partial edge blocks.
        let (width, height) = (SIDE - 2, SIDE - 1);
        let cut: Vec<u8> = (0..height as usize).flat_map(|y| {
            let row = y * SIDE as usize * RGBA_BYTES;
            rgba[row..row + width as usize * RGBA_BYTES].to_vec()
        }).collect();
        let extent = ImageExtent::new(width, height).unwrap();
        let blocks = encode_to_bc7(&cut, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap().into_blocks();
        let conditioner = PageConditioner::new(&cut, width, height, &blocks, &placements(), &adjacent).unwrap();
        let steps = [Step::Coarse { bits: 2, rms: 4.0 }, Step::Zrdo { lambda: 100.0, rms: 6.0, coder: Coder::Zstd }];
        assert_eq!(conditioner.condition(&steps).len(), blocks.len());
    }

    #[test]
    fn a_placement_owns_its_allocation_and_bad_input_is_refused() {
        let rgba = page();
        let blocks = vec![0u8; (SIDE / BC7_BLOCK_EDGE * SIDE / BC7_BLOCK_EDGE) as usize * BC7_BLOCK_BYTES];
        let adjacent: Vec<Vec<(usize, ())>> = vec![Vec::new(), Vec::new()];
        let conditioner = PageConditioner::new(&rgba, SIDE, SIDE, &blocks, &placements(), &adjacent).unwrap();
        let wide = (SIDE / BC7_BLOCK_EDGE) as usize;
        // The first frame's allocation is blocks 0..4 of rows 0..4 (gutter, content, gutter), the second's 4..8.
        assert_eq!(conditioner.owner(0), Some(0));
        assert_eq!(conditioner.owner(3 * wide + 3), Some(0));
        assert_eq!(conditioner.owner(4), Some(1));
        assert_eq!(conditioner.owner(4 * wide), None);
        assert!(PageConditioner::new(&rgba, SIDE - BC7_BLOCK_EDGE, SIDE, &blocks, &placements(), &adjacent).is_err());
        assert!(PageConditioner::new(&rgba, SIDE, SIDE, &blocks[1..], &placements(), &adjacent).is_err());
        let wrong = vec![vec![(2, ())], Vec::new()];
        assert!(PageConditioner::new(&rgba, SIDE, SIDE, &blocks, &placements(), &wrong).is_err());
    }
}
