//! LQ2x, LQ3x, LQ4x (Low Quality) upscaling algorithms.
//!
//! Original by Derek Liauw Kie Fa.
//! Reference: https://github.com/daelsepara/PixelScalerWin/
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c

pub fn apply_lq(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    let target_width = width * scale;
    let target_height = height * scale;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    match scale {
        2 => apply_lq2x(width, height, rgba, &mut out_rgba),
        3 => apply_lq3x(width, height, rgba, &mut out_rgba),
        4 => apply_lq4x(width, height, rgba, &mut out_rgba),
        _ => {
            // Fallback for other scales
            use image::imageops::{self, FilterType};
            use image::{ImageBuffer, Rgba};
            let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
            let upscaled =
                imageops::resize(&img, target_width, target_height, FilterType::CatmullRom);
            out_rgba.copy_from_slice(&upscaled.into_raw());
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_lq2x(width: u32, height: u32, rgba: &[u8], out_rgba: &mut [u8]) {
    let width = width as usize;
    let height = height as usize;
    let target_width = width * 2;

    for y in 0..height {
        let y_next = if y + 1 < height { y + 1 } else { y };

        for x in 0..width {
            let x_next = if x + 1 < width { x + 1 } else { x };

            let a = get_pixel(rgba, width, x, y);
            let b = get_pixel(rgba, width, x_next, y);
            let c = get_pixel(rgba, width, x, y_next);
            let d = get_pixel(rgba, width, x_next, y_next);

            let p0 = a;
            let p1;
            let p2;
            let p3;

            if a != d && b != c {
                p1 = interpolate(a, b);
                p2 = interpolate(a, c);
                p3 = interpolate2(a, b, c, d);
            } else if a == d && b == c {
                if a == b {
                    p1 = a;
                    p2 = a;
                    p3 = a;
                } else {
                    p1 = interpolate(a, b);
                    p2 = interpolate(a, c);
                    p3 = interpolate2(a, b, c, d);
                }
            } else if a == d {
                p1 = interpolate(a, b);
                p2 = interpolate(a, c);
                p3 = a;
            } else {
                p1 = b;
                p2 = c;
                p3 = b;
            }

            let tx = x * 2;
            let ty = y * 2;
            set_pixel(out_rgba, target_width, tx, ty, p0);
            set_pixel(out_rgba, target_width, tx + 1, ty, p1);
            set_pixel(out_rgba, target_width, tx, ty + 1, p2);
            set_pixel(out_rgba, target_width, tx + 1, ty + 1, p3);
        }
    }
}

fn apply_lq3x(width: u32, height: u32, rgba: &[u8], out_rgba: &mut [u8]) {
    // Simple 3x expansion with bilinear-like edges
    let width = width as usize;
    let height = height as usize;
    let target_width = width * 3;

    for y in 0..height {
        let y_prev = y.saturating_sub(1);
        let y_next = if y + 1 < height { y + 1 } else { y };

        for x in 0..width {
            let x_prev = x.saturating_sub(1);
            let x_next = if x + 1 < width { x + 1 } else { x };

            let p_e = get_pixel(rgba, width, x, y);
            let p_b = get_pixel(rgba, width, x, y_prev);
            let p_d = get_pixel(rgba, width, x_prev, y);
            let p_f = get_pixel(rgba, width, x_next, y);
            let p_h = get_pixel(rgba, width, x, y_next);

            let mut out = [p_e; 9];

            if p_b != p_h && p_d != p_f {
                if p_d == p_b {
                    out[0] = p_d;
                }
                if p_d == p_b || p_b == p_f {
                    out[1] = p_b;
                }
                if p_b == p_f {
                    out[2] = p_f;
                }
                if p_d == p_b || p_d == p_h {
                    out[3] = p_d;
                }
                // out[4] is p_e
                if p_b == p_f || p_f == p_h {
                    out[5] = p_f;
                }
                if p_d == p_h {
                    out[6] = p_d;
                }
                if p_d == p_h || p_f == p_h {
                    out[7] = p_h;
                }
                if p_f == p_h {
                    out[8] = p_f;
                }
            }

            let base_tx = x * 3;
            let base_ty = y * 3;
            for ty in 0..3 {
                for tx in 0..3 {
                    set_pixel(
                        out_rgba,
                        target_width,
                        base_tx + tx,
                        base_ty + ty,
                        out[ty * 3 + tx],
                    );
                }
            }
        }
    }
}

fn apply_lq4x(width: u32, height: u32, rgba: &[u8], out_rgba: &mut [u8]) {
    let w2 = width * 2;
    let h2 = height * 2;
    let mut r2 = vec![0u8; (w2 * h2 * 4) as usize];
    apply_lq2x(width, height, rgba, &mut r2);
    apply_lq2x(w2, h2, &r2, out_rgba);
}

#[inline]
fn get_pixel(rgba: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let idx = (y * width + x) * 4;
    [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
}

#[inline]
fn set_pixel(out_rgba: &mut [u8], target_width: usize, x: usize, y: usize, color: [u8; 4]) {
    let idx = (y * target_width + x) * 4;
    out_rgba[idx..idx + 4].copy_from_slice(&color);
}

fn interpolate(c1: [u8; 4], c2: [u8; 4]) -> [u8; 4] {
    [
        ((c1[0] as u16 + c2[0] as u16) >> 1) as u8,
        ((c1[1] as u16 + c2[1] as u16) >> 1) as u8,
        ((c1[2] as u16 + c2[2] as u16) >> 1) as u8,
        ((c1[3] as u16 + c2[3] as u16) >> 1) as u8,
    ]
}

fn interpolate2(c1: [u8; 4], c2: [u8; 4], c3: [u8; 4], c4: [u8; 4]) -> [u8; 4] {
    [
        ((c1[0] as u16 + c2[0] as u16 + c3[0] as u16 + c4[0] as u16) >> 2) as u8,
        ((c1[1] as u16 + c2[1] as u16 + c3[1] as u16 + c4[1] as u16) >> 2) as u8,
        ((c1[2] as u16 + c2[2] as u16 + c3[2] as u16 + c4[2] as u16) >> 2) as u8,
        ((c1[3] as u16 + c2[3] as u16 + c3[3] as u16 + c4[3] as u16) >> 2) as u8,
    ]
}
