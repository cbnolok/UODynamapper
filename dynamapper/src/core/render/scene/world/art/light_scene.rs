//! The "uod" light cover (`LightCover::Uod`): no light is hidden for being covered; each glow is clipped pixel by
//! pixel, to the statics its light reaches.
//!
//! - **The light scene.** After the opaque pass, the art redraws its batches into an image the size of the view
//!   (`render_light_scene`, art/light_scene.wesl): for each pixel, the cell, the z and the class of the static drawn
//!   there, from the fragment whose depth is the one drawn. On a wall or a solid static the z is the height of the
//!   point of its face under the pixel, so a floor over a light cuts the wall it meets, and its top, its cap, is a
//!   floor at its top z (the static's tiledata height rides in its flags, `static_light_class_flags`). The land, the
//!   mobiles and whatever else is not a static leave 0. The class comes from the tile flags
//!   (`static_light_class_flags`): a wall, a solid static, or any other (a floor, a roof, flat art).
//! - **The clip.** The glows draw with `LightClipMaterial` (art/light_clip.wesl), which reads the light scene under
//!   each fragment and drops it where the light does not reach: a floor, a roof or another static from
//!   `LIGHT_COVER_MIN_RISE` above the light up; a wall or a solid static from the light's ceiling up (the lowest floor
//!   or roof over it, `light_cover::static_light_ceiling_z`), or on a face turned away from it. It reads the texels a
//!   few pixels around too (`LIGHT_CLIP_EDGE_REACH`): a filtered static's blended edge leaves the static behind it in
//!   the light scene. The light's cell, z and ceiling ride in the glow's `MeshTag` (`light_clip_tag`).
//! - **When they draw.** The light.mul decals of Classic draw in the world view's transparent pass, after the light
//!   scene, with no depth test: the clip stands in for it, and lets a room's glow reach the street in front of its
//!   wall. The light sprites of KR-like, Enhanced and UOD-Painterly belong to the light buffer camera, which renders
//!   before the world view: their pipelines draw nothing there (`LightClipMode::Deferred`), and the world view draws
//!   them into the light buffer once the light scene is done (`render_deferred_light_sprites`), before the client
//!   night composite reads it.

use super::gpu_picking::DrawPixelQuery;
use super::transparent_order::StaticTransparentBatchOrder;
use crate::core::render::scene::camera::PlayerCamera;
use crate::core::render::scene::world::effects::client_night::LightBufferCamera;
use bevy::camera::CameraUpdateSystems;
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d, Transparent3d};
use bevy::core_pipeline::Core3d;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::batching::gpu_preprocessing::{
    IndirectParametersBuffers, IndirectParametersIndexed, IndirectParametersNonIndexed,
};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{DrawFunctions, PhaseItemExtraIndex, ViewSortedRenderPhases};
use bevy::render::render_resource::*;
use bevy::render::sync_world::MainEntity;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewDepthStencilTexture, ViewTarget};
use bevy::render::RenderApp;
use bevy::shader::ShaderDefVal;
use crate::console_logger::{self, LogAbout, LogSev};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use uocf::enhanced::tileart::TaeFlag;

/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is a wall to the light scene. Kept in
/// sync with art/light_scene.wesl.
pub(crate) const STATIC_FLAG_LIGHT_WALL: u32 = 1 << 13;
/// `SpriteInstance` and `GroundTileInstance::is_wet_flags` bit: the static is a solid block to the light scene. Kept
/// in sync with art/light_scene.wesl.
pub(crate) const STATIC_FLAG_LIGHT_BLOCK: u32 = 1 << 14;
/// `SpriteInstance::is_wet_flags` bits of a wall or a solid block: its tiledata height, `STATIC_LIGHT_HEIGHT_MASK`
/// from `STATIC_LIGHT_HEIGHT_SHIFT`, which tells its top (its cap) from its faces in the light scene. Kept in sync
/// with art/light_scene.wesl.
const STATIC_LIGHT_HEIGHT_SHIFT: u32 = 16;
const STATIC_LIGHT_HEIGHT_MASK: u32 = 0xff;
/// The statics that lie flat over the light's floor, or that a light shows through: never walls or blocks.
const LIGHT_SCENE_NEVER_SOLID_FLAGS: u64 = TaeFlag::Surface.bits() | TaeFlag::Roof.bits() | TaeFlag::Foliage.bits()
    | TaeFlag::Transparent.bits() | TaeFlag::Translucent.bits();

