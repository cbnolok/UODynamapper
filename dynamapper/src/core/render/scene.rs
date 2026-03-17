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

/// Calculates the visible chunk set from the camera projection and transform.
///
/// We project the viewport corners onto the ground plane (Y=0), derive an XZ AABB,
/// then convert that area to chunk coordinates. This is much more accurate than
/// player-centered width/height estimates at high zoom-out.
fn compute_visible_chunks(
    camera_transform: &Transform,
    projection: &Projection,
    player_pos_fallback: Vec3,
    window_width: f32,
    window_height: f32,
    zoom: f32,
    overscan: f32,
    map_width: u32,
    map_height: u32,
) -> std::collections::HashSet<(u32, u32)> {
    let chunk_size = TILE_NUM_PER_CHUNK_DIM;
    let map_chunks_x = (map_width / chunk_size) as i32;
    let map_chunks_y = (map_height / chunk_size) as i32;

    let margin_factor = overscan.clamp(1.0, 2.5);

    // Preferred path: camera/projection-based footprint on ground plane.
    if let Projection::Orthographic(ortho) = projection {
        if let bevy::camera::ScalingMode::Fixed { width, height } = ortho.scaling_mode {
            let half_w = width * ortho.scale * 0.5 * margin_factor;
            let half_h = height * ortho.scale * 0.5 * margin_factor;

            let cam_pos = camera_transform.translation;
            let cam_right = camera_transform.rotation * Vec3::X;
            let cam_up = camera_transform.rotation * Vec3::Y;
            let cam_forward = camera_transform.rotation * -Vec3::Z;

            // Avoid division by very small numbers if camera becomes near-parallel to ground.
            if cam_forward.y.abs() > 1e-5 {
                let corners = [(-1.0_f32, -1.0_f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];

                let mut min_x = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut min_z = f32::INFINITY;
                let mut max_z = f32::NEG_INFINITY;

                for (sx, sy) in corners {
                    let ray_origin = cam_pos + cam_right * (sx * half_w) + cam_up * (sy * half_h);
                    let t = (0.0 - ray_origin.y) / cam_forward.y;
                    let hit = ray_origin + cam_forward * t;

                    min_x = min_x.min(hit.x);
                    max_x = max_x.max(hit.x);
                    min_z = min_z.min(hit.z);
                    max_z = max_z.max(hit.z);
                }

                // Conservative 1-tile safety pad avoids precision-edge misses
                // at extreme zoom/window sizes.
                let edge_pad_tiles = 1.0_f32;
                let tile_x0 = (min_x - edge_pad_tiles).floor() as i32;
                let tile_x1 = (max_x + edge_pad_tiles).ceil() as i32;
                let tile_y0 = (min_z - edge_pad_tiles).floor() as i32;
                let tile_y1 = (max_z + edge_pad_tiles).ceil() as i32;

                let chunk_x0 = (tile_x0.div_euclid(chunk_size as i32)).max(0);
                let chunk_x1 = ((tile_x1 as f32) / chunk_size as f32).ceil() as i32;
                let chunk_y0 = (tile_y0.div_euclid(chunk_size as i32)).max(0);
                let chunk_y1 = ((tile_y1 as f32) / chunk_size as f32).ceil() as i32;

                let mut set = std::collections::HashSet::new();
                for gx in chunk_x0..=chunk_x1.min(map_chunks_x - 1) {
                    for gy in chunk_y0..=chunk_y1.min(map_chunks_y - 1) {
                        set.insert((gx as u32, gy as u32));
                    }
                }
                return set;
            }
        }
    }

    // Fallback path (should be rare): player-centered approximation.
    // In orthographic projection, higher scale = zoomed out = each world unit takes
    // fewer screen pixels. So pixel_size_per_tile shrinks as zoom grows.
    let corrected_pixel_size = UO_TILE_PIXEL_SIZE / zoom;
    let half_visible_tiles_x = (((window_width / corrected_pixel_size).ceil() * margin_factor) * 0.5) as i32;
    let half_visible_tiles_y = (((window_height / corrected_pixel_size).ceil() * margin_factor) * 0.5) as i32;

    let player_tile_x = player_pos_fallback.x as i32;
    let player_tile_y = player_pos_fallback.z as i32;

    let tile_x0 = player_tile_x - half_visible_tiles_x;
    let tile_x1 = player_tile_x + half_visible_tiles_x;
    let tile_y0 = player_tile_y - half_visible_tiles_y;
    let tile_y1 = player_tile_y + half_visible_tiles_y;

    // Now convert these to chunk indices (and always round DOWN for min, UP for max)
    // so that *any partially overlapping chunk is included*.
    let chunk_x0 = (tile_x0.div_euclid(chunk_size as i32)).max(0);
    let chunk_x1 = ((tile_x1 as f32) / chunk_size as f32).ceil() as i32;
    let chunk_y0 = (tile_y0.div_euclid(chunk_size as i32)).max(0);
    let chunk_y1 = ((tile_y1 as f32) / chunk_size as f32).ceil() as i32;

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
    camera_q: Query<(&Transform, &Projection), With<camera::PlayerCamera>>,
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh)>,
    mut last_camera_chunk: Local<Option<(i32, i32)>>,
    mut last_zoom: Local<f32>,
    mut last_window_size: Local<Option<(u32, u32)>>,
    mut pending_resize_recomputes: Local<u8>,
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
    let (camera_transform, camera_projection) = camera_q.single().unwrap();
    let zoom: f32 = render_zoom_res.0.clamp(MIN_ZOOM, MAX_ZOOM);

    let current_camera_chunk = (
        (camera_transform.translation.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
        (camera_transform.translation.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
    );
    let current_window_size = (window.width() as u32, window.height() as u32);
    let has_recompute_event = event.read().next().is_some();
    let camera_chunk_changed = *last_camera_chunk != Some(current_camera_chunk);
    let zoom_changed = (zoom - *last_zoom).abs() > 0.02;
    let window_changed = *last_window_size != Some(current_window_size);
    if window_changed {
        // Camera projection update may land in a different frame/order.
        // Recompute a couple of frames to avoid stale-projection holes.
        *pending_resize_recomputes = 2;
    }
    let has_pending_resize_recompute = *pending_resize_recomputes > 0;

    if !has_recompute_event
        && !map_switch
        && !camera_chunk_changed
        && !zoom_changed
        && !window_changed
        && !has_pending_resize_recompute
    {
        return;
    }

    *last_camera_chunk = Some(current_camera_chunk);
    *last_zoom = zoom;
    *last_window_size = Some(current_window_size);
    if *pending_resize_recomputes > 0 {
        *pending_resize_recomputes -= 1;
    }

    //let current_map_id = scene_state_data_res.map_id;
    let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
        .maps
        .get(&new_map_id)
        .unwrap_or_else(|| panic!("Requested metadata for uncached map {new_map_id}"));

    // Compute correct visible chunk set
    let required_chunks: HashSet<(u32, u32)> = compute_visible_chunks(
        camera_transform,
        camera_projection,
        player_pos_translation,
        window.width(),
        window.height(),
        zoom,
        settings.app.performance.chunk_visibility_overscan,
        new_map_plane_metadata.width,
        new_map_plane_metadata.height,
    );
    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!("Visible chunk target: {}", required_chunks.len()),
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
