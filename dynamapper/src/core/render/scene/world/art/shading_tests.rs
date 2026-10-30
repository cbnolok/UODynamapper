use bevy::asset::{uuid::Uuid, AssetId};
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource};

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for art lighting changes"]
fn local_light_intensity_preserves_unlit_art_and_scales_collected_light() {
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
        for (index, (path, source)) in [
            ("shaders/world/art/shading.wesl", include_str!("../../../../../../assets/shaders/world/art/shading.wesl")),
            ("shaders/world/land/noise.wesl", include_str!("../../../../../../assets/shaders/world/land/noise.wesl")),
        ].into_iter().enumerate() {
            shaders.set_shader(id(index as u128 + 1), Shader::from_wesl(source, path));
        }
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::art::shading::apply_art_surface_shading;
@group(0) @binding(0) var<storage, read_write> result: array<vec4<f32>>;
fn shade(role: u32, ground: bool, fake: u32, intensity: f32, local: vec4<f32>, highlight: f32) -> vec4<f32> {
    return vec4<f32>(apply_art_surface_shading(vec3<f32>(0.3, 0.4, 0.5),
        vec2<f32>(0.25, 0.4), vec3<f32>(0.1, 0.2, 0.3), role,
        normalize(vec3<f32>(0.5, 0.8, 0.25)), vec3<f32>(1.1, 1.0, 0.9),
        vec3<f32>(0.8, 0.9, 1.1), fake, 0.2, highlight, 0.3, 0.1, 0.0,
        intensity, 0.7, local, ground), 1.0);
}
@compute @workgroup_size(1)
fn main() {
    for (var family = 0u; family < 20u; family += 1u) {
        let role = family % 5u;
        let ground = ((family / 5u) % 2u) != 0u;
        let fake = family / 10u;
        for (var step = 0u; step < 3u; step += 1u) {
            result[family * 6u + step] = shade(role, ground, fake, f32(step), vec4<f32>(0.0), 0.4);
            result[family * 6u + 3u + step] = shade(role, ground, fake, f32(step), vec4<f32>(1.0, 0.7, 0.3, 0.8), 0.4);
        }
    }
    result[120] = shade(0u, false, 0u, 1.0, vec4<f32>(0.0), 0.0);
}
"#, "shaders/test/art_lighting.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("art local-light isolation"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        let bytes = 121 * 16;
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
                wgpu::BindGroupEntry { binding: 0, resource: output.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        for family in 0..20 {
            let samples = &actual[family * 6..family * 6 + 6];
            for channel in 0..3 {
                for step in 0..3 {
                    assert!((samples[step][channel] - samples[0][channel]).abs() < 0.00001,
                        "unlit family {family}, step {step}, channel {channel}");
                }
                assert!((samples[3][channel] - samples[0][channel]).abs() < 0.00001);
                assert!(samples[4][channel] > samples[3][channel], "local family {family}, channel {channel}");
                assert!((samples[5][channel] - samples[3][channel]
                    - 2.0 * (samples[4][channel] - samples[3][channel])).abs() < 0.00001);
            }
        }
        for channel in 0..3 {
            assert!(actual[1][channel] > actual[120][channel], "global art highlight must remain effective");
        }
    });
}
