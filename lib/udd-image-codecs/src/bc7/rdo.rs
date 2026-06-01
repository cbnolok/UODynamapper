use super::tables::{
    BC7_ANCHOR_SECOND_SUBSET, BC7_ANCHOR_THIRD_SUBSET1, BC7_ANCHOR_THIRD_SUBSET2,
    BC7_PARTITION2, BC7_PARTITION3,
};
use rayon::prelude::*;
use std::cmp::max;

// ─── ERT constants (matching bc7enc_rdo ert.cpp) ─────────────────────────────
/// Bits charged per literal byte when estimating match cost.
const LITERAL_BITS: f32 = 13.0;
/// Reduced cost for a match that continues directly from the previous one.
const MATCH_CONTINUE_BITS: f32 = 1.0;
/// Reduced cost for a REP0 match (re-using the previous match distance).
const MATCH_REP0_BITS: f32 = 4.0;

const PARALLEL_RDO_BLOCK_THRESHOLD: usize = 2048;
const PARALLEL_RDO_MIN_CHUNK_BLOCKS: usize = 2048;
const RDO_PROGRESS_BLOCK_BATCH: usize = 256;
const ULTRASMOOTH_BLOCK_STD_DEV_THRESHOLD: f32 = 2.9;
const ULTRASMOOTH_DARK_THRESHOLD: f64 = 13.0;
const ULTRASMOOTH_BRIGHT_THRESHOLD: f64 = 222.0;
const ULTRASMOOTH_BLOCK_MSE_SCALE: f32 = 120.0;
const ULTRASMOOTH_REGION_TOO_SMALL_THRESHOLD: usize = 64;
const BC7_SEGMENT_MASKS: [u128; 17] = bc7_segment_masks();
const FIXED_RATE_SKIP_OFFSETS_THROUGH_LEN: [u64; 17] = fixed_rate_skip_offsets_through_len();
const MODE1_PIXEL_DESCS: [[u16; 16]; 64] = mode1_pixel_descs();
const MODE7_PIXEL_DESCS: [[u16; 16]; 64] = mode7_pixel_descs();

type RgbaBlock = [[u8; 4]; 16];

macro_rules! stat_add {
    ($collect:expr, $stats:ident, $field:ident, $value:expr) => {{
        if $collect {
            if let Some(stats) = $stats.as_deref_mut() {
                stats.$field += $value;
            }
        }
    }};
}

macro_rules! stat_array_add {
    ($collect:expr, $stats:ident, $field:ident, $index:expr, $value:expr) => {{
        if $collect {
            if let Some(stats) = $stats.as_deref_mut() {
                stats.$field[$index] += $value;
            }
        }
    }};
}

macro_rules! decode_bc7_error_bounded_for_stats {
    ($stats:ident, $block_bits:expr, $source:expr, $mode_hint:expr, $trust_mode_hint:expr, $max_error:expr) => {{
        if COLLECT_STATS {
            decode_bc7_error_bounded::<true>(
                $block_bits,
                $source,
                $mode_hint,
                $trust_mode_hint,
                $max_error,
                $stats.as_deref_mut(),
            )
        } else {
            decode_bc7_error_bounded::<false>(
                $block_bits,
                $source,
                $mode_hint,
                $trust_mode_hint,
                $max_error,
                None,
            )
        }
    }};
}

const fn bc7_segment_masks() -> [u128; 17] {
    let mut masks = [0u128; 17];
    let mut len = 1usize;
    while len < 16 {
        masks[len] = (1u128 << (len * 8)) - 1;
        len += 1;
    }
    masks[16] = u128::MAX;
    masks
}

const fn fixed_rate_skip_offsets_through_len() -> [u64; 17] {
    let mut offsets = [0u64; 17];
    let mut len = 3usize;
    let mut total = 0u64;
    while len <= 16 {
        total += (17 - len) as u64;
        offsets[len] = total;
        len += 1;
    }
    offsets
}

const fn mode1_pixel_descs() -> [[u16; 16]; 64] {
    let mut descs = [[0u16; 16]; 64];
    let mut partition_id = 0usize;
    while partition_id < 64 {
        let anchor = BC7_ANCHOR_SECOND_SUBSET[partition_id] as usize;
        let mut bit_ofs = 2usize;
        let mut pixel = 0usize;
        while pixel < 16 {
            let subset = BC7_PARTITION2[partition_id * 16 + pixel] as u16;
            let weight_bits = if pixel == 0 || pixel == anchor { 2usize } else { 3usize };
            let weight_mask = if weight_bits == 2 { 0x03u16 } else { 0x07u16 };
            descs[partition_id][pixel] =
                subset | ((bit_ofs as u16) << 1) | (weight_mask << 8);
            bit_ofs += weight_bits;
            pixel += 1;
        }
        partition_id += 1;
    }
    descs
}

const fn mode7_pixel_descs() -> [[u16; 16]; 64] {
    let mut descs = [[0u16; 16]; 64];
    let mut partition_id = 0usize;
    while partition_id < 64 {
        let anchor = BC7_ANCHOR_SECOND_SUBSET[partition_id] as usize;
        let mut bit_ofs = 34usize;
        let mut pixel = 0usize;
        while pixel < 16 {
            let subset = BC7_PARTITION2[partition_id * 16 + pixel] as u16;
            let weight_bits = if pixel == 0 || pixel == anchor { 1usize } else { 2usize };
            let weight_mask = if weight_bits == 1 { 0x01u16 } else { 0x03u16 };
            descs[partition_id][pixel] =
                subset | ((bit_ofs as u16) << 1) | (weight_mask << 8);
            bit_ofs += weight_bits;
            pixel += 1;
        }
        partition_id += 1;
    }
    descs
}

struct ModeHistory {
    entries: Vec<usize>,
    first_recent_index: usize,
}

impl ModeHistory {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            first_recent_index: 0,
        }
    }

    #[inline]
    fn push(&mut self, block_index: usize) {
        self.entries.push(block_index);
    }

    #[inline]
    fn recent_from(&mut self, first_block_to_check: usize) -> &[usize] {
        while self.first_recent_index < self.entries.len()
            && self.entries[self.first_recent_index] < first_block_to_check
        {
            self.first_recent_index += 1;
        }
        &self.entries[self.first_recent_index..]
    }
}

#[derive(Clone, Copy)]
struct RelativeCandidate {
    src_shift: u8,
    dst_shift: u8,
    dst_ofs: u8,
    dist_index: u8,
}

struct RelativeCandidateLayout {
    candidates_by_len: [Vec<RelativeCandidate>; 17],
    candidate_count_by_len: [u64; 17],
    skipped_offsets_by_len: [u64; 17],
}

impl RelativeCandidateLayout {
    fn new(max_relative_delta: usize) -> Self {
        let mut candidate_count_by_len = [0u64; 17];
        let mut skipped_offsets_by_len = [0u64; 17];
        let candidates_by_len = std::array::from_fn(|len| {
            let mut candidates = Vec::new();
            if !(3..=16).contains(&len) {
                return candidates;
            }

            for src_ofs in 0usize..=(16 - len) {
                let full_dst_count = 17 - len;
                let dst_start = src_ofs.saturating_sub(max_relative_delta);
                let dst_end = (src_ofs + max_relative_delta).min(16 - len);
                let dst_count = dst_end - dst_start + 1;
                skipped_offsets_by_len[len] += (full_dst_count - dst_count) as u64;

                for dst_ofs in dst_start..=dst_end {
                    candidates.push(RelativeCandidate {
                        src_shift: (src_ofs * 8) as u8,
                        dst_shift: (dst_ofs * 8) as u8,
                        dst_ofs: dst_ofs as u8,
                        dist_index: (dst_ofs as i32 - src_ofs as i32 + 15) as u8,
                    });
                }
            }
            candidate_count_by_len[len] = candidates.len() as u64;
            candidates
        });

        Self {
            candidates_by_len,
            candidate_count_by_len,
            skipped_offsets_by_len,
        }
    }
}

struct SecondMatchLayout {
    offsets_by_span_len: Vec<[Vec<u8>; 17]>,
    overlap_count_by_span_len: Vec<[u64; 17]>,
}

impl SecondMatchLayout {
    fn new() -> Self {
        let mut offsets_by_span_len = Vec::with_capacity(16 * 17);
        let mut overlap_count_by_span_len = Vec::with_capacity(16 * 17);

        for first_start in 0usize..16 {
            for first_len in 0usize..=16 {
                let first_end = first_start + first_len;
                let mut overlap_counts = [0u64; 17];
                let offsets_by_len = std::array::from_fn(|len| {
                    let mut offsets = Vec::new();
                    if !(3..=16).contains(&len) || first_len == 0 || first_end > 16 {
                        return offsets;
                    }

                    for ofs in 0usize..=(16 - len) {
                        if ofs < first_end && ofs + len > first_start {
                            overlap_counts[len] += 1;
                        } else {
                            offsets.push(ofs as u8);
                        }
                    }
                    offsets
                });

                offsets_by_span_len.push(offsets_by_len);
                overlap_count_by_span_len.push(overlap_counts);
            }
        }

        Self {
            offsets_by_span_len,
            overlap_count_by_span_len,
        }
    }

    #[inline(always)]
    fn span_index(first_start: usize, first_len: usize) -> usize {
        first_start * 17 + first_len
    }

    #[inline(always)]
    fn offsets(&self, first_start: usize, first_len: usize, len: usize) -> &[u8] {
        &self.offsets_by_span_len[Self::span_index(first_start, first_len)][len]
    }

    #[inline(always)]
    fn overlap_count(&self, first_start: usize, first_len: usize, len: usize) -> u64 {
        self.overlap_count_by_span_len[Self::span_index(first_start, first_len)][len]
    }
}

struct DistanceCostLayout {
    normal_bits_by_delta: Vec<f32>,
    normal_match_bits_by_delta: Vec<[f32; 17]>,
    normal_trial_lambda_by_delta: Vec<[f32; 17]>,
    relative_bits_by_delta: Option<Vec<[f32; 31]>>,
}

