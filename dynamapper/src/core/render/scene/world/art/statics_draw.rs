use crate::core::render::scene::world::art::statics_collect::{
    GroundTileInstance, RenderStaticChunkBatches, RenderStaticInstances,
    RenderStaticLandInstances, SpriteInstance, StaticChunkBatchKey,
};
use crate::configs::settings::{ClientTextureSource, Settings};
use crate::core::texture_cache::art::{
    ArtPageAtlas, GroundArtPageAtlas, GroundArtPageAtlasHandle, SpriteArtPageAtlas,
    SpriteArtPageAtlasHandle,
};
use crate::core::uo_files_loader::{
    CcArtPackageRes, EcArtPackageRes, EcLandPackageRes, TileMetaPackageRes,
};
use crate::console_logger::{self, LogAbout, LogSev};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::render::storage::ShaderStorageBuffer;
use std::collections::HashMap;
use uddconv::bc7::{ImageExtent, VramTextureFormat};
use uddconv::cc_art::PagePixelFormat;

#[derive(ShaderType, Clone)]
pub struct SpriteParams {
    pub render_mode: u32,
    pub alpha_cutoff: f32,
    pub pass_mode: u32,
    pub _pad: u32,
    pub map_width_tiles: f32,
    pub map_height_tiles: f32,
    pub _pad2: UVec2,
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
        "shaders/worldmap/art/main.wgsl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/worldmap/art/main.wgsl".into()
    }
}

