use bevy::{asset::{uuid::Uuid, AssetId}, prelude::*,
    shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for sprite priority changes"]
fn sprite_tie_bias_stays_local_across_perspective_distances() {
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
        shaders.set_shader(id(1), Shader::from_wesl(
            include_str!("../../../../../../assets/shaders/world/art/depth.wesl"), "shaders/world/art/depth.wesl"));
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::art::depth::static_priority_depth;
struct DepthCase { projection: mat4x4<f32>, clip: vec4<f32>, ordinal: vec4<u32> }
@group(0) @binding(0) var<storage, read> cases: array<DepthCase>;
@group(0) @binding(1) var<storage, read_write> result: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let test = cases[index.x];
    result[index.x] = static_priority_depth(test.clip, test.projection, test.ordinal.x);
}
"#, "shaders/test/art_depth.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite priority depth"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct DepthCase { projection: [[f32; 4]; 4], clip: [f32; 4], ordinal: [u32; 4] }
        let mut inputs = Vec::new();
        let mut expected = Vec::new();
        for perspective in [false, true] {
            let mut projection = if perspective {
                Projection::Perspective(PerspectiveProjection { near: 0.1, ..default() })
            } else {
                Projection::Orthographic(OrthographicProjection {
                    near: -1000.0, far: 1000.0, ..OrthographicProjection::default_3d()
                })
            };
            projection.update(1280.0, 800.0);
            let matrix = projection.get_clip_from_view();
            for distance in [-5000.0, 1.0, 25.0, 90.0, 250.0, 1000.0, 5000.0] {
                if perspective && distance <= 0.0 { continue; }
                for ordinal in [0u32, 1, 4, 16, 32] {
                    let position = Vec3::new(0.2, -0.4, -distance);
                    let clip = matrix * position.extend(1.0);
                    let raw = clip.z / clip.w;
                    let target = if perspective {
                        matrix.project_point3(position - Vec3::Z * ordinal as f32 * 0.002).z
                    } else {
                        raw.clamp(0.0, 1.0) - ordinal as f32 * 0.000001
                    }.clamp(0.0, 1.0);
                    inputs.push(DepthCase { projection: matrix.to_cols_array_2d(),
                        clip: clip.to_array(), ordinal: [ordinal, 0, 0, 0] });
                    expected.push((target, raw, perspective, distance, ordinal));
                }
            }
        }
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&inputs), usage: wgpu::BufferUsages::STORAGE,
        });
        let bytes = expected.len() as u64 * 4;
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
            pass.dispatch_workgroups(expected.len() as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let actual: &[f32] = bytemuck::cast_slice(&mapped);
        for (&depth, &(target, raw, perspective, distance, ordinal)) in actual.iter().zip(&expected) {
            let tolerance = target.abs().max(0.000001) * 0.000001;
            assert!((depth - target).abs() <= tolerance,
                "perspective={perspective}, distance={distance}, ordinal={ordinal}: {depth} vs {target}");
            if perspective && ordinal != 0 {
                assert!(depth < raw, "equal-priority sprites must remain separated");
                // A half-cell camera-depth neighbor must stay behind this group.
                let neighbor = 0.1 / (distance + 0.5);
                assert!(depth > neighbor, "tie bias must not jump to the next cell");
            }
        }
        // The previous fixed offset fails this representative bridge/zoom case.
        assert!(0.1 / 90.0 - 16.0 * 0.000001 < 0.1 / 90.5);
    });
}
