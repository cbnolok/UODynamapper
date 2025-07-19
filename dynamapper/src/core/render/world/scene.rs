pub mod draw_land_chunk_mesh;
pub mod dynamic_light;

use crate::core::constants;
use crate::core::render::world::camera::{MAX_ZOOM, MIN_ZOOM, RenderZoom, UO_TILE_PIXEL_SIZE};
use crate::core::render::world::player::Player;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use draw_land_chunk_mesh::TILE_NUM_PER_CHUNK_1D;

pub const DUMMY_MAP_SIZE_X: u32 = 4096;
pub const DUMMY_MAP_SIZE_Y: u32 = 7120;

/// Plugin for scene setup, worldmap chunk management, and dynamic updates/despawns.
/// Now robust against map-plane switches and duplicated logic in chunk range handling.
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
        .insert_resource(SceneActiveMap {
            id: constants::PLAYER_START_P.m.into(),
            width: DUMMY_MAP_SIZE_X,
            height: DUMMY_MAP_SIZE_Y,
        })
        .insert_resource(ScenePlaneState::default())
        .add_systems(
            OnEnter(AppState::SetupScene),
            sys_spawn_worldmap_chunks_to_render.in_set(StartupSysSet::SetupScene),
        )
        .add_systems(
            Update,
            sys_update_worldmap_chunks_to_render.run_if(in_state(AppState::InGame)),
        );
    }
}

#[derive(Resource)]
pub struct SceneStartupData {
    pub player_start_pos: UOVec4,
}

#[derive(Resource)]
pub struct SceneActiveMap {
    pub id: u32,
    pub width: u32,
    pub height: u32,
}
/// Tracks the last map ID seen for detecting map switches.
#[derive(Resource, Default)]
pub struct ScenePlaneState {
    pub last_seen_map_id: Option<u32>,
}

fn log_chunk_spawn(gx: u32, gy: u32, map: u32) {
    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("Spawned chunk at: gx={gx}, gy={gy} (map={map})"),
    );
}

fn log_chunk_despawn(gx: u32, gy: u32, map: u32) {
    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("De-spawned chunk at: gx={gx}, gy={gy} (map={map})"),
    );
}

/// Calculates the set of visible chunk coordinates around the player,
/// sized so that the window is covered, even after padding, based on window size and zoom.
pub fn compute_visible_chunks(
    player_pos: Vec3,
    window_width: f32,
    window_height: f32,
    zoom: f32,
    map_width: u32,
    map_height: u32,
    tile_pixel_size: f32,
    chunk_size: u32, // e.g., TILE_NUM_PER_CHUNK_1D
    padding: u32,    // tiles to pad on all sides
) -> std::collections::HashSet<(u32, u32)> {
    // Visible tile region (rounded up with padding)
    let visible_tiles_x = (window_width / (tile_pixel_size * zoom)).ceil() as u32 + padding * 2;
    let visible_tiles_y = (window_height / (tile_pixel_size * zoom)).ceil() as u32 + padding * 2;
    // Convert player's position to TILE coordinates
    let player_tile_x = player_pos.x as i32;
    let player_tile_y = player_pos.z as i32; // y is z in Bevy "forward"

    // Compute chunk region to fully cover the visible area, *including all overlapping*
    // Start/end in TILES (not chunks yet)
    let half_tiles_x = (visible_tiles_x / 2) as i32;
    let half_tiles_y = (visible_tiles_y / 2) as i32;
    let tile_x0 = player_tile_x - half_tiles_x;
    let tile_x1 = player_tile_x + half_tiles_x;
    let tile_y0 = player_tile_y - half_tiles_y;
    let tile_y1 = player_tile_y + half_tiles_y;

    // Now convert these to chunk indices (and always round DOWN for min, UP for max)
    // so that *any partially overlapping chunk is included*.
    let chunk_x0 = (tile_x0.div_euclid(chunk_size as i32)).max(0);
    let chunk_x1 = ((tile_x1 as f32) / chunk_size as f32).ceil() as i32;
    let chunk_y0 = (tile_y0.div_euclid(chunk_size as i32)).max(0);
    let chunk_y1 = ((tile_y1 as f32) / chunk_size as f32).ceil() as i32;

    let map_chunks_x = (map_width / chunk_size) as i32;
    let map_chunks_y = (map_height / chunk_size) as i32;

    let mut set = std::collections::HashSet::new();
    for gx in chunk_x0..=chunk_x1.min(map_chunks_x - 1) {
        for gy in chunk_y0..=chunk_y1.min(map_chunks_y - 1) {
            set.insert((gx as u32, gy as u32));
        }
    }
    set
}

