//! Compare shading records, exact alpha and grime with the original reconstruction.
use super::*;
use bevy::{asset::{uuid::Uuid, AssetId}, shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for terrain stack changes"]
fn cached_layers_and_alpha_match_original_gpu_reconstruction() {
    bevy::tasks::block_on(async {
        let mut params = vec![None; 16384];
        for id in 1..=61 {
            params[id] = Some(EcTerrainParams([id as u32, id as u32, 242 | (153 << 8), 137]));
        }
        params[1].as_mut().unwrap().0[1] = 0x80000000; // Keys compare as signed integers.
        params[3] = params[2]; // The first raw alias must survive.
        params[5] = Some(EcTerrainParams([5, 5000, 197 | (19 << 8), 138]));
        params[6] = Some(EcTerrainParams([6, 6, 0, 1])); // Point coverage.
        params[8] = None; // Present image, unsupported material class.
        params[60].as_mut().unwrap().0[1] = 10000;
        params[61].as_mut().unwrap().0[1] = 20000; // Excluded by the last pass key.
        let mut available = vec![true; 16384];
        available[7] = false;
        let mut raw = [2u16; HALO * HALO];
        raw[HALO + 1] = 1;
        raw[6 * HALO + 6] = 3;
        for x in 0..HALO { raw[x] = 1; }
        for y in 0..SIDE {
            for x in 0..SIDE {
                let value = if (14..18).contains(&x) && (14..18).contains(&y) {
                    Some((10 + (y - 14) * 4 + x - 14) as u16)
                } else if x >= 20 && (8..25).contains(&y) { Some(5) }
                else if (1..4).contains(&x) && (14..20).contains(&y) { Some(6) }
                else if x == 10 && y == 10 { Some(7) }
                else if x == 11 && y == 10 { Some(8) }
                else if x == 12 && y == 10 { Some(61) }
                else { None };
                if let Some(value) = value { raw[(y + 1) * HALO + x + 1] = value; }
            }
        }
        let liquids = std::array::from_fn(|index| raw[index] == 5 || (index < HALO && index % 2 != 0));
        let cached = build_stack_layer(UVec2::ONE, 0, &raw, &params, &available, &liquids, [1, 60]).unwrap();
        let probe_at = 15 * WIDTH + 15 * RECORDS_PER_CORNER;
        assert_eq!(cached[probe_at][0], 17);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
        let mut shaders = ShaderCache::<String, ()>::new((), |_, source, _| {
            let ShaderCacheSource::Wgsl(wgsl) = source else {
                return Err(ShaderCacheError::CreateShaderModule("expected WGSL".into()));
            }; Ok(wgsl)
        });
        let id = |value| AssetId::<Shader>::Uuid { uuid: Uuid::from_u128(value) };
        shaders.set_shader(id(1), Shader::from_wesl(
            include_str!("../../../../../../assets/shaders/world/common_bindings.wesl"), "shaders/world/common_bindings.wesl"));
        shaders.set_shader(id(2), Shader::from_wesl(
            include_str!("../../../../../../assets/shaders/world/land/land_bindings.wesl")
                .replace("@group(3)", "@group(0)").replace("@binding(110)", "@binding(1)")
                .replace("@binding(117)", "@binding(2)").replace("@binding(118)", "@binding(3)"),
            "shaders/world/land/land_bindings.wesl"));
        shaders.set_shader(id(3), Shader::from_wesl(
            include_str!("../../../../../../assets/shaders/world/land/coverage.wesl"), "shaders/world/land/coverage.wesl"));
        shaders.set_shader(id(4), Shader::from_wesl(r#"
import package::shaders::world::land::land_bindings::TileUniform;
@group(0) @binding(0) var cells: texture_2d<u32>;
fn atlas_read_meta(x: i32, y: i32) -> TileUniform {
    let packed = textureLoad(cells, vec2<i32>(x, y), 0).x;
    return TileUniform(0.0, (packed >> 16u) & 15u, 0u, 0u, vec2<u32>(0u), vec2<u32>(1u),
        (packed >> 20u) & 1u, (packed >> 21u) & 7u, packed & 65535u, 1.0);
}
"#, "shaders/world/land/atlas.wesl"));
        shaders.set_shader(id(5), Shader::from_wesl(
            include_str!("../../../../../../assets/shaders/world/land/transitions.wesl"), "shaders/world/land/transitions.wesl"));
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::land::coverage::{EcTerrainStack, ec_terrain_cache, ec_cache_layer, ec_cache_grime_mask, ec_layer_tile, ec_terrain_stack_uncached};
import package::shaders::world::land::transitions::{ec_transition_boundary_weight, ec_transition_boundary_weight_from_mask};
@group(0) @binding(4) var<storage, read_write> result: array<vec4<u32>>;
@group(0) @binding(5) var<uniform> test_mode: vec4<u32>;
// Adapt the compact production view to the original stack representation so
// every shading field and alpha bit can be compared independently of shading.
fn cached_stack(world: vec2<f32>) -> EcTerrainStack {
    let cache = ec_terrain_cache(world);
    if (cache.count == 0u) { return ec_terrain_stack_uncached(world); }
    var stack: EcTerrainStack;
    stack.count = cache.count;
    for (var index = 0u; index < cache.count; index++) {
        let layer = ec_cache_layer(cache, index);
        stack.layers[index] = layer.material;
        stack.alpha[index] = layer.alpha;
    }
    return stack;
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (test_mode.x != 0u) {
        if (gid.x == 0u) {
            let cached = cached_stack(vec2<f32>(47.5));
            let original = ec_terrain_stack_uncached(vec2<f32>(47.5));
            result[0] = vec4<u32>(cached.count, original.count, cached.layers[0].x, original.layers[0].x);
        }
        return;
    }
    let index = gid.x;
    if (index >= 9216u) { return; }
    let subpixel = index / 1024u;
    let offsets = array<f32, 3>(0.05, 0.5, 0.95);
    let world = vec2<f32>(32.0 + f32(index % 32u) + offsets[subpixel % 3u],
        32.0 + f32((index / 32u) % 32u) + offsets[subpixel / 3u]);
    let cached = cached_stack(world);
    let cache = ec_terrain_cache(world);
    let original = ec_terrain_stack_uncached(world);
    var difference = u32(cached.count != original.count);
    for (var layer = 0u; layer < min(cached.count, original.count); layer++) {
        // The completed sort key word now stores four neighbor masks. Layer
        // order/aliases still must match the original signed-key reconstruction.
        if (any(cached.layers[layer].xzw != original.layers[layer].xzw)) { difference |= 2u; }
        if (bitcast<u32>(cached.alpha[layer]) != bitcast<u32>(original.alpha[layer])) { difference |= 4u; }
        if (cache.count > 0u) {
            let tile = ec_layer_tile(original.layers[layer], 0.0);
            let before = ec_transition_boundary_weight(tile, vec2<i32>(floor(world)), fract(world));
            let mask = ec_cache_grime_mask(cache, cached.layers[layer]);
            let after = ec_transition_boundary_weight_from_mask(tile, fract(world), mask);
            if (bitcast<u32>(before) != bitcast<u32>(after)) { difference |= 8u; }
        }
    }
    result[index] = vec4<u32>(difference, cached.count, original.count, 0u);
}
"#, "shaders/test/cached_terrain.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cached terrain equivalence"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        let texture = |width, height, format| device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let cells = texture(96, 96, wgpu::TextureFormat::R32Uint);
        let mut cell_data = vec![3u32 << 16; 96 * 96];
        for y in 0..HALO {
            for x in 0..HALO {
                let payload = raw[y * HALO + x];
                let liquid_bits = if !liquids[y * HALO + x] { 0 } else if (x + y) % 2 == 0 { 1 << 20 } else { 4 << 21 };
                cell_data[(31 + y) * 96 + 31 + x] = if available[usize::from(payload)] {
                    (2u32 << 16) | u32::from(payload) | liquid_bits
                } else { 3u32 << 16 };
            }
        }
        queue.write_texture(cells.as_image_copy(), bytemuck::cast_slice(&cell_data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(96 * 4), rows_per_image: Some(96) }, cells.size());
        let lookup = texture(256, 448, wgpu::TextureFormat::Rgba32Uint);
        let mut lookup_data = vec![[0u32; 4]; 7 * 16384];
        for (payload, value) in params.iter().enumerate() {
            if available[payload] { lookup_data[payload][3] = 1 | (1 << 16); }
            if let Some(value) = value { lookup_data[6 * 16384 + payload] = value.0; }
        }
        queue.write_texture(lookup.as_image_copy(), bytemuck::cast_slice(&lookup_data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 16), rows_per_image: Some(448) }, lookup.size());
        let bases = texture(3, 3, wgpu::TextureFormat::Rgba16Uint);
        let mut base_data = [[u16::MAX; 4]; 9];
        base_data[4] = [1, 60, 0, 0];
        queue.write_texture(bases.as_image_copy(), bytemuck::cast_slice(&base_data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(3 * 8), rows_per_image: Some(3) }, bases.size());
        let stacks = texture(WIDTH as u32, HEIGHT as u32, wgpu::TextureFormat::Rgba32Uint);
        queue.write_texture(stacks.as_image_copy(), bytemuck::cast_slice(&cached),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(WIDTH as u32 * 16), rows_per_image: Some(HEIGHT as u32) }, stacks.size());
        let views = [cells.create_view(&Default::default()), lookup.create_view(&Default::default()),
            bases.create_view(&Default::default()), stacks.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
            })];
        let bytes = 9216 * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes,
            mapped_at_creation: false, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC });
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes,
            mapped_at_creation: false, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST });
        let mode = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16,
            mapped_at_creation: false, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        queue.write_buffer(&mode, 0, bytemuck::cast_slice(&[0u32; 4]));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[3]) },
                wgpu::BindGroupEntry { binding: 4, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: mode.as_entire_binding() },
            ],
        });
        let dispatch = |groups| {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline); pass.set_bind_group(0, &group, &[]); pass.dispatch_workgroups(groups, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) }).unwrap();
            receiver.recv().unwrap().unwrap();
            let result = bytemuck::cast_slice::<u8, [u32; 4]>(&readback.slice(..).get_mapped_range().unwrap()).to_vec();
            readback.unmap(); result
        };
        let result = dispatch(9216 / 64);
        for (index, sample) in result.iter().enumerate() { assert_eq!(sample[0], 0, "sample {index}: {sample:?}"); }
        assert_eq!(result[4 * 1024 + 15 * 32 + 15][1], 17);

        queue.write_buffer(&mode, 0, bytemuck::cast_slice(&[1u32, 0, 0, 0]));
        let role_region = wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: 0, y: 384, z: 0 }, ..lookup.as_image_copy() };
        let role_extent = wgpu::Extent3d { width: 256, height: 64, depth_or_array_layers: 1 };
        let role_layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 16), rows_per_image: Some(64) };
        // Prove this is a cache hit, not a test that silently exercises fallback.
        queue.write_texture(role_region, &vec![0; 16384 * 16], role_layout, role_extent);
        assert_eq!(dispatch(1)[0], [17, 0, 1, 0]);
        queue.write_texture(role_region, bytemuck::cast_slice(&lookup_data[6 * 16384..]), role_layout, role_extent);
        let pixel_layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16), rows_per_image: Some(1) };
        let pixel_extent = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
        let write_stack = |x, y, value: [u32; 4]| queue.write_texture(wgpu::TexelCopyTextureInfo {
            origin: wgpu::Origin3d { x, y, z: 0 }, ..stacks.as_image_copy()
        }, bytemuck::cast_slice(&value), pixel_layout, pixel_extent);
        write_stack((15 * RECORDS_PER_CORNER + 1) as u32, 15, [0xdead, 1, 1, 0]);
        write_stack(0, 32, [1, 1, 1, 1]); // Wrong map: poisoned cache must be ignored.
        assert_eq!(dispatch(1)[0], [17, 17, 1, 1]);
        write_stack(0, 32, [2, 1, 0, 1]); // Wrong native chunk.
        assert_eq!(dispatch(1)[0], [17, 17, 1, 1]);
        write_stack(0, 32, [1, 1, 0, 1]);
        write_stack((15 * RECORDS_PER_CORNER) as u32, 15, [17, u32::MAX, u32::MAX, 0]);
        assert_eq!(dispatch(1)[0], [17, 17, 1, 1]); // Wrong raw owner.
    });
}
