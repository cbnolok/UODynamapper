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
}

fn gpu() -> Option<Gpu> {
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.ok()?;
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: if timestamps { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() },
            required_limits: adapter.limits(), ..Default::default()
        }).await.ok()?;
        Some(Gpu { device, queue, timestamps })
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
    let model = RifeModel::with_bilinear_final_warp(&gpu.device).unwrap();
    let mut failures = Vec::new();
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
                println!("{case} t={timestep} {what:<16} max {max:.6} mean {mean:.7}");
                if max > tolerance {
                    failures.push(format!("{case} t={timestep} {what}: max {max} > {tolerance}"));
                }
                at += length;
            };
            for block in 0..RIFE_BLOCKS {
                let expected = read_f32(&directory.join(format!("{tag}_flow_block{block}.f32")));
                check(format!("flow block{block}"), floats(flow_bytes), FLOW_TOLERANCE_PX, &expected);
            }
            let expected = read_f32(&directory.join(format!("{tag}_mask_logit.f32")));
            check("mask logit".into(), floats(mask_bytes), MASK_TOLERANCE, &expected);
            let expected = read_f32(&directory.join(format!("{tag}_merged.f32")));
            let merged_at = floats(slot as u64 * slot_bytes + RIFE_BLOCKS as u64 * flow_bytes + mask_bytes);
            let merged = &actual[merged_at..merged_at + floats(merged_bytes)];
            let premultiplied = (RIFE_IMAGE_CHANNELS - 4) as usize * plane as usize;
            let (max, mean) = errors(&merged[premultiplied..], &expected[premultiplied..]);
            println!("{case} t={timestep} {:<16} max {max:.6} ({:.3}/255) mean {mean:.7}", "merged RGBA", max * 255.0);
            check("merged, all 7".into(), floats(merged_bytes), MERGED_TOLERANCE, &expected);
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
    let model = RifeModel::new(&gpu.device).unwrap();
    println!("weights on the GPU: {:.1} MiB; GPU timestamps: {}", model.weight_bytes() as f64 / BYTES_PER_MIB, gpu.timestamps);
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
