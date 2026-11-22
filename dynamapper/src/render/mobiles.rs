//! The mobiles: animated bodies drawn as the clients draw them, with the statics' art sprite shader. The player is
//! the one mobile so far: the body and mount the settings name, standing, walking or running as it moves.

pub mod bc7_encoding;
pub mod clips;
pub mod compute_budget;
pub mod compute_warm_up;
pub mod draw;
pub mod frame_atlas;
pub mod in_between;
pub mod pace;
pub mod placement;
pub mod rife_in_betweens;
pub mod upscales;

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
use crate::world::mobile_motion::{MobileMotion, Movement, STEP_SECS_MOUNTED_WALK};
use crate::world::player::Player;
use clips::{AnimPackages, AnimSource, Clip, MobileClips};
use draw::{MobileActions, MobileAppearance, MobileShownLayers};
use pace::{GroundStride, MobileCycleClock, MobileStrides, MobileWalkPace};

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
    /// How long a cycle of the clips lasts.
    pub cycle: MobileCycle,
}

/// How long a mobile's cycle lasts. The lead layer, the mount if it has one, else the body, sets it, and every layer
/// shows the same point of it ([`MobileCycleClock`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MobileCycle {
    /// Each layer at its own clip's frame time, not in step.
    OwnClips,
    /// The lead clip's own length.
    #[default]
    LeadClip,
    /// The lead clip's own length times this.
    LeadClipTimes(f32),
    /// One cycle every so many seconds.
    Secs(f32),
    /// The lead clip paced to the ground.
    Ground(GroundStride),
}

impl MobileCycle {
    /// The seconds of a cycle led by `lead`; none for [`MobileCycle::OwnClips`].
    pub fn lead_secs(self, lead: &Clip) -> Option<f32> {
        match self {
            Self::OwnClips => None,
            Self::LeadClip => Some(lead.cycle_secs()),
            Self::LeadClipTimes(factor) => Some(lead.cycle_secs() * factor),
            Self::Secs(secs) => Some(secs),
            Self::Ground(stride) => Some(stride.cycle_secs(lead.cycle_secs())),
        }
    }
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
            .init_resource::<upscales::MobileUpscales>()
            .init_resource::<upscales::MobileUpscaleGate>()
            .init_resource::<MobileStrides>()
            .add_systems(Update, (
                sys_sync_player_appearance,
                sys_choose_mobile_clips,
                draw::sys_setup_mobile_draw,
                draw::sys_size_mobile_upscale_pages,
                upscales::sys_update_mobile_upscale_gate,
                draw::sys_draw_mobiles,
                rife_in_betweens::sys_schedule_mobile_in_betweens,
                upscales::sys_schedule_mobile_upscales,
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
            .init_resource::<upscales::render::RenderUpscales>()
            .init_resource::<compute_warm_up::ComputeWarmUp>()
            .init_resource::<compute_budget::MobileComputeBudget>()
            .init_resource::<bc7_encoding::MobileBc7Encoder>()
            .add_systems(ExtractSchedule, (
                frame_atlas::sys_extract_mobile_frame_uploads::<clips::AtlasEntry>,
                rife_in_betweens::render::sys_extract_mobile_in_betweens,
                compute_warm_up::sys_extract_block_encoder_wanted,
                upscales::render::sys_extract_mobile_upscales,
            ))
            .add_systems(bevy::render::Render, (
                compute_warm_up::sys_start_compute_warm_up,
                compute_budget::sys_start_mobile_compute_frame,
                frame_atlas::sys_render_upload_mobile_frames,
                rife_in_betweens::render::sys_render_mobile_in_betweens,
                upscales::render::sys_render_mobile_upscales,
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
                commands.entity(entity).insert((wanted, MobileClipChoice::default(), MobileCycleClock::default(),
                    MobileShownLayers::default()));
                if current.is_none() {
                    commands.entity(entity).insert(MobileMotion::default());
                }
            }
            (None, Some(_)) => {
                commands.entity(entity)
                    .remove::<(MobileAppearance, MobileClipChoice, MobileCycleClock, MobileShownLayers, MobileMotion)>();
            }
            _ => {}
        }
    }
}

/// The clips of each mobile for its movement: from the active art source's package, the other one when that one
/// is missing.
fn sys_choose_mobile_clips(
    packages: Res<MobileAnimPackages>,
    mut strides: ResMut<MobileStrides>,
    settings: Res<Settings>,
    art_source: Option<Res<StaticArtSourceState>>,
    mut mobiles_q: Query<(&MobileMotion, &MobileAppearance, &mut MobileClipChoice)>,
) {
    let preferred = match art_source.and_then(|state| state.active_source) {
        Some(ClientTextureSource::Ec) => AnimSource::Enhanced,
        _ => AnimSource::Classic,
    };
    if packages.is_changed() {
        *strides = MobileStrides::new(&packages.0);
    }
    let packages = &packages.0;
    let mut strides = (settings.world_rendering.mobile_walk_pace == MobileWalkPace::MatchTheGround)
        .then_some(&mut *strides);
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
                .map(|package| classic_choice(package, strides.as_deref_mut(), appearance, motion)),
            Some(AnimSource::Enhanced) => packages.enhanced.as_deref()
                .map(|package| enhanced_choice(package, strides.as_deref_mut(), appearance, motion)),
            None => None,
        }
        .unwrap_or_default();
        if chosen.source != choice.source || chosen.actions != choice.actions
            || chosen.body_offset_px != choice.body_offset_px || chosen.mount_offset_px != choice.mount_offset_px
            || chosen.cycle != choice.cycle {
            *choice = chosen;
        }
    }
}

