//! BC7 Analytical Encoder
#![allow(dead_code)]
//!
//! Architecture:
//!   1. Types and quantization helpers
//!   2. LS tables and BC7 error helpers
//!   3. Weight evaluation for each mode
//!   4. Least-squares endpoint fitting (1D / 3D / 4D)
//!   5. Encode functions (modes 0–7) with anchor-inversion logic
//!   6. Mode-selection entry points

use super::tables::*;
use std::cmp::min;
use wide::f32x4;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

// ─── 1. TYPES ────────────────────────────────────────────────────────────────

/// Simple 4-component float vector used throughout the encoder.
#[derive(Clone, Copy, Default, Debug)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    #[inline] pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self { Self { x, y, z, w } }
    #[inline] pub fn set(&mut self, x: f32, y: f32, z: f32, w: f32) { self.x=x; self.y=y; self.z=z; self.w=w; }
}

impl std::ops::Index<usize> for Vec4 {
    type Output = f32;
    fn index(&self, i: usize) -> &f32 { match i { 0=>&self.x, 1=>&self.y, 2=>&self.z, _=>&self.w } }
}
impl std::ops::IndexMut<usize> for Vec4 {
    fn index_mut(&mut self, i: usize) -> &mut f32 { match i { 0=>&mut self.x, 1=>&mut self.y, 2=>&mut self.z, _=>&mut self.w } }
}

pub type Pixel = [u8; 4];

// ─── PUBLIC FLAGS ─────────────────────────────────────────────────────────────

pub const FLAG_USE_DUAL_PLANE:   u32 = 1 << 0;  // try modes 4/5 for decorrelated channels
pub const FLAG_USE_TRIVIAL_M6:   u32 = 1 << 1;  // fast path for low-variance blocks
pub const FLAG_PBIT_OPT_M6:      u32 = 1 << 2;  // optimize p-bits for mode 6
pub const FLAG_USE_2SUBSETS:     u32 = 1 << 3;  // try modes 1/3/7 (2-subset)
pub const FLAG_USE_3SUBSETS:     u32 = 1 << 4;  // try modes 0/2 (3-subset)
pub const FLAG_PBIT_OPT:         u32 = 1 << 5;  // optimize p-bits for all modes

// ─── 2. QUANTIZATION HELPERS ─────────────────────────────────────────────────

/// Round float to nearest int (handles negative correctly).
#[inline] fn rnd(x: f32) -> i32 { if x >= 0.0 { (x + 0.5) as i32 } else { (x - 0.5) as i32 } }

/// Convert 8-bit value to 5-bit (no p-bit).
#[inline] fn to_5(c: i32) -> i32 { (c * 31 + 127) / 255 }
/// Convert 8-bit to 5-bit with p-bit constraint (6-bit rounded, then LSB forced).
#[inline] fn to_5p(c: i32, pbit: i32) -> i32 {
    let q6 = (c * 63 + 127) / 255;
    if (q6 & 1) == pbit { q6 >> 1 } else {
        let lhs = c * 63;
        let rhs = 255 * q6;
        let q6 = if lhs >= rhs { if q6 < 63 { q6+1 } else { q6-1 } }
                 else           { if q6 >  0 { q6-1 } else { q6+1 } };
        q6 >> 1
    }
}
/// Clamp-and-convert float 0–255 to 5-bit.
#[inline] fn to_5_clamp(c: f32, pbit: u32) -> u32 { to_5p(rnd(c).clamp(0, 255), pbit as i32) as u32 }
/// Convert float 0–255 to 5-bit without p-bit.
#[inline] fn to_5f(c: f32) -> i32 { to_5(rnd(c.clamp(0.0, 255.0))) }

/// Convert 8-bit to 6-bit with p-bit constraint (7-bit rounded, then LSB forced).
#[inline] fn to_6p(c: i32, pbit: i32) -> i32 {
    let q7 = (c * 127 + 127) / 255;
    if (q7 & 1) == pbit { q7 >> 1 } else {
        let lhs = c * 127;
        let rhs = 255 * q7;
        let q7 = if lhs >= rhs { if q7 < 127 { q7+1 } else { q7-1 } }
                 else           { if q7 >  0  { q7-1 } else { q7+1 } };
        q7 >> 1
    }
}
/// Clamp-and-convert float 0–255 to 6-bit with p-bit.
#[inline] fn to_6_clamp(c: f32, pbit: i32) -> i32 { to_6p(rnd(c).clamp(0, 255), pbit) }

/// Convert 8-bit to 7-bit, forcing p-bit to match.
#[inline] fn to_7p(c: i32, pbit: i32) -> i32 { min(127, ((c + (pbit ^ 1)) >> 1) as i32) }
/// Clamp float and convert to 7-bit with p-bit.
#[inline] fn to_7_clamp(c: f32, pbit: i32) -> i32 { to_7p(rnd(c).clamp(0, 255), pbit) }
/// Round float to 7-bit (no p-bit constraint).
#[inline] fn to_7f(c: f32) -> i32 { ((c * 127.0 / 255.0 + 0.5) as i32).clamp(0, 127) }

/// Expand 4-bit endpoint + unique p-bit → 8-bit: v = (v<<1)|p then replicate 3 bits.
#[inline] pub fn from_4(v: u32, p: u32) -> u32 { let x = (v<<1)|p; (x<<3)|(x>>2) }
/// Expand 5-bit endpoint → 8-bit: replicate 3 high bits into low.
#[inline] pub fn from_5(v: u32) -> u32 { (v<<3)|(v>>2) }
/// Expand 5-bit endpoint + unique p-bit → 8-bit.
#[inline] pub fn from_5p(v: u32, p: u32) -> u32 { let x=(v<<1)|p; (x<<2)|(x>>4) }
/// Expand 6-bit endpoint + shared p-bit → 8-bit.
#[inline] pub fn from_6(v: u32, p: u32) -> u32 { let x=(v<<1)|p; (x<<1)|(x>>6) }
/// Expand 7-bit endpoint + shared p-bit → 8-bit (just OR the p-bit in).
#[inline] pub fn from_7(v: u32, p: u32) -> u32 { (v<<1)|p }

// ─── 3. BC7 INTERPOLATION & ERROR EVALUATION ─────────────────────────────────

/// Reconstruct a channel value from endpoints and a weight (w in 0..64).
/// Matches the BC7 hardware formula: (lo*(64-w) + hi*w + 32) >> 6.
#[inline] fn bc7_interp(lo: i32, hi: i32, w: u32) -> i32 {
    let d = hi - lo;
    lo + ((d * w as i32 + 32) >> 6)
}

/// Squared error for a single channel.
#[inline] fn bc7_sse1(p: i32, lo: i32, d: i32, w: u32) -> u32 {
    let e = p - (lo + ((d * w as i32 + 32) >> 6));
    (e * e) as u32
}

/// Squared error for RGB channels.
#[inline] fn bc7_sse3(pr: i32, pg: i32, pb: i32,
                       lr: i32, lg: i32, lb: i32,
                       dr: i32, dg: i32, db: i32, w: u32) -> u32 {
    bc7_sse1(pr, lr, dr, w) + bc7_sse1(pg, lg, dg, w) + bc7_sse1(pb, lb, db, w)
}

/// Squared error for RGBA channels.
#[inline] fn bc7_sse4(pr: i32, pg: i32, pb: i32, pa: i32,
                       lr: i32, lg: i32, lb: i32, la: i32,
                       dr: i32, dg: i32, db: i32, da: i32, w: u32) -> u32 {
    bc7_sse1(pr,lr,dr,w) + bc7_sse1(pg,lg,dg,w) + bc7_sse1(pb,lb,db,w) + bc7_sse1(pa,la,da,w)
}

// ─── PCA / COVARIANCE ─────────────────────────────────────────────────────────

/// Power-method eigenvector estimation for a 3×3 covariance matrix.
/// The covariance is stored packed as [rr, rg, rb, gg, gb, bb].
/// Returns the dominant axis (xr, xg, xb).
pub fn dominant_axis_3d(icov: &[i32; 6]) -> (f32, f32, f32) {
    let mut xr = 1.0f32;
    let mut xg = 1.0f32;
    let mut xb = 1.0f32;
    for _ in 0..8 {
        let nr = icov[0] as f32 * xr + icov[1] as f32 * xg + icov[2] as f32 * xb;
        let ng = icov[1] as f32 * xr + icov[3] as f32 * xg + icov[4] as f32 * xb;
        let nb = icov[2] as f32 * xr + icov[4] as f32 * xg + icov[5] as f32 * xb;
        let norm = (nr*nr + ng*ng + nb*nb).sqrt();
        if norm < 1e-8 { break; }
        xr = nr/norm; xg = ng/norm; xb = nb/norm;
    }
    (xr, xg, xb)
}

/// Power-method eigenvector estimation for a 4×4 covariance matrix.
/// Packed: [rr,rg,rb,ra, gg,gb,ga, bb,ba, aa]
pub fn dominant_axis_4d(icov: &[i32; 10]) -> (f32, f32, f32, f32) {
    let mut x = [1.0f32; 4];
    for _ in 0..8 {
        let n = [
            icov[0] as f32*x[0] + icov[1] as f32*x[1] + icov[2] as f32*x[2] + icov[3] as f32*x[3],
            icov[1] as f32*x[0] + icov[4] as f32*x[1] + icov[5] as f32*x[2] + icov[6] as f32*x[3],
            icov[2] as f32*x[0] + icov[5] as f32*x[1] + icov[7] as f32*x[2] + icov[8] as f32*x[3],
            icov[3] as f32*x[0] + icov[6] as f32*x[1] + icov[8] as f32*x[2] + icov[9] as f32*x[3],
        ];
        let norm = (n[0]*n[0]+n[1]*n[1]+n[2]*n[2]+n[3]*n[3]).sqrt();
        if norm < 1e-8 { break; }
        x = [n[0]/norm, n[1]/norm, n[2]/norm, n[3]/norm];
    }
    (x[0], x[1], x[2], x[3])
}

/// Estimate how much error "falls off" the dominant axis.
/// Returns (slam_sse_estimate, ortho_ratio).
/// ortho_ratio > threshold → block benefits from multi-subset encoding.
pub fn estimate_slam_sse_3d(cov: &[f32; 6], xr: f32, xg: f32, xb: f32) -> (f32, f32) {
    let k2 = xr*xr + xg*xg + xb*xb;
    if k2 < 1e-8 { return (0.0, 0.0); }
    let inv = 1.0 / k2;
    let trace = cov[0] + cov[3] + cov[5];
    let axis_e = (xr*(cov[0]*xr + cov[1]*xg + cov[2]*xb)
               + xg*(cov[1]*xr + cov[3]*xg + cov[4]*xb)
               + xb*(cov[2]*xr + cov[4]*xg + cov[5]*xb)) * inv;
    let ortho = (trace - axis_e).max(0.0);
    let ratio = ortho / (axis_e + 1e-8);
    (ortho, ratio)
}

// ─── 4. LEAST-SQUARES ENDPOINT FITTING ───────────────────────────────────────
/// Fit optimal lo/hi for a single channel. Returns None if system is singular.
pub fn ls_fit_1d(n:usize,weights:&[u8],ls_tab:&[[f32;4]],pixels:&[Pixel],comp:usize,total:f32)->Option<(f32,f32)>{
    let(mut z00,mut z10,mut z11,mut q00)=(0.0f32,0.0f32,0.0f32,0.0f32);
    for i in 0..n{let t=&ls_tab[weights[i]as usize];z00+=t[0];z10+=t[1];z11+=t[2];q00+=t[3]*pixels[i][comp]as f32;}
    let q10=total-q00;let det=z00*z11-z10*z10;if det.abs()<1e-8{return None;}
    let inv=1.0/det;let(iz00,iz01,iz10,iz11)=(z11*inv,-z10*inv,-z10*inv,z00*inv);
    Some(((iz10*q00+iz11*q10).clamp(0.0,255.0),(iz00*q00+iz01*q10).clamp(0.0,255.0)))
}
/// Fit optimal lo/hi for RGB simultaneously.
pub fn ls_fit_3d(n:usize,weights:&[u8],ls_tab:&[[f32;4]],pixels:&[Pixel],tr:f32,tg:f32,tb:f32)->Option<([f32;3],[f32;3])>{
    let(mut z00,mut z10,mut z11)=(0.0f32,0.0f32,0.0f32);let(mut qr,mut qg,mut qb)=(0.0f32,0.0f32,0.0f32);
    for i in 0..n{let t=&ls_tab[weights[i]as usize];z00+=t[0];z10+=t[1];z11+=t[2];qr+=t[3]*pixels[i][0]as f32;qg+=t[3]*pixels[i][1]as f32;qb+=t[3]*pixels[i][2]as f32;}
    let det=z00*z11-z10*z10;if det.abs()<1e-8{return None;}
    let inv=1.0/det;let(iz00,iz01,iz10,iz11)=(z11*inv,-z10*inv,-z10*inv,z00*inv);
    Some(([(iz10*qr+iz11*(tr-qr)).clamp(0.0,255.0),(iz10*qg+iz11*(tg-qg)).clamp(0.0,255.0),(iz10*qb+iz11*(tb-qb)).clamp(0.0,255.0)],
          [(iz00*qr+iz01*(tr-qr)).clamp(0.0,255.0),(iz00*qg+iz01*(tg-qg)).clamp(0.0,255.0),(iz00*qb+iz01*(tb-qb)).clamp(0.0,255.0)]))
}
/// Fit optimal lo/hi for RGBA simultaneously.
pub fn ls_fit_4d(n:usize,weights:&[u8],ls_tab:&[[f32;4]],pixels:&[Pixel],tr:f32,tg:f32,tb:f32,ta:f32)->Option<([f32;4],[f32;4])>{
    let(mut z00,mut z10,mut z11)=(0.0f32,0.0f32,0.0f32);let(mut qr,mut qg,mut qb,mut qa)=(0.0f32,0.0f32,0.0f32,0.0f32);
    for i in 0..n{let t=&ls_tab[weights[i]as usize];z00+=t[0];z10+=t[1];z11+=t[2];qr+=t[3]*pixels[i][0]as f32;qg+=t[3]*pixels[i][1]as f32;qb+=t[3]*pixels[i][2]as f32;qa+=t[3]*pixels[i][3]as f32;}
    let det=z00*z11-z10*z10;if det.abs()<1e-8{return None;}
    let inv=1.0/det;let(iz00,iz01,iz10,iz11)=(z11*inv,-z10*inv,-z10*inv,z00*inv);
    Some(([(iz10*qr+iz11*(tr-qr)).clamp(0.0,255.0),(iz10*qg+iz11*(tg-qg)).clamp(0.0,255.0),(iz10*qb+iz11*(tb-qb)).clamp(0.0,255.0),(iz10*qa+iz11*(ta-qa)).clamp(0.0,255.0)],
          [(iz00*qr+iz01*(tr-qr)).clamp(0.0,255.0),(iz00*qg+iz01*(tg-qg)).clamp(0.0,255.0),(iz00*qb+iz01*(tb-qb)).clamp(0.0,255.0),(iz00*qa+iz01*(ta-qa)).clamp(0.0,255.0)]))
}

// ─── 5. WEIGHT EVALUATION ─────────────────────────────────────────────────────
#[inline] fn sse1(p:i32,lo:i32,d:i32,w:u32)->u32{let e=p-(lo+((d*w as i32+32)>>6));(e*e)as u32}
#[inline] fn sse3(px:&Pixel,lr:i32,lg:i32,lb:i32,dr:i32,dg:i32,db:i32,w:u32)->u32{sse1(px[0]as i32,lr,dr,w)+sse1(px[1]as i32,lg,dg,w)+sse1(px[2]as i32,lb,db,w)}
#[inline] fn sse4(px:&Pixel,lr:i32,lg:i32,lb:i32,la:i32,dr:i32,dg:i32,db:i32,da:i32,w:u32)->u32{sse1(px[0]as i32,lr,dr,w)+sse1(px[1]as i32,lg,dg,w)+sse1(px[2]as i32,lb,db,w)+sse1(px[3]as i32,la,da,w)}

#[inline(always)]
fn clamp_weight_sel(sel: i32, max_w: i32) -> i32 {
    if sel as u32 > max_w as u32 { (!sel >> 31) & max_w } else { sel }
}

