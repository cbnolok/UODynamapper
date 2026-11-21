//! The RDO's x86-64 forms: which instruction set it runs with, and the AVX2 and AVX-512 error of a whole BC7 mode 6 or
//! mode 4 block, which are nearly all the RDO's decode trials on sprite atlases.
//!
//! The scalar decoders stop at the first texel whose running error reaches the bound. These decode four (AVX2) or
//! eight (AVX-512) texels at once and compare the sum with the bound after each group. Every texel's error is a
//! non-negative integer, so the running sum reaches the bound exactly when it does texel by texel: both give the same
//! answer, and the RDO writes the same blocks at every level. The arithmetic is the scalar decoder's, in 16-bit lanes,
//! which hold every intermediate value (`(high - low) * weight + 32` lies within ±16352).
//!
//! The RDO stops at AVX2 unless [`limit_bc7_rdo_simd`] allows AVX-512: on a Zen 5 laptop (Ryzen AI 9 365), whose
//! AVX-512 runs on 256-bit units, AVX-512 took 1-6 % more CPU than AVX2 on the mobile animation pages
//! (docs/agent_notes/atlas_page_compression.md).

use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(target_arch = "x86_64")]
use super::RgbaBlock;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// The instruction set the RDO runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Bc7RdoSimd {
    Scalar = 0,
    /// AVX2 and BMI2.
    Avx2 = 1,
    /// AVX-512 F, BW and VL, with AVX2 and BMI2.
    Avx512 = 2,
}

pub(super) const SIMD_SCALAR: u8 = Bc7RdoSimd::Scalar as u8;
pub(super) const SIMD_AVX2: u8 = Bc7RdoSimd::Avx2 as u8;
pub(super) const SIMD_AVX512: u8 = Bc7RdoSimd::Avx512 as u8;

/// The level the RDO stops at unless told otherwise.
const DEFAULT_SIMD_LIMIT: Bc7RdoSimd = Bc7RdoSimd::Avx2;

static SIMD_LIMIT: AtomicU8 = AtomicU8::new(DEFAULT_SIMD_LIMIT as u8);

/// Sets the highest instruction set the RDO picks (AVX2 by default), for measuring the levels against each other or
/// trying AVX-512; the blocks are the same at every level.
pub fn limit_bc7_rdo_simd(limit: Bc7RdoSimd) {
    SIMD_LIMIT.store(limit as u8, Ordering::Relaxed);
}

/// The instruction set the RDO runs with: the best this CPU has, within [`limit_bc7_rdo_simd`].
pub fn bc7_rdo_simd() -> Bc7RdoSimd {
    let limit = SIMD_LIMIT.load(Ordering::Relaxed);
    let detected = detected_simd();
    if (detected as u8) <= limit {
        detected
    } else if limit >= SIMD_AVX2 {
        Bc7RdoSimd::Avx2
    } else {
        Bc7RdoSimd::Scalar
    }
}

