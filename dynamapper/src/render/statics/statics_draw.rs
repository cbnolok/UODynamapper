use super::DrawStaticSpritesPlugin;
use crate::settings::{ClientTextureSource, Settings};
use crate::render::uniform_state::{UniformSnapshot, UniformState};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::render::terrain;
use crate::render::statics::static_batches::{
    GroundTileInstance, RenderStaticChunkBatches, RenderStaticInstances, RenderStaticLandInstances, SpriteInstance,
    StaticChunkBatch, StaticChunkBatchKey,
};
use crate::render::statics::static_source::{KrStaticPackages, KrStaticSources, StaticLandStyle};
use crate::app::system_sets::StartupSystems;
use crate::render::assets::art::{
    ART_ATLAS_SLOTS, ArtPageAtlas, ArtPageAtlasSet, CLASSIC_ATLAS_SLOT, ENHANCED_ATLAS_SLOT, GroundArtPageAtlas,
    GroundArtPageAtlasHandles, SpriteArtPageAtlas, SpriteArtPageAtlasHandles,
};
use crate::content::{
    HuesPackageRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes,
    TileMetaPackageRes,
};
use crate::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{MeshTag, MeshVertexBufferLayoutRef};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites, DepthBiasState,
    PrimitiveTopology, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderDefVal;
use std::collections::HashMap;
use udd_assets::tex_art_cc::PagePixelFormat;

#[derive(ShaderType, Clone)]
pub struct SpriteParams {
    pub render_mode: u32,
    pub alpha_cutoff: f32,
    pub pass_mode: u32,
    pub hue_enabled: u32,
    pub map_width_tiles: f32,
    pub map_height_tiles: f32,
    pub _pad_sp: UVec2,
}

const PASS_MODE_OPAQUE: u32 = 0;
const PASS_MODE_TRANSPARENT: u32 = 1;
const PASS_MODE_SHADOW: u32 = 2;
/// The KR client's static shadow (art/client_shadow.wesl), drawn by the shadow material in place of the projected
/// shadow while `UniformState::client_shadows` is on.
const PASS_MODE_CLIENT_SHADOW: u32 = 3;
const ART_PROJECTED_SHADOW_DEPTH_BIAS: f32 = 64.0;
/// The client's shadow buffer S starts at 1 and each shadow texel multiplies it by 1 - alpha
/// (`UOSpriteShaderShadow` into a white target with multiplicative blending). Dynamapper keeps S in the main
/// target's alpha, which the opaque and translucent art and the land leave at 1 (masked art at 0.5 or more, which the
/// composite cannot tell from 1): `S = S * (1 - a)`, with the colour untouched. The client night composite then
/// darkens by `clamp(S + 1 - s)` (`lighting::client_night`).
const CLIENT_SHADOW_BUFFER_BLEND: BlendState = BlendState {
    color: BlendComponent::REPLACE,
    alpha: BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    },
};

/// The shadow material's pass for the active preset: the KR client's shadow where it is on, else the projected
/// shadow (which the presets switch on and off through `enable_art_projected_shadows`).
fn shadow_pass_mode(uniform_state: &UniformState) -> u32 {
    if uniform_state.client_shadows == Some(true) { PASS_MODE_CLIENT_SHADOW } else { PASS_MODE_SHADOW }
}

pub type ArtSpriteMaterial = ExtendedMaterial<StandardMaterial, ArtSpriteMaterialExtension>;
pub type ArtGroundMaterial = ExtendedMaterial<StandardMaterial, ArtGroundMaterialExtension>;

/// The pipeline variant of an art sprite material: the client shadow pass writes only the shadow buffer, and the
/// mobiles' materials read their instances' crossfades (`MOBILE_FRAMES_SHADER_DEF`).
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ArtSpriteMaterialKey {
    client_shadow: bool,
    mobile_frames: bool,
}

impl From<&ArtSpriteMaterialExtension> for ArtSpriteMaterialKey {
    fn from(material: &ArtSpriteMaterialExtension) -> Self {
        Self { client_shadow: material.params.pass_mode == PASS_MODE_CLIENT_SHADOW, mobile_frames: material.mobile_frames }
    }
}

/// The shader def of the mobiles' art sprite pipelines: their instances are `MobileSpriteInstance`s, each a sprite and
/// the crossfade of its frames (art/art_bindings.wesl, art/mobile_frames.wesl). The statics' pipelines go without it.
const MOBILE_FRAMES_SHADER_DEF: &str = "MOBILE_FRAMES";

#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[bind_group_data(ArtSpriteMaterialKey)]
pub struct ArtSpriteMaterialExtension {
    /// The Classic pages' atlas, whose sampler samples `atlas_enhanced` too (art/atlas_pages.wesl).
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[texture(104, dimension = "2d_array", visibility(vertex, fragment))]
    pub atlas_enhanced: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderBuffer>,
    #[uniform(103, visibility(vertex, fragment))]
    pub params: SpriteParams,
    #[uniform(105, visibility(vertex, fragment))]
    pub scene_uniform: terrain::mesh_material::SceneUniform,
    #[uniform(106, visibility(vertex, fragment))]
    pub effects_uniform: terrain::mesh_material::LandEffectsUniform,
    #[uniform(107, visibility(vertex, fragment))]
    pub global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms,
    #[texture(109, visibility(fragment))]
    #[sampler(108, visibility(fragment))]
    pub hues: Handle<Image>,
    #[texture(110, visibility(fragment))]
    pub visual_grunge_texture: Handle<Image>,
    #[texture(111, dimension = "2d_array", visibility(fragment))]
    #[sampler(113, visibility(fragment))]
    pub item_surface_atlas: Handle<Image>,
    #[texture(112, sample_type = "u_int", visibility(fragment))]
    pub item_surface_lookup: Handle<Image>,
    /// The KR client's grunge texture (client_grunge.rs), wrapping and bilinear.
    #[texture(114, visibility(fragment))]
    #[sampler(115, visibility(fragment))]
    pub client_grunge_texture: Handle<Image>,
    /// Whether `instances` are the mobiles' (`MobileSpriteInstance`, render/mobiles/draw.rs), whose frames crossfade:
    /// the pipeline compiles the shader with `MOBILE_FRAMES_SHADER_DEF`. Not a binding.
    pub mobile_frames: bool,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ArtGroundMaterialExtension {
    /// The Classic pages' atlas, whose sampler samples `atlas_enhanced` too (art/atlas_pages.wesl).
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[texture(104, dimension = "2d_array", visibility(vertex, fragment))]
    pub atlas_enhanced: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderBuffer>,
    #[uniform(103, visibility(vertex, fragment))]
    pub params: SpriteParams,
    #[uniform(105, visibility(vertex, fragment))]
    pub scene_uniform: terrain::mesh_material::SceneUniform,
    #[uniform(106, visibility(vertex, fragment))]
    pub effects_uniform: terrain::mesh_material::LandEffectsUniform,
    #[uniform(107, visibility(vertex, fragment))]
    pub global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms,
    #[texture(109, visibility(fragment))]
    #[sampler(108, visibility(fragment))]
    pub hues: Handle<Image>,
    #[texture(111, dimension = "2d_array", visibility(fragment))]
    #[sampler(116, visibility(fragment))]
    pub land_page_atlas: Handle<Image>,
    #[texture(112, sample_type = "u_int", visibility(fragment))]
    pub land_page_lookup: Handle<Image>,
    #[texture(113, visibility(fragment))]
    pub visual_grunge_texture: Handle<Image>,
    #[texture(114, dimension = "2d_array", visibility(fragment))]
    pub item_liquid_atlas: Handle<Image>,
    #[texture(115, sample_type = "u_int", visibility(fragment))]
    pub item_liquid_lookup: Handle<Image>,
    /// The KR client's grunge texture, as the sprite material's (client_grunge.rs).
    #[texture(117, visibility(fragment))]
    #[sampler(118, visibility(fragment))]
    pub client_grunge_texture: Handle<Image>,
}

#[derive(Resource, Clone)]
pub struct ArtSpriteRenderAssets {
    pub mesh: Handle<Mesh>,
    pub opaque_material: Handle<ArtSpriteMaterial>,
    pub transparent_material: Handle<ArtSpriteMaterial>,
    pub shadow_material: Handle<ArtSpriteMaterial>,
}

#[derive(Resource, Clone)]
pub struct ArtGroundRenderAssets {
    pub mesh: Handle<Mesh>,
    pub opaque_material: Handle<ArtGroundMaterial>,
    pub transparent_material: Handle<ArtGroundMaterial>,
}

fn projected_art_shadow_base_material() -> StandardMaterial {
    StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        unlit: true,
        depth_bias: ART_PROJECTED_SHADOW_DEPTH_BIAS,
        ..default()
    }
}

#[derive(Clone)]
struct LandPageBindingHandles {
    atlas: Handle<Image>,
    lookup: Handle<Image>,
}

#[derive(Resource, Default)]
pub struct StaticArtDrawDebugState {
    pub last_entity_count: Option<usize>,
    pub last_transparent_entity_count: Option<usize>,
    pub last_uploaded_instances: Option<usize>,
    pub last_ground_entity_count: Option<usize>,
    pub last_ground_transparent_entity_count: Option<usize>,
    pub last_uploaded_ground_instances: Option<usize>,
}

