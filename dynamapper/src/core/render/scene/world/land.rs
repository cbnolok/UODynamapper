pub mod draw_mesh;
pub mod mesh_material;
pub mod setup_base_mesh;
pub mod tile_atlas;

use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use mesh_material::LandCustomMeshMaterial;

/// How many tiles per chunk row/column? (chunks are squared)
pub const TILE_NUM_PER_CHUNK_DIM: u32 = 8;
/// How many tiles in one chunk total?
pub const TILE_NUM_PER_CHUNK_TOTAL: usize =
    (TILE_NUM_PER_CHUNK_DIM * TILE_NUM_PER_CHUNK_DIM) as usize;

/// Tag component: Marks entities which are Land Chunk Meshes, allows queries for those entities.
#[derive(Component)]
pub struct LCMesh {
    #[allow(unused)]
    pub parent_map_id: u32,
    pub gx: u32, // chunk grid coordinates
    pub gy: u32,
}

/// Establishes material, buffer pool, diagnostics, and the draw system.
pub struct DrawLandChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawLandChunkMeshPlugin);

pub fn sys_update_shared_land_material(
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    time: Res<Time>,
    tile_atlas: Res<tile_atlas::TileAtlas>,
    uniform_state: Res<crate::external_data::shader_presets::UniformState>,
    mut last_atlas_params: Local<Option<tile_atlas::AtlasParams>>,
    mut last_time: Local<f32>,
    mut last_global_lighting: Local<f32>,
) {
    if let Some(shared_mat) = shared_mat {
        // We only use get_mut if we actually intend to change something.
        // Even for time, we check if it changed.
        let current_time = time.elapsed().as_secs_f32();
        let current_global_lighting = uniform_state.global_lighting;
        
        // AtlasParams update check
        let atlas_changed = *last_atlas_params != Some(tile_atlas.params);
        let time_changed = (current_time - *last_time).abs() > 0.0001;
        let lighting_changed = (current_global_lighting - *last_global_lighting).abs() > 0.0001;

        if atlas_changed || time_changed || lighting_changed {
            if let Some(mat) = materials.get_mut(&shared_mat.0) {
                if time_changed {
                    mat.extension.scene_uniform.time_seconds = current_time;
                    *last_time = current_time;
                }
                if lighting_changed {
                    mat.extension.scene_uniform.global_lighting = current_global_lighting;
                    *last_global_lighting = current_global_lighting;
                }
                if atlas_changed {
                    mat.extension.atlas_params = tile_atlas.params;
                    *last_atlas_params = Some(tile_atlas.params);
                }
                
                // Note: effects_uniform and lighting_uniform are handled by TerrainUiPlugin::push_uniforms_if_dirty
                // which monitors the UniformState resource and its 'dirty' flag.
            }
        }
    }
}

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<LandCustomMeshMaterial>::default())
           .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<tile_atlas::TileAtlasImageHandle>::default())
            .add_systems(
                Update,
                (
                    draw_mesh::sys_draw_spawned_land_chunks
                        .in_set(SceneRenderLandSysSet::RenderLandChunks)
                        .after(SceneRenderLandSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame)),
                    sys_update_shared_land_material
                        .run_if(in_state(AppState::InGame)),
                ),
            )
            .add_systems(First, tile_atlas::sys_clear_atlas_uploads)
            .add_systems(Startup, setup_base_mesh::setup_land_mesh)
            // Run after Bevy's built-in compute_bounds system to guarantee our manual
            // AABB is never overwritten by automatic computation from the flat mesh vertices.
            .add_systems(
                PostUpdate,
                draw_mesh::sys_enforce_land_chunk_aabb
                    .run_if(in_state(crate::core::AppState::InGame)),
            );

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else { return; };
        render_app.init_resource::<tile_atlas::RenderAtlasUploads>();
        render_app.add_systems(bevy::render::ExtractSchedule, tile_atlas::sys_extract_atlas_uploads);
        render_app.add_systems(bevy::render::Render, tile_atlas::sys_render_upload_tile_atlas.in_set(bevy::render::RenderSystems::Queue));
    }
}
