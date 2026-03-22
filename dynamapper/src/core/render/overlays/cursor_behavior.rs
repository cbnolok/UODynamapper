use crate::core::render::{dialogs::get_egui_context_ready, scene::camera::{PlayerCamera, UiCameraResource}};
use crate::core::render::scene::player::Player;
use crate::ingame_sysmessage_logger;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;

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

impl Plugin for CursorBehaviorOverlayPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<CursorBehavior>()
            .add_systems(Update, sys_toggle_cursor_mode.run_if(in_state(AppState::InGame)))
            .add_systems(Update, sys_teleport_on_click.run_if(in_state(AppState::InGame)));
    }
}

fn sys_toggle_cursor_mode(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut cursor: ResMut<CursorBehavior>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    if let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    if keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight) {
        if keyboard.just_pressed(KeyCode::KeyT) {
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
    }
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

    let Ok((cam, cam_tf)) = camera_q.single() else { return };
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
        let map = player.current_pos.map(|p| p.m).unwrap_or(settings.core.world.start_p.m);
        let uo_pos = hit.to_uo_vec4(map);
        player.current_pos = Some(uo_pos);
        transform.translation = uo_pos.to_bevy_vec3_ignore_map();

        ingame_sysmessage_logger::normal(format!(
            "Teleported to [{}, {}, {}, {}] via cursor",
            uo_pos.x, uo_pos.y, uo_pos.z, uo_pos.m
        ));
    }
}
