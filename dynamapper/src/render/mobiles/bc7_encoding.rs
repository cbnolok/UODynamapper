//! The BC7 encoding of the frames the GPU makes for the BC7 frame atlas (frame_atlas.rs), shared in the render world by
//! the RIFE in-betweens and the SPAN upscales: the encoder of the warm-up (`MobileBlockEncoder`, at the BC7 profile
//! `world_rendering.mobile_bc7_quality` chooses), a scratch `Rgba8Unorm` texture the frames wait in, row by row, the
//! blocks they encode to, and the bounce buffer of the in-betweens' first frames' block copies. Each user records one
//! `compress` per submission of its own, after which the scratch rows are free again.
//!
//! A profile chosen while the encoder is ready compiles on a thread of its own, shown in the shader-compile overlay,
//! while the encoder ready goes on encoding; the new one takes its place once compiled (`switch_step`). The frames
//! already in the atlas keep the blocks they were encoded with and age out as the atlas evicts them: a flush would
//! drop every upscale and in-between on screen back to the stored frames and queue them all again, a visible step and
//! a crowd's backlog, for a difference only a 4x view shows.

use std::thread::JoinHandle;
use std::time::Instant;
use bevy::prelude::*;
use bevy::render::Extract;
use serde::{Deserialize, Serialize};
use crate::render::shader_compiles::begin_shader_compile;
use crate::settings::Settings;
use uot_gpu_compute::block_encode::{Bc7Quality, MobileBlockEncoder, MobileBlockFormat};
use uot_gpu_compute::warm_up::CompileReport;
use super::compute_warm_up::{ComputeWarmUp, Warm};
use super::frame_atlas::MOBILE_ATLAS_PAGE_SIZE;

/// The block layout of the frames the GPU encodes, for sizes and copies: a BC7 profile changes how a block is chosen,
/// not its size, so every `MobileBc7Quality` shares it (the encoder encodes at `Bc7Encoding::quality`).
pub const MOBILE_BLOCK_FORMAT: MobileBlockFormat = MobileBlockFormat::Bc7(Bc7Quality::UltraFast);
/// The scratch texture of the frames to encode, a frame's rows: a page's side square, so any frame fits.
pub const BC7_SCRATCH_SIDE: u32 = MOBILE_ATLAS_PAGE_SIZE;
/// The bounce buffer of the first frames' block copies: as large as a page's blocks.
pub const BC7_BOUNCE_BYTES: u64 = MOBILE_ATLAS_PAGE_SIZE as u64 * MOBILE_ATLAS_PAGE_SIZE as u64;

/// The BC7 encode of a frame at UltraFast, estimated: a fixed part and a part per million pixels (an encode alone on
/// the Radeon 880M: 0.27 ms at 64x64 and 128x128, 0.53 ms at 256x256; see mobile_animation_motion.md).
const BC7_ENCODE_MS: f64 = 0.25;
const BC7_ENCODE_MS_PER_MEGAPIXEL: f64 = 4.0;
/// The same at VeryFast, measured the same way and on the SPAN 2x frames (2026-11-28: 1.02 to 1.06 ms up to 256x256,
/// 2.05 ms at 113k and 147k pixels; a least-squares fit 0.87 + 9.1, these bound it as UltraFast's do; see
/// mobile_animation_motion.md, "Choosing the profile").
const BC7_VERY_FAST_ENCODE_MS: f64 = 1.05;
const BC7_VERY_FAST_ENCODE_MS_PER_MEGAPIXEL: f64 = 9.5;
const MEGA: f64 = 1e6;
/// The thread a profile chosen at run time compiles on.
const ENCODER_THREAD_NAME: &str = "bc7 encoder compile";

/// The BC7 profile the GPU encodes the frames it makes with, the RIFE in-betweens' and the SPAN upscales' alike. Kept
/// in the app-written world rendering settings as `world_rendering.mobile_bc7_quality`; the bundled
/// settings/world_rendering.toml holds the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileBc7Quality {
    /// Modes 4/5 and 6: compiles in about 5.7 s on a cold driver cache, encodes a large frame in about 0.35 ms.
    #[default]
    UltraFast,
    /// Modes 1/3/7 too: about 14.5 s to compile cold and 5 to 6 times UltraFast's GPU time a frame, for 2-3 dB on
    /// large Enhanced bodies (mobile_animation_motion.md, "BC7 profiles on SPAN 2x frames").
    VeryFast,
}

