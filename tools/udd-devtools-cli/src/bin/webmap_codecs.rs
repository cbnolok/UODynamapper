//! Colour-fidelity probe for web-map tile codecs, fed explicit PNG crops.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use clap::Parser;

use ssimulacra2::{ColorPrimaries, Rgb, TransferCharacteristic, compute_frame_ssimulacra2};

const CROP: u32 = 512;
const CHANNELS: usize = 3;
const TIMING_RUNS: usize = 3;
/// Dark saturated pixels: the largest channel in this range and (max - min) / max at least `SATURATION_MIN`.
const DARK_MAX_CHANNEL: u8 = 128;
const DARK_MIN_CHANNEL: u8 = 16;
const SATURATION_MIN: f32 = 0.5;
const P95: f64 = 0.95;
const BITS_PER_BYTE: f64 = 8.0;

#[derive(Clone, Copy, Debug)]
enum Codec {
    /// Lossless WebP: quality (effort), method.
    WebpLossless(f32, i32),
    /// Lossy WebP method 4, sharp YUV: quality.
    WebpSharp(f32),
    /// avifenc speed 6, 4:4:4: quality.
    Avif444(u8),
}

impl Codec {
    fn label(self) -> String {
        match self {
            Codec::WebpLossless(q, m) => format!("WebP lossless q{q} m{m}"),
            Codec::WebpSharp(q) => format!("WebP m4 sharp YUV q{q}"),
            Codec::Avif444(q) => format!("AVIF 4:4:4 s6 q{q}"),
        }
    }
}

const CODECS: &[Codec] = &[
    Codec::WebpLossless(75.0, 4),
    Codec::WebpSharp(85.0),
    Codec::WebpSharp(90.0),
    Codec::WebpSharp(95.0),
    Codec::Avif444(70),
    Codec::Avif444(80),
    Codec::Avif444(90),
];

fn encode_webp(rgb: &[u8], codec: Codec) -> Vec<u8> {
    use libwebp_sys::*;
    unsafe {
        let mut config = WebPConfig::new().unwrap();
        match codec {
            Codec::WebpLossless(q, m) => {
                config.lossless = 1;
                config.quality = q;
                config.method = m;
                config.exact = 1;
            }
            Codec::WebpSharp(q) => {
                config.quality = q;
                config.method = 4;
                config.use_sharp_yuv = 1;
            }
            _ => unreachable!(),
        }
        assert!(WebPValidateConfig(&config) != 0);
        let mut picture = WebPPicture::new().unwrap();
        picture.width = CROP as i32;
        picture.height = CROP as i32;
        picture.use_argb = 1;
        assert!(WebPPictureImportRGB(&mut picture, rgb.as_ptr(), (CROP as usize * CHANNELS) as i32) != 0);
        let mut writer = std::mem::zeroed::<WebPMemoryWriter>();
        WebPMemoryWriterInit(&mut writer);
        picture.writer = Some(WebPMemoryWrite);
        picture.custom_ptr = (&raw mut writer).cast();
        assert!(WebPEncode(&config, &mut picture) != 0, "WebPEncode failed: {:?}", picture.error_code);
        let out = std::slice::from_raw_parts(writer.mem, writer.size).to_vec();
        WebPMemoryWriterClear(&mut writer);
        WebPPictureFree(&mut picture);
        out
    }
}

fn decode_webp(data: &[u8]) -> Vec<u8> {
    use libwebp_sys::*;
    unsafe {
        let (mut w, mut h) = (0, 0);
        let ptr = WebPDecodeRGB(data.as_ptr(), data.len(), &mut w, &mut h);
        assert!(!ptr.is_null() && w as u32 == CROP && h as u32 == CROP);
        let out = std::slice::from_raw_parts(ptr, (w * h) as usize * CHANNELS).to_vec();
        WebPFree(ptr.cast());
        out
    }
}

/// avifenc/avifdec: (bytes, decoded, encode ms, decode ms), best of the runs, process start included.
fn avif(png: &Path, work: &Path, quality: u8) -> (Vec<u8>, Vec<u8>, f64, f64) {
    let avif = work.join(format!("{}.q{quality}.avif", png.file_stem().unwrap().to_string_lossy()));
    let back = work.join(format!("{}.q{quality}.avif.png", png.file_stem().unwrap().to_string_lossy()));
    let (mut enc, mut dec) = (f64::MAX, f64::MAX);
    for _ in 0..TIMING_RUNS {
        let start = Instant::now();
        let status = Command::new("avifenc")
            .args(["-s", "6", "-j", "1", "-y", "444", "-q", &quality.to_string()])
            .arg(png)
            .arg(&avif)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        enc = enc.min(start.elapsed().as_secs_f64() * 1e3);
        let start = Instant::now();
        let status = Command::new("avifdec")
            .args(["-j", "1"])
            .arg(&avif)
            .arg(&back)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        dec = dec.min(start.elapsed().as_secs_f64() * 1e3);
    }
    let decoded = image::open(&back).unwrap().to_rgb8().into_raw();
    (std::fs::read(&avif).unwrap(), decoded, enc, dec)
}

