#![allow(unused_parens, unused)]

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::{
    asset::RenderAssetUsages,
    //camera::visibility::NoFrustumCulling,
    ecs::system::SystemParam,
    mesh::{Indices, MeshVertexAttribute},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType},
    shader::ShaderRef,
};
use bytemuck::Zeroable;
use std::sync::Arc;
use std::time::Instant;
use uocf::classic::map::MapPlane;
use uocf::classic::{
    land_texture::{LandTextureSize, TexMap},
    map::{MapBlock, MapBlockRelPos},
};

use super::chunk_loader;

use super::TILE_NUM_PER_CHUNK_DIM;
use super::{mesh_material::*, LandUploadBudget, LCMesh, TILE_NUM_PER_CHUNK_TOTAL};
use crate::{
    core::{
        constants,
        maps::MapPlaneMetadata,
        render::scene::{
            camera::PlayerCamera, player::Player, world::WorldGeoData, SceneStateData,
        },
        texture_cache::land::cache::*,
        uo_files_loader::{MapPlanesRes, TexMap2DRes},
    },
    prelude::*,
    util_lib::array::*,
};

// ---- Shared Mesh Resource and Setup ----
//
// Design rationale — Two-Level Mesh Selection (Scale + LOD)
// =========================================================
//
// The terrain renderer uses a two-level dispatch for choosing the mesh
// assigned to each chunk entity:
//
//   1. **Chunk Scale** (`scale_from_zoom` → `mesh_for_scale`):  At higher zoom
//      levels, multiple 8×8 base blocks are merged into a single "super-chunk"
//      entity (16×16, 32×32, ... up to 256×256 tiles).  Each wide mesh keeps a
//      constant 81 vertices but increases the step between samples so the mesh
//      covers a larger world area.  This is the *primary* mechanism for
//      reducing draw-call and entity pressure as the camera zooms out.
//
//   2. **LOD within Scale=1** (`lod_from_zoom` → `mesh_for_lod`):  When scale
//      is 1 (zoom < 10), the standard 8×8-tile chunk can use three detail
//      levels: High (81 verts), Medium (25 verts), Low (9 verts).
//      `sys_update_existing_chunk_mesh_lod` live-swaps the `Mesh3d` handle on
//      all existing entities when the LOD threshold is crossed — no geometry
//      rebuild, just a handle reassignment.
//
// Analysis:
//
//   For real-time interactive rendering, the LOD variation at scale=1 saves
//   negligible GPU work (at most ~7 200 VS invocations across ~100 chunks),
//   since modern GPUs process millions of vertices per millisecond.
//
//   However, the LOD system becomes meaningful for **offline / export
//   rendering**: when rasterising the full 7 000 × 4 000 tile map at 1:1 or
//   1:10 zoom to an image file, the renderer must produce geometry for the
//   entire world at the chosen detail level.  In that scenario:
//     - 1:1 export at scale=1 → ~437 500 chunks × 81 verts = 35 M vertices.
//       Dropping to Medium (25 verts) or Low (9 verts) reduces this to
//       ~11 M or ~4 M, which can matter for the offline capture pipeline
//       even if the GPU could handle 35 M.
//     - The pre-built mesh assets cost trivial VRAM (8 meshes, ≤ 81 verts
//       each ≈ a few KB total), so the complexity cost is purely code-side.
//
//   Wide meshes (scale ≥ 2) are always 81 verts because at those zoom levels
//   the world area already guarantees low screen-space density.  LOD swaps
//   only fire at scale=1 and only on zoom boundary crossings (zoom 4.0 and
//   10.0), so the per-frame overhead is near-zero.
//

#[derive(Resource)]
pub struct LandMeshHandles {
    pub high: Handle<Mesh>,
    pub medium: Handle<Mesh>,
    pub low: Handle<Mesh>,
    /// 16×16 tile mesh (step=2, 81 verts) for zoom 10–25.
    pub wide16: Handle<Mesh>,
    /// 32×32 tile mesh (step=4, 81 verts) for zoom 25–50.
    pub wide32: Handle<Mesh>,
    /// 64×64 tile mesh (step=8, 81 verts) for zoom ≥50.
    pub wide64: Handle<Mesh>,
    /// 128×128 tile mesh (step=16, 81 verts) for extreme zoom-out.
    pub wide128: Handle<Mesh>,
    /// 256×256 tile mesh (step=32, 81 verts) for maximum zoom-out.
    pub wide256: Handle<Mesh>,
}

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LandMeshLod {
    #[default]
    High,
    Medium,
    Low,
}

