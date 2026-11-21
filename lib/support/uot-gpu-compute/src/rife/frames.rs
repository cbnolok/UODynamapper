//! The steps between a texture of frames and a [`RifeWorkspace`] (frames.wgsl): two frames, each at its place on a
//! card, into the workspace's input ([`RifeFrameStep::pack`]); a frame as it is ([`RifeFrameStep::copy_frame`]) or
//! the in-between ([`RifeFrameStep::store`]) into output rows of RGBA8, sRGB colour with straight alpha, which the
//! caller copies into a texture (an sRGB texture takes no storage writes). The source is a 2D array texture of
//! straight alpha whose loads return linear colour, such as an `Rgba8UnormSrgb` or `Bc7RgbaUnormSrgb` atlas.
//!
//! A step's parameters go into a uniform buffer of the caller's, each at an offset of its own (a multiple of
//! [`rife_frame_step_stride`]), so the steps of a submission share one bind group per source and workspace
//! ([`RifeModel::frame_bind_group`]).

use bytemuck::{Pod, Zeroable};
use crate::warm_up::CompileReport;
use crate::SourceRect;
use super::{shader_module, RifeModel, RifeWorkspace};

const FRAMES_SHADER: &str = include_str!("../../shaders/rife/frames.wgsl");
/// Matches THREADS in frames.wgsl.
const FRAME_STEP_THREADS: u32 = 64;
/// Bytes of an output texel.
const OUTPUT_TEXEL_BYTES: u32 = 4;
/// The compiles of [`FramePipelines::new`]: one module, its three entry points.
pub(super) const FRAME_STEP_COMPILES: usize = 1;

/// `FrameStep` in frames.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct FrameStepUniform {
    width: u32,
    height: u32,
    card_width: u32,
    card_height: u32,
    first_layer: u32,
    first_x: u32,
    first_y: u32,
    first_width: u32,
    first_height: u32,
    first_offset_x: u32,
    first_offset_y: u32,
    next_layer: u32,
    next_x: u32,
    next_y: u32,
    next_width: u32,
    next_height: u32,
    next_offset_x: u32,
    next_offset_y: u32,
    pitch: u32,
    snap_alpha: u32,
}

/// The frame steps' pipelines.
pub(super) struct FramePipelines {
    layout: wgpu::BindGroupLayout,
    pack: wgpu::ComputePipeline,
    copy_frame: wgpu::ComputePipeline,
    store: wgpu::ComputePipeline,
}

impl FramePipelines {
    pub(super) fn new(device: &wgpu::Device, report: &mut CompileReport) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None,
            },
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rife frame step"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(RIFE_FRAME_STEP_BYTES),
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false,
                    },
                },
                storage(2, false),
                storage(3, true),
                storage(4, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let name = "RIFE frame steps";
        let [pack, copy_frame, store] = report.compile(name, || {
            let module = shader_module(device, name, FRAMES_SHADER.to_string());
            ["pack", "copy_frame", "store"].map(|entry_point| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry_point), layout: Some(&pipeline_layout), module: &module,
                    entry_point: Some(entry_point), compilation_options: Default::default(), cache: None,
                })
            })
        });
        Self { layout, pack, copy_frame, store }
    }
}

/// Bytes of a step's parameters.
pub const RIFE_FRAME_STEP_BYTES: u64 = std::mem::size_of::<FrameStepUniform>() as u64;

/// How far apart steps' parameters sit in a uniform buffer: [`RIFE_FRAME_STEP_BYTES`] rounded up to the device's
/// uniform offset alignment.
pub fn rife_frame_step_stride(device: &wgpu::Device) -> u64 {
    RIFE_FRAME_STEP_BYTES.next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64)
}

