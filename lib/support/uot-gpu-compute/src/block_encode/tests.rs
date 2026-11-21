//! The evaluation of BC7 and BC1 for the frame atlas, on real frames of the animation packages and on RIFE
//! in-betweens: GPU time per frame (timestamp queries; each frame in a submission of its own, and the frames of a
//! group in one pass), the error against the source (RGB where the source has alpha, alpha everywhere; for BC1,
//! whether every alpha stays 0 or 255), and the bytes against RGBA8. It writes comparison sheets and the summary into
//! a folder. Ignored: it needs a GPU adapter and the packages. BC1 takes part with the crate's `bc1` feature
//! (`--features bc1`).
//!
//! ```sh
//! BLOCK_ENCODE_PACKAGES=/path/to/uddp BLOCK_ENCODE_OUTPUT=/path/to/bc-eval \
//!     cargo test -p uot-gpu-compute --lib -- --ignored --exact \
//!     block_encode::tests::evaluate_block_formats --nocapture
//! ```
//!
//! A sheet shows the frame 4x, nearest, over 50% grey: the source, then each encoding of [`SHEET_FORMATS`] the frame
//! takes; below each encoding, its difference from the source over the grey, amplified [`DIFF_GAIN`] times.

use super::*;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;
use udd_assets::animation::mobile_anim_cc::{self as anim_cc, MobileAnimCcPackage};
use udd_assets::animation::mobile_anim_ec::{self as anim_ec, MobileAnimEcPackage};
use uot_texture_codecs::bc7::{Bc7EncoderBackend, ImageExtent, RawImageFormat};
use udd_assets::animation::mobile_strides::{MobileAnimClips, MobileClip};
use crate::network_shader_module;
use crate::rife::{RifeModel, RifeWorkspace, RIFE_ALPHA_CHANNEL, RIFE_IMAGE_CHANNELS};

const PACKAGES_VARIABLE: &str = "BLOCK_ENCODE_PACKAGES";
const OUTPUT_VARIABLE: &str = "BLOCK_ENCODE_OUTPUT";
/// block_compression's shader/bc7.wgsl, to time it without wgpu's loop bounding (block_compression keeps it on for
/// BC7) and compare its blocks with the bounded ones'.
const UNBOUNDED_BC7_VARIABLE: &str = "BLOCK_ENCODE_BC7_SHADER";
const UNBOUNDED_BC7_ENTRY: &str = "compress_bc7";
const UNBOUNDED_BC7_WORKGROUP_SIDE: u32 = 8;
const CLASSIC_PACKAGE: &str = "mobile_anim_cc.uddp";
const ENHANCED_PACKAGE: &str = "mobile_anim_ec.uddp";
const SUMMARY_FILE: &str = "summary.txt";
/// Horse, human, dog.
const CLASSIC_BODIES: [u32; 3] = [200, 400, 217];
/// Dog, horse, wyrm, daemon, stygian.
const ENHANCED_BODIES: [u32; 5] = [217, 200, 46, 9, 826];
/// A side facing, where the walk shows the legs apart.
const DIRECTION: u8 = 5;
const FRAMES_PER_CLIP: usize = 4;
/// Timed runs after one uncounted run (the driver compiles the pipelines then).
const RUNS: usize = 5;
const IN_BETWEEN_SIZES: [u32; 3] = [64, 128, 256];
const IN_BETWEEN_T: f32 = 0.5;
const IN_BETWEEN_SLOT: usize = 0;
/// RIFE's backdrop and the Classic alpha snap, as rife/frames.wgsl gives them.
const BACKDROP_GREY: f32 = 0.5;
const CLASSIC_ALPHA_SNAP: f32 = 0.5;
#[cfg(feature = "bc1")]
const SHEET_FORMATS: [MobileBlockFormat; 4] = [MobileBlockFormat::Bc7(Bc7Quality::UltraFast),
    MobileBlockFormat::Bc7(Bc7Quality::Fast), MobileBlockFormat::Bc7(Bc7Quality::Basic), MobileBlockFormat::Bc1];
#[cfg(not(feature = "bc1"))]
const SHEET_FORMATS: [MobileBlockFormat; 3] = [MobileBlockFormat::Bc7(Bc7Quality::UltraFast),
    MobileBlockFormat::Bc7(Bc7Quality::Fast), MobileBlockFormat::Bc7(Bc7Quality::Basic)];
/// The formats evaluated besides BC7's qualities: BC1 with the `bc1` feature.
#[cfg(feature = "bc1")]
const OTHER_FORMATS: &[MobileBlockFormat] = &[MobileBlockFormat::Bc1];
#[cfg(not(feature = "bc1"))]
const OTHER_FORMATS: &[MobileBlockFormat] = &[];
const ZOOM: u32 = 4;
const DIFF_GAIN: f32 = 8.0;
const SHEET_GREY: u8 = 128;
const SHEET_BACKGROUND: u8 = 32;
const SHEET_GAP: u32 = 4;
/// The frame's place in the decoding check's layer: away from the corner, on a block.
const CHECK_ORIGIN: u32 = 8;
const CHECK_LAYERS: u32 = 2;
const CHECK_LAYER: u32 = 1;
const CHECK_THREADS: u32 = 64;
const RGBA_BYTES: usize = 4;
const ALPHA: usize = 3;
const F32_BYTES: u64 = 4;
const CHANNEL_MAX: f32 = 255.0;
const OUTPUT_ALIGNMENT: u64 = 256;
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
const NANOS_PER_MILLI: f64 = 1e6;
const KIB: f64 = 1024.0;
// sRGB's encoding (IEC 61966-2-1).
const SRGB_LINEAR_LIMIT: f32 = 0.0031308;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_SCALE: f32 = 1.055;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_EXPONENT: f32 = 2.4;

