//! Atlas pages upscaled 2x, 3x and 4x, stored as RGBA, BC1, BC7 and BC7 with RDO, against the 1x page: size with
//! zstd 19 as one frame and in 128-row strips, and with LZ4 HC; one-thread decode and strips on an 8-thread pool.
//! EC art and land and CC land take Lanczos3; CC art takes MMPX at 2x and 4x and xBRZ at 3x, which has no MMPX.
//! Each page is cut to its first `BAND_ROWS` rows so a 4x band stays a few hundred MB; figures are for that band.
//! BC1 also runs with RDO at a few lambdas, its PSNR beside plain BC1's in the form's name.
//! Usage: upscaled_pages <uddp dir> [ec|cc|both] [all|bc1]
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::{TexArtCcPackage, TexArtEcPackage, TexLandCcPackage, TexLandEcPackage};
use uot_image_postprocess::upscaling::{apply_upscale_passes_owned, UpscaleFilter, UpscalePass};
use uot_texture_codecs::bc1::rdo::Bc1RdoParams;
use uot_texture_codecs::bc1::{decode_bc1_to_rgba8888, encode_bc1, encode_bc1_with_rdo};
use uot_texture_codecs::bc7::{encode_to_bc7_with_rdo_lambda, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const RUNS: usize = 3;
const THREADS: usize = 8;
/// Source rows of each page studied: a band, so the 4x page fits in memory and zstd 19 finishes in seconds.
const BAND_ROWS: usize = 512;
/// Pixel rows in a strip, as the converter's `PAGE_STRIP_ROWS`.
const STRIP_PIXEL_ROWS: usize = 128;
const ZSTD_LEVEL: i32 = 19;
const LZ4_HC_LEVEL: i32 = 12;
const BC1_RDO_LAMBDAS: [f32; 3] = [0.5, 1.0, 2.0];

fn best(mut f: impl FnMut()) -> f64 {
    (0..RUNS).map(|_| { let s = Instant::now(); f(); s.elapsed().as_secs_f64() * 1e3 }).fold(f64::MAX, f64::min)
}

fn mib(bytes: usize) -> f64 { bytes as f64 / 1048576.0 }

/// One stored form: zstd 19 one frame and strips, LZ4 HC one frame. Returns the zstd 19 one-frame size.
fn row(label: &str, data: &[u8], row_bytes: usize, strip_rows: usize, base: usize, pool: &rayon::ThreadPool) -> usize {
    let strip = strip_rows * row_bytes;
    let mut out = vec![0u8; data.len()];
    let one = zstd::bulk::compress(data, ZSTD_LEVEL).unwrap();
    let one_ms = best(|| { zstd::bulk::Decompressor::new().unwrap().decompress_to_buffer(&one, &mut out).unwrap(); });
    let frames: Vec<Vec<u8>> = data.par_chunks(strip).map(|s| zstd::bulk::compress(s, ZSTD_LEVEL).unwrap()).collect();
    let strips: usize = frames.iter().map(Vec::len).sum();
    let par = pool.install(|| best(|| frames.par_iter().zip(out.par_chunks_mut(strip)).for_each(|(f, d)| {
        zstd::bulk::Decompressor::new().unwrap().decompress_to_buffer(f, d).unwrap();
    })));
    assert!(out == data);
    let lz4 = lz4::block::compress(data, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(LZ4_HC_LEVEL)), false).unwrap();
    let lz4_ms = best(|| { lz4::block::decompress_to_buffer(&lz4, Some(data.len() as i32), &mut out).unwrap(); });
    assert!(out == data);
    let base = if base == 0 { one.len() } else { base };
    println!(
        "| {label} | {:.1} | {:.2} | {:.2} | {one_ms:.1} | {:.2} ({:+.1} %) | {par:.1} | {:.2} | {lz4_ms:.1} |",
        mib(data.len()), mib(one.len()), one.len() as f64 / base as f64, mib(strips),
        (strips as f64 / one.len() as f64 - 1.0) * 100.0, mib(lz4.len()),
    );
    one.len()
}

/// PSNR of BC1 blocks against the pixels they encode, over what BC1 can draw (colour where alpha is at least half).
fn bc1_psnr(blocks: &[u8], rgba: &[u8], extent: ImageExtent) -> f64 {
    let decoded = decode_bc1_to_rgba8888(blocks, extent).unwrap();
    let (mut sum, mut n) = (0f64, 0usize);
    for (d, s) in decoded.chunks_exact(4).zip(rgba.chunks_exact(4)) {
        if s[3] < 128 { continue; }
        for c in 0..3 { let e = d[c] as f64 - s[c] as f64; sum += e * e; }
        n += 3;
    }
    if sum == 0.0 { return f64::INFINITY; }
    10.0 * (255.0f64 * 255.0 / (sum / n as f64)).log10()
}

