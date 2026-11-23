//! No-reference measures of a frame's speckle, over its interior: the solid (opaque) texels whose eight neighbours are
//! solid too, so the outline and the anti-aliased edge never count.
//!
//! - **Isolated outliers**: a texel at least `OUTLIER_MIN_DISTANCE` (RGB, 0..255) from the channel median of its eight
//!   neighbours, and `OUTLIER_SPREAD_FACTOR` times farther than those neighbours are from it (their median distance):
//!   a speck on a smooth area, not a texel of a busy texture. `dark` counts those darker than the median.
//! - **High-frequency energy**: the mean RGB distance from each texel to the mean of its solid neighbours.
//! - **Residual lag-1 correlation**: the high-pass residual (texel minus its neighbours' mean) correlated with the
//!   next texel's, along rows and columns. Painted texture and smooth shading correlate positively; error-diffusion
//!   dither, which pushes each texel's error onto the next ones, negatively.
//!
//! And against a reference (the source frame), how much a filter moved:
//! - **Low-pass PSNR**: both frames blurred (binomial 5x5 over the solid texels) and compared over the interior: the
//!   local mean colour a filter must keep.
//! - **Edge ratio**: the mean Sobel gradient of the filtered frame over that of the source, on the texels where the
//!   blurred source has its strongest tenth of gradients (real edges, not noise).

use crate::source::{RGBA_BYTES, distance};

pub const OUTLIER_MIN_DISTANCE: f32 = 20.0;
pub const OUTLIER_SPREAD_FACTOR: f32 = 3.0;
const NEIGHBOURS: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
const PEAK_SQUARED: f64 = 255.0 * 255.0;
const OPAQUE: u8 = u8::MAX;
/// Binomial 5-tap weights.
const BINOMIAL: [f32; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
/// The share of interior texels taken as strong edges.
const EDGE_SHARE: f32 = 0.1;
/// Luma weights (Rec. 601).
pub const LUMA: [f32; 3] = [0.299, 0.587, 0.114];
/// A channel at or above this is clipped (the render's highlight saturates it).
pub const CLIP_LEVEL: f32 = 250.0;

/// Sums over frames: the counts add up and the means weigh by interior texels.
#[derive(Default, Clone, Copy, Debug)]
pub struct Speckle {
    pub interior: u64,
    pub outliers: u64,
    pub dark: u64,
    /// Interior texels with a clipped channel, and outliers where the texel or its neighbours' median is clipped.
    pub clipped: u64,
    pub clipped_outliers: u64,
    pub high_frequency: f64,
    lag_product: f64,
    lag_square: f64,
}

impl Speckle {
    pub fn add(&mut self, other: &Speckle) {
        self.interior += other.interior;
        self.outliers += other.outliers;
        self.dark += other.dark;
        self.clipped += other.clipped;
        self.clipped_outliers += other.clipped_outliers;
        self.high_frequency += other.high_frequency;
        self.lag_product += other.lag_product;
        self.lag_square += other.lag_square;
    }

    /// Outliers per thousand interior texels.
    pub fn per_mille(&self) -> f64 {
        1000.0 * self.outliers as f64 / self.interior.max(1) as f64
    }

    pub fn dark_share(&self) -> f64 {
        self.dark as f64 / self.outliers.max(1) as f64
    }

    pub fn mean_high_frequency(&self) -> f64 {
        self.high_frequency / self.interior.max(1) as f64
    }

    pub fn lag_correlation(&self) -> f64 {
        self.lag_product / self.lag_square.max(f64::MIN_POSITIVE)
    }
}

pub fn rgb(rgba: &[u8], i: usize) -> [f32; 3] {
    let at = i * RGBA_BYTES;
    [f32::from(rgba[at]), f32::from(rgba[at + 1]), f32::from(rgba[at + 2])]
}

pub fn luma(c: &[f32; 3]) -> f32 {
    LUMA[0] * c[0] + LUMA[1] * c[1] + LUMA[2] * c[2]
}

/// Solid texels: opaque ones (the decoder makes the solid pixels 255 and the edge pixels below).
pub fn solid_of(rgba: &[u8]) -> Vec<bool> {
    rgba.chunks_exact(RGBA_BYTES).map(|t| t[3] == OPAQUE).collect()
}

pub fn interior(solid: &[bool], width: usize, height: usize) -> Vec<bool> {
    (0..width * height).map(|i| {
        let (x, y) = ((i % width) as isize, (i / width) as isize);
        solid[i] && NEIGHBOURS.iter().all(|&(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height
                && solid[ny as usize * width + nx as usize]
        })
    }).collect()
}

pub fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { 0.5 * (values[n / 2 - 1] + values[n / 2]) }
}