/// The light scene's texel: cell x, cell y, z and code (art/light_scene.wesl).
const LIGHT_SCENE_FORMAT: TextureFormat = TextureFormat::Rgba16Uint;
/// The light scene's side until the world camera's target size is known.
const INITIAL_LIGHT_SCENE_SIDE: u32 = 1;
/// The fragment entry point of the art shaders' light scene pipelines.
const LIGHT_SCENE_ENTRY: &str = "light_scene_fragment";

/// The light glows' fragment shader.
const LIGHT_CLIP_SHADER: &str = "shaders/world/art/light_clip.wesl";
/// The light clip tag's fields (art/light_clip.wesl): the light's cell from the cell under the quad's origin, x then
/// y, each in `LIGHT_CLIP_CELL_BITS` signed bits; then its z and its ceiling, each a signed byte.
const LIGHT_CLIP_CELL_BITS: u32 = 6;
const LIGHT_CLIP_DY_SHIFT: u32 = LIGHT_CLIP_CELL_BITS;
const LIGHT_CLIP_Z_SHIFT: u32 = 2 * LIGHT_CLIP_CELL_BITS;
const LIGHT_CLIP_CEILING_SHIFT: u32 = LIGHT_CLIP_Z_SHIFT + u8::BITS;
const LIGHT_CLIP_CELL_MASK: u32 = (1 << LIGHT_CLIP_CELL_BITS) - 1;
/// The farthest a light's cell may lie from the cell under its quad's origin, along each axis: the sprites the
/// tileart's offsets throw farthest land about 5 cells away.
const LIGHT_CLIP_CELL_REACH: i32 = (1 << (LIGHT_CLIP_CELL_BITS - 1)) - 1;
/// The ceiling of a light with nothing over it.
const LIGHT_CLIP_NO_CEILING: i8 = i8::MIN;

/// The light scene class bits of a static with tile flags `flags` and tiledata height `height`:
/// `STATIC_FLAG_LIGHT_WALL` for a wall, `STATIC_FLAG_LIGHT_BLOCK` for another impassable static, each with its
/// height (`STATIC_LIGHT_HEIGHT_SHIFT`), and none for a floor, a roof, foliage, a see-through static or anything a
/// light shows on from above.
pub(crate) fn static_light_class_flags(flags: u64, height: i8) -> u32 {
    let class = if flags & LIGHT_SCENE_NEVER_SOLID_FLAGS != 0 {
        0
    } else if flags & TaeFlag::Wall.bits() != 0 {
        STATIC_FLAG_LIGHT_WALL
    } else if flags & TaeFlag::Impassable.bits() != 0 {
        STATIC_FLAG_LIGHT_BLOCK
    } else {
        0
    };
    if class == 0 {
        return 0;
    }
    class | (height.max(0) as u32 & STATIC_LIGHT_HEIGHT_MASK) << STATIC_LIGHT_HEIGHT_SHIFT
}

/// The `MeshTag` of a clipped glow whose quad sits at `origin`: its light's cell (`tile_x`, `tile_y`), from the cell
/// under the origin, its z and its ceiling (`light_cover::static_light_ceiling_z`). A cell beyond
/// `LIGHT_CLIP_CELL_REACH` is clamped to it.
pub(crate) fn light_clip_tag(tile_x: u32, tile_y: u32, z: i8, ceiling: Option<i8>, origin: Vec3) -> u32 {
    let offset = |tile: u32, origin: f32| {
        let offset = i64::from(tile) - origin.floor() as i64;
        let clamped = offset.clamp(-i64::from(LIGHT_CLIP_CELL_REACH) - 1, i64::from(LIGHT_CLIP_CELL_REACH));
        clamped as u32 & LIGHT_CLIP_CELL_MASK
    };
    offset(tile_x, origin.x)
        | offset(tile_y, origin.z) << LIGHT_CLIP_DY_SHIFT
        | u32::from(z as u8) << LIGHT_CLIP_Z_SHIFT
        | u32::from(ceiling.unwrap_or(LIGHT_CLIP_NO_CEILING) as u8) << LIGHT_CLIP_CEILING_SHIFT
}