impl DistanceCostLayout {
    fn new(max_block_delta: usize, include_relative: bool, rate_costs: &RateCostLayout) -> Self {
        let mut normal_bits_by_delta = vec![0.0f32; max_block_delta + 1];
        let mut normal_match_bits_by_delta = vec![[0.0f32; 17]; max_block_delta + 1];
        let mut normal_trial_lambda_by_delta = vec![[0.0f32; 17]; max_block_delta + 1];
        let mut relative_bits_by_delta = if include_relative {
            Some(vec![[0.0f32; 31]; max_block_delta + 1])
        } else {
            None
        };

        for block_delta in 1..=max_block_delta {
            let dist = (block_delta * 16) as u32;
            let normal_bits = compute_dist_cost_estimate(dist) as f32;
            normal_bits_by_delta[block_delta] = normal_bits;
            for len in 3..=16 {
                let normal_match_bits = normal_bits + rate_costs.match_len_bits[len];
                normal_match_bits_by_delta[block_delta][len] = normal_match_bits;
                normal_trial_lambda_by_delta[block_delta][len] =
                    (rate_costs.literal_bits_by_match_len[len] + normal_match_bits) * rate_costs.lambda;
            }
            if let Some(ref mut relative_bits_by_delta) = relative_bits_by_delta {
                let relative_bits = compute_relative_dist_costs(dist);
                for (dst, src) in relative_bits_by_delta[block_delta]
                    .iter_mut()
                    .zip(relative_bits.iter())
                {
                    *dst = *src as f32;
                }
            }
        }

        Self {
            normal_bits_by_delta,
            normal_match_bits_by_delta,
            normal_trial_lambda_by_delta,
            relative_bits_by_delta,
        }
    }

    #[inline(always)]
    fn normal_bits(&self, block_delta: usize) -> f32 {
        self.normal_bits_by_delta[block_delta]
    }

    #[inline(always)]
    fn normal_match_bits(&self, block_delta: usize, len: usize) -> f32 {
        self.normal_match_bits_by_delta[block_delta][len]
    }

    #[inline(always)]
    fn normal_trial_lambda(&self, block_delta: usize, len: usize) -> f32 {
        self.normal_trial_lambda_by_delta[block_delta][len]
    }

    #[inline(always)]
    fn relative_bits(&self, block_delta: usize) -> &[f32; 31] {
        &self
            .relative_bits_by_delta
            .as_ref()
            .expect("relative distance costs exist when relative movement is enabled")[block_delta]
    }
}

struct RateCostLayout {
    lambda: f32,
    match_len_bits: [f32; 17],
    literal_bits_by_match_len: [f32; 17],
    continuation_trial_lambda_by_len: [f32; 17],
    rep0_trial_lambda_by_len: [f32; 17],
}

impl RateCostLayout {
    fn new(lambda: f32) -> Self {
        let match_len_bits = compute_match_len_bits();
        let literal_bits_by_match_len = compute_literal_bits_by_match_len();
        let mut continuation_trial_lambda_by_len = [0.0f32; 17];
        let mut rep0_trial_lambda_by_len = [0.0f32; 17];
        for len in 3..=16 {
            continuation_trial_lambda_by_len[len] =
                (literal_bits_by_match_len[len] + MATCH_CONTINUE_BITS) * lambda;
            rep0_trial_lambda_by_len[len] =
                (literal_bits_by_match_len[len] + MATCH_REP0_BITS) * lambda;
        }

        Self {
            lambda,
            match_len_bits,
            literal_bits_by_match_len,
            continuation_trial_lambda_by_len,
            rep0_trial_lambda_by_len,
        }
    }
}

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
            lookback_window_size: 16 * 64,
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
    reduce_entropy_bc7_parallel_impl(blocks, rgba_blocks, blocks_x, blocks_y, params, None)
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

pub fn reduce_entropy_bc7_with_progress<F>(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    progress: F,
) -> u32
where
    F: Fn(usize),
{
    reduce_entropy_bc7_impl_with_progress::<false>(
        blocks,
        rgba_blocks,
        blocks_x,
        blocks_y,
        params,
        None,
        Some(&progress),
    )
}

pub fn reduce_entropy_bc7_parallel(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
) -> u32 {
    reduce_entropy_bc7_parallel_impl(blocks, rgba_blocks, blocks_x, blocks_y, params, None)
}

pub fn reduce_entropy_bc7_parallel_with_progress<F>(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    progress: F,
) -> u32
where
    F: Fn(usize) + Sync,
{
    reduce_entropy_bc7_parallel_impl(blocks, rgba_blocks, blocks_x, blocks_y, params, Some(&progress))
}

fn reduce_entropy_bc7_impl(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    mut stats: Option<&mut Bc7RdoStats>,
) -> u32 {
    if let Some(stats) = stats.as_deref_mut() {
        reduce_entropy_bc7_impl_with_progress::<true>(
            blocks,
            rgba_blocks,
            blocks_x,
            blocks_y,
            params,
            Some(stats),
            None,
        )
    } else {
        reduce_entropy_bc7_impl_with_progress::<false>(
            blocks,
            rgba_blocks,
            blocks_x,
            blocks_y,
            params,
            None,
            None,
        )
    }
}

fn reduce_entropy_bc7_parallel_impl(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    progress: Option<&(dyn Fn(usize) + Sync)>,
) -> u32 {
    let num_blocks = blocks.len();
    debug_assert_eq!(num_blocks, blocks_x * blocks_y);
    debug_assert_eq!(rgba_blocks.len(), num_blocks * 16);

    if params.lambda <= 0.0 || num_blocks < PARALLEL_RDO_BLOCK_THRESHOLD || blocks_x == 0 {
        return reduce_entropy_bc7_impl_with_progress::<false>(
            blocks,
            rgba_blocks,
            blocks_x,
            blocks_y,
            params,
            None,
            progress.map(|progress| progress as &dyn Fn(usize)),
        );
    }

    let lookback_blocks = max(1, params.lookback_window_size / 16);
    let chunk_blocks =
        parallel_rdo_chunk_blocks(num_blocks, blocks_x, lookback_blocks, rayon::current_num_threads());

    blocks
        .par_chunks_mut(chunk_blocks)
        .zip(rgba_blocks.par_chunks(chunk_blocks * 16))
        .map(|(block_chunk, rgba_chunk)| {
            let chunk_blocks_y = block_chunk.len() / blocks_x;
            reduce_entropy_bc7_impl_with_progress::<false>(
                block_chunk,
                rgba_chunk,
                blocks_x,
                chunk_blocks_y,
                params,
                None,
                progress.map(|progress| progress as &dyn Fn(usize)),
            )
        })
        .sum()
}

fn parallel_rdo_chunk_blocks(
    num_blocks: usize,
    blocks_x: usize,
    lookback_blocks: usize,
    thread_count: usize,
) -> usize {
    let lookback_rows = lookback_blocks.div_ceil(blocks_x);
    let target_parallel_blocks = (num_blocks / (thread_count * 2).max(1)).max(1);
    let chunk_blocks = max(
        PARALLEL_RDO_MIN_CHUNK_BLOCKS,
        max(lookback_blocks, target_parallel_blocks),
    );
    let chunk_rows = max(1, chunk_blocks.div_ceil(blocks_x).max(lookback_rows));
    chunk_rows * blocks_x
}

