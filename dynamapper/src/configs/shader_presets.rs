use crate::{
    core::render::scene::world::land::mesh_material::{
        GlobalLightingUniforms, LandEffectsUniform, LandLightingUniforms,
        LandMaterialUniformsPresets, LandRenderStylePresetsPerMode, LandShaderModePresets,
    },
    core::system_sets::StartupSysSet,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::prelude::*;
use std::path::PathBuf;


const SHADER_PRESETS_FILE_NAME: &str = "defaults/shader_presets.toml";
const SHADER_SETTINGS_FILE_NAME: &str = "settings/shaders/land.toml";

// Holds current values, a dirty flag, a saved snapshot for undo, and the active
// preset name (e.g. "kr.morning") so we know which mode × time to save into.
#[derive(Resource, Clone)]
pub struct UniformState {
    pub effects: LandEffectsUniform, // texture/rendering pipeline controls
    pub lighting: GlobalLightingUniforms, // global: grading, fog, gloom, tonemap, colors
    pub land_lighting: LandLightingUniforms, // land-specific: fill, rim, bent, diffuse intensities
    pub global_lighting: f32,        // scene-wide brightness scaler (maps to scene.global_lighting)
    pub dirty: bool,                 // when true, push to GPU materials this frame
    /// Snapshot of the last values saved to disk (for undo). `None` until first save.
    pub saved_snapshot: Option<UniformSnapshot>,
    /// Active preset key, e.g. "kr.morning" — used to decide which slot to save into
    /// and to record which preset to restore on next launch.
    pub active_preset: String,
}

/// A lightweight snapshot of the four values we persist (no dirty flag needed).
#[derive(Clone, Copy)]
pub struct UniformSnapshot {
    pub effects: LandEffectsUniform,
    pub lighting: GlobalLightingUniforms,
    pub land_lighting: LandLightingUniforms,
    pub global_lighting: f32,
}

impl UniformState {
    /// Take a snapshot of the current live values.
    pub fn snapshot(&self) -> UniformSnapshot {
        UniformSnapshot {
            effects: self.effects,
            lighting: self.lighting,
            land_lighting: self.land_lighting,
            global_lighting: self.global_lighting,
        }
    }

    /// Restore from a snapshot (used by Undo).
    pub fn restore(&mut self, snap: &UniformSnapshot) {
        self.effects = snap.effects;
        self.lighting = snap.lighting;
        self.land_lighting = snap.land_lighting;
        self.global_lighting = snap.global_lighting;
        self.dirty = true;
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
            .add_systems(Startup, setup_uniform_state);
    }
}

pub fn load_from_file() -> LandShaderModePresets {
    let presets_with_rel_path: PathBuf =
        crate::core::constants::valid_asset_dir().join(SHADER_PRESETS_FILE_NAME);

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

// ---------------------------------------------------------------------------
//  User shader settings persistence (land.toml)
// ---------------------------------------------------------------------------

/// Try to load user-saved shader settings from `settings/shaders/land.toml`.
/// Returns `None` if the file doesn't exist or can't be parsed.
pub fn load_shader_settings() -> Option<LandShaderModePresets> {
    let path = crate::core::constants::valid_asset_dir().join(SHADER_SETTINGS_FILE_NAME);
    let contents = std::fs::read_to_string(&path).ok()?;
    match toml::from_str::<LandShaderModePresets>(&contents) {
        Ok(p) => Some(p),
        Err(e) => {
            panic!("Failed to parse land.toml: {}", e.message());
        }
    }
}

/// Save the active shader settings to `settings/shaders/land.toml`.
/// Overwrites the full preset structure (all modes × time-of-day) plus the active preset key.
pub fn save_shader_settings(presets: &LandShaderModePresets) {
    let path = crate::core::constants::valid_asset_dir().join(SHADER_SETTINGS_FILE_NAME);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            bevy::log::error!("Failed to create shaders settings dir: {e}");
            return;
        }
    }

    match toml::to_string_pretty(presets) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&path, toml_str) {
                bevy::log::error!("Failed to save land.toml: {e}");
            } else {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Settings,
                    "Saved shader settings to land.toml",
                );
            }
        }
        Err(e) => {
            bevy::log::error!("Failed to serialize shader settings: {e}");
        }
    }
}

