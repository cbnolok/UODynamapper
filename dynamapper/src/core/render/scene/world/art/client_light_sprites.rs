//! The Enhanced and KR clients' light sprites, drawn by KR-like and Enhanced into the client night composite's light
//! buffer (`effects::client_night`).
//!
//! A static draws a sprite when its tileart names a light texture (tilemeta `item_lights`); the LightSource flag plays
//! no part. The texture is a `TerrainTexture.uop` entry packed in `tex_aux_ec.uddp`, drawn with its own colour and
//! alpha: no hue, even on a hued item. The quad is the EC static billboard of the clients' sprite builder: anchored
//! to the static's map position (never to its art, so CC and EC art place it alike), sized by the tileart light
//! scale, and moved by the light offsets in world units along the camera's right and up. Classic and UOD-Painterly
//! draw the light.mul decals of `static_lights` instead.
//!
//! Two settings depart from the clients in the two styles. The style's `local_lights.centre_on_items` centres the
//! sprites the tileart's default offsets throw far from their item on the item's cell
//! (`client_light_sprite_centred`), and `configs::light_scale` (LightScale.kdl) enlarges chosen items' sprites.
//!
//! The clients light the terrain and the statics through the composite alone: their sprites are unlit and their
//! terrain has no other light. The optional land light decals and static local lights (Light/Static Interaction,
//! off in the KR-like and Enhanced presets) read these sprites in the two styles (`client_light_sprite_static_light`),
//! unless the style's `local_lights.source` picks light.mul, as Classic (`static_lights`).

use super::light_cover::static_light_is_hidden;
use crate::core::render::scene::world::z_view::ZViewLimit;
use super::static_lights::{StaticLightInstance, StaticLightKey, StaticLightTarget, LIGHT_PIXELS_PER_WORLD_TILE,
    STATIC_LIGHT_Y_BIAS};
use super::statics_collect::{
    apply_static_world_anchor_translation, resolve_static_billboard_bounds, StaticBillboardBounds,
    StaticChunkBatchKey, CC_WORLD_XZ_PER_PIXEL, CC_WORLD_Y_PER_PIXEL, STATIC_BILLBOARD_RIGHT_XZ,
    STATIC_WORLD_Y_PER_XZ_PIXEL,
};
use crate::core::render::scene::camera::PlayerCamera;
use crate::configs::{light_scale::ItemLightScales, settings::Settings, shader_presets::UniformState};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::world::effects::client_night::LIGHT_BUFFER_LAYER;
use crate::core::render::scene::world::land::{LCMesh, CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{TexAuxEcPackageRes, TileMetaPackageRes};
use crate::util_lib::uo_coords::scale_uo_z_to_bevy_units;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat};
use std::collections::{BTreeSet, HashMap, HashSet};
use udd_assets::tex_art_ec::LEGACY_PIXEL_UNIT;
use udd_assets::tex_aux_ec::tex_aux_ec_terrain_texture_key;
use udd_assets::tilemeta::TileMetaItemLight;

/// The sprite's horizontal anchor: its left edge sits `CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS - W/2` texture pixels right
/// of the base point (W/2 an integer shift), so its centre sits 22 scaled pixels right, on a 44-pixel tile's centre.
const CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS: i32 = 22;
/// The sprite's vertical anchor: its bottom edge sits at the base point.
const CLIENT_LIGHT_SPRITE_ANCHOR_Y_PIXELS: i16 = 0;
/// How far, in tiles, the ground point under a sprite's glow may lie from its item's cell centre for
/// `centre_on_items` to keep the clients' placement. The offsets the tileart gives an item of its own keep the glow
/// within 4.04 tiles of it (window beams, wall torches and sconces, fireplace fires, candles, lanterns); the tileart's
/// default offsets, (-5, -4) on 378 items and (0, 0) at scale 2 on 7, and two candelabras' (-4.7, -2) throw it 4.78
/// tiles or more away, with no light in between.
pub(super) const CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH: f32 = 4.5;
/// From a tile's corner to its centre, along each map axis.
const TILE_CENTRE_OFFSET: f32 = 0.5;
/// The largest 8-bit channel value, which maps a light texture's channel to 0..1.
const LIGHT_TEXTURE_CHANNEL_MAX: f32 = 255.0;
/// The bytes of one RGBA8888 texel.
const RGBA_TEXEL_BYTES: usize = 4;