/// Instances changed since their last upload: the collector takes its outputs mutably only when it rebuilds them, and
/// an upload waits while the material is not loaded yet.
#[derive(Resource, Default)]
pub struct StaticArtUploadCache {
    sprite_pending: bool,
    ground_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StaticBatchSyncSignature {
    len: usize,
    hash: u64,
}

#[derive(Resource, Default)]
pub struct StaticBatchEntitySyncState {
    sprite: Option<StaticBatchSyncSignature>,
    sprite_shadow: Option<StaticBatchSyncSignature>,
    sprite_transparent: Option<StaticBatchSyncSignature>,
    ground: Option<StaticBatchSyncSignature>,
    ground_transparent: Option<StaticBatchSyncSignature>,
}

#[derive(Resource, Default)]
pub struct ActiveArtAtlasBindingState {
    /// The art source the bound atlases are built for (`art_atlas_source`).
    pub configured_source: Option<ClientTextureSource>,
    /// Whether the bound Enhanced atlases hold the pages of the Kingdom Reborn item art and land too
    /// (`atlas_kingdom_reborn_sources`).
    pub kingdom_reborn: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AtlasAllocationSpec {
    page_width: u32,
    page_height: u32,
    initial_layers: u32,
    max_layers: u32,
    pixel_format: PagePixelFormat,
}

impl MaterialExtension for ArtSpriteMaterialExtension {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/main.wesl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/main.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.mobile_frames {
            let def = ShaderDefVal::Bool(MOBILE_FRAMES_SHADER_DEF.into(), true);
            descriptor.vertex.shader_defs.push(def.clone());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push(def);
            }
        }
        if !key.bind_group_data.client_shadow {
            return Ok(());
        }
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(CLIENT_SHADOW_BUFFER_BLEND);
                target.write_mask = ColorWrites::ALPHA;
            }
        }
        // The fragment writes its own depth, the caster's (client_shadow.wesl); no projected-shadow bias.
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.bias = DepthBiasState::default();
        }
        Ok(())
    }
}

impl MaterialExtension for ArtGroundMaterialExtension {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/ground.wesl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/ground.wesl".into()
    }
}

#[derive(Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticsDrawEntity;

#[derive(Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticsTransparentDrawEntity;

#[derive(Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticsShadowDrawEntity;

#[derive(Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticsGroundDrawEntity;

#[derive(Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticsGroundTransparentDrawEntity;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[component(clone_behavior = Ignore)]
pub struct StaticChunkBatchEntity {
    pub key: StaticChunkBatchKey,
    pub start: u32,
    pub count: u32,
}

fn static_batch_sync_signature(batches: &[StaticChunkBatch]) -> StaticBatchSyncSignature {
    let mut hash = 0xcbf29ce484222325u64;
    for batch in batches {
        static_batch_sync_hash_u32(&mut hash, batch.key.map_id);
        static_batch_sync_hash_u32(&mut hash, batch.key.gx);
        static_batch_sync_hash_u32(&mut hash, batch.key.gy);
        static_batch_sync_hash_u32(&mut hash, batch.key.scale);
        static_batch_sync_hash_u32(&mut hash, batch.start);
        static_batch_sync_hash_u32(&mut hash, batch.count);
    }
    StaticBatchSyncSignature {
        len: batches.len(),
        hash,
    }
}

fn static_batch_sync_hash_u32(hash: &mut u64, value: u32) {
    *hash ^= u64::from(value);
    *hash = hash.wrapping_mul(0x100000001b3);
}

/// The batch meshes, one per instance count, which the batches of that many instances share: each quad carries its
/// index within the batch (`uv_b.x`), and the batch entity's `MeshTag` its first instance, the art shaders reading
/// instance tag + index. A batch whose first instance moves, as the chunks before it come and go, keeps its mesh.
/// Before, the mesh carried the absolute index: every such move rebuilt and re-uploaded the meshes of all the batches
/// after it, twice for the sprites with their shadows, 10-18 ms a frame in the Britain walk (2026-11-19).
#[derive(Resource, Default)]
pub struct ArtBatchMeshes {
    by_count: HashMap<u32, Handle<Mesh>>,
}

impl ArtBatchMeshes {
    /// The mesh of `count` instances, built at the first batch of that size.
    fn get(&mut self, meshes: &mut Assets<Mesh>, count: u32) -> Handle<Mesh> {
        self.by_count.entry(count).or_insert_with(|| meshes.add(build_art_batch_mesh(count))).clone()
    }

    /// Drops the meshes no batch entity holds any longer, which only this cache keeps alive.
    fn drop_unused(&mut self) {
        self.by_count.retain(|_, handle| match handle {
            Handle::Strong(strong) => std::sync::Arc::strong_count(strong) > 1,
            Handle::Uuid(..) => false,
        });
    }
}

/// Points the batch entity `entity`, drawn as `current`, at `batch`: a batch that moved takes its new first instance
/// in its tag, one that changed size the mesh of its new size.
fn retarget_batch_entity(
    commands: &mut Commands,
    entity: Entity,
    current: StaticChunkBatchEntity,
    batch: &StaticChunkBatch,
    batch_meshes: &mut ArtBatchMeshes,
    meshes: &mut Assets<Mesh>,
) {
    if current.start == batch.start && current.count == batch.count {
        return;
    }
    let mut entity = commands.entity(entity);
    entity.insert((
        MeshTag::new(batch.start),
        StaticChunkBatchEntity { key: batch.key, start: batch.start, count: batch.count },
    ));
    if current.count != batch.count {
        entity.insert(Mesh3d(batch_meshes.get(meshes, batch.count)));
    }
}

