pub mod player_movement;

use crate::prelude::*;
use bevy::prelude::*;

pub struct ControlsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ControlsPlugin);
impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((player_movement::PlayerMovementPlugin {
            registered_by: "ControlsPlugin",
        },));
    }
}
