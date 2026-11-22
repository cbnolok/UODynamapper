//! Where the packers' BC7 page encode and BC1 gump encode run ([`set_bc_encode_device`]): on the CPU, on the GPU
//! (uot_gpu_compute::page_encode), or by default (Auto) the BC1 on the GPU when a wgpu adapter is there and the BC7 on
//! the CPU (the user's choice until the optimized GPU BC7 encoder is weighed: its whole packs now come out as small
//! after Zstd as the CPU's). The GPU encodes BC7 at [`GPU_BC7_QUALITY`], the profile whose quality matches the CPU
//! encoder's, with blocks of its own; both give the same blocks from run to run. The GPU's BC1
//! ([`encode_bc1_on_device`]: the single Classic gumps) keeps the CPU's one bit of alpha on every texel, ten times
//! faster, half a dB below its colour (docs/agent_notes/atlas_page_compression.md, "GPU block encoding"). The device
//! starts at the first image encoded and stays for the process, compiling each format's pipeline at its first image;
//! the images encoded at once share it.
//! The BC7 RDO runs after the encode, on its own device (`bc7_rdo_device`).

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use crate::atlas::bc7_rdo_device::on_bc7_rdo_device;
use uot_gpu_compute::block_encode::Bc7Quality;
use uot_gpu_compute::page_encode::PageEncoderGpu;
use uot_texture_codecs::bc1::encode_bc1;
use uot_texture_codecs::bc7::{
    encode_to_bc7_with_rdo_options_and_stage_progress, preferred_bc7_encoder_backend, Bc7EncodeAccelerator,
    Bc7RdoOptions, ImageExtent, RawImageFormat, DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, NO_BC7_RDO_LAMBDA,
};

/// Bytes of a BC7 block.
const BLOCK_BYTES: usize = 16;

/// The GPU encoder's profile: the one whose quality matches the CPU encoder's on the mobile animation, art and land
/// pages (docs/agent_notes/atlas_page_compression.md, "BC7 encode on the GPU").
pub const GPU_BC7_QUALITY: Bc7Quality = Bc7Quality::UltraFast;

/// Where the BC7 page encode and the BC1 gump encode run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BcEncodeDevice {
    /// The BC1 on the GPU when an adapter is there, the CPU otherwise (and when the GPU fails); the BC7 on the CPU.
    #[default]
    Auto = 0,
    Cpu = 1,
    /// The GPU for both, failing the pack without one.
    Gpu = 2,
}

/// The formats the device setting places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EncodedFormat {
    Bc7,
    Bc1,
}

/// Where the images of one format are encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EncodeOn {
    Cpu,
    /// The GPU, the CPU when there is none or it fails.
    GpuOrCpu,
    /// The GPU, failing without one.
    Gpu,
}

impl BcEncodeDevice {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Cpu,
            2 => Self::Gpu,
            _ => Self::Auto,
        }
    }

    /// Where `format`'s images are encoded: Auto keeps the BC7 on the CPU, the user's choice
    /// (docs/agent_notes/atlas_page_compression.md, "BC7 encode on the GPU").
    fn encode_on(self, format: EncodedFormat) -> EncodeOn {
        match (self, format) {
            (Self::Cpu, _) | (Self::Auto, EncodedFormat::Bc7) => EncodeOn::Cpu,
            (Self::Auto, EncodedFormat::Bc1) => EncodeOn::GpuOrCpu,
            (Self::Gpu, _) => EncodeOn::Gpu,
        }
    }
}

/// Where the encode runs until [`set_bc_encode_device`]: the crate's own tests stay off the GPU, like its other GPU
/// work.
const INITIAL_DEVICE: BcEncodeDevice = if cfg!(test) { BcEncodeDevice::Cpu } else { BcEncodeDevice::Auto };

static DEVICE: AtomicU8 = AtomicU8::new(INITIAL_DEVICE as u8);

/// Sets where the packers' BC7 page encode and BC1 gump encode run (Auto by default, the CPU in this crate's tests).
pub fn set_bc_encode_device(device: BcEncodeDevice) {
    DEVICE.store(device as u8, Ordering::Relaxed);
}

