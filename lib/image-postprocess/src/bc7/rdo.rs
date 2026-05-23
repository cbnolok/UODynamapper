use bcdec_rs;
use std::cmp::max;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

// ─── ERT constants (matching bc7enc_rdo ert.cpp) ─────────────────────────────
/// Bits charged per literal byte when estimating match cost.
const LITERAL_BITS: f32 = 13.0;
/// Reduced cost for a match that continues directly from the previous one.
const MATCH_CONTINUE_BITS: f32 = 1.0;
/// Reduced cost for a REP0 match (re-using the previous match distance).
const MATCH_REP0_BITS: f32 = 4.0;

#[derive(Debug, Clone)]
pub struct Bc7RdoParams {
    pub lambda: f32,
    pub lookback_window_size: usize,
    pub smooth_block_max_mse_scale: f32,
    pub max_smooth_block_std_dev: f32,
    pub try_two_matches: bool,
    pub allow_relative_movement: bool,
    pub skip_zero_mse_blocks: bool,
    pub use_ultrasmooth_block_handling: bool,
    pub custom_smooth_block_error_scale: bool,
    pub max_allowed_rms_increase_ratio: f32,
    pub debug_output: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Bc7RdoStats {
    pub candidate_checks: u64,
    pub rate_skips: u64,
    pub hash_skips: u64,
    pub original_block_skips: u64,
    pub decode_trials: u64,
    pub bounded_error_exits: u64,
    pub accepted_matches: u64,
    pub modified_blocks: u64,
}

impl Default for Bc7RdoParams {
    fn default() -> Self {
        Self {
            lambda: 0.5,
            lookback_window_size: 16 * 1024,
            smooth_block_max_mse_scale: 0.0, // 0.0 means automatic if custom is false
            max_smooth_block_std_dev: 18.0,
            try_two_matches: false,
            allow_relative_movement: false,
            skip_zero_mse_blocks: false,
            use_ultrasmooth_block_handling: true,
            custom_smooth_block_error_scale: false,
            max_allowed_rms_increase_ratio: 10.0,
            debug_output: false,
        }
    }
}

/// BC7 RDO postprocess.
/// 
/// Accepts mutable BC7 blocks and original source pixels in block-raster order.
/// Returns the count of modified blocks.
pub fn reduce_entropy_bc7(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]], // Flat array of all pixels in 4x4 block order: num_blocks * 16
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
) -> u32 {
    reduce_entropy_bc7_impl(blocks, rgba_blocks, blocks_x, blocks_y, params, None)
}

pub fn reduce_entropy_bc7_with_stats(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    stats: &mut Bc7RdoStats,
) -> u32 {
    reduce_entropy_bc7_impl(blocks, rgba_blocks, blocks_x, blocks_y, params, Some(stats))
}

