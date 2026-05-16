// fog_post_process.rs — Fullscreen fog post-processing pass plugin.
//
// Implements fog as a fullscreen pass that runs after the main 3D render,
// covering both land terrain and art/sprite tiles uniformly. This solves
// the previous issue where fog was applied individually in each material
// shader, resulting in inconsistent coverage.
//
// Architecture:
// - Uses Bevy 0.18's `FullscreenMaterial` trait to register the pass.
// - `FogPostProcessUniform` is a `ShaderType` component attached to the
//   `PlayerCamera` entity and extracted to the GPU each frame.
// - `sys_sync_fog_uniform` reads from `UniformState` each frame and
//   writes to `FogPostProcessUniform`, keeping them in sync.

use crate::configs::shader_presets::UniformState;
use crate::core::render::scene::camera::PlayerCamera;
use crate::prelude::*;
use bevy::core_pipeline::{
    core_3d::graph::Node3d,
    fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin},
};
use bevy::prelude::*;
use bevy::render::{extract_component::ExtractComponent, render_graph::InternedRenderLabel, render_graph::RenderLabel};
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
/// Kept in sync each frame with `UniformState.lighting` by `sys_sync_fog_uniform`.
///
/// Must match the `FogPostProcessUniform` struct layout in `fog_pass.wgsl` exactly.
/// All fields must be aligned to 16 bytes per WGSL/encase rules.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
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
    pub _pad_ss: Vec2,
}

impl FullscreenMaterial for FogPostProcessUniform {
    fn fragment_shader() -> ShaderRef {
        "shaders/worldmap/fog_pass.wgsl".into()
    }

    fn node_edges() -> Vec<InternedRenderLabel> {
        // Run after tonemapping so fog is applied on the fully composed frame,
        // before the final end-of-post-processing blit.
        vec![
            Node3d::Tonemapping.intern(),
            Self::node_label().intern(),
            Node3d::EndMainPassPostProcessing.intern(),
        ]
    }
}

// ---- System ----------------------------------------------------------------

/// Syncs `UniformState.lighting` fog fields into the `FogPostProcessUniform`
/// component on the `PlayerCamera` entity so the GPU gets updated values.
fn sys_sync_fog_uniform(
    uniform_state: Res<UniformState>,
    windows: Query<&Window>,
    mut camera_q: Query<&mut FogPostProcessUniform, With<PlayerCamera>>,
) {
    if !uniform_state.is_changed() {
        return;
    }
    let Ok(mut fog_uniform) = camera_q.single_mut() else {
        return;
    };
    let lighting = &uniform_state.lighting;

    let screen_size = windows
        .iter()
        .next()
        .map(|w| Vec2::new(w.resolution.width(), w.resolution.height()))
        .unwrap_or(Vec2::ONE);

    fog_uniform.enable_fog = lighting.enable_fog;
    fog_uniform.fog_color = lighting.fog_color;
    fog_uniform.fog_night_color = lighting.fog_night_color;
    fog_uniform.fog_params = lighting.fog_params;
    fog_uniform.screen_size = screen_size;
}
