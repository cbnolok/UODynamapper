//! EPX (Eric's Pixel Expansion) / Scale2x upscaling algorithm.
//!
//! Reference: https://en.wikipedia.org/wiki/Pixel-art_scaling_algorithms#Scale2x
//! Original by Eric Johnston at LucasArts.

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

    let get_p = |x: i32, y: i32| -> [u8; 4] {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    };

    let mut set_p = |tx: u32, ty: u32, color: [u8; 4]| {
        let idx = (ty * target_width + tx) as usize * 4;
        out_rgba[idx..idx + 4].copy_from_slice(&color);
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            /*
                A B C
                D E F
                G H I
            */
            let p_e = get_p(x, y);
            let p_b = get_p(x, y - 1);
            let p_d = get_p(x - 1, y);
            let p_f = get_p(x + 1, y);
            let p_h = get_p(x, y + 1);

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

            set_p(x as u32 * 2, y as u32 * 2, p1);
            set_p(x as u32 * 2 + 1, y as u32 * 2, p2);
            set_p(x as u32 * 2, y as u32 * 2 + 1, p3);
            set_p(x as u32 * 2 + 1, y as u32 * 2 + 1, p4);
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_scale3x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let target_width = width * 3;
    let target_height = height * 3;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    let get_p = |x: i32, y: i32| -> [u8; 4] {
        let px = x.clamp(0, width as i32 - 1) as usize;
        let py = y.clamp(0, height as i32 - 1) as usize;
        let idx = (py * width as usize + px) * 4;
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    };

    let mut set_p = |tx: u32, ty: u32, color: [u8; 4]| {
        let idx = (ty * target_width + tx) as usize * 4;
        out_rgba[idx..idx + 4].copy_from_slice(&color);
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let p_a = get_p(x - 1, y - 1);
            let p_b = get_p(x, y - 1);
            let p_c = get_p(x + 1, y - 1);
            let p_d = get_p(x - 1, y);
            let p_e = get_p(x, y);
            let p_f = get_p(x + 1, y);
            let p_g = get_p(x - 1, y + 1);
            let p_h = get_p(x, y + 1);
            let p_i = get_p(x + 1, y + 1);

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

            for ty in 0..3 {
                for tx in 0..3 {
                    set_p(
                        x as u32 * 3 + tx,
                        y as u32 * 3 + ty,
                        out[(ty * 3 + tx) as usize],
                    );
                }
            }
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_scale4x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    // Scale4x is Scale2x applied twice
    let (w2, h2, r2) = apply_scale2x(width, height, rgba);
    apply_scale2x(w2, h2, &r2)
}
