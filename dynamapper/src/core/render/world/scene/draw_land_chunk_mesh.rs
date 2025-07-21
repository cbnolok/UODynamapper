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
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::time::Instant;
use uocf::geo::map::{MapBlock, MapBlockRelPos, MapCell, MapCellRelPos};

use crate::core::render::world::scene::{DUMMY_MAP_SIZE_X, DUMMY_MAP_SIZE_Y};
use crate::{
    core::render::world::player::Player,
    core::render::world::scene::SceneActiveMap,
    core::uo_files_loader::UoFileData,
    core::{constants, texture_cache::land::cache::*},
    prelude::*,
    util_lib::array::*,
};

// ==================================================================================
//                      PLUGIN AND RESOURCE REGISTRATION
// ==================================================================================

/// Register the chunk renderer plugin into your app.
/// Establishes material, buffer pool, diagnostics, and the draw system.
pub struct DrawLandChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawLandChunkMeshPlugin);

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<LandCustomMaterial>::default())
            .insert_resource(LandChunkMeshBufferPool::with_capacity(60)) // Preallocate 60 chunk buffers.
            .insert_resource(LandChunkMeshDiagnostics::default()) // Performance/statistics resource.
            .insert_resource(MeshBuildPerfHistory::new(64)) // Mesh build time history.
            .add_systems(
                Update,
                (
                    sys_draw_spawned_land_chunks.run_if(in_state(AppState::InGame)),
                    //print_render_stats,
                ),
            );
    }
}

// ==================================================================================
//                                MESH BUFFER POOL
// ==================================================================================

/// Pool to minimize dynamic allocations for chunk mesh vertex buffers.
/// Allocates [at most] `capacity` buffers up front, then dynamically allocates "spillover" as needed.
/// Buffers obtained from pool should be returned immediately after use.
/// Buffers that weren't pre-pooled are simply dropped instead of being recycled.
pub struct MeshBuffers {
    pub positions: Vec<[f32; 3]>, // Each vertex position (x, y, z)
    pub normals: Vec<[f32; 3]>,   // Per-vertex surface normal (affects lighting)
    pub uvs: Vec<[f32; 2]>,       // Per-vertex texture coordinate
    pub indices: Vec<u32>,        // Indices composing triangles from positions
    pool_alloc: bool,             // true if from pool, false if dynamically allocated
}

#[derive(Resource)]
pub struct LandChunkMeshBufferPool {
    pool: VecDeque<MeshBuffers>, // The available pooled buffers (up to fixed size)
    used: usize,                 // Count of currently checked out buffers
    allocs: usize,               // Running total of allocations (for diagnostics)
    high_water: usize,           // Max number of concurrent checked-out at once (diagnostics)
    #[allow(unused)]
    capacity: usize, // Pool capacity
}

impl LandChunkMeshBufferPool {
    /// Initialize the pool with a given fixed capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let buffer_template = || MeshBuffers {
            positions: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            normals: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            uvs: vec![[0.0; 2]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
            indices: vec![0u32; (TILE_NUM_PER_CHUNK_TOTAL * 6)],
            pool_alloc: true,
        };
        let mut pool = VecDeque::with_capacity(capacity);
        for _ in 0..capacity {
            pool.push_back(buffer_template());
        }
        Self {
            pool,
            used: 0,
            allocs: 0,
            high_water: 0,
            capacity,
        }
    }
    /// Allocate a mesh buffer, using the pool if not empty, otherwise dynamically.
    pub fn alloc(&mut self, diag: &mut LandChunkMeshDiagnostics) -> MeshBuffers {
        let buffers = self.pool.pop_front().unwrap_or_else(|| {
            // Dynamic: rare unless view frustum is huge or a bug causes leaks
            MeshBuffers {
                positions: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                normals: vec![[0.0; 3]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                uvs: vec![[0.0; 2]; (TILE_NUM_PER_CHUNK_1D as usize + 1).pow(2)],
                indices: vec![0u32; (TILE_NUM_PER_CHUNK_TOTAL * 6)],
                pool_alloc: false,
            }
        });
        self.used += 1;
        self.allocs += 1;
        self.high_water = self.high_water.max(self.used);
        diag.mesh_allocs = self.allocs;
        diag.alloc_high_water = self.high_water;
        buffers
    }
    /// Return a mesh buffer to the pool if compatible, otherwise drop it (let Rust reclaim).
    pub fn free(&mut self, buffers: MeshBuffers, diag: &mut LandChunkMeshDiagnostics) {
        if buffers.pool_alloc {
            self.pool.push_back(buffers);
        }
        if self.used > 0 {
            self.used -= 1;
        }
        diag.pool_in_positions = self.pool.len();
        diag.pool_in_normals = self.pool.len();
        diag.pool_in_uvs = self.pool.len();
        diag.pool_in_indices = self.pool.len();
    }
}