/// Reads a layer of a BC texture back as the GPU decodes it: linear RGB, as a shader samples an sRGB format.
const DECODE_CHECK_SHADER: &str = "
struct Region { width: u32, layer: u32 }
@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> texels: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> region: Region;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&texels)) { return; }
    texels[id.x] = textureLoad(atlas, vec2<u32>(id.x % region.width, id.x / region.width), region.layer, 0);
}";

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    timestamps: bool,
    bc_textures: bool,
}

fn gpu() -> Option<Gpu> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.ok()?;
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {})", info.name, info.backend, info.driver_info);
        let wanted = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TEXTURE_COMPRESSION_BC;
        let features = adapter.features() & wanted;
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features, required_limits: adapter.limits(), ..Default::default()
        }).await.ok()?;
        Some(Gpu { device, queue, timestamps: features.contains(wgpu::Features::TIMESTAMP_QUERY),
            bc_textures: features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) })
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

fn readback_buffer(gpu: &Gpu, size: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, mapped_at_creation: false, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    })
}

/// A frame to encode: straight-alpha sRGB RGBA8, rows top down.
struct Sample {
    name: String,
    group: String,
    /// A Classic frame, which BC1 takes (only with the `bc1` feature).
    #[cfg_attr(not(feature = "bc1"), expect(dead_code, reason = "only BC1 reads it"))]
    classic: bool,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    sheet: bool,
}

/// A sample on the GPU: an Rgba8Unorm texture of its padded size, the padding transparent.
struct Source {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn upload(gpu: &Gpu, sample: &Sample) -> Source {
    let (width, height) = padded_size(sample.width, sample.height);
    let mut padded = vec![0u8; (width * height) as usize * RGBA_BYTES];
    let row = sample.width as usize * RGBA_BYTES;
    for (y, source_row) in sample.rgba.chunks_exact(row).enumerate() {
        padded[y * width as usize * RGBA_BYTES..][..row].copy_from_slice(source_row);
    }
    let texture = gpu.device.create_texture_with_data(&gpu.queue, &wgpu::TextureDescriptor {
        label: Some("block encode source"), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
    }, wgpu::util::TextureDataOrder::LayerMajor, &padded);
    let view = texture.create_view(&Default::default());
    Source { _texture: texture, view }
}

fn format_name(format: MobileBlockFormat) -> String {
    match format {
        MobileBlockFormat::Bc7(quality) => format!("BC7 {quality:?}"),
        #[cfg(feature = "bc1")]
        MobileBlockFormat::Bc1 => "BC1".to_string(),
    }
}

#[cfg_attr(not(feature = "bc1"), allow(unused_variables))]
fn takes(format: MobileBlockFormat, sample: &Sample) -> bool {
    match format {
        MobileBlockFormat::Bc7(_) => true,
        #[cfg(feature = "bc1")]
        MobileBlockFormat::Bc1 => sample.classic,
    }
}

/// The encodings of a list of samples in one format, and their GPU times.
struct Encoded {
    /// Each sample's blocks.
    blocks: Vec<Vec<u8>>,
    /// Each sample's median GPU time encoded alone, ms.
    alone_ms: Vec<f64>,
    /// The median GPU time of all of them in one pass, ms.
    together_ms: f64,
}

fn time_ms(ticks: &[u64], period: f64, pair: usize) -> f64 {
    ticks[2 * pair + 1].wrapping_sub(ticks[2 * pair]) as f64 * period / NANOS_PER_MILLI
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
}

fn encode(gpu: &Gpu, encoder: &mut MobileBlockEncoder, format: MobileBlockFormat, samples: &[&Sample],
    sources: &[&Source]) -> Encoded {
    let offsets: Vec<u64> = samples.iter().scan(0u64, |next, sample| {
        let offset = *next;
        *next += format.encoded_bytes(sample.width, sample.height).div_ceil(OUTPUT_ALIGNMENT) * OUTPUT_ALIGNMENT;
        Some(offset)
    }).collect();
    let total = offsets.last().unwrap() + format.encoded_bytes(samples.last().unwrap().width,
        samples.last().unwrap().height);
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blocks"), size: total, mapped_at_creation: false,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    // A pair of queries per sample alone, and one for all of them together.
    let query_count = 2 * samples.len() as u32 + 2;
    let queries = gpu.timestamps.then(|| gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None, ty: wgpu::QueryType::Timestamp, count: query_count,
    }));
    let query_bytes = u64::from(query_count) * std::mem::size_of::<u64>() as u64;
    let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: query_bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    });
    let timestamp_readback = readback_buffer(gpu, query_bytes);
    let period = f64::from(gpu.queue.get_timestamp_period());
    let writes = |pair: u32| queries.as_ref().map(|query_set| wgpu::ComputePassTimestampWrites {
        query_set, beginning_of_pass_write_index: Some(2 * pair), end_of_pass_write_index: Some(2 * pair + 1),
    });
    let together_pair = samples.len() as u32;
    let mut alone = vec![Vec::new(); samples.len()];
    let mut together = Vec::new();
    for run in 0..=RUNS {
        // Alone: a submission per sample, since block_compression writes its uniforms through the queue.
        for (index, (sample, source)) in samples.iter().zip(sources).enumerate() {
            let mut commands = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None, timestamp_writes: writes(index as u32),
                });
                encoder.add_frame(format, &source.view, sample.width, sample.height, &output, offsets[index]);
                encoder.record(&mut pass);
            }
            gpu.queue.submit([commands.finish()]);
        }
        let mut commands = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None, timestamp_writes: writes(together_pair),
            });
            for (index, (sample, source)) in samples.iter().zip(sources).enumerate() {
                encoder.add_frame(format, &source.view, sample.width, sample.height, &output, offsets[index]);
            }
            encoder.record(&mut pass);
        }
        if let Some(query_set) = &queries {
            commands.resolve_query_set(query_set, 0..query_count, &resolve, 0);
            commands.copy_buffer_to_buffer(&resolve, 0, &timestamp_readback, 0, query_bytes);
        }
        gpu.queue.submit([commands.finish()]);
        wait(&gpu.device);
        if run == 0 || queries.is_none() {
            continue;
        }
        let ticks: Vec<u64> = bytemuck::cast_slice(&read_back(gpu, &timestamp_readback)).to_vec();
        for (index, times) in alone.iter_mut().enumerate() {
            times.push(time_ms(&ticks, period, index));
        }
        together.push(time_ms(&ticks, period, together_pair as usize));
    }
    let readback = readback_buffer(gpu, total);
    let mut commands = gpu.device.create_command_encoder(&Default::default());
    commands.copy_buffer_to_buffer(&output, 0, &readback, 0, total);
    gpu.queue.submit([commands.finish()]);
    let bytes = read_back(gpu, &readback);
    let blocks = samples.iter().zip(&offsets).map(|(sample, &offset)| {
        bytes[offset as usize..][..format.encoded_bytes(sample.width, sample.height) as usize].to_vec()
    }).collect();
    Encoded { blocks, alone_ms: alone.iter_mut().map(|times| median(times)).collect(), together_ms: median(&mut together) }
}