/// One static's light sprite. The key holds everything its placement and texture follow from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Component)]
#[component(clone_behavior = Ignore)]
pub struct ClientLightSpriteKey {
    pub map_id: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub z: i8,
    pub graphic: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientLightSpriteInstance {
    pub key: ClientLightSpriteKey,
    /// The `TerrainTexture.uop` entry of the light texture.
    pub texture_entry: u32,
    /// The quad's centre and its size along the billboard's right and world Y.
    pub center: Vec3,
    pub size: Vec2,
    /// The light texture's colour where it shows, the mean of its texels weighted by alpha (`light_texture_glow_rgb`),
    /// which the land light decals and the static local lights take as the light's colour.
    pub glow_rgb: [f32; 3],
}

/// The light sprites of the statics in and around the visible chunks.
#[derive(Resource, Default)]
pub struct RenderClientLightSprites {
    pub instances: Vec<ClientLightSpriteInstance>,
    /// What the last scan covered, whether it centred the sprites on their items and the z limit it hid covered
    /// lights under (`light_cover`); the scan reruns only when it changes.
    last_scan: Option<(u32, Vec<StaticChunkBatchKey>, bool, i32)>,
}

/// The sprites' materials, one per light texture (`None` once a texture failed to load), and the scan reach.
#[derive(Resource, Default)]
pub struct ClientLightSpriteCache {
    materials_by_entry: HashMap<u32, Option<Handle<StandardMaterial>>>,
    /// Each light texture's glow colour, `None` once the texture failed to load.
    glow_rgb_by_entry: HashMap<u32, Option<[f32; 3]>>,
    mesh: Option<Handle<Mesh>>,
    reach_blocks: Option<u32>,
    warned_entries: HashSet<u32>,
    warned_no_package: bool,
}

#[derive(Component)]
pub struct ClientLightSpriteEntity;

/// The local bounds of a light sprite around the static's base point, along the billboard's right (x) and world Y
/// (y): the clients' sprite window of the `width` x `height` light texture at the light's scale, then its offsets.
///
/// The pixel ratio `scale / LEGACY_PIXEL_UNIT` gives the client's K * scale per texture pixel (K = sqrt(2) / 66). The
/// offsets are world units along the camera's right and up, applied after the camera rotation; one camera-up unit is
/// `STATIC_WORLD_Y_PER_XZ_PIXEL` of world Y, as for the static sprites.
pub(super) fn client_light_sprite_bounds(light: &TileMetaItemLight, width: u32, height: u32) -> StaticBillboardBounds {
    let scale = light.scale;
    let anchor_x = CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS - (width / 2) as i32;
    let scaled_width = (width as f32 * scale).floor();
    let scaled_height = (height as f32 * scale).floor();
    let window = resolve_static_billboard_bounds(
        scale / LEGACY_PIXEL_UNIT,
        anchor_x as i16,
        CLIENT_LIGHT_SPRITE_ANCHOR_Y_PIXELS,
        scaled_width / scale,
        scaled_height / scale,
    );
    let offset_y = light.offset_y * STATIC_WORLD_Y_PER_XZ_PIXEL;
    StaticBillboardBounds {
        local_min_x: window.local_min_x + light.offset_x,
        local_max_x: window.local_max_x + light.offset_x,
        local_min_y: window.local_min_y + offset_y,
        local_max_y: window.local_max_y + offset_y,
    }
}

/// The static's base point: the EC static billboard's anchor (the client's (x + 0.5, y + 1.5)) at its z.
fn client_light_sprite_origin(tile_x: u32, tile_y: u32, z: i8) -> Vec3 {
    let (world_x, world_z) = apply_static_world_anchor_translation(tile_x as f32, tile_y as f32);
    Vec3::new(world_x, scale_uo_z_to_bevy_units(f32::from(z)), world_z)
}

fn billboard_right() -> Vec3 {
    Vec3::new(STATIC_BILLBOARD_RIGHT_XZ.x, 0.0, STATIC_BILLBOARD_RIGHT_XZ.y)
}

/// The quad's centre and size for the bounds around `origin`.
fn client_light_sprite_quad(origin: Vec3, bounds: StaticBillboardBounds) -> (Vec3, Vec2) {
    let center_x = (bounds.local_min_x + bounds.local_max_x) * 0.5;
    let center_y = (bounds.local_min_y + bounds.local_max_y) * 0.5;
    let center = origin + billboard_right() * center_x + Vec3::Y * center_y;
    let size = Vec2::new(bounds.local_max_x - bounds.local_min_x, bounds.local_max_y - bounds.local_min_y);
    (center, size)
}

/// The item's cell centre at its base height.
fn client_light_sprite_cell_centre(tile_x: u32, tile_y: u32, z: i8) -> Vec3 {
    Vec3::new(
        tile_x as f32 + TILE_CENTRE_OFFSET,
        scale_uo_z_to_bevy_units(f32::from(z)),
        tile_y as f32 + TILE_CENTRE_OFFSET,
    )
}

/// The point at height `base_y` that shows where `point` shows: `point` moved along the camera's view ray.
fn ground_under(point: Vec3, base_y: f32) -> Vec3 {
    let view_ray = PlayerCamera::BASE_OFFSET_FROM_PLAYER;
    point - view_ray * ((point.y - base_y) / view_ray.y)
}

/// The sprite centre `center` as `centre_on_items` places it: moved across the ground so that it shows over its
/// item's cell centre, when the clients' placement shows it more than `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH` tiles
/// away; as it is otherwise. The move is horizontal, so the glow keeps its height above the item, and the light on
/// the ground under it (`client_light_sprite_static_light`) lands on the cell centre.
pub(super) fn client_light_sprite_centred(center: Vec3, key: ClientLightSpriteKey) -> Vec3 {
    let cell = client_light_sprite_cell_centre(key.tile_x, key.tile_y, key.z);
    let ground = ground_under(center, cell.y);
    let shift = Vec3::new(cell.x - ground.x, 0.0, cell.z - ground.z);
    if shift.length() > CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH {
        center + shift
    } else {
        center
    }
}

/// A light texture's colour where it shows: the mean of its RGBA8888 texels' colour weighted by their alpha. `None`
/// for a texture without a visible texel.
fn light_texture_glow_rgb(rgba: &[u8]) -> Option<[f32; 3]> {
    let mut weighted = [0.0f64; 3];
    let mut coverage = 0.0f64;
    for texel in rgba.chunks_exact(RGBA_TEXEL_BYTES) {
        let alpha = f64::from(texel[3]);
        for (sum, channel) in weighted.iter_mut().zip(&texel[..3]) {
            *sum += f64::from(*channel) * alpha;
        }
        coverage += alpha;
    }
    (coverage > 0.0).then(|| weighted.map(|sum| (sum / coverage) as f32 / LIGHT_TEXTURE_CHANNEL_MAX))
}

/// The light that the land light decals and the static local lights take from a client light sprite.
///
/// - **Where:** the ground point that shows at the sprite's centre: the centre moved along the camera's view ray down
///   to the static's base height, so the land lights where the glow shows. The light sits at that height, as the
///   light.mul lights sit at their static's z.
/// - **Size:** the sprite's size in legacy pixels over `LIGHT_PIXELS_PER_WORLD_TILE`, as the light.mul lights give
///   their mask's.
/// - **Colour:** the texture's glow colour, with no hue, as the sprite draws.
/// - **Key:** the light id is the light texture's `TerrainTexture.uop` entry.
pub(super) fn client_light_sprite_static_light(sprite: &ClientLightSpriteInstance) -> StaticLightInstance {
    let key = sprite.key;
    let base_y = client_light_sprite_origin(key.tile_x, key.tile_y, key.z).y;
    let ground = ground_under(sprite.center, base_y);
    StaticLightInstance {
        key: StaticLightKey {
            map_id: key.map_id,
            tile_x: key.tile_x,
            tile_y: key.tile_y,
            z: key.z,
            graphic: key.graphic,
            light_id: sprite.texture_entry,
            hue_id: 0,
        },
        light_id: sprite.texture_entry,
        hue_id: 0,
        color_rgb: sprite.glow_rgb,
        world_x: ground.x,
        world_z: ground.z,
        world_y: base_y + STATIC_LIGHT_Y_BIAS,
        width_world: sprite.size.x / CC_WORLD_XZ_PER_PIXEL / LIGHT_PIXELS_PER_WORLD_TILE,
        height_world: sprite.size.y / CC_WORLD_Y_PER_PIXEL / LIGHT_PIXELS_PER_WORLD_TILE,
    }
}

/// How far a sprite reaches from its base point, in world units: the largest absolute bound.
fn client_light_sprite_reach(bounds: StaticBillboardBounds) -> f32 {
    [bounds.local_min_x, bounds.local_max_x, bounds.local_min_y, bounds.local_max_y]
        .into_iter()
        .map(f32::abs)
        .fold(0.0, f32::max)
}

/// A light whose scale gives no sprite.
fn has_drawable_scale(light: &TileMetaItemLight) -> bool {
    light.scale.is_finite() && light.scale > 0.0
}

/// The scan margin in map blocks around the visible chunks: the largest reach of a loaded light, enlarged by the
/// largest item light scale, so no sprite whose static lies off screen is missing. A world unit along the billboard's
/// right or world Y moves the sprite at most one tile along each map axis on screen. Centring only brings sprites
/// nearer their items.
fn client_light_sprite_reach_blocks(
    tilemeta: &TileMetaPackageRes,
    tex_aux: &TexAuxEcPackageRes,
    light_scales: &ItemLightScales,
) -> u32 {
    let reach = tilemeta
        .0
        .item_lights()
        .iter()
        .filter(|light| has_drawable_scale(light))
        .filter_map(|light| {
            let slot = tex_aux.0.slot_for_texture_id(tex_aux_ec_terrain_texture_key(light.light_texture_entry))?;
            Some(client_light_sprite_reach(client_light_sprite_bounds(light, u32::from(slot.width), u32::from(slot.height))))
        })
        .fold(0.0, f32::max);
    (reach * light_scales.largest() / MAP_STORAGE_BLOCK_TILE_DIM as f32).ceil() as u32
}

pub fn sys_collect_client_light_sprites(
    statics_res: Res<StaticsStoreRes>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    tex_aux_res: Option<Res<TexAuxEcPackageRes>>,
    settings: Res<Settings>,
    uniform_state: Res<UniformState>,
    light_scales: Res<ItemLightScales>,
    scene_state: Res<SceneStateData>,
    chunks_q: Query<&LCMesh>,
    z_view: Res<ZViewLimit>,
    mut cache: ResMut<ClientLightSpriteCache>,
    mut output: ResMut<RenderClientLightSprites>,
) {
    let drawn = settings.world_rendering.enable_static_lights
        && StaticLightTarget::for_uniform_state(&uniform_state) == StaticLightTarget::ClientLightBuffer;
    let (Some(tilemeta), true) = (tilemeta_res, drawn) else {
        if !output.instances.is_empty() || output.last_scan.is_some() {
            output.instances.clear();
            output.last_scan = None;
        }
        return;
    };
    let Some(tex_aux) = tex_aux_res else {
        if !cache.warned_no_package {
            log::warn!("KR-like and Enhanced draw no light sprites: there is no tex_aux_ec.uddp, which holds them");
            cache.warned_no_package = true;
        }
        return;
    };

    let map_id = scene_state.map_id;
    let mut visible_chunk_keys = chunks_q
        .iter()
        .filter(|chunk| chunk.parent_map_id == map_id)
        .map(|chunk| StaticChunkBatchKey { map_id, gx: chunk.gx, gy: chunk.gy, scale: chunk.scale })
        .collect::<Vec<_>>();
    visible_chunk_keys.sort_by_key(|key| (key.gy, key.gx, key.scale));
    let centre_on_items = uniform_state.local_lights.is_some_and(|local_lights| local_lights.centre_on_items);
    let max_z = z_view.0.max_z;
    if output.last_scan.as_ref().is_some_and(|(last_map, last_keys, last_centred, last_max_z)| {
        *last_map == map_id && *last_keys == visible_chunk_keys && *last_centred == centre_on_items
            && *last_max_z == max_z
    }) {
        return;
    }

    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|store| store.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();
    let reach_blocks = *cache
        .reach_blocks
        .get_or_insert_with(|| client_light_sprite_reach_blocks(&tilemeta, &tex_aux, &light_scales));

    let mut blocks = BTreeSet::new();
    for chunk_key in &visible_chunk_keys {
        let start_gx = (chunk_key.gx * CHUNK_STORAGE_BLOCKS_DIM).saturating_sub(reach_blocks);
        let start_gy = (chunk_key.gy * CHUNK_STORAGE_BLOCKS_DIM).saturating_sub(reach_blocks);
        let end_gx = (chunk_key.gx + chunk_key.scale) * CHUNK_STORAGE_BLOCKS_DIM + reach_blocks;
        let end_gy = (chunk_key.gy + chunk_key.scale) * CHUNK_STORAGE_BLOCKS_DIM + reach_blocks;
        for gy in start_gy..end_gy {
            for gx in start_gx..end_gx {
                blocks.insert((gy, gx));
            }
        }
    }

    let mut instances = Vec::new();
    let mut seen = HashSet::new();
    for (gy, gx) in blocks {
        let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
            continue;
        };
        // Copied out of the store, which `static_light_is_hidden` reads again for the cover cells.
        let lit_tiles = tiles
            .iter()
            .copied()
            .filter(|tile| tilemeta.0.item_light(u32::from(tile.graphic)).is_some())
            .collect::<Vec<_>>();
        for tile in lit_tiles {
            let Some(light) = tilemeta.0.item_light(u32::from(tile.graphic)) else {
                continue;
            };
            if !has_drawable_scale(light) {
                continue;
            }
            let entry = light.light_texture_entry;
            let Some(slot) = tex_aux.0.slot_for_texture_id(tex_aux_ec_terrain_texture_key(entry)) else {
                if cache.warned_entries.insert(entry) {
                    let graphic = tile.graphic;
                    log::warn!("tex_aux_ec.uddp has no light texture {entry} (item {graphic}): repack it");
                }
                continue;
            };
            let key = ClientLightSpriteKey {
                map_id,
                tile_x: gx * MAP_STORAGE_BLOCK_TILE_DIM + tile.x_offset() as u32,
                tile_y: gy * MAP_STORAGE_BLOCK_TILE_DIM + tile.y_offset() as u32,
                z: tile.z,
                graphic: tile.graphic,
            };
            if !seen.insert(key)
                || static_light_is_hidden(&mut statics_store, &tilemeta.0, key.tile_x, key.tile_y, key.z, max_z)
            {
                continue;
            }
            let Some(glow_rgb) = *cache.glow_rgb_by_entry.entry(entry).or_insert_with(|| {
                let (_, _, rgba) = tex_aux
                    .0
                    .load_terrain_texture_rgba(entry)
                    .inspect_err(|error| log::warn!("Cannot read light texture {entry} from tex_aux_ec.uddp: {error:#}"))
                    .ok()?;
                light_texture_glow_rgb(&rgba)
            }) else {
                continue;
            };
            let bounds = client_light_sprite_bounds(light, u32::from(slot.width), u32::from(slot.height));
            let (center, size) =
                client_light_sprite_quad(client_light_sprite_origin(key.tile_x, key.tile_y, key.z), bounds);
            let center = if centre_on_items { client_light_sprite_centred(center, key) } else { center };
            let size = size * light_scales.scale(key.graphic);
            instances.push(ClientLightSpriteInstance { key, texture_entry: entry, center, size, glow_rgb });
        }
    }