#[inline(always)]
fn pack_i16x4(a: i32, b: i32, c: i32, d: i32) -> i64 {
    ((a as i16 as u16 as u64)
        | ((b as i16 as u16 as u64) << 16)
        | ((c as i16 as u16 as u64) << 32)
        | ((d as i16 as u16 as u64) << 48)) as i64
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BC7_M6_BACKEND_UNKNOWN: u8 = 0;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BC7_M6_BACKEND_SCALAR: u8 = 1;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BC7_M6_BACKEND_SSE41: u8 = 2;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BC7_M6_BACKEND_AVX2: u8 = 3;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const BC7_M6_BACKEND_AVX512: u8 = 4;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
static BC7_M6_BACKEND: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(BC7_M6_BACKEND_UNKNOWN);

// Runtime x86 dispatch keeps older CPUs on scalar/wide code while selecting the
// fastest available mode-6 selector for SSE4.1, AVX2, or AVX512BW machines.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline(always)]
fn x86_m6_backend() -> u8 {
    let cached = BC7_M6_BACKEND.load(std::sync::atomic::Ordering::Relaxed);
    if cached != BC7_M6_BACKEND_UNKNOWN {
        return cached;
    }

    let backend = if std::is_x86_feature_detected!("avx512f")
        && std::is_x86_feature_detected!("avx512bw")
    {
        BC7_M6_BACKEND_AVX512
    } else if std::is_x86_feature_detected!("avx2") {
        BC7_M6_BACKEND_AVX2
    } else if std::is_x86_feature_detected!("sse4.1") {
        BC7_M6_BACKEND_SSE41
    } else {
        BC7_M6_BACKEND_SCALAR
    };
    BC7_M6_BACKEND.store(backend, std::sync::atomic::Ordering::Relaxed);
    backend
}

// Portable `wide` vectors accelerate four-lane weight selection for all targets
// and remain the fallback when no architecture-specific mode-6 kernel is used.
#[inline(always)]
fn eval_rgb_weights4(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    start: usize,
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
    sofs: i32,
    f: f32,
    max_w: i32,
    weight_tab: &[u32],
) -> u32 {
    let pr = f32x4::from([
        pixels[start][0] as f32,
        pixels[start + 1][0] as f32,
        pixels[start + 2][0] as f32,
        pixels[start + 3][0] as f32,
    ]);
    let pg = f32x4::from([
        pixels[start][1] as f32,
        pixels[start + 1][1] as f32,
        pixels[start + 2][1] as f32,
        pixels[start + 3][1] as f32,
    ]);
    let pb = f32x4::from([
        pixels[start][2] as f32,
        pixels[start + 1][2] as f32,
        pixels[start + 2][2] as f32,
        pixels[start + 3][2] as f32,
    ]);
    let sel_f = (pr * f32x4::splat(dr as f32)
        + pg * f32x4::splat(dg as f32)
        + pb * f32x4::splat(db as f32)
        + f32x4::splat(sofs as f32))
        * f32x4::splat(f)
        + f32x4::splat(0.5);
    let sel_f = sel_f.to_array();
    let mut sse = 0u32;
    for lane in 0..4 {
        let sel = clamp_weight_sel(sel_f[lane] as i32, max_w);
        weights[start + lane] = sel as u8;
        sse += sse3(&pixels[start + lane], lr, lg, lb, dr, dg, db, weight_tab[sel as usize]);
    }
    sse
}

#[inline(always)]
fn eval_rgb_partition_weights4(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    start: usize,
    subsets: [usize; 4],
    el: &[[i32; 3]],
    dlt: &[[i32; 3]],
    sofs: &[i32],
    f: &[f32],
    max_w: i32,
    weight_tab: &[u32],
) -> u32 {
    let pr = f32x4::from([
        pixels[start][0] as f32,
        pixels[start + 1][0] as f32,
        pixels[start + 2][0] as f32,
        pixels[start + 3][0] as f32,
    ]);
    let pg = f32x4::from([
        pixels[start][1] as f32,
        pixels[start + 1][1] as f32,
        pixels[start + 2][1] as f32,
        pixels[start + 3][1] as f32,
    ]);
    let pb = f32x4::from([
        pixels[start][2] as f32,
        pixels[start + 1][2] as f32,
        pixels[start + 2][2] as f32,
        pixels[start + 3][2] as f32,
    ]);
    let dr = f32x4::from([
        dlt[subsets[0]][0] as f32,
        dlt[subsets[1]][0] as f32,
        dlt[subsets[2]][0] as f32,
        dlt[subsets[3]][0] as f32,
    ]);
    let dg = f32x4::from([
        dlt[subsets[0]][1] as f32,
        dlt[subsets[1]][1] as f32,
        dlt[subsets[2]][1] as f32,
        dlt[subsets[3]][1] as f32,
    ]);
    let db = f32x4::from([
        dlt[subsets[0]][2] as f32,
        dlt[subsets[1]][2] as f32,
        dlt[subsets[2]][2] as f32,
        dlt[subsets[3]][2] as f32,
    ]);
    let bias = f32x4::from([
        -(sofs[subsets[0]] as f32),
        -(sofs[subsets[1]] as f32),
        -(sofs[subsets[2]] as f32),
        -(sofs[subsets[3]] as f32),
    ]);
    let scale = f32x4::from([f[subsets[0]], f[subsets[1]], f[subsets[2]], f[subsets[3]]]);
    let sel_f = ((pr * dr + pg * dg + pb * db + bias) * scale + f32x4::splat(0.5)).to_array();
    let mut sse = 0u32;
    for lane in 0..4 {
        let s = subsets[lane];
        let sel = clamp_weight_sel(sel_f[lane] as i32, max_w);
        weights[start + lane] = sel as u8;
        sse += sse3(&pixels[start + lane], el[s][0], el[s][1], el[s][2], dlt[s][0], dlt[s][1], dlt[s][2], weight_tab[sel as usize]);
    }
    sse
}

#[inline(always)]
fn eval_rgba_partition_weights4(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    start: usize,
    subsets: [usize; 4],
    el: &[[i32; 4]],
    dlt: &[[i32; 4]],
    sofs: &[i32],
    f: &[f32],
    max_w: i32,
    weight_tab: &[u32],
) -> u32 {
    let pr = f32x4::from([
        pixels[start][0] as f32,
        pixels[start + 1][0] as f32,
        pixels[start + 2][0] as f32,
        pixels[start + 3][0] as f32,
    ]);
    let pg = f32x4::from([
        pixels[start][1] as f32,
        pixels[start + 1][1] as f32,
        pixels[start + 2][1] as f32,
        pixels[start + 3][1] as f32,
    ]);
    let pb = f32x4::from([
        pixels[start][2] as f32,
        pixels[start + 1][2] as f32,
        pixels[start + 2][2] as f32,
        pixels[start + 3][2] as f32,
    ]);
    let pa = f32x4::from([
        pixels[start][3] as f32,
        pixels[start + 1][3] as f32,
        pixels[start + 2][3] as f32,
        pixels[start + 3][3] as f32,
    ]);
    let dr = f32x4::from([
        dlt[subsets[0]][0] as f32,
        dlt[subsets[1]][0] as f32,
        dlt[subsets[2]][0] as f32,
        dlt[subsets[3]][0] as f32,
    ]);
    let dg = f32x4::from([
        dlt[subsets[0]][1] as f32,
        dlt[subsets[1]][1] as f32,
        dlt[subsets[2]][1] as f32,
        dlt[subsets[3]][1] as f32,
    ]);
    let db = f32x4::from([
        dlt[subsets[0]][2] as f32,
        dlt[subsets[1]][2] as f32,
        dlt[subsets[2]][2] as f32,
        dlt[subsets[3]][2] as f32,
    ]);
    let da = f32x4::from([
        dlt[subsets[0]][3] as f32,
        dlt[subsets[1]][3] as f32,
        dlt[subsets[2]][3] as f32,
        dlt[subsets[3]][3] as f32,
    ]);
    let bias = f32x4::from([
        -(sofs[subsets[0]] as f32),
        -(sofs[subsets[1]] as f32),
        -(sofs[subsets[2]] as f32),
        -(sofs[subsets[3]] as f32),
    ]);
    let scale = f32x4::from([f[subsets[0]], f[subsets[1]], f[subsets[2]], f[subsets[3]]]);
    let sel_f = ((pr * dr + pg * dg + pb * db + pa * da + bias) * scale + f32x4::splat(0.5)).to_array();
    let mut sse = 0u32;
    for lane in 0..4 {
        let s = subsets[lane];
        let sel = clamp_weight_sel(sel_f[lane] as i32, max_w);
        weights[start + lane] = sel as u8;
        sse += sse4(&pixels[start + lane], el[s][0], el[s][1], el[s][2], el[s][3], dlt[s][0], dlt[s][1], dlt[s][2], dlt[s][3], weight_tab[sel as usize]);
    }
    sse
}

// SSE4.1/SSSE3 kernels evaluate four mode-6 pixels at a time using byte shuffles
// for channel extraction and integer multiply/add for reconstructed error.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse4.1,ssse3")]
unsafe fn sse_m6_rgb4_sse41(
    px: __m128i,
    packed_sel: u32,
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
) -> u32 {
    let r_mask = _mm_setr_epi8(0, 4, 8, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let g_mask = _mm_setr_epi8(1, 5, 9, 13, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let b_mask = _mm_setr_epi8(2, 6, 10, 14, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let pr = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, r_mask));
    let pg = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, g_mask));
    let pb = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, b_mask));
    let w = _mm_setr_epi32(
        BC7_WEIGHTS4[(packed_sel & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 8) & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 16) & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 24) & 0xff) as usize] as i32,
    );
    let half = _mm_set1_epi32(32);
    let recon_r = _mm_add_epi32(_mm_set1_epi32(lr), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(dr), w), half), 6));
    let recon_g = _mm_add_epi32(_mm_set1_epi32(lg), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(dg), w), half), 6));
    let recon_b = _mm_add_epi32(_mm_set1_epi32(lb), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(db), w), half), 6));
    let er = _mm_sub_epi32(pr, recon_r);
    let eg = _mm_sub_epi32(pg, recon_g);
    let eb = _mm_sub_epi32(pb, recon_b);
    let err = _mm_add_epi32(_mm_add_epi32(_mm_mullo_epi32(er, er), _mm_mullo_epi32(eg, eg)), _mm_mullo_epi32(eb, eb));
    let sum2 = _mm_add_epi32(err, _mm_shuffle_epi32(err, 0b10_11_00_01));
    let sum4 = _mm_add_epi32(sum2, _mm_shuffle_epi32(sum2, 0b01_00_11_10));
    _mm_cvtsi128_si32(sum4) as u32
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse4.1,ssse3")]
unsafe fn sse_m6_rgba4_sse41(
    px: __m128i,
    packed_sel: u32,
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
) -> u32 {
    let r_mask = _mm_setr_epi8(0, 4, 8, 12, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let g_mask = _mm_setr_epi8(1, 5, 9, 13, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let b_mask = _mm_setr_epi8(2, 6, 10, 14, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let a_mask = _mm_setr_epi8(3, 7, 11, 15, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1);
    let pr = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, r_mask));
    let pg = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, g_mask));
    let pb = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, b_mask));
    let pa = _mm_cvtepu8_epi32(_mm_shuffle_epi8(px, a_mask));
    let w = _mm_setr_epi32(
        BC7_WEIGHTS4[(packed_sel & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 8) & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 16) & 0xff) as usize] as i32,
        BC7_WEIGHTS4[((packed_sel >> 24) & 0xff) as usize] as i32,
    );
    let half = _mm_set1_epi32(32);
    let recon_r = _mm_add_epi32(_mm_set1_epi32(lr), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(dr), w), half), 6));
    let recon_g = _mm_add_epi32(_mm_set1_epi32(lg), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(dg), w), half), 6));
    let recon_b = _mm_add_epi32(_mm_set1_epi32(lb), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(db), w), half), 6));
    let recon_a = _mm_add_epi32(_mm_set1_epi32(la), _mm_srai_epi32(_mm_add_epi32(_mm_mullo_epi32(_mm_set1_epi32(da), w), half), 6));
    let er = _mm_sub_epi32(pr, recon_r);
    let eg = _mm_sub_epi32(pg, recon_g);
    let eb = _mm_sub_epi32(pb, recon_b);
    let ea = _mm_sub_epi32(pa, recon_a);
    let err = _mm_add_epi32(_mm_add_epi32(_mm_mullo_epi32(er, er), _mm_mullo_epi32(eg, eg)), _mm_add_epi32(_mm_mullo_epi32(eb, eb), _mm_mullo_epi32(ea, ea)));
    let sum2 = _mm_add_epi32(err, _mm_shuffle_epi32(err, 0b10_11_00_01));
    let sum4 = _mm_add_epi32(sum2, _mm_shuffle_epi32(sum2, 0b01_00_11_10));
    _mm_cvtsi128_si32(sum4) as u32
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse4.1,ssse3")]
unsafe fn eval_m6_rgb_sse41(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
    f: f32,
) -> u32 {
    let zero = _mm_setzero_si128();
    let fifteen = _mm_set1_epi32(15);
    let f = _mm_set1_ps(f);
    let half = _mm_set1_ps(0.5);
    let ep = _mm_set1_epi64x(pack_i16x4(lr, lg, lb, 0));
    let coef = _mm_set1_epi64x(pack_i16x4(dr, dg, db, 0));
    let mut sse = 0u32;

    for i in (0..16).step_by(4) {
        let px = _mm_loadu_si128(pixels.as_ptr().add(i) as *const __m128i);
        let lo16 = _mm_unpacklo_epi8(px, zero);
        let hi16 = _mm_unpackhi_epi8(px, zero);
        let lo_adj = _mm_sub_epi16(lo16, ep);
        let hi_adj = _mm_sub_epi16(hi16, ep);
        let lo32p = _mm_madd_epi16(lo_adj, coef);
        let hi32p = _mm_madd_epi16(hi_adj, coef);
        let lo_sum = _mm_add_epi32(lo32p, _mm_shuffle_epi32(lo32p, 0b10_11_00_01));
        let hi_sum = _mm_add_epi32(hi32p, _mm_shuffle_epi32(hi32p, 0b10_11_00_01));
        let pair01 = _mm_shuffle_epi32(lo_sum, 0b10_00_10_00);
        let pair23 = _mm_shuffle_epi32(hi_sum, 0b10_00_10_00);
        let dot32 = _mm_unpacklo_epi64(pair01, pair23);
        let y = _mm_add_ps(_mm_mul_ps(_mm_cvtepi32_ps(dot32), f), half);
        let sel32 = _mm_min_epi32(_mm_max_epi32(_mm_cvttps_epi32(y), zero), fifteen);
        let sel16 = _mm_packs_epi32(sel32, zero);
        let sel8 = _mm_packus_epi16(sel16, zero);
        let packed = _mm_cvtsi128_si32(sel8) as u32;

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u32, packed);
        sse += sse_m6_rgb4_sse41(px, packed, lr, lg, lb, dr, dg, db);
    }

    sse
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse4.1,ssse3")]
unsafe fn eval_m6_rgba_sse41(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
    f: f32,
) -> u32 {
    let zero = _mm_setzero_si128();
    let fifteen = _mm_set1_epi32(15);
    let f = _mm_set1_ps(f);
    let half = _mm_set1_ps(0.5);
    let ep = _mm_set1_epi64x(pack_i16x4(lr, lg, lb, la));
    let coef = _mm_set1_epi64x(pack_i16x4(dr, dg, db, da));
    let mut sse = 0u32;

    for i in (0..16).step_by(4) {
        let px = _mm_loadu_si128(pixels.as_ptr().add(i) as *const __m128i);
        let lo16 = _mm_unpacklo_epi8(px, zero);
        let hi16 = _mm_unpackhi_epi8(px, zero);
        let lo_adj = _mm_sub_epi16(lo16, ep);
        let hi_adj = _mm_sub_epi16(hi16, ep);
        let lo32p = _mm_madd_epi16(lo_adj, coef);
        let hi32p = _mm_madd_epi16(hi_adj, coef);
        let lo_sum = _mm_add_epi32(lo32p, _mm_shuffle_epi32(lo32p, 0b10_11_00_01));
        let hi_sum = _mm_add_epi32(hi32p, _mm_shuffle_epi32(hi32p, 0b10_11_00_01));
        let pair01 = _mm_shuffle_epi32(lo_sum, 0b10_00_10_00);
        let pair23 = _mm_shuffle_epi32(hi_sum, 0b10_00_10_00);
        let dot32 = _mm_unpacklo_epi64(pair01, pair23);
        let y = _mm_add_ps(_mm_mul_ps(_mm_cvtepi32_ps(dot32), f), half);
        let sel32 = _mm_min_epi32(_mm_max_epi32(_mm_cvttps_epi32(y), zero), fifteen);
        let sel16 = _mm_packs_epi32(sel32, zero);
        let sel8 = _mm_packus_epi16(sel16, zero);
        let packed = _mm_cvtsi128_si32(sel8) as u32;

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u32, packed);
        sse += sse_m6_rgba4_sse41(px, packed, lr, lg, lb, la, dr, dg, db, da);
    }

    sse
}

// AArch64 NEON kernels mirror the SSE4.1 path for four mode-6 pixels at a time.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn sse_m6_rgb4_neon(
    px: uint8x16_t,
    w: int32x4_t,
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
) -> u32 {
    const R_MASK: [u8; 16] = [0, 4, 8, 12, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    const G_MASK: [u8; 16] = [1, 5, 9, 13, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    const B_MASK: [u8; 16] = [2, 6, 10, 14, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    let pr = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(R_MASK.as_ptr())))))));
    let pg = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(G_MASK.as_ptr())))))));
    let pb = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(B_MASK.as_ptr())))))));
    let half = vdupq_n_s32(32);
    let recon_r = vaddq_s32(vdupq_n_s32(lr), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(dr), w), half)));
    let recon_g = vaddq_s32(vdupq_n_s32(lg), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(dg), w), half)));
    let recon_b = vaddq_s32(vdupq_n_s32(lb), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(db), w), half)));
    let er = vsubq_s32(pr, recon_r);
    let eg = vsubq_s32(pg, recon_g);
    let eb = vsubq_s32(pb, recon_b);
    vaddvq_s32(vaddq_s32(vaddq_s32(vmulq_s32(er, er), vmulq_s32(eg, eg)), vmulq_s32(eb, eb))) as u32
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn sse_m6_rgba4_neon(
    px: uint8x16_t,
    w: int32x4_t,
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
) -> u32 {
    const R_MASK: [u8; 16] = [0, 4, 8, 12, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    const G_MASK: [u8; 16] = [1, 5, 9, 13, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    const B_MASK: [u8; 16] = [2, 6, 10, 14, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    const A_MASK: [u8; 16] = [3, 7, 11, 15, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255];
    let pr = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(R_MASK.as_ptr())))))));
    let pg = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(G_MASK.as_ptr())))))));
    let pb = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(B_MASK.as_ptr())))))));
    let pa = vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(vmovl_u8(vget_low_u8(vqtbl1q_u8(px, vld1q_u8(A_MASK.as_ptr())))))));
    let half = vdupq_n_s32(32);
    let recon_r = vaddq_s32(vdupq_n_s32(lr), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(dr), w), half)));
    let recon_g = vaddq_s32(vdupq_n_s32(lg), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(dg), w), half)));
    let recon_b = vaddq_s32(vdupq_n_s32(lb), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(db), w), half)));
    let recon_a = vaddq_s32(vdupq_n_s32(la), vshrq_n_s32::<6>(vaddq_s32(vmulq_s32(vdupq_n_s32(da), w), half)));
    let er = vsubq_s32(pr, recon_r);
    let eg = vsubq_s32(pg, recon_g);
    let eb = vsubq_s32(pb, recon_b);
    let ea = vsubq_s32(pa, recon_a);
    vaddvq_s32(vaddq_s32(vaddq_s32(vmulq_s32(er, er), vmulq_s32(eg, eg)), vaddq_s32(vmulq_s32(eb, eb), vmulq_s32(ea, ea)))) as u32
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn eval_m6_rgb_neon(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
    f: f32,
) -> u32 {
    let ep = vreinterpretq_s16_u64(vdupq_n_u64(pack_i16x4(lr, lg, lb, 0) as u64));
    let coef = vreinterpretq_s16_u64(vdupq_n_u64(pack_i16x4(dr, dg, db, 0) as u64));
    let mut sse = 0u32;

    for i in (0..16).step_by(4) {
        let px = vld1q_u8(pixels.as_ptr().add(i) as *const u8);
        let lo = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(px))), ep);
        let hi = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(px))), ep);
        let prod0 = vmull_s16(vget_low_s16(lo), vget_low_s16(coef));
        let prod1 = vmull_s16(vget_high_s16(lo), vget_high_s16(coef));
        let prod2 = vmull_s16(vget_low_s16(hi), vget_low_s16(coef));
        let prod3 = vmull_s16(vget_high_s16(hi), vget_high_s16(coef));
        let dots = [
            vaddvq_s32(prod0),
            vaddvq_s32(prod1),
            vaddvq_s32(prod2),
            vaddvq_s32(prod3),
        ];
        let mut w = vdupq_n_s32(0);

        for lane in 0..4 {
            let dot = dots[lane];
            let sel = clamp_weight_sel((dot as f32 * f + 0.5) as i32, 15) as usize;
            weights[i + lane] = sel as u8;
            w = match lane {
                0 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 0),
                1 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 1),
                2 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 2),
                _ => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 3),
            };
        }
        sse += sse_m6_rgb4_neon(px, w, lr, lg, lb, dr, dg, db);
    }

    sse
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn eval_m6_rgba_neon(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
    f: f32,
) -> u32 {
    let ep = vreinterpretq_s16_u64(vdupq_n_u64(pack_i16x4(lr, lg, lb, la) as u64));
    let coef = vreinterpretq_s16_u64(vdupq_n_u64(pack_i16x4(dr, dg, db, da) as u64));
    let mut sse = 0u32;

    for i in (0..16).step_by(4) {
        let px = vld1q_u8(pixels.as_ptr().add(i) as *const u8);
        let lo = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(px))), ep);
        let hi = vsubq_s16(vreinterpretq_s16_u16(vmovl_u8(vget_high_u8(px))), ep);
        let prod0 = vmull_s16(vget_low_s16(lo), vget_low_s16(coef));
        let prod1 = vmull_s16(vget_high_s16(lo), vget_high_s16(coef));
        let prod2 = vmull_s16(vget_low_s16(hi), vget_low_s16(coef));
        let prod3 = vmull_s16(vget_high_s16(hi), vget_high_s16(coef));
        let dots = [
            vaddvq_s32(prod0),
            vaddvq_s32(prod1),
            vaddvq_s32(prod2),
            vaddvq_s32(prod3),
        ];
        let mut w = vdupq_n_s32(0);

        for lane in 0..4 {
            let dot = dots[lane];
            let sel = clamp_weight_sel((dot as f32 * f + 0.5) as i32, 15) as usize;
            weights[i + lane] = sel as u8;
            w = match lane {
                0 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 0),
                1 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 1),
                2 => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 2),
                _ => vsetq_lane_s32(BC7_WEIGHTS4[sel] as i32, w, 3),
            };
        }
        sse += sse_m6_rgba4_neon(px, w, lr, lg, lb, la, dr, dg, db, da);
    }

    sse
}