/// Automatic LOD selection based on camera zoom level.
/// At higher zoom (more zoomed out), fewer vertices are needed because each
/// chunk covers fewer screen pixels.
///
/// Only effective at scale=1 (zoom < 10).  For real-time interactive use the
/// vertex savings are negligible, but the distinction matters for offline
/// full-map exports where the entire 7k×4k world is rasterised at once.
fn lod_from_zoom(zoom: f32) -> LandMeshLod {
    if zoom >= 10.0 {
        LandMeshLod::Low // 9 verts, step=4  — chunks are tiny on screen
    } else if zoom >= 4.0 {
        LandMeshLod::Medium // 25 verts, step=2
    } else {
        LandMeshLod::High // 81 verts, step=1  — full detail up close
    }
}

fn mesh_for_lod(handles: &LandMeshHandles, lod: LandMeshLod) -> Handle<Mesh> {
    match lod {
        LandMeshLod::High => handles.high.clone(),
        LandMeshLod::Medium => handles.medium.clone(),
        LandMeshLod::Low => handles.low.clone(),
    }
}

/// Active chunk scale: how many base 8×8 blocks each entity covers per dimension.
/// 1 = standard (8×8 tiles), 2 = wide (16×16), 4 = extra-wide (32×32), 8 = ultra (64×64).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkScale(pub u32);
impl Default for ChunkScale {
    fn default() -> Self {
        Self(1)
    }
}

/// Choose chunk scale from zoom level.  Higher zoom → larger chunks → fewer entities.
///  - zoom <  10 → scale 1  (standard 8×8,   entity count ×1)
///  - zoom 10–25 → scale 2  (wide 16×16,     entity count ÷4)
///  - zoom 25–50 → scale 4  (extra-wide 32×32, entity count ÷16)
///  - zoom 50–64 → scale 8  (ultra 64×64,    entity count ÷64)
///  - zoom 64–80 → scale 16 (huge 128×128,   entity count ÷256)
///  - zoom ≥ 80  → scale 32 (massive 256×256, entity count ÷1024)
pub fn scale_from_zoom(zoom: f32) -> u32 {
    if zoom >= 50.0 {
        32
    } else if zoom >= 35.0 {
        16
    } else if zoom >= 20.0 {
        8
    } else if zoom >= 10.0 {
        4
    } else if zoom >= 5.0 {
        2
    } else {
        1
    }
}

/// Select the correct mesh handle for a given chunk scale and LOD.
///
/// Top-level dispatch: wide meshes for scale ≥ 2 (fixed 81 verts covering a
/// larger world area), or LOD-variable meshes for scale=1.  See the block
/// comment above `LandMeshHandles` for the full design rationale.
fn mesh_for_scale(handles: &LandMeshHandles, scale: u32, lod: LandMeshLod) -> Handle<Mesh> {
    match scale {
        32 => handles.wide256.clone(),
        16 => handles.wide128.clone(),
        8 => handles.wide64.clone(),
        4 => handles.wide32.clone(),
        2 => handles.wide16.clone(),
        _ => mesh_for_lod(handles, lod),
    }
}

use crate::core::render::scene::world::land::tile_atlas::{Rg16u, TileAtlas};

#[derive(Component)]
pub struct PendingTextureBake;

#[derive(Resource)]
pub struct SharedLandMaterial(pub Handle<LandCustomMeshMaterial>);

#[derive(Resource)]
pub struct LandMeshScratch {
    blocks_to_draw: Vec<MapBlockRelPos>,
    block_seen_bits: Vec<u64>,
    /// Fixed-size bitset (8 KB) — one bit per possible tile ID.
    missing_tile_bits: [u64; LandTextureCache::TILE_BITSET_SIZE],
    ids: Vec<u16>,
    texture_lookup_cache: Vec<u32>,
    /// Reusable bitmask for deduplicating uncached block coordinates.
    dedup_bits: Vec<u64>,
}
impl Default for LandMeshScratch {
    fn default() -> Self {
        Self {
            blocks_to_draw: Vec::new(),
            block_seen_bits: Vec::new(),
            missing_tile_bits: [0u64; LandTextureCache::TILE_BITSET_SIZE],
            ids: Vec::new(),
            texture_lookup_cache: vec![u32::MAX; LandTextureCache::MAX_TILE_ID],
            dedup_bits: Vec::new(),
        }
    }
}

pub fn sys_update_existing_chunk_mesh_lod(
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    land_mesh_handles_r: Res<LandMeshHandles>,
    mut current_lod: ResMut<LandMeshLod>,
    current_scale: Res<ChunkScale>,
    mut chunk_mesh_q: Query<(&LCMesh, &mut Mesh3d)>,
) {
    // Scale > 1 uses fixed wide meshes; LOD swaps only matter at scale=1.
    if current_scale.0 != 1 {
        return;
    }

    let next_lod = lod_from_zoom(render_zoom.0);
    if *current_lod == next_lod {
        return;
    }

    console_logger::one(
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!(
            "Land mesh LOD changed: {:?} -> {:?} (zoom={:.2})",
            *current_lod, next_lod, render_zoom.0
        ),
    );

    let next_mesh = mesh_for_lod(&land_mesh_handles_r, next_lod);
    for (chunk_mesh, mut mesh3d) in chunk_mesh_q.iter_mut() {
        if chunk_mesh.scale != 1 {
            continue;
        }
        mesh3d.0 = next_mesh.clone();
    }

    *current_lod = next_lod;
}

