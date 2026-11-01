//! On-demand one-pixel visibility queries. Reuses the drawing shaders, instance
//! buffers and transparent batch bindings; never writes the main color/depth.
use super::{statics_collect::{StaticPickTile, StaticChunkBatchKey}, transparent_order::StaticTransparentBatchOrder};
use bevy::prelude::*;
use bevy::core_pipeline::{Core3d, Core3dSystems, core_3d::Transparent3d};
use bevy::render::{RenderApp, Render, RenderSystems, extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_phase::{AddRenderCommand, DrawFunctions, SetItemPipeline, ViewSortedRenderPhases}, render_resource::*,
    renderer::{RenderContext, RenderDevice, ViewQuery},
    sync_world::MainEntity, view::{ExtractedView, ViewDepthStencilTexture}};
use bevy::shader::ShaderDefVal;
use bevy::pbr::{DrawMesh, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup, SetMaterialBindGroup};
use std::{collections::HashMap, sync::{Arc, Mutex, mpsc, atomic::{AtomicU64, AtomicUsize, Ordering}}};

#[derive(Clone, PartialEq)]
pub(crate) struct PickSignature {
    pub pixel: UVec2,
    pub viewport: UVec4,
    pub clip_from_world: Mat4,
    pub revisions: [u64; 5],
    pub alpha_controls: UVec4,
    pub animation: u32,
    pub map: u32,
}
impl PickSignature {
    fn same_view(&self, other: &Self) -> bool {
        self.pixel == other.pixel && self.viewport == other.viewport
            && self.clip_from_world == other.clip_from_world && self.revisions == other.revisions
            && self.alpha_controls == other.alpha_controls && self.map == other.map
    }
}

type Targets = Arc<[Vec<StaticPickTile>; 2]>;
// Art vertices already carry the global instance index and reconstruct their
// world positions. Their normal mesh bind group is unused, so query resources
// can occupy group 2 without increasing the renderer's bind-group requirement.
type DrawPixelQuery = (SetItemPipeline, SetMeshViewBindGroup<0>, SetMeshViewBindingArrayBindGroup<1>,
    SetMaterialBindGroup<3>, DrawMesh);
#[derive(Clone)]
struct Request {
    serial: u64,
    signature: PickSignature,
    targets: Targets,
    batches: Vec<StaticChunkBatchKey>,
    reply: mpsc::Sender<Reply>,
    render_seen: Arc<AtomicU64>,
    encoded: Arc<AtomicU64>,
}
struct Reply { serial: u64, signature: PickSignature, hit: Result<Option<StaticPickTile>, String> }

#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub(crate) struct PickRequest(Option<Request>);

#[derive(Resource)]
pub(crate) struct VisibleStaticPick {
    serial: u64,
    reply_serial: u64,
    signature: Option<PickSignature>,
    hit: Option<StaticPickTile>,
    pending: bool,
    ready: bool,
    targets: Targets,
    target_revisions: [u64; 2],
    sender: mpsc::Sender<Reply>,
    receiver: Mutex<mpsc::Receiver<Reply>>,
    render_seen: Arc<AtomicU64>,
    encoded: Arc<AtomicU64>,
}
impl Default for VisibleStaticPick {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self { serial: 0, reply_serial: 0, signature: None, hit: None, pending: false, ready: false,
            targets: Arc::new([Vec::new(), Vec::new()]), target_revisions: [0; 2],
            sender, receiver: Mutex::new(receiver), render_seen: Arc::default(), encoded: Arc::default() }
    }
}
impl VisibleStaticPick {
    pub(crate) fn query(&mut self, request: &mut PickRequest, signature: PickSignature,
        targets: impl FnOnce() -> [Vec<StaticPickTile>; 2], batches: Vec<StaticChunkBatchKey>) -> Option<StaticPickTile> {
        if self.signature.as_ref() != Some(&signature) {
            let revisions = [signature.revisions[0], signature.revisions[1]];
            if revisions != self.target_revisions || self.signature.is_none() {
                self.targets = Arc::new(targets());
                self.target_revisions = revisions;
            }
            self.serial = self.serial.wrapping_add(1);
            if self.signature.as_ref().is_none_or(|old| !old.same_view(&signature)) {
                self.hit = None;
                self.ready = false;
            }
            self.signature = Some(signature.clone());
            self.pending = true;
            request.0 = Some(Request { serial: self.serial, signature, targets: self.targets.clone(),
                batches, reply: self.sender.clone(), render_seen: self.render_seen.clone(), encoded: self.encoded.clone() });
        }
        self.hit
    }

