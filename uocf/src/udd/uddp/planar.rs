//! SIMD-accelerated planar transform for RGBA8888 data.
//!
//! Planar transform (de-interleaving) improves compression ratios by grouping
//! identical color channels together, which increases the probability of
//! finding long matches for Zstd or LZ4.

// use wide::*;

/// De-interleaves RGBA8888 into RRRR...GGGG...BBBB...AAAA...
///
/// Input must be a multiple of 4 bytes. Output must be at least the same size.
pub fn transform_rgba_to_planar(src: &[u8], dst: &mut [u8]) {
    let n = src.len() / 4;
    if n == 0 { return; }

    let (r_out, rest) = dst.split_at_mut(n);
    let (g_out, rest) = rest.split_at_mut(n);
    let (b_out, a_out) = rest.split_at_mut(n);

    let mut i = 0;
    while i + 15 < n {
        for j in 0..16 {
            let px = i + j;
            let po = px * 4;
            r_out[px] = src[po];
            g_out[px] = src[po + 1];
            b_out[px] = src[po + 2];
            a_out[px] = src[po + 3];
        }
        i += 16;
    }

    // Tail
    for j in i..n {
        let po = j * 4;
        r_out[j] = src[po];
        g_out[j] = src[po + 1];
        b_out[j] = src[po + 2];
        a_out[j] = src[po + 3];
    }
}

/// Re-interleaves RRRR...GGGG...BBBB...AAAA... back to RGBA8888
pub fn transform_planar_to_rgba(src: &[u8], dst: &mut [u8]) {
    let n = src.len() / 4;
    if n == 0 { return; }

    let r = &src[0..n];
    let g = &src[n..2*n];
    let b = &src[2*n..3*n];
    let a = &src[3*n..4*n];

    let mut i = 0;
    while i + 15 < n {
        for j in 0..16 {
            let px = i + j;
            let po = px * 4;
            dst[po]     = r[px];
            dst[po + 1] = g[px];
            dst[po + 2] = b[px];
            dst[po + 3] = a[px];
        }
        i += 16;
    }

    for j in i..n {
        let po = j * 4;
        dst[po]     = r[j];
        dst[po + 1] = g[j];
        dst[po + 2] = b[j];
        dst[po + 3] = a[j];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planar_roundtrip() {
        // 33 pixels (132 bytes), tests both SIMD path and tail
        let mut original = Vec::new();
        for i in 0..33 {
            original.push(i as u8);         // R
            original.push((i + 10) as u8);  // G
            original.push((i + 20) as u8);  // B
            original.push(255);             // A
        }

        let mut planar = vec![0u8; original.len()];
        transform_rgba_to_planar(&original, &mut planar);

        // Verify planar layout (R, then G, then B, then A)
        let n = 33;
        for i in 0..n {
            assert_eq!(planar[i], i as u8, "Red mismatch at {}", i);
            assert_eq!(planar[n + i], (i + 10) as u8, "Green mismatch at {}", i);
            assert_eq!(planar[2 * n + i], (i + 20) as u8, "Blue mismatch at {}", i);
            assert_eq!(planar[3 * n + i], 255, "Alpha mismatch at {}", i);
        }

        let mut roundtrip = vec![0u8; original.len()];
        transform_planar_to_rgba(&planar, &mut roundtrip);

        assert_eq!(original, roundtrip, "Roundtrip data mismatch");
    }

    #[test]
    fn test_planar_empty() {
        let src = [];
        let mut dst = [];
        transform_rgba_to_planar(&src, &mut dst);
        transform_planar_to_rgba(&src, &mut dst);
    }

    #[test]
    fn test_planar_small() {
        // 1 pixel
        let original = vec![1, 2, 3, 4];
        let mut planar = vec![0u8; 4];
        transform_rgba_to_planar(&original, &mut planar);
        assert_eq!(planar, vec![1, 2, 3, 4]);

        let mut roundtrip = vec![0u8; 4];
        transform_planar_to_rgba(&planar, &mut roundtrip);
        assert_eq!(original, roundtrip);
    }
}
