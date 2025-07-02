pub mod cache;
pub mod texarray;

use crate::prelude::*;
use crate::core::system_sets::*;
use bevy::prelude::*;

pub struct TerrainCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainCachePlugin);

impl Plugin for TerrainCachePlugin {
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            OnEnter(AppState::SetupScene),
            sys_setup_terrain_cache
                .in_set(StartupSysSet::SetupScene)
        );
    }
}

pub fn sys_setup_terrain_cache(mut cmd: Commands, mut images: ResMut<Assets<Image>>) {
    log_system_add_startup::<TerrainCachePlugin>(fname!());
    let handle = texarray::create_gpu_texture_array("terrain_cache", &mut images); //, &rd);
    cmd.insert_resource(cache::TextureCache::new(handle));
}