/// Where the packers' BC7 page encode and BC1 gump encode run.
pub fn bc_encode_device() -> BcEncodeDevice {
    BcEncodeDevice::from_u8(DEVICE.load(Ordering::Relaxed))
}

/// `options` with the page encoded on the device [`bc_encode_device`] names.
pub fn on_bc_encode_device(options: Bc7RdoOptions) -> Bc7RdoOptions {
    options.with_encoder(bc7_encoder(bc_encode_device()))
}

/// The BC7 encoder `device` places the pages on: none for the CPU's own.
fn bc7_encoder(device: BcEncodeDevice) -> Option<Arc<dyn Bc7EncodeAccelerator>> {
    match device.encode_on(EncodedFormat::Bc7) {
        EncodeOn::Cpu => None,
        EncodeOn::GpuOrCpu => Some(Arc::new(GpuEncode { fall_back: true })),
        EncodeOn::Gpu => Some(Arc::new(GpuEncode { fall_back: false })),
    }
}

/// `options` with the encode on the device [`bc_encode_device`] names and the RDO on the one
/// [`bc7_rdo_device`](crate::atlas::bc7_rdo_device::bc7_rdo_device) names: what the packers encode their pages with.
pub fn on_bc7_devices(options: Bc7RdoOptions) -> Bc7RdoOptions {
    on_bc_encode_device(on_bc7_rdo_device(options))
}

/// Encodes `rgba` (straight RGBA8888 of `extent`) to BC1 blocks on the device [`bc_encode_device`] names: the GPU's
/// BC1 or the CPU's, both with BC1's one bit of alpha (below one half is transparent black). Auto takes the GPU's,
/// falling back on the CPU without one or when it fails.
pub fn encode_bc1_on_device(rgba: &[u8], extent: ImageExtent) -> Result<Vec<u8>, String> {
    let fall_back = match bc_encode_device().encode_on(EncodedFormat::Bc1) {
        EncodeOn::Cpu => return encode_bc1(rgba, extent).map_err(|error| error.to_string()),
        EncodeOn::GpuOrCpu => true,
        EncodeOn::Gpu => false,
    };
    let result = match gpu() {
        Ok(gpu) => gpu.encode_bc1(rgba, extent.width(), extent.height())
            .map(|blocks| blocks.as_flattened().to_vec()).map_err(|error| format!("{error:#}")),
        Err(error) => Err(error.clone()),
    };
    match result {
        Err(error) if fall_back => {
            if gpu().is_ok() {
                log::warn!("BC1 encode on the GPU failed, the CPU encodes the image: {error}");
            }
            encode_bc1(rgba, extent).map_err(|error| error.to_string())
        }
        result => result,
    }
}

/// The process's GPU encoder, or why there is none.
fn gpu() -> &'static Result<PageEncoderGpu, String> {
    static GPU: OnceLock<Result<PageEncoderGpu, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        let gpu = PageEncoderGpu::new().map_err(|error| format!("{error:#}"));
        match &gpu {
            Ok(gpu) => log::info!("BC encode on the GPU: {}", gpu.adapter()),
            Err(error) => log::warn!("BC encode: no GPU ({error})"),
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

    /// Auto encodes the BC1 on the GPU (the CPU without one) and the BC7 on the CPU; GPU forces both onto the GPU, CPU
    /// both onto the CPU.
    #[test]
    fn auto_puts_the_bc1_on_the_gpu_and_the_bc7_on_the_cpu() {
        use EncodeOn::{Cpu, Gpu, GpuOrCpu};
        for (device, bc7, bc1) in [(BcEncodeDevice::Auto, Cpu, GpuOrCpu), (BcEncodeDevice::Cpu, Cpu, Cpu),
            (BcEncodeDevice::Gpu, Gpu, Gpu)] {
            assert_eq!(device.encode_on(EncodedFormat::Bc7), bc7, "{device:?} BC7");
            assert_eq!(device.encode_on(EncodedFormat::Bc1), bc1, "{device:?} BC1");
            assert_eq!(bc7_encoder(device).is_some(), bc7 != Cpu, "{device:?} BC7 encoder");
        }
    }
}
