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
use camera::{MAX_ZOOM, MIN_ZOOM, RenderZoom};
use player::Player;
use world::land::TILE_NUM_PER_CHUNK_DIM;
use world::land::draw_mesh::{ChunkScale, scale_from_zoom};
use world::{WorldGeoData, land};

#[derive(Resource)]
pub struct SceneStateData {
    pub map_id: u32,
}

/// Cached count of spawned land chunk entities, updated when chunks are spawned/despawned.
/// Used by the performance overlay to avoid a full ECS query scan every frame.
#[derive(Resource, Default)]
pub struct LandChunkCount(pub u32);

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
        .init_resource::<LandChunkCount>()
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

/// Calculates the visible chunk set using Bevy's own Camera projection API.
///
/// Uses `Camera::viewport_to_world()` to cast rays from the 4 screen corners plus
/// mid-edges, intersects them with the Y=0 ground plane, then derives an XZ AABB.
/// This is exact — no manual ortho math that can drift from Bevy's actual rendering.
///
/// When `chunk_scale > 1`, the grid is iterated at a coarser granularity:
///   scale 2 → 16×16 tile super-chunks (4× fewer entities)
///   scale 4 → 32×32 tile super-chunks (16× fewer entities)
/// Returned coordinates are in the base 8×8 grid, aligned to `chunk_scale` boundaries.
fn compute_visible_chunks(
    camera: &Camera,
    camera_global_transform: &GlobalTransform,
    window_width: f32,
    window_height: f32,
    map_width: u32,
    map_height: u32,
    chunk_scale: u32,
) -> std::collections::HashSet<(u32, u32)> {
    let base_chunk_size = TILE_NUM_PER_CHUNK_DIM;
    let scaled_tile_span = base_chunk_size * chunk_scale;
    let map_base_chunks_x = (map_width / base_chunk_size) as i32;
    let map_base_chunks_y = (map_height / base_chunk_size) as i32;

    // Sample 8 points: 4 corners + 4 mid-edges (mid-edges catch aspect-ratio distortions).
    let sample_points = [
        Vec2::new(0.0, 0.0),
        Vec2::new(window_width, 0.0),
        Vec2::new(window_width, window_height),
        Vec2::new(0.0, window_height),
        Vec2::new(window_width * 0.5, 0.0),
        Vec2::new(window_width, window_height * 0.5),
        Vec2::new(window_width * 0.5, window_height),
        Vec2::new(0.0, window_height * 0.5),
    ];

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    let mut any_hit = false;

    for &screen_pt in &sample_points {
        if let Ok(ray) = camera.viewport_to_world(camera_global_transform, screen_pt) {
            let dir_y = ray.direction.y;
            if dir_y.abs() > 1e-6 {
                let t = -ray.origin.y / dir_y;
                let hit = ray.origin + *ray.direction * t;
                min_x = min_x.min(hit.x);
                max_x = max_x.max(hit.x);
                min_z = min_z.min(hit.z);
                max_z = max_z.max(hit.z);
                any_hit = true;
            }
        }
    }

    if !any_hit {
        // Fallback: return an empty set if the camera can't see the ground.
        return std::collections::HashSet::new();
    }

    // Safety pad: 2 chunk rings (16 tiles) to account for vertex-displaced
    // mountain peaks (up to ±12.8m) that can peek into the viewport from
    // chunks whose Y=0 base is just outside the computed footprint.
    let edge_pad_tiles = (TILE_NUM_PER_CHUNK_DIM * 2) as f32;
    let tile_x0 = (min_x - edge_pad_tiles).floor() as i32;
    let tile_x1 = (max_x + edge_pad_tiles).ceil() as i32;
    let tile_y0 = (min_z - edge_pad_tiles).floor() as i32;
    let tile_y1 = (max_z + edge_pad_tiles).ceil() as i32;

    // Convert tile AABB to scaled chunk grid.
    // At scale > 1, iterate at coarser granularity (16- or 32-tile steps).
    let s = scaled_tile_span as i32;
    let chunk_x0 = (tile_x0 as f32 / s as f32).floor() as i32;
    let chunk_x1 = (tile_x1 as f32 / s as f32).ceil() as i32;
    let chunk_y0 = (tile_y0 as f32 / s as f32).floor() as i32;
    let chunk_y1 = (tile_y1 as f32 / s as f32).ceil() as i32;

    let mut set = std::collections::HashSet::new();
    for gx in chunk_x0.max(0)..chunk_x1 {
        for gy in chunk_y0.max(0)..chunk_y1 {
            // Coordinates in the base 8×8 grid, aligned to chunk_scale boundaries.
            let base_gx = (gx as u32) * chunk_scale;
            let base_gy = (gy as u32) * chunk_scale;
            // Ensure the ENTIRE super-chunk fits within map bounds.
            // A super-chunk at (base_gx, base_gy) covers base_gx..(base_gx+chunk_scale),
            // so the last base block is (base_gx + chunk_scale - 1).
            if (base_gx + chunk_scale) as i32 <= map_base_chunks_x
                && (base_gy + chunk_scale) as i32 <= map_base_chunks_y
            {
                set.insert((base_gx, base_gy));
            }
        }
    }
    set
}

