//! OmniScale pixel-art upscaling.
//!
//! Reference: https://github.com/Themaister/slang-shaders/tree/master/omniscale

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmniScaleMode {
    Scale2x,
    Scale3x,
    Scale4x,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

    fn mix(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: mix(self.r, other.r, t),
            g: mix(self.g, other.g, t),
            b: mix(self.b, other.b, t),
            a: mix(self.a, other.a, t),
        }
    }

    fn scale(self, v: f32) -> Self {
        Self {
            r: self.r * v,
            g: self.g * v,
            b: self.b * v,
            a: self.a * v,
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

    fn write_rgba(self, out: &mut [u8], index: usize) {
        let base = index * 4;
        out[base] = float_to_u8(self.r);
        out[base + 1] = float_to_u8(self.g);
        out[base + 2] = float_to_u8(self.b);
        out[base + 3] = float_to_u8(self.a);
    }
}

pub fn apply_omniscale(width: u32, height: u32, rgba: &[u8], mode: OmniScaleMode) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() {
        return (width, height, rgba.to_vec());
    }

    let scale = match mode {
        OmniScaleMode::Scale2x => 2,
        OmniScaleMode::Scale3x => 3,
        OmniScaleMode::Scale4x => 4,
    };
    let source: Vec<Pixel> = (0..(width * height) as usize)
        .map(|index| Pixel::from_rgba(rgba, index))
        .collect();
    let out_width = width * scale;
    let out_height = height * scale;
    let mut out = vec![0; (out_width * out_height * 4) as usize];

    for y in 0..out_height {
        for x in 0..out_width {
            let pixel = scale_pixel(width, height, &source, x, y, scale);
            pixel.write_rgba(&mut out, (y * out_width + x) as usize);
        }
    }

    (out_width, out_height, out)
}

