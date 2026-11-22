//! The GPU port against PyTorch's reference tensors, the per-pixel steps against the CPU, and the speed. All need a
//! GPU adapter and are ignored by default; the comparison also needs the reference export, a folder named by
//! UPSCALE_REFERENCE_DIR holding `reference.json` and, per case, `{case}_input.f32` (RGB over 50% grey) and
//! `{case}_output.f32` (the 4x RGB, unclamped), CHW f32 little-endian, computed with the same fp16 weights.

use super::*;
use std::time::{Duration, Instant};

const REFERENCE_DIR_VARIABLE: &str = "UPSCALE_REFERENCE_DIR";
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
/// fp16 against fp32 weights alone moves the output by up to 0.08/255; the port should stay well inside this.
const OUTPUT_TOLERANCE: f32 = 0.25 / 255.0;
/// On the matrix units, the activations rounded to fp16 too (0.2 to 0.3/255 on the references); inside the 0.64/255
/// the in-game colour is allowed.
const MATRIX_OUTPUT_TOLERANCE: f32 = 0.5 / 255.0;
/// The input planes: the shader rounds the sRGB decode and encode back to the atlas's bytes.
const INPUT_TOLERANCE: f32 = 1e-6;
/// The 2x texels against the CPU's, in 8-bit steps (rounding of nearly equal values).
const OUTPUT_STEP_TOLERANCE: u8 = 1;
const BENCHMARK_FRAMES: [(u32, u32); 4] = [(48, 40), (80, 85), (128, 128), (256, 256)];
const BENCHMARK_RUNS: usize = 20;
/// The benchmark's frames: the fractional parts of multiples of this, so every texel differs.
const BENCHMARK_INPUT_STEP: f32 = 0.618;
const SLOWEST_DISPATCHES: usize = 6;
const NANOS_PER_MILLI: f64 = 1e6;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;
const BYTE_MAX: f32 = 255.0;
/// The synthetic frame of the per-pixel test, where it sits in a two-layer atlas, the canvas (larger than the frame
/// needs, as a shared canvas would be) and the slot.
const TEST_FRAME: SourceRect = SourceRect { layer: 1, x: 5, y: 3, width: 37, height: 29 };
const TEST_ATLAS_SIZE: (u32, u32, u32) = (64, 48, 2);
const TEST_CANVAS: (u32, u32) = (48, 40);
const TEST_SLOT: usize = 2;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    timestamps: bool,
    /// Where the convolutions can run: the shader cores, and the matrix units when the device has them.
    units: Vec<ConvUnits>,
}

fn gpu() -> Option<Gpu> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.ok()?;
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: (if timestamps { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() })
                | (adapter.features() & ConvUnits::MATRIX_FEATURES),
            required_limits: adapter.limits(),
            // SAFETY: the cooperative matrices are a wgpu experimental feature, used only where ConvUnits::of finds
            // them supported.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }).await.ok()?;
        let mut units = vec![ConvUnits::ShaderCores];
        if ConvUnits::of(&adapter, &device) == ConvUnits::Matrix {
            units.push(ConvUnits::Matrix);
        }
        Some(Gpu { device, queue, timestamps, units })
    })
}

fn wait(device: &wgpu::Device) {
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(POLL_TIMEOUT) }).unwrap();
}

fn read_back(gpu: &Gpu, buffer: &wgpu::Buffer) -> Vec<u8> {
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| sender.send(result).unwrap());
    wait(&gpu.device);
    receiver.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().unwrap().to_vec();
    buffer.unmap();
    bytes
}

fn readback_buffer(gpu: &Gpu, bytes: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    })
}

fn read_f32(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    bytes.chunks_exact(4).map(|word| f32::from_le_bytes(word.try_into().unwrap())).collect()
}

