use super::tables::{
    BC7_ANCHOR_SECOND_SUBSET, BC7_ANCHOR_THIRD_SUBSET1, BC7_ANCHOR_THIRD_SUBSET2,
    BC7_PARTITION2, BC7_PARTITION3,
};
use std::cmp::max;

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
    pub relative_movement_max_offset_delta: usize,
    pub relative_movement_max_previous_blocks: usize,
    pub relative_movement_min_match_len: usize,
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
    pub relative_offset_skips: u64,
    pub relative_previous_block_limit_hits: u64,
    pub relative_length_skips: u64,
    pub original_block_skips: u64,
    pub decode_trials: u64,
    pub decode_mode_trials: [u64; 8],
    pub fused_mode0_trials: u64,
    pub fused_mode1_trials: u64,
    pub fused_mode2_trials: u64,
    pub fused_mode3_trials: u64,
    pub fused_mode4_trials: u64,
    pub fused_mode5_trials: u64,
    pub fused_mode6_trials: u64,
    pub fused_mode7_trials: u64,
    pub unsupported_mode_trials: u64,
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
            relative_movement_max_offset_delta: 15,
            relative_movement_max_previous_blocks: 0,
            relative_movement_min_match_len: 3,
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

        let cur_err = decode_bc7_error_bounded(&orig_blk, p_pixels, bc7_mode, u64::MAX, stats.as_deref_mut())
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
            let max_relative_delta = params.relative_movement_max_offset_delta.min(15);
            let max_relative_previous_blocks = params.relative_movement_max_previous_blocks;
            let min_relative_match_len = params.relative_movement_min_match_len.clamp(3, 16);
            let mut relative_previous_blocks_checked = 0usize;
            for &prev_block_index in previous_blocks_by_mode[bc7_mode as usize].iter().rev() {
                if prev_block_index < first_block_to_check {
                    break;
                }
                if max_relative_previous_blocks > 0
                    && relative_previous_blocks_checked >= max_relative_previous_blocks
                {
                    if let Some(stats) = stats.as_deref_mut() {
                        stats.relative_previous_block_limit_hits += 1;
                    }
                    break;
                }
                relative_previous_blocks_checked += 1;
                let prev_blk = blocks[prev_block_index];
                let base_dist = (block_index - prev_block_index) * 16;
                let relative_dist_bits = compute_relative_dist_costs(base_dist as u32);
                if let Some(stats) = stats.as_deref_mut() {
                    for len in 3..min_relative_match_len {
                        stats.relative_length_skips += relative_offset_candidate_count(len, max_relative_delta) as u64;
                    }
                }
                for len in (min_relative_match_len..=16).rev() {
                    let len_bits = compute_match_len_cost(len as u32) as f32;
                    for src_ofs in 0usize..=(16 - len) {
                        let full_dst_count = 17 - len;
                        let dst_start = src_ofs.saturating_sub(max_relative_delta);
                        let dst_end = (src_ofs + max_relative_delta).min(16 - len);
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.relative_offset_skips += (full_dst_count - (dst_end - dst_start + 1)) as u64;
                        }
                        for dst_ofs in dst_start..=dst_end {
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
                            if let Some(stats) = stats.as_deref_mut() {
                                stats.decode_trials += 1;
                            }
                            let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                            let Some(trial_err) = decode_bc7_error_bounded(
                                &trial_blk,
                                p_pixels,
                                bc7_mode,
                                max_trial_err,
                                stats.as_deref_mut(),
                            ) else {
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
                        if let Some(stats) = stats.as_deref_mut() {
                            stats.decode_trials += 1;
                        }
                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                        let Some(trial_err) = decode_bc7_error_bounded(
                            &trial_blk,
                            p_pixels,
                            bc7_mode,
                            max_trial_err,
                            stats.as_deref_mut(),
                        ) else {
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

                        if let Some(stats) = stats.as_deref_mut() {
                            stats.decode_trials += 1;
                        }
                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, smooth_block_error_scale);
                        let Some(trial_err) = decode_bc7_error_bounded(
                            &trial_blk,
                            p_pixels,
                            bc7_mode,
                            max_trial_err,
                            stats.as_deref_mut(),
                        ) else {
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
fn decode_bc7_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    _mode_hint: u32,
    max_error: u64,
    mut stats: Option<&mut Bc7RdoStats>,
) -> Option<u64> {
    let mode = get_bc7_mode(block);
    if let Some(stats) = stats.as_deref_mut() {
        if mode < 8 {
            stats.decode_mode_trials[mode as usize] += 1;
        }
    }
    if mode == 0 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode0_trials += 1;
        }
        return decode_bc7_mode0_error_bounded(block, source, max_error);
    }
    if mode == 1 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode1_trials += 1;
        }
        return decode_bc7_mode1_error_bounded(block, source, max_error);
    }
    if mode == 2 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode2_trials += 1;
        }
        return decode_bc7_mode2_error_bounded(block, source, max_error);
    }
    if mode == 3 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode3_trials += 1;
        }
        return decode_bc7_mode3_error_bounded(block, source, max_error);
    }
    if mode == 4 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode4_trials += 1;
        }
        return decode_bc7_mode4_error_bounded(block, source, max_error);
    }
    if mode == 5 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode5_trials += 1;
        }
        return decode_bc7_mode5_error_bounded(block, source, max_error);
    }
    if mode == 6 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode6_trials += 1;
        }
        return decode_bc7_mode6_error_bounded(block, source, max_error);
    }
    if mode == 7 {
        if let Some(stats) = stats.as_deref_mut() {
            stats.fused_mode7_trials += 1;
        }
        return decode_bc7_mode7_error_bounded(block, source, max_error);
    }

    if let Some(stats) = stats.as_deref_mut() {
        stats.unsupported_mode_trials += 1;
    }
    None
}

