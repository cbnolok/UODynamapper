//! Headless GPU cost of sprite layouts, from the creature crowd part `qd` dumped. Measurement only; ratios between layouts.
//!
//! `gpu-cost run <dir> <cc|ec>`: draws the crowd offscreen with timestamp queries.
//! `gpu-cost compile <dir> <cc|ec>`: creates every pipeline and prints a line naming each, so `RADV_DEBUG=shaderstats`'s
//! output can be read against them.

#[path = "../gpu/real.rs"]
mod real;
#[path = "../gpu/shaders.rs"]
mod shaders;
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};
use shaders::{source, Hue, Path};
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::Path as FilePath;
use std::time::Instant;
use wgpu::util::DeviceExt;

const TARGET_WIDTH: u32 = 1920;
const TARGET_HEIGHT: u32 = 1080;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const QUAD_VERTICES: u32 = 6;
const BLOCK_SIDE: u32 = 4;
const BC1_BLOCK_BYTES: u32 = 8;
const BC7_BLOCK_BYTES: u32 = 16;
const BC3_VIEW: usize = 7;
const ALTERNATE: usize = 2;
/// Draws per dressed character: one human and five equipment bodies, as in the dressed crowd.
const CHARACTER_LAYERS: usize = 6;
const CHANCE_STEPS: u32 = 1_000_000;
const RGBA_BYTES: u32 = 4;
const PALETTE_COLOURS: u32 = 256;
const PALETTE_ENTRY_BYTES: usize = 4;
const GUTTER: u32 = 4;
const ATLAS_WIDTH: u32 = 8192;
const HUE_ROWS: u32 = 1024;
const FRAME_SETS: u32 = 8;
const SET_STEP: u32 = 3;
const ROUNDS: usize = 7;
const PASSES_PER_SUBMIT: u32 = 16;
const WARM_SUBMITS: usize = 1;
const INTEGER_SCALE: f32 = 1.0;
const FRACTIONAL_SCALE: f32 = 1.5;
const BUDGET_60_FPS_MS: f64 = 1000.0 / 60.0;
const NS_PER_MS: f64 = 1_000_000.0;
const COVERAGE_LEVELS: u32 = 16;
const OPAQUE: u8 = 255;
const SEED: u32 = 0x9E37_79B9;
/// The mixes, as the share of sprites on the first format, in percent. 0 % shows what the branch costs the other path.
const MIXES: [u32; 4] = [100, 90, 50, 0];
const PERCENT: u32 = 100;
const UPLOAD_ROUNDS: usize = 9;
const MS_PER_SECOND: f64 = 1000.0;
const BYTES_PER_MB: f64 = 1_000_000.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    atlas_min: [f32; 2],
    frame_size: [f32; 2],
    palette: u32,
    hued_palette: u32,
    hue: u32,
    path: u32,
    rows: u32,
    pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    atlas: [f32; 2],
}

struct Frame {
    w: u32,
    h: u32,
    index: Vec<u8>,
    coverage: Vec<u8>,
    bc1: Vec<u8>,
    bc7: Vec<u8>,
    x: u32,
    y: u32,
    rows: u32,
}

struct Stream {
    palette: Vec<u8>,
    frames: Vec<Frame>,
}

fn read_u32(reader: &mut impl Read) -> u32 {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).expect("the GPU dump");
    u32::from_le_bytes(bytes)
}

fn read_bytes(reader: &mut impl Read, length: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes).expect("the GPU dump");
    bytes
}

fn blocks(w: u32, h: u32) -> usize {
    (w.div_ceil(BLOCK_SIDE) * h.div_ceil(BLOCK_SIDE)) as usize
}

fn read_streams(dir: &FilePath, tag: &str) -> Vec<Stream> {
    let enhanced = tag == "ec";
    let mut reader = BufReader::new(std::fs::File::open(dir.join(format!("{tag}.gpu"))).expect("the GPU dump"));
    let pairs = read_u32(&mut reader);
    (0..pairs)
        .map(|_| {
            let count = read_u32(&mut reader);
            let palette = read_bytes(&mut reader, PALETTE_COLOURS as usize * PALETTE_ENTRY_BYTES);
            let mut frames = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (w, h) = (read_u32(&mut reader), read_u32(&mut reader));
                let area = (w * h) as usize;
                let index = read_bytes(&mut reader, area);
                let coverage = if enhanced { read_bytes(&mut reader, area) } else { Vec::new() };
                let bc1 = read_bytes(&mut reader, blocks(w, h) * BC1_BLOCK_BYTES as usize);
                let bc7 = read_bytes(&mut reader, blocks(w, h) * BC7_BLOCK_BYTES as usize);
                if area > 0 {
                    frames.push(Frame { w, h, index, coverage, bc1, bc7, x: 0, y: 0, rows: 0 });
                }
            }
            Stream { palette, frames }
        })
        .collect()
}

/// Shelves over one atlas, frames at 4 x 4 block boundaries with a gutter, the same places for every format.
fn pack(streams: &mut [Stream]) -> (u32, u32) {
    let mut order: Vec<(usize, usize)> = streams.iter().enumerate().flat_map(|(s, stream)| (0..stream.frames.len()).map(move |f| (s, f))).collect();
    order.sort_by_key(|&(s, f)| std::cmp::Reverse(streams[s].frames[f].h));
    let (mut x, mut y, mut shelf) = (GUTTER, GUTTER, 0u32);
    for (s, f) in order {
        let frame = &mut streams[s].frames[f];
        let (w, h) = (frame.w.next_multiple_of(BLOCK_SIDE), frame.h.next_multiple_of(BLOCK_SIDE));
        if x + w + GUTTER > ATLAS_WIDTH {
            x = GUTTER;
            y += shelf + GUTTER;
            shelf = 0;
        }
        (frame.x, frame.y) = (x, y);
        x += w + GUTTER;
        shelf = shelf.max(h);
    }
    (ATLAS_WIDTH, (y + shelf + GUTTER).next_multiple_of(BLOCK_SIDE))
}

struct Atlases {
    size: (u32, u32),
    index: Vec<u8>,
    bc1: Vec<u8>,
    bc3: Vec<u8>,
    bc7: Vec<u8>,
    rgba: Vec<u8>,
    mask: Vec<u8>,
    rows: Vec<u32>,
    pixels: Vec<u8>,
}

