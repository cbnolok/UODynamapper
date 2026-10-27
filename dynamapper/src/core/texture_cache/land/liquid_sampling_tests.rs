use bevy::asset::{uuid::Uuid, AssetId};
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource};

// Executes the real WESL sampler on an sRGB array texture. The surrounding and
// unused-layer magenta pixels catch atlas leakage; the four source values make
// encoded-data interpolation distinguishable from filtered sRGB re-encoding.
#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for liquid sampling changes"]
fn liquid_sampling_gpu_preserves_data_and_source_edges() {
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
        let shader_id = |value| AssetId::<Shader>::Uuid { uuid: Uuid::from_u128(value) };
        shaders.set_shader(shader_id(1), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/texture_data.wesl"),
            "shaders/world/texture_data.wesl",
        ));
        shaders.set_shader(shader_id(2), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/ec_liquid.wesl"),
            "shaders/world/ec_liquid.wesl",
        ));
        shaders.set_shader(shader_id(0), Shader::from_wesl(r#"
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_liquid_sample};
@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> samples: array<vec4<f32>>;
@group(0) @binding(2) var linear_sampler: sampler;
@compute @workgroup_size(1)
fn main() {
    let slot = EcLiquidSlot(true, 1, vec2<u32>(1u), vec2<u32>(2u), 1.0);
    samples[0] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.5), true, true, true);
    samples[1] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.5), true, true, false);
    samples[2] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, true, true);
    samples[3] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, false, true);
    samples[4] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.49, 0.25), false, true, true);
    samples[5] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(-0.25, 0.25), true, true, true);
    samples[6] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.75, 0.25), true, true, true);
    let single = EcLiquidSlot(true, 1, vec2<u32>(1u), vec2<u32>(1u), 1.0);
    samples[7] = ec_liquid_sample(atlas, linear_sampler, single, vec2<f32>(0.0), true, true, true);
    samples[8] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0), true, true, true);
    samples[9] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(1.0), true, false, true);
    samples[10] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, true, false);
    samples[11] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, false, false);
    samples[12] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.49, 0.25), false, true, false);
    samples[13] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0), true, true, false);
}
"#, "shaders/test/liquid_sampling_gpu.wesl"));
        let wgsl = shaders.get(0, shader_id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("liquid sampling regression"),
            source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 2 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels = [[0u8, 32, 64, 255], [64, 96, 128, 128], [128, 160, 192, 64], [255, 224, 32, 0]];
        let mut bytes = [255u8, 0, 255, 255].repeat(32);
        for (i, pixel) in pixels.iter().enumerate() {
            let offset = (16 + (1 + i / 2) * 4 + 1 + i % 2) * 4;
            bytes[offset..offset + 4].copy_from_slice(pixel);
        }
        queue.write_texture(atlas.as_image_copy(), &bytes, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(16), rows_per_image: Some(4),
        }, atlas.size());
        let view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        let result_size = 14 * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: result_size, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: result_size, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, result_size);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| { sender.send(result).unwrap(); });
        device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(20)),
        }).unwrap();
        receiver.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        let normalized = pixels.map(|pixel| pixel.map(|channel| f32::from(channel) / 255.0));
        let mean = |values: &[[f32; 4]]| std::array::from_fn::<_, 4, _>(|channel| {
            values.iter().map(|pixel| pixel[channel]).sum::<f32>() / values.len() as f32
        });
        let linear = normalized.map(|pixel| std::array::from_fn(|channel| {
            let value = pixel[channel];
            if channel == 3 { value } else if value <= 0.04045 { value / 12.92 }
            else { ((value + 0.055) / 1.055).powf(2.4) }
        }));
        let expected = [mean(&normalized), mean(&linear), mean(&normalized[..2]),
            normalized[0], normalized[0], normalized[1], normalized[1], normalized[0],
            mean(&normalized), normalized[3], mean(&linear[..2]), linear[0], linear[0], mean(&linear)];
        for (sample, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            for channel in 0..4 {
                assert!((actual[channel] - expected[channel]).abs() < 0.001,
                    "sample {sample} channel {channel}: {} != {}", actual[channel], expected[channel]);
            }
        }
        drop(mapped);
        readback.unmap();
    });
}