fn decode_bc7_mode0_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let low = u64::from_le_bytes(block[0..8].try_into().expect("mode 0 low word"));
    let high = u64::from_le_bytes(block[8..16].try_into().expect("mode 0 high word"));
    let partition = ((low >> 1) & 0x0F) as usize;
    let partitions = &BC7_PARTITION3[partition * 16..partition * 16 + 16];
    let anchors = [
        0usize,
        BC7_ANCHOR_THIRD_SUBSET1[partition] as usize,
        BC7_ANCHOR_THIRD_SUBSET2[partition] as usize,
    ];

    let lr = [
        (low >> 5) & 0x0F,
        (low >> 13) & 0x0F,
        (low >> 21) & 0x0F,
    ];
    let hr = [
        (low >> 9) & 0x0F,
        (low >> 17) & 0x0F,
        (low >> 25) & 0x0F,
    ];
    let lg = [
        (low >> 29) & 0x0F,
        (low >> 37) & 0x0F,
        (low >> 45) & 0x0F,
    ];
    let hg = [
        (low >> 33) & 0x0F,
        (low >> 41) & 0x0F,
        (low >> 49) & 0x0F,
    ];
    let lb = [
        (low >> 53) & 0x0F,
        ((low >> 61) & 0x07) | ((high & 0x01) << 3),
        (high >> 5) & 0x0F,
    ];
    let hb = [
        (low >> 57) & 0x0F,
        (high >> 1) & 0x0F,
        (high >> 9) & 0x0F,
    ];
    let p = [
        (high >> 13) & 0x01,
        (high >> 14) & 0x01,
        (high >> 15) & 0x01,
        (high >> 16) & 0x01,
        (high >> 17) & 0x01,
        (high >> 18) & 0x01,
    ];

    let endpoints = [
        [
            [
                expand_5_endpoint((lr[0] << 1) | p[0]),
                expand_5_endpoint((lg[0] << 1) | p[0]),
                expand_5_endpoint((lb[0] << 1) | p[0]),
            ],
            [
                expand_5_endpoint((hr[0] << 1) | p[1]),
                expand_5_endpoint((hg[0] << 1) | p[1]),
                expand_5_endpoint((hb[0] << 1) | p[1]),
            ],
        ],
        [
            [
                expand_5_endpoint((lr[1] << 1) | p[2]),
                expand_5_endpoint((lg[1] << 1) | p[2]),
                expand_5_endpoint((lb[1] << 1) | p[2]),
            ],
            [
                expand_5_endpoint((hr[1] << 1) | p[3]),
                expand_5_endpoint((hg[1] << 1) | p[3]),
                expand_5_endpoint((hb[1] << 1) | p[3]),
            ],
        ],
        [
            [
                expand_5_endpoint((lr[2] << 1) | p[4]),
                expand_5_endpoint((lg[2] << 1) | p[4]),
                expand_5_endpoint((lb[2] << 1) | p[4]),
            ],
            [
                expand_5_endpoint((hr[2] << 1) | p[5]),
                expand_5_endpoint((hg[2] << 1) | p[5]),
                expand_5_endpoint((hb[2] << 1) | p[5]),
            ],
        ],
    ];
    decode_bc7_partitioned_rgb_error_bounded(source, max_error, partitions, &anchors, &endpoints, high >> 19, 3, &BC7_WEIGHTS3)
}

