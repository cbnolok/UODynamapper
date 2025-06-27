pub mod cache;
pub mod texarray;

use bevy::prelude::*;

pub struct TerrainCachePlugin;

impl Plugin for TerrainCachePlugin
{
    /// Allocate GPU texture array and TileCache.
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, sys_setup_terrain_cache);
    }    
}

pub fn sys_setup_terrain_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
) {
    let handle = texarray::create_gpu_texture_array("terrain_cache", &mut images); //, &rd);
    cmd.insert_resource(cache::TextureCache::new(handle));
}
