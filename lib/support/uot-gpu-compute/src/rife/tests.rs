//! The GPU port against PyTorch's reference tensors, and its speed. Both need a GPU adapter and are ignored by default;
//! the comparison also needs the reference export, a folder named by RIFE_REFERENCE_DIR holding `reference.json` and
//! CHW f32 little-endian tensors computed with the same fp16 weights (the input, each block's accumulated flow, the
//! mask logit and the merged frame at t = 0.25, 0.5 and 0.75). The comparison blends bilinearly, as Practical-RIFE
//! does.

use super::*;
use std::time::{Duration, Instant};

const REFERENCE_DIR_VARIABLE: &str = "RIFE_REFERENCE_DIR";
const TIMESTEPS: [f32; RIFE_TIMESTEP_SLOTS] = [0.25, 0.5, 0.75];
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
/// fp16 against fp32 weights alone moves the merged frame by up to 0.64/255; the port should stay well inside these.
const FLOW_TOLERANCE_PX: f32 = 0.05;
const MASK_TOLERANCE: f32 = 0.01;
const MERGED_TOLERANCE: f32 = 2.0 / 255.0;
/// On the matrix units, where the blocks' activations are rounded to fp16 too, the mask logit moves by up to 0.034
/// on the references (flow 0.018 px, merged frame 1.2/255, both within the f32 tolerances): through the sigmoid
/// (slope at most 1/4) at most 0.0125 of the blend's weight.
const MATRIX_MASK_TOLERANCE: f32 = 0.05;
const BENCHMARK_SIZES: [(u32, u32); 4] = [(64, 64), (128, 128), (256, 256), (576, 512)];
const BENCHMARK_RUNS: usize = 20;
/// The benchmark's input: the fractional parts of multiples of this, so every pixel differs.
const BENCHMARK_INPUT_STEP: f32 = 0.618;
const SLOWEST_DISPATCHES: usize = 6;
const NANOS_PER_MILLI: f64 = 1e6;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

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

fn read_back(gpu: &Gpu, buffer: &wgpu::Buffer) -> Vec<f32> {
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| sender.send(result).unwrap());
    wait(&gpu.device);
    receiver.recv().unwrap().unwrap();
    let values = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
    buffer.unmap();
    values
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

