use super::statics_collect::StaticChunkBatchKey;
use crate::configs::settings::Settings;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{TileMetaPackageRes, WorldLightsPackageRes};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat};
use std::collections::{HashMap, HashSet};

const TILE_FLAG_LIGHT_SOURCE: u64 = 0x00800000;
const LIGHT_PIXELS_PER_WORLD_TILE: f32 = 44.0;
const STATIC_LIGHT_Y_BIAS: f32 = 0.012;
const STATIC_LIGHT_ALPHA: f32 = 0.65;
const STATIC_LIGHT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, STATIC_LIGHT_ALPHA);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticLightInstance {
    pub key: StaticLightKey,
    pub light_id: u32,
    pub world_x: f32,
    pub world_z: f32,
    pub world_y: f32,
    pub width_world: f32,
    pub height_world: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Component)]
pub struct StaticLightKey {
    pub map_id: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub z: i8,
    pub graphic: u16,
    pub light_id: u32,
}

#[derive(Resource, Default)]
pub struct RenderStaticLightInstances(pub Vec<StaticLightInstance>);

#[derive(Resource, Default)]
pub struct StaticLightDrawDebugState {
    last_instance_count: Option<usize>,
}

#[derive(Resource, Default)]
pub struct StaticLightMaterialCache {
    materials_by_light_id: HashMap<u32, Handle<StandardMaterial>>,
}

#[derive(Component)]
pub struct StaticLightEntity;

pub fn build_static_light_mesh() -> Mesh {
    use bevy::mesh::Indices;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5, 0.0, -0.5],
            [0.5, 0.0, -0.5],
            [-0.5, 0.0, 0.5],
            [0.5, 0.0, 0.5],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
    mesh
}

pub fn sys_collect_visible_static_lights(
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    world_lights_res: Option<Res<WorldLightsPackageRes>>,
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
    mut output: ResMut<RenderStaticLightInstances>,
    mut debug_state: ResMut<StaticLightDrawDebugState>,
) {
    output.0.clear();

    if !settings.world_rendering.enable_statics {
        return;
    }

    let Some(tilemeta) = tilemeta_res else {
        return;
    };
    let Some(world_lights) = world_lights_res else {
        return;
    };

    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();

    let mut visible_chunk_keys = chunks_q
        .iter()
        .filter(|chunk| chunk.parent_map_id == map_id)
        .map(|chunk| StaticChunkBatchKey {
            map_id,
            gx: chunk.gx,
            gy: chunk.gy,
            scale: chunk.scale,
        })
        .collect::<Vec<_>>();
    visible_chunk_keys.sort_by_key(|key| (key.gy, key.gx, key.scale));

    let mut seen = HashSet::new();
    for chunk_key in visible_chunk_keys {
        let start_gx = chunk_key.gx * CHUNK_STORAGE_BLOCKS_DIM;
        let start_gy = chunk_key.gy * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gx = start_gx + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gy = start_gy + chunk_key.scale * CHUNK_STORAGE_BLOCKS_DIM;

        for gy in start_gy..end_gy {
            for gx in start_gx..end_gx {
                let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                    continue;
                };

                for tile in tiles {
                    let Some(meta) = tilemeta.0.item_tile(tile.graphic as u32) else {
                        continue;
                    };
                    if meta.flags & TILE_FLAG_LIGHT_SOURCE == 0 {
                        continue;
                    }

                    let light_id = meta.quality as u32;
                    let Some(slot) = world_lights.0.present_slot(light_id) else {
                        continue;
                    };
                    if slot.width == 0 || slot.height == 0 {
                        continue;
                    }

                    let tile_x = gx * MAP_STORAGE_BLOCK_TILE_DIM + tile.x_offset() as u32;
                    let tile_y = gy * MAP_STORAGE_BLOCK_TILE_DIM + tile.y_offset() as u32;
                    let key = StaticLightKey {
                        map_id,
                        tile_x,
                        tile_y,
                        z: tile.z,
                        graphic: tile.graphic,
                        light_id,
                    };
                    if !seen.insert(key) {
                        continue;
                    }

                    output.0.push(StaticLightInstance {
                        key,
                        light_id,
                        world_x: tile_x as f32 + 0.5,
                        world_z: tile_y as f32 + 0.5,
                        world_y: (tile.z as f32) * 0.1 + STATIC_LIGHT_Y_BIAS,
                        width_world: slot.width as f32 / LIGHT_PIXELS_PER_WORLD_TILE,
                        height_world: slot.height as f32 / LIGHT_PIXELS_PER_WORLD_TILE,
                    });
                }
            }
        }
    }

    if debug_state.last_instance_count != Some(output.0.len()) {
        console_logger::one(
            LogSev::Debug,
            LogAbout::RenderWorldArt,
            &format!("static light masks: instances={}", output.0.len()),
        );
        debug_state.last_instance_count = Some(output.0.len());
    }
}

