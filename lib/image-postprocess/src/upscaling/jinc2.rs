//! Hyllian Jinc2 windowed-jinc resampling with anti-ringing.
//!
//! Reference files:
//! - /home/claudio/Scaricati/jinc2.glsl
//! - /home/claudio/Scaricati/jinc2-sharp.glsl
//! - /home/claudio/Scaricati/jinc2-sharper.glsl
//! - /home/claudio/Scaricati/jinc2-sharpest.glsl.txt
//! Original shader by Hyllian, MIT licensed.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jinc2Mode {
    Jinc2,
    Sharp,
    Sharper,
    Sharpest,
}

#[derive(Clone, Copy, Debug)]
struct Jinc2Params {
    window_sinc: f32,
    sinc: f32,
    ar_strength: f32,
    center_bias: f32,
}

#[derive(Clone, Copy, Debug)]
struct Pixel {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Pixel {
    fn from_rgba(rgba: &[u8], index: usize) -> Self {
        let base = index * 4;
        Self {
            r: f32::from(rgba[base]) / 255.0,
            g: f32::from(rgba[base + 1]) / 255.0,
            b: f32::from(rgba[base + 2]) / 255.0,
            a: f32::from(rgba[base + 3]) / 255.0,
        }
    }

    fn scale(self, value: f32) -> Self {
        Self {
            r: self.r * value,
            g: self.g * value,
            b: self.b * value,
            a: self.a * value,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            r: self.r + other.r,
            g: self.g + other.g,
            b: self.b + other.b,
            a: self.a + other.a,
        }
    }

    fn div(self, value: f32) -> Self {
        Self {
            r: self.r / value,
            g: self.g / value,
            b: self.b / value,
            a: self.a / value,
        }
    }

    fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: mix(self.r, other.r, t),
            g: mix(self.g, other.g, t),
            b: mix(self.b, other.b, t),
            a: mix(self.a, other.a, t),
        }
    }

    fn min(self, other: Self) -> Self {
        Self {
            r: self.r.min(other.r),
            g: self.g.min(other.g),
            b: self.b.min(other.b),
            a: self.a.min(other.a),
        }
    }

    fn max(self, other: Self) -> Self {
        Self {
            r: self.r.max(other.r),
            g: self.g.max(other.g),
            b: self.b.max(other.b),
            a: self.a.max(other.a),
        }
    }

    fn clamp(self, min: Self, max: Self) -> Self {
        Self {
            r: self.r.clamp(min.r, max.r),
            g: self.g.clamp(min.g, max.g),
            b: self.b.clamp(min.b, max.b),
            a: self.a.clamp(min.a, max.a),
        }
    }

    fn write_rgba(self, out: &mut [u8], index: usize) {
        let base = index * 4;
        out[base] = float_to_u8(self.r);
        out[base + 1] = float_to_u8(self.g);
        out[base + 2] = float_to_u8(self.b);
        out[base + 3] = float_to_u8(self.a);
    }
}

pub fn apply_jinc2(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_width: u32,
    target_height: u32,
    mode: Jinc2Mode,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || target_width == 0 || target_height == 0 || rgba.is_empty() {
        return (width, height, rgba.to_vec());
    }

    let source: Vec<Pixel> = (0..(width * height) as usize)
        .map(|index| Pixel::from_rgba(rgba, index))
        .collect();
    let params = params(mode);
    let mut out = vec![0; (target_width * target_height * 4) as usize];

    for y in 0..target_height {
        for x in 0..target_width {
            let pixel = resample_pixel(width, height, &source, target_width, target_height, x, y, params);
            pixel.write_rgba(&mut out, (y * target_width + x) as usize);
        }
    }

    (target_width, target_height, out)
}

fn params(mode: Jinc2Mode) -> Jinc2Params {
    match mode {
        Jinc2Mode::Jinc2 => Jinc2Params {
            window_sinc: 0.405,
            sinc: 0.79,
            ar_strength: 0.8,
            center_bias: 0.5,
        },
        Jinc2Mode::Sharp => Jinc2Params {
            window_sinc: 0.377,
            sinc: 0.82,
            ar_strength: 0.8,
            center_bias: 0.5,
        },
        Jinc2Mode::Sharper => Jinc2Params {
            window_sinc: 0.329,
            sinc: 0.87,
            ar_strength: 0.8,
            center_bias: 0.5,
        },
        Jinc2Mode::Sharpest => Jinc2Params {
            window_sinc: 0.29,
            sinc: 0.92,
            ar_strength: 1.0,
            center_bias: 0.4999,
        },
    }
}

