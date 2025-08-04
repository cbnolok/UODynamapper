pub mod camera;
pub mod dynamic_light;
pub mod player;
pub mod world;

use std::collections::HashSet;

use crate::core::maps::MapPlaneMetadata;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::window::{Window, WindowResized};
use camera::{MAX_ZOOM, MIN_ZOOM, RenderZoom, UO_TILE_PIXEL_SIZE};
use player::Player;
use world::land::TILE_NUM_PER_CHUNK_1D;
use world::{WorldGeoData, land};

#[derive(Resource)]
pub struct SceneStateData {
    pub map_id: u32,
}

#[derive(Event, Debug, Clone, PartialEq)]
pub struct RecomputeVisibleChunksEvent;

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
            world::WorldPlugin {
                registered_by: "ScenePlugin",
            },
            dynamic_light::PlayerDynamicLightPlugin {
                registered_by: "ScenePlugin",
            },
            camera::CameraPlugin {
                registered_by: "ScenePlugin",
            },
            player::PlayerPlugin {
                registered_by: "ScenePlugin",
            },
        ))
        .insert_resource(SceneStateData {
            map_id: 0xFFFF, // placeholder
        })
        .add_event::<RecomputeVisibleChunksEvent>()
        .configure_sets(Update, (SceneRenderLandSysSet::SyncLandChunks.after(SceneRenderLandSysSet::ListenSyncRequests),
    SceneRenderLandSysSet::RenderLandChunks.after(SceneRenderLandSysSet::SyncLandChunks)))
        .add_systems(
            Startup,
            sys_setup_scene.in_set(StartupSysSet::SetupSceneStage2),
        )

        .add_systems(
            Update,
            sys_update_worldmap_chunks_to_render
                .in_set(SceneRenderLandSysSet::SyncLandChunks)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

pub fn sys_setup_scene(
    mut writer: EventWriter<RecomputeVisibleChunksEvent>,
) {
/*
    // Always clear out anything previously spawned!
    for (entity, _) in existing_chunks_q.iter() {
        commands.entity(entity).despawn();
    }
*/
    writer.write(RecomputeVisibleChunksEvent{});
}

pub fn sys_update_scene_on_window_resize(mut resize_events: EventReader<WindowResized>, mut writer: EventWriter<RecomputeVisibleChunksEvent>) {
    let _event = resize_events.read().last().unwrap();
    writer.write(RecomputeVisibleChunksEvent{});
}

fn log_chunk_spawn(gx: u32, gy: u32, map: u32) {
    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("Spawned chunk at: \t\tgx={gx}\tgy={gy}\t(map={map})"),
    );
}

fn log_chunk_despawn(gx: u32, gy: u32, map: u32) {
    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("De-spawned chunk at: \tgx={gx}\tgy={gy}\t(map={map})"),
    );
}

/// Calculates the set of visible chunk coordinates around the player,
/// sized so that the window is covered, even after padding, based on window size and zoom.
fn compute_visible_chunks(
    player_pos: Vec3,
    window_width: f32,
    window_height: f32,
    zoom: f32,
    map_width: u32,
    map_height: u32,
) -> std::collections::HashSet<(u32, u32)> {
    let corrected_pixel_size = UO_TILE_PIXEL_SIZE * zoom;

    // Visible tile region (rounded up)
    let visible_tiles_x = ((window_width / corrected_pixel_size).ceil()) as i32;
    let visible_tiles_y = ((window_height / corrected_pixel_size).ceil()) as i32;

    // Convert player's position to TILE coordinates
    let player_tile_x = player_pos.x as i32;
    let player_tile_y = player_pos.z as i32;

    // Compute chunk region to fully cover the visible area, including all overlapping
    // Start/end in TILES (not chunks yet)

    let tile_x0 = player_tile_x - visible_tiles_x;
    let tile_x1 = player_tile_x + (visible_tiles_x / 2);
    let tile_y0 = player_tile_y - (visible_tiles_y * 3 / 2);
    let tile_y1 = player_tile_y + (visible_tiles_y / 2);

    // Now convert these to chunk indices (and always round DOWN for min, UP for max)
    // so that *any partially overlapping chunk is included*.
    let chunk_size = TILE_NUM_PER_CHUNK_1D;
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

fn sys_update_worldmap_chunks_to_render(
    mut _event: EventReader<RecomputeVisibleChunksEvent>,
    mut commands: Commands,
    world_geo_data_res: Res<WorldGeoData>,
    render_zoom_res: Res<RenderZoom>,
    mut scene_state_data_res: ResMut<SceneStateData>,
    windows_q: Query<&Window>,
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh)>,
) {
    let (mut player_instance, player_transform) =
        player_q.single_mut().expect("More than 1 players?");
    let player_pos: Option<UOVec4> = player_instance.current_pos;
    if player_pos.is_none() {
        return;
    }
    let player_pos: UOVec4 = player_pos.unwrap();
    let player_pos_translation: Vec3 = player_transform.translation;

    let new_map_id: u32 = player_pos.m as u32;
    let map_switch: bool = {
        let old_map_id: Option<UOVec4> = player_instance.prev_rendered_pos;
        old_map_id.is_none() || (new_map_id != old_map_id.unwrap().m as u32)
    };

    // TODO: move the rendered player position to another system, when we'll render more stuff (not only the land chunks).
    player_instance.prev_rendered_pos = Some(player_pos);

    let window: &Window = windows_q.single().unwrap();
    let zoom: f32 = render_zoom_res.0.clamp(MIN_ZOOM, MAX_ZOOM);
    //let current_map_id = scene_state_data_res.map_id;
    let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
        .maps
        .get(&new_map_id)
        .expect(&format!("Requested metadata for uncached map {new_map_id}"));

    // Compute correct visible chunk set
    let required_chunks: HashSet<(u32, u32)> = compute_visible_chunks(
        player_pos_translation,
        window.physical_width() as f32,
        window.physical_height() as f32,
        zoom,
        new_map_plane_metadata.width,
        new_map_plane_metadata.height,
    );

    // If map plane changes, brute-force despawn all and respawn
    if map_switch {
        logger::one(
            None,
            LogSev::Info,
            LogAbout::RenderWorldLand,
            "Detected Map Plane change: despawn previously rendered land chunks and spawn new ones.",
        );

        for (entity, tcm) in existing_chunks_q.iter() {
            commands.entity(entity).despawn();
            log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
        }
        for &(gx, gy) in required_chunks.iter() {
            commands.spawn((
                land::LCMesh {
                    parent_map_id: new_map_id,
                    gx,
                    gy,
                },
                Transform::default(),
                GlobalTransform::default(),
            ));
            log_chunk_spawn(gx, gy, new_map_id);
        }
        scene_state_data_res.map_id = new_map_id;
        return;
    }

    // Otherwise, incrementally update as before
    let mut currently_spawned = HashSet::with_capacity(required_chunks.len());
    for (entity, tcm) in existing_chunks_q.iter() {
        let coords: (u32, u32) = (tcm.gx, tcm.gy);
        if required_chunks.contains(&coords) {
            currently_spawned.insert(coords);
        } else {
            commands.entity(entity).despawn();
            log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
        }
    }
    for coords in required_chunks.difference(&currently_spawned) {
        let (gx, gy) = *coords;
        commands.spawn((
            land::LCMesh {
                parent_map_id: new_map_id,
                gx,
                gy,
            },
            Transform::default(),
            GlobalTransform::default(),
        ));
        log_chunk_spawn(gx, gy, new_map_id);
    }
}