fn reduce_entropy_bc7_impl(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    mut stats: Option<&mut Bc7RdoStats>,
) -> u32 {
    if params.lambda <= 0.0 {
        return 0;
    }

    let num_blocks = blocks.len();
    let mut total_modified = 0;
    let mut actual_params = params.clone();

    // 1. Calculate ultrasmooth scales if requested
    let mut block_mse_scales = if params.use_ultrasmooth_block_handling {
        Some(compute_block_mse_scales(rgba_blocks, blocks_x, blocks_y, params.debug_output))
    } else {
        None
    };

    // 2. Automatic smooth block MSE scale derivation
    if !params.custom_smooth_block_error_scale {
        actual_params.smooth_block_max_mse_scale = lerp(15.0, 50.0, (params.lambda / 4.0).min(1.0));
        if params.debug_output {
            println!("Using an automatically computed smooth block error scale of {}", actual_params.smooth_block_max_mse_scale);
        }
    }

    // 3. Adjust ultrasmooth scales with lambda and smooth_block_max_mse_scale
    if let Some(ref mut scales) = block_mse_scales {
        for s in scales.iter_mut() {
            if *s > 0.0 {
                *s = actual_params.smooth_block_max_mse_scale.max(*s * params.lambda.min(3.0));
            }
        }
    }

    // 4. Main loop
    let total_blocks_to_check = max(1, params.lookback_window_size / 16);
    let mut hash_table = vec![0u32; 8192];
    let hash_mask = hash_table.len() - 1;
    let mut block_modes = blocks.iter().map(get_bc7_mode).collect::<Vec<_>>();
    let mut previous_blocks_by_mode = vec![Vec::<usize>::new(); 8];

    // REP0 and match-continuation tracking (ert.cpp ERT_FAVOR_CONT_AND_REP0_MATCHES):
    //   prev_cont_window_ofs: source-window offset just past the last accepted match end.
    //                         The next block can "continue" it for only MATCH_CONTINUE_BITS.
    //   prev_rep0_dist:       byte distance of the last accepted match for cheap REP0 reuse.
    let mut prev_cont_window_ofs: i64 = -1;
    let mut prev_rep0_dist:       i64 = -1;

    for block_index in 0..num_blocks {
        if (block_index & 0xFF) == 0 {
            hash_table.fill(0);
        }

        let orig_blk = blocks[block_index];
        let p_pixels = &rgba_blocks[block_index * 16..(block_index + 1) * 16];
        let bc7_mode = block_modes[block_index];
        if bc7_mode == 8 {
            continue; // Invalid block or mode 8 (reserved)
        }

        let mut decoded_bc7_block = [[0u8; 4]; 16];
        unpack_bc7(&orig_blk, &mut decoded_bc7_block);

        let cur_err = block_error_bounded(p_pixels, &decoded_bc7_block, u64::MAX)
            .expect("u64::MAX cannot be exceeded by a 4x4 RGBA block error");

        if params.skip_zero_mse_blocks && cur_err == 0 {
            previous_blocks_by_mode[bc7_mode as usize].push(block_index);
            continue;
        }

        let max_std_dev = compute_block_max_std_dev(p_pixels);
        let mut yl = (max_std_dev / actual_params.max_smooth_block_std_dev).clamp(0.0, 1.0);
        yl = yl * yl;
        
        let mut smooth_block_error_scale = lerp(actual_params.smooth_block_max_mse_scale, 1.0, yl);
        if let Some(ref scales) = block_mse_scales {
            if scales[block_index] > 0.0 {
                smooth_block_error_scale = scales[block_index];
            }
        }

        let cur_ms_err = cur_err as f32 / 64.0;
        let cur_t = cur_ms_err * smooth_block_error_scale + (LITERAL_BITS * 16.0) * params.lambda;
        let first_block_to_check = block_index.saturating_sub(total_blocks_to_check);

        let mut best_block = orig_blk;
        let mut best_t = cur_t;
        let mut best_match_len = 0usize;
        let mut best_match_dst_block_ofs = 0usize;
        let mut best_match_bits = 0.0f32;

        let thresh_ms_err = params.max_allowed_rms_increase_ratio
            * params.max_allowed_rms_increase_ratio
            * cur_ms_err.max(1.0);

        if params.allow_relative_movement {
            // ── Main search window: full relative-offset search ──
            for &prev_block_index in previous_blocks_by_mode[bc7_mode as usize].iter().rev() {
                if prev_block_index < first_block_to_check {
                    break;
                }
                let prev_blk = blocks[prev_block_index];
                let base_dist = (block_index - prev_block_index) * 16;
                let relative_dist_bits = compute_relative_dist_costs(base_dist as u32);
                for len in (3..=16).rev() {
                    let len_bits = compute_match_len_cost(len as u32) as f32;
                    for src_ofs in 0..=(16 - len) {
                        for dst_ofs in 0..=(16 - len) {
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.candidate_checks += 1;
                            }
                            let relative_dist_index = (dst_ofs as i32 - src_ofs as i32 + 15) as usize;
                            let mb = relative_dist_bits[relative_dist_index] as f32 + len_bits;
                            let trial_bits = (16 - len) as f32 * LITERAL_BITS + mb;
                            let trial_bits_times_lambda = trial_bits * params.lambda;
                            if trial_bits_times_lambda >= best_t {
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.rate_skips += 1;
                                }
                                continue;
                            }

                            // Hash check to skip redundant trials
                            let hs = hash_hsieh(&prev_blk[src_ofs..src_ofs + len], dst_ofs as u32);
                            let hash_check = hash_table[hs as usize & hash_mask];
                            if (hash_check & 0xFF) == (block_index as u32 & 0xFF)
                                && (hash_check >> 8) == (hs >> 8) {
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.hash_skips += 1;
                                }
                                continue;
                            }
                            hash_table[hs as usize & hash_mask] = (hs & 0xFFFFFF00) | (block_index as u32 & 0xFF);

                            let mut trial_blk = orig_blk;
                            trial_blk[dst_ofs..dst_ofs + len].copy_from_slice(&prev_blk[src_ofs..src_ofs + len]);
                            if trial_blk == orig_blk {
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.original_block_skips += 1;
                                }
                                let trial_ms_err = cur_ms_err;
                                if trial_ms_err < thresh_ms_err {
                                    let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                                    if t < best_t {
                                        best_t = t; best_block = trial_blk;
                                        best_match_len = len; best_match_dst_block_ofs = dst_ofs;
                                        best_match_bits = mb;
                                        if let Some(stats) = stats.as_deref_mut() {
                                            stats.accepted_matches += 1;
                                        }
                                    }
                                }
                                continue;
                            }
                            let mut trial_decoded = [[0u8; 4]; 16];
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.decode_trials += 1;
                            }
                            if !unpack_bc7(&trial_blk, &mut trial_decoded) { continue; }
                            let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                            let Some(trial_err) = block_error_bounded(p_pixels, &trial_decoded, max_trial_err) else {
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.bounded_error_exits += 1;
                                }
                                continue;
                            };
                            let trial_ms_err = trial_err as f32 / 64.0;
                            if trial_ms_err < thresh_ms_err {
                                let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                                if t < best_t {
                                    best_t = t; best_block = trial_blk;
                                    best_match_len = len; best_match_dst_block_ofs = dst_ofs;
                                    best_match_bits = mb;
                                    if let Some(stats) = stats.as_deref_mut() {
                                        stats.accepted_matches += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // ── Main search window: fixed-offset default path ──
            for &prev_block_index in previous_blocks_by_mode[bc7_mode as usize].iter().rev() {
                if prev_block_index < first_block_to_check {
                    break;
                }
                let prev_blk = blocks[prev_block_index];
                let dist = (block_index - prev_block_index) * 16;
                let normal_dist_bits = compute_dist_cost_estimate(dist as u32) as f32;
                for len in (3..=16).rev() {
                    // Fixed-offset search: src_ofs == dst_ofs
                    let normal_match_bits = normal_dist_bits + compute_match_len_cost(len as u32) as f32;
                    let normal_trial_bits_times_lambda =
                        ((16 - len) as f32 * LITERAL_BITS + normal_match_bits) * params.lambda;

                    for ofs in 0..=(16 - len) {
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.candidate_checks += 1;
                        }
                        let src_win_ofs = (prev_block_index * 16 + ofs) as i64;
                        let dst_win_ofs = (block_index      * 16 + ofs) as i64;

                        // REP0 / match-continuation cost reduction (ERT_FAVOR_CONT_AND_REP0_MATCHES)
                        let (trial_match_bits, trial_bits_times_lambda) =
                            if src_win_ofs == prev_cont_window_ofs && ofs == 0 {
                                // Continuation: the match continues directly from the previous block's match
                                let tb = (16 - len) as f32 * LITERAL_BITS + MATCH_CONTINUE_BITS;
                                (MATCH_CONTINUE_BITS, tb * params.lambda)
                            } else if prev_rep0_dist >= 0 && src_win_ofs == dst_win_ofs - prev_rep0_dist {
                                // REP0: re-using the last accepted match distance costs only MATCH_REP0_BITS
                                let tb = (16 - len) as f32 * LITERAL_BITS + MATCH_REP0_BITS;
                                (MATCH_REP0_BITS, tb * params.lambda)
                            } else {
                                if normal_trial_bits_times_lambda >= best_t {
                                    if let Some(stats) = stats.as_deref_mut() {
                                        stats.rate_skips += 1;
                                    }
                                    continue;
                                }
                                // Normal match: deduplicate via hash before decoding
                                let hs = hash_hsieh(&prev_blk[ofs..ofs + len], ofs as u32);
                                let hash_check = hash_table[hs as usize & hash_mask];
                                if (hash_check & 0xFF) == (block_index as u32 & 0xFF)
                                    && (hash_check >> 8) == (hs >> 8) {
                                    if let Some(stats) = stats.as_deref_mut() {
                                        stats.hash_skips += 1;
                                    }
                                    continue;
                                }
                                hash_table[hs as usize & hash_mask] =
                                    (hs & 0xFFFFFF00) | (block_index as u32 & 0xFF);
                                (normal_match_bits, normal_trial_bits_times_lambda)
                            };
                        if trial_bits_times_lambda >= best_t {
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.rate_skips += 1;
                            }
                            continue;
                        }

                        let mut trial_blk = orig_blk;
                        trial_blk[ofs..ofs + len].copy_from_slice(&prev_blk[ofs..ofs + len]);
                        if trial_blk == orig_blk {
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.original_block_skips += 1;
                            }
                            let trial_ms_err = cur_ms_err;
                            if trial_ms_err < thresh_ms_err {
                                let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                                if t < best_t {
                                    best_t = t; best_block = trial_blk;
                                    best_match_len = len; best_match_dst_block_ofs = ofs;
                                    best_match_bits = trial_match_bits;
                                    prev_cont_window_ofs = src_win_ofs + len as i64;
                                    prev_rep0_dist       = dst_win_ofs - src_win_ofs;
                                    if let Some(stats) = stats.as_deref_mut() {
                                        stats.accepted_matches += 1;
                                    }
                                }
                            }
                            continue;
                        }
                        let mut trial_decoded = [[0u8; 4]; 16];
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.decode_trials += 1;
                        }
                        if !unpack_bc7(&trial_blk, &mut trial_decoded) { continue; }
                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                        let Some(trial_err) = block_error_bounded(p_pixels, &trial_decoded, max_trial_err) else {
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.bounded_error_exits += 1;
                            }
                            continue;
                        };
                        let trial_ms_err = trial_err as f32 / 64.0;
                        if trial_ms_err < thresh_ms_err {
                            let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                            if t < best_t {
                                best_t = t; best_block = trial_blk;
                                best_match_len = len; best_match_dst_block_ofs = ofs;
                                best_match_bits = trial_match_bits;
                                // Update continuation/REP0 state for the next block
                                prev_cont_window_ofs = src_win_ofs + len as i64;
                                prev_rep0_dist       = dst_win_ofs - src_win_ofs;
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.accepted_matches += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Try a second non-overlapping match — only attempted when the first was accepted (best_t < cur_t)
        if params.try_two_matches && best_t < cur_t && best_match_len > 0 && best_match_len <= (16 - 3) {
            let orig_best_block = best_block;
            let best_match_end = best_match_dst_block_ofs + best_match_len;

            for &prev_block_index in previous_blocks_by_mode[bc7_mode as usize].iter().rev() {
                if prev_block_index < first_block_to_check {
                    break;
                }
                let prev_blk = blocks[prev_block_index];

                let dist = (block_index - prev_block_index) * 16;
                let dist_bits = compute_dist_cost_estimate(dist as u32) as f32;
                for len in 3..=(16 - best_match_len) {
                    let trial_bits = (16.0 - len as f32 - best_match_len as f32) * LITERAL_BITS
                        + dist_bits
                        + compute_match_len_cost(len as u32) as f32
                        + best_match_bits;
                    let trial_bits_times_lambda = trial_bits * params.lambda;
                    if trial_bits_times_lambda >= best_t {
                        if let Some(stats) = stats.as_deref_mut() {
                            let skipped_offsets = 17 - len;
                            stats.candidate_checks += skipped_offsets as u64;
                            stats.rate_skips += skipped_offsets as u64;
                        }
                        continue;
                    }

                    for ofs in 0..=(16 - len) {
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.candidate_checks += 1;
                        }
                        if ofs < best_match_end && ofs + len > best_match_dst_block_ofs {
                            continue;
                        }

                        let mut trial_blk = orig_best_block;
                        trial_blk[ofs..ofs + len].copy_from_slice(&prev_blk[ofs..ofs + len]);

                        let mut trial_decoded = [[0u8; 4]; 16];
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.decode_trials += 1;
                        }
                        if !unpack_bc7(&trial_blk, &mut trial_decoded) {
                            continue;
                        }

                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                        let Some(trial_err) = block_error_bounded(p_pixels, &trial_decoded, max_trial_err) else {
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.bounded_error_exits += 1;
                            }
                            continue;
                        };

                        let trial_ms_err = trial_err as f32 / 64.0;
                        if trial_ms_err < thresh_ms_err {
                            let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                            if t < best_t {
                                best_t = t;
                                best_block = trial_blk;
                                if let Some(stats) = stats.as_deref_mut() {
                                    stats.accepted_matches += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        if best_t < cur_t {
            blocks[block_index] = best_block;
            block_modes[block_index] = get_bc7_mode(&best_block);
            total_modified += 1;
            if let Some(stats) = stats.as_deref_mut() {
                stats.modified_blocks += 1;
            }
        }
        if block_modes[block_index] < 8 {
            previous_blocks_by_mode[block_modes[block_index] as usize].push(block_index);
        }
    }

    total_modified
}

#[inline(always)]
fn max_trial_error(best_t: f32, trial_bits_times_lambda: f32, smooth_block_error_scale: f32) -> u64 {
    if smooth_block_error_scale <= 0.0 {
        return u64::MAX;
    }
    (((best_t - trial_bits_times_lambda) * 64.0) / smooth_block_error_scale).max(0.0).ceil() as u64
}

#[inline(always)]
fn block_error_bounded(
    source: &[[u8; 4]],
    decoded: &[[u8; 4]; 16],
    max_error: u64,
) -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { block_error_bounded_sse2(source, decoded, max_error) }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            unsafe { block_error_bounded_sse2(source, decoded, max_error) }
        } else {
            block_error_bounded_scalar(source, decoded, max_error)
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { block_error_bounded_neon(source, decoded, max_error) }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        block_error_bounded_scalar(source, decoded, max_error)
    }
}

#[inline(always)]
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn block_error_bounded_scalar(
    source: &[[u8; 4]],
    decoded: &[[u8; 4]; 16],
    max_error: u64,
) -> Option<u64> {
    let mut err = 0u64;
    for i in 0..16 {
        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[i][c] as i32;
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }
    Some(err)
}

// SSE2 is enough for RDO's 4x4 RGBA squared-error checks: widen bytes to i16,
// square via madd, and stop after each half block once the candidate is hopeless.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn block_error_bounded_sse2(
    source: &[[u8; 4]],
    decoded: &[[u8; 4]; 16],
    max_error: u64,
) -> Option<u64> {
    let zero = _mm_setzero_si128();
    let mut sum = _mm_setzero_si128();
    let source_ptr = source.as_ptr() as *const __m128i;
    let decoded_ptr = decoded.as_ptr() as *const __m128i;
    let mut err = 0u64;

    for i in 0..4 {
        let src = _mm_loadu_si128(source_ptr.add(i));
        let dec = _mm_loadu_si128(decoded_ptr.add(i));
        let src_lo = _mm_unpacklo_epi8(src, zero);
        let src_hi = _mm_unpackhi_epi8(src, zero);
        let dec_lo = _mm_unpacklo_epi8(dec, zero);
        let dec_hi = _mm_unpackhi_epi8(dec, zero);
        let diff_lo = _mm_sub_epi16(src_lo, dec_lo);
        let diff_hi = _mm_sub_epi16(src_hi, dec_hi);
        sum = _mm_add_epi32(sum, _mm_madd_epi16(diff_lo, diff_lo));
        sum = _mm_add_epi32(sum, _mm_madd_epi16(diff_hi, diff_hi));

        if i == 1 || i == 3 {
            err += hsum_epi32_sse2(sum) as u64;
            if err >= max_error {
                return None;
            }
            sum = _mm_setzero_si128();
        }
    }

    Some(err)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn hsum_epi32_sse2(v: __m128i) -> u32 {
    let hi64 = _mm_srli_si128::<8>(v);
    let sum64 = _mm_add_epi32(v, hi64);
    let hi32 = _mm_srli_si128::<4>(sum64);
    _mm_cvtsi128_si32(_mm_add_epi32(sum64, hi32)) as u32
}

// AArch64 NEON mirrors the SSE2 path: process one 16-byte row at a time,
// widen to i16, square into i32 lanes, and keep the same half-block early exit.
#[cfg(target_arch = "aarch64")]
unsafe fn block_error_bounded_neon(
    source: &[[u8; 4]],
    decoded: &[[u8; 4]; 16],
    max_error: u64,
) -> Option<u64> {
    let source_ptr = source.as_ptr() as *const u8;
    let decoded_ptr = decoded.as_ptr() as *const u8;
    let mut sum = vdupq_n_s32(0);
    let mut err = 0u64;

    for i in 0..4 {
        let src = vld1q_u8(source_ptr.add(i * 16));
        let dec = vld1q_u8(decoded_ptr.add(i * 16));
        let src_lo = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(src)));
        let src_hi = vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(src)));
        let dec_lo = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(dec)));
        let dec_hi = vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(dec)));
        let diff_lo = vsubq_s16(src_lo, dec_lo);
        let diff_hi = vsubq_s16(src_hi, dec_hi);

        sum = vaddq_s32(sum, vmull_s16(vget_low_s16(diff_lo), vget_low_s16(diff_lo)));
        sum = vaddq_s32(sum, vmull_s16(vget_high_s16(diff_lo), vget_high_s16(diff_lo)));
        sum = vaddq_s32(sum, vmull_s16(vget_low_s16(diff_hi), vget_low_s16(diff_hi)));
        sum = vaddq_s32(sum, vmull_s16(vget_high_s16(diff_hi), vget_high_s16(diff_hi)));

        if i == 1 || i == 3 {
            err += vaddvq_s32(sum) as u64;
            if err >= max_error {
                return None;
            }
            sum = vdupq_n_s32(0);
        }
    }

    Some(err)
}

