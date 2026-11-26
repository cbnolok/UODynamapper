//! The mobiles' sprites: each frame drawn an instance of the statics' art sprite shader, from the mobiles' own frame
//! atlas and instance buffer. Three material instances copy the statics' opaque, transparent and projected-shadow
//! ones, so a mobile takes the same lighting, hues and shadows as the art around it; theirs compile the shader with
//! `MOBILE_FRAMES`, which reads each instance's crossfade and its body's frame filter beside it (in_between.rs,
//! sharpening.rs). The materials bind the RGBA8 frame
//! atlas as the sprites' first atlas and the BC7 one as the second (`atlas_enhanced`), each in both places when it is
//! the only one; an instance of a BC7 frame names its page with the second atlas's bit (`AtlasRect::instance_layer`).

use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bytemuck::{Pod, Zeroable};
use crate::render::statics::static_batches::{cave_instance_flags, SpriteInstance, CLIENT_SHADOW_FLAG_CASTS,
    STATIC_HUE_FLAG_EC_SPRITE};
use crate::render::lighting::static_lights::RenderStaticLightInstances;
use crate::render::scene::SceneStateData;
use crate::render::statics::statics_collect::static_art_billboard_local_light_corners;
use crate::render::statics::static_placement::{apply_cc_static_world_anchor_translation, StaticDepthClass,
    STATIC_ART_Y_BIAS};
use crate::settings::Settings;
use crate::render::statics::statics_draw::{build_art_batch_mesh, ArtSpriteMaterial, ArtSpriteRenderAssets};
use crate::world::caves::CavesRes;
use crate::world::mobile_motion::MobileMotion;
use crate::world::player::Player;
use super::clips::{AnimSource, AtlasEntry, ClipKey, MobileClips};
use super::frame_atlas::{AtlasFormat, MobileFrameAtlas};
use super::in_between::{layer_frames, FrameCrossfade, LayerFrames};
use super::placement::mobile_card_bounds;
use super::rife_in_betweens::MobileInBetweens;
use super::sharpening::{MobileFrameFilter, MobileSharpening};
use super::upscales::{atlas_pages_wanted, upscale_key, MobileUpscale, MobileUpscaleGate, MobileUpscaleSwitches,
    MobileUpscales, UpscalePriority, UpscaleSwitch};
use super::MobileClipChoice;
use super::pace::MobileCycleClock;
use super::ceiling::MobileCeilingCache;
use crate::world::z_view::ZViewLimit;

/// The most frames drawn at once: a mount and its rider for each mobile, and the parts of an upscaled frame wider than
/// a page. The player is the one mobile but for those the control channel spawns for tests (`spawn_mobile`).
pub const MAX_MOBILE_INSTANCES: u32 = 64;
/// A mobile's priority over its cell's z: ClassicUO's mobile `PriorityZ` is its z plus one.
const MOBILE_PRIORITY_Z_OFFSET: f32 = 1.0;
/// The tie levels of a mount and of its rider, over the statics of the same cell and priority (1..8 there; the
/// Classic client adds a mobile after the items of its cell): the rider over its mount.
const MOUNT_TIE_LEVEL: u32 = 7;
const RIDER_TIE_LEVEL: u32 = 8;
/// The pixels of the frames per Classic screen pixel: the packages hold them at the client's size.
const MOBILE_PIXEL_RATIO: f32 = 1.0;
/// How far into a cell a sliding mobile must be for its depth to take that cell: below this, the float error of a
/// mobile at rest on its cell.
const DEPTH_CELL_SLIDE_EPSILON: f32 = 1.0 / 1024.0;
const OPAQUE_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// A mobile frame's instance, as the art sprite shader reads it with `MOBILE_FRAMES` (art/art_bindings.wesl): the
/// statics' sprite instance, the crossfade of its frames, the ceiling drawn over it and its body's frame filter.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MobileSpriteInstance {
    pub sprite: SpriteInstance,
    pub crossfade: FrameCrossfade,
    pub ceiling: MobileCeiling,
    pub frame_filter: MobileFrameFilter,
}

/// The roof or upper floor drawn over a mobile (ceiling.rs `MobileCeilingCache`), as the art sprite shader reads it
/// (art/depth.wesl `mobile_fragment_view_z`): the z of its plane, when `present` is not 0. The mobile's pixels above
/// it are ordered under it there, so the roof tiles cover a head poking through them.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MobileCeiling {
    pub z: f32,
    pub present: u32,
    pub _pad: [u32; 2],
}

