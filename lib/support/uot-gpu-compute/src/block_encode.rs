//! GPU block compression of frames and atlas pages, BC7 or BC1, for Dynamapper's mobiles' frame atlas (evaluated in
//! `tests.rs`; Dynamapper's BC7 frame atlas encodes its RIFE in-betweens with it, at UltraFast) and for a converter's
//! pages. BC7 keeps soft alpha (Enhanced, KR); BC1 keeps one bit of it, which is all Classic art and its in-betweens
//! have. The kernels are our copies of block_compression's (a WGSL port of Intel's ISPC Texture Compressor kernels,
//! MIT; CREDITS.md):
//!
//! - bc7.wgsl, its BC7 kernel with the modes a profile runs as constants ([`Bc7Profile`]): each quality, with alpha
//!   or opaque, is a pipeline of its own, which compiles only the modes it selects. The crate reads all its settings at
//!   run time, so its one pipeline holds every mode (about 21 s on a cold RADV cache). The settings of the arithmetic
//!   stay a buffer, as the crate's (see bc7.wgsl).
//! - bc1.wgsl, its BC1 kernel with BC1's one bit of alpha added (the crate's encodes colour only).
//!
//! The BC7 blocks are the crate's byte for byte, and the opaque BC1 blocks the crate's BC1 ones (`tests.rs`).
//! [`MobileBlockEncoder::compile`] compiles BC7 UltraFast with alpha, Dynamapper's format, and
//! [`MobileBlockEncoder::lazy`] nothing; every other format compiles the first time a frame of it is added, or ahead
//! with [`MobileBlockEncoder::compile_format`].
//!
//! A frame to encode lies at the left edge of a texture whose view is a non-sRGB RGBA8 format (`Rgba8Unorm`) holding
//! the frame's sRGB-encoded, straight-alpha bytes: an `Rgba8UnormSrgb` texture needs `Rgba8Unorm` among its
//! `view_formats`. The encoders work on those bytes as they are, so an `*UnormSrgb` BC texture decodes them to the same
//! sRGB bytes and samples them as the RGBA8 atlas does. The texture covers the frame padded to whole 4x4 blocks
//! ([`padded_size`]), the padding transparent. The blocks go to a storage buffer row by row of blocks, tightly, and
//! [`record_copy_to_layer`] copies them into a layer of a BC texture.
//!
//! The GPU's zero fill suits a BC7 atlas (a block of zeros has no valid mode and decodes to transparent black) but not
//! a BC1 one: zero endpoints and indices decode to opaque black, so empty BC1 pages are filled with
//! `BC1_TRANSPARENT_BLOCK`.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt::Write as _;
use wgpu::util::DeviceExt;
use crate::warm_up::CompileReport;

/// Texels per side of a block.
pub const BLOCK_SIDE: u32 = 4;
const BC7_BLOCK_BYTES: u32 = 16;
const BC1_BLOCK_BYTES: u32 = 8;
/// A transparent BC1 block: equal endpoints (the three-colour mode) and index 3 everywhere.
pub const BC1_TRANSPARENT_BLOCK: [u8; BC1_BLOCK_BYTES as usize] = [0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
const U32_BYTES: u32 = 4;
/// The workgroup size of bc7.wgsl and bc1.wgsl, in blocks per side.
const WORKGROUP_SIDE: u32 = 8;
/// A frame's uniforms (the kernels' `Uniforms`): its padded width and height, its first row in the texture and where
/// its blocks start in the output, in u32.
const FRAME_UNIFORM_WORDS: usize = 4;
const FRAME_UNIFORM_BYTES: u64 = (FRAME_UNIFORM_WORDS as u32 * U32_BYTES) as u64;

const BC7_SHADER: &str = include_str!("../shaders/block_encode/bc7.wgsl");
const BC7_ENTRY: &str = "compress_bc7";
const BC1_SHADER: &str = include_str!("../shaders/block_encode/bc1.wgsl");
const BC1_ENTRY: &str = "compress_bc1";
/// bc7.wgsl's settings buffer.
const SETTINGS_BINDING: u32 = 3;

/// The format [`MobileBlockEncoder::compile`] compiles: Dynamapper's frames'.
pub const WARM_BLOCK_FORMAT: MobileBlockFormat = MobileBlockFormat::Bc7(Bc7Quality::UltraFast);
/// The compiles [`MobileBlockEncoder::compile`] reports: [`WARM_BLOCK_FORMAT`]'s.
pub const BLOCK_ENCODER_COMPILES: usize = 1;

/// block_compression's BC7 settings profiles, fastest to slowest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bc7Quality {
    UltraFast,
    VeryFast,
    Fast,
    Basic,
    Slow,
}

