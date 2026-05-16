pub mod dialogs;
pub mod fog_post_process;
pub mod overlays;
pub mod scene;

use crate::prelude::*;
use bevy::prelude::*;

pub struct RenderPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(RenderPlugin);
impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        #[cfg(feature = "gpu-profiling")]
        app.add_plugins(scene::world::land::profiling::LandProfilingPlugin {
            registered_by: "RenderPlugin",
        });
        app.add_plugins((
            scene::ScenePlugin {
                registered_by: "RenderPlugin",
            },
            overlays::OverlaysPlugin {
                registered_by: "RenderPlugin",
            },
            dialogs::DialogsPlugin {
                registered_by: "RenderPlugin",
            },
            fog_post_process::FogPostProcessPlugin {
                registered_by: "RenderPlugin",
            },
        ));
    }
}