// AVX512F/BW handles eight mode-6 weight selections at once, then reuses the
// SSE4.1 four-pixel error kernels for final reconstruction error.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx512f")]
unsafe fn select_m6_avx512(pairs: __m512i, f: f32) -> u64 {
    let pair_sums = _mm512_add_epi32(pairs, _mm512_shuffle_epi32(pairs, 0b10_11_00_01));
    let dots = _mm512_permutexvar_epi32(
        _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0),
        pair_sums,
    );
    let y = _mm512_add_ps(_mm512_mul_ps(_mm512_cvtepi32_ps(dots), _mm512_set1_ps(f)), _mm512_set1_ps(0.5));
    let sel32 = _mm512_min_epi32(_mm512_max_epi32(_mm512_cvttps_epi32(y), _mm512_setzero_si512()), _mm512_set1_epi32(15));
    let sel8 = _mm512_cvtusepi32_epi8(sel32);
    _mm_cvtsi128_si64(sel8) as u64
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn eval_m6_rgba_avx512(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
    f: f32,
) -> u32 {
    let ep = _mm512_set1_epi64(pack_i16x4(lr, lg, lb, la));
    let coef = _mm512_set1_epi64(pack_i16x4(dr, dg, db, da));
    let mut sse = 0u32;

    for i in (0..16).step_by(8) {
        let px = _mm256_loadu_si256(pixels.as_ptr().add(i) as *const __m256i);
        let px16 = _mm512_cvtepu8_epi16(px);
        let adj = _mm512_sub_epi16(px16, ep);
        let pairs = _mm512_madd_epi16(adj, coef);
        let packed = select_m6_avx512(pairs, f);

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u64, packed);
        let packed0 = packed as u32;
        let packed1 = (packed >> 32) as u32;
        sse += sse_m6_rgba4_sse41(_mm256_castsi256_si128(px), packed0, lr, lg, lb, la, dr, dg, db, da);
        sse += sse_m6_rgba4_sse41(_mm256_extracti128_si256::<1>(px), packed1, lr, lg, lb, la, dr, dg, db, da);
    }

    sse
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn eval_m6_rgb_avx512(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
    f: f32,
) -> u32 {
    let ep = _mm512_set1_epi64(pack_i16x4(lr, lg, lb, 0));
    let coef = _mm512_set1_epi64(pack_i16x4(dr, dg, db, 0));
    let mut sse = 0u32;

    for i in (0..16).step_by(8) {
        let px = _mm256_loadu_si256(pixels.as_ptr().add(i) as *const __m256i);
        let px16 = _mm512_cvtepu8_epi16(px);
        let adj = _mm512_sub_epi16(px16, ep);
        let pairs = _mm512_madd_epi16(adj, coef);
        let packed = select_m6_avx512(pairs, f);

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u64, packed);
        let packed0 = packed as u32;
        let packed1 = (packed >> 32) as u32;
        sse += sse_m6_rgb4_sse41(_mm256_castsi256_si128(px), packed0, lr, lg, lb, dr, dg, db);
        sse += sse_m6_rgb4_sse41(_mm256_extracti128_si256::<1>(px), packed1, lr, lg, lb, dr, dg, db);
    }

    sse
}

// AVX2 handles eight mode-6 weight selections at once on CPUs without AVX512.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn eval_m6_rgba_avx2(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
    f: f32,
) -> u32 {
    let zero128 = _mm_setzero_si128();
    let fifteen256 = _mm256_set1_epi32(15);
    let f = _mm_set1_ps(f);
    let half = _mm_set1_ps(0.5);
    let ep = _mm256_set1_epi64x(pack_i16x4(lr, lg, lb, la));
    let coef = _mm256_set1_epi64x(pack_i16x4(dr, dg, db, da));
    let mut sse = 0u32;

    for i in (0..16).step_by(8) {
        let px0 = _mm_loadu_si128(pixels.as_ptr().add(i) as *const __m128i);
        let px1 = _mm_loadu_si128(pixels.as_ptr().add(i + 4) as *const __m128i);
        let adj0 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(px0), ep);
        let adj1 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(px1), ep);
        let pairs0 = _mm256_madd_epi16(adj0, coef);
        let pairs1 = _mm256_madd_epi16(adj1, coef);
        let sums0 = _mm256_hadd_epi32(pairs0, pairs0);
        let sums1 = _mm256_hadd_epi32(pairs1, pairs1);
        let dots0 = _mm_unpacklo_epi64(_mm256_castsi256_si128(sums0), _mm256_extracti128_si256::<1>(sums0));
        let dots1 = _mm_unpacklo_epi64(_mm256_castsi256_si128(sums1), _mm256_extracti128_si256::<1>(sums1));
        let dots = _mm256_set_m128i(dots1, dots0);
        let y = _mm256_add_ps(_mm256_mul_ps(_mm256_cvtepi32_ps(dots), _mm256_set_m128(f, f)), _mm256_set_m128(half, half));
        let sel32 = _mm256_min_epi32(_mm256_max_epi32(_mm256_cvttps_epi32(y), _mm256_setzero_si256()), fifteen256);
        let sel16 = _mm_packus_epi32(_mm256_castsi256_si128(sel32), _mm256_extracti128_si256::<1>(sel32));
        let sel8 = _mm_packus_epi16(sel16, zero128);
        let packed = _mm_cvtsi128_si64(sel8) as u64;

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u64, packed);
        let packed0 = packed as u32;
        let packed1 = (packed >> 32) as u32;
        sse += sse_m6_rgba4_sse41(px0, packed0, lr, lg, lb, la, dr, dg, db, da);
        sse += sse_m6_rgba4_sse41(px1, packed1, lr, lg, lb, la, dr, dg, db, da);
    }

    sse
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn eval_m6_rgb_avx2(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    dr: i32,
    dg: i32,
    db: i32,
    f: f32,
) -> u32 {
    let zero128 = _mm_setzero_si128();
    let fifteen256 = _mm256_set1_epi32(15);
    let f = _mm_set1_ps(f);
    let half = _mm_set1_ps(0.5);
    let ep = _mm256_set1_epi64x(pack_i16x4(lr, lg, lb, 0));
    let coef = _mm256_set1_epi64x(pack_i16x4(dr, dg, db, 0));
    let mut sse = 0u32;

    for i in (0..16).step_by(8) {
        let px0 = _mm_loadu_si128(pixels.as_ptr().add(i) as *const __m128i);
        let px1 = _mm_loadu_si128(pixels.as_ptr().add(i + 4) as *const __m128i);
        let adj0 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(px0), ep);
        let adj1 = _mm256_sub_epi16(_mm256_cvtepu8_epi16(px1), ep);
        let pairs0 = _mm256_madd_epi16(adj0, coef);
        let pairs1 = _mm256_madd_epi16(adj1, coef);
        let sums0 = _mm256_hadd_epi32(pairs0, pairs0);
        let sums1 = _mm256_hadd_epi32(pairs1, pairs1);
        let dots0 = _mm_unpacklo_epi64(_mm256_castsi256_si128(sums0), _mm256_extracti128_si256::<1>(sums0));
        let dots1 = _mm_unpacklo_epi64(_mm256_castsi256_si128(sums1), _mm256_extracti128_si256::<1>(sums1));
        let dots = _mm256_set_m128i(dots1, dots0);
        let y = _mm256_add_ps(_mm256_mul_ps(_mm256_cvtepi32_ps(dots), _mm256_set_m128(f, f)), _mm256_set_m128(half, half));
        let sel32 = _mm256_min_epi32(_mm256_max_epi32(_mm256_cvttps_epi32(y), _mm256_setzero_si256()), fifteen256);
        let sel16 = _mm_packus_epi32(_mm256_castsi256_si128(sel32), _mm256_extracti128_si256::<1>(sel32));
        let sel8 = _mm_packus_epi16(sel16, zero128);
        let packed = _mm_cvtsi128_si64(sel8) as u64;

        std::ptr::write_unaligned(weights.as_mut_ptr().add(i) as *mut u64, packed);
        let packed0 = packed as u32;
        let packed1 = (packed >> 32) as u32;
        sse += sse_m6_rgb4_sse41(px0, packed0, lr, lg, lb, dr, dg, db);
        sse += sse_m6_rgb4_sse41(px1, packed1, lr, lg, lb, dr, dg, db);
    }

    sse
}

#[inline(always)]
fn eval_rgba_weights4(
    pixels: &[Pixel; 16],
    weights: &mut [u8; 16],
    start: usize,
    lr: i32,
    lg: i32,
    lb: i32,
    la: i32,
    dr: i32,
    dg: i32,
    db: i32,
    da: i32,
    sofs: i32,
    f: f32,
    max_w: i32,
    weight_tab: &[u32],
) -> u32 {
    let pr = f32x4::from([
        pixels[start][0] as f32,
        pixels[start + 1][0] as f32,
        pixels[start + 2][0] as f32,
        pixels[start + 3][0] as f32,
    ]);
    let pg = f32x4::from([
        pixels[start][1] as f32,
        pixels[start + 1][1] as f32,
        pixels[start + 2][1] as f32,
        pixels[start + 3][1] as f32,
    ]);
    let pb = f32x4::from([
        pixels[start][2] as f32,
        pixels[start + 1][2] as f32,
        pixels[start + 2][2] as f32,
        pixels[start + 3][2] as f32,
    ]);
    let pa = f32x4::from([
        pixels[start][3] as f32,
        pixels[start + 1][3] as f32,
        pixels[start + 2][3] as f32,
        pixels[start + 3][3] as f32,
    ]);
    let sel_f = (pr * f32x4::splat(dr as f32)
        + pg * f32x4::splat(dg as f32)
        + pb * f32x4::splat(db as f32)
        + pa * f32x4::splat(da as f32)
        + f32x4::splat(sofs as f32))
        * f32x4::splat(f)
        + f32x4::splat(0.5);
    let sel_f = sel_f.to_array();
    let mut sse = 0u32;
    for lane in 0..4 {
        let sel = clamp_weight_sel(sel_f[lane] as i32, max_w);
        weights[start + lane] = sel as u8;
        sse += sse4(&pixels[start + lane], lr, lg, lb, la, dr, dg, db, da, weight_tab[sel as usize]);
    }
    sse
}

pub fn eval_m6_rgb(pixels:&[Pixel;16],weights:&mut[u8;16],lr:i32,lg:i32,lb:i32,hr:i32,hg:i32,hb:i32,p0:u32,p1:u32)->u32{
    let(lr,lg,lb)=(from_7(lr as u32,p0)as i32,from_7(lg as u32,p0)as i32,from_7(lb as u32,p0)as i32);
    let(hr,hg,hb)=(from_7(hr as u32,p1)as i32,from_7(hg as u32,p1)as i32,from_7(hb as u32,p1)as i32);
    let(dr,dg,db)=(hr-lr,hg-lg,hb-lb);let f=15.0/((dr*dr+dg*dg+db*db)as f32+1.25e-7);
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    match x86_m6_backend() {
        BC7_M6_BACKEND_AVX512 => return unsafe { eval_m6_rgb_avx512(pixels, weights, lr, lg, lb, dr, dg, db, f) },
        BC7_M6_BACKEND_AVX2 => return unsafe { eval_m6_rgb_avx2(pixels, weights, lr, lg, lb, dr, dg, db, f) },
        BC7_M6_BACKEND_SSE41 => return unsafe { eval_m6_rgb_sse41(pixels, weights, lr, lg, lb, dr, dg, db, f) },
        _ => {}
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { eval_m6_rgb_neon(pixels, weights, lr, lg, lb, dr, dg, db, f) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let sofs=-(lr*dr+lg*dg+lb*db);
        let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgb_weights4(pixels,weights,i,lr,lg,lb,dr,dg,db,sofs,f,15,&BC7_WEIGHTS4);}sse
    }}

pub fn eval_m6_rgba(pixels:&[Pixel;16],weights:&mut[u8;16],lr:i32,lg:i32,lb:i32,la:i32,p0:u32,hr:i32,hg:i32,hb:i32,ha:i32,p1:u32)->u32{
    let(lr,lg,lb,la)=(from_7(lr as u32,p0)as i32,from_7(lg as u32,p0)as i32,from_7(lb as u32,p0)as i32,from_7(la as u32,p0)as i32);
    let(hr,hg,hb,ha)=(from_7(hr as u32,p1)as i32,from_7(hg as u32,p1)as i32,from_7(hb as u32,p1)as i32,from_7(ha as u32,p1)as i32);
    let(dr,dg,db,da)=(hr-lr,hg-lg,hb-lb,ha-la);let f=15.0/((dr*dr+dg*dg+db*db+da*da)as f32+1.25e-7);
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    match x86_m6_backend() {
        BC7_M6_BACKEND_AVX512 => return unsafe { eval_m6_rgba_avx512(pixels, weights, lr, lg, lb, la, dr, dg, db, da, f) },
        BC7_M6_BACKEND_AVX2 => return unsafe { eval_m6_rgba_avx2(pixels, weights, lr, lg, lb, la, dr, dg, db, da, f) },
        BC7_M6_BACKEND_SSE41 => return unsafe { eval_m6_rgba_sse41(pixels, weights, lr, lg, lb, la, dr, dg, db, da, f) },
        _ => {}
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { eval_m6_rgba_neon(pixels, weights, lr, lg, lb, la, dr, dg, db, da, f) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
    let sofs=-(lr*dr+lg*dg+lb*db+la*da);
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgba_weights4(pixels,weights,i,lr,lg,lb,la,dr,dg,db,da,sofs,f,15,&BC7_WEIGHTS4);}sse
    }}

pub fn eval_m1(pixels:&[Pixel;16],weights:&mut[u8;16],lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],pbits:&[u32;2],bmask:u16)->u32{
    let mut el=[[0i32;3];2];let mut dlt=[[0i32;3];2];let mut f=[0.0f32;2];let mut sofs=[0i32;2];
    for s in 0..2{el[s][0]=from_6(lr[s],pbits[s])as i32;let ehr=from_6(hr[s],pbits[s])as i32;el[s][1]=from_6(lg[s],pbits[s])as i32;let ehg=from_6(hg[s],pbits[s])as i32;el[s][2]=from_6(lb[s],pbits[s])as i32;let ehb=from_6(hb[s],pbits[s])as i32;dlt[s]=[ehr-el[s][0],ehg-el[s][1],ehb-el[s][2]];let d2=dlt[s][0]*dlt[s][0]+dlt[s][1]*dlt[s][1]+dlt[s][2]*dlt[s][2];f[s]=7.0/(d2 as f32+1.25e-7);sofs[s]=el[s][0]*dlt[s][0]+el[s][1]*dlt[s][1]+el[s][2]*dlt[s][2];}
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgb_partition_weights4(pixels,weights,i,[((bmask>>i)&1)as usize,((bmask>>(i+1))&1)as usize,((bmask>>(i+2))&1)as usize,((bmask>>(i+3))&1)as usize],&el,&dlt,&sofs,&f,7,&BC7_WEIGHTS3);}sse}

pub fn eval_m3(pixels:&[Pixel;16],weights:&mut[u8;16],lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],pbits:&[u32;4],bmask:u16)->u32{
    let mut el=[[0i32;3];2];let mut dlt=[[0i32;3];2];let mut f=[0.0f32;2];let mut sofs=[0i32;2];
    for s in 0..2{el[s][0]=from_7(lr[s],pbits[s*2])as i32;let ehr=from_7(hr[s],pbits[s*2+1])as i32;el[s][1]=from_7(lg[s],pbits[s*2])as i32;let ehg=from_7(hg[s],pbits[s*2+1])as i32;el[s][2]=from_7(lb[s],pbits[s*2])as i32;let ehb=from_7(hb[s],pbits[s*2+1])as i32;dlt[s]=[ehr-el[s][0],ehg-el[s][1],ehb-el[s][2]];let d2=dlt[s][0]*dlt[s][0]+dlt[s][1]*dlt[s][1]+dlt[s][2]*dlt[s][2];f[s]=3.0/(d2 as f32+1.25e-7);sofs[s]=el[s][0]*dlt[s][0]+el[s][1]*dlt[s][1]+el[s][2]*dlt[s][2];}
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgb_partition_weights4(pixels,weights,i,[((bmask>>i)&1)as usize,((bmask>>(i+1))&1)as usize,((bmask>>(i+2))&1)as usize,((bmask>>(i+3))&1)as usize],&el,&dlt,&sofs,&f,3,&BC7_WEIGHTS2);}sse}

pub fn eval_m7(pixels:&[Pixel;16],weights:&mut[u8;16],lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],la:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],ha:&[u32;2],pbits:&[u32;4],bmask:u16)->u32{
    let mut el=[[0i32;4];2];let mut dlt=[[0i32;4];2];let mut f=[0.0f32;2];let mut sofs=[0i32;2];
    for s in 0..2{el[s][0]=from_5p(lr[s],pbits[s*2])as i32;let ehr=from_5p(hr[s],pbits[s*2+1])as i32;el[s][1]=from_5p(lg[s],pbits[s*2])as i32;let ehg=from_5p(hg[s],pbits[s*2+1])as i32;el[s][2]=from_5p(lb[s],pbits[s*2])as i32;let ehb=from_5p(hb[s],pbits[s*2+1])as i32;el[s][3]=from_5p(la[s],pbits[s*2])as i32;let eha=from_5p(ha[s],pbits[s*2+1])as i32;dlt[s]=[ehr-el[s][0],ehg-el[s][1],ehb-el[s][2],eha-el[s][3]];let d2:i32=dlt[s].iter().map(|&x|x*x).sum();f[s]=3.0/(d2 as f32+1.25e-7);sofs[s]=dlt[s].iter().zip(el[s].iter()).map(|(&d,&e)|d*e).sum();}
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgba_partition_weights4(pixels,weights,i,[((bmask>>i)&1)as usize,((bmask>>(i+1))&1)as usize,((bmask>>(i+2))&1)as usize,((bmask>>(i+3))&1)as usize],&el,&dlt,&sofs,&f,3,&BC7_WEIGHTS2);}sse}

