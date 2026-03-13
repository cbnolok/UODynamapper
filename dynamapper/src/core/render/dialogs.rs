pub mod keybindings_help;
pub mod options;
pub mod teleport;
pub mod terrain_shader;

use crate::core::render::scene::camera::UiCameraResource;
use crate::impl_tracked_plugin;
use crate::util_lib::tracked_plugin::*;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use keybindings_help::KeybindingsHelpPlugin;
use options::OptionsDialogPlugin;
use teleport::TeleportPlugin;
use terrain_shader::TerrainUiPlugin;

pub struct DialogsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DialogsPlugin);

impl Plugin for DialogsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            OptionsDialogPlugin {
                registered_by: "DialogsPlugin",
            },
            TerrainUiPlugin {
                registered_by: "DialogsPlugin",
            },
            KeybindingsHelpPlugin {
                registered_by: "DialogsPlugin",
            },
            TeleportPlugin {
                registered_by: "DialogsPlugin",
            },
        ));
    }
}

fn get_egui_context_ready<'a>(
    egui_contexts: &'a mut EguiContexts,
    egui_ui_camera: &Res<UiCameraResource>,
) -> Option<&'a mut bevy_egui::egui::Context> {
    let ctx = egui_ui_camera.0.expect("No stored egui context?");
    let ctx = egui_contexts
        .ctx_for_entity_mut(ctx)
        .expect("Stored invalid egui context?");
    /*
    if ctx.wants_keyboard_input() {
        // Don't toggle if egui wants keyboard input (e.g., typing in a text field)
        return None;
    }
    */

    Some(ctx)
}
