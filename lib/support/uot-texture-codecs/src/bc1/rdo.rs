//! Rate-distortion optimisation of BC1 blocks: the entropy reduction transform of `bc7enc_rdo` (`ert.cpp`), as
//! `bc7::rdo` ports it for BC7, on BC1's 8-byte blocks.
//!
//! Each block, in order, may take a run of 3 to 8 bytes from one of the blocks before it, at the same offset, when the
//! error that adds costs less than the bits an LZ coder saves by matching the run. Lambda weighs the two. The costs,
//! the smooth-block and ultrasmooth error scales, and the cheaper match that continues the last one or repeats its
//! distance are the BC7 port's; the smooth-block scale ramps up over lambda / 8, as `rdo_bc_encoder.cpp` does for BC1.
//! Unlike that encoder, which weighs RGB only, the error counts alpha too: these blocks use BC1's punch-through
//! alpha, so a texel turning transparent or opaque is an error, measured as RGBA against what BC1 can draw.

use rayon::prelude::*;

use crate::bc7::rdo::{
    compute_block_mse_scales, compute_dist_cost_estimate, compute_match_len_cost, lerp, max_trial_error,
    max_trial_error_scale, parallel_rdo_chunk_blocks, rdo_hash_seen, rgba_block_at, smooth_block_error_scale_from_pixels,
    RgbaBlock, LITERAL_BITS, MATCH_CONTINUE_BITS, MATCH_REP0_BITS, PARALLEL_RDO_BLOCK_THRESHOLD,
};

use super::BC1_BLOCK_BYTES;

/// The shortest run worth matching, as in `ert.cpp`.
const MIN_MATCH_BYTES: usize = 3;
/// Squared error is averaged over a block's 16 pixels of 4 channels.
const BLOCK_CHANNELS: f32 = 64.0;
/// Below this alpha BC1 draws a pixel transparent, so the error is measured against transparent black.
const ALPHA_HALF: u8 = 128;
/// The automatic smooth-block error scale at lambda 0 and from lambda `SMOOTH_SCALE_LAMBDA_SPAN` up, as for BC1 in
/// `rdo_bc_encoder.cpp`.
const SMOOTH_SCALE_MIN: f32 = 15.0;
const SMOOTH_SCALE_MAX: f32 = 50.0;
const SMOOTH_SCALE_LAMBDA_SPAN: f32 = 8.0;
/// The most lambda raises the ultrasmooth error scale, as for BC7.
const ULTRASMOOTH_LAMBDA_SCALE_MAX: f32 = 3.0;
const HASH_SLOTS: usize = 8192;
const HASH_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;
const HASH_SHIFT: u32 = 32;

/// The settings of [`reduce_entropy_bc1`].
#[derive(Debug, Clone)]
pub struct Bc1RdoParams {
    /// Bits saved against error added: 0 turns the pass off.
    pub lambda: f32,
    /// How many blocks back a run may come from.
    pub lookback_blocks: usize,
    /// No block's RMS error may grow by more than this factor.
    pub max_allowed_rms_increase_ratio: f32,
    /// A block whose channels vary less than this is smooth, and its error weighs more.
    pub max_smooth_block_std_dev: f32,
    /// Weigh the error of large flat regions more, as for BC7.
    pub use_ultrasmooth_block_handling: bool,
    /// Leave blocks that are already exact alone (transparent gutters of a sparse atlas).
    pub skip_zero_mse_blocks: bool,
}

impl Default for Bc1RdoParams {
    fn default() -> Self {
        Self {
            lambda: 1.0,
            lookback_blocks: 64,
            max_allowed_rms_increase_ratio: 10.0,
            max_smooth_block_std_dev: 18.0,
            use_ultrasmooth_block_handling: true,
            skip_zero_mse_blocks: false,
        }
    }
}

