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
///  - zoom ≥  50 → scale 8  (ultra 64×64,    entity count ÷64)
pub fn scale_from_zoom(zoom: f32) -> u32 {
    if zoom >= 50.0 { 8 }
    else if zoom >= 25.0 { 4 }
    else if zoom >= 10.0 { 2 }
    else { 1 }
}

/// Select the correct mesh handle for a given chunk scale and LOD.
fn mesh_for_scale(handles: &LandMeshHandles, scale: u32, lod: LandMeshLod) -> Handle<Mesh> {
    match scale {
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

    texture_lookup_cache.clear();
    texture_lookup_cache.reserve(64);
    let mut texels = Vec::with_capacity(TILE_NUM_PER_CHUNK_TOTAL);

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
        texels.push(Rg16u::pack(layer as u16, cell.z, tex_size_bits));
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
        &texels,
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

/// Main system: finds visible land map chunks and ensures their mesh is generated and rendered.
pub fn sys_draw_spawned_land_chunks(
    mut commands: Commands,
    mut cache_r: ResMut<LandTextureCache>,
    mut tile_atlas_r: ResMut<TileAtlas>,
    mut land_mesh_scratch_r: ResMut<LandMeshScratch>,
    mut map_planes_r: ResMut<MapPlanesRes>,
    texmap_2d_r: Res<TexMap2DRes>,
    world_geo_data_r: Res<WorldGeoData>,
    scene_state_data_r: Res<SceneStateData>,
    chunk_q: Query<(Entity, &LCMesh, Option<&Mesh3d>)>,
    visible_chunk_q: Query<(&LCMesh, &Mesh3d)>,
    land_mesh_handles_r: Res<LandMeshHandles>,
    current_lod: Res<LandMeshLod>,
    shared_land_material_r: Res<SharedLandMaterial>,
    cache_settings_r: Res<crate::core::texture_cache::land::cache::LandTextureCacheSettings>,
) {
    // Step 1: Get camera/player state.
    let current_map_id = scene_state_data_r.map_id;
    let map_plane_metadata = world_geo_data_r.maps.get(&current_map_id).unwrap_or_else(|| panic!("Requested metadata for uncached map {current_map_id}"));

    let scratch = &mut *land_mesh_scratch_r;

    scratch.primary_chunks.clear();
    scratch.spawn_targets.clear();
    scratch.blocks_to_draw.clear();
    scratch.blocks_data.clear();
    if scratch.missing_tile_bits.len() != 1024 {
        scratch.missing_tile_bits.resize(1024, 0);
    } else {
        scratch.missing_tile_bits.fill(0);
    }
    scratch.ids.clear();
    scratch.texture_lookup_cache.clear();

    // Step 1: Collect all primary chunks that need meshing into a HashMap.
    // Process only chunks that don't have a mesh yet.
    // Stores (entity, scale) per chunk coordinate.
    let primary_chunks = &mut scratch.primary_chunks;
    for (entity, chunk_data, _) in chunk_q.iter().filter(|(_, _, mesh)| mesh.is_none()) {
        primary_chunks.insert((chunk_data.gx, chunk_data.gy), (entity, chunk_data.scale));
    }

    if primary_chunks.is_empty() {
        return;
    }

    // Step 2: Build the final set of chunks whose data we need to construct.
    // This includes the primary chunks and their immediate non-primary neighbors
    // (to get data for mesh stitching).
    let spawn_targets = &mut scratch.spawn_targets;

    let max_chunk_x = (map_plane_metadata.width / TILE_NUM_PER_CHUNK_DIM) as i32;
    let max_chunk_y = (map_plane_metadata.height / TILE_NUM_PER_CHUNK_DIM) as i32;

    // Iterate through the primary chunks. Add them and all sub-blocks (for super-chunks
    // at scale > 1) to the target list, then add border neighbors for data stitching.
    for (&(gx, gy), &(entity, scale)) in primary_chunks.iter() {
        // For a super-chunk at (gx,gy) covering `scale × scale` base blocks,
        // we need data for every sub-block inside it plus a 1-block border.
        let base_blocks_dim = scale as i32;

        // Insert the primary entity block (the one that gets the mesh).
        spawn_targets.insert(LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x: gx,
            chunk_origin_chunk_units_z: gy,
            chunk_scale: scale,
        });

        // Insert remaining sub-blocks inside this super-chunk (data-only).
        for sx in 0..base_blocks_dim {
            for sz in 0..base_blocks_dim {
                if sx == 0 && sz == 0 { continue; } // Already added as primary.
                let bx = gx as i32 + sx;
                let bz = gy as i32 + sz;
                if bx >= 0 && bx < max_chunk_x && bz >= 0 && bz < max_chunk_y {
                    spawn_targets.insert(LandChunkConstructionData {
                        entity: None,
                        chunk_origin_chunk_units_x: bx as u32,
                        chunk_origin_chunk_units_z: bz as u32,
                        chunk_scale: 1,
                    });
                }
            }
        }

        // 1-block border ring around the super-chunk for edge stitching.
        for edge in -1..=(base_blocks_dim) {
            for &(bx, bz) in &[
                (gx as i32 + edge, gy as i32 - 1),              // top row
                (gx as i32 + edge, gy as i32 + base_blocks_dim), // bottom row
                (gx as i32 - 1,    gy as i32 + edge),            // left col
                (gx as i32 + base_blocks_dim, gy as i32 + edge), // right col
            ] {
                if bx >= 0 && bx < max_chunk_x && bz >= 0 && bz < max_chunk_y {
                    let nc = (bx as u32, bz as u32);
                    if !primary_chunks.contains_key(&nc) {
                        spawn_targets.insert(LandChunkConstructionData {
                            entity: None,
                            chunk_origin_chunk_units_x: nc.0,
                            chunk_origin_chunk_units_z: nc.1,
                            chunk_scale: 1,
                        });
                    }
                }
            }
        }
    }

    // Also include currently visible chunks as data-only targets.
    // This keeps the texture usage hint proportional to the actual render area,
    // reducing the chance of evicting still-visible textures during zoom-out.
    for (chunk_data, _mesh) in visible_chunk_q.iter() {
        if (chunk_data.gx as i32) < max_chunk_x && (chunk_data.gy as i32) < max_chunk_y {
            spawn_targets.insert(LandChunkConstructionData {
                entity: None,
                chunk_origin_chunk_units_x: chunk_data.gx,
                chunk_origin_chunk_units_z: chunk_data.gy,
                chunk_scale: 1,
            });
        }
    }

    // Step 3: Collect the MapBlockRelPos for all target chunks and load them from UO data.
    let blocks_to_draw = &mut scratch.blocks_to_draw;
    blocks_to_draw.extend(spawn_targets.iter()
        .map(|d| MapBlockRelPos {
            x: d.chunk_origin_chunk_units_x,
            y: d.chunk_origin_chunk_units_z,
        }));
    //blocks_to_draw.sort();    // Already done by load_blocks.

    let blocks_data = &mut scratch.blocks_data;
    let missing_tile_bits = &mut scratch.missing_tile_bits;
    {
        // This lock only needed during the block loading from disk/memory.
        let mut uo_data_map_planes_arc = map_planes_r.0.clone();
        let mut uo_data_map_plane = uo_data_map_planes_arc
            .get_mut(&current_map_id)
            .expect("Requested map plane metadata is uncached?");
        let load_blocks_start = Instant::now();
        uo_data_map_plane
            .load_blocks(blocks_to_draw.as_mut_slice())
            .expect("Can't load map blocks");
        let load_blocks_us = load_blocks_start.elapsed().as_micros();
        if load_blocks_us > 1000 {
            console_logger::one(
                None,
                LogSev::Diagnostics,
                LogAbout::Performance,
                &format!("Perf: load_blocks took {} µs for {} blocks.", load_blocks_us, blocks_to_draw.len()),
            );
        }
        for block_coords in blocks_to_draw.iter().copied() {
            let Some(block_ref) = uo_data_map_plane.block(block_coords) else {
                console_logger::one(
                    None,
                    LogSev::Warn,
                    LogAbout::RenderWorldLand,
                    &format!(
                        "Skipping missing map block x={}, y={} (map={}).",
                        block_coords.x, block_coords.y, current_map_id
                    ),
                );
                continue;
            };
            let unique = blocks_data
                .insert(block_coords, block_ref.clone())
                .is_none();
            if !unique {
                panic!("Adding again the same key?");
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
    // Step 4: Aggregate all tile IDs needed for the primary chunks and their neighbors,
    // and perform a batch pre-cache (using MT compression if > 1000).
    {
        scratch.ids.clear();
        for (word_idx, &word) in scratch.missing_tile_bits.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                scratch.ids.push((word_idx * 64 + bit) as u16);
                bits &= bits - 1;
            }
        }

        cache_r.set_visible_texture_usage_hint(&scratch.ids);

        let ids: &[u16] = scratch.ids.as_slice();
        cache_r.precache_textures_parallel(
            &ids,
            texmap_2d_r.0.clone(),
            cache_settings_r.lossy_texture_compression,
        );
    }

    // Step 5: For every chunk that corresponds to a current entity (not filler neighbors), spawn the prebuilt map chunk mesh.
    let build_time_start = Instant::now();
    for chunk_data in spawn_targets.iter() {
        let entity = chunk_data.entity;

        enqueue_chunk_to_atlas_and_preload(
            &mut cache_r,
            &mut tile_atlas_r,
            texmap_2d_r.0.clone(),
            chunk_data,
            &blocks_data,
            &mut scratch.texture_lookup_cache,
            cache_settings_r.lossy_texture_compression,
        );

        if entity.is_none() {
            continue;
        }
        // Paranoid check, shouldn't ever happen.
        if commands.get_entity(entity.unwrap()).is_err() {
            console_logger::one(
                None,
                LogSev::Warn,
                LogAbout::RenderWorldLand,
                "Skipping drawing of invalid/unspawned entity at stage 'sys_draw_spawned_land_chunks'.",
            );
            continue;
        }

        draw_land_chunk(
            &mut commands,
            chunk_data,
            &land_mesh_handles_r,
            *current_lod,
            &shared_land_material_r,
            chunk_data.chunk_scale,
        );
    }
    let build_time: u128 = build_time_start.elapsed().as_micros();
    if build_time > 1000 {
        console_logger::one(
            None,
            LogSev::Diagnostics,
            LogAbout::Performance,
            &format!("Perf: chunk rendering preloader took {build_time} µs for {} chunks.", spawn_targets.len()),
        );
    }
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


