#![allow(unused_parens, unused)]

use bevy::{
    asset::RenderAssetUsages, camera::visibility::NoFrustumCulling, mesh::{Indices, MeshVertexAttribute}, pbr::{ExtendedMaterial, MaterialExtension}, prelude::*, render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType}, shader::ShaderRef
};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bytemuck::Zeroable;
use std::time::Instant;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use uocf::geo::{
    land_texture_2d::{LandTextureSize, TexMap2D},
    map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos},
};

use super::chunk_loader;

use super::TILE_NUM_PER_CHUNK_DIM;
use super::{LCMesh, mesh_material::*, TILE_NUM_PER_CHUNK_TOTAL};
use crate::{
    core::{
        constants,
        maps::MapPlaneMetadata,
        render::scene::{
            SceneStateData, camera::PlayerCamera, player::Player, world::WorldGeoData,
        },
        texture_cache::land::cache::*,
        uo_files_loader::{MapPlanesRes, TexMap2DRes},
    },
    prelude::*,
    util_lib::array::*,
};

// ---- Shared Mesh Resource and Setup ----

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
fn lod_from_zoom(zoom: f32) -> LandMeshLod {
    if zoom >= 10.0 {
        LandMeshLod::Low      // 9 verts, step=4  — chunks are tiny on screen
    } else if zoom >= 4.0 {
        LandMeshLod::Medium   // 25 verts, step=2
    } else {
        LandMeshLod::High     // 81 verts, step=1  — full detail up close
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
    fn default() -> Self { Self(1) }
}

/// Choose chunk scale from zoom level.  Higher zoom → larger chunks → fewer entities.
///  - zoom <  10 → scale 1  (standard 8×8,   entity count ×1)
///  - zoom 10–25 → scale 2  (wide 16×16,     entity count ÷4)
///  - zoom 25–50 → scale 4  (extra-wide 32×32, entity count ÷16)
///  - zoom 50–64 → scale 8  (ultra 64×64,    entity count ÷64)
///  - zoom 64–80 → scale 16 (huge 128×128,   entity count ÷256)
///  - zoom ≥ 80  → scale 32 (massive 256×256, entity count ÷1024)
pub fn scale_from_zoom(zoom: f32) -> u32 {
    if zoom >= 80.0 { 32 }
    else if zoom >= 64.0 { 16 }
    else if zoom >= 50.0 { 8 }
    else if zoom >= 25.0 { 4 }
    else if zoom >= 10.0 { 2 }
    else { 1 }
}

/// Select the correct mesh handle for a given chunk scale and LOD.
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

use crate::core::render::scene::world::land::tile_atlas::{TileAtlas, Rg16u};

#[derive(Resource)]
pub struct SharedLandMaterial(pub Handle<LandCustomMeshMaterial>);

#[derive(Resource, Default)]
pub struct LandMeshScratch {
    primary_chunks: HashMap<(u32, u32), (Entity, u32)>,
    spawn_targets: HashSet<LandChunkConstructionData>,
    blocks_to_draw: Vec<MapBlockRelPos>,
    blocks_data: HashMap<MapBlockRelPos, MapBlock>,
    missing_tile_bits: Vec<u64>,
    ids: Vec<u16>,
    texture_lookup_cache: HashMap<u16, (LandTextureSize, u32)>,
    /// Reusable buffer for atlas texel packing (avoids per-sub-block allocation).
    texels: Vec<Rg16u>,
}

pub fn sys_update_existing_chunk_mesh_lod(
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    land_mesh_handles_r: Res<LandMeshHandles>,
    mut current_lod: ResMut<LandMeshLod>,
    current_scale: Res<ChunkScale>,
    mut chunk_mesh_q: Query<&mut Mesh3d, With<LCMesh>>,
) {
    // TODO: add a log message when we change the LOD level because of the zoom level.

    // Scale > 1 uses fixed wide meshes; LOD swaps only matter at scale=1.
    if current_scale.0 != 1 { return; }

    let next_lod = lod_from_zoom(render_zoom.0);
    if *current_lod == next_lod {
        return;
    }

    let next_mesh = mesh_for_lod(&land_mesh_handles_r, next_lod);
    for mut mesh3d in chunk_mesh_q.iter_mut() {
        mesh3d.0 = next_mesh.clone();
    }

    *current_lod = next_lod;
}

/// Enqueues the 8x8 tile data for this chunk into the TileAtlas, and preloads the textures.
fn enqueue_chunk_to_atlas_and_preload(
    texture_cache: &mut ResMut<LandTextureCache>,
    tile_atlas: &mut ResMut<TileAtlas>,
    texmap_2d_r: Arc<TexMap2D>,
    chunk_data_ref: &LandChunkConstructionData,
    blocks_data_map: &HashMap<MapBlockRelPos, MapBlock>,
    texture_lookup_cache: &mut HashMap<u16, (LandTextureSize, u32)>,
    texels_buf: &mut Vec<Rg16u>,
    lossy_compression: bool,
) {
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_DIM;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_DIM;

    let chunk_rel_coords = MapBlockRelPos {
        x: chunk_data_ref.chunk_origin_chunk_units_x,
        y: chunk_data_ref.chunk_origin_chunk_units_z,
    };
    let Some(block) = blocks_data_map.get(&chunk_rel_coords) else {
        // Block not available (e.g. at map edge or missing data) — skip silently.
        return;
    };

    // NOTE: texture_lookup_cache is NOT cleared here — it persists across
    // all sub-blocks within a frame, ensuring consistent layer assignments
    // and avoiding redundant get_texture_size_layer lookups.
    texels_buf.clear();

    for cell in &block.cells {
        let (texture_size, layer) = *texture_lookup_cache.entry(cell.id).or_insert_with(|| {
            texture_cache.get_texture_size_layer(
                texmap_2d_r.clone(),
                cell.id,
                lossy_compression,
            )
        });

        let tex_size_bits = match texture_size {
            LandTextureSize::Small => 0,
            LandTextureSize::Big => 1,
        };

        // Use 'layer' instead of 'cell.id' because that's what the shader needs to sample the 2DArray!
        texels_buf.push(Rg16u::pack(layer as u16, cell.z, tex_size_bits));
    }


    let page_w = tile_atlas.params.page_texels.x;
    let page_h = tile_atlas.params.page_texels.y;
    debug_assert!(
        page_w % TILE_NUM_PER_CHUNK_DIM == 0 && page_h % TILE_NUM_PER_CHUNK_DIM == 0,
        "Page size must be a multiple of chunk size to avoid split logic"
    );

    let page_x = chunk_origin_tile_units_x / page_w;
    let page_y = chunk_origin_tile_units_z / page_h;

    let off_x_in_page = chunk_origin_tile_units_x % page_w;
    let off_y_in_page = chunk_origin_tile_units_z % page_h;

    let (layer, _evicted) = tile_atlas.ensure_layer_for_page(IVec2::new(page_x as i32, page_y as i32));

    tile_atlas.enqueue_rg16u_block(
        layer,
        UVec2::new(off_x_in_page, off_y_in_page),
        UVec2::new(TILE_NUM_PER_CHUNK_DIM, TILE_NUM_PER_CHUNK_DIM),
        texels_buf,
    );
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
}

/// Local state for the background chunk-data loader.
#[derive(Default)]
pub struct DrawMeshLocals {
    loader: Option<chunk_loader::ChunkLoaderThread>,
    pending: bool,
    /// Expanded block coordinates needed for a pending background load.
    /// Kept alive so we can skip re-expanding on the poll frame.
    pending_blocks: Vec<MapBlockRelPos>,
    /// Entity targets that were deferred because their blocks weren't cached yet.
    deferred_targets: Vec<LandChunkConstructionData>,
    /// Track the last observed scale to detect scale changes and clear pinned textures.
    last_scale: u32,
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
    chunk_q: Query<(Entity, &LCMesh), Without<Mesh3d>>,
    land_mesh_handles_r: Res<LandMeshHandles>,
    current_lod: Res<LandMeshLod>,
    shared_land_material_r: Res<SharedLandMaterial>,
    cache_settings_r: Res<crate::core::texture_cache::land::cache::LandTextureCacheSettings>,
    mut locals: Local<DrawMeshLocals>,
) {
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
        let current_scale = chunk_q.iter().next().map_or(0, |(_, lc)| lc.scale);
        if current_scale != 0 && current_scale != locals.last_scale {
            cache_r.clear_pinned_textures();
            locals.deferred_targets.clear();
            locals.last_scale = current_scale;
        }
    }

    // ── Poll for completed background load sub-batches ───────────────
    // The loader now sends results in sub-batches.  Drain all available
    // results each frame so deferred targets become ready progressively.
    if locals.pending {
        let results = locals.loader.as_ref().unwrap().drain_results();
        if !results.is_empty() {
            let planes_arc = map_planes_r.0.clone();
            let mut plane = planes_arc.get_mut(&current_map_id)
                .expect("Requested map plane metadata is uncached?");
            for result in results {
                plane.insert_preloaded_blocks(result.loaded_blocks);
                if result.is_final {
                    locals.pending = false;
                }
            }
            drop(plane);
        }
        // If still loading, DON'T return — process any chunks we CAN render.
    }

    let mut scratch = land_mesh_scratch_r;
    scratch.blocks_to_draw.clear();
    scratch.blocks_data.clear();
    scratch.texture_lookup_cache.clear();
    if scratch.missing_tile_bits.len() != 1024 {
        scratch.missing_tile_bits.resize(1024, 0);
    } else {
        scratch.missing_tile_bits.fill(0);
    }
    scratch.ids.clear();

    let mut targets: Vec<LandChunkConstructionData> = chunk_q
        .iter()
        .map(|(entity, chunk_data)| LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x: chunk_data.gx,
            chunk_origin_chunk_units_z: chunk_data.gy,
            chunk_scale: chunk_data.scale,
        })
        .collect();

    if targets.is_empty() && locals.deferred_targets.is_empty() {
        return;
    }

    let current_camera_chunk = camera_q
        .single()
        .ok()
        .map(|(_, camera_tf)| {
            let cam_translation = camera_tf.translation();
            (
                (cam_translation.x.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
                (cam_translation.z.floor() as i32).div_euclid(TILE_NUM_PER_CHUNK_DIM as i32),
            )
        })
        .unwrap_or((0, 0));
    sort_construction_targets(&mut targets, current_camera_chunk);

    // Re-add deferred targets from a previous frame's background load.
    // Then deduplicate: chunk_q may re-report entities that were deferred last frame.
    {
        let mut deferred = std::mem::take(&mut locals.deferred_targets);
        targets.append(&mut deferred);
        let mut seen_entities: HashSet<Entity> = HashSet::with_capacity(targets.len());
        targets.retain(|t| {
            if let Some(e) = t.entity {
                seen_entities.insert(e)
            } else {
                true
            }
        });
    }

    // Expand each primary chunk target into all required base-block coordinates.
    // For scale>1 super-chunks this includes all scale×scale sub-blocks.
    // Border ring is NOT needed here — the shader samples from the atlas which
    // is populated per-block; edge stitching is handled by enqueuing +1 border
    // blocks to the atlas in the rendering loop below.
    let map_meta = world_geo_data_r.maps.get(&current_map_id)
        .expect("Requested metadata for uncached map");
    let max_chunk_x = (map_meta.width / TILE_NUM_PER_CHUNK_DIM) as i32;
    let max_chunk_y = (map_meta.height / TILE_NUM_PER_CHUNK_DIM) as i32;

    // ── Partition targets: ready (all blocks cached) vs deferred ────────
    // Chunks whose data is already in the MapPlane cache render immediately.
    // Chunks with missing blocks are deferred and their blocks dispatched to
    // the background loader.
    let mut ready_targets: Vec<LandChunkConstructionData> = Vec::with_capacity(targets.len());
    let mut new_deferred: Vec<LandChunkConstructionData> = Vec::new();
    let mut uncached_blocks: Vec<MapBlockRelPos> = Vec::new();

    {
        let planes_arc = map_planes_r.0.clone();
        let plane_ref = planes_arc
            .get(&current_map_id)
            .expect("Requested map plane metadata is uncached?");

        for target in &targets {
            let gx = target.chunk_origin_chunk_units_x;
            let gy = target.chunk_origin_chunk_units_z;
            let scale = target.chunk_scale as i32;

            // Readiness: only CORE blocks (0..scale) must be cached.
            // Border blocks (-1 and +scale) are dispatched to the loader
            // but don't block rendering — they'll be picked up for atlas
            // enqueue when available.
            let mut all_cached = true;
            for sx in 0..scale {
                for sz in 0..scale {
                    let bx = gx as i32 + sx;
                    let bz = gy as i32 + sz;
                    if bx < 0 || bx >= max_chunk_x || bz < 0 || bz >= max_chunk_y {
                        continue;
                    }
                    let pos = MapBlockRelPos { x: bx as u32, y: bz as u32 };
                    if !plane_ref.is_block_cached(&pos) {
                        all_cached = false;
                        uncached_blocks.push(pos);
                    }
                }
            }

            // Also dispatch border ring blocks to the loader (but don't
            // gate readiness on them).
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
                    let pos = MapBlockRelPos { x: bx as u32, y: bz as u32 };
                    if !plane_ref.is_block_cached(&pos) {
                        uncached_blocks.push(pos);
                    }
                }
            }

            if all_cached {
                ready_targets.push(*target);
            } else {
                new_deferred.push(*target);
            }
        }
    }

    // Dispatch uncached blocks to background loader (if any and not already pending).
    if !uncached_blocks.is_empty() && !locals.pending {
        // Deduplicate
        uncached_blocks.sort_unstable();
        uncached_blocks.dedup();

        let planes_arc = map_planes_r.0.clone();
        let plane_ref = planes_arc
            .get(&current_map_id)
            .expect("Requested map plane metadata is uncached?");

        locals.loader.as_ref().unwrap().send_request(chunk_loader::LoadRequest {
            map_file_path: plane_ref.file_path().to_path_buf(),
            size_blocks_height: plane_ref.size_blocks.height,
            blocks_to_load: uncached_blocks,
            texmap_2d: texmap_2d_r.0.clone(),
        });
        locals.pending = true;
    }

    // Stash deferred targets for next frame.
    locals.deferred_targets = new_deferred;

    // ── Per-frame chunk budget ─────────────────────────────────────────
    // Cap the amount of atlas-enqueue + texture-precache + mesh work we
    // do in a single frame.  Without this, zooming out to scale 32 with
    // hundreds of ready chunks causes a multi-millisecond stall.
    // Closest-to-camera chunks are processed first; the remainder will
    // naturally re-appear in chunk_q next frame (they still lack Mesh3d).
    //
    // Budget unit = "equivalent base blocks" ≈ (scale + 2)² per chunk
    // (sub-blocks + border ring that the atlas-enqueue loop iterates).
    const MAX_ATLAS_BLOCKS_PER_FRAME: usize = 4096;

    sort_construction_targets(&mut ready_targets, current_camera_chunk);
    {
        let mut remaining = MAX_ATLAS_BLOCKS_PER_FRAME;
        let mut count = 0usize;
        for t in ready_targets.iter() {
            let cost = (t.chunk_scale as usize + 2).pow(2);
            // Always process at least one chunk to guarantee forward progress.
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
        let mut block_set: HashSet<MapBlockRelPos> = HashSet::with_capacity(ready_targets.len() * 4);
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
                        block_set.insert(MapBlockRelPos { x: bx as u32, y: bz as u32 });
                    }
                }
            }
        }
        scratch.blocks_to_draw.extend(block_set.iter());
    }

    let mut blocks_to_draw = std::mem::take(&mut scratch.blocks_to_draw);
    let LandMeshScratch {
        blocks_data,
        missing_tile_bits,
        texture_lookup_cache,
        ids,
        texels: texels_buf,
        ..
    } = &mut *scratch;

    // ── Populate blocks_data from the in-memory cache only ──────────────
    // All core blocks for ready_targets are guaranteed cached (readiness check).
    // Border ring blocks may or may not be cached yet — if not, they're
    // simply skipped here and the atlas enqueue will skip them too.
    // NO DISK I/O on the main thread.
    {
        let uo_data_map_planes_arc = map_planes_r.0.clone();
        let mut uo_data_map_plane = uo_data_map_planes_arc
            .get_mut(&current_map_id)
            .expect("Requested map plane metadata is uncached?");

        for block_coords in blocks_to_draw.iter().copied() {
            let Some(block_ref) = uo_data_map_plane.block(block_coords) else {
                continue; // Not cached (border block still loading) — skip.
            };
            if blocks_data.insert(block_coords, block_ref.clone()).is_some() {
                continue;
            }

            for tz in 0..8 {
                for tx in 0..8 {
                    if let Ok(cell) = block_ref.cell(tx, tz) {
                        let cell_id = cell.id as usize;
                        let word = cell_id >> 6;
                        let bit = cell_id & 63;
                        missing_tile_bits[word] |= 1u64 << bit;
                    }
                }
            }
        }
    }

    for (word_idx, &word) in missing_tile_bits.iter().enumerate() {
        let mut bits = word;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            ids.push((word_idx * 64 + bit) as u16);
            bits &= bits - 1;
        }
    }

    cache_r.set_visible_texture_usage_hint(ids);
    cache_r.precache_textures_parallel(
        ids.as_slice(),
        texmap_2d_r.0.clone(),
        cache_settings_r.lossy_texture_compression,
    );

    let build_time_start = Instant::now();
    for chunk_data in ready_targets.iter() {
        // For super-chunks (scale > 1), enqueue ALL sub-blocks into the tile atlas,
        // PLUS a +1 border ring so the edge vertices can sample neighbor heights.
        let gx = chunk_data.chunk_origin_chunk_units_x as i32;
        let gy = chunk_data.chunk_origin_chunk_units_z as i32;
        let scale = chunk_data.chunk_scale as i32;
        for sx in -1..=scale {
            for sz in -1..=scale {
                let bx = gx + sx;
                let bz = gy + sz;
                if bx < 0 || bx >= max_chunk_x || bz < 0 || bz >= max_chunk_y {
                    continue;
                }
                let sub = LandChunkConstructionData {
                    entity: None,
                    chunk_origin_chunk_units_x: bx as u32,
                    chunk_origin_chunk_units_z: bz as u32,
                    chunk_scale: 1,
                };
                enqueue_chunk_to_atlas_and_preload(
                    &mut cache_r,
                    &mut tile_atlas_r,
                    texmap_2d_r.0.clone(),
                    &sub,
                    blocks_data,
                    texture_lookup_cache,
                    texels_buf,
                    cache_settings_r.lossy_texture_compression,
                );
            }
        }

        if let Some(entity) = chunk_data.entity {
            if commands.get_entity(entity).is_ok() {
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

    let build_time: u128 = build_time_start.elapsed().as_micros();
    if build_time > 1000 {
        console_logger::one(
            None,
            LogSev::Diagnostics,
            LogAbout::Performance,
            &format!("Perf: chunk rendering preloader took {build_time} µs for {} chunks.", ready_targets.len()),
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
    let chunk_mesh_handle: Handle<Mesh> = mesh_for_scale(land_mesh_handles_r, chunk_scale, current_lod);
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
            None,
            LogSev::Error,
            LogAbout::RenderWorldLand,
            "Skipping drawing of invalid/unspawned entity at stage 'build_indexed_chunk_mesh'.",
        );
    }
}

/// Sort construction targets so the closest primary chunks are processed first,
/// followed by their dependent filler targets.
fn sort_construction_targets(chunks: &mut [LandChunkConstructionData], camera_chunk: (i32, i32)) {
    let cx = camera_chunk.0;
    let cy = camera_chunk.1;
    chunks.sort_unstable_by_key(|target| {
        let dx = target.chunk_origin_chunk_units_x as i32 - cx;
        let dy = target.chunk_origin_chunk_units_z as i32 - cy;
        // Primary chunks (entity: Some) before filler targets.
        (target.entity.is_none(), dx.abs().max(dy.abs()), dx.abs() + dy.abs())
    });
}


