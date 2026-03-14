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
        ))
        // Apply global egui scaling. We run this in PreUpdate to set the scale
        // before any UI rendering occurs in this frame.
        .add_systems(PreUpdate, sys_apply_global_egui_scale);
    }
}

/// Standard helper to get the egui context for the primary UI camera.
fn get_egui_context_ready<'a>(
    egui_contexts: &'a mut EguiContexts,
    egui_ui_camera: &Res<UiCameraResource>,
) -> Option<&'a mut bevy_egui::egui::Context> {
    let entity = egui_ui_camera.0?;
    egui_contexts.ctx_for_entity_mut(entity).ok()
}

fn sys_apply_global_egui_scale(
    settings: Res<crate::external_data::settings::Settings>,
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
) {
    let target_scale = settings.app.window.egui_scale;
    if let Some(entity) = egui_ui_camera.0 {
        if let Ok(ctx) = egui_contexts.ctx_for_entity_mut(entity) {
            if (ctx.pixels_per_point() - target_scale).abs() > 0.001 {
                ctx.set_pixels_per_point(target_scale);
            }
        }
    }
}
