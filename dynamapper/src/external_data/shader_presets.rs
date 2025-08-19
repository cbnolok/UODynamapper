use std::path::PathBuf;
use crate::{
    core::render::scene::world::land::mesh_material::{
        LandLightingUniforms, LandShaderModePresets, LandTunablesUniform,
    },
    prelude::*,
    util_lib::tracked_plugin::*,
    core::system_sets::StartupSysSet,
};
use bevy::prelude::*;

const SHADER_PRESETS_FILE_NAME: &str = "shader_presets.toml";

// Holds current values and a dirty flag.
// Bevy detects asset changes and re-uploads uniforms automatically.
#[derive(Resource, Clone, Copy)]
pub struct UniformState {
    pub tunables: LandTunablesUniform,  // modes/toggles + intensities
    pub lighting: LandLightingUniforms, // light/fill/rim + grading + gloom + exposure
    pub global_lighting: f32, // scene-wide brightness scaler (maps to land.global_lighting)
    pub dirty: bool,          // when true, push to GPU materials this frame
}

pub struct ShaderPresetsPlugin {
    pub registered_by: &'static str,
}

impl_tracked_plugin!(ShaderPresetsPlugin);

impl Plugin for ShaderPresetsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.insert_resource(load_from_file())
            .add_systems(Startup, setup_uniform_state);
    }
}

pub fn load_from_file() -> LandShaderModePresets {
    let presets_with_rel_path: PathBuf =
        PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string() + SHADER_PRESETS_FILE_NAME);

    let contents =
        std::fs::read_to_string(&presets_with_rel_path).expect("Failed to read shader presets file");
    let presets: LandShaderModePresets =
        toml::from_str(&contents).expect("Failed to parse shader presets TOML");

    presets
}

fn setup_uniform_state(mut commands: Commands, shader_presets: Res<LandShaderModePresets>) {
    log_system_add_startup::<ShaderPresetsPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());
    let preset = &shader_presets.classic.morning;   // TODO: move this in the presets file?
    commands.insert_resource(UniformState {
        tunables: preset.tunables,
        lighting: preset.lighting,
        global_lighting: 1.0,
        dirty: true,
    });
}