fn alpha_of(enhanced: bool, frame: &Frame, n: usize) -> u8 {
    if enhanced {
        (u32::from(frame.coverage[n]) * u32::from(OPAQUE) / COVERAGE_LEVELS) as u8
    } else if frame.index[n] != 0 {
        OPAQUE
    } else {
        0
    }
}

fn build_atlases(streams: &mut [Stream], enhanced: bool, size: (u32, u32)) -> Atlases {
    let (width, height) = (size.0 as usize, size.1 as usize);
    let index_bytes = if enhanced { 2 } else { 1 };
    let mut atlases = Atlases {
        size,
        index: vec![0u8; width * height * index_bytes],
        bc1: vec![0u8; width.div_ceil(BLOCK_SIDE as usize) * height.div_ceil(BLOCK_SIDE as usize) * BC1_BLOCK_BYTES as usize],
        bc7: vec![0u8; width.div_ceil(BLOCK_SIDE as usize) * height.div_ceil(BLOCK_SIDE as usize) * BC7_BLOCK_BYTES as usize],
        bc3: Vec::new(),
        rgba: vec![0u8; width * height * RGBA_BYTES as usize],
        mask: vec![0u8; width * height],
        rows: Vec::new(),
        pixels: Vec::new(),
    };
    for stream in streams.iter_mut() {
        let palette = &stream.palette;
        for frame in &mut stream.frames {
            let (w, h, x0, y0) = (frame.w as usize, frame.h as usize, frame.x as usize, frame.y as usize);
            frame.rows = atlases.rows.len() as u32;
            for y in 0..h {
                let (mut first, mut last) = (usize::MAX, 0usize);
                for x in 0..w {
                    let n = y * w + x;
                    let at = (y0 + y) * width + x0 + x;
                    let i = frame.index[n] as usize;
                    atlases.index[at * index_bytes] = frame.index[n];
                    let alpha = alpha_of(enhanced, frame, n);
                    if enhanced {
                        atlases.index[at * index_bytes + 1] = frame.coverage[n];
                    }
                    if alpha != 0 {
                        first = first.min(x);
                        last = x;
                        let colour = &palette[i * PALETTE_ENTRY_BYTES..(i + 1) * PALETTE_ENTRY_BYTES];
                        atlases.rgba[at * RGBA_BYTES as usize..(at + 1) * RGBA_BYTES as usize].copy_from_slice(&[colour[0], colour[1], colour[2], alpha]);
                        // The keep-colour byte: a hued frame's mask is one where the palette lets the hue through.
                        atlases.mask[at] = if colour[3] == 0 { OPAQUE } else { 0 };
                    }
                }
                let count = if first == usize::MAX { 0 } else { last - first + 1 };
                let start = atlases.pixels.len();
                if count > 0 {
                    for x in first..=last {
                        atlases.pixels.push(frame.index[y * w + x]);
                        if enhanced {
                            atlases.pixels.push(frame.coverage[y * w + x]);
                        }
                    }
                }
                atlases.rows.push(first.min(u16::MAX as usize) as u32 | ((count as u32) << 16));
                atlases.rows.push(start as u32);
            }
            let columns = frame.w.div_ceil(BLOCK_SIDE) as usize;
            let atlas_columns = width.div_ceil(BLOCK_SIDE as usize);
            let (bx, by) = (x0 / BLOCK_SIDE as usize, y0 / BLOCK_SIDE as usize);
            for row in 0..frame.h.div_ceil(BLOCK_SIDE) as usize {
                for (source, target, bytes) in [(&frame.bc1, &mut atlases.bc1, BC1_BLOCK_BYTES as usize), (&frame.bc7, &mut atlases.bc7, BC7_BLOCK_BYTES as usize)] {
                    let from = row * columns * bytes;
                    let to = ((by + row) * atlas_columns + bx) * bytes;
                    target[to..to + columns * bytes].copy_from_slice(&source[from..from + columns * bytes]);
                }
            }
        }
    }
    // BC3 by the block rows' encoder: frames sit on block corners, so a whole atlas codes the same blocks.
    atlases.bc3 = encode_bc3(&atlases.rgba, size);
    atlases
}

fn encode_bc3(rgba: &[u8], size: (u32, u32)) -> Vec<u8> {
    let mut options = dds::EncodeOptions::default();
    options.quality = dds::CompressionQuality::High;
    let view = dds::ImageView::new(rgba, dds::Size::new(size.0, size.1), dds::ColorFormat::RGBA_U8).expect("an atlas");
    let mut data = Vec::new();
    dds::encode(&mut data, view, dds::Format::BC3_UNORM, None, &options).expect("dds encodes the atlas");
    data
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    target: wgpu::TextureView,
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    period: f64,
}

fn open_gpu() -> Gpu {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).expect("a Vulkan adapter");
    let info = adapter.get_info();
    println!("adapter: {}, driver {} {}", info.name, info.driver, info.driver_info);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TEXTURE_COMPRESSION_BC,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("a device");
    let period = f64::from(queue.get_timestamp_period());
    let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    };
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            storage(1),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            texture(3),
            texture(4),
            texture(5),
            texture(6),
            texture(7),
            texture(8),
            texture(9),
            storage(10),
            storage(11),
            texture(12),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let target = device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: TARGET_WIDTH, height: TARGET_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: PASSES_PER_SUBMIT * 2 });
    let bytes = u64::from(PASSES_PER_SUBMIT) * 2 * wgpu::QUERY_SIZE as u64;
    let resolve = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    Gpu { device, queue, layout, pipeline_layout, target, queries, resolve, readback, period }
}

fn upload(gpu: &Gpu, size: (u32, u32), format: wgpu::TextureFormat, bytes: &[u8]) -> wgpu::TextureView {
    gpu.device
        .create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytes,
        )
        .create_view(&Default::default())
}

/// Waits until every queued write has reached the GPU.
fn finish_uploads(gpu: &Gpu) {
    gpu.queue.submit([]);
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device");
}