// ---- HELPER TRAITS / UTILS

// Allow easy [f32; 3] conversion from glam::Vec3 (for shaders/Bevy mesh attributes).
trait _Arrayable {
    fn to_array(&self) -> [f32; 3];
}
impl _Arrayable for Vec3 {
    fn to_array(&self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

// ----

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct LandChunkConstructionData {
    entity: Option<Entity>,
    chunk_origin_chunk_units_x: u32,
    chunk_origin_chunk_units_z: u32,
    chunk_scale: u32,
    has_mesh: bool,
}

/// Local state for the background chunk-data loader.
#[derive(Default)]
pub struct DrawMeshLocals {
    loader: Option<chunk_loader::ChunkLoaderThread>,
    pending: bool,
    /// Expanded block coordinates needed for a pending background load.
    /// Kept alive so we can skip re-expanded on the poll frame.
    pending_blocks: Vec<MapBlockRelPos>,
    /// Track the last observed scale to detect scale changes and clear pinned textures.
    last_scale: u32,
}

/// Small wrapper for the frame-pacing resources used by land chunk drawing.
///
/// `sys_draw_spawned_land_chunks` already needs a large number of ECS parameters, and Bevy
/// has a hard limit on how many can be passed directly to one system function. Grouping the
/// pacing-related resources keeps the call site readable and makes the scheduler-facing budget
/// controls explicit in one place.
#[derive(SystemParam)]
pub struct LandFramePacing<'w> {
    pub upload_budget: Res<'w, LandUploadBudget>,
    pub settings: Res<'w, crate::external_data::settings::Settings>,
    pub time: Res<'w, Time<Real>>,
}

/// Main system: finds visible land map chunks and ensures their mesh is generated and rendered.
pub fn sys_draw_spawned_land_chunks(
    mut commands: Commands,
    mut cache_r: ResMut<LandTextureCache>,
    mut tile_atlas_r: ResMut<TileAtlas>,
    mut land_mesh_scratch_r: ResMut<LandMeshScratch>,
    mut map_planes_r: ResMut<MapPlanesRes>,
    texmap_2d_r: Res<TexMap2DRes>,
    scene_state_data_r: Res<SceneStateData>,
    world_geo_data_r: Res<WorldGeoData>,
    camera_q: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut chunk_q: Query<
        (Entity, &mut LCMesh, Has<Mesh3d>),
        Or<(Without<Mesh3d>, With<PendingTextureBake>)>,
    >,
    land_mesh_handles_r: Res<LandMeshHandles>,
    current_lod: Res<LandMeshLod>,
    shared_land_material_r: Res<SharedLandMaterial>,
    frame_pacing: LandFramePacing,
    mut system_diagnostics: ResMut<crate::core::diagnostics::WorldmapSystemDiagnostics>,
    mut locals: Local<DrawMeshLocals>,
) {
    // NOTE: Briefly-used variables (map_meta, targets, uncached_blocks) are
    // scoped or dropped early to reduce peak memory and improve clarity.

    let _timer = crate::core::diagnostics::scoped_worldmap_timer(
        &mut system_diagnostics,
        crate::core::diagnostics::WorldmapTimedSystem::ChunkDraw,
    );
    let _trace_span = crate::tracy_span!("worldmap::chunk_draw");

    let now = frame_pacing
        .time
        .last_update()
        .unwrap_or_else(|| Instant::now());
    let current_map_id = scene_state_data_r.map_id;

    // ── Initialize background loader thread (once) ─────────────────────
    if locals.loader.is_none() {
        locals.loader = Some(chunk_loader::ChunkLoaderThread::new());
    }

    // ── Detect scale / map changes ─────────────────────────────────
    // When all chunks are despawned (scale or map switch), clear the
    // accumulated texture pins so the LRU can reclaim layers.
    {
        // Grab the current scale from any chunk in the query, or 0 if empty.
        let current_scale = chunk_q.iter().next().map_or(0, |(_, lc, _)| lc.scale);
        if current_scale != 0 && current_scale != locals.last_scale {
            locals.last_scale = current_scale;
        }
    }

    // ── Poll for completed background load sub-batches ───────────────
    // The loader now sends results in sub-batches.  Drain all available
    // results each frame so deferred targets become ready progressively.
    if locals.pending {
        let results = locals.loader.as_ref().unwrap().drain_results();
        if !results.is_empty() {
            if let Some(plane) = map_planes_r
                .0
                .get_mut(current_map_id as usize)
                .and_then(|opt| opt.as_mut())
            {
                for result in results {
                    plane.insert_preloaded_blocks(result.loaded_blocks);
                    if result.is_final {
                        locals.pending = false;
                    }
                }
            } else {
                panic!("Requested map plane metadata is uncached?");
            }
        }
        // If still loading, DON'T return — process any chunks we CAN render.
    }

    let mut scratch = land_mesh_scratch_r;
    let mut targets: Vec<LandChunkConstructionData> = {
        let _span = crate::tracy_span!("worldmap::chunk_draw_collect_targets");
        scratch.blocks_to_draw.clear();
        scratch.texture_lookup_cache.fill(u32::MAX);
        scratch.missing_tile_bits.fill(0);
        scratch.ids.clear();

        chunk_q
            .iter()
            .map(|(entity, chunk_data, has_mesh)| LandChunkConstructionData {
                entity: Some(entity),
                chunk_origin_chunk_units_x: chunk_data.gx,
                chunk_origin_chunk_units_z: chunk_data.gy,
                chunk_scale: chunk_data.scale,
                has_mesh,
            })
            .collect()
    };

    /*
    if targets.len() > 0 {
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!("[DBG-Queue] sys_draw_spawned_land_chunks: {} targets", targets.len()),
        );
    }
    */

    if targets.is_empty() {
        return;
    }

    let current_camera_chunk = camera_q
        .single()
        .ok()
        .map(|(_, camera_tf)| {
            let cam_translation = camera_tf.translation();
            // OPTIMIZATION: Using bitshift (>> 3) as a faster equivalent to
            // .div_euclid(8) for the tile-to-chunk coordinate conversion.
            (
                (cam_translation.x.floor() as i32) >> 3,
                (cam_translation.z.floor() as i32) >> 3,
            )
        })
        .unwrap_or((0, 0));
    sort_construction_targets(&mut targets, current_camera_chunk);

    // Expand each primary chunk target into all required base-block coordinates.
    // For scale>1 super-chunks this includes all scale×scale sub-blocks.
    // Border ring is NOT needed here — the shader samples from the atlas which
    // is populated per-block; edge stitching is handled by enqueuing +1 border
    // blocks to the atlas in the rendering loop below.
    // Scope map_meta — only needed to derive max_chunk dimensions.
    let (max_chunk_x, max_chunk_y) = {
        let map_meta = world_geo_data_r
            .maps
            .get(&current_map_id)
            .expect("Requested metadata for uncached map");
        (
            (map_meta.width / TILE_NUM_PER_CHUNK_DIM) as i32,
            (map_meta.height / TILE_NUM_PER_CHUNK_DIM) as i32,
        )
    };

    // ── Partition targets: ready (all blocks cached) vs deferred ────────
    // Chunks whose data is already in the MapPlane cache render immediately.
    // Chunks with missing blocks are deferred and their blocks dispatched to
    // the background loader.
    let mut ready_targets: Vec<LandChunkConstructionData> = Vec::with_capacity(targets.len());
    let mut uncached_blocks: Vec<MapBlockRelPos> = Vec::new();

    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_partition_targets");
        let plane_ref = map_planes_r
            .0
            .get(current_map_id as usize)
            .and_then(|opt| opt.as_ref())
            .expect("Requested map plane not found in MapPlanesRes");

        for target in &targets {
            let gx = target.chunk_origin_chunk_units_x;
            let gy = target.chunk_origin_chunk_units_z;
            let scale = target.chunk_scale as i32;

            let mut all_cached = true;

            let mut skip_core_loop = false;
            if let Ok((_, mut lc, _)) = chunk_q.get_mut(target.entity.unwrap()) {
                if lc.last_blocks_loaded_version == Some(plane_ref.blocks_loaded_version) {
                    all_cached = false;
                    skip_core_loop = true;
                }
            }

            if !skip_core_loop {
                for sx in -1..=scale {
                    for sz in -1..=scale {
                        let bx = gx as i32 + sx;
                        let bz = gy as i32 + sz;
                        if bx < 0 || bx >= max_chunk_x || bz < 0 || bz >= max_chunk_y {
                            continue;
                        }
                        let pos = MapBlockRelPos {
                            x: bx as u32,
                            y: bz as u32,
                        };
                        if !plane_ref.is_block_cached(&pos) {
                            all_cached = false;
                            if !locals.pending {
                                uncached_blocks.push(pos);
                            } else {
                                break;
                            }
                        }
                    }
                    if !all_cached && locals.pending {
                        break;
                    }
                }
            }

            // Also dispatch border ring blocks to the loader (but don't
            // gate readiness on them).
            if !locals.pending {
                for sx in -1..=scale {
                    for sz in -1..=scale {
                        // Skip the interior — already handled above.
                        if sx >= 0 && sx < scale && sz >= 0 && sz < scale {
                            continue;
                        }
                        let bx = gx as i32 + sx;
                        let bz = gy as i32 + sz;
                        if bx < 0 || bx >= max_chunk_x || bz < 0 || bz >= max_chunk_y {
                            continue;
                        }
                        let pos = MapBlockRelPos {
                            x: bx as u32,
                            y: bz as u32,
                        };
                        if !plane_ref.is_block_cached(&pos) {
                            uncached_blocks.push(pos);
                        }
                    }
                }
            }

            if all_cached {
                ready_targets.push(*target);
            } else if let Ok((_, mut lc, _)) = chunk_q.get_mut(target.entity.unwrap()) {
                lc.last_blocks_loaded_version = Some(plane_ref.blocks_loaded_version);
            }
        }
    }

