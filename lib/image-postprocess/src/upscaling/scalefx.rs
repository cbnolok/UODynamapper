//! ScaleFX pixel-art upscaling.
//!
//! Reference: https://github.com/Themaister/slang-shaders/tree/master/scalefx

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleFxMode {
    Scale2x,
    Scale3x,
    Scale4x,
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

    fn write_rgba(self, out: &mut [u8], index: usize) {
        let base = index * 4;
        out[base] = float_to_u8(self.r);
        out[base + 1] = float_to_u8(self.g);
        out[base + 2] = float_to_u8(self.b);
        out[base + 3] = float_to_u8(self.a);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Pass2Tag {
    corner: [bool; 4],
    horizontal: [bool; 4],
    vertical: [bool; 4],
    orientation: [bool; 4],
}

#[derive(Clone, Copy, Debug, Default)]
struct SubpixelTag {
    corner: [u8; 4],
    mid: [u8; 4],
}

pub fn apply_scalefx(width: u32, height: u32, rgba: &[u8], mode: ScaleFxMode) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 || rgba.is_empty() {
        return (width, height, rgba.to_vec());
    }

    let source: Vec<Pixel> = (0..(width * height) as usize)
        .map(|index| Pixel::from_rgba(rgba, index))
        .collect();
    let metrics = pass0_metrics(width, height, &source);
    let strengths = pass1_strengths(width, height, &metrics);
    let pass2 = pass2_tags(width, height, &metrics, &strengths);
    let subpixels = pass3_subpixels(width, height, &pass2);
    let native = pass4_output(width, height, &source, &subpixels);

    match mode {
        ScaleFxMode::Scale3x => (width * 3, height * 3, native),
        ScaleFxMode::Scale2x => resample_nearest(width * 3, height * 3, &native, width * 2, height * 2),
        ScaleFxMode::Scale4x => resample_nearest(width * 3, height * 3, &native, width * 4, height * 4),
    }
}

fn pass0_metrics(width: u32, height: u32, source: &[Pixel]) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let a = pixel_at(width, height, source, x as i32 - 1, y as i32 - 1);
            let b = pixel_at(width, height, source, x as i32, y as i32 - 1);
            let c = pixel_at(width, height, source, x as i32 + 1, y as i32 - 1);
            let e = pixel_at(width, height, source, x as i32, y as i32);
            let f = pixel_at(width, height, source, x as i32 + 1, y as i32);
            out[(y * width + x) as usize] = [
                color_distance(e, a),
                color_distance(e, b),
                color_distance(e, c),
                color_distance(e, f),
            ];
        }
    }
    out
}

fn pass1_strengths(width: u32, height: u32, metrics: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let a = value_at(width, height, metrics, x as i32 - 1, y as i32 - 1);
            let d = value_at(width, height, metrics, x as i32 - 1, y as i32);
            let e = value_at(width, height, metrics, x as i32, y as i32);
            let f = value_at(width, height, metrics, x as i32 + 1, y as i32);
            let g = value_at(width, height, metrics, x as i32 - 1, y as i32 + 1);
            let h = value_at(width, height, metrics, x as i32, y as i32 + 1);
            let i = value_at(width, height, metrics, x as i32 + 1, y as i32 + 1);

            out[(y * width + x) as usize] = [
                corner_strength(d[2], (d[3], e[1]), (a[3], d[1])),
                corner_strength(f[0], (e[3], e[1]), (value_at(width, height, metrics, x as i32, y as i32 - 1)[3], f[1])),
                corner_strength(h[2], (e[3], h[1]), (h[3], i[1])),
                corner_strength(h[0], (d[3], h[1]), (g[3], g[1])),
            ];
        }
    }
    out
}

