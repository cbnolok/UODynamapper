pub mod performance;
pub mod player_position;
pub mod sysmessages;

use crate::prelude::*;
use bevy::prelude::*;
use performance::PerformanceOverlayPlugin;
use player_position::PlayerPositionOverlayPlugin;
use sysmessages::SystemMessagesPlugin;

pub struct OverlaysPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(OverlaysPlugin);

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            PerformanceOverlayPlugin,
            PlayerPositionOverlayPlugin,
            SystemMessagesPlugin,
        ));
    }
}