fn resample_pixel(
    width: u32,
    height: u32,
    source: &[Pixel],
    target_width: u32,
    target_height: u32,
    out_x: u32,
    out_y: u32,
    params: Jinc2Params,
) -> Pixel {
    let pc_x = (out_x as f32 + 0.5) * width as f32 / target_width as f32;
    let pc_y = (out_y as f32 + 0.5) * height as f32 / target_height as f32;
    let base_x = (pc_x - params.center_bias).floor() as i32;
    let base_y = (pc_y - params.center_bias).floor() as i32;
    let tc_x = base_x as f32 + params.center_bias;
    let tc_y = base_y as f32 + params.center_bias;

    let mut weights = [[0.0; 4]; 4];
    for row in 0..4 {
        for col in 0..4 {
            let sample_x = tc_x + col as f32 - 1.0;
            let sample_y = tc_y + row as f32 - 1.0;
            let dist = distance(pc_x, pc_y, sample_x, sample_y);
            weights[row][col] = resampler(dist, params);
        }
    }

    let c00 = pixel_at(width, height, source, base_x - 1, base_y - 1);
    let c10 = pixel_at(width, height, source, base_x, base_y - 1);
    let c20 = pixel_at(width, height, source, base_x + 1, base_y - 1);
    let c30 = pixel_at(width, height, source, base_x + 2, base_y - 1);
    let c01 = pixel_at(width, height, source, base_x - 1, base_y);
    let c11 = pixel_at(width, height, source, base_x, base_y);
    let c21 = pixel_at(width, height, source, base_x + 1, base_y);
    let c31 = pixel_at(width, height, source, base_x + 2, base_y);
    let c02 = pixel_at(width, height, source, base_x - 1, base_y + 1);
    let c12 = pixel_at(width, height, source, base_x, base_y + 1);
    let c22 = pixel_at(width, height, source, base_x + 1, base_y + 1);
    let c32 = pixel_at(width, height, source, base_x + 2, base_y + 1);
    let c03 = pixel_at(width, height, source, base_x - 1, base_y + 2);
    let c13 = pixel_at(width, height, source, base_x, base_y + 2);
    let c23 = pixel_at(width, height, source, base_x + 1, base_y + 2);
    let c33 = pixel_at(width, height, source, base_x + 2, base_y + 2);

    let color = dot4([c00, c10, c20, c30], weights[0])
        .add(dot4([c01, c11, c21, c31], weights[1]))
        .add(dot4([c02, c12, c22, c32], weights[2]))
        .add(dot4([c03, c13, c23, c33], weights[3]));
    let weight_sum = weights.iter().flatten().copied().sum::<f32>();
    if weight_sum.abs() <= f32::EPSILON {
        return pixel_at(width, height, source, pc_x.floor() as i32, pc_y.floor() as i32);
    }

    let color = color.div(weight_sum);
    let min_sample = c11.min(c21).min(c12).min(c22);
    let max_sample = c11.max(c21).max(c12).max(c22);
    let clamped = color.clamp(min_sample, max_sample);

    color.mix(clamped, params.ar_strength)
}

fn dot4(pixels: [Pixel; 4], weights: [f32; 4]) -> Pixel {
    pixels[0]
        .scale(weights[0])
        .add(pixels[1].scale(weights[1]))
        .add(pixels[2].scale(weights[2]))
        .add(pixels[3].scale(weights[3]))
}

fn resampler(x: f32, params: Jinc2Params) -> f32 {
    let pi = std::f32::consts::PI;
    let wa = params.window_sinc * pi;
    let wb = params.sinc * pi;
    if x.abs() <= f32::EPSILON {
        wa * wb
    } else {
        (x * wa).sin() * (x * wb).sin() / (x * x)
    }
}

fn distance(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    let x = x1 - x0;
    let y = y1 - y0;
    (x * x + y * y).sqrt()
}

fn pixel_at(width: u32, height: u32, source: &[Pixel], x: i32, y: i32) -> Pixel {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jinc2_modes_scale_solid_image() {
        let rgba = vec![
            64, 128, 200, 77,
            64, 128, 200, 77,
        ];

        for mode in [Jinc2Mode::Jinc2, Jinc2Mode::Sharp, Jinc2Mode::Sharper, Jinc2Mode::Sharpest] {
            for scale in [2, 3, 4] {
                let (w, h, out) = apply_jinc2(2, 1, &rgba, 2 * scale, scale, mode);
                assert_eq!((w, h), (2 * scale, scale));
                assert_eq!(out.len(), (w * h * 4) as usize);
                for pixel in out.chunks_exact(4) {
                    assert_eq!(pixel, &[64, 128, 200, 77]);
                }
            }
        }
    }

    #[test]
    fn jinc2_sharpest_resamples_alpha() {
        let rgba = vec![
            255, 0, 0, 0,
            0, 255, 0, 255,
            0, 0, 255, 255,
            255, 255, 255, 0,
        ];

        let (w, h, out) = apply_jinc2(2, 2, &rgba, 8, 8, Jinc2Mode::Sharpest);
        assert_eq!((w, h), (8, 8));
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert!(out.chunks_exact(4).any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    }
}
