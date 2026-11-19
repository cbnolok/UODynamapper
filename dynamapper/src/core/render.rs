pub(crate) mod assets;
pub mod dialogs;
pub mod lighting;
pub mod terrain;
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
        app.add_plugins(crate::diagnostics::gpu_profiling::LandProfilingPlugin {
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
            scene::world::effects::fog_pass::FogPostProcessPlugin {
                registered_by: "RenderPlugin",
            },
            lighting::client_night::ClientNightLightingPlugin {
                registered_by: "RenderPlugin",
            },
        ));
    }
}