/// Bundled local state for `sys_update_worldmap_chunks_to_render` to stay within
/// Bevy's 16-parameter system limit.
#[derive(Default)]
struct ChunkRenderLocals {
    last_camera_chunk: Option<(i32, i32)>,
    last_zoom: f32,
    last_window_size: Option<(u32, u32)>,
    pending_resize_recomputes: u8,
    last_chunk_scale: u32,
    /// Pending spawn queue: chunks to spawn, sorted center-out, drained up to MAX_SPAWNS_PER_FRAME.
    pending_spawns: Vec<(u32, u32)>,
}

fn sys_update_worldmap_chunks_to_render(
    mut event: MessageReader<RecomputeVisibleChunksEvent>,
    mut commands: Commands,
    world_geo_data_res: Res<WorldGeoData>,
    render_zoom_res: Res<RenderZoom>,
    mut scene_state_data_res: ResMut<SceneStateData>,
    mut land_chunk_count: ResMut<LandChunkCount>,
    mut chunk_scale_res: ResMut<ChunkScale>,
    windows_q: Query<&Window>,
    camera_q: Query<(&Camera, &GlobalTransform), With<camera::PlayerCamera>>,
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh)>,
    mut locals: Local<ChunkRenderLocals>,
) {
    /// Maximum number of chunk entities spawned per frame to avoid burst stalls.
    const MAX_SPAWNS_PER_FRAME: usize = 512;

    let (mut player_instance, _player_transform) =
        player_q.single_mut().expect("More than 1 players?");
    let player_pos: Option<UOVec4> = player_instance.current_pos;
    if player_pos.is_none() {
        return;
    }
    let player_pos: UOVec4 = player_pos.unwrap();

    let new_map_id: u32 = player_pos.m as u32;
    let map_switch: bool = {
        let old_map_id: Option<UOVec4> = player_instance.prev_rendered_pos;
        old_map_id.is_none() || (new_map_id != old_map_id.unwrap().m as u32)
    };

    // TODO: move the rendered player position to another system, when we'll render more stuff (not only the land chunks).
    player_instance.prev_rendered_pos = Some(player_pos);

    let window: &Window = windows_q.single().unwrap();
    let (camera, camera_global_transform) = camera_q.single().unwrap();
    let zoom: f32 = render_zoom_res.0.clamp(MIN_ZOOM, MAX_ZOOM);

    let cam_translation = camera_global_transform.translation();
    let current_camera_chunk = (
        (cam_translation.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
        (cam_translation.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
    );
    let current_window_size = (window.width() as u32, window.height() as u32);
    let has_recompute_event = event.read().next().is_some();
    let camera_chunk_changed = locals.last_camera_chunk != Some(current_camera_chunk);
    let zoom_changed = (zoom - locals.last_zoom).abs() > 0.02;
    let window_changed = locals.last_window_size != Some(current_window_size);
    if window_changed {
        // Camera projection update may land in a different frame/order.
        // Recompute a couple of frames to avoid stale-projection holes.
        locals.pending_resize_recomputes = 2;
    }
    let has_pending_resize_recompute = locals.pending_resize_recomputes > 0;

    let needs_recompute = has_recompute_event
        || map_switch
        || camera_chunk_changed
        || zoom_changed
        || window_changed
        || has_pending_resize_recompute;

    // Even if no recompute is needed, drain pending spawns from previous frames.
    if !needs_recompute && locals.pending_spawns.is_empty() {
        return;
    }

    // If a recompute is needed, rebuild the required set and recompute the pending queue.
    if needs_recompute {
        locals.last_camera_chunk = Some(current_camera_chunk);
        locals.last_zoom = zoom;
        locals.last_window_size = Some(current_window_size);
        if locals.pending_resize_recomputes > 0 {
            locals.pending_resize_recomputes -= 1;
        }

        let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
            .maps
            .get(&new_map_id)
            .unwrap_or_else(|| panic!("Requested metadata for uncached map {new_map_id}"));

        // Determine chunk scale from current zoom.
        let chunk_scale = scale_from_zoom(zoom);
        let scale_changed = chunk_scale != locals.last_chunk_scale;
        locals.last_chunk_scale = chunk_scale;
        chunk_scale_res.0 = chunk_scale;

        // Compute exact visible chunk set at the current scale granularity.
        let required_chunks: HashSet<(u32, u32)> = compute_visible_chunks(
            camera,
            camera_global_transform,
            window.width(),
            window.height(),
            new_map_plane_metadata.width,
            new_map_plane_metadata.height,
            chunk_scale,
        );
        console_logger::one(
            None,
            LogSev::Debug,
            LogAbout::RenderWorldLand,
            &format!("Visible chunk target: {} (scale={})", required_chunks.len(), chunk_scale),
        );

        // If map plane OR chunk scale changes, brute-force despawn all and respawn.
        // Scale changes alter the grid granularity, so old entities don't match.
        if map_switch || scale_changed {
            if map_switch {
                console_logger::one(
                    None,
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    "Detected Map Plane change: despawn previously rendered land chunks and spawn new ones.",
                );
            }
            if scale_changed {
                console_logger::one(
                    None,
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    &format!("Chunk scale changed to {chunk_scale}: despawn all and respawn at new granularity."),
                );
            }

            for (entity, tcm) in existing_chunks_q.iter() {
                commands.entity(entity).despawn();
                log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
            }
            // All chunks go into the pending queue, sorted center-out.
            locals.pending_spawns.clear();
            locals.pending_spawns.extend(required_chunks.iter());
            sort_center_out(&mut locals.pending_spawns, current_camera_chunk);
            scene_state_data_res.map_id = new_map_id;
        } else {
            // Incremental update: despawn chunks no longer needed, queue new ones.
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

            // Build sorted pending spawn list: only chunks not yet spawned.
            locals.pending_spawns.clear();
            for &coords in required_chunks.difference(&currently_spawned) {
                locals.pending_spawns.push(coords);
            }
            sort_center_out(&mut locals.pending_spawns, current_camera_chunk);
        }
    }

    // Drain up to MAX_SPAWNS_PER_FRAME from the front (closest to camera first).
    let new_map_id = scene_state_data_res.map_id;
    let active_scale = chunk_scale_res.0;
    let batch_size = locals.pending_spawns.len().min(MAX_SPAWNS_PER_FRAME);
    for &(gx, gy) in locals.pending_spawns[..batch_size].iter() {
        let chunk_origin_tile_units_x = gx * land::TILE_NUM_PER_CHUNK_DIM;
        let chunk_origin_tile_units_z = gy * land::TILE_NUM_PER_CHUNK_DIM;
        commands.spawn((
            land::LCMesh {
                parent_map_id: new_map_id,
                gx,
                gy,
                scale: active_scale,
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
    locals.pending_spawns.drain(..batch_size);

    // Update chunk count: existing entities + what we just spawned - what we despawned.
    // Simplest accurate count: count remaining entities.
    land_chunk_count.0 = existing_chunks_q.iter().count() as u32;
}

/// Sort chunk coordinates so that chunks closest to the camera are first.
fn sort_center_out(chunks: &mut [(u32, u32)], camera_chunk: (i32, i32)) {
    let cx = camera_chunk.0;
    let cy = camera_chunk.1;
    chunks.sort_unstable_by_key(|&(gx, gy)| {
        let dx = gx as i32 - cx;
        let dy = gy as i32 - cy;
        // Chebyshev distance: sort by "ring" from camera, then by Manhattan for stability.
        (dx.abs().max(dy.abs()), dx.abs() + dy.abs())
    });
}