pub(crate) fn build_art_batch_mesh(count: u32) -> Mesh {
    use bevy::mesh::Indices;

    let quad_count = count as usize;
    let vertex_count = quad_count * 4;
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut uv0 = Vec::with_capacity(vertex_count);
    let mut uv1 = Vec::with_capacity(vertex_count);
    let mut colors = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(quad_count * 6);

    for local_index in 0..count {
        let vertex_base = local_index * 4;
        positions.extend_from_slice(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ]);
        normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
        uv0.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
        uv1.extend_from_slice(&[[local_index as f32, 0.0]; 4]);
        colors.extend_from_slice(&[[1.0, 1.0, 1.0, 1.0]; 4]);
        indices.extend_from_slice(&[
            vertex_base,
            vertex_base + 2,
            vertex_base + 1,
            vertex_base + 1,
            vertex_base + 2,
            vertex_base + 3,
        ]);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn source_available(
    source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> bool {
    match source {
        ClientTextureSource::Cc => tex_art_cc_res.is_some(),
        ClientTextureSource::Ec => {
            tex_art_ec_res.is_some() && tex_land_ec_res.is_some() && tilemeta_res.is_some()
        }
    }
}

fn resolve_effective_art_source(
    requested_source: ClientTextureSource,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> Option<ClientTextureSource> {
    if source_available(
        requested_source,
        tex_art_cc_res,
        tex_art_ec_res,
        tex_land_ec_res,
        tilemeta_res,
    ) {
        return Some(requested_source);
    }

    ClientTextureSource::ALL.into_iter().find(|candidate| {
        *candidate != requested_source
            && source_available(
                *candidate,
                tex_art_cc_res,
                tex_art_ec_res,
                tex_land_ec_res,
                tilemeta_res,
            )
    })
}

/// The art source whose atlases the statics need: the active one, or Enhanced while Classic art draws the statics that
/// continue Enhanced or KR land the Enhanced way (`StaticLandStyle`). The Enhanced sprite atlas holds the Classic pages
/// too, and the ground atlas holds the land liquids.
fn art_atlas_source(
    active_source: Option<ClientTextureSource>,
    land_style: StaticLandStyle,
    enhanced_available: bool,
) -> Option<ClientTextureSource> {
    match active_source {
        Some(ClientTextureSource::Cc) if land_style.is_enhanced() && enhanced_available => {
            Some(ClientTextureSource::Ec)
        }
        source => source,
    }
}

/// The Kingdom Reborn packages the Enhanced atlases hold the pages of: under the Kingdom Reborn land, when the statics
/// that continue it are drawn from them (`static_source::kingdom_reborn_static_visual_kind`).
fn atlas_kingdom_reborn_sources<'a>(
    atlas_source: Option<ClientTextureSource>,
    land_style: StaticLandStyle,
    kr_packages: &'a KrStaticPackages,
) -> Option<KrStaticSources<'a>> {
    kr_packages.sources().filter(|_| atlas_source == Some(ClientTextureSource::Ec)
        && land_style == StaticLandStyle::KingdomReborn)
}

/// The layers each atlas of a set starts with, by slot (`art_atlas_slot`), unless it holds fewer pages. Starting at
/// what the frame benchmark's Britain walks made resident (frame_benchmark.py: 21 Classic layers in the Classic view, 9
/// and 9 in the Enhanced ones) took their growth copies off the walks, but cost the Kingdom Reborn view 7-8% on every
/// frame, while the growths it spared hitched no frame past 66 ms.
const ART_ATLAS_INITIAL_LAYERS: [u32; ART_ATLAS_SLOTS] = [4, 4];

/// The page the Enhanced views' Classic sprite atlas keeps for the missing texture (`ArtPageKey::Missing`), which
/// fits in it.
const MISSING_TEXTURE_PAGES: usize = 1;
const MISSING_TEXTURE_PAGE_SIZE: u32 = 256;

/// The atlases of a set, as its packages fill them (`ArtPageAtlasSet`): each as large as its largest pages, in their
/// format, which the packages of an atlas share (a Kingdom Reborn section in another format than its Enhanced
/// package's does not load: `content::kr_section_page_mismatch`). An atlas no package fills stays `None`.
#[derive(Clone, Copy, Default)]
struct AtlasSetSpec {
    slots: [Option<AtlasAllocationSpec>; ART_ATLAS_SLOTS],
    formats: [Option<PagePixelFormat>; ART_ATLAS_SLOTS],
}

impl AtlasSetSpec {
    /// Room in the atlas `slot` for `pages` more pages of `width`×`height`, stored in `pixel_format`; `None` for pages
    /// that take the atlas's format on upload, as the missing texture's.
    fn with_pages(mut self, slot: usize, pages: usize, width: u32, height: u32, pixel_format: Option<PagePixelFormat>)
        -> Self {
        if pages == 0 {
            return self;
        }
        let spec = self.slots[slot].get_or_insert(AtlasAllocationSpec {
            page_width: width,
            page_height: height,
            initial_layers: 0,
            max_layers: 0,
            pixel_format: PagePixelFormat::Rgba8888,
        });
        spec.page_width = spec.page_width.max(width);
        spec.page_height = spec.page_height.max(height);
        spec.max_layers += pages as u32;
        if self.formats[slot].is_none() {
            self.formats[slot] = pixel_format;
        }
        self
    }

    /// Each atlas's allocation, starting with `ART_ATLAS_INITIAL_LAYERS`; an atlas no package fills is a blank one.
    fn allocations(self) -> [AtlasAllocationSpec; ART_ATLAS_SLOTS] {
        std::array::from_fn(|slot| self.slots[slot].map_or_else(blank_atlas_spec, |spec| AtlasAllocationSpec {
            initial_layers: ART_ATLAS_INITIAL_LAYERS[slot].min(spec.max_layers),
            pixel_format: self.formats[slot].unwrap_or(PagePixelFormat::Rgba8888),
            ..spec
        }))
    }
}

/// A package's pages' format: its first page's, RGBA8888 without pages.
fn package_pixel_format<T>(pages: &[T], get_pixel_format: impl Fn(&T) -> PagePixelFormat) -> PagePixelFormat {
    pages.first().map_or(PagePixelFormat::Rgba8888, get_pixel_format)
}

fn blank_atlas_spec() -> AtlasAllocationSpec {
    AtlasAllocationSpec {
        page_width: 1,
        page_height: 1,
        initial_layers: 1,
        max_layers: 1,
        pixel_format: PagePixelFormat::Rgba8888,
    }
}

/// `kingdom_reborn` adds the pages of the Kingdom Reborn item art to the Enhanced atlas (`atlas_kingdom_reborn_sources`).
fn sprite_atlas_spec(
    active_source: Option<ClientTextureSource>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    kingdom_reborn: Option<KrStaticSources>,
) -> AtlasSetSpec {
    let classic = |spec: AtlasSetSpec| match tex_art_cc_res {
        Some(package) => spec.with_pages(CLASSIC_ATLAS_SLOT, package.0.pages().len(), package.0.atlas_width(),
            package.0.atlas_height(), Some(package_pixel_format(package.0.pages(), |page| page.pixel_format))),
        None => spec,
    };
    match active_source {
        Some(ClientTextureSource::Cc) => classic(AtlasSetSpec::default()),
        Some(ClientTextureSource::Ec) => {
            let mut spec = classic(AtlasSetSpec::default()).with_pages(CLASSIC_ATLAS_SLOT, MISSING_TEXTURE_PAGES,
                MISSING_TEXTURE_PAGE_SIZE, MISSING_TEXTURE_PAGE_SIZE, None);
            if let Some(package) = tex_art_ec_res {
                spec = spec.with_pages(ENHANCED_ATLAS_SLOT, package.0.pages().len(), package.0.atlas_width(),
                    package.0.atlas_height(), Some(package_pixel_format(package.0.pages(), |page| page.pixel_format)));
            }
            match kingdom_reborn {
                Some(kr) => spec.with_pages(ENHANCED_ATLAS_SLOT, kr.art.pages().len(), kr.art.atlas_width(),
                    kr.art.atlas_height(), Some(package_pixel_format(kr.art.pages(), |page| page.pixel_format))),
                None => spec,
            }
        }
        None => AtlasSetSpec::default(),
    }
}

/// `kingdom_reborn` adds the pages of the Kingdom Reborn land to the Enhanced atlas (`atlas_kingdom_reborn_sources`).
fn ground_atlas_spec(
    active_source: Option<ClientTextureSource>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    kingdom_reborn: Option<KrStaticSources>,
) -> AtlasSetSpec {
    let empty = AtlasSetSpec::default();
    if active_source != Some(ClientTextureSource::Ec) {
        return empty;
    }

    let spec = match tex_land_ec_res {
        Some(package) => empty.with_pages(ENHANCED_ATLAS_SLOT, package.0.pages().len(),
            package.0.atlas_width(), package.0.atlas_height(),
            Some(package_pixel_format(package.0.pages(), |page| page.pixel_format))),
        None => empty,
    };
    match kingdom_reborn {
        Some(kr) => spec.with_pages(ENHANCED_ATLAS_SLOT, kr.land.pages().len(), kr.land.atlas_width(),
            kr.land.atlas_height(), Some(package_pixel_format(kr.land.pages(), |page| page.pixel_format))),
        None => spec,
    }
}

fn atlas_texture_format(pixel_format: PagePixelFormat) -> TextureFormat {
    match pixel_format {
        PagePixelFormat::Rgba8888 => TextureFormat::Rgba8UnormSrgb,
        PagePixelFormat::Bc7 => TextureFormat::Bc7RgbaUnormSrgb,
    }
}

fn create_art_atlas_image(
    images: &mut Assets<Image>,
    page_width: u32,
    page_height: u32,
    layers: u32,
    pixel_format: PagePixelFormat,
) -> Handle<Image> {
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    // No CPU data: the GPU zero-fills the layers, and a growth copies the resident ones from the image it replaces
    // (`ArtPageAtlas::apply_resize`), which `new_uninit`'s COPY_SRC usage allows.
    let mut image = Image::new_uninit(
        Extent3d {
            width: page_width,
            height: page_height,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        atlas_texture_format(pixel_format),
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    images.add(image)
}

fn create_hue_lookup_image(
    images: &mut Assets<Image>,
    hues_package: Option<&HuesPackageRes>,
) -> (Handle<Image>, u32) {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureUsages};

    let texture_bytes = hues_package.map(|package| package.0.texture_bytes().to_vec());
    let hue_enabled = texture_bytes.is_some() as u32;
    let data = texture_bytes.unwrap_or_else(|| {
        vec![
            255;
            udd_assets::hues::HUES_TEXTURE_WIDTH as usize
                * udd_assets::hues::HUES_TEXTURE_HEIGHT as usize
                * 4
        ]
    });

    let mut image = Image::new(
        Extent3d {
            width: udd_assets::hues::HUES_TEXTURE_WIDTH,
            height: udd_assets::hues::HUES_TEXTURE_HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    (images.add(image), hue_enabled)
}

fn create_blank_land_page_binding_images(images: &mut Assets<Image>) -> LandPageBindingHandles {
    use bevy::render::render_resource::{
        Extent3d, TextureDimension, TextureUsages, TextureViewDescriptor, TextureViewDimension,
    };

    let mut atlas = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; 4],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    atlas.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;
    atlas.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });

    let mut lookup = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; 16],
        TextureFormat::Rgba32Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    lookup.texture_descriptor.usage |= TextureUsages::TEXTURE_BINDING;

    LandPageBindingHandles {
        atlas: images.add(atlas),
        lookup: images.add(lookup),
    }
}

fn land_page_binding_handles(
    shared_land_material: Option<&terrain::draw_mesh::SharedLandMaterial>,
    land_materials: &Assets<terrain::mesh_material::LandCustomMeshMaterial>,
) -> Option<LandPageBindingHandles> {
    let material = land_materials.get(&shared_land_material?.0)?;
    Some(LandPageBindingHandles {
        atlas: material.extension.land_page_atlas.clone(),
        lookup: material.extension.land_page_lookup.clone(),
    })
}

fn build_art_atlas(images: &mut Assets<Image>, spec: AtlasAllocationSpec) -> ArtPageAtlas {
    let handle = create_art_atlas_image(
        images,
        spec.page_width,
        spec.page_height,
        spec.initial_layers,
        spec.pixel_format,
    );
    ArtPageAtlas::new(
        handle,
        spec.page_width,
        spec.page_height,
        spec.initial_layers,
        spec.max_layers,
        spec.pixel_format,
    )
}

fn build_art_atlas_set(images: &mut Assets<Image>, spec: AtlasSetSpec) -> ArtPageAtlasSet {
    ArtPageAtlasSet { atlases: spec.allocations().map(|allocation| build_art_atlas(images, allocation)) }
}

const BYTES_PER_MB: u64 = 1024 * 1024;

/// The pages each atlas of a set holds, by slot, for the logs.
const ART_ATLAS_CONTENT_NAMES: [&str; ART_ATLAS_SLOTS] = ["Classic", "Enhanced"];

/// A growth allocates the whole new array and copies the resident layers into it, a stall that grows with the atlas.
fn log_art_atlas_growth(atlas_name: &str, atlas: &ArtPageAtlas, new_layers: u32) {
    let format = atlas_texture_format(atlas.pixel_format);
    let (block_width, block_height) = format.block_dimensions();
    let block_bytes = u64::from(format.block_copy_size(None).unwrap_or(0));
    let layer_bytes = u64::from(atlas.page_width.div_ceil(block_width)) * u64::from(atlas.page_height.div_ceil(block_height))
        * block_bytes;
    console_logger::one(
        LogSev::Info,
        LogAbout::Performance,
        &format!(
            "Growing the {atlas_name} art atlas: {} → {new_layers} layers of {}x{} {format:?} ({} MB).",
            atlas.active_layers,
            atlas.page_width,
            atlas.page_height,
            layer_bytes * u64::from(new_layers) / BYTES_PER_MB,
        ),
    );
}