pub fn sys_spawn_worldmap_chunks_to_render(
    mut commands: Commands,
    scene_startup_data_res: Res<SceneStartupData>,
    scene_active_map: Res<SceneActiveMap>,
    mut plane_state: ResMut<ScenePlaneState>,
    existing_chunks_q: Query<Entity, With<draw_land_chunk_mesh::TCMesh>>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
) {
    log_system_add_onenter::<ScenePlugin>(AppState::SetupScene, fname!());

    // Always clear out anything previously spawned!
    for entity in existing_chunks_q.iter() {
        commands.entity(entity).despawn();
    }

    let window = windows.single().unwrap();
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // Player start position (centered focus)
    let player_pos = scene_startup_data_res
        .player_start_pos
        .to_bevy_vec3_ignore_map();

    // Compute set of visible chunks at this config:
    let visible_chunks = compute_visible_chunks(
        player_pos,
        window.physical_width() as f32,
        window.physical_height() as f32,
        zoom,
        scene_active_map.width,
        scene_active_map.height,
        UO_TILE_PIXEL_SIZE,
        TILE_NUM_PER_CHUNK_1D,
        2, // padding tiles (tune as desired)
    );

    for &(gx, gy) in visible_chunks.iter() {
        commands.spawn((
            draw_land_chunk_mesh::TCMesh {
                parent_map: scene_active_map.id,
                gx,
                gy,
            },
            Transform::default(),
            GlobalTransform::default(),
        ));
        log_chunk_spawn(gx, gy, scene_active_map.id);
    }

    plane_state.last_seen_map_id = Some(scene_active_map.id);
}

pub fn sys_update_worldmap_chunks_to_render(
    mut commands: Commands,
    player_q: Query<&Transform, With<Player>>,
    scene_active_map: Res<SceneActiveMap>,
    existing_chunks_q: Query<(Entity, &draw_land_chunk_mesh::TCMesh)>,
    windows: Query<&Window>,
    render_zoom: Res<RenderZoom>,
    mut plane_state: ResMut<ScenePlaneState>,
) {
    let player_pos = if let Ok(p) = player_q.single() {
        p.translation
    } else {
        return;
    };
    let new_map_id = scene_active_map.id;
    let map_switch = plane_state.last_seen_map_id != Some(new_map_id);

    let window = windows.single().unwrap();
    let zoom = render_zoom.0.clamp(MIN_ZOOM, MAX_ZOOM);

    // Compute correct visible chunk set
    let required_chunks = compute_visible_chunks(
        player_pos,
        window.physical_width() as f32,
        window.physical_height() as f32,
        zoom,
        scene_active_map.width,
        scene_active_map.height,
        UO_TILE_PIXEL_SIZE,
        TILE_NUM_PER_CHUNK_1D,
        2, // padding tiles (tune as needed, or make configurable)
    );

    // If map plane changes, brute-force despawn all and respawn
    if map_switch {
        for (entity, tcmesh) in existing_chunks_q.iter() {
            commands.entity(entity).despawn();
            log_chunk_despawn(tcmesh.gx, tcmesh.gy, scene_active_map.id);
        }
        for &(gx, gy) in required_chunks.iter() {
            commands.spawn((
                draw_land_chunk_mesh::TCMesh {
                    parent_map: new_map_id,
                    gx,
                    gy,
                },
                Transform::default(),
                GlobalTransform::default(),
            ));
            log_chunk_spawn(gx, gy, scene_active_map.id);
        }
        plane_state.last_seen_map_id = Some(new_map_id);
        return;
    }

    // Otherwise, incrementally update as before
    let mut currently_spawned = std::collections::HashSet::with_capacity(required_chunks.len());
    for (entity, tcm) in existing_chunks_q.iter() {
        let coords = (tcm.gx, tcm.gy);
        if required_chunks.contains(&coords) {
            currently_spawned.insert(coords);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for coords in required_chunks.difference(&currently_spawned) {
        let (gx, gy) = *coords;
        commands.spawn((
            draw_land_chunk_mesh::TCMesh {
                parent_map: new_map_id,
                gx,
                gy,
            },
            Transform::default(),
            GlobalTransform::default(),
        ));
    }
}
