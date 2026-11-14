use super::DrawStaticSpritesPlugin;
use crate::configs::settings::{ClientTextureSource, Settings};
use crate::configs::shader_presets::{UniformSnapshot, UniformState};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world;
use crate::core::render::scene::world::art::statics_collect::{
    GroundTileInstance, RenderStaticChunkBatches, RenderStaticInstances, RenderStaticLandInstances,
    SpriteInstance, StaticChunkBatch, StaticChunkBatchKey, StaticLandStyle,
};
use crate::core::system_sets::StartupSysSet;
use crate::core::texture_cache::art::{
    ArtPageAtlas, GroundArtPageAtlas, GroundArtPageAtlasHandle, SpriteArtPageAtlas,
    SpriteArtPageAtlasHandle,
};
use crate::core::uo_files_loader::{
    HuesPackageRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes,
    TileMetaPackageRes, TexAuxEcPackageRes,
};
use crate::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites, DepthBiasState,
    PrimitiveTopology, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use std::collections::HashMap;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_assets::bc7::{ImageExtent, VramTextureFormat};

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
/// darkens by `clamp(S + 1 - s)` (`world::effects::client_night`).
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

/// The pipeline variant of an art sprite material: the client shadow pass writes only the shadow buffer.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ArtSpriteMaterialKey {
    client_shadow: bool,
}

