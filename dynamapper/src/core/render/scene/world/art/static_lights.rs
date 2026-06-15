use super::statics_collect::StaticChunkBatchKey;
use crate::configs::{settings::Settings, shader_presets::UniformState};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{HuesPackageRes, TileMetaPackageRes, WorldLightsPackageRes};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

const TILE_FLAG_LIGHT_SOURCE: u64 = 0x00800000;
const LIGHT_PIXELS_PER_WORLD_TILE: f32 = 44.0;
const STATIC_LIGHT_Y_BIAS: f32 = 0.012;
const STATIC_LIGHT_ALPHA: f32 = 0.65;
const STATIC_LIGHT_DEPTH_BIAS: f32 = 128.0;
const STATIC_LIGHT_BILLBOARD_RIGHT: Vec3 = Vec3::new(-0.70710677, 0.0, 0.70710677);
const STATIC_LIGHT_BILLBOARD_UP: Vec3 = Vec3::new(0.4082483, -0.8164966, 0.4082483);
const STATIC_LIGHT_BILLBOARD_NORMAL: Vec3 = Vec3::new(0.57735026, 0.57735026, 0.57735026);
const CLASSICUO_LIGHT_CURVES: [[u8; 32]; 6] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
    [0, 1, 2, 4, 6, 8, 11, 14, 17, 20, 23, 26, 29, 30, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31, 31],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 17, 19, 21, 23, 25, 27],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 5, 10, 15, 20, 25, 30, 30, 18, 18, 18, 18, 18, 18, 18],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StaticLightHueSourceKind {
    StoredRgb,
    Package,
    ClassicLightShader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StaticLightRenderStyle {
    Classic,
    Enhanced,
    Kr,
}

#[derive(Clone, Copy)]
struct StaticLightStyleProfile {
    max_alpha: f32,
    daylight_alpha: f32,
    darkness_gamma: f32,
    saturation: f32,
    brightness: f32,
    alpha_gamma: f32,
    cool_edge_tint: f32,
    mottle_strength: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StaticLightMaterialKey {
    light_id: u32,
    hue_id: u16,
    hue_source: StaticLightHueSourceKind,
    style: StaticLightRenderStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticLightInstance {
    pub key: StaticLightKey,
    pub light_id: u32,
    pub hue_id: u16,
    pub color_rgb: [f32; 3],
    pub world_x: f32,
    pub world_z: f32,
    pub world_y: f32,
    pub width_world: f32,
    pub height_world: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Component)]
#[component(clone_behavior = Ignore)]
pub struct StaticLightKey {
    pub map_id: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub z: i8,
    pub graphic: u16,
    pub light_id: u32,
    pub hue_id: u16,
}

impl Hash for StaticLightKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let map_tile_x = (u64::from(self.map_id) << 32) | u64::from(self.tile_x);
        let tile_y_graphic_hue = (u64::from(self.tile_y) << 32)
            | (u64::from(self.graphic) << 16)
            | u64::from(self.hue_id);
        let light_z = (u64::from(self.light_id) << 8) | u64::from(self.z as u8);

        state.write_u64(map_tile_x);
        state.write_u64(tile_y_graphic_hue);
        state.write_u64(light_z);
    }
}

#[derive(Resource, Default)]
pub struct RenderStaticLightInstances(pub Vec<StaticLightInstance>);

#[derive(Resource, Default)]
pub struct StaticLightDrawDebugState {
    last_instance_count: Option<usize>,
}

#[derive(Resource, Default)]
pub struct StaticLightMaterialCache {
    materials_by_key: HashMap<StaticLightMaterialKey, Handle<StandardMaterial>>,
    hue_texture_bytes: Option<Vec<u8>>,
    hue_texture_failed: bool,
}

#[derive(Component)]
pub struct StaticLightEntity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticLightSyncSignature {
    world_lights_loaded: bool,
    render_style: StaticLightRenderStyle,
    material_alpha_bits: u32,
    instance_count: usize,
    instance_hash: u64,
}

