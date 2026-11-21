//! The BC7 encoding of the frames the GPU makes for the BC7 frame atlas (frame_atlas.rs), shared in the render world by
//! the RIFE in-betweens and the SPAN upscales: the encoder of the warm-up (`MobileBlockEncoder`, BC7 UltraFast), a
//! scratch `Rgba8Unorm` texture the frames wait in, row by row, the blocks they encode to, and the bounce buffer of
//! the in-betweens' first frames' block copies. The encoder takes its parameters through the queue, so each user
//! records one `compress` per submission of its own, after which the scratch rows are free again.

use bevy::prelude::*;
use crate::app::background::{background_pool, Priority, Task, TaskPoll};
use crate::render::shader_compiles::begin_shader_compile;
use uot_gpu_compute::block_encode::{Bc7Quality, MobileBlockEncoder, MobileBlockFormat};
use super::compute_warm_up::{ComputeWarmUp, Warm};
use super::frame_atlas::MOBILE_ATLAS_PAGE_SIZE;

/// The quality of the frames the GPU encodes.
const MOBILE_BC7_QUALITY: Bc7Quality = Bc7Quality::UltraFast;
pub const MOBILE_BLOCK_FORMAT: MobileBlockFormat = MobileBlockFormat::Bc7(MOBILE_BC7_QUALITY);
/// The scratch texture of the frames to encode, a frame's rows: a page's side square, so any frame fits.
pub const BC7_SCRATCH_SIDE: u32 = MOBILE_ATLAS_PAGE_SIZE;
/// The bounce buffer of the first frames' block copies: as large as a page's blocks.
pub const BC7_BOUNCE_BYTES: u64 = MOBILE_ATLAS_PAGE_SIZE as u64 * MOBILE_ATLAS_PAGE_SIZE as u64;

/// The BC7 encode of a frame, estimated: a fixed part and a part per million pixels (an encode alone on the Radeon
/// 880M: 0.27 ms at 64x64 and 128x128, 0.53 ms at 256x256; see mobile_animation_motion.md).
const BC7_ENCODE_MS: f64 = 0.25;
const BC7_ENCODE_MS_PER_MEGAPIXEL: f64 = 4.0;
const MEGA: f64 = 1e6;

/// The estimated GPU time of the BC7 encode of a frame of `pixels`, padded to whole blocks.
pub fn bc7_encode_ms(pixels: u32) -> f64 {
    BC7_ENCODE_MS + pixels as f64 / MEGA * BC7_ENCODE_MS_PER_MEGAPIXEL
}

/// The encoder and its buffers.
pub struct Bc7Encoding {
    pub encoder: MobileBlockEncoder,
    pub scratch: wgpu::Texture,
    pub scratch_view: wgpu::TextureView,
    pub blocks: wgpu::Buffer,
    pub bounce: wgpu::Buffer,
}

impl Bc7Encoding {
    fn new(device: &wgpu::Device, encoder: MobileBlockEncoder) -> Self {
        let scratch = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mobile frames to encode"),
            size: wgpu::Extent3d { width: BC7_SCRATCH_SIDE, height: BC7_SCRATCH_SIDE, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());
        let buffer = |label, size, usage| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size, usage, mapped_at_creation: false,
        });
        Self {
            encoder,
            scratch,
            scratch_view,
            blocks: buffer("mobile frame blocks", MOBILE_BLOCK_FORMAT.encoded_bytes(BC7_SCRATCH_SIDE, BC7_SCRATCH_SIDE),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
            bounce: buffer("mobile first frame blocks", BC7_BOUNCE_BYTES,
                wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST),
        }
    }
}

/// Where the blocks of the frame at scratch row `y` go: after the blocks of whole scratch rows above it, which those
/// of the frames above never exceed.
pub fn scratch_blocks_offset(y: u32) -> u64 {
    MOBILE_BLOCK_FORMAT.encoded_bytes(BC7_SCRATCH_SIDE, y)
}

#[derive(Default)]
enum EncoderState {
    #[default]
    Unloaded,
    Compiling(Task<MobileBlockEncoder>),
    Ready(Box<Bc7Encoding>),
    /// Logged; the clips of the BC7 atlas play without the frames the GPU makes.
    Failed,
}

/// The render world's BC7 encoding, made the first time a user wants it.
#[derive(Resource, Default)]
pub struct MobileBc7Encoder {
    state: EncoderState,
}

impl MobileBc7Encoder {
    /// Starts the encoder once `wanted` (the warm-up's, or one compiled on the background pool when the warm-up has
    /// none), and takes it once compiled.
    pub fn poll(&mut self, wanted: bool, warm_up: &mut ComputeWarmUp, device: &wgpu::Device, queue: &wgpu::Queue) {
        if let EncoderState::Unloaded = self.state && wanted {
            self.state = match warm_up.take_block_encoder() {
                Warm::Compiling => return,
                Warm::Ready(encoder) => EncoderState::Ready(Box::new(Bc7Encoding::new(device, encoder))),
                Warm::Unavailable => {
                    let (device, queue) = (device.clone(), queue.clone());
                    EncoderState::Compiling(background_pool().spawn(Priority::Visible, "bc7 block encoder", move || {
                        let _compile = begin_shader_compile("Mobile frames: BC7 encoder");
                        MobileBlockEncoder::new(&device, &queue)
                    }))
                }
            };
        }
        if let EncoderState::Compiling(task) = &mut self.state {
            self.state = match task.poll() {
                TaskPoll::Pending => return,
                TaskPoll::Ready(encoder) => EncoderState::Ready(Box::new(Bc7Encoding::new(device, encoder))),
                TaskPoll::Failed => {
                    error!("The BC7 encoder did not compile, so the clips of the BC7 atlas play without in-betweens \
                        or upscales.");
                    EncoderState::Failed
                }
            };
        }
    }

    pub fn ready(&self) -> Option<&Bc7Encoding> {
        match &self.state {
            EncoderState::Ready(encoding) => Some(encoding),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut Bc7Encoding> {
        match &mut self.state {
            EncoderState::Ready(encoding) => Some(encoding),
            _ => None,
        }
    }
}
