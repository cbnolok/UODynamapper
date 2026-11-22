//! Where the packers' BC7 RDO runs: on the GPU (uot_gpu_compute::bc7_rdo) when a wgpu adapter is there, on the CPU
//! otherwise, or on the one the run asks for ([`set_bc7_rdo_device`]). The GPU RDO keeps the CPU's price model and
//! error scales and comes to about its size and error at the same λ and look-back, with blocks of its own; both give
//! the same blocks from run to run. The device starts at the first page RDO'd and stays for the process; the pages
//! encoded at once share its batches.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use uot_gpu_compute::bc7_rdo::Bc7RdoGpu;
use uot_texture_codecs::bc7::rdo::{reduce_entropy_bc7_parallel_with_progress, Bc7RdoParams};
use uot_texture_codecs::bc7::{Bc7RdoAccelerator, Bc7RdoOptions};

/// Where the BC7 RDO runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Bc7RdoDevice {
    /// The GPU when an adapter is there, the CPU otherwise (and when the GPU fails).
    #[default]
    Auto = 0,
    Cpu = 1,
    /// The GPU, failing the pack without one.
    Gpu = 2,
}

impl Bc7RdoDevice {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Cpu,
            2 => Self::Gpu,
            _ => Self::Auto,
        }
    }
}

/// Where the RDO runs until [`set_bc7_rdo_device`]: the crate's own tests stay off the GPU, like its other GPU work.
const INITIAL_DEVICE: Bc7RdoDevice = if cfg!(test) { Bc7RdoDevice::Cpu } else { Bc7RdoDevice::Auto };

static DEVICE: AtomicU8 = AtomicU8::new(INITIAL_DEVICE as u8);

/// Sets where the packers' BC7 RDO runs (Auto by default, the CPU in this crate's tests).
pub fn set_bc7_rdo_device(device: Bc7RdoDevice) {
    DEVICE.store(device as u8, Ordering::Relaxed);
}

/// Where the packers' BC7 RDO runs.
pub fn bc7_rdo_device() -> Bc7RdoDevice {
    Bc7RdoDevice::from_u8(DEVICE.load(Ordering::Relaxed))
}

/// `options` with the RDO on the device [`bc7_rdo_device`] names.
pub fn on_bc7_rdo_device(options: Bc7RdoOptions) -> Bc7RdoOptions {
    let accelerator: Option<Arc<dyn Bc7RdoAccelerator>> = match bc7_rdo_device() {
        Bc7RdoDevice::Cpu => None,
        Bc7RdoDevice::Auto => Some(Arc::new(GpuRdo { fall_back: true })),
        Bc7RdoDevice::Gpu => Some(Arc::new(GpuRdo { fall_back: false })),
    };
    options.with_accelerator(accelerator)
}

/// The process's GPU RDO, or why there is none.
fn gpu() -> &'static Result<Bc7RdoGpu, String> {
    static GPU: OnceLock<Result<Bc7RdoGpu, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        let gpu = Bc7RdoGpu::new().map_err(|error| format!("{error:#}"));
        match &gpu {
            Ok(gpu) => log::info!("BC7 RDO on the GPU: {}", gpu.adapter()),
            Err(error) => log::warn!("BC7 RDO: no GPU ({error})"),
        }
        gpu
    })
}

/// The GPU RDO, falling back on the CPU's when told to.
struct GpuRdo {
    fall_back: bool,
}

impl Bc7RdoAccelerator for GpuRdo {
    fn reduce_entropy(&self, blocks: &mut [[u8; 16]], rgba_blocks: &[[u8; 4]], blocks_x: usize, blocks_y: usize,
        params: &Bc7RdoParams, progress: &(dyn Fn(usize) + Sync)) -> Result<(), String> {
        let result = match gpu() {
            Ok(gpu) => gpu.reduce_entropy(blocks, rgba_blocks, blocks_x, blocks_y, params)
                .map_err(|error| format!("{error:#}")),
            Err(error) => Err(error.clone()),
        };
        match result {
            Ok(()) => progress(blocks.len()),
            Err(error) if self.fall_back => {
                if gpu().is_ok() {
                    log::warn!("BC7 RDO on the GPU failed, the CPU RDOs the page: {error}");
                }
                reduce_entropy_bc7_parallel_with_progress(blocks, rgba_blocks, blocks_x, blocks_y, params,
                    |done| progress(done));
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
}
