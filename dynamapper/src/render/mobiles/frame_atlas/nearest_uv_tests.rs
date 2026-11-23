//! pixel_art_atlas_nearest_uv (pixel_art_filters.wesl), on the GPU with the atlases' nearest sampler: whatever uv a
//! frame's card interpolates, at any zoom and on the frame's edges, the tap reads one of the frame's texels, never its
//! gutter's. The gutters keep the texels of the frames a page held before it emptied, as the uploads write the frames
//! alone. A mirrored clip's card has the frame's right edge (`uv_max`) on its screen-left side, where the rasterizer
//! keeps a pixel centred on the edge; until 3ec27149 the seam filter took that pixel's uv to the gutter's column, and
//! the stale texels there showed as a pale line the frame's height beside the wing tips of the Enhanced daemon and
//! kirin walking east at zoom 0.4, for the frames whose edge lay on pixel centres (2026-11-22).
use bevy::asset::{uuid::Uuid, AssetId};
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource};
use wgpu::util::DeviceExt;

use super::MOBILE_ATLAS_PAGE_SIZE;

/// A frame near the page's far corner, where the uv's float steps are coarsest, and the frame's own colour; every
/// other texel of the page holds the stale colour.
const FRAME_ORIGIN: [u32; 2] = [987, 991];
const FRAME_SIZE: [u32; 2] = [29, 31];
const FRAME_TEXEL: [u8; 4] = [255, 0, 0, 255];
const STALE_TEXEL: [u8; 4] = [255, 255, 255, 255];
/// Screen pixels to a texel: the fractional zooms put pixel centres on texel edges.
const MAGNIFICATIONS: [f32; 7] = [1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0];
/// Sub-pixel placements of the card on the screen for each magnification.
const PLACEMENTS: u32 = 97;
/// How far an interpolated uv strays past the card's edge, in texels; 0 is a pixel centred on the edge.
const OVERSHOOTS: [f32; 4] = [1.0e-4, 1.0e-5, 1.0e-6, 0.0];
const WORKGROUP_SIZE: u32 = 64;

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for sprite atlas sampling changes"]
fn nearest_tap_reads_only_the_frames_texels() {
    let size = MOBILE_ATLAS_PAGE_SIZE as f32;
    let (min, max) = (FRAME_ORIGIN.map(|v| v as f32), [0, 1].map(|axis| (FRAME_ORIGIN[axis] + FRAME_SIZE[axis]) as f32));
    // The texel positions a card's pixel centres lie at, along x with y at the frame's middle and along y with x
    // there, and the edges themselves and a little past them.
    let mut positions = Vec::new();
    for axis in 0..2 {
        let other = 1 - axis;
        let middle = (min[other] + max[other]) / 2.0;
        let mut along = Vec::new();
        for magnification in MAGNIFICATIONS {
            let step = 1.0 / magnification;
            for placement in 0..PLACEMENTS {
                let mut position = min[axis] + step * (placement as f32 + 0.5) / PLACEMENTS as f32;
                while position < max[axis] {
                    along.push(position);
                    position += step;
                }
            }
        }
        for overshoot in OVERSHOOTS {
            along.extend([min[axis] - overshoot, max[axis] - overshoot, max[axis] + overshoot]);
        }
        positions.extend(along.into_iter().map(|position| {
            let mut point = [0.0; 2];
            point[axis] = position;
            point[other] = middle;
            [point[0] / size, point[1] / size]
        }));
    }
    let taps = positions.len() as u32;

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
            include_str!("../../../../assets/shaders/world/pixel_art_filters.wesl"),
            "shaders/world/pixel_art_filters.wesl"));
        shaders.set_shader(id(0), Shader::from_wesl(format!(r#"
import package::shaders::world::pixel_art_filters::pixel_art_atlas_nearest_uv;
@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var<storage, read> uvs: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> taps: array<vec4<f32>>;
@group(0) @binding(4) var<uniform> frame: vec4<f32>;
@compute @workgroup_size({WORKGROUP_SIZE})
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= arrayLength(&uvs)) {{
        return;
    }}
    let atlas_size = vec2<f32>(textureDimensions(atlas).xy);
    let uv = pixel_art_atlas_nearest_uv(uvs[id.x], frame.xy, frame.zw, atlas_size);
    taps[id.x] = textureSampleLevel(atlas, atlas_sampler, uv, 0, 0.0);
}}
"#), "shaders/test/nearest_uv.wesl"));
        let wgsl = shaders.get(0, id(0), &[]).unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("atlas nearest uv"), source: wgpu::ShaderSource::Wgsl(wgsl.as_str().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: None,
        });

        let side = MOBILE_ATLAS_PAGE_SIZE;
        let mut texels = STALE_TEXEL.repeat((side * side) as usize);
        for y in FRAME_ORIGIN[1]..FRAME_ORIGIN[1] + FRAME_SIZE[1] {
            for x in FRAME_ORIGIN[0]..FRAME_ORIGIN[0] + FRAME_SIZE[0] {
                let at = ((y * side + x) * 4) as usize;
                texels[at..at + 4].copy_from_slice(&FRAME_TEXEL);
            }
        }
        let atlas = device.create_texture_with_data(&queue, &wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb, usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }, wgpu::util::TextureDataOrder::LayerMajor, &texels);
        let view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        // The atlases' sampler: Bevy's default with ImagePlugin::default_nearest (app.rs).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let uvs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&positions), usage: wgpu::BufferUsages::STORAGE,
        });
        let frame = [min[0] / size, min[1] / size, max[0] / size, max[1] / size];
        let frame = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&frame), usage: wgpu::BufferUsages::UNIFORM,
        });
        let bytes = u64::from(taps) * 16;
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
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: uvs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: frame.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(taps.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |value| sender.send(value).unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) }).unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let read: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
        // The frame's texel reads (1, 0, 0, 1); a stale one has green.
        let stale: Vec<_> = positions.iter().zip(read).filter(|(_, tap)| tap[1] > 0.5)
            .map(|(uv, _)| [uv[0] * size, uv[1] * size]).collect();
        assert!(stale.is_empty(), "{} of {taps} taps read past the frame, at texel positions {:?}", stale.len(),
            &stale[..stale.len().min(8)]);
    });
}