impl Bc7Quality {
    pub const ALL: [Self; 5] = [Self::UltraFast, Self::VeryFast, Self::Fast, Self::Basic, Self::Slow];
}

/// A block format, and for BC7 the settings it encodes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MobileBlockFormat {
    /// 1 byte per texel, soft alpha.
    Bc7(Bc7Quality),
    /// BC7 of colour alone: the alpha is ignored and decodes opaque.
    Bc7Opaque(Bc7Quality),
    /// Half a byte per texel; a texel is opaque or transparent (alpha from one half up is opaque).
    Bc1,
}

impl MobileBlockFormat {
    /// Every format: BC7 with alpha at each quality, then opaque, then BC1.
    pub const ALL: [Self; 2 * Bc7Quality::ALL.len() + 1] = {
        let mut all = [Self::Bc1; 2 * Bc7Quality::ALL.len() + 1];
        let mut index = 0;
        while index < Bc7Quality::ALL.len() {
            all[index] = Self::Bc7(Bc7Quality::ALL[index]);
            all[Bc7Quality::ALL.len() + index] = Self::Bc7Opaque(Bc7Quality::ALL[index]);
            index += 1;
        }
        all
    };

    /// The texture format of an atlas of these blocks.
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bc7(_) | Self::Bc7Opaque(_) => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
            Self::Bc1 => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        }
    }

    pub fn block_bytes(self) -> u32 {
        match self {
            Self::Bc7(_) | Self::Bc7Opaque(_) => BC7_BLOCK_BYTES,
            Self::Bc1 => BC1_BLOCK_BYTES,
        }
    }

    /// Bytes of the blocks of a `width` by `height` frame.
    pub fn encoded_bytes(self, width: u32, height: u32) -> u64 {
        let (width, height) = padded_size(width, height);
        u64::from(width / BLOCK_SIDE) * u64::from(height / BLOCK_SIDE) * u64::from(self.block_bytes())
    }

    /// The name of its pipeline's compile, as reported.
    pub fn compile_name(self) -> String {
        match self {
            Self::Bc7(quality) => format!("BC7 encoder ({quality:?})"),
            Self::Bc7Opaque(quality) => format!("BC7 encoder ({quality:?}, opaque)"),
            Self::Bc1 => "BC1 encoder".to_string(),
        }
    }
}

/// A frame's size rounded up to whole blocks.
pub fn padded_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(BLOCK_SIDE) * BLOCK_SIDE, height.div_ceil(BLOCK_SIDE) * BLOCK_SIDE)
}

/// The settings of a BC7 profile, block_compression 0.11.0's `BC7Settings` (src/settings.rs) field by field: bc7.wgsl
/// reads `mode_selection` and `skip_mode2` as constants ([`Bc7Profile::wgsl_constants`]), and all of them from its
/// settings buffer ([`Bc7Profile::settings_words`]), as the crate's kernel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bc7Profile {
    pub refine_iterations: [u32; BC7_MODES],
    /// Modes 0 and 2, modes 1, 3 and 7, modes 4 and 5, mode 6.
    pub mode_selection: [bool; BC7_MODE_GROUPS],
    pub skip_mode2: bool,
    pub fast_skip_threshold_mode1: u32,
    pub fast_skip_threshold_mode3: u32,
    pub fast_skip_threshold_mode7: u32,
    /// The first channel modes 4 and 5 rotate into alpha (3, alpha itself, rotates none).
    pub mode45_channel0: u32,
    pub refine_iterations_channel: u32,
    /// 4 with alpha, 3 opaque.
    pub channels: u32,
}

const ALPHA_CHANNELS: u32 = 4;
const BC7_MODES: usize = 8;
const BC7_MODE_GROUPS: usize = 4;
/// The words of bc7.wgsl's `Settings`: the refinements by mode, the mode groups, then seven more.
pub const BC7_SETTINGS_WORDS: usize = BC7_MODES + BC7_MODE_GROUPS + 7;
const OPAQUE_CHANNELS: u32 = 3;