    // Dispatch uncached blocks to background loader (if any and not already pending).
    if !uncached_blocks.is_empty() && !locals.pending {
        let _span = crate::tracy_span!("worldmap::chunk_draw_dispatch_uncached");
        // Deduplicate while preserving the priority order produced by the
        // camera-distance sort above. This keeps nearby chunks at the front
        // of the background-loading queue after teleports.
        // OPTIMIZATION: Reusable bitmask from scratch provides O(1) membership
        // testing with zero heap allocation after the first frame.
        let dedup_len =
            (((max_chunk_x * max_chunk_y) as usize) >> LandTextureCache::TILE_ID_WORD_SHIFT) + 1;
        scratch.dedup_bits.resize(dedup_len, 0);
        scratch.dedup_bits.fill(0);
        let dedup_bits = &mut scratch.dedup_bits;
        uncached_blocks.retain(|pos| {
            let idx = (pos.x * max_chunk_y as u32) + pos.y;
            let word = (idx >> (LandTextureCache::TILE_ID_WORD_SHIFT as u32)) as usize; // idx / 64
            if word >= dedup_bits.len() {
                return true;
            }
            let bit = (idx as usize & LandTextureCache::TILE_ID_BIT_MASK); // idx % 64
            if (dedup_bits[word] & (1 << bit)) == 0 {
                dedup_bits[word] |= 1 << bit;
                true
            } else {
                false
            }
        });

        let plane_ref: &MapPlane = map_planes_r
            .0
            .get(current_map_id as usize)
            .and_then(|opt| opt.as_ref())
            .expect("Requested map plane metadata is uncached?");

        locals
            .loader
            .as_ref()
            .unwrap()
            .send_request(chunk_loader::LoadRequest {
                map_file_path: plane_ref.file_path().to_path_buf(),
                size_blocks_height: plane_ref.size_blocks.height,
                blocks_to_load: uncached_blocks,
                texmap_2d: texmap_2d_r.0.clone(),
            });
        locals.pending = true;
    }
    drop(targets); // No longer needed — only ready_targets used from here.

