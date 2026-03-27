pub mod camera;
pub mod dynamic_light;
pub mod player;
pub mod world;

use crate::core::maps::MapPlaneMetadata;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::prelude::*;
use bevy::window::{Window, WindowResized};
use camera::{PlayerCamera, RenderZoom, MAX_ZOOM, MIN_ZOOM};
use player::Player;
use world::land::draw_mesh::{scale_from_zoom, ChunkScale};
use world::land::TILE_NUM_PER_CHUNK_DIM;
use world::{land, WorldGeoData};

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
        .configure_sets(
            Update,
            (
                SceneRenderLandSysSet::SyncLandChunks
                    .after(SceneRenderLandSysSet::ListenSyncRequests),
                SceneRenderLandSysSet::RenderLandChunks
                    .after(SceneRenderLandSysSet::SyncLandChunks),
            ),
        )
        .add_systems(
            Startup,
            sys_setup_scene.in_set(StartupSysSet::SetupSceneStage2),
        )
        .add_systems(
            FixedUpdate,
            sys_update_scene_on_window_resize
                .in_set(SceneRenderLandSysSet::ListenSyncRequests)
                .run_if(in_state(AppState::InGame)),
        )
        .add_systems(
            Update,
            (sys_update_worldmap_chunks_to_render.in_set(SceneRenderLandSysSet::SyncLandChunks),)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

pub fn sys_setup_scene(mut writer: MessageWriter<RecomputeVisibleChunksEvent>) {
    /*
        // Always clear out anything previously spawned!
        for (entity, _) in existing_chunks_q.iter() {
            commands.entity(entity).despawn();
        }
    */
    writer.write(RecomputeVisibleChunksEvent {});
}

pub fn sys_update_scene_on_window_resize(
    mut resize_events: MessageReader<WindowResized>,
    mut writer: MessageWriter<RecomputeVisibleChunksEvent>,
) {
    let mut saw_resize = false;
    for _ in resize_events.read() {
        saw_resize = true;
    }

    if saw_resize {
        writer.write(RecomputeVisibleChunksEvent {});
    }
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

/// Calculates the visible chunk set from current-frame parameters only.
///
/// Computes the orthographic camera footprint on the Y=0 ground plane
/// using the known isometric camera geometry (fixed offset `(5,5,5)`),
/// current zoom level, and window dimensions.  This avoids using
/// `Camera::viewport_to_world()` which relies on Bevy's internal
/// view-projection matrices that are 1 frame stale during `Update`.
///
/// When `chunk_scale > 1`, the grid is iterated at a coarser granularity:
///   scale 2 → 16×16 tile super-chunks (4× fewer entities)
///   scale 4 → 32×32 tile super-chunks (16× fewer entities)
/// Returned coordinates are in the base 8×8 grid, aligned to `chunk_scale` boundaries.
fn compute_visible_chunks(
    player_translation: Vec3,
    zoom: f32,
    window_width: f32,
    window_height: f32,
    map_width: u32,
    map_height: u32,
    chunk_scale: u32,
) -> Vec<(u32, u32)> {
    let base_chunk_size = TILE_NUM_PER_CHUNK_DIM;
    let scaled_tile_span = base_chunk_size * chunk_scale;
    let map_base_chunks_x = (map_width / base_chunk_size) as i32;
    let map_base_chunks_y = (map_height / base_chunk_size) as i32;

    // Compute orthographic half-extents in world/tile units, matching
    // sys_update_camera_projection_to_view exactly.
    let ortho_width = window_width / camera::ORTHO_SIZE_FACTOR;
    let ortho_height =
        (window_height / camera::ORTHO_WIDTH_SCALE_FACTOR) / camera::ORTHO_SIZE_FACTOR;
    let hw = ortho_width * zoom / 2.0;
    let hh = ortho_height * zoom / 2.0;

    // Camera basis vectors for the fixed isometric angle
    // (Transform::from_translation(player + (5,5,5)).looking_at(player, Vec3::Y)):
    //   right = (-1/√2,  0,     1/√2)      → only XZ component
    //   up    = ( 1/√6, -2/√6,  1/√6)
    //   fwd   = (-1/√3, -1/√3, -1/√3)
    //
    // For an ortho frustum corner at camera-local (u, v):
    //   P = cam_pos + u*right + v*up
    //   Ray along fwd hits Y=0 at:  hit = P - P.y * (1, 1, 1)
    //     ⟹ hit.x = P.x - P.y = (cam.x - cam.y) + u*(-1/√2) + v*(3/√6)
    //     ⟹ hit.z = P.z - P.y = (cam.z - cam.y) + u*( 1/√2) + v*(3/√6)
    //
    // AABB half-extent = hw/√2 + hh * 3/√6
    let inv_sqrt2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let three_over_sqrt6: f32 = 3.0 / 6.0_f32.sqrt();

    let cam_pos = player_translation + camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER;
    let center_x = cam_pos.x - cam_pos.y;
    let center_z = cam_pos.z - cam_pos.y;

    let half_extent = hw * inv_sqrt2 + hh * three_over_sqrt6;

    // Safety pad: at least 2 base chunks, with a small extra cushion at
    // wider scales to avoid edge gaps.
    let scaled_pad = if chunk_scale <= 2 {
        2
    } else {
        chunk_scale + (chunk_scale / 4).max(1)
    };
    let edge_pad_tiles = (TILE_NUM_PER_CHUNK_DIM * scaled_pad) as f32;

    let tile_x0 = (center_x - half_extent - edge_pad_tiles).floor() as i32;
    let tile_x1 = (center_x + half_extent + edge_pad_tiles).ceil() as i32;
    let tile_y0 = (center_z - half_extent - edge_pad_tiles).floor() as i32;
    let tile_y1 = (center_z + half_extent + edge_pad_tiles).ceil() as i32;

    // Convert tile AABB to scaled chunk grid.
    // At scale > 1, iterate at coarser granularity (16- or 32-tile steps).
    let s = scaled_tile_span as i32;
    let chunk_x0 = (tile_x0 as f32 / s as f32).floor() as i32;
    let chunk_x1 = (tile_x1 as f32 / s as f32).ceil() as i32;
    let chunk_y0 = (tile_y0 as f32 / s as f32).floor() as i32;
    let chunk_y1 = (tile_y1 as f32 / s as f32).ceil() as i32;

    let mut chunks =
        Vec::with_capacity(((chunk_x1 - chunk_x0) * (chunk_y1 - chunk_y0)).max(0) as usize);
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
                chunks.push((base_gx, base_gy));
            }
        }
    }
    // Ensure the results are sorted for binary search later in sys_update_worldmap_chunks_to_render.
    chunks.sort_unstable();
    chunks
}

