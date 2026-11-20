
use crate::render::terrain;

use bevy::prelude::*;
use crate::prelude::*;
use crate::world::maps::{sys_populate_world_geo_data, WorldGeoData};
use crate::world::{caves, z_view};


use crate::app::system_sets::{MovementSystems, SceneRenderArtSystems, StartupSystems};

/// Wires the world into the frame: the map catalog, the world analyses (caves, floor-view cut) and the land and
/// statics draw plugins.
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
            .init_resource::<z_view::ZViewLimit>()
            .init_resource::<caves::CavesRes>()
            .init_resource::<crate::world::multis::PlacedMultisRes>()
            .add_systems(
                Startup,
                sys_populate_world_geo_data
                    .in_set(StartupSystems::SetupSceneStage1)
                    .after(StartupSystems::LoadStartupUOFiles),
            )
            .add_systems(
                Update,
                z_view::sys_update_z_view_limit
                    .after(MovementSystems::MovementActions)
                    .before(SceneRenderArtSystems::CollectVisibleStatics)
                    .before(crate::render::lighting::static_lights::sys_collect_visible_static_lights)
                    .before(crate::render::lighting::client_light_sprites::sys_collect_client_light_sprites)
                    .before(terrain::sys_update_shared_land_material)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                Update,
                caves::sys_update_player_cave
                    .after(MovementSystems::MovementActions)
                    .before(z_view::sys_update_z_view_limit)
                    .before(caves::sys_update_caves)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                Update,
                caves::sys_update_caves
                    .before(SceneRenderArtSystems::CollectVisibleStatics)
                    .before(crate::render::lighting::static_lights::sys_collect_visible_static_lights)
                    .before(crate::render::lighting::client_light_sprites::sys_collect_client_light_sprites)
                    .before(terrain::draw_mesh::sys_draw_spawned_land_chunks)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_plugins((
                terrain::DrawLandChunkMeshPlugin { registered_by: "WorldPlugin" },
                crate::render::statics::DrawStaticSpritesPlugin { registered_by: "WorldPlugin" },
                crate::render::mobiles::MobileSpritesPlugin { registered_by: "WorldPlugin" },
            ));
    }
}
