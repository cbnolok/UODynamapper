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

    // Helper to fetch cell from block/block coordinates. Always panics on OOB for safety.
    //#[track_caller]
    fn get_cell<'a>(
        blocks_data: &'a BTreeMap<MapBlockRelPos, MapBlock>,
        world_tile_x: usize,
        world_tile_z: usize,
    ) -> &'a MapCell {
        let chunk_rel_coords = MapBlockRelPos {
            x: world_tile_x as u32 / TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z as u32 / TILE_NUM_PER_CHUNK_1D,
        };
        let tile_rel_coords = MapCellRelPos {
            x: world_tile_x as u32 % TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z as u32 % TILE_NUM_PER_CHUNK_1D,
        };
        blocks_data
            .get(&chunk_rel_coords)
            .expect(&format!(
                "Requested uncached map block? {chunk_rel_coords:?}."
            ))
            .cell(tile_rel_coords.x, tile_rel_coords.y)
            .map_err(|err_str| format!("Cell {tile_rel_coords:?} error: {err_str}."))
            .unwrap()
    }
    let get_cell_z = |x: u32, z: u32| {
        scale_uo_z_to_bevy_units(get_cell(blocks_data_ref, x as usize, z as usize).z as f32)
    };

    // --------- Setup chunk-specific uniforms ---------
    // For shading, lighting, texturing. Synchronized with shader struct.
    let mut mat_ext_uniforms = LandUniforms::zeroed();
    mat_ext_uniforms.chunk_origin = Vec2::new(
        chunk_origin_tile_units_x as f32,
        chunk_origin_tile_units_z as f32,
    );
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    // --------- MESH DATA GENERATION (fixed UVs, per-tile quads) --------
    let mut meshbufs = pool_ref.alloc(diag_ref);
    meshbufs.positions.clear();
    meshbufs.normals.clear();
    meshbufs.uvs.clear();
    meshbufs.indices.clear();

    // Also record grid of heights for normal calculation
    let mut heights = [0.0f32; HEIGHTMAP_ARRAY_SIZE];
    for vy in 0..CHUNK_TILE_GRID_W {
        for vx in 0..CHUNK_TILE_GRID_H {
            let world_tx = chunk_origin_tile_units_x + vx;
            let world_tz = chunk_origin_tile_units_z + vy;

            let ty = get_cell_z(world_tx, world_tz);
            let hindex =
                get_1d_array_index_as_2d(CHUNK_TILE_GRID_W as usize, vx as usize, vy as usize);
            heights[hindex] = ty;
            /*// Debug
            let first_cell_id = get_cell(blocks_data_ref, world_tx as usize, world_tz as usize).id;
            println!(
                "Block at gx={}, gy={} has first cell at tx={}, ty={} with ID: {}",
                chunk_data_ref.chunk_origin_chunk_units_x,
                chunk_data_ref.chunk_origin_chunk_units_z,
                world_tx,
                world_tz,
                first_cell_id
            );*/
        }
    }

    //for ty in 0..TILE_NUM_PER_CHUNK_1D {
    //    for tx in 0..TILE_NUM_PER_CHUNK_1D {
    for ty in 0..(CHUNK_TILE_GRID_H - 1) {
        for tx in 0..(CHUNK_TILE_GRID_W - 1) {
            // Four corners per tile quad
            let vx = tx;
            let vy = ty;
            let vx1 = tx + 1;
            let vy1 = ty + 1;

            let world_tx0 = chunk_origin_tile_units_x + vx;
            let world_tz0 = chunk_origin_tile_units_z + vy;
            let world_tx1 = chunk_origin_tile_units_x + vx1;
            let world_tz1 = chunk_origin_tile_units_z + vy1;

            // Heights at four corners
            let h00 = heights[((vy * CHUNK_TILE_GRID_W) + vx) as usize];
            let h10 = heights[((vy * CHUNK_TILE_GRID_W) + vx1) as usize];
            let h11 = heights[((vy1 * CHUNK_TILE_GRID_W) + vx1) as usize];
            let h01 = heights[((vy1 * CHUNK_TILE_GRID_W) + vx) as usize];

            // Normals at four corners
            let get_norm = |wx: u32, wz: u32| {
                let center = get_cell_z(wx, wz);
                let left = if wx > 0 {
                    get_cell_z(wx - 1, wz)
                } else {
                    center
                };
                let right = if wx + 1 < map_plane_metadata_ref.width {
                    get_cell_z(wx + 1, wz)
                } else {
                    center
                };
                let down = if wz > 0 {
                    get_cell_z(wx, wz - 1)
                } else {
                    center
                };
                let up = if wz + 1 < map_plane_metadata_ref.height {
                    get_cell_z(wx, wz + 1)
                } else {
                    center
                };
                let dx = (right - left) * 0.5;
                let dz = (up - down) * 0.5;
                Vec3::new(-dx, 1.0, -dz).normalize().to_array()
            };

            let base = meshbufs.positions.len() as u32;

            // Top-left (0,0)
            meshbufs.positions.push([vx as f32, h00, vy as f32]);
            meshbufs.uvs.push([0.0, 0.0]);
            meshbufs.normals.push(get_norm(world_tx0, world_tz0));
            // Top-right (1,0)
            meshbufs.positions.push([vx1 as f32, h10, vy as f32]);
            meshbufs.uvs.push([1.0, 0.0]);
            meshbufs.normals.push(get_norm(world_tx1, world_tz0));
            // Bottom-right (1,1)
            meshbufs.positions.push([vx1 as f32, h11, vy1 as f32]);
            meshbufs.uvs.push([1.0, 1.0]);
            meshbufs.normals.push(get_norm(world_tx1, world_tz1));
            // Bottom-left (0,1)
            meshbufs.positions.push([vx as f32, h01, vy1 as f32]);
            meshbufs.uvs.push([0.0, 1.0]);
            meshbufs.normals.push(get_norm(world_tx0, world_tz1));

            // Two triangles for this quad
            meshbufs.indices.extend_from_slice(&[
                base + 0,
                base + 2,
                base + 1,
                base + 0,
                base + 3,
                base + 2,
            ]);
        }
    }

    // --------- Shader uniforms for texture layer (by tile) --------
    mat_ext_uniforms.chunk_origin = Vec2::new(
        chunk_origin_tile_units_x as f32,
        chunk_origin_tile_units_z as f32,
    );
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    for ty in 0..TILE_NUM_PER_CHUNK_1D as usize {
        for tx in 0..TILE_NUM_PER_CHUNK_1D as usize {
            let world_x = chunk_origin_tile_units_x as usize + tx;
            let world_y = chunk_origin_tile_units_z as usize + ty;
            let tile_ref: &MapCell = get_cell(blocks_data_ref, world_x, world_y);

            // Each quad (tile) uses two triangles (6 indices).
            // Get the layer (index) of the texture array housing this texture (map tile art).
            let (texture_size, layer) = land_texture_cache_rref.get_texture_size_layer(
                images_rref,
                uo_data_rref,
                tile_ref.id,
            );

            // Update values of the uniform buffer. This is per-chunk data (per mesh draw call).
            // We need to store the data not in a simple vector, but in a vector of 4D vectors, in order to meet
            // the 16-byte field alignment requisite.
            // We need to access the right layer, so start by picking the right 4D array:
            // Then get the correct one among the 4 elements.

            let tile_struct_elem_idx: usize =
                get_1d_array_index_as_2d(TILE_NUM_PER_CHUNK_1D as usize, tx, ty);
            let tile_uniform: &mut TileUniform = &mut mat_ext_uniforms.tiles[tile_struct_elem_idx];
            tile_uniform.tile_height = tile_ref.z as u32;
            tile_uniform.texture_size = match texture_size {
                LandTextureSize::Small => 0,
                LandTextureSize::Big => 1,
                //_ => unreachable!(),
            };
            tile_uniform.layer = layer;
            tile_uniform.hue = 0;
        }
    }

    // Step 4: Upload vertex data to Bevy's Mesh asset builder.
    let chunk_mesh_handle: Handle<Mesh> = {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, meshbufs.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, meshbufs.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, meshbufs.uvs.clone());
        // Add fake data for UV_1 as before
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[0.0, 0.0]; meshbufs.positions.len()],
        );
        mesh.insert_indices(Indices::U32(meshbufs.indices.clone()));
        meshes_rref.add(mesh)
    };

    let chunk_material_handle = {
        let mat = ExtendedMaterial {
            base: StandardMaterial {
                ..Default::default()
            },
            extension: LandMaterialExtension {
                texarray_small: land_texture_cache_rref.small.image_handle.clone(),
                texarray_big: land_texture_cache_rref.big.image_handle.clone(),
                uniforms: mat_ext_uniforms,
            },
        };
        materials_land_rref.add(mat)
    };

    // Step 5: Attach or update the Bevy entity with mesh/material/transform.
    let entity = chunk_data_ref
        .entity
        .expect("Entity cannot be None at this stage.");
    let entity_commands = commands.get_entity(entity);

    if entity_commands.is_err() {
        logger::one(
            None,
            LogSev::Error,
            LogAbout::RenderWorldLand,
            "Skipping drawing of invalid/unspawned entity at stage 'draw_land_chunk'.",
        );
    } else {
        entity_commands.unwrap().insert((
            Mesh3d(chunk_mesh_handle.clone()),
            MeshMaterial3d(chunk_material_handle.clone()),
            // Place at correct world position via transform.
            Transform::from_xyz(
                chunk_origin_tile_units_x as f32,
                0.0,
                chunk_origin_tile_units_z as f32,
            ),
            GlobalTransform::default(),
        ));
    }

    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        &format!(
            "Rendered new chunk at: \tgx={}\tgy={}\t(map={})",
            chunk_data_ref.chunk_origin_chunk_units_x,
            chunk_data_ref.chunk_origin_chunk_units_z,
            map_plane_metadata_ref.id
        ),
    );

    // Step 6: Return buffer to pool or drop, allowing efficient reuse and memory management.
    pool_ref.free(meshbufs, diag_ref);
}