fn reduce_entropy_bc7_impl_with_progress<const COLLECT_STATS: bool>(
    blocks: &mut [[u8; 16]],
    rgba_blocks: &[[u8; 4]],
    blocks_x: usize,
    blocks_y: usize,
    params: &Bc7RdoParams,
    mut stats: Option<&mut Bc7RdoStats>,
    progress: Option<&dyn Fn(usize)>,
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
        let lambda_scale = params.lambda.min(3.0);
        if scales.len() < PARALLEL_RDO_BLOCK_THRESHOLD {
            for s in scales.iter_mut() {
                if *s > 0.0 {
                    *s = actual_params.smooth_block_max_mse_scale.max(*s * lambda_scale);
                }
            }
        } else {
            scales.par_iter_mut().for_each(|s| {
                if *s > 0.0 {
                    *s = actual_params.smooth_block_max_mse_scale.max(*s * lambda_scale);
                }
            });
        }
    }

    // 4. Main loop
    let total_blocks_to_check = max(1, params.lookback_window_size / 16);
    let rate_costs = RateCostLayout::new(params.lambda);
    let max_block_delta = total_blocks_to_check.min(num_blocks.saturating_sub(1)).max(1);
    let distance_cost_layout =
        DistanceCostLayout::new(max_block_delta, params.allow_relative_movement, &rate_costs);
    let mut hash_table = vec![0u64; 8192];
    let hash_mask = hash_table.len() - 1;
    let (mut block_bits, mut block_modes) = if num_blocks < PARALLEL_RDO_BLOCK_THRESHOLD {
        let block_bits = blocks.iter().map(|block| bc7_block_bits(*block)).collect::<Vec<_>>();
        let block_modes = block_bits.iter().map(|&bits| get_bc7_mode_bits(bits)).collect::<Vec<_>>();
        (block_bits, block_modes)
    } else {
        let block_bits = blocks.par_iter().map(|block| bc7_block_bits(*block)).collect::<Vec<_>>();
        let block_modes = block_bits.par_iter().map(|&bits| get_bc7_mode_bits(bits)).collect::<Vec<_>>();
        (block_bits, block_modes)
    };
    let history_capacity = total_blocks_to_check.min(num_blocks).max(1);
    let mut previous_blocks_by_mode: [ModeHistory; 8] =
        std::array::from_fn(|_| ModeHistory::with_capacity(history_capacity));
    let relative_candidate_layout = if params.allow_relative_movement {
        Some(RelativeCandidateLayout::new(params.relative_movement_max_offset_delta.min(15)))
    } else {
        None
    };
    let second_match_layout = if params.try_two_matches {
        Some(SecondMatchLayout::new())
    } else {
        None
    };

    // REP0 and match-continuation tracking (ert.cpp ERT_FAVOR_CONT_AND_REP0_MATCHES):
    //   prev_cont_window_ofs: source-window offset just past the last accepted match end.
    //                         The next block can "continue" it for only MATCH_CONTINUE_BITS.
    //   prev_rep0_dist:       byte distance of the last accepted match for cheap REP0 reuse.
    let mut prev_cont_window_ofs: i64 = -1;
    let mut prev_rep0_dist:       i64 = -1;
    let mut pending_progress = 0usize;

    for block_index in 0..num_blocks {
        let orig_bits = block_bits[block_index];
        let p_pixels = rgba_block_at(rgba_blocks, block_index);
        let bc7_mode = block_modes[block_index];
        if bc7_mode == 8 {
            report_progress(progress, &mut pending_progress, 1);
            continue; // Invalid block or mode 8 (reserved)
        }

        let cur_err = decode_bc7_error_bounded_for_stats!(stats, orig_bits, p_pixels, bc7_mode, true, u64::MAX)
            .expect("u64::MAX cannot be exceeded by a 4x4 RGBA block error");

        if params.skip_zero_mse_blocks && cur_err == 0 {
            previous_blocks_by_mode[bc7_mode as usize].push(block_index);
            report_progress(progress, &mut pending_progress, 1);
            continue;
        }

        let smooth_block_error_scale = if let Some(ref scales) = block_mse_scales {
            let scale = scales[block_index];
            if scale > 0.0 {
                scale
            } else {
                smooth_block_error_scale_from_pixels(
                    p_pixels,
                    actual_params.max_smooth_block_std_dev,
                    actual_params.smooth_block_max_mse_scale,
                )
            }
        } else {
            smooth_block_error_scale_from_pixels(
                p_pixels,
                actual_params.max_smooth_block_std_dev,
                actual_params.smooth_block_max_mse_scale,
            )
        };

        let cur_ms_err = cur_err as f32 / 64.0;
        let cur_t = cur_ms_err * smooth_block_error_scale + (LITERAL_BITS * 16.0) * params.lambda;
        let trial_error_scale = max_trial_error_scale(smooth_block_error_scale);
        let first_block_to_check = block_index.saturating_sub(total_blocks_to_check);
        let hash_epoch = ((block_index as u64) + 1) << 32;

        let mut best_bits = orig_bits;
        let mut best_t = cur_t;
        let mut best_ms_err = cur_ms_err;
        let mut best_match_len = 0usize;
        let mut best_match_dst_block_ofs = 0usize;
        let mut best_match_bits = 0.0f32;

        let thresh_ms_err = params.max_allowed_rms_increase_ratio
            * params.max_allowed_rms_increase_ratio
            * cur_ms_err.max(1.0);

        if params.allow_relative_movement {
            // ── Main search window: full relative-offset search ──
            let relative_candidate_layout = relative_candidate_layout
                .as_ref()
                .expect("relative candidate layout exists when relative movement is enabled");
            let max_relative_previous_blocks = params.relative_movement_max_previous_blocks;
            let min_relative_match_len = params.relative_movement_min_match_len.clamp(3, 16);
            let mut relative_previous_blocks_checked = 0usize;
            let previous_blocks = previous_blocks_by_mode[bc7_mode as usize]
                .recent_from(first_block_to_check);
            for &prev_block_index in previous_blocks.iter().rev() {
                if max_relative_previous_blocks > 0
                    && relative_previous_blocks_checked >= max_relative_previous_blocks
                {
                    stat_add!(COLLECT_STATS, stats, relative_previous_block_limit_hits, 1);
                    break;
                }
                relative_previous_blocks_checked += 1;
                let prev_bits = block_bits[prev_block_index];
                let block_delta = block_index - prev_block_index;
                let relative_dist_bits = distance_cost_layout.relative_bits(block_delta);
                if COLLECT_STATS {
                    for len in 3..min_relative_match_len {
                        stat_add!(
                            COLLECT_STATS,
                            stats,
                            relative_length_skips,
                            relative_candidate_layout.candidate_count_by_len[len]
                        );
                    }
                }
                for len in (min_relative_match_len..=16).rev() {
                    let segment_mask = BC7_SEGMENT_MASKS[len];
                    let len_bits = rate_costs.match_len_bits[len];
                    stat_add!(
                        COLLECT_STATS,
                        stats,
                        relative_offset_skips,
                        relative_candidate_layout.skipped_offsets_by_len[len]
                    );
                    for candidate in &relative_candidate_layout.candidates_by_len[len] {
                        let dst_ofs = candidate.dst_ofs as usize;
                        stat_add!(COLLECT_STATS, stats, candidate_checks, 1);
                        let mb = relative_dist_bits[candidate.dist_index as usize] + len_bits;
                        let trial_bits = rate_costs.literal_bits_by_match_len[len] + mb;
                        let trial_bits_times_lambda = trial_bits * params.lambda;
                        if trial_bits_times_lambda >= best_t {
                            stat_add!(COLLECT_STATS, stats, rate_skips, 1);
                            continue;
                        }

                        // Hash check to skip redundant trials
                        let prev_segment = (prev_bits >> candidate.src_shift) & segment_mask;
                        let hs = hash_hsieh_bc7_segment(prev_segment, len, candidate.dst_ofs as u32);
                        if rdo_hash_seen(&mut hash_table, hash_mask, hash_epoch, hs) {
                            stat_add!(COLLECT_STATS, stats, hash_skips, 1);
                            continue;
                        }

                        if prev_segment == ((orig_bits >> candidate.dst_shift) & segment_mask) {
                            stat_add!(COLLECT_STATS, stats, original_block_skips, 1);
                            let trial_ms_err = cur_ms_err;
                            if trial_ms_err < thresh_ms_err {
                                let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                                if t < best_t {
                                    best_t = t; best_bits = orig_bits;
                                    best_ms_err = trial_ms_err;
                                    best_match_len = len; best_match_dst_block_ofs = dst_ofs;
                                    best_match_bits = mb;
                                    stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                                }
                            }
                            continue;
                        }
                        let trial_bits =
                            bc7_copy_segment_bits_from_segment(
                                orig_bits,
                                prev_segment,
                                candidate.dst_shift as usize,
                                segment_mask,
                            );
                        let trust_mode_hint = candidate.dst_shift > 0;
                        stat_add!(COLLECT_STATS, stats, decode_trials, 1);
                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, trial_error_scale);
                        let Some(trial_err) = decode_bc7_error_bounded_for_stats!(
                            stats,
                            trial_bits,
                            p_pixels,
                            bc7_mode,
                            trust_mode_hint,
                            max_trial_err
                        ) else {
                            stat_add!(COLLECT_STATS, stats, bounded_error_exits, 1);
                            continue;
                        };
                        let trial_ms_err = trial_err as f32 / 64.0;
                        if trial_ms_err < thresh_ms_err {
                            let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                            if t < best_t {
                                best_t = t; best_bits = trial_bits;
                                best_ms_err = trial_ms_err;
                                best_match_len = len; best_match_dst_block_ofs = dst_ofs;
                                best_match_bits = mb;
                                stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                            }
                        }
                    }
                }
            }
        } else {
            // ── Main search window: fixed-offset default path ──
            let previous_blocks = previous_blocks_by_mode[bc7_mode as usize]
                .recent_from(first_block_to_check);
            for &prev_block_index in previous_blocks.iter().rev() {
                let prev_bits = block_bits[prev_block_index];
                let block_delta = block_index - prev_block_index;
                let dist = block_delta * 16;
                let dist_i64 = dist as i64;
                let prev_block_base_i64 = (prev_block_index * 16) as i64;
                for len in (3..=16).rev() {
                    let segment_mask = BC7_SEGMENT_MASKS[len];
                    // Fixed-offset search: src_ofs == dst_ofs
                    let normal_match_bits = distance_cost_layout.normal_match_bits(block_delta, len);
                    let normal_trial_bits_times_lambda =
                        distance_cost_layout.normal_trial_lambda(block_delta, len);
                    let continuation_possible = prev_block_base_i64 == prev_cont_window_ofs;
                    let rep0_possible = prev_rep0_dist >= 0 && dist_i64 == prev_rep0_dist;
                    if normal_trial_bits_times_lambda >= best_t
                        && !continuation_possible
                        && !rep0_possible
                    {
                        let skipped_offsets = FIXED_RATE_SKIP_OFFSETS_THROUGH_LEN[len];
                        stat_add!(COLLECT_STATS, stats, candidate_checks, skipped_offsets);
                        stat_add!(COLLECT_STATS, stats, rate_skips, skipped_offsets);
                        break;
                    }

                    if !continuation_possible && !rep0_possible {
                        macro_rules! reduce_len {
                            ($len:literal) => {
                                reduce_fixed_normal_len::<$len, COLLECT_STATS>(
                                    prev_bits, orig_bits, p_pixels, bc7_mode, dist_i64,
                                    prev_block_base_i64, normal_match_bits,
                                    normal_trial_bits_times_lambda, cur_ms_err, thresh_ms_err,
                                    smooth_block_error_scale, trial_error_scale, &mut hash_table,
                                    hash_mask, hash_epoch, &mut best_t, &mut best_bits,
                                    &mut best_ms_err, &mut best_match_len,
                                    &mut best_match_dst_block_ofs, &mut best_match_bits,
                                    &mut prev_cont_window_ofs, &mut prev_rep0_dist, &mut stats,
                                )
                            };
                        }
                        match len {
                            16 => reduce_len!(16),
                            15 => reduce_len!(15),
                            14 => reduce_len!(14),
                            13 => reduce_len!(13),
                            12 => reduce_len!(12),
                            11 => reduce_len!(11),
                            10 => reduce_len!(10),
                            9 => reduce_len!(9),
                            8 => reduce_len!(8),
                            7 => reduce_len!(7),
                            6 => reduce_len!(6),
                            5 => reduce_len!(5),
                            4 => reduce_len!(4),
                            3 => reduce_len!(3),
                            _ => unreachable!("fixed BC7 RDO lengths are 3..=16"),
                        }
                        continue;
                    }

                    for ofs in 0..=(16 - len) {
                        stat_add!(COLLECT_STATS, stats, candidate_checks, 1);
                        let shift = ofs * 8;
                        let mut prev_segment = 0u128;
                        let mut prev_segment_loaded = false;

                        // REP0 / match-continuation cost reduction (ERT_FAVOR_CONT_AND_REP0_MATCHES)
                        let (trial_match_bits, trial_bits_times_lambda) =
                            if prev_block_base_i64 == prev_cont_window_ofs && ofs == 0 {
                                // Continuation: the match continues directly from the previous block's match
                                (MATCH_CONTINUE_BITS, rate_costs.continuation_trial_lambda_by_len[len])
                            } else if prev_rep0_dist >= 0 && dist_i64 == prev_rep0_dist {
                                // REP0: re-using the last accepted match distance costs only MATCH_REP0_BITS
                                (MATCH_REP0_BITS, rate_costs.rep0_trial_lambda_by_len[len])
                            } else {
                                if normal_trial_bits_times_lambda >= best_t {
                                    stat_add!(COLLECT_STATS, stats, rate_skips, 1);
                                    continue;
                                }
                                // Normal match: deduplicate via hash before decoding
                                prev_segment = (prev_bits >> shift) & segment_mask;
                                prev_segment_loaded = true;
                                let hs = hash_hsieh_bc7_segment(prev_segment, len, ofs as u32);
                                if rdo_hash_seen(&mut hash_table, hash_mask, hash_epoch, hs) {
                                    stat_add!(COLLECT_STATS, stats, hash_skips, 1);
                                    continue;
                                }
                                (normal_match_bits, normal_trial_bits_times_lambda)
                            };
                        if trial_bits_times_lambda >= best_t {
                            stat_add!(COLLECT_STATS, stats, rate_skips, 1);
                            continue;
                        }

                        if !prev_segment_loaded {
                            prev_segment = (prev_bits >> shift) & segment_mask;
                        }
                        if prev_segment == ((orig_bits >> shift) & segment_mask) {
                            stat_add!(COLLECT_STATS, stats, original_block_skips, 1);
                            let trial_ms_err = cur_ms_err;
                            if trial_ms_err < thresh_ms_err {
                                let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                                if t < best_t {
                                    best_t = t; best_bits = orig_bits;
                                    best_ms_err = trial_ms_err;
                                    best_match_len = len; best_match_dst_block_ofs = ofs;
                                    best_match_bits = trial_match_bits;
                                    prev_cont_window_ofs = prev_block_base_i64 + ofs as i64 + len as i64;
                                    prev_rep0_dist       = dist_i64;
                                    stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                                }
                            }
                            continue;
                        }
                        let trial_bits =
                            bc7_copy_segment_bits_from_segment(
                                orig_bits,
                                prev_segment,
                                shift,
                                segment_mask,
                            );
                        if ofs == 0 && !bc7_block_bits_has_mode(trial_bits, bc7_mode) {
                            stat_add!(COLLECT_STATS, stats, unsupported_mode_trials, 1);
                            continue;
                        }
                        stat_add!(COLLECT_STATS, stats, decode_trials, 1);
                        let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, trial_error_scale);
                        let Some(trial_err) = decode_bc7_error_bounded_for_stats!(
                            stats,
                            trial_bits,
                            p_pixels,
                            bc7_mode,
                            true,
                            max_trial_err
                        ) else {
                            stat_add!(COLLECT_STATS, stats, bounded_error_exits, 1);
                            continue;
                        };
                        let trial_ms_err = trial_err as f32 / 64.0;
                        if trial_ms_err < thresh_ms_err {
                            let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                            if t < best_t {
                                best_t = t; best_bits = trial_bits;
                                best_ms_err = trial_ms_err;
                                best_match_len = len; best_match_dst_block_ofs = ofs;
                                best_match_bits = trial_match_bits;
                                // Update continuation/REP0 state for the next block
                                prev_cont_window_ofs = prev_block_base_i64 + ofs as i64 + len as i64;
                                prev_rep0_dist       = dist_i64;
                                stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                            }
                        }
                    }
                }
            }
        }

        // Try a second non-overlapping match — only attempted when the first was accepted (best_t < cur_t)
        if params.try_two_matches && best_t < cur_t && best_match_len > 0 && best_match_len <= (16 - 3) {
            let second_match_layout = second_match_layout
                .as_ref()
                .expect("second-match layout exists when second matches are enabled");
            let orig_best_bits = best_bits;
            let orig_best_ms_err = best_ms_err;

            let previous_blocks = previous_blocks_by_mode[bc7_mode as usize]
                .recent_from(first_block_to_check);
            for &prev_block_index in previous_blocks.iter().rev() {
                let prev_bits = block_bits[prev_block_index];

                let block_delta = block_index - prev_block_index;
                let dist_bits = distance_cost_layout.normal_bits(block_delta);
                for len in 3..=(16 - best_match_len) {
                    let segment_mask = BC7_SEGMENT_MASKS[len];
                    let trial_bits = (16.0 - len as f32 - best_match_len as f32) * LITERAL_BITS
                        + dist_bits
                        + rate_costs.match_len_bits[len]
                        + best_match_bits;
                    let trial_bits_times_lambda = trial_bits * params.lambda;
                    if trial_bits_times_lambda >= best_t {
                        let skipped_offsets = (17 - len) as u64;
                        stat_add!(COLLECT_STATS, stats, candidate_checks, skipped_offsets);
                        stat_add!(COLLECT_STATS, stats, rate_skips, skipped_offsets);
                        continue;
                    }

                    stat_add!(
                        COLLECT_STATS,
                        stats,
                        candidate_checks,
                        second_match_layout.overlap_count(best_match_dst_block_ofs, best_match_len, len)
                    );
                    for &ofs in second_match_layout.offsets(best_match_dst_block_ofs, best_match_len, len) {
                        let ofs = ofs as usize;
                        stat_add!(COLLECT_STATS, stats, candidate_checks, 1);

                        let shift = ofs * 8;
                        let prev_segment = (prev_bits >> shift) & segment_mask;
                        let (trial_bits, trial_ms_err) =
                            if prev_segment == ((orig_best_bits >> shift) & segment_mask) {
                                (orig_best_bits, orig_best_ms_err)
                            } else {
                                let trial_bits =
                                    bc7_copy_segment_bits_from_segment(
                                        orig_best_bits,
                                        prev_segment,
                                        shift,
                                        segment_mask,
                                    );
                                let trust_mode_hint = !params.allow_relative_movement || ofs > 0;
                                if !params.allow_relative_movement
                                    && ofs == 0
                                    && !bc7_block_bits_has_mode(trial_bits, bc7_mode)
                                {
                                    stat_add!(COLLECT_STATS, stats, unsupported_mode_trials, 1);
                                    continue;
                                }

                                stat_add!(COLLECT_STATS, stats, decode_trials, 1);
                                let max_trial_err = max_trial_error(best_t, trial_bits_times_lambda, trial_error_scale);
                                let Some(trial_err) = decode_bc7_error_bounded_for_stats!(
                                    stats,
                                    trial_bits,
                                    p_pixels,
                                    bc7_mode,
                                    trust_mode_hint,
                                    max_trial_err
                                ) else {
                                    stat_add!(COLLECT_STATS, stats, bounded_error_exits, 1);
                                    continue;
                                };
                                (trial_bits, trial_err as f32 / 64.0)
                            };
                        if trial_ms_err < thresh_ms_err {
                            let t = trial_ms_err * smooth_block_error_scale + trial_bits_times_lambda;
                            if t < best_t {
                                best_t = t;
                                best_bits = trial_bits;
                                stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                            }
                        }
                    }
                }
            }
        }

        if best_t < cur_t {
            let best_block = best_bits.to_le_bytes();
            blocks[block_index] = best_block;
            block_modes[block_index] = get_bc7_mode_bits(best_bits);
            block_bits[block_index] = best_bits;
            total_modified += 1;
            stat_add!(COLLECT_STATS, stats, modified_blocks, 1);
        }
        if block_modes[block_index] < 8 {
            previous_blocks_by_mode[block_modes[block_index] as usize].push(block_index);
        }
        report_progress(progress, &mut pending_progress, 1);
    }
    flush_progress(progress, &mut pending_progress);

    total_modified
}