/// Build a full `LandShaderModePresets` from the current `UniformState`, writing
/// the live values into the slot matching `active_preset` and keeping the rest
/// from the base presets.
pub fn build_save_presets(u: &UniformState, base: &LandShaderModePresets) -> LandShaderModePresets {
    // Start with a clone of the base (either user-saved or factory defaults).
    // We only overwrite the active preset's slot so other modes are preserved.
    let mut out = clone_presets(base);

    // Write the live values into the active slot.
    if let Some(slot) = get_preset_slot_mut(&mut out, &u.active_preset) {
        slot.effects = u.effects;
        slot.lighting = u.lighting;
        slot.land_lighting = u.land_lighting;
        slot.global_lighting = u.global_lighting;
    }

    out.default_preset = u.active_preset.clone();
    out
}

/// Get a mutable reference to a particular mode×time slot by key.
fn get_preset_slot_mut<'a>(
    presets: &'a mut LandShaderModePresets,
    key: &str,
) -> Option<&'a mut LandMaterialUniformsPresets> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let mode_group = match parts[0] {
        "classic" => &mut presets.classic,
        "enhanced" => &mut presets.enhanced,
        "kr" => &mut presets.kr,
        _ => return None,
    };
    match parts[1] {
        "morning" => Some(&mut mode_group.morning),
        "afternoon" => Some(&mut mode_group.afternoon),
        "night" => Some(&mut mode_group.night),
        "cave" => Some(&mut mode_group.cave),
        _ => None,
    }
}

/// Get an immutable reference to a particular mode×time slot by key.
fn get_preset_slot<'a>(
    presets: &'a LandShaderModePresets,
    key: &str,
) -> Option<&'a LandMaterialUniformsPresets> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let mode_group = match parts[0] {
        "classic" => &presets.classic,
        "enhanced" => &presets.enhanced,
        "kr" => &presets.kr,
        _ => return None,
    };
    match parts[1] {
        "morning" => Some(&mode_group.morning),
        "afternoon" => Some(&mode_group.afternoon),
        "night" => Some(&mode_group.night),
        "cave" => Some(&mode_group.cave),
        _ => None,
    }
}

/// Clone a `LandShaderModePresets` (field-by-field since the inner Copy types make this easy).
fn clone_presets(p: &LandShaderModePresets) -> LandShaderModePresets {
    fn clone_mode(m: &LandRenderStylePresetsPerMode) -> LandRenderStylePresetsPerMode {
        LandRenderStylePresetsPerMode {
            morning: clone_slot(&m.morning),
            afternoon: clone_slot(&m.afternoon),
            night: clone_slot(&m.night),
            cave: clone_slot(&m.cave),
        }
    }
    fn clone_slot(s: &LandMaterialUniformsPresets) -> LandMaterialUniformsPresets {
        LandMaterialUniformsPresets {
            global_lighting: s.global_lighting,
            effects: s.effects,
            lighting: s.lighting,
            land_lighting: s.land_lighting,
        }
    }
    LandShaderModePresets {
        classic: clone_mode(&p.classic),
        enhanced: clone_mode(&p.enhanced),
        kr: clone_mode(&p.kr),
        default_preset: p.default_preset.clone(),
    }
}

// ---------------------------------------------------------------------------

fn setup_uniform_state(mut commands: Commands, shader_presets: Res<LandShaderModePresets>) {
    log_system_add_startup::<ShaderPresetsPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());

    // Try loading user-saved settings first, fall back to factory defaults.
    let (source, preset_key) = if let Some(saved) = load_shader_settings() {
        let key = saved.default_preset.clone();
        // Merge saved presets into the resource so the UI preset buttons work.
        // We use the saved data as the source of truth.
        (saved, key)
    } else {
        let key = shader_presets.default_preset.clone();
        (clone_presets(&shader_presets), key)
    };

    let preset = get_preset_slot(&source, &preset_key).unwrap_or_else(|| {
        let msg = format!(
            "Invalid active preset '{}', falling back to classic.morning",
            preset_key
        );
        console_logger::one(LogSev::Warn, LogAbout::Settings, &msg);
        &source.classic.morning
    });

    let state = UniformState {
        effects: preset.effects,
        lighting: preset.lighting,
        land_lighting: preset.land_lighting,
        global_lighting: preset.global_lighting,
        dirty: true,
        saved_snapshot: Some(UniformSnapshot {
            effects: preset.effects,
            lighting: preset.lighting,
            land_lighting: preset.land_lighting,
            global_lighting: preset.global_lighting,
        }),
        active_preset: preset_key,
    };

    commands.insert_resource(state);
}
