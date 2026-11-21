//! The RIFE v4.25 frame interpolation network (Practical-RIFE, Huang et al., ECCV 2022) as wgpu compute shaders: it
//! makes an in-between of two frames at a time t between them. Weights are fp16 on the GPU and the arithmetic is f32;
//! every tensor stays on the GPU, so a caller reads nothing back.
//!
//! [`RifeModel`] holds the weights and pipelines, once per device. [`RifeWorkspace`] holds the buffers of one frame
//! size and the dispatches that run the network on them, as lists a scheduler can spread over several frames: the
//! Head encoding of both frames ([`RifeWorkspace::encode`]), which does not depend on t and is shared by every t of
//! the pair, then for each t the five IFBlocks ([`RifeWorkspace::block`]) and the blend ([`RifeWorkspace::merge`]).
//! Dispatches of one inference must run in order, and one inference must end before the next t starts, since they
//! share the flow buffers; each t has its own timestep slot, so several can be recorded at once.

mod weights;
#[cfg(test)]
mod tests;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use weights::{PackedParams, WEIGHTS_FILE};
use crate::network_shader_module;
use crate::warm_up::CompileReport;

/// Frame sides are padded (zeros, right and bottom) to a multiple of this: block 0 works at 1/16 of the frame and
/// halves it twice more.
pub const RIFE_SIZE_MULTIPLE: u32 = 64;
/// Channels of one frame: RGB over 50% grey (the only ones the network looks at), premultiplied RGB, alpha.
pub const RIFE_IMAGE_CHANNELS: u32 = 7;
/// The alpha among a frame's channels.
pub const RIFE_ALPHA_CHANNEL: u32 = RIFE_IMAGE_CHANNELS - 1;
/// The input buffer: frame 0's channels, then frame 1's.
pub const RIFE_INPUT_CHANNELS: u32 = 2 * RIFE_IMAGE_CHANNELS;
/// The state buffer: flow t->0 xy and t->1 xy, the mask logit, 8 feature channels.
pub const RIFE_STATE_CHANNELS: u32 = 13;
pub const RIFE_FLOW_CHANNELS: u32 = 4;
pub const RIFE_MASK_CHANNEL: u32 = 4;
/// Timestep slots, so the three t of an x4 pair can be recorded together; they start at 1/4, 1/2 and 3/4.
pub const RIFE_TIMESTEP_SLOTS: usize = 3;
pub const RIFE_BLOCKS: usize = 5;

/// Each IFBlock's scale (the frame is resized by its inverse), width and input channels, as Practical-RIFE runs v4.25.
const BLOCK_SCALES: [u32; RIFE_BLOCKS] = [16, 8, 4, 2, 1];
const BLOCK_WIDTHS: [u32; RIFE_BLOCKS] = [192, 128, 96, 64, 32];
const FIRST_BLOCK_INPUT_CHANNELS: u32 = 15;
const BLOCK_INPUT_CHANNELS: u32 = 28;
const RESCONVS_PER_BLOCK: usize = 8;
/// A block's last convolution: 4 * 13 channels, pixel-shuffled into the 13 of the state.
const BLOCK_OUTPUT_CHANNELS: u32 = 4 * RIFE_STATE_CHANNELS;
/// The Head encoder: three 3x3 convolutions of 16 channels on the RGB over grey, then a transposed one to 4.
const HEAD_INPUT_CHANNELS: u32 = 3;
const HEAD_CHANNELS: u32 = 16;
const FEATURE_CHANNELS: u32 = 4;
const HEAD_CONVS: usize = 3;
/// A 3x3 convolution of stride 2 halves each side; a transposed one doubles it.
const STRIDE_2: u32 = 2;
const CONV_TAPS: u32 = 9;
const TRANSPOSED_TAPS: u32 = 4;
const TRANSPOSED_PHASES: u32 = 4;
const CONV_THREADS: u32 = 64;
/// Matches THREADS in pixel.wgsl.
const PIXEL_THREADS: u32 = 64;
const F32_BYTES: u64 = 4;
const PIXEL_TIMESTEP_BYTE: u64 = 20;
const CONV_SHADER: &str = include_str!("../shaders/rife/conv.wgsl");
const PIXEL_SHADER: &str = include_str!("../shaders/rife/pixel.wgsl");
/// pixel.wgsl's switch between the blend's nearest warp (its default) and Practical-RIFE's bilinear one.
const NEAREST_FINAL_WARP_OVERRIDE: &str = "NEAREST_FINAL_WARP";

