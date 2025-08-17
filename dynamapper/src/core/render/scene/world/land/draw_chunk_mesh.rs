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
use std::{collections::{BTreeMap, HashSet}, sync::Arc};
use std::time::Instant;
use uocf::geo::{
    land_texture_2d::{LandTextureSize, TexMap2D},
    map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos},
};
use wide::*;

use super::TILE_NUM_PER_CHUNK_1D;
use super::{LCMesh, mesh_material::*};
use crate::{
    core::{
        constants,
        maps::MapPlaneMetadata,
        render::scene::{camera::PlayerCamera, player::Player, world::WorldGeoData, SceneStateData},
        texture_cache::land::cache::*, uo_files_loader::{MapPlanesRes, TexMap2DRes},
    },
    prelude::*,
    util_lib::array::*,
};




// ---- Shared Mesh Resource and Setup ----

#[derive(Resource)]
pub struct LandMeshHandle(pub Handle<Mesh>);

/// This startup system generates a single, shared 9x9 grid mesh for all land chunks.
pub fn setup_land_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    const GRID_W: usize = (TILE_NUM_PER_CHUNK_1D + 1) as usize;
    const GRID_H: usize = (TILE_NUM_PER_CHUNK_1D + 1) as usize;
    const CORE_W: usize = TILE_NUM_PER_CHUNK_1D as usize;
    const CORE_H: usize = TILE_NUM_PER_CHUNK_1D as usize;

    let estimated_vertex_count = GRID_W * GRID_H;
    let mut positions = Vec::with_capacity(estimated_vertex_count);
    let mut uvs = Vec::with_capacity(estimated_vertex_count);
    let mut indices = Vec::new();

    // Create a flat 9x9 grid of vertices at y=0
    // Add dummy height values (0.0) because the real one will be calculated on the gpu, via the shader
    //  (we send tile height through a uniform buffer).
    // We are adding an extra row and column to avoid seam artifacts and to make the neighboring chunk minimum tiles data
    //  available for the shader to calculate normals.
    for gy in 0..GRID_H {
        for gx in 0..GRID_W {
            positions.push([gx as f32, 0.0, gy as f32]);
            uvs.push([
                gx as f32 / (CORE_W as f32),
                gy as f32 / (CORE_H as f32),
            ]);
        }
    }

    // Create indices for the 8x8 core of the grid
    for ty in 0..CORE_H {
        for tx in 0..CORE_W {
            let v0 = (ty * GRID_W + tx) as u32;
            let v1 = v0 + 1;
            let v2 = ((ty + 1) * GRID_W + tx) as u32;
            let v3 = v2 + 1;
            indices.extend_from_slice(&[v0, v3, v1, v0, v2, v3]);
        }
    }

    // Provide dummy normals and UV1s to match the shader's vertex format
    let dummy_normals = vec![[0.0, 1.0, 0.0]; estimated_vertex_count];
    let dummy_uv1s = vec![[0.0, 0.0]; estimated_vertex_count];

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, dummy_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, dummy_uv1s);
    mesh.insert_indices(Indices::U32(indices));

    let handle = meshes.add(mesh);
    commands.insert_resource(LandMeshHandle(handle));
}

