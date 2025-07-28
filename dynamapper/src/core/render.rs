pub mod overlays;
pub mod scene;

use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};
use bevy::prelude::*;

pub struct RenderPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(RenderPlugin);
impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            scene::ScenePlugin {
                registered_by: "RenderPlugin",
            },
            overlays::OverlaysPlugin {
                registered_by: "RenderPlugin",
            },
        ));
    }
}
