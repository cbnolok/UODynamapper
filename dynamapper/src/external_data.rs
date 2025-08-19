pub mod settings;
pub mod shader_presets;

use crate::{
    external_data::{settings::SettingsPlugin, shader_presets::ShaderPresetsPlugin},
    impl_tracked_plugin,
    util_lib::tracked_plugin::*,
};
use bevy::prelude::*;

pub struct ExternalDataPlugin {
    pub registered_by: &'static str,
}

impl_tracked_plugin!(ExternalDataPlugin);

impl Plugin for ExternalDataPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            SettingsPlugin {
                registered_by: "ExternalDataPlugin",
            },
            ShaderPresetsPlugin {
                registered_by: "ExternalDataPlugin",
            },
        ));
    }
}