    pub(crate) fn cancel(&mut self, request: &mut PickRequest) {
        if request.0.take().is_some() { self.serial = self.serial.wrapping_add(1); }
        self.signature = None;
        self.hit = None;
        self.pending = false;
        self.ready = false;
    }

    pub(crate) fn status(&self) -> (u64, bool, bool, Option<UVec2>, u64, u64) {
        (self.serial, self.pending, self.ready, self.signature.as_ref().map(|s| s.pixel),
            self.render_seen.load(Ordering::Relaxed), self.encoded.load(Ordering::Relaxed))
    }
}

fn poll_query_replies(mut pick: ResMut<VisibleStaticPick>) {
    loop {
        let reply = pick.receiver.lock().unwrap().try_recv();
        let Ok(reply) = reply else { break; };
        if reply.serial < pick.reply_serial || pick.signature.as_ref().is_none_or(|s| !s.same_view(&reply.signature)) { continue; }
        pick.reply_serial = reply.serial;
        pick.ready = reply.hit.is_ok();
        pick.pending = reply.serial != pick.serial;
        pick.hit = match reply.hit {
            Ok(hit) => hit,
            Err(error) => { warn!("Static visibility query failed: {error}"); None }
        };
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<VisibleStaticPick>().init_resource::<PickRequest>()
        .add_plugins(ExtractResourcePlugin::<PickRequest>::default())
        .add_systems(Update, poll_query_replies.before(crate::core::system_sets::SceneRenderArtSysSet::CollectVisibleStatics))
        .add_systems(PostUpdate, enable_depth_sampling);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<QueryState>()
            .add_render_command::<Transparent3d, DrawPixelQuery>()
            .add_systems(Core3d, render_query.after(Core3dSystems::MainPass).before(Core3dSystems::PostProcess))
            .add_systems(Render, map_query_buffers.in_set(RenderSystems::Cleanup));
    }
}

fn enable_depth_sampling(mut cameras: Query<&mut Camera3d, With<crate::core::render::scene::camera::PlayerCamera>>) {
    for mut camera in &mut cameras {
        let usage = TextureUsages::from(camera.depth_texture_usages);
        if !usage.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

#[derive(Resource, Default)]
struct QueryState(Mutex<QueryGpuState>);
#[derive(Default)]
struct QueryGpuState {
    last_serial: u64,
    pipelines: HashMap<(CachedRenderPipelineId, bool, bool), CachedRenderPipelineId>,
    target: Option<QueryTarget>,
    readbacks: Vec<(Buffer, Request)>,
    mapping: Arc<AtomicUsize>,
}
struct QueryTarget {
    samples: u32,
    color: Texture,
    color_view: TextureView,
    _depth: Texture,
    depth_view: TextureView,
    resolve: Option<(ComputePipeline, BindGroup, Buffer)>,
}

fn query_layout(multisampled: bool) -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new("static pixel query", &[
        BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false,
                min_binding_size: BufferSize::new(32) }, count: None },
        BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture { sample_type: TextureSampleType::Depth,
                view_dimension: TextureViewDimension::D2, multisampled }, count: None },
    ])
}

fn query_target(device: &RenderDevice, samples: u32) -> QueryTarget {
    let texture = |format, usage| device.create_texture(&TextureDescriptor {
        label: Some("static one-pixel query"), size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: samples, dimension: TextureDimension::D2,
        format, usage, view_formats: &[],
    });
    let color = texture(TextureFormat::Rgba8Unorm,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | if samples == 1 { TextureUsages::COPY_SRC } else { TextureUsages::empty() });
    let color_view = color.create_view(&TextureViewDescriptor::default());
    let depth = texture(TextureFormat::Depth32Float, TextureUsages::RENDER_ATTACHMENT);
    let depth_view = depth.create_view(&TextureViewDescriptor::default());
    let resolve = (samples > 1).then(|| {
        let module = device.wgpu_device().create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolve static pixel IDs"), source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../../../../assets/shaders/world/art/pick_resolve.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("resolve static pixel IDs"), layout: None, module: &module, entry_point: Some("resolve"),
            compilation_options: Default::default(), cache: None,
        });
        let output = device.create_buffer(&BufferDescriptor { label: Some("static pixel ID"), size: 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC, mapped_at_creation: false });
        let layout = BindGroupLayout::from(pipeline.get_bind_group_layout(0));
        let group = device.create_bind_group("resolve static pixel IDs", &layout, &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&color_view) },
            BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
        ]);
        (pipeline, group, output)
    });
    QueryTarget { samples, color, color_view, _depth: depth, depth_view, resolve }
}

