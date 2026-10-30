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
        shaders.set_shader(shader_id(6), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/common_bindings.wesl"),
            "shaders/world/common_bindings.wesl",
        ));
        shaders.set_shader(shader_id(7), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/land/land_bindings.wesl"),
            "shaders/world/land/land_bindings.wesl",
        ));
        // Synthetic metadata isolates the real bank-selection algorithm from
        // paging. It includes an alias, a higher bank, and lower submerged sand.
        shaders.set_shader(shader_id(8), Shader::from_wesl(r#"
import package::shaders::world::land::land_bindings::TileUniform;
fn bank_fixture_tile(payload: u32, height: f32, flags: u32, size: u32) -> TileUniform {
    return TileUniform(height, size, 0u, 0u, vec2<u32>(0u), vec2<u32>(2u), 0u, flags, payload, 1.0);
}
fn atlas_read_meta(x: i32, y: i32) -> TileUniform {
    if (x == 10 && y == 10) { return bank_fixture_tile(10u, 3.0, 4u, 2u); }
    if (x == 10 && y == 11) { return bank_fixture_tile(11u, 3.0, 4u, 2u); }
    if (x == 11 && y == 10) { return bank_fixture_tile(2u, 4.0, 0u, 2u); }
    if (x == 9 && y == 9) { return bank_fixture_tile(3u, 5.0, 0u, 2u); }
    if ((x == 9 && y == 10) || (x == 19 && y == 20)) { return bank_fixture_tile(4u, 2.0, 0u, 2u); }
    return bank_fixture_tile(0u, 0.0, 0u, 3u);
}
"#, "shaders/world/land/atlas.wesl"));
        shaders.set_shader(shader_id(9), Shader::from_wesl(r#"
fn ec_coverage_params(payload: u32) -> vec4<u32> {
    if (payload == 10u || payload == 11u) { return vec4<u32>(500u, 0u, 0u, 2u); }
    return vec4<u32>(payload, 0u, 0u, 1u);
}
"#, "shaders/world/land/coverage.wesl"));
        shaders.set_shader(shader_id(10), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/land/liquid_bank.wesl"),
            "shaders/world/land/liquid_bank.wesl",
        ));
        shaders.set_shader(shader_id(11), Shader::from_wesl(
            include_str!("../../../../assets/shaders/world/art/surface_blend.wesl"),
            "shaders/world/art/surface_blend.wesl",
        ));
        shaders.set_shader(shader_id(0), Shader::from_wesl(r#"
import package::shaders::world::ec_liquid::{EcLiquidSlot, ec_liquid_sample, ec_liquid_reflection_uv, ec_liquid_original_view};
import package::shaders::world::ec_item_surface::apply_ec_item_surface;
import package::shaders::world::ec_item_liquid::apply_ec_item_liquid;
import package::shaders::world::texture_data::texture_color_rgb;
import package::shaders::world::ec_item_quad::{ec_item_quad_screen, ec_item_quad_world};
import package::shaders::world::land::atlas::bank_fixture_tile;
import package::shaders::world::land::liquid_bank::{ec_liquid_bank, ec_bank_vertex, EcLiquidBank};
import package::shaders::world::art::surface_blend::static_surface_coverage;
@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var<storage, read_write> samples: array<vec4<f32>>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var surface_lookup: texture_2d<u32>;
@group(0) @binding(4) var item_lookup: texture_2d<u32>;
fn bank_result(bank: EcLiquidBank) -> vec4<f32> {
    return vec4<f32>(bank.weight, vec2<f32>(bank.cell), 0.0);
}
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
    let liquid = bank_fixture_tile(10u, 3.0, 4u, 2u);
    let rock = bank_fixture_tile(2u, 4.0, 0u, 2u);
    samples[43] = bank_result(ec_liquid_bank(vec3<f32>(10.9, 3.0, 10.5), liquid, 500u, 1.0));
    samples[44] = bank_result(ec_liquid_bank(vec3<f32>(10.9, 3.2, 10.5), liquid, 500u, 0.8));
    samples[45] = bank_result(ec_liquid_bank(vec3<f32>(10.999, 2.975, 10.5), liquid, 500u, 0.8));
    samples[46] = bank_result(ec_liquid_bank(vec3<f32>(20.9, 3.2, 20.5), liquid, 500u, 0.8));
    samples[47] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 4.0, 10.5), rock, 500u, 0.8));
    samples[48] = bank_result(ec_liquid_bank(vec3<f32>(11.2, 4.0, 11.1), rock, 500u, 0.8));
    samples[49] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 4.0, 10.5), rock, 600u, 0.8));
    samples[50] = bank_result(ec_liquid_bank(vec3<f32>(9.1, 2.0, 10.5), bank_fixture_tile(4u, 2.0, 0u, 2u), 500u, 0.8));
    samples[51] = bank_result(ec_liquid_bank(vec3<f32>(10.9, 3.2, 10.5), bank_fixture_tile(99u, 3.0, 0u, 3u), 500u, 0.8));
    // Coverage's bilinear footprint reaches two cells in either direction.
    samples[52] = bank_result(ec_liquid_bank(vec3<f32>(12.01, 4.0, 10.5), rock, 500u, 0.8));
    samples[53] = bank_result(ec_liquid_bank(vec3<f32>(8.99, 4.0, 10.5), rock, 500u, 0.8));
    samples[54] = bank_result(ec_liquid_bank(vec3<f32>(12.01, 4.0, 12.01), rock, 500u, 0.8));
    samples[55] = bank_result(ec_liquid_bank(vec3<f32>(8.99, 4.0, 8.99), rock, 500u, 0.8));
    samples[56] = bank_result(ec_liquid_bank(vec3<f32>(13.01, 4.0, 10.5), rock, 500u, 0.8));
    // Blend through the crust plane, with no lava on a fully raised bank.
    samples[57] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 3.0, 10.5), rock, 500u, 0.8));
    samples[58] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 2.9, 10.5), rock, 500u, 0.8));
    samples[59] = bank_result(ec_liquid_bank(vec3<f32>(10.999, 3.0, 10.5), liquid, 500u, 0.8));
    samples[60] = bank_result(ec_liquid_bank(vec3<f32>(10.5, 3.0, 10.5), liquid, 500u, 1.0));
    // Ease only raised EC banks; liquid planes, other terrain and CC stay pinned.
    samples[61] = vec4<f32>(ec_bank_vertex(vec2<i32>(10, 10), liquid), 0.0);
    samples[62] = vec4<f32>(ec_bank_vertex(vec2<i32>(11, 10), rock), 0.0);
    samples[63] = vec4<f32>(ec_bank_vertex(vec2<i32>(20, 20), rock), 0.0);
    samples[64] = vec4<f32>(ec_bank_vertex(vec2<i32>(11, 10), bank_fixture_tile(2u, 4.0, 0u, 4u)), 0.0);
    samples[65] = vec4<f32>(ec_bank_vertex(vec2<i32>(9, 9), bank_fixture_tile(3u, 5.0, 0u, 2u)), 0.0);
    samples[66] = vec4<f32>(ec_bank_vertex(vec2<i32>(10, 11), bank_fixture_tile(11u, 3.0, 4u, 2u)), 0.0);
    samples[67] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 2.975, 10.5), rock, 500u, 0.8));
    samples[68] = bank_result(ec_liquid_bank(vec3<f32>(10.5, 3.0, 10.5), liquid, 500u, 0.8));
    // Flat marsh/substrate coverage must not turn into a dry bank at equal Z.
    let level_solid = bank_fixture_tile(4u, 3.0, 0u, 2u);
    samples[69] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 3.0, 10.5), level_solid, 500u, 0.8));
    samples[70] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 3.1, 10.5), level_solid, 500u, 0.8));
    samples[71] = bank_result(ec_liquid_bank(vec3<f32>(11.1, 3.0, 10.5), bank_fixture_tile(4u, 2.0, 0u, 2u), 500u, 0.8));
    // Static ground patches expose map terrain only at unoccupied boundaries.
    samples[72] = vec4<f32>(static_surface_coverage(vec2<f32>(0.0), 255u), 0.0, 0.0, 0.0);
    samples[73] = vec4<f32>(static_surface_coverage(vec2<f32>(0.0, 0.5), 247u), 0.0, 0.0, 0.0);
    samples[74] = vec4<f32>(static_surface_coverage(vec2<f32>(0.41, 0.5), 247u), 0.0, 0.0, 0.0);
    samples[75] = vec4<f32>(static_surface_coverage(vec2<f32>(0.0), 254u), 0.0, 0.0, 0.0);
    samples[76] = vec4<f32>(static_surface_coverage(vec2<f32>(0.25), 254u), 0.0, 0.0, 0.0);
    samples[77] = vec4<f32>(static_surface_coverage(vec2<f32>(0.5), 0u), 0.0, 0.0, 0.0);
    // Adjacent edge cells meet opaquely; a missing diagonal cannot fade the center.
    samples[78] = vec4<f32>(static_surface_coverage(vec2<f32>(1.0, 0.5), 247u), 0.0, 0.0, 0.0);
    samples[79] = vec4<f32>(static_surface_coverage(vec2<f32>(0.0, 0.5), 239u), 0.0, 0.0, 0.0);
    samples[80] = vec4<f32>(static_surface_coverage(vec2<f32>(0.5), 254u), 0.0, 0.0, 0.0);
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
        let result_size = 81 * 16;
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
            [11.207105, 20.737105, 31.5, 1.0], [11.207105, 19.97, 31.5, 1.0], [0.2, 0.4, 0.6, mask[3]],
            [0.0, 10.0, 10.0, 0.0], [1.0, 11.0, 10.0, 0.0], [0.5, 11.0, 10.0, 0.0],
            [0.0, 20.0, 20.0, 0.0], [1.0, 11.0, 10.0, 0.0], [1.0, 11.0, 11.0, 0.0],
            [0.0, 11.0, 10.0, 0.0], [0.0, 9.0, 10.0, 0.0], [0.0, 10.0, 10.0, 0.0],
            [1.0, 12.0, 10.0, 0.0], [1.0, 8.0, 10.0, 0.0], [1.0, 12.0, 12.0, 0.0],
            [1.0, 8.0, 8.0, 0.0], [0.0, 13.0, 10.0, 0.0],
            [1.0, 11.0, 10.0, 0.0], [0.0, 11.0, 10.0, 0.0], [1.0, 11.0, 10.0, 0.0], [0.0, 10.0, 10.0, 0.0],
            [0.0, 3.0, 0.0, 0.0], [0.0, 3.85, 0.0, 0.0], [0.0, 4.0, 0.0, 0.0],
            [0.0, 4.0, 0.0, 0.0], [0.0, 4.85, 0.0, 0.0], [0.25, 3.0, -0.25, 0.0],
            [0.5, 11.0, 10.0, 0.0], [0.0, 11.0, 10.0, 0.0],
            [0.0, 11.0, 10.0, 0.0], [1.0, 11.0, 10.0, 0.0], [0.0, 11.0, 10.0, 0.0],
            [1.0, 0.0, 0.0, 0.0], [0.0; 4], [0.5, 0.0, 0.0, 0.0],
            [0.0; 4], [0.75, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]];
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