/// Runs the pass over `blocks` (`BC1_BLOCK_BYTES` each, `blocks_x` by `blocks_y`), whose source pixels are
/// `rgba_blocks` in block order (16 per block). Large images run in bands of block rows on rayon's pool, each band
/// matching only within itself. Returns how many blocks changed.
pub fn reduce_entropy_bc1(
    blocks: &mut [u8],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc1RdoParams,
) -> u32 {
    let num_blocks = blocks.len() / BC1_BLOCK_BYTES;
    debug_assert_eq!(blocks.len(), num_blocks * BC1_BLOCK_BYTES);
    debug_assert_eq!(num_blocks, blocks_x * blocks_y);
    debug_assert_eq!(rgba_blocks.len(), num_blocks * 16);
    if params.lambda <= 0.0 || !params.lambda.is_finite() || num_blocks == 0 {
        return 0;
    }
    if num_blocks < PARALLEL_RDO_BLOCK_THRESHOLD || blocks_x == 0 {
        return reduce_band(blocks, rgba_blocks, blocks_x, blocks_y, params);
    }
    let chunk_blocks = parallel_rdo_chunk_blocks(num_blocks, blocks_x, params.lookback_blocks.max(1), rayon::current_num_threads());
    blocks
        .par_chunks_mut(chunk_blocks * BC1_BLOCK_BYTES)
        .zip(rgba_blocks.par_chunks(chunk_blocks * 16))
        .map(|(band, pixels)| reduce_band(band, pixels, blocks_x, band.len() / BC1_BLOCK_BYTES / blocks_x, params))
        .sum()
}

fn reduce_band(blocks: &mut [u8], rgba_blocks: &[[u8; 4]], blocks_x: usize, blocks_y: usize, params: &Bc1RdoParams) -> u32 {
    let num_blocks = blocks.len() / BC1_BLOCK_BYTES;
    let lambda = params.lambda;
    let smooth_scale_max = lerp(SMOOTH_SCALE_MIN, SMOOTH_SCALE_MAX, (lambda / SMOOTH_SCALE_LAMBDA_SPAN).min(1.0));
    let ultrasmooth_scales = params.use_ultrasmooth_block_handling.then(|| {
        let lambda_scale = lambda.min(ULTRASMOOTH_LAMBDA_SCALE_MAX);
        let mut scales = compute_block_mse_scales(rgba_blocks, blocks_x, blocks_y, false);
        for scale in scales.iter_mut().filter(|scale| **scale > 0.0) {
            *scale = smooth_scale_max.max(*scale * lambda_scale);
        }
        scales
    });
    let lookback = params.lookback_blocks.max(1);
    let mut bits: Vec<u64> = blocks.chunks_exact(BC1_BLOCK_BYTES).map(|block| u64::from_le_bytes(block.try_into().unwrap())).collect();
    let mut hash_table = vec![0u64; HASH_SLOTS];
    let hash_mask = HASH_SLOTS - 1;
    // Where the last accepted run ended in the byte stream when it ran to its block's end (-1 otherwise), and how far
    // back it reached: the next block matching either pays less, as in `ert.cpp`'s ERT_FAVOR_CONT_AND_REP0_MATCHES.
    let mut prev_cont_window_ofs: i64 = -1;
    let mut prev_rep0_dist: i64 = -1;
    let mut modified = 0u32;

    for block_index in 0..num_blocks {
        let orig = bits[block_index];
        let target = drawn_target(rgba_block_at(rgba_blocks, block_index));
        let cur_err = bc1_error_bounded(orig, &target, u64::MAX).expect("an unbounded error always completes");
        if params.skip_zero_mse_blocks && cur_err == 0 {
            continue;
        }
        let smooth_scale = match ultrasmooth_scales.as_ref().map(|scales| scales[block_index]) {
            Some(scale) if scale > 0.0 => scale,
            _ => smooth_block_error_scale_from_pixels(&target, params.max_smooth_block_std_dev, smooth_scale_max),
        };
        let cur_ms_err = cur_err as f32 / BLOCK_CHANNELS;
        let cur_t = cur_ms_err * smooth_scale + LITERAL_BITS * BC1_BLOCK_BYTES as f32 * lambda;
        let trial_error_scale = max_trial_error_scale(smooth_scale);
        let thresh_ms_err = params.max_allowed_rms_increase_ratio * params.max_allowed_rms_increase_ratio * cur_ms_err.max(1.0);
        let hash_epoch = ((block_index as u64) + 1) << HASH_SHIFT;

        let mut best = orig;
        let mut best_t = cur_t;
        let mut best_cont_ofs = prev_cont_window_ofs;
        let mut best_rep0 = prev_rep0_dist;
        for prev_index in (block_index.saturating_sub(lookback)..block_index).rev() {
            let prev = bits[prev_index];
            let dist = ((block_index - prev_index) * BC1_BLOCK_BYTES) as i64;
            let prev_base = (prev_index * BC1_BLOCK_BYTES) as i64;
            let dist_bits = compute_dist_cost_estimate(dist as u32) as f32;
            let continues = prev_base == prev_cont_window_ofs;
            let repeats = dist == prev_rep0_dist;
            for len in (MIN_MATCH_BYTES..=BC1_BLOCK_BYTES).rev() {
                let literal_bits = (BC1_BLOCK_BYTES - len) as f32 * LITERAL_BITS;
                let normal_match_bits = dist_bits + compute_match_len_cost(len as u32) as f32;
                let normal_t = (literal_bits + normal_match_bits) * lambda;
                // Shorter runs leave more literal bytes, so no shorter run of this block can do better.
                if normal_t >= best_t && !continues && !repeats {
                    break;
                }
                let mask = if len == BC1_BLOCK_BYTES { u64::MAX } else { (1u64 << (len * 8)) - 1 };
                for ofs in 0..=(BC1_BLOCK_BYTES - len) {
                    let shift = ofs * 8;
                    let trial = (orig & !(mask << shift)) | (prev & (mask << shift));
                    let trial_t_bits = if continues && ofs == 0 {
                        (literal_bits + MATCH_CONTINUE_BITS) * lambda
                    } else if repeats {
                        (literal_bits + MATCH_REP0_BITS) * lambda
                    } else {
                        // The nearest block offering this exact trial is the cheapest: skip it further back.
                        if normal_t >= best_t || rdo_hash_seen(&mut hash_table, hash_mask, hash_epoch, trial_hash(trial)) {
                            continue;
                        }
                        normal_t
                    };
                    if trial_t_bits >= best_t {
                        continue;
                    }
                    let trial_err = if trial == orig {
                        cur_err
                    } else {
                        match bc1_error_bounded(trial, &target, max_trial_error(best_t, trial_t_bits, trial_error_scale)) {
                            Some(err) => err,
                            None => continue,
                        }
                    };
                    let trial_ms_err = trial_err as f32 / BLOCK_CHANNELS;
                    if trial_ms_err < thresh_ms_err {
                        let t = trial_ms_err * smooth_scale + trial_t_bits;
                        if t < best_t {
                            best_t = t;
                            best = trial;
                            best_cont_ofs = if ofs + len == BC1_BLOCK_BYTES { prev_base + BC1_BLOCK_BYTES as i64 } else { -1 };
                            best_rep0 = dist;
                        }
                    }
                }
            }
        }
        if best_t < cur_t {
            prev_cont_window_ofs = best_cont_ofs;
            prev_rep0_dist = best_rep0;
            if best != orig {
                bits[block_index] = best;
                blocks[block_index * BC1_BLOCK_BYTES..][..BC1_BLOCK_BYTES].copy_from_slice(&best.to_le_bytes());
                modified += 1;
            }
        } else {
            prev_cont_window_ofs = -1;
        }
    }
    modified
}