/// Grows the atlases of `atlases` that asked to, each into a new image (`ArtPageAtlas::apply_resize`), and says whether
/// any did.
fn apply_art_atlas_set_resizes(
    images: &mut Assets<Image>,
    atlas_name: &str,
    atlases: &mut ArtPageAtlasSet,
    atlas_handles: &mut [Handle<Image>; ART_ATLAS_SLOTS],
) -> bool {
    let mut grown = false;
    for ((atlas, atlas_handle), content) in atlases.atlases.iter_mut().zip(atlas_handles).zip(ART_ATLAS_CONTENT_NAMES) {
        let Some(new_layers) = atlas.take_resize_request() else {
            continue;
        };
        if new_layers <= atlas.active_layers {
            continue;
        }

        log_art_atlas_growth(&format!("{content} {atlas_name}"), atlas, new_layers);
        let new_handle = create_art_atlas_image(
            images,
            atlas.page_width,
            atlas.page_height,
            new_layers,
            atlas.pixel_format,
        );
        atlas.apply_resize(new_handle.clone(), new_layers);
        *atlas_handle = new_handle;
        grown = true;
    }
    grown
}

/// Binds the sprite atlases' images to the sprite materials.
fn bind_sprite_art_atlases(
    materials: &mut Assets<ArtSpriteMaterial>,
    render_assets: &ArtSpriteRenderAssets,
    [atlas, atlas_enhanced]: &[Handle<Image>; ART_ATLAS_SLOTS],
) {
    for material in [&render_assets.opaque_material, &render_assets.transparent_material, &render_assets.shadow_material] {
        if let Some(mut material) = materials.get_mut(material) {
            material.extension.atlas = atlas.clone();
            material.extension.atlas_enhanced = atlas_enhanced.clone();
        }
    }
}

/// Binds the ground atlases' images to the ground materials.
fn bind_ground_art_atlases(
    materials: &mut Assets<ArtGroundMaterial>,
    render_assets: &ArtGroundRenderAssets,
    [atlas, atlas_enhanced]: &[Handle<Image>; ART_ATLAS_SLOTS],
) {
    for material in [&render_assets.opaque_material, &render_assets.transparent_material] {
        if let Some(mut material) = materials.get_mut(material) {
            material.extension.atlas = atlas.clone();
            material.extension.atlas_enhanced = atlas_enhanced.clone();
        }
    }
}

fn rebind_active_art_atlases(
    images: &mut Assets<Image>,
    materials: &mut Assets<ArtSpriteMaterial>,
    ground_materials: &mut Assets<ArtGroundMaterial>,
    render_assets: &ArtSpriteRenderAssets,
    ground_render_assets: &ArtGroundRenderAssets,
    sprite_atlas: &mut SpriteArtPageAtlas,
    sprite_atlas_handles: &mut SpriteArtPageAtlasHandles,
    ground_atlas: &mut GroundArtPageAtlas,
    ground_atlas_handles: &mut GroundArtPageAtlasHandles,
    active_source: Option<ClientTextureSource>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
    kingdom_reborn: Option<KrStaticSources>,
) {
    let sprite_spec = sprite_atlas_spec(active_source, tex_art_cc_res, tex_art_ec_res, kingdom_reborn);
    let ground_spec = ground_atlas_spec(active_source, tex_land_ec_res, kingdom_reborn);

    sprite_atlas.0 = build_art_atlas_set(images, sprite_spec);
    sprite_atlas_handles.0 = sprite_atlas.handles();
    bind_sprite_art_atlases(materials, render_assets, &sprite_atlas_handles.0);

    ground_atlas.0 = build_art_atlas_set(images, ground_spec);
    ground_atlas_handles.0 = ground_atlas.handles();
    bind_ground_art_atlases(ground_materials, ground_render_assets, &ground_atlas_handles.0);
}

pub fn sys_setup_art_page_atlas(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    land_materials: Res<Assets<terrain::mesh_material::LandCustomMeshMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    settings: Res<Settings>,
    shared_land_material: Option<Res<terrain::draw_mesh::SharedLandMaterial>>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    hues_package_res: Option<Res<HuesPackageRes>>,
    // One tuple: a system takes at most 16 parameters.
    (item_liquid_images, item_surface_images): (
        Res<crate::render::assets::ec_item_liquid::EcItemLiquidImages>,
        Res<crate::render::assets::ec_item_surface::EcItemSurfaceImages>,
    ),
    client_grunge_packages: super::client_grunge::ClientGrungePackages,
) {
    log_system_add_startup::<DrawStaticSpritesPlugin>(StartupSystems::SetupSceneStage1, fname!());
    let active_source = resolve_effective_art_source(
        settings.graphics.art_texture_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    // The Kingdom Reborn pages join when the land style is known (`sys_sync_active_art_page_atlases`).
    let sprite_spec = sprite_atlas_spec(
        active_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        None,
    );
    let ground_spec = ground_atlas_spec(active_source, tex_land_ec_res.as_ref(), None);

    let sprite_atlas = build_art_atlas_set(&mut images, sprite_spec);
    let [sprite_atlas_handle, sprite_atlas_enhanced_handle] = sprite_atlas.handles();
    let ground_atlas = build_art_atlas_set(&mut images, ground_spec);
    let [ground_atlas_handle, ground_atlas_enhanced_handle] = ground_atlas.handles();
    let (hue_lookup_handle, hue_enabled) =
        create_hue_lookup_image(&mut images, hues_package_res.as_deref());
    let land_page_bindings =
        land_page_binding_handles(shared_land_material.as_deref(), &land_materials)
            .unwrap_or_else(|| create_blank_land_page_binding_images(&mut images));

    let initial_buffer = ShaderBuffer::from(vec![SpriteInstance {
        world_x: 0.0,
        world_z: 0.0,
        world_y: 0.0,
        layer: 0,
        depth_class: 0,
        base_world_y: 0.0,
        uv_min: [0.0, 0.0],
        uv_max: [0.0, 0.0],
        local_min: [0.0, 0.0],
        local_max: [0.0, 0.0],
        tile_x: 0.0,
        tile_y: 0.0,
        priority_z_units: 0.0,
        sort_bias_ordinal: 0,
        is_wet_flags: 0,
        hue_id: 0,
        hue_flags: 0,
        flat_quad: 0,
        texture_stretch: 0.0,
        material_item_id: 0,
        client_shadow_flags: 0,
        client_shadow_height: 0.0,
        client_grunge_anchor: [0.0; 2],
        local_light_corners: [0; 4],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);

    let buffer_handle = storage_buffers.add(initial_buffer);
    let opaque_sprite_atlas_handle = sprite_atlas_handle.clone();
    let transparent_sprite_atlas_handle = sprite_atlas_handle.clone();
    let visual_grunge_texture = images.add(crate::util_lib::image::visual_grunge_image(128));
    let (client_grunge_image, _) = client_grunge_packages.image();
    let client_grunge_texture = images.add(client_grunge_image);

    let opaque_material_handle = materials.add(ArtSpriteMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtSpriteMaterialExtension {
            atlas: opaque_sprite_atlas_handle,
            atlas_enhanced: sprite_atlas_enhanced_handle.clone(),
            instances: buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.5,
                pass_mode: PASS_MODE_OPAQUE,
                hue_enabled,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad_sp: UVec2::ZERO,
            },
            scene_uniform: terrain::mesh_material::SceneUniform::default(),
            effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
            mobile_frames: false,
        },
    });

    let transparent_material_handle = materials.add(ArtSpriteMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtSpriteMaterialExtension {
            atlas: transparent_sprite_atlas_handle,
            atlas_enhanced: sprite_atlas_enhanced_handle.clone(),
            instances: buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.5,
                pass_mode: PASS_MODE_TRANSPARENT,
                hue_enabled,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad_sp: UVec2::ZERO,
            },
            scene_uniform: terrain::mesh_material::SceneUniform::default(),
            effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
            mobile_frames: false,
        },
    });

    let shadow_material_handle = materials.add(ArtSpriteMaterial {
        base: projected_art_shadow_base_material(),
        extension: ArtSpriteMaterialExtension {
            atlas: sprite_atlas_handle.clone(),
            atlas_enhanced: sprite_atlas_enhanced_handle.clone(),
            instances: buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.0,
                pass_mode: PASS_MODE_SHADOW,
                hue_enabled: 0,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad_sp: UVec2::ZERO,
            },
            scene_uniform: terrain::mesh_material::SceneUniform::default(),
            effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
            mobile_frames: false,
        },
    });

    let initial_ground_buffer = ShaderBuffer::from(vec![GroundTileInstance {
        world_x: 0.0,
        world_z: 0.0,
        world_y: 0.0,
        layer: 0,
        depth_class: 0,
        base_world_y: 0.0,
        uv_min: [0.0, 0.0],
        uv_max: [0.0, 0.0],
        local_min: [0.0, 0.0],
        local_max: [1.0, 1.0],
        tile_x: 0.0,
        tile_y: 0.0,
        priority_z_units: 0.0,
        sort_bias_ordinal: 0,
        is_wet_flags: 0,
        texture_stretch: 0.0,
        hue_id: 0,
        hue_flags: 0,
        material_payload: 0,
        material_flags: 0,
        material_item_id: 0,
        _pad_material: 0,
        client_grunge_anchor: [0.0; 2],
        local_light_corners: [0; 4],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);
    let ground_buffer_handle = storage_buffers.add(initial_ground_buffer);
    let opaque_ground_atlas_handle = ground_atlas_handle.clone();
    let transparent_ground_atlas_handle = ground_atlas_handle.clone();

    let ground_material_handle = ground_materials.add(ArtGroundMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtGroundMaterialExtension {
            atlas: opaque_ground_atlas_handle,
            atlas_enhanced: ground_atlas_enhanced_handle.clone(),
            instances: ground_buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.5,
                pass_mode: PASS_MODE_OPAQUE,
                hue_enabled,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad_sp: UVec2::ZERO,
            },
            scene_uniform: terrain::mesh_material::SceneUniform::default(),
            effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            land_page_atlas: land_page_bindings.atlas.clone(),
            land_page_lookup: land_page_bindings.lookup.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_liquid_atlas: item_liquid_images.atlas.clone(),
            item_liquid_lookup: item_liquid_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
        },
    });

    let transparent_ground_material_handle = ground_materials.add(ArtGroundMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtGroundMaterialExtension {
            atlas: transparent_ground_atlas_handle,
            atlas_enhanced: ground_atlas_enhanced_handle.clone(),
            instances: ground_buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.5,
                pass_mode: PASS_MODE_TRANSPARENT,
                hue_enabled,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad_sp: UVec2::ZERO,
            },
            scene_uniform: terrain::mesh_material::SceneUniform::default(),
            effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle,
            land_page_atlas: land_page_bindings.atlas,
            land_page_lookup: land_page_bindings.lookup,
            visual_grunge_texture,
            item_liquid_atlas: item_liquid_images.atlas.clone(),
            item_liquid_lookup: item_liquid_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
        },
    });

    // Create a 1-triangle mesh that will be overridden by our instanced vertex shader
    // Actually we need 4 vertices for a quad.
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleStrip,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4]);
    // uv_b in shader usually corresponds to ATTRIBUTE_UV_1 if used by Bevy's default extractor,
    // but here we just need to satisfy the pipeline if the shader uses standard structs.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0, 0.0]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 4]);

    let mesh_handle = meshes.add(mesh);

    commands.insert_resource(ArtSpriteRenderAssets {
        mesh: mesh_handle.clone(),
        opaque_material: opaque_material_handle,
        transparent_material: transparent_material_handle,
        shadow_material: shadow_material_handle,
    });
    commands.insert_resource(ArtGroundRenderAssets {
        mesh: mesh_handle,
        opaque_material: ground_material_handle,
        transparent_material: transparent_ground_material_handle,
    });
    commands.insert_resource(SpriteArtPageAtlasHandles(sprite_atlas.handles()));
    commands.insert_resource(SpriteArtPageAtlas(sprite_atlas));
    commands.insert_resource(GroundArtPageAtlasHandles(ground_atlas.handles()));
    commands.insert_resource(GroundArtPageAtlas(ground_atlas));
    commands.insert_resource(ActiveArtAtlasBindingState {
        configured_source: active_source,
        kingdom_reborn: false,
    });
}

