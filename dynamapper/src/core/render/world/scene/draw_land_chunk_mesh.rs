#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};

use super::{DUMMY_MAP_SIZE_X, DUMMY_MAP_SIZE_Y};
use crate::core::render::world::player::Player;
use crate::core::render::world::scene::SceneActiveMap;
use crate::core::uo_files_loader::UoFileData;
use crate::prelude::*;
use crate::{
    core::{constants, texture_cache::land::cache::*},
    util_lib::array::*,
};
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
use uocf::geo::map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos};

pub struct DrawLandChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawLandChunkMeshPlugin);

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<LandCustomMaterial>::default()) // Register Asset
            .add_systems(Update, sys_draw_spawned_land_chunks.run_if(in_state(AppState::InGame)));
    }
}

// -------------------------------------------

// TODO: move to another file, or use uocf one.
#[derive(Clone, Copy, Default)]
pub struct UOMapTile {
    art_id: u16,
    hue: u16,
    height: u16, // in UO it's i8
}

pub const TILE_NUM_PER_CHUNK_1D: u32 = 8; // It's a square, 8 tiles on X axis, 8 tiles on Y axis.
pub const TILE_NUM_PER_CHUNK_TOTAL: usize = (TILE_NUM_PER_CHUNK_1D * TILE_NUM_PER_CHUNK_1D) as usize;

#[derive(Component)]
pub struct TCMesh {
    pub map: u32,
    pub gx: u32,
    pub gy: u32,
}

// -- Custom Material Definition --------------------------------------------

const CHUNK_TILE_NUM_TOTAL_VEC4: usize = (TILE_NUM_PER_CHUNK_TOTAL as usize + 3) / 4;
const LAND_SHADER_PATH: &str = "shaders/worldmap/land_base.wgsl";

// -- Define data and uniforms to be used in the shader. Rust side.

pub type LandCustomMaterial = ExtendedMaterial<StandardMaterial, LandMaterialExtension>;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct LandMaterialExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub tex_array: Handle<Image>,

    // ← This produces group(2), binding(2) as a 16-byte UBO
    #[uniform(102, min_binding_size = 16)]
    pub uniforms: LandUniforms,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        LAND_SHADER_PATH.into()
    }
    fn fragment_shader() -> ShaderRef {
        LAND_SHADER_PATH.into()
    }
}

// Uniform buffer -> just a fancy name for a struct that is passed to the shader, has
//  global scope and is passed per draw call (so for each chunk mesh).
// Uniform Buffer Size Limitations:
//    Most GPUs limit uniform buffers to 64KB (sometimes less!).
//    u32[2048] is 8192 bytes, twice is 16KB—OK, but you need to watch out if you want to add lots of fields.

// Uniform buffer layouts:
//  Most APIs demand 16-byte alignment per field.
//  For a field to be valid in a uniform buffer, each element of an array must be treated as a “vec4” (i.e., 16 bytes each), not simply a u32 (or f32)!
//  It’s a GPU shader hardware limitation—and applies to both WGSL and to Bevy encase/Buffer.

// In order to have 16-bytes (not bit!) alignment, we can use some packing helpers.
// UVec4 (from glam crate, used by Bevy) is a struct holding four unsigned 32-bit integers (u32 values), used as a “vector of four elements”:

#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, ShaderType, bytemuck::Zeroable)]
pub struct LandUniforms {
    pub light_dir: Vec3,
    _pad: f32,
    pub chunk_origin: Vec2,
    _pad2: Vec2,
    pub layers: [UVec4; CHUNK_TILE_NUM_TOTAL_VEC4],
    pub hues: [UVec4; CHUNK_TILE_NUM_TOTAL_VEC4],
}

// -- Helpers for building the mesh  ---------------------------

// Dummy function for tile heights (replace with your own)
// g x/y: grid x/y
// t x/y: tile x/y
fn get_tile_height(
    tile_heights: &[[f32; TILE_NUM_PER_CHUNK_1D as usize]],
    _gx: i32,
    _gy: i32,
    tx: u32,
    ty: u32,
) -> f32 {
    tile_heights[ty as usize][tx as usize]
}

