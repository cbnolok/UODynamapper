//! Whether the BC7 RDO pays on the GPU: a prototype of its fixed-offset search in a compute shader
//! (rdo_gpu_proto.wgsl), timed against the CPU RDO on the same mobile animation pages, with their sizes and PSNR.
//! The shader leaves out what keeps the CPU's choices exact (see its header), so its blocks differ; it does the same
//! decode trials, which are the cost. Both split a page into the same row chunks and run every chunk at once.
//! Usage: rdo_gpu_proto <uddp dir> <cc|ec> <pages> <lambda> <look-back blocks>
use rayon::prelude::*;
use std::time::Instant;
use udd_assets::{MobileAnimCcPackage, MobileAnimEcPackage};
use uot_texture_codecs::bc7::rdo::{reduce_entropy_bc7_parallel, Bc7RdoParams};
use uot_texture_codecs::bc7::{decode_bc7_to_rgba8888, encode_to_bc7, Bc7EncoderBackend, ImageExtent, RawImageFormat};
use wgpu::util::DeviceExt;

const SHADER: &str = include_str!("rdo_gpu_proto.wgsl");
const ZSTD_LEVEL: i32 = 9;
const BLOCK_BYTES: usize = 16;
const BLOCK_TEXELS: usize = 16;
const PEAK_SQUARED: f64 = 255.0 * 255.0;
/// The CPU RDO's chunking (`parallel_rdo_chunk_blocks`): at least this many blocks, whole rows, and at least the
/// look-back; otherwise a page's blocks over twice the threads.
const MIN_CHUNK_BLOCKS: usize = 2048;
/// The threads the CPU chunking divides by.
const CPU_THREADS: usize = 20;
/// Blocks of each chunk one submission walks, so that no dispatch holds the desktop's GPU for long.
const BLOCKS_PER_SUBMIT: u32 = 256;
/// The RDO parameters the shader mirrors (`Bc7RdoParams` defaults and their automatic smooth block scale).
const MAX_SMOOTH_BLOCK_STD_DEV: f32 = 18.0;
const MAX_RMS_INCREASE_RATIO: f32 = 10.0;
const SMOOTH_SCALE_LOW: f32 = 15.0;
const SMOOTH_SCALE_HIGH: f32 = 50.0;
const SMOOTH_SCALE_LAMBDA_SPAN: f32 = 4.0;
/// The distance price: 5 bits plus the extra bits of the byte distance (`compute_dist_cost_estimate` up to 1023).
const DISTANCE_BASE_BITS: f32 = 5.0;

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

/// RGB PSNR over the texels with alpha > 0.
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

fn smooth_error_scale(texels: &[[u8; 4]], lambda: f32) -> f32 {
    let max_mse_scale = SMOOTH_SCALE_LOW + (SMOOTH_SCALE_HIGH - SMOOTH_SCALE_LOW) * (lambda / SMOOTH_SCALE_LAMBDA_SPAN).min(1.0);
    let mut max_std_dev = 0.0f32;
    for c in 0..4 {
        let (mut sum, mut sum2) = (0u32, 0u32);
        for t in texels { sum += t[c] as u32; sum2 += t[c] as u32 * t[c] as u32; }
        let variance = (BLOCK_TEXELS as u32 * sum2).saturating_sub(sum * sum);
        max_std_dev = max_std_dev.max((variance as f64).sqrt() as f32 / BLOCK_TEXELS as f32);
    }
    let y = (max_std_dev / MAX_SMOOTH_BLOCK_STD_DEV).clamp(0.0, 1.0);
    max_mse_scale + (1.0 - max_mse_scale) * y * y
}