/// The light scene image, and whether it is drawn: the active style's light cover is "uod" and some glow shows.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct LightScene {
    pub image: Handle<Image>,
    pub active: bool,
}

/// How a glow's material draws (`LightClipExtension`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightClipMode {
    /// As Bevy's unlit StandardMaterial: the "classicuo" and "clients" light covers.
    #[default]
    Unclipped,
    /// Clipped, in its own pass with no depth test: the decals in the world view's transparent pass.
    InPlace,
    /// Clipped, drawn by `render_deferred_light_sprites` and nothing in its own pass: the light sprites.
    Deferred,
}

pub type LightClipMaterial = ExtendedMaterial<StandardMaterial, LightClipExtension>;

/// The pipeline variant of a glow's material.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct LightClipKey {
    mode: LightClipMode,
}

impl From<&LightClipExtension> for LightClipKey {
    fn from(material: &LightClipExtension) -> Self {
        Self { mode: material.mode }
    }
}

/// A glow's material over Bevy's unlit StandardMaterial: the light scene it reads, and how it draws.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[bind_group_data(LightClipKey)]
pub struct LightClipExtension {
    #[texture(100, sample_type = "u_int")]
    pub scene: Handle<Image>,
    pub mode: LightClipMode,
}

impl MaterialExtension for LightClipExtension {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        LIGHT_CLIP_SHADER.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let depth_compare = match key.bind_group_data.mode {
            LightClipMode::Unclipped => return Ok(()),
            LightClipMode::InPlace => CompareFunction::Always,
            LightClipMode::Deferred => CompareFunction::Never,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(ShaderDefVal::Bool("LIGHT_CLIP".into(), true));
        }
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_compare = Some(depth_compare);
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// A glow's material for `scene`, drawn as `mode` gives.
pub(crate) fn light_clip_material(base: StandardMaterial, scene: &LightScene, mode: LightClipMode) -> LightClipMaterial {
    LightClipMaterial { base, extension: LightClipExtension { scene: scene.image.clone(), mode } }
}

/// Set the glow materials `handles` to draw as `mode`.
pub(crate) fn set_light_clip_mode<'a>(
    materials: &mut Assets<LightClipMaterial>,
    handles: impl IntoIterator<Item = &'a Handle<LightClipMaterial>>,
    mode: LightClipMode,
) {
    for handle in handles {
        if materials.get(handle).is_some_and(|material| material.extension.mode != mode)
            && let Some(mut material) = materials.get_mut(handle)
        {
            material.extension.mode = mode;
        }
    }
}

pub(super) fn install(app: &mut App) {
    app.add_plugins((
        MaterialPlugin::<LightClipMaterial>::default(),
        ExtractResourcePlugin::<LightScene>::default(),
    ))
    .add_systems(Startup, sys_setup_light_scene)
    .add_systems(PostUpdate, sys_sync_light_scene_image.after(CameraUpdateSystems));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app.init_resource::<LightScenePipelines>().add_systems(
        Core3d,
        (render_light_scene, render_deferred_light_sprites)
            .chain()
            .after(main_opaque_pass_3d)
            .before(main_transparent_pass_3d),
    );
}

fn sys_setup_light_scene(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image =
        Image::new_target_texture(INITIAL_LIGHT_SCENE_SIDE, INITIAL_LIGHT_SCENE_SIDE, LIGHT_SCENE_FORMAT, None);
    commands.insert_resource(LightScene { image: images.add(image), active: false });
}