pub fn sys_sync_active_art_page_atlases(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    render_assets: Res<ArtSpriteRenderAssets>,
    ground_render_assets: Res<ArtGroundRenderAssets>,
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut sprite_atlas_handles: ResMut<SpriteArtPageAtlasHandles>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    mut ground_atlas_handles: ResMut<GroundArtPageAtlasHandles>,
    source_state: Res<crate::render::statics::static_source::StaticArtSourceState>,
    mut binding_state: ResMut<ActiveArtAtlasBindingState>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    kr_packages: KrStaticPackages,
) {
    let atlas_source = art_atlas_source(source_state.active_source, source_state.land_style,
        source_available(ClientTextureSource::Ec, tex_art_cc_res.as_ref(), tex_art_ec_res.as_ref(),
            tex_land_ec_res.as_ref(), tilemeta_res.as_ref()));
    let kingdom_reborn = atlas_kingdom_reborn_sources(atlas_source, source_state.land_style, &kr_packages);
    if binding_state.configured_source == atlas_source && binding_state.kingdom_reborn == kingdom_reborn.is_some() {
        return;
    }

    log_system_add_update::<DrawStaticSpritesPlugin>(fname!());

    rebind_active_art_atlases(
        &mut images,
        &mut materials,
        &mut ground_materials,
        &render_assets,
        &ground_render_assets,
        &mut sprite_atlas,
        &mut sprite_atlas_handles,
        &mut ground_atlas,
        &mut ground_atlas_handles,
        atlas_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        kingdom_reborn,
    );

    binding_state.configured_source = atlas_source;
    binding_state.kingdom_reborn = kingdom_reborn.is_some();
}

pub fn sys_apply_pending_art_page_atlas_resizes(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    render_assets: Res<ArtSpriteRenderAssets>,
    ground_render_assets: Res<ArtGroundRenderAssets>,
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut sprite_atlas_handles: ResMut<SpriteArtPageAtlasHandles>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    mut ground_atlas_handles: ResMut<GroundArtPageAtlasHandles>,
) {
    if apply_art_atlas_set_resizes(&mut images, "sprite", &mut sprite_atlas.0, &mut sprite_atlas_handles.0) {
        bind_sprite_art_atlases(&mut materials, &render_assets, &sprite_atlas_handles.0);
    }
    if apply_art_atlas_set_resizes(&mut images, "ground", &mut ground_atlas.0, &mut ground_atlas_handles.0) {
        bind_ground_art_atlases(&mut ground_materials, &ground_render_assets, &ground_atlas_handles.0);
    }
}

pub fn sys_sync_static_sprite_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut batch_meshes: ResMut<ArtBatchMeshes>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite == Some(signature) {
        return;
    }
    batch_meshes.drop_unused();

    let desired_count = chunk_batches.sprite.len();
    let existing_count = existing_q.iter().count();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            retarget_batch_entity(&mut commands, *entity, *current_batch, batch, &mut batch_meshes, &mut meshes);
        } else {
            commands.spawn((
                Mesh3d(batch_meshes.get(&mut meshes, batch.count)),
                MeshTag::new(batch.start),
                MeshMaterial3d(render_assets.opaque_material.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                StaticChunkBatchEntity {
                    key: batch.key,
                    start: batch.start,
                    count: batch.count,
                },
                StaticsDrawEntity,
            ));
        }
    }

    for (entity, batch) in existing_q.iter() {
        if !chunk_batches
            .sprite
            .iter()
            .any(|desired| desired.key == batch.key)
        {
            commands.entity(entity).despawn();
        }
    }

    if debug_state.last_entity_count != Some(desired_count) {
        console_logger::one(
            LogSev::Debug,
            LogAbout::RenderWorldArt,
            &format!(
                "static art draw entities: existing={} desired={}",
                existing_count, desired_count,
            ),
        );
        debug_state.last_entity_count = Some(desired_count);
    }

    sync_state.sprite = Some(signature);
}

pub fn sys_sync_static_sprite_shadow_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut batch_meshes: ResMut<ArtBatchMeshes>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsShadowDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite_shadow == Some(signature) {
        return;
    }
    batch_meshes.drop_unused();

    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            retarget_batch_entity(&mut commands, *entity, *current_batch, batch, &mut batch_meshes, &mut meshes);
        } else {
            commands.spawn((
                Mesh3d(batch_meshes.get(&mut meshes, batch.count)),
                MeshTag::new(batch.start),
                MeshMaterial3d(render_assets.shadow_material.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                StaticChunkBatchEntity {
                    key: batch.key,
                    start: batch.start,
                    count: batch.count,
                },
                StaticsShadowDrawEntity,
            ));
        }
    }

    for (entity, batch) in existing_q.iter() {
        if !chunk_batches
            .sprite
            .iter()
            .any(|desired| desired.key == batch.key)
        {
            commands.entity(entity).despawn();
        }
    }

    sync_state.sprite_shadow = Some(signature);
}

pub fn sys_sync_static_sprite_transparent_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut batch_meshes: ResMut<ArtBatchMeshes>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsTransparentDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite_transparent == Some(signature) {
        return;
    }
    batch_meshes.drop_unused();

    let desired_count = chunk_batches.sprite.len();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            retarget_batch_entity(&mut commands, *entity, *current_batch, batch, &mut batch_meshes, &mut meshes);
        } else {
            commands.spawn((
                Mesh3d(batch_meshes.get(&mut meshes, batch.count)),
                MeshTag::new(batch.start),
                MeshMaterial3d(render_assets.transparent_material.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                super::transparent_order::StaticTransparentBatchOrder { key: batch.key, kind: 1 },
                StaticChunkBatchEntity {
                    key: batch.key,
                    start: batch.start,
                    count: batch.count,
                },
                StaticsTransparentDrawEntity,
            ));
        }
    }

    for (entity, batch) in existing_q.iter() {
        if !chunk_batches
            .sprite
            .iter()
            .any(|desired| desired.key == batch.key)
        {
            commands.entity(entity).despawn();
        }
    }

    if debug_state.last_transparent_entity_count != Some(desired_count) {
        debug_state.last_transparent_entity_count = Some(desired_count);
    }

    sync_state.sprite_transparent = Some(signature);
}

