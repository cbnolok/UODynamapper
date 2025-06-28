pub mod camera;
pub mod player;
pub mod scene;

use bevy::prelude::*;
use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};

pub struct WorldPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WorldPlugin);
impl Plugin for WorldPlugin
{
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app
            .add_plugins((
                camera::CameraPlugin { registered_by: "WorldPlugin" },
                player::PlayerPlugin { registered_by: "WorldPlugin" },
                scene::ScenePlugin   { registered_by: "WorldPlugin" },
            ));
    }
}

