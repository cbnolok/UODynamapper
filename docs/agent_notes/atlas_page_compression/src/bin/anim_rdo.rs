//! BC7 RDO on mobile animation atlas pages, read from an RGBA8888 `mobile_anim_cc.uddp` or `mobile_anim_ec.uddp`: for
//! each lambda and look-back, the RDO's wall time on the whole pool and its CPU use, the Zstd 9 size of the blocks
//! (the packer's `--zstd=9`) and the PSNR against the RGBA page, beside plain BC7. Pages are sampled evenly.
//! Usage: anim_rdo <uddp dir> <cc|ec> <pages> [lambda,...] [look-back blocks,...] [stats|parallel]
//! `stats` adds the RDO's counters for the first lambda and look-back, run on one thread over the first sampled page.
//! `parallel` runs the pages' RDO at once, one task per page, as the mobile packers do, instead of one after another;
//! a page's RDO still splits by rows on its own.
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage};
use uot_texture_codecs::bc7::rdo::{reduce_entropy_bc7_parallel, reduce_entropy_bc7_with_stats, Bc7RdoParams, Bc7RdoStats};
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const ZSTD_LEVEL: i32 = 9;
const BLOCK_BYTES: usize = 16;
const BLOCK_TEXELS: usize = 16;
const DEFAULT_LAMBDAS: [f32; 4] = [0.05, 0.25, 0.5, 1.0];
const DEFAULT_LOOKBACKS: [usize; 3] = [8, 16, 64];
const PEAK_SQUARED: f64 = 255.0 * 255.0;

/// User plus system CPU seconds of this process so far (Linux `/proc/self/stat`, fields 14 and 15, in clock ticks).
fn process_cpu_seconds() -> f64 {
    const CLOCK_TICKS_PER_SECOND: f64 = 100.0;
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let after_name = &stat[stat.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = after_name.split(' ').collect();
    let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    ticks as f64 / CLOCK_TICKS_PER_SECOND
}

fn block_order(rgba: &[u8], width: usize, height: usize) -> Vec<[u8; 4]> {
    let (bw, bh) = (width.div_ceil(4), height.div_ceil(4));
    let mut out = Vec::with_capacity(bw * bh * BLOCK_TEXELS);
    for by in 0..bh {
        for bx in 0..bw {
            for y in 0..4 {
                let sy = (by * 4 + y).min(height - 1);
                for x in 0..4 {
                    let sx = (bx * 4 + x).min(width - 1);
                    let o = (sy * width + sx) * 4;
                    out.push([rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]);
                }
            }
        }
    }
    out
}

/// Squared errors summed: RGB over the texels the source shows (alpha > 0), and colour times alpha over every texel.
#[derive(Default, Clone, Copy)]
struct Error { rgb_sse: f64, rgb_count: f64, shown_sse: f64, shown_count: f64 }

impl Error {
    fn of(source: &[u8], decoded: &[u8]) -> Self {
        let mut e = Self::default();
        for (s, d) in source.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            if s[3] > 0 {
                for c in 0..3 { e.rgb_sse += (s[c] as f64 - d[c] as f64).powi(2); }
                e.rgb_count += 3.0;
            }
            for c in 0..3 {
                let shown = |p: &[u8]| p[c] as f64 * p[3] as f64 / 255.0;
                e.shown_sse += (shown(s) - shown(d)).powi(2);
            }
            e.shown_sse += (s[3] as f64 - d[3] as f64).powi(2);
            e.shown_count += 4.0;
        }
        e
    }
    fn add(&mut self, o: Self) { self.rgb_sse += o.rgb_sse; self.rgb_count += o.rgb_count; self.shown_sse += o.shown_sse; self.shown_count += o.shown_count; }
    fn psnr(sse: f64, count: f64) -> f64 { 10.0 * (PEAK_SQUARED / (sse / count).max(1e-12)).log10() }
    fn rgb_psnr(&self) -> f64 { Self::psnr(self.rgb_sse, self.rgb_count) }
    fn shown_psnr(&self) -> f64 { Self::psnr(self.shown_sse, self.shown_count) }
}

