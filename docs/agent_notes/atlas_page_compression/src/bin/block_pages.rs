//! Block-compressed atlas pages (BC1, BC7, BC7 with RDO), at 1x and 2x (Lanczos3), through Zstd levels and LZ4 HC,
//! as one frame and as 16 strips: size, one-thread decode, and decode on an 8-thread pool.
//! Usage: block_pages <uddp dir> [lanczos|mmpx] [ec|cc|both]. MMPX is the filter for Classic's pixel art.
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::{TexArtCcPackage, TexArtEcPackage};
use uot_image_postprocess::upscaling::{apply_upscale_passes_owned, UpscaleFilter, UpscalePass};
use uot_texture_codecs::bc1::encode_bc1;
use uot_texture_codecs::bc7::{encode_to_bc7_with_rdo_lambda, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const RUNS: usize = 5;
const STRIPS: usize = 16;
const THREADS: usize = 8;

fn best(mut f: impl FnMut()) -> f64 {
    (0..RUNS).map(|_| { let s = Instant::now(); f(); s.elapsed().as_secs_f64() * 1e3 }).fold(f64::MAX, f64::min)
}

fn zdec(c: &[u8], o: &mut [u8]) { assert_eq!(zstd::bulk::Decompressor::new().unwrap().decompress_to_buffer(c, o).unwrap(), o.len()); }
fn ldec(c: &[u8], o: &mut [u8]) { assert_eq!(lz4::block::decompress_to_buffer(c, Some(o.len() as i32), o).unwrap(), o.len()); }

fn sweep(label: &str, data: &[u8], row_bytes: usize, rows: usize, pool: &rayon::ThreadPool) {
    let strip = rows.div_ceil(STRIPS) * row_bytes;
    println!("  {label}: {:.2} MiB uncompressed", data.len() as f64 / 1048576.0);
    let codecs: Vec<(String, Box<dyn Fn(&[u8]) -> Vec<u8> + Sync>, fn(&[u8], &mut [u8]))> = vec![
        ("zstd 7".into(), Box::new(|r| zstd::bulk::compress(r, 7).unwrap()), zdec),
        ("zstd 12".into(), Box::new(|r| zstd::bulk::compress(r, 12).unwrap()), zdec),
        ("zstd 17".into(), Box::new(|r| zstd::bulk::compress(r, 17).unwrap()), zdec),
        ("zstd 19".into(), Box::new(|r| zstd::bulk::compress(r, 19).unwrap()), zdec),
        ("lz4 hc 12".into(), Box::new(|r| lz4::block::compress(r, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(12)), false).unwrap()), ldec),
    ];
    let mut out = vec![0u8; data.len()];
    for (name, enc, dec) in &codecs {
        let one = enc(data);
        let one_ms = best(|| dec(&one, &mut out));
        let frames: Vec<Vec<u8>> = data.par_chunks(strip).map(|s| enc(s)).collect();
        let size: usize = frames.iter().map(Vec::len).sum();
        let cpu = best(|| for (f, d) in frames.iter().zip(out.chunks_mut(strip)) { dec(f, d); });
        let par = pool.install(|| best(|| frames.par_iter().zip(out.par_chunks_mut(strip)).for_each(|(f, d)| dec(f, d))));
        assert!(out == data);
        println!("    {name:10} one frame {:6.2} MiB {one_ms:6.1} ms | strips {:6.2} MiB ({:+5.1}%) cpu {cpu:6.1} ms, {THREADS}t {par:5.1} ms",
            one.len() as f64 / 1048576.0, size as f64 / 1048576.0, (size as f64 / one.len() as f64 - 1.0) * 100.0);
    }
}

fn study(name: &str, raw: Vec<u8>, width: usize, upscale: UpscalePass, pool: &rayon::ThreadPool) {
    let height = raw.len() / (width * 4);
    let (w, h) = (width & !3, height & !3);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h { rgba.extend_from_slice(&raw[y * width * 4..][..w * 4]); }
    let t = Instant::now();
    let (w2, h2, rgba2, _, _) = apply_upscale_passes_owned(w as u32, h as u32, rgba.clone(), &[upscale]);
    println!("{name}: {w}x{h}; {upscale:?} to {w2}x{h2} in {:.1} s", t.elapsed().as_secs_f64());
    for (scale, (w, h, px)) in [(1, (w, h, &rgba)), (2, (w2 as usize, h2 as usize, &rgba2))] {
        let extent = ImageExtent::new(w as u32, h as u32).unwrap();
        if scale == 2 { sweep(&format!("RGBA {scale}x"), px, w * 4, h, pool); }
        let t = Instant::now();
        let bc1 = encode_bc1(px, extent).unwrap();
        println!("  (BC1 {scale}x encoded in {:.1} s)", t.elapsed().as_secs_f64());
        sweep(&format!("BC1 {scale}x"), &bc1, w / 4 * 8, h / 4, pool);
        for lambda in [0.0f32, 1.0] {
            let t = Instant::now();
            let bc7 = encode_to_bc7_with_rdo_lambda(px, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide, lambda).unwrap();
            println!("  (BC7 {scale}x RDO {lambda} encoded in {:.1} s)", t.elapsed().as_secs_f64());
            sweep(&format!("BC7 {scale}x RDO lambda {lambda}"), bc7.blocks(), w / 4 * 16, h / 4, pool);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap();
    let upscale = match args.next().as_deref() {
        None | Some("lanczos") => UpscalePass::Upscale(UpscaleFilter::Lanczos3_2x),
        Some("mmpx") => UpscalePass::Upscale(UpscaleFilter::Mmpx2x),
        Some(other) => panic!("unknown filter {other}"),
    };
    let pages = args.next().unwrap_or_else(|| "both".into());
    let pool = rayon::ThreadPoolBuilder::new().num_threads(THREADS).build().unwrap();
    if pages != "cc" {
        let ec = TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap();
        study("EC art page 0", ec.read_page_bytes(0).unwrap(), ec.pages()[0].used_width as usize, upscale, &pool);
    }
    if pages != "ec" {
        let cc = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
        study("CC art page 0", cc.read_page_bytes(0).unwrap(), cc.pages()[0].used_width as usize, upscale, &pool);
    }
}
