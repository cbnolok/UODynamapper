pub mod performance;
pub mod player_position;
pub mod cursor;
pub mod sysmessages;

use crate::prelude::*;
use bevy::prelude::*;
use performance::PerformanceOverlayPlugin;
use player_position::PlayerPositionOverlayPlugin;
use cursor::CursorOverlayPlugin;
use sysmessages::SystemMessagesPlugin;

pub struct OverlaysPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(OverlaysPlugin);

impl Plugin for OverlaysPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            PerformanceOverlayPlugin {
                registered_by: "OverlaysPlugin",
            },
            PlayerPositionOverlayPlugin {
                registered_by: "OverlaysPlugin",
            },
            CursorOverlayPlugin {
                registered_by: "OverlaysPlugin",
            },
            SystemMessagesPlugin {
                registered_by: "OverlaysPlugin",
            },
        ));
    }
}