#[inline]
fn report_progress(progress: Option<&dyn Fn(usize)>, pending_progress: &mut usize, blocks: usize) {
    if progress.is_some() {
        *pending_progress += blocks;
        if *pending_progress >= RDO_PROGRESS_BLOCK_BATCH {
            flush_progress(progress, pending_progress);
        }
    }
}

#[inline]
fn flush_progress(progress: Option<&dyn Fn(usize)>, pending_progress: &mut usize) {
    if *pending_progress > 0 {
        if let Some(progress) = progress {
            progress(*pending_progress);
        }
        *pending_progress = 0;
    }
}

#[inline(always)]
fn reduce_fixed_normal_len<const LEN: usize, const COLLECT_STATS: bool>(
    prev_bits: u128,
    orig_bits: u128,
    p_pixels: &RgbaBlock,
    bc7_mode: u8,
    dist_i64: i64,
    prev_block_base_i64: i64,
    normal_match_bits: f32,
    normal_trial_bits_times_lambda: f32,
    cur_ms_err: f32,
    thresh_ms_err: f32,
    smooth_block_error_scale: f32,
    trial_error_scale: f32,
    hash_table: &mut [u64],
    hash_mask: usize,
    hash_epoch: u64,
    best_t: &mut f32,
    best_bits: &mut u128,
    best_ms_err: &mut f32,
    best_match_len: &mut usize,
    best_match_dst_block_ofs: &mut usize,
    best_match_bits: &mut f32,
    prev_cont_window_ofs: &mut i64,
    prev_rep0_dist: &mut i64,
    stats: &mut Option<&mut Bc7RdoStats>,
) {
    debug_assert!((3..=16).contains(&LEN));
    let segment_mask = BC7_SEGMENT_MASKS[LEN];

    for ofs in 0..=(16 - LEN) {
        if normal_trial_bits_times_lambda >= *best_t {
            let skipped_offsets = (17 - LEN - ofs) as u64;
            stat_add!(COLLECT_STATS, stats, candidate_checks, skipped_offsets);
            stat_add!(COLLECT_STATS, stats, rate_skips, skipped_offsets);
            break;
        }

        stat_add!(COLLECT_STATS, stats, candidate_checks, 1);
        let shift = ofs * 8;
        let prev_segment = (prev_bits >> shift) & segment_mask;
        let hs = hash_hsieh_bc7_segment_fixed::<LEN>(prev_segment, ofs as u32);
        if rdo_hash_seen(hash_table, hash_mask, hash_epoch, hs) {
            stat_add!(COLLECT_STATS, stats, hash_skips, 1);
            continue;
        }

        if prev_segment == ((orig_bits >> shift) & segment_mask) {
            stat_add!(COLLECT_STATS, stats, original_block_skips, 1);
            let trial_ms_err = cur_ms_err;
            if trial_ms_err < thresh_ms_err {
                let t = trial_ms_err * smooth_block_error_scale + normal_trial_bits_times_lambda;
                if t < *best_t {
                    *best_t = t;
                    *best_bits = orig_bits;
                    *best_ms_err = trial_ms_err;
                    *best_match_len = LEN;
                    *best_match_dst_block_ofs = ofs;
                    *best_match_bits = normal_match_bits;
                    *prev_cont_window_ofs = prev_block_base_i64 + ofs as i64 + LEN as i64;
                    *prev_rep0_dist = dist_i64;
                    stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
                }
            }
            continue;
        }

        let trial_bits =
            bc7_copy_segment_bits_from_segment(
                orig_bits,
                prev_segment,
                shift,
                segment_mask,
            );
        if ofs == 0 && !bc7_block_bits_has_mode(trial_bits, bc7_mode) {
            stat_add!(COLLECT_STATS, stats, unsupported_mode_trials, 1);
            continue;
        }

        stat_add!(COLLECT_STATS, stats, decode_trials, 1);
        let max_trial_err = max_trial_error(*best_t, normal_trial_bits_times_lambda, trial_error_scale);
        let Some(trial_err) = decode_bc7_error_bounded_for_stats!(
            stats,
            trial_bits,
            p_pixels,
            bc7_mode,
            true,
            max_trial_err
        ) else {
            stat_add!(COLLECT_STATS, stats, bounded_error_exits, 1);
            continue;
        };
        let trial_ms_err = trial_err as f32 / 64.0;
        if trial_ms_err < thresh_ms_err {
            let t = trial_ms_err * smooth_block_error_scale + normal_trial_bits_times_lambda;
            if t < *best_t {
                *best_t = t;
                *best_bits = trial_bits;
                *best_ms_err = trial_ms_err;
                *best_match_len = LEN;
                *best_match_dst_block_ofs = ofs;
                *best_match_bits = normal_match_bits;
                *prev_cont_window_ofs = prev_block_base_i64 + ofs as i64 + LEN as i64;
                *prev_rep0_dist = dist_i64;
                stat_add!(COLLECT_STATS, stats, accepted_matches, 1);
            }
        }
    }
}

