use super::DrawStaticSpritesPlugin;
use crate::configs::settings::{ClientTextureSource, Settings};
use crate::configs::shader_presets::UniformState;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world;
use crate::core::render::scene::world::art::statics_collect::{
    GroundTileInstance, RenderStaticChunkBatches, RenderStaticInstances, RenderStaticLandInstances,
    SpriteInstance, StaticChunkBatchKey,
};
use crate::core::system_sets::StartupSysSet;
use crate::core::texture_cache::art::{
    ArtPageAtlas, GroundArtPageAtlas, GroundArtPageAtlasHandle, SpriteArtPageAtlas,
    SpriteArtPageAtlasHandle,
};
use crate::core::uo_files_loader::{
    HuesPackageRes, TexArtCcPackageRes, TexArtEcPackageRes, TexLandEcPackageRes,
    TileMetaPackageRes,
};
use crate::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType};
use bevy::render::storage::ShaderStorageBuffer;
use std::collections::HashMap;
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_conv::bc7::{ImageExtent, VramTextureFormat};

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

pub type ArtSpriteMaterial = ExtendedMaterial<StandardMaterial, ArtSpriteMaterialExtension>;
pub type ArtGroundMaterial = ExtendedMaterial<StandardMaterial, ArtGroundMaterialExtension>;

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ArtSpriteMaterialExtension {
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderStorageBuffer>,
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
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct ArtGroundMaterialExtension {
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub atlas: Handle<Image>,
    #[storage(102, read_only, visibility(vertex, fragment))]
    pub instances: Handle<ShaderStorageBuffer>,
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
}

#[derive(Resource, Clone)]
pub struct ArtSpriteRenderAssets {
    pub mesh: Handle<Mesh>,
    pub opaque_material: Handle<ArtSpriteMaterial>,
    pub transparent_material: Handle<ArtSpriteMaterial>,
}

#[derive(Resource, Clone)]
pub struct ArtGroundRenderAssets {
    pub mesh: Handle<Mesh>,
    pub opaque_material: Handle<ArtGroundMaterial>,
    pub transparent_material: Handle<ArtGroundMaterial>,
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

#[derive(Resource, Default)]
pub struct ActiveArtAtlasBindingState {
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
        "shaders/world/art/main.wgsl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/main.wgsl".into()
    }
}

impl MaterialExtension for ArtGroundMaterialExtension {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/ground.wgsl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/world/art/ground.wgsl".into()
    }
}

#[derive(Component)]
pub struct StaticsDrawEntity;

#[derive(Component)]
pub struct StaticsTransparentDrawEntity;

#[derive(Component)]
pub struct StaticsGroundDrawEntity;

#[derive(Component)]
pub struct StaticsGroundTransparentDrawEntity;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StaticChunkBatchEntity {
    pub key: StaticChunkBatchKey,
    pub start: u32,
    pub count: u32,
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
        Some(ClientTextureSource::Ec) => tex_art_ec_res
            .map(|package| AtlasAllocationSpec {
                page_width: package.0.atlas_width().max(1),
                page_height: package.0.atlas_height().max(1),
                initial_layers: (package.0.pages().len().max(1) as u32).min(4),
                max_layers: package.0.pages().len().max(1) as u32,
                pixel_format: package_pixel_format(package.0.pages(), |page| page.pixel_format),
            })
            .unwrap_or_else(blank_atlas_spec),
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
        PagePixelFormat::Rgba8888 => VramTextureFormat::Rgba8UnormSrgb.texture_format(),
        PagePixelFormat::Bc7 => VramTextureFormat::Bc7RgbaUnormSrgb.texture_format(),
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