pub fn eval_m0(pixels:&[Pixel;16],weights:&mut[u8;16],lr:&[u32;3],lg:&[u32;3],lb:&[u32;3],hr:&[u32;3],hg:&[u32;3],hb:&[u32;3],pbits:&[u32;6],pat_id:usize)->u32{
    let pm=&BC7_PARTITION3[pat_id*16..(pat_id+1)*16];
    let mut el=[[0i32;3];3];let mut dlt=[[0i32;3];3];let mut f=[0.0f32;3];let mut sofs=[0i32;3];
    for s in 0..3{el[s][0]=from_4(lr[s],pbits[s*2])as i32;let ehr=from_4(hr[s],pbits[s*2+1])as i32;el[s][1]=from_4(lg[s],pbits[s*2])as i32;let ehg=from_4(hg[s],pbits[s*2+1])as i32;el[s][2]=from_4(lb[s],pbits[s*2])as i32;let ehb=from_4(hb[s],pbits[s*2+1])as i32;dlt[s]=[ehr-el[s][0],ehg-el[s][1],ehb-el[s][2]];let d2=dlt[s][0]*dlt[s][0]+dlt[s][1]*dlt[s][1]+dlt[s][2]*dlt[s][2];f[s]=7.0/(d2 as f32+1.25e-7);sofs[s]=el[s][0]*dlt[s][0]+el[s][1]*dlt[s][1]+el[s][2]*dlt[s][2];}
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgb_partition_weights4(pixels,weights,i,[pm[i]as usize,pm[i+1]as usize,pm[i+2]as usize,pm[i+3]as usize],&el,&dlt,&sofs,&f,7,&BC7_WEIGHTS3);}sse}

pub fn eval_m2(pixels:&[Pixel;16],weights:&mut[u8;16],lr:&[u32;3],lg:&[u32;3],lb:&[u32;3],hr:&[u32;3],hg:&[u32;3],hb:&[u32;3],pat_id:usize)->u32{
    let pm=&BC7_PARTITION3[pat_id*16..(pat_id+1)*16];
    let mut el=[[0i32;3];3];let mut dlt=[[0i32;3];3];let mut f=[0.0f32;3];let mut sofs=[0i32;3];
    for s in 0..3{el[s][0]=from_5(lr[s])as i32;let ehr=from_5(hr[s])as i32;el[s][1]=from_5(lg[s])as i32;let ehg=from_5(hg[s])as i32;el[s][2]=from_5(lb[s])as i32;let ehb=from_5(hb[s])as i32;dlt[s]=[ehr-el[s][0],ehg-el[s][1],ehb-el[s][2]];let d2=dlt[s][0]*dlt[s][0]+dlt[s][1]*dlt[s][1]+dlt[s][2]*dlt[s][2];f[s]=3.0/(d2 as f32+1.25e-7);sofs[s]=el[s][0]*dlt[s][0]+el[s][1]*dlt[s][1]+el[s][2]*dlt[s][2];}
    let mut sse=0u32;for i in (0..16).step_by(4){sse+=eval_rgb_partition_weights4(pixels,weights,i,[pm[i]as usize,pm[i+1]as usize,pm[i+2]as usize,pm[i+3]as usize],&el,&dlt,&sofs,&f,3,&BC7_WEIGHTS2);}sse}

// ─── 6. ENCODE FUNCTIONS (Modes 0–7) ─────────────────────────────────────────
// Each function takes quantized endpoints, p-bits, weights, and writes a 16-byte BC7 block.
// Critical BC7 rule: the "anchor" pixel weight must have its MSB = 0.
// If it doesn't, we swap lo/hi endpoints and XOR all weights with the max weight value.

/// Mode 0: 3 subsets, 4-bit endpoints + unique p-bits, 3-bit weights.
/// part_id is 4-bit (0..15). Layout: 1+4+72+6+bits bits.
pub fn encode_mode0(block:&mut[u8;16],pat:u32,lr:&[u32;3],lg:&[u32;3],lb:&[u32;3],hr:&[u32;3],hg:&[u32;3],hb:&[u32;3],p:&[u32;6],w:&[u8;16]){
    let pm=&BC7_PARTITION3[pat as usize*16..pat as usize*16+16];
    let anc1=BC7_ANCHOR_THIRD_SUBSET1[pat as usize]as usize;
    let anc2=BC7_ANCHOR_THIRD_SUBSET2[pat as usize]as usize;
    // Inversion: for each subset, if anchor pixel weight MSB is set, swap lo/hi
    let mut lr=*lr;let mut lg=*lg;let mut lb=*lb;let mut hr=*hr;let mut hg=*hg;let mut hb=*hb;
    let mut p=*p;let w=*w;let mut inv=[0u8;3];
    if w[0]&4!=0{inv[0]=7;std::mem::swap(&mut lr[0],&mut hr[0]);std::mem::swap(&mut lg[0],&mut hg[0]);std::mem::swap(&mut lb[0],&mut hb[0]);{let _t=p[0];p[0]=p[1];p[1]=_t;};}
    if w[anc1]&4!=0{inv[1]=7;std::mem::swap(&mut lr[1],&mut hr[1]);std::mem::swap(&mut lg[1],&mut hg[1]);std::mem::swap(&mut lb[1],&mut hb[1]);{let _t=p[2];p[2]=p[3];p[3]=_t;};}
    if w[anc2]&4!=0{inv[2]=7;std::mem::swap(&mut lr[2],&mut hr[2]);std::mem::swap(&mut lg[2],&mut hg[2]);std::mem::swap(&mut lb[2],&mut hb[2]);{let _t=p[4];p[4]=p[5];p[5]=_t;};}
    // Pack 128 bits: mode(1)+part(4)+endpoints(72)+pbits(6)+weights(45)
    let lo=1u64|(pat as u64)<<1|(lr[0]as u64)<<5|(hr[0]as u64)<<9|(lr[1]as u64)<<13|(hr[1]as u64)<<17|(lr[2]as u64)<<21|(hr[2]as u64)<<25|(lg[0]as u64)<<29|(hg[0]as u64)<<33|(lg[1]as u64)<<37|(hg[1]as u64)<<41|(lg[2]as u64)<<45|(hg[2]as u64)<<49|(lb[0]as u64)<<53|(hb[0]as u64)<<57|(lb[1]as u64)<<61;
    block[0..8].copy_from_slice(&lo.to_le_bytes());
    let mut hi=(lb[1]>>3)as u64|(hb[1]as u64)<<1|(lb[2]as u64)<<5|(hb[2]as u64)<<9|(p[0]as u64)<<13|(p[1]as u64)<<14|(p[2]as u64)<<15|(p[3]as u64)<<16|(p[4]as u64)<<17|(p[5]as u64)<<18;
    let mut ofs=19usize;
    for i in 0..16{let s=pm[i]as usize;let ww=(w[i]^inv[s])as u64;hi|=ww<<ofs;ofs+=3-((i==0||i==anc1||i==anc2)as usize);}
    block[8..16].copy_from_slice(&hi.to_le_bytes());
}

/// Mode 1: 2 subsets, 6-bit endpoints + shared p-bits (2 p-bits), 3-bit weights.
pub fn encode_mode1(block:&mut[u8;16],pat:u32,lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],p0:u32,p1:u32,w:&[u8;16]){
    let pm=&BC7_PARTITION2[pat as usize*16..pat as usize*16+16];
    let anc=BC7_ANCHOR_SECOND_SUBSET[pat as usize]as usize;
    let mut lr=*lr;let mut lg=*lg;let mut lb=*lb;let mut hr=*hr;let mut hg=*hg;let mut hb=*hb;
    let(mut p0,mut p1)=(p0,p1);let w=*w;let mut inv=[0u8;2];
    if w[0]&4!=0{inv[0]=7;for c in 0..2{std::mem::swap(&mut lr[c],&mut hr[c]);std::mem::swap(&mut lg[c],&mut hg[c]);std::mem::swap(&mut lb[c],&mut hb[c]);}std::mem::swap(&mut p0,&mut p1);}
    // Note: for Mode 1 there is only ONE anchor for subset 1 (pixel 0 is implicit anchor for subset 0)
    if w[anc]&4!=0{inv[1]=7;std::mem::swap(&mut lr[1],&mut hr[1]);std::mem::swap(&mut lg[1],&mut hg[1]);std::mem::swap(&mut lb[1],&mut hb[1]);}
    block[0]=(0b10|(pat<<2))as u8;
    let x=lr[0]as u64|(hr[0]as u64)<<6|(lr[1]as u64)<<12|(hr[1]as u64)<<18|(lg[0]as u64)<<24|(hg[0]as u64)<<30|(lg[1]as u64)<<36|(hg[1]as u64)<<42|(lb[0]as u64)<<48|(hb[0]as u64)<<54|(lb[1]as u64)<<60;
    block[1..9].copy_from_slice(&x.to_le_bytes());
    block[9]=((lb[1]>>4)|(hb[1]<<2))as u8;
    let mut y=p0 as u64|(p1 as u64)<<1;let mut ofs=2usize;
    for i in 0..16{let s=pm[i]as usize;let ww=(w[i]^inv[s])as u64;y|=ww<<ofs;ofs+=3-((i==0||i==anc)as usize);}
    block[10..16].copy_from_slice(&y.to_le_bytes()[0..6]);
}

/// Mode 2: 3 subsets, 5-bit endpoints, no p-bits, 2-bit weights.
pub fn encode_mode2(block:&mut[u8;16],pat:u32,lr:&[u32;3],lg:&[u32;3],lb:&[u32;3],hr:&[u32;3],hg:&[u32;3],hb:&[u32;3],w:&[u8;16]){
    let pm=&BC7_PARTITION3[pat as usize*16..pat as usize*16+16];
    let anc1=BC7_ANCHOR_THIRD_SUBSET1[pat as usize]as usize;
    let anc2=BC7_ANCHOR_THIRD_SUBSET2[pat as usize]as usize;
    let mut lr=*lr;let mut lg=*lg;let mut lb=*lb;let mut hr=*hr;let mut hg=*hg;let mut hb=*hb;let w=*w;let mut inv=[0u8;3];
    if w[0]&2!=0{inv[0]=3;for c in 0..3{std::mem::swap(&mut lr[c],&mut hr[c]);std::mem::swap(&mut lg[c],&mut hg[c]);std::mem::swap(&mut lb[c],&mut hb[c]);}}
    if w[anc1]&2!=0{inv[1]=3;std::mem::swap(&mut lr[1],&mut hr[1]);std::mem::swap(&mut lg[1],&mut hg[1]);std::mem::swap(&mut lb[1],&mut hb[1]);}
    if w[anc2]&2!=0{inv[2]=3;std::mem::swap(&mut lr[2],&mut hr[2]);std::mem::swap(&mut lg[2],&mut hg[2]);std::mem::swap(&mut lb[2],&mut hb[2]);}
    let v=0b100u64|(pat as u64)<<3|(lr[0]as u64)<<9|(hr[0]as u64)<<14|(lr[1]as u64)<<19|(hr[1]as u64)<<24|(lr[2]as u64)<<29|(hr[2]as u64)<<34|(lg[0]as u64)<<39|(hg[0]as u64)<<44|(lg[1]as u64)<<49|(hg[1]as u64)<<54|(lg[2]as u64)<<59;
    block[0..8].copy_from_slice(&v.to_le_bytes());
    let mut v1=hg[2]as u64|(lb[0]as u64)<<5|(hb[0]as u64)<<10|(lb[1]as u64)<<15|(hb[1]as u64)<<20|(lb[2]as u64)<<25|(hb[2]as u64)<<30;
    block[8..12].copy_from_slice(&v1.to_le_bytes()[0..4]);v1>>=32;let mut ofs=3usize;
    for i in 0..16{let s=pm[i]as usize;let ww=(w[i]^inv[s])as u64;v1|=ww<<ofs;ofs+=2-((i==0||i==anc1||i==anc2)as usize);}
    block[12..16].copy_from_slice(&v1.to_le_bytes()[0..4]);
}

/// Mode 3: 2 subsets, 7-bit endpoints + unique p-bits (4 p-bits), 2-bit weights.
pub fn encode_mode3(block:&mut[u8;16],pat:u32,lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],p:&[u32;4],w:&[u8;16]){
    let pm=&BC7_PARTITION2[pat as usize*16..pat as usize*16+16];
    let anc=BC7_ANCHOR_SECOND_SUBSET[pat as usize]as usize;
    let mut lr=*lr;let mut lg=*lg;let mut lb=*lb;let mut hr=*hr;let mut hg=*hg;let mut hb=*hb;let mut p=*p;let w=*w;let mut inv=[0u8;2];
    if w[0]&2!=0{inv[0]=3;std::mem::swap(&mut lr[0],&mut hr[0]);std::mem::swap(&mut lg[0],&mut hg[0]);std::mem::swap(&mut lb[0],&mut hb[0]);{let _t=p[0];p[0]=p[1];p[1]=_t;};}
    if w[anc]&2!=0{inv[1]=3;std::mem::swap(&mut lr[1],&mut hr[1]);std::mem::swap(&mut lg[1],&mut hg[1]);std::mem::swap(&mut lb[1],&mut hb[1]);{let _t=p[2];p[2]=p[3];p[3]=_t;};}
    let x=0b1000u64|(pat as u64)<<4|(lr[0]as u64)<<10|(hr[0]as u64)<<17|(lr[1]as u64)<<24|(hr[1]as u64)<<31|(lg[0]as u64)<<38|(hg[0]as u64)<<45|(lg[1]as u64)<<52|(hg[1]as u64)<<59;
    block[0..8].copy_from_slice(&x.to_le_bytes());
    let mut y=(hg[1]>>5)as u64|(lb[0]as u64)<<2|(hb[0]as u64)<<9|(lb[1]as u64)<<16|(hb[1]as u64)<<23|(p[0]as u64)<<30|(p[1]as u64)<<31|(p[2]as u64)<<32|(p[3]as u64)<<33;
    let mut ofs=34usize;
    for i in 0..16{let s=pm[i]as usize;let ww=(w[i]^inv[s])as u64;y|=ww<<ofs;ofs+=2-((i==0||i==anc)as usize);}
    block[8..16].copy_from_slice(&y.to_le_bytes());
}

/// Mode 6: 1 subset, 7-bit RGBA endpoints + shared p-bits (2), 4-bit weights.
pub fn encode_mode6(block:&mut[u8;16],lr:u32,lg:u32,lb:u32,la:u32,p0:u32,hr:u32,hg:u32,hb:u32,ha:u32,p1:u32,w:&[u8;16]){
    let mut lr=lr;let mut lg=lg;let mut lb=lb;let mut la=la;let mut hr=hr;let mut hg=hg;let mut hb=hb;let mut ha=ha;let mut p0=p0;let mut p1=p1;let w=*w;
    // If anchor weight (pixel 0) has MSB set, swap endpoints so anchor weight becomes <8
    let mut inv=0u8;if w[0]&8!=0{inv=15;std::mem::swap(&mut lr,&mut hr);std::mem::swap(&mut lg,&mut hg);std::mem::swap(&mut lb,&mut hb);std::mem::swap(&mut la,&mut ha);std::mem::swap(&mut p0,&mut p1);}
    let x=0b1000000u64|(lr as u64)<<7|(hr as u64)<<14|(lg as u64)<<21|(hg as u64)<<28|(lb as u64)<<35|(hb as u64)<<42|(la as u64)<<49|(ha as u64)<<56;
    block[0..7].copy_from_slice(&x.to_le_bytes()[0..7]);block[7]=(x>>56)as u8|(p0 as u8)<<7;
    let mut y=p1 as u64;let mut ofs=1usize;
    for i in 0..16{let ww=(w[i]^inv)as u64;y|=ww<<ofs;ofs+=3+(i>0)as usize;}
    block[8..16].copy_from_slice(&y.to_le_bytes());
}

/// Mode 7: 2 subsets, 5-bit RGBA endpoints + unique p-bits (4), 2-bit weights.
pub fn encode_mode7(block:&mut[u8;16],pat:u32,lr:&[u32;2],lg:&[u32;2],lb:&[u32;2],la:&[u32;2],hr:&[u32;2],hg:&[u32;2],hb:&[u32;2],ha:&[u32;2],p:&[u32;4],w:&[u8;16]){
    let pm=&BC7_PARTITION2[pat as usize*16..pat as usize*16+16];
    let anc=BC7_ANCHOR_SECOND_SUBSET[pat as usize]as usize;
    let mut lr=*lr;let mut lg=*lg;let mut lb=*lb;let mut la=*la;
    let mut hr=*hr;let mut hg=*hg;let mut hb=*hb;let mut ha=*ha;let mut p=*p;let w=*w;let mut inv=[0u8;2];
    if w[0]&2!=0{inv[0]=3;std::mem::swap(&mut lr[0],&mut hr[0]);std::mem::swap(&mut lg[0],&mut hg[0]);std::mem::swap(&mut lb[0],&mut hb[0]);std::mem::swap(&mut la[0],&mut ha[0]);{let _t=p[0];p[0]=p[1];p[1]=_t;};}
    if w[anc]&2!=0{inv[1]=3;std::mem::swap(&mut lr[1],&mut hr[1]);std::mem::swap(&mut lg[1],&mut hg[1]);std::mem::swap(&mut lb[1],&mut hb[1]);std::mem::swap(&mut la[1],&mut ha[1]);{let _t=p[2];p[2]=p[3];p[3]=_t;};}
    let x=0x80u64|(pat as u64)<<8|(lr[0]as u64)<<14|(hr[0]as u64)<<19|(lr[1]as u64)<<24|(hr[1]as u64)<<29|(lg[0]as u64)<<34|(hg[0]as u64)<<39|(lg[1]as u64)<<44|(hg[1]as u64)<<49|(lb[0]as u64)<<54|(hb[0]as u64)<<59;
    block[0..8].copy_from_slice(&x.to_le_bytes());
    let mut y=(lb[1]as u64)|(hb[1]as u64)<<5|(la[0]as u64)<<10|(ha[0]as u64)<<15|(la[1]as u64)<<20|(ha[1]as u64)<<25|(p[0]as u64)<<30|(p[1]as u64)<<31|(p[2]as u64)<<32|(p[3]as u64)<<33;
    let mut ofs=34usize;
    for i in 0..16{let s=pm[i]as usize;let ww=(w[i]^inv[s])as u64;y|=ww<<ofs;ofs+=2-((i==0||i==anc)as usize);}
    block[8..16].copy_from_slice(&y.to_le_bytes());
}