impl Bc7Profile {
    /// block_compression's `BC7Settings::alpha_*` and `opaque_*` constructors, by quality.
    pub const fn of(quality: Bc7Quality, alpha: bool) -> Self {
        let (t, f) = (true, false);
        match (alpha, quality) {
            (true, Bc7Quality::UltraFast) => Self::new(ALPHA_CHANNELS, [f, f, t, t], t, 0, 0, 4, 3, 1, [2, 1, 2, 1, 1, 1, 2, 2]),
            (true, Bc7Quality::VeryFast) => Self::new(ALPHA_CHANNELS, [f, t, t, t], t, 0, 0, 4, 3, 2, [2, 1, 2, 1, 2, 2, 2, 2]),
            (true, Bc7Quality::Fast) => Self::new(ALPHA_CHANNELS, [f, t, t, t], t, 4, 4, 8, 3, 2, [2, 1, 2, 1, 2, 2, 2, 2]),
            (true, Bc7Quality::Basic) => Self::new(ALPHA_CHANNELS, [t, t, t, t], t, 12, 8, 8, 0, 2, [2; 8]),
            (true, Bc7Quality::Slow) => Self::new(ALPHA_CHANNELS, [t, t, t, t], f, 64, 64, 64, 0, 4, [4; 8]),
            (false, Bc7Quality::UltraFast) => Self::new(OPAQUE_CHANNELS, [f, f, f, t], t, 3, 1, 0, 0, 0, [2, 2, 2, 1, 2, 2, 1, 0]),
            (false, Bc7Quality::VeryFast) => Self::new(OPAQUE_CHANNELS, [f, t, f, t], t, 3, 1, 0, 0, 0, [2, 2, 2, 1, 2, 2, 1, 0]),
            (false, Bc7Quality::Fast) => Self::new(OPAQUE_CHANNELS, [f, t, f, t], t, 12, 4, 0, 0, 0, [2, 2, 2, 1, 2, 2, 2, 0]),
            (false, Bc7Quality::Basic) => Self::new(OPAQUE_CHANNELS, [t, t, t, t], t, 12, 8, 0, 0, 2, [2, 2, 2, 2, 2, 2, 2, 0]),
            (false, Bc7Quality::Slow) => Self::new(OPAQUE_CHANNELS, [t, t, t, t], f, 64, 64, 0, 0, 4, [4, 4, 4, 4, 4, 4, 4, 0]),
        }
    }

    /// In the order of the crate's constructors' fields.
    #[allow(clippy::too_many_arguments)]
    const fn new(channels: u32, mode_selection: [bool; BC7_MODE_GROUPS], skip_mode2: bool,
        fast_skip_threshold_mode1: u32, fast_skip_threshold_mode3: u32, fast_skip_threshold_mode7: u32,
        mode45_channel0: u32, refine_iterations_channel: u32, refine_iterations: [u32; BC7_MODES]) -> Self {
        Self { refine_iterations, mode_selection, skip_mode2, fast_skip_threshold_mode1, fast_skip_threshold_mode3,
            fast_skip_threshold_mode7, mode45_channel0, refine_iterations_channel, channels }
    }

    /// The WGSL constants bc7.wgsl reads: which modes run.
    pub fn wgsl_constants(&self) -> String {
        let [modes_0_2, modes_1_3_7, modes_4_5, mode_6] = self.mode_selection;
        let mut constants = String::from("\n// The profile's modes (block_encode.rs, Bc7Profile).\n");
        writeln!(constants, "const MODES_0_2 = {modes_0_2};\nconst MODES_1_3_7 = {modes_1_3_7};").unwrap();
        writeln!(constants, "const MODES_4_5 = {modes_4_5};\nconst MODE_6 = {mode_6};").unwrap();
        writeln!(constants, "const SKIP_MODE_2 = {};", self.skip_mode2).unwrap();
        constants
    }

    /// The settings as bc7.wgsl's `Settings` holds them, in the layout of the crate's `BC7Settings`.
    pub fn settings_words(&self) -> [u32; BC7_SETTINGS_WORDS] {
        let mut words = [0; BC7_SETTINGS_WORDS];
        words[..BC7_MODES].copy_from_slice(&self.refine_iterations);
        for (word, &selected) in words[BC7_MODES..].iter_mut().zip(&self.mode_selection) {
            *word = u32::from(selected);
        }
        words[BC7_MODES + BC7_MODE_GROUPS..].copy_from_slice(&[u32::from(self.skip_mode2),
            self.fast_skip_threshold_mode1, self.fast_skip_threshold_mode3, self.fast_skip_threshold_mode7,
            self.mode45_channel0, self.refine_iterations_channel, self.channels]);
        words
    }
}

