pub mod keybindings_help;
pub mod gump;
pub mod image_background;
pub mod paperdoll_ec;
pub mod paperdoll_overrides;
pub mod preferences;
pub mod teleport;
pub mod land_shader;

use crate::core::render::scene::camera::UiCameraResource;
use crate::impl_tracked_plugin;
use crate::prelude::*;
use crate::util_lib::tracked_plugin::*;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use gump::GumpDialogPlugin;
use keybindings_help::KeybindingsHelpPlugin;
use land_shader::LandUiPlugin;
use preferences::PreferencesDialogPlugin;
use teleport::TeleportPlugin;

pub struct DialogsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DialogsPlugin);

impl Plugin for DialogsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            PreferencesDialogPlugin {
                registered_by: "DialogsPlugin",
            },
            LandUiPlugin {
                registered_by: "DialogsPlugin",
            },
            KeybindingsHelpPlugin {
                registered_by: "DialogsPlugin",
            },
            GumpDialogPlugin {
                registered_by: "DialogsPlugin",
            },
            TeleportPlugin {
                registered_by: "DialogsPlugin",
            },
        ))
        .add_systems(Update, sys_sync_egui_context_scale_factor);
    }
}

fn sys_sync_egui_context_scale_factor(
    settings: Res<crate::configs::settings::Settings>,
    egui_ui_camera: Res<UiCameraResource>,
    mut egui_contexts: EguiContexts,
) {
    // `EguiContextSettings::scale_factor` was removed: UI scale is now egui's own
    // zoom factor, which bevy_egui only mirrors into `EguiZoomFactor`.
    let Some(ctx) = get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let target_scale = if settings.app.window.ui_scale.is_finite() {
        settings.app.window.ui_scale.clamp(0.5, 3.0)
    } else {
        1.0
    };

    if (ctx.zoom_factor() - target_scale).abs() > 0.001 {
        log_system_add_update::<DialogsPlugin>(fname!());
        ctx.set_zoom_factor(target_scale);
    }
}

/// Standard helper to get the egui context for the primary UI camera.
pub fn get_egui_context_ready_mut<'a>(
    egui_contexts: &'a mut EguiContexts,
    _egui_ui_camera: &Res<UiCameraResource>,
) -> Option<&'a mut bevy_egui::egui::Context> {
    // Reverted to standard egui: use the primary window context.
    egui_contexts.ctx_mut().ok()
}

/*
// No significant speed gain by using the immutable context (which is also discouraged in the bevy_egui code comments)

pub fn get_egui_context_ready<'a>(
    egui_contexts: &'a EguiContexts,
    _egui_ui_camera: &Res<UiCameraResource>,
) -> Option<&'a bevy_egui::egui::Context> {
    // Reverted to standard egui: use the primary window context.
    egui_contexts.ctx().ok()
}
*/