/// Draw the light scene while a clipped glow draws, at the world camera's target size: a decal, or a light sprite
/// while the light buffer camera renders. A resized image is a new
/// texture, which the glow materials bind anew.
fn sys_sync_light_scene_image(
    scene: Option<ResMut<LightScene>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<LightClipMaterial>>,
    glows: Query<&MeshMaterial3d<LightClipMaterial>, With<Mesh3d>>,
    world_cameras: Query<&Camera, (With<PlayerCamera>, Without<LightBufferCamera>)>,
    light_cameras: Query<&Camera, With<LightBufferCamera>>,
) {
    let (Some(mut scene), Ok(world_camera)) = (scene, world_cameras.single()) else {
        return;
    };
    let light_buffer_drawn = light_cameras.iter().any(|camera| camera.is_active);
    let active = glows.iter().any(|material| {
        materials.get(material).is_some_and(|material| match material.extension.mode {
            LightClipMode::Unclipped => false,
            LightClipMode::InPlace => true,
            LightClipMode::Deferred => light_buffer_drawn,
        })
    });
    if scene.active != active {
        scene.active = active;
    }
    let Some(size) = world_camera.physical_target_size().filter(|_| active) else {
        return;
    };
    let extent = Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 };
    if images.get(&scene.image).is_some_and(|image| image.texture_descriptor.size != extent)
        && let Some(mut image) = images.get_mut(&scene.image)
    {
        image.resize(extent);
        let ids = materials.ids().collect::<Vec<_>>();
        for id in ids {
            if let Some(mut material) = materials.get_mut(id) {
                material.extension.scene = scene.image.clone();
            }
        }
    }
}

/// A pipeline derived from the art's or the light sprites'.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DerivedPipeline {
    /// `light_scene_pipeline`, over a multisampled depth or not.
    LightScene { multisampled: bool },
    /// `deferred_sprite_pipeline`.
    DeferredSprite,
}

/// The derived pipelines by original, `None` where the original has none.
#[derive(Resource, Default)]
struct LightScenePipelines(Mutex<HashMap<(CachedRenderPipelineId, DerivedPipeline), Option<CachedRenderPipelineId>>>);

fn light_scene_layout(multisampled: bool) -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new("light scene depth", &[BindGroupLayoutEntry {
        binding: 0,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Depth,
            view_dimension: TextureViewDimension::D2,
            multisampled,
        },
        count: None,
    }])
}

/// An art pipeline as the light scene draws it: its light scene entry into `LIGHT_SCENE_FORMAT`, single-sampled, the
/// view's depth bound in group 2 (which art leaves unused) in place of a depth attachment.
fn light_scene_pipeline(
    cache: &PipelineCache,
    original: CachedRenderPipelineId,
    multisampled: bool,
) -> Option<CachedRenderPipelineId> {
    let mut descriptor = cache.get_render_pipeline_descriptor(original).clone();
    descriptor.label = Some("light scene".into());
    descriptor.layout[2] = light_scene_layout(multisampled);
    let defs = [
        ShaderDefVal::Bool("LIGHT_SCENE".into(), true),
        ShaderDefVal::Bool("LIGHT_SCENE_MSAA".into(), multisampled),
    ];
    descriptor.vertex.shader_defs.extend(defs.clone());
    let fragment = descriptor.fragment.as_mut()?;
    fragment.entry_point = Some(LIGHT_SCENE_ENTRY.into());
    fragment.shader_defs.extend(defs);
    fragment.targets = vec![Some(ColorTargetState { format: LIGHT_SCENE_FORMAT, blend: None, write_mask: ColorWrites::ALL })];
    descriptor.depth_stencil = None;
    descriptor.multisample = MultisampleState::default();
    Some(cache.queue_render_pipeline(descriptor))
}