// ─── 7. MODE SELECTION & MAIN ENTRY POINTS ────────────────────────────────────
//
// Mode selection mirrors the C++ `fast_pack_bc7_rgb_analytical` and
// `fast_pack_bc7_rgba_analytical` functions.  The decision tree is:
//
//   1. All pixels equal → Mode 5 solid (lossless for any RGBA value)
//   2. Compute block statistics: mean, covariance, dominant axis, ortho_ratio
//   3. Very low variance → trivial Mode 6 (only two distinct pixels on line)
//   4. Has alpha channel variance:
//      - Try Mode 7 (2-subset RGBA) when ortho_ratio is high
//      - Try Mode 6 RGBA with 4D LS refinement
//   5. RGB only (or alpha uniform):
//      - mode6_slam_sse  → used as the SSE budget for multi-subset modes
//      - High ortho_ratio AND ortho_ratio>threshold → try Mode 1/3 (2-subset)
//      - Very high ortho_ratio → also try Mode 0/2 (3-subset)
//      - Default: Mode 6 RGB

const TRIVIAL_BLOCK_VAR: i32 = 20 * 16;   // below this → trivial mode 6
const ORTHO_2SUBSET: f32 = 0.004;          // above this → try 2-subset modes
const ORTHO_3SUBSET: f32 = 0.020;          // above this → also try 3-subset modes
const DP_STRONG_CORR: f32 = 0.80;          // for dual-plane channel detection
const PART2_SEARCH_CANDIDATES: usize = 12;
const PART3_SEARCH_CANDIDATES: usize = 8;

/// Solid-block fast path using Mode 5 — lossless for any RGBA value.
///
/// Mode 5 uses precomputed optimal (lo, hi) endpoint pairs (7-bit each) for RGB,
/// and stores alpha as a raw 8-bit direct endpoint (la = ha = c[3]).
/// All 16 color weights are set to 1 (second weight out of {0,1,2,3}), and all
/// alpha weights are also set to 1, which reconstructs the original color exactly.
///
/// Reference: basist::g_bc7_mode_5_optimal_endpoints + pack_mode5_solid().
fn encode_solid_block(block: &mut [u8; 16], c: &Pixel) {
    let (lr, hr) = { let e = BC7_MODE5_OPTIMAL_ENDPOINTS[c[0] as usize]; (e.0 as u64, e.1 as u64) };
    let (lg, hg) = { let e = BC7_MODE5_OPTIMAL_ENDPOINTS[c[1] as usize]; (e.0 as u64, e.1 as u64) };
    let (lb, hb) = { let e = BC7_MODE5_OPTIMAL_ENDPOINTS[c[2] as usize]; (e.0 as u64, e.1 as u64) };
    let a = c[3] as u64;  // 8-bit alpha stored directly
    // Mode 5 bit 5 = 1, rotation = 0 (bits 6-7 = 0)
    // Layout low 64 bits: mode(6b)+rot(2b)+lr(7b)+hr(7b)+lg(7b)+hg(7b)+lb(7b)+hb(7b)+la(8b)+ha(8b)
    //   total = 6+2+42+16 = 66 → overflows into high word by 2 bits
    let low: u64 = (1u64 << 5)   // mode bit 5 = Mode 5
        | (lr << 8) | (hr << 15)
        | (lg << 22) | (hg << 29)
        | (lb << 36) | (hb << 43)
        | (a  << 50) | (a  << 58);
    block[0..8].copy_from_slice(&low.to_le_bytes());
    // High 64 bits: 2 leftover bits from second alpha (ha bits 6-7), then weights.
    // Color weights: anchor(1b)=1, then 15×2b=01 each → 0b0101...0101_01 << 2
    // Alpha weights: same pattern after color weights.
    // Using the C++ reference tail: 0xac,0xaa,0xaa,0xaa,0,0,0,0
    // This encodes: 2 leftover bits | anchor_c=1 | 15×01 | anchor_a=1 | 15×01
    //   = 2b(ha>>6) | 31b(color weights) | 31b(alpha weights)
    // The tail for weight=1 for all 16 pixels in both planes from the reference:
    let ha_overflow = (a >> 6) & 3;
    // Color plane: pixel0 anchor (1-bit)=1, then 15×(2-bit)=01, packed = 0b 01_01...01_1 = 0xAAA...AC (31 bits)
    // Alpha plane: same pattern
    // high = ha_overflow | (0xAAAAAAAAAC << 2 leftover for color) | (0xAAAAAAAAAC << 33)
    // Precomputed pattern: 0xAC | 0xAA×3 | 0×4 as the C++ reference uses
    let color_weights: u64 = 0b_01_01_01_01_01_01_01_01_01_01_01_01_01_01_01_1;  // 31 bits
    let alpha_weights: u64 = color_weights;
    let high = ha_overflow | (color_weights << 2) | (alpha_weights << 33);
    block[8..16].copy_from_slice(&high.to_le_bytes());
}

/// Pick the 2-subset partition whose assignment most closely matches the target
/// binary split defined by projecting pixels onto the dominant axis.
fn pick_best_part2(pixels: &[Pixel; 16], xr: f32, xg: f32, xb: f32) -> (usize, u16) {
    // Project all pixels onto dominant axis and split at midpoint
    let mut dots = [0.0f32; 16];
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for i in 0..16 {
        dots[i] = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb;
        mn = mn.min(dots[i]); mx = mx.max(dots[i]);
    }
    let split = (mn + mx) * 0.5;
    let mut desired = 0u16;
    for i in 0..16 { if dots[i] > split { desired |= 1 << i; } }

    // Find the partition with minimum Hamming distance to desired assignment.
    // Also check the inverted assignment (popcount trick from C++ reference).
    let mut best_pat = 0usize;
    let mut best_diff = u32::MAX;
    for p in 0..64 {
        let bmask = part2_bitmask(p);
        let diff = (bmask ^ desired).count_ones();
        let diff_inv = 16 - diff;
        let min_diff = diff.min(diff_inv);
        if min_diff < best_diff {
            best_diff = min_diff;
            best_pat = p;
        }
    }
    let bmask = part2_bitmask(best_pat);
    (best_pat, bmask)
}

/// Pick the 3-subset partition (Modes 0/2) using 4D projection (RGB+luma).
fn pick_best_part3(pixels: &[Pixel; 16], xr: f32, xg: f32, xb: f32) -> usize {
    let mut dots = [0.0f32; 16];
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for i in 0..16 {
        dots[i] = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb;
        mn = mn.min(dots[i]); mx = mx.max(dots[i]);
    }
    let range = (mx - mn).max(1e-8);
    // Quantize dots into 3 buckets → desired subset assignment
    let mut desired = [0u8; 16];
    for i in 0..16 {
        let q = ((dots[i] - mn) / range * 2.999) as usize;
        desired[i] = q.min(2) as u8;
    }
    let mut best_pat = 0usize;
    let mut best_diff = u32::MAX;
    for p in 0..64 {
        let pm = &BC7_PARTITION3[p*16..p*16+16];
        let diff: u32 = (0..16).map(|i| (pm[i] != desired[i]) as u32).sum();
        if diff < best_diff { best_diff = diff; best_pat = p; }
    }
    best_pat
}

#[inline(always)]
fn insert_partition_candidate<const N: usize>(
    candidates: &mut [usize; N],
    scores: &mut [u32; N],
    count: &mut usize,
    pat: usize,
    score: u32,
) {
    if *count == N && score >= scores[N - 1] {
        return;
    }
    let mut pos = 0usize;
    while pos < *count && scores[pos] <= score {
        pos += 1;
    }
    if pos == N {
        return;
    }
    let end = (*count).min(N - 1);
    for i in (pos..end).rev() {
        candidates[i + 1] = candidates[i];
        scores[i + 1] = scores[i];
    }
    candidates[pos] = pat;
    scores[pos] = score;
    *count = (*count + 1).min(N);
}

fn part2_candidates(
    pixels: &[Pixel; 16],
    xr: f32,
    xg: f32,
    xb: f32,
) -> ([usize; PART2_SEARCH_CANDIDATES], usize) {
    let mut dots = [0.0f32; 16];
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for i in 0..16 {
        dots[i] = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb;
        mn = mn.min(dots[i]); mx = mx.max(dots[i]);
    }
    let split = (mn + mx) * 0.5;
    let mut desired = 0u16;
    for i in 0..16 { if dots[i] > split { desired |= 1 << i; } }

    let mut candidates = [0usize; PART2_SEARCH_CANDIDATES];
    let mut scores = [u32::MAX; PART2_SEARCH_CANDIDATES];
    let mut count = 0usize;
    for pat in 0..64usize {
        let diff = (part2_bitmask(pat) ^ desired).count_ones();
        insert_partition_candidate(
            &mut candidates,
            &mut scores,
            &mut count,
            pat,
            diff.min(16 - diff),
        );
    }
    (candidates, count)
}

fn part3_candidates(
    pixels: &[Pixel; 16],
    xr: f32,
    xg: f32,
    xb: f32,
) -> ([usize; PART3_SEARCH_CANDIDATES], usize) {
    const PERMS: [[u8; 3]; 6] = [
        [0, 1, 2], [0, 2, 1],
        [1, 0, 2], [1, 2, 0],
        [2, 0, 1], [2, 1, 0],
    ];
    let mut dots = [0.0f32; 16];
    let (mut mn, mut mx) = (f32::MAX, f32::MIN);
    for i in 0..16 {
        dots[i] = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb;
        mn = mn.min(dots[i]); mx = mx.max(dots[i]);
    }
    let range = (mx - mn).max(1e-8);
    let mut desired = [0u8; 16];
    for i in 0..16 {
        let q = ((dots[i] - mn) / range * 2.999) as usize;
        desired[i] = q.min(2) as u8;
    }

    let mut candidates = [0usize; PART3_SEARCH_CANDIDATES];
    let mut scores = [u32::MAX; PART3_SEARCH_CANDIDATES];
    let mut count = 0usize;
    for pat in 0..64usize {
        let pm = &BC7_PARTITION3[pat*16..pat*16+16];
        let mut best_diff = u32::MAX;
        for perm in PERMS {
            let diff: u32 = (0..16).map(|i| (perm[pm[i] as usize] != desired[i]) as u32).sum();
            best_diff = best_diff.min(diff);
        }
        insert_partition_candidate(&mut candidates, &mut scores, &mut count, pat, best_diff);
    }
    (candidates, count)
}

// ─── LS REFINEMENT HELPERS ───────────────────────────────────────────────────

/// Collect pixels belonging to `subset` and run a 3D LS fit.
/// Returns (lo_6bit, hi_6bit) for Mode 1 (6-bit endpoints).
/// The LS table used matches the weight bit-depth of the mode.
fn ls_refine_subset_3d(
    pixels: &[Pixel; 16], weights: &[u8; 16],
    p_map: &[u8], subset: usize,
    ls_tab: &[[f32; 4]],
) -> Option<([f32; 3], [f32; 3])> {
    let mut sp = [[0u8; 4]; 16];
    let mut sw = [0u8; 16];
    let mut n = 0;
    let mut tr = 0.0f32; let mut tg = 0.0f32; let mut tb = 0.0f32;
    for i in 0..16 {
        if p_map[i] as usize == subset {
            sp[n] = pixels[i]; sw[n] = weights[i];
            tr += pixels[i][0] as f32;
            tg += pixels[i][1] as f32;
            tb += pixels[i][2] as f32;
            n += 1;
        }
    }
    if n == 0 { return None; }
    ls_fit_3d(n, &sw[..n], ls_tab, &sp[..n], tr, tg, tb)
}

/// Try all 4 shared p-bit combinations {p0,p1} for Mode 1 (6-bit+shared pbit)
/// and return the combo with the lowest SSE.
fn best_pbits_m1(
    pixels: &[Pixel; 16], weights: &mut [u8; 16],
    lr: &mut [u32;2], lg: &mut [u32;2], lb: &mut [u32;2],
    hr: &mut [u32;2], hg: &mut [u32;2], hb: &mut [u32;2],
    bmask: u16,
) -> [u32; 2] {
    let mut best_pbits = [0u32; 2];
    let mut best_sse = u32::MAX;
    for p0 in 0u32..2 {
        for p1 in 0u32..2 {
            let pbits = [p0, p1];
            let mut tw = [0u8; 16];
            let sse = eval_m1(pixels, &mut tw, lr, lg, lb, hr, hg, hb, &pbits, bmask);
            if sse < best_sse {
                best_sse = sse;
                best_pbits = pbits;
                *weights = tw;
            }
        }
    }
    best_pbits
}

/// Try all 4 unique p-bit combinations {p_lo0,p_hi0,p_lo1,p_hi1} for Mode 3
/// (7-bit+unique pbit) and return the combo with lowest SSE.
fn best_pbits_m3(
    pixels: &[Pixel; 16], weights: &mut [u8; 16],
    lr: &mut [u32;2], lg: &mut [u32;2], lb: &mut [u32;2],
    hr: &mut [u32;2], hg: &mut [u32;2], hb: &mut [u32;2],
    bmask: u16,
) -> [u32; 4] {
    let mut best_pbits = [0u32; 4];
    let mut best_sse = u32::MAX;
    for p in 0u32..16 {
        let pbits = [(p>>0)&1, (p>>1)&1, (p>>2)&1, (p>>3)&1];
        let mut tw = [0u8; 16];
        let sse = eval_m3(pixels, &mut tw, lr, lg, lb, hr, hg, hb, &pbits, bmask);
        if sse < best_sse {
            best_sse = sse;
            best_pbits = pbits;
            *weights = tw;
        }
    }
    best_pbits
}

/// Initialise Mode 1 (6-bit, shared pbit) endpoints for one subset
/// from pixel min/max projected onto the dominant axis.
fn init_endpoints_m1_subset(
    pixels: &[Pixel; 16], p_map: &[u8], subset: usize,
    xr: f32, xg: f32, xb: f32,
) -> ([u32;3], [u32;3]) {
    let (mut lo_dot, mut hi_dot) = (f32::MAX, f32::MIN);
    let (mut lo_i, mut hi_i) = (0usize, 0usize);
    for i in 0..16 {
        if p_map[i] as usize != subset { continue; }
        let d = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb + i as f32*1e-4;
        if d < lo_dot { lo_dot=d; lo_i=i; }
        if d > hi_dot { hi_dot=d; hi_i=i; }
    }
    let p = &pixels[lo_i];
    let q = &pixels[hi_i];
    // 6-bit quantisation using p=0 (p-bits optimised later)
    ([to_6p(p[0] as i32, 0) as u32, to_6p(p[1] as i32, 0) as u32, to_6p(p[2] as i32, 0) as u32],
     [to_6p(q[0] as i32, 0) as u32, to_6p(q[1] as i32, 0) as u32, to_6p(q[2] as i32, 0) as u32])
}

/// Same for Mode 3 (7-bit unique pbit).
fn init_endpoints_m3_subset(
    pixels: &[Pixel; 16], p_map: &[u8], subset: usize,
    xr: f32, xg: f32, xb: f32,
) -> ([u32;3], [u32;3]) {
    let (mut lo_dot, mut hi_dot) = (f32::MAX, f32::MIN);
    let (mut lo_i, mut hi_i) = (0usize, 0usize);
    for i in 0..16 {
        if p_map[i] as usize != subset { continue; }
        let d = pixels[i][0] as f32*xr + pixels[i][1] as f32*xg + pixels[i][2] as f32*xb + i as f32*1e-4;
        if d < lo_dot { lo_dot=d; lo_i=i; }
        if d > hi_dot { hi_dot=d; hi_i=i; }
    }
    let p = &pixels[lo_i];
    let q = &pixels[hi_i];
    ([to_7p(p[0] as i32, 0) as u32, to_7p(p[1] as i32, 0) as u32, to_7p(p[2] as i32, 0) as u32],
     [to_7p(q[0] as i32, 0) as u32, to_7p(q[1] as i32, 0) as u32, to_7p(q[2] as i32, 0) as u32])
}

// ─── PUBLIC ENTRY POINTS ──────────────────────────────────────────────────────

