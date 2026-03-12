pub mod performance;
pub mod player_position;

use bevy::prelude::*;
use crate::prelude::*;
use performance::PerformanceOverlayPlugin;
use player_position::PlayerPositionOverlayPlugin;

pub struct OverlaysPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(OverlaysPlugin);

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((PerformanceOverlayPlugin, PlayerPositionOverlayPlugin));
    }
}