/// Max and mean absolute difference.
fn errors(actual: &[f32], expected: &[f32]) -> (f32, f32) {
    assert_eq!(actual.len(), expected.len());
    let mut max = 0.0f32;
    let mut sum = 0.0f64;
    for (a, e) in actual.iter().zip(expected) {
        let difference = (a - e).abs();
        assert!(difference.is_finite(), "non-finite output");
        max = max.max(difference);
        sum += difference as f64;
    }
    (max, (sum / actual.len() as f64) as f32)
}

/// An RGBA8 sRGB 2D array texture holding `texels` (layer by layer, rows top down), and its view as the atlas's.
fn atlas_texture(gpu: &Gpu, (width, height, layers): (u32, u32, u32), texels: &[u8]) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: layers };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None, size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    gpu.queue.write_texture(texture.as_image_copy(), texels, wgpu::TexelCopyBufferLayout {
        offset: 0, bytes_per_row: Some(width * UPSCALE_OUTPUT_TEXEL_BYTES), rows_per_image: Some(height),
    }, size);
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
    });
    (texture, view)
}

fn record(encoder: &mut wgpu::CommandEncoder, dispatches: &mut dyn Iterator<Item = &UpscaleDispatch>) {
    let mut pass = encoder.begin_compute_pass(&Default::default());
    dispatches.for_each(|dispatch| dispatch.record(&mut pass));
}

/// PixelShuffle(4) of the upsampler's planes into the 4x RGB, CHW.
fn pixel_shuffle(network: &[f32], width: usize, height: usize) -> Vec<f32> {
    let scale = NETWORK_SCALE as usize;
    let (out_width, out_height) = (width * scale, height * scale);
    let mut rgb = vec![0.0; COLOR_CHANNELS as usize * out_width * out_height];
    for channel in 0..COLOR_CHANNELS as usize {
        for y in 0..out_height {
            for x in 0..out_width {
                let plane = channel * scale * scale + (y % scale) * scale + x % scale;
                rgb[(channel * out_height + y) * out_width + x] = network[(plane * height + y / scale) * width + x / scale];
            }
        }
    }
    rgb
}

