pub mod window_controls;
pub mod input_actions;
pub mod player_movement;

use crate::prelude::*;
use bevy::prelude::*;
use bevy::ecs::message::MessageReader;

pub struct ControlsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ControlsPlugin);
impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            player_movement::PlayerMovementPlugin {
                registered_by: "ControlsPlugin",
            },
            window_controls::WindowControlsPlugin {
                registered_by: "ControlsPlugin",
            },
        ));

        // CENTRAL INPUT DISPATCHER (Event-driven)
        app.add_systems(Update, sys_dispatch_keyboard_input);
    }
}

fn sys_dispatch_keyboard_input(
    mut evr_kbd: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut commands: Commands,
    settings: Res<Settings>,
    kbd_input: Res<ButtonInput<KeyCode>>,
    mut egui_contexts: bevy_egui::EguiContexts,
) {
    // Check if egui is currently using the keyboard to block global actions.
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    for event in evr_kbd.read() {
        if !event.state.is_pressed() {
            continue;
        }

        use input_actions::*;
        let key = event.key_code;

        // Helper for modifiers
        let ctrl = kbd_input.pressed(KeyCode::ControlLeft) || kbd_input.pressed(KeyCode::ControlRight);
        let alt = kbd_input.pressed(KeyCode::AltLeft) || kbd_input.pressed(KeyCode::AltRight);

        // Core UI Toggles
        if key == settings.keybindings.keybindings_help {
            commands.trigger(ActionToggleKeybindingsHelp);
        } else if key == settings.keybindings.user_settings {
            commands.trigger(ActionTogglePreferences);
        } else if key == settings.keybindings.shader_settings {
            commands.trigger(ActionToggleShaderSettings);
        } else if key == KeyCode::Escape {
            commands.trigger(ActionCloseActiveDialog);
        }
        // Combined actions
        else if key == KeyCode::F11 || (key == KeyCode::Enter && alt) {
            commands.trigger(ActionToggleFullscreen);
        } else if key == KeyCode::KeyG && ctrl {
            commands.trigger(ActionToggleTeleportDialog);
        } else if key == KeyCode::KeyT && ctrl {
            commands.trigger(ActionToggleCursorTeleportMode);
        } else if key == KeyCode::KeyI && !ctrl && !alt {
            commands.trigger(ActionToggleCursorInspectPanel);
        }
    }
}