/// block_compression's BC7 kernel built without wgpu's loop bounding (`shader` is its source).
struct UnboundedBc7 {
    pipeline: wgpu::ComputePipeline,
}

impl UnboundedBc7 {
    fn new(gpu: &Gpu, shader: &str) -> Self {
        // SAFETY: an evaluation only; block_compression's loops are bounded by constants or by the block's texels.
        let module = unsafe { network_shader_module(&gpu.device, "bc7 unbounded", shader.to_string()) };
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bc7 unbounded"), layout: None, module: &module, entry_point: Some(UNBOUNDED_BC7_ENTRY),
            compilation_options: Default::default(), cache: None,
        });
        Self { pipeline }
    }

    /// The sample's blocks and the median GPU time of its encoding, ms.
    fn encode(&self, gpu: &Gpu, quality: Bc7Quality, sample: &Sample, source: &Source) -> (Vec<u8>, f64) {
        let format = MobileBlockFormat::Bc7(quality);
        let (width, height) = padded_size(sample.width, sample.height);
        let size = format.encoded_bytes(sample.width, sample.height);
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size, mapped_at_creation: false, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        // Uniforms: width, height, texture_y_offset, blocks_offset.
        let uniforms = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&[width, height, 0, 0]), usage: wgpu::BufferUsages::UNIFORM,
        });
        let settings = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::bytes_of(&quality.settings()), usage: wgpu::BufferUsages::STORAGE,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source.view) },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: settings.as_entire_binding() },
            ],
        });
        let queries = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: None, ty: wgpu::QueryType::Timestamp, count: 2,
        });
        let query_bytes = 2 * std::mem::size_of::<u64>() as u64;
        let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: query_bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        });
        let timestamp_readback = readback_buffer(gpu, query_bytes);
        let period = f64::from(gpu.queue.get_timestamp_period());
        let workgroups = |side: u32| (side / BLOCK_SIDE).div_ceil(UNBOUNDED_BC7_WORKGROUP_SIDE);
        let mut times = Vec::new();
        for run in 0..=RUNS {
            let mut commands = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None, timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                        query_set: &queries, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1),
                    }),
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(workgroups(width), workgroups(height), 1);
            }
            commands.resolve_query_set(&queries, 0..2, &resolve, 0);
            commands.copy_buffer_to_buffer(&resolve, 0, &timestamp_readback, 0, query_bytes);
            gpu.queue.submit([commands.finish()]);
            let ticks: Vec<u64> = bytemuck::cast_slice(&read_back(gpu, &timestamp_readback)).to_vec();
            if run > 0 {
                times.push(time_ms(&ticks, period, 0));
            }
        }
        let readback = readback_buffer(gpu, size);
        let mut commands = gpu.device.create_command_encoder(&Default::default());
        commands.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        gpu.queue.submit([commands.finish()]);
        (read_back(gpu, &readback), median(&mut times))
    }
}

/// The sample's texels as the blocks decode them, on the CPU.
fn decode(format: MobileBlockFormat, sample: &Sample, blocks: &[u8]) -> Vec<u8> {
    let (width, height) = padded_size(sample.width, sample.height);
    let extent = ImageExtent::new(width, height).unwrap();
    let padded = match format {
        MobileBlockFormat::Bc7(_) => uot_texture_codecs::bc7::decode_bc7_to_rgba8888(blocks, extent),
        #[cfg(feature = "bc1")]
        MobileBlockFormat::Bc1 => uot_texture_codecs::bc1::decode_bc1_to_rgba8888(blocks, extent),
    }.unwrap();
    let row = sample.width as usize * RGBA_BYTES;
    (0..sample.height as usize).flat_map(|y| padded[y * width as usize * RGBA_BYTES..][..row].iter().copied()).collect()
}

/// Errors of decoded texels against their source.
#[derive(Default, Clone)]
struct Errors {
    rgb_squared: f64,
    rgb_count: u64,
    rgb_max: u8,
    alpha_squared: f64,
    alpha_count: u64,
    alpha_max: u8,
    /// The colour as it shows, premultiplied by alpha, over every texel.
    shown_squared: f64,
    shown_count: u64,
    shown_max: u8,
    /// Decoded alpha neither 0 nor 255.
    soft_alpha: u64,
    /// Transparent in the source, not decoded.
    halo: u64,
    halo_max: u8,
    /// Opaque in the source, not decoded.
    holes: u64,
}