#[test]
#[ignore = "requires a GPU adapter and UPSCALE_REFERENCE_DIR; run explicitly for upscaler changes"]
fn matches_pytorch_reference() {
    let Some(directory) = std::env::var_os(REFERENCE_DIR_VARIABLE).map(std::path::PathBuf::from) else {
        println!("skipped: {REFERENCE_DIR_VARIABLE} is not set");
        return;
    };
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let index: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.join("reference.json")).unwrap()).unwrap();
    let (_texture, source) = atlas_texture(&gpu, (1, 1, 1), &[0; UPSCALE_OUTPUT_TEXEL_BYTES as usize]);
    let mut failures = Vec::new();
    for &units in &gpu.units {
        let model = UpscaleModel::new(&gpu.device, units).unwrap();
        let tolerance = match units {
            ConvUnits::ShaderCores => OUTPUT_TOLERANCE,
            ConvUnits::Matrix => MATRIX_OUTPUT_TOLERANCE,
        };
        for (case, entry) in index.as_object().unwrap() {
            let width = entry["width"].as_u64().unwrap() as u32;
            let height = entry["height"].as_u64().unwrap() as u32;
            assert_eq!(entry["scale"].as_u64(), Some(NETWORK_SCALE as u64));
            // The reference runs on the frame alone, so the canvas is the frame and the input goes straight in.
            let workspace = UpscaleWorkspace::new(&model, &gpu.device, &source, width, height).unwrap();
            let plane_bytes = (width * height) as u64 * F32_BYTES;
            let input = read_f32(&directory.join(format!("{case}_input.f32")));
            gpu.queue.write_buffer(&workspace.arena, INPUT_PLANE as u64 * plane_bytes, bytemuck::cast_slice(&input));
            let network_bytes = UPSAMPLER_CHANNELS as u64 * plane_bytes;
            let readback = readback_buffer(&gpu, network_bytes);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            record(&mut encoder, &mut workspace.network().iter());
            encoder.copy_buffer_to_buffer(&workspace.arena, NETWORK_OUTPUT_PLANE as u64 * plane_bytes, &readback, 0,
                network_bytes);
            gpu.queue.submit([encoder.finish()]);
            let network: Vec<f32> = bytemuck::cast_slice(&read_back(&gpu, &readback)).to_vec();
            let actual = pixel_shuffle(&network, width as usize, height as usize);
            let expected = read_f32(&directory.join(format!("{case}_output.f32")));
            let (max, mean) = errors(&actual, &expected);
            println!("{units:?}, {case} {width}x{height}: 4x RGB max {max:.6} ({:.4}/255) mean {mean:.7} ({:.5}/255), {} \
                dispatches", max * BYTE_MAX, mean * BYTE_MAX, workspace.network().len());
            if max > tolerance {
                failures.push(format!("{units:?}, {case}: max {max} > {tolerance}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A test frame: colour gradients and a checker under an elliptic alpha with a soft rim and a half-transparent band.
fn synthetic_frame(width: u32, height: u32) -> Vec<u8> {
    let mut texels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / height as f32 * 2.0 - 1.0;
            let radius = (u * u + v * v).sqrt();
            let mut alpha = ((1.0 - radius) * 6.0).clamp(0.0, 1.0);
            if (y / 4) % 3 == 1 {
                alpha *= 0.5;
            }
            let checker = if (x / 3 + y / 3) % 2 == 0 { 0.2 } else { 0.0 };
            let red = (x as f32 / width as f32 + checker).min(1.0);
            let green = y as f32 / height as f32;
            let blue = 1.0 - red * 0.5 - checker;
            texels.extend([red, green, blue, alpha].map(|value| (value * BYTE_MAX).round() as u8));
        }
    }
    texels
}

fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 }
}

fn srgb_decode(encoded: f32) -> f32 {
    if encoded <= 0.04045 { encoded / 12.92 } else { ((encoded + 0.055) / 1.055).powf(2.4) }
}

/// pixel.wgsl's `finish` on the CPU, from the network's planes and the canvas's alpha.
fn cpu_finish(network: &[f32], alpha: &[f32], width: usize, height: usize) -> Vec<[u8; 4]> {
    const MATTE: f32 = 0.5;
    const MIN_ALPHA: f32 = 1.0 / 512.0;
    const CUBIC_A: f32 = -0.5;
    let scale = NETWORK_SCALE as usize;
    let side = BOX_SIDE as usize;
    let rgb = pixel_shuffle(network, width, height);
    let (wide, high) = (width * scale, height * scale);
    let cubic = |t: f32| [CUBIC_A * (t * t * t - 2.0 * t * t + t), (CUBIC_A + 2.0) * t * t * t - (CUBIC_A + 3.0) * t * t + 1.0,
        -(CUBIC_A + 2.0) * t * t * t + (2.0 * CUBIC_A + 3.0) * t * t - CUBIC_A * t, CUBIC_A * (t * t - t * t * t)];
    let alpha_at = |sx: usize, sy: usize| {
        let u = (sx as f32 + 0.5) / scale as f32 - 0.5;
        let v = (sy as f32 + 0.5) / scale as f32 - 0.5;
        let (wx, wy) = (cubic(u - u.floor()), cubic(v - v.floor()));
        let mut sum = 0.0;
        for (row, weight_y) in wy.iter().enumerate() {
            let y = v.floor() as i64 - 1 + row as i64;
            for (column, weight_x) in wx.iter().enumerate() {
                let x = u.floor() as i64 - 1 + column as i64;
                if y >= 0 && x >= 0 && (y as usize) < height && (x as usize) < width {
                    sum += weight_y * weight_x * alpha[y as usize * width + x as usize];
                }
            }
        }
        f32::clamp(sum, 0.0, 1.0)
    };
    let (out_width, out_height) = (wide / side, high / side);
    let mut texels = Vec::new();
    for out_y in 0..out_height {
        for out_x in 0..out_width {
            let mut premultiplied = [0.0f32; 3];
            let mut coverage = 0.0;
            for dy in 0..side {
                for dx in 0..side {
                    let (sx, sy) = (out_x * side + dx, out_y * side + dy);
                    let alpha = alpha_at(sx, sy);
                    if alpha <= MIN_ALPHA {
                        continue;
                    }
                    for (channel, sum) in premultiplied.iter_mut().enumerate() {
                        let over_grey = rgb[(channel * high + sy) * wide + sx];
                        let color = ((over_grey - MATTE * (1.0 - alpha)) / alpha).clamp(0.0, 1.0);
                        *sum += srgb_decode(color) * alpha;
                    }
                    coverage += alpha;
                }
            }
            let color = premultiplied.map(|sum| if coverage > 0.0 { srgb_encode(sum / coverage) } else { 0.0 });
            let alpha = coverage / (side * side) as f32;
            texels.push([color[0], color[1], color[2], alpha].map(|value| (value.clamp(0.0, 1.0) * BYTE_MAX).round() as u8));
        }
    }
    texels
}

#[test]
#[ignore = "requires a GPU adapter; run explicitly for upscaler changes"]
fn frame_steps_match_cpu() {
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let model = UpscaleModel::new(&gpu.device, *gpu.units.last().unwrap()).unwrap();
    let (atlas_width, atlas_height, layers) = TEST_ATLAS_SIZE;
    let frame = TEST_FRAME;
    let frame_texels = synthetic_frame(frame.width, frame.height);
    // The atlas around the frame is opaque white, which must not leak into the canvas.
    let mut atlas = vec![u8::MAX; (atlas_width * atlas_height * layers * UPSCALE_OUTPUT_TEXEL_BYTES) as usize];
    for y in 0..frame.height {
        for x in 0..frame.width {
            let at = (((frame.layer * atlas_height + frame.y + y) * atlas_width + frame.x + x) * UPSCALE_OUTPUT_TEXEL_BYTES) as usize;
            let from = ((y * frame.width + x) * UPSCALE_OUTPUT_TEXEL_BYTES) as usize;
            atlas[at..at + UPSCALE_OUTPUT_TEXEL_BYTES as usize].copy_from_slice(&frame_texels[from..from + UPSCALE_OUTPUT_TEXEL_BYTES as usize]);
        }
    }
    let (_texture, source) = atlas_texture(&gpu, TEST_ATLAS_SIZE, &atlas);
    let (width, height) = TEST_CANVAS;
    let workspace = UpscaleWorkspace::new(&model, &gpu.device, &source, width, height).unwrap();
    assert!(workspace.set_frame(&gpu.queue, TEST_SLOT, SourceRect { width, ..frame }, false).is_err());
    workspace.set_frame(&gpu.queue, TEST_SLOT, frame, false).unwrap();

    let plane_bytes = (width * height) as u64 * F32_BYTES;
    let input_bytes = INPUT_PLANES as u64 * plane_bytes;
    let network_bytes = UPSAMPLER_CHANNELS as u64 * plane_bytes;
    let output_bytes = workspace.output().size();
    let readback = readback_buffer(&gpu, input_bytes + network_bytes + output_bytes);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    record(&mut encoder, &mut workspace.upscale(TEST_SLOT));
    encoder.copy_buffer_to_buffer(&workspace.arena, INPUT_PLANE as u64 * plane_bytes, &readback, 0, input_bytes);
    encoder.copy_buffer_to_buffer(&workspace.arena, NETWORK_OUTPUT_PLANE as u64 * plane_bytes, &readback, input_bytes,
        network_bytes);
    encoder.copy_buffer_to_buffer(workspace.output(), 0, &readback, input_bytes + network_bytes, output_bytes);
    gpu.queue.submit([encoder.finish()]);
    let bytes = read_back(&gpu, &readback);
    let floats: &[f32] = bytemuck::cast_slice(&bytes[..(input_bytes + network_bytes) as usize]);
    let (input, network) = floats.split_at((input_bytes / F32_BYTES) as usize);

    // The input: the frame over grey at the border, grey around it.
    let pixels = (width * height) as usize;
    let mut expected = vec![0.0f32; INPUT_PLANES as usize * pixels];
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as i64 - UPSCALE_BORDER as i64, y as i64 - UPSCALE_BORDER as i64);
            let inside = fx >= 0 && fy >= 0 && fx < frame.width as i64 && fy < frame.height as i64;
            let texel = if inside {
                let at = ((fy as u32 * frame.width + fx as u32) * UPSCALE_OUTPUT_TEXEL_BYTES) as usize;
                std::array::from_fn::<f32, 4, _>(|channel| frame_texels[at + channel] as f32 / BYTE_MAX)
            } else {
                [0.0; 4]
            };
            let pixel = (y * width + x) as usize;
            for channel in 0..COLOR_CHANNELS as usize {
                expected[channel * pixels + pixel] = texel[channel] * texel[3] + 0.5 * (1.0 - texel[3]);
            }
            expected[ALPHA_PLANE as usize * pixels + pixel] = texel[3];
        }
    }
    let (max, mean) = errors(input, &expected);
    println!("input planes: max {max:.7} mean {mean:.8}");
    assert!(max <= INPUT_TOLERANCE, "input planes: max {max} > {INPUT_TOLERANCE}");

    // The output: the CPU's finish from the same network output and the input's alpha.
    let alpha = &input[ALPHA_PLANE as usize * pixels..];
    let expected = cpu_finish(network, alpha, width as usize, height as usize);
    let (out_width, out_height) = workspace.output_size();
    let row = workspace.output_bytes_per_row() as usize;
    let output = &bytes[(input_bytes + network_bytes) as usize..];
    let mut worst = 0u8;
    let mut opaque = 0;
    for y in 0..out_height as usize {
        for x in 0..out_width as usize {
            let at = y * row + x * UPSCALE_OUTPUT_TEXEL_BYTES as usize;
            let texel = &expected[y * out_width as usize + x];
            for channel in 0..UPSCALE_OUTPUT_TEXEL_BYTES as usize {
                worst = worst.max(output[at + channel].abs_diff(texel[channel]));
            }
            opaque += (output[at + 3] == u8::MAX) as usize;
        }
    }
    println!("output texels: worst difference {worst} steps; {opaque} opaque texels of {}", out_width * out_height);
    assert!(worst <= OUTPUT_STEP_TOLERANCE, "output texels differ by {worst} steps");
    assert!(opaque > 0, "the output has no opaque texel");
    // The canvas's corners are further than the bicubic reach from the frame: transparent.
    assert_eq!(output[3], 0, "the output's corner is not transparent");
}