fn query_pipeline(cache: &PipelineCache, state: &mut QueryGpuState,
    original: CachedRenderPipelineId, transparent: bool, multisampled: bool) -> CachedRenderPipelineId {
    *state.pipelines.entry((original, transparent, multisampled)).or_insert_with(|| {
        let mut descriptor = cache.get_render_pipeline_descriptor(original).clone();
        descriptor.label = Some("static pixel visibility".into());
        descriptor.layout[2] = query_layout(multisampled);
        descriptor.vertex.shader_defs.push(ShaderDefVal::Bool("STATIC_PICK".into(), true));
        descriptor.vertex.shader_defs.push(ShaderDefVal::Bool("STATIC_PICK_MSAA".into(), multisampled));
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.entry_point = Some("pick_fragment".into());
        fragment.shader_defs.push(ShaderDefVal::Bool("STATIC_PICK".into(), true));
        fragment.shader_defs.push(ShaderDefVal::Bool("STATIC_PICK_MSAA".into(), multisampled));
        fragment.targets = vec![Some(ColorTargetState { format: TextureFormat::Rgba8Unorm,
            blend: None, write_mask: ColorWrites::ALL })];
        descriptor.depth_stencil = Some(DepthStencilState { format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(!transparent), depth_compare: Some(CompareFunction::GreaterEqual),
            stencil: Default::default(), bias: Default::default() });
        let id = cache.queue_render_pipeline(descriptor);
        trace!("Queued static visibility pipeline {id:?} from {original:?}");
        id
    })
}