    if instances.len() != output.instances.len() {
        console_logger::one(
            LogSev::Debug,
            LogAbout::RenderWorldArt,
            &format!("client light sprites: instances={} reach_blocks={reach_blocks}", instances.len()),
        );
    }
    output.instances = instances;
    output.last_scan = Some((map_id, visible_chunk_keys, centre_on_items, max_z));
}

pub fn sys_sync_client_light_sprites(
    mut commands: Commands,
    sprites: Res<RenderClientLightSprites>,
    tex_aux_res: Option<Res<TexAuxEcPackageRes>>,
    mut cache: ResMut<ClientLightSpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut existing_q: Query<(Entity, &ClientLightSpriteKey, &mut Transform), With<ClientLightSpriteEntity>>,
) {
    if !sprites.is_changed() {
        return;
    }
    let desired = sprites.instances.iter().map(|instance| (instance.key, instance)).collect::<HashMap<_, _>>();
    let mut existing_keys = HashSet::new();
    for (entity, key, mut transform) in existing_q.iter_mut() {
        if let Some(instance) = desired.get(key) {
            // A kept sprite may have moved or grown: Local Lights' centring changed.
            transform.translation = instance.center;
            transform.scale = instance.size.extend(1.0);
            existing_keys.insert(*key);
        } else {
            commands.entity(entity).despawn();
        }
    }
    let Some(tex_aux) = tex_aux_res else {
        return;
    };

    let cache = &mut *cache;
    let mesh = cache.mesh.get_or_insert_with(|| meshes.add(build_client_light_sprite_mesh())).clone();
    let rotation = client_light_sprite_rotation();
    for instance in sprites.instances.iter().filter(|instance| !existing_keys.contains(&instance.key)) {
        let material = cache
            .materials_by_entry
            .entry(instance.texture_entry)
            .or_insert_with(|| client_light_sprite_material(&tex_aux, instance.texture_entry, &mut images, &mut materials));
        let Some(material) = material.clone() else {
            continue;
        };
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(instance.center)
                .with_rotation(rotation)
                .with_scale(instance.size.extend(1.0)),
            RenderLayers::layer(LIGHT_BUFFER_LAYER),
            NoFrustumCulling,
            instance.key,
            ClientLightSpriteEntity,
        ));
    }
}