/// The channel median of a texel's eight neighbours (all solid: interior texels only).
pub fn neighbour_median(rgba: &[u8], width: usize, i: usize) -> [f32; 3] {
    let (x, y) = ((i % width) as isize, (i / width) as isize);
    let colours: Vec<[f32; 3]> = NEIGHBOURS.iter()
        .map(|&(dx, dy)| rgb(rgba, ((y + dy) as usize) * width + (x + dx) as usize)).collect();
    let mut m = [0.0; 3];
    for c in 0..3 {
        let mut values: Vec<f32> = colours.iter().map(|colour| colour[c]).collect();
        m[c] = median(&mut values);
    }
    m
}

/// Whether an interior texel is an isolated outlier at the given thresholds, and the median it differs from.
pub fn is_outlier(rgba: &[u8], width: usize, i: usize, min_distance: f32, spread_factor: f32) -> Option<[f32; 3]> {
    let (x, y) = ((i % width) as isize, (i / width) as isize);
    let m = neighbour_median(rgba, width, i);
    let d = distance(&rgb(rgba, i), &m);
    if d < min_distance {
        return None;
    }
    let mut spread: Vec<f32> = NEIGHBOURS.iter()
        .map(|&(dx, dy)| distance(&rgb(rgba, ((y + dy) as usize) * width + (x + dx) as usize), &m)).collect();
    (d >= spread_factor * median(&mut spread)).then_some(m)
}

pub fn speckle(rgba: &[u8], width: usize, height: usize) -> Speckle {
    let solid = solid_of(rgba);
    let inner = interior(&solid, width, height);
    let mut s = Speckle::default();
    let mut residual = vec![[0.0f32; 3]; width * height];
    for i in (0..width * height).filter(|&i| inner[i]) {
        s.interior += 1;
        let clipped = |c: [f32; 3]| c.iter().any(|&v| v >= CLIP_LEVEL);
        if clipped(rgb(rgba, i)) {
            s.clipped += 1;
        }
        if let Some(m) = is_outlier(rgba, width, i, OUTLIER_MIN_DISTANCE, OUTLIER_SPREAD_FACTOR) {
            s.outliers += 1;
            if clipped(rgb(rgba, i)) || clipped(m) {
                s.clipped_outliers += 1;
            }
            if luma(&rgb(rgba, i)) < luma(&m) {
                s.dark += 1;
            }
        }
        let (x, y) = ((i % width) as isize, (i / width) as isize);
        let mut mean = [0.0f32; 3];
        for &(dx, dy) in &NEIGHBOURS {
            let n = rgb(rgba, ((y + dy) as usize) * width + (x + dx) as usize);
            for c in 0..3 { mean[c] += n[c] / NEIGHBOURS.len() as f32; }
        }
        let p = rgb(rgba, i);
        s.high_frequency += f64::from(distance(&p, &mean));
        residual[i] = [p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]];
    }
    for i in (0..width * height).filter(|&i| inner[i]) {
        let r = residual[i];
        s.lag_square += f64::from(r[0] * r[0] + r[1] * r[1] + r[2] * r[2]);
        for j in [i + 1, i + width].into_iter().filter(|&j| j < width * height && inner[j]) {
            let q = residual[j];
            s.lag_product += 0.5 * f64::from(r[0] * q[0] + r[1] * q[1] + r[2] * q[2]);
        }
    }
    s
}

/// Binomial 5x5 blur over the solid texels (normalized by their weights); the others stay zero.
fn blur(rgba: &[u8], solid: &[bool], width: usize, height: usize) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0f32; 3]; width * height];
    for i in (0..width * height).filter(|&i| solid[i]) {
        let (x, y) = ((i % width) as isize, (i / width) as isize);
        let (mut acc, mut weight) = ([0.0f32; 3], 0.0f32);
        for (ky, wy) in BINOMIAL.iter().enumerate() {
            for (kx, wx) in BINOMIAL.iter().enumerate() {
                let (nx, ny) = (x + kx as isize - 2, y + ky as isize - 2);
                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height { continue; }
                let j = ny as usize * width + nx as usize;
                if !solid[j] { continue; }
                let w = wx * wy;
                let c = rgb(rgba, j);
                for k in 0..3 { acc[k] += w * c[k]; }
                weight += w;
            }
        }
        out[i] = [acc[0] / weight, acc[1] / weight, acc[2] / weight];
    }
    out
}

