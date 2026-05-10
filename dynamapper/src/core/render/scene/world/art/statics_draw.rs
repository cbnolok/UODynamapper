use crate::core::render::scene::world::art::statics_collect::{
    GroundTileInstance, RenderStaticInstances, RenderStaticLandInstances, SpriteInstance,
};
use crate::core::texture_cache::art::{ArtPageAtlas, ArtPageAtlasHandle};
use crate::core::uo_files_loader::{CcArtPackageRes, EcArtPackageRes, EcLandPackageRes};
use crate::console_logger::{self, LogAbout, LogSev};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::render::storage::ShaderStorageBuffer;

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

pub fn sys_setup_art_page_atlas(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut ground_materials: ResMut<Assets<ArtGroundMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
) {
    // 32 layers still churns too aggressively for visible classic static-art page sets.
    // Keep this aligned with the working budget discussed in the render investigation.
    let max_layers = 64;
    let page_width = cc_art_res
        .as_ref()
        .map(|package| package.0.atlas_width())
        .into_iter()
        .chain(ec_art_res.as_ref().map(|package| package.0.atlas_width()))
        .chain(ec_land_res.as_ref().map(|package| package.0.atlas_width()))
        .max()
        .unwrap_or(2048)
        .max(2048);
    let page_height = cc_art_res
        .as_ref()
        .map(|package| package.0.atlas_height())
        .into_iter()
        .chain(ec_art_res.as_ref().map(|package| package.0.atlas_height()))
        .chain(ec_land_res.as_ref().map(|package| package.0.atlas_height()))
        .max()
        .unwrap_or(2048)
        .max(2048);

    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let mut image = Image::new_fill(
        Extent3d {
            width: page_width,
            height: page_height,
            depth_or_array_layers: max_layers,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let atlas_handle = images.add(image);

    commands.insert_resource(ArtPageAtlas::new(
        atlas_handle.clone(),
        page_width,
        page_height,
        max_layers,
    ));
    commands.insert_resource(ArtPageAtlasHandle(atlas_handle.clone()));

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
        _pad1: 0,
        _pad2: [0, 0],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);

    let buffer_handle = storage_buffers.add(initial_buffer);
    let sprite_atlas_handle = atlas_handle.clone();
    let ground_atlas_handle = atlas_handle.clone();

    let opaque_material_handle = materials.add(ArtSpriteMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtSpriteMaterialExtension {
            atlas: sprite_atlas_handle,
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
            atlas: atlas_handle.clone(),
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
        tile_x: 0.0,
        tile_y: 0.0,
        priority_z_units: 0.0,
        _pad1: 0,
        _pad2: [0, 0],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);
    let ground_buffer_handle = storage_buffers.add(initial_ground_buffer);

    let ground_material_handle = ground_materials.add(ArtGroundMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            unlit: true,
            ..default()
        },
        extension: ArtGroundMaterialExtension {
            atlas: ground_atlas_handle,
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
            atlas: atlas_handle.clone(),
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
}

pub fn sys_sync_static_sprite_entities(
    mut commands: Commands,
    instances: Res<RenderStaticInstances>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &MeshTag), With<StaticsDrawEntity>>,
) {
    let desired_count = instances.0.len();
    let existing_count = existing_q.iter().count();
    let mut existing_entities = existing_q.iter().map(|(entity, _)| entity);

    for entity in existing_entities.by_ref().skip(desired_count) {
        let _ = commands.entity(entity).despawn();
    }

    for (slot_index, (entity, mesh_tag)) in existing_q.iter().take(desired_count).enumerate() {
        let desired_tag = MeshTag(slot_index as u32);
        if *mesh_tag != desired_tag {
            let _ = commands.entity(entity).insert(desired_tag);
        }
    }

    for slot_index in existing_count..desired_count {
        commands.spawn((
            Mesh3d(render_assets.mesh.clone()),
            MeshMaterial3d(render_assets.opaque_material.clone()),
            MeshTag(slot_index as u32),
            Transform::IDENTITY,
            NoFrustumCulling,
            StaticsDrawEntity,
        ));
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
    instances: Res<RenderStaticInstances>,
    render_assets: Res<ArtSpriteRenderAssets>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &MeshTag), With<StaticsTransparentDrawEntity>>,
) {
    let desired_count = instances.0.len();
    let existing_count = existing_q.iter().count();
    let mut existing_entities = existing_q.iter().map(|(entity, _)| entity);

    for entity in existing_entities.by_ref().skip(desired_count) {
        let _ = commands.entity(entity).despawn();
    }

    for (slot_index, (entity, mesh_tag)) in existing_q.iter().take(desired_count).enumerate() {
        let desired_tag = MeshTag(slot_index as u32);
        if *mesh_tag != desired_tag {
            let _ = commands.entity(entity).insert(desired_tag);
        }
    }

    for slot_index in existing_count..desired_count {
        commands.spawn((
            Mesh3d(render_assets.mesh.clone()),
            MeshMaterial3d(render_assets.transparent_material.clone()),
            MeshTag(slot_index as u32),
            Transform::IDENTITY,
            NoFrustumCulling,
            StaticsTransparentDrawEntity,
        ));
    }

    if debug_state.last_transparent_entity_count != Some(desired_count) {
        debug_state.last_transparent_entity_count = Some(desired_count);
    }
}

pub fn sys_sync_static_ground_entities(
    mut commands: Commands,
    instances: Res<RenderStaticLandInstances>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &MeshTag), With<StaticsGroundDrawEntity>>,
) {
    let desired_count = instances.0.len();
    let existing_count = existing_q.iter().count();
    let mut existing_entities = existing_q.iter().map(|(entity, _)| entity);

    for entity in existing_entities.by_ref().skip(desired_count) {
        let _ = commands.entity(entity).despawn();
    }

    for (slot_index, (entity, mesh_tag)) in existing_q.iter().take(desired_count).enumerate() {
        let desired_tag = MeshTag(slot_index as u32);
        if *mesh_tag != desired_tag {
            let _ = commands.entity(entity).insert(desired_tag);
        }
    }

    for slot_index in existing_count..desired_count {
        commands.spawn((
            Mesh3d(render_assets.mesh.clone()),
            MeshMaterial3d(render_assets.opaque_material.clone()),
            MeshTag(slot_index as u32),
            Transform::IDENTITY,
            NoFrustumCulling,
            StaticsGroundDrawEntity,
        ));
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
    instances: Res<RenderStaticLandInstances>,
    render_assets: Res<ArtGroundRenderAssets>,
    mut debug_state: ResMut<StaticArtDrawDebugState>,
    existing_q: Query<(Entity, &MeshTag), With<StaticsGroundTransparentDrawEntity>>,
) {
    let desired_count = instances.0.len();
    let existing_count = existing_q.iter().count();
    let mut existing_entities = existing_q.iter().map(|(entity, _)| entity);

    for entity in existing_entities.by_ref().skip(desired_count) {
        let _ = commands.entity(entity).despawn();
    }

    for (slot_index, (entity, mesh_tag)) in existing_q.iter().take(desired_count).enumerate() {
        let desired_tag = MeshTag(slot_index as u32);
        if *mesh_tag != desired_tag {
            let _ = commands.entity(entity).insert(desired_tag);
        }
    }

    for slot_index in existing_count..desired_count {
        commands.spawn((
            Mesh3d(render_assets.mesh.clone()),
            MeshMaterial3d(render_assets.transparent_material.clone()),
            MeshTag(slot_index as u32),
            Transform::IDENTITY,
            NoFrustumCulling,
            StaticsGroundTransparentDrawEntity,
        ));
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
