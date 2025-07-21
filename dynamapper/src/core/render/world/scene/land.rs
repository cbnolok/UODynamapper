pub mod diagnostics;
pub mod draw_chunk_mesh;
pub mod mesh_buffer_pool;
pub mod mesh_material;

use bevy::prelude::*;
use crate::prelude::*;
use crate::core::system_sets::*;
use mesh_material::LandCustomMaterial;
use mesh_buffer_pool::LandChunkMeshBufferPool;
use diagnostics::*;


/// How many tiles per chunk row/column? (chunks are squared)
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
                    draw_chunk_mesh::sys_draw_spawned_land_chunks
                        .in_set(SceneRenderSysSet::RenderLandChunks)
                        .after(SceneRenderSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame)),
                    //print_render_stats,
                ),
            );
    }
}

