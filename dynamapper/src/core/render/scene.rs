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
    mut settings: ResMut<Settings>,
) {
    let mut saw_resize = false;
    let mut last_size = (0.0, 0.0);
    for event in resize_events.read() {
        saw_resize = true;
        last_size = (event.width, event.height);
    }

    if saw_resize {
        settings.app.window.width = last_size.0;
        settings.app.window.height = last_size.1;
        writer.write(RecomputeVisibleChunksEvent {});
    }
}

fn log_chunk_spawn(gx: u32, gy: u32, map: u32) {
    console_logger::one(
        LogSev::DebugVerbose,
        LogAbout::RenderWorldLand,
        &format!("Spawned chunk at: \t\tgx={gx}\tgy={gy}\t(map={map})"),
    );
}

fn log_chunk_despawn(gx: u32, gy: u32, map: u32) {
    console_logger::one(
        LogSev::DebugVerbose,
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
/// When `chunk_scale > 1`, the grid is iterated at a coarser granularity.
/// Returned coordinates are in the base logical 32×32 grid, aligned to `chunk_scale` boundaries.
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
    let chunk_x0 = tile_x0.div_euclid(s);
    let chunk_x1 = tile_x1.div_euclid(s) + 1;
    let chunk_y0 = tile_y0.div_euclid(s);
    let chunk_y1 = tile_y1.div_euclid(s) + 1;

    let mut chunks =
        Vec::with_capacity(((chunk_x1 - chunk_x0) * (chunk_y1 - chunk_y0)).max(0) as usize);
    for gx in chunk_x0.max(0)..chunk_x1 {
        for gy in chunk_y0.max(0)..chunk_y1 {
            // Coordinates in the base logical 32×32 grid, aligned to chunk_scale boundaries.
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
    pending_spawns: Vec<PendingChunkSpawn>,
    /// Pending despawn queue used to retire obsolete scales without a single large ECS burst.
    pending_despawns: Vec<Entity>,
    /// Scale currently guaranteed to cover the screen.
    committed_scale: Option<u32>,
    /// Target scale being introduced in parallel with the committed scale.
    transition_target_scale: Option<u32>,
    /// Required chunk coordinates for the target scale while a handoff is active.
    transition_required_chunks: Vec<(u32, u32)>,
    /// Last visible-chunk target we actually logged.
    ///
    /// The visible target can be recomputed many times in a row from resize,
    /// zoom, teleport, or follow-camera updates even when the final chunk set is
    /// identical.  Logging every recompute is noisy and was spamming the console
    /// with repeated "Visible chunk target: N" lines for the exact same target.
    last_logged_visible_target: Vec<(u32, u32)>,
    last_logged_visible_target_map_id: Option<u32>,
    last_logged_visible_target_scale: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingChunkSpawn {
    gx: u32,
    gy: u32,
    scale: u32,
}

fn queue_entity_for_despawn(queue: &mut Vec<Entity>, entity: Entity) {
    if !queue.contains(&entity) {
        queue.push(entity);
    }
}

/// Returns true when a committed-scale chunk is already fully covered by ready target chunks.
///
/// This lets the handoff retire old chunks incrementally instead of keeping the whole
/// previous scale alive until the entire target scale finishes streaming.
fn chunk_is_covered_by_ready_target(
    committed_gx: u32,
    committed_gy: u32,
    committed_scale: u32,
    target_scale: u32,
    ready_target_chunks: &[(u32, u32)],
) -> bool {
    if ready_target_chunks.is_empty() {
        return false;
    }

    if target_scale == committed_scale {
        return ready_target_chunks
            .binary_search(&(committed_gx, committed_gy))
            .is_ok();
    }

    if target_scale > committed_scale {
        let covering_target_gx = (committed_gx / target_scale) * target_scale;
        let covering_target_gy = (committed_gy / target_scale) * target_scale;
        return ready_target_chunks
            .binary_search(&(covering_target_gx, covering_target_gy))
            .is_ok();
    }

    let committed_end_gx = committed_gx + committed_scale;
    let committed_end_gy = committed_gy + committed_scale;

    let mut target_gx = committed_gx;
    while target_gx < committed_end_gx {
        let mut target_gy = committed_gy;
        while target_gy < committed_end_gy {
            if ready_target_chunks.binary_search(&(target_gx, target_gy)).is_err() {
                return false;
            }
            target_gy += target_scale;
        }
        target_gx += target_scale;
    }

    true
}

fn sys_update_worldmap_chunks_to_render(
    mut event: MessageReader<RecomputeVisibleChunksEvent>,
    mut commands: Commands,
    world_geo_data_res: Res<WorldGeoData>,
    render_zoom_res: Res<RenderZoom>,
    mut scene_state_data_res: ResMut<SceneStateData>,
    mut runtime_diagnostics: ResMut<crate::core::diagnostics::WorldmapRuntimeDiagnostics>,
    mut system_diagnostics: ResMut<crate::core::diagnostics::WorldmapSystemDiagnostics>,
    mut land_chunk_count: ResMut<LandChunkCount>,
    mut chunk_scale_res: ResMut<ChunkScale>,
    windows_q: Query<&Window>,
    mut player_q: Query<(&mut Player, &Transform)>,
    existing_chunks_q: Query<(Entity, &land::LCMesh, Has<Mesh3d>)>,
    mut locals: Local<ChunkRenderLocals>,
) {
    /// Maximum number of chunk entities spawned per frame to avoid burst stalls.
    const MAX_SPAWNS_PER_FRAME: usize = 512;
    /// Maximum number of obsolete chunk entities retired per frame.
    const MAX_DESPAWNS_PER_FRAME: usize = 1024;

    let _timer = crate::core::diagnostics::scoped_worldmap_timer(
        &mut system_diagnostics,
        crate::core::diagnostics::WorldmapTimedSystem::ChunkSync,
    );
    let _trace_span = crate::tracy_span!("worldmap::chunk_sync");

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

    // Keep the system alive while a transition is in flight so we can continue spawning,
    // retire obsolete chunks, and detect when the target scale is fully ready.
    if !needs_recompute
        && locals.pending_spawns.is_empty()
        && locals.pending_despawns.is_empty()
        && locals.transition_target_scale.is_none()
    {
        return;
    }

    // If a recompute is needed, rebuild the required set and recompute the pending queue.
    if needs_recompute {
        let window: &Window = windows_q.single().unwrap();
        let zoom: f32 = render_zoom_res.0.clamp(MIN_ZOOM, MAX_ZOOM);
        let cam_pos = player_transform.translation + PlayerCamera::BASE_OFFSET_FROM_PLAYER;
        let current_camera_chunk: (i32, i32) = (
            (cam_pos.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
            (cam_pos.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
        );

        let new_map_plane_metadata: &MapPlaneMetadata = world_geo_data_res
            .maps
            .get(&new_map_id)
            .unwrap_or_else(|| panic!("Requested metadata for uncached map {new_map_id}"));

        // Determine the desired chunk scale from current zoom.
        let desired_scale: u32 = scale_from_zoom(zoom);
        chunk_scale_res.0 = desired_scale;

        let committed_scale: u32 = locals.committed_scale.unwrap_or(desired_scale);
        let scale_changed = desired_scale != committed_scale;

        let viewport_size: (f32, f32) = (window.width(), window.height());

        // Compute the visible chunk set for the desired scale and, if a handoff is active,
        // also maintain the committed scale as a fallback until the target scale is ready.
        let required_desired_chunks: Vec<(u32, u32)> = compute_visible_chunks(
            player_transform.translation,
            zoom,
            viewport_size.0,
            viewport_size.1,
            new_map_plane_metadata.width,
            new_map_plane_metadata.height,
            desired_scale,
        );
        if locals.last_logged_visible_target_map_id != Some(new_map_id)
            || locals.last_logged_visible_target_scale != Some(desired_scale)
            || locals.last_logged_visible_target != required_desired_chunks
        {
            console_logger::one(
                LogSev::Debug, // DebugVerbose
                LogAbout::RenderWorldLand,
                &format!(
                    "Visible chunk target: {} (scale={}, zoom={}, viewport size: x={}, y={})",
                    required_desired_chunks.len(),
                    desired_scale, zoom,
                    viewport_size.0, viewport_size.1
                ),
            );
            locals.last_logged_visible_target_map_id = Some(new_map_id);
            locals.last_logged_visible_target_scale = Some(desired_scale);
            locals.last_logged_visible_target = required_desired_chunks.clone();
        }

        // Map switches and the initial population still use the brute-force path.
        // Teleports now reuse the incremental path so the new location can stream in without
        // forcing a visible full clear first.
        if map_switch || locals.committed_scale.is_none() {
            if map_switch {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    "Detected Map Plane change: despawn previously rendered land chunks and spawn new ones.",
                );
            }

            let mut despawned_count = 0i32;
            for (entity, tcm, _) in existing_chunks_q.iter() {
                commands.entity(entity).despawn();
                log_chunk_despawn(tcm.gx, tcm.gy, new_map_id);
                despawned_count += 1;
            }
            current_chunk_count = current_chunk_count.saturating_sub(despawned_count);
            // Fresh population: only the desired scale is kept.
            locals.pending_spawns.clear();
            locals.pending_despawns.clear();
            locals.transition_target_scale = None;
            locals.transition_required_chunks.clear();
            locals.committed_scale = Some(desired_scale);

            let mut sorted_spawns = required_desired_chunks.clone();
            sort_visible_chunks(&mut sorted_spawns, &required_desired_chunks, current_camera_chunk);
            locals
                .pending_spawns
                .extend(sorted_spawns.into_iter().map(|(gx, gy)| PendingChunkSpawn {
                    gx,
                    gy,
                    scale: desired_scale,
                }));
            scene_state_data_res.map_id = new_map_id;
        } else {
            if large_jump {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    "Large position jump detected (teleport): streaming new chunks in without a full despawn.",
                );
            }

            let mut required_committed_chunks = compute_visible_chunks(
                player_transform.translation,
                zoom,
                window.width(),
                window.height(),
                new_map_plane_metadata.width,
                new_map_plane_metadata.height,
                committed_scale,
            );

            if scale_changed {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    &format!(
                        "Chunk scale handoff started: {} -> {}. Keeping old chunks visible until the new scale is ready.",
                        committed_scale, desired_scale
                    ),
                );
                locals.transition_target_scale = Some(desired_scale);
                locals.transition_required_chunks = required_desired_chunks.clone();
            } else if locals.transition_target_scale.is_some() {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::RenderWorldLand,
                    &format!(
                        "Chunk scale handoff cancelled: staying on committed scale {}.",
                        committed_scale
                    ),
                );
                locals.transition_target_scale = None;
                locals.transition_required_chunks.clear();
            }

            if let Some(target_scale) = locals.transition_target_scale {
                if target_scale != desired_scale {
                    console_logger::one(
                        LogSev::Info,
                        LogAbout::RenderWorldLand,
                        &format!(
                            "Chunk scale handoff retargeted: {} -> {} (committed scale still {}).",
                            target_scale, desired_scale, committed_scale
                        ),
                    );
                    locals.transition_target_scale = Some(desired_scale);
                    locals.transition_required_chunks = required_desired_chunks.clone();
                }
            }

            required_committed_chunks.sort_unstable();
            locals.transition_required_chunks.sort_unstable();

            let active_target_scale = locals.transition_target_scale;
            let required_target_chunks = if active_target_scale.is_some() {
                Some(locals.transition_required_chunks.clone())
            } else {
                None
            };

            let mut currently_spawned_committed = Vec::with_capacity(required_committed_chunks.len());
            let mut currently_spawned_target =
                Vec::with_capacity(required_target_chunks.as_ref().map_or(0, |chunks| chunks.len()));

            for (entity, tcm, _) in existing_chunks_q.iter() {
                if tcm.parent_map_id != new_map_id {
                    queue_entity_for_despawn(&mut locals.pending_despawns, entity);
                    continue;
                }

                let coords = (tcm.gx, tcm.gy);
                let keep = if tcm.scale == committed_scale {
                    let keep = required_committed_chunks.binary_search(&coords).is_ok();
                    if keep {
                        currently_spawned_committed.push(coords);
                    }
                    keep
                } else if Some(tcm.scale) == active_target_scale {
                    let keep = required_target_chunks
                        .as_ref()
                        .is_some_and(|chunks| chunks.binary_search(&coords).is_ok());
                    if keep {
                        currently_spawned_target.push(coords);
                    }
                    keep
                } else {
                    false
                };

                if !keep {
                    queue_entity_for_despawn(&mut locals.pending_despawns, entity);
                }
            }

            currently_spawned_committed.sort_unstable();
            currently_spawned_target.sort_unstable();

            // Rebuild the pending spawn queue for the scales we still care about.
            locals.pending_spawns.clear();

            // During a scale handoff, keep already-spawned committed chunks as the
            // visual fallback, but do not expand the old scale into newly exposed
            // viewport area. Doing so duplicates the expensive streaming work for a
            // scale we are actively replacing and causes large zoom-out regressions.
            if active_target_scale.is_none() {
                let mut sorted_committed_spawns = Vec::new();
                for &coords in &required_committed_chunks {
                    if currently_spawned_committed.binary_search(&coords).is_err() {
                        sorted_committed_spawns.push(coords);
                    }
                }
                sort_visible_chunks(
                    &mut sorted_committed_spawns,
                    &required_committed_chunks,
                    current_camera_chunk,
                );
                locals.pending_spawns.extend(
                    sorted_committed_spawns
                        .into_iter()
                        .map(|(gx, gy)| PendingChunkSpawn {
                            gx,
                            gy,
                            scale: committed_scale,
                        }),
                );
            }

            if let Some(target_scale) = active_target_scale {
                let mut sorted_target_spawns = Vec::new();
                for &coords in locals.transition_required_chunks.iter() {
                    if currently_spawned_target.binary_search(&coords).is_err() {
                        sorted_target_spawns.push(coords);
                    }
                }
                sort_visible_chunks(
                    &mut sorted_target_spawns,
                    &locals.transition_required_chunks,
                    current_camera_chunk,
                );
                locals.pending_spawns.extend(
                    sorted_target_spawns
                        .into_iter()
                        .map(|(gx, gy)| PendingChunkSpawn {
                            gx,
                            gy,
                            scale: target_scale,
                        }),
                );
            }

            scene_state_data_res.map_id = new_map_id;
        }
    }

    // As target chunks become mesh-ready, retire only the old chunks they already cover.
    // This keeps the no-clear handoff while avoiding a large temporary draw-cost spike.
    if let (Some(committed_scale), Some(target_scale)) =
        (locals.committed_scale, locals.transition_target_scale)
    {
        let mut ready_target_chunks = Vec::with_capacity(locals.transition_required_chunks.len());
        for (_, tcm, has_mesh) in existing_chunks_q.iter() {
            if tcm.parent_map_id == scene_state_data_res.map_id && tcm.scale == target_scale && has_mesh {
                ready_target_chunks.push((tcm.gx, tcm.gy));
            }
        }
        ready_target_chunks.sort_unstable();

        for (entity, tcm, _) in existing_chunks_q.iter() {
            if tcm.parent_map_id != scene_state_data_res.map_id || tcm.scale != committed_scale {
                continue;
            }

            if chunk_is_covered_by_ready_target(
                tcm.gx,
                tcm.gy,
                committed_scale,
                target_scale,
                &ready_target_chunks,
            ) {
                queue_entity_for_despawn(&mut locals.pending_despawns, entity);
            }
        }

        locals.pending_spawns.retain(|spawn| {
            spawn.scale != committed_scale
                || !chunk_is_covered_by_ready_target(
                    spawn.gx,
                    spawn.gy,
                    committed_scale,
                    target_scale,
                    &ready_target_chunks,
                )
        });

        let target_ready = locals
            .transition_required_chunks
            .iter()
            .all(|coords| ready_target_chunks.binary_search(coords).is_ok());

        if target_ready {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldLand,
                &format!(
                    "Chunk scale handoff completed: {} -> {}. Retiring old chunks.",
                    committed_scale, target_scale
                ),
            );

            for (entity, tcm, _) in existing_chunks_q.iter() {
                if tcm.parent_map_id == scene_state_data_res.map_id && tcm.scale == committed_scale {
                    queue_entity_for_despawn(&mut locals.pending_despawns, entity);
                }
            }

            locals.pending_spawns.retain(|spawn| spawn.scale != committed_scale);
            locals.committed_scale = Some(target_scale);
            locals.transition_target_scale = None;
            locals.transition_required_chunks.clear();
        }
    }

    // Drain up to MAX_SPAWNS_PER_FRAME from the front (closest to camera first).
    let new_map_id = scene_state_data_res.map_id;
    let batch_size = locals.pending_spawns.len().min(MAX_SPAWNS_PER_FRAME);
    for spawn in locals.pending_spawns[..batch_size].iter().copied() {
        let gx = spawn.gx;
        let gy = spawn.gy;
        let chunk_origin_tile_units_x = gx * land::TILE_NUM_PER_CHUNK_DIM;
        let chunk_origin_tile_units_z = gy * land::TILE_NUM_PER_CHUNK_DIM;
        commands.spawn((
            land::LCMesh {
                parent_map_id: new_map_id,
                gx,
                gy,
                scale: spawn.scale,
                last_blocks_loaded_version: None,
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

    // Retire obsolete chunks after new ones have had a chance to appear.
    let despawn_batch_size = locals.pending_despawns.len().min(MAX_DESPAWNS_PER_FRAME);
    for entity in locals.pending_despawns[..despawn_batch_size].iter().copied() {
        if let Ok((_, tcm, _)) = existing_chunks_q.get(entity) {
            log_chunk_despawn(tcm.gx, tcm.gy, tcm.parent_map_id);
        }
        commands.entity(entity).despawn();
    }
    current_chunk_count = current_chunk_count.saturating_sub(despawn_batch_size as i32);
    locals.pending_despawns.drain(..despawn_batch_size);

    // Update chunk count incrementally; this avoids a full ECS scan every frame.
    land_chunk_count.0 = current_chunk_count.max(0) as u32;

    runtime_diagnostics.map_id = scene_state_data_res.map_id;
    runtime_diagnostics.visible_chunk_target = locals.last_logged_visible_target.len();
    runtime_diagnostics.desired_scale = Some(chunk_scale_res.0);
    runtime_diagnostics.committed_scale = locals.committed_scale;
    runtime_diagnostics.transition_target_scale = locals.transition_target_scale;
    runtime_diagnostics.live_chunks = land_chunk_count.0;
    runtime_diagnostics.pending_spawns = locals.pending_spawns.len();
    runtime_diagnostics.pending_despawns = locals.pending_despawns.len();
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
