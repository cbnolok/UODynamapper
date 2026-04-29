pub mod art;
pub mod land;
pub mod residency;

use bevy::prelude::*;
use crate::prelude::*;

// Shared abstractions live here; individual collections such as land texmaps build on
// them by providing their own grouping rules, byte loading path, and GPU upload logic.
pub use residency::{
    TextureResidencyGroupLayers,
    TextureResidencyPlan,
    TextureResidencyStrategy,
    resolve_layer_allocations,
    visit_grouped_layer_assignments,
    visit_grouped_texture_ids,
};

pub struct TextureCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TextureCachePlugin);

impl Plugin for TextureCachePlugin
{
    /// Allocate GPU texture array and Tile Caches.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(art::ArtTextureLoaderPlugin { registered_by: "TextureCachePlugin" });
        app.add_plugins(land::LandTextureCachePlugin { registered_by: "TextureCachePlugin" });
    }
}