/// Main entry for RGB blocks. Selects the best BC7 mode and writes a 16-byte block.
/// `flags` controls which modes are attempted (see FLAG_* constants).
pub fn pack_bc7_rgb(block: &mut [u8; 16], pixels: &[Pixel; 16], flags: u32) {
    // 1. Solid block fast-path: check if all pixels are identical
    if pixels.iter().all(|p| p == &pixels[0]) {
        encode_solid_block(block, &pixels[0]);
        return;
    }

    // 2. Compute block statistics
    let (mut tr, mut tg, mut tb) = (0i32, 0i32, 0i32);
    let (mut min_r, mut max_r) = (255i32, 0i32);
    let (mut min_g, mut max_g) = (255i32, 0i32);
    let (mut min_b, mut max_b) = (255i32, 0i32);
    for p in pixels.iter() {
        tr += p[0] as i32; tg += p[1] as i32; tb += p[2] as i32;
        min_r = min_r.min(p[0] as i32); max_r = max_r.max(p[0] as i32);
        min_g = min_g.min(p[1] as i32); max_g = max_g.max(p[1] as i32);
        min_b = min_b.min(p[2] as i32); max_b = max_b.max(p[2] as i32);
    }
    let mean_r = (tr + 8) >> 4;
    let mean_g = (tg + 8) >> 4;
    let mean_b = (tb + 8) >> 4;
    let mut icov = [0i32; 6];
    for p in pixels.iter() {
        let r = p[0] as i32 - mean_r;
        let g = p[1] as i32 - mean_g;
        let b = p[2] as i32 - mean_b;
        icov[0]+=r*r; icov[1]+=r*g; icov[2]+=r*b;
        icov[3]+=g*g; icov[4]+=g*b; icov[5]+=b*b;
    }
    let block_max_var = icov[0].max(icov[3]).max(icov[5]);
    if block_max_var == 0 {
        encode_solid_block(block, &pixels[0]);
        return;
    }

    // 3. PCA: dominant axis and orthogonal energy estimate
    let (xr, xg, xb) = dominant_axis_3d(&icov);
    let cov_f = [icov[0] as f32, icov[1] as f32, icov[2] as f32,
                 icov[3] as f32, icov[4] as f32, icov[5] as f32];
    let (slam_sse, ortho_ratio) = estimate_slam_sse_3d(&cov_f, xr, xg, xb);
    let _ = slam_sse; // used implicitly as SSE budget threshold

    // 4. Trivial Mode 6: very low variance, endpoints are min/max pixels
    if (flags & FLAG_USE_TRIVIAL_M6) != 0 && block_max_var < TRIVIAL_BLOCK_VAR {
        // Find lo/hi pixel by projecting onto dominant axis (same as C++ reference)
        let saxis_r = (xr * 2048.0) as i32;
        let saxis_g = (xg * 2048.0) as i32;
        let saxis_b = (xb * 2048.0) as i32;
        let (mut lo_dot, mut hi_dot) = (i32::MAX, i32::MIN);
        let (mut lo_c, mut hi_c) = (0usize, 0usize);
        for i in 0..16 {
            let dot = pixels[i][0] as i32 * saxis_r + pixels[i][1] as i32 * saxis_g + pixels[i][2] as i32 * saxis_b + i as i32;
            if dot < lo_dot { lo_dot = dot; lo_c = i; }
            if dot > hi_dot { hi_dot = dot; hi_c = i; }
        }
        let lr = to_7f(pixels[lo_c][0] as f32) as u32;
        let lg = to_7f(pixels[lo_c][1] as f32) as u32;
        let lb = to_7f(pixels[lo_c][2] as f32) as u32;
        let hr = to_7f(pixels[hi_c][0] as f32) as u32;
        let hg = to_7f(pixels[hi_c][1] as f32) as u32;
        let hb = to_7f(pixels[hi_c][2] as f32) as u32;
        let la = 127u32; let ha = 127u32;
        let mut w = [0u8; 16];
        eval_m6_rgb(pixels, &mut w, lr as i32, lg as i32, lb as i32, hr as i32, hg as i32, hb as i32, 1, 1);
        encode_mode6(block, lr, lg, lb, la, 1, hr, hg, hb, ha, 1, &w);
        return;
    }

    // 5. Mode 6 baseline (used as the SSE budget for multi-subset modes)
    let mut best_block = [0u8; 16];
    let mut best_sse = pack_bc7_rgb_mode6_sse(&mut best_block, pixels, tr, tg, tb, xr, xg, xb);

    // 6. Try 2-subset modes (Modes 1 and 3) — exhaustive partition search over all 64 partitions.
    // Only attempted when ortho_ratio suggests the block has meaningful off-axis energy.
    if (flags & FLAG_USE_2SUBSETS) != 0 && block_max_var >= 64 * 16 && ortho_ratio > ORTHO_2SUBSET {
        // Try both Mode 1 (6-bit endpoints, 3-bit weights, shared p-bit)
        // and Mode 3 (7-bit endpoints, 2-bit weights, unique p-bits) for each partition.
        let (partitions, partition_count) = part2_candidates(pixels, xr, xg, xb);
        for &pat in partitions[..partition_count].iter() {
            let bmask = part2_bitmask(pat);
            let pm = &BC7_PARTITION2[pat * 16..pat * 16 + 16];

            // ── Mode 1 ──
            {
                let mut lr = [0u32;2]; let mut lg = [0u32;2]; let mut lb = [0u32;2];
                let mut hr = [0u32;2]; let mut hg = [0u32;2]; let mut hb = [0u32;2];
                // Init endpoints from axis projection per-subset
                for s in 0..2 {
                    let (lo, hi) = init_endpoints_m1_subset(pixels, pm, s, xr, xg, xb);
                    lr[s]=lo[0]; lg[s]=lo[1]; lb[s]=lo[2];
                    hr[s]=hi[0]; hg[s]=hi[1]; hb[s]=hi[2];
                }
                let mut w = [0u8; 16];
                // Initial weight evaluation
                let mut pbits = [0u32; 2];
                eval_m1(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits, bmask);
                // LS refinement pass (weights → better endpoints)
                for s in 0..2 {
                    if let Some((lo, hi)) = ls_refine_subset_3d(pixels, &w, pm, s, &LS_TAB3) {
                        lr[s]=to_6p(rnd(lo[0]).clamp(0,255),0)as u32;
                        lg[s]=to_6p(rnd(lo[1]).clamp(0,255),0)as u32;
                        lb[s]=to_6p(rnd(lo[2]).clamp(0,255),0)as u32;
                        hr[s]=to_6p(rnd(hi[0]).clamp(0,255),0)as u32;
                        hg[s]=to_6p(rnd(hi[1]).clamp(0,255),0)as u32;
                        hb[s]=to_6p(rnd(hi[2]).clamp(0,255),0)as u32;
                    }
                }
                // Optimise p-bits over all 4 combinations
                pbits = best_pbits_m1(pixels, &mut w, &mut lr, &mut lg, &mut lb, &mut hr, &mut hg, &mut hb, bmask);
                // Second LS pass with optimised p-bits
                for s in 0..2 {
                    if let Some((lo, hi)) = ls_refine_subset_3d(pixels, &w, pm, s, &LS_TAB3) {
                        lr[s]=to_6p(rnd(lo[0]).clamp(0,255),pbits[s] as i32)as u32;
                        lg[s]=to_6p(rnd(lo[1]).clamp(0,255),pbits[s] as i32)as u32;
                        lb[s]=to_6p(rnd(lo[2]).clamp(0,255),pbits[s] as i32)as u32;
                        hr[s]=to_6p(rnd(hi[0]).clamp(0,255),pbits[s] as i32)as u32;
                        hg[s]=to_6p(rnd(hi[1]).clamp(0,255),pbits[s] as i32)as u32;
                        hb[s]=to_6p(rnd(hi[2]).clamp(0,255),pbits[s] as i32)as u32;
                    }
                }
                let sse = eval_m1(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits, bmask);
                if sse < best_sse {
                    best_sse = sse;
                    encode_mode1(&mut best_block, pat as u32, &lr, &lg, &lb, &hr, &hg, &hb, pbits[0], pbits[1], &w);
                }
            }

            // ── Mode 3 ──
            {
                let mut lr = [0u32;2]; let mut lg = [0u32;2]; let mut lb = [0u32;2];
                let mut hr = [0u32;2]; let mut hg = [0u32;2]; let mut hb = [0u32;2];
                for s in 0..2 {
                    let (lo, hi) = init_endpoints_m3_subset(pixels, pm, s, xr, xg, xb);
                    lr[s]=lo[0]; lg[s]=lo[1]; lb[s]=lo[2];
                    hr[s]=hi[0]; hg[s]=hi[1]; hb[s]=hi[2];
                }
                let mut w = [0u8; 16];
                let mut pbits4 = [0u32; 4];
                eval_m3(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits4, bmask);
                // LS refinement
                for s in 0..2 {
                    if let Some((lo, hi)) = ls_refine_subset_3d(pixels, &w, pm, s, &LS_TAB2) {
                        lr[s]=to_7p(rnd(lo[0]).clamp(0,255),0)as u32;
                        lg[s]=to_7p(rnd(lo[1]).clamp(0,255),0)as u32;
                        lb[s]=to_7p(rnd(lo[2]).clamp(0,255),0)as u32;
                        hr[s]=to_7p(rnd(hi[0]).clamp(0,255),0)as u32;
                        hg[s]=to_7p(rnd(hi[1]).clamp(0,255),0)as u32;
                        hb[s]=to_7p(rnd(hi[2]).clamp(0,255),0)as u32;
                    }
                }
                pbits4 = best_pbits_m3(pixels, &mut w, &mut lr, &mut lg, &mut lb, &mut hr, &mut hg, &mut hb, bmask);
                // Second LS pass
                for s in 0..2 {
                    let pb_lo = pbits4[s*2];
                    let pb_hi = pbits4[s*2+1];
                    if let Some((lo, hi)) = ls_refine_subset_3d(pixels, &w, pm, s, &LS_TAB2) {
                        lr[s]=to_7p(rnd(lo[0]).clamp(0,255),pb_lo as i32)as u32;
                        lg[s]=to_7p(rnd(lo[1]).clamp(0,255),pb_lo as i32)as u32;
                        lb[s]=to_7p(rnd(lo[2]).clamp(0,255),pb_lo as i32)as u32;
                        hr[s]=to_7p(rnd(hi[0]).clamp(0,255),pb_hi as i32)as u32;
                        hg[s]=to_7p(rnd(hi[1]).clamp(0,255),pb_hi as i32)as u32;
                        hb[s]=to_7p(rnd(hi[2]).clamp(0,255),pb_hi as i32)as u32;
                    }
                }
                let sse = eval_m3(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits4, bmask);
                if sse < best_sse {
                    best_sse = sse;
                    encode_mode3(&mut best_block, pat as u32, &lr, &lg, &lb, &hr, &hg, &hb, &pbits4, &w);
                }
            }
        }
    }

    // 7. Try 3-subset modes (Modes 0 and 2) for very high ortho_ratio blocks.
    // These are expensive (64 partitions × 3 subsets × 2 modes) but give the
    // best quality for blocks with three distinct colour clusters.
    if (flags & FLAG_USE_3SUBSETS) != 0 && block_max_var >= 128 * 16 && ortho_ratio > ORTHO_3SUBSET {
        let (partitions, partition_count) = part3_candidates(pixels, xr, xg, xb);
        for &pat in partitions[..partition_count].iter() {
            let pm = &BC7_PARTITION3[pat * 16..pat * 16 + 16];
            let _anc1 = BC7_ANCHOR_THIRD_SUBSET1[pat] as usize;
            let _anc2 = BC7_ANCHOR_THIRD_SUBSET2[pat] as usize;

            // ── Mode 2: 5-bit, no p-bits, 2-bit weights ──
            {
                let mut lr=[0u32;3]; let mut lg=[0u32;3]; let mut lb=[0u32;3];
                let mut hr=[0u32;3]; let mut hg=[0u32;3]; let mut hb=[0u32;3];
                for s in 0..3 {
                    let (mut lo_d,mut hi_d)=(f32::MAX,f32::MIN);
                    let (mut li,mut hi_i)=(0,0);
                    for i in 0..16 {
                        if pm[i] as usize != s { continue; }
                        let d=pixels[i][0] as f32*xr+pixels[i][1] as f32*xg+pixels[i][2] as f32*xb+i as f32*1e-4;
                        if d<lo_d{lo_d=d;li=i;} if d>hi_d{hi_d=d;hi_i=i;}
                    }
                    lr[s]=(pixels[li][0] as u32*31+127)/255;
                    lg[s]=(pixels[li][1] as u32*31+127)/255;
                    lb[s]=(pixels[li][2] as u32*31+127)/255;
                    hr[s]=(pixels[hi_i][0] as u32*31+127)/255;
                    hg[s]=(pixels[hi_i][1] as u32*31+127)/255;
                    hb[s]=(pixels[hi_i][2] as u32*31+127)/255;
                }
                let mut w = [0u8; 16];
                eval_m2(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, pat);
                // LS refine per subset
                for s in 0..3 {
                    if let Some((lo,hi)) = ls_refine_subset_3d(pixels,&w,pm,s,&LS_TAB2) {
                        lr[s]=((rnd(lo[0]).clamp(0,255)*31+127)/255) as u32;
                        lg[s]=((rnd(lo[1]).clamp(0,255)*31+127)/255) as u32;
                        lb[s]=((rnd(lo[2]).clamp(0,255)*31+127)/255) as u32;
                        hr[s]=((rnd(hi[0]).clamp(0,255)*31+127)/255) as u32;
                        hg[s]=((rnd(hi[1]).clamp(0,255)*31+127)/255) as u32;
                        hb[s]=((rnd(hi[2]).clamp(0,255)*31+127)/255) as u32;
                    }
                }
                let sse = eval_m2(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, pat);
                if sse < best_sse {
                    best_sse = sse;
                    encode_mode2(&mut best_block, pat as u32, &lr, &lg, &lb, &hr, &hg, &hb, &w);
                }
            }

            // ── Mode 0: 4-bit + unique p-bits, 3-bit weights ──
            {
                let mut lr=[0u32;3]; let mut lg=[0u32;3]; let mut lb=[0u32;3];
                let mut hr=[0u32;3]; let mut hg=[0u32;3]; let mut hb=[0u32;3];
                for s in 0..3 {
                    let (mut lo_d,mut hi_d)=(f32::MAX,f32::MIN);
                    let (mut li,mut hi_i)=(0,0);
                    for i in 0..16 {
                        if pm[i] as usize != s { continue; }
                        let d=pixels[i][0] as f32*xr+pixels[i][1] as f32*xg+pixels[i][2] as f32*xb+i as f32*1e-4;
                        if d<lo_d{lo_d=d;li=i;} if d>hi_d{hi_d=d;hi_i=i;}
                    }
                    // 4-bit endpoint: (v*15+127)/255
                    lr[s]=(pixels[li][0] as u32*15+127)/255;
                    lg[s]=(pixels[li][1] as u32*15+127)/255;
                    lb[s]=(pixels[li][2] as u32*15+127)/255;
                    hr[s]=(pixels[hi_i][0] as u32*15+127)/255;
                    hg[s]=(pixels[hi_i][1] as u32*15+127)/255;
                    hb[s]=(pixels[hi_i][2] as u32*15+127)/255;
                }
                let mut w = [0u8; 16];
                let mut pbits6 = [0u32; 6];
                eval_m0(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits6, pat);
                // Try all 64 p-bit combos (6 bits = 2^6)
                let mut best_p6 = [0u32;6];
                let mut best_sse0 = u32::MAX;
                for pbits_mask in 0u32..64 {
                    let p = [pbits_mask&1,(pbits_mask>>1)&1,(pbits_mask>>2)&1,
                             (pbits_mask>>3)&1,(pbits_mask>>4)&1,(pbits_mask>>5)&1];
                    let mut tw = [0u8;16];
                    let sse = eval_m0(pixels,&mut tw,&lr,&lg,&lb,&hr,&hg,&hb,&p,pat);
                    if sse<best_sse0 { best_sse0=sse; best_p6=p; w=tw; }
                }
                pbits6 = best_p6;
                // LS refine
                for s in 0..3 {
                    if let Some((lo,hi)) = ls_refine_subset_3d(pixels,&w,pm,s,&LS_TAB3) {
                        let p_lo = pbits6[s*2] as i32;
                        let p_hi = pbits6[s*2+1] as i32;
                        lr[s]=to_5p(rnd(lo[0]).clamp(0,255),p_lo)as u32;
                        lg[s]=to_5p(rnd(lo[1]).clamp(0,255),p_lo)as u32;
                        lb[s]=to_5p(rnd(lo[2]).clamp(0,255),p_lo)as u32;
                        hr[s]=to_5p(rnd(hi[0]).clamp(0,255),p_hi)as u32;
                        hg[s]=to_5p(rnd(hi[1]).clamp(0,255),p_hi)as u32;
                        hb[s]=to_5p(rnd(hi[2]).clamp(0,255),p_hi)as u32;
                    }
                }
                let sse = eval_m0(pixels, &mut w, &lr, &lg, &lb, &hr, &hg, &hb, &pbits6, pat);
                if sse < best_sse {
                    best_sse = sse;
                    encode_mode0(&mut best_block, pat as u32, &lr, &lg, &lb, &hr, &hg, &hb, &pbits6, &w);
                }
            }
        }
    }

    // 8. Dual-plane (Modes 4/5) for RGB blocks — when one RGB channel is decorrelated.
    if (flags & FLAG_USE_DUAL_PLANE) != 0 && block_max_var >= 32 * 16 {
        let mut icov4 = [0i32; 10];
        let mean_r = (tr + 8) >> 4; let mean_g = (tg + 8) >> 4; let mean_b = (tb + 8) >> 4;
        for p in pixels.iter() {
            let (r,g,b) = (p[0] as i32-mean_r, p[1] as i32-mean_g, p[2] as i32-mean_b);
            icov4[0]+=r*r; icov4[1]+=r*g; icov4[2]+=r*b;
            icov4[4]+=g*g; icov4[5]+=g*b; icov4[7]+=b*b;
        }
        let dp_chan = detect_dp_channel(&icov4);
        if dp_chan >= 0 && dp_chan < 3 {
            let mut rgba = [[0u8;4]; 16];
            for i in 0..16 { rgba[i] = [pixels[i][0], pixels[i][1], pixels[i][2], 255]; }
            let mut dp_block = [0u8; 16];
            let dp_sse = pack_mode4_or_5(&mut dp_block, &rgba, dp_chan as usize);
            if dp_sse < best_sse { let _ = best_sse; best_block = dp_block; }
        }
    }

    // 9. Write the best block found
    *block = best_block;
}

/// Mode 6 analytical encoding with full LS refinement.
/// Returns the achieved SSE so callers can compare against multi-subset modes.
fn pack_bc7_rgb_mode6_sse(
    block: &mut [u8; 16], pixels: &[Pixel; 16],
    tr: i32, tg: i32, tb: i32,
    xr: f32, xg: f32, xb: f32,
) -> u32 {
    // Find initial endpoints by projecting onto dominant axis (same as C++)
    let saxis_r = if xr.abs() >= 1e-6 { (xr * 2048.0) as i32 } else { 306 };
    let saxis_g = if xg.abs() >= 1e-6 { (xg * 2048.0) as i32 } else { 601 };
    let saxis_b = if xb.abs() >= 1e-6 { (xb * 2048.0) as i32 } else { 117 };
    let (mut lo_dot, mut hi_dot) = (i32::MAX, i32::MIN);
    let (mut lo_c, mut hi_c) = (0usize, 0usize);
    for i in 0..16 {
        let dot = pixels[i][0] as i32 * saxis_r + pixels[i][1] as i32 * saxis_g + pixels[i][2] as i32 * saxis_b + i as i32;
        if dot < lo_dot { lo_dot = dot; lo_c = i; }
        if dot > hi_dot { hi_dot = dot; hi_c = i; }
    }
    let mut lr = to_7f(pixels[lo_c][0] as f32) as i32;
    let mut lg = to_7f(pixels[lo_c][1] as f32) as i32;
    let mut lb = to_7f(pixels[lo_c][2] as f32) as i32;
    let mut hr = to_7f(pixels[hi_c][0] as f32) as i32;
    let mut hg = to_7f(pixels[hi_c][1] as f32) as i32;
    let mut hb = to_7f(pixels[hi_c][2] as f32) as i32;
    let (p0, p1) = (1u32, 1u32);
    let mut w = [0u8; 16];
    eval_m6_rgb(pixels, &mut w, lr, lg, lb, hr, hg, hb, p0, p1);
    // LS refinement
    let all_px: &[Pixel] = pixels;
    if let Some((lo, hi)) = ls_fit_3d(16, &w, &LS_TAB4, all_px, tr as f32, tg as f32, tb as f32) {
        lr = to_7f(lo[0]) as i32;
        lg = to_7f(lo[1]) as i32;
        lb = to_7f(lo[2]) as i32;
        hr = to_7f(hi[0]) as i32;
        hg = to_7f(hi[1]) as i32;
        hb = to_7f(hi[2]) as i32;
    }
    let sse = eval_m6_rgb(pixels, &mut w, lr, lg, lb, hr, hg, hb, p0, p1);
    encode_mode6(block, lr as u32, lg as u32, lb as u32, 127, p0, hr as u32, hg as u32, hb as u32, 127, p1, &w);
    sse
}

