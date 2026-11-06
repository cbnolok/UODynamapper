//! Super-xBR upscaling algorithm.
//!
//! Reference: https://github.com/hansonw/super-xbr
//! Original algorithm by Hyllian, MIT licensed.

const WGT1: f64 = 0.129633;
const WGT2: f64 = 0.175068;
const W1: f64 = -WGT1;
const W2: f64 = WGT1 + 0.5;
const W3: f64 = -WGT2;
const W4: f64 = WGT2 + 0.5;
const EDGE_WEIGHTS_STRONG: [f64; 6] = [2.0, 1.0, -1.0, 4.0, -1.0, 1.0];
const EDGE_WEIGHTS_SIMPLE: [f64; 6] = [2.0, 0.0, 0.0, 0.0, 0.0, 0.0];

#[derive(Clone, Copy)]
struct Matrices {
    r: [[u8; 4]; 4],
    g: [[u8; 4]; 4],
    b: [[u8; 4]; 4],
    a: [[u8; 4]; 4],
    y: [[f64; 4]; 4],
}

impl Matrices {
    fn new() -> Self {
        Self {
            r: [[0; 4]; 4],
            g: [[0; 4]; 4],
            b: [[0; 4]; 4],
            a: [[0; 4]; 4],
            y: [[0.0; 4]; 4],
        }
    }

    fn set(&mut self, x: usize, y: usize, pixel: u32) {
        let r = (pixel & 0xff) as u8;
        let g = ((pixel >> 8) & 0xff) as u8;
        let b = ((pixel >> 16) & 0xff) as u8;
        let a = ((pixel >> 24) & 0xff) as u8;
        self.r[x][y] = r;
        self.g[x][y] = g;
        self.b[x][y] = b;
        self.a[x][y] = a;
        self.y[x][y] = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
    }
}

pub fn apply_super_xbr(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let out_width = width * 2;
    let out_height = height * 2;
    let input = rgba_to_u32(rgba);
    let mut out = vec![0; (out_width * out_height) as usize];
    let mut mat = Matrices::new();

    first_pass(width, height, out_width, &input, &mut out, &mut mat);
    second_pass(out_width, out_height, &mut out, &mut mat);
    third_pass(out_width, out_height, &mut out, &mut mat);

    (out_width, out_height, u32_to_rgba(&out))
}

fn first_pass(
    width: u32,
    height: u32,
    out_width: u32,
    input: &[u32],
    out: &mut [u32],
    mat: &mut Matrices,
) {
    let mut y = 0;
    while y < height * 2 {
        let mut x = 0;
        while x < width * 2 {
            let cx = x / 2;
            let cy = y / 2;
            for sx in -1..=2 {
                for sy in -1..=2 {
                    let csy = clamp_i32(sy + cy as i32, 0, height as i32 - 1) as u32;
                    let csx = clamp_i32(sx + cx as i32, 0, width as i32 - 1) as u32;
                    mat.set(
                        (sx + 1) as usize,
                        (sy + 1) as usize,
                        input[(csy * width + csx) as usize],
                    );
                }
            }

            let (min_sample, max_sample) = sample_bounds(mat);
            let filtered = filtered_pixel(mat, &EDGE_WEIGHTS_STRONG, W1, W2, min_sample, max_sample);
            let source = input[(cy * width + cx) as usize];
            out[(y * out_width + x) as usize] = source;
            out[(y * out_width + x + 1) as usize] = source;
            out[((y + 1) * out_width + x) as usize] = source;
            out[((y + 1) * out_width + x + 1) as usize] = filtered;

            x += 2;
        }
        y += 2;
    }
}

fn second_pass(out_width: u32, out_height: u32, out: &mut [u32], mat: &mut Matrices) {
    let mut y = 0;
    while y < out_height {
        let mut x = 0;
        while x < out_width {
            sample_rotated(out_width, out_height, out, mat, x as i32, y as i32, 0, 0);
            let (min_sample, max_sample) = sample_bounds(mat);
            out[(y * out_width + x + 1) as usize] =
                filtered_pixel(mat, &EDGE_WEIGHTS_SIMPLE, W3, W4, min_sample, max_sample);

            sample_rotated(out_width, out_height, out, mat, x as i32, y as i32, -1, 1);
            out[((y + 1) * out_width + x) as usize] =
                filtered_pixel(mat, &EDGE_WEIGHTS_SIMPLE, W3, W4, min_sample, max_sample);

            x += 2;
        }
        y += 2;
    }
}

