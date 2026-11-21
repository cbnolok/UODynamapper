//! GPU compute on plain wgpu, with no app or package format around it: the RIFE frame interpolation network
//! ([`rife`]), the SPAN upscaler ([`upscale`]), the BC1/BC7 block encoder of frames ([`block_encode`]), and the
//! measurement of a walk cycle's stride with RIFE's flow ([`strides`]). Dynamapper runs the networks on its own
//! device; the udd tools measure the strides of the animation packages on a device of their own. [`warm_up`]
//! compiles the networks' pipelines ahead of use (the block encoder compiles on its own). Shaders and weights are
//! embedded in the binary.

pub mod block_encode;
pub mod rife;
pub mod strides;
pub mod upscale;
pub mod warm_up;

pub use warm_up::{warm_up, PipelineCompile, WarmPipelines, WARM_UP_COMPILES};

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