fn static_light_sync_signature(
    instances: &RenderStaticLightInstances,
    render_style: StaticLightRenderStyle,
    material_alpha: f32,
    world_lights_loaded: bool,
) -> StaticLightSyncSignature {
    let mut hash = 0xcbf29ce484222325u64;
    for instance in &instances.0 {
        static_light_sync_hash_u32(&mut hash, instance.key.map_id);
        static_light_sync_hash_u32(&mut hash, instance.key.tile_x);
        static_light_sync_hash_u32(&mut hash, instance.key.tile_y);
        static_light_sync_hash_u32(&mut hash, u32::from(instance.key.z as u8));
        static_light_sync_hash_u32(&mut hash, u32::from(instance.key.graphic));
        static_light_sync_hash_u32(&mut hash, instance.key.light_id);
        static_light_sync_hash_u32(&mut hash, u32::from(instance.key.hue_id));
        static_light_sync_hash_u32(&mut hash, instance.light_id);
        static_light_sync_hash_u32(&mut hash, u32::from(instance.hue_id));
        static_light_sync_hash_u32(&mut hash, instance.world_x.to_bits());
        static_light_sync_hash_u32(&mut hash, instance.world_y.to_bits());
        static_light_sync_hash_u32(&mut hash, instance.world_z.to_bits());
        static_light_sync_hash_u32(&mut hash, instance.width_world.to_bits());
        static_light_sync_hash_u32(&mut hash, instance.height_world.to_bits());
        for channel in instance.color_rgb {
            static_light_sync_hash_u32(&mut hash, channel.to_bits());
        }
    }

    StaticLightSyncSignature {
        world_lights_loaded,
        render_style,
        material_alpha_bits: material_alpha.to_bits(),
        instance_count: instances.0.len(),
        instance_hash: hash,
    }
}

fn static_light_sync_hash_u32(hash: &mut u64, value: u32) {
    *hash ^= u64::from(value);
    *hash = hash.wrapping_mul(0x100000001b3);
}

pub fn build_static_light_mesh() -> Mesh {
    use bevy::mesh::Indices;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [-0.5, 0.5, 0.0],
            [0.5, 0.5, 0.0],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
    mesh
}

fn static_light_billboard_rotation() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        STATIC_LIGHT_BILLBOARD_RIGHT,
        STATIC_LIGHT_BILLBOARD_UP,
        STATIC_LIGHT_BILLBOARD_NORMAL,
    ))
}