/// The bytes of an output row of a card `width` wide: a copy into a texture takes rows of a multiple of 256 bytes.
pub fn rife_output_bytes_per_row(width: u32) -> u32 {
    (width * OUTPUT_TEXEL_BYTES).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// A frame of the source and where its top left lies on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardFrame {
    pub source: SourceRect,
    pub offset: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepKind {
    Pack,
    CopyFrame,
    Store,
}

/// One step between the source and a workspace of a size, ready to record.
#[derive(Clone, Copy, Debug)]
pub struct RifeFrameStep {
    kind: StepKind,
    uniform: FrameStepUniform,
}

impl RifeFrameStep {
    /// Both frames at their places on the card into the input of a workspace of `size` (padded,
    /// [`RifeWorkspace::size`]); the rest of its planes is transparent.
    pub fn pack(size: (u32, u32), first: CardFrame, next: CardFrame) -> Self {
        let uniform = FrameStepUniform {
            first_layer: first.source.layer, first_x: first.source.x, first_y: first.source.y,
            first_width: first.source.width, first_height: first.source.height,
            first_offset_x: first.offset.0, first_offset_y: first.offset.1,
            next_layer: next.source.layer, next_x: next.source.x, next_y: next.source.y,
            next_width: next.source.width, next_height: next.source.height,
            next_offset_x: next.offset.0, next_offset_y: next.offset.1,
            ..Self::card_uniform(size, size)
        };
        Self { kind: StepKind::Pack, uniform }
    }

    /// `frame` as it is into the output rows, at their top left.
    pub fn copy_frame(size: (u32, u32), frame: SourceRect) -> Self {
        let uniform = FrameStepUniform {
            first_layer: frame.layer, first_x: frame.x, first_y: frame.y, first_width: frame.width,
            first_height: frame.height,
            ..Self::card_uniform(size, (frame.width, frame.height))
        };
        Self { kind: StepKind::CopyFrame, uniform }
    }

    /// The top-left `card` of the in-between into the output rows, straight alpha; with `snap_alpha`, its alpha 0
    /// or 1 at one half (frames of one bit of alpha, such as the Classic client's).
    pub fn store(size: (u32, u32), card: (u32, u32), snap_alpha: bool) -> Self {
        let uniform = FrameStepUniform { snap_alpha: snap_alpha as u32, ..Self::card_uniform(size, card) };
        Self { kind: StepKind::Store, uniform }
    }

    fn card_uniform(size: (u32, u32), card: (u32, u32)) -> FrameStepUniform {
        FrameStepUniform {
            width: size.0, height: size.1, card_width: card.0, card_height: card.1,
            pitch: rife_output_bytes_per_row(card.0) / OUTPUT_TEXEL_BYTES, ..Default::default()
        }
    }

    /// The parameters, for the uniform buffer.
    pub fn uniform(&self) -> &[u8] {
        bytemuck::bytes_of(&self.uniform)
    }

    /// The pixels it runs on: the workspace's for a pack, the card's otherwise.
    pub fn pixels(&self) -> u32 {
        let uniform = &self.uniform;
        match self.kind {
            StepKind::Pack => uniform.width * uniform.height,
            StepKind::CopyFrame | StepKind::Store => uniform.card_width * uniform.card_height,
        }
    }

    /// Records the step, its parameters at `uniform_offset` of the bind group's uniform buffer.
    pub fn record(&self, model: &RifeModel, pass: &mut wgpu::ComputePass<'_>, bind_group: &wgpu::BindGroup,
        uniform_offset: u32) {
        let pipelines = &model.frames;
        pass.set_pipeline(match self.kind {
            StepKind::Pack => &pipelines.pack,
            StepKind::CopyFrame => &pipelines.copy_frame,
            StepKind::Store => &pipelines.store,
        });
        pass.set_bind_group(0, bind_group, &[uniform_offset]);
        pass.dispatch_workgroups(self.pixels().div_ceil(FRAME_STEP_THREADS), 1, 1);
    }
}

impl RifeModel {
    /// The bind group of frame steps reading `source` and writing `workspace` and `output` (rows of
    /// [`rife_output_bytes_per_row`], as many as the card's height; STORAGE), their parameters in `uniforms`
    /// (UNIFORM) at a dynamic offset.
    pub fn frame_bind_group(&self, device: &wgpu::Device, workspace: &RifeWorkspace, output: &wgpu::Buffer,
        uniforms: &wgpu::Buffer, source: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rife frame step"), layout: &self.frames.layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniforms, offset: 0, size: wgpu::BufferSize::new(RIFE_FRAME_STEP_BYTES),
                }) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 2, resource: workspace.input().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: workspace.merged().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: output.as_entire_binding() },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_reads_a_step_as_wgsl_lays_it_out() {
        // Twenty u32.
        const WGSL_FRAME_STEP_SIZE: u64 = 80;
        assert_eq!(RIFE_FRAME_STEP_BYTES, WGSL_FRAME_STEP_SIZE);
    }

    #[test]
    fn output_rows_are_aligned_for_a_copy() {
        assert_eq!(rife_output_bytes_per_row(1), wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        assert_eq!(rife_output_bytes_per_row(65), 2 * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    }

    #[test]
    fn a_frame_copy_puts_the_frame_alone_at_the_card_corner() {
        let frame = SourceRect { layer: 2, x: 9, y: 4, width: 30, height: 20 };
        let step = RifeFrameStep::copy_frame((64, 64), frame);
        assert_eq!((step.uniform.first_offset_x, step.uniform.first_offset_y), (0, 0));
        assert_eq!((step.uniform.card_width, step.uniform.card_height), (30, 20));
        assert_eq!(step.pixels(), 600);
        let pack = RifeFrameStep::pack((64, 64), CardFrame { source: frame, offset: (3, 1) },
            CardFrame { source: frame, offset: (0, 0) });
        assert_eq!(pack.pixels(), 64 * 64);
    }
}
