pub mod draw_land_chunk_mesh;
pub mod dynamic_light;

use crate::core::constants;
use crate::core::system_sets::*;
use crate::prelude::*;
use crate::util_lib::math::*;
use bevy::prelude::*;
use draw_land_chunk_mesh::TILE_NUM_PER_CHUNK_1D;

pub const DUMMY_MAP_SIZE_X: u32 = 4096;
pub const DUMMY_MAP_SIZE_Y: u32 = 7120;

pub struct ScenePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ScenePlugin);

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins((
            draw_land_chunk_mesh::DrawLandChunkMeshPlugin {
                registered_by: "ScenePlugin",
            },
            dynamic_light::PlayerDynamicLightPlugin {
                registered_by: "ScenePlugin",
            },
        ))
        .insert_resource(SceneStartupData {
            player_start_pos: constants::PLAYER_START_P,
        })
        .insert_resource(SceneUpdateRules {
            chunk_draw_range: 4, // In 'chunk units' TODO: make it dynamic.
        })
        /*
        .insert_resource(SceneState {
            active_map: constants::PLAYER_START_P.m.into(),
        })
        */
        .insert_resource(SceneActiveMap {
            id: constants::PLAYER_START_P.m.into(),
            width: DUMMY_MAP_SIZE_X,
            height: DUMMY_MAP_SIZE_Y,
        })
        .add_systems(
            OnEnter(AppState::SetupScene),
            sys_spawn_worldmap_chunks_to_render.in_set(StartupSysSet::SetupScene),
        );
    }
}

#[derive(Resource)]
pub struct SceneStartupData {
    /// In tile/world units.
    pub player_start_pos: UOVec4,
}

#[derive(Resource)]
pub struct SceneUpdateRules {
    /// This number has to be a multiple of two! It's in 'chunk units', not tile coords.
    pub chunk_draw_range: u32,
}

#[derive(Resource)]
pub struct SceneActiveMap {
    pub id: u32,
    pub width: u32,
    pub height: u32,
}

/// Spawn a N×N grid of placeholder chunks (they will be rendered later).
pub fn sys_spawn_worldmap_chunks_to_render(
    mut commands: Commands,
    //cam_q: Query<&Transform, With<Camera3d>>,
    scene_startup_data_res: Res<SceneStartupData>,
    scene_update_data_res: Res<SceneUpdateRules>,
) {
    log_system_add_onenter::<ScenePlugin>(AppState::SetupScene, fname!());

    // TODO: check (via screen size and zoom) the amount of chunks to spawn.
    let player_start_chunk_coords =  // we need to convert this from tile/world position to chunk coords
        scene_startup_data_res.player_start_pos.to_bevy_vec3_ignore_map() / TILE_NUM_PER_CHUNK_1D as f32;

    let ux = f32_as_u32_expect(player_start_chunk_coords.x);
    let uy = f32_as_u32_expect(player_start_chunk_coords.z);
    let chunk_draw_range = scene_update_data_res.chunk_draw_range;
    let map = scene_startup_data_res.player_start_pos.m.into();

    let chunk_x0 = ux.saturating_sub(chunk_draw_range / 2);
    let chunk_y0 = uy.saturating_sub(chunk_draw_range / 2);
    let chunk_x1 = std::cmp::min(DUMMY_MAP_SIZE_X, ux + (chunk_draw_range / 2));
    let chunk_y1 = std::cmp::min(DUMMY_MAP_SIZE_Y, uy + (chunk_draw_range / 2));

    for gx in chunk_x0..=chunk_x1 {
        for gy in chunk_y0..=chunk_y1 {
            commands.spawn((
                draw_land_chunk_mesh::TCMesh { map, gx, gy },
                Transform::default(),
                GlobalTransform::default(),
            ));

            logger::one(
                None,
                LogSev::Debug,
                LogAbout::RenderWorldLand,
                format!("Spawned chunk at: gx={gx}, gy={gy}.").as_str(),
            );
        }
    }
}
