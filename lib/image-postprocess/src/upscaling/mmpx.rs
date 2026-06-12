//! MMPX (Multi-pixel Magnification) upscaling algorithm.
//!
//! References:
//! - https://casual-effects.com/research/McGuire2021PixelArt/McGuire2021PixelArt.pdf
//! - https://github.com/pierogis/mmpx-rs
//!
//! Native support for 2x magnification. 4x is implemented as two 2x passes.

pub fn apply_mmpx(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    match scale {
        2 => magnify2(width, height, rgba),
        4 => {
            let (width2, height2, rgba2) = magnify2(width, height, rgba);
            magnify2(width2, height2, &rgba2)
        }
        _ => (width, height, rgba.to_vec()),
    }
}

fn magnify2(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() {
        return (width.saturating_mul(2), height.saturating_mul(2), Vec::new());
    }

    let src_pixels = pack_rgba(rgba);
    assert_eq!(src_pixels.len(), width as usize * height as usize);

    let dst_width = width * 2;
    let dst_height = height * 2;
    let mut dst_pixels = vec![0; dst_width as usize * dst_height as usize];

    for y in 0..height as i32 {
        let mut a = pixel(&src_pixels, width, height, -1, y - 1);
        let mut b = pixel(&src_pixels, width, height, 0, y - 1);

        let mut d = pixel(&src_pixels, width, height, -1, y);
        let mut e = pixel(&src_pixels, width, height, 0, y);
        let mut f = pixel(&src_pixels, width, height, 1, y);

        let mut g = pixel(&src_pixels, width, height, -1, y + 1);
        let mut h = pixel(&src_pixels, width, height, 0, y + 1);

        let mut q = pixel(&src_pixels, width, height, -2, y);

        for x in 0..width as i32 {
            let c = pixel(&src_pixels, width, height, x + 1, y - 1);
            let r = pixel(&src_pixels, width, height, x + 2, y);
            let i = pixel(&src_pixels, width, height, x + 1, y + 1);
            let p = pixel(&src_pixels, width, height, x, y - 2);
            let s = pixel(&src_pixels, width, height, x, y + 2);

            let (j, k, l, m) = magnify_pixel(MmpxNeighborhood {
                a, b, c, d, e, f, g, h, i, p, q, r, s,
                x,
                y,
                src_pixels: &src_pixels,
                width,
                height,
            });

            write2x(&mut dst_pixels, dst_width, x as u32, y as u32, j, k, l, m);

            a = b;
            b = c;
            q = d;
            d = e;
            e = f;
            f = r;
            g = h;
            h = i;
        }
    }

    (dst_width, dst_height, unpack_rgba(&dst_pixels))
}

#[derive(Debug, Clone, Copy)]
struct MmpxNeighborhood<'a> {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
    f: u32,
    g: u32,
    h: u32,
    i: u32,
    p: u32,
    q: u32,
    r: u32,
    s: u32,
    x: i32,
    y: i32,
    src_pixels: &'a [u32],
    width: u32,
    height: u32,
}