/// The light texture as the light buffer takes it: its 8-bit values unchanged (not sRGB), so the buffer blends them
/// as the client blends its sprites (SRCALPHA, INVSRCALPHA) and the composite reads them back. Linear filtering and
/// clamped addressing, no mips, as the client's light map.
fn client_light_sprite_material(
    tex_aux: &TexAuxEcPackageRes,
    entry: u32,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let (width, height, rgba) = tex_aux
        .0
        .load_terrain_texture_rgba(entry)
        .inspect_err(|error| log::warn!("Cannot read light texture {entry} from tex_aux_ec.uddp: {error:#}"))
        .ok()?;
    let mut image = Image::new(
        Extent3d { width: u32::from(width), height: u32::from(height), depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Some(materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(image)),
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        unlit: true,
        fog_enabled: false,
        ..default()
    }))
}

/// A unit quad centred on the origin, texture row 0 at the top (+y): the sprite stands upright, as the EC statics.
fn build_client_light_sprite_mesh() -> Mesh {
    use bevy::mesh::Indices;

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [-0.5, 0.5, 0.0], [0.5, 0.5, 0.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 1, 3, 2]));
    mesh
}

/// The quad's local x along the billboard's right, local y along world Y, facing the camera.
fn client_light_sprite_rotation() -> Quat {
    let right = billboard_right();
    Quat::from_mat3(&Mat3::from_cols(right, Vec3::Y, right.cross(Vec3::Y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-3;
    /// Lantern 2594: light_generic, TerrainTexture.uop entry 1, 256 x 256, at scale 2 with offsets (0, -4.5).
    const LANTERN_LIGHT: TileMetaItemLight =
        TileMetaItemLight { tile_id: 2594, light_texture_entry: 1, scale: 2.0, offset_x: 0.0, offset_y: -4.5 };
    const LIGHT_GENERIC_SIDE: u32 = 256;

    #[test]
    fn the_lantern_light_has_the_clients_bounds() {
        let bounds = client_light_sprite_bounds(&LANTERN_LIGHT, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        assert!((bounds.local_min_x - -4.543).abs() < EPSILON, "{bounds:?}");
        assert!((bounds.local_max_x - 6.428).abs() < EPSILON, "{bounds:?}");
        assert!((bounds.local_min_y - -3.182).abs() < EPSILON, "{bounds:?}");
        assert!((bounds.local_max_y - 4.576).abs() < EPSILON, "{bounds:?}");
        // The centre: 0.697 world Y above the base point, 0.943 along the billboard's right.
        let (center, size) = client_light_sprite_quad(Vec3::ZERO, bounds);
        assert!((center.y - 0.697).abs() < EPSILON, "{center}");
        assert!((center.x - 0.943 * std::f32::consts::FRAC_1_SQRT_2).abs() < EPSILON, "{center}");
        assert!((center.z + 0.943 * std::f32::consts::FRAC_1_SQRT_2).abs() < EPSILON, "{center}");
        assert!((size.x - 10.971).abs() < EPSILON, "{size}");
    }

    #[test]
    fn the_offsets_move_the_sprite_in_world_units_whatever_the_scale() {
        let unmoved = TileMetaItemLight { offset_x: 0.0, offset_y: 0.0, scale: 1.5, ..LANTERN_LIGHT };
        let moved = TileMetaItemLight { offset_x: -5.0, offset_y: -4.0, ..unmoved };
        let a = client_light_sprite_bounds(&unmoved, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        let b = client_light_sprite_bounds(&moved, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        assert!((b.local_min_x - a.local_min_x - -5.0).abs() < EPSILON);
        assert!((b.local_max_y - a.local_max_y - -4.0 * std::f32::consts::FRAC_1_SQRT_2).abs() < EPSILON);
        // At scale 1.5 the sprite's centre sits on the tile's centre column: 22 pixels at K * 1.5 = sqrt(2) / 2.
        let center_x = (a.local_min_x + a.local_max_x) * 0.5;
        assert!((center_x - std::f32::consts::FRAC_1_SQRT_2).abs() < EPSILON, "{center_x}");
        assert_eq!(a.local_min_y, 0.0);
    }

    #[test]
    fn the_base_point_is_the_ec_static_anchor() {
        let origin = client_light_sprite_origin(2507, 560, 12);
        assert_eq!(origin.x, 2507.5);
        assert_eq!(origin.z, 561.5);
        assert_eq!(origin.y, scale_uo_z_to_bevy_units(12.0));
    }

    #[test]
    fn the_quad_faces_the_camera_upright() {
        let rotation = client_light_sprite_rotation();
        assert!((rotation * Vec3::X).abs_diff_eq(billboard_right(), EPSILON));
        assert!((rotation * Vec3::Y).abs_diff_eq(Vec3::Y, EPSILON));
        let toward_camera = Vec3::new(1.0, 0.0, 1.0).normalize();
        assert!((rotation * Vec3::Z).abs_diff_eq(toward_camera, EPSILON));
    }

    #[test]
    fn the_glow_colour_is_the_alpha_weighted_mean() {
        // An opaque red texel, a half-covered blue one and a clear green one, which counts for nothing.
        let rgba = [255, 0, 0, 255, 0, 0, 255, 128, 0, 255, 0, 0];
        let [red, green, blue] = light_texture_glow_rgb(&rgba).expect("visible");
        assert!((red - 255.0 / 383.0).abs() < EPSILON, "{red}");
        assert_eq!(green, 0.0);
        assert!((blue - 128.0 / 383.0).abs() < EPSILON, "{blue}");
        assert_eq!(light_texture_glow_rgb(&[255, 255, 255, 0]), None);
    }

    #[test]
    fn the_land_light_sits_on_the_ground_under_the_glow() {
        let bounds = client_light_sprite_bounds(&LANTERN_LIGHT, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        let key = ClientLightSpriteKey { map_id: 0, tile_x: 2507, tile_y: 560, z: 12, graphic: 2594 };
        let origin = client_light_sprite_origin(key.tile_x, key.tile_y, key.z);
        let (center, size) = client_light_sprite_quad(origin, bounds);
        let glow_rgb = [1.0, 0.5, 0.25];
        let sprite = ClientLightSpriteInstance { key, texture_entry: LANTERN_LIGHT.light_texture_entry, center, size, glow_rgb };
        let light = client_light_sprite_static_light(&sprite);
        // The centre, 0.697 above the base, shows over the ground 0.697 back along the (1, 1, 1) view ray.
        let lift = center.y - origin.y;
        assert!((light.world_x - (center.x - lift)).abs() < EPSILON, "{light:?}");
        assert!((light.world_z - (center.z - lift)).abs() < EPSILON, "{light:?}");
        assert!((light.world_y - (origin.y + STATIC_LIGHT_Y_BIAS)).abs() < EPSILON, "{light:?}");
        // 512 Enhanced pixels each way: 512 / 66 legacy tiles, as light.mul sizes its masks.
        let side_tiles = 2.0 * LIGHT_GENERIC_SIDE as f32 / 66.0;
        assert!((light.width_world - side_tiles).abs() < EPSILON, "{light:?}");
        assert!((light.height_world - side_tiles).abs() < EPSILON, "{light:?}");
        assert_eq!(light.color_rgb, glow_rgb);
        assert_eq!((light.light_id, light.hue_id, light.key.graphic), (1, 0, 2594));
    }

    /// Where the clients show a light's glow centre, on the ground, against its item's cell centre: the distance
    /// `centre_on_items` classifies by.
    fn glow_distance_from_cell(light: &TileMetaItemLight, side: u32) -> (f32, Vec3, ClientLightSpriteKey) {
        let key = ClientLightSpriteKey { map_id: 0, tile_x: 4622, tile_y: 3560, z: 30, graphic: light.tile_id as u16 };
        let origin = client_light_sprite_origin(key.tile_x, key.tile_y, key.z);
        let (center, _) = client_light_sprite_quad(origin, client_light_sprite_bounds(light, side, side));
        let cell = client_light_sprite_cell_centre(key.tile_x, key.tile_y, key.z);
        let ground = ground_under(center, cell.y);
        (Vec2::new(ground.x - cell.x, ground.z - cell.z).length(), center, key)
    }

    /// The tileart's own offsets keep the glow near its item and stay; its default offsets throw it about five tiles
    /// away and are centred. The tilemeta records of the census in `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH`.
    #[test]
    fn centring_moves_the_default_offsets_and_keeps_the_items_own() {
        const SMALL_SIDE: u32 = 128;
        let light = |tile_id, light_texture_entry, scale, offset_x, offset_y| TileMetaItemLight {
            tile_id,
            light_texture_entry,
            scale,
            offset_x,
            offset_y,
        };
        let kept = [
            (LANTERN_LIGHT, LIGHT_GENERIC_SIDE, 0.36),
            (light(14, 3, 1.0, -1.35, -3.0), LIGHT_GENERIC_SIDE, 1.86), // window, south wall: its beam
            (light(60, 4, 1.0, 1.75, -3.0), LIGHT_GENERIC_SIDE, 1.80), // window, east wall: its beam
            (light(2567, 9, 1.5, 0.21, -2.8), LIGHT_GENERIC_SIDE, 0.64), // wall torch
            (light(2401, 1, 2.0, -1.0, -4.0), LIGHT_GENERIC_SIDE, 1.09), // fireplace, its fire
            (light(2555, 9, 1.5, -3.15, -2.5), LIGHT_GENERIC_SIDE, 3.28), // wall sconce
            (light(1, 3, 1.0, -3.6, -3.0), LIGHT_GENERIC_SIDE, 3.96), // the farthest kept: a window
            (light(1, 11, 1.6, 0.1, -1.0), SMALL_SIDE, 0.50), // crystal, a 128-pixel texture
        ];
        let centred = [
            (light(4846, 1, 2.0, -5.0, -4.0), 4.83), // lava
            (light(2557, 1, 2.0, -5.0, -4.0), 4.83), // wall sconce
            (light(186, 1, 2.0, -5.0, -4.0), 4.83), // window
            (light(2402, 1, 2.0, -5.0, -4.0), 4.83), // fireplace, beside its fire
            (light(3555, 1, 2.0, 0.0, 0.0), 4.78), // campfire
            (light(1, 7, 2.0, -4.7, -2.0), 5.26), // candelabra
        ];
        for (light, side, distance) in kept {
            let (actual, center, key) = glow_distance_from_cell(&light, side);
            assert!((actual - distance).abs() < 0.01, "{light:?}: {actual}");
            assert!(actual <= CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH, "{light:?}");
            assert_eq!(client_light_sprite_centred(center, key), center, "{light:?}");
        }
        for (light, distance) in centred {
            let (actual, center, key) = glow_distance_from_cell(&light, LIGHT_GENERIC_SIDE);
            assert!((actual - distance).abs() < 0.01, "{light:?}: {actual}");
            let moved = client_light_sprite_centred(center, key);
            // Moved across the ground, at its height, over the cell centre.
            assert_eq!(moved.y, center.y, "{light:?}");
            let cell = client_light_sprite_cell_centre(key.tile_x, key.tile_y, key.z);
            assert!(ground_under(moved, cell.y).abs_diff_eq(cell, EPSILON), "{light:?}: {moved}");
        }
    }

    /// A centred lava light: the light on the ground lands on the lava's cell centre, as large as the sprite.
    #[test]
    fn a_centred_sprites_land_light_lands_on_its_cell() {
        const LAVA_SCALE: f32 = 1.5;
        let lava = TileMetaItemLight { tile_id: 4846, light_texture_entry: 1, scale: 2.0, offset_x: -5.0, offset_y: -4.0 };
        let (_, center, key) = glow_distance_from_cell(&lava, LIGHT_GENERIC_SIDE);
        let bounds = client_light_sprite_bounds(&lava, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        let (_, size) = client_light_sprite_quad(Vec3::ZERO, bounds);
        let sprite = ClientLightSpriteInstance {
            key,
            texture_entry: lava.light_texture_entry,
            center: client_light_sprite_centred(center, key),
            size: size * LAVA_SCALE,
            glow_rgb: [1.0; 3],
        };
        let light = client_light_sprite_static_light(&sprite);
        assert!((light.world_x - (key.tile_x as f32 + TILE_CENTRE_OFFSET)).abs() < EPSILON, "{light:?}");
        assert!((light.world_z - (key.tile_y as f32 + TILE_CENTRE_OFFSET)).abs() < EPSILON, "{light:?}");
        let side_tiles = LAVA_SCALE * 2.0 * LIGHT_GENERIC_SIDE as f32 / 66.0;
        assert!((light.width_world - side_tiles).abs() < EPSILON, "{light:?}");
    }

    #[test]
    fn the_reach_is_the_largest_bound() {
        let bounds = client_light_sprite_bounds(&LANTERN_LIGHT, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        assert!((client_light_sprite_reach(bounds) - 6.428).abs() < EPSILON);
    }
}
