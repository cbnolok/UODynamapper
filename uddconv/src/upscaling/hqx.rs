//! HQx (High Quality upscaler) algorithms.
//!
//! Reference: https://github.com/brunexgeek/hqx
//! Original by Maxim Stepin.
//! Logic: Pattern matching in a 3x3 window with thresholded color distance.

pub fn apply_hqx(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    match scale {
        2 => apply_hq2x(width, height, rgba),
        3 => apply_hq3x(width, height, rgba),
        4 => apply_hq4x(width, height, rgba),
        _ => (width, height, rgba.to_vec()),
    }
}

fn apply_hq2x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
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

    let is_diff = |c1: [u8; 4], c2: [u8; 4]| -> bool {
        let dr = (c1[0] as i32 - c2[0] as i32).abs();
        let dg = (c1[1] as i32 - c2[1] as i32).abs();
        let db = (c1[2] as i32 - c2[2] as i32).abs();
        (dr + dg + db) > 48
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            /*
                P1 P2 P3
                P4 P5 P6
                P7 P8 P9
            */
            let p5 = get_p(x, y);
            let p1 = get_p(x - 1, y - 1);
            let p2 = get_p(x, y - 1);
            let p3 = get_p(x + 1, y - 1);
            let p4 = get_p(x - 1, y);
            let p6 = get_p(x + 1, y);
            let p7 = get_p(x - 1, y + 1);
            let p8 = get_p(x, y + 1);
            let p9 = get_p(x + 1, y + 1);

            let mut out = [p5, p5, p5, p5];

            // Simplified HQ2x decision logic
            if is_diff(p4, p6) && is_diff(p2, p8) {
                if !is_diff(p4, p2) {
                    out[0] = interpolate(p4, p2);
                }
                if !is_diff(p2, p6) {
                    out[1] = interpolate(p2, p6);
                }
                if !is_diff(p4, p8) {
                    out[2] = interpolate(p4, p8);
                }
                if !is_diff(p8, p6) {
                    out[3] = interpolate(p8, p6);
                }
            }

            // Add more specific patterns if needed, but this is the core of hq2x:
            // smoothing edges based on 4-neighbor similarity.

            set_p(x as u32 * 2, y as u32 * 2, out[0]);
            set_p(x as u32 * 2 + 1, y as u32 * 2, out[1]);
            set_p(x as u32 * 2, y as u32 * 2 + 1, out[2]);
            set_p(x as u32 * 2 + 1, y as u32 * 2 + 1, out[3]);
        }
    }

    (target_width, target_height, out_rgba)
}

fn apply_hq3x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    // 3x implementation follows same pattern matching principle
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

    let is_diff = |c1: [u8; 4], c2: [u8; 4]| -> bool {
        let dr = (c1[0] as i32 - c2[0] as i32).abs();
        let dg = (c1[1] as i32 - c2[1] as i32).abs();
        let db = (c1[2] as i32 - c2[2] as i32).abs();
        (dr + dg + db) > 48
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let p5 = get_p(x, y);
            let p2 = get_p(x, y - 1);
            let p4 = get_p(x - 1, y);
            let p6 = get_p(x + 1, y);
            let p8 = get_p(x, y + 1);

            let mut out = [p5; 9];

            if is_diff(p4, p6) && is_diff(p2, p8) {
                if !is_diff(p4, p2) {
                    out[0] = interpolate(p4, p2);
                    out[1] = interpolate(p4, p2);
                    out[3] = interpolate(p4, p2);
                }
                if !is_diff(p2, p6) {
                    out[2] = interpolate(p2, p6);
                    out[1] = interpolate(p2, p6);
                    out[5] = interpolate(p2, p6);
                }
                if !is_diff(p4, p8) {
                    out[6] = interpolate(p4, p8);
                    out[3] = interpolate(p4, p8);
                    out[7] = interpolate(p4, p8);
                }
                if !is_diff(p8, p6) {
                    out[8] = interpolate(p8, p6);
                    out[5] = interpolate(p8, p6);
                    out[7] = interpolate(p8, p6);
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

fn apply_hq4x(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w2, h2, r2) = apply_hq2x(width, height, rgba);
    apply_hq2x(w2, h2, &r2)
}

fn get_p_static(x: i32, y: i32, w: u32, h: u32, rgba: &[u8]) -> [u8; 4] {
    let px = x.clamp(0, w as i32 - 1) as usize;
    let py = y.clamp(0, h as i32 - 1) as usize;
    let idx = (py * w as usize + px) * 4;
    [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
}

fn interpolate(c1: [u8; 4], c2: [u8; 4]) -> [u8; 4] {
    [
        ((c1[0] as u16 + c2[0] as u16) >> 1) as u8,
        ((c1[1] as u16 + c2[1] as u16) >> 1) as u8,
        ((c1[2] as u16 + c2[2] as u16) >> 1) as u8,
        ((c1[3] as u16 + c2[3] as u16) >> 1) as u8,
    ]
}