impl Errors {
    fn add(&mut self, source: &[u8], decoded: &[u8]) {
        for (s, d) in source.chunks_exact(RGBA_BYTES).zip(decoded.chunks_exact(RGBA_BYTES)) {
            if s[ALPHA] > 0 {
                for channel in 0..ALPHA {
                    let error = s[channel].abs_diff(d[channel]);
                    self.rgb_squared += f64::from(error).powi(2);
                    self.rgb_max = self.rgb_max.max(error);
                }
                self.rgb_count += ALPHA as u64;
            }
            let premultiplied = |texel: &[u8], channel: usize| {
                (f32::from(texel[channel]) * f32::from(texel[ALPHA]) / CHANNEL_MAX).round() as u8
            };
            for channel in 0..ALPHA {
                let error = premultiplied(s, channel).abs_diff(premultiplied(d, channel));
                self.shown_squared += f64::from(error).powi(2);
                self.shown_max = self.shown_max.max(error);
            }
            self.shown_count += ALPHA as u64;
            let error = s[ALPHA].abs_diff(d[ALPHA]);
            self.alpha_squared += f64::from(error).powi(2);
            self.alpha_count += 1;
            self.alpha_max = self.alpha_max.max(error);
            self.soft_alpha += u64::from(d[ALPHA] != 0 && d[ALPHA] != u8::MAX);
            self.halo += u64::from(s[ALPHA] == 0 && d[ALPHA] != 0);
            if s[ALPHA] == 0 {
                self.halo_max = self.halo_max.max(d[ALPHA]);
            }
            self.holes += u64::from(s[ALPHA] == u8::MAX && d[ALPHA] != u8::MAX);
        }
    }

    fn psnr(squared: f64, count: u64) -> f64 {
        let mse = squared / count.max(1) as f64;
        if mse == 0.0 { f64::INFINITY } else { 10.0 * (f64::from(CHANNEL_MAX).powi(2) / mse).log10() }
    }

    /// The table's error columns.
    fn columns(&self) -> String {
        format!("{:.2} | {} | {:.2} | {} | {:.2} | {} | {} | {} ({}) | {}", Self::psnr(self.rgb_squared, self.rgb_count),
            self.rgb_max, Self::psnr(self.shown_squared, self.shown_count), self.shown_max,
            Self::psnr(self.alpha_squared, self.alpha_count), self.alpha_max, self.soft_alpha, self.halo, self.halo_max,
            self.holes)
    }
}

fn composite(rgba: &[u8]) -> Vec<[u8; 3]> {
    rgba.chunks_exact(RGBA_BYTES).map(|texel| {
        let alpha = f32::from(texel[ALPHA]) / CHANNEL_MAX;
        std::array::from_fn(|channel| {
            (f32::from(texel[channel]) * alpha + f32::from(SHEET_GREY) * (1.0 - alpha)).round() as u8
        })
    }).collect()
}

/// The sheet of a sample: the source and its encodings, their differences below.
fn write_sheet(path: &Path, sample: &Sample, decoded: &[Vec<u8>]) {
    let (panel_width, panel_height) = (sample.width * ZOOM, sample.height * ZOOM);
    let columns = 1 + decoded.len() as u32;
    let width = columns * panel_width + (columns + 1) * SHEET_GAP;
    let height = 2 * panel_height + 3 * SHEET_GAP;
    let mut sheet = image::RgbImage::from_pixel(width, height, image::Rgb([SHEET_BACKGROUND; 3]));
    let mut draw = |column: u32, row: u32, texels: &[[u8; 3]]| {
        let (left, top) = (SHEET_GAP + column * (panel_width + SHEET_GAP), SHEET_GAP + row * (panel_height + SHEET_GAP));
        for y in 0..panel_height {
            for x in 0..panel_width {
                let texel = texels[((y / ZOOM) * sample.width + x / ZOOM) as usize];
                sheet.put_pixel(left + x, top + y, image::Rgb(texel));
            }
        }
    };
    let source = composite(&sample.rgba);
    draw(0, 0, &source);
    for (index, rgba) in decoded.iter().enumerate() {
        let column = 1 + index as u32;
        let shown = composite(rgba);
        draw(column, 0, &shown);
        let difference: Vec<[u8; 3]> = shown.iter().zip(&source).map(|(d, s)| {
            std::array::from_fn(|channel| (f32::from(d[channel].abs_diff(s[channel])) * DIFF_GAIN).min(CHANNEL_MAX) as u8)
        }).collect();
        draw(column, 1, &difference);
    }
    sheet.save(path).unwrap();
}

fn file_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// sRGB's encoding of a linear value to a byte (IEC 61966-2-1).
fn srgb_byte(linear: f32) -> u8 {
    let encoded = if linear <= SRGB_LINEAR_LIMIT { linear * SRGB_LINEAR_SLOPE } else {
        SRGB_SCALE * linear.powf(SRGB_EXPONENT.recip()) - SRGB_OFFSET
    };
    (encoded.clamp(0.0, 1.0) * CHANNEL_MAX).round() as u8
}