pub fn sys_sync_static_ground_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut batch_meshes: ResMut<ArtBatchMeshes>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.ground);
    if sync_state.ground == Some(signature) {
        return;
    }
    batch_meshes.drop_unused();

    let desired_count = chunk_batches.ground.len();
    let existing_count = existing_q.iter().count();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.ground {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            retarget_batch_entity(&mut commands, *entity, *current_batch, batch, &mut batch_meshes, &mut meshes);
        } else {
            commands.spawn((
                Mesh3d(batch_meshes.get(&mut meshes, batch.count)),
                MeshTag::new(batch.start),
                MeshMaterial3d(render_assets.opaque_material.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                StaticChunkBatchEntity {
                    key: batch.key,
                    start: batch.start,
                    count: batch.count,
                },
                StaticsGroundDrawEntity,
            ));
        }
    }

    for (entity, batch) in existing_q.iter() {
        if !chunk_batches
            .ground
            .iter()
            .any(|desired| desired.key == batch.key)
        {
            commands.entity(entity).despawn();
        }
    }

    if debug_state.last_ground_entity_count != Some(desired_count) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "static ground draw entities: existing={} desired={}",
                existing_count, desired_count,
            ),
        );
        debug_state.last_ground_entity_count = Some(desired_count);
    }

    sync_state.ground = Some(signature);
}

pub fn sys_sync_static_ground_transparent_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut batch_meshes: ResMut<ArtBatchMeshes>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundTransparentDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.ground);
    if sync_state.ground_transparent == Some(signature) {
        return;
    }
    batch_meshes.drop_unused();

    let desired_count = chunk_batches.ground.len();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.ground {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            retarget_batch_entity(&mut commands, *entity, *current_batch, batch, &mut batch_meshes, &mut meshes);
        } else {
            commands.spawn((
                Mesh3d(batch_meshes.get(&mut meshes, batch.count)),
                MeshTag::new(batch.start),
                MeshMaterial3d(render_assets.transparent_material.clone()),
                Transform::IDENTITY,
                NoFrustumCulling,
                super::transparent_order::StaticTransparentBatchOrder { key: batch.key, kind: 0 },
                StaticChunkBatchEntity {
                    key: batch.key,
                    start: batch.start,
                    count: batch.count,
                },
                StaticsGroundTransparentDrawEntity,
            ));
        }
    }

    for (entity, batch) in existing_q.iter() {
        if !chunk_batches
            .ground
            .iter()
            .any(|desired| desired.key == batch.key)
        {
            commands.entity(entity).despawn();
        }
    }

    if debug_state.last_ground_transparent_entity_count != Some(desired_count) {
        debug_state.last_ground_transparent_entity_count = Some(desired_count);
    }

    sync_state.ground_transparent = Some(signature);
}

pub fn sys_update_sprite_instance_buffer(
    instances: Res<RenderStaticInstances>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    zoom: Res<crate::render::view::camera::RenderZoom>,
    scene_state: Res<crate::render::scene::SceneStateData>,
    world_geo: Res<crate::world::maps::WorldGeoData>,
    mut upload_cache: ResMut<StaticArtUploadCache>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
) {
    if instances.is_changed() && !upload_cache.sprite_pending {
        upload_cache.sprite_pending = true;
    }
    if instances.0.is_empty() {
        return;
    }

    let render_mode = if zoom.0 >= 20.0 { 1 } else { 0 };
    let (map_width_tiles, map_height_tiles) = world_geo
        .maps
        .get(&scene_state.map_id)
        .map(|meta| (meta.width as f32, meta.height as f32))
        .unwrap_or((1.0, 1.0));

    // Taken mutably only when a parameter differs: a mutable access re-extracts the material for the GPU.
    let stale = |material: &ArtSpriteMaterial| {
        let params = &material.extension.params;
        params.render_mode != render_mode
            || params.map_width_tiles != map_width_tiles
            || params.map_height_tiles != map_height_tiles
    };
    for handle in [
        &render_assets.opaque_material,
        &render_assets.transparent_material,
        &render_assets.shadow_material,
    ] {
        let Some(material) = materials.get(handle) else {
            return;
        };
        if stale(material) {
            let Some(mut material) = materials.get_mut(handle) else {
                return;
            };
            material.extension.params.render_mode = render_mode;
            material.extension.params.map_width_tiles = map_width_tiles;
            material.extension.params.map_height_tiles = map_height_tiles;
        }
    }

    let upload_changed = upload_cache.sprite_pending;
    if upload_changed {
        let Some(opaque_material) = materials.get(&render_assets.opaque_material) else {
            return;
        };
        let _ = storage_buffers.insert(
            &opaque_material.extension.instances,
            ShaderBuffer::from(instances.0.clone()),
        );
        upload_cache.sprite_pending = false;
    }

    if upload_changed || debug_state.last_uploaded_instances != Some(instances.0.len()) {
        console_logger::one(
            LogSev::Debug,
            LogAbout::RenderWorldArt,
            &format!(
                "static art upload: instances={} render_mode={} changed={}",
                instances.0.len(),
                render_mode,
                upload_changed,
            ),
        );
        debug_state.last_uploaded_instances = Some(instances.0.len());
    }
}

pub fn sys_update_ground_instance_buffer(
    instances: Res<RenderStaticLandInstances>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut materials: ResMut<Assets<ArtGroundMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    scene_state: Res<crate::render::scene::SceneStateData>,
    world_geo: Res<crate::world::maps::WorldGeoData>,
    mut upload_cache: ResMut<StaticArtUploadCache>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
) {
    if instances.is_changed() && !upload_cache.ground_pending {
        upload_cache.ground_pending = true;
    }
    if instances.0.is_empty() {
        return;
    }

    let (map_width_tiles, map_height_tiles) = world_geo
        .maps
        .get(&scene_state.map_id)
        .map(|meta| (meta.width as f32, meta.height as f32))
        .unwrap_or((1.0, 1.0));

    // Taken mutably only when a parameter differs: a mutable access re-extracts the material for the GPU.
    for handle in [&render_assets.opaque_material, &render_assets.transparent_material] {
        let Some(material) = materials.get(handle) else {
            return;
        };
        let params = &material.extension.params;
        if params.map_width_tiles != map_width_tiles || params.map_height_tiles != map_height_tiles {
            let Some(mut material) = materials.get_mut(handle) else {
                return;
            };
            material.extension.params.map_width_tiles = map_width_tiles;
            material.extension.params.map_height_tiles = map_height_tiles;
        }
    }

    let upload_changed = upload_cache.ground_pending;
    if upload_changed {
        let Some(opaque_material) = materials.get(&render_assets.opaque_material) else {
            return;
        };
        let _ = storage_buffers.insert(
            &opaque_material.extension.instances,
            ShaderBuffer::from(instances.0.clone()),
        );
        upload_cache.ground_pending = false;
    }

    if upload_changed || debug_state.last_uploaded_ground_instances != Some(instances.0.len()) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "static ground upload: instances={} changed={}",
                instances.0.len(),
                upload_changed,
            ),
        );
        debug_state.last_uploaded_ground_instances = Some(instances.0.len());
    }
}

pub fn sys_update_art_materials(
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    land_materials: Res<Assets<terrain::mesh_material::LandCustomMeshMaterial>>,
    sprite_assets: Option<Res<ArtSpriteRenderAssets>>,
    ground_assets: Option<Res<ArtGroundRenderAssets>>,
    shared_land_material: Option<Res<terrain::draw_mesh::SharedLandMaterial>>,
    render_zoom: Res<crate::render::view::camera::RenderZoom>,
    player_q: Query<&Transform, With<crate::world::player::Player>>,
    uniform_state: Res<crate::render::uniform_state::UniformState>,
    camera_q: Query<&Projection, With<crate::render::view::camera::PlayerCamera>>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
    mut last_uniform_snapshot: Local<Option<UniformSnapshot>>,
    mut last_client_view_scale: Local<f32>,
) {
    let current_global_lighting = uniform_state.global_lighting;
    let current_render_zoom = render_zoom.0;
    let current_uniform_snapshot = uniform_state.snapshot();
    let current_client_view_scale =
        camera_q.iter().next().map_or(1.0, terrain::mesh_material::client_view_scale);

    let lighting_meaningfully_changed =
        (current_global_lighting - *last_global_lighting).abs() > 0.01;
    let zoom_changed = (current_render_zoom - *last_render_zoom).abs() > 0.1;
    let client_view_scale_changed = (current_client_view_scale - *last_client_view_scale).abs()
        > terrain::mesh_material::CLIENT_VIEW_SCALE_EPSILON;
    let uniforms_changed = last_uniform_snapshot
        .is_none_or(|snapshot| art_uniforms_differ(&snapshot, &current_uniform_snapshot));
    let land_page_bindings =
        land_page_binding_handles(shared_land_material.as_deref(), &land_materials);

    if let (Some(assets), Some(bindings)) = (ground_assets.as_ref(), land_page_bindings.as_ref()) {
        for handle in [&assets.opaque_material, &assets.transparent_material] {
            let changed = ground_materials.get(handle).is_some_and(|mat|
                mat.extension.land_page_atlas != bindings.atlas || mat.extension.land_page_lookup != bindings.lookup);
            if changed {
                if let Some(mut mat) = ground_materials.get_mut(handle) {
                    mat.extension.land_page_atlas = bindings.atlas.clone();
                    mat.extension.land_page_lookup = bindings.lookup.clone();
                }
            }
        }
    }

    if let Some(assets) = sprite_assets.as_ref() {
        let pass_mode = shadow_pass_mode(&uniform_state);
        let stale = materials
            .get(&assets.shadow_material)
            .is_some_and(|mat| mat.extension.params.pass_mode != pass_mode);
        if stale {
            if let Some(mut mat) = materials.get_mut(&assets.shadow_material) {
                mat.extension.params.pass_mode = pass_mode;
            }
        }
    }

    if !uniforms_changed && !zoom_changed && !client_view_scale_changed {
        return;
    }

    let camera_pos = player_q
        .iter()
        .next()
        .map(|t| t.translation)
        .unwrap_or(Vec3::ZERO);

    if let Some(assets) = sprite_assets {
        if let Some(mut mat) = materials.get_mut(&assets.opaque_material) {
            update_art_material_uniforms(
                &mut mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
                current_client_view_scale,
            );
        }
        if let Some(mut mat) = materials.get_mut(&assets.transparent_material) {
            update_art_material_uniforms(
                &mut mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
                current_client_view_scale,
            );
        }
        if let Some(mut mat) = materials.get_mut(&assets.shadow_material) {
            update_art_material_uniforms(
                &mut mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
                current_client_view_scale,
            );
        }
    }

    if let Some(assets) = ground_assets {
        if let Some(mut mat) = ground_materials.get_mut(&assets.opaque_material) {
            update_art_material_uniforms_ground(
                &mut mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
                current_client_view_scale,
            );
        }
        if let Some(mut mat) = ground_materials.get_mut(&assets.transparent_material) {
            update_art_material_uniforms_ground(
                &mut mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
                current_client_view_scale,
            );
        }
    }

    if lighting_meaningfully_changed || uniforms_changed {
        *last_global_lighting = current_global_lighting;
    }
    if zoom_changed {
        *last_render_zoom = current_render_zoom;
    }
    if uniforms_changed {
        *last_uniform_snapshot = Some(current_uniform_snapshot);
    }
    *last_client_view_scale = current_client_view_scale;
}

