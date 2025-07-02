#![allow(dead_code)]

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
use crate::prelude::*;
use crate::{
    util_lib::array::*,
    core::{constants, texture_cache::terrain::cache::*},
};
use super::{DUMMY_MAP_SIZE_X, DUMMY_MAP_SIZE_Y};


pub struct TerrainChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainChunkMeshPlugin);

impl Plugin for TerrainChunkMeshPlugin
{
    fn build(&self, app: &mut App) {
        app
            .add_plugins(MaterialPlugin::<TerrainMaterial>::default())   // ← register Assets<TerrainMaterial>
            .add_systems(Update, sys_build_visible_terrain_chunks.run_if(in_state(AppState::InGame)));
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

pub const CHUNK_TILE_NUM_1D: u32 = 16;
pub const CHUNK_TILE_NUM_TOTAL: usize = (CHUNK_TILE_NUM_1D * CHUNK_TILE_NUM_1D) as usize;

#[derive(Component)]
pub struct TCMesh {
    pub gx: u32,
    pub gy: u32,
}

// -- Custom Material Definition --------------------------------------------

const CHUNK_TILE_NUM_TOTAL_VEC4: usize = (CHUNK_TILE_NUM_TOTAL as usize + 3) / 4;
const TERRAIN_SHADER_PATH: &str = "shaders/worldmap/terrain_base.wgsl";


// -- Define data and uniforms to be used in the shader. Rust side.

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainMaterialExtension>;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct TerrainMaterialExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub tex_array: Handle<Image>,

    // ← This produces group(2), binding(2) as a 16-byte UBO
    #[uniform(102, min_binding_size = 16)]
    pub uniforms: TerrainUniforms,
}

impl MaterialExtension for TerrainMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
    }
    fn fragment_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
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
pub struct TerrainUniforms {
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
    tile_heights: &[[f32; CHUNK_TILE_NUM_1D as usize]],
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
    tile_heights: &[[f32; CHUNK_TILE_NUM_1D as usize]],
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
                if tx < CHUNK_TILE_NUM_1D && ty < CHUNK_TILE_NUM_1D {
                    sum += get_tile_height(tile_heights, gx, gy, tx, ty);
                    count += 1;
                }
            }
        }
    }
    if count > 0 {
        sum / count as f32
    } else {
        0.0
    }
}