/// Find vertex/corner height by averaging up to four tiles.
// g x/y: grid x/y
// v x/y: vert x/y
fn get_vertex_height(
    tile_heights: &[[f32; TILE_NUM_PER_CHUNK_1D as usize]],
    gx: i32,
    gy: i32,
    vx: u32,
    vy: u32,
) -> f32 {
    let mut sum = 0.0;
    let mut count = 0;
    for dy in 0..2 {
        for dx in 0..2 {
            let tx = vx.checked_sub(dx);
            let ty = vy.checked_sub(dy);
            if let (Some(tx), Some(ty)) = (tx, ty) {
                if tx < TILE_NUM_PER_CHUNK_1D && ty < TILE_NUM_PER_CHUNK_1D {
                    sum += get_tile_height(tile_heights, gx, gy, tx, ty);
                    count += 1;
                }
            }
        }
    }
    if count > 0 { sum / count as f32 } else { 0.0 }
}

// ----

#[derive(Eq, Hash, PartialEq)]
struct LandChunkConstructionData {
    entity: Option<Entity>,
    chunk_origin_chunk_units_x: u32,
    chunk_origin_chunk_units_z: u32,
}

// -----

/// Build a custom mesh which is a tile grid with the same shape of
///   a map.mul chunk (CHUNK_TILE_NUM_1D x CHUNK_TILE_NUM_1D).
pub fn sys_draw_spawned_land_chunks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials_land: ResMut<Assets<LandCustomMaterial>>,
    mut cache: ResMut<LandTextureCache>,
    mut images: ResMut<Assets<Image>>,
    uo_data: Res<UoFileData>,
    active_map: Res<SceneActiveMap>,
    player_q: Query<&mut Player>,
    cam_q: Query<&Transform, With<Camera3d>>,
    chunk_q: Query<(Entity, &TCMesh, Option<&Mesh3d>)>,
) {
    log_system_add_update::<DrawLandChunkMeshPlugin>(fname!());

    // Extract data from queries.
    let player_entity = player_q.single().expect("More than 1 players!");
    let cam_pos = cam_q.single().unwrap().translation;

    // Get the "render world" spawned chunk meshes we have to render.
    // A map chunk in the render world has the same size of a UO map block (8x8).
    //let mut chunks_to_spawn = Vec::<LandChunkConstructionData>::new();
    let mut chunks_to_spawn = HashSet::<LandChunkConstructionData>::new();
    for (entity, chunk_data, mesh_handle) in chunk_q.iter() {
        if mesh_handle.is_some() {
            continue; // Chunk already rendered.
        }

        let chunk_origin_chunk_units_x = chunk_data.gx;
        let chunk_origin_chunk_units_z = chunk_data.gy;
        if false == is_chunk_in_draw_range(cam_pos, chunk_origin_chunk_units_x, chunk_origin_chunk_units_z) {
            continue; // Can't see this chunk, do not render it.
        }

        chunks_to_spawn.insert(LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x,
            chunk_origin_chunk_units_z,
        });

        const NEIGHBOR_OFFSETS: &[(i32, i32)] = &[
            (-1, -1), (0, -1), (1, -1),
            (-1,  0), (0,  0), (1,  0),
            (-1,  1), (0,  1), (1,  1),
        ];

        // Load neighboring blocks
        for (dx, dy) in NEIGHBOR_OFFSETS.iter() {
            let nx: i32 = *dx + i32::try_from(chunk_origin_chunk_units_x).expect("X > i32::MAX?");
            let ny: i32 = *dy + i32::try_from(chunk_origin_chunk_units_z).expect("Y > i32::MAX?");

            // Only process if within bounds
            if nx >= 0 && nx < active_map.width as i32 && ny >= 0 && ny < active_map.height as i32 {
                chunks_to_spawn.insert(LandChunkConstructionData {
                    entity: None,
                    chunk_origin_chunk_units_x: nx as u32,
                    chunk_origin_chunk_units_z: ny as u32,
                });
            }
        }
    }

    // Get the UO map block coords.
    let player_map_plane: u8 = player_entity.current_pos.expect("Player position not yet set?!").m;
    let blocks_to_draw_coords_unique: HashSet<MapBlockRelPos> = chunks_to_spawn
        .iter()
        .map(|construction_data: &LandChunkConstructionData| MapBlockRelPos {
            x: construction_data.chunk_origin_chunk_units_x,
            y: construction_data.chunk_origin_chunk_units_z,
        })
        .collect();
    let blocks_to_draw_coords_vec = blocks_to_draw_coords_unique.iter().cloned().collect();

    // Fetch map data (blocks to draw + neighbors).
    //let mut blocks_to_draw_data = Vec::<uo_lib_map::MapBlock>::new();
    let mut blocks_data = BTreeMap::<MapBlockRelPos, MapBlock>::new();
    {
        // Create a new scope because uo_data is protected by a RwLock (conceptually it's a mutex).
        let mut uo_data_map_planes_lock = uo_data.map_planes.write().unwrap();
        let uo_data_map_plane = &mut uo_data_map_planes_lock.as_mut_slice()[player_map_plane as usize];

        // println!("Load blocks: {blocks_to_draw_coords_vec:#?}");

        // Ensure that uncached map blocks are loaded.
        uo_data_map_plane
            .load_blocks(&blocks_to_draw_coords_vec)
            .expect("Can't load the map blocks.");

        for block_coords in blocks_to_draw_coords_vec {
            let block_ref = uo_data_map_plane
                .block(block_coords)
                .expect("Requested map block is uncached?");
            println!("Adding block {block_coords:?}");
            let unique = blocks_data.insert(block_coords, block_ref.clone()).is_none();
            if !unique {
                panic!("Adding again the same key?");
            }
        }
    }

    // Draw each chunk with the map data.
    for chunk_data in chunks_to_spawn {
        if chunk_data.entity.is_none() {
            continue;
        }
        draw_land_chunk(
            &mut commands,
            &mut meshes,
            &mut materials_land,
            &mut cache,
            &mut images,
            &uo_data,
            &chunk_data,
            &blocks_data,
        );
    }
}

