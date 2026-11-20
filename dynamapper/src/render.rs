pub(crate) mod assets;
pub mod lighting;
pub mod terrain;
pub mod uniform_state;
pub mod postprocess;
pub(crate) mod picking;
pub mod pipeline_readiness;
pub mod scene;
pub mod statics;
pub mod view;

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
            crate::ui::overlays::OverlaysPlugin {
                registered_by: "RenderPlugin",
            },
            crate::ui::dialogs::DialogsPlugin {
                registered_by: "RenderPlugin",
            },
            postprocess::fog_pass::FogPostProcessPlugin {
                registered_by: "RenderPlugin",
            },
            lighting::client_night::ClientNightLightingPlugin {
                registered_by: "RenderPlugin",
            },
        ));
    }
}