fn scale_pixel(width: u32, height: u32, source: &[Pixel], out_x: u32, out_y: u32, scale: u32) -> Pixel {
    let mut sx = (out_x / scale) as i32;
    let mut sy = (out_y / scale) as i32;
    let mut step_x = 1;
    let mut step_y = 1;
    let mut p = (
        (out_x % scale) as f32 / scale as f32,
        (out_y % scale) as f32 / scale as f32,
    );

    if p.0 > 0.5 {
        step_x = -1;
        p.0 = 1.0 - p.0;
    }
    if p.1 > 0.5 {
        step_y = -1;
        p.1 = 1.0 - p.1;
    }
    if step_x < 0 {
        sx += 1;
    }
    if step_y < 0 {
        sy += 1;
    }

    let w0 = pixel_at(width, height, source, sx - step_x, sy - step_y);
    let w1 = pixel_at(width, height, source, sx, sy - step_y);
    let w2 = pixel_at(width, height, source, sx + step_x, sy - step_y);
    let w3 = pixel_at(width, height, source, sx - step_x, sy);
    let w4 = pixel_at(width, height, source, sx, sy);
    let w5 = pixel_at(width, height, source, sx + step_x, sy);
    let w6 = pixel_at(width, height, source, sx - step_x, sy + step_y);
    let w7 = pixel_at(width, height, source, sx, sy + step_y);
    let w8 = pixel_at(width, height, source, sx + step_x, sy + step_y);

    let mut pattern = 0u32;
    if is_different(w0, w4) { pattern |= 1 << 0; }
    if is_different(w1, w4) { pattern |= 1 << 1; }
    if is_different(w2, w4) { pattern |= 1 << 2; }
    if is_different(w3, w4) { pattern |= 1 << 3; }
    if is_different(w5, w4) { pattern |= 1 << 4; }
    if is_different(w6, w4) { pattern |= 1 << 5; }
    if is_different(w7, w4) { pattern |= 1 << 6; }
    if is_different(w8, w4) { pattern |= 1 << 7; }

    let pixel_size = (2.0f32).sqrt() / scale as f32;
    let pixel_size_slope = (5.0f32).sqrt() * pixel_size;

    if (pattern_matches(pattern, 0xbf, 0x37) || pattern_matches(pattern, 0xdb, 0x13)) && is_different(w1, w5) {
        return w4.mix(w3, 0.5 - p.0);
    }
    if (pattern_matches(pattern, 0xdb, 0x49) || pattern_matches(pattern, 0xef, 0x6d)) && is_different(w7, w3) {
        return w4.mix(w1, 0.5 - p.1);
    }
    if (pattern_matches(pattern, 0x0b, 0x0b) || pattern_matches(pattern, 0xfe, 0x4a) || pattern_matches(pattern, 0xfe, 0x1a))
        && is_different(w3, w1)
    {
        return w4;
    }
    if (pattern_matches(pattern, 0x6f, 0x2a)
        || pattern_matches(pattern, 0x5b, 0x0a)
        || pattern_matches(pattern, 0xbf, 0x3a)
        || pattern_matches(pattern, 0xdf, 0x5a)
        || pattern_matches(pattern, 0x9f, 0x8a)
        || pattern_matches(pattern, 0xcf, 0x8a)
        || pattern_matches(pattern, 0xef, 0x4e)
        || pattern_matches(pattern, 0x3f, 0x0e)
        || pattern_matches(pattern, 0xfb, 0x5a)
        || pattern_matches(pattern, 0xbb, 0x8a)
        || pattern_matches(pattern, 0x7f, 0x5a)
        || pattern_matches(pattern, 0xaf, 0x8a)
        || pattern_matches(pattern, 0xeb, 0x8a))
        && is_different(w3, w1)
    {
        return w4.mix(w4.mix(w0, 0.5 - p.0), 0.5 - p.1);
    }
    if pattern_matches(pattern, 0x0b, 0x08) {
        return w0.scale(0.375)
            .add(w1.scale(0.25))
            .add(w4.scale(0.375))
            .mix(w4.scale(0.5).add(w1.scale(0.5)), p.0 * 2.0)
            .mix(w4, p.1 * 2.0);
    }
    if pattern_matches(pattern, 0x0b, 0x02) {
        return w0.scale(0.375)
            .add(w3.scale(0.25))
            .add(w4.scale(0.375))
            .mix(w4.scale(0.5).add(w3.scale(0.5)), p.1 * 2.0)
            .mix(w4, p.0 * 2.0);
    }
    if pattern_matches(pattern, 0x2f, 0x2f) {
        let dist = length((p.0 - 0.5, p.1 - 0.5));
        if dist < 0.5 - pixel_size / 2.0 {
            return w4;
        }
        let r = diagonal_result(w0, w1, w3, p);
        if dist > 0.5 + pixel_size / 2.0 {
            return r;
        }
        return w4.mix(r, (dist - 0.5 + pixel_size / 2.0) / pixel_size);
    }
    if pattern_matches(pattern, 0xbf, 0x37) || pattern_matches(pattern, 0xdb, 0x13) {
        let dist = p.0 - 2.0 * p.1;
        if dist > pixel_size_slope / 2.0 {
            return w1;
        }
        let r = w3.mix(w4, p.0 + 0.5);
        if dist < -pixel_size_slope / 2.0 {
            return r;
        }
        return r.mix(w1, (dist + pixel_size_slope / 2.0) / pixel_size_slope);
    }
    if pattern_matches(pattern, 0xdb, 0x49) || pattern_matches(pattern, 0xef, 0x6d) {
        let dist = p.1 - 2.0 * p.0;
        if dist > pixel_size_slope / 2.0 {
            return w3;
        }
        let r = w1.mix(w4, p.0 + 0.5);
        if dist < -pixel_size_slope / 2.0 {
            return r;
        }
        return r.mix(w3, (dist + pixel_size_slope / 2.0) / pixel_size_slope);
    }
    if pattern_matches(pattern, 0xbf, 0x8f) || pattern_matches(pattern, 0x7e, 0x0e) {
        let dist = p.0 + 2.0 * p.1;
        if dist > 1.0 + pixel_size_slope / 2.0 {
            return w4;
        }
        let r = diagonal_result(w0, w1, w3, p);
        if dist < 1.0 - pixel_size_slope / 2.0 {
            return r;
        }
        return r.mix(w4, (dist + pixel_size_slope / 2.0 - 1.0) / pixel_size_slope);
    }
    if pattern_matches(pattern, 0x7e, 0x2a) || pattern_matches(pattern, 0xef, 0xab) {
        let dist = p.1 + 2.0 * p.0;
        if dist > 1.0 + pixel_size_slope / 2.0 {
            return w4;
        }
        let r = diagonal_result(w0, w1, w3, p);
        if dist < 1.0 - pixel_size_slope / 2.0 {
            return r;
        }
        return r.mix(w4, (dist + pixel_size_slope / 2.0 - 1.0) / pixel_size_slope);
    }
    if pattern_matches(pattern, 0x1b, 0x03)
        || pattern_matches(pattern, 0x4f, 0x43)
        || pattern_matches(pattern, 0x8b, 0x83)
        || pattern_matches(pattern, 0x6b, 0x43)
    {
        return w4.mix(w3, 0.5 - p.0);
    }
    if pattern_matches(pattern, 0x4b, 0x09)
        || pattern_matches(pattern, 0x8b, 0x89)
        || pattern_matches(pattern, 0x1f, 0x19)
        || pattern_matches(pattern, 0x3b, 0x19)
    {
        return w4.mix(w1, 0.5 - p.1);
    }
    if pattern_matches(pattern, 0xfb, 0x6a)
        || pattern_matches(pattern, 0x6f, 0x6e)
        || pattern_matches(pattern, 0x3f, 0x3e)
        || pattern_matches(pattern, 0xfb, 0xfa)
        || pattern_matches(pattern, 0xdf, 0xde)
        || pattern_matches(pattern, 0xdf, 0x1e)
    {
        return w4.mix(w0, (1.0 - p.0 - p.1) / 2.0);
    }
    if pattern_matches(pattern, 0x4f, 0x4b)
        || pattern_matches(pattern, 0x9f, 0x1b)
        || pattern_matches(pattern, 0x2f, 0x0b)
        || pattern_matches(pattern, 0xbe, 0x0a)
        || pattern_matches(pattern, 0xee, 0x0a)
        || pattern_matches(pattern, 0x7e, 0x0a)
        || pattern_matches(pattern, 0xeb, 0x4b)
        || pattern_matches(pattern, 0x3b, 0x1b)
    {
        let dist = p.0 + p.1;
        if dist > 0.5 + pixel_size / 2.0 {
            return w4;
        }
        let r = diagonal_result(w0, w1, w3, p);
        if dist < 0.5 - pixel_size / 2.0 {
            return r;
        }
        return r.mix(w4, (dist + pixel_size / 2.0 - 0.5) / pixel_size);
    }
    if pattern_matches(pattern, 0x0b, 0x01) {
        return w4
            .mix(w3, 0.5 - p.0)
            .mix(w1.mix(w1.add(w3).scale(0.5), 0.5 - p.0), 0.5 - p.1);
    }
    if pattern_matches(pattern, 0x0b, 0x00) {
        return w4
            .mix(w3, 0.5 - p.0)
            .mix(w1.mix(w0, 0.5 - p.0), 0.5 - p.1);
    }

    let dist = p.0 + p.1;
    if dist > 0.5 + pixel_size / 2.0 {
        return w4;
    }

    let x0 = pixel_at(width, height, source, sx - 2 * step_x, sy - 2 * step_y);
    let x1 = pixel_at(width, height, source, sx - step_x, sy - 2 * step_y);
    let x2 = pixel_at(width, height, source, sx, sy - 2 * step_y);
    let x3 = pixel_at(width, height, source, sx + step_x, sy - 2 * step_y);
    let x4 = pixel_at(width, height, source, sx - 2 * step_x, sy - step_y);
    let x5 = pixel_at(width, height, source, sx - 2 * step_x, sy);
    let x6 = pixel_at(width, height, source, sx - 2 * step_x, sy + step_y);

    if is_different(x0, w4) { pattern |= 1 << 8; }
    if is_different(x1, w4) { pattern |= 1 << 9; }
    if is_different(x2, w4) { pattern |= 1 << 10; }
    if is_different(x3, w4) { pattern |= 1 << 11; }
    if is_different(x4, w4) { pattern |= 1 << 12; }
    if is_different(x5, w4) { pattern |= 1 << 13; }
    if is_different(x6, w4) { pattern |= 1 << 14; }

    let diagonal_bias = pattern.count_ones() as i32 - 7;
    if diagonal_bias <= 0 {
        let r = w1.mix(w3, p.1 - p.0 + 0.5);
        if dist < 0.5 - pixel_size / 2.0 {
            return r;
        }
        return r.mix(w4, (dist + pixel_size / 2.0 - 0.5) / pixel_size);
    }

    w4
}

