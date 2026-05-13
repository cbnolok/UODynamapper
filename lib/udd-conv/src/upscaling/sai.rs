//! 2xSaI, Super2xSaI, and SuperEagle upscaling algorithms.
//!
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/2xsai.c
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/super2xsai.c
//! Reference: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/supereagle.c
//! Original by Derek Liauw Kie Fa.

use super::UpscaleFilter;

pub fn apply_sai(
    width: u32,
    height: u32,
    rgba: &[u8],
    _scale: u32,
    filter: UpscaleFilter,
) -> (u32, u32, Vec<u8>) {
    let target_width = width * 2;
    let target_height = height * 2;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    let get_pixel = |x: i32, y: i32| -> u32 {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        // Store as 0xAARRGGBB for easier processing
        ((rgba[idx + 3] as u32) << 24) | ((rgba[idx] as u32) << 16) | ((rgba[idx + 1] as u32) << 8) | (rgba[idx + 2] as u32)
    };

    let mut set_pixel = |tx: u32, ty: u32, val: u32| {
        let idx = (ty * target_width + tx) as usize * 4;
        out_rgba[idx] = ((val >> 16) & 0xFF) as u8;
        out_rgba[idx + 1] = ((val >> 8) & 0xFF) as u8;
        out_rgba[idx + 2] = (val & 0xFF) as u8;
        out_rgba[idx + 3] = ((val >> 24) & 0xFF) as u8;
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let c_i = get_pixel(x - 1, y - 1);
            let c_e = get_pixel(x, y - 1);
            let c_f = get_pixel(x + 1, y - 1);
            let c_j = get_pixel(x + 2, y - 1);
            
            let c_g = get_pixel(x - 1, y);
            let c_a = get_pixel(x, y);
            let c_b = get_pixel(x + 1, y);
            let c_k = get_pixel(x + 2, y);
            
            let c_h = get_pixel(x - 1, y + 1);
            let c_c = get_pixel(x, y + 1);
            let c_d = get_pixel(x + 1, y + 1);
            let c_l = get_pixel(x + 2, y + 1);
            
            let c_m = get_pixel(x - 1, y + 2);
            let c_n = get_pixel(x, y + 2);
            let c_o = get_pixel(x + 1, y + 2);

            let (p0, p1, p2, p3) = match filter {
                UpscaleFilter::TwoSai => apply_2xsai_logic(c_a, c_b, c_c, c_d, c_e, c_f, c_g, c_h, c_i, c_j, c_k, c_l, c_m, c_n, c_o),
                UpscaleFilter::SuperSai => apply_super2xsai_logic(c_a, c_b, c_c, c_d, c_e, c_f, c_g, c_h, c_i, c_j, c_k, c_l),
                UpscaleFilter::SuperEagle => apply_supereagle_logic(c_a, c_b, c_c, c_d),
                _ => (c_a, c_a, c_a, c_a),
            };

            set_pixel(x as u32 * 2, y as u32 * 2, p0);
            set_pixel(x as u32 * 2 + 1, y as u32 * 2, p1);
            set_pixel(x as u32 * 2, y as u32 * 2 + 1, p2);
            set_pixel(x as u32 * 2 + 1, y as u32 * 2 + 1, p3);
        }
    }

    (target_width, target_height, out_rgba)
}

fn interpolate(c1: u32, c2: u32) -> u32 {
    let r = (((c1 >> 16) & 0xFF) + ((c2 >> 16) & 0xFF)) >> 1;
    let g = (((c1 >> 8) & 0xFF) + ((c2 >> 8) & 0xFF)) >> 1;
    let b = ((c1 & 0xFF) + (c2 & 0xFF)) >> 1;
    let a = (((c1 >> 24) & 0xFF) + ((c2 >> 24) & 0xFF)) >> 1;
    (a << 24) | (r << 16) | (g << 8) | b
}

fn interpolate2(c1: u32, c2: u32, c3: u32, c4: u32) -> u32 {
    let r = (((c1 >> 16) & 0xFF) + ((c2 >> 16) & 0xFF) + ((c3 >> 16) & 0xFF) + ((c4 >> 16) & 0xFF)) >> 2;
    let g = (((c1 >> 8) & 0xFF) + ((c2 >> 8) & 0xFF) + ((c3 >> 8) & 0xFF) + ((c4 >> 8) & 0xFF)) >> 2;
    let b = ((c1 & 0xFF) + (c2 & 0xFF) + (c3 & 0xFF) + (c4 & 0xFF)) >> 2;
    let a = (((c1 >> 24) & 0xFF) + ((c2 >> 24) & 0xFF) + ((c3 >> 24) & 0xFF) + ((c4 >> 24) & 0xFF)) >> 2;
    (a << 24) | (r << 16) | (g << 8) | b
}

