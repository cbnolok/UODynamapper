//! Every Zstd level, a few Zstd parameters and the other codecs, each in strips of `rows` pixel rows (default 128):
//! size against one Zstd 19 frame, one-thread decode of all strips (the CPU cost), and decode on 4, 8 and 20 threads.
//! Usage: strip_levels <uddp dir> [rows] [codec name filter]
use rayon::prelude::*;
use std::io::{Read, Write};
use std::time::Instant;
use udd_assets::{TexArtCcPackage, TexArtEcPackage, TexLandEcPackage};
use zstd::zstd_safe::{CParameter, ParamSwitch};

const RUNS: usize = 5;
const RGBA: usize = 4;

fn best(mut f: impl FnMut()) -> f64 {
    (0..RUNS).map(|_| { let s = Instant::now(); f(); s.elapsed().as_secs_f64() * 1e3 }).fold(f64::MAX, f64::min)
}

type Enc = Box<dyn Fn(&[u8]) -> Vec<u8> + Sync>;
type Dec = Box<dyn Fn(&[u8], &mut [u8]) + Sync>;

fn zstd_enc(level: i32, min_match: Option<u32>, raw_literals: bool) -> Enc {
    Box::new(move |r| {
        let mut c = zstd::bulk::Compressor::new(level).unwrap();
        if let Some(m) = min_match { c.set_parameter(CParameter::MinMatch(m)).unwrap(); }
        if raw_literals { c.set_parameter(CParameter::LiteralCompressionMode(ParamSwitch::Disable)).unwrap(); }
        c.compress(r).unwrap()
    })
}
fn zstd_dec() -> Dec {
    Box::new(|c, o| { let n = zstd::bulk::Decompressor::new().unwrap().decompress_to_buffer(c, o).unwrap(); assert_eq!(n, o.len()); })
}
fn read_into(mut r: impl Read, o: &mut [u8]) { r.read_exact(o).unwrap(); }

fn configs() -> Vec<(String, Enc, Dec)> {
    let mut v: Vec<(String, Enc, Dec)> = Vec::new();
    for l in [3, 7, 9, 12, 15, 16, 17, 18, 19, 22] { v.push((format!("zstd {l}"), zstd_enc(l, None, false), zstd_dec())); }
    for m in [5, 6, 7] { v.push((format!("zstd 19 minmatch {m}"), zstd_enc(19, Some(m), false), zstd_dec())); }
    v.push(("zstd 19 raw literals".into(), zstd_enc(19, None, true), zstd_dec()));
    v.push(("zstd 12 minmatch 6".into(), zstd_enc(12, Some(6), false), zstd_dec()));
    v.push(("lz4 hc 12".into(),
        Box::new(|r| lz4::block::compress(r, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(12)), false).unwrap()),
        Box::new(|c, o| { let n = lz4::block::decompress_to_buffer(c, Some(o.len() as i32), o).unwrap(); assert_eq!(n, o.len()); })));
    v.push(("libdeflate 12".into(), Box::new(|r| {
        let mut e = libdeflater::Compressor::new(libdeflater::CompressionLvl::new(12).unwrap());
        let mut c = vec![0; e.deflate_compress_bound(r.len())];
        let n = e.deflate_compress(r, &mut c).unwrap(); c.truncate(n); c
    }), Box::new(|c, o| { libdeflater::Decompressor::new().deflate_decompress(c, o).unwrap(); })));
    v.push(("brotli q11 w22".into(), Box::new(|r| {
        let mut c = Vec::new(); brotli::CompressorWriter::new(&mut c, 1 << 16, 11, 22).write_all(r).unwrap(); c
    }), Box::new(|c, o| read_into(brotli::Decompressor::new(c, 1 << 16), o))));
    v.push(("xz 9".into(), Box::new(|r| {
        let mut e = liblzma::write::XzEncoder::new(Vec::new(), 9); e.write_all(r).unwrap(); e.finish().unwrap()
    }), Box::new(|c, o| read_into(liblzma::read::XzDecoder::new(c), o))));
    v
}

fn study(name: &str, raw: &[u8], width: usize, rows: usize, filter: &Option<String>, pools: &[(usize, rayon::ThreadPool)]) {
    let n = raw.len();
    let strip = rows * width * RGBA;
    // Reference: zstd 19 one frame.
    let one = zstd::bulk::compress(raw, 19).unwrap();
    let mut out = vec![0u8; n];
    let one_ms = best(|| { zstd_dec()(&one, &mut out); });
    println!("{name}: {:.1} MiB raw, {} strips of {rows} rows; zstd 19 one frame {:.2} MiB, {one_ms:.1} ms",
        n as f64 / 1048576.0, n.div_ceil(strip), one.len() as f64 / 1048576.0);
    print!("  {:24} {:>8} {:>8} {:>9}", "codec", "MiB", "vs one19", "cpu ms");
    for (t, _) in pools { print!(" {:>8}", format!("{t}t ms")); }
    println!();
    for (cname, enc, dec) in configs() {
        if let Some(f) = filter { if !cname.contains(f.as_str()) { continue; } }
        let frames: Vec<Vec<u8>> = raw.par_chunks(strip).map(|s| enc(s)).collect();
        let size: usize = frames.iter().map(Vec::len).sum();
        let mut out = vec![0u8; n];
        // CPU cost: all strips on one thread.
        let cpu = best(|| for (f, d) in frames.iter().zip(out.chunks_mut(strip)) { dec(f, d); });
        assert!(out == raw, "{cname} round trip");
        print!("  {cname:24} {:8.2} {:+7.1}% {cpu:9.1}", size as f64 / 1048576.0, (size as f64 / one.len() as f64 - 1.0) * 100.0);
        for (_, pool) in pools {
            let ms = pool.install(|| best(|| frames.par_iter().zip(out.par_chunks_mut(strip)).for_each(|(f, d)| dec(f, d))));
            print!(" {ms:8.1}");
        }
        println!();
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap();
    let rows: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(128);
    let filter = args.next();
    let pools: Vec<(usize, rayon::ThreadPool)> = [4usize, 8, 20].iter()
        .map(|&t| (t, rayon::ThreadPoolBuilder::new().num_threads(t).build().unwrap())).collect();
    let ec = TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap();
    for p in [0u32, 4] { study(&format!("EC art page {p}"), &ec.read_page_bytes(p).unwrap(), ec.pages()[p as usize].used_width as usize, rows, &filter, &pools); }
    let land = TexLandEcPackage::load(format!("{dir}/tex_land_ec.uddp")).unwrap();
    let lp = land.pages();
    println!("land page 0 width {} format {:?} pages {}", lp[0].used_width, lp[0].pixel_format, lp.len());
    study("EC land page 0", &land.read_page_bytes(0).unwrap(), lp[0].used_width as usize, rows, &filter, &pools);
    let cc = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
    study("CC art page 0", &cc.read_page_bytes(0).unwrap(), cc.pages()[0].used_width as usize, rows, &filter, &pools);
}