fn pass2_tags(width: u32, height: u32, metrics: &[[f32; 4]], strengths: &[[f32; 4]]) -> Vec<Pass2Tag> {
    let mut out = vec![Pass2Tag::default(); (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let a = value_at(width, height, metrics, x as i32 - 1, y as i32 - 1);
            let b = value_at(width, height, metrics, x as i32, y as i32 - 1);
            let d = value_at(width, height, metrics, x as i32 - 1, y as i32);
            let e = value_at(width, height, metrics, x as i32, y as i32);
            let f = value_at(width, height, metrics, x as i32 + 1, y as i32);
            let g = value_at(width, height, metrics, x as i32 - 1, y as i32 + 1);
            let h = value_at(width, height, metrics, x as i32, y as i32 + 1);
            let i = value_at(width, height, metrics, x as i32 + 1, y as i32 + 1);

            let as_ = value_at(width, height, strengths, x as i32 - 1, y as i32 - 1);
            let bs = value_at(width, height, strengths, x as i32, y as i32 - 1);
            let cs = value_at(width, height, strengths, x as i32 + 1, y as i32 - 1);
            let ds = value_at(width, height, strengths, x as i32 - 1, y as i32);
            let es = value_at(width, height, strengths, x as i32, y as i32);
            let fs = value_at(width, height, strengths, x as i32 + 1, y as i32);
            let gs = value_at(width, height, strengths, x as i32 - 1, y as i32 + 1);
            let hs = value_at(width, height, strengths, x as i32, y as i32 + 1);
            let is_ = value_at(width, height, strengths, x as i32 + 1, y as i32 + 1);

            let jdx = dominance([as_[1], as_[2], as_[3]], [bs[2], bs[3], bs[0]], [es[3], es[0], es[1]], [ds[0], ds[1], ds[2]]);
            let jdy = dominance([bs[1], bs[2], bs[3]], [cs[2], cs[3], cs[0]], [fs[3], fs[0], fs[1]], [es[0], es[1], es[2]]);
            let jdz = dominance([es[1], es[2], es[3]], [fs[2], fs[3], fs[0]], [is_[3], is_[0], is_[1]], [hs[0], hs[1], hs[2]]);
            let jdw = dominance([ds[1], ds[2], ds[3]], [es[2], es[3], es[0]], [hs[3], hs[0], hs[1]], [gs[0], gs[1], gs[2]]);
            let jx = junction_vote(jdx);
            let jy = junction_vote(jdy);
            let jz = junction_vote(jdz);
            let jw = junction_vote(jdw);
            let jsx = [as_[2], bs[3], es[0], ds[1]];
            let jsy = [bs[2], cs[3], fs[0], es[1]];
            let jsz = [es[2], fs[3], is_[0], hs[1]];
            let jsw = [ds[2], es[3], hs[0], gs[1]];

            let mut corner = [
                (jx[2] + not(jx[1]) * not(jx[3]) * ge(jsx[2], 0.0) * (jx[0] + ge(jsx[0] + jsx[2], jsx[1] + jsx[3]))).min(1.0),
                (jy[3] + not(jy[2]) * not(jy[0]) * ge(jsy[3], 0.0) * (jy[1] + ge(jsy[1] + jsy[3], jsy[0] + jsy[2]))).min(1.0),
                (jz[0] + not(jz[3]) * not(jz[1]) * ge(jsz[0], 0.0) * (jz[2] + ge(jsz[0] + jsz[2], jsz[1] + jsz[3]))).min(1.0),
                (jw[1] + not(jw[0]) * not(jw[2]) * ge(jsw[1], 0.0) * (jw[3] + ge(jsw[1] + jsw[3], jsw[0] + jsw[2]))).min(1.0),
            ];
            let direct = [jx[2], jy[3], jz[0], jw[1]];
            let old = corner;
            for index in 0..4 {
                let left = old[(index + 3) % 4];
                let right = old[(index + 1) % 4];
                corner[index] = (old[index] * (direct[index] + not(left * right))).min(1.0);
            }

            let clear = [
                clear((d[2], e[0]), (d[3], e[1]), (a[3], d[1])),
                clear((f[0], e[2]), (e[3], e[1]), (b[3], f[1])),
                clear((h[2], i[0]), (e[3], h[1]), (h[3], i[1])),
                clear((h[0], g[2]), (d[3], h[1]), (g[3], g[1])),
            ];
            let horizontal = [
                le(d[3].min(a[3]), e[1].min(d[1])) * clear[0],
                le(e[3].min(b[3]), e[1].min(f[1])) * clear[1],
                le(e[3].min(h[3]), h[1].min(i[1])) * clear[2],
                le(d[3].min(g[3]), h[1].min(g[1])) * clear[3],
            ];
            let vertical = [
                ge(d[3].min(a[3]), e[1].min(d[1])) * clear[0],
                ge(e[3].min(b[3]), e[1].min(f[1])) * clear[1],
                ge(e[3].min(h[3]), h[1].min(i[1])) * clear[2],
                ge(d[3].min(g[3]), h[1].min(g[1])) * clear[3],
            ];
            let orientation = [
                ge(d[3].min(a[3]) + d[3], e[1].min(d[1]) + e[1]),
                ge(e[3].min(b[3]) + e[3], e[1].min(f[1]) + e[1]),
                ge(e[3].min(h[3]) + e[3], h[1].min(i[1]) + h[1]),
                ge(d[3].min(g[3]) + d[3], h[1].min(g[1]) + h[1]),
            ];

            out[(y * width + x) as usize] = Pass2Tag {
                corner: corner.map(flag),
                horizontal: horizontal.map(flag),
                vertical: vertical.map(flag),
                orientation: orientation.map(flag),
            };
        }
    }
    out
}