/// The ground pace of `action` of `body` from `source` on the move, when `strides` (the setting on) has its stride: at
/// the step time of the movement, mounted or not.
fn ground_stride(strides: Option<&mut MobileStrides>, source: AnimSource, body: u32, action: Option<u16>,
    motion: &MobileMotion, mounted: bool) -> Option<GroundStride> {
    if motion.movement == Movement::Stand {
        return None;
    }
    let stride = strides?.get(source, body, action?)?;
    Some(GroundStride { stride, step_secs: motion.step_secs(mounted) })
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
/// which moves with its own; the rider sits the mount's offset lower (ClassicUO `Mounts`, `MobileView`). Matched to
/// the ground (`strides`), the body's stride paces the cycle on foot, and a ridden mount on the move gallops: a mounted
/// walk steps a tile in 0.2 s, a run's step on foot, and a Classic run cycle at the clients' frame time covers about two
/// tiles, so the mount plays its run and the rider its fast ride, one cycle in the run's own time on a mounted walk and
/// in half of it on a mounted run (`MobileCycle::LeadClipTimes`). The walk (a trot of about one tile a cycle) needs
/// five cycles a second for its hooves to keep up with a mounted walk, and the clients' two and a half let them slide.
/// The measured strides don't pace a mount: the flow loses the hooves of five-frame Classic clips, which jump 10 to 25
/// pixels a frame (the horse's gallop measures 1.08 tiles where its planted hooves travel 2 to 2.8).
fn classic_choice(package: &MobileAnimCcPackage, strides: Option<&mut MobileStrides>, appearance: &MobileAppearance,
    motion: &MobileMotion) -> MobileClipChoice {
    let gallop = appearance.mount_body.is_some() && motion.movement != Movement::Stand && strides.is_some();
    let movement = classic_movement(if gallop { Movement::Run } else { motion.movement });
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
        cycle: match mount {
            // A gallop's cycle shortens with the mounted step; standing, or at the clients' timing, ClassicUO steps
            // the rider and the mount with one frame counter at the same frame time.
            Some(_) if gallop => MobileCycle::LeadClipTimes(motion.step_secs(true) / STEP_SECS_MOUNTED_WALK),
            Some(_) => MobileCycle::OwnClips,
            None => ground_stride(strides, AnimSource::Classic, appearance.body, body_action, motion, false)
                .map_or(MobileCycle::LeadClip, MobileCycle::Ground),
        },
    }
}

/// The Enhanced client's actions: the body's peace group, or its mounted group on a mount, which plays its own
/// mounted or peace move; the layers sit where `mobile_layer_offsets` puts them. A rider and its mount share one cycle
/// ([`MobileCycle`]), not the client's two clocks. Matched to the ground (`strides`), the mount's stride, or the
/// body's on foot, paces the cycle.
fn enhanced_choice(package: &MobileAnimEcPackage, strides: Option<&mut MobileStrides>, appearance: &MobileAppearance,
    motion: &MobileMotion) -> MobileClipChoice {
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
        choice.body_offset_px = layer_screen_offset_px(offsets.body);
        if let Some(mount_offset) = offsets.mount {
            choice.mount_offset_px = layer_screen_offset_px(mount_offset);
        }
    }
    // The rider and the mount keep in step, paced by the steps on the move: by the mount's stride, or by eye
    // without one.
    choice.cycle = match mount {
        Some((mount, _)) => match ground_stride(strides, AnimSource::Enhanced, mount, mount_action, motion, true) {
            Some(stride) => MobileCycle::Ground(stride),
            None => match motion.movement {
                Movement::Stand => MobileCycle::LeadClip,
                Movement::Walk => MobileCycle::Secs(ENHANCED_MOUNTED_WALK_CYCLE_STEPS * motion.step_secs(true)),
                Movement::Run => MobileCycle::Secs(ENHANCED_MOUNTED_RUN_CYCLE_STEPS * motion.step_secs(true)),
            },
        },
        None => ground_stride(strides, AnimSource::Enhanced, appearance.body, body_action, motion, false)
            .map_or(MobileCycle::LeadClip, MobileCycle::Ground),
    };
    choice
}

/// The packages as `Arc`s, for the clip decodes on the background pool.
/// The screen offset (x right, y down) of an Enhanced layer at `mobile_layer_offsets`' position, whose y points up: the
/// client's -15 lowers every layer, and the one extra offset in the data, the stygian dragon's +125 on its flying walk,
/// lifts frames that reach 100-190 px below their anchor off the ground. Read as y down, the dragon walked 200-300 px
/// underground and every Enhanced body stood 30 px above the Classic one on the same cell.
fn layer_screen_offset_px((x, y): (i32, i32)) -> (f32, f32) {
    (x as f32, -(y as f32))
}

pub fn anim_packages(classic: Option<Arc<MobileAnimCcPackage>>, enhanced: Option<Arc<MobileAnimEcPackage>>)
    -> MobileAnimPackages {
    MobileAnimPackages(AnimPackages { classic, enhanced })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stygian dragon's flying walk (body 826, action 19) carries the data's one extra offset.
    const STYGIAN_FLYING_EXTRA_OFFSET_Y: i32 = 125;

    #[test]
    fn an_enhanced_layer_offset_lifts_with_its_y() {
        let on_foot = (0, anim_ec::MOBILE_LAYER_SHIFT_Y);
        assert_eq!(layer_screen_offset_px(on_foot), (0.0, -anim_ec::MOBILE_LAYER_SHIFT_Y as f32),
            "the client's shift lowers a body on foot");
        let flying = (0, STYGIAN_FLYING_EXTRA_OFFSET_Y + anim_ec::MOBILE_LAYER_SHIFT_Y);
        let (_, down) = layer_screen_offset_px(flying);
        assert!(down < 0.0, "the stygian's flying walk is lifted off its cell, not sunk {down} px into it");
    }
}
