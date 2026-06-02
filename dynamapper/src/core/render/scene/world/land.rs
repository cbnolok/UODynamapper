pub mod chunk_loader;
pub mod draw_mesh;
pub mod mesh_material;
pub mod profiling;
pub mod setup_base_mesh;
pub mod tile_atlas;

use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use mesh_material::{LandCustomMeshMaterial, LandStaticLightUniform, LAND_STATIC_LIGHT_MAX};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Native map storage block size in tiles.
pub const MAP_STORAGE_BLOCK_TILE_DIM: u32 = 8;
/// How many tiles are stored in one native map block.
pub const MAP_STORAGE_BLOCK_TILE_TOTAL: usize =
    (MAP_STORAGE_BLOCK_TILE_DIM * MAP_STORAGE_BLOCK_TILE_DIM) as usize;
/// How many tiles per logical terrain chunk row/column? (chunks are squared)
pub const TILE_NUM_PER_CHUNK_DIM: u32 = 32;
/// How many native 8x8 map blocks fit in one logical chunk row/column?
pub const CHUNK_STORAGE_BLOCKS_DIM: u32 = TILE_NUM_PER_CHUNK_DIM / MAP_STORAGE_BLOCK_TILE_DIM;
/// How many tiles in one logical chunk total?
pub const TILE_NUM_PER_CHUNK_TOTAL: usize =
    (TILE_NUM_PER_CHUNK_DIM * TILE_NUM_PER_CHUNK_DIM) as usize;

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
    pub last_blocks_loaded_version: Option<u64>,
}

/// Runtime terrain upload limits copied from developer-facing worldmap settings.
///
/// The same resource is used in both worlds:
/// - Main world: caps how much terrain preparation work `draw_mesh` can schedule.
/// - Render world: caps how many atlas writes can be flushed in a single Queue pass.
///
/// Keeping the values in one explicit resource makes the upload path easier to inspect
/// and avoids hiding frame-pacing behavior behind scattered constants.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, ExtractResource)]
pub struct LandUploadBudget {
    pub prepare_max_blocks_per_frame: usize,
    pub upload_max_ops_per_frame: usize,
    pub upload_max_bytes_per_frame: usize,
}

impl Default for LandUploadBudget {
    fn default() -> Self {
        Self {
            prepare_max_blocks_per_frame: 32_768,
            upload_max_ops_per_frame: 8,
            upload_max_bytes_per_frame: 16 * 1024 * 1024,
        }
    }
}

impl LandUploadBudget {
    pub fn from_settings(settings: &crate::configs::settings::Settings) -> Self {
        let land_streaming = &settings.world_rendering.land_streaming;
        Self {
            prepare_max_blocks_per_frame: land_streaming.prepare_max_blocks_per_frame,
            upload_max_ops_per_frame: land_streaming.upload_max_ops_per_frame,
            upload_max_bytes_per_frame: land_streaming.upload_max_bytes_per_frame,
        }
    }

    pub fn effective_prepare_max_blocks_per_frame(&self) -> usize {
        if self.prepare_max_blocks_per_frame == 0 {
            usize::MAX
        } else {
            self.prepare_max_blocks_per_frame
        }
    }

    pub fn effective_upload_max_ops_per_frame(&self) -> usize {
        if self.upload_max_ops_per_frame == 0 {
            usize::MAX
        } else {
            self.upload_max_ops_per_frame
        }
    }

    pub fn effective_upload_max_bytes_per_frame(&self) -> usize {
        if self.upload_max_bytes_per_frame == 0 {
            usize::MAX
        } else {
            self.upload_max_bytes_per_frame
        }
    }
}

#[derive(Default)]
struct LandUploadTelemetryShared {
    dirty_block_updates: AtomicU32,
    queued_ops: AtomicU32,
    queued_bytes: AtomicU64,
    submitted_ops: AtomicU32,
    submitted_bytes: AtomicU64,
    pending_ops: AtomicU32,
    pending_bytes: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LandUploadTelemetrySnapshot {
    pub dirty_block_updates: u32,
    pub queued_ops: u32,
    pub queued_bytes: u64,
    pub submitted_ops: u32,
    pub submitted_bytes: u64,
    pub pending_ops: u32,
    pub pending_bytes: u64,
}

/// Shared telemetry for the land metadata-atlas upload path.
///
/// The main world populates the queued counters after dirty regions are gathered, while the
/// render world updates the submitted and backlog counters after applying the per-frame caps.
/// We store the counters in an `Arc` so both worlds see the same numbers without introducing
/// a bespoke cross-world messaging path.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct LandUploadTelemetry {
    shared: Arc<LandUploadTelemetryShared>,
}

