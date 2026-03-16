#![allow(unused_parens, unused)]

use bevy::{
    asset::RenderAssetUsages, camera::visibility::NoFrustumCulling, mesh::{Indices, MeshVertexAttribute}, pbr::{ExtendedMaterial, MaterialExtension}, prelude::*, render::render_resource::{AsBindGroup, PrimitiveTopology, ShaderType}, shader::ShaderRef
};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bytemuck::Zeroable;
use std::time::Instant;
use std::{
    collections::{BTreeMap, HashSet},
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
pub struct LandMeshHandle(pub Handle<Mesh>);

use crate::core::render::scene::world::land::tile_atlas::{TileAtlas, Rg16u};

#[derive(Resource)]
pub struct SharedLandMaterial(pub Handle<LandCustomMeshMaterial>);

/// Enqueues the 8x8 tile data for this chunk into the TileAtlas, and preloads the textures.
fn enqueue_chunk_to_atlas_and_preload(
    texture_cache: &mut ResMut<LandTextureCache>,
    tile_atlas: &mut ResMut<TileAtlas>,
    texmap_2d_r: Arc<TexMap2D>,
    chunk_data_ref: &LandChunkConstructionData,
    blocks_data_map: &BTreeMap<MapBlockRelPos, MapBlock>,
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
    let block = blocks_data_map.get(&chunk_rel_coords).unwrap();

    let mut unique_tile_ids = HashSet::new();
    let mut texels = Vec::with_capacity(TILE_NUM_PER_CHUNK_TOTAL);

    for tz in 0..TILE_NUM_PER_CHUNK_DIM {
        for tx in 0..TILE_NUM_PER_CHUNK_DIM {
            let cell = block.cell(tx, tz).unwrap();
            unique_tile_ids.insert(cell.id);

            let (texture_size, layer) = texture_cache.get_texture_size_layer(
                texmap_2d_r.clone(),
                cell.id,
                lossy_compression,
            );

            let tex_size_bits = match texture_size {
                LandTextureSize::Small => 0,
                LandTextureSize::Big => 1,
            };

            // Use 'layer' instead of 'cell.id' because that's what the shader needs to sample the 2DArray!
            texels.push(Rg16u::pack(layer as u16, cell.z, tex_size_bits));
        }
    }


    let page_w = tile_atlas.params.page_texels.x;
    let page_h = tile_atlas.params.page_texels.y;
    assert!(
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
}

/// Main system: finds visible land map chunks and ensures their mesh is generated and rendered.
pub fn sys_draw_spawned_land_chunks(
    mut commands: Commands,
    mut meshes_r: ResMut<Assets<Mesh>>,
    mut cache_r: ResMut<LandTextureCache>,
    mut tile_atlas_r: ResMut<TileAtlas>,
    mut map_planes_r: ResMut<MapPlanesRes>,
    texmap_2d_r: Res<TexMap2DRes>,
    world_geo_data_r: Res<WorldGeoData>,
    scene_state_data_r: Res<SceneStateData>,
    player_q: Query<&Player>,
    cam_q: Query<&Transform, With<Camera3d>>,
    chunk_q: Query<(Entity, &LCMesh, Option<&Mesh3d>)>,
    visible_chunk_q: Query<(&LCMesh, &Mesh3d)>,
    land_mesh_handle_r: Res<LandMeshHandle>,
    shared_land_material_r: Res<SharedLandMaterial>,
    cache_settings_r: Res<crate::core::texture_cache::land::cache::LandTextureCacheSettings>,
) {
    // Step 1: Get camera/player state.
    let cam_pos = cam_q.single().unwrap().translation;
    let player_entity = player_q.single().expect("More than 1 player!");
    let current_map_id = scene_state_data_r.map_id;
    let map_plane_metadata = world_geo_data_r.maps.get(&current_map_id).unwrap_or_else(|| panic!("Requested metadata for uncached map {current_map_id}"));

    // Step 1: Collect all primary chunks that need meshing into a HashMap.
    // Process only chunks that don't have a mesh yet.
    let mut primary_chunks = std::collections::HashMap::new();
    for (entity, chunk_data, _) in chunk_q.iter().filter(|(_, _, mesh)| mesh.is_none()) {
        primary_chunks.insert((chunk_data.gx, chunk_data.gy), entity);
    }

    if primary_chunks.is_empty() {
        return;
    }

    // Step 2: Build the final set of chunks whose data we need to construct.
    // This includes the primary chunks and their immediate non-primary neighbors
    // (to get data for mesh stitching).
    let mut spawn_targets = HashSet::<LandChunkConstructionData>::new();

    #[rustfmt::skip]
    const NEIGHBOR_OFFSETS: &[(i32, i32)] = &[
        (-1, -1), (0, -1), (1, -1),
        (-1,  0),          (1,  0), // The primary chunk (0,0) is handled separately.
        (-1,  1), (0,  1), (1,  1),
    ];

    // Iterate through the primary chunks. Add them to the target list,
    // then add any neighbors that are not already primary chunks themselves.
    for (&(gx, gy), &entity) in primary_chunks.iter() {
        // Add the primary chunk itself. Its entity is guaranteed to be Some(entity).
        spawn_targets.insert(LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x: gx,
            chunk_origin_chunk_units_z: gy,
        });

        // Add its valid neighbors that ARE NOT already primary chunks.
        // This ensures we get their data for seamless mesh generation without
        // overwriting a primary chunk's entity reference.
        for (dx, dy) in NEIGHBOR_OFFSETS {
            let nx = gx as i32 + dx;
            let ny = gy as i32 + dy;

            // Ensure the neighbor is within map boundaries.
            if nx >= 0
                && nx < map_plane_metadata.width as i32
                && ny >= 0
                && ny < map_plane_metadata.height as i32
            {
                let neighbor_coords = (nx as u32, ny as u32);

                // If the neighbor is not a primary chunk, we need its data for the mesh.
                // Since `spawn_targets` is a HashSet, duplicate inserts of the same
                // neighbor from different primary chunks are handled automatically.
                if !primary_chunks.contains_key(&neighbor_coords) {
                    spawn_targets.insert(LandChunkConstructionData {
                        entity: None, // It's just a neighbor, not a spawned entity.
                        chunk_origin_chunk_units_x: neighbor_coords.0,
                        chunk_origin_chunk_units_z: neighbor_coords.1,
                    });
                }
            }
        }
    }

    // Step 3: Collect the MapBlockRelPos for all target chunks and load them from UO data.
    let mut blocks_to_draw: Vec<MapBlockRelPos> = spawn_targets
        .iter()
        .map(|d| MapBlockRelPos {
            x: d.chunk_origin_chunk_units_x,
            y: d.chunk_origin_chunk_units_z,
        })
        .collect();
    //blocks_to_draw.sort();    // Already done by load_blocks.

    let mut blocks_data = BTreeMap::<MapBlockRelPos, MapBlock>::new();
    {
        // This lock only needed during the block loading from disk/memory.
        let mut uo_data_map_planes_arc = map_planes_r.0.clone();
        let mut uo_data_map_plane = uo_data_map_planes_arc
            .get_mut(&current_map_id)
            .expect("Requested map plane metadata is uncached?");
        let load_blocks_start = Instant::now();
        uo_data_map_plane
            .load_blocks(&mut blocks_to_draw)
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
        for block_coords in blocks_to_draw {
            let block_ref = uo_data_map_plane
                .block(block_coords)
                .expect("Requested map block is uncached?");
            let unique = blocks_data
                .insert(block_coords, block_ref.clone())
                .is_none();
            if !unique {
                panic!("Adding again the same key?");
            }
        }
    }
    // Step 4: Aggregate all tile IDs needed for the primary chunks and their neighbors,
    // and perform a batch pre-cache (using MT compression if > 1000).
    {
        let mut missing_tile_ids = HashSet::new();
        for chunk_data in &spawn_targets {
            let chunk_rel_coords = MapBlockRelPos {
                x: chunk_data.chunk_origin_chunk_units_x,
                y: chunk_data.chunk_origin_chunk_units_z,
            };
            if let Some(block) = blocks_data.get(&chunk_rel_coords) {
                for tz in 0..8 {
                    for tx in 0..8 {
                        if let Ok(cell) = block.cell(tx, tz) {
                            missing_tile_ids.insert(cell.id);
                        }
                    }
                }
            }
        }

        let ids: Vec<u16> = missing_tile_ids.into_iter().collect();
        cache_r.precache_textures_parallel(
            &ids,
            texmap_2d_r.0.clone(),
            cache_settings_r.lossy_texture_compression,
        );
    }

    // Step 5: For every chunk that corresponds to a current entity (not filler neighbors), spawn the prebuilt map chunk mesh.
    let build_time_start = Instant::now();
    for chunk_data in &spawn_targets {
        let entity = chunk_data.entity;

        enqueue_chunk_to_atlas_and_preload(
            &mut cache_r,
            &mut tile_atlas_r,
            texmap_2d_r.0.clone(),
            chunk_data,
            &blocks_data,
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
            &land_mesh_handle_r,
            &shared_land_material_r,
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
    land_mesh_handle_r: &Res<LandMeshHandle>,
    shared_land_material_r: &Res<SharedLandMaterial>,
) {
    // Use the mesh prebuilt in setup_land_mesh.
    let chunk_mesh_handle: Handle<Mesh> = land_mesh_handle_r.0.clone();
    let chunk_material_handle: Handle<LandCustomMeshMaterial> = shared_land_material_r.0.clone();

    // Compute chunk origin (in tile units) for the transform.
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_DIM;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_DIM;

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
            // TODO: Explain what is AABB and what's frustum culling!
            // Why? In Ultima Online, land meshes are flat grids (y=0) on the CPU side.
            // However, our vertex shader displaces these vertices vertically (up to ~12.8m).
            //
            // If we let Bevy automatically compute the AABB from the flat mesh vertices,
            // the culling system remains unaware of the height of mountains/valleys.
            // Result: chunks disappear ("pop") as soon as their flat base leaves the
            // camera view, even if their peaks should still be visible.
            //
            // Solution:
            // 1. Add `NoAutoAabb` to stop Bevy from overwriting our custom bounds.
            // 2. Insert a manual `Aabb` that covers the full possible displacement.
            //
            // Tailored Bounds Calculation:
            // - Mesh Size: 8x8 tiles = 9x9 vertices -> local XZ spans [0.0, 8.0].
            // - Height Range: UO uses -128 to +127, scaled by 0.1 in shader -> [-12.8, 12.7].
            // - Padding: We add ~1.0m padding in XZ for stitching and enough Y margin
            //   to account for any projection distortions.
            NoAutoAabb,
            Aabb::from_min_max(Vec3::new(-1.0, -20.0, -1.0), Vec3::new(9.0, 20.0, 9.0)),
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