/// The other half of "drawable": the crowd's faced frames going to the GPU, in one write and frame by frame.
fn upload_rows(gpu: &Gpu, atlases: &Atlases, streams: &[Stream], index_format: wgpu::TextureFormat, index_bytes: u32, pixels: &[u8]) {
    let frames = streams.iter().flat_map(|s| s.frames.iter()).count();
    println!("\n#### Uploads of the crowd's faced frames, {frames} frames, median of {UPLOAD_ROUNDS} rounds; CPU time until the queue is idle\n");
    println!("| layout | MB in one write | ms | MB/s | MB frame by frame | ms | MB/s |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    let sets: [(&str, wgpu::TextureFormat, &[u8], u32, u32); 5] = [
        ("index", index_format, &atlases.index, 1, index_bytes),
        ("BC1", wgpu::TextureFormat::Bc1RgbaUnorm, &atlases.bc1, BLOCK_SIDE, BC1_BLOCK_BYTES),
        ("BC3", wgpu::TextureFormat::Bc3RgbaUnorm, &atlases.bc3, BLOCK_SIDE, BC7_BLOCK_BYTES),
        ("BC7", wgpu::TextureFormat::Bc7RgbaUnorm, &atlases.bc7, BLOCK_SIDE, BC7_BLOCK_BYTES),
        ("RGBA8", wgpu::TextureFormat::Rgba8Unorm, &atlases.rgba, 1, RGBA_BYTES),
    ];
    for (name, format, bytes, side, block_bytes) in sets {
        let extent = wgpu::Extent3d { width: atlases.size.0, height: atlases.size.1, depth_or_array_layers: 1 };
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let columns = atlases.size.0.div_ceil(side);
        let row_bytes = columns * block_bytes;
        let (mut whole, mut each) = (Vec::new(), Vec::new());
        let mut frame_bytes = 0u64;
        for _ in 0..UPLOAD_ROUNDS {
            let start = Instant::now();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row_bytes), rows_per_image: None },
                extent,
            );
            finish_uploads(gpu);
            whole.push(start.elapsed().as_secs_f64() * MS_PER_SECOND);
            frame_bytes = 0;
            let start = Instant::now();
            for frame in streams.iter().flat_map(|s| s.frames.iter()) {
                let (w, h) = (frame.w.next_multiple_of(side), frame.h.next_multiple_of(side));
                let offset = u64::from(frame.y / side) * u64::from(row_bytes) + u64::from(frame.x / side) * u64::from(block_bytes);
                frame_bytes += u64::from(w / side) * u64::from(h / side) * u64::from(block_bytes);
                gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d { x: frame.x, y: frame.y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    bytes,
                    wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(row_bytes), rows_per_image: None },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
            finish_uploads(gpu);
            each.push(start.elapsed().as_secs_f64() * MS_PER_SECOND);
        }
        let (whole_ms, each_ms) = (median(&mut whole), median(&mut each));
        let (whole_mb, each_mb) = (bytes.len() as f64 / BYTES_PER_MB, frame_bytes as f64 / BYTES_PER_MB);
        println!("| {name} | {whole_mb:.1} | {whole_ms:.2} | {:.0} | {each_mb:.1} | {each_ms:.2} | {:.0} |", whole_mb / whole_ms * MS_PER_SECOND, each_mb / each_ms * MS_PER_SECOND);
    }
    let rows: &[u8] = bytemuck::cast_slice(&atlases.rows);
    let make = |bytes: &[u8]| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes.len() as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let (rows_buffer, pixels_buffer) = (make(rows), make(pixels));
    let mut times = Vec::new();
    for _ in 0..UPLOAD_ROUNDS {
        let start = Instant::now();
        gpu.queue.write_buffer(&rows_buffer, 0, rows);
        gpu.queue.write_buffer(&pixels_buffer, 0, pixels);
        finish_uploads(gpu);
        times.push(start.elapsed().as_secs_f64() * MS_PER_SECOND);
    }
    let (mb, ms) = ((rows.len() + pixels.len()) as f64 / BYTES_PER_MB, median(&mut times));
    println!("| row spans: table and pixels, storage buffers | {mb:.1} | {ms:.2} | {:.0} | | | |", mb / ms * MS_PER_SECOND);
}

struct Bound {
    globals: wgpu::Buffer,
    sampler: wgpu::Sampler,
    views: [wgpu::TextureView; 8],
    rows: wgpu::Buffer,
    pixels: wgpu::Buffer,
}

fn bind(gpu: &Gpu, bound: &Bound, instances: &wgpu::Buffer) -> wgpu::BindGroup {
    bind_slots(gpu, bound, instances, 1)
}

/// `bc1_slot` names which view sits on BC1's binding: a second array on the same binding is what a bind group switch changes.
fn bind_slots(gpu: &Gpu, bound: &Bound, instances: &wgpu::Buffer, bc1_slot: usize) -> wgpu::BindGroup {
    let view = |binding: u32, n: usize| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(&bound.views[n]) };
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &gpu.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: bound.globals.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: instances.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&bound.sampler) },
            view(3, 0),
            view(4, bc1_slot),
            view(5, 2),
            view(6, 3),
            view(7, 4),
            view(8, 5),
            view(9, 6),
            wgpu::BindGroupEntry { binding: 10, resource: bound.rows.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 11, resource: bound.pixels.as_entire_binding() },
            view(12, BC3_VIEW),
        ],
    })
}

fn pipeline(gpu: &Gpu, label: &str, paths: &[Path], hue: Hue, enhanced: bool) -> wgpu::RenderPipeline {
    eprintln!("=== PIPELINE {label}");
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source(paths, hue, enhanced).into()) });
    let made = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&gpu.pipeline_layout),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: TARGET_FORMAT, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device");
    made
}

/// A little congruential generator: the crowd's places must be the same in every run, and no crate is needed for it.
fn next_random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) ^ SEED;
    *state >> 8
}