fn render_query(world: &World, view: ViewQuery<(&ExtractedView, &ViewDepthStencilTexture)>,
    request: Res<PickRequest>, state: Res<QueryState>, cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    batches: Query<(&MainEntity, &StaticTransparentBatchOrder)>, mut context: RenderContext) {
    let Some(request) = request.0.as_ref() else { return; };
    request.render_seen.store(request.serial, Ordering::Relaxed);
    let view_entity = view.entity();
    let (extracted, depth) = view.into_inner();
    let Some(phase) = phases.get(&extracted.retained_view_entity) else { return; };
    let mut state = state.0.lock().unwrap();
    if state.last_serial == request.serial { return; }
    let samples = depth.texture().sample_count();
    let orders: HashMap<_, _> = batches.iter().filter(|(_, order)| request.batches.contains(&order.key))
        .map(|(entity, order)| (*entity, *order)).collect();
    let mut items = Vec::new();
    let mut ready = true;
    for item in phase.items.values() {
        if !orders.contains_key(&item.entity.1) || item.batch_range.is_empty() { continue; }
        if cache.get_render_pipeline(item.pipeline).is_none() { ready = false; continue; }
        let opaque = query_pipeline(&cache, &mut state, item.pipeline, false, samples > 1);
        let transparent = query_pipeline(&cache, &mut state, item.pipeline, true, samples > 1);
        ready &= cache.get_render_pipeline(opaque).is_some() && cache.get_render_pipeline(transparent).is_some();
        items.push((item, [opaque, transparent]));
    }
    if !ready { return; }
    if state.target.as_ref().is_none_or(|target| target.samples != samples) {
        state.target = Some(query_target(&device, samples));
    }
    let target = state.target.as_ref().unwrap();
    let signature = &request.signature;
    let size = depth.texture().size();
    if signature.pixel.x >= size.width || signature.pixel.y >= size.height { return; }
    let buffers: Vec<_> = (0..2u32).map(|pass| {
        let bytes: [u32; 8] = [
            (signature.pixel.x as f32).to_bits(), (signature.pixel.y as f32).to_bits(),
            (signature.viewport.z as f32).to_bits(), (signature.viewport.w as f32).to_bits(),
            pass, signature.viewport.x, signature.viewport.y, samples,
        ];
        device.create_buffer_with_data(&wgpu::util::BufferInitDescriptor {
            label: Some("static pixel query coordinates"), contents: bytemuck::cast_slice(&bytes), usage: BufferUsages::UNIFORM,
        })
    }).collect();
    let layout = cache.get_bind_group_layout(&query_layout(samples > 1));
    let groups: Vec<_> = buffers.iter().map(|buffer| device.create_bind_group("static pixel query", &layout, &[
        BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
        BindGroupEntry { binding: 1, resource: BindingResource::TextureView(&depth.attachment.texture.default_view) },
    ])).collect();
    let draw_functions = world.resource::<DrawFunctions<Transparent3d>>();
    let mut functions = draw_functions.write();
    functions.prepare(world);
    let draw_id = functions.id::<DrawPixelQuery>();
    for pass_index in 0..2 {
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("static one-pixel visibility"),
            color_attachments: &[Some(RenderPassColorAttachment { view: &target.color_view,
                resolve_target: None, depth_slice: None, ops: Operations {
                    load: if pass_index == 0 { LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { LoadOp::Load }, store: StoreOp::Store } })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment { view: &target.depth_view,
                depth_ops: Some(Operations { load: if pass_index == 0 { LoadOp::Clear(0.0) } else { LoadOp::Load }, store: StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_bind_group(2, &groups[pass_index], &[]);
        for (original, pipelines) in &items {
            let item = Transparent3d { pipeline: pipelines[pass_index],
                sorting_info: original.sorting_info, distance: original.distance, entity: original.entity,
                draw_function: draw_id, batch_range: original.batch_range.clone(),
                extra_index: original.extra_index.clone(), indexed: original.indexed };
            if let Some(draw) = functions.get_mut(item.draw_function) {
                if let Err(error) = draw.draw(world, &mut pass, view_entity, &item) {
                    error!("Static visibility draw failed: {error:?}");
                    return;
                }
            }
        }
    }
    drop(functions);
    let readback = device.create_buffer(&BufferDescriptor { label: Some("static pixel readback"), size: 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ, mapped_at_creation: false });
    let encoder = context.command_encoder();
    if let Some((pipeline, group, output)) = &target.resolve {
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(output, 0, &readback, 0, 4);
    } else {
        encoder.copy_texture_to_buffer(target.color.as_image_copy(), TexelCopyBufferInfo {
            buffer: &readback, layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
        }, Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
    }
    state.last_serial = request.serial;
    request.encoded.store(request.serial, Ordering::Relaxed);
    state.readbacks.push((readback, request.clone()));
    trace!("Encoded static visibility query {}", request.serial);
}

fn map_query_buffers(state: Res<QueryState>, device: Res<RenderDevice>) {
    let (readbacks, mapping) = {
        let mut state = state.0.lock().unwrap();
        (std::mem::take(&mut state.readbacks), state.mapping.clone())
    };
    for (buffer, request) in readbacks {
        let mapped = buffer.clone();
        mapping.fetch_add(1, Ordering::Relaxed);
        let completed = mapping.clone();
        buffer.slice(..).map_async(MapMode::Read, move |result| {
            let hit = result.map_err(|error| error.to_string()).and_then(|()| {
                let bytes = mapped.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
                let id = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                let index = (id & 0x7fff_ffff).checked_sub(1);
                Ok(index.and_then(|index| request.targets[(id >> 31) as usize].get(index as usize).copied()))
            });
            mapped.unmap();
            completed.fetch_sub(1, Ordering::Relaxed);
            trace!("Read static visibility query {}: {:?}", request.serial, hit);
            let _ = request.reply.send(Reply { serial: request.serial, signature: request.signature, hit });
        });
    }
    // Pump callbacks while queries are in flight, without waiting for the GPU.
    if mapping.load(Ordering::Relaxed) != 0 { let _ = device.poll(wgpu::PollType::Poll); }
}

#[cfg(test)]
mod tests;
