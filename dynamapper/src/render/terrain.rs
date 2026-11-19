pub mod cave_mouth_veil;
pub mod chunk_loader;
pub mod coverage;
pub mod draw_mesh;
pub mod mesh_material;
pub mod setup_base_mesh;

use crate::app::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use mesh_material::{LandCustomMeshMaterial, LandStaticLightUniform, LAND_STATIC_LIGHT_MAX};
use crate::render::assets::tile_atlas::{self, LandUploadBudget, LandUploadTelemetry};

/// Native map storage block size in tiles.
pub const MAP_STORAGE_BLOCK_TILE_DIM: u32 = 8;
/// How many tiles are stored in one native map block.
pub const MAP_STORAGE_BLOCK_TILE_TOTAL: usize =
    (MAP_STORAGE_BLOCK_TILE_DIM * MAP_STORAGE_BLOCK_TILE_DIM) as usize;
/// How many tiles per logical terrain chunk row/column? (chunks are squared)
pub const TILE_NUM_PER_CHUNK_DIM: u32 = 32;
/// How many native 8x8 map blocks fit in one logical chunk row/column?
pub const CHUNK_STORAGE_BLOCKS_DIM: u32 = TILE_NUM_PER_CHUNK_DIM / MAP_STORAGE_BLOCK_TILE_DIM;

/// Tag component: Marks entities which are Land Chunk Meshes, allows queries for those entities.
#[derive(Component)]
pub struct LCMesh {
    #[allow(unused)]
    pub parent_map_id: u32,
    pub gx: u32, // chunk grid coordinates (in base 32×32 grid)
    pub gy: u32,
    /// Determines which mesh and AABB are used.
    pub scale: u32,
    /// Last blocks_loaded_version checked from MapPlane. Used to skip is_block_cached polling.
    /// Recorded only while the chunk misses blocks: those come only with a load, which bumps the version. An eviction
    /// does not bump it, and need not: it only takes more blocks away, which the recheck after that load finds
    /// missing and dispatches too.
    pub last_blocks_loaded_version: Option<u64>,
}

/// Synchronize the extracted upload budget resource with the current settings.
///
/// The resource is cheap to copy and is used by both main-world terrain preparation and
/// render-world atlas uploads, so we update it whenever settings change instead of making
/// those systems reach into the full Settings resource directly.
fn sys_sync_land_upload_budget(
    settings: Res<crate::settings::Settings>,
    mut upload_budget: ResMut<LandUploadBudget>,
) {
    if !settings.is_changed() && !upload_budget.is_added() {
        return;
    }

    log_system_add_update::<DrawLandChunkMeshPlugin>(fname!());

    *upload_budget = LandUploadBudget::from_settings(&settings);
}

fn sys_apply_land_shader_simplification_override(
    settings: Res<crate::settings::Settings>,
    uniform_state: Res<crate::settings::shader_presets::UniformState>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    mut last_override: Local<Option<crate::settings::SectWorldRenderingShaderSimplification>>,
) {
    let override_config = settings.world_rendering.shader_simplification.clone();
    let force_changed = *last_override != Some(override_config.clone());
    let force_minimal = override_config.force_minimal_shader;
    let force_flat_fragment = override_config.force_flat_fragment_shader;
    let any_override = force_minimal || force_flat_fragment;

    if !force_changed && !(any_override && uniform_state.is_changed()) {
        return;
    }

    log_system_add_update::<DrawLandChunkMeshPlugin>(fname!());

    let Some(shared_mat) = shared_mat else {
        return;
    };
    let Some(mut mat) = materials.get_mut(&shared_mat.0) else {
        return;
    };

    if force_flat_fragment || force_minimal {
        mat.extension.effects_uniform.shading_mode = if force_flat_fragment { 3 } else { 0 };
        mat.extension.effects_uniform.normal_mode = 0;
        mat.extension.effects_uniform.enable_blur = 0;
        mat.extension.effects_uniform.enable_linear_filtering = 0;
        mat.extension.effects_uniform.sharpening_amount = 0.0;
        mat.extension.effects_uniform.blur_strength = 0.0;
        mat.extension.effects_uniform.blur_radius = 0.0;

        mat.extension.global_lighting_uniform.enable_fog = 0;
        mat.extension.global_lighting_uniform.enable_grading = 0;
        mat.extension.global_lighting_uniform.enable_gloom = 0;

        mat.extension.land_lighting_uniform.enable_bent = 0;
        mat.extension.land_lighting_uniform.specular_strength = 0.0;
        mat.extension.land_lighting_uniform.rim_strength = 0.0;
        mat.extension.land_lighting_uniform.fill_strength = 0.0;
        mat.extension.land_lighting_uniform.sharpness_mix = 0.0;

        // Make the fragment path take the intentionally cheap branch even when zoom would
        // normally keep full-res sampling.
        mat.extension.scene_uniform.adaptive_zoom_simplification = 1.0;

        if force_changed {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldLand,
                if force_flat_fragment {
                    "Forced flat land fragment shader override enabled."
                } else {
                    "Forced minimal land shader override enabled."
                },
            );
        }
    } else {
        mat.extension.effects_uniform = uniform_state.effects;
        mat.extension.global_lighting_uniform = uniform_state.lighting;
        mat.extension.land_lighting_uniform = uniform_state.land_lighting;
        mat.extension.scene_uniform.global_lighting = uniform_state.global_lighting;
        mat.extension.scene_uniform.light_direction = uniform_state.sun.light_direction();
        mat.extension.scene_uniform.adaptive_zoom_simplification = 0.0;

        if force_changed {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldLand,
                "Forced land shader override disabled.",
            );
        }
    }

    *last_override = Some(override_config);
}