fn decode_bc7_mode2_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let low = u64::from_le_bytes(block[0..8].try_into().expect("mode 2 low word"));
    let tail = u64::from_le_bytes(block[8..16].try_into().expect("mode 2 tail word"));
    let partition = ((low >> 3) & 0x3F) as usize;
    let partitions = &BC7_PARTITION3[partition * 16..partition * 16 + 16];
    let anchors = [
        0usize,
        BC7_ANCHOR_THIRD_SUBSET1[partition] as usize,
        BC7_ANCHOR_THIRD_SUBSET2[partition] as usize,
    ];

    let endpoints = [
        [
            [
                expand_5_endpoint((low >> 9) & 0x1F),
                expand_5_endpoint((low >> 39) & 0x1F),
                expand_5_endpoint((tail >> 5) & 0x1F),
            ],
            [
                expand_5_endpoint((low >> 14) & 0x1F),
                expand_5_endpoint((low >> 44) & 0x1F),
                expand_5_endpoint((tail >> 10) & 0x1F),
            ],
        ],
        [
            [
                expand_5_endpoint((low >> 19) & 0x1F),
                expand_5_endpoint((low >> 49) & 0x1F),
                expand_5_endpoint((tail >> 15) & 0x1F),
            ],
            [
                expand_5_endpoint((low >> 24) & 0x1F),
                expand_5_endpoint((low >> 54) & 0x1F),
                expand_5_endpoint((tail >> 20) & 0x1F),
            ],
        ],
        [
            [
                expand_5_endpoint((low >> 29) & 0x1F),
                expand_5_endpoint((low >> 59) & 0x1F),
                expand_5_endpoint((tail >> 25) & 0x1F),
            ],
            [
                expand_5_endpoint((low >> 34) & 0x1F),
                expand_5_endpoint(tail & 0x1F),
                expand_5_endpoint((tail >> 30) & 0x1F),
            ],
        ],
    ];
    decode_bc7_partitioned_rgb_error_bounded(source, max_error, partitions, &anchors, &endpoints, tail >> 35, 2, &BC7_WEIGHTS2)
}

fn decode_bc7_mode3_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let low = u64::from_le_bytes(block[0..8].try_into().expect("mode 3 low word"));
    let high = u64::from_le_bytes(block[8..16].try_into().expect("mode 3 high word"));
    let partition = ((low >> 4) & 0x3F) as usize;
    let partitions = &BC7_PARTITION2[partition * 16..partition * 16 + 16];
    let anchors = [0usize, BC7_ANCHOR_SECOND_SUBSET[partition] as usize];
    let p = [
        (high >> 30) & 0x01,
        (high >> 31) & 0x01,
        (high >> 32) & 0x01,
        (high >> 33) & 0x01,
    ];

    let endpoints = [
        [
            [
                (((low >> 10) & 0x7F) << 1 | p[0]) as i32,
                (((low >> 38) & 0x7F) << 1 | p[0]) as i32,
                (((high >> 2) & 0x7F) << 1 | p[0]) as i32,
            ],
            [
                (((low >> 17) & 0x7F) << 1 | p[1]) as i32,
                (((low >> 45) & 0x7F) << 1 | p[1]) as i32,
                (((high >> 9) & 0x7F) << 1 | p[1]) as i32,
            ],
        ],
        [
            [
                (((low >> 24) & 0x7F) << 1 | p[2]) as i32,
                (((low >> 52) & 0x7F) << 1 | p[2]) as i32,
                (((high >> 16) & 0x7F) << 1 | p[2]) as i32,
            ],
            [
                (((low >> 31) & 0x7F) << 1 | p[3]) as i32,
                ((((low >> 59) & 0x1F) | ((high & 0x03) << 5)) << 1 | p[3]) as i32,
                (((high >> 23) & 0x7F) << 1 | p[3]) as i32,
            ],
        ],
    ];
    decode_bc7_partitioned_rgb_error_bounded(source, max_error, partitions, &anchors, &endpoints, high >> 34, 2, &BC7_WEIGHTS2)
}