/// Creates a new material with the specific uniform data for a single land chunk.
fn create_land_chunk_material(
    materials_land_rref: &mut ResMut<Assets<LandCustomMaterial>>,
    land_texture_cache_rref: &mut ResMut<LandTextureCache>,
    images_rref: &mut ResMut<Assets<Image>>,
    time_r: &Res<Time>,
    texmap_2d: Arc<TexMap2D>,
    chunk_data_ref: &LandChunkConstructionData,
    blocks_data_ref: &BTreeMap<MapBlockRelPos, MapBlock>,
) -> Handle<LandCustomMaterial> {
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

    // Helper to fetch a cell from the loaded block data.
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
            .unwrap()
            .cell(tile_rel_coords.x, tile_rel_coords.y)
            .unwrap()
    }

    const CHUNK_TILE_DATA_SIDE: i32 = (TILE_NUM_PER_CHUNK_1D + 5) as i32; // 8 + 5 = 13
    const BORDER: i32 = 2;

    // 1) Gather all cell data for the 13x13 grid in one pass.
    let mut cell_grid: Vec<&MapCell> =
        Vec::with_capacity((CHUNK_TILE_DATA_SIDE * CHUNK_TILE_DATA_SIDE) as usize);
    for gy in -BORDER..(TILE_NUM_PER_CHUNK_1D as i32 + BORDER + 1) {
        for gx in -BORDER..(TILE_NUM_PER_CHUNK_1D as i32 + BORDER + 1) {
            let world_tx = (chunk_origin_tile_units_x as i32 + gx).max(0) as u32;
            let world_tz = (chunk_origin_tile_units_z as i32 + gy).max(0) as u32;
            cell_grid.push(get_cell(blocks_data_ref, world_tx, world_tz));
        }
    }

    // 2) Prepare Uniforms. This now includes all data for the 13x13 grid.
    let mut mat_ext_land_uniforms = LandUniform::zeroed();
    mat_ext_land_uniforms.chunk_origin = Vec2::new(
        chunk_origin_tile_units_x as f32,
        chunk_origin_tile_units_z as f32,
    );
    mat_ext_land_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT.normalize();

    // Preload all unique textures for the 13x13 grid.
    let unique_tile_ids: HashSet<u16> = cell_grid.iter().map(|cell| cell.id).collect();
    land_texture_cache_rref.preload_textures(
        images_rref,
        texmap_2d.clone(),
        &unique_tile_ids,
    );

    // Fill the 13x13 uniform grid.
    for i in 0..cell_grid.len() {
        let tile_ref = cell_grid[i];
        let (texture_size, layer) = land_texture_cache_rref.get_texture_size_layer(
            images_rref,
            texmap_2d.clone(),
            tile_ref.id,
        );
        mat_ext_land_uniforms.tiles[i] = TileUniform {
            tile_height: scale_uo_z_to_bevy_units(tile_ref.z as f32),
            texture_size: match texture_size {
                LandTextureSize::Small => 0,
                LandTextureSize::Big => 1,
            },
            texture_layer: layer,
            texture_hue: 0,
        };
    }

    // Scene data
    let mut mat_ext_scene_uniform = SceneUniform::zeroed();
    mat_ext_scene_uniform.camera_position = PlayerCamera::BASE_OFFSET_FROM_PLAYER;
    mat_ext_scene_uniform.light_direction = constants::BAKED_GLOBAL_LIGHT;

    // Tunables are separate.
    let mut mat_ext_tunables_uniform = TunablesUniform::zeroed();
    mat_ext_tunables_uniform.use_vertex_lighting = 1;
    mat_ext_tunables_uniform.sharpness_factor = 1.0;
    mat_ext_tunables_uniform.sharpness_mix_factor = 1.0;

    // Visuals
    let mut mat_ext_visual_uniform = VisualUniform::zeroed();
    mat_ext_visual_uniform.fog_color = Vec4::new(0.7, 0.8, 0.9, 0.5);
    mat_ext_visual_uniform.fog_params = Vec4::new(0.1, 0.1, 0.01, 0.01);
    mat_ext_visual_uniform.fill_sky_color = Vec4::new(0.5, 0.6, 0.8, 0.2);
    mat_ext_visual_uniform.fill_ground_color = Vec4::new(0.4, 0.3, 0.2, 0.1);
    mat_ext_visual_uniform.rim_color = Vec4::new(1.0, 1.0, 0.8, 4.0);
    mat_ext_visual_uniform.grade_warm_color = Vec4::new(1.0, 0.9, 0.8, 1.0);
    mat_ext_visual_uniform.grade_cool_color = Vec4::new(0.8, 0.9, 1.0, 1.0);
    mat_ext_visual_uniform.grade_params = Vec4::new(0.1, 0.0, 0.0, 0.0);
    mat_ext_visual_uniform.time_seconds = time_r.elapsed().as_secs_f32();

    // 3) Create and return the material handle.
    let mat = ExtendedMaterial {
        base: StandardMaterial::default(),
        extension: LandMaterialExtension {
            texarray_small: land_texture_cache_rref.small.image_handle.clone(),
            texarray_big: land_texture_cache_rref.big.image_handle.clone(),
            land_uniform: mat_ext_land_uniforms,
            scene_uniform: mat_ext_scene_uniform,
            tunables_uniform: mat_ext_tunables_uniform,
            visual_uniform: mat_ext_visual_uniform,
            lighting_uniform: LightingUniforms::default(),
        },
    };
    materials_land_rref.add(mat)
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
    mut materials_land_r: ResMut<Assets<LandCustomMaterial>>,
    mut cache_r: ResMut<LandTextureCache>,
    mut images_r: ResMut<Assets<Image>>,
    mut map_planes_r: ResMut<MapPlanesRes>,
    time_r: Res<Time>,
    texmap_2d_r: Res<TexMap2DRes>,
    world_geo_data_r: Res<WorldGeoData>,
    scene_state_data_r: Res<SceneStateData>,
    player_q: Query<&Player>,
    cam_q: Query<&Transform, With<Camera3d>>,
    chunk_q: Query<(Entity, &LCMesh, Option<&Mesh3d>)>,
    visible_chunk_q: Query<(&LCMesh, &Mesh3d)>,
    land_mesh_handle_r: Res<LandMeshHandle>,
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
            &mut meshes_r,
            &mut materials_land_r,
            &mut cache_r,
            &mut images_r,
            &time_r,
            texmap_2d_r.0.clone(),
            &map_plane_metadata,
            &chunk_data,
            &blocks_data,
            // pass the shared mesh handle
            &land_mesh_handle_r,
        );
    }
    let build_time: u128 = build_time_start.elapsed().as_micros();
    println!("Perf: chunk rendered in {build_time} ms.");
}