#[test]
#[ignore = "requires a GPU adapter and RIFE_REFERENCE_DIR; run explicitly for RIFE changes"]
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
    let mut failures = Vec::new();
    for &units in &gpu.units {
        let model = RifeModel::with_bilinear_final_warp(&gpu.device, units).unwrap();
        let mask_tolerance = match units {
            ConvUnits::ShaderCores => MASK_TOLERANCE,
            ConvUnits::Matrix => MATRIX_MASK_TOLERANCE,
        };
        for (case, entry) in index.as_object().unwrap() {
            let width = entry["width"].as_u64().unwrap() as u32;
            let height = entry["height"].as_u64().unwrap() as u32;
            let workspace = RifeWorkspace::new(&model, &gpu.device, width, height).unwrap();
            assert_eq!(workspace.size(), (width, height), "reference sizes are padded already");
            let plane = (width * height) as u64;
            let input = read_f32(&directory.join(entry["files"]["input"].as_str().unwrap()));
            gpu.queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&input));
            // Per slot: each block's flow, the mask logit, the merged frame.
            let flow_bytes = RIFE_FLOW_CHANNELS as u64 * plane * F32_BYTES;
            let mask_bytes = plane * F32_BYTES;
            let merged_bytes = RIFE_IMAGE_CHANNELS as u64 * plane * F32_BYTES;
            let slot_bytes = RIFE_BLOCKS as u64 * flow_bytes + mask_bytes + merged_bytes;
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None, size: slot_bytes * RIFE_TIMESTEP_SLOTS as u64, mapped_at_creation: false,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let record = |encoder: &mut wgpu::CommandEncoder, dispatches: &mut dyn Iterator<Item = &RifeDispatch>| {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                dispatches.for_each(|dispatch| dispatch.record(&mut pass));
            };
            record(&mut encoder, &mut workspace.encode().iter());
            for (slot, &timestep) in TIMESTEPS.iter().enumerate() {
                workspace.set_timestep(&gpu.queue, slot, timestep);
                let base = slot as u64 * slot_bytes;
                for block in 0..RIFE_BLOCKS {
                    record(&mut encoder, &mut workspace.block(slot, block).iter());
                    encoder.copy_buffer_to_buffer(workspace.state(), 0, &readback, base + block as u64 * flow_bytes, flow_bytes);
                }
                let mask_offset = RIFE_MASK_CHANNEL as u64 * plane * F32_BYTES;
                encoder.copy_buffer_to_buffer(workspace.state(), mask_offset, &readback,
                    base + RIFE_BLOCKS as u64 * flow_bytes, mask_bytes);
                record(&mut encoder, &mut workspace.merge().iter());
                encoder.copy_buffer_to_buffer(workspace.merged(), 0, &readback,
                    base + RIFE_BLOCKS as u64 * flow_bytes + mask_bytes, merged_bytes);
            }
            gpu.queue.submit([encoder.finish()]);
            let actual = read_back(&gpu, &readback);
            let floats = |bytes: u64| (bytes / F32_BYTES) as usize;
            for (slot, &timestep) in TIMESTEPS.iter().enumerate() {
                let tag = entry["files"][timestep.to_string()].as_str().unwrap();
                let mut at = floats(slot as u64 * slot_bytes);
                let mut check = |what: String, length: usize, tolerance: f32, expected: &[f32]| {
                    let (max, mean) = errors(&actual[at..at + length], expected);
                    println!("{units:?}, {case} t={timestep} {what:<16} max {max:.6} mean {mean:.7}");
                    if max > tolerance {
                        failures.push(format!("{units:?}, {case} t={timestep} {what}: max {max} > {tolerance}"));
                    }
                    at += length;
                };
                for block in 0..RIFE_BLOCKS {
                    let expected = read_f32(&directory.join(format!("{tag}_flow_block{block}.f32")));
                    check(format!("flow block{block}"), floats(flow_bytes), FLOW_TOLERANCE_PX, &expected);
                }
                let expected = read_f32(&directory.join(format!("{tag}_mask_logit.f32")));
                check("mask logit".into(), floats(mask_bytes), mask_tolerance, &expected);
                let expected = read_f32(&directory.join(format!("{tag}_merged.f32")));
                let merged_at = floats(slot as u64 * slot_bytes + RIFE_BLOCKS as u64 * flow_bytes + mask_bytes);
                let merged = &actual[merged_at..merged_at + floats(merged_bytes)];
                let premultiplied = (RIFE_IMAGE_CHANNELS - 4) as usize * plane as usize;
                let (max, mean) = errors(&merged[premultiplied..], &expected[premultiplied..]);
                println!("{units:?}, {case} t={timestep} {:<16} max {max:.6} ({:.3}/255) mean {mean:.7}", "merged RGBA",
                    max * 255.0);
                check("merged, all 7".into(), floats(merged_bytes), MERGED_TOLERANCE, &expected);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Runs each list of dispatches in a compute pass of its own, [`BENCHMARK_RUNS`] times after one uncounted run (the
/// driver compiles the pipelines then), and gives each pass's median GPU time and the median wall time of a run, in
/// ms; without timestamp queries the pass times are NaN.
fn time_passes(gpu: &Gpu, passes: &[Vec<&RifeDispatch>]) -> (Vec<f64>, f64) {
    let query_count = 2 * passes.len() as u32;
    let queries = gpu.timestamps.then(|| gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None, ty: wgpu::QueryType::Timestamp, count: query_count,
    }));
    let query_bytes = query_count as u64 * std::mem::size_of::<u64>() as u64;
    let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: query_bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    });
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: query_bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    });
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
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
            wait(&gpu.device);
            let ticks: Vec<u64> = bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec();
            readback.unmap();
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
#[ignore = "requires a GPU adapter; run explicitly to measure RIFE"]
fn benchmark() {
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    for &units in &gpu.units {
        let model = RifeModel::new(&gpu.device, units).unwrap();
        println!("convolutions on the {units:?}; weights on the GPU: {:.1} MiB; GPU timestamps: {}",
            model.weight_bytes() as f64 / BYTES_PER_MIB, gpu.timestamps);
        for (width, height) in BENCHMARK_SIZES {
            let workspace = RifeWorkspace::new(&model, &gpu.device, width, height).unwrap();
            let input: Vec<f32> = (0..RIFE_INPUT_CHANNELS * width * height)
                .map(|index| (index as f32 * BENCHMARK_INPUT_STEP).fract()).collect();
            gpu.queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&input));
            // The encoding, slot 0's blocks one by one and its blend, then slots 1 and 2 whole.
            let stages: Vec<Vec<&RifeDispatch>> = std::iter::once(workspace.encode().iter().collect())
                .chain((0..RIFE_BLOCKS).map(|block| workspace.block(0, block).iter().collect()))
                .chain(std::iter::once(workspace.merge().iter().collect()))
                .chain((1..RIFE_TIMESTEP_SLOTS).map(|slot| workspace.interpolate(slot).collect()))
                .collect();
            let (stage, wall) = time_passes(&gpu, &stages);
            let encode = stage[0];
            let one_t: f64 = stage[1..=RIFE_BLOCKS + 1].iter().sum();
            let pair = encode + one_t + stage[RIFE_BLOCKS + 2..].iter().sum::<f64>();
            // Each dispatch of one t alone, for the slowest ones.
            let single: Vec<&RifeDispatch> = workspace.encode().iter().chain(workspace.interpolate(0)).collect();
            let (single_ms, _) = time_passes(&gpu, &single.iter().map(|&dispatch| vec![dispatch]).collect::<Vec<_>>());
            let mut slowest: Vec<(f64, &RifeDispatch)> = single_ms.into_iter().zip(single.iter().copied()).collect();
            slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
            let macs = |dispatches: &mut dyn Iterator<Item = &RifeDispatch>| {
                dispatches.map(RifeDispatch::multiply_adds).sum::<u64>() as f64 / 1e9
            };
            println!("{width}x{height}: GPU one t {:.3} ms (encode {encode:.3} + t {one_t:.3}), pair of 3 t {pair:.3} ms, \
                wall pair {wall:.3} ms", encode + one_t);
            println!("  blocks {:?} ms, merge {:.3} ms", stage[1..=RIFE_BLOCKS].iter().map(|ms| format!("{ms:.3}")).collect::<Vec<_>>(),
                stage[RIFE_BLOCKS + 1]);
            println!("  dispatches: encode {}, per t {} ({} per block, merge 1); GMAC encode {:.3}, per t {:.3}; \
                workspace {:.1} MiB", workspace.encode().len(), workspace.interpolate(0).count(), workspace.block(0, 0).len(),
                macs(&mut workspace.encode().iter()), macs(&mut workspace.interpolate(0)),
                workspace.working_bytes() as f64 / BYTES_PER_MIB);
            for (ms, dispatch) in &slowest[..SLOWEST_DISPATCHES] {
                println!("    {ms:.3} ms {} {:?} workgroups, {:.1} MMAC", dispatch.name(), dispatch.workgroups(),
                    dispatch.multiply_adds() as f64 / 1e6);
            }
        }
    }
}