    let texture_bytes = hues_package.and_then(|package| match package.0.read_texture_bytes() {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            bevy::log::warn!("Failed to read hues.uddp lookup texture: {error}");
            None
        }
    });
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
    active_source: Option<ClientTextureSource>,
    tex_art_cc_res: Option<&Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<&Res<TexArtEcPackageRes>>,
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

    match active_source {
        Some(ClientTextureSource::Cc) => {
            if let Some(package) = tex_art_cc_res {
                for (page_index, layer) in resident_pages {
                    if let Some(page) = package.0.pages().get(page_index as usize) {
                        atlas.queue_page_upload(
                            page_index,
                            layer,
                            atlas.page_width,
                            atlas.page_height,
                            page.used_width,
                            page.used_height,
                            page.pixel_format,
                            || package.0.read_page_bytes(page_index),
                        );
                    }
                }
            }
        }
        Some(ClientTextureSource::Ec) => {
            if let Some(package) = tex_art_ec_res {
                for (page_index, layer) in resident_pages {
                    if let Some(page) = package.0.pages().get(page_index as usize) {
                        atlas.queue_page_upload(
                            page_index,
                            layer,
                            atlas.page_width,
                            atlas.page_height,
                            page.used_width,
                            page.used_height,
                            page.pixel_format,
                            || package.0.read_page_bytes(page_index),
                        );
                    }
                }
            }
        }
        None => {}
    }

    *atlas_handle = new_handle.clone();

    if let Some(material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = new_handle.clone();
    }
    if let Some(material) = materials.get_mut(&render_assets.transparent_material) {
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

    if let Some(package) = tex_land_ec_res {
        for (page_index, layer) in resident_pages {
            if let Some(page) = package.0.pages().get(page_index as usize) {
                atlas.queue_page_upload(
                    page_index,
                    layer,
                    atlas.page_width,
                    atlas.page_height,
                    page.used_width,
                    page.used_height,
                    page.pixel_format,
                    || package.0.read_page_bytes(page_index),
                );
            }
        }
    }

    *atlas_handle = new_handle.clone();

    if let Some(material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = new_handle.clone();
    }
    if let Some(material) = materials.get_mut(&render_assets.transparent_material) {
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

    if let Some(material) = materials.get_mut(&render_assets.opaque_material) {
        material.extension.atlas = next_sprite_handle.clone();
    }
    if let Some(material) = materials.get_mut(&render_assets.transparent_material) {
        material.extension.atlas = next_sprite_handle;
    }

    let next_ground_atlas = build_art_atlas(images, ground_spec);
    let next_ground_handle = next_ground_atlas.gpu_handle.clone();
    ground_atlas.0 = next_ground_atlas;
    ground_atlas_handle.0 = next_ground_handle.clone();

    if let Some(material) = ground_materials.get_mut(&ground_render_assets.opaque_material) {
        material.extension.atlas = next_ground_handle.clone();
    }
    if let Some(material) = ground_materials.get_mut(&ground_render_assets.transparent_material) {
        material.extension.atlas = next_ground_handle;
    }
}

pub fn sys_setup_art_page_atlas(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
    settings: Res<Settings>,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    hues_package_res: Option<Res<HuesPackageRes>>,
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

    let initial_buffer = ShaderStorageBuffer::from(vec![SpriteInstance {
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
        _pad_inst: 0,
        _pad_hue: [0; 2],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);

    let buffer_handle = storage_buffers.add(initial_buffer);
    let opaque_sprite_atlas_handle = sprite_atlas_handle.clone();
    let transparent_sprite_atlas_handle = sprite_atlas_handle.clone();

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
        },
    });

    let initial_ground_buffer = ShaderStorageBuffer::from(vec![GroundTileInstance {
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
        _pad_hue: [0; 2],
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
) {
    if binding_state.configured_source == source_state.active_source {
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
        source_state.active_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
        tex_land_ec_res.as_ref(),
    );

    binding_state.configured_source = source_state.active_source;
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
    source_state: Res<
        crate::core::render::scene::world::art::statics_collect::StaticArtSourceState,
    >,
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    tex_art_ec_res: Option<Res<TexArtEcPackageRes>>,
    tex_land_ec_res: Option<Res<TexLandEcPackageRes>>,
) {
    apply_sprite_art_atlas_resize(
        &mut images,
        &mut materials,
        &render_assets,
        &mut sprite_atlas,
        &mut sprite_atlas_handle.0,
        source_state.active_source,
        tex_art_cc_res.as_ref(),
        tex_art_ec_res.as_ref(),
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
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsDrawEntity>>,
) {
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
            let _ = commands.entity(entity).despawn();
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
}

pub fn sys_sync_static_sprite_transparent_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsTransparentDrawEntity>>,
) {
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
            let _ = commands.entity(entity).despawn();
        }
    }

    if debug_state.last_transparent_entity_count != Some(desired_count) {
        debug_state.last_transparent_entity_count = Some(desired_count);
    }
}

pub fn sys_sync_static_ground_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundDrawEntity>>,
) {
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
            let _ = commands.entity(entity).despawn();
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
}

pub fn sys_sync_static_ground_transparent_entities(
    mut commands: Commands,
    chunk_batches: Res<RenderStaticChunkBatches>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &StaticChunkBatchEntity), With<StaticsGroundTransparentDrawEntity>>,
) {
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
            let _ = commands.entity(entity).despawn();
        }
    }

    if debug_state.last_ground_transparent_entity_count != Some(desired_count) {
        debug_state.last_ground_transparent_entity_count = Some(desired_count);
    }
}

