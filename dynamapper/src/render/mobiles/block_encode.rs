//! GPU block compression of the mobiles' frames, for a BC7 or BC1 frame atlas (evaluated in `tests.rs`; not wired to
//! the atlas yet). BC7 keeps soft alpha (Enhanced, KR); BC1 keeps one bit of it, which is all Classic art and its
//! in-betweens have. The encoding is block_compression's (a WGSL port of Intel's ISPC Texture Compressor kernels,
//! MIT): its BC7 as it is, its BC1 followed by [`PUNCH_THROUGH_SHADER`], since that kernel encodes colour only.
//!
//! A frame to encode lies at the top-left corner of a texture whose view is a non-sRGB RGBA8 format (`Rgba8Unorm`)
//! holding the frame's sRGB-encoded, straight-alpha bytes: an `Rgba8UnormSrgb` texture needs `Rgba8Unorm` among its
//! `view_formats`. The encoders work on those bytes as they are, so an `*UnormSrgb` BC texture decodes them to the same
//! sRGB bytes and samples them as the RGBA8 atlas does. The texture covers the frame padded to whole 4x4 blocks
//! ([`padded_size`]), the padding transparent. The blocks go to a storage buffer row by row of blocks, tightly, and
//! [`record_copy_to_layer`] copies them into a layer of a BC texture.
//!
//! The GPU's zero fill suits a BC7 atlas (a block of zeros has no valid mode and decodes to transparent black) but not
//! a BC1 one: zero endpoints and indices decode to opaque black, so empty BC1 pages are filled with
//! [`BC1_TRANSPARENT_BLOCK`].

#[cfg(test)]
mod tests;

use block_compression::{BC7Settings, CompressionVariant, GpuBlockCompressor};
use wgpu::util::DeviceExt;
use super::network_shader_module;

/// Texels per side of a block.
pub const BLOCK_SIDE: u32 = 4;
const BC7_BLOCK_BYTES: u32 = 16;
const BC1_BLOCK_BYTES: u32 = 8;
/// A transparent BC1 block: equal endpoints (the three-colour mode) and index 3 everywhere.
pub const BC1_TRANSPARENT_BLOCK: [u8; BC1_BLOCK_BYTES as usize] = [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
const U32_BYTES: u64 = 4;
/// Matches the workgroup size of bc1_punch_through.wgsl.
const PUNCH_THROUGH_WORKGROUP_SIDE: u32 = 8;

const PUNCH_THROUGH_SHADER: &str = include_str!("../../../assets/shaders/block_encode/bc1_punch_through.wgsl");

/// block_compression's BC7 settings for colour with alpha, fastest to slowest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bc7Quality {
    UltraFast,
    VeryFast,
    Fast,
    Basic,
    Slow,
}

impl Bc7Quality {
    pub const ALL: [Self; 5] = [Self::UltraFast, Self::VeryFast, Self::Fast, Self::Basic, Self::Slow];

    fn settings(self) -> BC7Settings {
        match self {
            Self::UltraFast => BC7Settings::alpha_ultrafast(),
            Self::VeryFast => BC7Settings::alpha_very_fast(),
            Self::Fast => BC7Settings::alpha_fast(),
            Self::Basic => BC7Settings::alpha_basic(),
            Self::Slow => BC7Settings::alpha_slow(),
        }
    }
}

/// A block format of the frame atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobileBlockFormat {
    /// 1 byte per texel, soft alpha.
    Bc7(Bc7Quality),
    /// Half a byte per texel; a texel is opaque or transparent (alpha from one half up is opaque).
    Bc1,
}

impl MobileBlockFormat {
    /// The texture format of an atlas of these blocks.
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bc7(_) => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            Self::Bc1 => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        }
    }

    pub fn block_bytes(self) -> u32 {
        match self {
            Self::Bc7(_) => BC7_BLOCK_BYTES,
            Self::Bc1 => BC1_BLOCK_BYTES,
        }
    }

    /// Bytes of the blocks of a `width` by `height` frame.
    pub fn encoded_bytes(self, width: u32, height: u32) -> u64 {
        let (width, height) = padded_size(width, height);
        u64::from(width / BLOCK_SIDE) * u64::from(height / BLOCK_SIDE) * u64::from(self.block_bytes())
    }

    fn variant(self) -> CompressionVariant {
        match self {
            Self::Bc7(quality) => CompressionVariant::BC7(quality.settings()),
            Self::Bc1 => CompressionVariant::BC1,
        }
    }
}

/// A frame's size rounded up to whole blocks.
pub fn padded_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(BLOCK_SIDE) * BLOCK_SIDE, height.div_ceil(BLOCK_SIDE) * BLOCK_SIDE)
}

