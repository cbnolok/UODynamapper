//! The SPAN 4x super-resolution network (Wan et al., "Swift Parameter-free Attention Network for Efficient
//! Super-Resolution", CVPR 2024 NTIRE workshop) with the 4xmssim_span_pretrain weights (Philip Hofmann, CC-BY-4.0), as
//! wgpu compute shaders: it turns an Enhanced/KR mobile frame of the atlas into a 2x frame (the 4x result averaged
//! 2x2), ready to copy into an atlas. Weights are fp16 on the GPU and the arithmetic is f32; every tensor stays on the
//! GPU, so a caller reads nothing back.
//!
//! [`UpscaleModel`] holds the weights and pipelines, once per device. [`UpscaleWorkspace`] holds the buffers of one
//! canvas size and the dispatches that run on them, as lists a scheduler can spread over several frames: the input
//! ([`UpscaleWorkspace::prepare`]), the 22 convolutions ([`UpscaleWorkspace::network`]) and the output
//! ([`UpscaleWorkspace::finish`]). Dispatches of one frame must run in order, and one frame must end (its output
//! copied away) before the next starts on the same workspace, since they share the buffers; each frame slot has its
//! own source rectangle, so several frames can be recorded in one submission.
//!
//! The network sees colour premultiplied over 50% grey; alpha does not go through it but is upscaled bicubically and
//! takes the grey matte off the network's colour (see shaders/upscale/pixel.wgsl). The frame sits on a canvas with a
//! transparent border of [`UPSCALE_BORDER`] texels, so outlines and the bicubic alpha can grow past the frame's edge:
//! the output covers the whole canvas at 2x, the frame's top left texel starting at `2 * UPSCALE_BORDER`.

mod weights;
#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use weights::{PackedParams, WEIGHTS_FILE};
use super::frame_atlas::AtlasRect;
use super::network_shader_module;

/// The transparent texels around the frame on the canvas: Catmull-Rom alpha reaches 1.5 texels past the last opaque
/// one.
pub const UPSCALE_BORDER: u32 = 2;
/// The output's scale: the network's 4x, averaged 2x2.
pub const UPSCALE_OUTPUT_SCALE: u32 = NETWORK_SCALE / BOX_SIDE;
/// Frame slots, each with its own source rectangle, so several frames can be recorded in one submission.
pub const UPSCALE_FRAME_SLOTS: usize = 4;
/// Bytes of one output texel (RGBA8).
pub const UPSCALE_OUTPUT_TEXEL_BYTES: u32 = 4;

const NETWORK_SCALE: u32 = 4;
const BOX_SIDE: u32 = 2;
const FEATURES: u32 = 48;
const COLOR_CHANNELS: u32 = 3;
const SPAB_BLOCKS: u32 = 6;
const CONV_SIDE: u32 = 3;
const POINTWISE_SIDE: u32 = 1;
/// The upsampler's convolution: each colour channel's 4x4 sub-pixels, pixel-shuffled by `finish`.
const UPSAMPLER_CHANNELS: u32 = COLOR_CHANNELS * NETWORK_SCALE * NETWORK_SCALE;
// Every layer writes FEATURES channels, the upsampler too.
const _: () = assert!(UPSAMPLER_CHANNELS == FEATURES);

/// The arena's channel planes, each a canvas of f32. The four tensors conv_cat concatenates sit together from
/// `CONCAT`: conv_1's features, conv_2's output, block 1's output and block 6's first convolution (after its SiLU,
/// which PyTorch applies in place).
const INPUT_PLANE: u32 = 0;
const ALPHA_PLANE: u32 = COLOR_CHANNELS;
const INPUT_PLANES: u32 = COLOR_CHANNELS + 1;
const CONCAT_PLANE: u32 = INPUT_PLANES;
const FEATURE_PLANE: u32 = CONCAT_PLANE;
const CONV_2_PLANE: u32 = CONCAT_PLANE + FEATURES;
const BLOCK_1_PLANE: u32 = CONCAT_PLANE + 2 * FEATURES;
const BLOCK_6_FIRST_PLANE: u32 = CONCAT_PLANE + 3 * FEATURES;
const CONCAT_CHANNELS: u32 = 4 * FEATURES;
/// The running block output, overwritten in place by blocks 3 to 6, then two temporaries.
const BLOCK_PLANE: u32 = CONCAT_PLANE + CONCAT_CHANNELS;
const TEMP_A_PLANE: u32 = BLOCK_PLANE + FEATURES;
const TEMP_B_PLANE: u32 = TEMP_A_PLANE + FEATURES;
const ARENA_PLANES: u32 = TEMP_B_PLANE + FEATURES;
/// The upsampler writes where the second temporary was.
const NETWORK_OUTPUT_PLANE: u32 = TEMP_B_PLANE;

