// render::postprocess::fog_pass — Fullscreen fog post-processing pass plugin.
//
// Implements fog as a fullscreen pass that runs after the main 3D render,
// covering both land terrain and art/sprite tiles uniformly. This solves
// the previous issue where fog was applied individually in each material
// shader, resulting in inconsistent coverage.
//
// Architecture:
// - Uses Bevy's `FullscreenMaterial` trait to register the pass.
// - `FogPostProcessUniform` is a `ShaderType` component on the `PlayerCamera`
//   entity, extracted to the GPU each frame, while the fog is on: without it
//   the pass does not run.
// - `sys_sync_fog_uniform` inserts it from `UniformState`, keeps it in sync,
//   and removes it when the fog is off.

use crate::settings::shader_presets::UniformState;
use crate::render::view::camera::PlayerCamera;
use crate::prelude::*;
use bevy::core_pipeline::{
    fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin},
    tonemapping::tonemapping,
    Core3dSystems,
};
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use bevy::window::Window;

// ---- Plugin ----------------------------------------------------------------

pub struct FogPostProcessPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(FogPostProcessPlugin);

impl Plugin for FogPostProcessPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(FullscreenMaterialPlugin::<FogPostProcessUniform>::default())
            .add_systems(Update, sys_sync_fog_uniform);
    }
}

// ---- Uniform ---------------------------------------------------------------

/// Fog parameters passed to the fullscreen fog shader.
/// Kept in sync with `UniformState.lighting` by `sys_sync_fog_uniform`, on the player camera only while the fog is on.
///
/// Must match the `FogPostProcessUniform` struct layout in `fog_pass.wesl` exactly.
/// All fields must be aligned to 16 bytes per WGSL/encase rules.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
#[extract_app(bevy::render::RenderApp)]
pub struct FogPostProcessUniform {
    // --- Toggle ---
    pub enable_fog: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,

    // --- Colors ---
    pub fog_color: Vec4,
    pub fog_night_color: Vec4,

    // --- Params ---
    // [distance_density, height_density, noise_scale, noise_strength]
    pub fog_params: Vec4,

    // --- Screen size (for radial fog) ---
    pub screen_size: Vec2,
    pub time_seconds: f32,
    pub _pad_time: f32,
}

/// The fog pass's place in `Core3d`, for passes that must run after it (the client night composite).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FogPassSystems;

impl FullscreenMaterial for FogPostProcessUniform {
    fn fragment_shader() -> ShaderRef {
        "shaders/world/effects/fog_pass.wesl".into()
    }

    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        // Run after tonemapping so fog is applied on the fully composed frame.
        // Upscaling (the final blit) runs after the whole `PostProcess` set.
        system.in_set(Core3dSystems::PostProcess).in_set(FogPassSystems).after(tonemapping)
    }
}

// ---- System ----------------------------------------------------------------

/// Syncs `UniformState.lighting` fog fields into the `FogPostProcessUniform`
/// component on the `PlayerCamera` entity so the GPU gets updated values. With the fog off the component is removed,
/// so the fullscreen pass, which would pass every pixel through unchanged, does not run.
fn sys_sync_fog_uniform(
    mut commands: Commands,
    uniform_state: Res<UniformState>,
    time: Res<Time>,
    windows: Query<&Window>,
    mut camera_q: Query<(Entity, Option<&mut FogPostProcessUniform>), With<PlayerCamera>>,
) {
    let Ok((camera, fog_uniform)) = camera_q.single_mut() else {
        return;
    };
    let lighting = &uniform_state.lighting;
    if lighting.enable_fog == 0 {
        if fog_uniform.is_some() {
            commands.entity(camera).remove::<FogPostProcessUniform>();
        }
        return;
    }
    let dynamic_fog = lighting.fog_params.z > 0.001 && lighting.fog_params.w > 0.001;
    if fog_uniform.is_some() && !uniform_state.is_changed() && !dynamic_fog {
        return;
    }

    let screen_size = windows
        .iter()
        .next()
        .map(|w| Vec2::new(w.resolution.width(), w.resolution.height()))
        .unwrap_or(Vec2::ONE);
    let uniform = FogPostProcessUniform {
        enable_fog: lighting.enable_fog,
        fog_color: lighting.fog_color,
        fog_night_color: lighting.fog_night_color,
        fog_params: lighting.fog_params,
        screen_size,
        time_seconds: time.elapsed_secs(),
        ..default()
    };
    match fog_uniform {
        Some(mut fog_uniform) => *fog_uniform = uniform,
        None => {
            commands.entity(camera).insert(uniform);
        }
    }
}
