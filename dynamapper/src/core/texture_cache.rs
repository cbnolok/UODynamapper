pub mod land;

use bevy::prelude::*;
use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};

pub struct TextureCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TextureCachePlugin);

impl Plugin for TextureCachePlugin
{
    /// Allocate GPU texture array and Tile Caches.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(land::LandTextureCachePlugin { registered_by: "TextureCachePlugin" });
    }
}