fn result_cb(a: u32, b: u32, c: u32, d: u32) -> i32 {
    if a == c && a == d && a != b { 1 }
    else if b == c && b == d && b != a { -1 }
    else { 0 }
}

fn apply_2xsai_logic(a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32, j: u32, k: u32, l: u32, m: u32, n: u32, o: u32) -> (u32, u32, u32, u32) {
    let p0 = a;
    let mut p1;
    let mut p2;
    let mut p3;

    if a == d && b != c {
        if (a == e && b == l) || (a == c && a == f && b != e && b == j) {
            p1 = a;
        } else {
            p1 = interpolate(a, b);
        }
        if (a == g && c == o) || (a == b && a == h && g != c && c == m) {
            p2 = a;
        } else {
            p2 = interpolate(a, c);
        }
        p3 = a;
    } else if b == c && a != d {
        if (b == f && a == h) || (b == e && b == d && a != f && a == i) {
            p1 = b;
        } else {
            p1 = interpolate(a, b);
        }
        if (c == h && a == f) || (c == g && c == d && a != h && a == i) {
            p2 = c;
        } else {
            p2 = interpolate(a, c);
        }
        p3 = b;
    } else if a == d && b == c {
        if a == b {
            p1 = a;
            p2 = a;
            p3 = a;
        } else {
            let mut r = 0;
            p1 = interpolate(a, b);
            p2 = interpolate(a, c);
            r += result_cb(a, b, g, e);
            r += result_cb(b, a, k, f);
            r += result_cb(b, a, h, n);
            r += result_cb(a, b, l, o);
            if r > 0 { p3 = a; }
            else if r < 0 { p3 = b; }
            else { p3 = interpolate2(a, b, c, d); }
        }
    } else {
        p3 = interpolate2(a, b, c, d);
        if a == c && a == f && b != e && b == j {
            p1 = a;
        } else if b == e && b == d && a != f && a == i {
            p1 = b;
        } else {
            p1 = interpolate(a, b);
        }
        if a == b && a == h && g != c && c == m {
            p2 = a;
        } else if c == g && c == d && a != h && a == i {
            p2 = c;
        } else {
            p2 = interpolate(a, c);
        }
    }

    (p0, p1, p2, p3)
}

fn apply_super2xsai_logic(a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32, j: u32, k: u32, l: u32) -> (u32, u32, u32, u32) {
    let p0 = a;
    let mut p1;
    let mut p2;
    let mut p3;

    if a == d && b != c {
        if (a == e && b == l) || (a == c && a == f && b != e && b == j) {
            p1 = a;
        } else {
            p1 = interpolate(a, b);
        }
        if (a == g && c == j) || (a == b && a == h && g != c && c == f) { 
             p2 = a;
        } else {
             p2 = interpolate(a, c);
        }
        p3 = interpolate2(a, a, b, c);
    } else if b == c && a != d {
        if (b == f && a == h) || (b == e && b == d && a != f && a == i) {
            p1 = b;
        } else {
            p1 = interpolate(a, b);
        }
        if (c == h && a == f) || (c == g && c == d && a != h && a == i) {
            p2 = c;
        } else {
            p2 = interpolate(a, c);
        }
        p3 = interpolate2(b, b, a, d);
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
    } else {
        p3 = interpolate2(a, b, c, d);
        if a == c && a == f && b != e && b == j {
            p1 = a;
        } else if b == e && b == d && a != f && a == i {
            p1 = b;
        } else {
            p1 = interpolate(a, b);
        }
        if a == b && a == h && g != c && c == f { 
            p2 = a;
        } else if c == g && c == d && a != h && a == i {
            p2 = c;
        } else {
            p2 = interpolate(a, c);
        }
    }

    (p0, p1, p2, p3)
}

fn apply_supereagle_logic(a: u32, b: u32, c: u32, d: u32) -> (u32, u32, u32, u32) {
    if a == d && b == c {
        if a == b {
            (a, a, a, a)
        } else {
            (a, interpolate(a, b), interpolate(a, c), interpolate2(a, b, c, d))
        }
    } else if a == d {
        (a, interpolate(a, b), interpolate(a, c), a)
    } else if b == c {
        (a, b, c, b)
    } else {
        (a, interpolate(a, b), interpolate(a, c), interpolate2(a, b, c, d))
    }
}
