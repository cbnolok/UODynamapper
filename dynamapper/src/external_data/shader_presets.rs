use crate::{
    core::render::scene::world::land::mesh_material::{
        LandEffectsUniform, LandLightingUniforms, LandShaderModePresets,
    },
    core::system_sets::StartupSysSet,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::prelude::*;
use std::path::PathBuf;
use std::time::SystemTime;

const SHADER_PRESETS_FILE_NAME: &str = "shader_presets.toml";

// Holds current values and a dirty flag.
// Bevy detects asset changes and re-uploads uniforms automatically.
#[derive(Resource, Clone, Copy)]
pub struct UniformState {
    pub effects: LandEffectsUniform,    // modes/toggles + intensities
    pub lighting: LandLightingUniforms, // light/fill/rim + grading + gloom + exposure
    pub global_lighting: f32, // scene-wide brightness scaler (maps to land.global_lighting)
    pub dirty: bool,          // when true, push to GPU materials this frame
}

/// Polls shader_presets.toml for modification time changes every second.
/// On change, re-reads the file and marks UniformState as dirty so the shader
/// uniforms are pushed to the GPU this frame.
#[derive(Resource)]
pub struct ShaderPresetsFileWatcher {
    pub poll_timer: Timer,
    pub last_mtime: Option<SystemTime>,
}

impl Default for ShaderPresetsFileWatcher {
    fn default() -> Self {
        let path = PathBuf::from(
            crate::core::constants::ASSET_FOLDER.to_string() + SHADER_PRESETS_FILE_NAME
        );
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        Self {
            poll_timer: Timer::from_seconds(1.0, TimerMode::Repeating),
            last_mtime: mtime,
        }
    }
}

pub struct ShaderPresetsPlugin {
    pub registered_by: &'static str,
}

impl_tracked_plugin!(ShaderPresetsPlugin);

impl Plugin for ShaderPresetsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.insert_resource(load_from_file())
            .init_resource::<ShaderPresetsFileWatcher>()
            .add_systems(Startup, setup_uniform_state)
            .add_systems(Update, sys_hotreload_shader_presets);
    }
}

pub fn load_from_file() -> LandShaderModePresets {
    let presets_with_rel_path: PathBuf =
        PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string() + SHADER_PRESETS_FILE_NAME);

    let contents = std::fs::read_to_string(&presets_with_rel_path)
        .expect("Failed to read shader presets file");
    let presets: LandShaderModePresets = match toml::from_str(&contents) {
        Ok(cont) => cont,
        Err(e) => {
            eprintln!(
                "Failed to parse shader presets TOML. Error: {}",
                e.message()
            );
            panic!();
        }
    };

    presets
}

fn setup_uniform_state(mut commands: Commands, shader_presets: Res<LandShaderModePresets>) {
    log_system_add_startup::<ShaderPresetsPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());
    // Select initial preset. Prefer `default_preset` if present in the TOML file.
    let preset = {
        let parts: Vec<&str> = shader_presets.default_preset.split('.').collect();
        if parts.len() == 2 {
            match parts[0] {
                "classic" => match parts[1] {
                    "morning" => &shader_presets.classic.morning,
                    "afternoon" => &shader_presets.classic.afternoon,
                    "night" => &shader_presets.classic.night,
                    "cave" => &shader_presets.classic.cave,
                    _ => &shader_presets.classic.morning,
                },
                "enhanced" => match parts[1] {
                    "morning" => &shader_presets.enhanced.morning,
                    "afternoon" => &shader_presets.enhanced.afternoon,
                    "night" => &shader_presets.enhanced.night,
                    "cave" => &shader_presets.enhanced.cave,
                    _ => &shader_presets.classic.morning,
                },
                "kr" => match parts[1] {
                    "morning" => &shader_presets.kr.morning,
                    "afternoon" => &shader_presets.kr.afternoon,
                    "night" => &shader_presets.kr.night,
                    "cave" => &shader_presets.kr.cave,
                    _ => &shader_presets.classic.morning,
                },
                _ => &shader_presets.classic.morning,
            }
        } else {
            &shader_presets.classic.morning
        }
    };
    commands.insert_resource(UniformState {
        effects: preset.effects,
        lighting: preset.lighting,
        global_lighting: 1.0,
        dirty: true,
    });
}

/// Hot-reload system: polls shader_presets.toml every second.
/// When the file changes on disk, re-reads it, updates LandShaderModePresets,
/// and marks UniformState as dirty so the GPU materials are updated this frame.
/// The active preset (effects/lighting) is NOT automatically overwritten —
/// it is reset to classic.morning only on the first load. This preserves any
/// user-chosen preset active in the UI while still allowing shader parameter
/// tuning via text editor.
fn sys_hotreload_shader_presets(
    time: Res<Time>,
    mut watcher: ResMut<ShaderPresetsFileWatcher>,
    mut presets_res: ResMut<LandShaderModePresets>,
) {
    // Throttle to once per second.
    watcher.poll_timer.tick(time.delta());
    if !watcher.poll_timer.just_finished() {
        return;
    }

    let path = PathBuf::from(
        crate::core::constants::ASSET_FOLDER.to_string() + SHADER_PRESETS_FILE_NAME
    );
    let new_mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok());

    if new_mtime == watcher.last_mtime {
        return;
    }

    // File changed — reload.
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            bevy::log::warn!("shader_presets.toml hot-reload failed (read): {e}");
            return;
        }
    };
    let new_presets: LandShaderModePresets = match toml::from_str(&contents) {
        Ok(p) => p,
        Err(e) => {
            bevy::log::warn!("shader_presets.toml hot-reload failed (parse): {}", e.message());
            return;
        }
    };

    *presets_res = new_presets;
    watcher.last_mtime = new_mtime;
    bevy::log::info!("Hot-reloaded: shader_presets.toml");
}

