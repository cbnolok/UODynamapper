//! GPU compute on plain wgpu, with no app or package format around it: the RIFE frame interpolation network
//! ([`rife`]), the SPAN upscaler ([`upscale`]), the BC1/BC7 block encoder of frames ([`block_encode`]), and the
//! measurement of a walk cycle's stride with RIFE's flow ([`strides`]), and RIFE in-betweens of a clip's plain RGBA
//! frames ([`in_betweens`]), and the BC7 RDO of texture pages ([`bc7_rdo`]). Dynamapper runs the networks on its own
//! device; the udd tools measure the strides of the animation packages, preview the in-betweens and RDO the packages'
//! pages on a device of their own. [`warm_up`] compiles the networks' pipelines ahead of use (the block encoder and
//! the RDO compile on their own). Shaders and weights are embedded in the binary.

pub mod bc7_rdo;
pub mod block_encode;
pub mod frame_upscales;
pub mod in_betweens;
pub mod rife;
pub mod strides;
pub mod upscale;
pub mod warm_up;

pub use warm_up::{warm_up, PipelineCompile, WarmPipelines, WARM_UP_COMPILES};

use color_eyre::eyre::{self, WrapErr};

/// A device of a tool's own, and its queue.
pub(crate) struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

/// A device of the most powerful adapter, and the adapter's description.
pub(crate) fn own_device() -> eyre::Result<(Gpu, String)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.wrap_err("no GPU adapter")?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?}, {})", info.name, info.backend, info.driver_info);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(), ..Default::default()
        }).await.wrap_err("no GPU device")?;
        Ok((Gpu { device, queue }, adapter_name))
    })
}

/// Where a frame lies in a source texture array (an atlas): its layer, and its texels on the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRect {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Compiles a compute shader of the networks (RIFE, the upscaler) without the loop counters naga adds by default to
/// end infinite loops: they keep the driver from unrolling the convolutions' constant loops, which turns each
/// multiply-add into a counter update and a branch and makes the networks several times slower
/// (docs/agent_notes/wgsl_loop_bounding_blocks_unrolling.md). The bounds checks stay.
///
/// # Safety
///
/// Every loop in `source` must end: each runs to a compile-time constant or to a uniform's value, counting up by a
/// positive step.
pub unsafe fn network_shader_module(device: &wgpu::Device, label: &str, source: String) -> wgpu::ShaderModule {
    let descriptor = wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) };
    let checks = wgpu::ShaderRuntimeChecks { force_loop_bounding: false, ..wgpu::ShaderRuntimeChecks::checked() };
    // SAFETY: the caller guarantees every loop ends, which is all the loop bounding protects against.
    unsafe { device.create_shader_module_trusted(descriptor, checks) }
}