fn hash_hsieh(buf: &[u8], salt: u32) -> u32 {
    let len = buf.len();
    if len == 0 { return 0; }

    let mut h = (len as u32).wrapping_add(salt << 16);
    let mut i = 0;
    let mut rem = len;

    while rem >= 4 {
        let w0 = u16::from_le_bytes([buf[i], buf[i+1]]) as u32;
        let w1 = u16::from_le_bytes([buf[i+2], buf[i+3]]) as u32;
        
        h = h.wrapping_add(w0);
        let t = (w1 << 11) ^ h;
        h = (h << 16) ^ t;
        
        i += 4;
        rem -= 4;
        h = h.wrapping_add(h >> 11);
    }

    match rem {
        3 => {
            h = h.wrapping_add(u16::from_le_bytes([buf[i], buf[i+1]]) as u32);
            h ^= h << 16;
            h ^= (buf[i+2] as i8 as u32) << 18;
            h = h.wrapping_add(h >> 11);
        }
        2 => {
            h = h.wrapping_add(u16::from_le_bytes([buf[i], buf[i+1]]) as u32);
            h ^= h << 11;
            h = h.wrapping_add(h >> 17);
        }
        1 => {
            h = h.wrapping_add(buf[i] as i8 as u32);
            h ^= h << 10;
            h = h.wrapping_add(h >> 1);
        }
        _ => {}
    }

    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);

    h
}

