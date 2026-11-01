use super::*;
use super::super::statics_collect::StaticRenderTile;

fn signature() -> PickSignature {
    PickSignature { pixel: UVec2::new(640, 400), viewport: UVec4::new(0, 0, 1280, 800),
        clip_from_world: Mat4::IDENTITY, revisions: [1, 2, 3, 4, 5], alpha_controls: UVec4::ZERO, animation: 0, map: 0 }
}
fn tile(graphic: u16) -> StaticPickTile {
    StaticPickTile { tile: StaticRenderTile { graphic, world_x: 500.0, world_z: 769.0, z: -5, hue: 33 }, is_wet: false }
}
fn targets() -> [Vec<StaticPickTile>; 2] { [vec![tile(18910)], vec![tile(6044)]] }

#[test]
fn completed_empty_hit_is_ready_and_unchanged_queries_do_not_restart() {
    let mut app = App::new();
    app.init_resource::<VisibleStaticPick>().add_systems(Update, poll_query_replies);
    let mut request = PickRequest::default();
    app.world_mut().resource_mut::<VisibleStaticPick>().query(&mut request, signature(), targets, Vec::new());
    let query = request.0.as_ref().unwrap().clone();
    query.reply.send(Reply { serial: query.serial, signature: query.signature, hit: Ok(None) }).unwrap();
    app.update();
    let mut pick = app.world_mut().resource_mut::<VisibleStaticPick>();
    assert_eq!(pick.status(), (1, false, true, Some(UVec2::new(640, 400)), 0, 0));
    assert!(pick.query(&mut request, signature(), || panic!("unchanged owner snapshot rebuilt"), Vec::new()).is_none());
    assert_eq!(pick.serial, 1);
}

#[test]
fn stale_pointer_replies_are_rejected_but_animation_can_use_a_recent_matching_frame() {
    let mut app = App::new();
    app.init_resource::<VisibleStaticPick>().add_systems(Update, poll_query_replies);
    let mut request = PickRequest::default();
    app.world_mut().resource_mut::<VisibleStaticPick>().query(&mut request, signature(), targets, Vec::new());
    let old = request.0.as_ref().unwrap().clone();
    let mut moved = signature(); moved.pixel.x += 1;
    app.world_mut().resource_mut::<VisibleStaticPick>().query(&mut request, moved.clone(), targets, Vec::new());
    old.reply.send(Reply { serial: old.serial, signature: old.signature, hit: Ok(Some(tile(18910))) }).unwrap();
    app.update();
    assert!(app.world().resource::<VisibleStaticPick>().hit.is_none());
    assert!(!app.world().resource::<VisibleStaticPick>().ready);
    let previous_frame = request.0.as_ref().unwrap().clone();
    let mut animated = moved; animated.animation = 10;
    app.world_mut().resource_mut::<VisibleStaticPick>().query(&mut request, animated, targets, Vec::new());
    previous_frame.reply.send(Reply { serial: previous_frame.serial, signature: previous_frame.signature,
        hit: Ok(Some(tile(18910))) }).unwrap();
    app.update();
    let pick = app.world().resource::<VisibleStaticPick>();
    assert_eq!(pick.hit, Some(tile(18910)));
    assert!(pick.ready && pick.pending);
}

#[test]
fn source_replacement_invalidates_inflight_ids_and_preserves_signed_object_data() {
    let mut pick = VisibleStaticPick::default();
    let mut request = PickRequest::default();
    pick.query(&mut request, signature(), targets, Vec::new());
    let old = request.0.as_ref().unwrap().clone();
    let mut replaced = signature(); replaced.revisions[0] += 1;
    pick.query(&mut request, replaced, || [vec![tile(39297)], Vec::new()], Vec::new());
    assert_eq!(old.targets[0][0], tile(18910));
    assert_eq!(request.0.as_ref().unwrap().targets[0][0], tile(39297));
    assert_eq!(old.targets[0][0].tile.z, -5);
    assert_eq!(old.targets[0][0].tile.hue, 33);
    pick.cancel(&mut request);
    assert!(request.0.is_none() && !pick.ready && pick.hit.is_none());
}

