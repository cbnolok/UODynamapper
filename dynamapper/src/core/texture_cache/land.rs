pub mod cache;
pub mod texture_array;

use crate::prelude::*;
use crate::core::system_sets::*;
use bevy::prelude::*;
use uocf::geo::land_texture_2d::LandTextureSize;

pub struct LandCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(LandCachePlugin);

impl Plugin for LandCachePlugin {
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_terrain_cache
                .in_set(StartupSysSet::SetupSceneStage1)
                .after(StartupSysSet::LoadStartupUOFiles)
        );
    }
}

pub fn sys_setup_terrain_cache(mut cmd: Commands, mut images: ResMut<Assets<Image>>) {
    log_system_add_startup::<LandCachePlugin>(StartupSysSet::SetupSceneStage1, fname!());

    let handle_small = texture_array::create_gpu_texture_array("land_small_texture_cache", &mut images, LandTextureSize::Small);
    let handle_big = texture_array::create_gpu_texture_array("land_big_texture_cache", &mut images, LandTextureSize::Big);
    cmd.insert_resource(cache::LandTextureCache::new(handle_small, handle_big));
}
