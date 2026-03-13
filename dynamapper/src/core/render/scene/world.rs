pub mod land;

use std::collections::HashMap;
use bevy::prelude::*;
use crate::prelude::*;
use crate::core::maps::MapPlaneMetadata;


use crate::core::uo_files_loader::MapPlanesRes;
use crate::core::system_sets::StartupSysSet;

#[derive(Resource, Default)]
pub struct WorldGeoData {
    pub maps: HashMap<u32, MapPlaneMetadata>,
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
            .init_resource::<WorldGeoData>()
            .add_systems(
                Startup,
                sys_populate_world_geo_data.after(StartupSysSet::LoadStartupUOFiles),
            )
            .add_plugins(
                land::DrawLandChunkMeshPlugin { registered_by: "WorldPlugin" },
            );
    }
}

fn sys_populate_world_geo_data(
    map_planes_res: Res<MapPlanesRes>,
    mut world_geo_data_res: ResMut<WorldGeoData>,
) {
    if world_geo_data_res.maps.is_empty() {
        for entry in map_planes_res.0.iter() {
            let (&id, plane) = entry.pair();
            world_geo_data_res.maps.insert(id, MapPlaneMetadata {
                id: id as u8,
                width: plane.size_blocks.width * 8, 
                height: plane.size_blocks.height * 8,
            });
            console_logger::one(
                None,
                LogSev::Info,
                LogAbout::RenderWorldLand,
                &format!("Populated WorldGeoData for map {id} ({}x{} tiles).", plane.size_blocks.width * 8, plane.size_blocks.height * 8),
            );
        }
    }
}

