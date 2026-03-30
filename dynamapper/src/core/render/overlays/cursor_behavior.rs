use crate::core::controls::input_actions::ActionToggleCursorTeleportMode;
use crate::core::render::scene::player::Player;
use crate::core::render::{
    dialogs::get_egui_context_ready,
    scene::{
        camera::{PlayerCamera, UiCameraResource},
        RecomputeVisibleChunksEvent,
    },
};
use crate::core::uo_files_loader::MapPlanesRes;
use crate::ingame_sysmessage_logger;
use crate::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use bevy::text::{FontSmoothing, LineHeight};
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;
use uocf::geo::map::{MapCell, MapCellCoords};

const FONT_SIZE: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMode {
    Select,
    Teleport,
}

impl Default for CursorMode {
    fn default() -> Self {
        CursorMode::Select
    }
}

#[derive(Resource, Default)]
pub struct CursorBehavior {
    pub mode: CursorMode,
}

pub struct CursorBehaviorOverlayPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(CursorBehaviorOverlayPlugin);

#[derive(Component)]
pub struct OverlayCursorBehaviorContainer;

#[derive(Component)]
pub struct OverlayCursorModeText;

#[derive(Component)]
pub struct OverlayCursorPositionText;

impl Plugin for CursorBehaviorOverlayPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<CursorBehavior>()
            .add_observer(sys_toggle_cursor_mode)
            .add_systems(OnEnter(AppState::InGame), setup_overlay_cursor_behavior)
            .add_systems(
                Update,
                (
                    update_cursor_behavior_text.run_if(in_state(AppState::InGame)),
                    sys_teleport_on_click.run_if(in_state(AppState::InGame)),
                ),
            );
    }
}

pub fn setup_overlay_cursor_behavior(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Res<crate::external_data::settings::Settings>,
) {
    let font: Handle<Font> = asset_server.load("fonts/uo/UOClassicRough.ttf");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(108.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Start,
                justify_content: JustifyContent::Start,
                padding: UiRect::all(Val::Px(7.0 * settings.app.window.player_position_scale)),
                display: if settings.app.performance.show_overlay {
                    Display::Flex
                } else {
                    Display::None
                },
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            GlobalZIndex(100),
            OverlayCursorBehaviorContainer,
        ))
        .with_children(|builder| {
            let scale = settings.app.window.player_position_scale;
            builder.spawn((
                Text::new("Cursor mode: Select"),
                TextFont {
                    font: font.clone(),
                    font_size: FONT_SIZE * scale,
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Color::WHITE),
                OverlayCursorModeText,
            ));
            builder.spawn((
                Text::new("Cursor position: (NA, NA, NA)"),
                TextFont {
                    font,
                    font_size: FONT_SIZE * scale,
                    font_smoothing: FontSmoothing::AntiAliased,
                    ..default()
                },
                LineHeight::Px(FONT_SIZE * scale),
                TextColor(Color::WHITE),
                OverlayCursorPositionText,
            ));
        });
}