pub fn sys_update_sprite_instance_buffer(
    instances: Res<RenderStaticInstances>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
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
        let Some(material) = materials.get_mut(&render_assets.opaque_material) else {
            return;
        };
        material.extension.params.render_mode = render_mode;
        material.extension.params.map_width_tiles = map_width_tiles;
        material.extension.params.map_height_tiles = map_height_tiles;
        material.extension.instances.clone()
    };

    {
        let Some(transparent_material) = materials.get_mut(&render_assets.transparent_material)
        else {
            return;
        };
        transparent_material.extension.params.render_mode = render_mode;
        transparent_material.extension.params.map_width_tiles = map_width_tiles;
        transparent_material.extension.params.map_height_tiles = map_height_tiles;
    }

    let upload_changed = upload_cache.sprite_instances != instances.0;
    if upload_changed {
        let _ = storage_buffers.insert(
            &opaque_buffer_handle,
            ShaderStorageBuffer::from(instances.0.clone()),
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
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
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
        let Some(material) = materials.get_mut(&render_assets.opaque_material) else {
            return;
        };
        material.extension.params.map_width_tiles = map_width_tiles;
        material.extension.params.map_height_tiles = map_height_tiles;
        material.extension.instances.clone()
    };

    {
        let Some(transparent_material) = materials.get_mut(&render_assets.transparent_material)
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
            ShaderStorageBuffer::from(instances.0.clone()),
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
    sprite_assets: Option<Res<ArtSpriteRenderAssets>>,
    ground_assets: Option<Res<ArtGroundRenderAssets>>,
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    player_q: Query<&Transform, With<crate::core::render::scene::player::Player>>,
    uniform_state: Res<crate::configs::shader_presets::UniformState>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
) {
    let current_global_lighting = uniform_state.global_lighting;
    let current_render_zoom = render_zoom.0;

    let lighting_meaningfully_changed =
        (current_global_lighting - *last_global_lighting).abs() > 0.01;
    let zoom_changed = (current_render_zoom - *last_render_zoom).abs() > 0.1;
    let uniforms_dirty = uniform_state.dirty;

    if !lighting_meaningfully_changed && !zoom_changed && !uniforms_dirty {
        return;
    }

    let camera_pos = player_q
        .iter()
        .next()
        .map(|t| t.translation)
        .unwrap_or(Vec3::ZERO);

    if let Some(assets) = sprite_assets {
        if let Some(mat) = materials.get_mut(&assets.opaque_material) {
            update_art_material_uniforms(
                mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
            );
        }
        if let Some(mat) = materials.get_mut(&assets.transparent_material) {
            update_art_material_uniforms(
                mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
            );
        }
    }

    if let Some(assets) = ground_assets {
        if let Some(mat) = ground_materials.get_mut(&assets.opaque_material) {
            update_art_material_uniforms_ground(
                mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
            );
        }
        if let Some(mat) = ground_materials.get_mut(&assets.transparent_material) {
            update_art_material_uniforms_ground(
                mat,
                &uniform_state,
                current_global_lighting,
                current_render_zoom,
                camera_pos,
            );
        }
    }

    if lighting_meaningfully_changed {
        *last_global_lighting = current_global_lighting;
    }
    if zoom_changed {
        *last_render_zoom = current_render_zoom;
    }
}

fn update_art_material_uniforms(
    mat: &mut ArtSpriteMaterial,
    uniform_state: &UniformState,
    global_lighting: f32,
    render_zoom: f32,
    camera_pos: Vec3,
) {
    mat.extension.scene_uniform.global_lighting = global_lighting;
    mat.extension.scene_uniform.render_zoom = render_zoom;
    mat.extension.scene_uniform.camera_position = camera_pos;
    mat.extension.effects_uniform = uniform_state.effects;
    mat.extension.global_lighting_uniform = uniform_state.lighting;
}

fn update_art_material_uniforms_ground(
    mat: &mut ArtGroundMaterial,
    uniform_state: &UniformState,
    global_lighting: f32,
    render_zoom: f32,
    camera_pos: Vec3,
) {
    mat.extension.scene_uniform.global_lighting = global_lighting;
    mat.extension.scene_uniform.render_zoom = render_zoom;
    mat.extension.scene_uniform.camera_position = camera_pos;
    mat.extension.effects_uniform = uniform_state.effects;
    mat.extension.global_lighting_uniform = uniform_state.lighting;
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

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
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
}
