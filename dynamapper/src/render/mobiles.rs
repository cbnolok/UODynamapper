//! The mobiles: animated bodies drawn as the clients draw them, with the statics' art sprite shader. The player is
//! the one mobile so far: the body and mount the settings name, standing, walking or running as it moves.

pub mod clips;
pub mod draw;
pub mod frame_atlas;
pub mod in_between;
pub mod placement;
pub mod rife;
pub mod rife_in_betweens;

use std::sync::Arc;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, RenderApp};
use udd_assets::animation::mobile_anim_cc::{self as anim_cc, MobileAnimCcPackage};
use udd_assets::animation::mobile_anim_ec::{self as anim_ec, MobileAnimEcPackage};
use crate::app::app_states::AppState;
use crate::app::system_sets::{MovementSystems, SceneRenderArtSystems};
use crate::prelude::*;
use crate::render::statics::static_source::StaticArtSourceState;
use crate::settings::{ClientTextureSource, Settings};
use crate::world::mobile_motion::{MobileMotion, Movement};
use crate::world::player::Player;
use clips::{AnimPackages, AnimSource, MobileClips};
use draw::{MobileActions, MobileAppearance};

/// The animation packages, as loaded at startup (`content`).
#[derive(Resource, Default)]
pub struct MobileAnimPackages(pub AnimPackages);

/// The clips a mobile shows now: whose frames, which actions, and where its layers sit.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct MobileClipChoice {
    pub source: Option<AnimSource>,
    pub actions: Option<MobileActions>,
    /// The anchor offsets of the body and of the mount, in screen pixels (x right, y down).
    pub body_offset_px: (f32, f32),
    pub mount_offset_px: (f32, f32),
    /// How a rider and its mount keep in step; none when each layer plays on its own clock.
    pub mounted_cycle: Option<MountedCycle>,
}

/// The cycle a rider and its mount share: both layers show the same point of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MountedCycle {
    /// One cycle every so many seconds.
    Secs(f32),
    /// The mount clip's own length.
    MountClip,
}

/// Steps per cycle of an Enhanced rider and mount on the move, chosen by eye. The client plays the rider (12 fps) and
/// the mount (9 fps) on clocks of their own, so the rider drifts off the mount's stride, and at the mounted step
/// times the hooves slide; ClassicUO's 2 and 4 steps looked too fast with the Enhanced frames.
const ENHANCED_MOUNTED_WALK_CYCLE_STEPS: f32 = 3.0;
const ENHANCED_MOUNTED_RUN_CYCLE_STEPS: f32 = 5.0;

pub struct MobileSpritesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(MobileSpritesPlugin);

impl Plugin for MobileSpritesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<MobileClips>()
            .init_resource::<MobileAnimPackages>()
            .init_resource::<rife_in_betweens::MobileInBetweens>()
            .add_systems(Update, (
                sys_sync_player_appearance,
                sys_choose_mobile_clips,
                draw::sys_setup_mobile_draw,
                draw::sys_draw_mobiles,
                rife_in_betweens::sys_schedule_mobile_in_betweens,
                frame_atlas::sys_stage_mobile_frame_uploads,
                draw::sys_sync_mobile_materials
                    .after(crate::render::statics::statics_draw::sys_update_art_materials),
            ).chain()
                .after(SceneRenderArtSystems::RenderStaticSprites)
                .after(MovementSystems::MovementActions)
                .run_if(in_state(AppState::InGame)));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.init_resource::<frame_atlas::RenderMobileFrameUploads>()
            .init_resource::<rife_in_betweens::render::RenderInBetweens>()
            .add_systems(ExtractSchedule, (
                frame_atlas::sys_extract_mobile_frame_uploads::<clips::AtlasEntry>,
                rife_in_betweens::render::sys_extract_mobile_in_betweens,
            ))
            .add_systems(bevy::render::Render, (
                frame_atlas::sys_render_upload_mobile_frames,
                rife_in_betweens::render::sys_render_mobile_in_betweens,
            ).chain().in_set(bevy::render::RenderSystems::Queue));
    }
}

/// Gives the player the body and mount the settings name: a body of 0, or a hidden player, draws none.
fn sys_sync_player_appearance(
    mut commands: Commands,
    settings: Res<Settings>,
    player_q: Query<(Entity, Option<&MobileAppearance>), With<Player>>,
) {
    let world = &settings.core.world;
    let wanted = (world.player_body != 0 && !world.hide_player).then(|| MobileAppearance {
        body: u32::from(world.player_body),
        mount_body: (world.player_mount_body != 0).then_some(u32::from(world.player_mount_body)),
    });
    for (entity, current) in player_q.iter() {
        match (wanted, current) {
            (Some(wanted), current) if current != Some(&wanted) => {
                commands.entity(entity).insert((wanted, MobileClipChoice::default()));
                if current.is_none() {
                    commands.entity(entity).insert(MobileMotion::default());
                }
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<(MobileAppearance, MobileClipChoice, MobileMotion)>();
            }
            _ => {}
        }
    }
}