/// FRAME_SETS sets of a sprite per stream, in painter's order, on the first path or the second by `first_percent`.
fn make_instances(streams: &[Stream], scale: f32, first_percent: u32, hued: bool) -> Vec<Instance> {
    let mut random = SEED;
    let places: Vec<(f32, f32, u32)> = streams.iter().map(|_| (next_random(&mut random) as f32, next_random(&mut random) as f32, next_random(&mut random))).collect();
    let mut out = Vec::new();
    for set in 0..FRAME_SETS {
        for (n, stream) in streams.iter().enumerate() {
            if stream.frames.is_empty() {
                continue;
            }
            let frame = &stream.frames[(n as u32 + set * SET_STEP) as usize % stream.frames.len()];
            let (x, y, pick) = places[n];
            let size = [frame.w as f32 * scale, frame.h as f32 * scale];
            // Whole pixels on the target: at integer scale a fragment's centre lands on a texel's.
            let pos = [(x % (TARGET_WIDTH as f32 - size[0]).max(1.0)).floor(), (y % (TARGET_HEIGHT as f32 - size[1]).max(1.0)).floor()];
            out.push(Instance {
                pos,
                size,
                atlas_min: [frame.x as f32, frame.y as f32],
                frame_size: [frame.w as f32, frame.h as f32],
                palette: n as u32,
                hued_palette: streams.len() as u32 + n as u32,
                hue: if hued { (n as u32 * SET_STEP + 1) % HUE_ROWS } else { 0 },
                path: u32::from(pick % PERCENT >= first_percent),
                rows: frame.rows,
                pad: [0; 3],
            });
        }
    }
    out
}

/// What the character crowd is: draws per set in painter's order, each run of one format, and how many mobiles of each kind.
struct MobileCrowd {
    instances: Vec<Instance>,
    runs: Vec<(u32, u32, u32)>,
    characters: u32,
    creatures: u32,
}

/// Mobiles in painter's order: a character is CHARACTER_LAYERS draws on the first path at one place, all one format; a creature is one
/// draw on the second. `first_percent` is the share of draws on characters, as in MIXES. Layers are the dump's creature frames: it holds no equipment.
fn make_mobiles(streams: &[Stream], scale: f32, first_percent: u32) -> MobileCrowd {
    let live: Vec<usize> = (0..streams.len()).filter(|&n| !streams[n].frames.is_empty()).collect();
    let share = f64::from(first_percent) / f64::from(PERCENT);
    let chance = share / (share + CHARACTER_LAYERS as f64 * (1.0 - share));
    let mut random = SEED;
    let mut assigned: Vec<(u32, f32, f32)> = Vec::with_capacity(live.len());
    let (mut characters, mut creatures) = (0u32, 0u32);
    while assigned.len() < live.len() {
        let (x, y, pick) = (next_random(&mut random) as f32, next_random(&mut random) as f32, next_random(&mut random));
        let character = f64::from(pick % CHANCE_STEPS) < chance * f64::from(CHANCE_STEPS);
        if character {
            characters += 1;
        } else {
            creatures += 1;
        }
        let layers = if character { CHARACTER_LAYERS } else { 1 };
        for _ in 0..layers.min(live.len() - assigned.len()) {
            assigned.push((u32::from(!character), x, y));
        }
    }
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    for (n, a) in assigned.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.0 == a.0 => run.2 += 1,
            _ => runs.push((a.0, n as u32, 1)),
        }
    }
    let mut instances = Vec::with_capacity(live.len() * FRAME_SETS as usize);
    for set in 0..FRAME_SETS {
        for (&n, &(path, x, y)) in live.iter().zip(&assigned) {
            let frame = &streams[n].frames[(n as u32 + set * SET_STEP) as usize % streams[n].frames.len()];
            let size = [frame.w as f32 * scale, frame.h as f32 * scale];
            let pos = [(x % (TARGET_WIDTH as f32 - size[0]).max(1.0)).floor(), (y % (TARGET_HEIGHT as f32 - size[1]).max(1.0)).floor()];
            instances.push(Instance { pos, size, atlas_min: [frame.x as f32, frame.y as f32], frame_size: [frame.w as f32, frame.h as f32], palette: n as u32, hued_palette: streams.len() as u32 + n as u32, hue: 0, path, rows: frame.rows, pad: [0; 3] });
        }
    }
    MobileCrowd { instances, runs, characters, creatures }
}

/// The same sets, the first path's sprites before the second's in each: what grouping by pipeline does to painter's order.
fn grouped(instances: &[Instance], per_set: usize) -> (Vec<Instance>, Vec<usize>) {
    let mut out = Vec::with_capacity(instances.len());
    let mut splits = Vec::new();
    for set in instances.chunks_exact(per_set) {
        out.extend(set.iter().filter(|i| i.path == 0));
        splits.push(set.iter().filter(|i| i.path == 0).count());
        out.extend(set.iter().filter(|i| i.path != 0));
    }
    (out, splits)
}

enum Draws<'a> {
    Empty,
    One { pipeline: &'a wgpu::RenderPipeline, group: &'a wgpu::BindGroup },
    Grouped { first: &'a wgpu::RenderPipeline, second: &'a wgpu::RenderPipeline, group: &'a wgpu::BindGroup, splits: &'a [usize] },
    /// A draw per sprite on one pipeline and one bind group: draw calls alone.
    Separate { pipeline: &'a wgpu::RenderPipeline, group: &'a wgpu::BindGroup },
    /// A draw per sprite, the bind group following the sprite's format.
    Switch { pipeline: &'a wgpu::RenderPipeline, groups: [&'a wgpu::BindGroup; ALTERNATE], instances: &'a [Instance] },
    /// One draw per run of a format, in painter's order, the pipeline following the format.
    Runs { first: &'a wgpu::RenderPipeline, second: &'a wgpu::RenderPipeline, group: &'a wgpu::BindGroup, runs: &'a [(u32, u32, u32)] },
    Each { first: &'a wgpu::RenderPipeline, second: &'a wgpu::RenderPipeline, group: &'a wgpu::BindGroup, instances: &'a [Instance] },
}

struct Timed {
    gpu_ms: f64,
    cpu_ms: f64,
}

