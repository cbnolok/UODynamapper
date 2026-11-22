//! GPU compute on plain wgpu, with no app or package format around it: the RIFE frame interpolation network
//! ([`rife`]), the SPAN upscaler ([`upscale`]), the BC1/BC7 block encoder of frames ([`block_encode`]), and the
//! measurement of a walk cycle's stride with RIFE's flow ([`strides`]), and RIFE in-betweens of a clip's plain RGBA
//! frames ([`in_betweens`]), and the BC7 and BC1 encode ([`page_encode`]) and RDO ([`bc7_rdo`]) of texture pages.
//! Dynamapper runs the networks on its own device; the udd tools measure the strides of the animation packages,
//! preview the in-betweens and encode and RDO the packages' pages on a device of their own. [`warm_up`] compiles the
//! networks' pipelines ahead of use (the block encoder and the RDO compile on their own). Shaders and weights are
//! embedded in the binary.

pub mod bc7_rdo;
pub mod block_encode;
pub mod frame_upscales;
pub mod in_betweens;
pub mod page_encode;
pub mod rife;
pub mod strides;
pub mod upscale;
pub mod warm_up;

pub use warm_up::{warm_up, warm_up_compiles, PipelineCompile, WarmPipelines};

use color_eyre::eyre::{self, WrapErr};

/// A device of a tool's own, and its queue.
pub(crate) struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

/// A device of the most powerful adapter, and the adapter's description.
pub(crate) fn own_device() -> eyre::Result<(Gpu, String)> {
    own_device_and_units().map(|(gpu, adapter, _)| (gpu, adapter))
}

/// A device of the most powerful adapter, with the matrix units' features when it has them, the adapter's description,
/// and where the networks' convolutions run on it.
pub(crate) fn own_device_and_units() -> eyre::Result<(Gpu, String, ConvUnits)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default()
        }).await.wrap_err("no GPU adapter")?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?}, {})", info.name, info.backend, info.driver_info);
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features() & ConvUnits::MATRIX_FEATURES,
            required_limits: adapter.limits(),
            // SAFETY: the cooperative matrices are a wgpu experimental feature; the networks use them only where
            // ConvUnits::of finds the product they need supported.
            experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
            ..Default::default()
        }).await.wrap_err("no GPU device")?;
        let units = ConvUnits::of(&adapter, &device);
        Ok((Gpu { device, queue }, adapter_name, units))
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

/// Where the networks' convolutions do their multiply-adds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConvUnits {
    /// The shader cores, in f32: any device.
    ShaderCores,
    /// The matrix units: 16x16x16 products of fp16 matrices into f32 sums (wgpu's experimental cooperative
    /// matrices: AMD's WMMA on RDNA 3, NVIDIA's tensor cores). Only SPAN uses them: on a Radeon 880M it runs 1.4 to 2
    /// times as fast, within 0.3/255 of PyTorch's colour instead of 0.2/255
    /// (docs/agent_notes/mobile_animation_motion.md).
    Matrix,
}

impl ConvUnits {
    /// The features a device needs for [`ConvUnits::Matrix`], besides wgpu's experimental features enabled.
    pub const MATRIX_FEATURES: wgpu::Features = wgpu::Features::SHADER_F16.union(wgpu::Features::SUBGROUP)
        .union(wgpu::Features::EXPERIMENTAL_COOPERATIVE_MATRIX);

    /// The matrix units when `device` (of `adapter`) has [`ConvUnits::MATRIX_FEATURES`] and the adapter multiplies
    /// 16x16x16 fp16 matrices into f32 sums; the shader cores otherwise.
    pub fn of(adapter: &wgpu::Adapter, device: &wgpu::Device) -> Self {
        use wgpu::CooperativeScalarType::{F16, F32};
        let product = adapter.cooperative_matrix_properties().into_iter().any(|properties| {
            (properties.m_size, properties.n_size, properties.k_size) == (MATRIX_SIDE, MATRIX_SIDE, MATRIX_SIDE)
                && properties.ab_type == F16 && properties.cr_type == F32
        });
        if device.features().contains(Self::MATRIX_FEATURES) && product { Self::Matrix } else { Self::ShaderCores }
    }
}

/// The side of the matrix units' matrices the networks use.
pub(crate) const MATRIX_SIDE: u32 = 16;

/// The extensions a shader on the matrix units declares, ahead of everything else in its source.
pub(crate) const MATRIX_EXTENSIONS: &str = "enable f16;\nenable wgpu_cooperative_matrix;\n";

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