/// A convolution workgroup's tile: `m` pixels by `n` output channels, `k` rows of the sum staged at a time, each
/// thread keeping `micro_m` by `micro_n` sums.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tile {
    m: u32,
    n: u32,
    k: u32,
    micro_m: u32,
    micro_n: u32,
}

/// Most layers: many pixels, 32 or more channels.
const WIDE_TILE: Tile = Tile { m: 64, n: 32, k: 16, micro_m: 4, micro_n: 8 };
/// Layers of 16 channels or fewer (the Head, block 4's first convolution).
const NARROW_TILE: Tile = Tile { m: 64, n: 16, k: 16, micro_m: 4, micro_n: 4 };
/// The coarse blocks: few pixels, many channels.
const DEEP_TILE: Tile = Tile { m: 16, n: 64, k: 16, micro_m: 2, micro_n: 8 };
/// The Head's last layer, of 4 channels.
const TINY_TILE: Tile = Tile { m: 64, n: 4, k: 16, micro_m: 1, micro_n: 4 };
const TILES: [Tile; 4] = [WIDE_TILE, NARROW_TILE, DEEP_TILE, TINY_TILE];
/// A layer whose tiles make fewer workgroups than this has its sum split into ranges (see conv.wgsl) until it makes
/// about this many, each range keeping at least [`MIN_SPLIT_STEPS`] steps of the tile's `k` rows.
const TARGET_WORKGROUPS: u32 = 96;
const MIN_SPLIT_STEPS: u32 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ConvKind {
    Stride1,
    Stride2,
    Transposed,
    /// Transposed, then PixelShuffle(2).
    TransposedShuffle,
}

/// What follows the sum and bias (values match conv.wgsl).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Epilogue {
    Leaky = 0,
    Linear = 1,
    /// leaky(conv(x) * beta + x).
    Residual = 2,
}

/// The convolution variants the network uses.
const CONV_VARIANTS: [(ConvKind, Epilogue); 5] = [
    (ConvKind::Stride1, Epilogue::Leaky),
    (ConvKind::Stride2, Epilogue::Leaky),
    (ConvKind::Stride1, Epilogue::Residual),
    (ConvKind::Transposed, Epilogue::Linear),
    (ConvKind::TransposedShuffle, Epilogue::Linear),
];

/// The per-pixel entry points of pixel.wgsl.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PixelStep {
    BlockInput,
    Upsample,
    Merge,
}

const PIXEL_STEPS: [PixelStep; 3] = [PixelStep::BlockInput, PixelStep::Upsample, PixelStep::Merge];

impl PixelStep {
    fn entry_point(self) -> &'static str {
        match self {
            PixelStep::BlockInput => "block_input",
            PixelStep::Upsample => "upsample",
            PixelStep::Merge => "merge",
        }
    }
}

/// `Conv` in conv.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ConvUniform {
    in_width: u32,
    in_height: u32,
    in_channels: u32,
    in_base: u32,
    out_width: u32,
    out_height: u32,
    out_channels: u32,
    out_base: u32,
    grid_width: u32,
    grid_pixels: u32,
    weights: u32,
    bias: u32,
    beta: u32,
    splits: u32,
    split_steps: u32,
    _pad: u32,
}

/// `Pixel` in pixel.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PixelUniform {
    width: u32,
    height: u32,
    block_width: u32,
    block_height: u32,
    scale: f32,
    timestep: f32,
    first_block: u32,
    _pad: u32,
}

/// The compiles [`RifePipelines::new`] reports: a convolution variant and tile each (its tiled pass and its
/// reduction), then the per-pixel steps.
pub const RIFE_COMPILES: usize = CONV_VARIANTS.len() * TILES.len() + PIXEL_STEPS.len();

/// The pipelines, once per device, without the weights: what [`crate::warm_up`] compiles ahead.
pub struct RifePipelines {
    conv_layout: wgpu::BindGroupLayout,
    pixel_layout: wgpu::BindGroupLayout,
    conv_pipelines: Vec<((ConvKind, Epilogue, Tile), [wgpu::ComputePipeline; 2])>,
    pixel_pipelines: Vec<(PixelStep, wgpu::ComputePipeline)>,
}