fn magnify_pixel(n: MmpxNeighborhood<'_>) -> (u32, u32, u32, u32) {
    let MmpxNeighborhood {
        a,
        b,
        c,
        d,
        e,
        f,
        g,
        h,
        i,
        p,
        q,
        r,
        s,
        x,
        y,
        src_pixels,
        width,
        height,
    } = n;

    let mut j = e;
    let mut k = e;
    let mut l = e;
    let mut m = e;

    if none_eq8(e, a, b, c, d, f, g, h, i) {
        return (j, k, l, m);
    }

    let b_luma = luma(b);
    let d_luma = luma(d);
    let e_luma = luma(e);
    let f_luma = luma(f);
    let h_luma = luma(h);

    if (d == b && d != h && d != f)
        && (e_luma >= d_luma || e == a)
        && any_eq3(e, a, c, g)
        && (e_luma < d_luma || a != d || e != p || e != q)
    {
        j = d;
    }

    if (b == f && b != d && b != h)
        && (e_luma >= b_luma || e == c)
        && any_eq3(e, a, c, i)
        && (e_luma < b_luma || c != b || e != p || e != r)
    {
        k = b;
    }

    if (h == d && h != f && h != b)
        && (e_luma >= h_luma || e == g)
        && any_eq3(e, a, g, i)
        && (e_luma < h_luma || g != h || e != s || e != q)
    {
        l = h;
    }

    if (f == h && f != b && f != d)
        && (e_luma >= f_luma || e == i)
        && any_eq3(e, c, g, i)
        && (e_luma < f_luma || i != h || e != r || e != s)
    {
        m = f;
    }

    if (e != f && all_eq4(e, c, i, d, q) && all_eq2(f, b, h))
        && f != pixel(src_pixels, width, height, x + 3, y)
    {
        k = f;
        m = f;
    }
    if (e != d && all_eq4(e, a, g, f, r) && all_eq2(d, b, h))
        && d != pixel(src_pixels, width, height, x - 3, y)
    {
        j = d;
        l = d;
    }
    if (e != h && all_eq4(e, g, i, b, p) && all_eq2(h, d, f))
        && h != pixel(src_pixels, width, height, x, y + 3)
    {
        l = h;
        m = h;
    }
    if (e != b && all_eq4(e, a, c, h, s) && all_eq2(b, d, f))
        && b != pixel(src_pixels, width, height, x, y - 3)
    {
        j = b;
        k = b;
    }

    if b_luma < e_luma && all_eq4(e, g, h, i, s) && none_eq4(e, a, d, c, f) {
        j = b;
        k = b;
    }
    if h_luma < e_luma && all_eq4(e, a, b, c, p) && none_eq4(e, d, g, i, f) {
        l = h;
        m = h;
    }
    if f_luma < e_luma && all_eq4(e, a, d, g, q) && none_eq4(e, b, c, i, h) {
        k = f;
        m = f;
    }
    if d_luma < e_luma && all_eq4(e, c, f, i, r) && none_eq4(e, b, a, g, h) {
        j = d;
        l = d;
    }

    if h != b {
        if h != a && h != e && h != c {
            if all_eq3(h, g, f, r) && none_eq2(h, d, pixel(src_pixels, width, height, x + 2, y - 1)) {
                l = m;
            }
            if all_eq3(h, i, d, q) && none_eq2(h, f, pixel(src_pixels, width, height, x - 2, y - 1)) {
                m = l;
            }
        }

        if b != i && b != g && b != e {
            if all_eq3(b, a, f, r) && none_eq2(b, d, pixel(src_pixels, width, height, x + 2, y + 1)) {
                j = k;
            }
            if all_eq3(b, c, d, q) && none_eq2(b, f, pixel(src_pixels, width, height, x - 2, y + 1)) {
                k = j;
            }
        }
    }

    if f != d {
        if d != i && d != e && d != c {
            if all_eq3(d, a, h, s) && none_eq2(d, b, pixel(src_pixels, width, height, x + 1, y + 2)) {
                j = l;
            }
            if all_eq3(d, g, b, p) && none_eq2(d, h, pixel(src_pixels, width, height, x + 1, y - 2)) {
                l = j;
            }
        }

        if f != e && f != a && f != g {
            if all_eq3(f, c, h, s) && none_eq2(f, b, pixel(src_pixels, width, height, x - 1, y + 2)) {
                k = m;
            }
            if all_eq3(f, i, b, p) && none_eq2(f, h, pixel(src_pixels, width, height, x - 1, y - 2)) {
                m = k;
            }
        }
    }

    (j, k, l, m)
}

fn pack_rgba(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|px| u32::from_le_bytes([px[0], px[1], px[2], px[3]]))
        .collect()
}

fn unpack_rgba(pixels: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&pixel.to_le_bytes());
    }
    rgba
}

fn pixel(pixels: &[u32], width: u32, height: u32, x: i32, y: i32) -> u32 {
    let clamped_x = x.clamp(0, width as i32 - 1) as u32;
    let clamped_y = y.clamp(0, height as i32 - 1) as u32;
    pixels[(clamped_y * width + clamped_x) as usize]
}