// ==================================================================================
//                               CONSTANTS & STRUCTS
// ==================================================================================

/// How many tiles per chunk row/column (chunks are square)?
pub const TILE_NUM_PER_CHUNK_1D: u32 = 8;
/// How many tiles in one chunk total?
pub const TILE_NUM_PER_CHUNK_TOTAL: usize =
    (TILE_NUM_PER_CHUNK_1D * TILE_NUM_PER_CHUNK_1D) as usize;

/// Tag component: Marks entities which are chunk meshes, allows queries for those entities.
#[derive(Component)]
pub struct TCMesh {
    #[allow(unused)]
    pub parent_map: u32,
    pub gx: u32, // chunk grid coordinates
    pub gy: u32,
}

// ------------- Land material/shader data -------------
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

/// Each chunk mesh gets a shader material generated per-chunk, with this struct as its extension.
///
/// See comments above LandUniforms for why uniforms are aligned the way they are.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, ShaderType, bytemuck::Zeroable)]
pub struct LandUniforms {
    pub light_dir: Vec3,
    _pad: f32,
    pub chunk_origin: Vec2,
    _pad2: Vec2,
    pub layers: [UVec4; (TILE_NUM_PER_CHUNK_TOTAL as usize + 3) / 4],
    pub hues: [UVec4; (TILE_NUM_PER_CHUNK_TOTAL as usize + 3) / 4],
}

pub type LandCustomMaterial = ExtendedMaterial<StandardMaterial, LandMaterialExtension>;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct LandMaterialExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub tex_array: Handle<Image>,
    #[uniform(102, min_binding_size = 16)]
    pub uniforms: LandUniforms,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct LandChunkConstructionData {
    entity: Option<Entity>,
    chunk_origin_chunk_units_x: u32,
    chunk_origin_chunk_units_z: u32,
}

// ==================================================================================
//                             MAIN RENDER SYSTEM
// ==================================================================================