impl MaterialExtension for ArtGroundMaterialExtension {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        "shaders/worldmap/art/ground.wgsl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/worldmap/art/ground.wgsl".into()
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
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
    ec_land_res: Option<&Res<EcLandPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> bool {
    match source {
        ClientTextureSource::Cc => cc_art_res.is_some(),
        ClientTextureSource::Ec => {
            ec_art_res.is_some() && ec_land_res.is_some() && tilemeta_res.is_some()
        }
    }
}

fn resolve_effective_art_source(
    requested_source: ClientTextureSource,
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
    ec_land_res: Option<&Res<EcLandPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> Option<ClientTextureSource> {
    if source_available(
        requested_source,
        cc_art_res,
        ec_art_res,
        ec_land_res,
        tilemeta_res,
    ) {
        return Some(requested_source);
    }

    ClientTextureSource::ALL.into_iter().find(|candidate| {
        *candidate != requested_source
            && source_available(*candidate, cc_art_res, ec_art_res, ec_land_res, tilemeta_res)
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
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
) -> AtlasAllocationSpec {
    match active_source {
        Some(ClientTextureSource::Cc) => cc_art_res
            .map(|package| AtlasAllocationSpec {
                page_width: package.0.atlas_width().max(1),
                page_height: package.0.atlas_height().max(1),
                initial_layers: (package.0.pages().len().max(1) as u32).min(4),
                max_layers: package.0.pages().len().max(1) as u32,
                pixel_format: package_pixel_format(package.0.pages(), |page| page.pixel_format),
            })
            .unwrap_or_else(blank_atlas_spec),
        Some(ClientTextureSource::Ec) => ec_art_res
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
    ec_land_res: Option<&Res<EcLandPackageRes>>,
) -> AtlasAllocationSpec {
    if active_source != Some(ClientTextureSource::Ec) {
        return blank_atlas_spec();
    }

    ec_land_res
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
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
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
            if let Some(package) = cc_art_res {
                for (page_index, layer) in resident_pages {
                    if let Some(page) = package.0.pages().get(page_index as usize) {
                        atlas.queue_page_upload(
                            page_index,
                            layer,
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
            if let Some(package) = ec_art_res {
                for (page_index, layer) in resident_pages {
                    if let Some(page) = package.0.pages().get(page_index as usize) {
                        atlas.queue_page_upload(
                            page_index,
                            layer,
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
    ec_land_res: Option<&Res<EcLandPackageRes>>,
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

    if let Some(package) = ec_land_res {
        for (page_index, layer) in resident_pages {
            if let Some(page) = package.0.pages().get(page_index as usize) {
                atlas.queue_page_upload(
                    page_index,
                    layer,
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
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
    ec_land_res: Option<&Res<EcLandPackageRes>>,
) {
    let sprite_spec = sprite_atlas_spec(active_source, cc_art_res, ec_art_res);
    let ground_spec = ground_atlas_spec(active_source, ec_land_res);

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
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
) {
    let active_source = resolve_effective_art_source(
        settings.graphics.art_texture_source,
        cc_art_res.as_ref(),
        ec_art_res.as_ref(),
        ec_land_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    let sprite_spec = sprite_atlas_spec(active_source, cc_art_res.as_ref(), ec_art_res.as_ref());
    let ground_spec = ground_atlas_spec(active_source, ec_land_res.as_ref());

    let sprite_atlas = build_art_atlas(&mut images, sprite_spec);
    let sprite_atlas_handle = sprite_atlas.gpu_handle.clone();
    let ground_atlas = build_art_atlas(&mut images, ground_spec);
    let ground_atlas_handle = ground_atlas.gpu_handle.clone();

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
        _pad2: [0, 0],
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
                _pad: 0,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad2: UVec2::ZERO,
            },
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
                _pad: 0,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad2: UVec2::ZERO,
            },
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
        _pad2: [0, 0],
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
                _pad: 0,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad2: UVec2::ZERO,
            },
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
                _pad: 0,
                map_width_tiles: 1.0,
                map_height_tiles: 1.0,
                _pad2: UVec2::ZERO,
            },
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
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 1.0, 0.0]; 4],
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0]; 4],
    );
    // uv_b in shader usually corresponds to ATTRIBUTE_UV_1 if used by Bevy's default extractor,
    // but here we just need to satisfy the pipeline if the shader uses standard structs.
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_1,
        vec![[0.0, 0.0]; 4],
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[1.0, 1.0, 1.0, 1.0]; 4],
    );

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
    source_state: Res<crate::core::render::scene::world::art::statics_collect::StaticArtSourceState>,
    mut binding_state: ResMut<ActiveArtAtlasBindingState>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
) {
    if binding_state.configured_source == source_state.active_source {
        return;
    }

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
        cc_art_res.as_ref(),
        ec_art_res.as_ref(),
        ec_land_res.as_ref(),
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
    source_state: Res<crate::core::render::scene::world::art::statics_collect::StaticArtSourceState>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
) {
    apply_sprite_art_atlas_resize(
        &mut images,
        &mut materials,
        &render_assets,
        &mut sprite_atlas,
        &mut sprite_atlas_handle.0,
        source_state.active_source,
        cc_art_res.as_ref(),
        ec_art_res.as_ref(),
    );
    apply_ground_art_atlas_resize(
        &mut images,
        &mut ground_materials,
        &ground_render_assets,
        &mut ground_atlas,
        &mut ground_atlas_handle.0,
        ec_land_res.as_ref(),
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
        if !chunk_batches.sprite.iter().any(|desired| desired.key == batch.key) {
            let _ = commands.entity(entity).despawn();
        }
    }

    if debug_state.last_entity_count != Some(desired_count) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "static art draw entities: existing={} desired={}",
                existing_count,
                desired_count,
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
        if !chunk_batches.sprite.iter().any(|desired| desired.key == batch.key) {
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
        if !chunk_batches.ground.iter().any(|desired| desired.key == batch.key) {
            let _ = commands.entity(entity).despawn();
        }
    }

    if debug_state.last_ground_entity_count != Some(desired_count) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "static ground draw entities: existing={} desired={}",
                existing_count,
                desired_count,
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
        if !chunk_batches.ground.iter().any(|desired| desired.key == batch.key) {
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
        let Some(transparent_material) = materials.get_mut(&render_assets.transparent_material) else {
            return;
        };
        transparent_material.extension.params.render_mode = render_mode;
        transparent_material.extension.params.map_width_tiles = map_width_tiles;
        transparent_material.extension.params.map_height_tiles = map_height_tiles;
    }

    // Update buffer with new instances
    let _ = storage_buffers.insert(
        &opaque_buffer_handle,
        ShaderStorageBuffer::from(instances.0.clone()),
    );

    if debug_state.last_uploaded_instances != Some(instances.0.len()) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "static art upload: instances={} render_mode={}",
                instances.0.len(),
                render_mode,
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
        let Some(transparent_material) = materials.get_mut(&render_assets.transparent_material) else {
            return;
        };
        transparent_material.extension.params.map_width_tiles = map_width_tiles;
        transparent_material.extension.params.map_height_tiles = map_height_tiles;
    }

    let _ = storage_buffers.insert(
        &opaque_buffer_handle,
        ShaderStorageBuffer::from(instances.0.clone()),
    );

    if debug_state.last_uploaded_ground_instances != Some(instances.0.len()) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!("static ground upload: instances={}", instances.0.len()),
        );
        debug_state.last_uploaded_ground_instances = Some(instances.0.len());
    }
}