/// Copies a sample's blocks into a layer of an sRGB BC array texture with [`record_copy_to_layer`], reads it back
/// through a shader, and gives the largest difference from the CPU decoding inside the frame and the largest alpha
/// around it, in 8-bit steps.
fn check_gpu_decoding(gpu: &Gpu, format: MobileBlockFormat, sample: &Sample, blocks: &[u8], cpu: &[u8]) -> (u8, u8) {
    let (padded_width, padded_height) = padded_size(sample.width, sample.height);
    let (width, height) = (padded_width + 2 * CHECK_ORIGIN, padded_height + 2 * CHECK_ORIGIN);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("decoding check"), size: wgpu::Extent3d { width, height, depth_or_array_layers: CHECK_LAYERS },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: format.texture_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    #[cfg(feature = "bc1")]
    if format == MobileBlockFormat::Bc1 {
        let blocks_per_layer = (width / BLOCK_SIDE * height / BLOCK_SIDE) as usize;
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: CHECK_LAYER }, aspect: wgpu::TextureAspect::All },
            &BC1_TRANSPARENT_BLOCK.repeat(blocks_per_layer),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width / BLOCK_SIDE * format.block_bytes()),
                rows_per_image: None },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
    let staged = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None, contents: blocks, usage: wgpu::BufferUsages::COPY_SRC,
    });
    let texel_count = u64::from(width * height);
    let texels = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: texel_count * RGBA_BYTES as u64 * F32_BYTES, mapped_at_creation: false,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let region = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None, contents: bytemuck::cast_slice(&[width, CHECK_LAYER]), usage: wgpu::BufferUsages::UNIFORM,
    });
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("decoding check"), source: wgpu::ShaderSource::Wgsl(DECODE_CHECK_SHADER.into()),
    });
    let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("decoding check"), layout: None, module: &module, entry_point: Some("main"),
        compilation_options: Default::default(), cache: None,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
    });
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: texels.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: region.as_entire_binding() },
        ],
    });
    let readback = readback_buffer(gpu, texels.size());
    let mut commands = gpu.device.create_command_encoder(&Default::default());
    record_copy_to_layer(&mut commands, format, &staged, 0, sample.width, sample.height, &texture, CHECK_LAYER,
        CHECK_ORIGIN, CHECK_ORIGIN);
    {
        let mut pass = commands.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups((texel_count as u32).div_ceil(CHECK_THREADS), 1, 1);
    }
    commands.copy_buffer_to_buffer(&texels, 0, &readback, 0, texels.size());
    gpu.queue.submit([commands.finish()]);
    let linear: Vec<f32> = bytemuck::cast_slice(&read_back(gpu, &readback)).to_vec();
    let (mut inside, mut around) = (0u8, 0u8);
    for y in 0..height {
        for x in 0..width {
            let texel = &linear[(y * width + x) as usize * RGBA_BYTES..][..RGBA_BYTES];
            let gpu_bytes = [srgb_byte(texel[0]), srgb_byte(texel[1]), srgb_byte(texel[2]),
                (texel[ALPHA] * CHANNEL_MAX).round() as u8];
            let (fx, fy) = (x.wrapping_sub(CHECK_ORIGIN), y.wrapping_sub(CHECK_ORIGIN));
            if fx < sample.width && fy < sample.height {
                let cpu_texel = &cpu[(fy * sample.width + fx) as usize * RGBA_BYTES..][..RGBA_BYTES];
                // Transparent BC1 texels are black either way; the colour of others only matters with alpha.
                let channels = if cpu_texel[ALPHA] == 0 { ALPHA..RGBA_BYTES } else { 0..RGBA_BYTES };
                for channel in channels {
                    inside = inside.max(gpu_bytes[channel].abs_diff(cpu_texel[channel]));
                }
            } else if fx >= padded_width || fy >= padded_height {
                around = around.max(gpu_bytes[ALPHA]);
            }
        }
    }
    (inside, around)
}

/// The two first frames of a clip on a `side` square canvas, by their anchors, if they fit and fill more than half
/// of it.
fn canvas_pair(clip: &MobileClip, side: u32) -> Option<[Vec<u8>; 2]> {
    let frames = clip.frames.get(..2)?;
    let left = frames.iter().map(|frame| -frame.anchor_x).min()?;
    let top = frames.iter().map(|frame| -frame.anchor_y).min()?;
    let right = frames.iter().map(|frame| i32::from(frame.width) - frame.anchor_x).max()?;
    let bottom = frames.iter().map(|frame| i32::from(frame.height) - frame.anchor_y).max()?;
    let extent = (right - left).max(bottom - top);
    if extent > side as i32 || 2 * extent <= side as i32 {
        return None;
    }
    let (margin_x, margin_y) = ((side as i32 - (right - left)) / 2, (side as i32 - (bottom - top)) / 2);
    Some(std::array::from_fn(|index| {
        let frame = &frames[index];
        let mut canvas = vec![0u8; (side * side) as usize * RGBA_BYTES];
        let x0 = (margin_x - frame.anchor_x - left) as usize;
        let y0 = (margin_y - frame.anchor_y - top) as usize;
        let row = usize::from(frame.width) * RGBA_BYTES;
        for (y, source_row) in frame.rgba.chunks_exact(row).enumerate() {
            canvas[((y0 + y) * side as usize + x0) * RGBA_BYTES..][..row].copy_from_slice(source_row);
        }
        canvas
    }))
}

