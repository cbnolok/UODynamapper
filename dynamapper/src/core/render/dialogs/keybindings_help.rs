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

fn sys_render_keybindings_help(
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

            egui::Grid::new("keybindings_grid")
                .num_columns(2)
                .spacing([40.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Keybindings Help");
                    ui.label(format!("{:?}", settings.keybindings.keybindings_help));
                    ui.end_row();

                    ui.label("Options Menu");
                    ui.label(format!("{:?}", settings.keybindings.user_settings));
                    ui.end_row();

                    ui.label("Shader Settings");
                    ui.label(format!("{:?}", settings.keybindings.shader_settings));
                    ui.end_row();

                    ui.label("Close Dialog");
                    ui.label("Escape");
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.label("Note: Keybindings can be modified in 'assets/keybindings.toml'.");
        });

    state.open = window_open;
}
