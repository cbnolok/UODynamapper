use crate::{
    core::render::{dialogs::get_egui_context_ready, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use crate::core::render::scene::player::Player;
use crate::ingame_logger;

#[derive(Resource, Default)]
pub struct TeleportDialogState {
    pub open: bool,
    pub x: String,
    pub y: String,
    pub z: String,
    pub m: String,
}

pub struct TeleportPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TeleportPlugin);

impl Plugin for TeleportPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<TeleportDialogState>()
            .add_systems(Update, sys_toggle_teleport_dialog.run_if(in_state(AppState::InGame)))
            .add_systems(EguiPrimaryContextPass, sys_render_teleport_dialog.run_if(in_state(AppState::InGame)));
    }
}

fn sys_toggle_teleport_dialog(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<TeleportDialogState>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    if let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    if keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight) {
        if keyboard.just_pressed(KeyCode::KeyG) {
            state.open = !state.open;
            if state.open {
                // Initialize with current values could be nice, but leaving empty for now is fine
            }
        }
    }
    
    if keyboard.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
    }
}

pub fn sys_render_teleport_dialog(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut state: ResMut<TeleportDialogState>,
    mut player_q: Query<(&mut Player, &mut Transform)>,
) {
    if !state.open {
        return;
    }

    // Try to get the egui context - if it fails, skip rendering this frame
    let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    egui::Window::new("Teleport (GoTo)").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("X:");
            ui.text_edit_singleline(&mut state.x);
        });
        ui.horizontal(|ui| {
            ui.label("Y:");
            ui.text_edit_singleline(&mut state.y);
        });
        ui.horizontal(|ui| {
            ui.label("Z:");
            ui.text_edit_singleline(&mut state.z);
        });
        ui.horizontal(|ui| {
            ui.label("M:");
            ui.text_edit_singleline(&mut state.m);
        });

        if ui.button("Teleport").clicked() {
            if let (Ok(x), Ok(y), Ok(z), Ok(m)) = (
                state.x.parse::<u16>(),
                state.y.parse::<u16>(),
                state.z.parse::<i8>(),
                state.m.parse::<u8>(),
            ) {
                if let Ok((mut player, mut transform)) = player_q.single_mut() {
                    let uo_pos = UOVec4::new(x, y, z, m);
                    player.current_pos = Some(uo_pos);
                    let bevy_pos = uo_pos.to_bevy_vec3_ignore_map();
                    transform.translation = bevy_pos;
                    
                    ingame_logger::normal(format!("Teleported to [{}, {}, {}, {}]", x, y, z, m));
                    state.open = false;
                }
            }
        }
    });
}