fn study(name: &str, raw: Vec<u8>, width: usize, filters: [UpscaleFilter; 3], pool: &rayon::ThreadPool, only_bc1: bool) {
    let height = raw.len() / (width * 4);
    let (w, h) = (width & !3, height.min(BAND_ROWS) & !3);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h { rgba.extend_from_slice(&raw[y * width * 4..][..w * 4]); }
    println!("\n### {name}: {w}x{h} band of a {width}x{height} page ({:.2} of its rows)\n", h as f64 / height as f64);
    println!("| form | raw MiB | zstd 19 MiB | x 1x RGBA | 1 thread ms | strips MiB | strips {THREADS} threads ms | LZ4 HC MiB | LZ4 HC ms |");
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut base = 0;
    let scaled = [(1, None), (2, Some(filters[0])), (3, Some(filters[1])), (4, Some(filters[2]))];
    for (scale, filter) in scaled {
        let (sw, sh, px) = match filter {
            None => (w, h, rgba.clone()),
            Some(f) => {
                let t = Instant::now();
                let (sw, sh, px, _, _) = apply_upscale_passes_owned(w as u32, h as u32, rgba.clone(), &[UpscalePass::Upscale(f)]);
                eprintln!("{name}: {f:?} in {:.1} s", t.elapsed().as_secs_f64());
                (sw as usize, sh as usize, px)
            }
        };
        let tag = match filter { None => "1x".to_string(), Some(f) => format!("{scale}x {f:?}") };
        let extent = ImageExtent::new(sw as u32, sh as u32).unwrap();
        if base == 0 { base = zstd::bulk::compress(&px, ZSTD_LEVEL).unwrap().len(); }
        if !only_bc1 { row(&format!("{tag} RGBA"), &px, sw * 4, STRIP_PIXEL_ROWS, base, pool); }
        let bc1 = encode_bc1(&px, extent).unwrap();
        row(&format!("{tag} BC1 ({:.2} dB)", bc1_psnr(&bc1, &px, extent)), &bc1, sw / 4 * 8, STRIP_PIXEL_ROWS / 4, base, pool);
        for lambda in BC1_RDO_LAMBDAS {
            let t = Instant::now();
            let rdo = encode_bc1_with_rdo(&px, extent, &Bc1RdoParams { lambda, ..Bc1RdoParams::default() }).unwrap();
            eprintln!("{name} {tag}: BC1 RDO {lambda} in {:.2} s", t.elapsed().as_secs_f64());
            row(&format!("{tag} BC1 RDO {lambda} ({:.2} dB)", bc1_psnr(&rdo, &px, extent)), &rdo, sw / 4 * 8, STRIP_PIXEL_ROWS / 4, base, pool);
        }
        if only_bc1 { continue; }
        for lambda in [0.0f32, 1.0] {
            let bc7 = encode_to_bc7_with_rdo_lambda(&px, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide, lambda).unwrap();
            let form = if lambda == 0.0 { "BC7".to_string() } else { format!("BC7 RDO {lambda}") };
            row(&format!("{tag} {form}"), bc7.blocks(), sw / 4 * 16, STRIP_PIXEL_ROWS / 4, base, pool);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap();
    let pages = args.next().unwrap_or_else(|| "both".into());
    let only_bc1 = args.next().as_deref() == Some("bc1");
    let pool = rayon::ThreadPoolBuilder::new().num_threads(THREADS).build().unwrap();
    let lanczos = [UpscaleFilter::Lanczos3_2x, UpscaleFilter::Lanczos3_3x, UpscaleFilter::Lanczos3_4x];
    if pages != "cc" {
        let art = TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap();
        study("EC art page 0", art.read_page_bytes(0).unwrap(), art.pages()[0].used_width as usize, lanczos, &pool, only_bc1);
        let land = TexLandEcPackage::load(format!("{dir}/tex_land_ec.uddp")).unwrap();
        study("EC land page 0", land.read_page_bytes(0).unwrap(), land.pages()[0].used_width as usize, lanczos, &pool, only_bc1);
    }
    if pages != "ec" {
        let art = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
        let pixel_art = [UpscaleFilter::Mmpx2x, UpscaleFilter::Xbrz3x, UpscaleFilter::Mmpx4x];
        study("CC art page 0", art.read_page_bytes(0).unwrap(), art.pages()[0].used_width as usize, pixel_art, &pool, only_bc1);
        let land = TexLandCcPackage::load(format!("{dir}/tex_land_cc.uddp")).unwrap();
        study("CC land page 0", land.read_page_bytes(0).unwrap(), land.pages()[0].used_width as usize, lanczos, &pool, only_bc1);
    }
}
