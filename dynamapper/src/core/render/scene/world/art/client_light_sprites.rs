//! The Enhanced and KR clients' light sprites, drawn by KR-like, Enhanced and UOD-Painterly into the light buffer
//! (`effects::client_night`). KR-like and Enhanced show it through the client night composite; UOD-Painterly, which
//! dims through its scene multiplier, adds it over the frame as a glow (`static_lights::light_sprite_glow_strength`).
//!
//! A static draws a sprite when its tileart names a light texture (tilemeta `item_lights`); the LightSource flag plays
//! no part. The texture is a `TerrainTexture.uop` entry packed in `tex_aux_ec.uddp`, drawn with its own colour and
//! alpha: no hue, even on a hued item. The KR and Enhanced clients ship the same textures, so the three styles draw
//! one set. The quad is the EC static billboard of the clients' sprite builder: anchored to the static's map
//! position (never to its art, so CC and EC art place it alike), sized by the tileart light scale, and moved by the
//! light offsets in world units along the camera's right and up. Classic draws the light.mul decals of
//! `static_lights` instead.
//!
//! Two settings depart from the clients in these styles. The style's `local_lights.centre_on_items` puts each
//! sprite's light where its item is drawn: on the flame of the item's Classic art where it has one, else the sprites
//! the tileart's default offsets throw far from their item on the art's centre, the others on the nearest point of
//! that art (`client_light_sprite_centred`).
//! `configs::light_scale` (LightScale.kdl) enlarges chosen items' sprites.
//!
//! The clients light the terrain and the statics through the composite alone: their sprites are unlit and their
//! terrain has no other light. The optional land light decals and static local lights (Light/Static Interaction,
//! off in the KR-like and Enhanced presets, on in UOD-Painterly's) read these sprites in these styles
//! (`client_light_sprite_static_light`), unless the style's `local_lights.source` picks light.mul, as Classic
//! (`static_lights`).

use super::light_cover::{static_light_ceiling_z, static_light_is_hidden, LightCoverRule, MultiCoverParts};
use super::light_scene::{
    light_clip_material, light_clip_tag, set_light_clip_mode, LightClipMaterial, LightClipMode, LightScene,
};
use crate::core::render::scene::world::caves::CavesRes;
use crate::core::render::scene::world::z_view::{ZViewCut, ZViewLimit};
use super::static_lights::{StaticLightInstance, StaticLightKey, StaticLightTarget, LIGHT_PIXELS_PER_WORLD_TILE,
    STATIC_LIGHT_Y_BIAS};
use super::statics_collect::{
    apply_cc_static_world_anchor_translation, apply_static_world_anchor_translation, classic_static_visual_kind,
    resolve_static_billboard_bounds, static_land_z, StaticBillboardBounds, StaticChunkBatchKey, StaticVisualKind,
    CC_TILE_PIXEL_WIDTH, CC_WORLD_XZ_PER_PIXEL, CC_WORLD_Y_PER_PIXEL, STATIC_BILLBOARD_RIGHT_XZ,
    STATIC_WORLD_Y_PER_XZ_PIXEL,
};
use crate::core::render::scene::camera::PlayerCamera;
use crate::configs::{light_scale::ItemLightScales, settings::Settings, shader_presets::UniformState};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::multis::{MultiDefinitionsRes, PlacedMultisRes};
use crate::core::render::scene::world::effects::client_night::LIGHT_BUFFER_LAYER;
use crate::core::render::scene::world::land::{LCMesh, CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM};
use crate::core::render::scene::SceneStateData;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{MapPlanesRes, TexArtCcPackageRes, TexAuxEcPackageRes, TileMetaPackageRes};
use crate::util_lib::uo_coords::scale_uo_z_to_bevy_units;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat};
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use udd_assets::tex_art_cc::TexArtCcPackage;
use udd_assets::tex_art_ec::LEGACY_PIXEL_UNIT;
use udd_assets::tex_aux_ec::tex_aux_ec_terrain_texture_key;
use udd_assets::tilemeta::{TileMetaItemLight, TileMetaPackage};

/// The sprite's horizontal anchor: its left edge sits `CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS - W/2` texture pixels right
/// of the base point (W/2 an integer shift), so its centre sits 22 scaled pixels right, on a 44-pixel tile's centre.
const CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS: i32 = 22;
/// The sprite's vertical anchor: its bottom edge sits at the base point.
const CLIENT_LIGHT_SPRITE_ANCHOR_Y_PIXELS: i16 = 0;
/// How far, in tiles, the ground point under a sprite's glow may lie from its item's cell centre for
/// `centre_on_items` to take the offsets as the item's own. The offsets the tileart gives an item of its own keep the
/// glow within 4.04 tiles of it (window beams, wall torches and sconces, fireplace fires, candles, lanterns); the
/// tileart's default offsets, (-5, -4) on 378 items and (0, 0) at scale 2 on 7, and two candelabras' (-4.7, -2) throw
/// it 4.78 tiles or more away, with no light in between. The default offsets place nothing: their light goes on the
/// art's centre. An item's own offsets place it, but not always on the item (a sconce's flame 17 pixels right of and
/// 47 below its glow, a candle's 65 right, a wall torch's 51 above): their light goes on the nearest point of the
/// art. Either way a flame in the art takes the light first (`client_light_sprite_centred`).
pub(super) const CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH: f32 = 4.5;
/// A light texture's source, (u, v) with v from the top, where the texture draws no source of its own: its centre.
const LIGHT_TEXTURE_CENTRE_UV: Vec2 = Vec2::splat(0.5);
/// The `TerrainTexture.uop` entry of the lamp posts' light (lamp posts 2848, 2850, 2852 and 44555, 44557, 44559): a
/// post lit by its lamp, whose centre sits at the post's foot.
const LAMP_POST_LIGHT_TEXTURE_ENTRY: u32 = 2;
/// The lamp in the lamp posts' light texture.
const LAMP_POST_LIGHT_SOURCE_UV: Vec2 = Vec2::new(0.47, 0.24);
/// The `TerrainTexture.uop` entry of the south walls' window beam: a lit pane at the top right, its beam down to the
/// left across the floor.
const SOUTH_WINDOW_BEAM_TEXTURE_ENTRY: u32 = 3;
/// The pane in the south walls' window beam.
const SOUTH_WINDOW_BEAM_SOURCE_UV: Vec2 = Vec2::new(0.736, 0.178);
/// The `TerrainTexture.uop` entry of the east walls' window beam: the south beam mirrored.
const EAST_WINDOW_BEAM_TEXTURE_ENTRY: u32 = 4;
/// The pane in the east walls' window beam.
const EAST_WINDOW_BEAM_SOURCE_UV: Vec2 = Vec2::new(0.260, 0.178);
/// The fewest Classic art pixels each way that place a light. Smaller art is a stand-in for an item drawn otherwise
/// (fire column 14119's animation, the one-pixel light markers), whose light is placed as if it had no art.
const CLIENT_LIGHT_ART_MIN_PIXELS: f32 = 2.0;
/// The hues, in degrees, of an art's flame texels (`is_flame_texel`): orange to yellow. Red is left out, as the lava's
/// and the red cloths' and gems' red.
const FLAME_HUE_MIN_DEGREES: f32 = 15.0;
const FLAME_HUE_MAX_DEGREES: f32 = 65.0;
/// The least saturation of a flame texel, (max - min) / max of its channels: the flames' pure orange and yellow, not
/// the sandstone's, the brass' or the gold's paler hues of the same.
const FLAME_SATURATION_MIN: f32 = 0.85;
/// The least value of a flame texel, its largest channel over `LIGHT_TEXTURE_CHANNEL_MAX`: the dim flames of an
/// animation's frames too, not the dark wood of a torch.
const FLAME_VALUE_MIN: f32 = 0.75;
/// The value at least one texel of a flame reaches: its hot core.
const FLAME_HOT_VALUE_MIN: f32 = 0.9;
/// The fewest Classic art pixels of one flame: a candle's flame has 3 to 5.
const FLAME_CLUSTER_MIN_PIXELS: usize = 3;
/// The share of the art's box, both ways, that flames spanning it make the whole item a glow (lava, a campfire's
/// blaze, the glowing Windows): its light is placed as without a flame.
const FLAME_SPREAD_FRACTION: f32 = 0.5;
/// The degrees of hue between two of red, green and blue.
const HUE_SECTOR_DEGREES: f32 = 60.0;
/// The degrees of the hue circle.
const HUE_CIRCLE_DEGREES: f32 = 360.0;
/// From a texel's corner to its centre.
const TEXEL_CENTRE_OFFSET: f32 = 0.5;
/// From a tile's corner to its centre, along each map axis.
const TILE_CENTRE_OFFSET: f32 = 0.5;
/// The largest 8-bit channel value, which maps a light texture's or an art's channel to 0..1.
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
    /// Under `LightCover::Uod`, the z of the lowest floor or roof over the light (`static_light_ceiling_z`), which its
    /// glow does not light walls from; `None` under the other covers and with nothing over it.
    pub ceiling: Option<i8>,
}

