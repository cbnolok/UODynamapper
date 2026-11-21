//! The mobiles' sprites: each frame drawn an instance of the statics' art sprite shader, from the mobiles' own frame
//! atlas and instance buffer. Three material instances copy the statics' opaque, transparent and projected-shadow
//! ones, so a mobile takes the same lighting, hues and shadows as the art around it; theirs compile the shader with
//! `MOBILE_FRAMES`, which reads each instance's crossfade beside it (in_between.rs).

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bytemuck::{Pod, Zeroable};
use crate::render::statics::static_batches::{SpriteInstance, CLIENT_SHADOW_FLAG_CASTS, STATIC_HUE_FLAG_EC_SPRITE};
use crate::render::lighting::static_lights::RenderStaticLightInstances;
use crate::render::scene::SceneStateData;
use crate::render::statics::statics_collect::static_art_billboard_local_light_corners;
use crate::render::statics::static_placement::{apply_cc_static_world_anchor_translation, StaticDepthClass,
    STATIC_ART_Y_BIAS};
use crate::settings::Settings;
use crate::render::statics::statics_draw::{build_art_batch_mesh, ArtSpriteMaterial, ArtSpriteRenderAssets};
use crate::world::mobile_motion::MobileMotion;
use super::clips::{AnimSource, AtlasEntry, ClipKey, MobileClips};
use super::frame_atlas::MobileFrameAtlas;
use super::in_between::{layer_frames, FrameCrossfade, LayerFrames};
use super::placement::mobile_card_bounds;
use super::rife_in_betweens::MobileInBetweens;
use super::MobileClipChoice;
use super::pace::MobileCycleClock;

/// The most frames drawn at once: a mount and its rider for each mobile, here the player alone.
pub const MAX_MOBILE_INSTANCES: u32 = 2;
/// A mobile's priority over its cell's z: ClassicUO's mobile `PriorityZ` is its z plus one.
const MOBILE_PRIORITY_Z_OFFSET: f32 = 1.0;
/// The tie levels of a mount and of its rider, over the statics of the same cell and priority (1..8 there; the
/// Classic client adds a mobile after the items of its cell): the rider over its mount.
const MOUNT_TIE_LEVEL: u32 = 7;
const RIDER_TIE_LEVEL: u32 = 8;
/// The pixels of the frames per Classic screen pixel: the packages hold them at the client's size.
const MOBILE_PIXEL_RATIO: f32 = 1.0;
const OPAQUE_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A mobile frame's instance, as the art sprite shader reads it with `MOBILE_FRAMES` (art/art_bindings.wesl): the
/// statics' sprite instance, then the crossfade of its frames.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MobileSpriteInstance {
    pub sprite: SpriteInstance,
    pub crossfade: FrameCrossfade,
}

/// What a mobile looks like: its body, and the body of its mount, if it rides one, and the actions they show.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MobileAppearance {
    pub body: u32,
    pub mount_body: Option<u32>,
}

/// The actions a mobile's body and mount show, from the clients' tables (`MobileClipChoice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobileActions {
    pub body_action: u16,
    pub mount_action: Option<u16>,
}

/// The mobiles' draw state: their materials, instance buffer and draw entities.
#[derive(Resource)]
pub struct MobileDrawAssets {
    opaque: Handle<ArtSpriteMaterial>,
    transparent: Handle<ArtSpriteMaterial>,
    shadow: Handle<ArtSpriteMaterial>,
    instances: Handle<ShaderBuffer>,
    mesh: Handle<Mesh>,
    entities: Vec<Entity>,
    drawn: u32,
}

/// Creates the mobiles' atlas, materials and buffer, once the statics' materials exist to copy.
pub fn sys_setup_mobile_draw(
    mut commands: Commands,
    existing: Option<Res<MobileDrawAssets>>,
    art_assets: Option<Res<ArtSpriteRenderAssets>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if existing.is_some() {
        return;
    }
    let Some(art_assets) = art_assets else { return };
    let atlas = MobileFrameAtlas::<AtlasEntry>::new(&mut images);
    let instances = storage_buffers.add(ShaderBuffer::from(vec![MobileSpriteInstance::zeroed(); MAX_MOBILE_INSTANCES as usize]));
    let mut copy = |handle: &Handle<ArtSpriteMaterial>| {
        let mut material = materials.get(handle).expect("the statics' sprite materials exist").clone();
        material.extension.atlas = atlas.image().clone();
        material.extension.atlas_enhanced = atlas.image().clone();
        material.extension.instances = instances.clone();
        material.extension.mobile_frames = true;
        materials.add(material)
    };
    let assets = MobileDrawAssets {
        opaque: copy(&art_assets.opaque_material),
        transparent: copy(&art_assets.transparent_material),
        shadow: copy(&art_assets.shadow_material),
        instances,
        mesh: meshes.add(build_art_batch_mesh(0)),
        entities: Vec::new(),
        drawn: 0,
    };
    commands.insert_resource(atlas);
    commands.insert_resource(assets);
}