impl LandUploadTelemetry {
    pub fn snapshot(&self) -> LandUploadTelemetrySnapshot {
        LandUploadTelemetrySnapshot {
            dirty_block_updates: self.shared.dirty_block_updates.load(Ordering::Relaxed),
            queued_ops: self.shared.queued_ops.load(Ordering::Relaxed),
            queued_bytes: self.shared.queued_bytes.load(Ordering::Relaxed),
            submitted_ops: self.shared.submitted_ops.load(Ordering::Relaxed),
            submitted_bytes: self.shared.submitted_bytes.load(Ordering::Relaxed),
            pending_ops: self.shared.pending_ops.load(Ordering::Relaxed),
            pending_bytes: self.shared.pending_bytes.load(Ordering::Relaxed),
        }
    }

    pub fn record_stage(&self, dirty_block_updates: u32, queued_ops: u32, queued_bytes: u64) {
        self.shared
            .dirty_block_updates
            .store(dirty_block_updates, Ordering::Relaxed);
        self.shared.queued_ops.store(queued_ops, Ordering::Relaxed);
        self.shared
            .queued_bytes
            .store(queued_bytes, Ordering::Relaxed);
    }

    pub fn record_submit(
        &self,
        submitted_ops: u32,
        submitted_bytes: u64,
        pending_ops: u32,
        pending_bytes: u64,
    ) {
        self.shared
            .submitted_ops
            .store(submitted_ops, Ordering::Relaxed);
        self.shared
            .submitted_bytes
            .store(submitted_bytes, Ordering::Relaxed);
        self.shared
            .pending_ops
            .store(pending_ops, Ordering::Relaxed);
        self.shared
            .pending_bytes
            .store(pending_bytes, Ordering::Relaxed);
    }
}

/// Synchronize the extracted upload budget resource with the current settings.
///
/// The resource is cheap to copy and is used by both main-world terrain preparation and
/// render-world atlas uploads, so we update it whenever settings change instead of making
/// those systems reach into the full Settings resource directly.
fn sys_sync_land_upload_budget(
    settings: Res<crate::configs::settings::Settings>,
    mut upload_budget: ResMut<LandUploadBudget>,
) {
    if !settings.is_changed() && !upload_budget.is_added() {
        return;
    }

    log_system_add_update::<DrawLandChunkMeshPlugin>(fname!());

    *upload_budget = LandUploadBudget::from_settings(&settings);
}

fn sys_apply_land_shader_simplification_override(
    settings: Res<crate::configs::settings::Settings>,
    uniform_state: Res<crate::configs::shader_presets::UniformState>,
    shared_mat: Option<Res<draw_mesh::SharedLandMaterial>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    mut last_override: Local<Option<crate::configs::settings::SectWorldRenderingShaderSimplification>>,
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
    let Some(mat) = materials.get_mut(&shared_mat.0) else {
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
    render_zoom: Res<crate::core::render::scene::camera::RenderZoom>,
    tile_atlas: Res<tile_atlas::TileAtlas>,
    uniform_state: Res<crate::configs::shader_presets::UniformState>,
    static_lights: Option<Res<crate::core::render::scene::world::art::static_lights::RenderStaticLightInstances>>,
    player_q: Query<&Transform, With<crate::core::render::scene::player::Player>>,
    mut last_atlas_params: Local<Option<tile_atlas::AtlasParams>>,
    mut last_global_lighting: Local<f32>,
    mut last_render_zoom: Local<f32>,
    mut last_static_light_signature: Local<u64>,
) {
    let Some(shared_mat) = shared_mat else {
        return;
    };

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
    let static_light_focus_xz = player_q
        .iter()
        .next()
        .map(|t| Vec2::new(t.translation.x, t.translation.z));
    let static_light_uniform =
        build_land_static_light_uniform(static_lights.as_deref(), static_light_focus_xz);
    let static_light_signature = land_static_light_signature(&static_light_uniform);
    let static_lights_changed = static_light_signature != *last_static_light_signature;

    // ONLY call get_mut() when something actually changed.
    // This avoids triggering Bevy's asset change detection, which would force
    // re-extraction of the material bind group for all chunk entities.
    if atlas_changed || lighting_meaningfully_changed || zoom_changed || static_lights_changed {
        if let Some(mat) = materials.get_mut(&shared_mat.0) {
            /*
            console_logger::one(
                None,
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "[DBG-Mat] atlas={} lighting={} (diff={:.4}) zoom={} (diff={:.4})",
                    atlas_changed,
                    lighting_meaningfully_changed,
                    (current_global_lighting - *last_global_lighting).abs(),
                    zoom_changed,
                    (current_render_zoom - *last_render_zoom).abs()
                ),
            );
            */
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
        }
    }
}

fn build_land_static_light_uniform(
    static_lights: Option<&crate::core::render::scene::world::art::static_lights::RenderStaticLightInstances>,
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
    light: &crate::core::render::scene::world::art::static_lights::StaticLightInstance,
) -> f32 {
    light.width_world.max(light.height_world).max(1.0) * 0.62 + 1.25
}

fn land_static_light_selection_score(
    light: &crate::core::render::scene::world::art::static_lights::StaticLightInstance,
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
    use crate::core::render::scene::world::art::static_lights::{
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