// Completed!
fn draw_land_chunk(
    commands: &mut Commands,
    meshes_rref: &mut ResMut<Assets<Mesh>>,
    materials_land_rref: &mut ResMut<Assets<LandCustomMaterial>>,
    land_texture_cache_rref: &mut ResMut<LandTextureCache>,
    images_rref: &mut ResMut<Assets<Image>>,
    time_r: &Res<Time>,
    texmap_2d: Arc<TexMap2D>,
    map_plane_metadata_ref: &MapPlaneMetadata,
    chunk_data_ref: &LandChunkConstructionData,
    blocks_data_ref: &BTreeMap<MapBlockRelPos, MapBlock>,
    land_mesh_handle_r: &Res<LandMeshHandle>,
) {
    // Use the mesh prebuilt in setup_land_mesh.
    let chunk_mesh_handle: Handle<Mesh> = land_mesh_handle_r.0.clone();

    // Create the material with create_land_chunk_material and attach it to the entity for the new map chunk.
    let chunk_material_handle: Handle<LandCustomMaterial> = create_land_chunk_material(
        materials_land_rref,
        land_texture_cache_rref,
        images_rref,
        time_r,
        texmap_2d,
        chunk_data_ref,
        blocks_data_ref,
    );

    // Compute chunk origin (in tile units) for the transform.
    let chunk_origin_tile_units_x =
        chunk_data_ref.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_tile_units_z =
        chunk_data_ref.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

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
            GlobalTransform::default(),
        ));
    } else {
        logger::one(
            None,
            LogSev::Error,
            LogAbout::RenderWorldLand,
            "Skipping drawing of invalid/unspawned entity at stage 'build_indexed_chunk_mesh'.",
        );
    }
}