#[cfg(target_arch = "x86_64")]
pub(super) fn detected_simd() -> Bc7RdoSimd {
    let avx2 = is_x86_feature_detected!("avx2") && is_x86_feature_detected!("bmi2");
    if avx2
        && is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512bw")
        && is_x86_feature_detected!("avx512vl")
    {
        Bc7RdoSimd::Avx512
    } else if avx2 {
        Bc7RdoSimd::Avx2
    } else {
        Bc7RdoSimd::Scalar
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn detected_simd() -> Bc7RdoSimd {
    Bc7RdoSimd::Scalar
}

/// BC7's interpolation weights by index, as `pshufb` tables (the 2- and 3-bit tables padded with zeros).
#[cfg(target_arch = "x86_64")]
const WEIGHTS4_TABLE: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
#[cfg(target_arch = "x86_64")]
const WEIGHTS3_TABLE: [u8; 16] = [0, 9, 18, 27, 37, 46, 55, 64, 0, 0, 0, 0, 0, 0, 0, 0];
#[cfg(target_arch = "x86_64")]
const WEIGHTS2_TABLE: [u8; 16] = [0, 21, 43, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// The rounding term and shift of BC7's interpolation, `low + (((high - low) * weight + 32) >> 6)`.
#[cfg(target_arch = "x86_64")]
const INTERPOLATION_ROUNDING: i16 = 32;
#[cfg(target_arch = "x86_64")]
const INTERPOLATION_SHIFT: u32 = 6;
/// Texels a 16-lane group of 16-bit channels holds (four channels each), and the texels of a block.
#[cfg(target_arch = "x86_64")]
const TEXELS_PER_GROUP: usize = 4;
#[cfg(target_arch = "x86_64")]
const BLOCK_TEXELS: usize = 16;
/// The alpha channel's lane within a texel's four.
#[cfg(target_arch = "x86_64")]
const ALPHA_LANE: usize = 3;
/// The bits of a channel lane, for packing four endpoints into a 64-bit broadcast.
#[cfg(target_arch = "x86_64")]
const LANE_BITS: u32 = 16;

/// The low and high endpoints of a block, four 16-bit channels each in RGBA lane order, and the lane that interpolates
/// with the scalar weights instead of the colour weights (mode 4's rotated channel; mode 6 has the same weights in all).
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct Endpoints {
    low: u64,
    high: u64,
    scalar_lane: usize,
}

#[cfg(target_arch = "x86_64")]
fn pack_lanes(channels: [i32; 4]) -> u64 {
    channels
        .iter()
        .enumerate()
        .fold(0u64, |packed, (lane, &value)| packed | ((value as u16 as u64) << (lane as u32 * LANE_BITS)))
}

#[cfg(target_arch = "x86_64")]
fn mode6_endpoints(lo: u64, hi: u64) -> Endpoints {
    let p0 = (lo >> 63) as u32;
    let p1 = (hi & 1) as u32;
    let field = |shift: u32, p: u32| -> i32 { ((((lo >> shift) & 0x7F) as u32) << 1 | p) as i32 };
    Endpoints {
        low: pack_lanes([field(7, p0), field(21, p0), field(35, p0), field(49, p0)]),
        high: pack_lanes([field(14, p1), field(28, p1), field(42, p1), field(56, p1)]),
        scalar_lane: ALPHA_LANE,
    }
}

/// Mode 6's 16 four-bit indices, texel 0's (three bits) widened to a nibble like the others.
#[cfg(target_arch = "x86_64")]
fn mode6_index_nibbles(hi: u64) -> u64 {
    (hi & !0x0F) | ((hi >> 1) & 0x07)
}

/// Mode 4's endpoints, already swapped as its rotation swaps the decoded channels, and its two index streams, each
/// with texel 0's field (one bit shorter) widened in place: 16 two-bit and 16 three-bit indices.
#[cfg(target_arch = "x86_64")]
fn mode4_fields(block_bits: u128) -> (Endpoints, u64, u64, bool) {
    let rotation = ((block_bits >> 5) & 0x03) as usize;
    let index_flag = ((block_bits >> 7) & 1) != 0;
    let x = ((block_bits >> 8) & ((1u128 << 42) - 1)) as u64;
    let y_low = ((block_bits >> 48) & 0xFFFF_FFFF) as u64;
    let z = (block_bits >> 80) as u64;
    let expand5 = |v: u64| -> i32 {
        let v = (v & 0x1F) as u32;
        ((v << 3) | (v >> 2)) as i32
    };
    let expand6 = |v: u64| -> i32 {
        let v = (v & 0x3F) as u32;
        ((v << 2) | (v >> 4)) as i32
    };
    let mut low = [expand5(x), expand5(x >> 10), expand5(x >> 20), expand6(x >> 30)];
    let mut high = [expand5(x >> 5), expand5(x >> 15), expand5(x >> 25), expand6(x >> 36)];
    let scalar_lane = if rotation == 0 { ALPHA_LANE } else { rotation - 1 };
    low.swap(scalar_lane, ALPHA_LANE);
    high.swap(scalar_lane, ALPHA_LANE);

    let p2_stream = y_low | ((z & 1) << 32);
    let p3_stream = z >> 1;
    let two_bit = ((p2_stream >> 1) & !0x03) | ((p2_stream >> 2) & 0x01);
    let three_bit = ((p3_stream << 1) & !0x07) | (p3_stream & 0x03);
    let endpoints = Endpoints {
        low: pack_lanes(low),
        high: pack_lanes(high),
        scalar_lane,
    };
    (endpoints, two_bit, three_bit, index_flag)
}

/// Splits 16 fields of `BITS` bits (texel 0 lowest) into 16 bytes, one per texel.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn index_bytes<const BITS: u32>(fields: u64) -> __m128i {
    let field_mask = _mm256_set1_epi32(((1u32 << BITS) - 1) as i32);
    let shifts = _mm256_setr_epi32(
        0,
        BITS as i32,
        2 * BITS as i32,
        3 * BITS as i32,
        4 * BITS as i32,
        5 * BITS as i32,
        6 * BITS as i32,
        7 * BITS as i32,
    );
    let first = _mm256_and_si256(_mm256_srlv_epi32(_mm256_set1_epi32(fields as u32 as i32), shifts), field_mask);
    let second = _mm256_and_si256(
        _mm256_srlv_epi32(_mm256_set1_epi32((fields >> (8 * BITS)) as u32 as i32), shifts),
        field_mask,
    );
    // [first 0..3, second 0..3 | first 4..7, second 4..7] as 16-bit, then in texel order.
    let words = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packus_epi32(first, second));
    let bytes = _mm256_packus_epi16(words, words);
    _mm_unpacklo_epi64(_mm256_castsi256_si128(bytes), _mm256_extracti128_si256::<1>(bytes))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn lookup_weights(table: [u8; 16], indices: __m128i) -> __m128i {
    // SAFETY: the table is 16 bytes.
    _mm_shuffle_epi8(unsafe { _mm_loadu_si128(table.as_ptr().cast()) }, indices)
}

/// Four texels' 16 channel lanes, all ones in the scalar lane of each texel.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn scalar_lane_mask_avx2(scalar_lane: usize) -> __m256i {
    let lanes = _mm256_setr_epi16(0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3);
    _mm256_cmpeq_epi16(lanes, _mm256_set1_epi16(scalar_lane as i16))
}

/// The eight 32-bit lanes of `sum` added.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn horizontal_sum_avx2(sum: __m256i) -> u64 {
    let half = _mm_add_epi32(_mm256_castsi256_si128(sum), _mm256_extracti128_si256::<1>(sum));
    let quarter = _mm_add_epi32(half, _mm_shuffle_epi32::<0b01_00_11_10>(half));
    let total = _mm_add_epi32(quarter, _mm_shuffle_epi32::<0b10_11_00_01>(quarter));
    _mm_cvtsi128_si32(total) as u32 as u64
}

