#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RgbaBounds {
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
}

const FOUR_PIXEL_ALPHA_MASK: u128 = 0xFF00_0000_FF00_0000_FF00_0000_FF00_0000;
const FOUR_PIXEL_BYTES: usize = 16;
const EIGHT_PIXEL_BYTES: usize = 32;
const SIXTEEN_PIXEL_BYTES: usize = 64;
const AVX2_ALPHA_BYTE_MASK: u32 = 0x8888_8888;

pub(crate) fn nonzero_alpha_bounds(
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Option<RgbaBounds> {
    nonzero_alpha_bounds_in_rect(rgba, width, height, 0, 0, width, height)
}

pub(crate) fn count_nonzero_alpha(rgba: &[u8]) -> u64 {
    debug_assert_eq!(rgba.len() % 4, 0);

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if std::is_x86_feature_detected!("avx2") {
        return unsafe { count_nonzero_alpha_avx2(rgba) };
    }

    count_nonzero_alpha_scalar(rgba)
}

pub(crate) fn nonzero_alpha_bounds_in_rect(
    rgba: &[u8],
    width: usize,
    height: usize,
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
) -> Option<RgbaBounds> {
    debug_assert_eq!(rgba.len(), width * height * 4);
    debug_assert!(left <= right && right <= width);
    debug_assert!(top <= bottom && bottom <= height);

    if left >= right || top >= bottom {
        return None;
    }

    let stride = width * 4;
    let mut min_x = right;
    let mut min_y = bottom;
    let mut max_x = left;
    let mut max_y = top;
    let mut found = false;

    for y in top..bottom {
        let row_start = y * stride + left * 4;
        let row_end = y * stride + right * 4;
        let Some((row_min, row_max)) = row_nonzero_alpha_span(&rgba[row_start..row_end]) else {
            continue;
        };

        found = true;
        min_x = min_x.min(left + row_min);
        min_y = min_y.min(y);
        max_x = max_x.max(left + row_max);
        max_y = max_y.max(y);
    }

    found.then_some(RgbaBounds {
        left: min_x,
        top: min_y,
        right: max_x + 1,
        bottom: max_y + 1,
    })
}

fn count_nonzero_alpha_scalar(rgba: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut index = 3usize;
    while index < rgba.len() {
        count += u64::from(rgba[index] != 0);
        index += 4;
    }
    count
}

fn row_nonzero_alpha_span(row: &[u8]) -> Option<(usize, usize)> {
    debug_assert_eq!(row.len() % 4, 0);
    let first = row_first_nonzero_alpha(row)?;
    let last = row_last_nonzero_alpha(row);
    Some((first, last))
}

fn row_first_nonzero_alpha(row: &[u8]) -> Option<usize> {
    let mut pixel_base = 0usize;
    let mut chunks = row.chunks_exact(SIXTEEN_PIXEL_BYTES);
    for chunk in chunks.by_ref() {
        if let Some(lane) = first_nonzero_alpha_lane_64(chunk) {
            return Some(pixel_base + lane);
        }
        pixel_base += 16;
    }

    let mut chunks = chunks.remainder().chunks_exact(FOUR_PIXEL_BYTES);
    for chunk in chunks.by_ref() {
        let alpha_bits = four_pixel_alpha_bits(chunk);
        if alpha_bits != 0 {
            return Some(pixel_base + first_alpha_lane(alpha_bits));
        }
        pixel_base += 4;
    }

    for (lane, pixel) in chunks.remainder().chunks_exact(4).enumerate() {
        if pixel[3] != 0 {
            return Some(pixel_base + lane);
        }
    }

    None
}

fn row_last_nonzero_alpha(row: &[u8]) -> usize {
    let chunk_count = row.len() / SIXTEEN_PIXEL_BYTES;
    let remainder = &row[chunk_count * SIXTEEN_PIXEL_BYTES..];
    let remainder_chunk_count = remainder.len() / FOUR_PIXEL_BYTES;
    let remainder_tail = &remainder[remainder_chunk_count * FOUR_PIXEL_BYTES..];
    for lane in (0..remainder_tail.len() / 4).rev() {
        if remainder_tail[lane * 4 + 3] != 0 {
            return chunk_count * 16 + remainder_chunk_count * 4 + lane;
        }
    }

    for chunk_index in (0..remainder_chunk_count).rev() {
        let chunk_start = chunk_index * FOUR_PIXEL_BYTES;
        let chunk = &remainder[chunk_start..chunk_start + FOUR_PIXEL_BYTES];
        let alpha_bits = four_pixel_alpha_bits(chunk);
        if alpha_bits != 0 {
            return chunk_count * 16 + chunk_index * 4 + last_alpha_lane(alpha_bits);
        }
    }

    for chunk_index in (0..chunk_count).rev() {
        let chunk_start = chunk_index * SIXTEEN_PIXEL_BYTES;
        let chunk = &row[chunk_start..chunk_start + SIXTEEN_PIXEL_BYTES];
        if let Some(lane) = last_nonzero_alpha_lane_64(chunk) {
            return chunk_index * 16 + lane;
        }
    }

    unreachable!("last alpha lookup is only called after finding nonzero alpha")
}

#[inline(always)]
fn four_pixel_alpha_bits(chunk: &[u8]) -> u128 {
    debug_assert_eq!(chunk.len(), FOUR_PIXEL_BYTES);
    let bytes: [u8; 16] = chunk.try_into().expect("chunk length is checked");
    u128::from_le_bytes(bytes) & FOUR_PIXEL_ALPHA_MASK
}

#[inline(always)]
fn first_nonzero_alpha_lane_64(chunk: &[u8]) -> Option<usize> {
    debug_assert_eq!(chunk.len(), SIXTEEN_PIXEL_BYTES);
    for lane_base in [0usize, 4, 8, 12] {
        let chunk_start = lane_base * 4;
        let alpha_bits = four_pixel_alpha_bits(&chunk[chunk_start..chunk_start + FOUR_PIXEL_BYTES]);
        if alpha_bits != 0 {
            return Some(lane_base + first_alpha_lane(alpha_bits));
        }
    }
    None
}

#[inline(always)]
fn last_nonzero_alpha_lane_64(chunk: &[u8]) -> Option<usize> {
    debug_assert_eq!(chunk.len(), SIXTEEN_PIXEL_BYTES);
    for lane_base in [12usize, 8, 4, 0] {
        let chunk_start = lane_base * 4;
        let alpha_bits = four_pixel_alpha_bits(&chunk[chunk_start..chunk_start + FOUR_PIXEL_BYTES]);
        if alpha_bits != 0 {
            return Some(lane_base + last_alpha_lane(alpha_bits));
        }
    }
    None
}

#[inline(always)]
fn first_alpha_lane(alpha_bits: u128) -> usize {
    debug_assert_ne!(alpha_bits, 0);
    alpha_bits.trailing_zeros() as usize / 32
}

#[inline(always)]
fn last_alpha_lane(alpha_bits: u128) -> usize {
    debug_assert_ne!(alpha_bits, 0);
    (127 - alpha_bits.leading_zeros() as usize) / 32
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn count_nonzero_alpha_avx2(rgba: &[u8]) -> u64 {
    let mut count = 0u64;
    let mut chunks = rgba.chunks_exact(EIGHT_PIXEL_BYTES);
    for chunk in chunks.by_ref() {
        count += u64::from(avx2_nonzero_alpha_mask_32(chunk.as_ptr()).count_ones());
    }
    count + count_nonzero_alpha_scalar(chunks.remainder())
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn avx2_nonzero_alpha_mask_32(ptr: *const u8) -> u32 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    let bytes = _mm256_loadu_si256(ptr.cast());
    let zero = _mm256_setzero_si256();
    let zero_mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(bytes, zero)) as u32;
    !zero_mask & AVX2_ALPHA_BYTE_MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bounds_across_full_image() {
        let mut rgba = vec![0u8; 7 * 3 * 4];
        set_alpha(&mut rgba, 7, 2, 1, 99);
        set_alpha(&mut rgba, 7, 5, 2, 255);

        assert_eq!(
            nonzero_alpha_bounds(&rgba, 7, 3),
            Some(RgbaBounds {
                left: 2,
                top: 1,
                right: 6,
                bottom: 3,
            })
        );
    }

    #[test]
    fn respects_clip_rect() {
        let mut rgba = vec![0u8; 8 * 4 * 4];
        set_alpha(&mut rgba, 8, 1, 1, 255);
        set_alpha(&mut rgba, 8, 6, 3, 255);
        set_alpha(&mut rgba, 8, 4, 2, 255);

        assert_eq!(
            nonzero_alpha_bounds_in_rect(&rgba, 8, 4, 2, 1, 6, 4),
            Some(RgbaBounds {
                left: 4,
                top: 2,
                right: 5,
                bottom: 3,
            })
        );
    }

    #[test]
    fn returns_none_for_empty_alpha() {
        let rgba = vec![0u8; 5 * 5 * 4];
        assert_eq!(nonzero_alpha_bounds(&rgba, 5, 5), None);
    }

    #[test]
    fn counts_nonzero_alpha_pixels() {
        let mut rgba = vec![0u8; 6 * 4];
        rgba[3] = 1;
        rgba[11] = 255;
        rgba[19] = 7;

        assert_eq!(count_nonzero_alpha(&rgba), 3);
    }

    #[test]
    fn counts_nonzero_alpha_across_wide_blocks_and_tail() {
        let mut rgba = vec![0u8; 34 * 4];
        for x in [0usize, 15, 16, 31, 32, 33] {
            rgba[x * 4 + 3] = x as u8 + 1;
        }

        assert_eq!(count_nonzero_alpha(&rgba), 6);
    }

    #[test]
    fn row_span_handles_large_empty_prefix_and_tail() {
        let mut row = vec![0u8; 21 * 4];
        row[16 * 4 + 3] = 1;
        row[20 * 4 + 3] = 2;

        assert_eq!(row_nonzero_alpha_span(&row), Some((16, 20)));
    }

    fn set_alpha(rgba: &mut [u8], width: usize, x: usize, y: usize, alpha: u8) {
        rgba[(y * width + x) * 4 + 3] = alpha;
    }
}