const CONV_THREADS: u32 = 64;
/// Matches THREADS in pixel.wgsl.
const PIXEL_THREADS: u32 = 64;
const F32_BYTES: u64 = 4;
const CONV_SHADER: &str = include_str!("../../../assets/shaders/upscale/conv.wgsl");
const PIXEL_SHADER: &str = include_str!("../../../assets/shaders/upscale/pixel.wgsl");

/// A convolution workgroup's tile: `m` pixels by `n` output channels, `k` rows of the sum staged at a time, each
/// thread keeping `micro_m` by `micro_n` sums. Every layer has 48 output channels, one tile wide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tile {
    m: u32,
    n: u32,
    k: u32,
    micro_m: u32,
    micro_n: u32,
}

const WIDE_TILE: Tile = Tile { m: 64, n: 48, k: 16, micro_m: 4, micro_n: 12 };
/// Small canvases: twice the workgroups of the wide tile.
const NARROW_TILE: Tile = Tile { m: 32, n: 48, k: 16, micro_m: 2, micro_n: 12 };
const TILES: [Tile; 2] = [WIDE_TILE, NARROW_TILE];
/// A canvas whose wide tiles make fewer workgroups than this takes the narrow tile (measured on a Radeon 880M:
/// a 48x40 frame 7% faster).
const NARROW_BELOW_WORKGROUPS: u32 = 96;

/// What follows the sum and bias (values match conv.wgsl).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Epilogue {
    Linear = 0,
    Silu = 1,
    /// SPAB's attention, with the block's input as the residual.
    Attention = 2,
}

/// The convolution variants the network uses: kernel side and epilogue.
const CONV_VARIANTS: [(u32, Epilogue); 4] = [
    (CONV_SIDE, Epilogue::Linear),
    (CONV_SIDE, Epilogue::Silu),
    (CONV_SIDE, Epilogue::Attention),
    (POINTWISE_SIDE, Epilogue::Linear),
];

/// `Conv` in conv.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ConvUniform {
    width: u32,
    height: u32,
    pixels: u32,
    in_channels: u32,
    in_base: u32,
    out_channels: u32,
    out_base: u32,
    residual_base: u32,
    weights: u32,
    bias: u32,
    _pad: [u32; 2],
}

/// `Pixel` in pixel.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PixelUniform {
    width: u32,
    height: u32,
    pixels: u32,
    layer: u32,
    source_x: u32,
    source_y: u32,
    frame_width: u32,
    frame_height: u32,
    border: u32,
    alpha_base: u32,
    network_base: u32,
    row_texels: u32,
}

/// The weights and pipelines, once per device.
pub struct UpscaleModel {
    params: wgpu::Buffer,
    packed: PackedParams,
    conv_layout: wgpu::BindGroupLayout,
    prepare_layout: wgpu::BindGroupLayout,
    finish_layout: wgpu::BindGroupLayout,
    conv_pipelines: Vec<((u32, Epilogue, Tile), wgpu::ComputePipeline)>,
    prepare_pipeline: wgpu::ComputePipeline,
    finish_pipeline: wgpu::ComputePipeline,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None,
        },
    }
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0, visibility: wgpu::ShaderStages::COMPUTE, count: None,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
    }
}

/// Binding 1 of pixel.wgsl: the atlas, read with textureLoad.
fn source_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 1, visibility: wgpu::ShaderStages::COMPUTE, count: None,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
        },
    }
}

/// Compiles conv.wgsl or pixel.wgsl without naga's loop bounding ([`network_shader_module`]).
fn shader_module(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    // SAFETY: every loop in conv.wgsl and pixel.wgsl ends: each runs to a compile-time constant or to a uniform's
    // value, counting up by a positive step.
    unsafe { network_shader_module(device, label, source) }
}