/// A BC1 frame waiting for its alpha pass.
struct PunchThroughFrame {
    bind_group: wgpu::BindGroup,
    workgroups: (u32, u32),
}

/// The encoders and the frames recorded for the next [`MobileBlockEncoder::record`].
pub struct MobileBlockEncoder {
    device: wgpu::Device,
    compressor: GpuBlockCompressor,
    punch_through_layout: wgpu::BindGroupLayout,
    punch_through: wgpu::ComputePipeline,
    punch_through_frames: Vec<PunchThroughFrame>,
}

impl MobileBlockEncoder {
    /// Compiles the encoders' pipelines: block_compression builds all of its BC1 to BC5 and BC7 pipelines here.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let compressor = GpuBlockCompressor::new(device.clone(), queue.clone());
        // SAFETY: every loop of bc1_punch_through.wgsl runs to a constant, counting up by one.
        let module = unsafe { network_shader_module(device, "bc1 punch-through", PUNCH_THROUGH_SHADER.to_string()) };
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None,
        };
        let punch_through_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bc1 punch-through"),
            entries: &[
                entry(0, wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                }),
                entry(1, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                entry(2, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None,
                }),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bc1 punch-through"), bind_group_layouts: &[Some(&punch_through_layout)], immediate_size: 0,
        });
        let punch_through = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bc1 punch-through"), layout: Some(&layout), module: &module, entry_point: Some("punch_through"),
            compilation_options: Default::default(), cache: None,
        });
        Self { device: device.clone(), compressor, punch_through_layout, punch_through, punch_through_frames: Vec::new() }
    }

    /// Adds the encoding of a `width` by `height` frame at the top-left corner of `source` (see the module doc) to
    /// the next [`MobileBlockEncoder::record`]; its blocks go to `output`, a storage buffer, from byte `offset` (a
    /// multiple of 4).
    pub fn add_frame(&mut self, format: MobileBlockFormat, source: &wgpu::TextureView, width: u32, height: u32,
        output: &wgpu::Buffer, offset: u64) {
        let (width, height) = padded_size(width, height);
        let blocks_offset = u32::try_from(offset).expect("block offsets fit u32");
        self.compressor.add_compression_task(format.variant(), source, width, height, output, None, Some(blocks_offset));
        if format != MobileBlockFormat::Bc1 {
            return;
        }
        let uniforms = [width, height, (offset / U32_BYTES) as u32, 0];
        let uniforms = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bc1 punch-through frame"), contents: bytemuck::cast_slice(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bc1 punch-through"), layout: &self.punch_through_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: uniforms.as_entire_binding() },
            ],
        });
        let blocks = |side: u32| (side / BLOCK_SIDE).div_ceil(PUNCH_THROUGH_WORKGROUP_SIDE);
        self.punch_through_frames.push(PunchThroughFrame { bind_group, workgroups: (blocks(width), blocks(height)) });
    }

    /// Records the encoding of the frames added since the last call into `pass`.
    pub fn record(&mut self, pass: &mut wgpu::ComputePass<'_>) {
        self.compressor.compress(pass);
        if self.punch_through_frames.is_empty() {
            return;
        }
        pass.set_pipeline(&self.punch_through);
        for frame in self.punch_through_frames.drain(..) {
            pass.set_bind_group(0, &frame.bind_group, &[]);
            pass.dispatch_workgroups(frame.workgroups.0, frame.workgroups.1, 1);
        }
    }
}

/// Records copies of a frame's blocks, `width` by `height` texels from byte `offset` of `blocks`, into `layer` of
/// `texture` (of `format`'s texture format) at `x`, `y` (multiples of [`BLOCK_SIDE`]). A copy of several rows needs
/// rows of 256 bytes, so each row of blocks is a copy of its own.
pub fn record_copy_to_layer(encoder: &mut wgpu::CommandEncoder, format: MobileBlockFormat, blocks: &wgpu::Buffer,
    offset: u64, width: u32, height: u32, texture: &wgpu::Texture, layer: u32, x: u32, y: u32) {
    debug_assert!(x % BLOCK_SIDE == 0 && y % BLOCK_SIDE == 0, "a block copy starts on a block");
    let (width, height) = padded_size(width, height);
    let row_bytes = u64::from(width / BLOCK_SIDE * format.block_bytes());
    for row in 0..height / BLOCK_SIDE {
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: blocks,
                layout: wgpu::TexelCopyBufferLayout { offset: offset + u64::from(row) * row_bytes, bytes_per_row: None,
                    rows_per_image: None },
            },
            wgpu::TexelCopyTextureInfo {
                texture, mip_level: 0, origin: wgpu::Origin3d { x, y: y + row * BLOCK_SIDE, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d { width, height: BLOCK_SIDE, depth_or_array_layers: 1 },
        );
    }
}