pub fn sys_collect_visible_static_lights(
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    world_lights_res: Option<Res<WorldLightsPackageRes>>,
    hues_package_res: Option<Res<HuesPackageRes>>,
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
    mut material_cache: ResMut<StaticLightMaterialCache>,
    mut output: ResMut<RenderStaticLightInstances>,
    mut debug_state: ResMut<StaticLightDrawDebugState>,
) {
    output.0.clear();

    if !settings.world_rendering.enable_static_lights {
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
                        hue_id: tile.hue,
                    };
                    if !seen.insert(key) {
                        continue;
                    }

                    output.0.push(StaticLightInstance {
                        key,
                        light_id,
                        hue_id: tile.hue,
                        color_rgb: static_light_response_color_from_sources(
                            tile.graphic,
                            tile.hue,
                            light_id,
                            &world_lights.0,
                            hues_package_res.as_deref(),
                            &mut material_cache,
                        ),
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
    hues_package_res: Option<Res<HuesPackageRes>>,
    uniform_state: Res<UniformState>,
    mut material_cache: ResMut<StaticLightMaterialCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_handle: Local<Option<Handle<Mesh>>>,
    mut last_alpha: Local<Option<f32>>,
    mut last_sync_signature: Local<Option<StaticLightSyncSignature>>,
    existing_q: Query<(Entity, &StaticLightKey), With<StaticLightEntity>>,
) {
    let render_style = StaticLightRenderStyle::from_shading_mode(uniform_state.effects.shading_mode);
    let material_alpha = static_light_decal_alpha(
        uniform_state.global_lighting,
        uniform_state.effects.static_light_decal_visibility,
        render_style,
    );
    let Some(world_lights) = world_lights_res else {
        let signature = static_light_sync_signature(
            &instances,
            render_style,
            material_alpha,
            false,
        );
        if *last_sync_signature == Some(signature) {
            return;
        }

        for (entity, _) in existing_q.iter() {
            commands.entity(entity).despawn();
        }
        *last_alpha = None;
        *last_sync_signature = Some(signature);
        return;
    };

    let signature = static_light_sync_signature(
        &instances,
        render_style,
        material_alpha,
        true,
    );
    if *last_sync_signature == Some(signature) {
        return;
    }

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
    let mut sync_complete = true;
    for instance in &instances.0 {
        let Some(material_handle) = static_light_material(
            instance.key.graphic,
            instance.light_id,
            instance.hue_id,
            render_style,
            material_alpha,
            &world_lights.0,
            hues_package_res.as_deref(),
            &mut material_cache,
            &mut images,
            &mut materials,
        ) else {
            sync_complete = false;
            continue;
        };

        let transform = Transform::from_xyz(instance.world_x, instance.world_y, instance.world_z)
            .with_rotation(static_light_billboard_rotation())
            .with_scale(Vec3::new(instance.width_world, instance.height_world, 1.0));

        if let Some(entity) = existing_by_key.get(&instance.key) {
            let _ = commands
                .entity(*entity)
                .insert((transform, MeshMaterial3d(material_handle)));
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

    if *last_alpha != Some(material_alpha) {
        for handle in material_cache.materials_by_key.values() {
            if let Some(material) = materials.get_mut(handle) {
                material.base_color = static_light_base_color(material_alpha);
            }
        }
        *last_alpha = Some(material_alpha);
    }

    for (entity, key) in existing_q.iter() {
        if !desired_keys.contains(key) {
            commands.entity(entity).despawn();
        }
    }

    if sync_complete {
        *last_sync_signature = Some(signature);
    } else {
        *last_sync_signature = None;
    }
}

fn static_light_material(
    graphic: u16,
    light_id: u32,
    hue_id: u16,
    render_style: StaticLightRenderStyle,
    material_alpha: f32,
    world_lights: &udd_assets::world_lights::WorldLightsPackage,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let hue_source_kind = resolve_static_light_hue_source_kind(
        graphic,
        hue_id,
        hues_package,
        material_cache,
    );
    let material_key = StaticLightMaterialKey {
        light_id,
        hue_id: static_light_material_color_key(graphic, hue_id, hue_source_kind),
        hue_source: hue_source_kind,
        style: render_style,
    };
    if let Some(handle) = material_cache.materials_by_key.get(&material_key) {
        return Some(handle.clone());
    }

    let slot = world_lights.present_slot(light_id)?;
    let mut rgba = match world_lights.read_light_bytes(light_id) {
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

    {
        let hue_source = resolve_static_light_hue_source(
            graphic,
            hue_id,
            hues_package,
            material_cache,
        );
        apply_static_light_hue(&mut rgba, hue_source);
        apply_static_light_style(&mut rgba, slot.width as u32, slot.height as u32, render_style);
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
        base_color: static_light_base_color(material_alpha),
        base_color_texture: Some(image_handle),
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        unlit: true,
        fog_enabled: false,
        depth_bias: STATIC_LIGHT_DEPTH_BIAS,
        ..default()
    });
    material_cache
        .materials_by_key
        .insert(material_key, material_handle.clone());
    Some(material_handle)
}

impl StaticLightRenderStyle {
    const fn from_shading_mode(shading_mode: u32) -> Self {
        match shading_mode {
            1 => Self::Enhanced,
            2 => Self::Kr,
            _ => Self::Classic,
        }
    }
}

fn static_light_style_profile(style: StaticLightRenderStyle) -> StaticLightStyleProfile {
    match style {
        StaticLightRenderStyle::Classic => StaticLightStyleProfile {
            max_alpha: STATIC_LIGHT_ALPHA,
            daylight_alpha: 0.0,
            darkness_gamma: 1.0,
            saturation: 1.05,
            brightness: 1.0,
            alpha_gamma: 1.0,
            cool_edge_tint: 0.0,
            mottle_strength: 0.0,
        },
        StaticLightRenderStyle::Enhanced => StaticLightStyleProfile {
            max_alpha: 0.50,
            daylight_alpha: 0.03,
            darkness_gamma: 1.12,
            saturation: 0.82,
            brightness: 0.92,
            alpha_gamma: 0.82,
            cool_edge_tint: 0.0,
            mottle_strength: 0.0,
        },
        StaticLightRenderStyle::Kr => StaticLightStyleProfile {
            max_alpha: 0.20,
            daylight_alpha: 0.0,
            darkness_gamma: 1.50,
            saturation: 0.62,
            brightness: 0.66,
            alpha_gamma: 1.70,
            cool_edge_tint: 0.18,
            mottle_strength: 0.22,
        },
    }
}

fn static_light_alpha_for_global_lighting(
    global_lighting: f32,
    style: StaticLightRenderStyle,
) -> f32 {
    let profile = static_light_style_profile(style);
    let darkness = (1.0 - global_lighting).clamp(0.0, 1.0);
    (profile.daylight_alpha + darkness.powf(profile.darkness_gamma))
        .clamp(0.0, profile.daylight_alpha + profile.max_alpha)
}

fn static_light_decal_alpha(
    global_lighting: f32,
    decal_visibility: f32,
    style: StaticLightRenderStyle,
) -> f32 {
    let profile = static_light_style_profile(style);
    let interaction = decal_visibility.clamp(0.0, 2.0);
    let alpha = static_light_alpha_for_global_lighting(global_lighting, style) * interaction;
    alpha.clamp(0.0, (profile.daylight_alpha + profile.max_alpha) * 2.0)
}

fn static_light_base_color(alpha: f32) -> Color {
    Color::srgba(1.0, 1.0, 1.0, alpha)
}

pub fn static_light_response_color(graphic: u16) -> [f32; 3] {
    let Some(shader_id) = classicuo_light_shader_id(graphic) else {
        return [1.0, 0.72, 0.42];
    };
    if shader_id == 0 {
        return [1.0, 0.72, 0.42];
    }

    let color = sample_classicuo_light_shader(shader_id, 248);
    [
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
    ]
}

fn static_light_response_color_from_sources(
    graphic: u16,
    hue_id: u16,
    light_id: u32,
    world_lights: &udd_assets::world_lights::WorldLightsPackage,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
) -> [f32; 3] {
    if classicuo_light_shader_id(graphic).is_some_and(|shader_id| shader_id != 0) {
        return static_light_response_color(graphic);
    }

    if let Some(color) = package_hue_response_color(hue_id, hues_package, material_cache) {
        return color;
    }

    world_lights
        .read_light_bytes(light_id)
        .ok()
        .and_then(|rgba| sampled_static_light_mask_color(&rgba))
        .unwrap_or([1.0, 0.72, 0.42])
}

fn package_hue_response_color(
    hue_id: u16,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
) -> Option<[f32; 3]> {
    let StaticLightHueSource::Package {
        hue_id,
        texture_bytes,
    } = package_hue_source(hue_id, hues_package, material_cache)?
    else {
        return None;
    };

    let color = sample_hue_lookup(texture_bytes, hue_id, 248)?;
    normalized_response_color_from_rgba(color)
}

fn normalized_response_color_from_rgba(color: [u8; 4]) -> Option<[f32; 3]> {
    let mut rgb = [
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
    ];
    let max_channel = rgb[0].max(rgb[1]).max(rgb[2]);
    if max_channel <= 0.001 {
        return None;
    }

    rgb[0] /= max_channel;
    rgb[1] /= max_channel;
    rgb[2] /= max_channel;
    Some(rgb)
}

fn sampled_static_light_mask_color(rgba: &[u8]) -> Option<[f32; 3]> {
    let mut weighted_rgb = [0.0; 3];
    let mut alpha_sum = 0.0;
    for pixel in rgba.chunks_exact(4) {
        let alpha = f32::from(pixel[3]) / 255.0;
        if alpha <= 0.0 {
            continue;
        }
        weighted_rgb[0] += f32::from(pixel[0]) / 255.0 * alpha;
        weighted_rgb[1] += f32::from(pixel[1]) / 255.0 * alpha;
        weighted_rgb[2] += f32::from(pixel[2]) / 255.0 * alpha;
        alpha_sum += alpha;
    }
    if alpha_sum <= 0.001 {
        return None;
    }

    let mut color = [
        weighted_rgb[0] / alpha_sum,
        weighted_rgb[1] / alpha_sum,
        weighted_rgb[2] / alpha_sum,
    ];
    let max_channel = color[0].max(color[1]).max(color[2]);
    let min_channel = color[0].min(color[1]).min(color[2]);
    if max_channel <= 0.001 || max_channel - min_channel < 0.08 {
        return None;
    }

    color[0] /= max_channel;
    color[1] /= max_channel;
    color[2] /= max_channel;
    Some(color)
}

fn resolve_static_light_hue_source_kind(
    graphic: u16,
    hue_id: u16,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
) -> StaticLightHueSourceKind {
    resolve_static_light_hue_source(
        graphic,
        hue_id,
        hues_package,
        material_cache,
    )
    .kind()
}

#[derive(Clone, Copy)]
enum StaticLightHueSource<'a> {
    StoredRgb,
    ClassicLightShader(u16),
    Package {
        hue_id: u16,
        texture_bytes: &'a [u8],
    },
}

impl StaticLightHueSource<'_> {
    const fn kind(self) -> StaticLightHueSourceKind {
        match self {
            Self::StoredRgb => StaticLightHueSourceKind::StoredRgb,
            Self::ClassicLightShader(_) => StaticLightHueSourceKind::ClassicLightShader,
            Self::Package { .. } => StaticLightHueSourceKind::Package,
        }
    }
}

fn static_light_material_color_key(
    graphic: u16,
    hue_id: u16,
    hue_source_kind: StaticLightHueSourceKind,
) -> u16 {
    match hue_source_kind {
        StaticLightHueSourceKind::StoredRgb => 0,
        StaticLightHueSourceKind::ClassicLightShader => {
            classicuo_light_shader_id(graphic).unwrap_or(0)
        }
        _ => hue_id,
    }
}

fn resolve_static_light_hue_source<'a>(
    graphic: u16,
    hue_id: u16,
    hues_package: Option<&'a HuesPackageRes>,
    material_cache: &'a mut StaticLightMaterialCache,
) -> StaticLightHueSource<'a> {
    if let Some(shader_id) = classicuo_light_shader_id(graphic) {
        if shader_id != 0 {
            return StaticLightHueSource::ClassicLightShader(shader_id);
        }
        return StaticLightHueSource::StoredRgb;
    }

    if hue_id == 0 {
        return StaticLightHueSource::StoredRgb;
    }

    package_hue_source(hue_id, hues_package, material_cache)
        .unwrap_or(StaticLightHueSource::StoredRgb)
}

