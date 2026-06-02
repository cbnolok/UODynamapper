//! NEDI (New Edge-Directed Interpolation) upscaling algorithm.
//!
//! Reference: https://en.wikipedia.org/wiki/Edge-directed_interpolation
//! Reference: Li, X., & Orchard, M. T. (2001). New edge-directed interpolation. IEEE Transactions on Image Processing.

pub fn apply_nedi(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let target_width = width * 2;
    let target_height = height * 2;
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];

    // Pass 1: Fill the center of 2x2 blocks (the (1,1) positions in target)
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let p00 = get_p(x, y, width, height, rgba);
            set_tp(x as u32 * 2, y as u32 * 2, target_width, &mut out_rgba, p00);

            let p10 = get_p(x + 1, y, width, height, rgba);
            let p01 = get_p(x, y + 1, width, height, rgba);
            let p11 = get_p(x + 1, y + 1, width, height, rgba);

            let grad1 = diff(&p00, &p11);
            let grad2 = diff(&p10, &p01);

            let p_center = if grad1 < grad2 {
                interpolate(&p00, &p11)
            } else if grad2 < grad1 {
                interpolate(&p10, &p01)
            } else {
                interpolate4(&p00, &p10, &p01, &p11)
            };
            set_tp(x as u32 * 2 + 1, y as u32 * 2 + 1, target_width, &mut out_rgba, p_center);
        }
    }

    // Pass 2: Fill the remaining pixels (2x+1, 2y) and (2x, 2y+1) using cross neighbors
    for y in 0..height {
        let ty = y * 2;
        for x in 0..width {
            let tx = x * 2;
            set_missing_tp(tx + 1, ty, target_width, target_height, &mut out_rgba);
            set_missing_tp(tx, ty + 1, target_width, target_height, &mut out_rgba);
        }
    }

    (target_width, target_height, out_rgba)
}

fn set_missing_tp(tx: u32, ty: u32, tw: u32, th: u32, rgba: &mut [u8]) {
    let x = tx as i32;
    let y = ty as i32;
    let p_up = get_tp_f(x, y - 1, tw, th, rgba);
    let p_down = get_tp_f(x, y + 1, tw, th, rgba);
    let p_left = get_tp_f(x - 1, y, tw, th, rgba);
    let p_right = get_tp_f(x + 1, y, tw, th, rgba);

    let grad_v = diff(&p_up, &p_down);
    let grad_h = diff(&p_left, &p_right);

    let p_final = if grad_v < grad_h {
        interpolate(&p_up, &p_down)
    } else if grad_h < grad_v {
        interpolate(&p_left, &p_right)
    } else {
        interpolate4(&p_up, &p_down, &p_left, &p_right)
    };

    let idx = (ty as usize * tw as usize + tx as usize) * 4;
    rgba[idx] = p_final[0].clamp(0.0, 255.0) as u8;
    rgba[idx + 1] = p_final[1].clamp(0.0, 255.0) as u8;
    rgba[idx + 2] = p_final[2].clamp(0.0, 255.0) as u8;
    rgba[idx + 3] = p_final[3].clamp(0.0, 255.0) as u8;
}

fn get_p(x: i32, y: i32, w: u32, h: u32, rgba: &[u8]) -> [f32; 4] {
    let px = x.clamp(0, w as i32 - 1) as usize;
    let py = y.clamp(0, h as i32 - 1) as usize;
    let idx = (py * w as usize + px) * 4;
    [
        rgba[idx] as f32,
        rgba[idx + 1] as f32,
        rgba[idx + 2] as f32,
        rgba[idx + 3] as f32,
    ]
}

fn set_tp(tx: u32, ty: u32, tw: u32, rgba: &mut [u8], color: [f32; 4]) {
    let idx = (ty as usize * tw as usize + tx as usize) * 4;
    rgba[idx] = color[0].clamp(0.0, 255.0) as u8;
    rgba[idx + 1] = color[1].clamp(0.0, 255.0) as u8;
    rgba[idx + 2] = color[2].clamp(0.0, 255.0) as u8;
    rgba[idx + 3] = color[3].clamp(0.0, 255.0) as u8;
}

fn diff(c1: &[f32; 4], c2: &[f32; 4]) -> f32 {
    (c1[0] - c2[0]).abs() + (c1[1] - c2[1]).abs() + (c1[2] - c2[2]).abs()
}

fn interpolate(c1: &[f32; 4], c2: &[f32; 4]) -> [f32; 4] {
    [
        (c1[0] + c2[0]) * 0.5,
        (c1[1] + c2[1]) * 0.5,
        (c1[2] + c2[2]) * 0.5,
        (c1[3] + c2[3]) * 0.5,
    ]
}

fn interpolate4(c1: &[f32; 4], c2: &[f32; 4], c3: &[f32; 4], c4: &[f32; 4]) -> [f32; 4] {
    [
        (c1[0] + c2[0] + c3[0] + c4[0]) * 0.25,
        (c1[1] + c2[1] + c3[1] + c4[1]) * 0.25,
        (c1[2] + c2[2] + c3[2] + c4[2]) * 0.25,
        (c1[3] + c2[3] + c3[3] + c4[3]) * 0.25,
    ]
}

fn get_tp_f(x: i32, y: i32, w: u32, h: u32, rgba: &[u8]) -> [f32; 4] {
    let px = x.clamp(0, w as i32 - 1) as usize;
    let py = y.clamp(0, h as i32 - 1) as usize;
    let idx = (py * w as usize + px) * 4;
    [
        rgba[idx] as f32,
        rgba[idx + 1] as f32,
        rgba[idx + 2] as f32,
        rgba[idx + 3] as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nedi_preserves_expected_output_size() {
        let rgba = vec![
            0, 0, 0, 255,
            255, 0, 0, 255,
            0, 255, 0, 255,
            0, 0, 255, 255,
        ];

        let (width, height, out) = apply_nedi(2, 2, &rgba);

        assert_eq!((width, height), (4, 4));
        assert_eq!(out.len(), 4 * 4 * 4);
    }

    #[test]
    fn nedi_single_pixel_keeps_source_corners() {
        let rgba = vec![12, 34, 56, 78];

        let (width, height, out) = apply_nedi(1, 1, &rgba);

        assert_eq!((width, height), (2, 2));
        assert_eq!(&out[0..4], rgba.as_slice());
        assert_eq!(&out[12..16], rgba.as_slice());
    }
}