    // ── Per-frame chunk budget ─────────────────────────────────────────
    // Cap the amount of atlas-enqueue + texture-precache + mesh work we
    // do in a single frame.  Without this, zooming out to scale 32 with
    // hundreds of ready chunks causes a multi-millisecond stall.
    // Closest-to-camera chunks are processed first; the remainder will
    // naturally re-appear in chunk_q next frame (they still lack Mesh3d).
    //
    // Budget unit = "equivalent base blocks" ≈ (scale + 2)² per chunk
    // (sub-blocks + border ring that the atlas-enqueue loop iterates).
    //
    // The budget comes from a shared resource instead of a hardcoded constant so we can
    // tune terrain pacing from config and keep the main-world preparation budget aligned
    // with the render-world upload budget.
    let max_atlas_blocks_per_frame = frame_pacing
        .upload_budget
        .effective_prepare_max_blocks_per_frame();

    {
        let mut remaining = max_atlas_blocks_per_frame;
        let mut count = 0usize;
        for t in ready_targets.iter() {
            let cost = (t.chunk_scale as usize + 2).pow(2);
            if count > 0 && remaining < cost {
                break;
            }
            remaining = remaining.saturating_sub(cost);
            count += 1;
        }
        ready_targets.truncate(count);
    }

    if ready_targets.is_empty() {
        return;
    }

    // ── Build blocks_to_draw for the ready targets only ─────────────────
    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_build_blocks_to_draw");
        let seen_bits_len = (((max_chunk_x as usize) * (max_chunk_y as usize))
            >> LandTextureCache::TILE_ID_WORD_SHIFT)
            + 1;
        if scratch.block_seen_bits.len() != seen_bits_len {
            scratch.block_seen_bits.resize(seen_bits_len, 0);
        } else {
            scratch.block_seen_bits.fill(0);
        }