fn package_hue_source<'a>(
    hue_id: u16,
    hues_package: Option<&'a HuesPackageRes>,
    material_cache: &'a mut StaticLightMaterialCache,
) -> Option<StaticLightHueSource<'a>> {
    let package = hues_package?;
    package.0.texture_coord_for_hue(hue_id)?;
    if material_cache.hue_texture_bytes.is_none()
        && !material_cache.hue_texture_failed
    {
        match package.0.read_texture_bytes() {
            Ok(bytes) => {
                material_cache.hue_texture_bytes = Some(bytes);
            }
            Err(error) => {
                material_cache.hue_texture_failed = true;
                log::warn!("Failed to read hues.uddp texture for static light hues: {error}");
            }
        }
    }

    material_cache
        .hue_texture_bytes
        .as_deref()
        .map(|texture_bytes| StaticLightHueSource::Package {
            hue_id,
            texture_bytes,
        })
}

fn apply_static_light_hue(rgba: &mut [u8], source: StaticLightHueSource<'_>) {
    match source {
        StaticLightHueSource::StoredRgb => {}
        StaticLightHueSource::ClassicLightShader(shader_id) => {
            apply_static_light_hue_with_sampler(rgba, |luma| {
                sample_classicuo_light_shader(shader_id, luma)
            });
        }
        StaticLightHueSource::Package {
            hue_id,
            texture_bytes,
        } => {
            apply_static_light_hue_with_sampler(rgba, |luma| {
                sample_hue_lookup(texture_bytes, hue_id, luma).unwrap_or([luma, luma, luma, 255])
            });
        }
    }
}