/// Runs each list of dispatches in a compute pass of its own, [`BENCHMARK_RUNS`] times after one uncounted run (the
/// driver compiles the pipelines then), and gives each pass's median GPU time and the median wall time of a run, in
/// ms; without timestamp queries the pass times are NaN.
fn time_passes(gpu: &Gpu, passes: &[Vec<&UpscaleDispatch>]) -> (Vec<f64>, f64) {
    let query_count = 2 * passes.len() as u32;
    let queries = gpu.timestamps.then(|| gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None, ty: wgpu::QueryType::Timestamp, count: query_count,
    }));
    let query_bytes = query_count as u64 * std::mem::size_of::<u64>() as u64;
    let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: query_bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    });
    let readback = readback_buffer(gpu, query_bytes);
    let period = gpu.queue.get_timestamp_period() as f64;
    let mut pass_ms = vec![Vec::new(); passes.len()];
    let mut wall_ms = Vec::new();
    for run in 0..=BENCHMARK_RUNS {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for (index, dispatches) in passes.iter().enumerate() {
            let timestamp_writes = queries.as_ref().map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set, beginning_of_pass_write_index: Some(2 * index as u32),
                end_of_pass_write_index: Some(2 * index as u32 + 1),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes });
            dispatches.iter().for_each(|dispatch| dispatch.record(&mut pass));
        }
        if let Some(query_set) = &queries {
            encoder.resolve_query_set(query_set, 0..query_count, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, query_bytes);
        }
        let start = Instant::now();
        gpu.queue.submit([encoder.finish()]);
        wait(&gpu.device);
        let wall = start.elapsed().as_secs_f64() * 1e3;
        if run == 0 {
            continue;
        }
        wall_ms.push(wall);
        if queries.is_some() {
            let ticks: Vec<u64> = bytemuck::cast_slice(&read_back(gpu, &readback)).to_vec();
            for (index, times) in pass_ms.iter_mut().enumerate() {
                times.push(ticks[2 * index + 1].wrapping_sub(ticks[2 * index]) as f64 * period / NANOS_PER_MILLI);
            }
        }
    }
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
    };
    (pass_ms.iter_mut().map(median).collect(), median(&mut wall_ms))
}