fn is_chunk_in_draw_range(cam_pos: Vec3, chunk_origin_chunk_units_x: u32, chunk_origin_chunk_units_z: u32) -> bool {
    // Compute chunk origin in tile/world units
    let chunk_origin_tile_units_x = chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_tile_units_z = chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

    // For distance culling (uses chunk center in world)
    let center = Vec3::new(
        (chunk_origin_tile_units_x + TILE_NUM_PER_CHUNK_1D) as f32 / 2.0,
        0.0,
        (chunk_origin_tile_units_z + TILE_NUM_PER_CHUNK_1D) as f32 / 2.0,
    );
    if cam_pos.distance(center) > constants::RENDER_DISTANCE_FROM_PLAYER {
        // TODO: adjust this dynamically accounting for zoom, window size, etc. Use a function to calc this? Apply the same logic to the chunk spawning?
        //println!("cam pos {}", cam_pos);
        //println!("center {}", center);
        return false;
    }
    return true;
}

fn draw_land_chunk(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials_land: &mut ResMut<Assets<LandCustomMaterial>>,
    land_texture_cache: &mut ResMut<LandTextureCache>,
    images: &mut ResMut<Assets<Image>>,
    uo_data: &Res<UoFileData>,
    chunk_data: &LandChunkConstructionData,
    blocks_data: &BTreeMap<MapBlockRelPos, MapBlock>,
) {
    // Compute chunk origin in tile/world units
    let current_chunk_origin_world_tile_units_x = chunk_data.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let current_chunk_origin_world_tile_units_z = chunk_data.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

    // To retrieve cell data from the cached blocks data.
    #[track_caller]
    fn get_cell(blocks_data: &BTreeMap<MapBlockRelPos, MapBlock>, world_tile_x: usize, world_tile_z: usize) -> &MapCell {
        // From world_x/z, get the in-chunk relative coords, if necessary change the chunk id.

        let chunk_rel_coords = MapBlockRelPos {
            x: world_tile_x as u32 / TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z as u32 / TILE_NUM_PER_CHUNK_1D,
        };
        let tile_rel_coords = MapCellRelPos {
            x: world_tile_x as u32 % TILE_NUM_PER_CHUNK_1D,
            y: world_tile_z as u32 % TILE_NUM_PER_CHUNK_1D,
        };
        //println!("Requesting tile at x,y={world_tile_x},{world_tile_z} -> {chunk_rel_coords:?}, {tile_rel_coords:?}");
        blocks_data
            .get(&chunk_rel_coords)
            .expect("Requested uncached map block?")
            .cell(tile_rel_coords.x, tile_rel_coords.y)
            .map_err(|err_str| format!("Error: {err_str}."))
            .unwrap()
    }

    // --- Generate mesh for the tile grid (in local space: [0, CHUNK_SIZE] on X/Z) ---
    let grid_w: usize = TILE_NUM_PER_CHUNK_1D as usize; // + 1;
    let grid_h: usize = TILE_NUM_PER_CHUNK_1D as usize; // + 1;

    let mut mat_ext_uniforms = LandUniforms::zeroed();
    mat_ext_uniforms.chunk_origin = Vec2::new(
        current_chunk_origin_world_tile_units_x as f32,
        current_chunk_origin_world_tile_units_z as f32,
    );
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    let mut verts = Vec::with_capacity(grid_w * grid_h);
    let mut heights = vec![0.0f32; grid_w * grid_h];

    // Define positions of each vertex.
    for vy in 0..grid_h {
        for vx in 0..grid_w {
            let world_tx = current_chunk_origin_world_tile_units_x as usize + vx;
            let world_tz = current_chunk_origin_world_tile_units_z as usize + vy;
            let tile_h = get_cell(blocks_data, world_tx, world_tz).z as f32;
            heights[vy * grid_w + vx] = tile_h;

            verts.push(LandVertexAttrs {
                pos: [vx as f32, tile_h, vy as f32],
                uv: [
                    vx as f32 / (TILE_NUM_PER_CHUNK_1D as f32),
                    vy as f32 / (TILE_NUM_PER_CHUNK_1D as f32),
                ],
                norm: [0.0, 1.0, 0.0], // placeholder, they will be calculated after
            });
        }
    }

    // Calculate Smooth Normals: finite difference using derived vertex heights.
    for vy in 0..grid_h {
        for vx in 0..grid_w {
            let world_tx = current_chunk_origin_world_tile_units_x as usize + vx;
            let world_tz = current_chunk_origin_world_tile_units_z as usize + vy;
            let center = get_cell(blocks_data, world_tx, world_tz).z as f32;

            /*
            Each chunk computes normals only from heights inside its own chunk, so edge normals miss out on what’s just over the border in the global heightgrid.
            As a result, normals on shared edges are not identical on both sides, even though the vertex positions are.
            → Lighting (dot product with light_dir) gives different brightness per chunk = visible “lighting seam”
             */
            // let center = heights[vy * grid_w + vx];

            let left = if world_tx > 0 {
                get_cell(blocks_data, world_tx - 1, world_tz).z as f32
            } else {
                center
            };
            let right = if world_tx + 1 < DUMMY_MAP_SIZE_X as usize {
                get_cell(blocks_data, world_tx + 1, world_tz).z as f32
            } else {
                center
            };
            let down = if world_tz > 0 {
                get_cell(blocks_data, world_tx, world_tz - 1).z as f32
            } else {
                center
            };
            let up = if world_tz + 1 < DUMMY_MAP_SIZE_Y as usize {
                get_cell(blocks_data, world_tx, world_tz + 1).z as f32
            } else {
                center
            };

            let dx = (right - left) * 0.5;
            let dz = (up - down) * 0.5;
            let normal = Vec3::new(-dx, 1.0, -dz).normalize();
            verts[vy * grid_w + vx].norm = normal.to_array();
        }
    }

    // Build triangle indices (tiles reference vertices).
    let mut idxs = Vec::with_capacity(TILE_NUM_PER_CHUNK_TOTAL * 6);
    for ty in 0..TILE_NUM_PER_CHUNK_1D as usize {
        for tx in 0..TILE_NUM_PER_CHUNK_1D as usize {
            let i0 = ty * grid_w + tx;
            let i1 = ty * grid_w + (tx + 1);
            let i2 = (ty + 1) * grid_w + (tx + 1);
            let i3 = (ty + 1) * grid_w + tx;
            // Vertex winding order: counter-clockwise -> normals will point up.
            idxs.extend([i0 as u32, i2 as u32, i1 as u32, i0 as u32, i3 as u32, i2 as u32]);

            // We pass tile data (read from the MUL files) as a uniform buffer to the wgsl shader.
            // ***** TODO: real tile lookup goes here *****
            let tile = UOMapTile {
                art_id: ((tx + ty * TILE_NUM_PER_CHUNK_1D as usize) & 0x7FF) as u16, // temp, dummy
                hue: 0,
                height: ((tx + ty) & 1) as u16, // temp, dummy
            };
            // Get the layer (index) of the texture array housing this texture (map tile art).
            let layer = land_texture_cache.layer_of(commands, images, uo_data, tile.art_id);
            // TODO: set uo_tile_data[layer] = tile.

            // Per-tile (in the chunk) data. I actually might not need this.
            //let mut td = &mut uo_tile_data[get_tile_array_index(tx, ty)];

            // Update values of the uniform buffer. This is per-chunk data (per mesh draw call).
            // We need to store the data not in a simple vector, but in a vector of 4D vectors, in order to meet
            // the 16-byte field alignment requisite.

            /*
            // We need to access the right layer, so start by picking the right 4D array:
            let mut layers_u4_arr_ref = &mut mat_ext_uniforms.layers[get_1d_array_index_as_2d(tx, ty)];
            // Now get the correct one among the 4 elements.
            let mut layer_ref = uvec4_elem_get_mut(layers_u4_arr_ref, get_1d_array_index_as_2d(tx, ty));
            */
            let layer_ref = uvec4_elem_get_mut(
                &mut mat_ext_uniforms.layers,
                get_1d_array_index_as_2d(TILE_NUM_PER_CHUNK_1D as usize, tx as usize, ty as usize),
            );
            *layer_ref = layer;
        }
    }

    // Build Bevy mesh (local space [0,CHUNK_SIZE]).
    let chunk_mesh_handle: Handle<Mesh> = {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            verts.iter().map(|v| v.pos).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, verts.iter().map(|v| v.norm).collect::<Vec<_>>());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, verts.iter().map(|v| v.uv).collect::<Vec<_>>());

        // Add dummy data to unused fields, to be used internally by us in the shader:
        // We'll use this field (second pair of UV coords) to pass shading data from the vertex to the fragment shader.
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0, 0.0]; verts.len()]);

        mesh.insert_indices(Indices::U32(idxs));
        meshes.add(mesh)
    };

    let chunk_material_handle = {
        let mat = ExtendedMaterial {
            base: StandardMaterial { ..Default::default() },
            extension: LandMaterialExtension {
                tex_array: land_texture_cache.image_handle.clone(),
                uniforms: mat_ext_uniforms,
            },
        };
        materials_land.add(mat)
    };

    // 💡 Place at correct world position via transform!
    commands.entity(chunk_data.entity.unwrap()).insert((
        Mesh3d(chunk_mesh_handle.clone()),
        MeshMaterial3d(chunk_material_handle.clone()),
        Transform::from_xyz(
            current_chunk_origin_world_tile_units_x as f32,
            0.0,
            current_chunk_origin_world_tile_units_z as f32,
        ),
        GlobalTransform::default(),
    ));

    logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        format!(
            "Rendered chunk at: gx={}, gy={}, tx={}, ty={}.",
            chunk_data.chunk_origin_chunk_units_x,
            chunk_data.chunk_origin_chunk_units_z,
            current_chunk_origin_world_tile_units_x,
            current_chunk_origin_world_tile_units_z
        )
        .as_str(),
    );
}

use Vec3;
trait _Arrayable {
    fn to_array(&self) -> [f32; 3];
}
impl _Arrayable for Vec3 {
    fn to_array(&self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

// Base mesh attributes that we need to provide.
#[derive(Clone, Copy)]
struct LandVertexAttrs {
    pos: [f32; 3],
    uv: [f32; 2],
    norm: [f32; 3],
}