fn apply_static_light_style(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    style: StaticLightRenderStyle,
) {
    if style == StaticLightRenderStyle::Classic {
        return;
    }

    let profile = static_light_style_profile(style);
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }

        let alpha = f32::from(pixel[3]) / 255.0;
        let luma = f32::from(linear_luma_u8(pixel[0], pixel[1], pixel[2])) / 255.0;
        let mut rgb = [
            f32::from(pixel[0]) / 255.0,
            f32::from(pixel[1]) / 255.0,
            f32::from(pixel[2]) / 255.0,
        ];
        for channel in &mut rgb {
            *channel = (luma + (*channel - luma) * profile.saturation) * profile.brightness;
        }

        if profile.cool_edge_tint > 0.0 {
            let edge = (1.0 - alpha).clamp(0.0, 1.0);
            let cool = profile.cool_edge_tint * edge;
            rgb[0] *= 1.0 - cool * 0.55;
            rgb[1] *= 1.0 - cool * 0.20;
            rgb[2] *= 1.0 + cool * 0.18;
        }

        let mut alpha_out = alpha.powf(profile.alpha_gamma);
        if profile.mottle_strength > 0.0 && width > 0 && height > 0 {
            let x = (index as u32) % width;
            let y = ((index as u32) / width).min(height - 1);
            let edge = (1.0 - alpha).clamp(0.0, 1.0);
            let mottle = static_light_mottle(x, y);
            alpha_out *= 1.0 - profile.mottle_strength * edge * mottle;
            let shade = 1.0 - profile.mottle_strength * 0.35 * edge * mottle;
            for channel in &mut rgb {
                *channel *= shade;
            }
        }

        pixel[0] = (rgb[0].clamp(0.0, 1.0) * 255.0) as u8;
        pixel[1] = (rgb[1].clamp(0.0, 1.0) * 255.0) as u8;
        pixel[2] = (rgb[2].clamp(0.0, 1.0) * 255.0) as u8;
        pixel[3] = (alpha_out.clamp(0.0, 1.0) * 255.0) as u8;
    }
}

fn static_light_mottle(x: u32, y: u32) -> f32 {
    let hash = x
        .wrapping_mul(0x8DA6_B343)
        .wrapping_add(y.wrapping_mul(0xD816_3841))
        .rotate_left(13)
        ^ 0xA3C5_9AC3;
    f32::from(((hash ^ (hash >> 16)) & 0xFF) as u8) / 255.0
}