/// The weights and pipelines, once per device.
pub struct RifeModel {
    params: wgpu::Buffer,
    packed: PackedParams,
    conv_layout: wgpu::BindGroupLayout,
    pixel_layout: wgpu::BindGroupLayout,
    /// The tiled pass and the split sums' reduction of each variant.
    conv_pipelines: Vec<((ConvKind, Epilogue, Tile), [wgpu::ComputePipeline; 2])>,
    pixel_pipelines: Vec<(PixelStep, wgpu::ComputePipeline)>,
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

/// Compiles conv.wgsl or pixel.wgsl without naga's loop bounding ([`network_shader_module`]).
fn shader_module(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    // SAFETY: every loop in conv.wgsl and pixel.wgsl ends: each runs to a compile-time constant or to a uniform's
    // value, counting up by a positive step.
    unsafe { network_shader_module(device, label, source) }
}

impl RifePipelines {
    /// Compiles the pipelines, telling `report` of each of [`RIFE_COMPILES`]. The blend warps the frames to the
    /// nearest pixel (pixel.wgsl's `NEAREST_FINAL_WARP`).
    pub fn new(device: &wgpu::Device, report: &mut CompileReport) -> Self {
        Self::with_pixel_constants(device, &[], report)
    }

    /// `constants` sets pixel.wgsl's overridable constants.
    fn with_pixel_constants(device: &wgpu::Device, constants: &[(&str, f64)], report: &mut CompileReport) -> Self {
        let conv_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rife convolution"),
            entries: &[uniform_entry(), storage_entry(1, true), storage_entry(2, true), storage_entry(3, false),
                storage_entry(4, false)],
        });
        let pixel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rife pixel step"),
            entries: &[uniform_entry(), storage_entry(1, true), storage_entry(2, true), storage_entry(3, false),
                storage_entry(4, false), storage_entry(5, false)],
        });
        let pipeline_layout = |layout| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(layout)], immediate_size: 0,
        });
        let conv_pipeline_layout = pipeline_layout(&conv_layout);
        let mut conv_pipelines = Vec::new();
        for (kind, epilogue) in CONV_VARIANTS {
            for tile in TILES {
                let transposed = matches!(kind, ConvKind::Transposed | ConvKind::TransposedShuffle);
                let constants = format!(
                    "const THREADS: u32 = {CONV_THREADS}u;\nconst TILE_M: u32 = {}u;\nconst TILE_N: u32 = {}u;\n\
                     const TILE_K: u32 = {}u;\nconst MICRO_M: u32 = {}u;\nconst MICRO_N: u32 = {}u;\n\
                     const TRANSPOSED: bool = {transposed};\nconst TAPS: u32 = {}u;\nconst PHASES: u32 = {}u;\n\
                     const STRIDE: u32 = {}u;\nconst PIXEL_SHUFFLE: bool = {};\nconst EPILOGUE: u32 = {}u;\n",
                    tile.m, tile.n, tile.k, tile.micro_m, tile.micro_n,
                    if transposed { TRANSPOSED_TAPS } else { CONV_TAPS },
                    if transposed { TRANSPOSED_PHASES } else { 1 },
                    if kind == ConvKind::Stride2 { STRIDE_2 } else { 1 },
                    kind == ConvKind::TransposedShuffle, epilogue as u32);
                let name = format!("RIFE conv {kind:?}/{epilogue:?} tile {}x{}", tile.m, tile.n);
                let pipelines = report.compile(&name, || {
                    let module = shader_module(device, &name, constants + CONV_SHADER);
                    let pipeline = |entry_point| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(&name), layout: Some(&conv_pipeline_layout), module: &module,
                        entry_point: Some(entry_point), compilation_options: Default::default(), cache: None,
                    });
                    [pipeline("main"), pipeline("reduce")]
                });
                conv_pipelines.push(((kind, epilogue, tile), pipelines));
            }
        }
        let pixel_pipeline_layout = pipeline_layout(&pixel_layout);
        let pixel_module = shader_module(device, "RIFE pixel steps", PIXEL_SHADER.to_string());
        let pixel_pipelines = PIXEL_STEPS.into_iter().map(|step| {
            let name = format!("RIFE pixel step: {}", step.entry_point());
            (step, report.compile(&name, || device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&name), layout: Some(&pixel_pipeline_layout), module: &pixel_module,
                entry_point: Some(step.entry_point()),
                compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                cache: None,
            })))
        }).collect();
        Self { conv_layout, pixel_layout, conv_pipelines, pixel_pipelines }
    }
}