fn get_bc7_mode(block: &[u8; 16]) -> u32 {
    let first_byte = block[0];
    if first_byte == 0 { return 8; }
    for mode in 0..8 {
        if (first_byte & (1 << mode)) != 0 {
            return mode as u32;
        }
    }
    8
}

fn unpack_bc7(block: &[u8; 16], pixels: &mut [[u8; 4]; 16]) -> bool {
    bcdec_rs::bc7(block, pixels.as_flattened_mut(), 16);
    true
}

fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

fn compute_block_max_std_dev(pixels: &[[u8; 4]]) -> f32 {
    let mut max_std_dev = 0.0f32;
    for c in 0..4 {
        let mut sum = 0.0f64;
        let mut sum2 = 0.0f64;
        for i in 0..16 {
            let val = pixels[i][c] as f64;
            sum += val;
            sum2 += val * val;
        }
        let std_dev = ((16.0 * sum2 - sum * sum).max(0.0).sqrt() / 16.0) as f32;
        if std_dev > max_std_dev {
            max_std_dev = std_dev;
        }
    }
    max_std_dev
}

#[inline(always)]
fn compute_match_len_cost(match_len: u32) -> u32 {
    if match_len >= 12 {
        9
    } else if match_len >= 8 {
        8
    } else if match_len >= 6 {
        7
    } else {
        6
    }
}