/// Keeps the mobiles' materials on the statics' parameters and uniforms. A material is taken mutably only when it
/// differs: a mutable access sends it to the GPU again.
pub fn sys_sync_mobile_materials(
    art_assets: Option<Res<ArtSpriteRenderAssets>>,
    draw_assets: Option<Res<MobileDrawAssets>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
) {
    let (Some(art_assets), Some(draw_assets)) = (art_assets, draw_assets) else { return };
    for (source, target) in [
        (&art_assets.opaque_material, &draw_assets.opaque),
        (&art_assets.transparent_material, &draw_assets.transparent),
        (&art_assets.shadow_material, &draw_assets.shadow),
    ] {
        let (Some(source), Some(current)) = (materials.get(source), materials.get(target)) else { continue };
        if !shared_state_differs(source, current) {
            continue;
        }
        let mut synced = source.clone();
        synced.extension.atlas = current.extension.atlas.clone();
        synced.extension.atlas_enhanced = current.extension.atlas_enhanced.clone();
        synced.extension.instances = current.extension.instances.clone();
        synced.extension.mobile_frames = current.extension.mobile_frames;
        if let Some(mut target) = materials.get_mut(target) {
            *target = synced;
        }
    }
}

fn shared_state_differs(source: &ArtSpriteMaterial, target: &ArtSpriteMaterial) -> bool {
    let (a, b) = (&source.extension, &target.extension);
    let params_differ = a.params.render_mode != b.params.render_mode
        || a.params.pass_mode != b.params.pass_mode
        || a.params.hue_enabled != b.params.hue_enabled
        || a.params.alpha_cutoff != b.params.alpha_cutoff
        || a.params.map_width_tiles != b.params.map_width_tiles
        || a.params.map_height_tiles != b.params.map_height_tiles;
    params_differ
        || bytemuck::bytes_of(&a.scene_uniform) != bytemuck::bytes_of(&b.scene_uniform)
        || a.effects_uniform != b.effects_uniform
        || a.global_lighting_uniform != b.global_lighting_uniform
        || a.hues != b.hues
        || a.item_surface_atlas != b.item_surface_atlas
        || a.item_surface_lookup != b.item_surface_lookup
        || a.client_grunge_texture != b.client_grunge_texture
        || source.base.alpha_mode != target.base.alpha_mode
}

/// One frame to draw: its clip's current frame at a mobile's place.
struct DrawnFrame {
    key: ClipKey,
    tie_level: u32,
    /// The anchor's offset in screen pixels, x right and y down: a rider's on its mount.
    offset_px: (f32, f32),
}

/// Builds the instances of the mobiles' current frames, and the draw entities for them.
#[allow(clippy::too_many_arguments)]
pub fn sys_draw_mobiles(
    mut commands: Commands,
    time: Res<Time>,
    mut mobiles_q: Query<(&Transform, &MobileMotion, &MobileAppearance, &MobileClipChoice, &mut MobileCycleClock)>,
    draw_assets: Option<ResMut<MobileDrawAssets>>,
    atlas: Option<ResMut<MobileFrameAtlas<AtlasEntry>>>,
    mut clips: ResMut<MobileClips>,
    mut in_betweens: ResMut<MobileInBetweens>,
    packages: Res<super::MobileAnimPackages>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<Settings>,
    scene_state: Res<SceneStateData>,
    static_lights: Option<Res<RenderStaticLightInstances>>,
) {
    let (Some(mut draw_assets), Some(mut atlas)) = (draw_assets, atlas) else { return };
    let now = time.elapsed_secs_f64();
    // The static lights on the cards, as on the statics' when that option is on.
    let local_lights = static_lights.as_deref()
        .filter(|_| settings.world_rendering.enable_static_art_local_lights)
        .map(|lights| (lights, scene_state.map_id));
    let in_between_mode = settings.world_rendering.mobile_in_between_frames;
    let evicted = clips.place_decoded(&mut atlas, now);
    in_betweens.forget(&evicted);

    let mut instances = Vec::with_capacity(MAX_MOBILE_INSTANCES as usize);
    for (transform, motion, appearance, choice, mut clock) in mobiles_q.iter_mut() {
        let (Some(source), Some(actions)) = (choice.source, choice.actions) else { continue };
        let mut frames = Vec::with_capacity(2);
        if let (Some(mount_body), Some(mount_action)) = (appearance.mount_body, actions.mount_action) {
            frames.push(DrawnFrame {
                key: ClipKey { source, body: mount_body, action: mount_action, direction: motion.facing },
                tie_level: MOUNT_TIE_LEVEL,
                offset_px: choice.mount_offset_px,
            });
        }
        frames.push(DrawnFrame {
            key: ClipKey { source, body: appearance.body, action: actions.body_action, direction: motion.facing },
            tie_level: RIDER_TIE_LEVEL,
            offset_px: choice.body_offset_px,
        });
        // The mount comes first, so its clip can lead the cycle the rider shares.
        let mut cycles = None;
        for drawn in frames {
            if instances.len() >= MAX_MOBILE_INSTANCES as usize {
                break;
            }
            let Some(clip) = clips.get_or_load(drawn.key, &packages.0) else { continue };
            // With a RIFE mode, the clip's sequence with its in-betweens once they are made, mirrored as the clip;
            // the clip's own page stays in use, as the sequence's frames come from it.
            let mirrored = clip.mirrored;
            if let Some(frame) = clip.frames.first() {
                atlas.touch(frame.rect.layer, now);
            }
            let clip = in_betweens.shown(drawn.key, clip, in_between_mode, now).unwrap_or(clip);
            let pick = match choice.cycle.lead_secs(clip) {
                Some(lead_secs) => clip.frame_at_cycles(
                    *cycles.get_or_insert_with(|| clock.advance(motion.clip_secs(), lead_secs))),
                None => clip.frame_at(motion.clip_secs()),
            };
            let Some(frames) = pick.and_then(|pick| layer_frames(in_between_mode, clip, pick)) else { continue };
            atlas.touch(frames.layer, now);
            // Both frames of an in-between come from the clip, so they share its mirroring.
            instances.push(mobile_instance(transform.translation, frames, mirrored, drawn.offset_px,
                drawn.tie_level, source, local_lights));
        }
    }

    let count = instances.len() as u32;
    if count > 0 {
        let _ = storage_buffers.insert(&draw_assets.instances, ShaderBuffer::from(instances));
    }
    if count != draw_assets.drawn {
        draw_assets.mesh = meshes.add(build_art_batch_mesh(count));
        draw_assets.drawn = count;
        for entity in draw_assets.entities.drain(..) {
            commands.entity(entity).despawn();
        }
        if count > 0 {
            let materials = [draw_assets.opaque.clone(), draw_assets.transparent.clone(), draw_assets.shadow.clone()];
            draw_assets.entities = materials.into_iter().map(|material| commands.spawn((
                Mesh3d(draw_assets.mesh.clone()),
                MeshTag::new(0),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                NoFrustumCulling,
            )).id()).collect();
        }
    }
}