/// A kernel's pipeline: the source texture, the output blocks and the frame's uniforms, at a dynamic offset; for BC7,
/// the profile's settings too.
struct Kernel {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    settings: Option<wgpu::Buffer>,
}

impl Kernel {
    fn compile(device: &wgpu::Device, format: MobileBlockFormat) -> Self {
        let label = format.compile_name();
        let mut settings = None;
        let (module, entry_point) = match format {
            MobileBlockFormat::Bc7(quality) | MobileBlockFormat::Bc7Opaque(quality) => {
                let profile = Bc7Profile::of(quality, matches!(format, MobileBlockFormat::Bc7(_)));
                settings = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&label), contents: bytemuck::cast_slice(&profile.settings_words()),
                    usage: wgpu::BufferUsages::STORAGE,
                }));
                // block_compression's checks for its BC7 kernel, loop bounding kept: the crate found that without
                // it RADV/Navi can encode alpha and Basic wrongly. The checks left out concern ray queries, task and
                // mesh shaders, of which the kernel has none.
                let checks = wgpu::ShaderRuntimeChecks {
                    bounds_checks: true, force_loop_bounding: true, ray_query_initialization_tracking: false,
                    task_shader_dispatch_tracking: false, mesh_shader_primitive_indices_clamp: false,
                    int_div_checks: true,
                };
                let source = format!("{BC7_SHADER}{}", profile.wgsl_constants());
                let descriptor = wgpu::ShaderModuleDescriptor {
                    label: Some(&label), source: wgpu::ShaderSource::Wgsl(source.into()),
                };
                // SAFETY: the checks turned off guard ray queries and task and mesh shaders, which the module has none
                // of.
                (unsafe { device.create_shader_module_trusted(descriptor, checks) }, BC7_ENTRY)
            }
            // Checked, as block_compression creates its BC1 kernel.
            MobileBlockFormat::Bc1 => (device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&label), source: wgpu::ShaderSource::Wgsl(BC1_SHADER.into()),
            }), BC1_ENTRY),
        };
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None,
        };
        let mut entries = vec![
            entry(0, wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
            }),
            entry(1, wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false,
                min_binding_size: None,
            }),
            entry(2, wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(FRAME_UNIFORM_BYTES),
            }),
        ];
        if settings.is_some() {
            entries.push(entry(SETTINGS_BINDING, wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false,
                min_binding_size: None,
            }));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&label), entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&label), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&label), layout: Some(&pipeline_layout), module: &module, entry_point: Some(entry_point),
            compilation_options: Default::default(), cache: None,
        });
        Self { layout, pipeline, settings }
    }
}

/// A frame added for the next [`MobileBlockEncoder::record`].
struct Frame {
    format: MobileBlockFormat,
    source: wgpu::TextureView,
    output: wgpu::Buffer,
    uniforms: [u32; FRAME_UNIFORM_WORDS],
    workgroups: (u32, u32),
}

/// The encoders' pipelines compiled so far, and the frames recorded for the next [`MobileBlockEncoder::record`].
pub struct MobileBlockEncoder {
    device: wgpu::Device,
    kernels: HashMap<MobileBlockFormat, Kernel>,
    /// A frame's uniforms' stride in the buffer of a record: the device's dynamic offset alignment.
    uniform_stride: u64,
    frames: Vec<Frame>,
}