impl MobileCeiling {
    /// The ceiling at the plane z `plane_z`, or none.
    pub fn at(plane_z: Option<i32>) -> Self {
        plane_z.map_or_else(Self::default, |z| Self { z: z as f32, present: 1, _pad: [0; 2] })
    }
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
    packages: Res<super::MobileAnimPackages>,
) {
    if existing.is_some() {
        return;
    }
    let Some(art_assets) = art_assets else { return };
    let atlas = MobileFrameAtlas::<AtlasEntry>::new(&mut images, packages.0.formats());
    let [first, second] = [[AtlasFormat::Rgba8, AtlasFormat::Bc7], [AtlasFormat::Bc7, AtlasFormat::Rgba8]]
        .map(|order| order.into_iter().find_map(|format| atlas.image(format)).expect("an atlas exists").clone());
    let instances = storage_buffers.add(ShaderBuffer::from(vec![MobileSpriteInstance::zeroed(); MAX_MOBILE_INSTANCES as usize]));
    let mut copy = |handle: &Handle<ArtSpriteMaterial>| {
        let mut material = materials.get(handle).expect("the statics' sprite materials exist").clone();
        material.extension.atlas = first.clone();
        material.extension.atlas_enhanced = second.clone();
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawnFrame {
    key: ClipKey,
    tie_level: u32,
    /// The anchor's offset in screen pixels, x right and y down: a rider's on its mount.
    offset_px: (f32, f32),
}

/// The layers a mobile drew last. It keeps showing them while the clips of its new ones decode (a facing or an action
/// shown for the first time, or again after the atlas dropped it), so that neither the mount nor its rider drops out
/// for those frames; the mount's clip often lands a few frames before the rider's.
#[derive(Component, Debug, Default)]
pub struct MobileShownLayers {
    layers: Vec<DrawnFrame>,
    /// Whether each layer shows its 2x frames (upscales.rs).
    upscales: MobileUpscaleSwitches,
}

/// The layers to draw of `wanted`: while one of their clips is `loading`, the layers shown before if theirs are all
/// `resident`; else `wanted`, which become the layers shown.
fn layers_to_draw(wanted: Vec<DrawnFrame>, shown: &mut MobileShownLayers, loading: impl Fn(ClipKey) -> bool,
    resident: impl Fn(ClipKey) -> bool) -> Vec<DrawnFrame> {
    let held = !shown.layers.is_empty() && shown.layers.iter().all(|layer| resident(layer.key));
    if held && wanted.iter().any(|layer| loading(layer.key)) {
        return shown.layers.clone();
    }
    shown.layers.clone_from(&wanted);
    wanted
}

/// Gives the frame atlases the upscaled pages the setting wants (upscales.rs). An atlas remade is empty: every clip,
/// in-between and upscale is forgotten, and the mobiles' materials go to the GPU again to bind the new textures.
#[allow(clippy::too_many_arguments)]
pub fn sys_size_mobile_upscale_pages(
    settings: Res<Settings>,
    atlas: Option<ResMut<MobileFrameAtlas<AtlasEntry>>>,
    draw_assets: Option<Res<MobileDrawAssets>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ArtSpriteMaterial>>,
    mut clips: ResMut<MobileClips>,
    mut in_betweens: ResMut<MobileInBetweens>,
    mut upscales: ResMut<MobileUpscales>,
) {
    let (Some(mut atlas), Some(draw_assets)) = (atlas, draw_assets) else { return };
    let pages = atlas_pages_wanted(&settings);
    if atlas.upscaled_pages() == pages {
        return;
    }
    atlas.set_upscaled_pages(&mut images, pages);
    clips.clear();
    in_betweens.clear();
    upscales.clear();
    for material in [&draw_assets.opaque, &draw_assets.transparent, &draw_assets.shadow] {
        if let Some(material) = materials.get_mut(material) {
            material.into_inner();
        }
    }
    info!("The mobiles' frame atlases are remade with {pages} pages for upscaled frames.");
}

/// Builds the instances of the mobiles' current frames, and the draw entities for them.
#[allow(clippy::too_many_arguments)]
pub fn sys_draw_mobiles(
    mut commands: Commands,
    time: Res<Time>,
    mut mobiles_q: Query<(&Transform, &MobileMotion, &MobileAppearance, &MobileClipChoice, &mut MobileCycleClock,
        &mut MobileShownLayers, &MobileCeilingCache)>,
    draw_assets: Option<ResMut<MobileDrawAssets>>,
    atlas: Option<ResMut<MobileFrameAtlas<AtlasEntry>>>,
    mut clips: ResMut<MobileClips>,
    mut in_betweens: ResMut<MobileInBetweens>,
    mut upscales: ResMut<MobileUpscales>,
    upscale_gate: Res<MobileUpscaleGate>,
    player_q: Query<&Transform, With<Player>>,
    packages: Res<super::MobileAnimPackages>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    settings: Res<Settings>,
    (scene_state, caves, mut sharpening, z_view): (Res<SceneStateData>, Res<CavesRes>, ResMut<MobileSharpening>,
        Res<ZViewLimit>),
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
    upscales.forget(&evicted);
    let upscaling = settings.world_rendering.mobile_upscale != MobileUpscale::Off && upscale_gate.open;
    let player = player_q.iter().next().map(|transform| transform.translation);
    if packages.is_changed() {
        sharpening.set_packages();
    }

    let mut instances = Vec::with_capacity(MAX_MOBILE_INSTANCES as usize);
    for (transform, motion, appearance, choice, mut clock, mut shown, ceiling) in mobiles_q.iter_mut() {
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
        for drawn in &frames {
            clips.get_or_load(drawn.key, &packages.0);
        }
        let frames = layers_to_draw(frames, &mut shown, |key| clips.loading(key), |key| clips.resident(key).is_some());
        let cave_flags = mobile_cave_flags(&caves, transform.translation);
        // The roof or upper floor over it, while the z-view's cut draws it.
        let ceiling = MobileCeiling::at(ceiling.drawn_plane_z(&z_view.0));
        // The upscales of the nearest the player go first.
        let distance = player.map_or(0.0, |player| transform.translation.xz().distance(player.xz()));
        let mut switches = Vec::with_capacity(frames.len());
        // The mount comes first, so its clip can lead the cycle the rider shares.
        let mut cycles = None;
        for drawn in frames {
            if instances.len() >= MAX_MOBILE_INSTANCES as usize {
                break;
            }
            let Some(clip) = clips.get_or_load(drawn.key, &packages.0) else { continue };
            // With a RIFE mode, the clip's sequence with its in-betweens once they are made, mirrored as the clip;
            // the clip's own pages stay in use, as the sequence's frames come from them.
            let mirrored = clip.mirrored;
            clip.touch_pages(&mut atlas, now);
            // A clip whose RIFE sequence is still to come upscales once it is there.
            let rife_waits = in_betweens.waits(drawn.key, clip, in_between_mode);
            let base = clip;
            let (clip, factor) = match in_betweens.shown(drawn.key, base, in_between_mode, now) {
                Some(sequence) => (sequence, in_between_mode.rife_factor().unwrap_or(1)),
                None => (base, 1),
            };
            let pick = match choice.cycle.lead_secs(clip) {
                Some(lead_secs) => clip.frame_at_cycles(
                    *cycles.get_or_insert_with(|| clock.advance(motion.clip_secs(), lead_secs))),
                None => clip.frame_at(motion.clip_secs()),
            };
            let Some(pick) = pick else { continue };
            // The sequence's 2x frames once all are made, switched to and from as it starts over.
            let key = upscale_key(drawn.key, base, factor);
            let wanted = upscaling && !rife_waits
                && upscales.body_upscales(drawn.key, &packages.0, in_between_mode.rife_factor().is_some(),
                    settings.world_rendering.mobile_upscale);
            if wanted {
                let placement = clip.frames[pick.index].placement;
                let area = placement.width * placement.height * upscale_gate.magnification.powi(2);
                upscales.want(key, drawn.key, UpscalePriority { distance, area }, now);
            }
            let upscaled = upscales.ready(&key);
            let switch = UpscaleSwitch::next(shown.upscales.get(drawn.tie_level), drawn.tie_level, key,
                clip.frames.len(), pick.index, wanted, upscaled.is_some());
            switches.push(switch);
            if wanted && let Some(parts) = upscaled {
                parts.iter().for_each(|part| part.touch_pages(&mut atlas, now));
            }
            // The 2x frames come in parts side by side when wider than a page: a card each.
            let shown = upscaled.filter(|_| switch.upscaled).unwrap_or(std::slice::from_ref(clip));
            for clip in shown {
                if instances.len() >= MAX_MOBILE_INSTANCES as usize {
                    break;
                }
                let Some(frames) = layer_frames(in_between_mode, clip, pick) else { continue };
                clip.touch_pages(&mut atlas, now);
                // The body's filter; an outline's card has room for it around the frames.
                let filter = sharpening.filter(source, drawn.key.body, &packages.0);
                let frames = frames.padded(filter.padding_texels, clip.texel_scale);
                // Both frames of an in-between come from the clip, so they share its mirroring.
                instances.push(mobile_instance(transform.translation, frames, filter, mirrored, drawn.offset_px,
                    drawn.tie_level, source, cave_flags, ceiling, local_lights));
            }
        }
        shown.upscales = MobileUpscaleSwitches(switches);
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

/// The cave bits of the instances of a mobile at `translation` (`cave_instance_flags`), from the cell it is ordered
/// from: on a cave's floor it is lifted with the floor, which would otherwise be drawn in front of it and cover it
/// whole; outside the player's cave it is darkened with what is around it.
fn mobile_cave_flags(caves: &CavesRes, translation: Vec3) -> u32 {
    let (x, y) = mobile_depth_cell(translation.x, translation.z);
    let z = crate::world::coords::scale_bevy_units_to_uo_z(translation.y).round() as i8;
    cave_instance_flags(caves.mobile_view(x as i32, y as i32, z), caves.player().is_some())
}

/// The instance of `frames` at a mobile's position `translation` (world x and z the UO x and y, world y its z), with
/// the cave bits `cave_flags` (`mobile_cave_flags`) and the `ceiling` over it, lit by `local_lights` of a map when
/// given, filtered by `filter`.
#[allow(clippy::too_many_arguments)]
fn mobile_instance(
    translation: Vec3,
    frames: LayerFrames,
    filter: MobileFrameFilter,
    mirrored: bool,
    offset_px: (f32, f32),
    tie_level: u32,
    source: AnimSource,
    cave_flags: u32,
    ceiling: MobileCeiling,
    local_lights: Option<(&RenderStaticLightInstances, u32)>,
) -> MobileSpriteInstance {
    let (world_x, world_z) = apply_cc_static_world_anchor_translation(translation.x, translation.z);
    let (depth_x, depth_z) = mobile_depth_cell(translation.x, translation.z);
    let (tile_x, tile_y) = apply_cc_static_world_anchor_translation(depth_x, depth_z);
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
        tile_x,
        tile_y,
        priority_z_units,
        sort_bias_ordinal: tie_level,
        is_wet_flags: cave_flags,
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
    MobileSpriteInstance { sprite, crossfade: frames.crossfade, ceiling, frame_filter: filter }
}

/// The cell a mobile at (`x`, `z`) is ordered from: its own at rest, and while it slides between two cells, the
/// nearer to the camera in each map axis, as the client orders a walking mobile (ClassicUO `View.CalculateDepthZ`
/// takes the next cell east and south of its step's origin, the origin north and west). Ordered from the cell it
/// slides out of, or from its fractional position, the land of the cells in front would cover its lower half
/// until the step ends.
pub(super) fn mobile_depth_cell(x: f32, z: f32) -> (f32, f32) {
    ((x - DEPTH_CELL_SLIDE_EPSILON).ceil(), (z - DEPTH_CELL_SLIDE_EPSILON).ceil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sliding_mobile_is_ordered_from_the_nearer_cell_of_its_step() {
        assert_eq!(mobile_depth_cell(1473.0, 1722.0), (1473.0, 1722.0), "at rest");
        assert_eq!(mobile_depth_cell(1473.25, 1722.0), (1474.0, 1722.0), "east, from the cell it steps into");
        assert_eq!(mobile_depth_cell(1473.75, 1721.25), (1474.0, 1722.0), "north-east");
        assert_eq!(mobile_depth_cell(1472.5, 1722.5), (1473.0, 1723.0), "south-west");
        assert_eq!(mobile_depth_cell(1472.75, 1721.75), (1473.0, 1722.0), "north-west, from the cell it leaves");
    }

    /// A mobile on a cave's floor is lifted with it, darkened outside the player's cave, and never left out; away from
    /// the caves it keeps its depth. Without caves no cell is a cave's.
    #[test]
    fn a_mobile_takes_the_cave_bits_of_where_it_stands() {
        use crate::render::statics::static_batches::{STATIC_FLAG_CAVE, STATIC_FLAG_CAVE_OUTSIDE};
        use crate::world::caves::CaveStatic;
        assert_eq!(cave_instance_flags(CaveStatic::Lifted, true), STATIC_FLAG_CAVE);
        assert_eq!(cave_instance_flags(CaveStatic::Lifted, false), STATIC_FLAG_CAVE);
        assert_eq!(cave_instance_flags(CaveStatic::Outside, true), STATIC_FLAG_CAVE_OUTSIDE);
        assert_eq!(cave_instance_flags(CaveStatic::Hidden, true), STATIC_FLAG_CAVE_OUTSIDE);
        assert_eq!(cave_instance_flags(CaveStatic::Outside, false), 0);
        assert_eq!(cave_instance_flags(CaveStatic::Hidden, false), 0);
        let on_floor = crate::world::coords::UOVec4::new(2565, 487, 0, 1).to_bevy_vec3_ignore_map();
        assert_eq!(mobile_cave_flags(&CavesRes::default(), on_floor), 0);
    }

    #[test]
    fn the_shader_reads_a_mobile_instance_as_wgsl_lays_it_out() {
        // A SpriteInstance (144 bytes, aligned to its vec4), a FrameCrossfade (48), a MobileCeiling (16) and a
        // MobileFrameFilter (32), on a 16-byte stride.
        const WGSL_SPRITE_INSTANCE_SIZE: usize = 144;
        const WGSL_MOBILE_CEILING_SIZE: usize = 16;
        const WGSL_MOBILE_SPRITE_INSTANCE_SIZE: usize = 240;
        assert_eq!(std::mem::size_of::<SpriteInstance>(), WGSL_SPRITE_INSTANCE_SIZE);
        assert_eq!(std::mem::size_of::<MobileCeiling>(), WGSL_MOBILE_CEILING_SIZE);
        assert_eq!(std::mem::size_of::<MobileSpriteInstance>(), WGSL_MOBILE_SPRITE_INSTANCE_SIZE);
    }

    const DEPTH_WESL: &str = include_str!("../../../assets/shaders/world/art/depth.wesl");
    /// The isometric camera's line of sight (camera.rs: the camera looks along (-1, -1, -1)).
    const ISOMETRIC_SIGHT: Vec3 = Vec3::ONE;
    /// A mobile at rest and sliding east, south-east and north-west (map x, y).
    const MOBILE_POSITIONS: [(f32, f32); 4] = [(1473.0, 1722.0), (1473.4, 1722.0), (1473.5, 1722.5), (1472.75, 1721.75)];
    /// How far a frame reaches from its anchor, in screen pixels: sideways, up and down; each pixel at its centre.
    const REACH_SIDEWAYS_PX: i32 = 120;
    const REACH_UP_PX: i32 = 160;
    const REACH_DOWN_PX: i32 = 120;
    const PIXEL_STEP_PX: usize = 2;
    const PIXEL_CENTRE_PX: f32 = 0.5;

    /// Every pixel of a frame's reach around its anchor, (right, below) in screen pixels.
    fn frame_pixels() -> impl Iterator<Item = (f32, f32)> {
        (-REACH_SIDEWAYS_PX..=REACH_SIDEWAYS_PX).step_by(PIXEL_STEP_PX).flat_map(|right| {
            (-REACH_UP_PX..REACH_DOWN_PX).step_by(PIXEL_STEP_PX)
                .map(move |below| (right as f32 + PIXEL_CENTRE_PX, below as f32 + PIXEL_CENTRE_PX))
        })
    }

    /// The point on the card of a mobile at (`x`, `y`) and `z` of its pixel `right_px` right of its anchor and
    /// `below_px` below it (placement.rs: the card at the cell's near vertex, the anchor on the cell's centre).
    fn card_point(x: f32, y: f32, z: f32, right_px: f32, below_px: f32) -> Vec3 {
        use crate::render::statics::static_placement::{CC_WORLD_XZ_PER_PIXEL, STATIC_BILLBOARD_RIGHT_XZ};
        let (near_x, near_z) = apply_cc_static_world_anchor_translation(x, y);
        let right = STATIC_BILLBOARD_RIGHT_XZ * right_px * CC_WORLD_XZ_PER_PIXEL;
        Vec3::new(near_x + right.x,
            crate::world::coords::scale_uo_z_to_bevy_units(z)
                + (CELL_CENTRE_ABOVE_NEAR_VERTEX_PX - below_px) * CC_WORLD_Y_PER_PIXEL,
            near_z + right.y)
    }

    /// The cell where the isometric line of sight through `point` meets the plane at z `plane_z`.
    fn plane_cell(point: Vec3, plane_z: f32) -> (f32, f32) {
        let hit = point + ISOMETRIC_SIGHT * (crate::world::coords::scale_uo_z_to_bevy_units(plane_z) - point.y);
        (hit.x.floor(), hit.z.floor())
    }

    /// The logical distance (static_placement.rs `logical_draw_distance`, which the GPU's depth follows:
    /// depth_tests.rs) of a surface lying on the cell (`x`, `y`) at `priority_z`, tie level 0.
    fn cell_distance((x, y): (f32, f32), priority_z: f32) -> f64 {
        let (anchor_x, anchor_y) = apply_cc_static_world_anchor_translation(x, y);
        logical_draw_distance(f64::from(anchor_x + anchor_y), f64::from(priority_z), 0)
    }

    /// art/depth.wesl `mobile_fragment_view_z` in the isometric camera, as a logical distance: the pixel
    /// (`right_px`, `below_px`) of a frame of tie level `tie_level` of a mobile at (`x`, `y`) and `z`, under the ceiling
    /// plane at z `ceiling_z` if any.
    fn mobile_pixel_distance(x: f32, y: f32, z: f32, (right_px, below_px): (f32, f32), tie_level: u32,
        ceiling_z: Option<f32>) -> f64 {
        let ceiling_offset = wesl_f32_const(DEPTH_WESL, "MOBILE_CEILING_PRIORITY_Z_OFFSET");
        let point = card_point(x, y, z, right_px, below_px);
        let (depth_x, depth_y) = mobile_depth_cell(x, y);
        let (tile_x, tile_y) = apply_cc_static_world_anchor_translation(depth_x, depth_y);
        let priority_z = z + MOBILE_PRIORITY_Z_OFFSET;
        let card = logical_draw_distance(f64::from(tile_x + tile_y), f64::from(priority_z), tie_level);
        let mut distance = card.max(cell_distance(plane_cell(point, z), priority_z));
        if let Some(ceiling_z) = ceiling_z
            && point.y > crate::world::coords::scale_uo_z_to_bevy_units(ceiling_z) {
            distance = distance.min(cell_distance(plane_cell(point, ceiling_z), ceiling_z + ceiling_offset));
        }
        distance
    }

    use crate::render::statics::static_placement::{logical_draw_distance, CC_WORLD_Y_PER_PIXEL};
    use crate::util_lib::wesl_source::wesl_f32_const;
    use super::super::placement::CELL_CENTRE_ABOVE_NEAR_VERTEX_PX;

    /// The flat land, the Background floors and the flat items at the mobile's z never cover a pixel of its frame: the
    /// legs and a tail below the anchor lie on the cell they cover, over what lies flat on it, at rest and sliding.
    #[test]
    fn nothing_flat_at_the_mobiles_z_covers_its_frame() {
        const Z: f32 = 0.0;
        // ClassicUO's priority z of a Background floor and of a flat item over their z; a flat item's highest tie level.
        const BACKGROUND_PRIORITY_Z_OFFSET: f32 = -1.0;
        const FLAT_ITEM_TIE_LEVEL: u32 = 8;
        let land_offset = wesl_f32_const(DEPTH_WESL, "LAND_PRIORITY_Z_OFFSET");
        for (x, y) in MOBILE_POSITIONS {
            for pixel in frame_pixels() {
                let cell = plane_cell(card_point(x, y, Z, pixel.0, pixel.1), Z);
                let (anchor_x, anchor_y) = apply_cc_static_world_anchor_translation(cell.0, cell.1);
                let row = f64::from(anchor_x + anchor_y);
                for tie_level in [MOUNT_TIE_LEVEL, RIDER_TIE_LEVEL] {
                    let mobile = mobile_pixel_distance(x, y, Z, pixel, tie_level, None);
                    assert!(mobile > cell_distance(cell, Z + land_offset), "({x}, {y}): the land covers {pixel:?}");
                    assert!(mobile > cell_distance(cell, Z + BACKGROUND_PRIORITY_Z_OFFSET), "a floor, {pixel:?}");
                    assert!(mobile > logical_draw_distance(row, f64::from(Z), FLAT_ITEM_TIE_LEVEL),
                        "({x}, {y}): a flat item covers {pixel:?}");
                }
            }
        }
    }

    /// A static standing at the mobile's z covers the pixels below the anchor that lie on its cell, and land rising
    /// there more than 3 z over the mobile covers them too: a planter one row in front hides the tail behind it. Above
    /// the anchor the frame keeps its card's depth, and so do the pixels below it on its own cell.
    #[test]
    fn what_stands_in_front_covers_the_pixels_that_lie_on_its_cell() {
        const Z: f32 = 10.0;
        // The tie level of an upright static (assign_sprite_tie_levels: 1 and up), its priority z over its z.
        const STATIC_TIE_LEVEL: u32 = 1;
        const STANDING_PRIORITY_Z_OFFSET: f32 = 1.0;
        const RISING_LAND_Z: f32 = Z + 4.0;
        let land_offset = wesl_f32_const(DEPTH_WESL, "LAND_PRIORITY_Z_OFFSET");
        for (x, y) in MOBILE_POSITIONS {
            let (depth_x, depth_y) = mobile_depth_cell(x, y);
            let (tile_x, tile_y) = apply_cc_static_world_anchor_translation(depth_x, depth_y);
            let card = logical_draw_distance(f64::from(tile_x + tile_y), f64::from(Z + MOBILE_PRIORITY_Z_OFFSET),
                RIDER_TIE_LEVEL);
            let mut covered = 0;
            for pixel in frame_pixels() {
                let mobile = mobile_pixel_distance(x, y, Z, pixel, RIDER_TIE_LEVEL, None);
                let cell = plane_cell(card_point(x, y, Z, pixel.0, pixel.1), Z);
                if cell.0 + cell.1 <= depth_x + depth_y {
                    assert_eq!(mobile, card, "({x}, {y}): {pixel:?} lies behind or on the mobile's row");
                    continue;
                }
                let (anchor_x, anchor_y) = apply_cc_static_world_anchor_translation(cell.0, cell.1);
                let standing = logical_draw_distance(f64::from(anchor_x + anchor_y),
                    f64::from(Z + STANDING_PRIORITY_Z_OFFSET), STATIC_TIE_LEVEL);
                assert!(standing > mobile, "({x}, {y}): a static on {cell:?} stays under {pixel:?}");
                assert!(cell_distance(cell, RISING_LAND_Z + land_offset) > mobile, "rising land, {pixel:?}");
                covered += 1;
            }
            assert!(covered > 0);
        }
    }

    /// The mount's and the rider's pixels below the anchor lie on the same ground at tie level 0: where both cover a
    /// pixel there the rider, drawn after its mount, shows (the depth test passes an equal depth).
    #[test]
    fn a_rider_and_its_mount_lie_level_on_the_ground_in_front() {
        const Z: f32 = 0.0;
        const BELOW_PX: f32 = 30.5;
        let (x, y) = MOBILE_POSITIONS[0];
        assert_eq!(mobile_pixel_distance(x, y, Z, (0.5, BELOW_PX), MOUNT_TIE_LEVEL, None),
            mobile_pixel_distance(x, y, Z, (0.5, BELOW_PX), RIDER_TIE_LEVEL, None));
        assert!(mobile_pixel_distance(x, y, Z, (0.5, -BELOW_PX), MOUNT_TIE_LEVEL, None)
            < mobile_pixel_distance(x, y, Z, (0.5, -BELOW_PX), RIDER_TIE_LEVEL, None), "above it, on its mount");
    }

    /// Under a ceiling, the roof tiles on the cells where the pixels above it meet its plane cover them, so a tall
    /// body's head stays under the roof; where none is drawn the head shows over the land and what stands on the
    /// mobile's floor there. The pixels below the plane keep their depth.
    #[test]
    fn a_roof_covers_a_head_poking_through_it() {
        const Z: f32 = 20.0;
        const CEILING_Z: f32 = Z + 25.0;
        // A Background roof tile's priority z over its z: the lowest a roof tile takes.
        const BACKGROUND_PRIORITY_Z_OFFSET: f32 = -1.0;
        const STANDING_PRIORITY_Z_OFFSET: f32 = 1.0;
        const HIGHEST_TIE_LEVEL: u32 = 8;
        let land_offset = wesl_f32_const(DEPTH_WESL, "LAND_PRIORITY_Z_OFFSET");
        let ceiling_y = crate::world::coords::scale_uo_z_to_bevy_units(CEILING_Z);
        for (x, y) in MOBILE_POSITIONS {
            let (depth_x, depth_y) = mobile_depth_cell(x, y);
            let mut under_the_roof = 0;
            for pixel in frame_pixels() {
                let point = card_point(x, y, Z, pixel.0, pixel.1);
                let open = mobile_pixel_distance(x, y, Z, pixel, RIDER_TIE_LEVEL, None);
                let roofed = mobile_pixel_distance(x, y, Z, pixel, RIDER_TIE_LEVEL, Some(CEILING_Z));
                let cell = plane_cell(point, CEILING_Z);
                if point.y <= ceiling_y || cell.0 + cell.1 >= depth_x + depth_y {
                    assert_eq!(roofed, open, "({x}, {y}): {pixel:?} does not reach over the roof behind");
                    continue;
                }
                under_the_roof += 1;
                assert!(cell_distance(cell, CEILING_Z + BACKGROUND_PRIORITY_Z_OFFSET) > roofed,
                    "({x}, {y}): the roof tile on {cell:?} stays under {pixel:?}");
                let (anchor_x, anchor_y) = apply_cc_static_world_anchor_translation(cell.0, cell.1);
                assert!(roofed > logical_draw_distance(f64::from(anchor_x + anchor_y),
                    f64::from(Z + STANDING_PRIORITY_Z_OFFSET), HIGHEST_TIE_LEVEL), "the floor's statics, {pixel:?}");
                assert!(roofed > cell_distance(cell, Z + land_offset), "the land at the floor's z, {pixel:?}");
            }
            assert!(under_the_roof > 0);
        }
    }

    #[test]
    fn a_turning_rider_keeps_its_layers_until_the_new_clips_are_resident() {
        use std::cell::RefCell;
        use std::collections::HashSet;
        let layers = |facing: u8| {
            let key = |body| ClipKey { source: AnimSource::Classic, body, action: 0, direction: facing };
            vec![DrawnFrame { key: key(200), tie_level: MOUNT_TIE_LEVEL, offset_px: (0.0, 0.0) },
                DrawnFrame { key: key(400), tie_level: RIDER_TIE_LEVEL, offset_px: (0.0, f32::from(facing)) }]
        };
        let (east, west) = (layers(2), layers(6));
        let resident = RefCell::new(east.iter().map(|layer| layer.key).collect::<HashSet<_>>());
        let loading = RefCell::new(west.iter().map(|layer| layer.key).collect::<HashSet<_>>());
        let draw = |shown: &mut MobileShownLayers, wanted: &[DrawnFrame]| layers_to_draw(wanted.to_vec(), shown,
            |key| loading.borrow().contains(&key), |key| resident.borrow().contains(&key));
        let mut shown = MobileShownLayers::default();
        assert_eq!(draw(&mut shown, &east), east);
        // Turned west: the mount's clip lands first, the rider's a few frames later. Until then both stay east.
        assert_eq!(draw(&mut shown, &west), east);
        loading.borrow_mut().remove(&west[0].key);
        resident.borrow_mut().insert(west[0].key);
        assert_eq!(draw(&mut shown, &west), east);
        loading.borrow_mut().remove(&west[1].key);
        resident.borrow_mut().insert(west[1].key);
        assert_eq!(draw(&mut shown, &west), west);
        // A clip the package lacks (not loading, never resident) is not waited for; nor are layers no longer resident.
        let north = layers(0);
        assert_eq!(draw(&mut shown, &north), north);
        loading.borrow_mut().extend(east.iter().map(|layer| layer.key));
        assert_eq!(draw(&mut shown, &east), east);
        // Nothing shown before, a mobile draws what it has while the rest loads.
        assert_eq!(draw(&mut MobileShownLayers::default(), &east), east);
    }
}