fn run_submit(gpu: &Gpu, draws: &Draws, per_set: u32) -> Timed {
    let begun = Instant::now();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for pass_n in 0..PASSES_PER_SUBMIT {
        let set = pass_n % FRAME_SETS;
        let base = set * per_set;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &gpu.target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &gpu.queries, beginning_of_pass_write_index: Some(pass_n * 2), end_of_pass_write_index: Some(pass_n * 2 + 1) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        match draws {
            Draws::Empty => {}
            Draws::One { pipeline, group } => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, Some(*group), &[]);
                pass.draw(0..QUAD_VERTICES, base..base + per_set);
            }
            Draws::Grouped { first, second, group, splits } => {
                pass.set_bind_group(0, Some(*group), &[]);
                let split = splits[set as usize] as u32;
                if split > 0 {
                    pass.set_pipeline(first);
                    pass.draw(0..QUAD_VERTICES, base..base + split);
                }
                if split < per_set {
                    pass.set_pipeline(second);
                    pass.draw(0..QUAD_VERTICES, base + split..base + per_set);
                }
            }
            Draws::Separate { pipeline, group } => {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, Some(*group), &[]);
                for n in base..base + per_set {
                    pass.draw(0..QUAD_VERTICES, n..n + 1);
                }
            }
            Draws::Switch { pipeline, groups, instances } => {
                pass.set_pipeline(pipeline);
                for n in base..base + per_set {
                    pass.set_bind_group(0, Some(groups[instances[n as usize].path as usize]), &[]);
                    pass.draw(0..QUAD_VERTICES, n..n + 1);
                }
            }
            Draws::Runs { first, second, group, runs } => {
                pass.set_bind_group(0, Some(*group), &[]);
                for &(path, start, count) in runs.iter() {
                    pass.set_pipeline(if path == 0 { first } else { second });
                    pass.draw(0..QUAD_VERTICES, base + start..base + start + count);
                }
            }
            Draws::Each { first, second, group, instances } => {
                pass.set_bind_group(0, Some(*group), &[]);
                let mut current = u32::MAX;
                for n in base..base + per_set {
                    let path = instances[n as usize].path;
                    if path != current {
                        pass.set_pipeline(if path == 0 { first } else { second });
                        current = path;
                    }
                    pass.draw(0..QUAD_VERTICES, n..n + 1);
                }
            }
        }
    }
    encoder.resolve_query_set(&gpu.queries, 0..PASSES_PER_SUBMIT * 2, &gpu.resolve, 0);
    encoder.copy_buffer_to_buffer(&gpu.resolve, 0, &gpu.readback, 0, gpu.resolve.size());
    let commands = encoder.finish();
    let cpu_ms = begun.elapsed().as_secs_f64() * 1000.0 / f64::from(PASSES_PER_SUBMIT);
    gpu.queue.submit([commands]);
    let slice = gpu.readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("the query results"));
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device");
    let stamps: Vec<u64> = bytemuck::pod_collect_to_vec(&slice.get_mapped_range().expect("the mapping"));
    gpu.readback.unmap();
    let mut passes: Vec<f64> = stamps.chunks_exact(2).map(|pair| pair[1].saturating_sub(pair[0]) as f64 * gpu.period / NS_PER_MS).collect();
    passes.sort_by(f64::total_cmp);
    Timed { gpu_ms: passes[passes.len() / 2], cpu_ms }
}

struct Row<'a> {
    section: String,
    name: String,
    fragments: f64,
    draws: Draws<'a>,
    per_set: u32,
    rounds: Vec<(f64, f64)>,
}

