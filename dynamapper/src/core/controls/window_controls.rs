use crate::core::controls::input_actions::ActionToggleFullscreen;
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
        app.add_observer(sys_handle_fullscreen_toggle);
    }
}

fn sys_handle_fullscreen_toggle(
    _trigger: On<ActionToggleFullscreen>,
    mut windows: Query<&mut Window>,
) {
    if let Ok(mut window) = windows.single_mut() {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}
