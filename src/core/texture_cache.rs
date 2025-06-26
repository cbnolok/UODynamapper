pub mod terrain;

use bevy::prelude::*;

pub struct TextureCachePlugin;

impl Plugin for TextureCachePlugin
{
    /// Allocate GPU texture array and TileCache.
    fn build(&self, app: &mut App) {
        app.add_plugins(terrain::TerrainCachePlugin);
    }    
}