#[inline(always)]
fn compute_dist_cost_estimate(dist: u32) -> u32 {
    let mut dist_cost = 5;
    if dist < 512 {
        dist_cost += SMALL_DIST_EXTRA[dist as usize & 511] as u32;
    } else {
        dist_cost += LARGE_DIST_EXTRA[(dist.min(32767) >> 8) as usize] as u32;
        let mut d = dist;
        while d >= 32768 {
            dist_cost += 1;
            d >>= 1;
        }
    }
    dist_cost
}

#[inline(always)]
fn compute_relative_dist_costs(base_dist: u32) -> [u32; 31] {
    let mut costs = [0u32; 31];
    for (i, cost) in costs.iter_mut().enumerate() {
        let dist = (base_dist as i32 + i as i32 - 15) as u32;
        *cost = compute_dist_cost_estimate(dist);
    }
    costs
}

const SMALL_DIST_EXTRA: [u8; 512] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
];

const LARGE_DIST_EXTRA: [u8; 128] = [
    0, 0, 8, 8, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
];

fn compute_block_mse_scales(
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    _debug_output: bool,
) -> Vec<f32> {
    let total_blocks = blocks_x * blocks_y;
    let mut block_mse_scales = vec![-1.0f32; total_blocks];

    let ultrasmooth_block_std_dev_threshold = 2.9f32;
    let dark_threshold = 13.0f32;
    let bright_threshold = 222.0f32;
    let ultrasmooth_block_mse_scale = 120.0f32;
    let ultrasmooth_region_too_small_threshold = 64;

    let mut is_ultrasmooth = vec![false; total_blocks];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let block_index = bx + by * blocks_x;
            let pixels = &rgba_blocks[block_index * 16..(block_index + 1) * 16];

            let mut luma_sum = 0.0f64;
            for i in 0..16 {
                let l = 0.299 * pixels[i][0] as f64 + 0.587 * pixels[i][1] as f64 + 0.114 * pixels[i][2] as f64;
                luma_sum += l;
            }
            let luma_avg = luma_sum / 16.0;

            let max_std_dev = compute_block_max_std_dev(pixels);
            let mut yl = (max_std_dev / ultrasmooth_block_std_dev_threshold).clamp(0.0, 1.0);
            yl = yl * yl;

            if luma_avg < dark_threshold as f64 || luma_avg >= bright_threshold as f64 {
                yl = 1.0;
            }

            if yl == 0.0 {
                is_ultrasmooth[block_index] = true;
            }
        }
    }

    let mut current_mask = is_ultrasmooth.clone();

    // Pass 1: Erosion of ultrasmooth (dilation of non-ultrasmooth)
    let mut next_mask = current_mask.clone();
    for y in 0..blocks_y {
        for x in 0..blocks_x {
            let mut any_non_ultrasmooth = false;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && nx < blocks_x as i32 && ny >= 0 && ny < blocks_y as i32 {
                        if !current_mask[nx as usize + ny as usize * blocks_x] {
                            any_non_ultrasmooth = true;
                            break;
                        }
                    }
                }
                if any_non_ultrasmooth { break; }
            }
            if any_non_ultrasmooth {
                next_mask[x + y * blocks_x] = false;
            }
        }
    }
    current_mask = next_mask;

    // 32 passes of "median-like" erosion
    for _ in 0..32 {
        let mut next_mask = current_mask.clone();
        for y in 0..blocks_y {
            for x in 0..blocks_x {
                if current_mask[x + y * blocks_x] {
                    let mut non_ultrasmooth_count = 0;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let nx = x as i32 + dx;
                            let ny = y as i32 + dy;
                            if nx >= 0 && nx < blocks_x as i32 && ny >= 0 && ny < blocks_y as i32 {
                                if !current_mask[nx as usize + ny as usize * blocks_x] {
                                    non_ultrasmooth_count += 1;
                                }
                            }
                        }
                    }
                    if non_ultrasmooth_count >= 5 {
                        next_mask[x + y * blocks_x] = false;
                    }
                }
            }
        }
        current_mask = next_mask;
    }

    // Flood fill to remove small ULTRASMOOTH regions
    let mut final_mask = current_mask.clone();
    let mut visited = vec![false; total_blocks];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let idx = bx + by * blocks_x;
            if current_mask[idx] && !visited[idx] {
                let mut component = Vec::new();
                let mut stack = vec![(bx, by)];
                visited[idx] = true;
                while let Some((cx, cy)) = stack.pop() {
                    component.push((cx, cy));
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx >= 0 && nx < blocks_x as i32 && ny >= 0 && ny < blocks_y as i32 {
                            let nidx = nx as usize + ny as usize * blocks_x;
                            if current_mask[nidx] && !visited[nidx] {
                                visited[nidx] = true;
                                stack.push((nx as usize, ny as usize));
                            }
                        }
                    }
                }
                if component.len() < ultrasmooth_region_too_small_threshold {
                    for (cx, cy) in component {
                        final_mask[cx + cy * blocks_x] = false;
                    }
                }
            }
        }
    }

    for i in 0..total_blocks {
        if final_mask[i] {
            block_mse_scales[i] = ultrasmooth_block_mse_scale;
        }
    }

    block_mse_scales
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bc7::analytical::{pack_bc7_rgba, FLAG_PBIT_OPT_M6, FLAG_USE_DUAL_PLANE};

    fn fixture_rgba_blocks(blocks_x: usize, blocks_y: usize) -> Vec<[u8; 4]> {
        let num_blocks = blocks_x * blocks_y;
        let mut rgba_blocks = vec![[0u8; 4]; num_blocks * 16];
        for b in 0..num_blocks {
            let bx = b % blocks_x;
            let by = b / blocks_x;
            let pattern = (bx * 3 + by * 5) as u8;
            for i in 0..16 {
                let x = (i % 4) as u8;
                let y = (i / 4) as u8;
                rgba_blocks[b * 16 + i] = [
                    x.wrapping_mul(47).wrapping_add(pattern),
                    y.wrapping_mul(53).wrapping_add(pattern.wrapping_mul(2)),
                    (x ^ y).wrapping_mul(37).wrapping_add((b as u8).wrapping_mul(3)),
                    255,
                ];
            }
        }
        rgba_blocks
    }

    fn encode_fixture_blocks(rgba_blocks: &[[u8; 4]]) -> Vec<[u8; 16]> {
        let num_blocks = rgba_blocks.len() / 16;
        let mut blocks = vec![[0u8; 16]; num_blocks];
        for b in 0..num_blocks {
            let pixels: &[crate::bc7::analytical::Pixel; 16] =
                rgba_blocks[b * 16..(b + 1) * 16].try_into().unwrap();
            pack_bc7_rgba(&mut blocks[b], pixels, FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE);
        }
        blocks
    }

    fn checksum_blocks(blocks: &[[u8; 16]]) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for b in blocks.iter().flat_map(|block| block.iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    #[test]
    fn rdo_default_fixture_output_is_stable() {
        let blocks_x = 8;
        let blocks_y = 6;
        let rgba_blocks = fixture_rgba_blocks(blocks_x, blocks_y);
        let mut blocks = encode_fixture_blocks(&rgba_blocks);
        let params = Bc7RdoParams {
            lambda: 0.5,
            lookback_window_size: 1024,
            ..Default::default()
        };

        let modified = reduce_entropy_bc7(&mut blocks, &rgba_blocks, blocks_x, blocks_y, &params);

        assert_eq!(modified, 47);
        assert_eq!(checksum_blocks(&blocks), 0xc97bdf2c143b665f);
    }

    #[test]
    fn test_bc7_analytical_and_rdo_roundtrip_with_bcdec() {
        let blocks_x = 4;
        let blocks_y = 4;
        let num_blocks = blocks_x * blocks_y;
        
        // Generate a 16-block image with similar but slightly different gradient patterns to allow lossy RDO optimization
        let mut rgba_blocks = vec![[0u8; 4]; num_blocks * 16];
        for b in 0..num_blocks {
            let pattern_type = b % 2;
            let offset = b as u8 * 2;
            for i in 0..16 {
                let px_idx = b * 16 + i;
                let x = (i % 4) as u8 * 50;
                let y = (i / 4) as u8 * 50;
                if pattern_type == 0 {
                    rgba_blocks[px_idx] = [
                        x.saturating_add(offset),
                        y.saturating_add(offset),
                        128,
                        255,
                    ];
                } else {
                    rgba_blocks[px_idx] = [
                        y.saturating_add(offset),
                        x.saturating_add(offset),
                        64,
                        255,
                    ];
                }
            }
        }

        // 1. Encode without RDO (Analytical Encoder)
        let mut original_blocks = vec![[0u8; 16]; num_blocks];
        for b in 0..num_blocks {
            let pixels: &[crate::bc7::analytical::Pixel; 16] = rgba_blocks[b * 16..(b + 1) * 16].try_into().unwrap();
            pack_bc7_rgba(&mut original_blocks[b], pixels, FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE);
        }

        // 2. Decode the non-RDO blocks using bcdec_rs to verify validity
        let mut decoded_non_rdo = vec![[0u8; 4]; num_blocks * 16];
        for b in 0..num_blocks {
            let mut decoded_pixels = [[0u8; 4]; 16];
            bcdec_rs::bc7(&original_blocks[b], decoded_pixels.as_flattened_mut(), 16);
            decoded_non_rdo[b * 16..(b + 1) * 16].copy_from_slice(&decoded_pixels);
        }

        // Calculate Mean Squared Error (MSE) for non-RDO to make sure it's high quality
        let mut sse_non_rdo = 0.0;
        for i in 0..(num_blocks * 16) {
            for c in 0..4 {
                let diff = rgba_blocks[i][c] as f32 - decoded_non_rdo[i][c] as f32;
                sse_non_rdo += diff * diff;
            }
        }
        let mse_non_rdo = sse_non_rdo / (num_blocks * 16 * 4) as f32;
        println!("Analytical BC7 MSE (without RDO): {}", mse_non_rdo);
        assert!(mse_non_rdo < 150.0, "Analytical BC7 quality is too low! MSE: {}", mse_non_rdo);

        // 3. Apply RDO to reduce entropy/reuse matches
        let mut rdo_blocks = original_blocks.clone();
        let params = Bc7RdoParams {
            lambda: 100.0, // High lambda to force optimization/modifications
            lookback_window_size: 4096,
            skip_zero_mse_blocks: false,
            use_ultrasmooth_block_handling: true,
            custom_smooth_block_error_scale: false,
            max_allowed_rms_increase_ratio: 20.0,
            debug_output: true,
            ..Default::default()
        };

        let modified_count = reduce_entropy_bc7(
            &mut rdo_blocks,
            &rgba_blocks,
            blocks_x,
            blocks_y,
            &params,
        );
        println!("RDO modified {} out of {} blocks", modified_count, num_blocks);
        
        // Assert that RDO actually found and modified some blocks
        assert!(modified_count > 0, "RDO should have modified at least one block to optimize rate-distortion!");

        // Verify that the RDO blocks actually differ from the original blocks
        assert_ne!(original_blocks, rdo_blocks, "RDO blocks should be different from non-RDO blocks!");

        // 4. Decode the RDO blocks using bcdec_rs to verify they are still valid decodable BC7 blocks
        let mut decoded_rdo = vec![[0u8; 4]; num_blocks * 16];
        for b in 0..num_blocks {
            let mut decoded_pixels = [[0u8; 4]; 16];
            bcdec_rs::bc7(&rdo_blocks[b], decoded_pixels.as_flattened_mut(), 16);
            decoded_rdo[b * 16..(b + 1) * 16].copy_from_slice(&decoded_pixels);
        }

        // Calculate MSE for RDO blocks to verify that the reconstructed quality is still very good
        let mut sse_rdo = 0.0;
        for i in 0..(num_blocks * 16) {
            for c in 0..4 {
                let diff = rgba_blocks[i][c] as f32 - decoded_rdo[i][c] as f32;
                sse_rdo += diff * diff;
            }
        }
        let mse_rdo = sse_rdo / (num_blocks * 16 * 4) as f32;
        println!("RDO BC7 MSE: {}", mse_rdo);
        
        // Rate-distortion optimizes for rate (entropy) as well, so distortion might be slightly higher,
        // but it should still be very reasonable and well within quality boundaries.
        assert!(mse_rdo < 250.0, "RDO BC7 quality is too low! MSE: {}", mse_rdo);
    }
}