fn chunk_blocks(num_blocks: usize, blocks_x: usize, lookback: usize) -> usize {
    let chunk = MIN_CHUNK_BLOCKS.max(lookback).max((num_blocks / (CPU_THREADS * 2)).max(1));
    chunk.div_ceil(blocks_x).max(lookback.div_ceil(blocks_x)).max(1) * blocks_x
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params { lambda: f32, lookback_blocks: u32, thresh_ratio_squared: f32, chunk_count: u32, block_begin: u32, block_count: u32, pad: [u32; 2] }

struct Result { blocks: Vec<Vec<[u8; 16]>>, wall: f64 }

fn print(label: &str, pages: &[Page], blocks: &[Vec<[u8; 16]>], plain_zstd: usize, plain: &[Vec<[u8; 16]>], wall: f64) -> usize {
    let (mut zstd, mut sse, mut count, mut modified, mut total) = (0usize, 0.0, 0.0, 0usize, 0usize);
    for ((page, b), p) in pages.iter().zip(blocks).zip(plain) {
        let flat: Vec<u8> = b.concat();
        zstd += zstd::bulk::compress(&flat, ZSTD_LEVEL).unwrap().len();
        let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
        let (s, c) = rgb_sse(&page.rgba, &decode_bc7_to_rgba8888(&flat, extent).unwrap());
        sse += s;
        count += c;
        modified += b.iter().zip(p).filter(|(x, y)| x != y).count();
        total += b.len();
    }
    println!("  {label:24} wall {wall:7.2} s  zstd {:8.2} MiB ({:+5.1} %)  RGB PSNR {:5.2} dB  modified {:5.1} %",
        zstd as f64 / 1048576.0, (zstd as f64 / plain_zstd.max(1) as f64 - 1.0) * 100.0,
        10.0 * (PEAK_SQUARED / (sse / count)).log10(), modified as f64 * 100.0 / total as f64);
    zstd
}

fn gpu_rdo(plain: &[Vec<[u8; 16]>], orders: &[Vec<[u8; 4]>], pages: &[Page], lambda: f32, lookback: usize, runs: usize) -> Result {
    let (device, queue, name) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.unwrap();
        let info = adapter.get_info();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.unwrap();
        (device, queue, format!("{} ({:?})", info.name, info.backend))
    });
    println!("  GPU {name}");

    // Every page's blocks, texels, scales and chunks, end to end.
    let (mut blocks, mut texels, mut scales, mut chunks) = (Vec::<u8>::new(), Vec::<u32>::new(), Vec::<f32>::new(), Vec::<[u32; 2]>::new());
    let mut page_starts = Vec::new();
    for ((page, b), order) in pages.iter().zip(plain).zip(orders) {
        let base = blocks.len() / BLOCK_BYTES;
        page_starts.push(base);
        blocks.extend(b.iter().flatten());
        texels.extend(order.iter().map(|t| u32::from_le_bytes(*t)));
        scales.extend(order.chunks_exact(BLOCK_TEXELS).map(|t| smooth_error_scale(t, lambda)));
        let n = b.len();
        let step = chunk_blocks(n, page.width.div_ceil(4), lookback);
        let mut start = 0;
        while start < n {
            chunks.push([(base + start) as u32, (base + (start + step).min(n)) as u32]);
            start += step;
        }
    }
    let longest_chunk = chunks.iter().map(|c| c[1] - c[0]).max().unwrap();
    let distance_bits: Vec<f32> = (0..=lookback).map(|delta| {
        let dist = (delta * BLOCK_BYTES) as f32;
        if delta == 0 { 0.0 } else { DISTANCE_BASE_BITS + (dist.log2().floor() - 1.0).max(0.0) }
    }).collect();

    let usage = wgpu::BufferUsages::STORAGE;
    let block_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("blocks"), contents: &blocks, usage: usage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST });
    let texel_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("texels"), contents: bytemuck::cast_slice(&texels), usage });
    let chunk_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("chunks"), contents: bytemuck::cast_slice(&chunks), usage });
    let scale_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("scales"), contents: bytemuck::cast_slice(&scales), usage });
    let distance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("distance"), contents: bytemuck::cast_slice(&distance_bits), usage });
    let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"), size: std::mem::size_of::<Params>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"), size: blocks.len() as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

    let descriptor = wgpu::ShaderModuleDescriptor { label: Some("rdo"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) };
    let checks = wgpu::ShaderRuntimeChecks { force_loop_bounding: false, ..wgpu::ShaderRuntimeChecks::checked() };
    // SAFETY: every loop runs to a constant, a uniform or a chunk end read once, counting up.
    let module = unsafe { device.create_shader_module_trusted(descriptor, checks) };
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("rdo"), layout: None, module: &module, entry_point: Some("rdo"),
        compilation_options: Default::default(), cache: None });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rdo"), layout: &pipeline.get_bind_group_layout(0),
        entries: &[&block_buffer, &texel_buffer, &chunk_buffer, &scale_buffer, &distance_buffer, &params_buffer]
            .iter().enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
            .collect::<Vec<_>>() });

    let mut wall = f64::MAX;
    for _ in 0..runs {
        queue.write_buffer(&block_buffer, 0, &blocks);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let t = Instant::now();
        let mut begin = 0;
        while begin < longest_chunk {
            let params = Params {
                lambda, lookback_blocks: lookback as u32, thresh_ratio_squared: MAX_RMS_INCREASE_RATIO * MAX_RMS_INCREASE_RATIO,
                chunk_count: chunks.len() as u32, block_begin: begin, block_count: BLOCKS_PER_SUBMIT, pad: [0; 2] };
            queue.write_buffer(&params_buffer, 0, bytemuck::bytes_of(&params));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(chunks.len() as u32, 1, 1);
            }
            queue.submit([encoder.finish()]);
            begin += BLOCKS_PER_SUBMIT;
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        wall = wall.min(t.elapsed().as_secs_f64());
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&block_buffer, 0, &readback, 0, blocks.len() as u64);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let out: Vec<[u8; 16]> = readback.slice(..).get_mapped_range().unwrap().chunks_exact(BLOCK_BYTES).map(|b| b.try_into().unwrap()).collect();
    let mut result = Vec::new();
    for (i, b) in plain.iter().enumerate() {
        result.push(out[page_starts[i]..page_starts[i] + b.len()].to_vec());
    }
    Result { blocks: result, wall }
}

