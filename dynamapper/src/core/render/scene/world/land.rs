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
    pub gx: u32, // chunk grid coordinates (in base 8×8 grid)
    pub gy: u32,
    /// Chunk scale: 1 = standard 8×8, 2 = 16×16, 4 = 32×32, etc. Used to reduce chunks number when using massive zoom-outs.
    /// Determines which mesh and AABB are used.
    pub scale: u32,
}

/// Establishes material, buffer pool, diagnostics, and the draw system.
pub struct DrawLandChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawLandChunkMeshPlugin);

pub fn sys_update_shared_land_material(
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    tile_atlas: Res<tile_atlas::TileAtlas>,
    uniform_state: Res<crate::external_data::shader_presets::UniformState>,
    mut last_atlas_params: Local<Option<tile_atlas::AtlasParams>>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
) {
    let Some(shared_mat) = shared_mat else { return; };

    // Time is now handled by Bevy's built-in `globals.time` uniform in the shader,
    // which is updated automatically every frame WITHOUT triggering material change
    // detection. This eliminates the catastrophic feedback loop where get_mut()
    // marked the material as changed every frame, causing Bevy to re-extract
    // all of the thousands of chunk bind groups.

    let current_global_lighting = uniform_state.global_lighting;
    let current_render_zoom = render_zoom.0;

    let atlas_changed = *last_atlas_params != Some(tile_atlas.params);
    let lighting_meaningfully_changed = (current_global_lighting - *last_global_lighting).abs() > 0.005;
    // TODO: isn't 0.05 too sensitive?
    let zoom_changed = (current_render_zoom - *last_render_zoom).abs() > 0.05;

    // ONLY call get_mut() when something actually changed.
    // This avoids triggering Bevy's asset change detection, which would force
    // re-extraction of the material bind group for all chunk entities.
    if atlas_changed || lighting_meaningfully_changed || zoom_changed {
        if let Some(mat) = materials.get_mut(&shared_mat.0) {
            if lighting_meaningfully_changed {
                mat.extension.scene_uniform.global_lighting = current_global_lighting;
                *last_global_lighting = current_global_lighting;
            }
            if zoom_changed {
                mat.extension.scene_uniform.render_zoom = current_render_zoom;
                *last_render_zoom = current_render_zoom;
            }
            if atlas_changed {
                mat.extension.atlas_params = tile_atlas.params;
                *last_atlas_params = Some(tile_atlas.params);
            }
        }
    }
}

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<draw_mesh::LandMeshScratch>()
            .add_plugins(MaterialPlugin::<LandCustomMeshMaterial>::default())
            // TODO: explain what ExtractResourcePlugin is.
           .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<tile_atlas::TileAtlasImageHandle>::default())
            .add_systems(
                Update,
                (
                    draw_mesh::sys_update_existing_chunk_mesh_lod
                        .in_set(SceneRenderLandSysSet::RenderLandChunks)
                        .after(SceneRenderLandSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame)),
                    draw_mesh::sys_draw_spawned_land_chunks
                        .in_set(SceneRenderLandSysSet::RenderLandChunks)
                        .after(SceneRenderLandSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame)),
                    sys_update_shared_land_material
                        .run_if(in_state(AppState::InGame)),
                ),
            )
            .add_systems(First, tile_atlas::sys_clear_atlas_uploads)
            .add_systems(Startup, setup_base_mesh::setup_land_mesh);
            // Redundant AABB enforcement removed to save CPU/GPU cycles.
            // .add_systems(
            //     PostUpdate,
            //     draw_mesh::sys_enforce_land_chunk_aabb
            //         .run_if(in_state(crate::core::AppState::InGame)),
            // );

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else { return; };
        // TODO: explain why it is needed, where is this resource used.
        render_app.init_resource::<tile_atlas::RenderAtlasUploads>();
        render_app.add_systems(bevy::render::ExtractSchedule, tile_atlas::sys_extract_atlas_uploads);
        render_app.add_systems(bevy::render::Render, tile_atlas::sys_render_upload_tile_atlas.in_set(bevy::render::RenderSystems::Queue));
    }
}
