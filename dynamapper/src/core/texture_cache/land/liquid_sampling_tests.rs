use bevy::asset::{uuid::Uuid, AssetId};
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource};

// Executes the real WESL sampler on the EC UNORM array. Magenta guards catch
// atlas leakage; contrasting texels distinguish encoded interpolation followed
// by color conversion from interpolation of already decoded colors.
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
        shaders.set_shader(shader_id(3), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/ec_item_surface.wesl"),
            "shaders/world/ec_item_surface.wesl",
        ));
        shaders.set_shader(shader_id(4), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/ec_item_liquid.wesl"),
            "shaders/world/ec_item_liquid.wesl",
        ));
        shaders.set_shader(shader_id(5), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/ec_item_quad.wesl"),
            "shaders/world/ec_item_quad.wesl",
        ));
        shaders.set_shader(shader_id(0), Shader::from_wesl(r#"
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_liquid_sample, ec_liquid_reflection_uv, ec_liquid_original_view};
import package::shaders::world::ec_item_surface::apply_ec_item_surface;
import package::shaders::world::ec_item_liquid::apply_ec_item_liquid;
import package::shaders::world::texture_data::texture_color_rgb;
import package::shaders::world::ec_item_quad::{ec_item_quad_screen, ec_item_quad_world};
@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> samples: array<vec4<f32>>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var surface_lookup: texture_2d<u32>;
@group(0) @binding(4) var item_lookup: texture_2d<u32>;
fn color_sample(value: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(texture_color_rgb(value.rgb), value.a);
}
@compute @workgroup_size(1)
fn main() {
    let slot = EcLiquidSlot(true, 1, vec2<u32>(1u), vec2<u32>(2u), 1.0);
    samples[0] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.5), true, true);
    samples[1] = color_sample(ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.5), true, true));
    samples[2] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, true);
    samples[3] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, false);
    samples[4] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.49, 0.25), false, true);
    samples[5] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(-0.25, 0.25), true, true);
    samples[6] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.75, 0.25), true, true);
    let single = EcLiquidSlot(true, 1, vec2<u32>(1u), vec2<u32>(1u), 1.0);
    samples[7] = ec_liquid_sample(atlas, linear_sampler, single, vec2<f32>(0.0), true, true);
    samples[8] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0), true, true);
    samples[9] = ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(1.0), true, false);
    samples[10] = color_sample(ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, true));
    samples[11] = color_sample(ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0, 0.25), true, false));
    samples[12] = color_sample(ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.49, 0.25), false, true));
    samples[13] = color_sample(ec_liquid_sample(atlas, linear_sampler, slot, vec2<f32>(0.0), true, true));
    // The original meshes have no normal stream: reflecting about zero leaves
    // the view direction intact. A fabricated horizontal normal changes Y.
    samples[14] = vec4<f32>(ec_liquid_reflection_uv(vec3<f32>(0.0, -3.0, -4.0)), 0.0, 0.0);
    samples[15] = vec4<f32>(ec_liquid_reflection_uv(vec3<f32>(6.0, -6.0, -3.0)), 0.0, 0.0);
    samples[16] = vec4<f32>(ec_liquid_reflection_uv(vec3<f32>(0.0)), 0.0, 0.0);
    // This layer uses the production native-texel padding helper, as in tex_aux_ec.
    let auxiliary = EcLiquidSlot(true, 2, vec2<u32>(0u), vec2<u32>(2u), 1.0);
    samples[17] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.5), true, true);
    samples[18] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.0, 0.25), true, true);
    samples[19] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.0, 0.25), true, false);
    samples[20] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.49, 0.25), false, true);
    samples[21] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(-0.25, 0.25), true, true);
    samples[22] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(1.0), true, false);
    samples[23] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.375, 0.25), true, false);
    samples[24] = ec_liquid_sample(atlas, linear_sampler, auxiliary, vec2<f32>(0.0), true, true);
    let base = vec4<f32>(0.2, 0.4, 0.6, 0.25);
    samples[25] = apply_ec_item_surface(base, atlas, surface_lookup, linear_sampler,
        vec4<u32>(1u, 7u, 0u, 0u), vec2<f32>(0.5), vec2<f32>(0.0), true, true);
    samples[26] = apply_ec_item_surface(base, atlas, surface_lookup, linear_sampler,
        vec4<u32>(1u, 1u, 0u, 0u), vec2<f32>(0.5), vec2<f32>(0.0), true, true);
    samples[27] = apply_ec_item_surface(base, atlas, surface_lookup, linear_sampler,
        vec4<u32>(1u, 0u, 0u, 0u), vec2<f32>(0.5), vec2<f32>(0.0), true, true);
    samples[28] = apply_ec_item_surface(base, atlas, surface_lookup, linear_sampler,
        vec4<u32>(0u), vec2<f32>(0.5), vec2<f32>(0.0), true, true);
    samples[29] = vec4<f32>(ec_liquid_reflection_uv(ec_liquid_original_view(vec3<f32>(0.0))), 0.0, 0.0);
    samples[30] = vec4<f32>(ec_liquid_reflection_uv(ec_liquid_original_view(vec3<f32>(1.0, 0.0, 0.0))), 0.0, 0.0);
    samples[31] = vec4<f32>(ec_liquid_reflection_uv(ec_liquid_original_view(vec3<f32>(0.0, 0.0, 1.0))), 0.0, 0.0);
    samples[32] = vec4<f32>(ec_liquid_reflection_uv(ec_liquid_original_view(vec3<f32>(0.0, 1.0, 0.0))), 0.0, 0.0);
    let lit = vec4<f32>(0.4, 0.3, 0.2, 0.8);
    samples[33] = apply_ec_item_liquid(lit, atlas, item_lookup, linear_sampler, 0u,
        vec2<f32>(0.5), vec2<f32>(0.0), 0.0, false, true);
    samples[34] = apply_ec_item_liquid(lit, atlas, item_lookup, linear_sampler, 1u,
        vec2<f32>(0.5), vec2<f32>(0.0), 0.0, false, true);
    samples[35] = apply_ec_item_liquid(lit, atlas, item_lookup, linear_sampler, 2u,
        vec2<f32>(0.5), vec2<f32>(0.0), 0.0, false, true);
    samples[36] = vec4<f32>(ec_item_quad_screen(vec2<f32>(0.0)), 0.0, 1.0);
    samples[37] = vec4<f32>(ec_item_quad_screen(vec2<f32>(1.0, 0.0)), 0.0, 1.0);
    samples[38] = vec4<f32>(ec_item_quad_screen(vec2<f32>(0.0, 1.0)), 0.0, 1.0);
    samples[39] = vec4<f32>(ec_item_quad_screen(vec2<f32>(1.0)), 0.0, 1.0);
    samples[40] = vec4<f32>(ec_item_quad_world(vec3<f32>(10.0, 20.0, 30.0), vec2<f32>(0.0),
        vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), 0.5), 1.0);
    samples[41] = vec4<f32>(ec_item_quad_world(vec3<f32>(10.0, 20.0, 30.0), vec2<f32>(1.0),
        vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), 0.5), 1.0);
    // Coverage can filter independently of nearest albedo in Enhanced styling.
    samples[42] = apply_ec_item_surface(base, atlas, surface_lookup, linear_sampler,
        vec4<u32>(1u, 7u, 0u, 0u), vec2<f32>(0.5), vec2<f32>(0.0), false, true);
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
            label: None, size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 3 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels = [[0u8, 32, 64, 255], [64, 96, 128, 128], [128, 160, 192, 64], [255, 224, 32, 0]];
        let mut bytes = [255u8, 0, 255, 255].repeat(48);
        for (i, pixel) in pixels.iter().enumerate() {
            let offset = (16 + (1 + i / 2) * 4 + 1 + i % 2) * 4;
            bytes[offset..offset + 4].copy_from_slice(pixel);
        }
        let padded = super::pad_aux_rgba(bytemuck::cast_slice(&pixels), 2, 2, 4, 4);
        bytes[32 * 4..].copy_from_slice(&padded);
        queue.write_texture(atlas.as_image_copy(), &bytes, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(16), rows_per_image: Some(4),
        }, atlas.size());
        let view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        let lookup = device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 7, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let records = [[0u32; 4], [2, 2, 2, 0], [1.0f32.to_bits(), 0, 0, 0],
            [2, 2, 2, 0], [1.0f32.to_bits(), 0, 0, 0], [0; 4], [0; 4]];
        queue.write_texture(lookup.as_image_copy(), bytemuck::cast_slice(&records), wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(7 * 16), rows_per_image: Some(1),
        }, lookup.size());
        let lookup_view = lookup.create_view(&wgpu::TextureViewDescriptor::default());
        let item_lookup = device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 6, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let params = [1.0f32.to_bits(), 0, 0, 0];
        let records = [[2u32, 2, 2, 1], params, [2, 2, 2, 2], params, [2, 2, 2, 0], params];
        queue.write_texture(item_lookup.as_image_copy(), bytemuck::cast_slice(&records), wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(6 * 16), rows_per_image: Some(1),
        }, item_lookup.size());
        let item_view = item_lookup.create_view(&wgpu::TextureViewDescriptor::default());
        let result_size = 43 * 16;
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
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&lookup_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&item_view) },
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
        let decode = |pixel: [f32; 4]| std::array::from_fn(|channel| {
            let value = pixel[channel];
            if channel == 3 { value } else if value <= 0.04045 { value / 12.92 }
            else { ((value + 0.055) / 1.055).powf(2.4) }
        });
        let linear = normalized.map(decode);
        let mask = mean(&normalized);
        let encoded_base = [0.2f32, 0.4, 0.6].map(|v| 1.055 * v.powf(1.0 / 2.4) - 0.055);
        // ASM binds source1 to TEMP, source2 to alpha, then source0 to COLOR.
        // Distinct source RGB and alpha make a positional-stage mistake observable.
        let separate_bases = decode([
            mask[0] * (1.0 - mask[3]) + encoded_base[0] * mask[3],
            mask[1] * (1.0 - mask[3]) + encoded_base[1] * mask[3],
            mask[2] * (1.0 - mask[3]) + encoded_base[2] * mask[3], 1.0,
        ]);
        let quarter = std::array::from_fn(|channel| normalized[0][channel] * 0.75 + normalized[1][channel] * 0.25);
        // Independently construct the documented row-major camera matrix.
        let tilt = std::f64::consts::FRAC_PI_4 - 0.001;
        let (sin_tilt, cos_tilt) = tilt.sin_cos();
        let (sin_roll, cos_roll) = 0.05f64.sin_cos();
        let original_uv = |x: f64, height: f64, map_y: f64| {
            let right = sin_tilt * x - cos_tilt * map_y;
            let up = -cos_tilt * cos_roll * x - sin_tilt * cos_roll * map_y + height * 1.2856454;
            let forward = 10.0 - cos_tilt * sin_roll * x - sin_tilt * sin_roll * map_y;
            let length = (right * right + up * up + forward * forward).sqrt();
            [(right / length) as f32, (up / length) as f32, 0.0, 0.0]
        };
        let expected = [mean(&normalized), decode(mask), mean(&normalized[..2]),
            normalized[0], normalized[0], normalized[1], normalized[1], normalized[0],
            mean(&normalized), normalized[3], decode(mean(&normalized[..2])), linear[0], linear[0], decode(mask),
            [0.0, -0.6, 0.0, 0.0], [2.0 / 3.0, -2.0 / 3.0, 0.0, 0.0], [0.0; 4],
            mask, mean(&normalized[..2]), normalized[0], normalized[0], normalized[1], normalized[3], quarter, mask,
            [0.2, 0.4, 0.6, mask[3]], [0.2, 0.4, 0.6, 1.0], separate_bases, [0.2, 0.4, 0.6, 0.25],
            original_uv(0.0, 0.0, 0.0), original_uv(1.0, 0.0, 0.0),
            original_uv(0.0, 0.0, 1.0), original_uv(0.0, 1.0, 0.0),
            [0.4 * mask[0], 0.3 * mask[1], 0.2 * mask[2], mask[3]],
            [0.4 * (1.0 - mask[3]) + mask[0] * mask[3],
             0.3 * (1.0 - mask[3]) + mask[1] * mask[3],
             0.2 * (1.0 - mask[3]) + mask[2] * mask[3], 0.8],
            [(0.4 + mask[0]).min(1.0), (0.3 + mask[1]).min(1.0), (0.2 + mask[2]).min(1.0), mask[3]],
            [0.707105, 1.4742099, 0.0, 1.0], [1.4742099, 0.707105, 0.0, 1.0],
            [-0.06, 0.707105, 0.0, 1.0], [0.707105, -0.06, 0.0, 1.0],
            [11.207105, 20.737105, 31.5, 1.0], [11.207105, 19.97, 31.5, 1.0], [0.2, 0.4, 0.6, mask[3]]];
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
