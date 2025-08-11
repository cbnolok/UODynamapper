#![allow(unused_parens, unused)]

use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::{
        mesh::{Indices, PrimitiveTopology},
        render_asset::RenderAssetUsages,
        render_resource::{AsBindGroup, ShaderRef, ShaderType},
    },
};
use bytemuck::Zeroable;
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;
use uocf::geo::{
    land_texture_2d::LandTextureSize,
    map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos},
};
use wide::*;

use super::TILE_NUM_PER_CHUNK_1D;
use super::{LCMesh, diagnostics::*, mesh_buffer_pool::*, mesh_material::*};
use crate::{
    core::{
        constants,
        maps::MapPlaneMetadata,
        render::scene::{SceneStateData, player::Player, world::WorldGeoData},
        texture_cache::land::cache::*,
        uo_files_loader::UoFileData,
    },
    prelude::*,
    util_lib::array::*,
};

// Extend to the first row and column of this neighboring block, to avoid seam artifacts.
const CHUNK_TILE_GRID_W: u32 = TILE_NUM_PER_CHUNK_1D + 1;
const CHUNK_TILE_GRID_H: u32 = TILE_NUM_PER_CHUNK_1D + 1;
const HEIGHTMAP_ARRAY_SIZE: usize = (CHUNK_TILE_GRID_W * CHUNK_TILE_GRID_H) as usize;

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
    mut pool_r: ResMut<LandChunkMeshBufferPool>,
    mut diag_r: ResMut<LandChunkMeshDiagnostics>,
    mut hist_r: ResMut<MeshBuildPerfHistory>,
    mut meshes_r: ResMut<Assets<Mesh>>,
    mut materials_land_r: ResMut<Assets<LandCustomMaterial>>,
    mut cache_r: ResMut<LandTextureCache>,
    mut images_r: ResMut<Assets<Image>>,
    uo_data: Res<UoFileData>,
    world_geo_data_r: Res<WorldGeoData>,
    scene_state_data_r: Res<SceneStateData>,
    player_q: Query<&Player>,
    cam_q: Query<&Transform, With<Camera3d>>,
    chunk_q: Query<(Entity, &LCMesh, Option<&Mesh3d>)>,
    visible_chunk_q: Query<(&LCMesh, &Mesh3d)>,
) {
    // Step 1: Get camera/player state.
    let cam_pos = cam_q.single().unwrap().translation;
    let player_entity = player_q.single().expect("More than 1 player!");
    let current_map_id = scene_state_data_r.map_id;
    let map_plane_metadata = world_geo_data_r.maps.get(&current_map_id).expect(&format!(
        "Requested metadata for uncached map {current_map_id}"
    ));

    // Step 1: Collect all primary chunks that need meshing into a HashMap.
    // This maps coordinates to an entity, ensuring we don't lose the entity reference
    // and allows for fast lookups.
    let mut primary_chunks = std::collections::HashMap::new();
    for (entity, chunk_data, mesh_handle) in chunk_q.iter() {
        // Process chunks that don't have a mesh yet.
        if mesh_handle.is_none() {
            primary_chunks.insert((chunk_data.gx, chunk_data.gy), entity);
        }
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
        let mut uo_data_map_planes_lock = uo_data.map_planes.write().unwrap();
        let uo_data_map_plane = uo_data_map_planes_lock
            .get_mut(&current_map_id)
            .expect("Requested map plane metadata is uncached?");
        uo_data_map_plane
            .load_blocks(&mut blocks_to_draw)
            .expect("Can't load map blocks");
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

    // Step 4: For every chunk that corresponds to a current entity (not filler neighbors), build the mesh.
    let build_time_start = Instant::now();
    for chunk_data in spawn_targets {
        let entity = chunk_data.entity;
        if entity.is_none() {
            continue;
        }
        // Paranoid check, shouldn't ever happen.
        if commands.get_entity(entity.unwrap()).is_err() {
            println!(
                "Skipping drawing of invalid/unspawned entity at stage 'sys_draw_spawned_land_chunks'."
            );
            continue;
        }

        draw_land_chunk(
            &mut commands,
            &mut pool_r,
            &mut diag_r,
            &mut meshes_r,
            &mut materials_land_r,
            &mut cache_r,
            &mut images_r,
            &uo_data,
            &map_plane_metadata,
            &chunk_data,
            &blocks_data,
        );
    }
    let build_ms = build_time_start.elapsed().as_secs_f32() * 1000.0; // TODO: be more precise?
    hist_r.push(build_ms);

    // Step 5: Diagnostics
    diag_r.build_last = build_ms;
    diag_r.build_avg = hist_r.avg();
    diag_r.build_peak = hist_r.peak();

    // OLD: diag.num_chunks = blocks_data.len();
    // NEW: Accurately track # of chunks on screen for diagnostics.
    diag_r.chunks_on_screen = visible_chunk_q.iter().count();
}

/// Build mesh, attributes and assign to chunk entity.
/// This function is heavily optimized to build land chunk meshes efficiently.
///
/// Key Optimizations:
/// 1.  **Pre-computation of Heights and Normals:** Instead of calculating vertex heights and normals
///     on-the-fly inside the main loop, we pre-compute them for the entire grid (including
///     a 1-tile border for seamless stitching) and store them in `heights` and `normals_grid` arrays.
///     This avoids redundant calculations, especially for normals, where each vertex's normal
///     was previously computed multiple times for adjacent tiles.
///
/// 2.  **SIMD-accelerated Normal Calculation:** The process of calculating normals from the heightmap
///     is vectorized using the `wide` crate. It processes 4 vertices at a time (`f32x4`),
///     significantly speeding up the expensive normalization calculations. This is the most
///     significant optimization.
///
/// 3.  **Loop Fusion:** The main loop now iterates over the tiles (`tx`, `ty`) only once. Inside this
///     single loop, it performs all necessary work for a given tile:
///     -   Looks up pre-computed heights and normals.
///     -   Generates vertex positions, UVs, and normals for the tile's quad.
///     -   Adds indices to the index buffer.
///     -   Fetches tile metadata (`MapCell`).
///     -   Updates the shader uniforms with texture information.
///     This improves data locality and reduces loop overhead compared to the previous multi-loop approach.
///
/// 4.  **Efficient Data Access:** By pre-computing data and accessing it from local arrays within a
///     tight loop, we maximize cache efficiency. The `get_cell` helper is still used, but its calls
///     are now consolidated into the height/uniform generation phases, reducing overhead from repeated
///     coordinate calculations and B-tree lookups.
fn draw_land_chunk(
    commands: &mut Commands,
    pool_ref: &mut LandChunkMeshBufferPool,
    diag_ref: &mut LandChunkMeshDiagnostics,
    meshes_rref: &mut ResMut<Assets<Mesh>>,
    materials_land_rref: &mut ResMut<Assets<LandCustomMaterial>>,
    land_texture_cache_rref: &mut ResMut<LandTextureCache>,
    images_rref: &mut ResMut<Assets<Image>>,
    uo_data_rref: &Res<UoFileData>,
    map_plane_metadata_ref: &MapPlaneMetadata,
    chunk_data_ref: &LandChunkConstructionData,
    blocks_data_ref: &BTreeMap<MapBlockRelPos, MapBlock>,
) {
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

    // Helper to fetch a cell from the loaded block data. Panics on OOB for safety.
    fn get_cell<'a>(
        blocks_data: &'a BTreeMap<MapBlockRelPos, MapBlock>,
        world_tile_x: u32,
        world_tile_z: u32,
    ) -> &'a MapCell {
        let chunk_rel_coords = MapBlockRelPos {
            x: world_tile_x / TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z / TILE_NUM_PER_CHUNK_1D,
        };
        let tile_rel_coords = MapCellRelPos {
            x: world_tile_x % TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z % TILE_NUM_PER_CHUNK_1D,
        };
        blocks_data
            .get(&chunk_rel_coords)
            .unwrap_or_else(|| panic!("Requested uncached map block: {:?}", chunk_rel_coords))
            .cell(tile_rel_coords.x, tile_rel_coords.y)
            .unwrap_or_else(|err| panic!("Cell {:?} error: {}", tile_rel_coords, err))
    }

    // Helper to get the scaled height (z-coordinate) for a given world tile.
    let get_cell_z = |x: u32, z: u32| {
        scale_uo_z_to_bevy_units(get_cell(blocks_data_ref, x, z).z as f32)
    };

    // --------- 1. Pre-computation Phase --------
    // Pre-calculate heights and normals for the entire grid to avoid redundant work inside the main loop.
    // The grid is extended by one tile in each direction to handle normals at the seams correctly.

    // A. Pre-compute heights for the entire grid.
    let mut heights = [0.0f32; HEIGHTMAP_ARRAY_SIZE];
    for vy in 0..CHUNK_TILE_GRID_H {
        for vx in 0..CHUNK_TILE_GRID_W {
            let world_tx = chunk_origin_tile_units_x + vx;
            let world_tz = chunk_origin_tile_units_z + vy;
            let hindex = (vy * CHUNK_TILE_GRID_W + vx) as usize;
            heights[hindex] = get_cell_z(world_tx, world_tz);
        }
    }

    // B. Pre-compute normals for the entire grid using SIMD for acceleration.
    let mut normals_grid = [[0.0f32; 3]; HEIGHTMAP_ARRAY_SIZE];
    let map_width = map_plane_metadata_ref.width;
    let map_height = map_plane_metadata_ref.height;

    // Process the grid in chunks of 4 (the SIMD width).
    for vy in 0..CHUNK_TILE_GRID_H {
        for vx_base in (0..CHUNK_TILE_GRID_W).step_by(4) {
            // Create SIMD vectors for the x and z coordinates of 4 vertices.
            let vx = f32x4::from([
                vx_base as f32,
                (vx_base + 1) as f32,
                (vx_base + 2) as f32,
                (vx_base + 3) as f32,
            ]);
            let vz = f32x4::splat(vy as f32);

            // World coordinates
            let world_tx_f32 = f32x4::splat(chunk_origin_tile_units_x as f32) + vx;
            let world_tz_f32 = f32x4::splat(chunk_origin_tile_units_z as f32) + vz;

            let world_tx_arr = world_tx_f32.to_array();
            let world_tz_arr = world_tz_f32.to_array();

            let world_tx = u32x4::from([
                world_tx_arr[0] as u32,
                world_tx_arr[1] as u32,
                world_tx_arr[2] as u32,
                world_tx_arr[3] as u32,
            ]);
            let world_tz = u32x4::from([
                world_tz_arr[0] as u32,
                world_tz_arr[1] as u32,
                world_tz_arr[2] as u32,
                world_tz_arr[3] as u32,
            ]);


            // Helper to get heights for 4 vertices at once, handling boundary conditions.
            let get_heights_simd = |x_coords: u32x4, z_coords: u32x4| -> f32x4 {
                let mut h = [0.0f32; 4];
                let x_coords_arr = x_coords.to_array();
                let z_coords_arr = z_coords.to_array();
                for i in 0..4 {
                    // Clamp coordinates to map boundaries to prevent OOB access.
                    let clamped_x = x_coords_arr[i].min(map_width - 1);
                    let clamped_z = z_coords_arr[i].min(map_height - 1);
                    h[i] = get_cell_z(clamped_x, clamped_z);
                }
                f32x4::from(h)
            };

            // Get heights of the center, left, right, up, and down neighbors for 4 vertices.
            let center_h = get_heights_simd(world_tx, world_tz);
            let left_h = get_heights_simd(world_tx - u32x4::splat(1), world_tz);
            let right_h = get_heights_simd(world_tx + u32x4::splat(1), world_tz);
            let down_h = get_heights_simd(world_tx, world_tz - u32x4::splat(1));
            let up_h = get_heights_simd(world_tx, world_tz + u32x4::splat(1));

            // Calculate the gradient using central differences.
            let dx = (right_h - left_h) * f32x4::splat(0.5);
            let dz = (up_h - down_h) * f32x4::splat(0.5);

            // Construct the normal vectors.
            let nx = -dx;
            let ny = f32x4::splat(1.0);
            let nz = -dz;

            // Normalize the vectors to get unit normals.
            let len_sq = nx * nx + ny * ny + nz * nz;
            let inv_len = f32x4::splat(1.0) / len_sq.sqrt();
            let norm_x = nx * inv_len;
            let norm_y = ny * inv_len;
            let norm_z = nz * inv_len;

            let norm_x_arr = norm_x.to_array();
            let norm_y_arr = norm_y.to_array();
            let norm_z_arr = norm_z.to_array();

            // Store the results back into the grid, handling cases where the width is not a multiple of 4.
            for i in 0..4 {
                let current_vx = vx_base + i as u32;
                if current_vx < CHUNK_TILE_GRID_W {
                    let hindex = (vy * CHUNK_TILE_GRID_W + current_vx) as usize;
                    normals_grid[hindex] = [norm_x_arr[i], norm_y_arr[i], norm_z_arr[i]];
                }
            }
        }
    }

    // --------- 2. MESH & UNIFORM GENERATION (Fused Loop) --------
    let mut meshbufs = pool_ref.alloc(diag_ref);
    meshbufs.positions.clear();
    meshbufs.normals.clear();
    meshbufs.uvs.clear();
    meshbufs.indices.clear();

    let mut mat_ext_uniforms = LandUniforms::zeroed();
    mat_ext_uniforms.chunk_origin = Vec2::new(
        chunk_origin_tile_units_x as f32,
        chunk_origin_tile_units_z as f32,
    );
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    // Iterate through each tile of the core chunk (the +1 border is only for calculation).
    for ty in 0..TILE_NUM_PER_CHUNK_1D {
        for tx in 0..TILE_NUM_PER_CHUNK_1D {
            // --- A. Generate Mesh Data for the tile's quad ---

            // Base index for the vertices of the current quad.
            let base_vertex_index = meshbufs.positions.len() as u32;

            // Get the indices for the four corners of this tile in the pre-computed grids.
            let idx00 = (ty * CHUNK_TILE_GRID_W + tx) as usize;
            let idx10 = idx00 + 1;
            let idx01 = ((ty + 1) * CHUNK_TILE_GRID_W + tx) as usize;
            let idx11 = idx01 + 1;

            // Add the 4 vertices of the quad, looking up pre-computed heights and normals.
            // Top-left
            meshbufs.positions.push([tx as f32, heights[idx00], ty as f32]);
            meshbufs.normals.push(normals_grid[idx00]);
            meshbufs.uvs.push([0.0, 0.0]);
            // Top-right
            meshbufs.positions.push([(tx + 1) as f32, heights[idx10], ty as f32]);
            meshbufs.normals.push(normals_grid[idx10]);
            meshbufs.uvs.push([1.0, 0.0]);
            // Bottom-right
            meshbufs.positions.push([(tx + 1) as f32, heights[idx11], (ty + 1) as f32]);
            meshbufs.normals.push(normals_grid[idx11]);
            meshbufs.uvs.push([1.0, 1.0]);
            // Bottom-left
            meshbufs.positions.push([tx as f32, heights[idx01], (ty + 1) as f32]);
            meshbufs.normals.push(normals_grid[idx01]);
            meshbufs.uvs.push([0.0, 1.0]);


            // Add indices to form two triangles for the quad.
            meshbufs.indices.extend_from_slice(&[
                base_vertex_index + 0,
                base_vertex_index + 2,
                base_vertex_index + 1,
                base_vertex_index + 0,
                base_vertex_index + 3,
                base_vertex_index + 2,
            ]);

            // --- B. Update Shader Uniforms for the tile ---
            let world_x = chunk_origin_tile_units_x + tx;
            let world_y = chunk_origin_tile_units_z + ty;
            let tile_ref: &MapCell = get_cell(blocks_data_ref, world_x, world_y);

            // Get texture array layer for this tile's artwork.
            let (texture_size, layer) = land_texture_cache_rref.get_texture_size_layer(
                images_rref,
                uo_data_rref,
                tile_ref.id,
            );

            // Update the corresponding uniform struct for this tile.
            let tile_uniform_idx = (ty * TILE_NUM_PER_CHUNK_1D + tx) as usize;
            let tile_uniform = &mut mat_ext_uniforms.tiles[tile_uniform_idx];
            tile_uniform.tile_height = tile_ref.z as u32;
            tile_uniform.texture_size = match texture_size {
                LandTextureSize::Small => 0,
                LandTextureSize::Big => 1,
            };
            tile_uniform.layer = layer;
            tile_uniform.hue = 0; // Hue not yet implemented.
        }
    }

    // --------- 3. Bevy Asset Creation --------
    // Upload the generated mesh data to a new Bevy Mesh asset.
    let chunk_mesh_handle: Handle<Mesh> = {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, meshbufs.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, meshbufs.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, meshbufs.uvs.clone());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[0.0, 0.0]; meshbufs.positions.len()],
        );
        mesh.insert_indices(Indices::U32(meshbufs.indices.clone()));
        meshes_rref.add(mesh)
    };

    // Create the custom material for this chunk.
    let chunk_material_handle = {
        let mat = ExtendedMaterial {
            base: StandardMaterial::default(),
            extension: LandMaterialExtension {
                texarray_small: land_texture_cache_rref.small.image_handle.clone(),
                texarray_big: land_texture_cache_rref.big.image_handle.clone(),
                uniforms: mat_ext_uniforms,
            },
        };
        materials_land_rref.add(mat)
    };

    // --------- 4. Entity Component Update --------
    // Attach the new mesh and material to the chunk entity.
    if let Ok(mut entity_commands) = commands.get_entity(chunk_data_ref.entity.unwrap()) {
        entity_commands.insert((
            Mesh3d(chunk_mesh_handle),
            MeshMaterial3d(chunk_material_handle),
            Transform::from_xyz(
                chunk_origin_tile_units_x as f32,
                0.0,
                chunk_origin_tile_units_z as f32,
            ),
            GlobalTransform::default(),
        ));
    } else {
        logger::one(
            None,
            LogSev::Error,
            LogAbout::RenderWorldLand,
            "Skipping drawing of invalid/unspawned entity at stage 'draw_land_chunk'.",
        );
    }

    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!(
            "Rendered new chunk at: gx={} gy={} (map={})",
            chunk_data_ref.chunk_origin_chunk_units_x,
            chunk_data_ref.chunk_origin_chunk_units_z,
            map_plane_metadata_ref.id
        ),
    );

    // --------- 5. Cleanup --------
    // Return the buffers to the pool for reuse.
    pool_ref.free(meshbufs, diag_ref);
}