fn median(values: &mut Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn gpu_busy() -> String {
    let mut out = Vec::new();
    for card in ["card0", "card1", "card2"] {
        if let Ok(text) = std::fs::read_to_string(format!("/sys/class/drm/{card}/device/gpu_busy_percent")) {
            out.push(format!("{card} {} %", text.trim()));
        }
    }
    out.join(", ")
}

fn fragments_of(instances: &[Instance], per_set: usize) -> f64 {
    instances.iter().map(|i| f64::from(i.size[0] * i.size[1])).sum::<f64>() / (instances.len() / per_set) as f64 / 1_000_000.0
}

fn path_sets(enhanced: bool) -> Vec<(Path, Hue, f32)> {
    let _ = enhanced;
    let mut out = Vec::new();
    for scale in [INTEGER_SCALE, FRACTIONAL_SCALE] {
        for path in [Path::IndexGather, Path::IndexGatherSkip, Path::SpansFilter, Path::Bc1, Path::Bc3, Path::Bc7, Path::Rgba] {
            out.push((path, Hue::None, scale));
        }
    }
    out.push((Path::IndexLoad, Hue::None, INTEGER_SCALE));
    out.push((Path::SpansLoad, Hue::None, INTEGER_SCALE));
    // Hue, at the scale that shows each path's cost.
    out.push((Path::IndexLoad, Hue::PerPixel, INTEGER_SCALE));
    out.push((Path::IndexLoad, Hue::CachedRow, INTEGER_SCALE));
    out.push((Path::SpansLoad, Hue::PerPixel, INTEGER_SCALE));
    out.push((Path::SpansLoad, Hue::CachedRow, INTEGER_SCALE));
    for scale in [INTEGER_SCALE, FRACTIONAL_SCALE] {
        for path in [Path::IndexGather, Path::SpansFilter] {
            out.push((path, Hue::PerPixel, scale));
            out.push((path, Hue::CachedRow, scale));
        }
        for path in [Path::Bc1, Path::Bc3, Path::Bc7, Path::Rgba] {
            out.push((path, Hue::PerPixel, scale));
            out.push((path, Hue::Masked, scale));
        }
    }
    out
}

/// Which paths the branch and pipeline rows pair up, with the scale each is drawn at.
fn mix_pairs(enhanced: bool) -> Vec<(Path, Path, f32)> {
    if enhanced {
        vec![(Path::IndexLoad, Path::Bc7, INTEGER_SCALE), (Path::IndexGather, Path::Bc7, FRACTIONAL_SCALE), (Path::IndexGather, Path::Bc1, FRACTIONAL_SCALE), (Path::SpansFilter, Path::Bc7, FRACTIONAL_SCALE)]
    } else {
        vec![(Path::IndexLoad, Path::Bc1, INTEGER_SCALE), (Path::IndexGather, Path::Bc1, FRACTIONAL_SCALE), (Path::SpansFilter, Path::Bc1, FRACTIONAL_SCALE)]
    }
}

#[derive(Parser)]
#[command(about = "Headless sprite layout GPU experiments; timings describe this adapter only")]
struct Arguments {
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dynamapper/assets"))]
    assets: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile the actual art shader and the documented text-rewrite variants.
    Real,
    /// Time synthetic sprite layouts from an animation-codec GPU dump.
    Run { dump_dir: PathBuf, client: Client },
    /// Compile synthetic layout pipelines without timing them.
    Compile { dump_dir: PathBuf, client: Client },
    /// Measure uploads of the faced frames.
    Upload { dump_dir: PathBuf, client: Client },
}

#[derive(Clone, Copy, ValueEnum)]
enum Client { Cc, Ec }

fn main() {
    let args = Arguments::parse();
    if matches!(args.command, Command::Real) {
        let gpu = open_gpu();
        for variant in &real::VARIANTS {
            let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let made = real::pipeline(&gpu.device, TARGET_FORMAT, variant, &args.assets);
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device");
            if let Some(error) = pollster::block_on(scope.pop()) {
                panic!("the real shader, {}: {error}", variant.name);
            }
            drop(made);
        }
        return;
    }
    let (command, directory, client) = match args.command {
        Command::Run { dump_dir, client } => ("run", dump_dir, client),
        Command::Compile { dump_dir, client } => ("compile", dump_dir, client),
        Command::Upload { dump_dir, client } => ("upload", dump_dir, client),
        Command::Real => unreachable!(),
    };
    let dir = directory.as_path();
    let tag = match client { Client::Cc => "cc", Client::Ec => "ec" };
    let enhanced = tag == "ec";
    let mut streams = read_streams(dir, tag);
    let size = pack(&mut streams);
    let atlases = build_atlases(&mut streams, enhanced, size);
    let per_set = streams.iter().filter(|s| !s.frames.is_empty()).count() as u32;
    println!("{tag}: {} streams, {} frames; atlas {} x {}; row spans table {:.1} MB, pixels {:.1} MB", streams.len(), streams.iter().map(|s| s.frames.len()).sum::<usize>(), size.0, size.1, (atlases.rows.len() * 4) as f64 / 1e6, atlases.pixels.len() as f64 / 1e6);
    let gpu = open_gpu();
    let index_format = if enhanced { wgpu::TextureFormat::Rg8Unorm } else { wgpu::TextureFormat::R8Unorm };
    // Palette rows: each stream's own, then the same rows hued. The hue tables are a stand-in ramp: only their reads count.
    let mut palette = Vec::with_capacity(streams.len() * 2 * PALETTE_COLOURS as usize * PALETTE_ENTRY_BYTES);
    for stream in &streams {
        palette.extend_from_slice(&stream.palette);
    }
    for stream in &streams {
        palette.extend_from_slice(&stream.palette);
    }
    let hues: Vec<u8> = (0..HUE_ROWS * PALETTE_COLOURS)
        .flat_map(|n| {
            let key = (n % PALETTE_COLOURS) as u8;
            let row = (n / PALETTE_COLOURS) as u8;
            [key, key.wrapping_add(row), row, OPAQUE]
        })
        .collect();
    let mut pixels = atlases.pixels.clone();
    pixels.resize(pixels.len().next_multiple_of(4).max(4), 0);
    if command == "upload" {
        upload_rows(&gpu, &atlases, &streams, index_format, if enhanced { 2 } else { 1 }, &pixels);
        return;
    }
    let bound = Bound {
        globals: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::bytes_of(&Globals { viewport: [TARGET_WIDTH as f32, TARGET_HEIGHT as f32], atlas: [size.0 as f32, size.1 as f32] }), usage: wgpu::BufferUsages::UNIFORM }),
        sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
        views: [
            upload(&gpu, atlases.size, index_format, &atlases.index),
            upload(&gpu, atlases.size, wgpu::TextureFormat::Bc1RgbaUnorm, &atlases.bc1),
            upload(&gpu, atlases.size, wgpu::TextureFormat::Bc7RgbaUnorm, &atlases.bc7),
            upload(&gpu, atlases.size, wgpu::TextureFormat::Rgba8Unorm, &atlases.rgba),
            upload(&gpu, (PALETTE_COLOURS, streams.len() as u32 * 2), wgpu::TextureFormat::Rgba8Unorm, &palette),
            upload(&gpu, (PALETTE_COLOURS, HUE_ROWS), wgpu::TextureFormat::Rgba8Unorm, &hues),
            upload(&gpu, atlases.size, wgpu::TextureFormat::R8Unorm, &atlases.mask),
            upload(&gpu, atlases.size, wgpu::TextureFormat::Bc3RgbaUnorm, &atlases.bc3),
        ],
        rows: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&atlases.rows), usage: wgpu::BufferUsages::STORAGE }),
        pixels: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &pixels, usage: wgpu::BufferUsages::STORAGE }),
    };
    drop(atlases);

    // Every pipeline first; each single path and each branch once.
    let mut pipelines: HashMap<(String, u32), wgpu::RenderPipeline> = HashMap::new();
    let key = |path: Path, hue: Hue| (format!("{path:?}"), hue as u32);
    let sets = path_sets(enhanced);
    for &(path, hue, _) in &sets {
        pipelines.entry(key(path, hue)).or_insert_with(|| pipeline(&gpu, &format!("{} / {}", path.name(enhanced), hue.name()), &[path], hue, enhanced));
    }
    let pairs = mix_pairs(enhanced);
    let mut branches: HashMap<String, wgpu::RenderPipeline> = HashMap::new();
    for &(first, second, _) in &pairs {
        branches.insert(format!("{first:?}+{second:?}"), pipeline(&gpu, &format!("branch: {} + {}", first.name(enhanced), second.name(enhanced)), &[first, second], Hue::None, enhanced));
    }
    branches.insert("Bc1+Bc3".to_string(), pipeline(&gpu, "branch: BC1 + BC3", &[Path::Bc1, Path::Bc3], Hue::None, enhanced));
    let alternating_pairs: Vec<(Path, f32)> = if enhanced { Vec::new() } else { vec![(Path::IndexLoad, INTEGER_SCALE), (Path::IndexGather, FRACTIONAL_SCALE)] };
    for &(first, _) in &alternating_pairs {
        branches.entry(format!("{first:?}+Bc1")).or_insert_with(|| pipeline(&gpu, &format!("branch: {} + BC1", first.name(enhanced)), &[first, Path::Bc1], Hue::None, enhanced));
        pipelines.entry(key(first, Hue::None)).or_insert_with(|| pipeline(&gpu, first.name(enhanced), &[first], Hue::None, enhanced));
    }
    if command == "compile" {
        return;
    }

    // Instance buffers and their bind groups.
    let storage = |data: &[Instance]| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(data), usage: wgpu::BufferUsages::STORAGE });
    struct Mix {
        instances: Vec<Instance>,
        group: wgpu::BindGroup,
        grouped_group: wgpu::BindGroup,
        splits: Vec<usize>,
    }
    let mut mixes: HashMap<(u32, u32), Mix> = HashMap::new();
    for scale in [INTEGER_SCALE, FRACTIONAL_SCALE] {
        for mix in MIXES {
            let instances = make_instances(&streams, scale, mix, true);
            let (reordered, splits) = grouped(&instances, per_set as usize);
            let group = bind(&gpu, &bound, &storage(&instances));
            let grouped_group = bind(&gpu, &bound, &storage(&reordered));
            mixes.insert(((scale * 10.0) as u32, mix), Mix { instances, group, grouped_group, splits });
        }
    }
    let mix_at = |scale: f32, mix: u32| &mixes[&(((scale * 10.0) as u32), mix)];
    // The alternating crowd: every other draw on the second format, painter's order kept.
    struct Alternating {
        instances: Vec<Instance>,
        plain: wgpu::BindGroup,
        swapped: wgpu::BindGroup,
    }
    let alternating: Vec<(f32, Alternating)> = [INTEGER_SCALE, FRACTIONAL_SCALE]
        .iter()
        .map(|&scale| {
            let mut instances = make_instances(&streams, scale, PERCENT, false);
            for (n, instance) in instances.iter_mut().enumerate() {
                instance.path = (n % ALTERNATE) as u32;
            }
            let buffer = storage(&instances);
            let plain = bind_slots(&gpu, &bound, &buffer, 1);
            let swapped = bind_slots(&gpu, &bound, &buffer, BC3_VIEW);
            (scale, Alternating { instances, plain, swapped })
        })
        .collect();
    // Characters and creatures: a dressed character's layers share one format, so the format changes between mobiles only.
    let mut mobiles: Vec<(f32, String, MobileCrowd, wgpu::BindGroup)> = alternating_pairs
        .iter()
        .flat_map(|&(_, scale)| MIXES.iter().map(move |&mix| (scale, mix)))
        .map(|(scale, mix)| {
            let crowd = make_mobiles(&streams, scale, mix);
            let group = bind(&gpu, &bound, &storage(&crowd.instances));
            (scale, format!("{mix} / {} of draws, characters of {CHARACTER_LAYERS} layers", PERCENT - mix), crowd, group)
        })
        .collect();
    // The creature crowd as the lossless set splits it: each stream's own body decides.
    if let Ok(text) = std::fs::read_to_string(dir.join(format!("{tag}.split.txt"))) {
        let split: Vec<u32> = text.lines().map(|line| line.parse().expect("a path")).collect();
        assert_eq!(split.len(), streams.len());
        for &(_, scale) in &alternating_pairs {
            let mut instances = make_instances(&streams, scale, PERCENT, false);
            for instance in &mut instances {
                instance.path = split[instance.palette as usize];
            }
            let mut runs: Vec<(u32, u32, u32)> = Vec::new();
            for (n, instance) in instances[..per_set as usize].iter().enumerate() {
                match runs.last_mut() {
                    Some(run) if run.0 == instance.path => run.2 += 1,
                    _ => runs.push((instance.path, n as u32, 1)),
                }
            }
            let characters = instances[..per_set as usize].iter().filter(|i| i.path == 0).count() as u32;
            let group = bind(&gpu, &bound, &storage(&instances));
            mobiles.push((scale, "the creature crowd as the lossless set splits it".to_string(), MobileCrowd { instances, runs, characters, creatures: per_set - characters }, group));
        }
    }
    if !mobiles.is_empty() {
        println!("\n| crowd, R8 / BC1 | draws per frame | R8 mobiles | BC1 mobiles | format changes per frame | runs per frame |\n|---|---:|---:|---:|---:|---:|");
        for (scale, label, crowd, _) in &mobiles {
            if *scale == INTEGER_SCALE {
                println!("| {label} | {per_set} | {} | {} | {} | {} |", crowd.characters, crowd.creatures, crowd.runs.len().saturating_sub(1), crowd.runs.len());
            }
        }
        println!();
    }
    let full = 100u32;

    let mut rows: Vec<Row> = Vec::new();
    let scale_name = |scale: f32| if scale == INTEGER_SCALE { "1 x".to_string() } else { format!("{scale} x") };
    rows.push(Row { section: "baseline".to_string(), name: "an empty pass: the clear".to_string(), fragments: 0.0, draws: Draws::Empty, per_set, rounds: Vec::new() });
    for &(path, hue, scale) in &sets {
        let mix = mix_at(scale, full);
        rows.push(Row {
            section: if hue == Hue::None { "layouts".to_string() } else { "hue, every sprite".to_string() },
            name: format!("{}, {}, {}", path.name(enhanced), hue.name(), scale_name(scale)),
            fragments: fragments_of(&mix.instances, per_set as usize),
            draws: Draws::One { pipeline: &pipelines[&key(path, hue)], group: &mix.group },
            per_set,
            rounds: Vec::new(),
        });
    }
    for &(first, second, scale) in &pairs {
        let (a, b) = (&pipelines[&key(first, Hue::None)], &pipelines[&key(second, Hue::None)]);
        let branch = &branches[&format!("{first:?}+{second:?}")];
        for mix_share in MIXES {
            let mix = mix_at(scale, mix_share);
            let label = format!("{} + {}, {} x, {mix_share} / {}", first.name(enhanced), second.name(enhanced), scale, PERCENT - mix_share);
            let fragments = fragments_of(&mix.instances, per_set as usize);
            rows.push(Row { section: "pipelines".to_string(), name: format!("a, a pipeline per format, grouped: {label}"), fragments, draws: Draws::Grouped { first: a, second: b, group: &mix.grouped_group, splits: &mix.splits }, per_set, rounds: Vec::new() });
            rows.push(Row { section: "pipelines".to_string(), name: format!("b, one shader, a branch per instance: {label}"), fragments, draws: Draws::One { pipeline: branch, group: &mix.group }, per_set, rounds: Vec::new() });
            rows.push(Row { section: "pipelines".to_string(), name: format!("c, a pipeline per format, a draw per sprite in painter's order: {label}"), fragments, draws: Draws::Each { first: a, second: b, group: &mix.group, instances: &mix.instances }, per_set, rounds: Vec::new() });
        }
    }

    for (scale, alt) in &alternating {
        let fragments = fragments_of(&alt.instances, per_set as usize);
        let bc1 = &pipelines[&key(Path::Bc1, Hue::None)];
        let bc3 = &pipelines[&key(Path::Bc3, Hue::None)];
        let mut added: Vec<(String, Draws)> = Vec::new();
        let mut add = |name: String, draws| added.push((name, draws));
        add(format!("a, one array, one instanced draw: BC1 only, {scale} x"), Draws::One { pipeline: bc1, group: &alt.plain });
        add(format!("a, one array, one instanced draw: BC3 only, {scale} x"), Draws::One { pipeline: bc1, group: &alt.swapped });
        add(format!("one array, a draw per sprite, no switch: BC1 only, {scale} x"), Draws::Separate { pipeline: bc1, group: &alt.plain });
        add(format!("one array, a draw per sprite, no switch: BC3 only, {scale} x"), Draws::Separate { pipeline: bc1, group: &alt.swapped });
        add(format!("b, two arrays, a bind group switch per draw: BC1 / BC3 alternating, {scale} x"), Draws::Switch { pipeline: bc1, groups: [&alt.plain, &alt.swapped], instances: &alt.instances });
        add(format!("c, both arrays in one bind group, a branch per instance: BC1 / BC3 alternating, {scale} x"), Draws::One { pipeline: &branches["Bc1+Bc3"], group: &alt.plain });
        add(format!("a pipeline per format, a draw per sprite: BC1 / BC3 alternating, {scale} x"), Draws::Each { first: bc1, second: bc3, group: &alt.plain, instances: &alt.instances });
        for &(first, at) in &alternating_pairs {
            if at == *scale {
                add(format!("every draw, the upper bound: {} / BC1, one shader with a branch, {scale} x", first.name(enhanced)), Draws::One { pipeline: &branches[&format!("{first:?}+Bc1")], group: &alt.plain });
                add(format!("every draw, the upper bound: {} / BC1, a pipeline per format, a draw per sprite, {scale} x", first.name(enhanced)), Draws::Each { first: &pipelines[&key(first, Hue::None)], second: bc1, group: &alt.plain, instances: &alt.instances });
            }
        }
        for (name, draws) in added {
            rows.push(Row { section: "alternation".to_string(), name, fragments, draws, per_set, rounds: Vec::new() });
        }
    }

    for (scale, crowd_label, crowd, group) in &mobiles {
        let first = alternating_pairs.iter().find(|p| p.1 == *scale).expect("a path").0;
        let (r8, bc1) = (&pipelines[&key(first, Hue::None)], &pipelines[&key(Path::Bc1, Hue::None)]);
        let label = format!("{crowd_label}, {scale} x");
        let fragments = fragments_of(&crowd.instances, per_set as usize);
        let mut push = |name: String, draws| rows.push(Row { section: "characters and creatures".to_string(), name, fragments, draws, per_set, rounds: Vec::new() });
        push(format!("one shader with a branch, one instanced draw: R8 / BC1, {label}"), Draws::One { pipeline: &branches[&format!("{first:?}+Bc1")], group });
        push(format!("a pipeline per format, a draw per run: R8 / BC1, {label}"), Draws::Runs { first: r8, second: bc1, group, runs: &crowd.runs });
        push(format!("a pipeline per format, a draw per sprite: R8 / BC1, {label}"), Draws::Each { first: r8, second: bc1, group, instances: &crowd.instances });
    }

    println!("timed {} rows; {ROUNDS} rounds of {PASSES_PER_SUBMIT} passes; GPU busy before: {}", rows.len(), gpu_busy());
    let mut random = SEED;
    let anchor = rows.iter().position(|r| r.name.starts_with(Path::IndexLoad.name(enhanced)) && r.name.contains(Hue::None.name())).expect("the anchor row");
    for round in 0..ROUNDS {
        let mut order: Vec<usize> = (0..rows.len()).collect();
        for n in (1..order.len()).rev() {
            order.swap(n, next_random(&mut random) as usize % (n + 1));
        }
        let begun = Instant::now();
        let mut results: Vec<(f64, f64)> = vec![(0.0, 0.0); rows.len()];
        for &n in &order {
            for _ in 0..WARM_SUBMITS {
                run_submit(&gpu, &rows[n].draws, rows[n].per_set);
            }
            let timed = run_submit(&gpu, &rows[n].draws, rows[n].per_set);
            results[n] = (timed.gpu_ms, timed.cpu_ms);
        }
        for (row, result) in rows.iter_mut().zip(&results) {
            row.rounds.push(*result);
        }
        println!("  round {} of {ROUNDS}: {:.0} s; anchor {:.3} ms; GPU busy {}", round + 1, begun.elapsed().as_secs_f64(), results[anchor].0, gpu_busy());
    }
    let anchors: Vec<f64> = rows[anchor].rounds.iter().map(|r| r.0).collect();
    println!("\n| section | row | fragments Mpx | GPU ms, median of rounds | lowest round | highest round | x {} | share of a 60 fps frame | CPU encode ms |\n|---|---|---:|---:|---:|---:|---:|---:|---:|", Path::IndexLoad.name(enhanced));
    for row in &rows {
        let mut ms: Vec<f64> = row.rounds.iter().map(|r| r.0).collect();
        let mut ratios: Vec<f64> = ms.iter().zip(&anchors).map(|(m, a)| m / a).collect();
        let mut cpu: Vec<f64> = row.rounds.iter().map(|r| r.1).collect();
        let middle = median(&mut ms);
        println!("| {} | {} | {:.2} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} % | {:.3} |", row.section, row.name, row.fragments, middle, ms[0], ms[ms.len() - 1], median(&mut ratios), middle / BUDGET_60_FPS_MS * 100.0, median(&mut cpu));
    }
}
