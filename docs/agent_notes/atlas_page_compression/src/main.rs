//! Times one atlas page through each codec: compressed size, single-thread decode, encode time, and Zstd split into
//! independently decoded strips. Pages come from `tex_art_ec.uddp` and `tex_art_cc.uddp` in the directory given.

use std::io::{Read, Write};
use std::time::Instant;
use udd_assets::{TexArtCcPackage, TexArtEcPackage};

/// Each decode is timed this many times and the best run is kept.
const DECODE_RUNS: usize = 3;
/// Rows of pixels per strip in the strip rows.
const STRIP_ROWS: usize = 128;
const RGBA_BYTES: usize = 4;

fn elapsed_ms(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1e3 }

fn best_ms(mut run: impl FnMut()) -> f64 {
    (0..DECODE_RUNS).map(|_| { let start = Instant::now(); run(); elapsed_ms(start) }).fold(f64::MAX, f64::min)
}

fn row(name: &str, raw: &[u8], encode: impl FnOnce(&[u8]) -> Vec<u8>, decode: impl Fn(&[u8]) -> Vec<u8>) {
    let start = Instant::now();
    let compressed = encode(raw);
    let encode_ms = elapsed_ms(start);
    assert!(decode(&compressed) == raw, "{name} does not round-trip");
    let decode_ms = best_ms(|| { std::hint::black_box(decode(&compressed)); });
    println!("  {name:22} {:6.2} MiB ({:4.1}:1)  decode {decode_ms:7.1} ms ({:5.0} MB/s)  encode {encode_ms:7.0} ms",
        compressed.len() as f64 / 1048576.0, raw.len() as f64 / compressed.len() as f64,
        raw.len() as f64 / 1e3 / decode_ms);
}

fn read_all(mut reader: impl Read, size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    reader.read_to_end(&mut out).unwrap();
    out
}

fn study(name: &str, raw: &[u8], width: usize) {
    let n = raw.len();
    println!("{name}: {:.1} MiB raw", n as f64 / 1048576.0);
    println!("  {:22} decode {:7.1} ms", "memcpy", best_ms(|| { std::hint::black_box(raw.to_vec()); }));
    for level in [1, 3, 7, 12, 19] {
        row(&format!("zstd {level}"), raw, |r| zstd::bulk::compress(r, level).unwrap(),
            |c| zstd::bulk::decompress(c, n).unwrap());
    }
    let strip = STRIP_ROWS * width * RGBA_BYTES;
    let strips: Vec<Vec<u8>> = raw.chunks(strip).map(|s| zstd::bulk::compress(s, 19).unwrap()).collect();
    let strip_bytes: usize = strips.iter().map(Vec::len).sum();
    let strip_ms = best_ms(|| {
        let mut out = vec![0u8; n];
        std::thread::scope(|scope| for (frame, dst) in strips.iter().zip(out.chunks_mut(strip)) {
            scope.spawn(move || { zstd::bulk::Decompressor::new().unwrap().decompress_to_buffer(frame, dst).unwrap(); });
        });
        std::hint::black_box(out);
    });
    println!("  {:22} {:6.2} MiB, {} strips decoded in parallel in {strip_ms:.1} ms",
        format!("zstd 19, {STRIP_ROWS}-row strips"), strip_bytes as f64 / 1048576.0, strips.len());
    for (name, mode) in [("lz4", lz4::block::CompressionMode::DEFAULT),
        ("lz4 hc 9", lz4::block::CompressionMode::HIGHCOMPRESSION(9)),
        ("lz4 hc 12", lz4::block::CompressionMode::HIGHCOMPRESSION(12))] {
        row(name, raw, |r| lz4::block::compress(r, Some(mode), false).unwrap(),
            |c| lz4::block::decompress(c, Some(n as i32)).unwrap());
    }
    for level in [6u32, 9] {
        row(&format!("zlib-ng {level}"), raw, |r| {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level));
            e.write_all(r).unwrap();
            e.finish().unwrap()
        }, |c| read_all(flate2::read::ZlibDecoder::new(c), n));
    }
    for level in [9, 12] {
        row(&format!("libdeflate {level}"), raw, |r| {
            let mut e = libdeflater::Compressor::new(libdeflater::CompressionLvl::new(level).unwrap());
            let mut c = vec![0; e.deflate_compress_bound(r.len())];
            let len = e.deflate_compress(r, &mut c).unwrap();
            c.truncate(len);
            c
        }, |c| { let mut o = vec![0; n]; libdeflater::Decompressor::new().deflate_decompress(c, &mut o).unwrap(); o });
    }
    for (quality, window) in [(9u32, 22u32), (11, 24)] {
        row(&format!("brotli q{quality} w{window}"), raw, |r| {
            let mut c = Vec::new();
            brotli::CompressorWriter::new(&mut c, 1 << 16, quality, window).write_all(r).unwrap();
            c
        }, |c| read_all(brotli::Decompressor::new(c, 1 << 16), n));
    }
    row("bzip2 9", raw, |r| {
        let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
        e.write_all(r).unwrap();
        e.finish().unwrap()
    }, |c| read_all(bzip2::read::BzDecoder::new(c), n));
    for level in [6u32, 9] {
        row(&format!("xz {level}"), raw, |r| {
            let mut e = liblzma::write::XzEncoder::new(Vec::new(), level);
            e.write_all(r).unwrap();
            e.finish().unwrap()
        }, |c| read_all(liblzma::read::XzDecoder::new(c), n));
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: atlas-page-compression-bench <directory of .uddp packages>");
    let ec = TexArtEcPackage::load(format!("{dir}/tex_art_ec.uddp")).unwrap();
    for page in [0u32, 4] {
        study(&format!("EC art page {page}"), &ec.read_page_bytes(page).unwrap(), ec.pages()[page as usize].used_width as usize);
    }
    let cc = TexArtCcPackage::load(format!("{dir}/tex_art_cc.uddp")).unwrap();
    study("CC art page 0", &cc.read_page_bytes(0).unwrap(), cc.pages()[0].used_width as usize);
    println!("threads: {}", std::thread::available_parallelism().unwrap());
}