/// Main entry for RGBA blocks. Selects between Mode 4/5 (dual-plane), Mode 6 (1-subset RGBA),
/// and Mode 7 (2-subset RGBA) depending on block statistics and flags.
pub fn pack_bc7_rgba(block: &mut [u8; 16], pixels: &[Pixel; 16], flags: u32) {
    if pixels.iter().all(|p| p[3] == 255) {
        pack_bc7_rgb(block, pixels, flags);
        return;
    }

    // ── Solid block fast-path ──
    if pixels.iter().all(|p| p == &pixels[0]) {
        encode_solid_block(block, &pixels[0]);
        return;
    }

    // ── Compute full 4D statistics ──
    let mut total = [0i32; 4];
    let mut mn = [255i32; 4]; let mut mx = [0i32; 4];
    for p in pixels.iter() {
        for c in 0..4 { total[c] += p[c] as i32; mn[c] = mn[c].min(p[c] as i32); mx[c] = mx[c].max(p[c] as i32); }
    }
    let (tr, tg, tb, ta) = (total[0] as f32, total[1] as f32, total[2] as f32, total[3] as f32);
    let mean = total.map(|t| (t + 8) >> 4);
    let mut icov4 = [0i32; 10];
    for p in pixels.iter() {
        let (r,g,b,a) = (p[0] as i32-mean[0], p[1] as i32-mean[1], p[2] as i32-mean[2], p[3] as i32-mean[3]);
        icov4[0]+=r*r; icov4[1]+=r*g; icov4[2]+=r*b; icov4[3]+=r*a;
        icov4[4]+=g*g; icov4[5]+=g*b; icov4[6]+=g*a;
        icov4[7]+=b*b; icov4[8]+=b*a; icov4[9]+=a*a;
    }
    let block_max_var4 = *[icov4[0],icov4[4],icov4[7],icov4[9]].iter().max().unwrap();

    // ── Detect decorrelated channel for dual-plane modes ──
    let dp_chan = if (flags & FLAG_USE_DUAL_PLANE) != 0 && block_max_var4 >= 32*16 {
        detect_dp_channel(&icov4)
    } else { -1 };

    // ── Mode 6 baseline (single-subset RGBA, full 4D LS) ──
    let mut best_block = [0u8; 16];
    // ── Mode 6 baseline: single-subset RGBA with 4D LS + p-bit optimisation ──
    // Project along the 4D dominant axis to seed the initial endpoint pair.
    let mut best_sse = {
        let (xr,xg,xb,xa) = dominant_axis_4d(&icov4);
        let (sr,sg,sb,sa) = ((xr*2048.0)as i32,(xg*2048.0)as i32,(xb*2048.0)as i32,(xa*2048.0)as i32);
        let (mut lo_dot,mut hi_dot) = (i32::MAX,i32::MIN);
        let (mut lo_c,mut hi_c) = (0usize,0usize);
        for i in 0..16 {
            let dot=pixels[i][0]as i32*sr+pixels[i][1]as i32*sg+pixels[i][2]as i32*sb+pixels[i][3]as i32*sa+i as i32;
            if dot<lo_dot{lo_dot=dot;lo_c=i;} if dot>hi_dot{hi_dot=dot;hi_c=i;}
        }
        // Seed from the extreme pixels on the dominant axis
        let lo_init = [pixels[lo_c][0]as i32, pixels[lo_c][1]as i32, pixels[lo_c][2]as i32, pixels[lo_c][3]as i32];
        let hi_init = [pixels[hi_c][0]as i32, pixels[hi_c][1]as i32, pixels[hi_c][2]as i32, pixels[hi_c][3]as i32];
        // Run full encoder with p-bit optimisation (FLAG_PBIT_OPT_M6 controls this)
        let use_pbit = (flags & FLAG_PBIT_OPT_M6) != 0;
        pack_bc7_mode6_rgba_full(&mut best_block, pixels, tr, tg, tb, ta,
                                  lo_init, hi_init, use_pbit)
    };

    // ── Mode 4/5 (dual-plane) ──
    if dp_chan >= 0 {
        let mut dp_block = [0u8; 16];
        let dp_sse = pack_mode4_or_5(&mut dp_block, pixels, dp_chan as usize);
        if dp_sse < best_sse { best_sse = dp_sse; best_block = dp_block; }
    }

    // ── Mode 7 (2-subset RGBA) when ortho_ratio is significant ──
    if (flags & FLAG_USE_2SUBSETS) != 0 && block_max_var4 >= 64 * 16 {
        let icov3 = [icov4[0], icov4[1], icov4[2], icov4[4], icov4[5], icov4[7]];
        let (xr, xg, xb) = dominant_axis_3d(&icov3);
        let cov_f = icov3.map(|v| v as f32);
        let (_slam, ortho_ratio) = estimate_slam_sse_3d(&cov_f, xr, xg, xb);
        if ortho_ratio > ORTHO_2SUBSET {
            let (partitions, partition_count) = part2_candidates(pixels, xr, xg, xb);
            for &pat in partitions[..partition_count].iter() {
                let bmask = part2_bitmask(pat);
                let pm = &BC7_PARTITION2[pat*16..pat*16+16];
                let mut lr=[0u32;2];let mut lg=[0u32;2];let mut lb=[0u32;2];let mut la=[0u32;2];
                let mut hr=[0u32;2];let mut hg=[0u32;2];let mut hb=[0u32;2];let mut ha=[0u32;2];
                for s in 0..2 {
                    let (mut lo_d,mut hi_d)=(f32::MAX,f32::MIN);
                    let (mut li,mut hi_i)=(0usize,0usize);
                    for i in 0..16 {
                        if pm[i] as usize != s { continue; }
                        let d=pixels[i][0]as f32*xr+pixels[i][1]as f32*xg+pixels[i][2]as f32*xb+i as f32*1e-4;
                        if d<lo_d{lo_d=d;li=i;} if d>hi_d{hi_d=d;hi_i=i;}
                    }
                    lr[s]=(pixels[li][0]as u32*31+127)/255; hr[s]=(pixels[hi_i][0]as u32*31+127)/255;
                    lg[s]=(pixels[li][1]as u32*31+127)/255; hg[s]=(pixels[hi_i][1]as u32*31+127)/255;
                    lb[s]=(pixels[li][2]as u32*31+127)/255; hb[s]=(pixels[hi_i][2]as u32*31+127)/255;
                    la[s]=(pixels[li][3]as u32*31+127)/255; ha[s]=(pixels[hi_i][3]as u32*31+127)/255;
                }
                let mut w=[0u8;16];
                let mut pbits4=[0u32;4];
                eval_m7(pixels,&mut w,&lr,&lg,&lb,&la,&hr,&hg,&hb,&ha,&pbits4,bmask);
                // P-bit optimisation
                let mut best_p4=[0u32;4]; let mut best_sse7=u32::MAX;
                for pm_bits in 0u32..16 {
                    let p=[pm_bits&1,(pm_bits>>1)&1,(pm_bits>>2)&1,(pm_bits>>3)&1];
                    let mut tw=[0u8;16];
                    let sse=eval_m7(pixels,&mut tw,&lr,&lg,&lb,&la,&hr,&hg,&hb,&ha,&p,bmask);
                    if sse<best_sse7{best_sse7=sse;best_p4=p;w=tw;}
                }
                pbits4=best_p4;
                let sse=eval_m7(pixels,&mut w,&lr,&lg,&lb,&la,&hr,&hg,&hb,&ha,&pbits4,bmask);
                if sse < best_sse {
                    best_sse = sse;
                    encode_mode7(&mut best_block, pat as u32, &lr, &lg, &lb, &la, &hr, &hg, &hb, &ha, &pbits4, &w);
                }
            }
        }
    }

    *block = best_block;
}

// ─── 8. MODES 4 AND 5 (DUAL-PLANE) ───────────────────────────────────────────
//
// Both Mode 4 and Mode 5 encode two independent planes: one for RGB and one for
// a single "decorrelated" channel (alpha by default, or R/G/B if that channel
// is weakly correlated with the rest).
//
// Rotation encoding:
//   rotation 0 = no swap  (second plane = alpha)
//   rotation 1 = swap A↔R (second plane = R, packed where alpha goes)
//   rotation 2 = swap A↔G (second plane = G)
//   rotation 3 = swap A↔B (second plane = B)
//
// The `rot` in the encoded block is stored as `(dp_chan + 1) & 3` by the C++
// reference, so that channel 3 (alpha) → rot=0, channel 0 (R) → rot=1, etc.
//
// Mode 4 layout (5-bit RGB, 6-bit A):
//   mode(5b) | rot(2b) | index_flag(1b) | RGB endpoints (30b) | A endpoints (12b)
//   | A-plane 2-bit weights (31b, anchor=1b) | RGB-plane 3-bit weights (47b)
//   Total: 5+2+1+30+12+31+47 = 128 bits
//
// Mode 5 layout (7-bit RGB, 8-bit A):
//   mode(6b, bit5 set) | rot(2b) | RGB endpoints (42b) | A endpoints (16b)
//   | RGB-plane 2-bit weights (31b) | A-plane 2-bit weights (31b)
//   Total: 6+2+42+16+31+31 = 128 bits

/// Evaluate 1-channel weights and compute SSE for Mode 4/5 alpha plane (scalar channel).
/// `from_fn`: expander for the alpha endpoint bit-depth (6-bit for Mode 4, raw 8-bit for Mode 5).
fn eval_alpha_weights(
    pixels: &[Pixel; 16], weights: &mut [u8; 16],
    la_raw: i32, ha_raw: i32, max_w: u32,
    from_fn: fn(i32) -> i32,
) -> u32 {
    let la = from_fn(la_raw);
    let ha = from_fn(ha_raw);
    let da = ha - la;
    let f = (max_w as f32) / (da as f32 + 1.25e-7);
    let tab = if max_w == 7 { &BC7_WEIGHTS3[..] } else { &BC7_WEIGHTS2[..] };
    let mut sse = 0u32;
    for i in (0..16).step_by(4) {
        let pa = f32x4::from([
            pixels[i][3] as f32,
            pixels[i + 1][3] as f32,
            pixels[i + 2][3] as f32,
            pixels[i + 3][3] as f32,
        ]);
        let sel_f = (pa - f32x4::splat(la as f32)) * f32x4::splat(f) + f32x4::splat(0.5);
        let sel_f = sel_f.to_array();
        for lane in 0..4 {
            let sel = clamp_weight_sel(sel_f[lane] as i32, max_w as i32);
            weights[i + lane] = sel as u8;
            let recon = la + ((da * tab[sel as usize] as i32 + 32) >> 6);
            let e = pixels[i + lane][3] as i32 - recon;
            sse += (e * e) as u32;
        }
    }
    sse
}

/// Expand 6-bit value to 8-bit by replication: x→(x<<2)|(x>>4)
#[inline] fn from_6i(v: i32) -> i32 { (v << 2) | (v >> 4) }
/// Identity: Mode 5 alpha endpoints are stored as raw 8-bit values.
#[inline] fn from_8i(v: i32) -> i32 { v }

/// Evaluate 3-bit RGB weights for Mode 4 (5-bit endpoints, expanded to 8-bit).
fn eval_m4_rgb3(pixels: &[Pixel; 16], weights: &mut [u8; 16],
    lr: i32, lg: i32, lb: i32, hr: i32, hg: i32, hb: i32) -> u32 {
    let (lr8, lg8, lb8) = (from_5(lr as u32) as i32, from_5(lg as u32) as i32, from_5(lb as u32) as i32);
    let (hr8, hg8, hb8) = (from_5(hr as u32) as i32, from_5(hg as u32) as i32, from_5(hb as u32) as i32);
    let (dr, dg, db) = (hr8-lr8, hg8-lg8, hb8-lb8);
    let f = 7.0 / ((dr*dr + dg*dg + db*db) as f32 + 1.25e-7);
    let sofs = -(lr8*dr + lg8*dg + lb8*db);
    let mut sse = 0u32;
    for i in (0..16).step_by(4) { sse += eval_rgb_weights4(pixels, weights, i, lr8, lg8, lb8, dr, dg, db, sofs, f, 7, &BC7_WEIGHTS3); }
    sse
}

/// Evaluate 2-bit RGB weights for Mode 4 (alternate index_flag=0 path) and Mode 5.
fn eval_m4_rgb2(pixels: &[Pixel; 16], weights: &mut [u8; 16],
    lr: i32, lg: i32, lb: i32, hr: i32, hg: i32, hb: i32,
    from: fn(u32) -> u32) -> u32 {
    let (lr8, lg8, lb8) = (from(lr as u32) as i32, from(lg as u32) as i32, from(lb as u32) as i32);
    let (hr8, hg8, hb8) = (from(hr as u32) as i32, from(hg as u32) as i32, from(hb as u32) as i32);
    let (dr, dg, db) = (hr8-lr8, hg8-lg8, hb8-lb8);
    let f = 3.0 / ((dr*dr + dg*dg + db*db) as f32 + 1.25e-7);
    let sofs = -(lr8*dr + lg8*dg + lb8*db);
    let mut sse = 0u32;
    for i in (0..16).step_by(4) { sse += eval_rgb_weights4(pixels, weights, i, lr8, lg8, lb8, dr, dg, db, sofs, f, 3, &BC7_WEIGHTS2); }
    sse
}

/// Encode Mode 4 block. index_flag=1 means 3-bit weights for RGB, 2-bit for alpha.
/// index_flag=0 reverses this. rot_index = (dp_chan+1)&3.
pub fn encode_mode4(
    block: &mut [u8; 16],
    lr: u32, lg: u32, lb: u32, la: u32,  // 5-bit RGB, 6-bit A
    hr: u32, hg: u32, hb: u32, ha: u32,
    w0: &[u8; 16], w1: &[u8; 16],        // w0=3-bit plane, w1=2-bit plane
    rot_idx: u32, index_flag: u32,
) {
    let (mut lr, mut lg, mut lb, mut la) = (lr, lg, lb, la);
    let (mut hr, mut hg, mut hb, mut ha) = (hr, hg, hb, ha);
    let w0 = *w0; let w1 = *w1;
    // p2bit = the 2-bit-plane weights; p3bit = the 3-bit-plane weights
    let (inv0, inv1);  // XOR mask for each plane
    // The 3-bit plane anchor (pixel 0) MSB must be 0
    if index_flag == 1 {
        // w0=RGB(3-bit), w1=A(2-bit)
        if w0[0] & 4 != 0 { let t=lr;lr=hr;hr=t; let t=lg;lg=hg;hg=t; let t=lb;lb=hb;hb=t; inv0=7; } else { inv0=0; }
        if w1[0] & 2 != 0 { let t=la;la=ha;ha=t; inv1=3; } else { inv1=0; }
    } else {
        // w0=A(3-bit), w1=RGB(2-bit)
        if w0[0] & 4 != 0 { let t=la;la=ha;ha=t; inv0=7; } else { inv0=0; }
        if w1[0] & 2 != 0 { let t=lr;lr=hr;hr=t; let t=lg;lg=hg;hg=t; let t=lb;lb=hb;hb=t; inv1=3; } else { inv1=0; }
    }
    // block[0]: mode bits (bit4=1), rotation (bits 5-6), index_flag (bit7)
    block[0] = (0b00010000 | (rot_idx << 5) | (index_flag << 7)) as u8;
    // 42 bits of endpoints: 6×5-bit RGB + 2×6-bit A
    let x = lr as u64 | (hr as u64)<<5 | (lg as u64)<<10 | (hg as u64)<<15
          | (lb as u64)<<20 | (hb as u64)<<25 | (la as u64)<<30 | (ha as u64)<<36;
    block[1..6].copy_from_slice(&x.to_le_bytes()[0..5]);
    // 2 leftover bits from ha (ha is 6-bit, 30+6+6=42 total, 42-40=2 left)
    let mut y = (x >> 40) & 3;
    let mut ofs = 2usize;
    // 2-bit alpha indices (if index_flag=1, w1 is alpha; if 0, w1 is RGB)
    let (p2, p3) = if index_flag == 1 { (&w1, &w0) } else { (&w0, &w1) };
    for i in 0..16 { let w = (p2[i] ^ inv1) as u64; y |= w << ofs; ofs += 2 - (i==0) as usize; }
    block[6..10].copy_from_slice(&y.to_le_bytes()[0..4]);
    // 3-bit RGB indices
    let mut z = y >> 32; ofs = 1;
    for i in 0..16 { let w = (p3[i] ^ inv0) as u64; z |= w << ofs; ofs += 3 - (i==0) as usize; }
    block[10..16].copy_from_slice(&z.to_le_bytes()[0..6]);
}

/// Encode Mode 5 block. 7-bit RGB endpoints, 8-bit alpha endpoints, no p-bits.
/// Both weight planes are 2-bit. rot_index = (dp_chan+1)&3.
pub fn encode_mode5(
    block: &mut [u8; 16],
    lr: u32, lg: u32, lb: u32, la: u32,  // 7-bit RGB, 8-bit A
    hr: u32, hg: u32, hb: u32, ha: u32,
    wrgb: &[u8; 16], wa: &[u8; 16],
    rot_idx: u32,
) {
    let (mut lr, mut lg, mut lb, mut la) = (lr, lg, lb, la);
    let (mut hr, mut hg, mut hb, mut ha) = (hr, hg, hb, ha);
    let wrgb = *wrgb; let wa = *wa;
    let (c_inv, a_inv);
    if wrgb[0] & 2 != 0 { let t=lr;lr=hr;hr=t; let t=lg;lg=hg;hg=t; let t=lb;lb=hb;hb=t; c_inv=3; } else { c_inv=0; }
    if wa[0]   & 2 != 0 { let t=la;la=ha;ha=t; a_inv=3; } else { a_inv=0; }
    let low = (1u64<<5) | (rot_idx as u64)<<6
            | (lr as u64)<<8  | (hr as u64)<<15
            | (lg as u64)<<22 | (hg as u64)<<29
            | (lb as u64)<<36 | (hb as u64)<<43
            | (la as u64)<<50 | (ha as u64)<<58;
    block[0..8].copy_from_slice(&low.to_le_bytes());
    // high word: 2 leftover bits from ha (bits 56-57 of la+ha span bytes 7 onwards)
    let mut high = (ha as u64 >> 6) & 3;
    let mut ofs = 2usize;
    for i in 0..16 { let w = (wrgb[i] ^ c_inv) as u64; high |= w << ofs; ofs += 2 - (i==0) as usize; }
    for i in 0..16 { let w = (wa[i]   ^ a_inv) as u64; high |= w << ofs; ofs += 2 - (i==0) as usize; }
    block[8..16].copy_from_slice(&high.to_le_bytes());
}

/// Dual-plane channel detection from 4D covariance.
/// Returns the channel index (0=R,1=G,2=B,3=A) that is most decorrelated from
/// the others, or -1 if none qualifies.
pub fn detect_dp_channel(icov4: &[i32; 10]) -> i32 {
    // Thresholds from the C++ reference
    const ALPHA_DECORR: f32 = 0.35;    // alpha vs RGB correlation threshold
    const RGB_DECORR:   f32 = 0.25;    // inter-channel RGB correlation threshold

    let (rv, gv, bv, av) = (icov4[0] as f32, icov4[4] as f32, icov4[7] as f32, icov4[9] as f32);
    // Prefer alpha if it has variance and is weakly correlated with all RGB channels
    if av > 0.0 {
        let p_ra = if icov4[0] > 0 { (icov4[3] as f32).abs() / (rv * av).sqrt() } else { 1.0 };
        let p_ga = if icov4[4] > 0 { (icov4[6] as f32).abs() / (gv * av).sqrt() } else { 1.0 };
        let p_ba = if icov4[7] > 0 { (icov4[8] as f32).abs() / (bv * av).sqrt() } else { 1.0 };
        if p_ra < ALPHA_DECORR && p_ga < ALPHA_DECORR && p_ba < ALPHA_DECORR {
            return 3;
        }
    }
    // Check RGB channels
    let has_r = icov4[0] > 16; let has_g = icov4[4] > 16; let has_b = icov4[7] > 16;
    let n_active = has_r as u32 + has_g as u32 + has_b as u32;
    if n_active >= 2 {
        let rg = if has_r && has_g { (icov4[1] as f32).abs() / (rv*gv).sqrt() } else { 1.0 };
        let rb = if has_r && has_b { (icov4[2] as f32).abs() / (rv*bv).sqrt() } else { 1.0 };
        let gb = if has_g && has_b { (icov4[5] as f32).abs() / (gv*bv).sqrt() } else { 1.0 };
        let min_p = rg.min(rb).min(gb);
        if min_p < RGB_DECORR {
            if n_active == 2 {
                return if !has_r { 1 } else { 0 };
            }
            // All three active: find which pair is least correlated
            return if rg < gb && rb < gb { 0 }
                   else if rg < rb && gb < rb { 1 }
                   else { 2 };
        }
    }
    -1
}