fn apply_static_light_hue_with_sampler(
    rgba: &mut [u8],
    mut sample_color: impl FnMut(u8) -> [u8; 4],
) {
    for pixel in rgba.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }

        let luma = linear_luma_u8(pixel[0], pixel[1], pixel[2]);
        let color = sample_color(luma);
        pixel[0] = color[0];
        pixel[1] = color[1];
        pixel[2] = color[2];
        pixel[3] = ((u16::from(pixel[3]) * u16::from(color[3])) / 255) as u8;
    }
}

fn sample_hue_lookup(texture_bytes: &[u8], hue_id: u16, luma: u8) -> Option<[u8; 4]> {
    let coord = uocf::enhanced::hues::atlas_coord_for_hue(hue_id)?;
    let x = coord.x + u32::from(luma).min(udd_assets::hues::HUE_STRIP_WIDTH - 1);
    let y = coord.y;
    if x >= udd_assets::hues::HUES_TEXTURE_WIDTH || y >= udd_assets::hues::HUES_TEXTURE_HEIGHT {
        return None;
    }

    let offset = ((y * udd_assets::hues::HUES_TEXTURE_WIDTH + x) * 4) as usize;
    let end = offset.checked_add(4)?;
    let color = texture_bytes.get(offset..end)?;
    Some([color[0], color[1], color[2], color[3]])
}

fn linear_luma_u8(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 54 + u32::from(g) * 183 + u32::from(b) * 19) / 256) as u8
}

fn classicuo_light_shader_id(graphic: u16) -> Option<u16> {
    let mut color = match graphic {
        0x088C => Some(31),
        0x0FAC => Some(30),
        0x0FB1 => Some(60),
        0x1647 => Some(61),
        0x19BB | 0x1F2B => Some(40),
        0x9F66 => Some(0),
        _ => None,
    };

    if (0x09FB..=0x0A14).contains(&graphic) {
        color = Some(30);
    } else if (0x0A15..=0x0A29).contains(&graphic)
        || (0x0B1A..=0x0B1F).contains(&graphic)
        || (0x0B20..=0x0B25).contains(&graphic)
        || (0x0B26..=0x0B28).contains(&graphic)
    {
        color = Some(0);
    } else if (0x0DE1..=0x0DEA).contains(&graphic) {
        color = Some(31);
    } else if (0x1849..=0x1850).contains(&graphic)
        || (0x1853..=0x185A).contains(&graphic)
    {
        color = Some(61);
    } else if (0x197A..=0x19A9).contains(&graphic)
        || (0x19AB..=0x19B6).contains(&graphic)
    {
        color = Some(60);
    } else if (0x1ECD..=0x1ECF).contains(&graphic)
        || (0x1ED0..=0x1ED2).contains(&graphic)
    {
        color = Some(1);
    }

    if graphic == 0x1FD4 || graphic == 0x0F6C {
        color = Some(2);
    }

    if (0x0E2D..=0x0E30).contains(&graphic) {
        color = Some(62);
    } else if (0x0E31..=0x0E33).contains(&graphic) {
        color = Some(40);
    } else if (0x0E5C..=0x0E6A).contains(&graphic) {
        color = Some(6);
    } else if (0x12EE..=0x134D).contains(&graphic)
        || (0x306A..=0x329B).contains(&graphic)
        || (0x343B..=0x346C).contains(&graphic)
        || (0x3547..=0x354C).contains(&graphic)
    {
        color = Some(31);
    } else if (0x3914..=0x3929).contains(&graphic) {
        color = Some(1);
    } else if (0x3946..=0x3964).contains(&graphic)
        || (0x3967..=0x397A).contains(&graphic)
    {
        color = Some(6);
    } else if (0x398C..=0x399F).contains(&graphic) {
        color = Some(31);
    } else if (0x3E02..=0x3E0B).contains(&graphic) {
        color = Some(1);
    } else if (0x3E27..=0x3E3A).contains(&graphic) {
        color = Some(31);
    } else {
        match graphic {
            0x40FE => color = Some(40),
            0x40FF => color = Some(10),
            0x4100 => color = Some(20),
            0x4101 => color = Some(32),
            _ => {
                if (0x983B..=0x983D).contains(&graphic)
                    || (0x983F..=0x9841).contains(&graphic)
                {
                    color = Some(30);
                }
            }
        }
    }

    color
}