fn pass3_subpixels(width: u32, height: u32, tags: &[Pass2Tag]) -> Vec<SubpixelTag> {
    let mut out = vec![SubpixelTag::default(); (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let e = tag_at(width, height, tags, x as i32, y as i32);
            let d = tag_at(width, height, tags, x as i32 - 1, y as i32);
            let d0 = tag_at(width, height, tags, x as i32 - 2, y as i32);
            let d1 = tag_at(width, height, tags, x as i32 - 3, y as i32);
            let f = tag_at(width, height, tags, x as i32 + 1, y as i32);
            let f0 = tag_at(width, height, tags, x as i32 + 2, y as i32);
            let f1 = tag_at(width, height, tags, x as i32 + 3, y as i32);
            let b = tag_at(width, height, tags, x as i32, y as i32 - 1);
            let b0 = tag_at(width, height, tags, x as i32, y as i32 - 2);
            let b1 = tag_at(width, height, tags, x as i32, y as i32 - 3);
            let h = tag_at(width, height, tags, x as i32, y as i32 + 1);
            let h0 = tag_at(width, height, tags, x as i32, y as i32 + 2);
            let h1 = tag_at(width, height, tags, x as i32, y as i32 + 3);

            let lvl1x = e.corner[0] && (d.corner[2] || b.corner[2] || true);
            let lvl1y = e.corner[1] && (f.corner[3] || b.corner[3] || true);
            let lvl1z = e.corner[2] && (f.corner[0] || h.corner[0] || true);
            let lvl1w = e.corner[3] && (d.corner[1] || h.corner[1] || true);

            let lvl2x = [(e.corner[0] && e.horizontal[1]) && d.corner[2], (e.corner[1] && e.horizontal[0]) && f.corner[3]];
            let lvl2y = [(e.corner[1] && e.vertical[2]) && b.corner[3], (e.corner[2] && e.vertical[1]) && h.corner[0]];
            let lvl2z = [(e.corner[3] && e.horizontal[2]) && d.corner[1], (e.corner[2] && e.horizontal[3]) && f.corner[0]];
            let lvl2w = [(e.corner[0] && e.vertical[3]) && b.corner[2], (e.corner[3] && e.vertical[0]) && h.corner[1]];

            let lvl3x = [lvl2x[1] && (d.horizontal[1] && d.horizontal[0]) && f.horizontal[2], lvl2w[1] && (b.vertical[3] && b.vertical[0]) && h.vertical[2]];
            let lvl3y = [lvl2x[0] && (f.horizontal[0] && f.horizontal[1]) && d.horizontal[3], lvl2y[1] && (b.vertical[2] && b.vertical[1]) && h.vertical[3]];
            let lvl3z = [lvl2z[0] && (f.horizontal[3] && f.horizontal[2]) && d.horizontal[0], lvl2y[0] && (h.vertical[1] && h.vertical[2]) && b.vertical[0]];
            let lvl3w = [lvl2z[1] && (d.horizontal[2] && d.horizontal[3]) && f.horizontal[1], lvl2w[0] && (h.vertical[0] && h.vertical[3]) && b.vertical[1]];

            let lvl4x = [
                (d.corner[0] && d.horizontal[1] && e.horizontal[0] && e.horizontal[1] && f.horizontal[0] && f.horizontal[1]) && (d0.corner[2] && d0.horizontal[3]),
                (b.corner[0] && b.vertical[3] && e.vertical[0] && e.vertical[3] && h.vertical[0] && h.vertical[3]) && (b0.corner[2] && b0.vertical[1]),
            ];
            let lvl4y = [
                (f.corner[1] && f.horizontal[0] && e.horizontal[1] && e.horizontal[0] && d.horizontal[1] && d.horizontal[0]) && (f0.corner[3] && f0.horizontal[2]),
                (b.corner[1] && b.vertical[2] && e.vertical[1] && e.vertical[2] && h.vertical[1] && h.vertical[2]) && (b0.corner[3] && b0.vertical[0]),
            ];
            let lvl4z = [
                (f.corner[2] && f.horizontal[3] && e.horizontal[2] && e.horizontal[3] && d.horizontal[2] && d.horizontal[3]) && (f0.corner[0] && f0.horizontal[1]),
                (h.corner[2] && h.vertical[1] && e.vertical[2] && e.vertical[1] && b.vertical[2] && b.vertical[1]) && (h0.corner[0] && h0.vertical[3]),
            ];
            let lvl4w = [
                (d.corner[3] && d.horizontal[2] && e.horizontal[3] && e.horizontal[2] && f.horizontal[3] && f.horizontal[2]) && (d0.corner[1] && d0.horizontal[0]),
                (h.corner[3] && h.vertical[0] && e.vertical[3] && e.vertical[0] && b.vertical[3] && b.vertical[0]) && (h0.corner[1] && h0.vertical[2]),
            ];

            let lvl5x = [lvl4x[0] && (f0.horizontal[0] && f0.horizontal[1]) && (d1.horizontal[2] && d1.horizontal[3]), lvl4y[0] && (d0.horizontal[1] && d0.horizontal[0]) && (f1.horizontal[3] && f1.horizontal[2])];
            let lvl5y = [lvl4y[1] && (h0.vertical[1] && h0.vertical[2]) && (b1.vertical[3] && b1.vertical[0]), lvl4z[1] && (b0.vertical[2] && b0.vertical[1]) && (h1.vertical[0] && h1.vertical[3])];
            let lvl5z = [lvl4w[0] && (f0.horizontal[3] && f0.horizontal[2]) && (d1.horizontal[1] && d1.horizontal[0]), lvl4z[0] && (d0.horizontal[2] && d0.horizontal[3]) && (f1.horizontal[0] && f1.horizontal[1])];
            let lvl5w = [lvl4x[1] && (h0.vertical[0] && h0.vertical[3]) && (b1.vertical[2] && b1.vertical[1]), lvl4w[1] && (b0.vertical[3] && b0.vertical[0]) && (h1.vertical[1] && h1.vertical[2])];

            let lvl6x = [lvl5x[1] && (d1.horizontal[1] && d1.horizontal[0]), lvl5w[1] && (b1.vertical[3] && b1.vertical[0])];
            let lvl6y = [lvl5x[0] && (f1.horizontal[0] && f1.horizontal[1]), lvl5y[1] && (b1.vertical[2] && b1.vertical[1])];
            let lvl6z = [lvl5z[0] && (f1.horizontal[3] && f1.horizontal[2]), lvl5y[0] && (h1.vertical[1] && h1.vertical[2])];
            let lvl6w = [lvl5z[1] && (d1.horizontal[2] && d1.horizontal[3]), lvl5w[0] && (h1.vertical[0] && h1.vertical[3])];

            let mut tag = SubpixelTag::default();
            tag.corner[0] = choose_corner(
                (lvl1x && e.orientation[0]) || (lvl3x[0] && e.orientation[1]) || (lvl4x[0] && d.orientation[0]) || (lvl6x[0] && f.orientation[1]),
                lvl1x || (lvl3x[1] && !e.orientation[3]) || (lvl4x[1] && !b.orientation[0]) || (lvl6x[1] && !h.orientation[3]),
                [lvl3x[0], lvl3x[1], lvl4x[0], lvl4x[1], lvl6x[0], lvl6x[1]],
                [3, 7, 2, 6, 4, 8],
                5,
                1,
            );
            tag.corner[1] = choose_corner(
                (lvl1y && e.orientation[1]) || (lvl3y[0] && e.orientation[0]) || (lvl4y[0] && f.orientation[1]) || (lvl6y[0] && d.orientation[0]),
                lvl1y || (lvl3y[1] && !e.orientation[2]) || (lvl4y[1] && !b.orientation[1]) || (lvl6y[1] && !h.orientation[2]),
                [lvl3y[0], lvl3y[1], lvl4y[0], lvl4y[1], lvl6y[0], lvl6y[1]],
                [1, 7, 4, 6, 2, 8],
                5,
                3,
            );
            tag.corner[2] = choose_corner(
                (lvl1z && e.orientation[2]) || (lvl3z[0] && e.orientation[3]) || (lvl4z[0] && f.orientation[2]) || (lvl6z[0] && d.orientation[3]),
                lvl1z || (lvl3z[1] && !e.orientation[1]) || (lvl4z[1] && !h.orientation[2]) || (lvl6z[1] && !b.orientation[1]),
                [lvl3z[0], lvl3z[1], lvl4z[0], lvl4z[1], lvl6z[0], lvl6z[1]],
                [1, 5, 4, 8, 2, 6],
                7,
                3,
            );
            tag.corner[3] = choose_corner(
                (lvl1w && e.orientation[3]) || (lvl3w[0] && e.orientation[2]) || (lvl4w[0] && d.orientation[3]) || (lvl6w[0] && f.orientation[2]),
                lvl1w || (lvl3w[1] && !e.orientation[0]) || (lvl4w[1] && !h.orientation[3]) || (lvl6w[1] && !b.orientation[0]),
                [lvl3w[0], lvl3w[1], lvl4w[0], lvl4w[1], lvl6w[0], lvl6w[1]],
                [3, 5, 2, 8, 4, 6],
                7,
                1,
            );

            tag.mid[0] = choose_mid(
                (lvl2x[0] && e.orientation[0]) || (lvl2x[1] && e.orientation[1]) || (lvl5x[0] && d.orientation[0]) || (lvl5x[1] && f.orientation[1]),
                [lvl2x[0], lvl2x[1], lvl5x[0], lvl5x[1]],
                [1, 3, 2, 4],
                if e.corner[0] && d.corner[2] && e.corner[1] && f.corner[3] {
                    if e.orientation[0] {
                        if e.orientation[1] { 5 } else { 3 }
                    } else {
                        1
                    }
                } else {
                    0
                },
                5,
            );
            tag.mid[1] = choose_mid(
                (lvl2y[0] && !e.orientation[1]) || (lvl2y[1] && !e.orientation[2]) || (lvl5y[0] && !b.orientation[1]) || (lvl5y[1] && !h.orientation[2]),
                [lvl2y[0], lvl2y[1], lvl5y[0], lvl5y[1]],
                [5, 7, 6, 8],
                if e.corner[1] && b.corner[3] && e.corner[2] && h.corner[0] {
                    if !e.orientation[1] {
                        if !e.orientation[2] { 3 } else { 7 }
                    } else {
                        5
                    }
                } else {
                    0
                },
                3,
            );
            tag.mid[2] = choose_mid(
                (lvl2z[0] && e.orientation[3]) || (lvl2z[1] && e.orientation[2]) || (lvl5z[0] && d.orientation[3]) || (lvl5z[1] && f.orientation[2]),
                [lvl2z[0], lvl2z[1], lvl5z[0], lvl5z[1]],
                [1, 3, 2, 4],
                if e.corner[2] && f.corner[0] && e.corner[3] && d.corner[1] {
                    if e.orientation[2] {
                        if e.orientation[3] { 7 } else { 1 }
                    } else {
                        3
                    }
                } else {
                    0
                },
                7,
            );
            tag.mid[3] = choose_mid(
                (lvl2w[0] && !e.orientation[0]) || (lvl2w[1] && !e.orientation[3]) || (lvl5w[0] && !b.orientation[0]) || (lvl5w[1] && !h.orientation[3]),
                [lvl2w[0], lvl2w[1], lvl5w[0], lvl5w[1]],
                [5, 7, 6, 8],
                if e.corner[3] && h.corner[1] && e.corner[0] && b.corner[2] {
                    if !e.orientation[3] {
                        if !e.orientation[0] { 1 } else { 5 }
                    } else {
                        7
                    }
                } else {
                    0
                },
                1,
            );

            out[(y * width + x) as usize] = tag;
        }
    }
    out
}

