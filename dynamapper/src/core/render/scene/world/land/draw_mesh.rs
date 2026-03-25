#![allow(unused_parens, unused)]

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    mesh::{Indices, MeshVertexAttribute},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType},
    shader::ShaderRef,
};
use bytemuck::Zeroable;
use std::time::Instant;
use std::sync::Arc;
use uocf::geo::map::MapPlane;
use uocf::geo::{
    land_texture_2d::{LandTextureSize, TexMap2D},
    map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos},
};

use super::chunk_loader;

use super::TILE_NUM_PER_CHUNK_DIM;
use super::{mesh_material::*, LCMesh, TILE_NUM_PER_CHUNK_TOTAL};
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
    if zoom >= 80.0 {
        32
    } else if zoom >= 64.0 {
        16
    } else if zoom >= 50.0 {
        8
    } else if zoom >= 25.0 {
        4
    } else if zoom >= 10.0 {
        2
    } else {
        1
    }
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

use crate::core::render::scene::world::land::tile_atlas::{Rg16u, TileAtlas};

#[derive(Component)]
pub struct PendingTextureBake;

#[derive(Resource)]
pub struct SharedLandMaterial(pub Handle<LandCustomMeshMaterial>);

#[derive(Resource)]
pub struct LandMeshScratch {
    blocks_to_draw: Vec<MapBlockRelPos>,
    block_seen_bits: Vec<u64>,
    blocks_data: Vec<(u64, MapBlock)>,
    missing_tile_bits: Vec<u64>,
    ids: Vec<u16>,
    texture_lookup_cache: Vec<u32>,
    /// Reusable buffer for atlas texel packing (avoids per-sub-block allocation).
    texels: Vec<Rg16u>,
}
impl Default for LandMeshScratch {
    fn default() -> Self {
        Self {
            blocks_to_draw: Vec::new(),
            block_seen_bits: Vec::new(),
            blocks_data: Vec::new(),
            missing_tile_bits: vec![0; LandTextureCache::TILE_BITSET_SIZE],
            ids: Vec::new(),
            texture_lookup_cache: vec![u32::MAX; LandTextureCache::MAX_TILE_ID],
            texels: Vec::new(),
        }
    }
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
    if current_scale.0 != 1 {
        return;
    }

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
    blocks_data_map: &[(u64, MapBlock)],
    texture_lookup_cache: &mut [u32],
    texels_buf: &mut Vec<Rg16u>,
    lossy_compression: bool,
) -> bool {
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_DIM;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_DIM;

    let chunk_rel_coords = MapBlockRelPos {
        x: chunk_data_ref.chunk_origin_chunk_units_x,
        y: chunk_data_ref.chunk_origin_chunk_units_z,
    };
    
    let block = match blocks_data_map.binary_search_by_key(&chunk_rel_coords.as_u64(), |(k, _)| *k) {
        Ok(idx) => &blocks_data_map[idx].1,
        Err(_) => return false,
    };

    // NOTE: texture_lookup_cache is NOT cleared here — it persists across
    // all sub-blocks within a frame, ensuring consistent layer assignments
    // and avoiding redundant get_texture_size_layer lookups.
    texels_buf.clear();
    let mut has_fallback = false;

    for cell in &block.cells {
        let packed = texture_lookup_cache[cell.id as usize];
        let (texture_size, layer) = if packed != u32::MAX {
            let size_bit = packed & 1;
            let layer = packed >> 1;
            let size = if size_bit == 0 { LandTextureSize::Small } else { LandTextureSize::Big };
            (size, layer)
        } else {
            let (size, layer) = texture_cache.get_texture_size_layer(texmap_2d_r.clone(), cell.id, lossy_compression);
            let size_bit = match size { LandTextureSize::Small => 0, LandTextureSize::Big => 1 };
            texture_lookup_cache[cell.id as usize] = (layer << 1) | size_bit;
            (size, layer)
        };

        if layer == 0 {
            has_fallback = true;
        }

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

    let (layer, _evicted) =
        tile_atlas.ensure_layer_for_page(IVec2::new(page_x as i32, page_y as i32));

    tile_atlas.enqueue_rg16u_block(
        layer,
        UVec2::new(off_x_in_page, off_y_in_page),
        UVec2::new(TILE_NUM_PER_CHUNK_DIM, TILE_NUM_PER_CHUNK_DIM),
        texels_buf,
    );

    has_fallback
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
    chunk_q: Query<(Entity, &LCMesh, Has<Mesh3d>), Or<(Without<Mesh3d>, With<PendingTextureBake>)>>,
    land_mesh_handles_r: Res<LandMeshHandles>,
    current_lod: Res<LandMeshLod>,
    shared_land_material_r: Res<SharedLandMaterial>,
    settings: Res<crate::external_data::settings::Settings>,
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
            if let Some((_, plane)) = map_planes_r
                .0
                .iter_mut()
                .find(|(id, _)| *id == current_map_id)
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
    scratch.blocks_to_draw.clear();
    scratch.blocks_data.clear();
    scratch.texture_lookup_cache.fill(u32::MAX);
    if scratch.missing_tile_bits.len() != 1024 {
        scratch.missing_tile_bits.resize(1024, 0);
    } else {
        scratch.missing_tile_bits.fill(0);
    }
    scratch.ids.clear();

    let mut targets: Vec<LandChunkConstructionData> = chunk_q
        .iter()
        .map(|(entity, chunk_data, has_mesh)| LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x: chunk_data.gx,
            chunk_origin_chunk_units_z: chunk_data.gy,
            chunk_scale: chunk_data.scale,
            has_mesh,
        })
        .collect();

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
    let map_meta = world_geo_data_r
        .maps
        .get(&current_map_id)
        .expect("Requested metadata for uncached map");
    let max_chunk_x = (map_meta.width / TILE_NUM_PER_CHUNK_DIM) as i32;
    let max_chunk_y = (map_meta.height / TILE_NUM_PER_CHUNK_DIM) as i32;

    // ── Partition targets: ready (all blocks cached) vs deferred ────────
    // Chunks whose data is already in the MapPlane cache render immediately.
    // Chunks with missing blocks are deferred and their blocks dispatched to
    // the background loader.
    let mut ready_targets: Vec<LandChunkConstructionData> = Vec::with_capacity(targets.len());
    let mut uncached_blocks: Vec<MapBlockRelPos> = Vec::new();

    {
        let plane_ref = map_planes_r
            .0
            .iter()
            .find(|(id, _)| *id == current_map_id)
            .map(|(_, p)| p)
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
            }
        }
    }

    // Dispatch uncached blocks to background loader (if any and not already pending).
    if !uncached_blocks.is_empty() && !locals.pending {
        // Deduplicate while preserving the priority order produced by the
        // camera-distance sort above. This keeps nearby chunks at the front
        // of the background-loading queue after teleports.
        // OPTIMIZATION: Replacing HashMap with a bitmask-based deduplication.
        // Bitmasks provide O(1) membership testing and insertion with zero
        // heap allocation after the initial vector is created.
        let mut bitmask = vec![0u64; (((max_chunk_x * max_chunk_y) as usize) >> LandTextureCache::TILE_ID_WORD_SHIFT) + 1]; // Equivalent to / 64
        uncached_blocks.retain(|pos| {
            let idx = (pos.x * max_chunk_y as u32) + pos.y;
            let word = (idx >> (LandTextureCache::TILE_ID_WORD_SHIFT as u32)) as usize; // idx / 64
            if word >= bitmask.len() {
                return true;
            }
            let bit = (idx as usize & LandTextureCache::TILE_ID_BIT_MASK); // idx % 64
            if (bitmask[word] & (1 << bit)) == 0 {
                bitmask[word] |= 1 << bit;
                true
            } else {
                false
            }
        });

        let plane_ref: &MapPlane = map_planes_r
            .0
            .iter()
            .find_map(|(id, map_plane)| {
                if *id == current_map_id {
                    Some(map_plane)
                } else {
                    None
                }
            })
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
        let seen_bits_len = (((max_chunk_x as usize) * (max_chunk_y as usize)) >> LandTextureCache::TILE_ID_WORD_SHIFT) + 1;
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
                        let word = idx >> 6;
                        let bit = idx & 63;
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
        let uo_data_map_plane = map_planes_r
            .0
            .iter_mut()
            .find(|(id, _)| *id == current_map_id)
            .map(|(_, p)| p)
            .expect("Requested map plane metadata is uncached?");

        for block_coords in blocks_to_draw.iter().copied() {
            let Some(block_ref) = uo_data_map_plane.block(block_coords) else {
                continue; // Not cached (border block still loading) — skip.
            };
            
            blocks_data.push((block_coords.as_u64(), block_ref.clone()));

            for tz in 0..8 {
                for tx in 0..8 {
                    // Extract the logic to speed up this function call
                    // if let Ok(cell) = block_ref.cell(tx, tz) {
                        let cell = &block_ref.cells[((MapBlock::CELLS_PER_COLUMN * tz) + tx) as usize];
                        let cell_id = cell.id as usize;
                        let word = cell_id >> LandTextureCache::TILE_ID_WORD_SHIFT;
                        let bit = cell_id & LandTextureCache::TILE_ID_BIT_MASK;
                        missing_tile_bits[word] |= 1u64 << bit;

                }
            }
        }
        // Important: Sort for binary search later in enqueue_chunk_to_atlas_and_preload
        blocks_data.sort_unstable_by_key(|(k, _)| *k);
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
    cache_r.precache_textures_parallel(
        ids.as_slice(),
        texmap_2d_r.0.clone(),
        settings.core.graphics.lossy_texture_compression,
    );

    let build_time_start = Instant::now();
    for chunk_data in ready_targets.iter().rev() {
        // For super-chunks (scale > 1), enqueue ALL sub-blocks into the tile atlas,
        // PLUS a +1 border ring so the edge vertices can sample neighbor heights.
        let gx = chunk_data.chunk_origin_chunk_units_x as i32;
        let gy = chunk_data.chunk_origin_chunk_units_z as i32;
        let scale = chunk_data.chunk_scale as i32;
        let mut overall_has_fallback = false;

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
                    has_mesh: false,
                };
                let fallback = enqueue_chunk_to_atlas_and_preload(
                    &mut cache_r,
                    &mut tile_atlas_r,
                    texmap_2d_r.0.clone(),
                    &sub,
                    blocks_data,
                    texture_lookup_cache,
                    texels_buf,
                    settings.core.graphics.lossy_texture_compression,
                );
                if fallback {
                    overall_has_fallback = true;
                }
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

    let build_time: u128 = build_time_start.elapsed().as_micros();
    if build_time > 1000 {
        console_logger::one(
            None,
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
            None,
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