impl RifeModel {
    /// Uploads the bundled weights and compiles the pipelines. The blend warps the frames to the nearest pixel
    /// (pixel.wgsl's `NEAREST_FINAL_WARP`).
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        Self::with_pipelines(device, RifePipelines::new(device, &mut CompileReport::silent(RIFE_COMPILES)))
    }

    /// Uploads the bundled weights for pipelines compiled already ([`crate::warm_up`]).
    pub fn with_pipelines(device: &wgpu::Device, pipelines: RifePipelines) -> Result<Self, String> {
        let packed = PackedParams::parse(WEIGHTS_FILE)?;
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rife weights"), contents: bytemuck::cast_slice(&packed.halves),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let RifePipelines { conv_layout, pixel_layout, conv_pipelines, pixel_pipelines } = pipelines;
        Ok(Self { params, packed, conv_layout, pixel_layout, conv_pipelines, pixel_pipelines })
    }

    /// The model with Practical-RIFE's bilinear blend, as the PyTorch reference computes it and the stride
    /// measurement picks the feet by ([`crate::strides`]).
    pub fn with_bilinear_final_warp(device: &wgpu::Device) -> Result<Self, String> {
        Self::with_pipelines(device, RifePipelines::with_pixel_constants(device, &[(NEAREST_FINAL_WARP_OVERRIDE, 0.0)],
            &mut CompileReport::silent(RIFE_COMPILES)))
    }

    /// Bytes of the weights on the GPU.
    pub fn weight_bytes(&self) -> u64 {
        self.params.size()
    }

    fn conv_pipelines(&self, kind: ConvKind, epilogue: Epilogue, tile: Tile) -> &[wgpu::ComputePipeline; 2] {
        &self.conv_pipelines.iter().find(|(key, _)| *key == (kind, epilogue, tile)).unwrap().1
    }

    fn pixel_pipeline(&self, step: PixelStep) -> &wgpu::ComputePipeline {
        &self.pixel_pipelines.iter().find(|(key, _)| *key == step).unwrap().1
    }
}

/// One compute dispatch of the network, ready to record into a compute pass.
#[derive(Clone)]
pub struct RifeDispatch {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    workgroups: [u32; 3],
    multiply_adds: u64,
    name: String,
}

impl RifeDispatch {
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

    /// The layer, named as in IFNet_HDv3.py where it is one.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Buf {
    Images,
    Features,
    State,
    Block,
    ScratchA,
    ScratchB,
    Merged,
    /// The split layers' partial sums.
    Partial,
}

const BUFS: [Buf; 8] =
    [Buf::Images, Buf::Features, Buf::State, Buf::Block, Buf::ScratchA, Buf::ScratchB, Buf::Merged, Buf::Partial];

/// A convolution layer of the plan: its tensors' name, and where it reads and writes.
struct ConvLayer {
    /// The weight and bias are `{name}.weight` and `{name}.bias`.
    name: String,
    /// A ResConv's beta.
    beta: Option<String>,
    kind: ConvKind,
    epilogue: Epilogue,
    input: (Buf, u32),
    in_size: (u32, u32),
    in_channels: u32,
    output: (Buf, u32),
    out_channels: u32,
}

enum Binding {
    Conv { input: Buf, output: Buf },
    Pixel,
}

/// A dispatch of the plan, before the uniform buffer and bind groups exist.
struct PendingDispatch {
    pipeline: wgpu::ComputePipeline,
    uniform: Vec<u8>,
    binding: Binding,
    workgroups: [u32; 3],
    multiply_adds: u64,
    name: String,
}

/// Collects the dispatches of a plan.
struct Planner<'a> {
    model: &'a RifeModel,
    pending: Vec<PendingDispatch>,
    /// The largest split layer's partial sums.
    partial_floats: u32,
}