impl From<&ArtSpriteMaterialExtension> for ArtSpriteMaterialKey {
    fn from(material: &ArtSpriteMaterialExtension) -> Self {
        Self { client_shadow: material.params.pass_mode == PASS_MODE_CLIENT_SHADOW }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[bind_group_data(ArtSpriteMaterialKey)]
pub struct ArtSpriteMaterialExtension {
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderBuffer>,
    #[uniform(103, visibility(vertex, fragment))]
    pub params: SpriteParams,
    #[uniform(105, visibility(vertex, fragment))]
    pub scene_uniform: world::land::mesh_material::SceneUniform,
    #[uniform(106, visibility(vertex, fragment))]
    pub effects_uniform: world::land::mesh_material::LandEffectsUniform,
    #[uniform(107, visibility(vertex, fragment))]
    pub global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms,
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
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ArtGroundMaterialExtension {
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderBuffer>,
    #[uniform(103, visibility(vertex, fragment))]
    pub params: SpriteParams,
    #[uniform(105, visibility(vertex, fragment))]
    pub scene_uniform: world::land::mesh_material::SceneUniform,
    #[uniform(106, visibility(vertex, fragment))]
    pub effects_uniform: world::land::mesh_material::LandEffectsUniform,
    #[uniform(107, visibility(vertex, fragment))]
    pub global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms,
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

#[derive(Resource, Default)]
pub struct StaticArtUploadCache {
    pub sprite_instances: Vec<SpriteInstance>,
    pub ground_instances: Vec<GroundTileInstance>,
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
}

#[derive(Clone, Copy)]
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

fn build_art_batch_mesh(start: u32, count: u32) -> Mesh {
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
        let instance_index = start + local_index;
        let vertex_base = local_index * 4;
        positions.extend_from_slice(&[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ]);
        normals.extend_from_slice(&[[0.0, 1.0, 0.0]; 4]);
        uv0.extend_from_slice(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
        uv1.extend_from_slice(&[[instance_index as f32, 0.0]; 4]);
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
        Some(ClientTextureSource::Cc) if land_style == StaticLandStyle::Enhanced && enhanced_available => {
            Some(ClientTextureSource::Ec)
        }
        source => source,
    }
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

fn package_pixel_format<T>(
    pages: &[T],
    get_pixel_format: impl Fn(&T) -> PagePixelFormat,
) -> PagePixelFormat {
    pages
        .first()
        .map(get_pixel_format)
        .unwrap_or(PagePixelFormat::Rgba8888)
}

fn sprite_atlas_spec(
    active_source: Option<ClientTextureSource>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
) -> AtlasAllocationSpec {
    match active_source {
        Some(ClientTextureSource::Cc) => tex_art_cc_res
            .map(|package| AtlasAllocationSpec {
                page_width: package.0.atlas_width().max(1),
                page_height: package.0.atlas_height().max(1),
                initial_layers: (package.0.pages().len().max(1) as u32).min(4),
                max_layers: package.0.pages().len().max(1) as u32,
                pixel_format: package_pixel_format(package.0.pages(), |page| page.pixel_format),
            })
            .unwrap_or_else(blank_atlas_spec),
        Some(ClientTextureSource::Ec) => {
            let enhanced = tex_art_ec_res.map(|p| p.0.as_ref());
            let classic = tex_art_cc_res.map(|p| p.0.as_ref());
            let layers = enhanced.map_or(0, |p| p.pages().len()) + classic.map_or(0, |p| p.pages().len()) + 1;
            let format = enhanced.map(|p| package_pixel_format(p.pages(), |p| p.pixel_format))
                .or_else(|| classic.map(|p| package_pixel_format(p.pages(), |p| p.pixel_format)))
                .unwrap_or(PagePixelFormat::Rgba8888);
            // Keep compressed uploads when both sources share BC7. Mixed
            // storage formats need a common lossless runtime representation.
            let format = if classic.is_some_and(|p|
                package_pixel_format(p.pages(), |p| p.pixel_format) != format) {
                PagePixelFormat::Rgba8888
            } else { format };
            AtlasAllocationSpec {
                page_width: enhanced.map_or(0, |p| p.atlas_width()).max(classic.map_or(0, |p| p.atlas_width())).max(256),
                page_height: enhanced.map_or(0, |p| p.atlas_height()).max(classic.map_or(0, |p| p.atlas_height())).max(256),
                initial_layers: (layers as u32).min(4),
                max_layers: layers as u32,
                pixel_format: format,
            }
        }
        None => blank_atlas_spec(),
    }
}

fn ground_atlas_spec(
    active_source: Option<ClientTextureSource>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
) -> AtlasAllocationSpec {
    if active_source != Some(ClientTextureSource::Ec) {
        return blank_atlas_spec();
    }

    tex_land_ec_res
        .map(|package| AtlasAllocationSpec {
            page_width: package.0.atlas_width().max(1),
            page_height: package.0.atlas_height().max(1),
            initial_layers: (package.0.pages().len().max(1) as u32).min(4),
            max_layers: package.0.pages().len().max(1) as u32,
            pixel_format: package_pixel_format(package.0.pages(), |page| page.pixel_format),
        })
        .unwrap_or_else(blank_atlas_spec)
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
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureUsages};

    let extent = ImageExtent::new(page_width.max(1), page_height.max(1)).unwrap();
    let byte_len = match pixel_format {
        PagePixelFormat::Rgba8888 => VramTextureFormat::Rgba8UnormSrgb.expected_byte_len(extent),
        PagePixelFormat::Bc7 => VramTextureFormat::Bc7RgbaUnormSrgb.expected_byte_len(extent),
    } * layers as usize;
    let mut image = Image::new(
        Extent3d {
            width: page_width,
            height: page_height,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        vec![0; byte_len],
        atlas_texture_format(pixel_format),
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
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
    shared_land_material: Option<&world::land::draw_mesh::SharedLandMaterial>,
    land_materials: &Assets<world::land::mesh_material::LandCustomMeshMaterial>,
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

fn apply_sprite_art_atlas_resize(
    images: &mut Assets<Image>,
    materials: &mut Assets<ArtSpriteMaterial>,
    render_assets: &ArtSpriteRenderAssets,
    atlas: &mut ArtPageAtlas,
    atlas_handle: &mut Handle<Image>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_aux_ec_res: Option<&Res<TexAuxEcPackageRes>>,
) {
    let Some(new_layers) = atlas.take_resize_request() else {
        return;
    };
    if new_layers <= atlas.active_layers {
        return;
    }

    let resident_pages = atlas.resident_page_layers();
    let new_handle = create_art_atlas_image(
        images,
        atlas.page_width,
        atlas.page_height,
        new_layers,
        atlas.pixel_format,
    );
    atlas.apply_resize(new_handle.clone(), new_layers);

    for (key, layer) in resident_pages {
        atlas.requeue_page(key, layer, tex_art_cc_res.map(|p| p.0.as_ref()),
            tex_art_ec_res.map(|p| p.0.as_ref()), None, tex_aux_ec_res.map(|p| p.0.as_ref()));
    }

    *atlas_handle = new_handle.clone();

    if let Some(mut material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = new_handle.clone();
    }
    if let Some(mut material) = materials.get_mut(&render_assets.transparent_material) {
        material.extension.atlas = new_handle.clone();
    }
    if let Some(mut material) = materials.get_mut(&render_assets.shadow_material) {
        material.extension.atlas = new_handle;
    }
}

fn apply_ground_art_atlas_resize(
    images: &mut Assets<Image>,
    materials: &mut Assets<ArtGroundMaterial>,
    render_assets: &ArtGroundRenderAssets,
    atlas: &mut ArtPageAtlas,
    atlas_handle: &mut Handle<Image>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
) {
    let Some(new_layers) = atlas.take_resize_request() else {
        return;
    };
    if new_layers <= atlas.active_layers {
        return;
    }

    let resident_pages = atlas.resident_page_layers();
    let new_handle = create_art_atlas_image(
        images,
        atlas.page_width,
        atlas.page_height,
        new_layers,
        atlas.pixel_format,
    );
    atlas.apply_resize(new_handle.clone(), new_layers);

    for (key, layer) in resident_pages {
        atlas.requeue_page(key, layer, None, None, tex_land_ec_res.map(|p| p.0.as_ref()), None);
    }

    *atlas_handle = new_handle.clone();

    if let Some(mut material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = new_handle.clone();
    }
    if let Some(mut material) = materials.get_mut(&render_assets.transparent_material) {
        material.extension.atlas = new_handle;
    }
}

fn rebind_active_art_atlases(
    images: &mut Assets<Image>,
    materials: &mut Assets<ArtSpriteMaterial>,
    ground_materials: &mut Assets<ArtGroundMaterial>,
    render_assets: &ArtSpriteRenderAssets,
    ground_render_assets: &ArtGroundRenderAssets,
    sprite_atlas: &mut SpriteArtPageAtlas,
    sprite_atlas_handle: &mut SpriteArtPageAtlasHandle,
    ground_atlas: &mut GroundArtPageAtlas,
    ground_atlas_handle: &mut GroundArtPageAtlasHandle,
    active_source: Option<ClientTextureSource>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<&Res<TexLandEcPackageRes>>,
) {
    let sprite_spec = sprite_atlas_spec(active_source, tex_art_cc_res, tex_art_ec_res);
    let ground_spec = ground_atlas_spec(active_source, tex_land_ec_res);

    let next_sprite_atlas = build_art_atlas(images, sprite_spec);
    let next_sprite_handle = next_sprite_atlas.gpu_handle.clone();
    sprite_atlas.0 = next_sprite_atlas;
    sprite_atlas_handle.0 = next_sprite_handle.clone();

    if let Some(mut material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = next_sprite_handle.clone();
    }
    if let Some(mut material) = materials.get_mut(&render_assets.transparent_material) {
        material.extension.atlas = next_sprite_handle.clone();
    }
    if let Some(mut material) = materials.get_mut(&render_assets.shadow_material) {
        material.extension.atlas = next_sprite_handle;
    }

    let next_ground_atlas = build_art_atlas(images, ground_spec);
    let next_ground_handle = next_ground_atlas.gpu_handle.clone();
    ground_atlas.0 = next_ground_atlas;
    ground_atlas_handle.0 = next_ground_handle.clone();

    if let Some(mut material) = ground_materials.get_mut(&ground_render_assets.opaque_material) {
        material.extension.atlas = next_ground_handle.clone();
    }
    if let Some(mut material) = ground_materials.get_mut(&ground_render_assets.transparent_material) {
        material.extension.atlas = next_ground_handle;
    }
}

pub fn sys_setup_art_page_atlas(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    land_materials: Res<Assets<world::land::mesh_material::LandCustomMeshMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    settings: Res<Settings>,
    shared_land_material: Option<Res<world::land::draw_mesh::SharedLandMaterial>>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    hues_package_res: Option<Res<HuesPackageRes>>,
    // One tuple: a system takes at most 16 parameters.
    (item_liquid_images, item_surface_images): (
        Res<crate::core::texture_cache::ec_item_liquid::EcItemLiquidImages>,
        Res<crate::core::texture_cache::ec_item_surface::EcItemSurfaceImages>,
    ),
    client_grunge_packages: super::client_grunge::ClientGrungePackages,
) {
    log_system_add_startup::<DrawStaticSpritesPlugin>(StartupSysSet::SetupSceneStage1, fname!());
    let active_source = resolve_effective_art_source(
        settings.graphics.art_texture_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    let sprite_spec = sprite_atlas_spec(
        active_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
    );
    let ground_spec = ground_atlas_spec(active_source, tex_land_ec_res.as_ref());

    let sprite_atlas = build_art_atlas(&mut images, sprite_spec);
    let sprite_atlas_handle = sprite_atlas.gpu_handle.clone();
    let ground_atlas = build_art_atlas(&mut images, ground_spec);
    let ground_atlas_handle = ground_atlas.gpu_handle.clone();
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
            scene_uniform: world::land::mesh_material::SceneUniform::default(),
            effects_uniform: world::land::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
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
            scene_uniform: world::land::mesh_material::SceneUniform::default(),
            effects_uniform: world::land::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
        },
    });

    let shadow_material_handle = materials.add(ArtSpriteMaterial {
        base: projected_art_shadow_base_material(),
        extension: ArtSpriteMaterialExtension {
            atlas: sprite_atlas_handle.clone(),
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
            scene_uniform: world::land::mesh_material::SceneUniform::default(),
            effects_uniform: world::land::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms::default(),
            hues: hue_lookup_handle.clone(),
            visual_grunge_texture: visual_grunge_texture.clone(),
            item_surface_atlas: item_surface_images.atlas.clone(),
            item_surface_lookup: item_surface_images.lookup.clone(),
            client_grunge_texture: client_grunge_texture.clone(),
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
            scene_uniform: world::land::mesh_material::SceneUniform::default(),
            effects_uniform: world::land::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms::default(),
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
            scene_uniform: world::land::mesh_material::SceneUniform::default(),
            effects_uniform: world::land::mesh_material::LandEffectsUniform::default(),
            global_lighting_uniform: world::land::mesh_material::GlobalLightingUniforms::default(),
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
    commands.insert_resource(SpriteArtPageAtlas(sprite_atlas));
    commands.insert_resource(SpriteArtPageAtlasHandle(sprite_atlas_handle.clone()));
    commands.insert_resource(GroundArtPageAtlas(ground_atlas));
    commands.insert_resource(GroundArtPageAtlasHandle(ground_atlas_handle.clone()));
    commands.insert_resource(ActiveArtAtlasBindingState {
        configured_source: active_source,
    });
}

pub fn sys_sync_active_art_page_atlases(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    render_assets: Res<ArtSpriteRenderAssets>,
    ground_render_assets: Res<ArtGroundRenderAssets>,
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut sprite_atlas_handle: ResMut<SpriteArtPageAtlasHandle>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    mut ground_atlas_handle: ResMut<GroundArtPageAtlasHandle>,
    source_state: Res<world::art::statics_collect::StaticArtSourceState>,
    mut binding_state: ResMut<ActiveArtAtlasBindingState>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
) {
    let atlas_source = art_atlas_source(source_state.active_source, source_state.land_style,
        source_available(ClientTextureSource::Ec, tex_art_cc_res.as_ref(), tex_art_ec_res.as_ref(),
            tex_land_ec_res.as_ref(), tilemeta_res.as_ref()));
    if binding_state.configured_source == atlas_source {
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
        &mut sprite_atlas_handle,
        &mut ground_atlas,
        &mut ground_atlas_handle,
        atlas_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
    );

    binding_state.configured_source = atlas_source;
}

pub fn sys_apply_pending_art_page_atlas_resizes(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    render_assets: Res<ArtSpriteRenderAssets>,
    ground_render_assets: Res<ArtGroundRenderAssets>,
    mut sprite_atlas: ResMut<SpriteArtPageAtlas>,
    mut sprite_atlas_handle: ResMut<SpriteArtPageAtlasHandle>,
    mut ground_atlas: ResMut<GroundArtPageAtlas>,
    mut ground_atlas_handle: ResMut<GroundArtPageAtlasHandle>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_aux_ec_res: Option<Res<TexAuxEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
) {
    apply_sprite_art_atlas_resize(
        &mut images,
        &mut materials,
        &render_assets,
        &mut sprite_atlas,
        &mut sprite_atlas_handle.0,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_aux_ec_res.as_ref(),
    );
    apply_ground_art_atlas_resize(
        &mut images,
        &mut ground_materials,
        &ground_render_assets,
        &mut ground_atlas,
        &mut ground_atlas_handle.0,
        tex_land_ec_res.as_ref(),
    );
}

pub fn sys_sync_static_sprite_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite == Some(signature) {
        return;
    }

    let desired_count = chunk_batches.sprite.len();
    let existing_count = existing_q.iter().count();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            if current_batch.start != batch.start || current_batch.count != batch.count {
                let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
                let _ = commands.entity(*entity).insert((
                    Mesh3d(mesh_handle),
                    StaticChunkBatchEntity {
                        key: batch.key,
                        start: batch.start,
                        count: batch.count,
                    },
                ));
            }
        } else {
            let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
            commands.spawn((
                Mesh3d(mesh_handle),
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
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsShadowDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite_shadow == Some(signature) {
        return;
    }

    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            if current_batch.start != batch.start || current_batch.count != batch.count {
                let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
                let _ = commands.entity(*entity).insert((
                    Mesh3d(mesh_handle),
                    StaticChunkBatchEntity {
                        key: batch.key,
                        start: batch.start,
                        count: batch.count,
                    },
                ));
            }
        } else {
            let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
            commands.spawn((
                Mesh3d(mesh_handle),
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
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsTransparentDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.sprite);
    if sync_state.sprite_transparent == Some(signature) {
        return;
    }

    let desired_count = chunk_batches.sprite.len();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.sprite {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            if current_batch.start != batch.start || current_batch.count != batch.count {
                let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
                let _ = commands.entity(*entity).insert((
                    Mesh3d(mesh_handle),
                    StaticChunkBatchEntity {
                        key: batch.key,
                        start: batch.start,
                        count: batch.count,
                    },
                ));
            }
        } else {
            let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
            commands.spawn((
                Mesh3d(mesh_handle),
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
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.ground);
    if sync_state.ground == Some(signature) {
        return;
    }

    let desired_count = chunk_batches.ground.len();
    let existing_count = existing_q.iter().count();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.ground {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            if current_batch.start != batch.start || current_batch.count != batch.count {
                let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
                let _ = commands.entity(*entity).insert((
                    Mesh3d(mesh_handle),
                    StaticChunkBatchEntity {
                        key: batch.key,
                        start: batch.start,
                        count: batch.count,
                    },
                ));
            }
        } else {
            let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
            commands.spawn((
                Mesh3d(mesh_handle),
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
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    mut sync_state: ResMut<StaticBatchEntitySyncState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundTransparentDrawEntity>>,
) {
    let signature = static_batch_sync_signature(&chunk_batches.ground);
    if sync_state.ground_transparent == Some(signature) {
        return;
    }

    let desired_count = chunk_batches.ground.len();
    let existing_by_key: HashMap<_, _> = existing_q
        .iter()
        .map(|(entity, batch)| (batch.key, (entity, *batch)))
        .collect();

    for batch in &chunk_batches.ground {
        if let Some((entity, current_batch)) = existing_by_key.get(&batch.key) {
            if current_batch.start != batch.start || current_batch.count != batch.count {
                let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
                let _ = commands.entity(*entity).insert((
                    Mesh3d(mesh_handle),
                    StaticChunkBatchEntity {
                        key: batch.key,
                        start: batch.start,
                        count: batch.count,
                    },
                ));
            }
        } else {
            let mesh_handle = meshes.add(build_art_batch_mesh(batch.start, batch.count));
            commands.spawn((
                Mesh3d(mesh_handle),
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
    zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    scene_state: Res<crate::core::render::scene::SceneStateData>,
    world_geo: Res<crate::core::render::scene::world::WorldGeoData>,
    mut upload_cache: ResMut<StaticArtUploadCache>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
) {
    if instances.0.is_empty() {
        return;
    }

    let render_mode = if zoom.0 >= 20.0 { 1 } else { 0 };
    let (map_width_tiles, map_height_tiles) = world_geo
        .maps
        .get(&scene_state.map_id)
        .map(|meta| (meta.width as f32, meta.height as f32))
        .unwrap_or((1.0, 1.0));

    let opaque_buffer_handle = {
        let Some(mut material) = materials.get_mut(&render_assets.opaque_material) else {
            return;
        };
        material.extension.params.render_mode = render_mode;
        material.extension.params.map_width_tiles = map_width_tiles;
        material.extension.params.map_height_tiles = map_height_tiles;
        material.extension.instances.clone()
    };

    {
        let Some(mut transparent_material) = materials.get_mut(&render_assets.transparent_material)
        else {
            return;
        };
        transparent_material.extension.params.render_mode = render_mode;
        transparent_material.extension.params.map_width_tiles = map_width_tiles;
        transparent_material.extension.params.map_height_tiles = map_height_tiles;
    }
    {
        let Some(mut shadow_material) = materials.get_mut(&render_assets.shadow_material) else {
            return;
        };
        shadow_material.extension.params.render_mode = render_mode;
        shadow_material.extension.params.map_width_tiles = map_width_tiles;
        shadow_material.extension.params.map_height_tiles = map_height_tiles;
    }

    let upload_changed = upload_cache.sprite_instances != instances.0;
    if upload_changed {
        let _ = storage_buffers.insert(
            &opaque_buffer_handle,
            ShaderBuffer::from(instances.0.clone()),
        );
        upload_cache.sprite_instances.clone_from(&instances.0);
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
    scene_state: Res<crate::core::render::scene::SceneStateData>,
    world_geo: Res<crate::core::render::scene::world::WorldGeoData>,
    mut upload_cache: ResMut<StaticArtUploadCache>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
) {
    if instances.0.is_empty() {
        return;
    }

    let (map_width_tiles, map_height_tiles) = world_geo
        .maps
        .get(&scene_state.map_id)
        .map(|meta| (meta.width as f32, meta.height as f32))
        .unwrap_or((1.0, 1.0));

    let opaque_buffer_handle = {
        let Some(mut material) = materials.get_mut(&render_assets.opaque_material) else {
            return;
        };
        material.extension.params.map_width_tiles = map_width_tiles;
        material.extension.params.map_height_tiles = map_height_tiles;
        material.extension.instances.clone()
    };

    {
        let Some(mut transparent_material) = materials.get_mut(&render_assets.transparent_material)
        else {
            return;
        };
        transparent_material.extension.params.map_width_tiles = map_width_tiles;
        transparent_material.extension.params.map_height_tiles = map_height_tiles;
    }

    let upload_changed = upload_cache.ground_instances != instances.0;
    if upload_changed {
        let _ = storage_buffers.insert(
            &opaque_buffer_handle,
            ShaderBuffer::from(instances.0.clone()),
        );
        upload_cache.ground_instances.clone_from(&instances.0);
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
    land_materials: Res<Assets<world::land::mesh_material::LandCustomMeshMaterial>>,
    sprite_assets: Option<Res<ArtSpriteRenderAssets>>,
    ground_assets: Option<Res<ArtGroundRenderAssets>>,
    shared_land_material: Option<Res<world::land::draw_mesh::SharedLandMaterial>>,
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    player_q: Query<&Transform, With<crate::core::render::scene::player::Player>>,
    uniform_state: Res<crate::configs::shader_presets::UniformState>,
    camera_q: Query<&Projection, With<crate::core::render::scene::camera::PlayerCamera>>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
    mut last_uniform_snapshot: Local<Option<UniformSnapshot>>,
    mut last_client_view_scale: Local<f32>,
) {
    let current_global_lighting = uniform_state.global_lighting;
    let current_render_zoom = render_zoom.0;
    let current_uniform_snapshot = uniform_state.snapshot();
    let current_client_view_scale =
        camera_q.iter().next().map_or(1.0, world::land::mesh_material::client_view_scale);

    let lighting_meaningfully_changed =
        (current_global_lighting - *last_global_lighting).abs() > 0.01;
    let zoom_changed = (current_render_zoom - *last_render_zoom).abs() > 0.1;
    let client_view_scale_changed = (current_client_view_scale - *last_client_view_scale).abs()
        > world::land::mesh_material::CLIENT_VIEW_SCALE_EPSILON;
    let uniforms_changed = last_uniform_snapshot.is_none_or(|snapshot| {
        snapshot.effects != current_uniform_snapshot.effects
            || snapshot.lighting != current_uniform_snapshot.lighting
            || snapshot.sun != current_uniform_snapshot.sun
            || (snapshot.global_lighting - current_uniform_snapshot.global_lighting).abs()
                > f32::EPSILON
    });
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
        crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER.length();
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
        crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER.length();
    mat.extension.scene_uniform.light_direction = uniform_state.sun.light_direction();
    mat.extension.effects_uniform = uniform_state.effects;
    mat.extension.global_lighting_uniform = uniform_state.lighting;
    // See update_art_material_uniforms: keep the flat-quad water diffuse in sync.
    mat.extension.global_lighting_uniform._pad2_.x = uniform_state.land_lighting.diffuse_strength;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
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
            effects: world::land::mesh_material::LandEffectsUniform::default(),
            lighting: world::land::mesh_material::GlobalLightingUniforms::default(),
            land_lighting: world::land::mesh_material::LandLightingUniforms::default(),
            global_lighting: 1.0,
            client_lighting: None,
            client_shadows: None,
            local_lights: None,
            land_texture_variation: None,
            light_cover: world::land::mesh_material::LightCover::ClassicUo,
            player_floor_rule: None,
            sun: world::land::mesh_material::SunAngles::default(),
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
