pub mod dynamic_light;
pub mod terrain_chunk_mesh;

use bevy::prelude::*;
use std::cmp::{max, min};
use crate::{fname, impl_tracked_plugin, util_lib::tracked_plugin::*};
use terrain_chunk_mesh::CHUNK_TILE_NUM_1D;

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
            terrain_chunk_mesh::TerrainChunkMeshPlugin  { registered_by: "ScenePlugin" },
            dynamic_light::PlayerDynamicLightPlugin     { registered_by: "ScenePlugin" },
        ))
        .add_systems(Startup, sys_spawn_worldmap_chunks)
        .insert_resource(SceneStartupData {
            player_start_pos: Vec3 {
                x: 24.0,
                y: 0.0,
                z: 24.0,
            },
        })
        .insert_resource(SceneUpdateData {
            chunk_draw_range: 2, // In 'chunk units'
        });
    }
}

#[derive(Resource)]
pub struct SceneStartupData {
    /// In tile/world units.
    pub player_start_pos: Vec3,
}

#[derive(Resource)]
pub struct SceneUpdateData {
    /// This number has to be a multiple of two! It's in 'chunk units', not tile coords.
    pub chunk_draw_range: i32,
}

/// Spawn a N×N grid of placeholder chunks.
pub fn sys_spawn_worldmap_chunks(
    mut commands: Commands,
    //cam_q: Query<&Transform, With<Camera3d>>,
    scene_startup_data_res: Option<Res<SceneStartupData>>,
    scene_update_data_res: Option<Res<SceneUpdateData>>,
) {
    log_system_add_startup::<ScenePlugin>(fname!());
    // TODO: check (via screen size and zoom) the amount of chunks to spawn.
    let player_start_pos =  // we need to convert this from tile/world position to chunk coords
        scene_startup_data_res.unwrap().player_start_pos / CHUNK_TILE_NUM_1D as f32;
    let chunk_draw_range = scene_update_data_res.unwrap().chunk_draw_range;
    let chunk_x0 = max(0, (player_start_pos.x as i32) - (chunk_draw_range / 2)) as u32;
    let chunk_y0 = max(0, (player_start_pos.y as i32) - (chunk_draw_range / 2)) as u32;
    let chunk_x1 = min(
        DUMMY_MAP_SIZE_X as i32,
        (player_start_pos.x as i32) + (chunk_draw_range / 2),
    ) as u32;
    let chunk_y1 = min(
        DUMMY_MAP_SIZE_Y as i32,
        (player_start_pos.y as i32) + (chunk_draw_range / 2),
    ) as u32;
    for gx in chunk_x0..=chunk_x1 {
        for gy in chunk_y0..=chunk_y1 {
            commands.spawn((
                terrain_chunk_mesh::TCMesh { gx, gy },
                Transform::default(),
                GlobalTransform::default(),
            ));

            println!("Spawned chunk at: gx={}, gy={}", gx, gy);
        }
    }
}