/// The instance of `frames` at a mobile's position `translation` (world x and z the UO x and y, world y its z), lit by
/// `local_lights` of a map when given.
fn mobile_instance(
    translation: Vec3,
    frames: LayerFrames,
    mirrored: bool,
    offset_px: (f32, f32),
    tie_level: u32,
    source: AnimSource,
    local_lights: Option<(&RenderStaticLightInstances, u32)>,
) -> MobileSpriteInstance {
    let (world_x, world_z) = apply_cc_static_world_anchor_translation(translation.x, translation.z);
    let bounds = mobile_card_bounds(frames.placement, mirrored, offset_px, MOBILE_PIXEL_RATIO);
    let priority_z_units = crate::world::coords::scale_bevy_units_to_uo_z(translation.y).round()
        + MOBILE_PRIORITY_Z_OFFSET;
    let sprite = SpriteInstance {
        world_x,
        world_z,
        world_y: translation.y + STATIC_ART_Y_BIAS,
        layer: frames.layer,
        depth_class: StaticDepthClass::Regular.encoded(),
        base_world_y: translation.y,
        uv_min: frames.uv_min.to_array(),
        uv_max: frames.uv_max.to_array(),
        local_min: [bounds.local_min_x, bounds.local_min_y],
        local_max: [bounds.local_max_x, bounds.local_max_y],
        tile_x: world_x,
        tile_y: world_z,
        priority_z_units,
        sort_bias_ordinal: tie_level,
        is_wet_flags: 0,
        hue_id: 0,
        hue_flags: if source == AnimSource::Enhanced { STATIC_HUE_FLAG_EC_SPRITE } else { 0 },
        flat_quad: 0,
        texture_stretch: 0.0,
        material_item_id: 0,
        client_shadow_flags: CLIENT_SHADOW_FLAG_CASTS,
        client_shadow_height: 0.0,
        client_grunge_anchor: [world_x, world_z],
        local_light_corners: local_lights.map_or([0; 4], |(lights, map_id)| static_art_billboard_local_light_corners(
            true, lights, map_id, world_x, world_z, translation.y, bounds, false)),
        color_rgba: OPAQUE_COLOR,
    };
    MobileSpriteInstance { sprite, crossfade: frames.crossfade }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_reads_a_mobile_instance_as_wgsl_lays_it_out() {
        // A SpriteInstance (144 bytes, aligned to its vec4) then a FrameCrossfade (48), on a 16-byte stride.
        const WGSL_SPRITE_INSTANCE_SIZE: usize = 144;
        const WGSL_MOBILE_SPRITE_INSTANCE_SIZE: usize = 192;
        assert_eq!(std::mem::size_of::<SpriteInstance>(), WGSL_SPRITE_INSTANCE_SIZE);
        assert_eq!(std::mem::size_of::<MobileSpriteInstance>(), WGSL_MOBILE_SPRITE_INSTANCE_SIZE);
    }
}