fn pass4_output(width: u32, height: u32, source: &[Pixel], tags: &[SubpixelTag]) -> Vec<u8> {
    let out_width = width * 3;
    let out_height = height * 3;
    let mut out = vec![0; (out_width * out_height * 4) as usize];
    for y in 0..out_height {
        for x in 0..out_width {
            let source_x = x / 3;
            let source_y = y / 3;
            let tag = tags[(source_y * width + source_x) as usize];
            let local_x = x % 3;
            let local_y = y % 3;
            let sample = match (local_x, local_y) {
                (0, 0) => tag.corner[0],
                (1, 0) => tag.mid[0],
                (2, 0) => tag.corner[1],
                (0, 1) => tag.mid[3],
                (1, 1) => 0,
                (2, 1) => tag.mid[1],
                (0, 2) => tag.corner[3],
                (1, 2) => tag.mid[2],
                _ => tag.corner[2],
            };
            let (dx, dy) = sample_offset(sample);
            let pixel = pixel_at(width, height, source, source_x as i32 + dx, source_y as i32 + dy);
            pixel.write_rgba(&mut out, (y * out_width + x) as usize);
        }
    }
    out
}

fn resample_nearest(width: u32, height: u32, rgba: &[u8], target_width: u32, target_height: u32) -> (u32, u32, Vec<u8>) {
    let mut out = vec![0; (target_width * target_height * 4) as usize];
    for y in 0..target_height {
        let sy = ((u64::from(y) * u64::from(height)) / u64::from(target_height)).min(u64::from(height - 1)) as u32;
        for x in 0..target_width {
            let sx = ((u64::from(x) * u64::from(width)) / u64::from(target_width)).min(u64::from(width - 1)) as u32;
            let src = ((sy * width + sx) * 4) as usize;
            let dst = ((y * target_width + x) * 4) as usize;
            out[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    (target_width, target_height, out)
}

fn corner_strength(d: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    const SFX_CLR: f32 = 0.50;
    let diff = a.0 - a.1;
    let weight1 = (SFX_CLR - d).max(0.0) / SFX_CLR;
    let directional = if a.0.min(b.0) + a.0 > a.1.min(b.1) + a.1 {
        diff
    } else {
        -diff
    };
    let weight2 = ((1.0 - d) + directional).clamp(0.0, 1.0);
    (weight1 * weight2) * (a.0 * a.1)
}

fn dominance(x: [f32; 3], y: [f32; 3], z: [f32; 3], w: [f32; 3]) -> [f32; 4] {
    [
        2.0 * x[1] - (x[0] + x[2]),
        2.0 * y[1] - (y[0] + y[2]),
        2.0 * z[1] - (z[0] + z[2]),
        2.0 * w[1] - (w[0] + w[2]),
    ]
}

fn junction_vote(values: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for index in 0..4 {
        let prev = values[(index + 3) % 4];
        let next = values[(index + 1) % 4];
        let opposite = values[(index + 2) % 4];
        let majority = leq(next, 0.0) * leq(prev, 0.0)
            + ge(values[index] + opposite, next + prev);
        out[index] = (ge(values[index], 0.0) * majority).min(1.0);
    }
    out
}

fn clear(corner: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    if corner.0 >= a.0.min(a.1).max(b.0.min(b.1))
        && corner.1 >= a.0.min(b.1).max(b.0.min(a.1))
    {
        1.0
    } else {
        0.0
    }
}

fn choose_corner(
    preferred_a: bool,
    preferred_b: bool,
    fallbacks: [bool; 6],
    fallback_codes: [u8; 6],
    preferred_a_code: u8,
    preferred_b_code: u8,
) -> u8 {
    if preferred_a {
        preferred_a_code
    } else if preferred_b {
        preferred_b_code
    } else {
        fallbacks
            .iter()
            .zip(fallback_codes)
            .find_map(|(enabled, code)| enabled.then_some(code))
            .unwrap_or(0)
    }
}

fn choose_mid(
    preferred: bool,
    fallbacks: [bool; 4],
    fallback_codes: [u8; 4],
    ambiguous: u8,
    preferred_code: u8,
) -> u8 {
    if preferred {
        preferred_code
    } else {
        fallbacks
            .iter()
            .zip(fallback_codes)
            .find_map(|(enabled, code)| enabled.then_some(code))
            .unwrap_or(ambiguous)
    }
}

fn sample_offset(code: u8) -> (i32, i32) {
    match code {
        1 => (-1, 0),
        2 => (-2, 0),
        3 => (1, 0),
        4 => (2, 0),
        5 => (0, -1),
        6 => (0, -2),
        7 => (0, 1),
        8 => (0, 2),
        _ => (0, 0),
    }
}

fn color_distance(a: Pixel, b: Pixel) -> f32 {
    let r = 0.5 * (a.r + b.r);
    let dr = a.r - b.r;
    let dg = a.g - b.g;
    let db = a.b - b.b;
    (((2.0 + r) * dr * dr + 4.0 * dg * dg + (3.0 - r) * db * db).sqrt()) / 3.0
}

fn pixel_at(width: u32, height: u32, source: &[Pixel], x: i32, y: i32) -> Pixel {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn value_at<const N: usize>(width: u32, height: u32, source: &[[f32; N]], x: i32, y: i32) -> [f32; N] {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn tag_at(width: u32, height: u32, source: &[Pass2Tag], x: i32, y: i32) -> Pass2Tag {
    let x = x.clamp(0, width as i32 - 1) as u32;
    let y = y.clamp(0, height as i32 - 1) as u32;
    source[(y * width + x) as usize]
}

fn ge(x: f32, y: f32) -> f32 {
    if x >= y { 1.0 } else { 0.0 }
}

fn le(x: f32, y: f32) -> f32 {
    if x < y { 1.0 } else { 0.0 }
}

fn leq(x: f32, y: f32) -> f32 {
    if x <= y { 1.0 } else { 0.0 }
}

fn not(x: f32) -> f32 {
    1.0 - x
}

fn flag(x: f32) -> bool {
    x >= 0.5
}

fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::{apply_scalefx, ScaleFxMode};

    #[test]
    fn scalefx_modes_scale_solid_image() {
        let rgba = vec![
            27, 39, 51, 255, 27, 39, 51, 255,
            27, 39, 51, 255, 27, 39, 51, 255,
        ];

        for (mode, expected) in [
            (ScaleFxMode::Scale2x, (4, 4)),
            (ScaleFxMode::Scale3x, (6, 6)),
            (ScaleFxMode::Scale4x, (8, 8)),
        ] {
            let (width, height, out) = apply_scalefx(2, 2, &rgba, mode);
            assert_eq!((width, height), expected);
            assert!(out.chunks_exact(4).all(|px| px == [27, 39, 51, 255]));
        }
    }

    #[test]
    fn scalefx_uses_only_source_colors() {
        let rgba = vec![
            255, 0, 0, 64, 0, 255, 0, 128,
            0, 0, 255, 192, 255, 255, 255, 255,
        ];
        let source_colors: Vec<[u8; 4]> = rgba
            .chunks_exact(4)
            .map(|px| [px[0], px[1], px[2], px[3]])
            .collect();

        let (_, _, out) = apply_scalefx(2, 2, &rgba, ScaleFxMode::Scale3x);

        assert!(out.chunks_exact(4).all(|px| {
            source_colors.contains(&[px[0], px[1], px[2], px[3]])
        }));
    }
}
