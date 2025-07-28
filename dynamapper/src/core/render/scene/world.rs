pub mod land;

use std::collections::HashMap;
use bevy::prelude::*;
use crate::prelude::*;
use crate::core::maps::MapPlaneMetadata;


const DUMMY_MAP_SIZE_X: u32 = 4096;
const DUMMY_MAP_SIZE_Y: u32 = 7120;

#[derive(Resource)]
pub struct WorldGeoData {
    pub maps: HashMap<u32, MapPlaneMetadata>,
}
impl Default for WorldGeoData {
    fn default() -> Self {
        let mut def = Self {maps: HashMap::new()};
        def.maps.insert(0, MapPlaneMetadata {
            id: 0,
            width: DUMMY_MAP_SIZE_X,
            height: DUMMY_MAP_SIZE_Y,
        });
        def
    }
}

pub struct WorldPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(WorldPlugin);
impl Plugin for WorldPlugin
{
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app
            .insert_resource(WorldGeoData::default())
            .add_plugins(
                land::DrawLandChunkMeshPlugin { registered_by: "WorldPlugin" },
            );
    }
}