impl UpscaleModel {
    /// Uploads the bundled weights and compiles the pipelines.
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        let packed = PackedParams::parse(WEIGHTS_FILE)?;
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("upscale weights"), contents: bytemuck::cast_slice(&packed.halves), usage: wgpu::BufferUsages::STORAGE,
        });
        let conv_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale convolution"),
            entries: &[uniform_entry(), storage_entry(1, true), storage_entry(2, false)],
        });
        let prepare_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale prepare"), entries: &[uniform_entry(), source_entry(), storage_entry(2, false)],
        });
        let finish_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale finish"), entries: &[uniform_entry(), storage_entry(2, false), storage_entry(3, false)],
        });
        let pipeline_layout = |layout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(layout)], immediate_size: 0,
        });
        let conv_pipeline_layout = pipeline_layout(&conv_layout);
        let mut conv_pipelines = Vec::new();
        for (side, epilogue) in CONV_VARIANTS {
            for tile in TILES {
                let constants = format!(
                    "const THREADS: u32 = {CONV_THREADS}u;\nconst TILE_M: u32 = {}u;\nconst TILE_N: u32 = {}u;\n\
                     const TILE_K: u32 = {}u;\nconst MICRO_M: u32 = {}u;\nconst MICRO_N: u32 = {}u;\n\
                     const KERNEL_SIDE: u32 = {side}u;\nconst EPILOGUE: u32 = {}u;\n",
                    tile.m, tile.n, tile.k, tile.micro_m, tile.micro_n, epilogue as u32);
                let module = shader_module(device, "upscale convolution", constants + CONV_SHADER);
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("upscale convolution"), layout: Some(&conv_pipeline_layout), module: &module,
                    entry_point: Some("main"), compilation_options: Default::default(), cache: None,
                });
                conv_pipelines.push(((side, epilogue, tile), pipeline));
            }
        }
        let pixel_module = shader_module(device, "upscale pixel steps", PIXEL_SHADER.to_string());
        let pixel_pipeline = |layout, entry_point| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry_point), layout: Some(&pipeline_layout(layout)), module: &pixel_module,
            entry_point: Some(entry_point), compilation_options: Default::default(), cache: None,
        });
        let prepare_pipeline = pixel_pipeline(&prepare_layout, "prepare");
        let finish_pipeline = pixel_pipeline(&finish_layout, "finish");
        Ok(Self {
            params, packed, conv_layout, prepare_layout, finish_layout, conv_pipelines, prepare_pipeline, finish_pipeline,
        })
    }

    /// Bytes of the weights on the GPU.
    pub fn weight_bytes(&self) -> u64 {
        self.params.size()
    }

    fn conv_pipeline(&self, side: u32, epilogue: Epilogue, tile: Tile) -> &wgpu::ComputePipeline {
        &self.conv_pipelines.iter().find(|(key, _)| *key == (side, epilogue, tile)).unwrap().1
    }
}

/// One compute dispatch, ready to record into a compute pass.
#[derive(Clone)]
pub struct UpscaleDispatch {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    workgroups: [u32; 3],
    multiply_adds: u64,
    name: String,
}

impl UpscaleDispatch {
    pub fn record(&self, pass: &mut wgpu::ComputePass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(self.workgroups[0], self.workgroups[1], self.workgroups[2]);
    }

    /// The convolution's multiply-adds (zero for the per-pixel steps), as a measure of its cost.
    pub fn multiply_adds(&self) -> u64 {
        self.multiply_adds
    }

    pub fn workgroups(&self) -> [u32; 3] {
        self.workgroups
    }

    /// The layer, named as its tensors are in the weights file where it is one.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A convolution layer of the plan: its tensors' name, and the arena planes it reads and writes.
struct ConvLayer {
    /// The weight and bias are `{name}.weight` and `{name}.bias`.
    name: String,
    side: u32,
    epilogue: Epilogue,
    input: u32,
    in_channels: u32,
    output: u32,
    /// The attention's residual.
    residual: u32,
}

/// A dispatch of the plan, before the uniform buffer and bind groups exist.
struct PendingDispatch {
    pipeline: wgpu::ComputePipeline,
    uniform: Vec<u8>,
    workgroups: [u32; 3],
    multiply_adds: u64,
    name: String,
}

/// Collects the convolutions' dispatches.
struct Planner<'a> {
    model: &'a UpscaleModel,
    width: u32,
    height: u32,
    pending: Vec<PendingDispatch>,
}

