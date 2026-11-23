//! The candidate filters. Each changes the solid texels only and reads only solid texels of the same hue class (a
//! hueable texel's red byte is its hue key, so it never mixes with a texel that keeps its colour); the anti-aliased
//! edge (coverage below 16) and the transparent texels stay as they are.

use crate::metrics::{self, LUMA, rgb};
use crate::source::{Frame, Palette, RGBA_BYTES, distance};

/// Palette-aware de-quantization: a Gaussian mean over the texels whose colour lies within `PAL_ADMIT` palette
/// spacings of the texel's own palette colour (near-adjacent palette entries, so a real edge between distant colours
/// never mixes), then clamped to `PAL_CLAMP` spacings from that colour (the quantization cell, widened since a dither
/// picks a neighbouring entry on purpose).
pub const PAL_RADIUS: isize = 2;
pub const PAL_SIGMA: f32 = 1.2;
pub const PAL_ADMIT: f32 = 2.5;
pub const PAL_CLAMP: f32 = 1.0;
/// Bilateral: spatial and range sigmas (RGB, 0..255).
pub const BILATERAL_RADIUS: isize = 2;
pub const BILATERAL_SIGMA: f32 = 1.2;
pub const BILATERAL_RANGE: f32 = 16.0;
/// Selective median: the outlier test with lower thresholds than the measure's.
pub const MEDIAN_MIN_DISTANCE: f32 = 12.0;
pub const MEDIAN_SPREAD_FACTOR: f32 = 2.5;
/// Guided filter (self-guided on luma): window radius and regularization (RGB units squared).
pub const GUIDED_RADIUS: isize = 2;
pub const GUIDED_EPS: f32 = 12.0 * 12.0;
/// Highlight-only selective median: the `Median` filter's test and replacement, only where at least
/// `HIGHLIGHT_CLIPPED` of the 3x3 window's texels have a clipped channel. There only the other channels carry the
/// texture's shading, so its brightness steps show as hue steps (pale and saturated texels side by side).
pub const HIGHLIGHT_CLIPPED: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    Source,
    Palette,
    Palette2,
    Bilateral,
    Median,
    Guided,
    Highlight,
}

impl Filter {
    pub const ALL: [Filter; 7] = [Filter::Source, Filter::Palette, Filter::Palette2, Filter::Bilateral, Filter::Median,
        Filter::Guided, Filter::Highlight];

    pub fn name(self) -> &'static str {
        match self {
            Filter::Source => "source",
            Filter::Palette => "palette",
            Filter::Palette2 => "palette2",
            Filter::Bilateral => "bilateral",
            Filter::Median => "median",
            Filter::Guided => "guided",
            Filter::Highlight => "highlight",
        }
    }

    pub fn parse(name: &str) -> Filter {
        *Filter::ALL.iter().find(|f| f.name() == name).unwrap_or_else(|| panic!("no filter {name}"))
    }

    pub fn apply(self, frame: &Frame, palette: &Palette) -> Vec<u8> {
        match self {
            Filter::Source => frame.rgba.clone(),
            Filter::Palette => palette_pass(frame, &frame.rgba, palette),
            Filter::Palette2 => palette_pass(frame, &palette_pass(frame, &frame.rgba, palette), palette),
            Filter::Bilateral => bilateral(frame),
            Filter::Median => selective_median(frame, 0),
            Filter::Guided => guided(frame),
            Filter::Highlight => selective_median(frame, HIGHLIGHT_CLIPPED),
        }
    }
}

fn store(out: &mut [u8], i: usize, c: [f32; 3]) {
    for k in 0..3 {
        out[i * RGBA_BYTES + k] = c[k].round().clamp(0.0, 255.0) as u8;
    }
}

/// The solid texels of the same hue class within `radius`, with their offsets.
fn neighbours(frame: &Frame, i: usize, radius: isize) -> impl Iterator<Item = (isize, isize, usize)> + '_ {
    let (w, h) = (frame.width as isize, frame.height as isize);
    let (x, y) = ((i % frame.width) as isize, (i / frame.width) as isize);
    (-radius..=radius).flat_map(move |dy| (-radius..=radius).map(move |dx| (dx, dy)))
        .filter(move |&(dx, dy)| x + dx >= 0 && y + dy >= 0 && x + dx < w && y + dy < h)
        .map(move |(dx, dy)| (dx, dy, ((y + dy) * w + x + dx) as usize))
        .filter(move |&(_, _, j)| frame.solid[j] && frame.hueable[j] == frame.hueable[i])
}