/// The squared error of a block whose texels interpolate `endpoints` by the colour and scalar weights (one byte per
/// texel), four texels to a 256-bit group. It stops after a group that brings the error to `bound`, returning the error
/// so far: most trials pass the bound within the first few texels.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,bmi2")]
fn block_error_avx2(
    source: &RgbaBlock,
    endpoints: Endpoints,
    colour_weights: __m128i,
    scalar_weights: __m128i,
    bound: u64,
) -> u64 {
    let low = _mm256_set1_epi64x(endpoints.low as i64);
    let span = _mm256_sub_epi16(_mm256_set1_epi64x(endpoints.high as i64), low);
    let rounding = _mm256_set1_epi16(INTERPOLATION_ROUNDING);
    let scalar_mask = scalar_lane_mask_avx2(endpoints.scalar_lane);
    // SAFETY: `RgbaBlock` is 64 contiguous bytes.
    let source_bytes = unsafe {
        [
            _mm_loadu_si128(source.as_ptr().cast()),
            _mm_loadu_si128(source.as_ptr().add(TEXELS_PER_GROUP).cast()),
            _mm_loadu_si128(source.as_ptr().add(2 * TEXELS_PER_GROUP).cast()),
            _mm_loadu_si128(source.as_ptr().add(3 * TEXELS_PER_GROUP).cast()),
        ]
    };
    let mut sum = _mm256_setzero_si256();
    for (group, source_group) in source_bytes.into_iter().enumerate() {
        let first = (group * TEXELS_PER_GROUP) as i8;
        let repeat = _mm_setr_epi8(
            first,
            first,
            first,
            first,
            first + 1,
            first + 1,
            first + 1,
            first + 1,
            first + 2,
            first + 2,
            first + 2,
            first + 2,
            first + 3,
            first + 3,
            first + 3,
            first + 3,
        );
        let colour = _mm256_cvtepu8_epi16(_mm_shuffle_epi8(colour_weights, repeat));
        let scalar = _mm256_cvtepu8_epi16(_mm_shuffle_epi8(scalar_weights, repeat));
        let weights = _mm256_blendv_epi8(colour, scalar, scalar_mask);
        let step = _mm256_srai_epi16::<{ INTERPOLATION_SHIFT as i32 }>(_mm256_add_epi16(
            _mm256_mullo_epi16(span, weights),
            rounding,
        ));
        let decoded = _mm256_add_epi16(low, step);
        let difference = _mm256_sub_epi16(_mm256_cvtepu8_epi16(source_group), decoded);
        sum = _mm256_add_epi32(sum, _mm256_madd_epi16(difference, difference));
        if group + 1 < source_bytes.len() {
            let error = horizontal_sum_avx2(sum);
            if error >= bound {
                return error;
            }
        }
    }
    horizontal_sum_avx2(sum)
}

