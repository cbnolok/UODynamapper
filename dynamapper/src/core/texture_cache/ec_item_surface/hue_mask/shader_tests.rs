use bevy::asset::{uuid::Uuid, AssetId};
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource};

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for sprite hue changes"]
fn original_hue_uses_encoded_red_mask_alpha_and_native_block_seams() {
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
            ("shaders/world/art/hue.wesl", include_str!("../../../../../assets/shaders/world/art/hue.wesl")),
            ("shaders/world/ec_item_surface.wesl", include_str!("../../../../../assets/shaders/world/ec_item_surface.wesl")),
            ("shaders/world/ec_liquid.wesl", include_str!("../../../../../assets/shaders/world/ec_liquid.wesl")),
            ("shaders/world/texture_data.wesl", include_str!("../../../../../assets/shaders/world/texture_data.wesl")),
        ].into_iter().enumerate() {
            shaders.set_shader(id(index as u128 + 1), Shader::from_wesl(source, path));
        }
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::art::hue::apply_ec_sprite_hue;
import package::shaders::world::texture_data::texture_color_rgb;
@group(0) @binding(0) var masks: texture_2d_array<f32>;
@group(0) @binding(1) var palette: texture_2d<f32>;
@group(0) @binding(2) var lookup: texture_2d<u32>;
@group(0) @binding(3) var filtering: sampler;
@group(0) @binding(4) var<storage, read_write> result: array<vec4<f32>>;
fn sample(uv: vec2<f32>, header: vec4<u32>, linear: bool) -> vec4<f32> {
    let color = vec4<f32>(texture_color_rgb(vec3<f32>(0.2, 0.7, 0.4)), 0.8);
    return apply_ec_sprite_hue(color, 1u, header, uv, palette, filtering, masks, lookup, linear);
}
@compute @workgroup_size(1)
fn main() {
    let masked = vec4<u32>(0u, 768u, 2u, 300u | (2u << 16u));
    // Offset zero is reserved for missing descriptors; the fixture starts at one.
    let header = vec4<u32>(1u, masked.yzw);
    result[0] = sample(vec2<f32>(0.0), header, true);
    result[1] = sample(vec2<f32>(1.0), header, true);
    result[2] = sample(vec2<f32>(256.0 / 300.0, 0.5), header, true);
    result[3] = sample(vec2<f32>(255.5 / 300.0, 0.5), header, true);
    result[4] = sample(vec2<f32>(256.5 / 300.0, 0.5), header, true);
    result[5] = sample(vec2<f32>(255.99 / 300.0, 0.5), header, false);
    result[6] = sample(vec2<f32>(0.5), vec4<u32>(0u, 256u, 0u, 0u), true);
    result[7] = sample(vec2<f32>(0.5), masked, true);
}
"#, "shaders/test/sprite_hue.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("original sprite hue"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        let texture = |width, height, layers, format| device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let masks = texture(256, 256, 2, wgpu::TextureFormat::Rgba8Unorm);
        let mut bytes = vec![0u8; 256 * 256 * 2 * 4];
        for y in 0..2 {
            bytes[(y * 256 + 255) * 4 + 3] = 64;
            for x in 0..44 { bytes[((256 * 256) + y * 256 + x) * 4 + 3] = 255; }
        }
        queue.write_texture(masks.as_image_copy(), &bytes, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(256),
        }, masks.size());
        let palette = texture(1024, 1024, 1, wgpu::TextureFormat::Rgba8UnormSrgb);
        let mut hues = vec![0u8; 1024 * 1024 * 4];
        for x in 0..256 { hues[(1024 + x) * 4..(1024 + x + 1) * 4].copy_from_slice(&[x as u8, 255, 0, 128]); }
        queue.write_texture(palette.as_image_copy(), &hues, wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(1024 * 4), rows_per_image: Some(1024),
        }, palette.size());
        let lookup = texture(3, 1, 1, wgpu::TextureFormat::Rgba32Uint);
        let records = [[0u32; 4], [0, 0, 0, 256 | (2 << 16)], [1, 0, 0, 44 | (2 << 16)]];
        queue.write_texture(lookup.as_image_copy(), bytemuck::cast_slice(&records), wgpu::TexelCopyBufferLayout {
            offset: 0, bytes_per_row: Some(3 * 16), rows_per_image: Some(1),
        }, lookup.size());
        let view = masks.create_view(&wgpu::TextureViewDescriptor::default());
        let palette_view = palette.create_view(&wgpu::TextureViewDescriptor::default());
        let lookup_view = lookup.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default()
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: 8 * 16, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: 8 * 16, mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&palette_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&lookup_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: output.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 8 * 16);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        let decode = |v: f32| ((v + 0.055) / 1.055).powf(2.4);
        let base = [decode(0.2), decode(0.7), decode(0.4), 0.8];
        let hued = [decode(0.2), 1.0, 0.0, 0.8 * 128.0 / 255.0];
        for (sample, weight) in [0.0, 1.0, 159.5 / 255.0, 64.0 / 255.0, 1.0, 64.0 / 255.0, 1.0, 0.0].into_iter().enumerate() {
            for channel in 0..4 {
                let expected = base[channel] * (1.0 - weight) + hued[channel] * weight;
                assert!((actual[sample][channel] - expected).abs() < 0.001,
                    "hue sample {sample} channel {channel}: {} != {expected}", actual[sample][channel]);
            }
        }
    });
}