/// A RIFE in-between at t = 1/2 of two `side` square canvases, as rife/frames.wgsl stores it: straight alpha, a
/// Classic one's alpha snapped.
fn in_between(gpu: &Gpu, model: &RifeModel, side: u32, canvases: &[Vec<u8>; 2], classic: bool) -> Vec<u8> {
    let workspace = RifeWorkspace::new(model, &gpu.device, side, side).unwrap();
    workspace.set_timestep(&gpu.queue, IN_BETWEEN_SLOT, IN_BETWEEN_T);
    let plane = (side * side) as usize;
    let image_floats = RIFE_IMAGE_CHANNELS as usize * plane;
    let mut input = vec![0f32; 2 * image_floats];
    for (canvas, image) in canvases.iter().zip(input.chunks_exact_mut(image_floats)) {
        for (at, texel) in canvas.chunks_exact(RGBA_BYTES).enumerate() {
            let alpha = f32::from(texel[ALPHA]) / CHANNEL_MAX;
            for channel in 0..ALPHA {
                let premultiplied = f32::from(texel[channel]) / CHANNEL_MAX * alpha;
                image[channel * plane + at] = premultiplied + BACKDROP_GREY * (1.0 - alpha);
                image[(ALPHA + channel) * plane + at] = premultiplied;
            }
            image[RIFE_ALPHA_CHANNEL as usize * plane + at] = alpha;
        }
    }
    gpu.queue.write_buffer(workspace.input(), 0, bytemuck::cast_slice(&input));
    let readback = readback_buffer(gpu, image_floats as u64 * F32_BYTES);
    let mut commands = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = commands.begin_compute_pass(&Default::default());
        workspace.encode().iter().chain(workspace.interpolate(IN_BETWEEN_SLOT)).for_each(|dispatch| dispatch.record(&mut pass));
    }
    commands.copy_buffer_to_buffer(workspace.merged(), 0, &readback, 0, readback.size());
    gpu.queue.submit([commands.finish()]);
    let merged: Vec<f32> = bytemuck::cast_slice(&read_back(gpu, &readback)).to_vec();
    let to_byte = |value: f32| (value.clamp(0.0, 1.0) * CHANNEL_MAX).round() as u8;
    (0..plane).flat_map(|at| {
        let coverage = merged[RIFE_ALPHA_CHANNEL as usize * plane + at].clamp(0.0, 1.0);
        let alpha = if classic { if coverage >= CLASSIC_ALPHA_SNAP { 1.0 } else { 0.0 } } else { coverage };
        let color: [u8; 3] = std::array::from_fn(|channel| {
            if alpha > 0.0 { to_byte(merged[(ALPHA + channel) * plane + at] / coverage) } else { 0 }
        });
        [color[0], color[1], color[2], to_byte(alpha)]
    }).collect()
}

/// The page layout of a package, for uploading its BC7 pages' blocks as they are.
fn layout_line(name: &str, formats: Vec<String>, gutter: u16, packing: String, frames: impl Iterator<Item = (u16, u16)>)
    -> String {
    let (mut count, mut unaligned) = (0usize, 0usize);
    for (x, y) in frames {
        count += 1;
        unaligned += usize::from(u32::from(x) % BLOCK_SIDE != 0 || u32::from(y) % BLOCK_SIDE != 0);
    }
    let mut formats = formats;
    formats.sort();
    formats.dedup_by(|a, b| a == b);
    format!("{name}: page formats {formats:?}, gutter {gutter}, packing {packing}; {count} frames, {unaligned} not on \
        a block corner")
}

#[test]
#[ignore = "requires a GPU adapter and the animation packages in BLOCK_ENCODE_PACKAGES; run explicitly to evaluate \
            BC7 and BC1 for the frame atlas"]