/// Build a custom mesh which is a tile grid with the same shape of
///   a map.mul chunk (CHUNK_TILE_NUM_1D x CHUNK_TILE_NUM_1D).
pub fn sys_build_visible_terrain_chunks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials_terrain: ResMut<Assets<TerrainMaterial>>,
    mut cache: ResMut<TextureCache>,
    mut images: ResMut<Assets<Image>>,
    cam_q: Query<&Transform, With<Camera3d>>,
    chunk_q: Query<(Entity, &TCMesh, Option<&Mesh3d>)>,
) {
    log_system_add_update::<TerrainChunkMeshPlugin>(fname!());
    let cam_pos = cam_q.single().unwrap().translation;

    // TODO: Demo heights: Replace with the actual per-tile map data
    let mut map_dummy_tile_heights = vec![[0.0f32; DUMMY_MAP_SIZE_X as usize + 1]; DUMMY_MAP_SIZE_Y as usize+ 1];
    for ty in 0..DUMMY_MAP_SIZE_Y as usize {
        for tx in 0..DUMMY_MAP_SIZE_X as usize {
            map_dummy_tile_heights[ty][tx] = if (tx + ty) % 2 == 0 { 0.0 } else { 1.0 };
        }
    }

    for (entity, chunk_data, mesh_handle) in chunk_q.iter() {
        if mesh_handle.is_some() {
            continue;
        }

        // Compute chunk origin in world
        let chunk_world_x = chunk_data.gx as f32 * CHUNK_TILE_NUM_1D as f32;
        let chunk_world_z = chunk_data.gy as f32 * CHUNK_TILE_NUM_1D as f32;

        // For distance culling (uses chunk center in world)
        let center = Vec3::new(
            chunk_world_x + CHUNK_TILE_NUM_1D as f32 / 2.0,
            0.0,
            chunk_world_z + CHUNK_TILE_NUM_1D as f32 / 2.0,
        );
        if cam_pos.distance(center) > 80.0 {    // TODO: adjust this dynamically accounting for zoom, window size, etc. Use a function to calc this?
            //println!("cam pos {}", cam_pos);
            //println!("center {}", center);
            continue;
        }

        // --- Generate tile grid in local space: [0, CHUNK_SIZE] on X/Z ---
        let grid_w: usize = CHUNK_TILE_NUM_1D as usize + 1;
        let grid_h: usize = CHUNK_TILE_NUM_1D as usize + 1;
        //let max_arr_idx: usize = (grid_w * grid_h);

        //let mut uo_tile_data = vec![UOMapTile::default(); max_arr_idx];
        let mut mat_ext_uniforms = TerrainUniforms::zeroed();
        mat_ext_uniforms.chunk_origin = Vec2::new(chunk_world_x, chunk_world_z);
        mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

        let mut verts = Vec::with_capacity(grid_w * grid_h);
        let mut heights = vec![0.0f32; grid_w * grid_h];

        // Define positions of each vertex.
        for vy in 0..grid_h {
            for vx in 0..grid_w {
                let world_tx = (chunk_data.gx * CHUNK_TILE_NUM_1D) as usize + vx;
                let world_ty = (chunk_data.gy * CHUNK_TILE_NUM_1D) as usize + vy;
                //let h = get_vertex_height(&dummy_tile_heights, chunk_data.gx, chunk_data.gy, vx, vy);
                let h = map_dummy_tile_heights[world_ty][world_tx]; // direct lookup, not averaging!
                heights[vy * grid_w + vx] = h;

                verts.push(TerrainVertexAttrs {
                    pos: [vx as f32, h, vy as f32],
                    uv: [
                        vx as f32 / (CHUNK_TILE_NUM_1D as f32),
                        vy as f32 / (CHUNK_TILE_NUM_1D as f32),
                    ],
                    norm: [0.0, 1.0, 0.0], // placeholder, they will be calculated after
                });
            }
        }

        // Calculate Smooth Normals: finite difference using derived vertex heights.
        for vy in 0..grid_h {
            for vx in 0..grid_w {
                let world_tx = (chunk_data.gx * CHUNK_TILE_NUM_1D) as usize + vx;
                let world_ty = (chunk_data.gy * CHUNK_TILE_NUM_1D) as usize + vy;
                let center = map_dummy_tile_heights[world_ty][world_tx];

                /*
                Each chunk computes normals only from heights inside its own chunk, so edge normals miss out on what’s just over the border in the global heightgrid.
                As a result, normals on shared edges are not identical on both sides, even though the vertex positions are.
                → Lighting (dot product with light_dir) gives different brightness per chunk = visible “lighting seam”
                 */
                // let center = heights[vy * grid_w + vx];

                let left = if world_tx > 0 {
                    map_dummy_tile_heights[world_ty][world_tx - 1]
                } else {
                    center
                };
                let right = if world_tx + 1 < DUMMY_MAP_SIZE_X as usize{
                    map_dummy_tile_heights[world_ty][world_tx + 1]
                } else {
                    center
                };
                let down = if world_ty > 0 {
                    map_dummy_tile_heights[world_ty - 1][world_tx]
                } else {
                    center
                };
                let up = if world_ty + 1 < DUMMY_MAP_SIZE_Y as usize{
                    map_dummy_tile_heights[world_ty + 1][world_tx]
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
        let mut idxs = Vec::with_capacity(CHUNK_TILE_NUM_TOTAL * 6);
        for ty in 0..CHUNK_TILE_NUM_1D as usize {
            for tx in 0..CHUNK_TILE_NUM_1D as usize {
                let i0 = ty * grid_w + tx;
                let i1 = ty * grid_w + (tx + 1);
                let i2 = (ty + 1) * grid_w + (tx + 1);
                let i3 = (ty + 1) * grid_w + tx;
                // Vertex winding order: counter-clockwise -> normals will point up.
                idxs.extend([
                    i0 as u32, i2 as u32, i1 as u32, i0 as u32, i3 as u32, i2 as u32,
                ]);

                // We pass tile data (read from the MUL files) as a uniform buffer to the wgsl shader.
                // ***** TODO: real tile lookup goes here *****
                let tile = UOMapTile {
                    art_id: ((tx + ty * CHUNK_TILE_NUM_1D as usize) & 0x7FF) as u16, // temp, dummy
                    hue: 0,
                    height: ((tx + ty) & 1) as u16, // temp, dummy
                };
                // Get the layer (index) of the texture array housing this texture (map tile art).
                let layer = cache.layer_of(tile.art_id, &mut commands, &mut images);
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
                    get_1d_array_index_as_2d(CHUNK_TILE_NUM_1D as usize, tx as usize, ty as usize),
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
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_NORMAL,
                verts.iter().map(|v| v.norm).collect::<Vec<_>>(),
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                verts.iter().map(|v| v.uv).collect::<Vec<_>>(),
            );

            // Add dummy data to unused fields, to be used internally by us in the shader:
            // We'll use this field (second pair of UV coords) to pass shading data from the vertex to the fragment shader.
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0, 0.0]; verts.len()]);

            mesh.insert_indices(Indices::U32(idxs));
            meshes.add(mesh)
        };

        let chunk_material_handle = {
            let mat = ExtendedMaterial {
                base: StandardMaterial {
                    ..Default::default()
                },
                extension: TerrainMaterialExtension {
                    tex_array: cache.image_handle.clone(),
                    uniforms: mat_ext_uniforms,
                },
            };
            materials_terrain.add(mat)
        };

        // 💡 Place at correct world position via transform!
        commands.entity(entity).insert((
            Mesh3d(chunk_mesh_handle.clone()),
            MeshMaterial3d(chunk_material_handle.clone()),
            Transform::from_xyz(chunk_world_x, 0.0, chunk_world_z),
            GlobalTransform::default(),
        ));

        logger::one(
        None,
        LogSev::Debug,
        LogAbout::RenderWorldLand,
        format!("Rendered chunk at: gx={}, gy={}.", chunk_data.gx, chunk_data.gy)
            .as_str(),
        );
    }
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
struct TerrainVertexAttrs {
    pos: [f32; 3],
    uv: [f32; 2],
    norm: [f32; 3],
}