fn third_pass(out_width: u32, out_height: u32, out: &mut [u32], mat: &mut Matrices) {
    for y in (0..out_height).rev() {
        for x in (0..out_width).rev() {
            for sx in -2..=1 {
                for sy in -2..=1 {
                    let csy = clamp_i32(sy + y as i32, 0, out_height as i32 - 1) as u32;
                    let csx = clamp_i32(sx + x as i32, 0, out_width as i32 - 1) as u32;
                    mat.set(
                        (sx + 2) as usize,
                        (sy + 2) as usize,
                        out[(csy * out_width + csx) as usize],
                    );
                }
            }

            let (min_sample, max_sample) = sample_bounds(mat);
            out[(y * out_width + x) as usize] =
                filtered_pixel(mat, &EDGE_WEIGHTS_STRONG, W1, W2, min_sample, max_sample);
        }
    }
}

fn sample_rotated(
    out_width: u32,
    out_height: u32,
    out: &[u32],
    mat: &mut Matrices,
    x: i32,
    y: i32,
    x_bias: i32,
    y_bias: i32,
) {
    for sx in -1..=2 {
        for sy in -1..=2 {
            let csy = clamp_i32(sx - sy + y_bias + y, 0, out_height as i32 - 1) as u32;
            let csx = clamp_i32(sx + sy + x_bias + x, 0, out_width as i32 - 1) as u32;
            mat.set((sx + 1) as usize, (sy + 1) as usize, out[(csy * out_width + csx) as usize]);
        }
    }
}

fn filtered_pixel(
    mat: &Matrices,
    edge_weights: &[f64; 6],
    filter_weight_1: f64,
    filter_weight_2: f64,
    min_sample: [u8; 4],
    max_sample: [u8; 4],
) -> u32 {
    let diagonal_edge = diagonal_edge(&mat.y, edge_weights);
    let (a0, a1, b0, b1) = if diagonal_edge <= 0.0 {
        ((0, 3), (3, 0), (1, 2), (2, 1))
    } else {
        ((0, 0), (3, 3), (1, 1), (2, 2))
    };

    let r = filter_channel(
        &mat.r,
        a0,
        a1,
        b0,
        b1,
        filter_weight_1,
        filter_weight_2,
        min_sample[0],
        max_sample[0],
    );
    let g = filter_channel(
        &mat.g,
        a0,
        a1,
        b0,
        b1,
        filter_weight_1,
        filter_weight_2,
        min_sample[1],
        max_sample[1],
    );
    let b = filter_channel(
        &mat.b,
        a0,
        a1,
        b0,
        b1,
        filter_weight_1,
        filter_weight_2,
        min_sample[2],
        max_sample[2],
    );
    let a = filter_channel(
        &mat.a,
        a0,
        a1,
        b0,
        b1,
        filter_weight_1,
        filter_weight_2,
        min_sample[3],
        max_sample[3],
    );

    pack_rgba(r, g, b, a)
}

fn filter_channel(
    mat: &[[u8; 4]; 4],
    a0: (usize, usize),
    a1: (usize, usize),
    b0: (usize, usize),
    b1: (usize, usize),
    filter_weight_1: f64,
    filter_weight_2: f64,
    min_sample: u8,
    max_sample: u8,
) -> u8 {
    let value = filter_weight_1 * (f64::from(mat[a0.0][a0.1]) + f64::from(mat[a1.0][a1.1]))
        + filter_weight_2 * (f64::from(mat[b0.0][b0.1]) + f64::from(mat[b1.0][b1.1]));
    clamp_f64(value, f64::from(min_sample), f64::from(max_sample)).ceil().clamp(0.0, 255.0) as u8
}