fn evaluate_block_formats() {
    let Some(folder) = std::env::var_os(PACKAGES_VARIABLE).map(PathBuf::from) else {
        println!("skipped: {PACKAGES_VARIABLE} is not set");
        return;
    };
    let output = PathBuf::from(std::env::var_os(OUTPUT_VARIABLE)
        .unwrap_or_else(|| panic!("set {OUTPUT_VARIABLE} to the folder for the sheets and the summary")));
    std::fs::create_dir_all(&output).unwrap();
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let mut summary = String::new();
    writeln!(summary, "GPU timestamps: {}; BC textures: {}", gpu.timestamps, gpu.bc_textures).unwrap();

    let classic = MobileAnimCcPackage::load(folder.join(CLASSIC_PACKAGE)).unwrap();
    let enhanced = MobileAnimEcPackage::load(folder.join(ENHANCED_PACKAGE)).unwrap();
    writeln!(summary, "{}", layout_line("Classic", classic.pages().iter().map(|page| format!("{:?}", page.pixel_format))
        .collect(), classic.gutter(), format!("{:?}", classic.packing_mode()),
        classic.frames().iter().filter(|frame| frame.width > 0).map(|frame| (frame.x, frame.y)))).unwrap();
    writeln!(summary, "{}", layout_line("Enhanced", enhanced.pages().iter().map(|page| format!("{:?}", page.pixel_format))
        .collect(), enhanced.gutter(), format!("{:?}", enhanced.packing_mode()),
        enhanced.frames().iter().filter(|frame| frame.width > 0).map(|frame| (frame.x, frame.y)))).unwrap();

    let mut clips: Vec<(String, bool, MobileClip)> = Vec::new();
    for body in CLASSIC_BODIES {
        let action = classic.movement_action(body as u16, anim_cc::MOVE_WALK, false).expect("a Classic walk");
        let clip = classic.clip(body, action, DIRECTION).unwrap().expect("Classic frames");
        clips.push((format!("Classic body {body} walk"), true, clip));
    }
    for body in ENHANCED_BODIES {
        let action = enhanced.move_action(body, anim_ec::GROUP_PEACE, anim_ec::MOVE_WALK).expect("an Enhanced walk");
        let clip = enhanced.clip(body, action, DIRECTION).unwrap().expect("Enhanced frames");
        clips.push((format!("Enhanced body {body} walk"), false, clip));
    }
    let mut samples = Vec::new();
    for (name, classic, clip) in &clips {
        for (index, frame) in clip.frames.iter().take(FRAMES_PER_CLIP).enumerate() {
            samples.push(Sample {
                name: format!("{name} frame {index}"),
                group: if *classic { "Classic frames" } else { "Enhanced frames" }.to_string(),
                classic: *classic, width: u32::from(frame.width), height: u32::from(frame.height),
                rgba: frame.rgba.clone(), sheet: index == 0,
            });
        }
    }
    for (name, classic, clip) in &clips {
        let transparent = clip.frames.iter().flat_map(|frame| frame.rgba.chunks_exact(RGBA_BYTES))
            .filter(|texel| texel[ALPHA] == 0).count();
        let coloured = clip.frames.iter().flat_map(|frame| frame.rgba.chunks_exact(RGBA_BYTES))
            .filter(|texel| texel[ALPHA] == 0 && texel[..ALPHA].iter().any(|&c| c != 0)).count();
        let soft = clip.frames.iter().flat_map(|frame| frame.rgba.chunks_exact(RGBA_BYTES))
            .filter(|texel| texel[ALPHA] != 0 && texel[ALPHA] != u8::MAX).count();
        let sizes: Vec<String> = clip.frames.iter().take(FRAMES_PER_CLIP)
            .map(|frame| format!("{}x{}", frame.width, frame.height)).collect();
        writeln!(summary, "{name} (classic {classic}): {} frames, sizes {sizes:?}; transparent texels {transparent}, \
            {coloured} of them with colour; soft alpha texels {soft}", clip.frames.len()).unwrap();
    }

    // The in-betweens: for each size, the first clip whose two first frames fill more than half of the canvas.
    let model = RifeModel::new(&gpu.device).unwrap();
    for side in IN_BETWEEN_SIZES {
        let Some((name, classic, canvases)) = clips.iter()
            .find_map(|(name, classic, clip)| canvas_pair(clip, side).map(|canvases| (name, *classic, canvases))) else {
            writeln!(summary, "no clip fits a {side}x{side} in-between").unwrap();
            continue;
        };
        samples.push(Sample {
            name: format!("in-between {side}x{side} {name}"), group: format!("in-betweens {side}x{side}"), classic,
            width: side, height: side, rgba: in_between(&gpu, &model, side, &canvases, classic), sheet: true,
        });
    }

    let started = Instant::now();
    let mut encoder = MobileBlockEncoder::new(&gpu.device, &gpu.queue);
    writeln!(summary, "encoder pipelines compiled in {:.0} ms", started.elapsed().as_secs_f64() * 1e3).unwrap();
    let sources: Vec<Source> = samples.iter().map(|sample| upload(&gpu, sample)).collect();
    let unbounded = std::env::var_os(UNBOUNDED_BC7_VARIABLE)
        .map(|path| UnboundedBc7::new(&gpu, &std::fs::read_to_string(path).unwrap()));
    let formats: Vec<MobileBlockFormat> = Bc7Quality::ALL.iter().map(|&quality| MobileBlockFormat::Bc7(quality))
        .chain(OTHER_FORMATS.iter().copied()).collect();
    let mut groups: Vec<String> = samples.iter().map(|sample| sample.group.clone()).collect();
    groups.dedup();

    writeln!(summary, "\n| group | format | frames | mean px | GPU ms/frame alone (median of frames) | GPU ms/frame in \
        one pass | RGB PSNR dB (alpha > 0) | RGB max | shown PSNR dB (premultiplied) | shown max | alpha PSNR dB | \
        alpha max | soft alpha | halo (max alpha) | holes | KiB (RGBA8 KiB) |").unwrap();
    writeln!(summary, "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|").unwrap();
    let mut decoded_by_sample: Vec<Vec<(MobileBlockFormat, Vec<u8>)>> = vec![Vec::new(); samples.len()];
    let mut checks = Vec::new();
    for group in &groups {
        let members: Vec<usize> = (0..samples.len()).filter(|&index| &samples[index].group == group).collect();
        for &format in &formats {
            let chosen: Vec<usize> = members.iter().copied().filter(|&index| takes(format, &samples[index])).collect();
            if chosen.is_empty() {
                continue;
            }
            let group_samples: Vec<&Sample> = chosen.iter().map(|&index| &samples[index]).collect();
            let group_sources: Vec<&Source> = chosen.iter().map(|&index| &sources[index]).collect();
            let encoded = encode(&gpu, &mut encoder, format, &group_samples, &group_sources);
            let mut errors = Errors::default();
            let (mut bytes, mut rgba_bytes, mut texels) = (0u64, 0u64, 0u64);
            for (position, &index) in chosen.iter().enumerate() {
                let sample = &samples[index];
                let decoded = decode(format, sample, &encoded.blocks[position]);
                errors.add(&sample.rgba, &decoded);
                bytes += format.encoded_bytes(sample.width, sample.height);
                texels += u64::from(sample.width * sample.height);
                rgba_bytes += u64::from(sample.width * sample.height) * RGBA_BYTES as u64;
                if gpu.bc_textures && position == 0 {
                    let (inside, around) = check_gpu_decoding(&gpu, format, sample, &encoded.blocks[position], &decoded);
                    checks.push(format!("{} {}: GPU decoding against the CPU's, max {inside}; alpha around the frame, \
                        max {around}", format_name(format), sample.name));
                }
                decoded_by_sample[index].push((format, decoded));
            }
            let mut alone = encoded.alone_ms.clone();
            let row = format!("| {group} | {} | {} | {} | {:.3} | {:.3} | {} | {:.1} ({:.1}) |", format_name(format),
                chosen.len(), texels / chosen.len() as u64, median(&mut alone), encoded.together_ms / chosen.len() as f64,
                errors.columns(), bytes as f64 / KIB, rgba_bytes as f64 / KIB);
            println!("{row}");
            writeln!(summary, "{row}").unwrap();
        }
        if let Some(unbounded) = unbounded.as_ref().filter(|_| gpu.timestamps) {
            for quality in Bc7Quality::ALL {
                let format = MobileBlockFormat::Bc7(quality);
                let (mut times, mut differing, mut texels) = (Vec::new(), 0usize, 0usize);
                let mut errors = Errors::default();
                for &index in &members {
                    let (encoded, ms) = unbounded.encode(&gpu, quality, &samples[index], &sources[index]);
                    times.push(ms);
                    let bounded = &decoded_by_sample[index].iter().find(|(f, _)| *f == format).unwrap().1;
                    let decoded = decode(format, &samples[index], &encoded);
                    errors.add(&samples[index].rgba, &decoded);
                    differing += decoded.chunks_exact(RGBA_BYTES).zip(bounded.chunks_exact(RGBA_BYTES))
                        .filter(|(a, b)| a != b).count();
                    texels += decoded.len() / RGBA_BYTES;
                }
                let row = format!("| {group} | {} without loop bounding ({differing} of {texels} texels differ from \
                    bounded) | {} | | {:.3} | | {} | |", format_name(format), members.len(), median(&mut times),
                    errors.columns());
                println!("{row}");
                writeln!(summary, "{row}").unwrap();
            }
        }
        // The CPU encoder of the packages (uot-texture-codecs), for what BC7 can do on the same frames.
        let mut errors = Errors::default();
        let mut times = Vec::new();
        for &index in &members {
            let sample = &samples[index];
            let (width, height) = padded_size(sample.width, sample.height);
            let mut padded = vec![0u8; (width * height) as usize * RGBA_BYTES];
            let row = sample.width as usize * RGBA_BYTES;
            for (y, source_row) in sample.rgba.chunks_exact(row).enumerate() {
                padded[y * width as usize * RGBA_BYTES..][..row].copy_from_slice(source_row);
            }
            let started = Instant::now();
            let encoded = uot_texture_codecs::bc7::encode_to_bc7(&padded, ImageExtent::new(width, height).unwrap(),
                RawImageFormat::Rgba8888, Bc7EncoderBackend::Analytical).unwrap();
            times.push(started.elapsed().as_secs_f64() * 1e3);
            errors.add(&sample.rgba, &decode(MobileBlockFormat::Bc7(Bc7Quality::Slow), sample, encoded.blocks()));
        }
        let row = format!("| {group} | CPU BC7 Analytical (reference; wall ms) | {} | | {:.3} | | {} | |", members.len(),
            median(&mut times), errors.columns());
        println!("{row}");
        writeln!(summary, "{row}").unwrap();
    }
    writeln!(summary).unwrap();
    for check in &checks {
        writeln!(summary, "{check}").unwrap();
    }

    writeln!(summary, "\nSheets (columns: source, then {:?} where the frame takes them; below each, the difference \
        over grey x{DIFF_GAIN}):", SHEET_FORMATS.map(format_name)).unwrap();
    for (sample, decoded) in samples.iter().zip(&decoded_by_sample) {
        if !sample.sheet {
            continue;
        }
        let shown: Vec<Vec<u8>> = SHEET_FORMATS.iter()
            .filter_map(|format| decoded.iter().find(|(f, _)| f == format).map(|(_, rgba)| rgba.clone())).collect();
        let path = output.join(format!("{}.png", file_name(&sample.name)));
        write_sheet(&path, sample, &shown);
        writeln!(summary, "{}", path.display()).unwrap();
    }
    println!("{summary}");
    std::fs::write(output.join(SUMMARY_FILE), summary).unwrap();
}

