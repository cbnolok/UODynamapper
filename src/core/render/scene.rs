pub mod terrain_chunk_mesh;

use std::cmp::{max, min};
use bevy::prelude::*;
use crate::core::render::scene::terrain_chunk_mesh::CHUNK_TILE_NUM_1D;

pub const DUMMY_MAP_SIZE_X: u32 = 4096;
pub const DUMMY_MAP_SIZE_Y: u32 = 7120;

pub struct ScenePlugin;
impl Plugin for ScenePlugin
{
    fn build(&self, app: &mut App) {
        app
        .add_plugins(terrain_chunk_mesh::TerrainChunkMeshPlugin)
        .add_systems(Startup, sys_spawn_worldmap_chunks)
        .insert_resource(SceneStartupData {
            player_start_pos: Vec3{x: 24.0, y: 0.0, z: 24.0},
        })
        .insert_resource(SceneUpdateData {
            chunk_draw_range: 2,    // In 'chunk units'
        });
    }
}

#[derive(Resource)]
pub struct SceneStartupData {
    pub player_start_pos: Vec3,
}

#[derive(Resource)]
pub struct SceneUpdateData {
    /// Multiples of two!
    pub chunk_draw_range: i32,
}

// Spawn a 3×3 grid of placeholder chunks.
pub fn sys_spawn_worldmap_chunks(
    mut commands: Commands,
    //cam_q: Query<&Transform, With<Camera3d>>,
    scene_startup_data_res: Option<Res<SceneStartupData>>,
    scene_update_data_res: Option<Res<SceneUpdateData>>,
) {
    // TODO: check (via screen size and zoom) the amount of chunks to spawn.
    let player_start_pos = scene_startup_data_res.unwrap().player_start_pos / CHUNK_TILE_NUM_1D as f32;
    let chunk_draw_range = scene_update_data_res.unwrap().chunk_draw_range;
    let chunk_x0 = max(0, (player_start_pos.x as i32) - (chunk_draw_range / 2)) as u32;
    let chunk_y0 = max(0, (player_start_pos.y as i32) - (chunk_draw_range / 2)) as u32;
    let chunk_x1 = min(DUMMY_MAP_SIZE_X as i32, (player_start_pos.x as i32) + (chunk_draw_range / 2)) as u32;
    let chunk_y1 = min(DUMMY_MAP_SIZE_Y as i32, (player_start_pos.y as i32) + (chunk_draw_range / 2)) as u32;
    for gx in chunk_x0..=chunk_x1 {
        for gy in chunk_y0..=chunk_y1 {
            commands.spawn((
                terrain_chunk_mesh::TCMesh { gx, gy },
                Transform::default(),
                GlobalTransform::default(),
            ));

            println!(
                "Spawned chunk at: gx={}, gy={}",
                gx, gy
            );
        }
    }
}