impl MobileBc7Quality {
    pub const ALL: [Self; 2] = [Self::UltraFast, Self::VeryFast];

    pub fn label(self) -> &'static str {
        match self {
            Self::UltraFast => "UltraFast",
            Self::VeryFast => "VeryFast",
        }
    }

    /// The encoder's format: BC7 with alpha at this profile.
    pub fn format(self) -> MobileBlockFormat {
        MobileBlockFormat::Bc7(match self {
            Self::UltraFast => Bc7Quality::UltraFast,
            Self::VeryFast => Bc7Quality::VeryFast,
        })
    }

    /// The estimated GPU time of the BC7 encode of a frame of `pixels`, padded to whole blocks.
    pub fn encode_ms(self, pixels: u32) -> f64 {
        let (fixed, per_megapixel) = match self {
            Self::UltraFast => (BC7_ENCODE_MS, BC7_ENCODE_MS_PER_MEGAPIXEL),
            Self::VeryFast => (BC7_VERY_FAST_ENCODE_MS, BC7_VERY_FAST_ENCODE_MS_PER_MEGAPIXEL),
        };
        fixed + pixels as f64 / MEGA * per_megapixel
    }
}

/// An encoder of `quality` alone, compiled, telling `report` of its one compile.
pub fn compile_encoder(device: &wgpu::Device, quality: MobileBc7Quality, report: &mut CompileReport)
    -> MobileBlockEncoder {
    let mut encoder = MobileBlockEncoder::lazy(device);
    encoder.compile_format(quality.format(), report);
    encoder
}

/// The encoder and its buffers.
pub struct Bc7Encoding {
    pub encoder: MobileBlockEncoder,
    /// The profile `encoder` encodes at.
    pub quality: MobileBc7Quality,
    pub scratch: wgpu::Texture,
    pub scratch_view: wgpu::TextureView,
    pub blocks: wgpu::Buffer,
    pub bounce: wgpu::Buffer,
}

impl Bc7Encoding {
    fn new(device: &wgpu::Device, encoder: MobileBlockEncoder, quality: MobileBc7Quality) -> Self {
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
            quality,
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

/// An encoder compiling on a thread of its own: a cold driver cache takes seconds (VeryFast about 14.5 s), which would
/// hold a worker of the background pool (whose jobs are never interrupted) all that time. Dropped before it ends, its
/// thread compiles on and its encoder is dropped.
struct EncoderCompile {
    quality: MobileBc7Quality,
    thread: JoinHandle<MobileBlockEncoder>,
}

impl EncoderCompile {
    /// Starts the compile of `quality`'s encoder; `None`, logged, when the thread did not start.
    fn start(device: &wgpu::Device, quality: MobileBc7Quality) -> Option<Self> {
        let device = device.clone();
        let spawned = std::thread::Builder::new().name(ENCODER_THREAD_NAME.to_string()).spawn(move || {
            let started = Instant::now();
            let encoder = {
                let _compile = begin_shader_compile(format!("Mobile frames: {}", quality.format().compile_name()));
                compile_encoder(&device, quality, &mut CompileReport::silent(1))
            };
            info!("The BC7 block encoder ({}) is compiled, in {:.1} s.", quality.label(),
                started.elapsed().as_secs_f32());
            encoder
        });
        match spawned {
            Ok(thread) => Some(Self { quality, thread }),
            Err(error) => {
                error!("The {ENCODER_THREAD_NAME} did not start: {error}");
                None
            }
        }
    }

    fn finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// The encoder, once `finished`; `None` when the thread panicked.
    fn join(self) -> Option<MobileBlockEncoder> {
        self.thread.join().ok()
    }
}

#[derive(Default)]
enum EncoderState {
    #[default]
    Unloaded,
    Compiling(EncoderCompile),
    Ready(Box<Bc7Encoding>),
    /// Logged; the clips of the BC7 atlas play without the frames the GPU makes.
    Failed,
}

/// The encoder of a profile chosen while another one is ready.
#[derive(Default)]
enum NextEncoder {
    #[default]
    Idle,
    Compiling(EncoderCompile),
    /// Its compile failed (logged): the encoder ready stays until another profile is chosen.
    Failed(MobileBc7Quality),
}

/// What `NextEncoder` holds, as `switch_step` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NextView {
    Idle,
    Compiling { quality: MobileBc7Quality, finished: bool },
    Failed(MobileBc7Quality),
}

impl NextEncoder {
    fn view(&self) -> NextView {
        match self {
            Self::Idle => NextView::Idle,
            Self::Compiling(compile) => NextView::Compiling { quality: compile.quality, finished: compile.finished() },
            Self::Failed(quality) => NextView::Failed(*quality),
        }
    }
}

/// What the switch to the profile chosen does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwitchStep {
    /// Nothing: the encoder ready encodes at the profile chosen, or the profile chosen failed to compile.
    Stay,
    /// Compile the profile chosen, in place of any other compile.
    Compile,
    /// The profile chosen compiles: the encoder ready goes on encoding.
    Wait,
    /// The profile chosen compiled: it takes the place of the encoder ready.
    Swap,
    /// The profile ready is chosen again: what the switch held goes.
    Forget,
}