/// Establishes material, buffer pool, diagnostics, and the draw system.
pub struct DrawLandChunkMeshPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawLandChunkMeshPlugin);

pub fn sys_update_shared_land_material(
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    render_zoom: Res<crate::render::view::camera::RenderZoom>,
    tile_atlas: Res<tile_atlas::TileAtlas>,
    uniform_state: Res<crate::settings::shader_presets::UniformState>,
    static_lights: Option<Res<crate::render::lighting::static_lights::RenderStaticLightInstances>>,
    player_q: Query<&Transform, With<crate::render::scene::player::Player>>,
    camera_q: Query<&Projection, With<crate::render::view::camera::PlayerCamera>>,
    view_cuts: (
        Res<crate::world::z_view::ZViewLimit>,
        Res<crate::world::caves::CavesRes>,
    ),
    mut last_atlas_params: Local<Option<tile_atlas::AtlasParams>>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
    mut last_static_light_signature: Local<u64>,
    mut last_client_view_scale: Local<f32>,
    mut last_land_max_z: Local<Option<f32>>,
    mut last_land_outside_dimming: Local<f32>,
) {
    let Some(shared_mat) = shared_mat else {
        return;
    };
    let (z_view, caves) = view_cuts;

    // Time is now handled by Bevy's built-in `globals.time` uniform in the shader,
    // which is updated automatically every frame WITHOUT triggering material change
    // detection. This eliminates the catastrophic feedback loop where get_mut()
    // marked the material as changed every frame, causing Bevy to re-extract
    // all of the thousands of chunk bind groups.

    let current_global_lighting = uniform_state.global_lighting;
    let current_render_zoom = render_zoom.0;

    let atlas_changed = *last_atlas_params != Some(tile_atlas.params);
    let lighting_meaningfully_changed =
        (current_global_lighting - *last_global_lighting).abs() > 0.01;
    let zoom_changed = (current_render_zoom - *last_render_zoom).abs() > 0.1;
    let current_client_view_scale =
        camera_q.iter().next().map_or(1.0, mesh_material::client_view_scale);
    let client_view_scale_changed = (current_client_view_scale - *last_client_view_scale).abs()
        > mesh_material::CLIENT_VIEW_SCALE_EPSILON;
    let static_light_focus_xz = player_q
        .iter()
        .next()
        .map(|t| Vec2::new(t.translation.x, t.translation.z));
    let static_light_uniform =
        build_land_static_light_uniform(static_lights.as_deref(), static_light_focus_xz);
    let static_light_signature = land_static_light_signature(&static_light_uniform);
    let static_lights_changed = static_light_signature != *last_static_light_signature;
    let current_land_max_z = z_view.0.land_max_z();
    let land_max_z_changed = *last_land_max_z != Some(current_land_max_z);
    let current_land_outside_dimming = caves.land_outside_dimming();
    let land_outside_dimming_changed = current_land_outside_dimming != *last_land_outside_dimming;

    // ONLY call get_mut() when something actually changed.
    // This avoids triggering Bevy's asset change detection, which would force
    // re-extraction of the material bind group for all chunk entities.
    if atlas_changed
        || lighting_meaningfully_changed
        || zoom_changed
        || static_lights_changed
        || client_view_scale_changed
        || land_max_z_changed
        || land_outside_dimming_changed
    {
        if let Some(mut mat) = materials.get_mut(&shared_mat.0) {
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
            if static_lights_changed {
                mat.extension.static_light_uniform = static_light_uniform;
                *last_static_light_signature = static_light_signature;
            }
            if client_view_scale_changed {
                mat.extension.scene_uniform.client_view_scale = current_client_view_scale;
                *last_client_view_scale = current_client_view_scale;
            }
            if land_max_z_changed {
                mat.extension.scene_uniform.land_max_z = current_land_max_z;
                *last_land_max_z = Some(current_land_max_z);
            }
            if land_outside_dimming_changed {
                mat.extension.scene_uniform.land_outside_dimming = current_land_outside_dimming;
                *last_land_outside_dimming = current_land_outside_dimming;
            }
        }
    }
}

