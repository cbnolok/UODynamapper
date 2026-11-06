//! EPX (Eric's Pixel Expansion) / Scale2x upscaling algorithm.
//!
//! Original by Eric Johnston at LucasArts.
//! Reference: https://en.wikipedia.org/wiki/Pixel-art_scaling_algorithms#Scale2x
//! Reference: https://github.com/daelsepara/PixelScalerWin
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/epx.c


pub fn apply_epx(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    match scale {
        2 => apply_scale2x(width, height, rgba),
        3 => apply_scale3x(width, height, rgba),
        4 => apply_scale4x(width, height, rgba),
        _ => (width, height, rgba.to_vec()),
    }
}

fn apply_scale2x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let target_width = width * 2;
    let target_height = height * 2;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    let width = width as usize;
    let height = height as usize;
    let target_width_usize = target_width as usize;
    let source_stride = width * 4;
    let target_stride = target_width_usize * 4;

    for y in 0..height {
        let row = y * source_stride;
        let row_above = y.saturating_sub(1) * source_stride;
        let row_below = (y + 1).min(height - 1) * source_stride;
        let target_row = y * 2 * target_stride;

        for x in 0..width {
            /*
                A B C
                D E F
                G H I
            */
            let col = x * 4;
            let col_left = x.saturating_sub(1) * 4;
            let col_right = (x + 1).min(width - 1) * 4;

            let p_e = read_pixel(rgba, row + col);
            let p_b = read_pixel(rgba, row_above + col);
            let p_d = read_pixel(rgba, row + col_left);
            let p_f = read_pixel(rgba, row + col_right);
            let p_h = read_pixel(rgba, row_below + col);

            let mut p1 = p_e;
            let mut p2 = p_e;
            let mut p3 = p_e;
            let mut p4 = p_e;

            if p_b != p_h && p_d != p_f {
                if p_d == p_b {
                    p1 = p_d;
                }
                if p_b == p_f {
                    p2 = p_f;
                }
                if p_d == p_h {
                    p3 = p_d;
                }
                if p_f == p_h {
                    p4 = p_f;
                }
            }

            let target_idx = target_row + x * 8;
            write_pixel(&mut out_rgba, target_idx, p1);
            write_pixel(&mut out_rgba, target_idx + 4, p2);
            write_pixel(&mut out_rgba, target_idx + target_stride, p3);
            write_pixel(&mut out_rgba, target_idx + target_stride + 4, p4);
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_scale3x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let target_width = width * 3;
    let target_height = height * 3;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    let width = width as usize;
    let height = height as usize;
    let target_width_usize = target_width as usize;
    let source_stride = width * 4;
    let target_stride = target_width_usize * 4;

    for y in 0..height {
        let row = y * source_stride;
        let row_above = y.saturating_sub(1) * source_stride;
        let row_below = (y + 1).min(height - 1) * source_stride;
        let target_row = y * 3 * target_stride;

        for x in 0..width {
            let col = x * 4;
            let col_left = x.saturating_sub(1) * 4;
            let col_right = (x + 1).min(width - 1) * 4;

            let p_a = read_pixel(rgba, row_above + col_left);
            let p_b = read_pixel(rgba, row_above + col);
            let p_c = read_pixel(rgba, row_above + col_right);
            let p_d = read_pixel(rgba, row + col_left);
            let p_e = read_pixel(rgba, row + col);
            let p_f = read_pixel(rgba, row + col_right);
            let p_g = read_pixel(rgba, row_below + col_left);
            let p_h = read_pixel(rgba, row_below + col);
            let p_i = read_pixel(rgba, row_below + col_right);

            let mut out = [p_e; 9];

            if p_b != p_h && p_d != p_f {
                if p_d == p_b {
                    out[0] = p_d;
                }
                if (p_d == p_b && p_e != p_c) || (p_b == p_f && p_e != p_a) {
                    out[1] = p_b;
                }
                if p_b == p_f {
                    out[2] = p_f;
                }
                if (p_d == p_b && p_e != p_g) || (p_d == p_h && p_e != p_a) {
                    out[3] = p_d;
                }
                // out[4] is always p_e
                if (p_b == p_f && p_e != p_i) || (p_f == p_h && p_e != p_c) {
                    out[5] = p_f;
                }
                if p_d == p_h {
                    out[6] = p_d;
                }
                if (p_d == p_h && p_e != p_i) || (p_f == p_h && p_e != p_g) {
                    out[7] = p_h;
                }
                if p_f == p_h {
                    out[8] = p_f;
                }
            }

            let target_idx = target_row + x * 12;
            write_pixel(&mut out_rgba, target_idx, out[0]);
            write_pixel(&mut out_rgba, target_idx + 4, out[1]);
            write_pixel(&mut out_rgba, target_idx + 8, out[2]);
            write_pixel(&mut out_rgba, target_idx + target_stride, out[3]);
            write_pixel(&mut out_rgba, target_idx + target_stride + 4, out[4]);
            write_pixel(&mut out_rgba, target_idx + target_stride + 8, out[5]);
            write_pixel(&mut out_rgba, target_idx + target_stride * 2, out[6]);
            write_pixel(&mut out_rgba, target_idx + target_stride * 2 + 4, out[7]);
            write_pixel(&mut out_rgba, target_idx + target_stride * 2 + 8, out[8]);
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_scale4x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    // Scale4x is Scale2x applied twice
    let (w2, h2, r2) = apply_scale2x(width, height, rgba);
    apply_scale2x(w2, h2, &r2)
}

#[inline]
fn read_pixel(rgba: &[u8], idx: usize) -> [u8; 4] {
    [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
}

#[inline]
fn write_pixel(rgba: &mut [u8], idx: usize, pixel: [u8; 4]) {
    rgba[idx..idx + 4].copy_from_slice(&pixel);
}
