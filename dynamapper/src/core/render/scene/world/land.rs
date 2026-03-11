pub mod draw_mesh;
pub mod mesh_material;
pub mod setup_base_mesh;
pub mod tile_atlas;

use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use mesh_material::LandCustomMaterial;

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
    mut materials: ResMut<Assets<LandCustomMaterial>>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    time: Res<Time>,
    presets: Res<mesh_material::LandShaderModePresets>,
    tile_atlas: Res<tile_atlas::TileAtlas>,
) {
    if let Some(shared_mat) = shared_mat {
        if let Some(mat) = materials.get_mut(&shared_mat.0) {
            mat.extension.scene_uniform.time_seconds = time.elapsed().as_secs_f32();
            let preset = &presets.classic.morning; // dynamically selected based on logic later
            mat.extension.effects_uniform = preset.effects;
            mat.extension.lighting_uniform = preset.lighting;
            mat.extension.atlas_params = tile_atlas.params;
        }
    }
}

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<LandCustomMaterial>::default())
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
            .add_systems(Startup, setup_base_mesh::setup_land_mesh);

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else { return; };
        render_app.init_resource::<tile_atlas::RenderAtlasUploads>();
        render_app.add_systems(bevy::render::ExtractSchedule, tile_atlas::sys_extract_atlas_uploads);
        render_app.add_systems(bevy::render::Render, tile_atlas::sys_render_upload_tile_atlas.in_set(bevy::render::RenderSet::Queue));
    }
}