fn sample_bounds(mat: &Matrices) -> ([u8; 4], [u8; 4]) {
    (
        [
            min4(mat.r[1][1], mat.r[2][1], mat.r[1][2], mat.r[2][2]),
            min4(mat.g[1][1], mat.g[2][1], mat.g[1][2], mat.g[2][2]),
            min4(mat.b[1][1], mat.b[2][1], mat.b[1][2], mat.b[2][2]),
            min4(mat.a[1][1], mat.a[2][1], mat.a[1][2], mat.a[2][2]),
        ],
        [
            max4(mat.r[1][1], mat.r[2][1], mat.r[1][2], mat.r[2][2]),
            max4(mat.g[1][1], mat.g[2][1], mat.g[1][2], mat.g[2][2]),
            max4(mat.b[1][1], mat.b[2][1], mat.b[1][2], mat.b[2][2]),
            max4(mat.a[1][1], mat.a[2][1], mat.a[1][2], mat.a[2][2]),
        ],
    )
}

fn diagonal_edge(mat: &[[f64; 4]; 4], wp: &[f64; 6]) -> f64 {
    let dw1 = wp[0] * (df(mat[0][2], mat[1][1]) + df(mat[1][1], mat[2][0])
        + df(mat[1][3], mat[2][2]) + df(mat[2][2], mat[3][1]))
        + wp[1] * (df(mat[0][3], mat[1][2]) + df(mat[2][1], mat[3][0]))
        + wp[2] * (df(mat[0][3], mat[2][1]) + df(mat[1][2], mat[3][0]))
        + wp[3] * df(mat[1][2], mat[2][1])
        + wp[4] * (df(mat[0][2], mat[2][0]) + df(mat[1][3], mat[3][1]))
        + wp[5] * (df(mat[0][1], mat[1][0]) + df(mat[2][3], mat[3][2]));

    let dw2 = wp[0] * (df(mat[0][1], mat[1][2]) + df(mat[1][2], mat[2][3])
        + df(mat[1][0], mat[2][1]) + df(mat[2][1], mat[3][2]))
        + wp[1] * (df(mat[0][0], mat[1][1]) + df(mat[2][2], mat[3][3]))
        + wp[2] * (df(mat[0][0], mat[2][2]) + df(mat[1][1], mat[3][3]))
        + wp[3] * df(mat[1][1], mat[2][2])
        + wp[4] * (df(mat[1][0], mat[3][2]) + df(mat[0][1], mat[2][3]))
        + wp[5] * (df(mat[0][2], mat[1][3]) + df(mat[2][0], mat[3][1]));

    dw1 - dw2
}

fn rgba_to_u32(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|px| pack_rgba(px[0], px[1], px[2], px[3]))
        .collect()
}

fn u32_to_rgba(pixels: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&pixel.to_le_bytes());
    }
    rgba
}

fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (u32::from(a) << 24)
}

fn df(a: f64, b: f64) -> f64 {
    (a - b).abs()
}

fn clamp_i32(x: i32, floor: i32, ceil: i32) -> i32 {
    x.clamp(floor, ceil)
}

fn clamp_f64(x: f64, floor: f64, ceil: f64) -> f64 {
    x.max(floor).min(ceil)
}

fn min4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    a.min(b).min(c).min(d)
}

fn max4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    a.max(b).max(c).max(d)
}

#[cfg(test)]
mod tests {
    use super::apply_super_xbr;

    #[test]
    fn super_xbr_scales_solid_image_to_2x() {
        let rgba = vec![
            13, 29, 47, 255, 13, 29, 47, 255,
            13, 29, 47, 255, 13, 29, 47, 255,
        ];

        let (width, height, out) = apply_super_xbr(2, 2, &rgba);

        assert_eq!((width, height), (4, 4));
        assert_eq!(out.len(), 4 * 4 * 4);
        assert!(out.chunks_exact(4).all(|px| px == [13, 29, 47, 255]));
    }

    #[test]
    fn super_xbr_preserves_alpha_channel() {
        let rgba = vec![
            255, 0, 0, 64, 0, 255, 0, 128,
            0, 0, 255, 192, 255, 255, 255, 255,
        ];

        let (_, _, out) = apply_super_xbr(2, 2, &rgba);

        let alpha_values: Vec<u8> = out.chunks_exact(4).map(|px| px[3]).collect();
        assert!(alpha_values.iter().all(|alpha| *alpha >= 64));
        assert_eq!(alpha_values.iter().copied().max(), Some(255));
    }
}
