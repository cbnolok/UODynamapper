use bevy::{asset::{uuid::Uuid, AssetId}, prelude::*,
    shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};
use wgpu::util::DeviceExt;

use super::statics_collect::logical_draw_distance;

// A view far into the map (Britain), where the world coordinates keep the fewest fraction bits.
const TEST_FOCUS: Vec3 = Vec3::new(1496.0, 0.9, 1628.0);
// The orthographic camera's offset from its focus along each axis (camera.rs BASE_OFFSET_FROM_PLAYER) and the
// perspective camera's orbit distances at a 1080-pixel window and zoom 1 and 4 (camera/perspective.rs
// orbit_distance), all along (1, 1, 1). The farther the camera, the fewer bits of its view z are left for the levels.
const TEST_ORTHO_OFFSET: f32 = 5.0;
const TEST_PERSPECTIVE_DISTANCES: [f32; 2] = [138.0, 552.0];
// The art view's orthographic range (camera.rs ORTHO_NEAR, ORTHO_FAR), the perspective lens (camera/perspective.rs).
const TEST_ORTHO_NEAR: f32 = -1000.0;
const TEST_ORTHO_FAR: f32 = 1000.0;
const TEST_PERSPECTIVE_FOV: f32 = 0.25;
const TEST_PERSPECTIVE_NEAR: f32 = 0.1;
// Priority z the cases span: the land's lowest (z -128 - 2) to the highest static's (127 + 2).
const TEST_PRIORITY_Z: [f32; 9] = [-130.0, -2.0, 0.0, 5.0, 6.0, 7.0, 8.0, 40.0, 129.0];
const TEST_TIE_LEVELS: [u32; 4] = [0, 1, 2, 8];
// Cells around the focus: three rows, more than one cell in a row.
const TEST_CELL_OFFSETS: [(f32, f32); 6] = [(-1.0, 0.0), (0.0, 0.0), (1.0, -1.0), (1.0, 0.0), (2.0, -1.0), (0.0, 2.0)];

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for draw order changes"]
fn gpu_depth_follows_the_cpu_draw_order_in_both_cameras() {
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
import package::shaders::world::art::depth::{priority_logical_height, logical_view_z_from, projected_view_z_depth};
struct DepthCase { projection: mat4x4<f32>, camera: vec4<f32>, axis: vec4<f32>, cell: vec4<f32> }
@group(0) @binding(0) var<storage, read> cases: array<DepthCase>;
@group(0) @binding(1) var<storage, read_write> result: array<f32>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let test = cases[index.x];
    let view_z = logical_view_z_from(test.cell.xy, priority_logical_height(test.cell.z, test.cell.w),
        test.camera.xyz, test.axis.xyz);
    result[index.x] = projected_view_z_depth(view_z, test.projection);
}
"#, "shaders/test/art_depth.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("draw order depth"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct DepthCase { projection: [[f32; 4]; 4], camera: [f32; 4], axis: [f32; 4], cell: [f32; 4] }
        let mut inputs = Vec::new();
        // Each case's camera, CPU logical distance and cell.
        let mut expected = Vec::new();
        let cameras = [(false, TEST_ORTHO_OFFSET * 3f32.sqrt()), (true, TEST_PERSPECTIVE_DISTANCES[0]),
            (true, TEST_PERSPECTIVE_DISTANCES[1])];
        for (camera_index, (perspective, distance)) in cameras.into_iter().enumerate() {
            let mut projection = if perspective {
                Projection::Perspective(PerspectiveProjection {
                    fov: TEST_PERSPECTIVE_FOV, near: TEST_PERSPECTIVE_NEAR, ..default() })
            } else {
                Projection::Orthographic(OrthographicProjection {
                    near: TEST_ORTHO_NEAR, far: TEST_ORTHO_FAR, ..OrthographicProjection::default_3d()
                })
            };
            projection.update(1280.0, 800.0);
            let matrix = projection.get_clip_from_view();
            let camera = Transform::from_translation(TEST_FOCUS + Vec3::ONE.normalize() * distance)
                .looking_at(TEST_FOCUS, Vec3::Y);
            let axis = camera.back();
            for (dx, dy) in TEST_CELL_OFFSETS {
                for priority_z in TEST_PRIORITY_Z {
                    for level in TEST_TIE_LEVELS {
                        let cell = [TEST_FOCUS.x + dx, TEST_FOCUS.z + dy, priority_z, level as f32];
                        inputs.push(DepthCase { projection: matrix.to_cols_array_2d(),
                            camera: camera.translation.extend(0.0).to_array(), axis: axis.extend(0.0).to_array(),
                            cell });
                        expected.push((camera_index, logical_draw_distance(f64::from(cell[0] + cell[1]),
                            f64::from(priority_z), level), cell));
                    }
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
        // Reverse Z: the nearer case has the larger depth. Every pair the CPU orders, the GPU orders the same way,
        // strictly, down to adjacent tie levels at Britain's map position.
        let mut ordered_pairs = 0;
        for (i, &(camera, distance, cell)) in expected.iter().enumerate() {
            for (j, &(other_camera, other_distance, other_cell)) in expected.iter().enumerate() {
                if camera != other_camera || distance <= other_distance {
                    continue;
                }
                ordered_pairs += 1;
                assert!(actual[i] > actual[j], "camera {:?}: {cell:?} (L {distance}, depth {}) must \
                    draw over {other_cell:?} (L {other_distance}, depth {})", cameras[camera], actual[i], actual[j]);
            }
        }
        assert!(ordered_pairs > 0);
    });
}