/// A light sprite's pipeline as the world view draws it into the light buffer, `None` for a pipeline that is not a
/// deferred glow's (`LightClipMode::Deferred`): without the depth test that kept it out of its own pass, into
/// `format`.
fn deferred_sprite_pipeline(
    cache: &PipelineCache,
    original: CachedRenderPipelineId,
    format: TextureFormat,
) -> Option<CachedRenderPipelineId> {
    let mut descriptor = cache.get_render_pipeline_descriptor(original).clone();
    let deferred = descriptor.depth_stencil.as_ref().and_then(|depth| depth.depth_compare) == Some(CompareFunction::Never);
    if !deferred {
        return None;
    }
    descriptor.label = Some("deferred light sprite".into());
    descriptor.depth_stencil = None;
    for target in descriptor.fragment.as_mut()?.targets.iter_mut().flatten() {
        target.format = format;
    }
    Some(cache.queue_render_pipeline(descriptor))
}

/// The light scene: the world view's static batches redrawn after its opaque pass, each fragment that drew its
/// pixel writing its static's texel. The light buffer camera's view holds no statics.
fn render_light_scene(
    world: &World,
    view: ViewQuery<(&ExtractedView, &ViewDepthStencilTexture), Without<LightBufferCamera>>,
    scene: Option<Res<LightScene>>,
    images: Res<RenderAssets<GpuImage>>,
    pipelines: Res<LightScenePipelines>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    batches: Query<&MainEntity, With<StaticTransparentBatchOrder>>,
    mut context: RenderContext,
) {
    let Some(scene) = scene.filter(|scene| scene.active) else { return };
    let Some(target) = images.get(&scene.image) else { return };
    let view_entity = view.entity();
    let (extracted, depth) = view.into_inner();
    let Some(phase) = phases.get(&extracted.retained_view_entity) else { return };
    let size = depth.texture().size();
    // Until the image takes the view's size the glows go unclipped (light_clip.wesl).
    if target.size_2d() != UVec2::new(size.width, size.height) {
        return;
    }
    let multisampled = depth.texture().sample_count() > 1;
    // The phase items name their batch by its main world entity, as `gpu_picking` reads them.
    let batches = batches.iter().copied().collect::<HashSet<_>>();
    let items = {
        let mut derived = pipelines.0.lock().unwrap();
        phase
            .items
            .values()
            .filter(|item| batches.contains(&item.entity.1) && !item.batch_range.is_empty())
            .filter_map(|item| {
                let pipeline = (*derived
                    .entry((item.pipeline, DerivedPipeline::LightScene { multisampled }))
                    .or_insert_with(|| light_scene_pipeline(&cache, item.pipeline, multisampled)))?;
                cache.get_render_pipeline(pipeline).is_some().then_some((item, pipeline))
            })
            .collect::<Vec<_>>()
    };
    let layout = cache.get_bind_group_layout(&light_scene_layout(multisampled));
    let depth_group = device.create_bind_group("light scene depth", &layout, &[BindGroupEntry {
        binding: 0,
        resource: BindingResource::TextureView(&depth.attachment.texture.default_view),
    }]);
    let draw_functions = world.resource::<DrawFunctions<Transparent3d>>();
    let mut functions = draw_functions.write();
    functions.prepare(world);
    let draw_id = functions.id::<DrawPixelQuery>();
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("light_scene"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &target.texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(wgpu::Color::TRANSPARENT), store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut pass, "light_scene");
    pass.set_bind_group(2, &depth_group, &[]);
    for (original, pipeline) in items {
        let item = Transparent3d {
            pipeline,
            sorting_info: original.sorting_info,
            distance: original.distance,
            entity: original.entity,
            draw_function: draw_id,
            batch_range: original.batch_range.clone(),
            extra_index: original.extra_index.clone(),
            indexed: original.indexed,
        };
        if let Some(draw) = functions.get_mut(draw_id)
            && let Err(error) = draw.draw(world, &mut pass, view_entity, &item)
        {
            error!("Light scene draw failed: {error:?}");
            break;
        }
    }
    pass_span.end(&mut pass);
}