impl Planner<'_> {
    /// Plans a layer's dispatch.
    fn conv(&mut self, layer: ConvLayer) -> Result<(), String> {
        let pixels = self.width * self.height;
        let tile = if pixels.div_ceil(WIDE_TILE.m) >= NARROW_BELOW_WORKGROUPS { WIDE_TILE } else { NARROW_TILE };
        let packed = &self.model.packed;
        let uniform = ConvUniform {
            width: self.width, height: self.height, pixels, in_channels: layer.in_channels,
            in_base: layer.input * pixels, out_channels: FEATURES, out_base: layer.output * pixels,
            residual_base: layer.residual * pixels,
            weights: packed.offset(&format!("{}.weight", layer.name))?,
            bias: packed.offset(&format!("{}.bias", layer.name))?,
            _pad: [0; 2],
        };
        self.pending.push(PendingDispatch {
            pipeline: self.model.conv_pipeline(layer.side, layer.epilogue, tile).clone(),
            uniform: bytemuck::bytes_of(&uniform).to_vec(),
            workgroups: [pixels.div_ceil(tile.m), FEATURES.div_ceil(tile.n), 1],
            multiply_adds: (pixels * FEATURES) as u64 * (layer.in_channels * layer.side * layer.side) as u64,
            name: layer.name,
        });
        Ok(())
    }
}

