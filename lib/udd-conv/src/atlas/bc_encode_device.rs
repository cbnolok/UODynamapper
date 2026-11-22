//! Where the packers' BC7 page encode runs: on the GPU (uot_gpu_compute::page_encode) when a wgpu adapter is there,
//! on the CPU otherwise, or on the one the run asks for ([`set_bc_encode_device`]). The GPU encodes at
//! [`GPU_BC7_QUALITY`], the profile whose quality matches the CPU encoder's, with blocks of its own; both give the
//! same blocks from run to run. The device starts at the first page encoded and stays for the process; the pages
//! encoded at once share it. The BC7 RDO runs after the encode, on its own device (`bc7_rdo_device`). BC1 stays on
//! the CPU.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use crate::atlas::bc7_rdo_device::on_bc7_rdo_device;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::page_encode::Bc7PageEncoderGpu;
use uot_texture_codecs::bc7::{
    encode_to_bc7_with_rdo_options_and_stage_progress, preferred_bc7_encoder_backend, Bc7EncodeAccelerator,
    Bc7RdoOptions, ImageExtent, RawImageFormat, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, NO_BC7_RDO_LAMBDA,
};

/// Bytes of a BC7 block.
const BLOCK_BYTES: usize = 16;

/// The GPU encoder's profile: the one whose quality matches the CPU encoder's on the mobile animation, art and land
/// pages (docs/agent_notes/atlas_page_compression.md, "BC7 encode on the GPU").
pub const GPU_BC7_QUALITY: Bc7Quality = Bc7Quality::UltraFast;

/// Where the BC7 page encode runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BcEncodeDevice {
    /// The GPU when an adapter is there, the CPU otherwise (and when the GPU fails).
    #[default]
    Auto = 0,
    Cpu = 1,
    /// The GPU, failing the pack without one.
    Gpu = 2,
}

impl BcEncodeDevice {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Cpu,
            2 => Self::Gpu,
            _ => Self::Auto,
        }
    }
}

/// Where the encode runs until [`set_bc_encode_device`]: the crate's own tests stay off the GPU, like its other GPU
/// work.
const INITIAL_DEVICE: BcEncodeDevice = if cfg!(test) { BcEncodeDevice::Cpu } else { BcEncodeDevice::Auto };

static DEVICE: AtomicU8 = AtomicU8::new(INITIAL_DEVICE as u8);

/// Sets where the packers' BC7 page encode runs (Auto by default, the CPU in this crate's tests).
pub fn set_bc_encode_device(device: BcEncodeDevice) {
    DEVICE.store(device as u8, Ordering::Relaxed);
}

/// Where the packers' BC7 page encode runs.
pub fn bc_encode_device() -> BcEncodeDevice {
    BcEncodeDevice::from_u8(DEVICE.load(Ordering::Relaxed))
}

/// `options` with the page encoded on the device [`bc_encode_device`] names.
pub fn on_bc_encode_device(options: Bc7RdoOptions) -> Bc7RdoOptions {
    let encoder: Option<Arc<dyn Bc7EncodeAccelerator>> = match bc_encode_device() {
        BcEncodeDevice::Cpu => None,
        BcEncodeDevice::Auto => Some(Arc::new(GpuEncode { fall_back: true })),
        BcEncodeDevice::Gpu => Some(Arc::new(GpuEncode { fall_back: false })),
    };
    options.with_encoder(encoder)
}

/// `options` with the encode on the device [`bc_encode_device`] names and the RDO on the one
/// [`bc7_rdo_device`](crate::atlas::bc7_rdo_device::bc7_rdo_device) names: what the packers encode their pages with.
pub fn on_bc7_devices(options: Bc7RdoOptions) -> Bc7RdoOptions {
    on_bc_encode_device(on_bc7_rdo_device(options))
}

/// The process's GPU encoder, or why there is none.
fn gpu() -> &'static Result<Bc7PageEncoderGpu, String> {
    static GPU: OnceLock<Result<Bc7PageEncoderGpu, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        let gpu = Bc7PageEncoderGpu::new().map_err(|error| format!("{error:#}"));
        match &gpu {
            Ok(gpu) => log::info!("BC7 encode on the GPU: {}", gpu.adapter()),
            Err(error) => log::warn!("BC7 encode: no GPU ({error})"),
        }
        gpu
    })
}

/// The GPU encode, falling back on the CPU's when told to.
struct GpuEncode {
    fall_back: bool,
}

impl Bc7EncodeAccelerator for GpuEncode {
    fn encode(&self, rgba: &[u8], extent: ImageExtent, progress: &(dyn Fn(usize) + Sync))
        -> Result<Vec<[u8; BLOCK_BYTES]>, String> {
        let result = match gpu() {
            Ok(gpu) => gpu.encode(rgba, extent.width(), extent.height(), GPU_BC7_QUALITY)
                .map_err(|error| format!("{error:#}")),
            Err(error) => Err(error.clone()),
        };
        match result {
            Ok(blocks) => {
                progress(blocks.len());
                Ok(blocks)
            }
            Err(error) if self.fall_back => {
                if gpu().is_ok() {
                    log::warn!("BC7 encode on the GPU failed, the CPU encodes the page: {error}");
                }
                let plain = Bc7RdoOptions::new(NO_BC7_RDO_LAMBDA, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS);
                let texture = encode_to_bc7_with_rdo_options_and_stage_progress(rgba, extent,
                    RawImageFormat::Rgba8888, preferred_bc7_encoder_backend(), &plain, |_, done| progress(done))
                    .map_err(|error| error.to_string())?;
                Ok(texture.blocks().chunks_exact(BLOCK_BYTES)
                    .map(|block| block.try_into().expect("whole BC7 blocks")).collect())
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's tests start on the CPU, where every packer test encodes, and the device takes each setting.
    #[test]
    fn the_encode_starts_on_the_cpu_in_tests_and_takes_each_device() {
        assert_eq!(INITIAL_DEVICE, BcEncodeDevice::Cpu);
        for device in [BcEncodeDevice::Auto, BcEncodeDevice::Cpu, BcEncodeDevice::Gpu] {
            assert_eq!(BcEncodeDevice::from_u8(device as u8), device);
        }
        assert!(on_bc_encode_device(Bc7RdoOptions::default()).encoder.is_none());
    }
}
