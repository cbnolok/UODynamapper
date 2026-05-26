use super::statics_collect::StaticChunkBatchKey;
use crate::configs::settings::{HueSourcePreference, Settings};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world::land::{
    CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM,
};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{
    ClassicHuesRes, HuesPackageRes, TileMetaPackageRes, WorldLightsPackageRes,
};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat};
use std::collections::{HashMap, HashSet};
use uocf::classic::hues::HueEntry;

const TILE_FLAG_LIGHT_SOURCE: u64 = 0x00800000;
const LIGHT_PIXELS_PER_WORLD_TILE: f32 = 44.0;
const STATIC_LIGHT_Y_BIAS: f32 = 0.012;
const STATIC_LIGHT_ALPHA: f32 = 0.65;
const STATIC_LIGHT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, STATIC_LIGHT_ALPHA);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StaticLightHueSourceKind {
    StoredRgb,
    Classic,
    Enhanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StaticLightMaterialKey {
    light_id: u32,
    hue_id: u16,
    hue_source: StaticLightHueSourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticLightInstance {
    pub key: StaticLightKey,
    pub light_id: u32,
    pub hue_id: u16,
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
    pub hue_id: u16,
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
    enhanced_hue_texture_bytes: Option<Vec<u8>>,
    enhanced_hue_texture_failed: bool,
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
    classic_hues_res: Option<Res<ClassicHuesRes>>,
    hues_package_res: Option<Res<HuesPackageRes>>,
    settings: Res<Settings>,
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
            instance.hue_id,
            settings.graphics.hue_source,
            &world_lights.0,
            classic_hues_res.as_deref(),
            hues_package_res.as_deref(),
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
    hue_id: u16,
    hue_preference: HueSourcePreference,
    world_lights: &udd_assets::world_lights::WorldLightsPackage,
    classic_hues: Option<&ClassicHuesRes>,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let hue_source_kind = resolve_static_light_hue_source_kind(
        hue_id,
        hue_preference,
        classic_hues,
        hues_package,
        material_cache,
    );
    let material_key = StaticLightMaterialKey {
        light_id,
        hue_id: if hue_source_kind == StaticLightHueSourceKind::StoredRgb {
            0
        } else {
            hue_id
        },
        hue_source: hue_source_kind,
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
            hue_id,
            hue_preference,
            classic_hues,
            hues_package,
            material_cache,
        );
        apply_static_light_hue(&mut rgba, hue_source);
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
        .materials_by_key
        .insert(material_key, material_handle.clone());
    Some(material_handle)
}

fn resolve_static_light_hue_source_kind(
    hue_id: u16,
    preference: HueSourcePreference,
    classic_hues: Option<&ClassicHuesRes>,
    hues_package: Option<&HuesPackageRes>,
    material_cache: &mut StaticLightMaterialCache,
) -> StaticLightHueSourceKind {
    resolve_static_light_hue_source(
        hue_id,
        preference,
        classic_hues,
        hues_package,
        material_cache,
    )
    .kind()
}

#[derive(Clone, Copy)]
enum StaticLightHueSource<'a> {
    StoredRgb,
    Classic(&'a HueEntry),
    Enhanced {
        hue_id: u16,
        texture_bytes: &'a [u8],
    },
}

impl StaticLightHueSource<'_> {
    const fn kind(self) -> StaticLightHueSourceKind {
        match self {
            Self::StoredRgb => StaticLightHueSourceKind::StoredRgb,
            Self::Classic(_) => StaticLightHueSourceKind::Classic,
            Self::Enhanced { .. } => StaticLightHueSourceKind::Enhanced,
        }
    }
}

fn resolve_static_light_hue_source<'a>(
    hue_id: u16,
    preference: HueSourcePreference,
    classic_hues: Option<&'a ClassicHuesRes>,
    hues_package: Option<&'a HuesPackageRes>,
    material_cache: &'a mut StaticLightMaterialCache,
) -> StaticLightHueSource<'a> {
    if hue_id == 0 {
        return StaticLightHueSource::StoredRgb;
    }

    match preference {
        HueSourcePreference::Cc => classic_hue_source(hue_id, classic_hues),
        HueSourcePreference::Ec => enhanced_hue_source(hue_id, hues_package, material_cache),
        HueSourcePreference::Auto => enhanced_hue_source(hue_id, hues_package, material_cache)
            .or_else(|| classic_hue_source(hue_id, classic_hues)),
    }
    .unwrap_or(StaticLightHueSource::StoredRgb)
}