/// The switch's next step, from the profile `ready` encodes at, the profile `chosen` and the `next` encoder.
fn switch_step(ready: MobileBc7Quality, chosen: MobileBc7Quality, next: NextView) -> SwitchStep {
    match next {
        NextView::Idle if chosen == ready => SwitchStep::Stay,
        _ if chosen == ready => SwitchStep::Forget,
        NextView::Compiling { quality, finished } if quality == chosen =>
            if finished { SwitchStep::Swap } else { SwitchStep::Wait },
        NextView::Failed(quality) if quality == chosen => SwitchStep::Stay,
        NextView::Idle | NextView::Compiling { .. } | NextView::Failed(_) => SwitchStep::Compile,
    }
}

/// The render world's BC7 encoding, made the first time a user wants it.
#[derive(Resource, Default)]
pub struct MobileBc7Encoder {
    state: EncoderState,
    /// The profile the settings choose, as extracted.
    chosen: MobileBc7Quality,
    next: NextEncoder,
}

/// Notes the profile the settings choose.
pub fn sys_extract_mobile_bc7_quality(settings: Extract<Option<Res<Settings>>>, mut bc7: ResMut<MobileBc7Encoder>) {
    if let Some(settings) = settings.as_ref() {
        bc7.chosen = settings.world_rendering.mobile_bc7_quality;
    }
}

impl MobileBc7Encoder {
    /// Starts the encoder once `wanted` (the warm-up's, or one compiled on a thread of its own when the warm-up has
    /// none), takes it once compiled, and switches it to the profile chosen.
    pub fn poll(&mut self, wanted: bool, warm_up: &mut ComputeWarmUp, device: &wgpu::Device) {
        if let EncoderState::Unloaded = self.state && wanted {
            self.state = match warm_up.take_block_encoder() {
                Warm::Compiling => return,
                Warm::Ready((quality, encoder)) =>
                    EncoderState::Ready(Box::new(Bc7Encoding::new(device, encoder, quality))),
                Warm::Unavailable =>
                    EncoderCompile::start(device, self.chosen).map_or(EncoderState::Failed, EncoderState::Compiling),
            };
        }
        if let EncoderState::Compiling(compile) = &self.state {
            if !compile.finished() {
                return;
            }
            let EncoderState::Compiling(compile) = std::mem::take(&mut self.state) else { unreachable!() };
            let quality = compile.quality;
            self.state = match compile.join() {
                Some(encoder) => EncoderState::Ready(Box::new(Bc7Encoding::new(device, encoder, quality))),
                None => {
                    error!("The BC7 encoder did not compile, so the clips of the BC7 atlas play without in-betweens \
                        or upscales.");
                    EncoderState::Failed
                }
            };
        }
        let EncoderState::Ready(encoding) = &mut self.state else { return };
        match switch_step(encoding.quality, self.chosen, self.next.view()) {
            SwitchStep::Stay | SwitchStep::Wait => {}
            SwitchStep::Compile => {
                self.next = EncoderCompile::start(device, self.chosen)
                    .map_or(NextEncoder::Failed(self.chosen), NextEncoder::Compiling);
            }
            SwitchStep::Swap => {
                let NextEncoder::Compiling(compile) = std::mem::take(&mut self.next) else { unreachable!() };
                let quality = compile.quality;
                match compile.join() {
                    Some(encoder) => {
                        info!("The mobile frames the GPU makes encode at BC7 {} from now on; those in the atlas keep \
                            their blocks until evicted.", quality.label());
                        encoding.encoder = encoder;
                        encoding.quality = quality;
                    }
                    None => {
                        error!("The BC7 encoder ({}) did not compile, so the frames go on encoding at {}.",
                            quality.label(), encoding.quality.label());
                        self.next = NextEncoder::Failed(quality);
                    }
                }
            }
            SwitchStep::Forget => self.next = NextEncoder::Idle,
        }
    }