fn build_land_static_light_uniform(
    static_lights: Option<&crate::render::lighting::static_lights::RenderStaticLightInstances>,
    focus_xz: Option<Vec2>,
) -> LandStaticLightUniform {
    let mut uniform = LandStaticLightUniform::default();
    let Some(static_lights) = static_lights else {
        return uniform;
    };

    let mut selected_lights = static_lights.0.iter().collect::<Vec<_>>();
    selected_lights.sort_by(|a, b| {
        let a_score = land_static_light_selection_score(a, focus_xz);
        let b_score = land_static_light_selection_score(b, focus_xz);
        b_score
            .total_cmp(&a_score)
            .then_with(|| a.key.tile_y.cmp(&b.key.tile_y))
            .then_with(|| a.key.tile_x.cmp(&b.key.tile_x))
            .then_with(|| a.key.z.cmp(&b.key.z))
            .then_with(|| a.key.graphic.cmp(&b.key.graphic))
            .then_with(|| a.key.light_id.cmp(&b.key.light_id))
            .then_with(|| a.key.hue_id.cmp(&b.key.hue_id))
    });

    let light_count = selected_lights.len().min(LAND_STATIC_LIGHT_MAX);
    for ((dst, color_dst), light) in uniform
        .lights
        .iter_mut()
        .zip(uniform.colors.iter_mut())
        .zip(selected_lights.into_iter().take(light_count))
    {
        let radius = land_static_light_radius(light);
        *dst = Vec4::new(light.world_x, light.world_y, light.world_z, radius);
        *color_dst = Vec4::new(light.color_rgb[0], light.color_rgb[1], light.color_rgb[2], 0.0);
    }
    uniform.params = UVec4::new(light_count as u32, 0, 0, 0);
    uniform
}

fn land_static_light_radius(
    light: &crate::render::lighting::static_lights::StaticLightInstance,
) -> f32 {
    light.width_world.max(light.height_world).max(1.0) * 0.62 + 1.25
}

fn land_static_light_selection_score(
    light: &crate::render::lighting::static_lights::StaticLightInstance,
    focus_xz: Option<Vec2>,
) -> f32 {
    let radius = land_static_light_radius(light);
    let strength = radius * radius;
    let Some(focus_xz) = focus_xz else {
        return strength;
    };

    let dx = light.world_x - focus_xz.x;
    let dz = light.world_z - focus_xz.y;
    let distance_sq = dx * dx + dz * dz;
    strength / (1.0 + distance_sq / strength.max(1.0))
}