fn to_rgb_frame(rgb: &[u8]) -> Rgb {
    let data = rgb
        .chunks_exact(CHANNELS)
        .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
        .collect();
    Rgb::new(data, CROP as usize, CROP as usize, TransferCharacteristic::SRGB, ColorPrimaries::BT709).unwrap()
}

fn srgb_to_lab(p: &[u8]) -> [f64; 3] {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let delta: f64 = 6.0 / 29.0;
    let f = |t: f64| if t > delta.powi(3) { t.cbrt() } else { t / (3.0 * delta * delta) + 4.0 / 29.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn ciede2000(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let [l1, a1, b1] = lab1;
    let [l2, a2, b2] = lab2;
    let pow25_7 = 25f64.powi(7);
    let cbar = ((a1 * a1 + b1 * b1).sqrt() + (a2 * a2 + b2 * b2).sqrt()) / 2.0;
    let g = 0.5 * (1.0 - (cbar.powi(7) / (cbar.powi(7) + pow25_7)).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |b: f64, a: f64| if b == 0.0 && a == 0.0 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d.abs() <= 180.0 { d } else if d > 180.0 { d - 360.0 } else { d + 360.0 }
    };
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();
    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hbarp - 30.0).to_radians().cos() + 0.24 * (2.0 * hbarp).to_radians().cos()
        + 0.32 * (3.0 * hbarp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hbarp - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hbarp - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cbarp.powi(7) / (cbarp.powi(7) + pow25_7)).sqrt();
    let sl = 1.0 + 0.015 * (lbarp - 50.0).powi(2) / (20.0 + (lbarp - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -(2.0 * dtheta).to_radians().sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh)).sqrt()
}

fn is_dark_saturated(p: &[u8]) -> bool {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    (DARK_MIN_CHANNEL..=DARK_MAX_CHANNEL).contains(&max) && (max - min) as f32 / max as f32 >= SATURATION_MIN
}

#[derive(Default)]
struct Totals {
    bytes: u64,
    log_bpp: f64,
    zero_bpp: bool,
    ssim: Vec<f64>,
    abs_sum: [f64; CHANNELS],
    max_abs: [u8; CHANNELS],
    signed_sum: [f64; CHANNELS],
    pixels: f64,
    water_red: Option<f64>,
    delta_e: Vec<f64>,
    enc_ms: f64,
    dec_ms: f64,
    crops: usize,
}

fn best_of<T>(mut run: impl FnMut() -> T) -> (T, f64) {
    let mut best = f64::MAX;
    let mut out = None;
    for _ in 0..TIMING_RUNS {
        let start = Instant::now();
        let value = run();
        best = best.min(start.elapsed().as_secs_f64() * 1e3);
        out = Some(value);
    }
    (out.unwrap(), best)
}

#[derive(Parser)]
#[command(about = "Compare WebP and AVIF on explicit 512x512 PNG crops")]
struct Arguments {
    #[arg(long)]
    output: PathBuf,
    /// PNG crops of exactly 512x512, with distinct file stems.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Skip AVIF when avifenc/avifdec are unavailable.
    #[arg(long)]
    skip_avif: bool,
    /// Optional input file stem for the zero-red water colour diagnostic.
    #[arg(long)]
    water_crop: Option<String>,
}

fn main() {
    let args = Arguments::parse();
    // Sharma, Wu and Dalal (2005), pair 1.
    let check = ciede2000([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485]);
    assert!((check - 2.0425).abs() < 1e-4, "CIEDE2000 self-check {check}");
    let out = args.output;
    let codecs: Vec<Codec> = CODECS.iter().copied().filter(|codec| !args.skip_avif || !matches!(codec, Codec::Avif444(_))).collect();
    let crops_dir = out.join("crops");
    std::fs::create_dir_all(&crops_dir).unwrap();
    let mut crops = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for input in args.inputs {
        let name = input.file_stem().and_then(|name| name.to_str()).expect("a UTF-8 crop name").to_owned();
        assert!(names.insert(name.clone()), "duplicate crop name: {name}");
        let crop = image::open(&input).expect("read PNG crop").to_rgb8();
        assert_eq!(crop.dimensions(), (CROP, CROP), "{}: expected 512x512", input.display());
        let png = crops_dir.join(format!("{name}.png"));
        crop.save(&png).unwrap();
        crops.push((name, crop.into_raw(), png));
    }
    if let Some(name) = &args.water_crop {
        assert!(names.contains(name), "water crop is not one of the inputs: {name}");
    }
    let pixels = (CROP * CROP) as f64;
    let mut rows = String::from("crop\tcodec\tbytes\tbpp\tssim2\tmaeR\tmaeG\tmaeB\tmaxR\tmaxG\tmaxB\tenc_ms\tdec_ms\n");
    let mut totals: Vec<Totals> = codecs.iter().map(|_| Totals::default()).collect();
    for (name, source, png) in &crops {
        let dark: Vec<usize> = (0..source.len() / CHANNELS).filter(|&i| is_dark_saturated(&source[i * 3..i * 3 + 3])).collect();
        let source_lab: Vec<[f64; 3]> = dark.iter().map(|&i| srgb_to_lab(&source[i * 3..i * 3 + 3])).collect();
        for (codec, total) in codecs.iter().zip(&mut totals) {
            let (data, decoded, enc, dec) = match codec {
                Codec::WebpLossless(..) | Codec::WebpSharp(..) => {
                    let (data, enc) = best_of(|| encode_webp(source, *codec));
                    let (decoded, dec) = best_of(|| decode_webp(&data));
                    (data, decoded, enc, dec)
                }
                Codec::Avif444(q) => avif(png, &crops_dir, *q),
            };
            assert_eq!(decoded.len(), source.len(), "{name} {}", codec.label());
            if matches!(codec, Codec::WebpLossless(..)) {
                assert_eq!(&decoded, source, "{name} {}: lossless", codec.label());
            }
            let ssim = compute_frame_ssimulacra2(to_rgb_frame(source), to_rgb_frame(&decoded)).unwrap();
            let mut abs = [0f64; CHANNELS];
            let mut signed = [0f64; CHANNELS];
            let mut max = [0u8; CHANNELS];
            for (s, d) in source.chunks_exact(CHANNELS).zip(decoded.chunks_exact(CHANNELS)) {
                for c in 0..CHANNELS {
                    let diff = d[c] as i32 - s[c] as i32;
                    abs[c] += diff.unsigned_abs() as f64;
                    signed[c] += diff as f64;
                    max[c] = max[c].max(diff.unsigned_abs() as u8);
                }
            }
            if args.water_crop.as_deref() == Some(name.as_str()) {
                let red: Vec<f64> = source
                    .chunks_exact(CHANNELS)
                    .zip(decoded.chunks_exact(CHANNELS))
                    .filter(|(s, _)| s[0] == 0)
                    .map(|(_, d)| d[0] as f64)
                    .collect();
                if !red.is_empty() {
                    total.water_red = Some(red.iter().sum::<f64>() / red.len() as f64);
                }
            }
            for (&i, lab) in dark.iter().zip(&source_lab) {
                total.delta_e.push(ciede2000(*lab, srgb_to_lab(&decoded[i * 3..i * 3 + 3])));
            }
            let bpp = data.len() as f64 * BITS_PER_BYTE / pixels;
            total.bytes += data.len() as u64;
            total.log_bpp += bpp.ln();
            total.zero_bpp |= bpp == 0.0;
            total.ssim.push(ssim);
            for c in 0..CHANNELS {
                total.abs_sum[c] += abs[c];
                total.signed_sum[c] += signed[c];
                total.max_abs[c] = total.max_abs[c].max(max[c]);
            }
            total.pixels += pixels;
            total.enc_ms += enc;
            total.dec_ms += dec;
            total.crops += 1;
            rows.push_str(&format!(
                "{name}\t{}\t{}\t{bpp:.4}\t{ssim:.2}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{enc:.1}\t{dec:.2}\n",
                codec.label(),
                data.len(),
                abs[0] / pixels,
                abs[1] / pixels,
                abs[2] / pixels,
                max[0],
                max[1],
                max[2]
            ));
        }
        eprintln!("{name}: {} dark saturated pixels", dark.len());
    }
    std::fs::write(out.join("per_crop.tsv"), rows).unwrap();
    println!("| Setting | Total KB | bpp (geo mean) | SSIMULACRA2 mean / min | MAE R / G / B | Bias R / G / B | Max err R / G / B | Water mean red | ΔE2000 dark sat. mean / p95 / max | Encode ms | Decode ms |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (codec, total) in codecs.iter().zip(&mut totals) {
        let n = total.crops as f64;
        total.delta_e.sort_by(f64::total_cmp);
        let de = &total.delta_e;
        let de_summary = if de.is_empty() {
            "n/a".to_owned()
        } else {
            let mean = de.iter().sum::<f64>() / de.len() as f64;
            let p95 = de[((de.len() as f64 * P95) as usize).min(de.len() - 1)];
            format!("{mean:.2} / {p95:.2} / {:.1}", de.last().unwrap())
        };
        let ssim_mean = total.ssim.iter().sum::<f64>() / n;
        let ssim_min = total.ssim.iter().copied().fold(f64::MAX, f64::min);
        let mae = total.abs_sum.map(|s| s / total.pixels);
        let bias = total.signed_sum.map(|s| s / total.pixels);
        println!(
            "| {} | {:.0} | {:.3} | {:.1} / {:.1} | {:.2} / {:.2} / {:.2} | {:+.2} / {:+.2} / {:+.2} | {} / {} / {} | {} | {} | {:.1} | {:.2} |",
            codec.label(),
            total.bytes as f64 / 1024.0,
            if total.zero_bpp { 0.0 } else { (total.log_bpp / n).exp() },
            ssim_mean,
            ssim_min,
            mae[0],
            mae[1],
            mae[2],
            bias[0],
            bias[1],
            bias[2],
            total.max_abs[0],
            total.max_abs[1],
            total.max_abs[2],
            total.water_red.map_or_else(|| "n/a".to_owned(), |value| format!("{value:.2}")),
            de_summary,
            total.enc_ms / n,
            total.dec_ms / n
        );
    }
}