/// Try Mode 4 and Mode 5 for the given `dp_chan` and return the best block + SSE.
/// Swaps `dp_chan` with alpha in a local pixel copy so the encoder always sees
/// alpha as the decorrelated plane.
pub fn pack_mode4_or_5(block: &mut [u8; 16], pixels: &[Pixel; 16], dp_chan: usize) -> u32 {
    // Rotate pixels so that dp_chan occupies the alpha slot
    let mut rotpx = *pixels;
    if dp_chan != 3 {
        for p in rotpx.iter_mut() { p.swap(dp_chan, 3); }
    }
    let rot_idx = ((dp_chan + 1) & 3) as u32;
    let px = &rotpx;

    // --- compute RGB stats on the rotated pixels ---
    let mut total = [0i32; 4];
    let mut mn = [255i32; 4]; let mut mx = [0i32; 4];
    for p in px.iter() {
        for c in 0..4 { total[c] += p[c] as i32; mn[c] = mn[c].min(p[c] as i32); mx[c] = mx[c].max(p[c] as i32); }
    }
    let mean = [0,1,2].map(|c| (total[c] + 8) >> 4);
    let mut icov = [0i32; 6];
    for p in px.iter() {
        let (r,g,b) = ((p[0] as i32-mean[0]), (p[1] as i32-mean[1]), (p[2] as i32-mean[2]));
        icov[0]+=r*r; icov[1]+=r*g; icov[2]+=r*b; icov[3]+=g*g; icov[4]+=g*b; icov[5]+=b*b;
    }
    let (xr, xg, xb) = dominant_axis_3d(&icov);
    // Project RGB onto dominant axis to find lo/hi pixels
    let (sr, sg, sb) = ((xr*32768.0) as i32, (xg*32768.0) as i32, (xb*32768.0) as i32);
    let (mut lo_d, mut hi_d) = (i32::MAX, i32::MIN);
    let (mut lo_c, mut hi_c) = (0usize, 0usize);
    for i in 0..16 {
        let d = px[i][0] as i32*sr + px[i][1] as i32*sg + px[i][2] as i32*sb + i as i32;
        if d < lo_d { lo_d=d; lo_c=i; } if d > hi_d { hi_d=d; hi_c=i; }
    }

    let mut best_block = [0u8; 16];
    let mut best_sse = u32::MAX;

    // ── Mode 5: 7-bit RGB, 8-bit alpha, 2-bit weights for both planes ──
    {
        let mut lr = to_7f(px[lo_c][0] as f32) as i32; let mut lg = to_7f(px[lo_c][1] as f32) as i32;
        let mut lb = to_7f(px[lo_c][2] as f32) as i32;
        let mut hr = to_7f(px[hi_c][0] as f32) as i32; let mut hg = to_7f(px[hi_c][1] as f32) as i32;
        let mut hb = to_7f(px[hi_c][2] as f32) as i32;
        let (mut la, mut ha) = (mn[3] as i32, mx[3] as i32);
        let mut wrgb = [0u8; 16]; let mut wa = [0u8; 16];
        // RGB weights → LS refine
        eval_m4_rgb2(px, &mut wrgb, lr, lg, lb, hr, hg, hb, |v| from_7(v,0));
        if let Some((lo, hi)) = ls_fit_3d(16, &wrgb, &LS_TAB2, px, total[0] as f32, total[1] as f32, total[2] as f32) {
            lr=to_7f(lo[0])as i32; lg=to_7f(lo[1])as i32; lb=to_7f(lo[2])as i32;
            hr=to_7f(hi[0])as i32; hg=to_7f(hi[1])as i32; hb=to_7f(hi[2])as i32;
        }
        let rgb_sse = eval_m4_rgb2(px, &mut wrgb, lr, lg, lb, hr, hg, hb, |v| from_7(v,0));
        // Alpha weights → LS refine (8-bit endpoints, so just min/max, LS on 1D)
        let a_sse = eval_alpha_weights(px, &mut wa, la, ha, 3, from_8i);
        if let Some((nla, nha)) = ls_fit_1d(16, &wa, &LS_TAB2, px, 3, total[3] as f32) {
            la = rnd(nla).clamp(0,255); ha = rnd(nha).clamp(0,255);
        }
        let a_sse = a_sse + eval_alpha_weights(px, &mut wa, la, ha, 3, from_8i);
        let sse = rgb_sse + a_sse / 2; // average both passes
        encode_mode5(&mut best_block, lr as u32, lg as u32, lb as u32, la as u32, hr as u32, hg as u32, hb as u32, ha as u32, &wrgb, &wa, rot_idx);
        if sse < best_sse { best_sse = sse; *block = best_block; }
    }

    // ── Mode 4, index_flag=1: 5-bit RGB (3-bit weights), 6-bit alpha (2-bit weights) ──
    // Used when RGB span is large and alpha span is small.
    {
        let mut lr = to_5f(px[lo_c][0] as f32) as i32; let mut lg = to_5f(px[lo_c][1] as f32) as i32;
        let mut lb = to_5f(px[lo_c][2] as f32) as i32;
        let mut hr = to_5f(px[hi_c][0] as f32) as i32; let mut hg = to_5f(px[hi_c][1] as f32) as i32;
        let mut hb = to_5f(px[hi_c][2] as f32) as i32;
        let (mut la, mut ha) = ((mn[3] as u32*63+127)/255, (mx[3] as u32*63+127)/255);
        let (la_i, ha_i) = (la as i32, ha as i32);
        let mut w3 = [0u8; 16]; let mut w2 = [0u8; 16];
        // 3-bit RGB weights + LS refine
        eval_m4_rgb3(px, &mut w3, lr, lg, lb, hr, hg, hb);
        if let Some((lo, hi)) = ls_fit_3d(16, &w3, &LS_TAB3, px, total[0] as f32, total[1] as f32, total[2] as f32) {
            lr=to_5f(lo[0])as i32; lg=to_5f(lo[1])as i32; lb=to_5f(lo[2])as i32;
            hr=to_5f(hi[0])as i32; hg=to_5f(hi[1])as i32; hb=to_5f(hi[2])as i32;
        }
        let rgb_sse = eval_m4_rgb3(px, &mut w3, lr, lg, lb, hr, hg, hb);
        // 2-bit alpha weights (6-bit endpoints)
        let a_sse = eval_alpha_weights(px, &mut w2, la_i, ha_i, 3, from_6i);
        if let Some((nla, nha)) = ls_fit_1d(16, &w2, &LS_TAB2, px, 3, total[3] as f32) {
            la = ((rnd(nla).clamp(0,255) as u32)*63+127)/255;
            ha = ((rnd(nha).clamp(0,255) as u32)*63+127)/255;
        }
        let a_sse2 = eval_alpha_weights(px, &mut w2, la as i32, ha as i32, 3, from_6i);
        let sse = rgb_sse + (a_sse + a_sse2) / 2;
        let mut cand = [0u8; 16];
        encode_mode4(&mut cand, lr as u32, lg as u32, lb as u32, la, hr as u32, hg as u32, hb as u32, ha, &w3, &w2, rot_idx, 1);
        if sse < best_sse { best_sse = sse; *block = cand; }
    }

    // ── Mode 4, index_flag=0: 5-bit RGB (2-bit weights), 6-bit alpha (3-bit weights) ──
    // Used when alpha span is large and RGB span is small.
    {
        let mut lr = to_5f(px[lo_c][0] as f32) as i32; let mut lg = to_5f(px[lo_c][1] as f32) as i32;
        let mut lb = to_5f(px[lo_c][2] as f32) as i32;
        let mut hr = to_5f(px[hi_c][0] as f32) as i32; let mut hg = to_5f(px[hi_c][1] as f32) as i32;
        let mut hb = to_5f(px[hi_c][2] as f32) as i32;
        let (mut la, mut ha) = ((mn[3] as u32*63+127)/255, (mx[3] as u32*63+127)/255);
        let (la_i, ha_i) = (la as i32, ha as i32);
        let mut w3 = [0u8; 16]; let mut w2 = [0u8; 16];
        eval_m4_rgb2(px, &mut w2, lr, lg, lb, hr, hg, hb, from_5);
        if let Some((lo, hi)) = ls_fit_3d(16, &w2, &LS_TAB2, px, total[0] as f32, total[1] as f32, total[2] as f32) {
            lr=to_5f(lo[0])as i32; lg=to_5f(lo[1])as i32; lb=to_5f(lo[2])as i32;
            hr=to_5f(hi[0])as i32; hg=to_5f(hi[1])as i32; hb=to_5f(hi[2])as i32;
        }
        let rgb_sse = eval_m4_rgb2(px, &mut w2, lr, lg, lb, hr, hg, hb, from_5);
        let a_sse = eval_alpha_weights(px, &mut w3, la_i, ha_i, 7, from_6i);
        if let Some((nla, nha)) = ls_fit_1d(16, &w3, &LS_TAB3, px, 3, total[3] as f32) {
            la = ((rnd(nla).clamp(0,255) as u32)*63+127)/255;
            ha = ((rnd(nha).clamp(0,255) as u32)*63+127)/255;
        }
        let a_sse2 = eval_alpha_weights(px, &mut w3, la as i32, ha as i32, 7, from_6i);
        let sse = rgb_sse + (a_sse + a_sse2) / 2;
        let mut cand = [0u8; 16];
        encode_mode4(&mut cand, lr as u32, lg as u32, lb as u32, la, hr as u32, hg as u32, hb as u32, ha, &w3, &w2, rot_idx, 0);
        if sse < best_sse { best_sse = sse; *block = cand; }
    }

    best_sse
}

// ─── 9. MODE 6 P-BIT OPTIMIZATION ────────────────────────────────────────────
//
// The C++ reference `determine_unique_pbits` finds the best pair (p0, p1) for
// Mode 6 endpoints given float-normalised [0,1] endpoint values.
// For Mode 6, each endpoint is 7-bit + 1 p-bit = 8-bit total after expansion:
//   expanded = (raw_7bit << 1) | p  →  replicated = (expanded << 0) (it's already 8-bit)
// We try p0 ∈ {0,1} and p1 ∈ {0,1} independently to minimise the endpoint
// quantisation error, then re-run weight evaluation with the optimised p-bits.

/// Given float endpoints `xl` and `xh` (each channel in [0,255]), find the pair of
/// p-bits (p0 for lo, p1 for hi) that minimises quantisation error for Mode 6
/// (7-bit endpoints with unique p-bits).
///
/// Returns (p0, p1, quantised_lo[4], quantised_hi[4]) where each component is
/// the 7-bit stored value.
pub fn determine_unique_pbits_m6(
    xl: &[f32; 4], xh: &[f32; 4],
) -> (u32, u32, [u32; 4], [u32; 4]) {
    // For each endpoint independently, try p=0 and p=1 and pick the one with
    // lower squared error to the target (in 8-bit space after expansion).
    // The 8-bit expansion for Mode 6 is: expand8 = (q7 << 1) | p
    // where q7 is the 7-bit stored value.  After that the GPU replicates the
    // top bit into the bottom: final = expand8 | (expand8 >> 8) = expand8
    // (since Mode 6 uses 8-bit endpoints directly from the (7+pbit) concat).

    let mut best_p0 = 0u32;
    let mut best_p1 = 0u32;
    let mut best_lo = [0u32; 4];
    let mut best_hi = [0u32; 4];
    let mut best_err0 = f32::MAX;
    let mut best_err1 = f32::MAX;

    for p in 0u32..2 {
        // Quantise lo endpoint with this p-bit
        let mut lo_q = [0u32; 4];
        let mut err0 = 0.0f32;
        for c in 0..4 {
            // q7 = round((x - p) / 2) constrained so that (q7<<1)|p stays 0..=255
            let x = xl[c];
            // nearest 7-bit value whose expanded form minimises |x - ((q7<<1)|p)|
            let q7 = ((x - p as f32) * 0.5 + 0.5) as i32;
            let q7 = q7.clamp(0, 127) as u32;
            let expanded = (q7 << 1) | p;
            err0 += (expanded as f32 - x) * (expanded as f32 - x);
            lo_q[c] = q7;
        }
        if err0 < best_err0 {
            best_err0 = err0;
            best_p0 = p;
            best_lo = lo_q;
        }
        // Quantise hi endpoint with this p-bit
        let mut hi_q = [0u32; 4];
        let mut err1 = 0.0f32;
        for c in 0..4 {
            let x = xh[c];
            let q7 = ((x - p as f32) * 0.5 + 0.5) as i32;
            let q7 = q7.clamp(0, 127) as u32;
            let expanded = (q7 << 1) | p;
            err1 += (expanded as f32 - x) * (expanded as f32 - x);
            hi_q[c] = q7;
        }
        if err1 < best_err1 {
            best_err1 = err1;
            best_p1 = p;
            best_hi = hi_q;
        }
    }

    (best_p0, best_p1, best_lo, best_hi)
}

/// Full Mode 6 RGBA encoder with p-bit optimisation.
/// Runs initial LS, then tries all 4 p-bit combos and keeps the best SSE result.
pub fn pack_bc7_mode6_rgba_full(
    block: &mut [u8; 16], pixels: &[Pixel; 16],
    tr: f32, tg: f32, tb: f32, ta: f32,
    lo_init: [i32; 4], hi_init: [i32; 4],
    use_pbit_opt: bool,
) -> u32 {
    let mut best_sse = u32::MAX;
    // Candidates: either all 4 p-bit combos, or just p0=p1=0
    let p_range: &[(u32, u32)] = if use_pbit_opt {
        &[(0,0),(0,1),(1,0),(1,1)]
    } else {
        &[(0,0)]
    };
    let all_px: &[Pixel] = pixels;
    for &(p0, p1) in p_range {
        // Convert initial endpoints to the right 7-bit values for this p-bit
        let lr = to_7p(lo_init[0], p0 as i32) as i32;
        let lg = to_7p(lo_init[1], p0 as i32) as i32;
        let lb = to_7p(lo_init[2], p0 as i32) as i32;
        let la = to_7p(lo_init[3], p0 as i32) as i32;
        let hr = to_7p(hi_init[0], p1 as i32) as i32;
        let hg = to_7p(hi_init[1], p1 as i32) as i32;
        let hb = to_7p(hi_init[2], p1 as i32) as i32;
        let ha = to_7p(hi_init[3], p1 as i32) as i32;
        let mut w = [0u8; 16];
        eval_m6_rgba(pixels, &mut w, lr, lg, lb, la, p0, hr, hg, hb, ha, p1);
        // LS refinement with this p-bit
        let (mut flr, mut flg, mut flb, mut fla) = (lr, lg, lb, la);
        let (mut fhr, mut fhg, mut fhb, mut fha) = (hr, hg, hb, ha);
        if let Some((lo, hi)) = ls_fit_4d(16, &w, &LS_TAB4, all_px, tr, tg, tb, ta) {
            flr = to_7_clamp(lo[0], p0 as i32);
            flg = to_7_clamp(lo[1], p0 as i32);
            flb = to_7_clamp(lo[2], p0 as i32);
            fla = to_7_clamp(lo[3], p0 as i32);
            fhr = to_7_clamp(hi[0], p1 as i32);
            fhg = to_7_clamp(hi[1], p1 as i32);
            fhb = to_7_clamp(hi[2], p1 as i32);
            fha = to_7_clamp(hi[3], p1 as i32);
        }
        let sse = eval_m6_rgba(pixels, &mut w, flr, flg, flb, fla, p0, fhr, fhg, fhb, fha, p1);
        if sse < best_sse {
            best_sse = sse;
            encode_mode6(block, flr as u32, flg as u32, flb as u32, fla as u32, p0,
                                 fhr as u32, fhg as u32, fhb as u32, fha as u32, p1, &w);
        }
    }
    best_sse
}

/// Full Mode 6 RGB encoder with p-bit optimisation.
pub fn pack_bc7_mode6_rgb_full(
    block: &mut [u8; 16], pixels: &[Pixel; 16],
    tr: f32, tg: f32, tb: f32,
    lo_init: [i32; 3], hi_init: [i32; 3],
    use_pbit_opt: bool,
) -> u32 {
    let mut best_sse = u32::MAX;
    let p_range: &[(u32, u32)] = if use_pbit_opt {
        &[(0,0),(0,1),(1,0),(1,1)]
    } else {
        &[(0,0)]
    };
    let all_px: &[Pixel] = pixels;
    for &(p0, p1) in p_range {
        let lr = to_7p(lo_init[0], p0 as i32) as i32;
        let lg = to_7p(lo_init[1], p0 as i32) as i32;
        let lb = to_7p(lo_init[2], p0 as i32) as i32;
        let hr = to_7p(hi_init[0], p1 as i32) as i32;
        let hg = to_7p(hi_init[1], p1 as i32) as i32;
        let hb = to_7p(hi_init[2], p1 as i32) as i32;
        let mut w = [0u8; 16];
        eval_m6_rgb(pixels, &mut w, lr, lg, lb, hr, hg, hb, p0, p1);
        let (mut flr, mut flg, mut flb) = (lr, lg, lb);
        let (mut fhr, mut fhg, mut fhb) = (hr, hg, hb);
        if let Some((lo, hi)) = ls_fit_3d(16, &w, &LS_TAB4, all_px, tr, tg, tb) {
            flr = to_7_clamp(lo[0], p0 as i32);
            flg = to_7_clamp(lo[1], p0 as i32);
            flb = to_7_clamp(lo[2], p0 as i32);
            fhr = to_7_clamp(hi[0], p1 as i32);
            fhg = to_7_clamp(hi[1], p1 as i32);
            fhb = to_7_clamp(hi[2], p1 as i32);
        }
        let sse = eval_m6_rgb(pixels, &mut w, flr, flg, flb, fhr, fhg, fhb, p0, p1);
        if sse < best_sse {
            best_sse = sse;
            encode_mode6(block, flr as u32, flg as u32, flb as u32, 127, p0,
                                 fhr as u32, fhg as u32, fhb as u32, 127, p1, &w);
        }
    }
    best_sse
}