/// Main system: finds visible land map chunks and ensures their mesh is generated and rendered.
/// Highly commented for clarity:
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
    visible_chunk_q: Query<(&TCMesh, &Mesh3d)>, // <-- Added for diagnostics: live chunk meshes
    mut pool: ResMut<LandChunkMeshBufferPool>,
    mut diag: ResMut<LandChunkMeshDiagnostics>,
    mut hist: ResMut<MeshBuildPerfHistory>,
) {
    // Step 1: Get camera/player state.
    let cam_pos = cam_q.single().unwrap().translation;
    let player_entity = player_q.single().expect("More than 1 player!");

    // Step 2: Compute which chunk positions are needed for this frame.
    // Chunks that need to be spawned include currently visible, and direct neighbors (to hide seams between loaded chunks).
    let mut spawn_targets = HashSet::<LandChunkConstructionData>::new();
    for (entity, chunk_data, mesh_handle) in chunk_q.iter() {
        // Optimization: Only process chunks not already meshed, avoid duplicate work.
        if mesh_handle.is_some() {
            continue;
        }
        let chunk_origin_x = chunk_data.gx;
        let chunk_origin_z = chunk_data.gy;
        if !is_chunk_in_draw_range(cam_pos, chunk_origin_x, chunk_origin_z) {
            continue;
        }
        spawn_targets.insert(LandChunkConstructionData {
            entity: Some(entity),
            chunk_origin_chunk_units_x: chunk_origin_x,
            chunk_origin_chunk_units_z: chunk_origin_z,
        });

        // Also include all direct neighbors (to avoid mesh seams).
        const NEIGHBOR_OFFSETS: &[(i32, i32)] = &[
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (0, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ];
        for (dx, dy) in NEIGHBOR_OFFSETS {
            let nx = *dx + chunk_origin_x as i32;
            let ny = *dy + chunk_origin_z as i32;
            if nx >= 0 && nx < active_map.width as i32 && ny >= 0 && ny < active_map.height as i32 {
                spawn_targets.insert(LandChunkConstructionData {
                    entity: None,
                    chunk_origin_chunk_units_x: nx as u32,
                    chunk_origin_chunk_units_z: ny as u32,
                });
            }
        }
    }

    // Step 3: Collect the MapBlockRelPos for all target chunks and load them from UO data.
    let player_map_plane = player_entity
        .current_pos
        .expect("Player position not yet set?!")
        .m;
    let blocks_to_draw: HashSet<MapBlockRelPos> = spawn_targets
        .iter()
        .map(|d| MapBlockRelPos {
            x: d.chunk_origin_chunk_units_x,
            y: d.chunk_origin_chunk_units_z,
        })
        .collect();
    let blocks_vec = blocks_to_draw.iter().cloned().collect::<Vec<_>>();

    let mut blocks_data = BTreeMap::<MapBlockRelPos, MapBlock>::new();
    {
        // This lock only needed during the block loading from disk/memory.
        let mut uo_data_map_planes_lock = uo_data.map_planes.write().unwrap();
        let uo_data_map_plane =
            &mut uo_data_map_planes_lock.as_mut_slice()[player_map_plane as usize];
        uo_data_map_plane
            .load_blocks(&blocks_vec)
            .expect("Can't load map blocks");
        for block_coords in blocks_vec {
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
            &mut pool,
            &mut diag,
        );
    }
    let build_ms = build_time_start.elapsed().as_secs_f32() * 1000.0;
    hist.push(build_ms);

    // Step 5: Diagnostics
    diag.build_last = build_ms;
    diag.build_avg = hist.avg();
    diag.build_peak = hist.peak();

    // OLD: diag.num_chunks = blocks_data.len();
    // NEW: Accurately track # of chunks on screen for diagnostics.
    diag.chunks_on_screen = visible_chunk_q.iter().count();
}

/// Helper: Is a chunk in the draw distance from the camera/player?
/// TODO: unify
fn is_chunk_in_draw_range(
    cam_pos: Vec3,
    chunk_origin_chunk_units_x: u32,
    chunk_origin_chunk_units_z: u32,
) -> bool {
    let chunk_origin_tile_units_x = chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_tile_units_z = chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;
    let center = Vec3::new(
        (chunk_origin_tile_units_x + TILE_NUM_PER_CHUNK_1D) as f32 / 2.0,
        0.0,
        (chunk_origin_tile_units_z + TILE_NUM_PER_CHUNK_1D) as f32 / 2.0,
    );
    cam_pos.distance(center) <= constants::RENDER_DISTANCE_FROM_PLAYER
}

/// Build mesh, attributes and assign to chunk entity.
/// Comments explain everything for unfamiliar devs.
fn draw_land_chunk(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials_land: &mut ResMut<Assets<LandCustomMaterial>>,
    land_texture_cache: &mut ResMut<LandTextureCache>,
    images: &mut ResMut<Assets<Image>>,
    uo_data: &Res<UoFileData>,
    chunk_data: &LandChunkConstructionData,
    blocks_data_ref: &BTreeMap<MapBlockRelPos, MapBlock>,
    pool: &mut LandChunkMeshBufferPool,
    diag: &mut LandChunkMeshDiagnostics,
) {
    let chunk_origin_x = chunk_data.chunk_origin_chunk_units_x * TILE_NUM_PER_CHUNK_1D;
    let chunk_origin_z = chunk_data.chunk_origin_chunk_units_z * TILE_NUM_PER_CHUNK_1D;

    // Inline: Helper to fetch cell from block/block coordinates. Always panics on OOB for safety.
    #[inline]
    #[track_caller]
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
            .expect("Requested uncached map block?")
            .cell(tile_rel_coords.x, tile_rel_coords.y)
            .map_err(|err_str| format!("Error: {err_str}."))
            .unwrap()
    }

    const GRID_W: u32 = TILE_NUM_PER_CHUNK_1D + 1;
    const GRID_H: u32 = TILE_NUM_PER_CHUNK_1D + 1;

    // --------- Setup chunk-specific uniforms ---------
    // For shading, lighting, texturing. Synchronized with shader struct.
    let mut mat_ext_uniforms = LandUniforms::zeroed();
    mat_ext_uniforms.chunk_origin = Vec2::new(chunk_origin_x as f32, chunk_origin_z as f32);
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    // --------- MESH DATA GENERATION (fixed UVs, per-tile quads) --------
    let mut meshbufs = pool.alloc(diag);
    meshbufs.positions.clear();
    meshbufs.normals.clear();
    meshbufs.uvs.clear();
    meshbufs.indices.clear();

    // Also record grid of heights for normal calculation (original logic)
    let mut heights = vec![0.0f32; (GRID_W * GRID_H) as usize];
    for vy in 0..GRID_H {
        for vx in 0..GRID_W {
            let world_tx = chunk_origin_x as usize + vx as usize;
            let world_tz = chunk_origin_z as usize + vy as usize;
            heights[(vy * GRID_W + vx) as usize] =
                get_cell(blocks_data_ref, world_tx, world_tz).z as f32;
        }
    }

    for ty in 0..TILE_NUM_PER_CHUNK_1D {
        for tx in 0..TILE_NUM_PER_CHUNK_1D {
            // Four corners per tile quad
            let vx = tx;
            let vy = ty;
            let vx1 = tx + 1;
            let vy1 = ty + 1;

            let world_tx0 = chunk_origin_x + vx;
            let world_tz0 = chunk_origin_z + vy;
            let world_tx1 = chunk_origin_x + vx1;
            let world_tz1 = chunk_origin_z + vy1;

            // Heights at four corners
            let h00 = heights[((vy * GRID_W) + vx) as usize];
            let h10 = heights[((vy * GRID_W) + vx1) as usize];
            let h11 = heights[((vy1 * GRID_W) + vx1) as usize];
            let h01 = heights[((vy1 * GRID_W) + vx) as usize];

            // Normals at four corners (copying original normal logic)
            let get_norm = |wx: u32, wz: u32| {
                let center = get_cell(blocks_data_ref, wx as usize, wz as usize).z as f32;
                let left = if wx > 0 {
                    get_cell(blocks_data_ref, (wx - 1) as usize, wz as usize).z as f32
                } else {
                    center
                };
                let right = if wx + 1 < DUMMY_MAP_SIZE_X {
                    get_cell(
                        blocks_data_ref,
                        (wx + 1) as usize,
                        wz as usize,
                    )
                    .z as f32
                } else {
                    center
                };
                let down = if wz > 0 {
                    get_cell(blocks_data_ref, wx as usize, (wz - 1) as usize).z as f32
                } else {
                    center
                };
                let up = if wz + 1 < DUMMY_MAP_SIZE_Y {
                    get_cell(
                        blocks_data_ref,
                        wx as usize,
                        (wz + 1) as usize,
                    )
                    .z as f32
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
                base + 0, base + 2, base + 1,
                base + 0, base + 3, base + 2,
            ]);
        }
    }

    // --------- Shader uniforms for texture layer (by tile) --------
    mat_ext_uniforms.chunk_origin = Vec2::new(chunk_origin_x as f32, chunk_origin_z as f32);
    mat_ext_uniforms.light_dir = constants::BAKED_GLOBAL_LIGHT;

    for ty in 0..TILE_NUM_PER_CHUNK_1D as usize {
        for tx in 0..TILE_NUM_PER_CHUNK_1D as usize {
            let world_x = chunk_origin_x as usize + tx;
            let world_y = chunk_origin_z as usize + ty;
            let tile_ref = get_cell(blocks_data_ref, world_x, world_y);

            // Each quad (tile) uses two triangles (6 indices).
            // Get the layer (index) of the texture array housing this texture (map tile art).
            let layer = land_texture_cache.layer_of(commands, images, uo_data, tile_ref.id);

            // Update values of the uniform buffer. This is per-chunk data (per mesh draw call).
            // We need to store the data not in a simple vector, but in a vector of 4D vectors, in order to meet
            // the 16-byte field alignment requisite.
            // We need to access the right layer, so start by picking the right 4D array:
            // Then get the correct one among the 4 elements.

            let layer_ref = uvec4_elem_get_mut(
                &mut mat_ext_uniforms.layers,
                get_1d_array_index_as_2d(TILE_NUM_PER_CHUNK_1D as usize, tx, ty),
            );
            *layer_ref = layer;
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
        meshes.add(mesh)
    };

    let chunk_material_handle = {
        let mat = ExtendedMaterial {
            base: StandardMaterial {
                ..Default::default()
            },
            extension: LandMaterialExtension {
                tex_array: land_texture_cache.image_handle.clone(),
                uniforms: mat_ext_uniforms,
            },
        };
        materials_land.add(mat)
    };

    // Step 5: Attach or update the Bevy entity with mesh/material/transform.
    commands.entity(chunk_data.entity.unwrap()).insert((
        Mesh3d(chunk_mesh_handle.clone()),
        MeshMaterial3d(chunk_material_handle.clone()),
        // 💡 Place at correct world position via transform!
        Transform::from_xyz(chunk_origin_x as f32, 0.0, chunk_origin_z as f32),
        GlobalTransform::default(),
    ));

    // Step 6: Return buffer to pool or drop, allowing efficient reuse and memory management.
    pool.free(meshbufs, diag);
}

// ==================================================================================
//                               DIAGNOSTICS / LOGGING
// ==================================================================================

// Contains full performance and memory resource tracking fields for debugging and profiling.
#[derive(Resource, Default)]
pub struct LandChunkMeshDiagnostics {
    pub mesh_allocs: usize,
    pub alloc_high_water: usize,
    pub build_avg: f32,
    pub build_last: f32,
    pub build_peak: f32,
    pub pool_in_positions: usize,
    pub pool_in_normals: usize,
    pub pool_in_uvs: usize,
    pub pool_in_indices: usize,
    pub chunks_on_screen: usize, // Number of rendered chunk meshes (diagnostic log field)
}
impl LandChunkMeshDiagnostics {
    pub fn log(&self) {
        println!(
            // ChunksOnScreen: actual rendered chunk mesh count this frame.
            "[LandMeshDiag] ChunksOnScreen: {} | Pool avail: {} | Allocs: {} (peak {}) | Mesh ms (avg/latest/peak): {:.1}/{:.1}/{:.1}",
            self.chunks_on_screen,
            self.pool_in_positions,
            self.mesh_allocs,
            self.alloc_high_water,
            self.build_avg,
            self.build_last,
            self.build_peak,
        );
    }
}

/// Simple circular buffer for logging history of mesh build times.
/// This is great for understanding steady-state vs. peak/burst mesh gen.
#[derive(Resource)]
pub struct MeshBuildPerfHistory {
    buckets: Vec<f32>,
    pos: usize,
    count: usize,
}
impl MeshBuildPerfHistory {
    pub fn new(size: usize) -> Self {
        Self {
            buckets: vec![0.0; size],
            pos: 0,
            count: 0,
        }
    }
    pub fn push(&mut self, val: f32) {
        self.buckets[self.pos] = val;
        self.pos = (self.pos + 1) % self.buckets.len();
        if self.count < self.buckets.len() {
            self.count += 1;
        }
    }
    pub fn avg(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            self.buckets.iter().take(self.count).sum::<f32>() / (self.count as f32)
        }
    }
    /// Highest mesh-building time (ms) observed in window (all history).
    pub fn peak(&self) -> f32 {
        self.buckets
            .iter()
            .take(self.count)
            .copied()
            .fold(0.0, f32::max)
    }
}

/// Print key diagnostics to stdout at a throttled interval (every 2 seconds by default).
fn print_render_stats(
    mut timer: Local<Option<Timer>>,
    time: Res<Time>,
    diag: Res<LandChunkMeshDiagnostics>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(2.0, TimerMode::Repeating));
    timer.tick(time.delta());
    if timer.finished() {
        diag.log();
    }
}

// ==================================================================================
//                               HELPER TRAITS / UTILS
// ==================================================================================

// Allow easy [f32; 3] conversion from glam::Vec3 (for shaders/Bevy mesh attributes).
trait _Arrayable {
    fn to_array(&self) -> [f32; 3];
}
impl _Arrayable for Vec3 {
    fn to_array(&self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}