fn main() {
    const GPU_RUNS: usize = 2;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, source, count) = (&args[0], &args[1], args[2].parse::<usize>().unwrap());
    let lambda: f32 = args[3].parse().unwrap();
    let lookback: usize = args[4].parse().unwrap();
    let pages = sample_pages(dir, source, count);
    let orders: Vec<Vec<[u8; 4]>> = pages.iter().map(|p| block_order(&p.rgba, p.width, p.height)).collect();
    let plain: Vec<Vec<[u8; 16]>> = pages.par_iter().map(|page| {
        let extent = ImageExtent::new(page.width as u32, page.height as u32).unwrap();
        let encoded = encode_to_bc7(&page.rgba, extent, RawImageFormat::Rgba8888, Bc7EncoderBackend::AnalyticalWide).unwrap();
        encoded.blocks().chunks_exact(BLOCK_BYTES).map(|b| b.try_into().unwrap()).collect()
    }).collect();
    let texels: usize = pages.iter().map(|p| p.width * p.height).sum();
    println!("{source}: {} pages, {:.1} Mpx, λ {lambda}, look-back {lookback}", pages.len(), texels as f64 / 1e6);
    let plain_zstd = plain.iter().map(|b| zstd::bulk::compress(&b.concat(), ZSTD_LEVEL).unwrap().len()).sum();
    print("BC7", &pages, &plain, plain_zstd, &plain, 0.0);

    let params = Bc7RdoParams { lambda, lookback_window_size: lookback * BLOCK_BYTES, skip_zero_mse_blocks: true, ..Default::default() };
    let mut cpu = plain.clone();
    let t = Instant::now();
    pages.par_iter().zip(cpu.par_iter_mut()).zip(orders.par_iter()).for_each(|((page, blocks), order)| {
        reduce_entropy_bc7_parallel(blocks, order, page.width.div_ceil(4), page.height.div_ceil(4), &params);
    });
    print("CPU RDO", &pages, &cpu, plain_zstd, &plain, t.elapsed().as_secs_f64());

    let gpu = gpu_rdo(&plain, &orders, &pages, lambda, lookback, GPU_RUNS);
    print("GPU prototype", &pages, &gpu.blocks, plain_zstd, &plain, gpu.wall);
}