impl Planner<'_> {
    /// Plans a layer's dispatches (two if its sum is split) and appends their indices to `list`.
    fn conv(&mut self, list: &mut Vec<usize>, layer: ConvLayer) -> Result<(), String> {
        let transposed = matches!(layer.kind, ConvKind::Transposed | ConvKind::TransposedShuffle);
        let (in_width, in_height) = layer.in_size;
        let (out_width, out_height) = match layer.kind {
            ConvKind::Stride1 => layer.in_size,
            ConvKind::Stride2 => (in_width / STRIDE_2, in_height / STRIDE_2),
            ConvKind::Transposed => (in_width * STRIDE_2, in_height * STRIDE_2),
            ConvKind::TransposedShuffle => (in_width * STRIDE_2 * STRIDE_2, in_height * STRIDE_2 * STRIDE_2),
        };
        let (grid_width, grid_pixels, phases, taps) = if transposed {
            (in_width, in_width * in_height, TRANSPOSED_PHASES, TRANSPOSED_TAPS)
        } else {
            (out_width, out_width * out_height, 1, CONV_TAPS)
        };
        let tile = if layer.out_channels <= TINY_TILE.n {
            TINY_TILE
        } else if layer.out_channels <= NARROW_TILE.n {
            NARROW_TILE
        } else if grid_pixels < WIDE_TILE.m {
            DEEP_TILE
        } else {
            WIDE_TILE
        };
        let workgroups = [grid_pixels.div_ceil(tile.m), layer.out_channels.div_ceil(tile.n), phases];
        let steps = (layer.in_channels * taps).div_ceil(tile.k);
        let tiles = workgroups.iter().product::<u32>();
        let wanted = if tiles >= TARGET_WORKGROUPS { 1 } else { TARGET_WORKGROUPS.div_ceil(tiles) };
        let split_steps = steps.div_ceil(wanted.min(steps / MIN_SPLIT_STEPS).max(1));
        let splits = steps.div_ceil(split_steps);
        let packed = &self.model.packed;
        let uniform = ConvUniform {
            in_width, in_height, in_channels: layer.in_channels, in_base: layer.input.1,
            out_width, out_height, out_channels: layer.out_channels, out_base: layer.output.1,
            grid_width, grid_pixels,
            weights: packed.offset(&format!("{}.weight", layer.name))?,
            bias: packed.offset(&format!("{}.bias", layer.name))?,
            beta: layer.beta.as_deref().map_or(Ok(0), |beta| packed.offset(beta))?,
            splits, split_steps, _pad: 0,
        };
        let [tiled, reduce] = self.model.conv_pipelines(layer.kind, layer.epilogue, tile);
        let outputs = grid_pixels * phases * layer.out_channels;
        let binding = || Binding::Conv { input: layer.input.0, output: layer.output.0 };
        self.pending.push(PendingDispatch {
            pipeline: tiled.clone(), uniform: bytemuck::bytes_of(&uniform).to_vec(), binding: binding(),
            workgroups: [workgroups[0], workgroups[1], workgroups[2] * splits],
            multiply_adds: outputs as u64 * (layer.in_channels * taps) as u64, name: layer.name.clone(),
        });
        list.push(self.pending.len() - 1);
        if splits > 1 {
            self.partial_floats = self.partial_floats.max(splits * outputs);
            self.pending.push(PendingDispatch {
                pipeline: reduce.clone(), uniform: bytemuck::bytes_of(&uniform).to_vec(), binding: binding(),
                workgroups: [outputs.div_ceil(CONV_THREADS), 1, 1], multiply_adds: 0,
                name: format!("{} reduce", layer.name),
            });
            list.push(self.pending.len() - 1);
        }
        Ok(())
    }

    fn pixel(&mut self, step: PixelStep, uniform: PixelUniform, threads: u32, name: String) -> usize {
        self.pending.push(PendingDispatch {
            pipeline: self.model.pixel_pipeline(step).clone(), uniform: bytemuck::bytes_of(&uniform).to_vec(),
            binding: Binding::Pixel, workgroups: [threads.div_ceil(PIXEL_THREADS), 1, 1], multiply_adds: 0, name,
        });
        self.pending.len() - 1
    }
}

fn other_scratch(buf: Buf) -> Buf {
    if buf == Buf::ScratchA { Buf::ScratchB } else { Buf::ScratchA }
}

/// The buffers of one frame size and the dispatches that interpolate on them. Buffers are f32 channel planes of the
/// padded frame unless said otherwise.
pub struct RifeWorkspace {
    width: u32,
    height: u32,
    buffers: Vec<wgpu::Buffer>,
    uniforms: wgpu::Buffer,
    encode: Vec<RifeDispatch>,
    blocks: Vec<[Vec<RifeDispatch>; RIFE_BLOCKS]>,
    merge: Vec<RifeDispatch>,
    /// Where each slot's timestep sits in the uniform buffer, once per block.
    timestep_offsets: Vec<[u64; RIFE_BLOCKS]>,
}