/// Whether the art materials' uniforms read anything that differs between `a` and `b`. The land lighting counts: its
/// sun strength reaches the art (`update_art_material_uniforms`), for the flat water and the projected shadows' depth.
fn art_uniforms_differ(a: &UniformSnapshot, b: &UniformSnapshot) -> bool {
    a.effects != b.effects
        || a.lighting != b.lighting
        || a.land_lighting.diffuse_strength != b.land_lighting.diffuse_strength
        || a.sun != b.sun
        || (a.global_lighting - b.global_lighting).abs() > f32::EPSILON
}

fn update_art_material_uniforms(
    mat: &mut ArtSpriteMaterial,
    uniform_state: &UniformState,
    global_lighting: f32,
    render_zoom: f32,
    camera_pos: Vec3,
    client_view_scale: f32,
) {
    mat.extension.scene_uniform.global_lighting = global_lighting;
    mat.extension.scene_uniform.client_view_scale = client_view_scale;
    mat.extension.scene_uniform.render_zoom = render_zoom;
    mat.extension.scene_uniform.camera_position = camera_pos;
    mat.extension.scene_uniform.ec_reflection_camera_distance =
        crate::render::view::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER.length();
    mat.extension.scene_uniform.light_direction = uniform_state.sun.light_direction();
    mat.extension.effects_uniform = uniform_state.effects;
    mat.extension.global_lighting_uniform = uniform_state.lighting;
    // Flat-quad CC water shades itself with a flat ambient+diffuse response (see art
    // main.wesl); surface the land diffuse strength through the shared uniform's spare
    // `_pad2_.x` slot (WGSL: land_diffuse_strength) so it matches the terrain water.
    mat.extension.global_lighting_uniform._pad2_.x = uniform_state.land_lighting.diffuse_strength;
}