#[test]
#[ignore = "requires a Vulkan adapter; run explicitly for pixel-query shader/depth changes"]
fn pixel_query_uses_opaque_depth_and_resolves_discrete_msaa_ids() {
    use bevy::{asset::{AssetId, uuid::Uuid}, shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource}};
    bevy::tasks::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
        let mut shaders = ShaderCache::<String, ()>::new((), |_, source, _| {
            let ShaderCacheSource::Wgsl(source) = source else { return Err(ShaderCacheError::CreateShaderModule("expected WGSL".into())); };
            Ok(source)
        });
        let id = |value| AssetId::<Shader>::Uuid { uuid: Uuid::from_u128(value) };
        shaders.set_shader(id(1), Shader::from_wesl(include_str!("../../../../../../../assets/shaders/world/art/picking.wesl"),
            "shaders/world/art/picking.wesl"));
        shaders.set_shader(id(0), Shader::from_wesl(r#"
import package::shaders::world::art::picking::{pick_clip, pick_color};
@group(0) @binding(0) var<uniform> test: vec4<u32>;
@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return pick_clip(vec4<f32>(p[index], 0.7, 1.0));
}
@fragment fn fragment(@builtin(sample_index) sample_index: u32) -> @location(0) vec4<f32> {
    if (test.w == 2u && sample_index == 0u) { discard; }
    let index = test.y + select(0u, 1u, test.w == 1u && sample_index > 0u);
    return pick_color(index, test.z != 0u, bitcast<f32>(test.x), sample_index);
}
"#, "shaders/test/static_query.wesl"));
        let resolve_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../../../../../assets/shaders/world/art/pick_resolve.wgsl").into()) });
        let resolve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None,
            layout: None, module: &resolve_module, entry_point: Some("resolve"), compilation_options: Default::default(), cache: None });
        for samples in [1, 4] {
            let code = shaders.get(0, id(0), &[ShaderDefVal::Bool("STATIC_PICK".into(), true),
                ShaderDefVal::Bool("STATIC_PICK_MSAA".into(), samples > 1)]).unwrap();
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(code.as_str().into()) });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None, layout: None,
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vertex"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fragment"),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
                primitive: Default::default(), depth_stencil: None,
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() }, multiview_mask: None, cache: None,
            });
            let texture = |format, usage| device.create_texture(&wgpu::TextureDescriptor { label: None,
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1,
                sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let opaque = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            let opaque_view = opaque.create_view(&Default::default());
            let color = texture(wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING | if samples == 1 { wgpu::TextureUsages::COPY_SRC } else { wgpu::TextureUsages::empty() });
            let color_view = color.create_view(&Default::default());
            let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let query = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let query_words = [0f32.to_bits(), 0f32.to_bits(), 1f32.to_bits(), 1f32.to_bits(), 0, 0, 0, samples];
            queue.write_buffer(&query, 0, bytemuck::cast_slice(&query_words));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None,
                layout: &pipeline.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }] });
            let empty = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(1), entries: &[] });
            let resources = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(2), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: query.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&opaque_view) },
            ] });
            let output = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let resolve_group = (samples > 1).then(|| device.create_bind_group(&wgpu::BindGroupDescriptor { label: None,
                layout: &resolve.get_bind_group_layout(0), entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
                ] }));
            // Index zero is a valid object; encoded zero means no visible hit.
            for (depth, index, ground, mode) in [(0.5f32, 0u32, 0u32, 0u32), (0.6, 0, 0, 0),
                (0.8, 254, 0, 0), (0.8, 65_534, 1, 0), (0.8, 65_534, 0, 1), (0.8, 7, 1, 2)] {
                queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&[depth.to_bits(), index, ground, mode]));
                let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 256,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &opaque_view,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.6), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                        timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
                }
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &color_view, depth_slice: None,
                            resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
                    pass.set_pipeline(&pipeline); pass.set_bind_group(0, &group, &[]);
                    pass.set_bind_group(1, &empty, &[]); pass.set_bind_group(2, &resources, &[]); pass.draw(0..3, 0..1);
                }
                if samples > 1 {
                    {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&resolve); pass.set_bind_group(0, resolve_group.as_ref().unwrap(), &[]); pass.dispatch_workgroups(1, 1, 1);
                    }
                    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4);
                } else {
                    encoder.copy_texture_to_buffer(color.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None } },
                        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
                }
                queue.submit([encoder.finish()]);
                let (sender, receiver) = mpsc::channel();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |result| sender.send(result).unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); receiver.recv().unwrap().unwrap();
                let bytes = readback.slice(..).get_mapped_range().unwrap();
                let actual = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                let expected = if depth < 0.6 || (samples == 1 && mode == 2) { 0 }
                    else { (index + 1 + u32::from(samples > 1 && mode == 1)) | (ground << 31) };
                assert_eq!(actual, expected, "samples={samples} depth={depth} mode={mode}");
            }
        }
    });
}

#[test]
fn failed_readback_is_not_reported_as_a_completed_empty_hit() {
    let mut app = App::new();
    app.init_resource::<VisibleStaticPick>().add_systems(Update, poll_query_replies);
    let mut request = PickRequest::default();
    app.world_mut().resource_mut::<VisibleStaticPick>().query(&mut request, signature(), targets, Vec::new());
    let query = request.0.as_ref().unwrap();
    query.reply.send(Reply { serial: query.serial, signature: query.signature.clone(), hit: Err("test map failure".into()) }).unwrap();
    app.update();
    let pick = app.world().resource::<VisibleStaticPick>();
    assert!(!pick.ready && !pick.pending && pick.hit.is_none());
}
