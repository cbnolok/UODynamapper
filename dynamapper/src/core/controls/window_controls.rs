use crate::prelude::*;
use bevy::prelude::*;
use bevy::window::WindowMode;

pub struct WindowControlsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WindowControlsPlugin);

impl Plugin for WindowControlsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(FixedUpdate, sys_handle_window_controls);
    }
}

fn sys_handle_window_controls(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut Window>,
) {
    let alt = keyboard_input.pressed(KeyCode::AltLeft) || keyboard_input.pressed(KeyCode::AltRight);
    let enter = keyboard_input.just_pressed(KeyCode::Enter);
    let f11 = keyboard_input.just_pressed(KeyCode::F11);

    if f11 || (alt && enter) {
        if let Ok(mut window) = windows.single_mut() {
            window.mode = match window.mode {
                WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                _ => WindowMode::Windowed,
            };
        }
    }
}