/// The light sprites of the light buffer camera's view, clipped by the light scene: drawn into the light buffer
/// after it, from the world view's graph, as the light buffer camera renders first.
/// Whether the indirect draw parameters of `item` lie inside this frame's buffers `buffers` (`DrawMesh` reads them
/// from the buffers of its phase type); an item drawn directly always does.
fn indirect_parameters_fit(item: &Transparent3d, buffers: Option<&IndirectParametersBuffers>) -> bool {
    let PhaseItemExtraIndex::IndirectParametersIndex { range, .. } = &item.extra_index else {
        return true;
    };
    let Some(phase_buffers) = buffers.and_then(|buffers| buffers.get(&TypeId::of::<Transparent3d>())) else {
        return false;
    };
    let (buffer, stride) = if item.indexed {
        (phase_buffers.indexed.data_buffer(), size_of::<IndirectParametersIndexed>())
    } else {
        (phase_buffers.non_indexed.data_buffer(), size_of::<IndirectParametersNonIndexed>())
    };
    buffer.is_some_and(|buffer| u64::from(range.end) * stride as u64 <= buffer.size())
}

fn render_deferred_light_sprites(
    world: &World,
    _world_view: ViewQuery<&ExtractedView, Without<LightBufferCamera>>,
    scene: Option<Res<LightScene>>,
    light_views: Query<(Entity, &ExtractedView, &ViewTarget), With<LightBufferCamera>>,
    pipelines: Res<LightScenePipelines>,
    cache: Res<PipelineCache>,
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    mut context: RenderContext,
) {
    if !scene.is_some_and(|scene| scene.active) {
        return;
    }
    let Ok((light_view, extracted, light_target)) = light_views.single() else { return };
    let (Some(phase), Some(output), Some(format)) = (
        phases.get(&extracted.retained_view_entity),
        light_target.out_texture(),
        light_target.out_texture_view_format(),
    ) else {
        return;
    };
    let indirect_buffers = world.get_resource::<IndirectParametersBuffers>();
    if phase.items.values().any(|item| !item.batch_range.is_empty() && !indirect_parameters_fit(item, indirect_buffers)) {
        // Seen in the first frames of a start: a light sprite's indirect draw past the end of this frame's buffer,
        // which wgpu refuses and Bevy quits on. The sprites skip the frame.
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            "Deferred light sprites skip a frame: their indirect draws lie past this frame's buffers.",
        );
        return;
    }
    let items = {
        let mut derived = pipelines.0.lock().unwrap();
        phase
            .items
            .values()
            .filter(|item| !item.batch_range.is_empty())
            .filter_map(|item| {
                let pipeline = (*derived
                    .entry((item.pipeline, DerivedPipeline::DeferredSprite))
                    .or_insert_with(|| deferred_sprite_pipeline(&cache, item.pipeline, format)))?;
                cache.get_render_pipeline(pipeline).is_some().then_some((item, pipeline))
            })
            .collect::<Vec<_>>()
    };
    if items.is_empty() {
        return;
    }
    let draw_functions = world.resource::<DrawFunctions<Transparent3d>>();
    let mut functions = draw_functions.write();
    functions.prepare(world);
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("deferred_light_sprites"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: output,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut pass, "deferred_light_sprites");
    for (original, pipeline) in items {
        let item = Transparent3d {
            pipeline,
            sorting_info: original.sorting_info,
            distance: original.distance,
            entity: original.entity,
            draw_function: original.draw_function,
            batch_range: original.batch_range.clone(),
            extra_index: original.extra_index.clone(),
            indexed: original.indexed,
        };
        if let Some(draw) = functions.get_mut(item.draw_function)
            && let Err(error) = draw.draw(world, &mut pass, light_view, &item)
        {
            error!("Deferred light sprite draw failed: {error:?}");
            break;
        }
    }
    pass_span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: u64 = TaeFlag::Wall.bits() | TaeFlag::Impassable.bits();
    const ROOF: u64 = TaeFlag::Roof.bits() | TaeFlag::Impassable.bits();
    const FLOOR: u64 = TaeFlag::Surface.bits() | TaeFlag::Impassable.bits();
    const TREE: u64 = TaeFlag::Background.bits() | TaeFlag::Impassable.bits();
    const LEAVES: u64 = TaeFlag::Foliage.bits() | TaeFlag::Impassable.bits();
    const WINDOW: u64 = WALL | TaeFlag::Window.bits();

    /// Walls (with their windows and doors) are walls; other solid statics, such as a tree's trunk, blocks; floors,
    /// roofs, foliage and passable statics neither.
    #[test]
    fn statics_take_their_light_scene_class_from_their_flags() {
        let class = |flags| static_light_class_flags(flags, 0);
        assert_eq!(class(WALL), STATIC_FLAG_LIGHT_WALL);
        assert_eq!(class(WINDOW), STATIC_FLAG_LIGHT_WALL);
        assert_eq!(class(TREE), STATIC_FLAG_LIGHT_BLOCK);
        for flags in [ROOF, FLOOR, LEAVES, 0, TaeFlag::Transparent.bits() | TaeFlag::Impassable.bits()] {
            assert_eq!(class(flags), 0, "{flags:#x}");
        }
    }

    /// The height of a wall or a block, which art/light_scene.wesl reads to tell its cap from its faces, rides in its
    /// own bits; the other statics carry none, and a negative height reads as none.
    #[test]
    fn walls_and_blocks_carry_their_tiledata_height() {
        let height = |bits: u32| bits >> STATIC_LIGHT_HEIGHT_SHIFT & STATIC_LIGHT_HEIGHT_MASK;
        // The lower wall of Britain's torch walkway (map 1, 1391,1567: 0x0084 at z 30, height 22).
        let wall = static_light_class_flags(WALL, 22);
        assert_eq!(wall & !(STATIC_LIGHT_HEIGHT_MASK << STATIC_LIGHT_HEIGHT_SHIFT), STATIC_FLAG_LIGHT_WALL);
        assert_eq!(height(wall), 22);
        assert_eq!(height(static_light_class_flags(TREE, i8::MAX)), i8::MAX as u32);
        assert_eq!(height(static_light_class_flags(WALL, -3)), 0);
        assert_eq!(static_light_class_flags(FLOOR, 22), 0);
        // Above the other is_wet_flags bits, the class bits the highest of them.
        assert!(STATIC_FLAG_LIGHT_BLOCK < 1 << STATIC_LIGHT_HEIGHT_SHIFT);
    }

    /// The fields light_clip.wesl unpacks: signed cell offsets from the cell under the origin, the z and the ceiling.
    fn unpack(tag: u32) -> (i32, i32, i8, i8) {
        let signed = |shift: u32| (((tag >> shift) & LIGHT_CLIP_CELL_MASK) as i32) << (32 - LIGHT_CLIP_CELL_BITS) >> (32 - LIGHT_CLIP_CELL_BITS);
        (signed(0), signed(LIGHT_CLIP_DY_SHIFT), (tag >> LIGHT_CLIP_Z_SHIFT) as u8 as i8, (tag >> LIGHT_CLIP_CEILING_SHIFT) as u8 as i8)
    }

    #[test]
    fn the_light_clip_tag_carries_the_cell_z_and_ceiling() {
        // A sprite thrown by the tileart's default offsets, (-5, -4), up and to the left of its item.
        let tag = light_clip_tag(1407, 1596, -12, Some(70), Vec3::new(1402.3, 9.0, 1592.9));
        assert_eq!(unpack(tag), (5, 4, -12, 70));
        let tag = light_clip_tag(1407, 1596, 50, None, Vec3::new(1407.5, 4.6, 1596.5));
        assert_eq!(unpack(tag), (0, 0, 50, LIGHT_CLIP_NO_CEILING));
        let tag = light_clip_tag(10, 10, 0, None, Vec3::new(12.5, 0.0, 11.0));
        assert_eq!(unpack(tag), (-2, -1, 0, LIGHT_CLIP_NO_CEILING));
    }

    #[test]
    fn a_light_cell_beyond_the_tag_reach_is_clamped() {
        let tag = light_clip_tag(100, 0, 0, None, Vec3::new(0.0, 0.0, 200.0));
        assert_eq!(unpack(tag), (LIGHT_CLIP_CELL_REACH, -LIGHT_CLIP_CELL_REACH - 1, 0, LIGHT_CLIP_NO_CEILING));
    }
}
