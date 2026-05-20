pub mod land;
pub mod art;
pub mod effects;

use nohash_hasher::BuildNoHashHasher;
use indexmap::IndexMap;
use bevy::prelude::*;
use crate::prelude::*;
use crate::core::maps::MapPlaneMetadata;


use crate::core::uo_files_loader::MapPlanesRes;
use crate::core::system_sets::StartupSysSet;

#[derive(Resource, Default)]
pub struct WorldGeoData {
    pub maps: IndexMap<u32, MapPlaneMetadata, BuildNoHashHasher<u32>>,
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
                sys_populate_world_geo_data
                    .in_set(StartupSysSet::SetupSceneStage1)
                    .after(StartupSysSet::LoadStartupUOFiles),
            )
            .add_plugins((
                land::DrawLandChunkMeshPlugin { registered_by: "WorldPlugin" },
                art::DrawStaticSpritesPlugin { registered_by: "WorldPlugin" },
            ));
    }
}

fn sys_populate_world_geo_data(
    map_planes_res: Option<Res<MapPlanesRes>>,
    mut world_geo_data_res: ResMut<WorldGeoData>,
) {
    log_system_add_startup::<WorldPlugin>(StartupSysSet::SetupSceneStage1, fname!());
    let Some(map_planes_res) = map_planes_res else {
        return;
    };
    if world_geo_data_res.maps.is_empty() {
        for (id, plane) in map_planes_res.0.iter().enumerate() {
            // MapPlanesRes now contains Option<MapPlane> indexed by map ID.
            let Some(plane) = plane else { continue; };
            let id = id as u32;
            world_geo_data_res.maps.insert(id, MapPlaneMetadata {
                id: id as u8,
                width: plane.size_blocks.width * 8,
                height: plane.size_blocks.height * 8,
            });
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldLand,
                &format!("Populated WorldGeoData for map {id} ({}x{} tiles).", plane.size_blocks.width * 8, plane.size_blocks.height * 8),
            );
        }
    }
}
