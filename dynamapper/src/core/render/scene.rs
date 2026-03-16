pub mod camera;
pub mod dynamic_light;
pub mod player;
pub mod world;

use std::collections::HashSet;

use crate::core::maps::MapPlaneMetadata;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::window::{Window, WindowResized};
use camera::{MAX_ZOOM, MIN_ZOOM, RenderZoom, UO_TILE_PIXEL_SIZE};
use player::Player;
use world::land::TILE_NUM_PER_CHUNK_DIM;
use world::{WorldGeoData, land};

#[derive(Resource)]
pub struct SceneStateData {
    pub map_id: u32,
}

#[derive(Message, Debug, Clone, PartialEq)]
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
        .add_message::<RecomputeVisibleChunksEvent>()
        .configure_sets(Update, (SceneRenderLandSysSet::SyncLandChunks.after(SceneRenderLandSysSet::ListenSyncRequests),
    SceneRenderLandSysSet::RenderLandChunks.after(SceneRenderLandSysSet::SyncLandChunks)))
        .add_systems(
            Startup,
            sys_setup_scene.in_set(StartupSysSet::SetupSceneStage2),
        )

        .add_systems(
            Update,
            (sys_update_worldmap_chunks_to_render
                .in_set(SceneRenderLandSysSet::SyncLandChunks),)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

pub fn sys_setup_scene(
    mut writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
/*
    // Always clear out anything previously spawned!
    for (entity, _) in existing_chunks_q.iter() {
        commands.entity(entity).despawn();
    }
*/
    writer.write(RecomputeVisibleChunksEvent{});
}

pub fn sys_update_scene_on_window_resize(mut resize_events: MessageReader<WindowResized>, mut writer: MessageWriter<RecomputeVisibleChunksEvent>) {
    let _event = resize_events.read().last().unwrap();
    writer.write(RecomputeVisibleChunksEvent{});
}

fn log_chunk_spawn(gx: u32, gy: u32, map: u32) {
    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("Spawned chunk at: \t\tgx={gx}\tgy={gy}\t(map={map})"),
    );
}

fn log_chunk_despawn(gx: u32, gy: u32, map: u32) {
    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("De-spawned chunk at: \t\tgx={gx}\tgy={gy}\t(map={map})"),
    );
}

/// Calculates the set of visible chunk coordinates around the player,
/// sized so that the window is covered, even after padding, based on window size and zoom.
fn compute_visible_chunks(
    player_pos: Vec3,
    window_width: f32,
    window_height: f32,
    zoom: f32,
    overscan: f32,
    map_width: u32,
    map_height: u32,
) -> std::collections::HashSet<(u32, u32)> {
    // In orthographic projection, higher scale = zoomed out = each world unit takes
    // fewer screen pixels. So pixel_size_per_tile shrinks as zoom grows.
    let corrected_pixel_size = UO_TILE_PIXEL_SIZE / zoom;

    // Visible tile region (rounded up). Overscan is user-controlled and applied
    // uniformly to all chunks (no center/distance prioritization).
    let margin_factor = overscan.clamp(1.0, 2.5);
    let visible_tiles_x = ((window_width / corrected_pixel_size).ceil() * margin_factor) as i32;
    let visible_tiles_y = ((window_height / corrected_pixel_size).ceil() * margin_factor) as i32;

    // Convert player's position to TILE coordinates
    let player_tile_x = player_pos.x as i32;
    let player_tile_y = player_pos.z as i32;

    // Compute chunk region symmetrically around the player
    let tile_x0 = player_tile_x - visible_tiles_x;
    let tile_x1 = player_tile_x + visible_tiles_x;
    let tile_y0 = player_tile_y - visible_tiles_y;
    let tile_y1 = player_tile_y + visible_tiles_y;

    // Now convert these to chunk indices (and always round DOWN for min, UP for max)
    // so that *any partially overlapping chunk is included*.
    let chunk_size = TILE_NUM_PER_CHUNK_DIM;
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
    mut event: MessageReader<RecomputeVisibleChunksEvent>,
    mut commands: Commands,
    world_geo_data_res: Res<WorldGeoData>,
    render_zoom_res: Res<RenderZoom>,
    settings: Res<Settings>,
    mut scene_state_data_res: ResMut<SceneStateData>,
    windows_q: Query<&Window>,
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh)>,
    mut last_player_chunk: Local<Option<(i32, i32)>>,
    mut last_zoom: Local<f32>,
    mut last_window_size: Local<Option<(u32, u32)>>,
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

    let current_player_chunk = (
        (player_pos_translation.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
        (player_pos_translation.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
    );
    let current_window_size = (window.width() as u32, window.height() as u32);
    let has_recompute_event = event.read().next().is_some();
    let player_chunk_changed = *last_player_chunk != Some(current_player_chunk);
    let zoom_changed = (zoom - *last_zoom).abs() > 0.02;
    let window_changed = *last_window_size != Some(current_window_size);

    if !has_recompute_event && !map_switch && !player_chunk_changed && !zoom_changed && !window_changed {
        return;
    }

    *last_player_chunk = Some(current_player_chunk);
    *last_zoom = zoom;
    *last_window_size = Some(current_window_size);

    //let current_map_id = scene_state_data_res.map_id;
    let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
        .maps
        .get(&new_map_id)
        .unwrap_or_else(|| panic!("Requested metadata for uncached map {new_map_id}"));

    // Compute correct visible chunk set
    let required_chunks: HashSet<(u32, u32)> = compute_visible_chunks(
        player_pos_translation,
        window.width(),
        window.height(),
        zoom,
        settings.app.performance.chunk_visibility_overscan,
        new_map_plane_metadata.width,
        new_map_plane_metadata.height,
    );

    // If map plane changes, brute-force despawn all and respawn
    if map_switch {
        console_logger::one(
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
            let chunk_origin_tile_units_x = gx * land::TILE_NUM_PER_CHUNK_DIM;
            let chunk_origin_tile_units_z = gy * land::TILE_NUM_PER_CHUNK_DIM;
            commands.spawn((
                land::LCMesh {
                    parent_map_id: new_map_id,
                    gx,
                    gy,
                },
                Transform::from_xyz(
                    chunk_origin_tile_units_x as f32,
                    0.0,
                    chunk_origin_tile_units_z as f32,
                ),
                GlobalTransform::default(),
                Visibility::default(),
                InheritedVisibility::default(),
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
        let chunk_origin_tile_units_x = gx * land::TILE_NUM_PER_CHUNK_DIM;
        let chunk_origin_tile_units_z = gy * land::TILE_NUM_PER_CHUNK_DIM;
        commands.spawn((
            land::LCMesh {
                parent_map_id: new_map_id,
                gx,
                gy,
            },
            Transform::from_xyz(
                chunk_origin_tile_units_x as f32,
                0.0,
                chunk_origin_tile_units_z as f32,
            ),
            GlobalTransform::default(),
            Visibility::default(),
            InheritedVisibility::default(),
        ));
        log_chunk_spawn(gx, gy, new_map_id);
    }
}