fn update_art_material_uniforms_ground(
    mat: &mut ArtGroundMaterial,
    uniform_state: &UniformState,
    global_lighting: f32,
    render_zoom: f32,
    camera_pos: Vec3,
    client_view_scale: f32,
) {
    mat.extension.scene_uniform.global_lighting = global_lighting;
    mat.extension.scene_uniform.client_view_scale = client_view_scale;
    mat.extension.scene_uniform.render_zoom = render_zoom;
    mat.extension.scene_uniform.camera_position = camera_pos;
    mat.extension.scene_uniform.ec_reflection_camera_distance =
        crate::render::view::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER.length();
    mat.extension.scene_uniform.light_direction = uniform_state.sun.light_direction();
    mat.extension.effects_uniform = uniform_state.effects;
    mat.extension.global_lighting_uniform = uniform_state.lighting;
    // See update_art_material_uniforms: keep the flat-quad water diffuse in sync.
    mat.extension.global_lighting_uniform._pad2_.x = uniform_state.land_lighting.diffuse_strength;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::storage::ShaderBufferData;
    use std::sync::Arc;

    /// A map size no map has: the system's first frame sets the scene's.
    const UNSET_MAP_TILES: f32 = 0.0;

    fn test_ground_material(instances: Handle<ShaderBuffer>) -> ArtGroundMaterial {
        ArtGroundMaterial {
            base: StandardMaterial::default(),
            extension: ArtGroundMaterialExtension {
                atlas: Handle::default(),
                atlas_enhanced: Handle::default(),
                instances,
                params: SpriteParams {
                    render_mode: 0,
                    alpha_cutoff: 0.5,
                    pass_mode: PASS_MODE_OPAQUE,
                    hue_enabled: 0,
                    map_width_tiles: UNSET_MAP_TILES,
                    map_height_tiles: UNSET_MAP_TILES,
                    _pad_sp: UVec2::ZERO,
                },
                scene_uniform: terrain::mesh_material::SceneUniform::default(),
                effects_uniform: terrain::mesh_material::LandEffectsUniform::default(),
                global_lighting_uniform: terrain::mesh_material::GlobalLightingUniforms::default(),
                hues: Handle::default(),
                land_page_atlas: Handle::default(),
                land_page_lookup: Handle::default(),
                visual_grunge_texture: Handle::default(),
                item_liquid_atlas: Handle::default(),
                item_liquid_lookup: Handle::default(),
                client_grunge_texture: Handle::default(),
            },
        }
    }

    #[derive(Resource, Default)]
    struct GroundMaterialsTouched(bool);

    fn record_ground_materials_touched(
        materials: Res<Assets<ArtGroundMaterial>>,
        mut touched: ResMut<GroundMaterialsTouched>,
    ) {
        touched.0 = materials.is_changed();
    }

    /// The ground instances are uploaded when the collector changed them, not on every frame, and the materials are
    /// taken mutably, which re-extracts them for the GPU, only when a parameter differs.
    #[test]
    fn ground_instances_upload_only_when_the_collector_changed_them() {
        let mut app = App::new();
        let mut buffers = Assets::<ShaderBuffer>::default();
        let buffer = buffers.add(ShaderBuffer::from(Vec::<GroundTileInstance>::new()));
        let mut materials = Assets::<ArtGroundMaterial>::default();
        let render_assets = ArtGroundRenderAssets {
            mesh: Handle::default(),
            opaque_material: materials.add(test_ground_material(buffer.clone())),
            transparent_material: materials.add(test_ground_material(buffer.clone())),
        };
        app.insert_resource(buffers)
            .insert_resource(materials)
            .insert_resource(render_assets)
            .insert_resource(RenderStaticLandInstances(vec![bytemuck::Zeroable::zeroed()]))
            .insert_resource(crate::render::scene::SceneStateData { map_id: 0 })
            .init_resource::<crate::world::maps::WorldGeoData>()
            .init_resource::<StaticArtUploadCache>()
            .init_resource::<StaticArtDrawDebugState>()
            .init_resource::<GroundMaterialsTouched>()
            .add_systems(Update, (sys_update_ground_instance_buffer, record_ground_materials_touched).chain());
        let materials_touched = |app: &App| app.world().resource::<GroundMaterialsTouched>().0;
        // An upload replaces the buffer with initialized data.
        let uploaded = |app: &App| {
            let buffers = app.world().resource::<Assets<ShaderBuffer>>();
            matches!(buffers.get(&buffer).map(|b| &b.data), Some(ShaderBufferData::Initialized { .. }))
        };
        let replace_with_marker = |app: &mut App| {
            let marker = ShaderBuffer::with_size(1, bevy::asset::RenderAssetUsages::default());
            app.world_mut().resource_mut::<Assets<ShaderBuffer>>().insert(&buffer, marker).unwrap();
        };
        app.update();
        assert!(uploaded(&app), "the first frame uploads the instances");
        assert!(materials_touched(&app), "the map size is set");

        replace_with_marker(&mut app);
        app.update();
        assert!(!uploaded(&app), "an unchanged frame uploaded the instances again");
        assert!(!materials_touched(&app), "unchanged materials taken mutably");

        app.world_mut().resource_mut::<RenderStaticLandInstances>().set_changed();
        app.update();
        assert!(uploaded(&app), "changed instances were not uploaded");
    }

    #[test]
    fn kingdom_reborn_pages_widen_the_enhanced_atlas() {
        use PagePixelFormat::{Bc7, Rgba8888};
        const CC_PAGES: usize = 5;
        const EC_PAGES: usize = 10;
        const KR_PAGES: usize = 3;
        const CC_DIM: u32 = 2048;
        const EC_DIM: u32 = 4096;
        let spec = AtlasSetSpec::default()
            .with_pages(CLASSIC_ATLAS_SLOT, CC_PAGES, CC_DIM, CC_DIM, Some(Rgba8888))
            .with_pages(ENHANCED_ATLAS_SLOT, EC_PAGES, EC_DIM, CC_DIM, Some(Bc7));
        let [classic, enhanced] = spec.with_pages(ENHANCED_ATLAS_SLOT, KR_PAGES, CC_DIM, EC_DIM, Some(Bc7)).allocations();
        // The Classic pages keep their size and format; the Enhanced atlas is as large as its largest pages.
        assert_eq!(classic, AtlasAllocationSpec { page_width: CC_DIM, page_height: CC_DIM,
            initial_layers: ART_ATLAS_INITIAL_LAYERS[CLASSIC_ATLAS_SLOT], max_layers: CC_PAGES as u32,
            pixel_format: Rgba8888 });
        assert_eq!(enhanced, AtlasAllocationSpec { page_width: EC_DIM, page_height: EC_DIM,
            initial_layers: ART_ATLAS_INITIAL_LAYERS[ENHANCED_ATLAS_SLOT], max_layers: (EC_PAGES + KR_PAGES) as u32,
            pixel_format: Bc7 });
        // The missing texture's page takes the format of its atlas, a BC7 one included.
        let [classic, _] = AtlasSetSpec::default().with_pages(CLASSIC_ATLAS_SLOT, CC_PAGES, CC_DIM, CC_DIM, Some(Bc7))
            .with_pages(CLASSIC_ATLAS_SLOT, MISSING_TEXTURE_PAGES, MISSING_TEXTURE_PAGE_SIZE, MISSING_TEXTURE_PAGE_SIZE,
                None)
            .allocations();
        assert_eq!((classic.pixel_format, classic.max_layers), (Bc7, CC_PAGES as u32 + 1));
        // An atlas no package fills is a blank one; one with fewer pages than its start has just them.
        let [classic, enhanced] = AtlasSetSpec::default().with_pages(CLASSIC_ATLAS_SLOT, 1, CC_DIM, CC_DIM, Some(Rgba8888))
            .allocations();
        assert_eq!((classic.initial_layers, classic.max_layers), (1, 1));
        assert_eq!(enhanced, blank_atlas_spec());
    }

    #[test]
    fn classic_art_takes_the_enhanced_atlases_under_kingdom_reborn_land() {
        for style in [StaticLandStyle::Enhanced, StaticLandStyle::KingdomReborn] {
            assert_eq!(art_atlas_source(Some(ClientTextureSource::Cc), style, true), Some(ClientTextureSource::Ec));
        }
        assert_eq!(art_atlas_source(Some(ClientTextureSource::Cc), StaticLandStyle::Classic, true),
            Some(ClientTextureSource::Cc));
    }
    use udd_container::{
        AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder, UddpReader,
    };

    fn build_hues_package(texture: &[u8]) -> udd_assets::HuesPackage {
        let records = vec![udd_assets::hues::HueSlotRecord {
            hue_id: 1,
            name: "test".to_string(),
            table_start: 0,
            table_end: 31,
            texture_column: 0,
            texture_row: 1,
            palette_width_pixels: udd_assets::hues::HUE_STRIP_WIDTH,
            flags: udd_assets::hues::HUE_FLAG_PRESENT,
        }];
        let csv = udd_assets::hues::encode_hues_csv(&records).expect("encode hues csv");

        let header = udd_assets::hues::encode_hues_package_header();
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(udd_assets::hues::HUES_PACKAGE_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &header,
            })
            .expect("add hues header");
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(udd_assets::hues::HUES_METADATA_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &csv,
            })
            .expect("add hues metadata");
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Texture as u8,
                compression: CompressionFlag::None,
                width: udd_assets::hues::HUES_TEXTURE_WIDTH,
                height: udd_assets::hues::HUES_TEXTURE_HEIGHT,
                virtual_path: Some(udd_assets::hues::HUES_TEXTURE_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: texture,
            })
            .expect("add hues texture");

        udd_assets::HuesPackage::from_uddp_package(
            UddpReader::open(builder.build().expect("build hues package"))
                .expect("open hues package"),
        )
        .expect("load hues package")
    }

    #[test]
    fn hue_lookup_image_uses_hues_uddp_texture_bytes() {
        let mut images = Assets::<Image>::default();
        let texture_len = udd_assets::hues::HUES_TEXTURE_WIDTH as usize
            * udd_assets::hues::HUES_TEXTURE_HEIGHT as usize
            * 4;
        let texture = vec![17u8; texture_len];
        let package = HuesPackageRes(Arc::new(build_hues_package(&texture)));

        let (handle, hue_enabled) = create_hue_lookup_image(&mut images, Some(&package));
        let image = images.get(&handle).expect("hue image");

        assert_eq!(hue_enabled, 1);
        assert_eq!(image.texture_descriptor.size.width, udd_assets::hues::HUES_TEXTURE_WIDTH);
        assert_eq!(image.texture_descriptor.size.height, udd_assets::hues::HUES_TEXTURE_HEIGHT);
        assert_eq!(image.data.as_ref().expect("image data"), &texture);
    }

    #[test]
    fn hue_lookup_image_falls_back_when_hues_uddp_is_missing() {
        let mut images = Assets::<Image>::default();

        let (handle, hue_enabled) = create_hue_lookup_image(&mut images, None);
        let image = images.get(&handle).expect("hue image");

        assert_eq!(hue_enabled, 0);
        assert_eq!(image.texture_descriptor.size.width, udd_assets::hues::HUES_TEXTURE_WIDTH);
        assert_eq!(image.texture_descriptor.size.height, udd_assets::hues::HUES_TEXTURE_HEIGHT);
    }

    #[test]
    fn the_shadow_material_draws_the_client_shadow_only_where_it_is_on() {
        let mut u = UniformState {
            effects: terrain::mesh_material::LandEffectsUniform::default(),
            lighting: terrain::mesh_material::GlobalLightingUniforms::default(),
            land_lighting: terrain::mesh_material::LandLightingUniforms::default(),
            global_lighting: 1.0,
            client_lighting: None,
            client_shadows: None,
            local_lights: None,
            land_texture_variation: None,
            light_cover: crate::render::lighting::light_cover::LightCover::ClassicUo,
            player_floor_rule: None,
            mobile_sharpening: terrain::mesh_material::MOBILE_SHARPENING_OFF,
            sun: terrain::mesh_material::SunAngles::default(),
            dirty: false,
            saved_snapshot: None,
            active_preset: String::new(),
        };
        assert_eq!(shadow_pass_mode(&u), PASS_MODE_SHADOW);
        u.client_shadows = Some(false);
        assert_eq!(shadow_pass_mode(&u), PASS_MODE_SHADOW);
        u.client_shadows = Some(true);
        assert_eq!(shadow_pass_mode(&u), PASS_MODE_CLIENT_SHADOW);
    }

    #[test]
    fn a_new_sun_strength_reaches_the_art_materials() {
        let u = UniformState {
            effects: terrain::mesh_material::LandEffectsUniform::default(),
            lighting: terrain::mesh_material::GlobalLightingUniforms::default(),
            land_lighting: terrain::mesh_material::LandLightingUniforms::default(),
            global_lighting: 1.0,
            client_lighting: None,
            client_shadows: None,
            local_lights: None,
            land_texture_variation: None,
            mobile_sharpening: terrain::mesh_material::MOBILE_SHARPENING_OFF,
            light_cover: crate::render::lighting::light_cover::LightCover::ClassicUo,
            player_floor_rule: None,
            sun: terrain::mesh_material::SunAngles::default(),
            dirty: false,
            saved_snapshot: None,
            active_preset: String::new(),
        };
        let before = u.snapshot();
        assert!(!art_uniforms_differ(&before, &u.snapshot()));
        let mut stronger_sun = u.snapshot();
        stronger_sun.land_lighting.diffuse_strength += 0.5;
        assert!(art_uniforms_differ(&before, &stronger_sun), "the projected shadows and flat water read the sun strength");
    }

    #[test]
    fn the_client_shadow_multiplies_the_buffer_by_one_minus_the_texel_alpha() {
        // S' = src.a * Zero + S * (1 - src.a); the colour passes through untouched (and is masked off anyway).
        assert_eq!(CLIENT_SHADOW_BUFFER_BLEND.alpha.src_factor, BlendFactor::Zero);
        assert_eq!(CLIENT_SHADOW_BUFFER_BLEND.alpha.dst_factor, BlendFactor::OneMinusSrcAlpha);
        assert_eq!(CLIENT_SHADOW_BUFFER_BLEND.alpha.operation, BlendOperation::Add);
    }

    #[test]
    fn projected_shadow_material_sorts_behind_transparent_art() {
        let material = projected_art_shadow_base_material();

        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert!(material.unlit);
        assert!(material.depth_bias > 0.0);
    }

    #[test]
    fn static_batch_sync_signature_tracks_batch_identity_and_range() {
        let batch = StaticChunkBatch {
            key: StaticChunkBatchKey {
                map_id: 1,
                gx: 2,
                gy: 3,
                scale: 4,
            },
            start: 5,
            count: 6,
        };
        let base = static_batch_sync_signature(&[batch]);

        let mut moved = batch;
        moved.start += 1;
        assert_ne!(base, static_batch_sync_signature(&[moved]));

        let mut other_chunk = batch;
        other_chunk.key.gx += 1;
        assert_ne!(base, static_batch_sync_signature(&[other_chunk]));

        assert_eq!(base, static_batch_sync_signature(&[batch]));
    }
}
