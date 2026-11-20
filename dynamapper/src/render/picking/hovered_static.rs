//! The static drawn under the cursor: the GPU pick's tile with the geometry it is drawn with, and its diagnostic
//! outline (`highlight_hovered_static_object`). The cursor inspector describes the same static.
use super::cursor_map_id;
use super::static_geometry::{StaticArtRoute, StaticPickingResources};
use crate::app::{app_states::AppState, system_sets::SceneRenderArtSystems};
use crate::input::{InjectedPointer, effective_cursor_position};
use crate::render::statics::static_batches::StaticPickTile;
use crate::render::view::camera::PlayerCamera;
use crate::settings::Settings;
use crate::world::player::Player;
use bevy::{prelude::*, window::PrimaryWindow};
use uocf::classic::statics::PackedStaticTile;

const HOVERED_STATIC_HIGHLIGHT_Y_LIFT: f32 = 0.01;

#[derive(Clone, Copy, Debug)]
pub(crate) enum HoveredStaticKind {
    Sprite,
    Ground,
}

#[derive(Clone)]
pub(crate) struct HoveredStatic {
    pub tile: PackedStaticTile,
    pub kind: HoveredStaticKind,
    corners: [Vec3; 4],
    pub global_x: u16,
    pub global_y: u16,
    pub route: StaticArtRoute,
}

pub(crate) fn install(app: &mut App) {
    app.add_systems(Update, draw_hovered_static_highlight.after(SceneRenderArtSystems::CollectVisibleStatics)
        .run_if(in_state(AppState::InGame)));
}

/// The static drawn at `cursor_pos`, if statics are drawn.
pub(crate) fn hovered_static(
    picking: &mut StaticPickingResources,
    cursor_pos: Vec2,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    settings: &Settings,
    map_id: u8,
) -> Option<HoveredStatic> {
    if !settings.world_rendering.enable_statics { return None; }
    let tile = picking.visible_hit(cursor_pos, camera, camera_tf, u32::from(map_id))?;
    hovered_static_of(picking, camera, camera_tf, tile)
}

fn hovered_static_of(
    picking: &mut StaticPickingResources,
    camera: &Camera,
    camera_tf: &GlobalTransform,
    candidate: StaticPickTile,
) -> Option<HoveredStatic> {
    let source = candidate.tile;
    let tile = PackedStaticTile { graphic: source.graphic,
        xy_packed: (source.world_x as u32 & 7) as u8 | (((source.world_z as u32 & 7) as u8) << 3),
        z: source.z, hue: source.hue };
    let graphic = tile.graphic;
    let world_x = source.world_x;
    let world_z = source.world_z;
    // None for a NoDraw static, by the tile data it is drawn by (Kingdom Reborn's for the statics that continue the
    // Kingdom Reborn land).
    let geometry = picking.resolve(graphic, world_x, world_z, tile.z)?;
    let kind = if geometry.flat_quad == 0 { HoveredStaticKind::Sprite } else { HoveredStaticKind::Ground };
    let corners = geometry.corners(camera, camera_tf);

    // Visibility is the GPU material/depth result. Reconstruct only the logical
    // owner and its highlight; do not re-reject it with a different CPU sampler.

    Some(HoveredStatic {
        tile,
        kind,
        corners,
        global_x: world_x as u16,
        global_y: world_z as u16,
        route: geometry.route,
    })
}

fn draw_hovered_static_highlight(
    mut picking: StaticPickingResources,
    settings: Res<Settings>,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
    injected_pointer: Option<Res<InjectedPointer>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut gizmos: Gizmos,
) {
    if !settings
        .world_rendering
        .diagnostics
        .highlight_hovered_static_object
    {
        return;
    }

    let Some(cursor_pos) = windows.single().ok().and_then(|window| effective_cursor_position(window, injected_pointer.as_deref())) else {
        return;
    };
    let Some((camera, camera_tf)) = camera_q.single().ok() else {
        return;
    };
    let Some(player) = player_q.single().ok() else {
        return;
    };
    let Some(map_id) = cursor_map_id(Some(player), &settings) else { return; };

    let Some(hovered) = hovered_static(&mut picking, cursor_pos, camera, camera_tf, &settings, map_id) else {
        return;
    };

    let color = match hovered.kind {
        HoveredStaticKind::Sprite => Color::srgb(0.1, 1.0, 0.2),
        HoveredStaticKind::Ground => Color::srgb(1.0, 0.85, 0.2),
    };
    let lift = Vec3::new(0.0, HOVERED_STATIC_HIGHLIGHT_Y_LIFT, 0.0);
    let [corner0, corner1, corner2, corner3] = hovered.corners;
    let corner0 = corner0 + lift;
    let corner1 = corner1 + lift;
    let corner2 = corner2 + lift;
    let corner3 = corner3 + lift;

    gizmos.line(corner0, corner1, color);
    gizmos.line(corner1, corner3, color);
    gizmos.line(corner3, corner2, color);
    gizmos.line(corner2, corner0, color);
    gizmos.line(corner0, corner3, color);
    gizmos.line(corner1, corner2, color);
}