pub fn sys_sync_static_light_entities(
    mut commands: Commands,
    instances: Res<RenderStaticLightInstances>,
    world_lights_res: Option<Res<WorldLightsPackageRes>>,
    mut material_cache: ResMut<StaticLightMaterialCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_handle: Local<Option<Handle<Mesh>>>,
    existing_q: Query<(Entity, &StaticLightKey), With<StaticLightEntity>>,
) {
    let Some(world_lights) = world_lights_res else {
        for (entity, _) in existing_q.iter() {
            let _ = commands.entity(entity).despawn();
        }
        return;
    };

    let desired_keys = instances
        .0
        .iter()
        .map(|instance| instance.key)
        .collect::<HashSet<_>>();
    let existing_by_key = existing_q
        .iter()
        .map(|(entity, key)| (*key, entity))
        .collect::<HashMap<_, _>>();

    let mesh_handle = mesh_handle
        .get_or_insert_with(|| meshes.add(build_static_light_mesh()))
        .clone();
    for instance in &instances.0 {
        let Some(material_handle) = static_light_material(
            instance.light_id,
            &world_lights.0,
            &mut material_cache,
            &mut images,
            &mut materials,
        ) else {
            continue;
        };

        let transform = Transform::from_xyz(instance.world_x, instance.world_y, instance.world_z)
            .with_scale(Vec3::new(instance.width_world, 1.0, instance.height_world));

        if let Some(entity) = existing_by_key.get(&instance.key) {
            let _ = commands.entity(*entity).insert(transform);
        } else {
            commands.spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle),
                transform,
                NoFrustumCulling,
                instance.key,
                StaticLightEntity,
            ));
        }
    }

    for (entity, key) in existing_q.iter() {
        if !desired_keys.contains(key) {
            let _ = commands.entity(entity).despawn();
        }
    }
}

fn static_light_material(
    light_id: u32,
    world_lights: &udd_assets::world_lights::WorldLightsPackage,
    material_cache: &mut StaticLightMaterialCache,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    if let Some(handle) = material_cache.materials_by_light_id.get(&light_id) {
        return Some(handle.clone());
    }

    let slot = world_lights.present_slot(light_id)?;
    let rgba = match world_lights.read_light_bytes(light_id) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("Failed to read static light mask {light_id}: {error}");
            return None;
        }
    };
    if rgba.len() != slot.width as usize * slot.height as usize * 4 {
        log::warn!(
            "Static light mask {light_id} has invalid byte length {} for {}x{} rgba8888.",
            rgba.len(),
            slot.width,
            slot.height,
        );
        return None;
    }

    let mut image = Image::new(
        Extent3d {
            width: slot.width as u32,
            height: slot.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    let image_handle = images.add(image);
    let material_handle = materials.add(StandardMaterial {
        base_color: STATIC_LIGHT_COLOR,
        base_color_texture: Some(image_handle),
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        unlit: true,
        fog_enabled: false,
        depth_bias: 1.0,
        ..default()
    });
    material_cache
        .materials_by_light_id
        .insert(light_id, material_handle.clone());
    Some(material_handle)
}