/// The light sprites of the statics in and around the visible chunks.
#[derive(Resource, Default)]
pub struct RenderClientLightSprites {
    pub instances: Vec<ClientLightSpriteInstance>,
    /// What the last scan covered, whether it centred the sprites on their items, and the rule, z-view cut and multi
    /// parts it hid covered lights by (`light_cover`), and the caves drawn over the map (`CavesRes::revision`); the scan
    /// reruns only when it changes. The last field is the map plane's `blocks_loaded_version` when the scan read the
    /// land z of a cell whose block was not loaded yet: the scan reruns once more land loads, so a scan made before the
    /// land does not keep its stand-in land z.
    last_scan: Option<(u32, Vec<StaticChunkBatchKey>, bool, LightCoverRule, ZViewCut, bool, u64, Option<u64>)>,
}

/// The sprites' materials, one per light texture (`None` once a texture failed to load), and the scan reach.
#[derive(Resource, Default)]
pub struct ClientLightSpriteCache {
    materials_by_entry: HashMap<u32, Option<Handle<LightClipMaterial>>>,
    /// Each light texture's glow colour, `None` once the texture failed to load.
    glow_rgb_by_entry: HashMap<u32, Option<[f32; 3]>>,
    /// Each lit item's Classic art (`cache_client_light_sprite_art`), `None` for an item without one.
    art_by_graphic: HashMap<u16, Option<ClientLightSpriteArt>>,
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

/// Whether the clients' placement shows the sprite centre `center` more than `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH`
/// tiles from its item's cell centre, on the ground: whether its offsets are the tileart's defaults.
fn client_light_sprite_has_default_offsets(center: Vec3, key: ClientLightSpriteKey) -> bool {
    let cell = client_light_sprite_cell_centre(key.tile_x, key.tile_y, key.z);
    let ground = ground_under(center, cell.y);
    Vec2::new(cell.x - ground.x, cell.z - ground.z).length() > CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH
}

/// Where in a light texture its light comes from, (u, v) with v from the top: the lamp of the lamp posts' light, the
/// pane of the window beams, the centre of the others.
fn light_texture_source_uv(texture_entry: u32) -> Vec2 {
    match texture_entry {
        LAMP_POST_LIGHT_TEXTURE_ENTRY => LAMP_POST_LIGHT_SOURCE_UV,
        SOUTH_WINDOW_BEAM_TEXTURE_ENTRY => SOUTH_WINDOW_BEAM_SOURCE_UV,
        EAST_WINDOW_BEAM_TEXTURE_ENTRY => EAST_WINDOW_BEAM_SOURCE_UV,
        _ => LIGHT_TEXTURE_CENTRE_UV,
    }
}

/// The box around an art's texels with any alpha, in its pixels from its top left, its far edges past the last
/// texels: `[min_x, min_y, max_x, max_y]`. `None` for an art without one.
fn art_opaque_rect(rgba: &[u8], width: u32) -> Option<[u32; 4]> {
    let width = width as usize;
    let mut rect: Option<[u32; 4]> = None;
    for (index, texel) in rgba.chunks_exact(RGBA_TEXEL_BYTES).enumerate() {
        if texel[3] == 0 {
            continue;
        }
        let (x, y) = ((index % width) as u32, (index / width) as u32);
        let [min_x, min_y, max_x, max_y] = rect.get_or_insert([x, y, x + 1, y + 1]);
        (*min_x, *min_y, *max_x, *max_y) = ((*min_x).min(x), (*min_y).min(y), (*max_x).max(x + 1), (*max_y).max(y + 1));
    }
    rect
}

/// Whether an art texel is a flame's: orange to yellow (`FLAME_HUE_MIN_DEGREES` to `FLAME_HUE_MAX_DEGREES`), pure
/// (`FLAME_SATURATION_MIN`) and bright (`FLAME_VALUE_MIN`), and opaque.
fn is_flame_texel(texel: &[u8]) -> bool {
    let [r, g, b] = [texel[0], texel[1], texel[2]].map(f32::from);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let chroma = max - min;
    if texel[3] == 0 || max < FLAME_VALUE_MIN * LIGHT_TEXTURE_CHANNEL_MAX || chroma < FLAME_SATURATION_MIN * max {
        return false;
    }
    let sector = if max == r {
        (g - b) / chroma
    } else if max == g {
        2.0 + (b - r) / chroma
    } else {
        4.0 + (r - g) / chroma
    };
    let hue = (sector * HUE_SECTOR_DEGREES).rem_euclid(HUE_CIRCLE_DEGREES);
    (FLAME_HUE_MIN_DEGREES..=FLAME_HUE_MAX_DEGREES).contains(&hue)
}

/// Where an art's flames burn, in its pixels from its top left: the centroid of its flame texels (`is_flame_texel`)
/// that touch each other (diagonally too) in groups of at least `FLAME_CLUSTER_MIN_PIXELS` Classic pixels with a hot
/// core (`FLAME_HOT_VALUE_MIN`); `opaque` is its box (`art_opaque_rect`). `None` for art without a flame, and for art
/// whose flames span `FLAME_SPREAD_FRACTION` of its box both ways: the whole item glows (lava, a campfire's blaze).
fn art_flame_centroid(rgba: &[u8], width: u32, upscale_factor: u16, opaque: [u32; 4]) -> Option<Vec2> {
    let width = width as usize;
    let texels = rgba.chunks_exact(RGBA_TEXEL_BYTES).collect::<Vec<_>>();
    if width == 0 || texels.is_empty() {
        return None;
    }
    let height = texels.len() / width;
    let flame = texels.iter().map(|texel| is_flame_texel(texel)).collect::<Vec<_>>();
    let hot = |index: usize| {
        let texel = texels[index];
        f32::from(texel[0].max(texel[1]).max(texel[2])) >= FLAME_HOT_VALUE_MIN * LIGHT_TEXTURE_CHANNEL_MAX
    };
    let upscale = usize::from(upscale_factor.max(1));
    let cluster_min = FLAME_CLUSTER_MIN_PIXELS * upscale * upscale;
    let mut seen = vec![false; flame.len()];
    let mut kept = Vec::new();
    let mut stack = Vec::new();
    for start in 0..height * width {
        if !flame[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let cluster_start = kept.len();
        let mut has_hot_core = false;
        while let Some(index) = stack.pop() {
            kept.push(index);
            has_hot_core |= hot(index);
            let (x, y) = (index % width, index / width);
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if flame[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        if kept.len() - cluster_start < cluster_min || !has_hot_core {
            kept.truncate(cluster_start);
        }
    }
    if kept.is_empty() {
        return None;
    }
    let (mut min, mut max, mut sum) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN), Vec2::ZERO);
    for &index in &kept {
        let texel = Vec2::new((index % width) as f32, (index / width) as f32);
        (min, max, sum) = (min.min(texel), max.max(texel), sum + texel);
    }
    let [opaque_min_x, opaque_min_y, opaque_max_x, opaque_max_y] = opaque;
    let opaque_size = Vec2::new((opaque_max_x - opaque_min_x) as f32, (opaque_max_y - opaque_min_y) as f32);
    let flame_size = max - min + Vec2::ONE;
    if flame_size.cmpge(opaque_size * FLAME_SPREAD_FRACTION).all() {
        return None;
    }
    Some(sum / kept.len() as f32 + Vec2::splat(TEXEL_CENTRE_OFFSET))
}

/// Where a point of a Classic static art, in the slot's upscaled pixels from its top left, shows in the sprite's local
/// coordinates (`client_light_sprite_bounds`): the art stands at its CC anchor, the cell's near vertex, 22 pixels
/// right of the sprite's base point in the same upright plane, moved by its draw offsets.
fn client_light_sprite_art_point(point: Vec2, upscale_factor: u16, draw_offset: (i16, i16), logical_height: f32)
    -> Vec2 {
    let (cc_x, cc_z) = apply_cc_static_world_anchor_translation(0.0, 0.0);
    let (ec_x, ec_z) = apply_static_world_anchor_translation(0.0, 0.0);
    let anchor_right = Vec2::new(cc_x - ec_x, cc_z - ec_z).dot(STATIC_BILLBOARD_RIGHT_XZ);
    let pixels = point / f32::from(upscale_factor.max(1));
    Vec2::new(
        anchor_right + (f32::from(draw_offset.0) + pixels.x) * CC_WORLD_XZ_PER_PIXEL,
        (logical_height - f32::from(draw_offset.1) - pixels.y) * CC_WORLD_Y_PER_PIXEL,
    )
}

/// A Classic static art's opaque box (`art_opaque_rect`, in the slot's upscaled pixels) as the static draws it, in
/// the sprite's local coordinates (`client_light_sprite_art_point`). `None` for a box too small to place a light
/// (`CLIENT_LIGHT_ART_MIN_PIXELS`).
fn client_light_sprite_art_bounds(rect: [u32; 4], upscale_factor: u16, draw_offset: (i16, i16), logical_height: f32)
    -> Option<StaticBillboardBounds> {
    let [min_x, min_y, max_x, max_y] = rect;
    let size = Vec2::new((max_x - min_x) as f32, (max_y - min_y) as f32) / f32::from(upscale_factor.max(1));
    if size.min_element() < CLIENT_LIGHT_ART_MIN_PIXELS {
        return None;
    }
    let point = |x: u32, y: u32| {
        client_light_sprite_art_point(Vec2::new(x as f32, y as f32), upscale_factor, draw_offset, logical_height)
    };
    let (bottom_left, top_right) = (point(min_x, max_y), point(max_x, min_y));
    Some(StaticBillboardBounds {
        local_min_x: bottom_left.x,
        local_max_x: top_right.x,
        local_min_y: bottom_left.y,
        local_max_y: top_right.y,
    })
}

/// A lit item's Classic art as `centre_on_items` places its light by, in the sprite's local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ClientLightSpriteArt {
    /// The box around its visible texels (`client_light_sprite_art_bounds`).
    pub bounds: StaticBillboardBounds,
    /// Where its flames burn (`art_flame_centroid`), `None` for art without a flame.
    pub flame: Option<Vec2>,
}

/// An item's Classic art (`ClientLightSpriteArt`) from its slot's RGBA, `width` upscaled pixels wide. `None` for art
/// too small to place a light.
fn client_light_sprite_art(rgba: &[u8], width: u32, upscale_factor: u16, draw_offset: (i16, i16), logical_height: f32)
    -> Option<ClientLightSpriteArt> {
    let rect = art_opaque_rect(rgba, width)?;
    let bounds = client_light_sprite_art_bounds(rect, upscale_factor, draw_offset, logical_height)?;
    let flame = art_flame_centroid(rgba, width, upscale_factor, rect)
        .map(|point| client_light_sprite_art_point(point, upscale_factor, draw_offset, logical_height));
    Some(ClientLightSpriteArt { bounds, flame })
}

/// Caches the Classic art (`client_light_sprite_art`) of the items `graphics` not cached yet, reading each art page
/// once: the art the Classic statics draw an item with, its own or its tilemeta's stand-in. `None` without the art (or
/// `tex_art_cc.uddp`), or with art too small to place a light.
fn cache_client_light_sprite_art(
    art_by_graphic: &mut HashMap<u16, Option<ClientLightSpriteArt>>,
    tex_art_cc: Option<&TexArtCcPackage>,
    tilemeta: &TileMetaPackage,
    graphics: impl IntoIterator<Item = u16>,
) {
    let mut slots = Vec::new();
    for graphic in graphics {
        if art_by_graphic.contains_key(&graphic) {
            continue;
        }
        let slot = tex_art_cc.and_then(|tex_art_cc| {
            let StaticVisualKind::CcRegular { art_id, fallback_art_id } =
                classic_static_visual_kind(graphic, tilemeta.item_tile(u32::from(graphic)))
            else {
                return None;
            };
            tex_art_cc.present_slot(art_id).or_else(|| tex_art_cc.present_slot(fallback_art_id)).copied()
        });
        match slot {
            Some(slot) => slots.push((graphic, slot)),
            None => {
                art_by_graphic.insert(graphic, None);
            }
        }
    }
    let Some(tex_art_cc) = tex_art_cc.filter(|_| !slots.is_empty()) else {
        return;
    };
    let art_ids = slots.iter().map(|(_, slot)| slot.art_id).collect::<Vec<_>>();
    for ((graphic, slot), rgba) in slots.into_iter().zip(tex_art_cc.get_pixel_data_many(&art_ids)) {
        let art = rgba.and_then(|rgba| {
            client_light_sprite_art(&rgba, u32::from(slot.width), slot.upscale_factor,
                (slot.draw_offset_x, slot.draw_offset_y), slot.logical_height())
        });
        art_by_graphic.insert(graphic, art);
    }
}

/// A tile's Classic art bounds, the 44-pixel diamond over the cell: where a light without art goes when its offsets
/// are the tileart's defaults, its centre the cell centre's.
fn client_light_sprite_tile_bounds() -> StaticBillboardBounds {
    let side = CC_TILE_PIXEL_WIDTH as u32;
    let half = (CC_TILE_PIXEL_WIDTH / 2.0) as i16;
    client_light_sprite_art_bounds([0, 0, side, side], 1, (-half, 0), CC_TILE_PIXEL_WIDTH)
        .expect("a tile is large enough to place a light")
}

/// The sprite centre `center`, of a sprite `size` large drawing `texture_entry`, as `centre_on_items` places it:
/// moved so that its light (`light_texture_source_uv`) shows where its item is drawn, `art` its Classic art
/// (`cache_client_light_sprite_art`).
///
/// - **A flame** in the art (`art_flame_centroid`): the light goes on it, whatever the offsets (candles, sconces,
///   wall torches, candelabras, braziers, fireplaces and hearths, fields of fire). The light textures with a source
///   of their own, the lamp posts' and the window beams', keep the rules below.
/// - **Default offsets** (`client_light_sprite_has_default_offsets`), which place nothing: the light goes on the
///   art's centre, or without art on the cell centre's, where the light on the ground under the sprite
///   (`client_light_sprite_static_light`) lands.
/// - **The item's own offsets:** the light goes on the nearest point of the art. A light already on its art stays
///   (window beams, lamp posts, lanterns); one beside it moves onto its edge. Without art it stays.
///
/// The move is along the sprite's plane, upright through the item's art, so the light shows on the art.
pub(super) fn client_light_sprite_centred(
    center: Vec3,
    size: Vec2,
    key: ClientLightSpriteKey,
    texture_entry: u32,
    art: Option<ClientLightSpriteArt>,
) -> Vec3 {
    let origin = client_light_sprite_origin(key.tile_x, key.tile_y, key.z);
    let right = billboard_right();
    let local = Vec2::new((center - origin).dot(right), center.y - origin.y);
    let source_uv = light_texture_source_uv(texture_entry);
    let source = local + (source_uv - LIGHT_TEXTURE_CENTRE_UV) * size * Vec2::new(1.0, -1.0);
    let box_min = |bounds: StaticBillboardBounds| Vec2::new(bounds.local_min_x, bounds.local_min_y);
    let box_max = |bounds: StaticBillboardBounds| Vec2::new(bounds.local_max_x, bounds.local_max_y);
    let flame = art.and_then(|art| art.flame).filter(|_| source_uv == LIGHT_TEXTURE_CENTRE_UV);
    let target = if let Some(flame) = flame {
        flame
    } else if client_light_sprite_has_default_offsets(center, key) {
        let bounds = art.map_or_else(client_light_sprite_tile_bounds, |art| art.bounds);
        (box_min(bounds) + box_max(bounds)) * 0.5
    } else if let Some(art) = art {
        source.clamp(box_min(art.bounds), box_max(art.bounds))
    } else {
        source
    };
    let shift = target - source;
    center + right * shift.x + Vec3::Y * shift.y
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
        ceiling: sprite.ceiling,
        // No decal draws it.
        cave_outside: false,
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
    tex_art_cc_res: Option<Res<TexArtCcPackageRes>>,
    map_planes: Option<Res<MapPlanesRes>>,
    multis: (Option<Res<MultiDefinitionsRes>>, Res<PlacedMultisRes>),
    settings: Res<Settings>,
    uniform_state: Res<UniformState>,
    light_scales: Res<ItemLightScales>,
    scene_state: Res<SceneStateData>,
    chunks_q: Query<&LCMesh>,
    view_cuts: (Res<ZViewLimit>, Res<CavesRes>),
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
            log::warn!(
                "KR-like, Enhanced and UOD-Painterly draw no light sprites: there is no tex_aux_ec.uddp, which holds them"
            );
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
    let light_cover = uniform_state.light_cover_rule();
    let (z_view, caves) = view_cuts;
    let z_cut = z_view.0;
    let show_hidden_multi_parts = settings.world_rendering.show_hidden_multi_parts;
    let land_version = map_planes
        .as_deref()
        .and_then(|planes| planes.0.get(map_id as usize)?.as_ref())
        .map(|plane| plane.blocks_loaded_version);
    if output.last_scan.as_ref().is_some_and(
        |(last_map, last_keys, last_centred, last_cover, last_cut, last_hidden, last_caves, land_wait)| {
            *last_map == map_id && *last_keys == visible_chunk_keys && *last_centred == centre_on_items
                && *last_cover == light_cover && *last_cut == z_cut && *last_hidden == show_hidden_multi_parts
                && *last_caves == caves.revision()
                && land_wait.is_none_or(|version| Some(version) == land_version)
        },
    ) {
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

    let block_range = |coordinate: fn(&(u32, u32)) -> u32| {
        let start = blocks.iter().map(coordinate).min().unwrap_or(0);
        start..blocks.iter().map(coordinate).max().map_or(start, |end| end + 1)
    };
    let (blocks_x, blocks_y) = (block_range(|&(_, gx)| gx), block_range(|&(gy, _)| gy));
    let multi_parts = MultiCoverParts::read(
        &tilemeta.0,
        multis.0.as_deref(),
        &multis.1,
        show_hidden_multi_parts,
        map_id,
        blocks_x,
        blocks_y,
    );

    if centre_on_items {
        let mut lit_graphics = BTreeSet::new();
        for &(gy, gx) in &blocks {
            if let Ok(tiles) = statics_store.block_tiles(gx, gy) {
                lit_graphics.extend(
                    tiles.iter().map(|tile| tile.graphic).filter(|graphic| {
                        tilemeta.0.item_light(u32::from(*graphic)).is_some()
                    }),
                );
            }
        }
        cache_client_light_sprite_art(&mut cache.art_by_graphic,
            tex_art_cc_res.as_deref().map(|package| package.0.as_ref()), &tilemeta.0, lit_graphics);
    }

    let mut instances = Vec::new();
    let mut seen = HashSet::new();
    let land_missed = Cell::new(false);
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
            let land_z = |x: u32, y: u32| {
                let land_z = static_land_z(map_planes.as_deref(), map_id, (x as i32, y as i32));
                land_missed.set(land_missed.get() || land_z.is_none());
                land_z
            };
            if !seen.insert(key) {
                continue;
            }
            // The player's cave hides it, or draws it whatever the view's limit (caves.rs). Outside it the sprite is
            // not darkened itself: the light buffer multiplies the scene, darkened already.
            let flags_height = tilemeta.0.item_tile(u32::from(key.graphic)).map_or((0, 0), |meta| (meta.flags, meta.height));
            let Some(cave_light) = caves.light_view(key.tile_x, key.tile_y, key.z, flags_height.0, flags_height.1,
                || land_z(key.tile_x, key.tile_y).map(i32::from), z_cut.max_z) else {
                continue;
            };
            let light_max_z = cave_light.max_z;
            if static_light_is_hidden(
                &mut statics_store,
                &tilemeta.0,
                &multi_parts,
                light_cover,
                &land_z,
                key.tile_x,
                key.tile_y,
                key.z,
                light_max_z,
                z_cut.roof_layer,
            ) {
                continue;
            }
            let ceiling = (light_cover == LightCoverRule::Uod)
                .then(|| {
                    static_light_ceiling_z(
                        &mut statics_store,
                        &tilemeta.0,
                        &multi_parts,
                        &land_z,
                        key.tile_x,
                        key.tile_y,
                        key.z,
                        light_max_z,
                        z_cut.roof_layer,
                    )
                })
                .flatten();
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
            let size = size * light_scales.scale(key.graphic);
            let center = if centre_on_items {
                let art = cache.art_by_graphic.get(&key.graphic).copied().flatten();
                client_light_sprite_centred(center, size, key, entry, art)
            } else {
                center
            };
            instances.push(ClientLightSpriteInstance { key, texture_entry: entry, center, size, glow_rgb, ceiling });
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
    let land_wait = land_version.filter(|_| land_missed.get());
    output.last_scan = Some((map_id, visible_chunk_keys, centre_on_items, light_cover, z_cut, show_hidden_multi_parts,
        caves.revision(), land_wait));
}

pub fn sys_sync_client_light_sprites(
    mut commands: Commands,
    sprites: Res<RenderClientLightSprites>,
    tex_aux_res: Option<Res<TexAuxEcPackageRes>>,
    uniform_state: Res<UniformState>,
    light_scene: Option<Res<LightScene>>,
    mut cache: ResMut<ClientLightSpriteCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<LightClipMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_clip_mode: Local<Option<LightClipMode>>,
    mut existing_q: Query<(Entity, &ClientLightSpriteKey, &mut Transform), With<ClientLightSpriteEntity>>,
) {
    let Some(light_scene) = light_scene else {
        return;
    };
    // Under the "uod" light cover the sprites are clipped to what their light reaches, drawn into the light buffer
    // after the world view's light scene (`light_scene::render_deferred_light_sprites`).
    let clip_mode = if uniform_state.light_cover_rule() == LightCoverRule::Uod {
        LightClipMode::Deferred
    } else {
        LightClipMode::Unclipped
    };
    if *last_clip_mode != Some(clip_mode) {
        set_light_clip_mode(&mut materials, cache.materials_by_entry.values().flatten(), clip_mode);
        *last_clip_mode = Some(clip_mode);
    }
    if !sprites.is_changed() {
        return;
    }
    let desired = sprites.instances.iter().map(|instance| (instance.key, instance)).collect::<HashMap<_, _>>();
    let mut existing_keys = HashSet::new();
    for (entity, key, mut transform) in existing_q.iter_mut() {
        if let Some(instance) = desired.get(key) {
            // A kept sprite may have moved or grown: Local Lights' centring changed. Its light's ceiling may have
            // changed with the light cover.
            transform.translation = instance.center;
            transform.scale = instance.size.extend(1.0);
            commands.entity(entity).insert(client_light_sprite_tag(instance));
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
            .or_insert_with(|| {
                let base = client_light_sprite_material(&tex_aux, instance.texture_entry, &mut images)?;
                Some(materials.add(light_clip_material(base, &light_scene, clip_mode)))
            });
        let Some(material) = material.clone() else {
            continue;
        };
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(instance.center)
                .with_rotation(rotation)
                .with_scale(instance.size.extend(1.0)),
            client_light_sprite_tag(instance),
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
) -> Option<StandardMaterial> {
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
    Some(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(image)),
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        unlit: true,
        fog_enabled: false,
        ..default()
    })
}

/// The light clip tag of a sprite (`light_scene::light_clip_tag`): its static's cell, z and ceiling.
fn client_light_sprite_tag(instance: &ClientLightSpriteInstance) -> MeshTag {
    let key = instance.key;
    MeshTag::new(light_clip_tag(key.tile_x, key.tile_y, key.z, instance.ceiling, false, instance.center))
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
        let sprite = ClientLightSpriteInstance { key, texture_entry: LANTERN_LIGHT.light_texture_entry, center, size, glow_rgb, ceiling: None };
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

    /// The tileart's own offsets keep the glow near its item; its default offsets throw it about five tiles away. The
    /// tilemeta records of the census in `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH`.
    #[test]
    fn the_default_offsets_are_told_from_the_items_own() {
        const SMALL_SIDE: u32 = 128;
        let light = |tile_id, light_texture_entry, scale, offset_x, offset_y| TileMetaItemLight {
            tile_id,
            light_texture_entry,
            scale,
            offset_x,
            offset_y,
        };
        let own = [
            (LANTERN_LIGHT, LIGHT_GENERIC_SIDE, 0.36),
            (light(14, 3, 1.0, -1.35, -3.0), LIGHT_GENERIC_SIDE, 1.86), // window, south wall: its beam
            (light(60, 4, 1.0, 1.75, -3.0), LIGHT_GENERIC_SIDE, 1.80), // window, east wall: its beam
            (light(2567, 9, 1.5, 0.21, -2.8), LIGHT_GENERIC_SIDE, 0.64), // wall torch
            (light(2401, 1, 2.0, -1.0, -4.0), LIGHT_GENERIC_SIDE, 1.09), // fireplace, its fire
            (light(2555, 9, 1.5, -3.15, -2.5), LIGHT_GENERIC_SIDE, 3.28), // wall sconce
            (light(1, 3, 1.0, -3.6, -3.0), LIGHT_GENERIC_SIDE, 3.96), // the farthest of the items' own: a window
            (light(1, 11, 1.6, 0.1, -1.0), SMALL_SIDE, 0.50), // crystal, a 128-pixel texture
        ];
        let default = [
            (light(4846, 1, 2.0, -5.0, -4.0), 4.83), // lava
            (light(2557, 1, 2.0, -5.0, -4.0), 4.83), // wall sconce
            (light(186, 1, 2.0, -5.0, -4.0), 4.83), // window
            (light(2402, 1, 2.0, -5.0, -4.0), 4.83), // fireplace, beside its fire
            (light(3555, 1, 2.0, 0.0, 0.0), 4.78), // campfire
            (light(1, 7, 2.0, -4.7, -2.0), 5.26), // candelabra
        ];
        for (light, side, distance) in own {
            let (actual, center, key) = glow_distance_from_cell(&light, side);
            assert!((actual - distance).abs() < 0.01, "{light:?}: {actual}");
            assert!(!client_light_sprite_has_default_offsets(center, key), "{light:?}");
        }
        for (light, distance) in default {
            let (actual, center, key) = glow_distance_from_cell(&light, LIGHT_GENERIC_SIDE);
            assert!((actual - distance).abs() < 0.01, "{light:?}: {actual}");
            assert!(client_light_sprite_has_default_offsets(center, key), "{light:?}");
        }
    }

    /// The bounds of a Classic static art `width` x `height`, centred on its anchor as `classic_static_draw_offset`
    /// draws it, whose opaque texels span `rect` (`art_opaque_rect`).
    fn classic_art_bounds(width: u32, height: u32, rect: [u32; 4]) -> Option<StaticBillboardBounds> {
        client_light_sprite_art_bounds(rect, 1, (-((width / 2) as i16), 0), height as f32)
    }

    /// The art (`ClientLightSpriteArt`) of `classic_art_bounds`, its flame at `flame` in its pixels from its top left.
    fn classic_art(width: u32, height: u32, rect: [u32; 4], flame: Option<Vec2>) -> Option<ClientLightSpriteArt> {
        let draw_offset = (-((width / 2) as i16), 0);
        Some(ClientLightSpriteArt {
            bounds: classic_art_bounds(width, height, rect)?,
            flame: flame.map(|point| client_light_sprite_art_point(point, 1, draw_offset, height as f32)),
        })
    }

    /// A lamp post's lit glass, where a flame shows in its art.
    const LAMP_POST_GLASS: Vec2 = Vec2::new(11.0, 12.0);

    /// A 256-pixel light sprite of `light` on its item at the lava's cell, placed by `centre_on_items` over the art
    /// `art`: where its light shows, in Classic pixels right of and above the art's anchor (the cell's near vertex),
    /// as the clients place it and as it is placed; the placed centre and the key.
    fn shown_light(light: &TileMetaItemLight, art: Option<ClientLightSpriteArt>)
        -> (Vec2, Vec2, Vec3, ClientLightSpriteKey) {
        let (_, center, key) = glow_distance_from_cell(light, LIGHT_GENERIC_SIDE);
        let origin = client_light_sprite_origin(key.tile_x, key.tile_y, key.z);
        let bounds = client_light_sprite_bounds(light, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        let (_, size) = client_light_sprite_quad(origin, bounds);
        let entry = light.light_texture_entry;
        let source_offset = (light_texture_source_uv(entry) - LIGHT_TEXTURE_CENTRE_UV) * size * Vec2::new(1.0, -1.0);
        let shown = |center: Vec3| {
            let source = Vec2::new((center - origin).dot(billboard_right()), center.y - origin.y) + source_offset;
            let anchor_right = std::f32::consts::FRAC_1_SQRT_2;
            Vec2::new((source.x - anchor_right) / CC_WORLD_XZ_PER_PIXEL, source.y / CC_WORLD_Y_PER_PIXEL)
        };
        let placed = client_light_sprite_centred(center, size, key, entry, art);
        (shown(center), shown(placed), placed, key)
    }

    /// The census' pixel precision.
    const PIXEL_EPSILON: f32 = 0.1;

    /// Wall sconce 2562 in the Britain keep: the clients show its glow 15.6 pixels left of and 81.3 above its anchor,
    /// up and to the left of the sconce, whose art spans -2..11 x 21..44 and whose flame burns at (1.8, 40.8)
    /// (`a_sconces_flame_is_found`). The glow moves onto the flame. Candle 2843 likewise, from 69.6 pixels left of its
    /// art, and wall torch 15764, whose offsets put its glow under its art: onto its flame, not its foot.
    #[test]
    fn a_sconces_light_moves_onto_its_flame() {
        let sconce =
            TileMetaItemLight { tile_id: 2562, light_texture_entry: 8, scale: 1.5, offset_x: -0.5, offset_y: -1.5 };
        let sconce_art = classic_art(44, 44, [20, 0, 33, 23], Some(Vec2::new(23.8, 3.2)));
        let (clients, placed, center, key) = shown_light(&sconce, sconce_art);
        assert!(clients.abs_diff_eq(Vec2::new(-15.6, 81.3), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(Vec2::new(1.8, 40.8), PIXEL_EPSILON), "{placed}");
        // The move stays in the sprite's upright plane.
        let local = center - client_light_sprite_origin(key.tile_x, key.tile_y, key.z);
        let off_plane = local - billboard_right() * local.dot(billboard_right());
        assert!(off_plane.x.abs() < EPSILON && off_plane.z.abs() < EPSILON, "{center}");
        let candle =
            TileMetaItemLight { tile_id: 2843, light_texture_entry: 7, scale: 1.0, offset_x: -2.0, offset_y: -1.0 };
        let (clients, placed, _, _) =
            shown_light(&candle, classic_art(44, 44, [17, 0, 28, 30], Some(Vec2::new(23.1, 2.9))));
        assert!(clients.abs_diff_eq(Vec2::new(-69.6, 54.2), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(Vec2::new(1.1, 41.1), PIXEL_EPSILON), "{placed}");
        let torch =
            TileMetaItemLight { tile_id: 15764, light_texture_entry: 1, scale: 2.0, offset_x: 0.0, offset_y: -4.5 };
        let (clients, placed, _, _) =
            shown_light(&torch, classic_art(44, 88, [23, 1, 40, 33], Some(Vec2::new(26.0, 6.4))));
        assert!(clients.abs_diff_eq(Vec2::new(7.3, 30.7), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(Vec2::new(4.0, 81.6), PIXEL_EPSILON), "{placed}");
    }

    /// Art without a flame (a lantern's pale glass; here sconce 2562's art without its flame): the light of the
    /// item's own offsets goes on the nearest point of the art, the sconce's top left corner.
    #[test]
    fn without_a_flame_the_light_moves_onto_its_art() {
        let sconce =
            TileMetaItemLight { tile_id: 2562, light_texture_entry: 8, scale: 1.5, offset_x: -0.5, offset_y: -1.5 };
        let (_, placed, _, _) = shown_light(&sconce, classic_art(44, 44, [20, 0, 33, 23], None));
        assert!(placed.abs_diff_eq(Vec2::new(-2.0, 44.0), PIXEL_EPSILON), "{placed}");
    }

    /// The default offsets with a flame (fireplace 1141, flame at (13.6, 50) of art -22..22 x 0..167): the light goes
    /// on the flame, not the art's centre high on the chimney.
    #[test]
    fn the_default_offsets_put_the_light_on_the_flame() {
        let fireplace =
            TileMetaItemLight { tile_id: 1141, light_texture_entry: 1, scale: 2.0, offset_x: 1.0, offset_y: -4.0 };
        let art = classic_art(44, 168, [0, 1, 44, 168], Some(Vec2::new(35.6, 118.0)));
        let (_, placed, _, _) = shown_light(&fireplace, art);
        assert!(placed.abs_diff_eq(Vec2::new(13.6, 50.0), PIXEL_EPSILON), "{placed}");
    }

    /// An art `width` x `height` of `background` texels, with `texels` painted over, as RGBA.
    fn painted_art(width: u32, height: u32, background: [u8; 4], texels: &[((u32, u32), [u8; 3])]) -> Vec<u8> {
        let mut rgba = background.repeat((width * height) as usize);
        for &((x, y), [r, g, b]) in texels {
            let index = (y * width + x) as usize * RGBA_TEXEL_BYTES;
            rgba[index..index + RGBA_TEXEL_BYTES].copy_from_slice(&[r, g, b, u8::MAX]);
        }
        rgba
    }

    /// Census colours of the legacy art (5-bit channels times 8).
    const FLAME_ORANGE: [u8; 3] = [240, 136, 0];
    const FLAME_YELLOW: [u8; 3] = [248, 200, 0];
    const FLAME_DIM_ORANGE: [u8; 3] = [200, 104, 8];
    const FLAME_DIM_YELLOW: [u8; 3] = [216, 152, 16];
    const FLAME_PALE_CAP: [u8; 3] = [224, 216, 144];
    const BRASS: [u8; 3] = [208, 152, 40];
    const GOLD: [u8; 3] = [208, 176, 56];
    const SANDSTONE: [u8; 3] = [240, 192, 136];
    const LAVA_RED: [u8; 3] = [200, 0, 0];
    const IRON: [u8; 4] = [40, 48, 64, u8::MAX];

    /// The flames' pure, bright orange and yellow, dim ones too; not their pale cap, brass, gold, sandstone or red.
    #[test]
    fn flame_texels_are_pure_bright_orange_and_yellow() {
        let flame = |[r, g, b]: [u8; 3]| is_flame_texel(&[r, g, b, u8::MAX]);
        for colour in [FLAME_ORANGE, FLAME_YELLOW, FLAME_DIM_ORANGE, FLAME_DIM_YELLOW] {
            assert!(flame(colour), "{colour:?}");
        }
        for colour in [FLAME_PALE_CAP, BRASS, GOLD, SANDSTONE, LAVA_RED] {
            assert!(!flame(colour), "{colour:?}");
        }
        let [r, g, b] = FLAME_YELLOW;
        assert!(!is_flame_texel(&[r, g, b, 0]));
    }

    /// Wall sconce 2562's art (its top 23 rows): a three-texel flame over a pale cap, on an iron and brass sconce. The
    /// flame burns at (23.8, 3.2) of the art, (1.8, 40.8) from its anchor, as the census finds it.
    #[test]
    fn a_sconces_flame_is_found() {
        const WIDTH: u32 = 44;
        const HEIGHT: u32 = 23;
        let mut texels = vec![((23, 2), FLAME_ORANGE), ((23, 3), FLAME_YELLOW), ((24, 3), FLAME_ORANGE)];
        texels.extend([((23, 4), FLAME_PALE_CAP), ((21, 13), GOLD), ((22, 15), BRASS), ((22, 16), BRASS)]);
        let mut rgba = painted_art(WIDTH, HEIGHT, [0; 4], &texels);
        for y in 0..HEIGHT {
            rgba[(y * WIDTH + 30) as usize * RGBA_TEXEL_BYTES..][..RGBA_TEXEL_BYTES].copy_from_slice(&IRON);
        }
        let rect = art_opaque_rect(&rgba, WIDTH).expect("the sconce has texels");
        let flame = art_flame_centroid(&rgba, WIDTH, 1, rect).expect("the sconce has a flame");
        assert!(flame.abs_diff_eq(Vec2::new(23.0 + 1.0 / 3.0 + 0.5, 2.0 + 2.0 / 3.0 + 0.5), EPSILON), "{flame}");
        let art = client_light_sprite_art(&rgba, WIDTH, 1, (-((WIDTH / 2) as i16), 0), 44.0).expect("art");
        let anchor_right = std::f32::consts::FRAC_1_SQRT_2;
        let shown = art.flame.map(|flame| {
            Vec2::new((flame.x - anchor_right) / CC_WORLD_XZ_PER_PIXEL, flame.y / CC_WORLD_Y_PER_PIXEL)
        });
        assert!(shown.is_some_and(|shown| shown.abs_diff_eq(Vec2::new(1.8, 40.8), PIXEL_EPSILON)), "{shown:?}");
        // Upscaled twice, the same flame burns at the same logical point.
        let upscaled_texels = texels
            .iter()
            .flat_map(|&((x, y), colour)| {
                [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| ((2 * x + dx, 2 * y + dy), colour))
            })
            .collect::<Vec<_>>();
        let upscaled = painted_art(2 * WIDTH, 2 * HEIGHT, [0; 4], &upscaled_texels);
        let upscaled_rect = art_opaque_rect(&upscaled, 2 * WIDTH).expect("texels");
        let upscaled_flame = art_flame_centroid(&upscaled, 2 * WIDTH, 2, upscaled_rect).expect("flame");
        assert!(upscaled_flame.abs_diff_eq(flame * 2.0, EPSILON), "{upscaled_flame}");
    }

    /// No flame: a dim animation frame's flame without a hot core (sconce 2564), warm texels in groups under three
    /// (a pentagram's ring), and flames that fill the art both ways (lava, a campfire's blaze: the item glows
    /// whole). A candelabra's three flames burn at their centroid.
    #[test]
    fn small_dim_or_whole_glows_are_no_flame() {
        const WIDTH: u32 = 20;
        const HEIGHT: u32 = 20;
        let background = [IRON[0], IRON[1], IRON[2], u8::MAX];
        let flame_of = |texels: &[((u32, u32), [u8; 3])]| {
            let rgba = painted_art(WIDTH, HEIGHT, background, texels);
            art_flame_centroid(&rgba, WIDTH, 1, [0, 0, WIDTH, HEIGHT])
        };
        let dim = [((5, 2), FLAME_DIM_ORANGE), ((5, 3), FLAME_DIM_YELLOW), ((6, 3), FLAME_DIM_ORANGE)];
        assert_eq!(flame_of(&dim), None);
        let ring = [((2, 2), FLAME_ORANGE), ((3, 3), FLAME_YELLOW), ((10, 15), FLAME_ORANGE), ((17, 8), FLAME_YELLOW)];
        assert_eq!(flame_of(&ring), None);
        let blaze = (0..HEIGHT)
            .flat_map(|y| (0..WIDTH).map(move |x| ((x, y), if (x + y) % 2 == 0 { FLAME_ORANGE } else { FLAME_YELLOW })))
            .filter(|&((x, y), _)| (2..18).contains(&x) && (3..15).contains(&y))
            .collect::<Vec<_>>();
        assert_eq!(flame_of(&blaze), None);
        let candelabra = [2, 9, 16]
            .into_iter()
            .flat_map(|x| [((x, 2), FLAME_ORANGE), ((x, 3), FLAME_YELLOW), ((x + 1, 3), FLAME_ORANGE)])
            .collect::<Vec<_>>();
        let flame = flame_of(&candelabra).expect("the candelabra has flames");
        assert!(flame.abs_diff_eq(Vec2::new(9.0 + 1.0 / 3.0 + 0.5, 2.0 + 2.0 / 3.0 + 0.5), EPSILON), "{flame}");
    }

    /// Lights the clients already show on their item stay: window 14's beam, whose pane shows on the window, lamp
    /// post 2848's light, whose lamp shows on the post's top even where its glass shows as a flame (the textures with
    /// a source of their own keep it), and a light of the item's own without art.
    #[test]
    fn lights_on_their_art_stay() {
        let window =
            TileMetaItemLight { tile_id: 14, light_texture_entry: 3, scale: 1.0, offset_x: -1.35, offset_y: -3.0 };
        let (clients, placed, _, _) = shown_light(&window, classic_art(44, 106, [0, 1, 25, 105], None));
        assert!(clients.abs_diff_eq(Vec2::new(-9.1, 46.9), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(clients, EPSILON), "{placed}");
        let lamp_post =
            TileMetaItemLight { tile_id: 2848, light_texture_entry: 2, scale: 2.0, offset_x: 0.0, offset_y: -5.5 };
        let lamp_post_art = classic_art(22, 100, [1, 0, 20, 100], Some(LAMP_POST_GLASS));
        let (clients, placed, _, _) = shown_light(&lamp_post, lamp_post_art);
        assert!(clients.abs_diff_eq(Vec2::new(-2.9, 88.3), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(clients, EPSILON), "{placed}");
        let (clients, placed, _, _) = shown_light(&LANTERN_LIGHT, None);
        assert!(placed.abs_diff_eq(clients, EPSILON), "{placed}");
    }

    /// The default offsets put the light on the art's centre, or without art on the cell centre's, where the light on
    /// the ground under the sprite lands.
    #[test]
    fn the_default_offsets_put_the_light_on_the_arts_centre() {
        let lava =
            TileMetaItemLight { tile_id: 4846, light_texture_entry: 1, scale: 2.0, offset_x: -5.0, offset_y: -4.0 };
        // Over window 14's art, -22..3 x 1..105.
        let (clients, placed, _, _) = shown_light(&lava, classic_art(44, 106, [0, 1, 25, 105], None));
        assert!(clients.abs_diff_eq(Vec2::new(-148.2, 46.2), PIXEL_EPSILON), "{clients}");
        assert!(placed.abs_diff_eq(Vec2::new(-9.5, 53.0), PIXEL_EPSILON), "{placed}");
        for art in [None, classic_art(44, 44, [0, 0, 44, 44], None)] {
            let (_, placed, center, key) = shown_light(&lava, art);
            assert!(placed.abs_diff_eq(Vec2::new(0.0, 22.0), PIXEL_EPSILON), "{placed}");
            let cell = client_light_sprite_cell_centre(key.tile_x, key.tile_y, key.z);
            assert!(ground_under(center, cell.y).abs_diff_eq(cell, EPSILON), "{center}");
        }
    }

    /// A centred lava light: the light on the ground lands on the lava's cell centre, as large as the sprite.
    #[test]
    fn a_centred_sprites_land_light_lands_on_its_cell() {
        const LAVA_SCALE: f32 = 1.5;
        let lava =
            TileMetaItemLight { tile_id: 4846, light_texture_entry: 1, scale: 2.0, offset_x: -5.0, offset_y: -4.0 };
        let (_, center, key) = glow_distance_from_cell(&lava, LIGHT_GENERIC_SIDE);
        let bounds = client_light_sprite_bounds(&lava, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        let (_, size) = client_light_sprite_quad(Vec3::ZERO, bounds);
        let size = size * LAVA_SCALE;
        let sprite = ClientLightSpriteInstance {
            key,
            texture_entry: lava.light_texture_entry,
            center: client_light_sprite_centred(center, size, key, lava.light_texture_entry, None),
            size,
            glow_rgb: [1.0; 3],
            ceiling: None,
        };
        let light = client_light_sprite_static_light(&sprite);
        assert!((light.world_x - (key.tile_x as f32 + TILE_CENTRE_OFFSET)).abs() < EPSILON, "{light:?}");
        assert!((light.world_z - (key.tile_y as f32 + TILE_CENTRE_OFFSET)).abs() < EPSILON, "{light:?}");
        let side_tiles = LAVA_SCALE * 2.0 * LIGHT_GENERIC_SIDE as f32 / 66.0;
        assert!((light.width_world - side_tiles).abs() < EPSILON, "{light:?}");
    }

    /// The opaque box spans the texels with alpha; art of less than two pixels either way places no light (fire
    /// column 14119's one-pixel stand-in).
    #[test]
    fn the_art_box_spans_the_visible_texels() {
        const WIDTH: u32 = 4;
        const HEIGHT: u32 = 3;
        let mut rgba = vec![0u8; (WIDTH * HEIGHT) as usize * RGBA_TEXEL_BYTES];
        for (x, y) in [(1u32, 0u32), (2, 2)] {
            rgba[(y * WIDTH + x) as usize * RGBA_TEXEL_BYTES + 3] = 1;
        }
        assert_eq!(art_opaque_rect(&rgba, WIDTH), Some([1, 0, 3, 3]));
        assert_eq!(art_opaque_rect(&[0u8; RGBA_TEXEL_BYTES * 2], WIDTH), None);
        assert!(classic_art_bounds(80, 140, [79, 0, 80, 1]).is_none());
        // An upscaled art's box is its logical pixels'.
        let upscaled = client_light_sprite_art_bounds([40, 0, 66, 46], 2, (-22, 0), 44.0);
        assert_eq!(upscaled, classic_art_bounds(44, 44, [20, 0, 33, 23]));
        // The tile's centre shows 22 pixels above its near vertex, where its cell centre shows.
        let tile = client_light_sprite_tile_bounds();
        let centre_y = (tile.local_min_y + tile.local_max_y) * 0.5 / CC_WORLD_Y_PER_PIXEL;
        assert!((centre_y - CC_TILE_PIXEL_WIDTH / 2.0).abs() < EPSILON, "{tile:?}");
    }

    #[test]
    fn the_reach_is_the_largest_bound() {
        let bounds = client_light_sprite_bounds(&LANTERN_LIGHT, LIGHT_GENERIC_SIDE, LIGHT_GENERIC_SIDE);
        assert!((client_light_sprite_reach(bounds) - 6.428).abs() < EPSILON);
    }
}