/// A frame encodes to the same blocks at a row of its texture below other frames (`add_frame_at`, as Dynamapper's
/// in-betweens wait in their scratch texture) as at the corner of its own, both in one record.
#[test]
#[ignore = "requires a GPU adapter; run explicitly for block encoder changes"]
fn a_frame_below_others_encodes_as_at_its_own_corner() {
    const SIDE: u32 = 24;
    const ROW: u32 = 12;
    const FORMAT: MobileBlockFormat = MobileBlockFormat::Bc7(Bc7Quality::UltraFast);
    let Some(gpu) = gpu() else {
        println!("skipped: no GPU adapter");
        return;
    };
    let texel = |x: u32, y: u32| {
        let alpha = if (x + y) % 3 == 0 { 0 } else { u8::MAX };
        [(x * 10) as u8, (y * 10) as u8, ((x + y) * 5) as u8, alpha]
    };
    let rgba = (0..SIDE).flat_map(|y| (0..SIDE).flat_map(move |x| texel(x, y))).collect::<Vec<_>>();
    // Another frame above it, then the frame at row ROW.
    let mut stacked = rgba.iter().map(|byte| byte.wrapping_mul(7)).take((ROW * SIDE) as usize * RGBA_BYTES)
        .collect::<Vec<_>>();
    stacked.extend_from_slice(&rgba);
    let sample = |rgba: Vec<u8>, height| Sample { name: String::new(), group: String::new(), classic: false,
        width: SIDE, height, rgba, sheet: false };
    let (frame, stacked) = (sample(rgba, SIDE), sample(stacked, ROW + SIDE));
    let (own, below) = (upload(&gpu, &frame), upload(&gpu, &stacked));
    let bytes = FORMAT.encoded_bytes(SIDE, SIDE);
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 2 * bytes, mapped_at_creation: false,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let mut encoder = MobileBlockEncoder::new(&gpu.device, &gpu.queue);
    let mut commands = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = commands.begin_compute_pass(&Default::default());
        encoder.add_frame(FORMAT, &own.view, SIDE, SIDE, &output, 0);
        encoder.add_frame_at(FORMAT, &below.view, ROW, SIDE, SIDE, &output, bytes);
        encoder.record(&mut pass);
    }
    let readback = readback_buffer(&gpu, 2 * bytes);
    commands.copy_buffer_to_buffer(&output, 0, &readback, 0, 2 * bytes);
    gpu.queue.submit([commands.finish()]);
    let blocks = read_back(&gpu, &readback);
    let (at_corner, at_row) = blocks.split_at(bytes as usize);
    assert!(at_corner.iter().any(|&byte| byte != 0));
    assert_eq!(at_corner, at_row);
}