fn land_static_light_signature(uniform: &LandStaticLightUniform) -> u64 {
    let mut signature = uniform.params.x as u64;
    for light in uniform.lights.iter().take(uniform.params.x as usize) {
        for value in [
            (light.x * 100.0).round() as i32,
            (light.y * 100.0).round() as i32,
            (light.z * 100.0).round() as i32,
            (light.w * 100.0).round() as i32,
        ] {
            signature ^= value as u32 as u64;
            signature = signature.wrapping_mul(0x100000001b3);
        }
    }
    for color in uniform.colors.iter().take(uniform.params.x as usize) {
        for value in [
            (color.x * 255.0).round() as i32,
            (color.y * 255.0).round() as i32,
            (color.z * 255.0).round() as i32,
        ] {
            signature ^= value as u32 as u64;
            signature = signature.wrapping_mul(0x100000001b3);
        }
    }
    signature
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::lighting::static_lights::{
        RenderStaticLightInstances, StaticLightInstance, StaticLightKey,
    };

    fn test_light(tile_x: u32, tile_y: u32, width_world: f32) -> StaticLightInstance {
        StaticLightInstance {
            key: StaticLightKey {
                map_id: 0,
                tile_x,
                tile_y,
                z: 0,
                graphic: tile_x as u16,
                light_id: tile_x,
                hue_id: 0,
            },
            light_id: tile_x,
            hue_id: 0,
            color_rgb: [1.0, 0.72, 0.42],
            world_x: tile_x as f32,
            world_z: tile_y as f32,
            world_y: 0.0,
            width_world,
            height_world: width_world,
            ceiling: None,
            cave_outside: false,
        }
    }

    #[test]
    fn land_static_light_uniform_prefers_near_focus_lights() {
        let mut lights = vec![test_light(1000, 1000, 1.0)];
        for i in 0..LAND_STATIC_LIGHT_MAX {
            lights.push(test_light(i as u32, 0, 1.0));
        }

        let uniform = build_land_static_light_uniform(
            Some(&RenderStaticLightInstances(lights)),
            Some(Vec2::ZERO),
        );

        assert_eq!(uniform.params.x, LAND_STATIC_LIGHT_MAX as u32);
        assert!(
            uniform
                .lights
                .iter()
                .take(uniform.params.x as usize)
                .all(|light| light.x < 1000.0)
        );
    }

    #[test]
    fn land_static_light_uniform_without_focus_prefers_stronger_lights() {
        let mut lights = (0..LAND_STATIC_LIGHT_MAX)
            .map(|i| test_light(i as u32, 0, 1.0))
            .collect::<Vec<_>>();
        lights.push(test_light(1000, 1000, 8.0));

        let uniform = build_land_static_light_uniform(
            Some(&RenderStaticLightInstances(lights)),
            None,
        );

        assert_eq!(uniform.params.x, LAND_STATIC_LIGHT_MAX as u32);
        assert!(
            uniform
                .lights
                .iter()
                .take(uniform.params.x as usize)
                .any(|light| light.x == 1000.0)
        );
    }
}

use bevy::time::common_conditions::on_real_timer;
use std::time::Duration;

impl Plugin for DrawLandChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        crate::render::picking::terrain_cursor::install(app);
        app.init_resource::<draw_mesh::LandMeshScratch>()
            .init_resource::<LandUploadBudget>()
            .init_resource::<LandUploadTelemetry>()
            .add_plugins(MaterialPlugin::<LandCustomMeshMaterial>::default())
            // Copies the TileAtlasImageHandle resource from the main world into the
            // render world every frame, so that the GPU pipeline can access the atlas texture.
            .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
                tile_atlas::TileAtlasImageHandle,
            >::default())
            .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
                LandUploadBudget,
            >::default())
            .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
                LandUploadTelemetry,
            >::default())
            .add_systems(
                Update,
                (
                    sys_sync_land_upload_budget.before(SceneRenderLandSysSet::RenderLandChunks),
                    draw_mesh::sys_update_existing_chunk_mesh_lod
                        .in_set(SceneRenderLandSysSet::RenderLandChunks)
                        .after(SceneRenderLandSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame))
                        .run_if(on_real_timer(Duration::from_secs_f32(1.0 / 4.0))),
                    draw_mesh::sys_draw_spawned_land_chunks
                        .in_set(SceneRenderLandSysSet::RenderLandChunks)
                        .after(SceneRenderLandSysSet::SyncLandChunks)
                        .run_if(in_state(AppState::InGame)),
                    sys_update_shared_land_material.run_if(in_state(AppState::InGame)),
                    sys_apply_land_shader_simplification_override
                        .run_if(in_state(AppState::InGame))
                        .after(sys_update_shared_land_material),
                    cave_mouth_veil::sys_sync_cave_mouth_veil_material
                        .run_if(in_state(AppState::InGame))
                        .after(sys_apply_land_shader_simplification_override),
                    cave_mouth_veil::sys_sync_cave_mouth_veils
                        .after(SceneRenderLandSysSet::RenderLandChunks)
                        .after(cave_mouth_veil::sys_sync_cave_mouth_veil_material)
                        .run_if(in_state(AppState::InGame)),
                ),
            )
            .add_systems(First, tile_atlas::sys_clear_atlas_uploads)
            .add_systems(Startup, setup_base_mesh::setup_land_mesh);

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        // Render-world counterpart that receives tile atlas upload commands extracted
        // from the main world. Consumed by sys_render_upload_tile_atlas during the Queue phase.
        render_app.init_resource::<tile_atlas::RenderAtlasUploads>();
        render_app.add_systems(
            bevy::render::ExtractSchedule,
            tile_atlas::sys_extract_atlas_uploads,
        );
        render_app.add_systems(
            bevy::render::Render,
            tile_atlas::sys_render_upload_tile_atlas.in_set(bevy::render::RenderSystems::Queue),
        );
    }
}