#[inline(always)]
fn bc7_block_bits(block: [u8; 16]) -> u128 {
    u128::from_le_bytes(block)
}

#[inline(always)]
fn rgba_block_at(rgba_blocks: &[[u8; 4]], block_index: usize) -> &RgbaBlock {
    let start = block_index * 16;
    debug_assert!(start + 16 <= rgba_blocks.len());
    // SAFETY: `RgbaBlock` is exactly 16 contiguous RGBA pixels with the same
    // alignment as `[u8; 4]`; callers validate that the source is block-raster.
    unsafe { &*(rgba_blocks.as_ptr().add(start) as *const RgbaBlock) }
}

#[inline(always)]
fn bc7_copy_segment_bits_from_segment(
    orig_bits: u128,
    src_segment: u128,
    dst_shift: usize,
    src_mask: u128,
) -> u128 {
    let dst_mask = src_mask << dst_shift;
    let src_segment = src_segment << dst_shift;
    orig_bits & !dst_mask | src_segment
}

#[inline(always)]
fn max_trial_error_scale(smooth_block_error_scale: f32) -> f32 {
    if smooth_block_error_scale <= 0.0 {
        0.0
    } else {
        64.0 / smooth_block_error_scale
    }
}

#[inline(always)]
fn max_trial_error(best_t: f32, trial_bits_times_lambda: f32, max_trial_error_scale: f32) -> u64 {
    if max_trial_error_scale <= 0.0 {
        return u64::MAX;
    }
    ((best_t - trial_bits_times_lambda) * max_trial_error_scale).max(0.0).ceil() as u64
}

#[inline(always)]
fn smooth_block_error_scale_from_pixels(
    pixels: &RgbaBlock,
    max_smooth_block_std_dev: f32,
    smooth_block_max_mse_scale: f32,
) -> f32 {
    let max_std_dev = compute_block_max_std_dev(pixels);
    let mut yl = (max_std_dev / max_smooth_block_std_dev).clamp(0.0, 1.0);
    yl = yl * yl;
    lerp(smooth_block_max_mse_scale, 1.0, yl)
}

#[inline(always)]
fn decode_bc7_error_bounded<const COLLECT_STATS: bool>(
    block_bits: u128,
    source: &RgbaBlock,
    mode_hint: u8,
    trust_mode_hint: bool,
    max_error: u64,
    mut stats: Option<&mut Bc7RdoStats>,
) -> Option<u64> {
    let mode = if trust_mode_hint {
        debug_assert!(bc7_block_bits_has_mode(block_bits, mode_hint));
        mode_hint
    } else {
        get_bc7_mode_bits(block_bits)
    };
    if COLLECT_STATS && mode < 8 {
        stat_array_add!(COLLECT_STATS, stats, decode_mode_trials, mode as usize, 1);
    }
    match mode {
        0 => {
            stat_add!(COLLECT_STATS, stats, fused_mode0_trials, 1);
            decode_bc7_mode0_error_bounded(block_bits, source, max_error)
        }
        1 => {
            stat_add!(COLLECT_STATS, stats, fused_mode1_trials, 1);
            decode_bc7_mode1_error_bounded(block_bits, source, max_error)
        }
        2 => {
            stat_add!(COLLECT_STATS, stats, fused_mode2_trials, 1);
            decode_bc7_mode2_error_bounded(block_bits, source, max_error)
        }
        3 => {
            stat_add!(COLLECT_STATS, stats, fused_mode3_trials, 1);
            decode_bc7_mode3_error_bounded(block_bits, source, max_error)
        }
        4 => {
            stat_add!(COLLECT_STATS, stats, fused_mode4_trials, 1);
            decode_bc7_mode4_error_bounded(block_bits, source, max_error)
        }
        5 => {
            stat_add!(COLLECT_STATS, stats, fused_mode5_trials, 1);
            decode_bc7_mode5_error_bounded(block_bits, source, max_error)
        }
        6 => {
            stat_add!(COLLECT_STATS, stats, fused_mode6_trials, 1);
            decode_bc7_mode6_error_bounded(block_bits, source, max_error)
        }
        7 => {
            stat_add!(COLLECT_STATS, stats, fused_mode7_trials, 1);
            decode_bc7_mode7_error_bounded(block_bits, source, max_error)
        }
        _ => {
            stat_add!(COLLECT_STATS, stats, unsupported_mode_trials, 1);
            None
        }
    }
}

fn decode_bc7_mode0_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let low = block_bits as u64;
    let high = (block_bits >> 64) as u64;
    let partition = ((low >> 1) & 0x0F) as usize;
    let partitions = &BC7_PARTITION3[partition * 16..partition * 16 + 16];
    let anchor1 = BC7_ANCHOR_THIRD_SUBSET1[partition] as usize;
    let anchor2 = BC7_ANCHOR_THIRD_SUBSET2[partition] as usize;

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
    decode_bc7_partitioned_rgb3_weights3_error_bounded(
        source,
        max_error,
        partitions,
        anchor1,
        anchor2,
        &endpoints,
        high >> 19,
    )
}

fn decode_bc7_mode2_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let low = block_bits as u64;
    let tail = (block_bits >> 64) as u64;
    let partition = ((low >> 3) & 0x3F) as usize;
    let partitions = &BC7_PARTITION3[partition * 16..partition * 16 + 16];
    let anchor1 = BC7_ANCHOR_THIRD_SUBSET1[partition] as usize;
    let anchor2 = BC7_ANCHOR_THIRD_SUBSET2[partition] as usize;

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
    decode_bc7_partitioned_rgb3_weights2_error_bounded(
        source,
        max_error,
        partitions,
        anchor1,
        anchor2,
        &endpoints,
        tail >> 35,
    )
}

fn decode_bc7_mode3_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let low = block_bits as u64;
    let high = (block_bits >> 64) as u64;
    let partition = ((low >> 4) & 0x3F) as usize;
    let partitions = &BC7_PARTITION2[partition * 16..partition * 16 + 16];
    let anchor = BC7_ANCHOR_SECOND_SUBSET[partition] as usize;
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
    decode_bc7_partitioned_rgb2_weights2_error_bounded(
        source,
        max_error,
        partitions,
        anchor,
        &endpoints,
        high >> 34,
    )
}

fn decode_bc7_mode4_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let rotation = ((block_bits >> 5) & 0x03) as usize;
    let index_flag = ((block_bits >> 7) & 1) != 0;
    let x = ((block_bits >> 8) & ((1u128 << 42) - 1)) as u64;
    let y_low = ((block_bits >> 48) & 0xFFFF_FFFF) as u64;
    let z = (block_bits >> 80) as u64;

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

    match (rotation, index_flag) {
        (0, false) => decode_bc7_mode4_pixels_error_bounded::<0, false>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (1, false) => decode_bc7_mode4_pixels_error_bounded::<1, false>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (2, false) => decode_bc7_mode4_pixels_error_bounded::<2, false>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (3, false) => decode_bc7_mode4_pixels_error_bounded::<3, false>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (0, true) => decode_bc7_mode4_pixels_error_bounded::<0, true>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (1, true) => decode_bc7_mode4_pixels_error_bounded::<1, true>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (2, true) => decode_bc7_mode4_pixels_error_bounded::<2, true>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        (3, true) => decode_bc7_mode4_pixels_error_bounded::<3, true>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, p2_stream, p3_stream,
        ),
        _ => unreachable!("BC7 mode 4 rotation is two bits"),
    }
}

fn decode_bc7_mode5_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let low = block_bits as u64;
    let high = (block_bits >> 64) as u64;
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

    match rotation {
        0 => decode_bc7_mode5_pixels_error_bounded::<0>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, rgb_stream, alpha_stream,
        ),
        1 => decode_bc7_mode5_pixels_error_bounded::<1>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, rgb_stream, alpha_stream,
        ),
        2 => decode_bc7_mode5_pixels_error_bounded::<2>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, rgb_stream, alpha_stream,
        ),
        3 => decode_bc7_mode5_pixels_error_bounded::<3>(
            source, max_error, lr, hr, lg, hg, lb, hb, la, ha, rgb_stream, alpha_stream,
        ),
        _ => unreachable!("BC7 mode 5 rotation is two bits"),
    }
}

