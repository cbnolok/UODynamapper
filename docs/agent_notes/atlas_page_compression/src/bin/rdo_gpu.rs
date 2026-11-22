//! The GPU BC7 RDO (uot_gpu_compute::bc7_rdo) against the CPU RDO (parallel, AVX2) on the same mobile animation
//! pages: wall time, Zstd size and RGB PSNR against plain BC7, and whether two GPU runs give the same blocks. Times
//! the RDO pipeline's compile first (cold when MESA_SHADER_CACHE_DIR points at an empty folder). With a crop
//! directory, also writes one page's crop decoded from plain BC7, the CPU RDO and the GPU RDO, as raw RGBA.
//! Usage: rdo_gpu <uddp dir> <cc|ec> <pages> <lambdas, comma-separated> <look-backs, comma-separated>
//!        [<crop dir> <page> <x> <y> <width> <height>]
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage};
use uot_gpu_compute::bc7_rdo::{Bc7RdoGpu, Bc7RdoPage, BC7_RDO_COMPILES};
use uot_gpu_compute::warm_up::CompileReport;
use uot_texture_codecs::bc7::rdo::{reduce_entropy_bc7_parallel, Bc7RdoParams};
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};

const ZSTD_LEVEL: i32 = 9;
const BLOCK_BYTES: usize = 16;
const BLOCK_TEXELS: usize = 16;
const PEAK_SQUARED: f64 = 255.0 * 255.0;
const MIB: f64 = 1048576.0;

struct Page { width: usize, height: usize, rgba: Vec<u8> }

fn sample_pages(dir: &str, source: &str, count: usize) -> Vec<Page> {
    let pick = |total: usize| -> Vec<usize> { (0..count.min(total)).map(|k| k * total / count.min(total)).collect() };
    let mut pages = Vec::new();
    match source {
        "cc" => {
            let package = MobileAnimCcPackage::load(format!("{dir}/mobile_anim_cc.uddp")).unwrap();
            for i in pick(package.pages().len()) {
                let record = package.pages()[i];
                pages.push(Page { width: record.used_width as usize, height: record.used_height as usize,
                    rgba: package.read_page_rgba(record.page_index).unwrap() });
            }
        }
        "ec" => {
            let package = MobileAnimEcPackage::load(format!("{dir}/mobile_anim_ec.uddp")).unwrap();
            for i in pick(package.pages().len()) {
                let record = package.pages()[i];
                pages.push(Page { width: record.used_width as usize, height: record.used_height as usize,
                    rgba: package.read_page_rgba(record.page_index).unwrap() });
            }
        }
        other => panic!("unknown source {other}"),
    }
    pages
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

/// RGB squared error and channel count over the texels with alpha > 0.
fn rgb_sse(source: &[u8], decoded: &[u8]) -> (f64, f64) {
    let (mut sse, mut count) = (0.0, 0.0);
    for (s, d) in source.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        if s[3] > 0 {
            for c in 0..3 { sse += (s[c] as f64 - d[c] as f64).powi(2); }
            count += 3.0;
        }
    }
    (sse, count)
}

fn decode(page: &Page, blocks: &[[u8; 16]]) -> Vec<u8> {
    let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
    decode_bc7_to_rgba8888(&blocks.concat(), extent).unwrap()
}

fn print(label: &str, pages: &[Page], blocks: &[Vec<[u8; 16]>], plain_zstd: usize, plain: &[Vec<[u8; 16]>],
    wall: f64) {
    let rows: Vec<(usize, f64, f64, usize)> = pages.par_iter().zip(blocks).zip(plain).map(|((page, b), p)| {
        let zstd = zstd::bulk::compress(&b.concat(), ZSTD_LEVEL).unwrap().len();
        let (sse, count) = rgb_sse(&page.rgba, &decode(page, b));
        (zstd, sse, count, b.iter().zip(p).filter(|(x, y)| x != y).count())
    }).collect();
    let zstd: usize = rows.iter().map(|row| row.0).sum();
    let sse: f64 = rows.iter().map(|row| row.1).sum();
    let count: f64 = rows.iter().map(|row| row.2).sum();
    let modified: usize = rows.iter().map(|row| row.3).sum();
    let total: usize = blocks.iter().map(Vec::len).sum();
    println!("  {label:22} wall {wall:7.2} s  zstd {:8.2} MiB ({:+5.1} %)  RGB PSNR {:5.2} dB  modified {:5.1} %",
        zstd as f64 / MIB, (zstd as f64 / plain_zstd.max(1) as f64 - 1.0) * 100.0,
        10.0 * (PEAK_SQUARED / (sse / count)).log10(), modified as f64 * 100.0 / total as f64);
}

fn gpu_run(gpu: &Bc7RdoGpu, plain: &[Vec<[u8; 16]>], orders: &[Vec<[u8; 4]>], pages: &[Page],
    params: &Bc7RdoParams) -> (Vec<Vec<[u8; 16]>>, f64) {
    let mut blocks = plain.to_vec();
    let started = Instant::now();
    let mut batch: Vec<Bc7RdoPage> = blocks.iter_mut().zip(orders).zip(pages).map(|((blocks, order), page)| {
        Bc7RdoPage { blocks, rgba_blocks: order, blocks_x: page.width.div_ceil(4), blocks_y: page.height.div_ceil(4) }
    }).collect();
    gpu.reduce_pages(&mut batch, params).unwrap();
    (blocks, started.elapsed().as_secs_f64())
}