        for target in &ready_targets {
            let gx = target.chunk_origin_chunk_units_x;
            let gy = target.chunk_origin_chunk_units_z;
            let scale = target.chunk_scale as i32;

            // All sub-blocks inside the super-chunk + 1-block border for atlas edge data.
            for sx in -1..=scale {
                for sz in -1..=scale {
                    let bx = gx as i32 + sx;
                    let bz = gy as i32 + sz;
                    if bx >= 0 && bx < max_chunk_x && bz >= 0 && bz < max_chunk_y {
                        let idx = (bx as usize * max_chunk_y as usize) + bz as usize;
                        let word = idx >> 6; // / 64
                        let bit = idx & 63; // % 64
                        let mask = 1u64 << bit;
                        if (scratch.block_seen_bits[word] & mask) == 0 {
                            scratch.block_seen_bits[word] |= mask;
                            scratch.blocks_to_draw.push(MapBlockRelPos {
                                x: bx as u32,
                                y: bz as u32,
                            });
                        }
                    }
                }
            }
        }
    }

    let mut blocks_to_draw = std::mem::take(&mut scratch.blocks_to_draw);
    let LandMeshScratch {
        missing_tile_bits,
        texture_lookup_cache,
        ids,
        ..
    } = &mut *scratch;

    // ── Collect missing_tile_bits from visible blocks (parallel plane read) ──
    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_collect_missing_tiles");
        let plane_ref = map_planes_r
            .0
            .get(current_map_id as usize)
            .and_then(|opt| opt.as_ref())
            .expect("Requested map plane metadata is uncached?");

        // Divide blocks_to_draw into sub-arrays for parallel map-reduce
        let pool = bevy::tasks::ComputeTaskPool::get();
        let chunk_size = (blocks_to_draw.len() / pool.thread_num()).max(256);

        let thread_bitmasks = pool.scope(|s| {
            for batch in blocks_to_draw.chunks(chunk_size) {
                s.spawn(async move {
                    let mut local_bits = [0u64; LandTextureCache::TILE_BITSET_SIZE];
                    for &block_coords in batch {
                        if let Some(block_ref) = plane_ref.block_no_update(block_coords) {
                            for cell in &block_ref.cells {
                                let cell_id = cell.id as usize;
                                let word = cell_id >> LandTextureCache::TILE_ID_WORD_SHIFT;
                                let bit = cell_id & LandTextureCache::TILE_ID_BIT_MASK;
                                local_bits[word] |= 1u64 << bit;
                            }
                        }
                    }
                    local_bits
                });
            }
        });

        // Reduce locally into the master bitmask
        // TODO (Architectural Review): The reduced loop does exactly 1024 * n_threads iterations.
        // For ~12 threads, that's 12,000 u64 integer OR operations, which is fundamentally
        // instant. This map-reduce pattern perfectly matches the workload characteristics.
        for local_bits in thread_bitmasks {
            for (i, &word) in local_bits.iter().enumerate() {
                missing_tile_bits[i] |= word;
            }
        }
    }

    for (word_idx, &word) in missing_tile_bits.iter().enumerate() {
        let mut bits = word;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            let id = (word_idx << LandTextureCache::TILE_ID_WORD_SHIFT) + bit;
            ids.push(id as u16);
            if word_idx < cache_r.pinned_visible_bits.len() {
                cache_r.pinned_visible_bits[word_idx] |= 1u64 << bit;
            }
            bits &= bits - 1;
        }
    }

    // `ids` contains textures dynamically required. Pinned tracking continues async.
    let compression = crate::core::texture_cache::land::texture_array::TerrainTextureCompression::from_graphics_settings(
        &frame_pacing.settings.graphics,
    );
    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_precache_textures");
        cache_r.precache_textures_parallel(
            ids.as_slice(),
            texmap_2d_r.0.clone(),
            compression,
            now,
        );

        // Pre-populate lookup cache sequentially so background threads don't need mutable cache access
        for &id in &*ids {
            let (size, layer) =
                cache_r.get_texture_size_layer(&texmap_2d_r.0, id, compression, now);
            let size_bit = match size {
                LandTextureSize::Small => 0u32,
                LandTextureSize::Big => 1u32,
            };
            texture_lookup_cache[id as usize] = (layer << 1) | size_bit;
        }
    }

    let build_time_start = Instant::now();

    // Borrow the plane immutably for the enqueue loop
    let plane_ref = map_planes_r
        .0
        .get(current_map_id as usize)
        .and_then(|opt| opt.as_ref())
        .expect("Requested map plane metadata is uncached?");

    // Struct to hold packed payloads from background threads
    struct PackedChunk {
        bx: u32,
        bz: u32,
        has_fallback: bool,
        texels: [Rg16u; TILE_NUM_PER_CHUNK_TOTAL],
    }
    struct SuperChunkResult {
        target_idx: usize,
        sub_chunks: Vec<PackedChunk>,
    }

    // Process blocks in parallel using Bevy's ComputeTaskPool
    let pool: &bevy::tasks::ComputeTaskPool = bevy::tasks::ComputeTaskPool::get();
    let thread_results: Vec<SuperChunkResult> = {
        let _span = crate::tracy_span!("worldmap::chunk_draw_build_chunk_payloads");
        pool.scope(|s| {
            for (target_idx, chunk_data) in ready_targets.iter().enumerate() {
                // Because texture_lookup_cache is populated and read-only, we can share pointers safely
                let lookup_ptr = texture_lookup_cache.as_ptr() as usize;

                s.spawn(async move {
                    let gx = chunk_data.chunk_origin_chunk_units_x as i32;
                    let gy = chunk_data.chunk_origin_chunk_units_z as i32;
                    let scale = chunk_data.chunk_scale as i32;

                    let mut sub_chunks = Vec::with_capacity(((scale + 2) * (scale + 2)) as usize);

                    // Re-hydrate the raw pointer back to a slice safely (reads only)
                    let lookup_slice = unsafe {
                        std::slice::from_raw_parts(
                            lookup_ptr as *const u32,
                            LandTextureCache::MAX_TILE_ID,
                        )
                    };

                    for sx in -1..=scale {
                        for sz in -1..=scale {
                            let bx = gx + sx;
                            let bz = gy + sz;
                            if bx < 0 || bx >= max_chunk_x || bz < 0 || bz >= max_chunk_y {
                                continue;
                            }

                            let chunk_rel_coords = MapBlockRelPos {
                                x: bx as u32,
                                y: bz as u32,
                            };
                            let Some(block) = plane_ref.block_no_update(chunk_rel_coords) else {
                                continue;
                            };

                            let mut texels_local = [Rg16u::zeroed(); TILE_NUM_PER_CHUNK_TOTAL];
                            let mut texel_count = 0;
                            let mut has_fallback = false;

                            for cell in &block.cells {
                                let packed = lookup_slice[cell.id as usize];
                                // ids were pre-populated, this should never be u32::MAX theoretically.
                                debug_assert!(
                                    packed != u32::MAX,
                                    "Missing lookup cache for {}",
                                    cell.id
                                );

                                let size_bit = packed & 1;
                                let layer = packed >> 1;
                                if layer == 0 {
                                    has_fallback = true;
                                }
                                texels_local[texel_count] =
                                    Rg16u::pack(layer as u16, cell.z, size_bit as u16);
                                texel_count += 1;
                            }

                            sub_chunks.push(PackedChunk {
                                bx: bx as u32,
                                bz: bz as u32,
                                has_fallback,
                                texels: texels_local,
                            });
                        }
                    }

                    SuperChunkResult {
                        target_idx,
                        sub_chunks,
                    }
                });
            }
        })
    };

    // Back on the main thread, sequentially enqueue to TileAtlas which mutates it safely.
    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_enqueue_tile_atlas");
        for res in thread_results {
            let chunk_data = &ready_targets[res.target_idx];
            let mut overall_has_fallback = false;

            let page_w = tile_atlas_r.params.page_texels.x;
            let page_h = tile_atlas_r.params.page_texels.y;

            for sub in res.sub_chunks {
                let chunk_origin_tile_units_x = sub.bx * TILE_NUM_PER_CHUNK_DIM;
                let chunk_origin_tile_units_z = sub.bz * TILE_NUM_PER_CHUNK_DIM;

                let page_x = chunk_origin_tile_units_x >> page_w.trailing_zeros();
                let page_y = chunk_origin_tile_units_z >> page_h.trailing_zeros();

                let off_x_in_page = chunk_origin_tile_units_x & (page_w - 1);
                let off_y_in_page = chunk_origin_tile_units_z & (page_h - 1);

                let (layer, _) = tile_atlas_r.ensure_layer_for_page(bevy::prelude::IVec2::new(
                    page_x as i32,
                    page_y as i32,
                ));

                tile_atlas_r.enqueue_rg16u_block(
                    layer,
                    bevy::prelude::UVec2::new(off_x_in_page, off_y_in_page),
                    bevy::prelude::UVec2::new(TILE_NUM_PER_CHUNK_DIM, TILE_NUM_PER_CHUNK_DIM),
                    sub.texels.as_slice(),
                );

                if sub.has_fallback {
                    overall_has_fallback = true;
                }
            }

            if let Some(entity) = chunk_data.entity {
                if let Ok(mut entity_cmds) = commands.get_entity(entity) {
                    if overall_has_fallback {
                        entity_cmds.insert(PendingTextureBake);
                    } else {
                        entity_cmds.remove::<PendingTextureBake>();
                    }

                    if !chunk_data.has_mesh {
                        draw_land_chunk(
                            &mut commands,
                            chunk_data,
                            &land_mesh_handles_r,
                            *current_lod,
                            &shared_land_material_r,
                            chunk_data.chunk_scale,
                        );
                    }
                }
            }
        }
    }
    // NLL ends the `plane_ref` borrow after the loop above, so
    // the mutable borrow below is valid without an explicit drop.

    // ── Deferred last_accessed touch ───────────────────────────────────
    // Touch all blocks that were accessed this frame. This is separated from
    // the enqueue loop to allow immutable plane borrowing during rendering.
    {
        let _span = crate::tracy_span!("worldmap::chunk_draw_touch_accessed_blocks");
        let plane_mut = map_planes_r
            .0
            .get_mut(current_map_id as usize)
            .and_then(|opt| opt.as_mut())
            .expect("Requested map plane metadata is uncached?");

        for block_coords in blocks_to_draw.iter().copied() {
            // block() updates last_accessed; ignore the return value.
            let _ = plane_mut.block(block_coords);
        }
    }

    let build_time: u128 = build_time_start.elapsed().as_micros();
    if build_time > 1000 {
        console_logger::one(
            LogSev::Diagnostics,
            LogAbout::Performance,
            &format!(
                "Perf: chunk rendering preloader took {build_time} µs for {} chunks.",
                ready_targets.len()
            ),
        );
    }

    // Return the blocks_to_draw Vec to scratch so its capacity is reused next frame.
    blocks_to_draw.clear();
    scratch.blocks_to_draw = blocks_to_draw;
}