#[derive(Default, Clone, Copy)]
struct Row { wall: f64, cpu: f64, zstd: usize, error: Error, modified: u64, digest: u64 }

struct Page { width: usize, height: usize, rgba: Vec<u8> }

fn sample_pages(dir: &str, source: &str, count: usize) -> Vec<Page> {
    let pick = |total: usize| -> Vec<usize> { (0..count.min(total)).map(|k| k * total / count.min(total)).collect() };
    let mut pages = Vec::new();
    match source {
        "cc" => {
            let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
            for i in pick(package.pages().len()) {
                let record = package.pages()[i];
                let rgba = package.read_page_rgba(record.page_index).unwrap();
                pages.push(Page { width: record.used_width as usize, height: record.used_height as usize, rgba });
            }
        }
        "ec" => {
            let package = MobileAnimEcPackage::load(format!("{dir}/mobile_anim_ec.uddp")).unwrap();
            for i in pick(package.pages().len()) {
                let record = package.pages()[i];
                let rgba = package.read_page_rgba(record.page_index).unwrap();
                pages.push(Page { width: record.used_width as usize, height: record.used_height as usize, rgba });
            }
        }
        other => panic!("unknown source {other}"),
    }
    for page in &pages { assert_eq!(page.rgba.len(), page.width * page.height * 4); }
    pages
}

