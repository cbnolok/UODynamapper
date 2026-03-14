use crate::{
    core::render::{dialogs::get_egui_context_ready, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

#[derive(Resource, Default)]
pub struct KeybindingsHelpState {
    pub open: bool,
}

pub struct KeybindingsHelpPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(KeybindingsHelpPlugin);

impl Plugin for KeybindingsHelpPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeybindingsHelpState>()
            .add_systems(Update, sys_toggle_keybindings_help)
            .add_systems(EguiPrimaryContextPass, sys_render_keybindings_help);
    }
}

fn sys_toggle_keybindings_help(
    keyboard: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut state: ResMut<KeybindingsHelpState>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    if let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) {
        if ctx.wants_keyboard_input() {
            return;
        }
    }
    if keyboard.just_pressed(settings.keybindings.keybindings_help) {
        state.open = !state.open;
    }
    if keyboard.just_pressed(KeyCode::Escape) && state.open {
        state.open = false;
    }
}

pub fn sys_render_keybindings_help(
    mut state: ResMut<KeybindingsHelpState>,
    settings: Res<Settings>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    // Try to get the egui context - if it fails, skip rendering this frame
    let Some(ctx) = get_egui_context_ready(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let mut window_open = state.open;
    let title = format!(
        "Keybindings Help [{:?}]",
        settings.keybindings.keybindings_help
    );

    egui::Window::new(title)
        .default_pos([400.0, 80.0])
        .collapsible(false)
        .resizable(false)
        .open(&mut window_open)
        .show(ctx, |ui| {
            ui.heading("Keybindings");
            ui.separator();

            ui.heading("General");
            egui::Grid::new("gen_grid").num_columns(2).spacing([40.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("Keybindings Help");
                ui.label(format!("{:?}", settings.keybindings.keybindings_help));
                ui.end_row();

                ui.label("Options Menu");
                ui.label(format!("{:?}", settings.keybindings.user_settings));
                ui.end_row();

                ui.label("Shader Settings");
                ui.label(format!("{:?}", settings.keybindings.shader_settings));
                ui.end_row();

                ui.label("Toggle Fullscreen");
                ui.label("F11 / Alt+Enter");
                ui.end_row();

                ui.label("Close Dialog");
                ui.label("Escape");
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.heading("Player Movement");
            egui::Grid::new("move_grid").num_columns(2).spacing([40.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("Walk/Run");
                ui.label("W, A, S, D");
                ui.end_row();

                ui.label("Move to Cursor");
                ui.label("Right Click (Hold)");
                ui.end_row();

                ui.label("Altitude Up/Down");
                ui.label("PageUp / PageDown");
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.heading("Features");
            egui::Grid::new("feat_grid").num_columns(2).spacing([40.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("Teleport Dialog");
                ui.label("Ctrl + G");
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.heading("Free Camera Control");
            egui::Grid::new("cam_grid").num_columns(2).spacing([40.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("Pan");
                ui.label("Arrows");
                ui.end_row();

                ui.label("Altitude");
                ui.label("Shift + Arrows");
                ui.end_row();

                ui.label("Pitch (X-Axis)");
                ui.label("R-Shift + W / S");
                ui.end_row();

                ui.label("Yaw (Y-Axis)");
                ui.label("R-Shift + A / D");
                ui.end_row();

                ui.label("Roll (Z-Axis)");
                ui.label("R-Shift + Q / E");
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.label("Note: Keybindings can be modified in 'assets/keybindings.toml'.");
        });

    state.open = window_open;
}