fn classic_hue_source<'a>(
    hue_id: u16,
    classic_hues: Option<&'a ClassicHuesRes>,
) -> Option<StaticLightHueSource<'a>> {
    classic_hues
        .and_then(|hues| hues.0.get(hue_id.saturating_sub(1) as usize))
        .map(StaticLightHueSource::Classic)
}

fn enhanced_hue_source<'a>(
    hue_id: u16,
    hues_package: Option<&'a HuesPackageRes>,
    material_cache: &'a mut StaticLightMaterialCache,
) -> Option<StaticLightHueSource<'a>> {
    let package = hues_package?;
    package.0.texture_coord_for_hue(hue_id)?;
    if material_cache.enhanced_hue_texture_bytes.is_none()
        && !material_cache.enhanced_hue_texture_failed
    {
        match package.0.read_texture_bytes() {
            Ok(bytes) => {
                material_cache.enhanced_hue_texture_bytes = Some(bytes);
            }
            Err(error) => {
                material_cache.enhanced_hue_texture_failed = true;
                log::warn!("Failed to read hues.uddp texture for static light hues: {error}");
            }
        }
    }

    material_cache
        .enhanced_hue_texture_bytes
        .as_deref()
        .map(|texture_bytes| StaticLightHueSource::Enhanced {
            hue_id,
            texture_bytes,
        })
}

fn apply_static_light_hue(rgba: &mut [u8], source: StaticLightHueSource<'_>) {
    match source {
        StaticLightHueSource::StoredRgb => {}
        StaticLightHueSource::Classic(hue) => {
            apply_static_light_hue_with_sampler(rgba, |luma| {
                let index = (luma >> 3).min(31) as usize;
                argb1555_to_rgba8888(hue.color_table[index])
            });
        }
        StaticLightHueSource::Enhanced {
            hue_id,
            texture_bytes,
        } => {
            apply_static_light_hue_with_sampler(rgba, |luma| {
                sample_enhanced_hue(texture_bytes, hue_id, luma).unwrap_or([luma, luma, luma, 255])
            });
        }
    }
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

fn sample_enhanced_hue(texture_bytes: &[u8], hue_id: u16, luma: u8) -> Option<[u8; 4]> {
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

fn argb1555_to_rgba8888(color16: u16) -> [u8; 4] {
    let r5 = ((color16 >> 10) & 0x1F) as u32;
    let g5 = ((color16 >> 5) & 0x1F) as u32;
    let b5 = (color16 & 0x1F) as u32;
    [
        ((r5 * 255) / 31) as u8,
        ((g5 * 255) / 31) as u8,
        ((b5 * 255) / 31) as u8,
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue_entry_with_index_color(index: usize, color: u16) -> HueEntry {
        let mut color_table = [0u16; 32];
        color_table[index] = color;
        HueEntry {
            id: 1,
            color_table,
            table_start: 0,
            table_end: 31,
            name: [0; 20],
        }
    }

    #[test]
    fn classic_hue_source_recolors_light_by_luma() {
        let hue = hue_entry_with_index_color(15, 0x7C00);
        let mut rgba = vec![120, 120, 120, 200];

        apply_static_light_hue(&mut rgba, StaticLightHueSource::Classic(&hue));

        assert_eq!(&rgba, &[255, 0, 0, 200]);
    }

    #[test]
    fn enhanced_hue_source_samples_packed_hue_row() {
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
            StaticLightHueSource::Enhanced {
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
}
