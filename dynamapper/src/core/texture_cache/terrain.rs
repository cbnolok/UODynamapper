pub mod cache;
pub mod texarray;

use bevy::prelude::*;
use crate::{fname, impl_tracked_plugin, util_lib::tracked_plugin::*};

pub struct TerrainCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainCachePlugin);

impl Plugin for TerrainCachePlugin
{
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(Startup, sys_setup_terrain_cache);
    }
}

pub fn sys_setup_terrain_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
) {
    log_system_add_startup::<TerrainCachePlugin>(fname!());
    let handle = texarray::create_gpu_texture_array("terrain_cache", &mut images); //, &rd);
    cmd.insert_resource(cache::TextureCache::new(handle));
}