fn classicuo_light_shader_data(shader_id: u16) -> ([u8; 3], [usize; 3]) {
    match shader_id {
        1 => ([0x00, 0xFF, 0x00], [0, 1, 0]),
        2 => ([0x7F, 0x7F, 0xFF], [0, 0, 0]),
        6 => ([0xFF, 0x00, 0xFF], [2, 0, 1]),
        10 => ([0x3F, 0x3F, 0xFF], [0, 0, 0]),
        20 => ([0x00, 0xFF, 0x00], [0, 0, 0]),
        30 => ([0xFF, 0x7F, 0x00], [3, 3, 0]),
        31 => ([0xFF, 0x7F, 0x00], [1, 1, 0]),
        32 => ([0xFF, 0x00, 0xFF], [0, 0, 0]),
        40 => ([0xFF, 0x00, 0x00], [0, 0, 0]),
        50 => ([0xFF, 0xFF, 0x00], [0, 0, 0]),
        60 => ([0xFF, 0xFF, 0x00], [1, 1, 0]),
        61 => ([0xFF, 0xFF, 0x00], [4, 4, 0]),
        62 => ([0xFF, 0xFF, 0xFF], [4, 4, 4]),
        63 => ([0xFF, 0xFF, 0xFF], [5, 5, 5]),
        _ => ([0xFF, 0xFF, 0xFF], [0, 0, 0]),
    }
}