/// Bundled local state for `sys_update_worldmap_chunks_to_render` to stay within
/// Bevy's 16-parameter system limit.
#[derive(Default)]
struct ChunkRenderLocals {
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
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh)>,
    mut locals: Local<ChunkRenderLocals>,
) {
    /// Maximum number of chunk entities spawned per frame to avoid burst stalls.
    const MAX_SPAWNS_PER_FRAME: usize = 512;

    let mut current_chunk_count: i32 = land_chunk_count.0 as i32;

    let (mut player_instance, player_transform) =
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

    let has_recompute_event = event.read().next().is_some();
    // Detect a large position jump (teleport without an explicit event, e.g. cursor-click
    // teleport that forgot to send the event, or any future path).  Any move of more than
    // 256 tiles in one frame is unambiguously a teleport, not normal walking.
    // NOTE: Must read prev_rendered_pos BEFORE overwriting it below.
    let large_jump = if let Some(prev) = player_instance.prev_rendered_pos {
        let dx = (player_pos.x as i32 - prev.x as i32).abs();
        let dy = (player_pos.y as i32 - prev.y as i32).abs();
        dx > 256 || dy > 256
    } else {
        false
    };
    let needs_recompute = has_recompute_event || map_switch || large_jump;

    // TODO: move the rendered player position to another system, when we'll render more stuff (not only the land chunks).
    player_instance.prev_rendered_pos = Some(player_pos);

    // Even if no recompute is needed, drain pending spawns from previous frames.
    if !needs_recompute && locals.pending_spawns.is_empty() {
        return;
    }

    // If a recompute is needed, rebuild the required set and recompute the pending queue.
    if needs_recompute {
        let window: &Window = windows_q.single().unwrap();
        let zoom: f32 = render_zoom_res.0.clamp(MIN_ZOOM, MAX_ZOOM);
        let cam_pos = player_transform.translation + PlayerCamera::BASE_OFFSET_FROM_PLAYER;
        let current_camera_chunk = (
            (cam_pos.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
            (cam_pos.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
        );

        let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
            .maps
            .get(&new_map_id)
            .unwrap_or_else(|| panic!("Requested metadata for uncached map {new_map_id}"));

        // Determine chunk scale from current zoom.
        let chunk_scale = scale_from_zoom(zoom);
        let scale_changed = chunk_scale != chunk_scale_res.0;
        chunk_scale_res.0 = chunk_scale;

        // Compute exact visible chunk set at the current scale granularity.
        let required_chunks: Vec<(u32, u32)> = compute_visible_chunks(
            player_transform.translation,
            zoom,
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
            &format!(
                "Visible chunk target: {} (scale={})",
                required_chunks.len(),
                chunk_scale
            ),
        );

        // If map plane, chunk scale, or a large position jump (teleport), brute-force
        // despawn all and respawn so no stale out-of-view entities remain on screen.
        if map_switch || scale_changed || large_jump {
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
            if large_jump {
                console_logger::one(
                    None,
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    "Large position jump detected (teleport): despawn all and respawn at new position.",
                );
            }

            let mut despawned_count = 0i32;
            for (entity, tcm) in existing_chunks_q.iter() {
                commands.entity(entity).despawn();
                log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
                despawned_count += 1;
            }
            current_chunk_count = current_chunk_count.saturating_sub(despawned_count);
            // All chunks go into the pending queue, sorted center-out.
            locals.pending_spawns.clear();
            locals.pending_spawns.extend(required_chunks.iter());
            sort_visible_chunks(
                &mut locals.pending_spawns,
                &required_chunks,
                current_camera_chunk,
            );
            scene_state_data_res.map_id = new_map_id;
        } else {
            // Incremental update: despawn chunks no longer needed, queue new ones.
            // Using a packed u64 representation for faster sorting/searching.
            let mut currently_spawned = Vec::with_capacity(required_chunks.len());
            let mut despawned_count = 0i32;

            for (entity, tcm) in existing_chunks_q.iter() {
                let coords: (u32, u32) = (tcm.gx, tcm.gy);
                // Perform binary search in the sorted required_chunks Vec.
                if required_chunks.binary_search(&coords).is_ok() {
                    currently_spawned.push(coords);
                } else {
                    commands.entity(entity).despawn();
                    log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
                    despawned_count += 1;
                }
            }
            currently_spawned.sort_unstable(); // Ensure it's sorted for subsequent lookup.
            current_chunk_count = current_chunk_count.saturating_sub(despawned_count);

            // Build sorted pending spawn list: only chunks not yet spawned.
            locals.pending_spawns.clear();
            for &coords in &required_chunks {
                if currently_spawned.binary_search(&coords).is_err() {
                    locals.pending_spawns.push(coords);
                }
            }
            sort_visible_chunks(
                &mut locals.pending_spawns,
                &required_chunks,
                current_camera_chunk,
            );
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
    current_chunk_count += batch_size as i32;
    locals.pending_spawns.drain(..batch_size);

    // Update chunk count incrementally; this avoids a full ECS scan every frame.
    land_chunk_count.0 = current_chunk_count.max(0) as u32;
}

/// Sort chunk coordinates so the visible corner tiles are spawned first,
/// then the rest of the border, then the interior.
///
/// This keeps the four screen corners from staying empty during large
/// zoom-outs without increasing the per-frame spawn budget.
fn sort_visible_chunks(
    chunks: &mut [(u32, u32)],
    required_chunks: &[(u32, u32)],
    camera_chunk: (i32, i32),
) {
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;
    for &(gx, gy) in required_chunks.iter() {
        min_x = min_x.min(gx);
        max_x = max_x.max(gx);
        min_y = min_y.min(gy);
        max_y = max_y.max(gy);
    }

    let cx = camera_chunk.0;
    let cy = camera_chunk.1;
    chunks.sort_unstable_by_key(|&(gx, gy)| {
        let dx = gx as i32 - cx;
        let dy = gy as i32 - cy;
        let corner_distance = [
            gx.abs_diff(min_x).max(gy.abs_diff(min_y)),
            gx.abs_diff(max_x).max(gy.abs_diff(min_y)),
            gx.abs_diff(min_x).max(gy.abs_diff(max_y)),
            gx.abs_diff(max_x).max(gy.abs_diff(max_y)),
        ]
        .into_iter()
        .min()
        .unwrap_or(0);

        // Spawn the exact visible corners first, then the rest of the
        // border ring, and only then the interior.  Tie-break by camera
        // distance so the center still converges quickly.
        (corner_distance, dx.abs().max(dy.abs()), dx.abs() + dy.abs())
    });
}