fn write2x(
    dst: &mut [u32],
    dst_width: u32,
    src_x: u32,
    src_y: u32,
    j: u32,
    k: u32,
    l: u32,
    m: u32,
) {
    let dst_x = src_x * 2;
    let dst_y = src_y * 2;
    let top = (dst_y * dst_width + dst_x) as usize;
    let bottom = top + dst_width as usize;

    dst[top] = j;
    dst[top + 1] = k;
    dst[bottom] = l;
    dst[bottom + 1] = m;
}

fn luma(pixel: u32) -> u32 {
    let alpha = pixel >> 24;
    let red = pixel & 0x000000FF;
    let green = (pixel & 0x0000FF00) >> 8;
    let blue = (pixel & 0x00FF0000) >> 16;
    (red + green + blue + 1) * (256 - alpha)
}

fn all_eq2(b: u32, a0: u32, a1: u32) -> bool {
    ((b ^ a0) | (b ^ a1)) == 0
}

fn all_eq3(b: u32, a0: u32, a1: u32, a2: u32) -> bool {
    ((b ^ a0) | (b ^ a1) | (b ^ a2)) == 0
}

fn all_eq4(b: u32, a0: u32, a1: u32, a2: u32, a3: u32) -> bool {
    ((b ^ a0) | (b ^ a1) | (b ^ a2) | (b ^ a3)) == 0
}

fn any_eq3(b: u32, a0: u32, a1: u32, a2: u32) -> bool {
    b == a0 || b == a1 || b == a2
}

fn none_eq2(b: u32, a0: u32, a1: u32) -> bool {
    b != a0 && b != a1
}

fn none_eq4(b: u32, a0: u32, a1: u32, a2: u32, a3: u32) -> bool {
    b != a0 && b != a1 && b != a2 && b != a3
}

fn none_eq8(b: u32, a0: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, a6: u32, a7: u32) -> bool {
    a0 != b && a1 != b && a2 != b && a3 != b && a4 != b && a5 != b && a6 != b && a7 != b
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn mmpx_scales_2x_and_4x() {
        let rgba = [BLACK, WHITE, WHITE, BLACK].concat();

        let (width2, height2, out2) = apply_mmpx(2, 2, &rgba, 2);
        assert_eq!((width2, height2), (4, 4));
        assert_eq!(out2.len(), 4 * 4 * 4);

        let (width4, height4, out4) = apply_mmpx(2, 2, &rgba, 4);
        assert_eq!((width4, height4), (8, 8));
        assert_eq!(out4.len(), 8 * 8 * 4);
    }

    #[test]
    fn mmpx_keeps_solid_image_solid() {
        let rgba = vec![BLACK[0], BLACK[1], BLACK[2], BLACK[3]].repeat(9);

        let (_, _, out) = apply_mmpx(3, 3, &rgba, 2);

        assert_eq!(out, vec![BLACK[0], BLACK[1], BLACK[2], BLACK[3]].repeat(36));
    }

    #[test]
    fn mmpx_keeps_isolated_transparent_pixel_nearest() {
        let rgba = [
            BLACK, WHITE, BLACK,
            WHITE, CLEAR, WHITE,
            BLACK, WHITE, BLACK,
        ].concat();

        let (_, _, out) = apply_mmpx(3, 3, &rgba, 2);
        let center = ((2 * 6 + 2) * 4) as usize;

        assert_eq!(&out[center..center + 4], &CLEAR);
        assert_eq!(&out[center + 4..center + 8], &CLEAR);
        assert_eq!(&out[center + 6 * 4..center + 6 * 4 + 4], &CLEAR);
        assert_eq!(&out[center + 7 * 4..center + 7 * 4 + 4], &CLEAR);
    }

    #[test]
    fn mmpx_refines_a_diagonal_edge_corner() {
        let rgba = [
            BLACK, BLACK, WHITE,
            BLACK, WHITE, WHITE,
            WHITE, WHITE, WHITE,
        ].concat();

        let (_, _, out) = apply_mmpx(3, 3, &rgba, 2);
        let center_top_left = ((2 * 6 + 2) * 4) as usize;

        assert_eq!(&out[center_top_left..center_top_left + 4], &BLACK);
    }
}
