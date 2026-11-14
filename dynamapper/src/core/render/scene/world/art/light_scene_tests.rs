//! art/light_scene.wesl's face heights and cap test, on the GPU: the points of a wall's faces and of its top, seen
//! through the wall's card under the default camera, read where the light clip (art/light_clip.wesl) expects them.
use bevy::{asset::{uuid::Uuid, AssetId}, prelude::*,
    shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};
use wgpu::util::DeviceExt;

/// The world height of one z (uo_coords.wesl, UO_Z_TO_WORLD_SCALE).
const HEIGHT_SCALE: f32 = 4.0 / 44.0;
/// The static's z is stored as z + this (light_scene.wesl, LIGHT_SCENE_Z_BIAS).
const Z_BIAS: i32 = 128;
/// light_scene.wesl's texel code: every static, its class from this shift, and the left half of its card.
const ART: u32 = 1;
const CLASS_SHIFT: u32 = 1;
const CLASS_WALL: u32 = 1;
const CLASS_BLOCK: u32 = 2;
const LEFT: u32 = 8;
/// From a cell's corner to the statics' anchor (statics_collect.rs, EC_STATIC_TILE_TRANSLATION_X and _Z).
const ANCHOR_OFFSET: Vec2 = Vec2::new(0.5, 1.5);
/// The keep's wall at the sconces (map 1, 0x0081 on row 1593, z 50, 22 high).
const CELL: Vec2 = Vec2::new(1335.0, 1593.0);
const WALL_Z: i32 = 50;
const WALL_HEIGHT: i32 = 22;
/// The view line climbs a cell's depth by one world unit under the default camera (camera.rs,
/// BASE_OFFSET_FROM_PLAYER, (5, 5, 5)): 11 z.
const DEFAULT_CELL_RISE: i32 = 11;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TexelCase { world_pos: [f32; 4], args: [i32; 4] }

/// The point of the wall's card that shows the world point `point`: along the view line to the card's plane, upright
/// through the anchor across the view.
fn on_card(point: Vec3, view_dir: Vec3) -> Vec3 {
    let anchor = CELL + ANCHOR_OFFSET;
    let normal = Vec2::new(std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    let t = -(point.xz() - anchor).dot(normal) / view_dir.xz().dot(normal);
    point + view_dir * t
}

/// The height of the middle of the z `z`.
fn y_of(z: i32) -> f32 {
    (z as f32 + 0.5) * HEIGHT_SCALE
}

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for light scene changes"]
fn a_walls_faces_read_their_height_and_its_top_reads_as_its_cap() {
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
            include_str!("../../../../../../assets/shaders/world/art/light_scene.wesl"),
            "shaders/world/art/light_scene.wesl"));
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::art::light_scene::{light_scene_texel, light_scene_cell_rise, light_scene_may_be_cap};
struct TexelCase { world_pos: vec4<f32>, args: vec4<i32> }
struct Constants { anchor: vec2<f32>, right_xz: vec2<f32>, view_dir: vec4<f32> }
@group(0) @binding(0) var<storage, read> cases: array<TexelCase>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<u32>>;
@group(0) @binding(2) var<uniform> constants: Constants;
const HEIGHT_SCALE: f32 = 4.0 / 44.0;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) index: vec3<u32>) {
    let test = cases[index.x];
    let texel = light_scene_texel(test.world_pos.xyz, constants.anchor, test.args.x, u32(test.args.y),
        constants.right_xz, constants.view_dir.xyz, HEIGHT_SCALE);
    let rise = light_scene_cell_rise(constants.view_dir.xyz, HEIGHT_SCALE);
    result[index.x] = vec4<u32>(texel.z, texel.w, u32(rise), select(0u, 1u, light_scene_may_be_cap(texel,
        test.args.x + test.args.z, rise)));
}
"#, "shaders/test/light_scene_texel.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light scene texel"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });

        let view_dir = Vec3::ONE.normalize();
        let wall_y = |z| y_of(WALL_Z + z);
        let top = WALL_Z + WALL_HEIGHT;
        let mut inputs = Vec::new();
        // (z read, code, may be the cap) expected of each case.
        let mut expected = Vec::new();
        for class in [CLASS_WALL, CLASS_BLOCK] {
            for z in [0, 7, 15, WALL_HEIGHT - 1] {
                // The south face on the cell's south edge, on the card's left half; the east face on its east edge,
                // on the right half.
                for (point, left) in [(Vec3::new(CELL.x + 0.5, wall_y(z), CELL.y + 1.0), true),
                    (Vec3::new(CELL.x + 1.0, wall_y(z), CELL.y + 0.5), false)] {
                    inputs.push(TexelCase { world_pos: on_card(point, view_dir).extend(1.0).to_array(),
                        args: [WALL_Z, class as i32, WALL_HEIGHT, 0] });
                    expected.push((WALL_Z + z, ART | class << CLASS_SHIFT | if left { LEFT } else { 0 }, true));
                }
            }
        }
        // The top, its cap, up to the cell's far corner: above the top by no more than a cell's rise.
        for (x, y) in [(0.8, 0.6), (0.5, 0.3), (0.2, 0.1), (0.3, 0.8), (0.1, 0.6), (0.05, 0.02)] {
            let point = Vec3::new(CELL.x + x, top as f32 * HEIGHT_SCALE, CELL.y + y);
            inputs.push(TexelCase { world_pos: on_card(point, view_dir).extend(1.0).to_array(),
                args: [WALL_Z, CLASS_WALL as i32, WALL_HEIGHT, 0] });
            expected.push((i32::MIN, 0, true));
        }
        // Art drawn taller than the wall's height (a bookcase): a face point higher above the top stays a face.
        let point = Vec3::new(CELL.x + 0.5, wall_y(WALL_HEIGHT + DEFAULT_CELL_RISE + 1), CELL.y + 1.0);
        inputs.push(TexelCase { world_pos: on_card(point, view_dir).extend(1.0).to_array(),
            args: [WALL_Z, CLASS_BLOCK as i32, WALL_HEIGHT, 0] });
        expected.push((top + DEFAULT_CELL_RISE + 1, ART | CLASS_BLOCK << CLASS_SHIFT | LEFT, false));

        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&inputs), usage: wgpu::BufferUsages::STORAGE,
        });
        let right = Vec2::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);
        let anchor = CELL + ANCHOR_OFFSET;
        let constants: [f32; 8] = [anchor.x, anchor.y, right.x, right.y, view_dir.x, view_dir.y, view_dir.z, 0.0];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&constants), usage: wgpu::BufferUsages::UNIFORM,
        });
        let bytes = (expected.len() * std::mem::size_of::<[u32; 4]>()) as u64;
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
                wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
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
        let actual: &[[u32; 4]] = bytemuck::cast_slice(&mapped);
        for (case, (&[z, code, rise, may_be_cap], &(want_z, want_code, want_cap))) in
            actual.iter().zip(&expected).enumerate()
        {
            let z = z as i32 - Z_BIAS;
            assert_eq!(rise as i32, DEFAULT_CELL_RISE, "case {case}");
            assert_eq!(may_be_cap != 0, want_cap, "case {case}: z {z}");
            if want_z == i32::MIN {
                assert!(z >= top && z <= top + DEFAULT_CELL_RISE, "case {case}: the cap reads {z}");
            } else {
                assert_eq!((z, code), (want_z, want_code), "case {case}");
            }
        }
    });
}