/// The clips of each mobile for its movement: from the active art source's package, the other one when that one
/// is missing.
fn sys_choose_mobile_clips(
    packages: Res<MobileAnimPackages>,
    art_source: Option<Res<StaticArtSourceState>>,
    mut mobiles_q: Query<(&MobileMotion, &MobileAppearance, &mut MobileClipChoice)>,
) {
    let preferred = match art_source.and_then(|state| state.active_source) {
        Some(ClientTextureSource::Ec) => AnimSource::Enhanced,
        _ => AnimSource::Classic,
    };
    let packages = &packages.0;
    let source = match preferred {
        AnimSource::Enhanced if packages.enhanced.is_some() => Some(AnimSource::Enhanced),
        AnimSource::Classic if packages.classic.is_some() => Some(AnimSource::Classic),
        _ if packages.enhanced.is_some() => Some(AnimSource::Enhanced),
        _ if packages.classic.is_some() => Some(AnimSource::Classic),
        _ => None,
    };
    for (motion, appearance, mut choice) in mobiles_q.iter_mut() {
        let chosen = match source {
            Some(AnimSource::Classic) => packages.classic.as_deref()
                .map(|package| classic_choice(package, appearance, motion)),
            Some(AnimSource::Enhanced) => packages.enhanced.as_deref()
                .map(|package| enhanced_choice(package, appearance, motion)),
            None => None,
        }
        .unwrap_or_default();
        if chosen.source != choice.source || chosen.actions != choice.actions
            || chosen.body_offset_px != choice.body_offset_px || chosen.mount_offset_px != choice.mount_offset_px
            || chosen.mounted_cycle != choice.mounted_cycle {
            *choice = chosen;
        }
    }
}

fn classic_movement(movement: Movement) -> u8 {
    match movement {
        Movement::Stand => anim_cc::MOVE_STAND,
        Movement::Walk => anim_cc::MOVE_WALK,
        Movement::Run => anim_cc::MOVE_RUN,
    }
}

fn enhanced_movement(movement: Movement) -> u8 {
    match movement {
        Movement::Stand => anim_ec::MOVE_STAND,
        Movement::Walk => anim_ec::MOVE_WALK,
        Movement::Run => anim_ec::MOVE_RUN,
    }
}

/// The Classic client's actions (`MobileAnimCcPackage::movement_action`: a UOP body's sequence actions, ClassicUO
/// `GetGroupForAnimation` for the rest): the body's own, or its mounted ones on a mount,
/// which moves with its own; the rider sits the mount's offset lower (ClassicUO `Mounts`, `MobileView`).
fn classic_choice(package: &MobileAnimCcPackage, appearance: &MobileAppearance, motion: &MobileMotion)
    -> MobileClipChoice {
    let movement = classic_movement(motion.movement);
    let body = u16::try_from(appearance.body).ok();
    let mount = appearance.mount_body.and_then(|mount| u16::try_from(mount).ok());
    let body_action = body.and_then(|body| package.movement_action(body, movement, mount.is_some()));
    let mount_action = mount.and_then(|mount| package.movement_action(mount, movement, false));
    // The mount's height offset of `Bodyconv.def` (ClassicUO `MountedHeightOffset`; its `Mounts` table by mount
    // item agrees where both have one).
    let rider_offset_y = mount.and_then(|mount| package.body_resolve_record(mount)).map_or(0, |record| record.mount_height);
    MobileClipChoice {
        source: Some(AnimSource::Classic),
        actions: body_action.map(|body_action| MobileActions { body_action, mount_action }),
        body_offset_px: (0.0, f32::from(rider_offset_y)),
        mount_offset_px: (0.0, 0.0),
        // ClassicUO steps the rider and the mount with one frame counter at the same frame time already.
        mounted_cycle: None,
    }
}

/// The Enhanced client's actions: the body's peace group, or its mounted group on a mount, which plays its own
/// mounted or peace move; the layers sit where `mobile_layer_offsets` puts them. A rider and its mount share one cycle
/// ([`MountedCycle`]), not the client's two clocks.
fn enhanced_choice(package: &MobileAnimEcPackage, appearance: &MobileAppearance, motion: &MobileMotion)
    -> MobileClipChoice {
    const FLYING_ANIMATIONS: bool = true;
    let movement = enhanced_movement(motion.movement);
    let mount = appearance.mount_body.and_then(|mount| package.body(mount).map(|record| (mount, record)));
    let group = if mount.is_some() { anim_ec::GROUP_MOUNTED } else { anim_ec::GROUP_PEACE };
    let body_action = package.move_action(appearance.body, group, movement);
    let mount_action = mount.and_then(|(_, record)| record.mount_move_action(movement));
    let record = |body: u32, action: Option<u16>| action.and_then(|action|
        package.resolve_animation(body, action, motion.facing, FLYING_ANIMATIONS).map(|resolved| resolved.record));
    let mut choice = MobileClipChoice {
        source: Some(AnimSource::Enhanced),
        actions: body_action.map(|body_action| MobileActions { body_action, mount_action }),
        ..default()
    };
    if let Some(body_record) = record(appearance.body, body_action) {
        let mount_record = mount.and_then(|(mount, _)| record(mount, mount_action));
        let offsets = anim_ec::mobile_layer_offsets(body_record, mount_record);
        choice.body_offset_px = (offsets.body.0 as f32, offsets.body.1 as f32);
        if let Some(mount_offset) = offsets.mount {
            choice.mount_offset_px = (mount_offset.0 as f32, mount_offset.1 as f32);
        }
    }
    // The rider and the mount keep in step, paced by the steps on the move.
    choice.mounted_cycle = mount.map(|_| match motion.movement {
        Movement::Stand => MountedCycle::MountClip,
        Movement::Walk => MountedCycle::Secs(ENHANCED_MOUNTED_WALK_CYCLE_STEPS * motion.step_secs(true)),
        Movement::Run => MountedCycle::Secs(ENHANCED_MOUNTED_RUN_CYCLE_STEPS * motion.step_secs(true)),
    });
    choice
}

/// The packages as `Arc`s, for the clip decodes on the background pool.
pub fn anim_packages(classic: Option<Arc<MobileAnimCcPackage>>, enhanced: Option<Arc<MobileAnimEcPackage>>)
    -> MobileAnimPackages {
    MobileAnimPackages(AnimPackages { classic, enhanced })
}
