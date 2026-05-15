//! LQ2x, LQ3x, LQ4x (Low Quality) upscaling algorithms.
//!
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c
//! Original by Derek Liauw Kie Fa.

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
    let target_width = width * 2;

    let get_pixel = |x: i32, y: i32| -> [u8; 4] {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    };

    let mut set_pixel = |tx: u32, ty: u32, color: [u8; 4]| {
        let idx = (ty * target_width + tx) as usize * 4;
        out_rgba[idx..idx + 4].copy_from_slice(&color);
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let a = get_pixel(x, y);
            let b = get_pixel(x + 1, y);
            let c = get_pixel(x, y + 1);
            let d = get_pixel(x + 1, y + 1);

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

            set_pixel(x as u32 * 2, y as u32 * 2, p0);
            set_pixel(x as u32 * 2 + 1, y as u32 * 2, p1);
            set_pixel(x as u32 * 2, y as u32 * 2 + 1, p2);
            set_pixel(x as u32 * 2 + 1, y as u32 * 2 + 1, p3);
        }
    }
}

fn apply_lq3x(width: u32, height: u32, rgba: &[u8], out_rgba: &mut [u8]) {
    // Simple 3x expansion with bilinear-like edges
    let target_width = width * 3;
    let _target_height = height * 3;

    let get_pixel = |x: i32, y: i32| -> [u8; 4] {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    };

    let mut set_pixel = |tx: u32, ty: u32, color: [u8; 4]| {
        let idx = (ty * target_width + tx) as usize * 4;
        out_rgba[idx..idx + 4].copy_from_slice(&color);
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let p_e = get_pixel(x, y);
            let p_b = get_pixel(x, y - 1);
            let p_d = get_pixel(x - 1, y);
            let p_f = get_pixel(x + 1, y);
            let p_h = get_pixel(x, y + 1);

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

            for ty in 0..3 {
                for tx in 0..3 {
                    set_pixel(
                        x as u32 * 3 + tx,
                        y as u32 * 3 + ty,
                        out[(ty * 3 + tx) as usize],
                    );
                }
            }
        }
    }
}

fn apply_lq4x(width: u32, height: u32, rgba: &[u8], out_rgba: &mut [u8]) {
    let (w2, h2, r2) = apply_lq(width, height, rgba, 2);
    let (_, _, r4) = apply_lq(w2, h2, &r2, 2);
    out_rgba.copy_from_slice(&r4);
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