fn decode_bc7_mode4_pixels_error_bounded<const ROTATION: usize, const INDEX_FLAG: bool>(
    source: &RgbaBlock,
    max_error: u64,
    lr: i32,
    hr: i32,
    lg: i32,
    hg: i32,
    lb: i32,
    hb: i32,
    la: i32,
    ha: i32,
    p2_stream: u64,
    p3_stream: u64,
) -> Option<u64> {
    let mut err = 0u64;
    let p2_index = ((p2_stream >> 2) & 0x01) as usize;
    let p3_index = (p3_stream & 0x03) as usize;
    let (rgb_weight, scalar_weight) = if INDEX_FLAG {
        (BC7_WEIGHTS3[p3_index] as i32, BC7_WEIGHTS2[p2_index] as i32)
    } else {
        (BC7_WEIGHTS2[p2_index] as i32, BC7_WEIGHTS3[p3_index] as i32)
    };
    err += mode45_pixel_sse::<ROTATION>(
        &source[0],
        interpolate_bc7(lr, hr, rgb_weight),
        interpolate_bc7(lg, hg, rgb_weight),
        interpolate_bc7(lb, hb, rgb_weight),
        interpolate_bc7(la, ha, scalar_weight),
    );
    if err >= max_error {
        return None;
    }

    let mut p2_bit_ofs = 3usize;
    let mut p3_bit_ofs = 2usize;
    for i in 1..16 {
        let p2_index = ((p2_stream >> p2_bit_ofs) & 0x03) as usize;
        let p3_index = ((p3_stream >> p3_bit_ofs) & 0x07) as usize;
        p2_bit_ofs += 2;
        p3_bit_ofs += 3;
        let (rgb_weight, scalar_weight) = if INDEX_FLAG {
            (BC7_WEIGHTS3[p3_index] as i32, BC7_WEIGHTS2[p2_index] as i32)
        } else {
            (BC7_WEIGHTS2[p2_index] as i32, BC7_WEIGHTS3[p3_index] as i32)
        };
        err += mode45_pixel_sse::<ROTATION>(
            &source[i],
            interpolate_bc7(lr, hr, rgb_weight),
            interpolate_bc7(lg, hg, rgb_weight),
            interpolate_bc7(lb, hb, rgb_weight),
            interpolate_bc7(la, ha, scalar_weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode5_pixels_error_bounded<const ROTATION: usize>(
    source: &RgbaBlock,
    max_error: u64,
    lr: i32,
    hr: i32,
    lg: i32,
    hg: i32,
    lb: i32,
    hb: i32,
    la: i32,
    ha: i32,
    rgb_stream: u64,
    alpha_stream: u64,
) -> Option<u64> {
    let mut err = 0u64;
    let rgb_index = (rgb_stream & 0x01) as usize;
    let alpha_index = (alpha_stream & 0x01) as usize;
    let rgb_weight = BC7_WEIGHTS2[rgb_index] as i32;
    let alpha_weight = BC7_WEIGHTS2[alpha_index] as i32;
    err += mode45_pixel_sse::<ROTATION>(
        &source[0],
        interpolate_bc7(lr, hr, rgb_weight),
        interpolate_bc7(lg, hg, rgb_weight),
        interpolate_bc7(lb, hb, rgb_weight),
        interpolate_bc7(la, ha, alpha_weight),
    );
    if err >= max_error {
        return None;
    }

    let mut rgb_bit_ofs = 1usize;
    let mut alpha_bit_ofs = 1usize;
    for i in 1..16 {
        let rgb_index = ((rgb_stream >> rgb_bit_ofs) & 0x03) as usize;
        let alpha_index = ((alpha_stream >> alpha_bit_ofs) & 0x03) as usize;
        rgb_bit_ofs += 2;
        alpha_bit_ofs += 2;
        let rgb_weight = BC7_WEIGHTS2[rgb_index] as i32;
        let alpha_weight = BC7_WEIGHTS2[alpha_index] as i32;
        err += mode45_pixel_sse::<ROTATION>(
            &source[i],
            interpolate_bc7(lr, hr, rgb_weight),
            interpolate_bc7(lg, hg, rgb_weight),
            interpolate_bc7(lb, hb, rgb_weight),
            interpolate_bc7(la, ha, alpha_weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

#[inline(always)]
fn mode45_pixel_sse<const ROTATION: usize>(
    source: &[u8; 4],
    r: i32,
    g: i32,
    b: i32,
    scalar: i32,
) -> u64 {
    let (dr, dg, db, da) = match ROTATION {
        0 => (
            source[0] as i32 - r,
            source[1] as i32 - g,
            source[2] as i32 - b,
            source[3] as i32 - scalar,
        ),
        1 => (
            source[0] as i32 - scalar,
            source[1] as i32 - g,
            source[2] as i32 - b,
            source[3] as i32 - r,
        ),
        2 => (
            source[0] as i32 - r,
            source[1] as i32 - scalar,
            source[2] as i32 - b,
            source[3] as i32 - g,
        ),
        3 => (
            source[0] as i32 - r,
            source[1] as i32 - g,
            source[2] as i32 - scalar,
            source[3] as i32 - b,
        ),
        _ => unreachable!("BC7 mode 4/5 rotation is two bits"),
    };
    (dr * dr + dg * dg + db * db + da * da) as u64
}

fn decode_bc7_mode1_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let part_id = ((block_bits >> 2) & 0x3F) as usize;
    let x = (block_bits >> 8) as u64;
    let y = (block_bits >> 80) as u64;
    let block9 = ((block_bits >> 72) & 0xFF) as u64;

    let pbits = [(y & 1) as u32, ((y >> 1) & 1) as u32];
    let pixel_descs = &MODE1_PIXEL_DESCS[part_id];
    let mut err = 0u64;
    let desc = pixel_descs[0];
    let subset = (desc & 0x01) as usize;
    let weight_index = ((y >> ((desc >> 1) & 0x7F)) & ((desc >> 8) as u64)) as usize;
    let weight = BC7_WEIGHTS3[weight_index] as i32;
    let (first_lr, first_hr, first_lg, first_hg, first_lb, first_hb) =
        if subset == 0 {
            (
                expand_mode1_endpoint(x & 0x3F, pbits[0]),
                expand_mode1_endpoint((x >> 6) & 0x3F, pbits[0]),
                expand_mode1_endpoint((x >> 24) & 0x3F, pbits[0]),
                expand_mode1_endpoint((x >> 30) & 0x3F, pbits[0]),
                expand_mode1_endpoint((x >> 48) & 0x3F, pbits[0]),
                expand_mode1_endpoint((x >> 54) & 0x3F, pbits[0]),
            )
        } else {
            let lb1 = ((x >> 60) & 0xF) | ((block9 & 0x03) << 4);
            (
                expand_mode1_endpoint((x >> 12) & 0x3F, pbits[1]),
                expand_mode1_endpoint((x >> 18) & 0x3F, pbits[1]),
                expand_mode1_endpoint((x >> 36) & 0x3F, pbits[1]),
                expand_mode1_endpoint((x >> 42) & 0x3F, pbits[1]),
                expand_mode1_endpoint(lb1, pbits[1]),
                expand_mode1_endpoint((block9 >> 2) & 0x3F, pbits[1]),
            )
        };
    err += rgb_alpha_255_pixel_sse(
        &source[0],
        interpolate_bc7(first_lr, first_hr, weight),
        interpolate_bc7(first_lg, first_hg, weight),
        interpolate_bc7(first_lb, first_hb, weight),
    );
    if err >= max_error {
        return None;
    }

    let lb1 = ((x >> 60) & 0xF) | ((block9 & 0x03) << 4);
    let (lr, hr, lg, hg, lb, hb) = if subset == 0 {
        (
            [first_lr, expand_mode1_endpoint((x >> 12) & 0x3F, pbits[1])],
            [first_hr, expand_mode1_endpoint((x >> 18) & 0x3F, pbits[1])],
            [first_lg, expand_mode1_endpoint((x >> 36) & 0x3F, pbits[1])],
            [first_hg, expand_mode1_endpoint((x >> 42) & 0x3F, pbits[1])],
            [first_lb, expand_mode1_endpoint(lb1, pbits[1])],
            [first_hb, expand_mode1_endpoint((block9 >> 2) & 0x3F, pbits[1])],
        )
    } else {
        (
            [expand_mode1_endpoint(x & 0x3F, pbits[0]), first_lr],
            [expand_mode1_endpoint((x >> 6) & 0x3F, pbits[0]), first_hr],
            [expand_mode1_endpoint((x >> 24) & 0x3F, pbits[0]), first_lg],
            [expand_mode1_endpoint((x >> 30) & 0x3F, pbits[0]), first_hg],
            [expand_mode1_endpoint((x >> 48) & 0x3F, pbits[0]), first_lb],
            [expand_mode1_endpoint((x >> 54) & 0x3F, pbits[0]), first_hb],
        )
    };

    for i in 1..16 {
        let desc = pixel_descs[i];
        let subset = (desc & 0x01) as usize;
        let weight_index = ((y >> ((desc >> 1) & 0x7F)) & ((desc >> 8) as u64)) as usize;
        let weight = BC7_WEIGHTS3[weight_index] as i32;
        err += rgb_alpha_255_pixel_sse(
            &source[i],
            interpolate_bc7(lr[subset], hr[subset], weight),
            interpolate_bc7(lg[subset], hg[subset], weight),
            interpolate_bc7(lb[subset], hb[subset], weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode6_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let lo = block_bits as u64;
    let hi = (block_bits >> 64) as u64;

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
    let weight_index = ((hi >> 1) & 0x07) as usize;
    let weight = BC7_WEIGHTS4[weight_index] as i32;
    let r = interpolate_bc7(lr, hr, weight);
    let g = interpolate_bc7(lg, hg, weight);
    let b = interpolate_bc7(lb, hb, weight);
    let a = interpolate_bc7(la, ha, weight);
    let dr = source[0][0] as i32 - r;
    let dg = source[0][1] as i32 - g;
    let db = source[0][2] as i32 - b;
    let da = source[0][3] as i32 - a;
    err += (dr * dr + dg * dg + db * db + da * da) as u64;
    if err >= max_error {
        return None;
    }

    let mut weight_bit_ofs = 4usize;
    for i in 1..16 {
        let weight_index = ((hi >> weight_bit_ofs) & 0x0F) as usize;
        weight_bit_ofs += 4;
        let weight = BC7_WEIGHTS4[weight_index] as i32;
        let r = interpolate_bc7(lr, hr, weight);
        let g = interpolate_bc7(lg, hg, weight);
        let b = interpolate_bc7(lb, hb, weight);
        let a = interpolate_bc7(la, ha, weight);
        let dr = source[i][0] as i32 - r;
        let dg = source[i][1] as i32 - g;
        let db = source[i][2] as i32 - b;
        let da = source[i][3] as i32 - a;
        err += (dr * dr + dg * dg + db * db + da * da) as u64;
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_mode7_error_bounded(
    block_bits: u128,
    source: &RgbaBlock,
    max_error: u64,
) -> Option<u64> {
    let lo = block_bits as u64;
    let hi = (block_bits >> 64) as u64;

    let part_id = ((lo >> 8) & 0x3F) as usize;
    let pbits = [
        ((hi >> 30) & 1) as u32,
        ((hi >> 31) & 1) as u32,
        ((hi >> 32) & 1) as u32,
        ((hi >> 33) & 1) as u32,
    ];

    let pixel_descs = &MODE7_PIXEL_DESCS[part_id];
    let mut err = 0u64;
    let desc = pixel_descs[0];
    let subset = (desc & 0x01) as usize;
    let weight_index = ((hi >> ((desc >> 1) & 0x7F)) & ((desc >> 8) as u64)) as usize;
    let weight = BC7_WEIGHTS2[weight_index] as i32;
    let (first_lr, first_hr, first_lg, first_hg, first_lb, first_hb, first_la, first_ha) =
        if subset == 0 {
            (
                expand_mode7_endpoint((lo >> 14) & 0x1F, pbits[0]),
                expand_mode7_endpoint((lo >> 19) & 0x1F, pbits[1]),
                expand_mode7_endpoint((lo >> 34) & 0x1F, pbits[0]),
                expand_mode7_endpoint((lo >> 39) & 0x1F, pbits[1]),
                expand_mode7_endpoint((lo >> 54) & 0x1F, pbits[0]),
                expand_mode7_endpoint((lo >> 59) & 0x1F, pbits[1]),
                expand_mode7_endpoint((hi >> 10) & 0x1F, pbits[0]),
                expand_mode7_endpoint((hi >> 15) & 0x1F, pbits[1]),
            )
        } else {
            (
                expand_mode7_endpoint((lo >> 24) & 0x1F, pbits[2]),
                expand_mode7_endpoint((lo >> 29) & 0x1F, pbits[3]),
                expand_mode7_endpoint((lo >> 44) & 0x1F, pbits[2]),
                expand_mode7_endpoint((lo >> 49) & 0x1F, pbits[3]),
                expand_mode7_endpoint(hi & 0x1F, pbits[2]),
                expand_mode7_endpoint((hi >> 5) & 0x1F, pbits[3]),
                expand_mode7_endpoint((hi >> 20) & 0x1F, pbits[2]),
                expand_mode7_endpoint((hi >> 25) & 0x1F, pbits[3]),
            )
        };
    err += rgba_pixel_sse(
        &source[0],
        interpolate_bc7(first_lr, first_hr, weight),
        interpolate_bc7(first_lg, first_hg, weight),
        interpolate_bc7(first_lb, first_hb, weight),
        interpolate_bc7(first_la, first_ha, weight),
    );
    if err >= max_error {
        return None;
    }

    let (lr, hr, lg, hg, lb, hb, la, ha) = if subset == 0 {
        (
            [first_lr, expand_mode7_endpoint((lo >> 24) & 0x1F, pbits[2])],
            [first_hr, expand_mode7_endpoint((lo >> 29) & 0x1F, pbits[3])],
            [first_lg, expand_mode7_endpoint((lo >> 44) & 0x1F, pbits[2])],
            [first_hg, expand_mode7_endpoint((lo >> 49) & 0x1F, pbits[3])],
            [first_lb, expand_mode7_endpoint(hi & 0x1F, pbits[2])],
            [first_hb, expand_mode7_endpoint((hi >> 5) & 0x1F, pbits[3])],
            [first_la, expand_mode7_endpoint((hi >> 20) & 0x1F, pbits[2])],
            [first_ha, expand_mode7_endpoint((hi >> 25) & 0x1F, pbits[3])],
        )
    } else {
        (
            [expand_mode7_endpoint((lo >> 14) & 0x1F, pbits[0]), first_lr],
            [expand_mode7_endpoint((lo >> 19) & 0x1F, pbits[1]), first_hr],
            [expand_mode7_endpoint((lo >> 34) & 0x1F, pbits[0]), first_lg],
            [expand_mode7_endpoint((lo >> 39) & 0x1F, pbits[1]), first_hg],
            [expand_mode7_endpoint((lo >> 54) & 0x1F, pbits[0]), first_lb],
            [expand_mode7_endpoint((lo >> 59) & 0x1F, pbits[1]), first_hb],
            [expand_mode7_endpoint((hi >> 10) & 0x1F, pbits[0]), first_la],
            [expand_mode7_endpoint((hi >> 15) & 0x1F, pbits[1]), first_ha],
        )
    };

    for i in 1..16 {
        let desc = pixel_descs[i];
        let subset = (desc & 0x01) as usize;
        let weight_index = ((hi >> ((desc >> 1) & 0x7F)) & ((desc >> 8) as u64)) as usize;
        let weight = BC7_WEIGHTS2[weight_index] as i32;
        err += rgba_pixel_sse(
            &source[i],
            interpolate_bc7(lr[subset], hr[subset], weight),
            interpolate_bc7(lg[subset], hg[subset], weight),
            interpolate_bc7(lb[subset], hb[subset], weight),
            interpolate_bc7(la[subset], ha[subset], weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_partitioned_rgb3_weights3_error_bounded(
    source: &RgbaBlock,
    max_error: u64,
    partitions: &[u8],
    anchor1: usize,
    anchor2: usize,
    endpoints: &[[[i32; 3]; 2]; 3],
    index_stream: u64,
) -> Option<u64> {
    let mut bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let subset = partitions[i] as usize;
        let bits = if i == 0 || i == anchor1 || i == anchor2 { 2 } else { 3 };
        let mask = if bits == 2 { 0x03 } else { 0x07 };
        let index = ((index_stream >> bit_ofs) & mask) as usize;
        bit_ofs += bits;

        let weight = BC7_WEIGHTS3[index] as i32;
        err += rgb_alpha_255_pixel_sse(
            &source[i],
            interpolate_bc7(endpoints[subset][0][0], endpoints[subset][1][0], weight),
            interpolate_bc7(endpoints[subset][0][1], endpoints[subset][1][1], weight),
            interpolate_bc7(endpoints[subset][0][2], endpoints[subset][1][2], weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_partitioned_rgb3_weights2_error_bounded(
    source: &RgbaBlock,
    max_error: u64,
    partitions: &[u8],
    anchor1: usize,
    anchor2: usize,
    endpoints: &[[[i32; 3]; 2]; 3],
    index_stream: u64,
) -> Option<u64> {
    let mut bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let subset = partitions[i] as usize;
        let bits = if i == 0 || i == anchor1 || i == anchor2 { 1 } else { 2 };
        let mask = if bits == 1 { 0x01 } else { 0x03 };
        let index = ((index_stream >> bit_ofs) & mask) as usize;
        bit_ofs += bits;

        let weight = BC7_WEIGHTS2[index] as i32;
        err += rgb_alpha_255_pixel_sse(
            &source[i],
            interpolate_bc7(endpoints[subset][0][0], endpoints[subset][1][0], weight),
            interpolate_bc7(endpoints[subset][0][1], endpoints[subset][1][1], weight),
            interpolate_bc7(endpoints[subset][0][2], endpoints[subset][1][2], weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

fn decode_bc7_partitioned_rgb2_weights2_error_bounded(
    source: &RgbaBlock,
    max_error: u64,
    partitions: &[u8],
    anchor: usize,
    endpoints: &[[[i32; 3]; 2]; 2],
    index_stream: u64,
) -> Option<u64> {
    let mut bit_ofs = 0usize;
    let mut err = 0u64;

    for i in 0..16 {
        let subset = partitions[i] as usize;
        let bits = if i == 0 || i == anchor { 1 } else { 2 };
        let mask = if bits == 1 { 0x01 } else { 0x03 };
        let index = ((index_stream >> bit_ofs) & mask) as usize;
        bit_ofs += bits;

        let weight = BC7_WEIGHTS2[index] as i32;
        err += rgb_alpha_255_pixel_sse(
            &source[i],
            interpolate_bc7(endpoints[subset][0][0], endpoints[subset][1][0], weight),
            interpolate_bc7(endpoints[subset][0][1], endpoints[subset][1][1], weight),
            interpolate_bc7(endpoints[subset][0][2], endpoints[subset][1][2], weight),
        );
        if err >= max_error {
            return None;
        }
    }

    Some(err)
}

#[inline(always)]
fn rgb_alpha_255_pixel_sse(source: &[u8; 4], r: i32, g: i32, b: i32) -> u64 {
    let dr = source[0] as i32 - r;
    let dg = source[1] as i32 - g;
    let db = source[2] as i32 - b;
    let da = source[3] as i32 - 255;
    (dr * dr + dg * dg + db * db + da * da) as u64
}

#[inline(always)]
fn rgba_pixel_sse(source: &[u8; 4], r: i32, g: i32, b: i32, a: i32) -> u64 {
    let dr = source[0] as i32 - r;
    let dg = source[1] as i32 - g;
    let db = source[2] as i32 - b;
    let da = source[3] as i32 - a;
    (dr * dr + dg * dg + db * db + da * da) as u64
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
fn interpolate_bc7(lo: i32, hi: i32, weight: i32) -> i32 {
    lo + (((hi - lo) * weight + 32) >> 6)
}

const BC7_WEIGHTS4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const BC7_WEIGHTS3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const BC7_WEIGHTS2: [u8; 4] = [0, 21, 43, 64];

#[inline(always)]
fn hash_hsieh_bc7_segment(segment: u128, len: usize, salt: u32) -> u32 {
    debug_assert!(len > 0);
    match len {
        3 => hash_hsieh_bc7_segment_fixed::<3>(segment, salt),
        4 => hash_hsieh_bc7_segment_fixed::<4>(segment, salt),
        5 => hash_hsieh_bc7_segment_fixed::<5>(segment, salt),
        6 => hash_hsieh_bc7_segment_fixed::<6>(segment, salt),
        7 => hash_hsieh_bc7_segment_fixed::<7>(segment, salt),
        8 => hash_hsieh_bc7_segment_fixed::<8>(segment, salt),
        9 => hash_hsieh_bc7_segment_fixed::<9>(segment, salt),
        10 => hash_hsieh_bc7_segment_fixed::<10>(segment, salt),
        11 => hash_hsieh_bc7_segment_fixed::<11>(segment, salt),
        12 => hash_hsieh_bc7_segment_fixed::<12>(segment, salt),
        13 => hash_hsieh_bc7_segment_fixed::<13>(segment, salt),
        14 => hash_hsieh_bc7_segment_fixed::<14>(segment, salt),
        15 => hash_hsieh_bc7_segment_fixed::<15>(segment, salt),
        16 => hash_hsieh_bc7_segment_fixed::<16>(segment, salt),
        _ => hash_hsieh_bc7_segment_variable(segment, len, salt),
    }
}

#[inline(always)]
fn hash_hsieh_bc7_segment_fixed<const LEN: usize>(segment: u128, salt: u32) -> u32 {
    let mut h = (LEN as u32).wrapping_add(salt << 16);
    let mut i = 0usize;
    let mut rem = LEN;

    while rem >= 4 {
        let w0 = ((segment >> (i * 8)) & 0xFFFF) as u32;
        let w1 = ((segment >> ((i + 2) * 8)) & 0xFFFF) as u32;

        h = h.wrapping_add(w0);
        let t = (w1 << 11) ^ h;
        h = (h << 16) ^ t;

        i += 4;
        rem -= 4;
        h = h.wrapping_add(h >> 11);
    }

    match rem {
        3 => {
            h = h.wrapping_add(((segment >> (i * 8)) & 0xFFFF) as u32);
            h ^= h << 16;
            h ^= ((segment >> ((i + 2) * 8)) as u8 as i8 as u32) << 18;
            h = h.wrapping_add(h >> 11);
        }
        2 => {
            h = h.wrapping_add(((segment >> (i * 8)) & 0xFFFF) as u32);
            h ^= h << 11;
            h = h.wrapping_add(h >> 17);
        }
        1 => {
            h = h.wrapping_add((segment >> (i * 8)) as u8 as i8 as u32);
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

#[inline(always)]
fn hash_hsieh_bc7_segment_variable(segment: u128, len: usize, salt: u32) -> u32 {
    let mut h = (len as u32).wrapping_add(salt << 16);
    let mut i = 0usize;
    let mut rem = len;

    while rem >= 4 {
        let w0 = ((segment >> (i * 8)) & 0xFFFF) as u32;
        let w1 = ((segment >> ((i + 2) * 8)) & 0xFFFF) as u32;
        
        h = h.wrapping_add(w0);
        let t = (w1 << 11) ^ h;
        h = (h << 16) ^ t;
        
        i += 4;
        rem -= 4;
        h = h.wrapping_add(h >> 11);
    }

    match rem {
        3 => {
            h = h.wrapping_add(((segment >> (i * 8)) & 0xFFFF) as u32);
            h ^= h << 16;
            h ^= ((segment >> ((i + 2) * 8)) as u8 as i8 as u32) << 18;
            h = h.wrapping_add(h >> 11);
        }
        2 => {
            h = h.wrapping_add(((segment >> (i * 8)) & 0xFFFF) as u32);
            h ^= h << 11;
            h = h.wrapping_add(h >> 17);
        }
        1 => {
            h = h.wrapping_add((segment >> (i * 8)) as u8 as i8 as u32);
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

#[inline]
fn rdo_hash_seen(hash_table: &mut [u64], hash_mask: usize, hash_epoch: u64, hs: u32) -> bool {
    let entry = hash_epoch | ((hs >> 8) as u64);
    let slot = &mut hash_table[hs as usize & hash_mask];
    let seen = *slot == entry;
    *slot = entry;
    seen
}

#[inline(always)]
fn get_bc7_mode_bits(block_bits: u128) -> u8 {
    get_bc7_mode_from_first_byte(block_bits as u8)
}

#[inline(always)]
fn get_bc7_mode_from_first_byte(first_byte: u8) -> u8 {
    if first_byte == 0 {
        8
    } else {
        first_byte.trailing_zeros() as u8
    }
}

#[inline(always)]
fn bc7_block_bits_has_mode(block_bits: u128, mode: u8) -> bool {
    bc7_first_byte_has_mode(block_bits as u8, mode)
}

#[inline(always)]
fn bc7_first_byte_has_mode(first_byte: u8, mode: u8) -> bool {
    debug_assert!(mode < 8);
    let first_byte = first_byte as u16;
    let mode_bit = 1u16 << mode;
    first_byte & ((mode_bit << 1) - 1) == mode_bit
}

fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

fn compute_block_max_std_dev(pixels: &RgbaBlock) -> f32 {
    let mut max_std_dev = 0.0f32;
    for c in 0..4 {
        let mut sum = 0u32;
        let mut sum2 = 0u32;
        for i in 0..16 {
            let val = pixels[i][c] as u32;
            sum += val;
            sum2 += val * val;
        }
        let variance = (16 * sum2).saturating_sub(sum * sum);
        let std_dev = (variance as f64).sqrt() as f32 * (1.0 / 16.0);
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

fn compute_match_len_bits() -> [f32; 17] {
    let mut bits = [0.0f32; 17];
    for len in 0..=16 {
        bits[len] = compute_match_len_cost(len as u32) as f32;
    }
    bits
}

fn compute_literal_bits_by_match_len() -> [f32; 17] {
    let mut bits = [0.0f32; 17];
    for len in 0..=16 {
        bits[len] = (16 - len) as f32 * LITERAL_BITS;
    }
    bits
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
    let use_parallel = total_blocks >= PARALLEL_RDO_BLOCK_THRESHOLD;

    let mut is_ultrasmooth = vec![false; total_blocks];
    if use_parallel {
        is_ultrasmooth
            .par_iter_mut()
            .enumerate()
            .for_each(|(block_index, is_ultrasmooth)| {
                *is_ultrasmooth = is_ultrasmooth_seed_block(rgba_block_at(rgba_blocks, block_index));
            });
    } else {
        for (block_index, is_ultrasmooth) in is_ultrasmooth.iter_mut().enumerate() {
            *is_ultrasmooth = is_ultrasmooth_seed_block(rgba_block_at(rgba_blocks, block_index));
        }
    }

    let mut current_mask = is_ultrasmooth.clone();
    let mut next_mask = vec![false; total_blocks];

    // Pass 1: Erosion of ultrasmooth (dilation of non-ultrasmooth)
    if use_parallel {
        next_mask
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, next)| {
                *next = erode_ultrasmooth_mask_at(idx, &current_mask, blocks_x, blocks_y);
            });
    } else {
        for (idx, next) in next_mask.iter_mut().enumerate() {
            *next = erode_ultrasmooth_mask_at(idx, &current_mask, blocks_x, blocks_y);
        }
    }
    std::mem::swap(&mut current_mask, &mut next_mask);

    // 32 passes of "median-like" erosion
    for _ in 0..32 {
        if use_parallel {
            next_mask
                .par_iter_mut()
                .enumerate()
                .for_each(|(idx, next)| {
                    *next = median_erode_ultrasmooth_mask_at(idx, &current_mask, blocks_x, blocks_y);
                });
        } else {
            for (idx, next) in next_mask.iter_mut().enumerate() {
                *next = median_erode_ultrasmooth_mask_at(idx, &current_mask, blocks_x, blocks_y);
            }
        }
        std::mem::swap(&mut current_mask, &mut next_mask);
    }

    // Flood fill to remove small ULTRASMOOTH regions
    let mut final_mask = current_mask.clone();
    let mut visited = vec![false; total_blocks];
    let mut component = Vec::new();
    let mut stack = Vec::new();
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let idx = bx + by * blocks_x;
            if current_mask[idx] && !visited[idx] {
                component.clear();
                stack.clear();
                stack.push((bx, by));
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
                if component.len() < ULTRASMOOTH_REGION_TOO_SMALL_THRESHOLD {
                    for &(cx, cy) in &component {
                        final_mask[cx + cy * blocks_x] = false;
                    }
                }
            }
        }
    }

    if use_parallel {
        block_mse_scales
            .par_iter_mut()
            .zip(final_mask.par_iter())
            .for_each(|(scale, is_ultrasmooth)| {
                if *is_ultrasmooth {
                    *scale = ULTRASMOOTH_BLOCK_MSE_SCALE;
                }
            });
    } else {
        for (scale, is_ultrasmooth) in block_mse_scales.iter_mut().zip(final_mask.iter()) {
            if *is_ultrasmooth {
                *scale = ULTRASMOOTH_BLOCK_MSE_SCALE;
            }
        }
    }

    block_mse_scales
}

#[inline]
fn is_ultrasmooth_seed_block(pixels: &RgbaBlock) -> bool {
    let mut luma_sum = 0.0f64;
    for i in 0..16 {
        let l = 0.299 * pixels[i][0] as f64 + 0.587 * pixels[i][1] as f64 + 0.114 * pixels[i][2] as f64;
        luma_sum += l;
    }
    let luma_avg = luma_sum / 16.0;

    if luma_avg < ULTRASMOOTH_DARK_THRESHOLD || luma_avg >= ULTRASMOOTH_BRIGHT_THRESHOLD {
        return false;
    }

    let max_std_dev = compute_block_max_std_dev(pixels);
    let mut yl = (max_std_dev / ULTRASMOOTH_BLOCK_STD_DEV_THRESHOLD).clamp(0.0, 1.0);
    yl = yl * yl;

    yl == 0.0
}

#[inline]
fn erode_ultrasmooth_mask_at(
    idx: usize,
    current_mask: &[bool],
    blocks_x: usize,
    blocks_y: usize,
) -> bool {
    if !current_mask[idx] {
        return false;
    }

    let x = idx % blocks_x;
    let y = idx / blocks_x;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx >= 0 && nx < blocks_x as i32 && ny >= 0 && ny < blocks_y as i32 {
                if !current_mask[nx as usize + ny as usize * blocks_x] {
                    return false;
                }
            }
        }
    }

    true
}

#[inline]
fn median_erode_ultrasmooth_mask_at(
    idx: usize,
    current_mask: &[bool],
    blocks_x: usize,
    blocks_y: usize,
) -> bool {
    if !current_mask[idx] {
        return false;
    }

    let x = idx % blocks_x;
    let y = idx / blocks_x;
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

    non_ultrasmooth_count < 5
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
            let pixels = rgba_block_at(rgba_blocks, block_index);
            let block_bits = bc7_block_bits(*block);
            sse += decode_bc7_error_bounded::<false>(block_bits, pixels, get_bc7_mode_bits(block_bits), true, u64::MAX, None)
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
    fn parallel_rdo_chunk_blocks_respects_rows_and_lookback() {
        let chunk_blocks = parallel_rdo_chunk_blocks(4096, 64, 1024, 8);
        assert_eq!(chunk_blocks % 64, 0);
        assert!(chunk_blocks >= 1024);
        assert!(chunk_blocks >= PARALLEL_RDO_MIN_CHUNK_BLOCKS);

        let tall_chunk_blocks = parallel_rdo_chunk_blocks(4096, 32, 2048, 8);
        assert_eq!(tall_chunk_blocks % 32, 0);
        assert!(tall_chunk_blocks >= 2048);
    }

    #[test]
    fn parallel_rdo_progress_reports_all_blocks() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let blocks_x = 64;
        let blocks_y = 33;
        let num_blocks = blocks_x * blocks_y;
        let mut blocks = vec![[0u8; 16]; num_blocks];
        let rgba_blocks = vec![[0u8; 4]; num_blocks * 16];
        let progress_blocks = AtomicUsize::new(0);
        let params = Bc7RdoParams {
            use_ultrasmooth_block_handling: false,
            ..Default::default()
        };

        let modified = reduce_entropy_bc7_parallel_with_progress(
            &mut blocks,
            &rgba_blocks,
            blocks_x,
            blocks_y,
            &params,
            |blocks| {
                progress_blocks.fetch_add(blocks, Ordering::Relaxed);
            },
        );

        assert_eq!(modified, 0);
        assert_eq!(progress_blocks.load(Ordering::Relaxed), num_blocks);
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode0_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 0 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode0_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode6_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 6 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode6_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode1_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 1 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode1_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode2_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 2 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode2_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode3_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 3 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode3_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode4_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 4 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode4_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode5_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 5 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode5_error_bounded(block_bits, &pixels, max_error)
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

        let block_bits = bc7_block_bits(block);
        let actual = decode_bc7_mode7_error_bounded(block_bits, &pixels, u64::MAX)
            .expect("unbounded fused mode 7 error should not early-exit");

        assert_bounded_error_exits_at_exact_error(actual, |max_error| {
            decode_bc7_mode7_error_bounded(block_bits, &pixels, max_error)
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