/// [`block_error_avx2`] with eight texels to a 512-bit group.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2,bmi2")]
fn block_error_avx512(
    source: &RgbaBlock,
    endpoints: Endpoints,
    colour_weights: __m128i,
    scalar_weights: __m128i,
    bound: u64,
) -> u64 {
    const TEXELS_PER_HALF: i16 = (BLOCK_TEXELS / 2) as i16;
    let low = _mm512_set1_epi64(endpoints.low as i64);
    let span = _mm512_sub_epi16(_mm512_set1_epi64(endpoints.high as i64), low);
    let rounding = _mm512_set1_epi16(INTERPOLATION_ROUNDING);
    let scalar_mask: __mmask32 = 0x1111_1111u32 << endpoints.scalar_lane;
    // Each texel's weight index repeated over its four channel lanes, for the first and the second eight texels.
    let repeat_first = _mm512_set_epi16(
        7, 7, 7, 7, 6, 6, 6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 0, 0, 0, 0,
    );
    let repeat_second = _mm512_add_epi16(repeat_first, _mm512_set1_epi16(TEXELS_PER_HALF));
    let colour = _mm512_castsi256_si512(_mm256_cvtepu8_epi16(colour_weights));
    let scalar = _mm512_castsi256_si512(_mm256_cvtepu8_epi16(scalar_weights));
    // SAFETY: `RgbaBlock` is 64 contiguous bytes.
    let source_bytes = unsafe { _mm512_loadu_si512(source.as_ptr().cast()) };
    let halves = [
        (repeat_first, _mm512_castsi512_si256(source_bytes)),
        (repeat_second, _mm512_extracti64x4_epi64::<1>(source_bytes)),
    ];
    let mut sum = _mm512_setzero_si512();
    for (half, (repeat, source_half)) in halves.into_iter().enumerate() {
        let weights = _mm512_mask_blend_epi16(
            scalar_mask,
            _mm512_permutexvar_epi16(repeat, colour),
            _mm512_permutexvar_epi16(repeat, scalar),
        );
        let step = _mm512_srai_epi16::<{ INTERPOLATION_SHIFT }>(_mm512_add_epi16(
            _mm512_mullo_epi16(span, weights),
            rounding,
        ));
        let decoded = _mm512_add_epi16(low, step);
        let difference = _mm512_sub_epi16(_mm512_cvtepu8_epi16(source_half), decoded);
        sum = _mm512_add_epi32(sum, _mm512_madd_epi16(difference, difference));
        if half == 0 {
            let error = _mm512_reduce_add_epi32(sum) as u32 as u64;
            if error >= bound {
                return error;
            }
        }
    }
    _mm512_reduce_add_epi32(sum) as u32 as u64
}

/// The squared error of a BC7 mode 6 block against `source`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,bmi2")]
pub(super) fn mode6_error_avx2(block_bits: u128, source: &RgbaBlock, bound: u64) -> u64 {
    let (lo, hi) = (block_bits as u64, (block_bits >> 64) as u64);
    let weights = lookup_weights(WEIGHTS4_TABLE, index_bytes::<4>(mode6_index_nibbles(hi)));
    block_error_avx2(source, mode6_endpoints(lo, hi), weights, weights, bound)
}

/// [`mode6_error_avx2`] in AVX-512.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2,bmi2")]
pub(super) fn mode6_error_avx512(block_bits: u128, source: &RgbaBlock, bound: u64) -> u64 {
    let (lo, hi) = (block_bits as u64, (block_bits >> 64) as u64);
    let weights = lookup_weights(WEIGHTS4_TABLE, index_bytes::<4>(mode6_index_nibbles(hi)));
    block_error_avx512(source, mode6_endpoints(lo, hi), weights, weights, bound)
}

/// Mode 4's colour and scalar weights: the index flag gives the colour the three-bit indices.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
fn mode4_weights(two_bit: u64, three_bit: u64, index_flag: bool) -> (__m128i, __m128i) {
    let two = lookup_weights(WEIGHTS2_TABLE, index_bytes::<2>(two_bit));
    let three = lookup_weights(WEIGHTS3_TABLE, index_bytes::<3>(three_bit));
    if index_flag { (three, two) } else { (two, three) }
}

/// The squared error of a BC7 mode 4 block against `source`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,bmi2")]
pub(super) fn mode4_error_avx2(block_bits: u128, source: &RgbaBlock, bound: u64) -> u64 {
    let (endpoints, two_bit, three_bit, index_flag) = mode4_fields(block_bits);
    let (colour, scalar) = mode4_weights(two_bit, three_bit, index_flag);
    block_error_avx2(source, endpoints, colour, scalar, bound)
}

/// [`mode4_error_avx2`] in AVX-512.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw,avx512vl,avx2,bmi2")]
pub(super) fn mode4_error_avx512(block_bits: u128, source: &RgbaBlock, bound: u64) -> u64 {
    let (endpoints, two_bit, three_bit, index_flag) = mode4_fields(block_bits);
    let (colour, scalar) = mode4_weights(two_bit, three_bit, index_flag);
    block_error_avx512(source, endpoints, colour, scalar, bound)
}