pub fn update_cursor_behavior_text(
    settings: Res<crate::external_data::settings::Settings>,
    cursor: Res<CursorBehavior>,
    player_q: Query<&Player>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    map_planes_r: Res<MapPlanesRes>,
    mut mode_text_q: Query<
        &mut Text,
        (
            With<OverlayCursorModeText>,
            Without<OverlayCursorPositionText>,
        ),
    >,
    mut position_text_q: Query<
        &mut Text,
        (
            With<OverlayCursorPositionText>,
            Without<OverlayCursorModeText>,
        ),
    >,
    mut node_q: Query<&mut Node, With<OverlayCursorBehaviorContainer>>,
    mut last_show_overlay: Local<bool>,
    mut last_mode: Local<Option<CursorMode>>,
    mut last_cursor_pos: Local<Option<Vec2>>,
    mut last_window_size: Local<Option<(f32, f32)>>,
    mut last_camera_translation: Local<Option<Vec3>>,
    mut last_player_map_id: Local<Option<u8>>,
) {
    let show_overlay = settings.app.performance.show_overlay;
    if *last_show_overlay != show_overlay {
        if let Ok(mut node) = node_q.single_mut() {
            node.display = if show_overlay {
                Display::Flex
            } else {
                Display::None
            };
        }
        *last_show_overlay = show_overlay;
    }

    if *last_mode != Some(cursor.mode) {
        if let Ok(mut mode_text) = mode_text_q.single_mut() {
            let next_text = format!(
                "Cursor mode: {}",
                match cursor.mode {
                    CursorMode::Select => "Select",
                    CursorMode::Teleport => "Teleport",
                }
            );
            if mode_text.0 != next_text {
                mode_text.0 = next_text;
            }
        }
        *last_mode = Some(cursor.mode);
    }

    let window_size = windows
        .single()
        .ok()
        .map(|window| (window.width(), window.height()));
    let camera_translation = camera_q
        .single()
        .ok()
        .map(|(_, camera_tf)| camera_tf.translation());
    let cursor_pos = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    let player_map_id = player_q
        .single()
        .ok()
        .and_then(|player| player.current_pos.map(|p| p.m));

    let needs_rebuild = *last_cursor_pos != cursor_pos
        || *last_window_size != window_size
        || *last_camera_translation != camera_translation
        || *last_player_map_id != player_map_id;

    if needs_rebuild {
        let cursor_position_label =
            match (cursor_pos, camera_q.single().ok(), player_q.single().ok()) {
                (Some(cursor_pos), Some((camera, camera_tf)), Some(player)) => {
                    match camera.viewport_to_world(camera_tf, cursor_pos).ok() {
                        Some(ray) if ray.direction.y.abs() > 1e-6 => {
                            let t = -ray.origin.y / ray.direction.y;
                            let hit = ray.origin + ray.direction * t;
                            let cursor_x = hit.x.round().max(0.0) as u16;
                            let cursor_y = hit.z.round().max(0.0) as u16;
                            let map_id = player
                                .current_pos
                                .map(|p| p.m)
                                .unwrap_or(settings.core.world.start_p.m);
                            let cursor_z =
                                resolve_cursor_map_z(&map_planes_r, map_id, cursor_x, cursor_y)
                                    .unwrap_or(0);
                            format!(
                                "Cursor position:\n[{}, {}, {}]",
                                cursor_x, cursor_y, cursor_z
                            )
                        }
                        _ => "Cursor position:\n[NA, NA, NA]".to_string(),
                    }
                }
                _ => "Cursor position:\n[NA, NA, NA]".to_string(),
            };

        if let Ok(mut position_text) = position_text_q.single_mut() {
            if position_text.0 != cursor_position_label {
                position_text.0 = cursor_position_label;
            }
        }

        *last_cursor_pos = cursor_pos;
        *last_window_size = window_size;
        *last_camera_translation = camera_translation;
        *last_player_map_id = player_map_id;
    }
}

pub fn resolve_cursor_map_z(map_planes_r: &MapPlanesRes, map_id: u8, x: u16, y: u16) -> Option<i8> {
    let plane = map_planes_r
        .0
        .get(map_id as usize)
        .and_then(|opt| opt.as_ref())
        .expect("Uncached Map in resolve_cursor_map_z");
    let cell = MapCellCoords {
        x: x as u32,
        y: y as u32,
    };
    let block_pos = MapCell::coords_of_parent_block(&cell);
    let block = plane.block_no_update(block_pos)?;
    let rel = MapCell::coords_in_block(&cell);
    Some(block.cell(rel.x, rel.y).ok()?.z)
}

fn sys_toggle_cursor_mode(
    _trigger: On<ActionToggleCursorTeleportMode>,
    mut cursor: ResMut<CursorBehavior>,
) {
    cursor.mode = match cursor.mode {
        CursorMode::Select => CursorMode::Teleport,
        CursorMode::Teleport => CursorMode::Select,
    };
    ingame_sysmessage_logger::normal(format!(
        "Cursor mode: {}",
        match cursor.mode {
            CursorMode::Select => "Select",
            CursorMode::Teleport => "Teleport",
        }
    ));
}

fn sys_teleport_on_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
    cursor: Res<CursorBehavior>,
    settings: Res<crate::external_data::settings::Settings>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut chunk_recompute_writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
    if cursor.mode != CursorMode::Teleport {
        return;
    }

    // If egui wants pointer input, don't teleport
    if let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) {
        if ctx.wants_pointer_input() {
            return;
        }
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(window) = windows.single().ok() else {
        return;
    };

    let cursor_pos = match window.cursor_position() {
        Some(pos) => pos,
        None => return,
    };

    let Ok((cam, cam_tf)) = camera_q.single() else {
        return;
    };
    let ray = match cam.viewport_to_world(cam_tf, cursor_pos).ok() {
        Some(ray) => ray,
        None => return,
    };

    let dir_y = ray.direction.y;
    if dir_y.abs() <= 1e-6 {
        return;
    }

    let t = -ray.origin.y / dir_y;
    let hit = ray.origin + ray.direction * t;

    if let Ok((mut player, mut transform)) = player_q.single_mut() {
        let map = player
            .current_pos
            .map(|p| p.m)
            .unwrap_or(settings.core.world.start_p.m);
        let uo_pos = hit.to_uo_vec4(map);
        player.current_pos = Some(uo_pos);
        transform.translation = uo_pos.to_bevy_vec3_ignore_map();
        // Always force a full recompute so the new visible area loads immediately.
        chunk_recompute_writer.write(RecomputeVisibleChunksEvent {});

        ingame_sysmessage_logger::normal(format!(
            "Teleported to [{}, {}, {}, {}] via cursor",
            uo_pos.x, uo_pos.y, uo_pos.z, uo_pos.m
        ));
    }
}