    /// The profile the frames encode at: the encoder ready's, else the one chosen.
    pub fn quality(&self) -> MobileBc7Quality {
        match &self.state {
            EncoderState::Ready(encoding) => encoding.quality,
            _ => self.chosen,
        }
    }

    /// The profile the settings choose, as extracted: the one the warm-up compiles.
    pub fn chosen(&self) -> MobileBc7Quality {
        self.chosen
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

#[cfg(test)]
mod tests {
    use super::*;
    use MobileBc7Quality::{UltraFast, VeryFast};

    /// A profile chosen compiles while the one ready encodes, and takes its place once compiled.
    #[test]
    fn a_profile_chosen_compiles_then_takes_the_encoders_place() {
        assert_eq!(switch_step(UltraFast, UltraFast, NextView::Idle), SwitchStep::Stay);
        assert_eq!(switch_step(UltraFast, VeryFast, NextView::Idle), SwitchStep::Compile);
        assert_eq!(switch_step(UltraFast, VeryFast, NextView::Compiling { quality: VeryFast, finished: false }),
            SwitchStep::Wait);
        assert_eq!(switch_step(UltraFast, VeryFast, NextView::Compiling { quality: VeryFast, finished: true }),
            SwitchStep::Swap);
        assert_eq!(switch_step(VeryFast, VeryFast, NextView::Idle), SwitchStep::Stay);
    }

    /// Choosing the profile ready again drops the compile or the failure, a failed compile is not retried until
    /// another profile is chosen, and a compile of a profile no longer chosen gives way to the one chosen.
    #[test]
    fn a_switch_undone_or_failed_keeps_the_encoder_ready() {
        for finished in [false, true] {
            assert_eq!(switch_step(UltraFast, UltraFast, NextView::Compiling { quality: VeryFast, finished }),
                SwitchStep::Forget);
        }
        assert_eq!(switch_step(UltraFast, VeryFast, NextView::Failed(VeryFast)), SwitchStep::Stay);
        assert_eq!(switch_step(UltraFast, UltraFast, NextView::Failed(VeryFast)), SwitchStep::Forget);
        assert_eq!(switch_step(VeryFast, UltraFast, NextView::Compiling { quality: VeryFast, finished: true }),
            SwitchStep::Compile);
        assert_eq!(switch_step(VeryFast, UltraFast, NextView::Failed(VeryFast)), SwitchStep::Compile);
    }

    /// Both profiles encode blocks of one layout, and VeryFast is estimated to cost more.
    #[test]
    fn the_profiles_share_the_block_layout() {
        for quality in MobileBc7Quality::ALL {
            assert_eq!(quality.format().block_bytes(), MOBILE_BLOCK_FORMAT.block_bytes());
            assert_eq!(quality.format().texture_format(), MOBILE_BLOCK_FORMAT.texture_format());
        }
        for pixels in [0, 64 * 64, 256 * 256, 512 * 512] {
            assert!(VeryFast.encode_ms(pixels) > UltraFast.encode_ms(pixels));
        }
    }
}
