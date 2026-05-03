use crate::core::render::scene::world::art::statics_collect::{
    RenderStaticInstances, SpriteInstance,
};
use crate::core::texture_cache::art::{ArtPageAtlas, ArtPageAtlasHandle};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::render::storage::ShaderStorageBuffer;

#[derive(ShaderType, Clone)]
pub struct SpriteParams {
    pub render_mode: u32,
    pub alpha_cutoff: f32,
    pub _pad: Vec2,
}

pub type ArtSpriteMaterial = ExtendedMaterial<StandardMaterial, ArtSpriteMaterialExtension>;

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

impl MaterialExtension for ArtSpriteMaterialExtension {
    fn vertex_shader() -> bevy::shader::ShaderRef {
        "shaders/worldmap/art/main.wgsl".into()
    }
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/worldmap/art/main.wgsl".into()
    }
}

#[derive(Component)]
pub struct StaticsDrawEntity;

pub fn sys_setup_art_page_atlas(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let max_layers = 16;
    let page_width = 2048;
    let page_height = 2048;

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
        uv_min: [0.0, 0.0],
        uv_max: [0.0, 0.0],
        pixel_size: [0.0, 0.0],
        _pad: [0.0, 0.0],
        color_rgba: [0.0, 0.0, 0.0, 0.0],
    }]);

    let buffer_handle = storage_buffers.add(initial_buffer);

    let material_handle = materials.add(ArtSpriteMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            unlit: true,
            ..default()
        },
        extension: ArtSpriteMaterialExtension {
            atlas: atlas_handle,
            instances: buffer_handle.clone(),
            params: SpriteParams {
                render_mode: 0,
                alpha_cutoff: 0.5,
                _pad: Vec2::ZERO,
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

    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material_handle),
        Transform::from_translation(Vec3::ZERO),
        bevy::camera::visibility::NoFrustumCulling,
        StaticsDrawEntity,
    ));
}

pub fn sys_update_sprite_instance_buffer(
    instances: Res<RenderStaticInstances>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderStorageBuffer>>,
    q_entity: Query<&MeshMaterial3d<ArtSpriteMaterial>, With<StaticsDrawEntity>>,
    zoom: Res<crate::core::render::scene::camera::RenderZoom>,
) {
    if instances.0.is_empty() {
        return;
    }

    let Some(material_handle) = q_entity.iter().next() else {
        return;
    };
    let Some(material) = materials.get_mut(&material_handle.0) else {
        return;
    };

    // Update buffer with new instances
    storage_buffers.insert(
        &material.extension.instances,
        ShaderStorageBuffer::from(instances.0.clone()),
    );

    // Update render mode based on zoom
    material.extension.params.render_mode = if zoom.0 >= 20.0 { 1 } else { 0 };
}