fn list<T: std::str::FromStr>(arg: Option<&String>, default: &[T]) -> Vec<T> where T: Copy, T::Err: std::fmt::Debug {
    arg.map(|a| a.split(',').map(|v| v.parse().unwrap()).collect()).unwrap_or_else(|| default.to_vec())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, source, count) = (&args[0], &args[1], args[2].parse::<usize>().unwrap());
    let lambdas: Vec<f32> = list(args.get(3), &DEFAULT_LAMBDAS);
    let lookbacks: Vec<usize> = list(args.get(4), &DEFAULT_LOOKBACKS);
    let with_stats = args.get(5).is_some_and(|a| a == "stats");
    let pages_in_parallel = args.get(5).is_some_and(|a| a == "parallel");
    let pages = sample_pages(dir, source, count);
    let texels: usize = pages.iter().map(|p| p.width * p.height).sum();
    println!("{source}: {} pages, {:.1} Mpx, {} threads", pages.len(), texels as f64 / 1e6, rayon::current_num_threads());

    // Plain BC7, and what the blocks are.
    let mut plain = Vec::new();
    let mut plain_row = Row::default();
    let (mut blocks_total, mut blocks_clear, mut blocks_clear_exact, mut clear_texels_with_colour) = (0u64, 0u64, 0u64, 0u64);
    for page in &pages {
        let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
        let (t, c) = (Instant::now(), process_cpu_seconds());
        let encoded = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide).unwrap();
        plain_row.wall += t.elapsed().as_secs_f64();
        plain_row.cpu += process_cpu_seconds() - c;
        let blocks = encoded.blocks().to_vec();
        let decoded = decode_bc7_to_rgba8888(&blocks, extent).unwrap();
        plain_row.error.add(Error::of(&page.rgba, &decoded));
        plain_row.zstd += zstd::bulk::compress(&blocks, ZSTD_LEVEL).unwrap().len();
        let order = block_order(&page.rgba, page.width, page.height);
        let decoded_order = block_order(&decoded, page.width, page.height);
        for (s, d) in order.chunks_exact(BLOCK_TEXELS).zip(decoded_order.chunks_exact(BLOCK_TEXELS)) {
            blocks_total += 1;
            if s.iter().all(|p| p[3] == 0) {
                blocks_clear += 1;
                if s == d { blocks_clear_exact += 1; }
            }
        }
        clear_texels_with_colour += page.rgba.chunks_exact(4).filter(|p| p[3] == 0 && (p[0] | p[1] | p[2]) != 0).count() as u64;
        plain.push((blocks, order));
    }
    println!("blocks {blocks_total}: all transparent {blocks_clear} ({:.1} %), of which encoded exactly {blocks_clear_exact}; \
        transparent texels with colour {clear_texels_with_colour}",
        blocks_clear as f64 * 100.0 / blocks_total as f64);
    let print = |label: &str, row: &Row| println!(
        "  {label:28} wall {:7.2} s  cpu {:7.1} s ({:4.1} cores)  zstd {ZSTD_LEVEL} {:8.2} MiB ({:+5.1} %)  RGB PSNR {:5.2} dB  shown {:5.2} dB  modified {:5.1} %  blocks {:016x}",
        row.wall, row.cpu, row.cpu / row.wall.max(1e-9), row.zstd as f64 / 1048576.0,
        (row.zstd as f64 / plain_row.zstd as f64 - 1.0) * 100.0, row.error.rgb_psnr(), row.error.shown_psnr(),
        row.modified as f64 * 100.0 / blocks_total as f64, row.digest);
    print("BC7 (encode)", &plain_row);

    for &lookback in &lookbacks {
        for &lambda in &lambdas {
            let params = Bc7RdoParams { lambda, lookback_window_size: lookback * BLOCK_BYTES, skip_zero_mse_blocks: true, ..Default::default() };
            let mut row = Row::default();
            let mut rdo_pages: Vec<Vec<[u8; 16]>> = plain
                .iter()
                .map(|(blocks, _)| blocks.chunks_exact(BLOCK_BYTES).map(|b| b.try_into().unwrap()).collect())
                .collect();
            let rdo_page = |page: &Page, rdo: &mut Vec<[u8; 16]>, order: &[[u8; 4]]| {
                reduce_entropy_bc7_parallel(rdo, order, page.width.div_ceil(4), page.height.div_ceil(4), &params) as u64
            };
            let (t, c) = (Instant::now(), process_cpu_seconds());
            row.modified = if pages_in_parallel {
                pages.par_iter().zip(rdo_pages.par_iter_mut()).zip(plain.par_iter())
                    .map(|((page, rdo), (_, order))| rdo_page(page, rdo, order)).sum()
            } else {
                pages.iter().zip(rdo_pages.iter_mut()).zip(&plain).map(|((page, rdo), (_, order))| rdo_page(page, rdo, order)).sum()
            };
            row.wall = t.elapsed().as_secs_f64();
            row.cpu = process_cpu_seconds() - c;
            let mut digest = std::collections::hash_map::DefaultHasher::new();
            for (page, rdo) in pages.iter().zip(&rdo_pages) {
                let flat: Vec<u8> = rdo.concat();
                std::hash::Hash::hash(&flat, &mut digest);
                let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
                row.error.add(Error::of(&page.rgba, &decode_bc7_to_rgba8888(&flat, extent).unwrap()));
                row.zstd += zstd::bulk::compress(&flat, ZSTD_LEVEL).unwrap().len();
            }
            row.digest = std::hash::Hasher::finish(&digest);
            print(&format!("RDO {lambda}, look-back {lookback}"), &row);
        }
    }

    if with_stats {
        let (page, (blocks, order)) = (&pages[0], &plain[0]);
        let params = Bc7RdoParams { lambda: lambdas[0], lookback_window_size: lookbacks[0] * BLOCK_BYTES, skip_zero_mse_blocks: true, ..Default::default() };
        let mut rdo: Vec<[u8; 16]> = blocks.chunks_exact(BLOCK_BYTES).map(|b| b.try_into().unwrap()).collect();
        let mut stats = Bc7RdoStats::default();
        let t = Instant::now();
        reduce_entropy_bc7_with_stats(&mut rdo, order, page.width.div_ceil(4), page.height.div_ceil(4), &params, &mut stats);
        println!("stats, one thread, first page, {:.2} s: {stats:#?}", t.elapsed().as_secs_f64());
    }
}