fn trial_hash(trial: u64) -> u32 {
    (trial.wrapping_mul(HASH_MULTIPLIER) >> HASH_SHIFT) as u32
}

/// What BC1 can draw of `pixels`: opaque colour, or transparent black below half alpha.
fn drawn_target(pixels: &RgbaBlock) -> RgbaBlock {
    pixels.map(|[r, g, b, a]| if a < ALPHA_HALF { [0; 4] } else { [r, g, b, u8::MAX] })
}

fn expand_565(colour: u16) -> [i32; 3] {
    let (r, g, b) = ((colour >> 11) as i32 & 31, (colour >> 5) as i32 & 63, colour as i32 & 31);
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

/// The squared RGBA error of the BC1 block `block` against `target`, or `None` once it passes `max_error`.
fn bc1_error_bounded(block: u64, target: &RgbaBlock, max_error: u64) -> Option<u64> {
    const OPAQUE: i32 = u8::MAX as i32;
    let (c0, c1) = (block as u16, (block >> 16) as u16);
    let (e0, e1) = (expand_565(c0), expand_565(c1));
    let mix = |w0: i32, w1: i32, d: i32| std::array::from_fn::<i32, 3, _>(|c| (w0 * e0[c] + w1 * e1[c]) / d);
    let palette: [[i32; 4]; 4] = if c0 > c1 {
        let (p2, p3) = (mix(2, 1, 3), mix(1, 2, 3));
        [[e0[0], e0[1], e0[2], OPAQUE], [e1[0], e1[1], e1[2], OPAQUE], [p2[0], p2[1], p2[2], OPAQUE], [p3[0], p3[1], p3[2], OPAQUE]]
    } else {
        let p2 = mix(1, 1, 2);
        [[e0[0], e0[1], e0[2], OPAQUE], [e1[0], e1[1], e1[2], OPAQUE], [p2[0], p2[1], p2[2], OPAQUE], [0; 4]]
    };
    let indices = (block >> 32) as u32;
    let mut error = 0u64;
    for (pixel, source) in target.iter().enumerate() {
        let drawn = palette[(indices >> (pixel * 2)) as usize & 3];
        error += (0..4).map(|c| { let d = drawn[c] - source[c] as i32; (d * d) as u64 }).sum::<u64>();
        if error > max_error {
            return None;
        }
    }
    Some(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth gradient: neighbouring blocks differ a little, so runs from them cost little error.
    fn gradient(blocks_x: usize, blocks_y: usize) -> Vec<[u8; 4]> {
        let mut pixels = Vec::with_capacity(blocks_x * blocks_y * 16);
        for by in 0..blocks_y {
            for bx in 0..blocks_x {
                for y in 0..4 {
                    for x in 0..4 {
                        let (px, py) = (bx * 4 + x, by * 4 + y);
                        pixels.push([(px * 3) as u8, (py * 2) as u8, ((px + py) / 2) as u8, u8::MAX]);
                    }
                }
            }
        }
        pixels
    }

    #[test]
    fn rdo_changes_blocks_within_the_error_bound_and_leaves_lambda_zero_alone() {
        const BLOCKS_X: usize = 16;
        const BLOCKS_Y: usize = 16;
        let pixels = gradient(BLOCKS_X, BLOCKS_Y);
        let mut rgba = vec![0u8; BLOCKS_X * BLOCKS_Y * 16 * 4];
        let width = BLOCKS_X * 4;
        for (index, pixel) in pixels.iter().enumerate() {
            let (block, within) = (index / 16, index % 16);
            let (x, y) = ((block % BLOCKS_X) * 4 + within % 4, (block / BLOCKS_X) * 4 + within / 4);
            rgba[(y * width + x) * 4..][..4].copy_from_slice(pixel);
        }
        let extent = crate::bc7::ImageExtent::new(width as u32, (BLOCKS_Y * 4) as u32).unwrap();
        let encoded = crate::bc1::encode_bc1(&rgba, extent).unwrap();

        let mut untouched = encoded.clone();
        let off = Bc1RdoParams { lambda: 0.0, ..Bc1RdoParams::default() };
        assert_eq!(reduce_entropy_bc1(&mut untouched, &pixels, BLOCKS_X, BLOCKS_Y, &off), 0);
        assert_eq!(untouched, encoded);

        let mut reduced = encoded.clone();
        let params = Bc1RdoParams { lambda: 4.0, ..Bc1RdoParams::default() };
        let changed = reduce_entropy_bc1(&mut reduced, &pixels, BLOCKS_X, BLOCKS_Y, &params);
        assert!(changed > 0);
        for block in 0..BLOCKS_X * BLOCKS_Y {
            let at = |blocks: &[u8]| u64::from_le_bytes(blocks[block * 8..][..8].try_into().unwrap());
            let target = drawn_target(rgba_block_at(&pixels, block));
            let before = bc1_error_bounded(at(&encoded), &target, u64::MAX).unwrap() as f32 / BLOCK_CHANNELS;
            let after = bc1_error_bounded(at(&reduced), &target, u64::MAX).unwrap() as f32 / BLOCK_CHANNELS;
            let ratio = params.max_allowed_rms_increase_ratio;
            assert!(after <= ratio * ratio * before.max(1.0), "block {block}: {before} -> {after}");
        }
    }

    #[test]
    fn bc1_error_matches_the_decoder_on_a_flat_block() {
        const RED_565: u16 = 31 << 11;
        let block = u64::from(RED_565) | (u64::from(RED_565) << 16);
        let target: RgbaBlock = [[255, 0, 0, 255]; 16];
        assert_eq!(bc1_error_bounded(block, &target, u64::MAX), Some(0));
        let transparent = block | (u64::from(u32::MAX) << 32);
        let clear: RgbaBlock = [[0; 4]; 16];
        assert_eq!(bc1_error_bounded(transparent, &clear, u64::MAX), Some(0));
    }
}