fn sobel_luma(values: &[[f32; 3]], inner: &[bool], width: usize, i: usize) -> f32 {
    let at = |dx: isize, dy: isize| {
        let (x, y) = ((i % width) as isize + dx, (i / width) as isize + dy);
        luma(&values[y as usize * width + x as usize])
    };
    debug_assert!(inner[i]);
    let gx = at(1, -1) + 2.0 * at(1, 0) + at(1, 1) - at(-1, -1) - 2.0 * at(-1, 0) - at(-1, 1);
    let gy = at(-1, 1) + 2.0 * at(0, 1) + at(1, 1) - at(-1, -1) - 2.0 * at(0, -1) - at(1, -1);
    (gx * gx + gy * gy).sqrt()
}

/// Squared error sums of the low-pass comparison and the Sobel sums of the edge ratio, to add over frames.
#[derive(Default, Clone, Copy, Debug)]
pub struct Fidelity {
    low_squared: f64,
    low_count: f64,
    full_squared: f64,
    edge_source: f64,
    edge_filtered: f64,
}

impl Fidelity {
    pub fn of(source: &[u8], filtered: &[u8], width: usize, height: usize) -> Self {
        let solid = solid_of(source);
        let inner = interior(&solid, width, height);
        let (a, b) = (blur(source, &solid, width, height), blur(filtered, &solid, width, height));
        let mut f = Fidelity::default();
        let raw = |rgba: &[u8]| -> Vec<[f32; 3]> { (0..width * height).map(|i| rgb(rgba, i)).collect() };
        let (source_raw, filtered_raw) = (raw(source), raw(filtered));
        let mut strength: Vec<(f32, usize)> = Vec::new();
        for i in (0..width * height).filter(|&i| inner[i]) {
            for c in 0..3 {
                f.low_squared += f64::from(a[i][c] - b[i][c]).powi(2);
                f.full_squared += f64::from(source_raw[i][c] - filtered_raw[i][c]).powi(2);
            }
            f.low_count += 3.0;
            strength.push((sobel_luma(&a, &inner, width, i), i));
        }
        strength.sort_by(|x, y| y.0.total_cmp(&x.0));
        let take = (strength.len() as f32 * EDGE_SHARE).ceil() as usize;
        for &(_, i) in strength.iter().take(take) {
            f.edge_source += f64::from(sobel_luma(&source_raw, &inner, width, i));
            f.edge_filtered += f64::from(sobel_luma(&filtered_raw, &inner, width, i));
        }
        f
    }

    pub fn add(&mut self, other: &Fidelity) {
        self.low_squared += other.low_squared;
        self.low_count += other.low_count;
        self.full_squared += other.full_squared;
        self.edge_source += other.edge_source;
        self.edge_filtered += other.edge_filtered;
    }

    pub fn low_pass_psnr(&self) -> f64 {
        psnr(self.low_squared, self.low_count)
    }

    pub fn full_psnr(&self) -> f64 {
        psnr(self.full_squared, self.low_count)
    }

    pub fn edge_ratio(&self) -> f64 {
        self.edge_filtered / self.edge_source.max(f64::MIN_POSITIVE)
    }
}

pub fn psnr(squared: f64, count: f64) -> f64 {
    if squared == 0.0 { f64::INFINITY } else { 10.0 * (PEAK_SQUARED / (squared / count.max(1.0))).log10() }
}

/// Squared RGB error over the reference's opaque texels, and their count x 3.
pub fn opaque_error(reference: &[u8], other: &[u8]) -> (f64, f64) {
    let (mut squared, mut count) = (0.0, 0.0);
    for (r, o) in reference.chunks_exact(RGBA_BYTES).zip(other.chunks_exact(RGBA_BYTES)) {
        if r[3] == OPAQUE {
            for c in 0..3 { squared += (f64::from(r[c]) - f64::from(o[c])).powi(2); }
            count += 3.0;
        }
    }
    (squared, count)
}
