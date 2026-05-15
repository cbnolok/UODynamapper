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
            let b0 = get_pixel(x - 1, y - 1);
            let b1 = get_pixel(x, y - 1);
            let b2 = get_pixel(x + 1, y - 1);
            let b3 = get_pixel(x + 2, y - 1);
            
            let c4 = get_pixel(x - 1, y);
            let c5 = get_pixel(x, y);
            let c6 = get_pixel(x + 1, y);
            let s2 = get_pixel(x + 2, y);
            
            let c1 = get_pixel(x - 1, y + 1);
            let c2 = get_pixel(x, y + 1);
            let c3 = get_pixel(x + 1, y + 1);
            let s1 = get_pixel(x + 2, y + 1);
            
            let a0 = get_pixel(x - 1, y + 2);
            let a1 = get_pixel(x, y + 2);
            let a2 = get_pixel(x + 1, y + 2);
            let a3 = get_pixel(x + 2, y + 2);

            let (p0, p1, p2, p3) = match filter {
                UpscaleFilter::TwoSai2x => apply_2xsai_logic(
                    c5, c6, c2, c3, b1, b2, c4, c1, b0, b3, s2, s1, a0, a1, a2
                ),
                UpscaleFilter::SuperSai2x => apply_super2xsai_logic(
                    b0, b1, b2, b3, c4, c5, c6, s2, c1, c2, c3, s1, a0, a1, a2, a3
                ),
                UpscaleFilter::SuperEagle2x => apply_supereagle_logic(
                    c5, c6, c2, c3, b1, b2, c4, c1, s2, s1, a1, a2
                ),
                _ => (c5, c5, c5, c5),
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
    let p1;
    let p2;
    let p3;

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

fn apply_super2xsai_logic(
    colorb0: u32, colorb1: u32, colorb2: u32, colorb3: u32,
    color4: u32, color5: u32, color6: u32, colors2: u32,
    color1: u32, color2: u32, color3: u32, colors1: u32,
    colora0: u32, colora1: u32, colora2: u32, colora3: u32
) -> (u32, u32, u32, u32) {
    let product1a;
    let product1b;
    let product2a;
    let product2b;

    if color2 == color6 && color5 != color3 {
        product2b = color2;
        product1b = color2;
    } else if color5 == color3 && color2 != color6 {
        product2b = color5;
        product1b = color5;
    } else if color5 == color3 && color2 == color6 {
        let mut r = 0;
        r += result_cb(color6, color5, color1, colora1);
        r += result_cb(color6, color5, color4, colorb1);
        r += result_cb(color6, color5, colora2, colors1);
        r += result_cb(color6, color5, colorb2, colors2);
        if r > 0 {
            product2b = color6;
            product1b = color6;
        } else if r < 0 {
            product2b = color5;
            product1b = color5;
        } else {
            product2b = interpolate(color5, color6);
            product1b = interpolate(color5, color6);
        }
    } else {
        if color6 == color3 && color3 == colora1 && color2 != colora2 && color3 != colora0 {
            product2b = interpolate2(color3, color3, color3, color2);
        } else if color5 == color2 && color2 == colora2 && colora1 != color3 && color2 != colora3 {
            product2b = interpolate2(color2, color2, color2, color3);
        } else {
            product2b = interpolate(color2, color3);
        }
        
        if color6 == color3 && color6 == colorb1 && color5 != colorb2 && color6 != colorb0 {
            product1b = interpolate2(color6, color6, color6, color5);
        } else if color5 == color2 && color5 == colorb2 && colorb1 != color6 && color5 != colorb3 {
            product1b = interpolate2(color6, color5, color5, color5);
        } else {
            product1b = interpolate(color5, color6);
        }
    }

    if color5 == color3 && color2 != color6 && color4 == color5 && color5 != colora2 {
        product2a = interpolate(color2, color5);
    } else if color5 == color1 && color6 == color5 && color4 != color2 && color5 != colora0 {
        product2a = interpolate(color2, color5);
    } else {
        product2a = color2;
    }

    if color2 == color6 && color5 != color3 && color1 == color2 && color2 != colorb2 {
        product1a = interpolate(color2, color5);
    } else if color4 == color2 && color3 == color2 && color1 != color5 && color2 != colorb0 {
        product1a = interpolate(color2, color5);
    } else {
        product1a = color5;
    }

    (product1a, product1b, product2a, product2b)
}

fn apply_supereagle_logic(
    c_a: u32, c_b: u32, c_c: u32, c_d: u32, 
    c_e: u32, c_f: u32, c_g: u32, c_h: u32, 
    c_k: u32, c_l: u32, c_n: u32, c_o: u32
) -> (u32, u32, u32, u32) {
    let color4 = c_g;
    let color5 = c_a;
    let color6 = c_b;
    let colors2 = c_k;
    let color1 = c_h;
    let color2 = c_c;
    let color3 = c_d;
    let colors1 = c_l;
    let colora1 = c_n;
    let colora2 = c_o;
    let colorb1 = c_e;
    let colorb2 = c_f;

    let mut product1a;
    let mut product1b;
    let mut product2a;
    let mut product2b;

    if color2 == color6 && color5 != color3 {
        product1b = color2;
        product2a = color2;
        if color1 == color2 || color6 == colorb2 {
            let t = interpolate(color2, color5);
            product1a = interpolate(color2, t);
        } else {
            product1a = interpolate(color5, color6);
        }
        if color6 == colors2 || color2 == colora1 {
            let t = interpolate(color2, color3);
            product2b = interpolate(color2, t);
        } else {
            product2b = interpolate(color2, color3);
        }
    } else if color5 == color3 && color2 != color6 {
        product2b = color5;
        product1a = color5;
        if colorb1 == color5 || color3 == colors1 {
            let t = interpolate(color5, color6);
            product1b = interpolate(color5, t);
        } else {
            product1b = interpolate(color5, color6);
        }
        if color3 == colora2 || color4 == color5 {
            let t = interpolate(color5, color2);
            product2a = interpolate(color5, t);
        } else {
            product2a = interpolate(color2, color3);
        }
    } else if color5 == color3 && color2 == color6 {
        let mut r = 0;
        r += result_cb(color6, color5, color1, colora1);
        r += result_cb(color6, color5, color4, colorb1);
        r += result_cb(color6, color5, colora2, colors1);
        r += result_cb(color6, color5, colorb2, colors2);

        if r > 0 {
            product1b = color2;
            product2a = color2;
            product1a = interpolate(color5, color6);
            product2b = interpolate(color5, color6);
        } else if r < 0 {
            product2b = color5;
            product1a = color5;
            product1b = interpolate(color5, color6);
            product2a = interpolate(color5, color6);
        } else {
            product2b = color5;
            product1a = color5;
            product1b = color2;
            product2a = color2;
        }
    } else {
        let t1 = interpolate(color2, color6);
        product2b = t1;
        product1a = t1;
        product2b = interpolate2(color3, color3, color3, product2b);
        product1a = interpolate2(color5, color5, color5, product1a);
        
        let t2 = interpolate(color5, color3);
        product2a = t2;
        product1b = t2;
        product2a = interpolate2(color2, color2, color2, product2a);
        product1b = interpolate2(color6, color6, color6, product1b);
    }

    (product1a, product1b, product2a, product2b)
}
