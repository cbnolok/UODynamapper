//! Running a WESL function on the GPU in tests: a compute shader that imports the shader modules under test reads one
//! vec4 per case and writes one vec4 per result, so a test checks the shader's own numbers rather than a Rust mirror.
use bevy::{asset::{uuid::Uuid, AssetId}, shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};
use wgpu::util::DeviceExt;

/// How long a test waits for the GPU to finish.
const GPU_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Runs `invocations` invocations of `main` of the WESL compute shader `source` over `inputs` (binding 0) and reads
/// `results` vec4s back (binding 1). `modules` are the WESL sources `source` imports, directly or not, each with its
/// path under the assets folder (`shaders/...`).
pub fn run_wesl_compute(modules: &[(&str, &str)], source: &str, inputs: &[[f32; 4]], invocations: usize,
    results: usize) -> Vec<[f32; 4]> {
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
        let mut shaders = ShaderCache::<String, ()>::new((), |_, source, _| {
            let ShaderCacheSource::Wgsl(wgsl) = source else {
                return Err(ShaderCacheError::CreateShaderModule("expected WGSL".into()));
            };
            Ok(wgsl)
        });
        let id = |value| AssetId::<Shader>::Uuid { uuid: Uuid::from_u128(value) };
        for (index, (module, path)) in modules.iter().enumerate() {
            shaders.set_shader(id(index as u128 + 1), Shader::from_wesl(module.to_string(), path.to_string()));
        }
        shaders.set_shader(id(0), Shader::from_wesl(source.to_owned(), "shaders/test/wesl_compute.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wesl compute test"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(inputs), usage: wgpu::BufferUsages::STORAGE,
        });
        let bytes = (results * std::mem::size_of::<[f32; 4]>()) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(invocations as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(GPU_WAIT) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
    })
}