/// The frame steps (frames.rs) against the CPU: two frames of a texture packed onto a card at their offsets, a frame
/// copied as it is, and an in-between stored with its alpha snapped.
#[test]
#[ignore = "requires a GPU adapter; run explicitly for frame step changes"]
fn frame_steps_carry_frames_between_a_texture_and_a_workspace() {
    use frames::{rife_frame_step_stride, rife_output_bytes_per_row, CardFrame, RifeFrameStep};
    use crate::SourceRect;
    const SOURCE_SIDE: u32 = 40;
    const SOURCE_LAYERS: u32 = 2;
    const TEXEL_BYTES: u32 = 4;
    const SIDE: u32 = RIFE_SIZE_MULTIPLE;
    const PREMULTIPLIED: usize = 3;
    const COLOR_CHANNELS: usize = 3;
    const GREY: f32 = 0.5;
    const SNAP: f32 = 0.5;
    const ALPHAS: [u8; 3] = [0, 128, 255];
    /// An sRGB texel loads linear and the steps encode it again: within a step of 8 bits and a half.
    const TOLERANCE: f32 = 1.5 / 255.0;
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let device = &gpu.device;
    let model = RifeModel::new(device, *gpu.units.last().unwrap()).unwrap();
    let workspace = RifeWorkspace::new(&model, device, SIDE, SIDE).unwrap();
    // Every texel its own; a third of them transparent, a third half covered.
    let texel = |layer: u32, x: u32, y: u32| -> [u8; 4] {
        let seed = layer * 7919 + y * 131 + x * 17;
        [(seed % 251) as u8, (seed * 3 % 241) as u8, (seed * 5 % 239) as u8, ALPHAS[(seed % 3) as usize]]
    };
    let mut pixels = Vec::new();
    for layer in 0..SOURCE_LAYERS {
        for y in 0..SOURCE_SIDE {
            for x in 0..SOURCE_SIDE {
                pixels.extend(texel(layer, x, y));
            }
        }
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: SOURCE_SIDE, height: SOURCE_SIDE, depth_or_array_layers: SOURCE_LAYERS },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    gpu.queue.write_texture(texture.as_image_copy(), &pixels, wgpu::TexelCopyBufferLayout {
        offset: 0, bytes_per_row: Some(SOURCE_SIDE * TEXEL_BYTES), rows_per_image: Some(SOURCE_SIDE),
    }, texture.size());
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
    });

    let first = CardFrame { source: SourceRect { layer: 1, x: 3, y: 5, width: 20, height: 18 }, offset: (2, 1) };
    let next = CardFrame { source: SourceRect { layer: 0, x: 10, y: 12, width: 16, height: 20 }, offset: (0, 0) };
    let card = (24, 22);
    // A merged frame: premultiplied colour and coverage per pixel.
    let plane = (SIDE * SIDE) as usize;
    let coverage = |index: usize| (index % 10) as f32 / 9.0;
    let premultiplied = |index: usize, channel: usize| coverage(index) * ((index * (channel + 3)) % 97) as f32 / 96.0;
    let mut merged = vec![0f32; RIFE_IMAGE_CHANNELS as usize * plane];
    for index in 0..plane {
        for channel in 0..COLOR_CHANNELS {
            merged[(PREMULTIPLIED + channel) * plane + index] = premultiplied(index, channel);
        }
        merged[RIFE_ALPHA_CHANNEL as usize * plane + index] = coverage(index);
    }
    gpu.queue.write_buffer(workspace.merged(), 0, bytemuck::cast_slice(&merged));

    let steps = [
        RifeFrameStep::pack((SIDE, SIDE), first, next),
        RifeFrameStep::copy_frame((SIDE, SIDE), first.source),
        RifeFrameStep::store((SIDE, SIDE), card, true),
    ];
    let stride = rife_frame_step_stride(device);
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: stride * steps.len() as u64, mapped_at_creation: false,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    for (index, step) in steps.iter().enumerate() {
        gpu.queue.write_buffer(&uniforms, index as u64 * stride, step.uniform());
    }
    let output_bytes = u64::from(rife_output_bytes_per_row(card.0)) * u64::from(card.1);
    let buffer = |size, usage| device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, usage, mapped_at_creation: false,
    });
    let outputs = [(); 2].map(|_| buffer(output_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC));
    let readbacks = [(); 2].map(|_| buffer(output_bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST));
    let input_bytes = workspace.input().size();
    let input_readback = buffer(input_bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
    let bind_groups =
        outputs.each_ref().map(|output| model.frame_bind_group(device, &workspace, output, &uniforms, &view));
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        steps[0].record(&model, &mut pass, &bind_groups[0], 0);
        steps[1].record(&model, &mut pass, &bind_groups[0], stride as u32);
        steps[2].record(&model, &mut pass, &bind_groups[1], 2 * stride as u32);
    }
    encoder.copy_buffer_to_buffer(workspace.input(), 0, &input_readback, 0, input_bytes);
    for (output, readback) in outputs.iter().zip(&readbacks) {
        encoder.copy_buffer_to_buffer(output, 0, readback, 0, output_bytes);
    }
    gpu.queue.submit([encoder.finish()]);
    let input = read_back(&gpu, &input_readback);
    let bytes = |readback: &wgpu::Buffer| bytemuck::cast_slice::<f32, u8>(&read_back(&gpu, readback)).to_vec();
    let (copied, stored) = (bytes(&readbacks[0]), bytes(&readbacks[1]));

    let unit = |byte: u8| f32::from(byte) / 255.0;
    let near = |actual: f32, expected: f32, what: &str| {
        assert!((actual - expected).abs() <= TOLERANCE, "{what}: {actual} for {expected}");
    };
    // The card's images: each frame's texel at its place, transparent around it, colour over grey and premultiplied.
    for (image, frame) in [first, next].iter().enumerate() {
        let base = image * RIFE_IMAGE_CHANNELS as usize;
        let (offset_x, offset_y) = frame.offset;
        for y in 0..SIDE {
            for x in 0..SIDE {
                let inside = x >= offset_x && y >= offset_y && x < offset_x + frame.source.width
                    && y < offset_y + frame.source.height;
                let rgba = if inside {
                    texel(frame.source.layer, frame.source.x + x - offset_x, frame.source.y + y - offset_y).map(unit)
                } else {
                    [0.0; 4]
                };
                let index = (y * SIDE + x) as usize;
                let alpha = rgba[3];
                for channel in 0..COLOR_CHANNELS {
                    near(input[(base + channel) * plane + index], rgba[channel] * alpha + GREY * (1.0 - alpha),
                        "over grey");
                    near(input[(base + PREMULTIPLIED + channel) * plane + index], rgba[channel] * alpha,
                        "premultiplied");
                }
                near(input[(base + RIFE_ALPHA_CHANNEL as usize) * plane + index], alpha, "alpha");
            }
        }
    }
    let pitch = rife_output_bytes_per_row(card.0) as usize;
    let texel_bytes = TEXEL_BYTES as usize;
    // The copy: the frame as it is, at the rows' corner.
    for y in 0..first.source.height {
        for x in 0..first.source.width {
            let at = y as usize * pitch + x as usize * texel_bytes;
            let expected = texel(first.source.layer, first.source.x + x, first.source.y + y);
            for channel in 0..texel_bytes {
                near(unit(copied[at + channel]), unit(expected[channel]), "copy");
            }
        }
    }
    // The store: straight colour, alpha snapped.
    for y in 0..card.1 {
        for x in 0..card.0 {
            let index = (y * SIDE + x) as usize;
            let at = y as usize * pitch + x as usize * texel_bytes;
            let alpha = if coverage(index) >= SNAP { 1.0 } else { 0.0 };
            near(unit(stored[at + COLOR_CHANNELS]), alpha, "snapped alpha");
            for channel in 0..COLOR_CHANNELS {
                let expected = if alpha > 0.0 {
                    (premultiplied(index, channel) / coverage(index)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                near(unit(stored[at + channel]), expected, "stored colour");
            }
        }
    }
}