impl MobileBlockEncoder {
    /// Compiles the pipeline of [`WARM_BLOCK_FORMAT`]. The queue is not used (the frames' parameters go into a buffer
    /// at each record); it stays for the callers of the encoder this one replaced, which wrote them through it.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::compile(device, queue, &mut CompileReport::silent(BLOCK_ENCODER_COMPILES))
    }

    /// [`MobileBlockEncoder::new`], telling `report` of each of [`BLOCK_ENCODER_COMPILES`].
    pub fn compile(device: &wgpu::Device, _queue: &wgpu::Queue, report: &mut CompileReport) -> Self {
        let mut encoder = Self::lazy(device);
        encoder.compile_format(WARM_BLOCK_FORMAT, report);
        encoder
    }

    /// An encoder with no pipeline compiled: each format compiles at its first frame, so a converter pays only for the
    /// formats its pages take.
    pub fn lazy(device: &wgpu::Device) -> Self {
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        Self {
            device: device.clone(), kernels: HashMap::new(),
            uniform_stride: FRAME_UNIFORM_BYTES.div_ceil(alignment) * alignment, frames: Vec::new(),
        }
    }

    /// Compiles `format`'s pipeline unless it is compiled already, telling `report` of it (one compile) if it compiles.
    /// [`MobileBlockEncoder::add_frame`] compiles a format not compiled yet itself, on the caller's thread: a slow BC7
    /// profile with every mode can take tens of seconds on a cold driver cache.
    pub fn compile_format(&mut self, format: MobileBlockFormat, report: &mut CompileReport) {
        if !self.kernels.contains_key(&format) {
            let kernel = report.compile(&format.compile_name(), || Kernel::compile(&self.device, format));
            self.kernels.insert(format, kernel);
        }
    }

    /// Whether `format`'s pipeline is compiled.
    pub fn is_compiled(&self, format: MobileBlockFormat) -> bool {
        self.kernels.contains_key(&format)
    }

    /// Adds the encoding of a `width` by `height` frame at the top-left corner of `source` (see the module doc) to
    /// the next [`MobileBlockEncoder::record`]; its blocks go to `output`, a storage buffer, from byte `offset` (a
    /// multiple of 4).
    pub fn add_frame(&mut self, format: MobileBlockFormat, source: &wgpu::TextureView, width: u32, height: u32,
        output: &wgpu::Buffer, offset: u64) {
        self.add_frame_at(format, source, 0, width, height, output, offset);
    }

    /// [`MobileBlockEncoder::add_frame`] for a frame at row `y` (a multiple of [`BLOCK_SIDE`]) of `source`'s left
    /// edge, so frames stacked in one texture encode in one [`MobileBlockEncoder::record`].
    #[allow(clippy::too_many_arguments)]
    pub fn add_frame_at(&mut self, format: MobileBlockFormat, source: &wgpu::TextureView, y: u32, width: u32,
        height: u32, output: &wgpu::Buffer, offset: u64) {
        assert!(y % BLOCK_SIDE == 0, "a frame starts on a block row");
        assert!(output.usage().contains(wgpu::BufferUsages::STORAGE), "the blocks go to a storage buffer");
        assert!(offset + format.encoded_bytes(width, height) <= output.size(), "the blocks fit the buffer");
        self.compile_format(format, &mut CompileReport::silent(1));
        let (width, height) = padded_size(width, height);
        let blocks_offset = u32::try_from(offset / u64::from(U32_BYTES)).expect("block offsets fit u32");
        let workgroups = |side: u32| (side / BLOCK_SIDE).div_ceil(WORKGROUP_SIDE);
        self.frames.push(Frame {
            format, source: source.clone(), output: output.clone(), uniforms: [width, height, y, blocks_offset],
            workgroups: (workgroups(width), workgroups(height)),
        });
    }

    /// Records the encoding of the frames added since the last call into `pass`. Their parameters go into a buffer of
    /// this record's own, so several records may share a submission.
    pub fn record(&mut self, pass: &mut wgpu::ComputePass<'_>) {
        if self.frames.is_empty() {
            return;
        }
        let stride = self.uniform_stride as usize;
        let mut contents = vec![0u8; self.frames.len() * stride];
        for (frame, slot) in self.frames.iter().zip(contents.chunks_exact_mut(stride)) {
            slot[..FRAME_UNIFORM_BYTES as usize].copy_from_slice(bytemuck::cast_slice(&frame.uniforms));
        }
        let uniforms = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("block encoder frames"), contents: &contents, usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut bound = None;
        for (index, frame) in self.frames.drain(..).enumerate() {
            let kernel = &self.kernels[&frame.format];
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame.source) },
                wgpu::BindGroupEntry { binding: 1, resource: frame.output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms, offset: 0, size: wgpu::BufferSize::new(FRAME_UNIFORM_BYTES),
                }) },
            ];
            if let Some(settings) = &kernel.settings {
                entries.push(wgpu::BindGroupEntry {
                    binding: SETTINGS_BINDING, resource: settings.as_entire_binding(),
                });
            }
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("block encoder frame"), layout: &kernel.layout, entries: &entries,
            });
            if bound != Some(frame.format) {
                pass.set_pipeline(&kernel.pipeline);
                bound = Some(frame.format);
            }
            let dynamic_offset = u32::try_from(index * stride).expect("uniform offsets fit u32");
            pass.set_bind_group(0, &bind_group, &[dynamic_offset]);
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