impl RifeWorkspace {
    /// Allocates the buffers for frames of this size (padded to [`RIFE_SIZE_MULTIPLE`]) and plans the dispatches.
    pub fn new(model: &RifeModel, device: &wgpu::Device, width: u32, height: u32) -> Result<Self, String> {
        let width = width.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE;
        let height = height.div_ceil(RIFE_SIZE_MULTIPLE) * RIFE_SIZE_MULTIPLE;
        let plane = width * height;
        let mut scratch = HEAD_CHANNELS * plane / (STRIDE_2 * STRIDE_2);
        let mut block = 0;
        for (&scale, &channels) in BLOCK_SCALES.iter().zip(&BLOCK_WIDTHS) {
            let block_plane = plane / (scale * scale);
            block = block.max(BLOCK_INPUT_CHANNELS.max(RIFE_STATE_CHANNELS) * block_plane);
            scratch = scratch.max(channels / 2 * block_plane / (STRIDE_2 * STRIDE_2));
        }

        let mut planner = Planner { model, pending: Vec::new(), partial_floats: 0 };

        // The Head, on each frame's RGB over grey: three convolutions ping-ponging in the scratch buffers, then the
        // transposed one into the features.
        let half_size = (width / STRIDE_2, height / STRIDE_2);
        let mut encode = Vec::new();
        for image in 0..2 {
            planner.conv(&mut encode, ConvLayer {
                name: "encode.cnn0".into(), beta: None, kind: ConvKind::Stride2, epilogue: Epilogue::Leaky,
                input: (Buf::Images, image * RIFE_IMAGE_CHANNELS * plane), in_size: (width, height),
                in_channels: HEAD_INPUT_CHANNELS, output: (Buf::ScratchA, 0), out_channels: HEAD_CHANNELS,
            })?;
            let mut source = Buf::ScratchA;
            for index in 1..HEAD_CONVS {
                let target = other_scratch(source);
                planner.conv(&mut encode, ConvLayer {
                    name: format!("encode.cnn{index}"), beta: None, kind: ConvKind::Stride1, epilogue: Epilogue::Leaky,
                    input: (source, 0), in_size: half_size, in_channels: HEAD_CHANNELS,
                    output: (target, 0), out_channels: HEAD_CHANNELS,
                })?;
                source = target;
            }
            planner.conv(&mut encode, ConvLayer {
                name: format!("encode.cnn{HEAD_CONVS}"), beta: None, kind: ConvKind::Transposed,
                epilogue: Epilogue::Linear, input: (source, 0), in_size: half_size, in_channels: HEAD_CHANNELS,
                output: (Buf::Features, image * FEATURE_CHANNELS * plane), out_channels: FEATURE_CHANNELS,
            })?;
        }

        // Each block: its input (per timestep slot), two stride 2 convolutions, eight ResConvs, the transposed
        // convolution into the block buffer (free once the first convolution has read the input), and the resize
        // back to the frame.
        let pixel_uniform = |block: usize, timestep: f32| {
            let scale = BLOCK_SCALES[block];
            PixelUniform {
                width, height, block_width: width / scale, block_height: height / scale, scale: scale as f32,
                timestep, first_block: (block == 0) as u32, _pad: 0,
            }
        };
        let mut block_inputs = vec![[0; RIFE_BLOCKS]; RIFE_TIMESTEP_SLOTS];
        let mut block_layers: Vec<Vec<usize>> = Vec::new();
        let mut block_upsamples = Vec::new();
        for block in 0..RIFE_BLOCKS {
            let scale = BLOCK_SCALES[block];
            let block_size = (width / scale, height / scale);
            for (slot, inputs) in block_inputs.iter_mut().enumerate() {
                inputs[block] = planner.pixel(PixelStep::BlockInput, pixel_uniform(block, default_timestep(slot)),
                    block_size.0 * block_size.1, format!("block{block} input"));
            }
            let channels = BLOCK_WIDTHS[block];
            let mut layers = Vec::new();
            planner.conv(&mut layers, ConvLayer {
                name: format!("block{block}.conv0.0.0"), beta: None, kind: ConvKind::Stride2, epilogue: Epilogue::Leaky,
                input: (Buf::Block, 0), in_size: block_size,
                in_channels: if block == 0 { FIRST_BLOCK_INPUT_CHANNELS } else { BLOCK_INPUT_CHANNELS },
                output: (Buf::ScratchA, 0), out_channels: channels / 2,
            })?;
            let half = (block_size.0 / STRIDE_2, block_size.1 / STRIDE_2);
            planner.conv(&mut layers, ConvLayer {
                name: format!("block{block}.conv0.1.0"), beta: None, kind: ConvKind::Stride2, epilogue: Epilogue::Leaky,
                input: (Buf::ScratchA, 0), in_size: half, in_channels: channels / 2,
                output: (Buf::ScratchB, 0), out_channels: channels,
            })?;
            let quarter = (half.0 / STRIDE_2, half.1 / STRIDE_2);
            let mut source = Buf::ScratchB;
            for index in 0..RESCONVS_PER_BLOCK {
                let target = other_scratch(source);
                planner.conv(&mut layers, ConvLayer {
                    name: format!("block{block}.convblock.{index}.conv"),
                    beta: Some(format!("block{block}.convblock.{index}.beta")),
                    kind: ConvKind::Stride1, epilogue: Epilogue::Residual, input: (source, 0), in_size: quarter,
                    in_channels: channels, output: (target, 0), out_channels: channels,
                })?;
                source = target;
            }
            planner.conv(&mut layers, ConvLayer {
                name: format!("block{block}.lastconv.0"), beta: None, kind: ConvKind::TransposedShuffle,
                epilogue: Epilogue::Linear, input: (source, 0), in_size: quarter, in_channels: channels,
                output: (Buf::Block, 0), out_channels: BLOCK_OUTPUT_CHANNELS,
            })?;
            block_layers.push(layers);
            block_upsamples.push(planner.pixel(PixelStep::Upsample, pixel_uniform(block, 0.0), plane,
                format!("block{block} upsample")));
        }
        let merge = planner.pixel(PixelStep::Merge, pixel_uniform(RIFE_BLOCKS - 1, 0.0), plane, "merge".to_string());
        let partial = planner.partial_floats.max(1);
        let pending = planner.pending;
        let floats = |buf| match buf {
            Buf::Images => RIFE_INPUT_CHANNELS * plane,
            Buf::Features => 2 * FEATURE_CHANNELS * plane,
            Buf::State => RIFE_STATE_CHANNELS * plane,
            Buf::Block => block,
            Buf::ScratchA | Buf::ScratchB => scratch,
            Buf::Merged => RIFE_IMAGE_CHANNELS * plane,
            Buf::Partial => partial,
        };
        let buffers: Vec<wgpu::Buffer> = BUFS.into_iter().map(|buf| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rife workspace"), size: floats(buf) as u64 * F32_BYTES, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        })).collect();

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
            label: Some("rife dispatch parameters"), contents: &uniform_bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let buffer = |buf: Buf| buffers[BUFS.iter().position(|&b| b == buf).unwrap()].as_entire_binding();
        let dispatches: Vec<RifeDispatch> = pending.into_iter().zip(&offsets).map(|(dispatch, &offset)| {
            let uniform = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &uniforms, offset: offset as u64, size: wgpu::BufferSize::new(dispatch.uniform.len() as u64),
            });
            let bind_group = match dispatch.binding {
                Binding::Conv { input, output } => device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("rife convolution"), layout: &model.conv_layout, entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: uniform },
                        wgpu::BindGroupEntry { binding: 1, resource: model.params.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: buffer(input) },
                        wgpu::BindGroupEntry { binding: 3, resource: buffer(output) },
                        wgpu::BindGroupEntry { binding: 4, resource: buffer(Buf::Partial) },
                    ],
                }),
                Binding::Pixel => device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("rife pixel step"), layout: &model.pixel_layout, entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: uniform },
                        wgpu::BindGroupEntry { binding: 1, resource: buffer(Buf::Images) },
                        wgpu::BindGroupEntry { binding: 2, resource: buffer(Buf::Features) },
                        wgpu::BindGroupEntry { binding: 3, resource: buffer(Buf::State) },
                        wgpu::BindGroupEntry { binding: 4, resource: buffer(Buf::Block) },
                        wgpu::BindGroupEntry { binding: 5, resource: buffer(Buf::Merged) },
                    ],
                }),
            };
            RifeDispatch {
                pipeline: dispatch.pipeline, bind_group, workgroups: dispatch.workgroups,
                multiply_adds: dispatch.multiply_adds, name: dispatch.name,
            }
        }).collect();

        let blocks = block_inputs.iter().map(|inputs| std::array::from_fn(|block| {
            std::iter::once(inputs[block]).chain(block_layers[block].iter().copied())
                .chain(std::iter::once(block_upsamples[block])).map(|index| dispatches[index].clone()).collect()
        })).collect();
        let timestep_offsets = block_inputs.iter()
            .map(|inputs| inputs.map(|index| offsets[index] as u64 + PIXEL_TIMESTEP_BYTE)).collect();
        Ok(Self {
            width, height, buffers, uniforms,
            encode: encode.into_iter().map(|index| dispatches[index].clone()).collect(),
            blocks, merge: vec![dispatches[merge].clone()], timestep_offsets,
        })
    }

    /// The padded frame size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn buffer(&self, buf: Buf) -> &wgpu::Buffer {
        &self.buffers[BUFS.iter().position(|&b| b == buf).unwrap()]
    }

    /// The two frames, [`RIFE_INPUT_CHANNELS`] planes; the padding is zero as PyTorch pads, or the frames' own
    /// background, as the mobiles' in-betweens give it (atlas.wgsl).
    pub fn input(&self) -> &wgpu::Buffer {
        self.buffer(Buf::Images)
    }

    /// The flow, mask logit and features of the last block run, [`RIFE_STATE_CHANNELS`] planes.
    pub fn state(&self) -> &wgpu::Buffer {
        self.buffer(Buf::State)
    }

    /// The in-between, [`RIFE_IMAGE_CHANNELS`] planes laid out as a frame.
    pub fn merged(&self) -> &wgpu::Buffer {
        self.buffer(Buf::Merged)
    }

    /// Bytes [`RifeWorkspace::copy_flow_and_alpha`] writes: [`RIFE_FLOW_CHANNELS`] planes, then one.
    pub fn flow_and_alpha_bytes(&self) -> u64 {
        (RIFE_FLOW_CHANNELS + 1) as u64 * (self.width * self.height) as u64 * F32_BYTES
    }

    /// Records copies of the last inference's flow (t->0 xy, t->1 xy, in pixels of the padded frame) and of the
    /// in-between's alpha into `target` at `offset`, for reading back.
    pub fn copy_flow_and_alpha(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer, offset: u64) {
        let plane_bytes = (self.width * self.height) as u64 * F32_BYTES;
        let flow_bytes = RIFE_FLOW_CHANNELS as u64 * plane_bytes;
        encoder.copy_buffer_to_buffer(self.state(), 0, target, offset, flow_bytes);
        encoder.copy_buffer_to_buffer(self.merged(), RIFE_ALPHA_CHANNEL as u64 * plane_bytes, target,
            offset + flow_bytes, plane_bytes);
    }

    /// Sets a slot's t, in (0, 1); it takes effect with the next queue submission.
    pub fn set_timestep(&self, queue: &wgpu::Queue, slot: usize, timestep: f32) {
        for offset in self.timestep_offsets[slot] {
            queue.write_buffer(&self.uniforms, offset, bytemuck::bytes_of(&timestep));
        }
    }

    /// The Head encoding of both frames, shared by every t of the pair.
    pub fn encode(&self) -> &[RifeDispatch] {
        &self.encode
    }

    /// One IFBlock at a slot's t: its input, convolutions and the resize of its flow, mask and features.
    pub fn block(&self, slot: usize, block: usize) -> &[RifeDispatch] {
        &self.blocks[slot][block]
    }

    /// The blend of the two warped frames into [`RifeWorkspace::merged`].
    pub fn merge(&self) -> &[RifeDispatch] {
        &self.merge
    }

    /// Everything after [`RifeWorkspace::encode`] for one t: the blocks, then the blend.
    pub fn interpolate(&self, slot: usize) -> impl Iterator<Item = &RifeDispatch> {
        self.blocks[slot].iter().flatten().chain(&self.merge)
    }

    /// Bytes of the workspace's buffers on the GPU, not counting the model's weights.
    pub fn working_bytes(&self) -> u64 {
        self.buffers.iter().map(wgpu::Buffer::size).sum::<u64>() + self.uniforms.size()
    }
}

/// The t a slot starts with: the quarters of an x4 pair.
fn default_timestep(slot: usize) -> f32 {
    (slot + 1) as f32 / (RIFE_TIMESTEP_SLOTS + 1) as f32
}