fn sample_classicuo_light_shader(shader_id: u16, luma: u8) -> [u8; 4] {
    let index = (luma >> 3).min(31) as usize;
    let (rgb, curves) = classicuo_light_shader_data(shader_id);
    [
        ((u16::from(CLASSICUO_LIGHT_CURVES[curves[0]][index]) * u16::from(rgb[0])) / 31) as u8,
        ((u16::from(CLASSICUO_LIGHT_CURVES[curves[1]][index]) * u16::from(rgb[1])) / 31) as u8,
        ((u16::from(CLASSICUO_LIGHT_CURVES[curves[2]][index]) * u16::from(rgb[2])) / 31) as u8,
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn static_light_key_hash(key: StaticLightKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn static_light_key_hash_uses_all_fields() {
        let base = StaticLightKey {
            map_id: 1,
            tile_x: 100,
            tile_y: 200,
            z: -4,
            graphic: 0x0e31,
            light_id: 40,
            hue_id: 12,
        };
        let base_hash = static_light_key_hash(base);

        for changed in [
            StaticLightKey { map_id: 2, ..base },
            StaticLightKey { tile_x: 101, ..base },
            StaticLightKey { tile_y: 201, ..base },
            StaticLightKey { z: -3, ..base },
            StaticLightKey { graphic: 0x0e32, ..base },
            StaticLightKey { light_id: 41, ..base },
            StaticLightKey { hue_id: 13, ..base },
        ] {
            assert_ne!(base_hash, static_light_key_hash(changed));
        }
    }

    fn test_static_light_instance() -> StaticLightInstance {
        StaticLightInstance {
            key: StaticLightKey {
                map_id: 1,
                tile_x: 10,
                tile_y: 20,
                z: 3,
                graphic: 0x0e31,
                light_id: 40,
                hue_id: 4,
            },
            light_id: 40,
            hue_id: 4,
            color_rgb: [1.0, 0.5, 0.25],
            world_x: 10.5,
            world_z: 20.5,
            world_y: 0.3,
            width_world: 2.0,
            height_world: 3.0,
        }
    }

    #[test]
    fn static_light_sync_signature_tracks_sync_inputs() {
        let instances = RenderStaticLightInstances(vec![test_static_light_instance()]);
        let base = static_light_sync_signature(
            &instances,
            StaticLightRenderStyle::Classic,
            0.5,
            true,
        );
        assert_eq!(
            base,
            static_light_sync_signature(
                &instances,
                StaticLightRenderStyle::Classic,
                0.5,
                true,
            )
        );

        let mut moved_instances = RenderStaticLightInstances(vec![test_static_light_instance()]);
        moved_instances.0[0].world_x += 1.0;

        assert_ne!(
            base,
            static_light_sync_signature(
                &moved_instances,
                StaticLightRenderStyle::Classic,
                0.5,
                true,
            )
        );
        assert_ne!(
            base,
            static_light_sync_signature(
                &instances,
                StaticLightRenderStyle::Enhanced,
                0.5,
                true,
            )
        );
        assert_ne!(
            base,
            static_light_sync_signature(
                &instances,
                StaticLightRenderStyle::Classic,
                0.25,
                true,
            )
        );
        assert_ne!(
            base,
            static_light_sync_signature(
                &instances,
                StaticLightRenderStyle::Classic,
                0.5,
                false,
            )
        );
    }

    #[test]
    fn package_hue_source_samples_packed_hue_row() {
        let mut texture = vec![
            0u8;
            udd_assets::hues::HUES_TEXTURE_WIDTH as usize
                * udd_assets::hues::HUES_TEXTURE_HEIGHT as usize
                * 4
        ];
        let coord = uocf::enhanced::hues::atlas_coord_for_hue(1).expect("hue 1 coord");
        let sample_x = coord.x + 128;
        let offset =
            ((coord.y * udd_assets::hues::HUES_TEXTURE_WIDTH + sample_x) * 4) as usize;
        texture[offset..offset + 4].copy_from_slice(&[10, 20, 30, 128]);
        let mut rgba = vec![128, 128, 128, 200];

        apply_static_light_hue(
            &mut rgba,
            StaticLightHueSource::Package {
                hue_id: 1,
                texture_bytes: &texture,
            },
        );

        assert_eq!(&rgba, &[10, 20, 30, 100]);
    }

    #[test]
    fn stored_rgb_source_keeps_colored_payload() {
        let mut rgba = vec![1, 2, 3, 4];

        apply_static_light_hue(&mut rgba, StaticLightHueSource::StoredRgb);

        assert_eq!(&rgba, &[1, 2, 3, 4]);
    }

    #[test]
    fn classicuo_graphic_light_shader_recolors_mask() {
        let mut rgba = vec![248, 248, 248, 255];

        apply_static_light_hue(&mut rgba, StaticLightHueSource::ClassicLightShader(40));

        assert_eq!(&rgba, &[255, 0, 0, 255]);
        assert_eq!(classicuo_light_shader_id(0x0E31), Some(40));
    }

    #[test]
    fn static_light_alpha_follows_global_lighting_gap() {
        assert_eq!(
            static_light_alpha_for_global_lighting(1.0, StaticLightRenderStyle::Classic),
            0.0
        );
        assert_eq!(
            static_light_alpha_for_global_lighting(0.75, StaticLightRenderStyle::Classic),
            0.25
        );
        assert_eq!(
            static_light_alpha_for_global_lighting(0.0, StaticLightRenderStyle::Classic),
            STATIC_LIGHT_ALPHA
        );
    }

    #[test]
    fn static_light_alpha_differs_by_render_style() {
        let classic = static_light_alpha_for_global_lighting(1.0, StaticLightRenderStyle::Classic);
        let enhanced = static_light_alpha_for_global_lighting(1.0, StaticLightRenderStyle::Enhanced);
        let kr_mid = static_light_alpha_for_global_lighting(0.75, StaticLightRenderStyle::Kr);

        assert_eq!(classic, 0.0);
        assert!(enhanced > classic);
        assert!(
            kr_mid
                < static_light_alpha_for_global_lighting(0.75, StaticLightRenderStyle::Classic)
        );
    }

    #[test]
    fn decal_visibility_gates_visible_static_light_alpha() {
        assert_eq!(
            static_light_decal_alpha(0.0, 0.0, StaticLightRenderStyle::Kr),
            0.0
        );
        assert!(
            static_light_decal_alpha(0.0, 0.5, StaticLightRenderStyle::Kr)
                < static_light_decal_alpha(0.0, 1.0, StaticLightRenderStyle::Kr)
        );
    }

    #[test]
    fn response_color_uses_classicuo_light_shader_when_known() {
        assert_eq!(static_light_response_color(0x0E31), [1.0, 0.0, 0.0]);
        assert_eq!(static_light_response_color(0x1234), [1.0, 0.72, 0.42]);
    }

    #[test]
    fn sampled_mask_response_color_uses_colored_alpha_weighted_pixels() {
        let rgba = vec![
            255, 64, 0, 255,
            0, 255, 0, 0,
            192, 32, 0, 128,
        ];

        let color = sampled_static_light_mask_color(&rgba).expect("colored mask response");

        assert_eq!(color[0], 1.0);
        assert!(color[1] > 0.17 && color[1] < 0.24);
        assert_eq!(color[2], 0.0);
    }

    #[test]
    fn sampled_mask_response_color_rejects_grayscale_masks() {
        let rgba = vec![
            220, 220, 220, 255,
            64, 64, 64, 128,
        ];

        assert_eq!(sampled_static_light_mask_color(&rgba), None);
    }

    #[test]
    fn package_hue_response_color_normalizes_sampled_hue() {
        let color = normalized_response_color_from_rgba([64, 128, 32, 255])
            .expect("normalized hue response");

        assert_eq!(color[0], 0.5);
        assert_eq!(color[1], 1.0);
        assert_eq!(color[2], 0.25);
    }

    #[test]
    fn package_hue_response_color_rejects_black_sample() {
        assert_eq!(normalized_response_color_from_rgba([0, 0, 0, 255]), None);
    }

    #[test]
    fn kr_style_desaturates_and_tightens_light_mask() {
        let mut rgba = vec![255, 96, 0, 128];

        apply_static_light_style(&mut rgba, 1, 1, StaticLightRenderStyle::Kr);

        assert!(rgba[0] < 255);
        assert!(rgba[0] - rgba[1] < 255 - 96);
        assert!(rgba[3] < 128);
    }
}