/// The output's row pitch for a canvas `width` wide, aligned for `copy_buffer_to_texture`.
fn output_bytes_per_row(width: u32) -> u32 {
    (width * UPSCALE_OUTPUT_SCALE * UPSCALE_OUTPUT_TEXEL_BYTES).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

/// The per-pixel steps' parameters for a canvas and a frame on it.
fn pixel_uniform(width: u32, height: u32, frame: AtlasRect) -> PixelUniform {
    let pixels = width * height;
    PixelUniform {
        width, height, pixels, layer: frame.layer, source_x: frame.x, source_y: frame.y, frame_width: frame.width,
        frame_height: frame.height, border: UPSCALE_BORDER, alpha_base: ALPHA_PLANE * pixels,
        network_base: NETWORK_OUTPUT_PLANE * pixels, row_texels: output_bytes_per_row(width) / UPSCALE_OUTPUT_TEXEL_BYTES,
    }
}

/// The canvas a frame needs: the frame and its transparent border on each side.
pub fn upscale_canvas_size(frame_width: u32, frame_height: u32) -> (u32, u32) {
    (frame_width + 2 * UPSCALE_BORDER, frame_height + 2 * UPSCALE_BORDER)
}

/// The buffers of one canvas size and the dispatches that upscale on them.
pub struct UpscaleWorkspace {
    width: u32,
    height: u32,
    /// The canvas's colour over grey and alpha, then every activation, as f32 channel planes of the canvas.
    arena: wgpu::Buffer,
    /// The 2x canvas, RGBA8 rows [`UpscaleWorkspace::output_bytes_per_row`] apart.
    output: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    /// Where each slot's source rectangle sits in the uniform buffer.
    slot_offsets: Vec<u64>,
    prepare: Vec<UpscaleDispatch>,
    network: Vec<UpscaleDispatch>,
    finish: Vec<UpscaleDispatch>,
}

impl UpscaleWorkspace {
    /// Allocates the buffers for a canvas of this size and plans the dispatches; `source` is the atlas the frames
    /// come from (an RGBA8 sRGB 2D array texture holding straight alpha), viewed as a 2D array. A frame fits if
    /// [`upscale_canvas_size`] of it does; frames of different sizes can share a canvas.
    pub fn new(model: &UpscaleModel, device: &wgpu::Device, source: &wgpu::TextureView, width: u32, height: u32)
        -> Result<Self, String> {
        let pixels = width * height;
        let mut planner = Planner { model, width, height, pending: Vec::new() };
        planner.conv(ConvLayer {
            name: "conv_1.eval_conv".into(), side: CONV_SIDE, epilogue: Epilogue::Linear,
            input: INPUT_PLANE, in_channels: COLOR_CHANNELS, output: FEATURE_PLANE, residual: 0,
        })?;
        // Each SPAB: c1_r and c2_r with SiLU into the temporaries, c3_r with the attention into the block's output.
        // Block 1 keeps its input (conv_1's features) and its output for conv_cat; block 2 starts the running output,
        // which blocks 3 to 6 overwrite in place; block 6's c1_r goes where conv_cat reads it.
        for block in 1..=SPAB_BLOCKS {
            let input = match block {
                1 => FEATURE_PLANE,
                2 => BLOCK_1_PLANE,
                _ => BLOCK_PLANE,
            };
            let output = if block == 1 { BLOCK_1_PLANE } else { BLOCK_PLANE };
            let first = if block == SPAB_BLOCKS { BLOCK_6_FIRST_PLANE } else { TEMP_A_PLANE };
            let second = if block == SPAB_BLOCKS { TEMP_A_PLANE } else { TEMP_B_PLANE };
            let layers = [("c1_r", input, first, Epilogue::Silu), ("c2_r", first, second, Epilogue::Silu),
                ("c3_r", second, output, Epilogue::Attention)];
            for (conv, from, to, epilogue) in layers {
                planner.conv(ConvLayer {
                    name: format!("block_{block}.{conv}.eval_conv"), side: CONV_SIDE, epilogue,
                    input: from, in_channels: FEATURES, output: to, residual: input,
                })?;
            }
        }
        planner.conv(ConvLayer {
            name: "conv_2.eval_conv".into(), side: CONV_SIDE, epilogue: Epilogue::Linear,
            input: BLOCK_PLANE, in_channels: FEATURES, output: CONV_2_PLANE, residual: 0,
        })?;
        planner.conv(ConvLayer {
            name: "conv_cat".into(), side: POINTWISE_SIDE, epilogue: Epilogue::Linear,
            input: CONCAT_PLANE, in_channels: CONCAT_CHANNELS, output: TEMP_A_PLANE, residual: 0,
        })?;
        planner.conv(ConvLayer {
            name: "upsampler.0".into(), side: CONV_SIDE, epilogue: Epilogue::Linear,
            input: TEMP_A_PLANE, in_channels: FEATURES, output: NETWORK_OUTPUT_PLANE, residual: 0,
        })?;
        let network_count = planner.pending.len();

        let (out_width, out_height) = (width * UPSCALE_OUTPUT_SCALE, height * UPSCALE_OUTPUT_SCALE);
        let pixel_uniform = pixel_uniform(width, height, AtlasRect { layer: 0, x: 0, y: 0, width: 0, height: 0 });
        let mut pending = planner.pending;
        for _ in 0..UPSCALE_FRAME_SLOTS {
            pending.push(PendingDispatch {
                pipeline: model.prepare_pipeline.clone(), uniform: bytemuck::bytes_of(&pixel_uniform).to_vec(),
                workgroups: [pixels.div_ceil(PIXEL_THREADS), 1, 1], multiply_adds: 0, name: "prepare".into(),
            });
        }
        pending.push(PendingDispatch {
            pipeline: model.finish_pipeline.clone(), uniform: bytemuck::bytes_of(&pixel_uniform).to_vec(),
            workgroups: [(out_width * out_height).div_ceil(PIXEL_THREADS), 1, 1], multiply_adds: 0,
            name: "finish".into(),
        });

        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let buffer = |label, bytes: u64| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size: bytes, mapped_at_creation: false, usage: storage,
        });
        let arena = buffer("upscale arena", (ARENA_PLANES * pixels) as u64 * F32_BYTES);
        let output = buffer("upscale output", (output_bytes_per_row(width) * out_height) as u64);

        let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
        let offsets: Vec<usize> = pending.iter().scan(0, |next, dispatch| {
            let offset = *next;
            *next = (offset + dispatch.uniform.len()).div_ceil(alignment) * alignment;
            Some(offset)
        }).collect();
        let mut uniform_bytes = vec![0u8; offsets.last().unwrap() + pending.last().unwrap().uniform.len()];
        for (dispatch, &offset) in pending.iter().zip(&offsets) {
            uniform_bytes[offset..offset + dispatch.uniform.len()].copy_from_slice(&dispatch.uniform);
        }
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("upscale dispatch parameters"), contents: &uniform_bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let mut dispatches: Vec<UpscaleDispatch> = pending.into_iter().zip(&offsets).enumerate()
            .map(|(index, (dispatch, &offset))| {
                let uniform = wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniforms, offset: offset as u64,
                        size: wgpu::BufferSize::new(dispatch.uniform.len() as u64),
                    }),
                };
                let bind_group = if index < network_count {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("upscale convolution"), layout: &model.conv_layout,
                        entries: &[uniform, entry(1, &model.params), entry(2, &arena)],
                    })
                } else if index < network_count + UPSCALE_FRAME_SLOTS {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("upscale prepare"), layout: &model.prepare_layout,
                        entries: &[uniform, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(source) },
                            entry(2, &arena)],
                    })
                } else {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("upscale finish"), layout: &model.finish_layout,
                        entries: &[uniform, entry(2, &arena), entry(3, &output)],
                    })
                };
                UpscaleDispatch {
                    pipeline: dispatch.pipeline, bind_group, workgroups: dispatch.workgroups,
                    multiply_adds: dispatch.multiply_adds, name: dispatch.name,
                }
            }).collect();
        let finish = dispatches.split_off(network_count + UPSCALE_FRAME_SLOTS);
        let prepare = dispatches.split_off(network_count);
        let slot_offsets = offsets[network_count..network_count + UPSCALE_FRAME_SLOTS].iter()
            .map(|&offset| offset as u64).collect();
        Ok(Self { width, height, arena, output, uniforms, slot_offsets, prepare, network: dispatches, finish })
    }

    /// The canvas size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Sets the frame a slot's [`UpscaleWorkspace::prepare`] reads from the atlas; it takes effect with the next
    /// queue submission. The frame's top left lands at ([`UPSCALE_BORDER`], [`UPSCALE_BORDER`]) on the canvas.
    pub fn set_frame(&self, queue: &wgpu::Queue, slot: usize, frame: AtlasRect) -> Result<(), String> {
        let (needed_width, needed_height) = upscale_canvas_size(frame.width, frame.height);
        if needed_width > self.width || needed_height > self.height {
            return Err(format!("upscale: a {}x{} frame needs a {needed_width}x{needed_height} canvas, this one is {}x{}",
                frame.width, frame.height, self.width, self.height));
        }
        let uniform = pixel_uniform(self.width, self.height, frame);
        queue.write_buffer(&self.uniforms, self.slot_offsets[slot], bytemuck::bytes_of(&uniform));
        Ok(())
    }

    /// The slot's frame onto the canvas, transparent around it, as the network's input.
    pub fn prepare(&self, slot: usize) -> &[UpscaleDispatch] {
        std::slice::from_ref(&self.prepare[slot])
    }

    /// The 22 convolutions, conv_1 to the upsampler.
    pub fn network(&self) -> &[UpscaleDispatch] {
        &self.network
    }

    /// The 2x canvas into [`UpscaleWorkspace::output`].
    pub fn finish(&self) -> &[UpscaleDispatch] {
        &self.finish
    }

    /// Everything for one frame of a slot, in order.
    pub fn upscale(&self, slot: usize) -> impl Iterator<Item = &UpscaleDispatch> {
        self.prepare(slot).iter().chain(&self.network).chain(&self.finish)
    }

    /// The 2x canvas: RGBA8 texels, sRGB colour and straight alpha like the atlas, rows
    /// [`UpscaleWorkspace::output_bytes_per_row`] apart (aligned for `copy_buffer_to_texture`).
    pub fn output(&self) -> &wgpu::Buffer {
        &self.output
    }

    pub fn output_bytes_per_row(&self) -> u32 {
        output_bytes_per_row(self.width)
    }

    /// The output's size in texels, the canvas at 2x.
    pub fn output_size(&self) -> (u32, u32) {
        (self.width * UPSCALE_OUTPUT_SCALE, self.height * UPSCALE_OUTPUT_SCALE)
    }

    /// Bytes of the workspace's buffers on the GPU, not counting the model's weights.
    pub fn working_bytes(&self) -> u64 {
        [&self.arena, &self.output, &self.uniforms].iter().map(|buffer| buffer.size()).sum()
    }
}