fn decode_bc7_mode4_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let rotation = ((block[0] >> 5) & 0x03) as usize;
    let index_flag = (block[0] >> 7) != 0;
    let mut x_bytes = [0u8; 8];
    x_bytes[0..5].copy_from_slice(&block[1..6]);
    let y_low = u32::from_le_bytes(block[6..10].try_into().expect("mode 4 y bytes")) as u64;
    let mut z_bytes = [0u8; 8];
    z_bytes[0..6].copy_from_slice(&block[10..16]);
    let z = u64::from_le_bytes(z_bytes);
    let x = u64::from_le_bytes(x_bytes) | ((y_low & 0x03) << 40);

    let lr = expand_5_endpoint(x & 0x1F);
    let hr = expand_5_endpoint((x >> 5) & 0x1F);
    let lg = expand_5_endpoint((x >> 10) & 0x1F);
    let hg = expand_5_endpoint((x >> 15) & 0x1F);
    let lb = expand_5_endpoint((x >> 20) & 0x1F);
    let hb = expand_5_endpoint((x >> 25) & 0x1F);
    let la = expand_6_endpoint((x >> 30) & 0x3F);
    let ha = expand_6_endpoint((x >> 36) & 0x3F);

    let p2_stream = y_low | ((z & 1) << 32);
    let p3_stream = z >> 1;
    let mut p2_bit_ofs = 2usize;
    let mut p3_bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let p2_bits = if i == 0 { 1 } else { 2 };
        let p3_bits = if i == 0 { 2 } else { 3 };
        let p2_index = ((p2_stream >> p2_bit_ofs) & ((1u64 << p2_bits) - 1)) as usize;
        let p3_index = ((p3_stream >> p3_bit_ofs) & ((1u64 << p3_bits) - 1)) as usize;
        p2_bit_ofs += p2_bits;
        p3_bit_ofs += p3_bits;

        let (rgb_weight, scalar_weight) = if index_flag {
            (BC7_WEIGHTS3[p3_index] as i32, BC7_WEIGHTS2[p2_index] as i32)
        } else {
            (BC7_WEIGHTS2[p2_index] as i32, BC7_WEIGHTS3[p3_index] as i32)
        };
        let mut decoded = [
            interpolate_bc7(lr, hr, rgb_weight),
            interpolate_bc7(lg, hg, rgb_weight),
            interpolate_bc7(lb, hb, rgb_weight),
            interpolate_bc7(la, ha, scalar_weight),
        ];
        unrotate_mode45_pixel(&mut decoded, rotation);

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode5_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let low = u64::from_le_bytes(block[0..8].try_into().expect("mode 5 low word"));
    let high = u64::from_le_bytes(block[8..16].try_into().expect("mode 5 high word"));
    let rotation = ((low >> 6) & 0x03) as usize;

    let lr = expand_7_endpoint((low >> 8) & 0x7F);
    let hr = expand_7_endpoint((low >> 15) & 0x7F);
    let lg = expand_7_endpoint((low >> 22) & 0x7F);
    let hg = expand_7_endpoint((low >> 29) & 0x7F);
    let lb = expand_7_endpoint((low >> 36) & 0x7F);
    let hb = expand_7_endpoint((low >> 43) & 0x7F);
    let la = ((low >> 50) & 0xFF) as i32;
    let ha = (((low >> 58) | ((high & 0x03) << 6)) & 0xFF) as i32;

    let rgb_stream = high >> 2;
    let alpha_stream = high >> 33;
    let mut rgb_bit_ofs = 0usize;
    let mut alpha_bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let bits = if i == 0 { 1 } else { 2 };
        let rgb_index = ((rgb_stream >> rgb_bit_ofs) & ((1u64 << bits) - 1)) as usize;
        let alpha_index = ((alpha_stream >> alpha_bit_ofs) & ((1u64 << bits) - 1)) as usize;
        rgb_bit_ofs += bits;
        alpha_bit_ofs += bits;

        let rgb_weight = BC7_WEIGHTS2[rgb_index] as i32;
        let alpha_weight = BC7_WEIGHTS2[alpha_index] as i32;
        let mut decoded = [
            interpolate_bc7(lr, hr, rgb_weight),
            interpolate_bc7(lg, hg, rgb_weight),
            interpolate_bc7(lb, hb, rgb_weight),
            interpolate_bc7(la, ha, alpha_weight),
        ];
        unrotate_mode45_pixel(&mut decoded, rotation);

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode1_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let part_id = (block[0] >> 2) as usize;
    let mut x_bytes = [0u8; 8];
    x_bytes.copy_from_slice(&block[1..9]);
    let x = u64::from_le_bytes(x_bytes);
    let mut y_bytes = [0u8; 8];
    y_bytes[0..6].copy_from_slice(&block[10..16]);
    let y = u64::from_le_bytes(y_bytes);

    let pbits = [(y & 1) as u32, ((y >> 1) & 1) as u32];
    let lr = [
        expand_mode1_endpoint(x & 0x3F, pbits[0]),
        expand_mode1_endpoint((x >> 12) & 0x3F, pbits[1]),
    ];
    let hr = [
        expand_mode1_endpoint((x >> 6) & 0x3F, pbits[0]),
        expand_mode1_endpoint((x >> 18) & 0x3F, pbits[1]),
    ];
    let lg = [
        expand_mode1_endpoint((x >> 24) & 0x3F, pbits[0]),
        expand_mode1_endpoint((x >> 36) & 0x3F, pbits[1]),
    ];
    let hg = [
        expand_mode1_endpoint((x >> 30) & 0x3F, pbits[0]),
        expand_mode1_endpoint((x >> 42) & 0x3F, pbits[1]),
    ];
    let lb1 = ((x >> 60) & 0xF) | (((block[9] & 0x03) as u64) << 4);
    let lb = [
        expand_mode1_endpoint((x >> 48) & 0x3F, pbits[0]),
        expand_mode1_endpoint(lb1, pbits[1]),
    ];
    let hb = [
        expand_mode1_endpoint((x >> 54) & 0x3F, pbits[0]),
        expand_mode1_endpoint(((block[9] >> 2) & 0x3F) as u64, pbits[1]),
    ];

    let partition = &BC7_PARTITION2[part_id * 16..part_id * 16 + 16];
    let anchor = BC7_ANCHOR_SECOND_SUBSET[part_id] as usize;
    let mut err = 0u64;
    let mut weight_bit_ofs = 2usize;
    for i in 0..16 {
        let subset = partition[i] as usize;
        let weight_bits = if i == 0 || i == anchor { 2 } else { 3 };
        let weight_index = ((y >> weight_bit_ofs) & ((1u64 << weight_bits) - 1)) as usize;
        weight_bit_ofs += weight_bits;
        let weight = BC7_WEIGHTS3[weight_index] as i32;
        let decoded = [
            interpolate_bc7(lr[subset], hr[subset], weight),
            interpolate_bc7(lg[subset], hg[subset], weight),
            interpolate_bc7(lb[subset], hb[subset], weight),
            255,
        ];

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode6_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let lo = u64::from_le_bytes(block[0..8].try_into().expect("BC7 block has 8-byte low word"));
    let hi = u64::from_le_bytes(block[8..16].try_into().expect("BC7 block has 8-byte high word"));

    let p0 = (lo >> 63) as u32;
    let p1 = (hi & 1) as u32;
    let lr = expand_mode6_endpoint((lo >> 7) & 0x7F, p0);
    let hr = expand_mode6_endpoint((lo >> 14) & 0x7F, p1);
    let lg = expand_mode6_endpoint((lo >> 21) & 0x7F, p0);
    let hg = expand_mode6_endpoint((lo >> 28) & 0x7F, p1);
    let lb = expand_mode6_endpoint((lo >> 35) & 0x7F, p0);
    let hb = expand_mode6_endpoint((lo >> 42) & 0x7F, p1);
    let la = expand_mode6_endpoint((lo >> 49) & 0x7F, p0);
    let ha = expand_mode6_endpoint((lo >> 56) & 0x7F, p1);

    let mut err = 0u64;
    let mut weight_bit_ofs = 1usize;
    for i in 0..16 {
        let weight_bits = if i == 0 { 3 } else { 4 };
        let weight_index = ((hi >> weight_bit_ofs) & ((1u64 << weight_bits) - 1)) as usize;
        weight_bit_ofs += weight_bits;
        let weight = BC7_WEIGHTS4[weight_index] as i32;
        let decoded = [
            interpolate_bc7(lr, hr, weight),
            interpolate_bc7(lg, hg, weight),
            interpolate_bc7(lb, hb, weight),
            interpolate_bc7(la, ha, weight),
        ];

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode7_error_bounded(
    block: &[u8; 16],
    source: &[[u8; 4]],
    max_error: u64,
) -> Option<u64> {
    let lo = u64::from_le_bytes(block[0..8].try_into().expect("BC7 block has 8-byte low word"));
    let hi = u64::from_le_bytes(block[8..16].try_into().expect("BC7 block has 8-byte high word"));

    let part_id = ((lo >> 8) & 0x3F) as usize;
    let pbits = [
        ((hi >> 30) & 1) as u32,
        ((hi >> 31) & 1) as u32,
        ((hi >> 32) & 1) as u32,
        ((hi >> 33) & 1) as u32,
    ];
    let lr = [
        expand_mode7_endpoint((lo >> 14) & 0x1F, pbits[0]),
        expand_mode7_endpoint((lo >> 24) & 0x1F, pbits[2]),
    ];
    let hr = [
        expand_mode7_endpoint((lo >> 19) & 0x1F, pbits[1]),
        expand_mode7_endpoint((lo >> 29) & 0x1F, pbits[3]),
    ];
    let lg = [
        expand_mode7_endpoint((lo >> 34) & 0x1F, pbits[0]),
        expand_mode7_endpoint((lo >> 44) & 0x1F, pbits[2]),
    ];
    let hg = [
        expand_mode7_endpoint((lo >> 39) & 0x1F, pbits[1]),
        expand_mode7_endpoint((lo >> 49) & 0x1F, pbits[3]),
    ];
    let lb = [
        expand_mode7_endpoint((lo >> 54) & 0x1F, pbits[0]),
        expand_mode7_endpoint(hi & 0x1F, pbits[2]),
    ];
    let hb = [
        expand_mode7_endpoint((lo >> 59) & 0x1F, pbits[1]),
        expand_mode7_endpoint((hi >> 5) & 0x1F, pbits[3]),
    ];
    let la = [
        expand_mode7_endpoint((hi >> 10) & 0x1F, pbits[0]),
        expand_mode7_endpoint((hi >> 20) & 0x1F, pbits[2]),
    ];
    let ha = [
        expand_mode7_endpoint((hi >> 15) & 0x1F, pbits[1]),
        expand_mode7_endpoint((hi >> 25) & 0x1F, pbits[3]),
    ];

    let partition = &BC7_PARTITION2[part_id * 16..part_id * 16 + 16];
    let anchor = BC7_ANCHOR_SECOND_SUBSET[part_id] as usize;
    let mut err = 0u64;
    let mut weight_bit_ofs = 34usize;
    for i in 0..16 {
        let subset = partition[i] as usize;
        let weight_bits = if i == 0 || i == anchor { 1 } else { 2 };
        let weight_index = ((hi >> weight_bit_ofs) & ((1u64 << weight_bits) - 1)) as usize;
        weight_bit_ofs += weight_bits;
        let weight = BC7_WEIGHTS2[weight_index] as i32;
        let decoded = [
            interpolate_bc7(lr[subset], hr[subset], weight),
            interpolate_bc7(lg[subset], hg[subset], weight),
            interpolate_bc7(lb[subset], hb[subset], weight),
            interpolate_bc7(la[subset], ha[subset], weight),
        ];

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_partitioned_rgb_error_bounded(
    source: &[[u8; 4]],
    max_error: u64,
    partitions: &[u8],
    anchors: &[usize],
    endpoints: &[[[i32; 3]; 2]],
    index_stream: u64,
    index_bits: usize,
    weights: &[u8],
) -> Option<u64> {
    let anchor_bits = index_bits - 1;
    let anchor_mask = (1u64 << anchor_bits) - 1;
    let index_mask = (1u64 << index_bits) - 1;
    let mut bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let subset = partitions[i] as usize;
        let bits = if anchors.contains(&i) { anchor_bits } else { index_bits };
        let mask = if bits == anchor_bits { anchor_mask } else { index_mask };
        let index = ((index_stream >> bit_ofs) & mask) as usize;
        bit_ofs += bits;

        let weight = weights[index] as i32;
        let decoded = [
            interpolate_bc7(endpoints[subset][0][0], endpoints[subset][1][0], weight),
            interpolate_bc7(endpoints[subset][0][1], endpoints[subset][1][1], weight),
            interpolate_bc7(endpoints[subset][0][2], endpoints[subset][1][2], weight),
            255,
        ];

        for c in 0..4 {
            let d = source[i][c] as i32 - decoded[c];
            err += (d * d) as u64;
        }
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

#[inline(always)]
fn expand_mode6_endpoint(v: u64, p: u32) -> i32 {
    ((v as u32) << 1 | p) as i32
}

#[inline(always)]
fn expand_mode1_endpoint(v: u64, p: u32) -> i32 {
    let x = ((v as u32) << 1) | p;
    ((x << 1) | (x >> 6)) as i32
}

#[inline(always)]
fn expand_mode7_endpoint(v: u64, p: u32) -> i32 {
    let x = ((v as u32) << 1) | p;
    ((x << 2) | (x >> 4)) as i32
}

#[inline(always)]
fn expand_5_endpoint(v: u64) -> i32 {
    let v = v as u32;
    ((v << 3) | (v >> 2)) as i32
}

#[inline(always)]
fn expand_6_endpoint(v: u64) -> i32 {
    let v = v as u32;
    ((v << 2) | (v >> 4)) as i32
}

#[inline(always)]
fn expand_7_endpoint(v: u64) -> i32 {
    let v = v as u32;
    ((v << 1) | (v >> 6)) as i32
}

#[inline(always)]
fn unrotate_mode45_pixel(pixel: &mut [i32; 4], rotation: usize) {
    if rotation != 0 {
        let dp_chan = rotation - 1;
        pixel.swap(dp_chan, 3);
    }
}

#[inline(always)]
fn interpolate_bc7(lo: i32, hi: i32, weight: i32) -> i32 {
    lo + (((hi - lo) * weight + 32) >> 6)
}

const BC7_WEIGHTS4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const BC7_WEIGHTS3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const BC7_WEIGHTS2: [u8; 4] = [0, 21, 43, 64];

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

fn relative_offset_candidate_count(len: usize, max_relative_delta: usize) -> usize {
    let mut count = 0usize;
    for src_ofs in 0usize..=(16 - len) {
        let dst_start = src_ofs.saturating_sub(max_relative_delta);
        let dst_end = (src_ofs + max_relative_delta).min(16 - len);
        count += dst_end - dst_start + 1;
    }
    count
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

    fn supported_blocks_mse(blocks: &[[u8; 16]], rgba_blocks: &[[u8; 4]]) -> f32 {
        let mut sse = 0u64;
        for (block_index, block) in blocks.iter().enumerate() {
            let pixels = &rgba_blocks[block_index * 16..(block_index + 1) * 16];
            sse += decode_bc7_error_bounded(block, pixels, get_bc7_mode(block), u64::MAX, None)
                .expect("RDO should only emit supported BC7 modes");
        }
        sse as f32 / (blocks.len() * 16 * 4) as f32
    }

    fn assert_bounded_error_exits_at_exact_error(
        actual: u64,
        error_fn: impl Fn(u64) -> Option<u64>,
    ) {
        assert_eq!(error_fn(u64::MAX), Some(actual));
        assert_eq!(error_fn(actual), None);
        assert_eq!(error_fn(actual + 1), Some(actual));
    }

    #[test]
    fn mode0_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4, 5, 6, 7, 0];
        crate::bc7::analytical::encode_mode0(
            &mut block,
            0,
            &[1, 4, 7],
            &[2, 5, 8],
            &[3, 6, 9],
            &[10, 12, 14],
            &[11, 13, 15],
            &[9, 10, 11],
            &[0, 1, 1, 0, 0, 1],
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(5).wrapping_add(11),
                (i as u8).wrapping_mul(7).wrapping_add(13),
                (i as u8).wrapping_mul(9).wrapping_add(17),
                255,
            ];
        }

        let actual = decode_bc7_mode0_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 0 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode0_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode6_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 7];
        crate::bc7::analytical::encode_mode6(
            &mut block,
            3,
            17,
            31,
            45,
            0,
            89,
            103,
            117,
            121,
            1,
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(7).wrapping_add(19),
                (i as u8).wrapping_mul(13).wrapping_add(23),
                (i as u8).wrapping_mul(17).wrapping_add(29),
                (i as u8).wrapping_mul(5).wrapping_add(31),
            ];
        }

        let actual = decode_bc7_mode6_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 6 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode6_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode1_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 0, 5, 6, 7, 1];
        crate::bc7::analytical::encode_mode1(
            &mut block,
            0,
            &[4, 12],
            &[20, 28],
            &[36, 44],
            &[58, 50],
            &[42, 34],
            &[26, 18],
            0,
            1,
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(17).wrapping_add(3),
                (i as u8).wrapping_mul(11).wrapping_add(7),
                (i as u8).wrapping_mul(5).wrapping_add(13),
                255,
            ];
        }

        let actual = decode_bc7_mode1_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 1 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode1_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode2_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 0, 3, 2, 1];
        crate::bc7::analytical::encode_mode2(
            &mut block,
            3,
            &[2, 8, 14],
            &[4, 10, 16],
            &[6, 12, 18],
            &[21, 24, 27],
            &[23, 26, 29],
            &[19, 22, 25],
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(3).wrapping_add(31),
                (i as u8).wrapping_mul(13).wrapping_add(5),
                (i as u8).wrapping_mul(17).wrapping_add(7),
                255,
            ];
        }

        let actual = decode_bc7_mode2_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 2 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode2_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode3_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 0, 3, 2, 1];
        crate::bc7::analytical::encode_mode3(
            &mut block,
            1,
            &[7, 31],
            &[11, 43],
            &[17, 53],
            &[73, 91],
            &[83, 101],
            &[67, 109],
            &[0, 1, 1, 0],
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(23).wrapping_add(2),
                (i as u8).wrapping_mul(29).wrapping_add(3),
                (i as u8).wrapping_mul(31).wrapping_add(5),
                255,
            ];
        }

        let actual = decode_bc7_mode3_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 3 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode3_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode4_fused_error_bounds() {
        let mut block = [0u8; 16];
        let w3 = [0, 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4, 5, 6, 7, 0];
        let w2 = [0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 0, 3, 2, 1];
        crate::bc7::analytical::encode_mode4(
            &mut block,
            3,
            8,
            13,
            9,
            21,
            25,
            29,
            47,
            &w3,
            &w2,
            1,
            1,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(17).wrapping_add(5),
                (i as u8).wrapping_mul(11).wrapping_add(9),
                (i as u8).wrapping_mul(23).wrapping_add(13),
                (i as u8).wrapping_mul(29).wrapping_add(3),
            ];
        }

        let actual = decode_bc7_mode4_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 4 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode4_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode5_fused_error_bounds() {
        let mut block = [0u8; 16];
        let wrgb = [0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 0, 3, 2, 1];
        let wa = [0, 1, 2, 3, 2, 1, 0, 3, 1, 2, 3, 0, 3, 2, 1, 0];
        crate::bc7::analytical::encode_mode5(
            &mut block,
            8,
            24,
            40,
            52,
            96,
            112,
            120,
            221,
            &wrgb,
            &wa,
            2,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(31).wrapping_add(1),
                (i as u8).wrapping_mul(3).wrapping_add(37),
                (i as u8).wrapping_mul(11).wrapping_add(41),
                (i as u8).wrapping_mul(19).wrapping_add(47),
            ];
        }

        let actual = decode_bc7_mode5_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 5 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode5_error_bounded(&block, &pixels, max_error)
        });
    }

    #[test]
    fn mode7_fused_error_bounds() {
        let mut block = [0u8; 16];
        let weights = [0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 0, 3, 2, 1];
        crate::bc7::analytical::encode_mode7(
            &mut block,
            0,
            &[4, 12],
            &[20, 28],
            &[8, 16],
            &[2, 22],
            &[26, 18],
            &[10, 6],
            &[30, 24],
            &[31, 9],
            &[0, 1, 1, 0],
            &weights,
        );
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [
                (i as u8).wrapping_mul(13).wrapping_add(3),
                (i as u8).wrapping_mul(19).wrapping_add(7),
                (i as u8).wrapping_mul(23).wrapping_add(11),
                (i as u8).wrapping_mul(29).wrapping_add(17),
            ];
        }

        let actual = decode_bc7_mode7_error_bounded(&block, &pixels, u64::MAX)
            .expect("unbounded fused mode 7 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode7_error_bounded(&block, &pixels, max_error)
        });
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
    fn test_bc7_analytical_and_rdo_quality_bounds() {
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

        let mse_non_rdo = supported_blocks_mse(&original_blocks, &rgba_blocks);
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

        let mse_rdo = supported_blocks_mse(&rdo_blocks, &rgba_blocks);
        println!("RDO BC7 MSE: {}", mse_rdo);
        
        // Rate-distortion optimizes for rate (entropy) as well, so distortion might be slightly higher,
        // but it should still be very reasonable and well within quality boundaries.
        assert!(mse_rdo < 250.0, "RDO BC7 quality is too low! MSE: {}", mse_rdo);
    }
}