/// The packers' way: every page's RDO called from the rayon threads at once, batched by the GPU RDO.
fn gpu_concurrent_run(gpu: &Bc7RdoGpu, plain: &[Vec<[u8; 16]>], orders: &[Vec<[u8; 4]>], pages: &[Page],
    params: &Bc7RdoParams) -> (Vec<Vec<[u8; 16]>>, f64) {
    let mut blocks = plain.to_vec();
    let started = Instant::now();
    blocks.par_iter_mut().zip(orders).zip(pages).for_each(|((blocks, order), page)| {
        gpu.reduce_entropy(blocks, order, page.width.div_ceil(4), page.height.div_ceil(4), params).unwrap();
    });
    (blocks, started.elapsed().as_secs_f64())
}

fn write_crop(path: &str, page: &Page, rgba: &[u8], (x, y, width, height): (usize, usize, usize, usize)) {
    let mut out = Vec::with_capacity(width * height * 4);
    for row in y..y + height {
        out.extend_from_slice(&rgba[(row * page.width + x) * 4..][..width * 4]);
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, source, count) = (&args[0], &args[1], args[2].parse::<usize>().unwrap());
    let lambdas: Vec<f32> = args[3].split(',').map(|v| v.parse().unwrap()).collect();
    let lookbacks: Vec<usize> = args[4].split(',').map(|v| v.parse().unwrap()).collect();

    let (device, queue, adapter) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.unwrap();
        let info = adapter.get_info();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.unwrap();
        (device, queue, format!("{} ({:?}, {})", info.name, info.backend, info.driver_info))
    });
    let mut report = CompileReport::new(BC7_RDO_COMPILES, |compile| if compile.done {
        println!("compile {}: {:.1} ms", compile.name, compile.elapsed.as_secs_f64() * 1e3);
    });
    let gpu = Bc7RdoGpu::with_device(device, queue, adapter, &mut report);
    println!("adapter: {}", gpu.adapter());

    let pages = sample_pages(dir, source, count);
    let orders: Vec<Vec<[u8; 4]>> = pages.iter().map(|p| block_order(&p.rgba, p.width, p.height)).collect();
    let plain: Vec<Vec<[u8; 16]>> = pages.par_iter().map(|page| {
        let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
        let encoded = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide)
            .unwrap();
        encoded.blocks().chunks_exact(BLOCK_BYTES).map(|b| b.try_into().unwrap()).collect()
    }).collect();
    let texels: usize = pages.iter().map(|p| p.width * p.height).sum();
    println!("{source}: {} pages, {:.1} Mpx", pages.len(), texels as f64 / 1e6);
    let plain_zstd = plain.par_iter().map(|b| zstd::bulk::compress(&b.concat(), ZSTD_LEVEL).unwrap().len()).sum();
    print("BC7", &pages, &plain, plain_zstd, &plain, 0.0);

    for &lambda in &lambdas {
        for &lookback in &lookbacks {
            println!("λ {lambda}, look-back {lookback}");
            let params = Bc7RdoParams { lambda, lookback_window_size: lookback * BLOCK_BYTES,
                skip_zero_mse_blocks: true, ..Default::default() };
            let mut cpu = plain.clone();
            let started = Instant::now();
            pages.par_iter().zip(cpu.par_iter_mut()).zip(orders.par_iter()).for_each(|((page, blocks), order)| {
                reduce_entropy_bc7_parallel(blocks, order, page.width.div_ceil(4), page.height.div_ceil(4), &params);
            });
            print("CPU RDO", &pages, &cpu, plain_zstd, &plain, started.elapsed().as_secs_f64());
            let (first, first_wall) = gpu_run(&gpu, &plain, &orders, &pages, &params);
            print("GPU RDO, one batch", &pages, &first, plain_zstd, &plain, first_wall);
            let (second, second_wall) = gpu_run(&gpu, &plain, &orders, &pages, &params);
            let (concurrent, concurrent_wall) = gpu_concurrent_run(&gpu, &plain, &orders, &pages, &params);
            println!("  GPU second run {second_wall:.2} s, concurrent callers {concurrent_wall:.2} s; same blocks: \
                {} and {}", second == first, concurrent == first);
            if args.len() >= 11 {
                let crop_dir = &args[5];
                let page_index: usize = args[6].parse().unwrap();
                let rect: Vec<usize> = args[7..11].iter().map(|v| v.parse().unwrap()).collect();
                let rect = (rect[0], rect[1], rect[2], rect[3]);
                let page = &pages[page_index];
                let tag = format!("l{lambda}_lb{lookback}");
                write_crop(&format!("{crop_dir}/source.rgba"), page, &page.rgba, rect);
                write_crop(&format!("{crop_dir}/plain.rgba"), page, &decode(page, &plain[page_index]), rect);
                write_crop(&format!("{crop_dir}/cpu_{tag}.rgba"), page, &decode(page, &cpu[page_index]), rect);
                write_crop(&format!("{crop_dir}/gpu_{tag}.rgba"), page, &decode(page, &first[page_index]), rect);
            }
        }
    }
}