#[test]
#[ignore = "requires a GPU adapter; run explicitly to measure the upscaler"]
fn benchmark() {
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let (atlas_width, atlas_height) = BENCHMARK_FRAMES.iter().fold((0, 0), |(w, h), &(fw, fh)| (w.max(fw), h.max(fh)));
    let texels: Vec<u8> = (0..atlas_width * atlas_height * UPSCALE_OUTPUT_TEXEL_BYTES)
        .map(|index| ((index as f32 * BENCHMARK_INPUT_STEP).fract() * BYTE_MAX) as u8).collect();
    let (_texture, source) = atlas_texture(&gpu, (atlas_width, atlas_height, 1), &texels);
    for &units in &gpu.units {
        let model = UpscaleModel::new(&gpu.device, units).unwrap();
        println!("convolutions on the {units:?}; weights on the GPU: {:.2} MiB; GPU timestamps: {}",
            model.weight_bytes() as f64 / BYTES_PER_MIB, gpu.timestamps);
        for (frame_width, frame_height) in BENCHMARK_FRAMES {
            let (width, height) = upscale_canvas_size(frame_width, frame_height);
            let workspace = UpscaleWorkspace::new(&model, &gpu.device, &source, width, height).unwrap();
            workspace.set_frame(&gpu.queue, 0, SourceRect { layer: 0, x: 0, y: 0, width: frame_width, height: frame_height },
                false)
                .unwrap();
            let stages: Vec<Vec<&UpscaleDispatch>> = vec![
                workspace.prepare(0).iter().collect(), workspace.network().iter().collect(), workspace.finish(0).iter().collect(),
                workspace.upscale(0).collect(),
            ];
            let (stage, wall) = time_passes(&gpu, &stages);
            let single: Vec<&UpscaleDispatch> = workspace.upscale(0).collect();
            let (single_ms, _) = time_passes(&gpu, &single.iter().map(|&dispatch| vec![dispatch]).collect::<Vec<_>>());
            let mut slowest: Vec<(f64, &UpscaleDispatch)> = single_ms.into_iter().zip(single.iter().copied()).collect();
            slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
            let gmac = workspace.network().iter().map(UpscaleDispatch::multiply_adds).sum::<u64>() as f64 / 1e9;
            println!("{frame_width}x{frame_height} (canvas {width}x{height}): GPU frame {:.3} ms (prepare {:.3} + network \
                {:.3} + finish {:.3}), wall {wall:.3} ms", stage[3], stage[0], stage[1], stage[2]);
            println!("  dispatches {} (network {}); {gmac:.3} GMAC, {:.0} GMAC/s; workspace {:.2} MiB",
                workspace.upscale(0).count(), workspace.network().len(), gmac / (stage[1] / 1e3),
                workspace.working_bytes() as f64 / BYTES_PER_MIB);
            for (ms, dispatch) in &slowest[..SLOWEST_DISPATCHES] {
                println!("    {ms:.3} ms {} {:?} workgroups, {:.1} MMAC", dispatch.name(), dispatch.workgroups(),
                    dispatch.multiply_adds() as f64 / 1e6);
            }
        }
    }
}