fn diagonal_result(w0: Pixel, w1: Pixel, w3: Pixel, p: (f32, f32)) -> Pixel {
    if is_different(w0, w1) || is_different(w0, w3) {
        w1.mix(w3, p.1 - p.0 + 0.5)
    } else {
        w1.scale(0.375)
            .add(w0.scale(0.25))
            .add(w3.scale(0.375))
            .mix(w3, p.1 * 2.0)
            .mix(w1, p.0 * 2.0)
    }
}

fn is_different(a: Pixel, b: Pixel) -> bool {
    let ca = hq_color_space(a);
    let cb = hq_color_space(b);
    (ca.0 - cb.0).abs() > 0.125
        || (ca.1 - cb.1).abs() > 0.027
        || (ca.2 - cb.2).abs() > 0.031
}

fn hq_color_space(pixel: Pixel) -> (f32, f32, f32) {
    (
        0.250 * pixel.r + 0.250 * pixel.g + 0.250 * pixel.b,
        0.250 * pixel.r - 0.250 * pixel.b,
        -0.125 * pixel.r + 0.250 * pixel.g - 0.125 * pixel.b,
    )
}

fn pixel_at(width: u32, height: u32, source: &[Pixel], x: i32, y: i32) -> Pixel {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn pattern_matches(pattern: u32, mask: u32, value: u32) -> bool {
    (pattern & mask) == value
}

fn length(v: (f32, f32)) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{apply_omniscale, OmniScaleMode};

    #[test]
    fn omniscale_modes_scale_solid_image() {
        let rgba = vec![
            41, 53, 67, 255, 41, 53, 67, 255,
            41, 53, 67, 255, 41, 53, 67, 255,
        ];

        for (mode, expected) in [
            (OmniScaleMode::Scale2x, (4, 4)),
            (OmniScaleMode::Scale3x, (6, 6)),
            (OmniScaleMode::Scale4x, (8, 8)),
        ] {
            let (width, height, out) = apply_omniscale(2, 2, &rgba, mode);
            assert_eq!((width, height), expected);
            assert!(out.chunks_exact(4).all(|px| px == [41, 53, 67, 255]));
        }
    }

    #[test]
    fn omniscale_preserves_alpha_bounds() {
        let rgba = vec![
            255, 0, 0, 32, 0, 255, 0, 96,
            0, 0, 255, 160, 255, 255, 255, 224,
        ];

        let (_, _, out) = apply_omniscale(2, 2, &rgba, OmniScaleMode::Scale3x);

        assert!(out.chunks_exact(4).all(|px| px[3] >= 32 && px[3] <= 224));
    }
}