fn draw_land_chunk(
    commands: &mut Commands,
    chunk_data_ref: &LandChunkConstructionData,
    land_mesh_handles_r: &Res<LandMeshHandles>,
    current_lod: LandMeshLod,
    shared_land_material_r: &Res<SharedLandMaterial>,
    chunk_scale: u32,
) {
    let chunk_mesh_handle: Handle<Mesh> =
        mesh_for_scale(land_mesh_handles_r, chunk_scale, current_lod);
    let chunk_material_handle: Handle<LandCustomMeshMaterial> = shared_land_material_r.0.clone();

    // Compute chunk origin (in tile units) for the transform.
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_DIM;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_DIM;

    // Scale-aware AABB.
    let tile_span = (TILE_NUM_PER_CHUNK_DIM * chunk_scale) as f32;
    let aabb = Aabb::from_min_max(
        Vec3::new(-1.0, -13.0, -1.0),
        Vec3::new(tile_span + 1.0, 13.0, tile_span + 1.0),
    );

    // 7) Attach to entity
    if let Ok(mut entity_commands) = commands.get_entity(chunk_data_ref.entity.unwrap()) {
        entity_commands.insert((
            Mesh3d(chunk_mesh_handle),
            MeshMaterial3d(chunk_material_handle),
            Transform::from_xyz(
                chunk_origin_tile_units_x as f32,
                0.0,
                chunk_origin_tile_units_z as f32,
            ),
            // --------------------------------------------------------------------------
            // MANUAL AABB & FRUSTUM CULLING
            // --------------------------------------------------------------------------
            // Vertex shader displaces Y by up to ±12.8m.  Without a manual AABB that
            // covers this range, Bevy would compute bounds from the flat mesh and cull
            // chunks whose displaced peaks/valleys are still visible.
            //
            // The AABB XZ span scales with chunk_scale:
            //   scale 1 → 8 tiles, scale 2 → 16, scale 4 → 32.
            NoAutoAabb,
            aabb,
        ));
    } else {
        console_logger::one(
            LogSev::Error,
            LogAbout::RenderWorldLand,
            "Skipping drawing of invalid/unspawned entity at stage 'build_indexed_chunk_mesh'.",
        );
    }
}

/// Sort construction targets so the closest primary chunks (near the camera)
/// are processed first, expanding outward.  This ensures the centre of the
/// viewport fills in first within the per-frame budget.
///
/// After an incremental walk, only a thin strip of newly-spawned edge/corner
/// chunks enters the `Without<Mesh3d>` query, and they easily fit within the
/// budget — so the sort order rarely matters for walking.  After a full
/// respawn (teleport / scale change) we want the player's surroundings
/// rendered before distant edges.
fn sort_construction_targets(chunks: &mut [LandChunkConstructionData], camera_chunk: (i32, i32)) {
    let cx = camera_chunk.0;
    let cy = camera_chunk.1;
    chunks.sort_unstable_by_key(|target| {
        let dx = target.chunk_origin_chunk_units_x as i32 - cx;
        let dy = target.chunk_origin_chunk_units_z as i32 - cy;
        let chebyshev = dx.abs().max(dy.abs());
        let manhattan = dx.abs() + dy.abs();
        // Filler targets (entity: None) always come last.
        // Among primary chunks: nearest to camera first → centre fills first.
        (target.entity.is_none() as u8, chebyshev, manhattan)
    });
}