fn palette_pass(frame: &Frame, rgba: &[u8], palette: &Palette) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for i in (0..frame.width * frame.height).filter(|&i| frame.solid[i]) {
        let own = palette.rgb[usize::from(frame.index[i])];
        let spacing = palette.spacing[usize::from(frame.index[i])];
        let (mut acc, mut weight) = ([0.0f32; 3], 0.0f32);
        for (dx, dy, j) in neighbours(frame, i, PAL_RADIUS) {
            let c = rgb(rgba, j);
            if distance(&c, &own) > PAL_ADMIT * spacing { continue; }
            let w = (-((dx * dx + dy * dy) as f32) / (2.0 * PAL_SIGMA * PAL_SIGMA)).exp();
            for k in 0..3 { acc[k] += w * c[k]; }
            weight += w;
        }
        let mean = [acc[0] / weight, acc[1] / weight, acc[2] / weight];
        let shift = [mean[0] - own[0], mean[1] - own[1], mean[2] - own[2]];
        let length = distance(&mean, &own);
        let scale = if length > PAL_CLAMP * spacing { PAL_CLAMP * spacing / length } else { 1.0 };
        store(&mut out, i, [own[0] + scale * shift[0], own[1] + scale * shift[1], own[2] + scale * shift[2]]);
    }
    out
}

fn bilateral(frame: &Frame) -> Vec<u8> {
    let mut out = frame.rgba.clone();
    for i in (0..frame.width * frame.height).filter(|&i| frame.solid[i]) {
        let p = rgb(&frame.rgba, i);
        let (mut acc, mut weight) = ([0.0f32; 3], 0.0f32);
        for (dx, dy, j) in neighbours(frame, i, BILATERAL_RADIUS) {
            let c = rgb(&frame.rgba, j);
            let spatial = -((dx * dx + dy * dy) as f32) / (2.0 * BILATERAL_SIGMA * BILATERAL_SIGMA);
            let range = -distance(&c, &p).powi(2) / (2.0 * BILATERAL_RANGE * BILATERAL_RANGE);
            let w = (spatial + range).exp();
            for k in 0..3 { acc[k] += w * c[k]; }
            weight += w;
        }
        store(&mut out, i, [acc[0] / weight, acc[1] / weight, acc[2] / weight]);
    }
    out
}

/// The selective median where at least `min_clipped` texels of the 3x3 window have a clipped channel (0: anywhere).
fn selective_median(frame: &Frame, min_clipped: usize) -> Vec<u8> {
    let inner = metrics::interior(&frame.solid, frame.width, frame.height);
    let clipped = |j: usize| rgb(&frame.rgba, j).iter().any(|&v| v >= metrics::CLIP_LEVEL);
    let mut out = frame.rgba.clone();
    for i in (0..frame.width * frame.height).filter(|&i| inner[i]) {
        // Mixing hue classes is what the outlier test would see as a speck; leave those texels.
        let same_class = neighbours(frame, i, 1).count() == 9;
        if !same_class { continue; }
        if min_clipped > 0 && neighbours(frame, i, 1).filter(|&(_, _, j)| clipped(j)).count() < min_clipped { continue; }
        if let Some(m) = metrics::is_outlier(&frame.rgba, frame.width, i, MEDIAN_MIN_DISTANCE, MEDIAN_SPREAD_FACTOR) {
            store(&mut out, i, m);
        }
    }
    out
}

/// He et al.'s guided filter with the frame's luma as guide, each channel filtered, means taken over the solid texels
/// of the same hue class in the window.
fn guided(frame: &Frame) -> Vec<u8> {
    let count = frame.width * frame.height;
    let guide: Vec<f32> = (0..count).map(|i| {
        let c = rgb(&frame.rgba, i);
        LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2]
    }).collect();
    let mut a = vec![[0.0f32; 3]; count];
    let mut b = vec![[0.0f32; 3]; count];
    for i in (0..count).filter(|&i| frame.solid[i]) {
        let (mut n, mut mean_i, mut mean_ii) = (0.0f32, 0.0f32, 0.0f32);
        let (mut mean_p, mut mean_ip) = ([0.0f32; 3], [0.0f32; 3]);
        for (_, _, j) in neighbours(frame, i, GUIDED_RADIUS) {
            let (g, c) = (guide[j], rgb(&frame.rgba, j));
            n += 1.0;
            mean_i += g;
            mean_ii += g * g;
            for k in 0..3 { mean_p[k] += c[k]; mean_ip[k] += g * c[k]; }
        }
        mean_i /= n;
        let variance = mean_ii / n - mean_i * mean_i;
        for k in 0..3 {
            let mp = mean_p[k] / n;
            let covariance = mean_ip[k] / n - mean_i * mp;
            a[i][k] = covariance / (variance + GUIDED_EPS);
            b[i][k] = mp - a[i][k] * mean_i;
        }
    }
    let mut out = frame.rgba.clone();
    for i in (0..count).filter(|&i| frame.solid[i]) {
        let (mut n, mut ma, mut mb) = (0.0f32, [0.0f32; 3], [0.0f32; 3]);
        for (_, _, j) in neighbours(frame, i, GUIDED_RADIUS) {
            n += 1.0;
            for k in 0..3 { ma[k] += a[j][k]; mb[k] += b[j][k]; }
        }
        let g = guide[i];
        store(&mut out, i, [ma[0] / n * g + mb[0] / n, ma[1] / n * g + mb[1] / n, ma[2] / n * g + mb[2] / n]);
    }
    out
}
