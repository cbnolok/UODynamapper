use crate::{
    core::render::scene::world::land::mesh_material::{
        GlobalLightingUniforms, LandEffectsUniform, LandLightingUniforms,
        LandMaterialUniformsPresets, LandRenderStylePresetsPerMode, LandShaderModePresets, SunAngles,
    },
    core::system_sets::StartupSysSet,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::prelude::*;
use color_eyre::eyre;


const SHADER_PRESETS_FILE_NAME: &str = "defaults/shader_presets.toml";
const SHADER_SETTINGS_FILE_NAME: &str = "settings/shaders/land.toml";

/// Preset groups, the first part of a preset key such as "kr.morning". One group per render style.
pub const CLASSIC_PRESET_GROUP: &str = "classic";
pub const ENHANCED_PRESET_GROUP: &str = "enhanced";
pub const KR_PRESET_GROUP: &str = "kr";
pub const PAINTERLY_PRESET_GROUP: &str = "painterly";

// Holds current values, a dirty flag, a saved snapshot for undo, and the active
// preset name (e.g. "kr.morning") so we know which mode × time to save into.
#[derive(Resource, Clone)]
pub struct UniformState {
    pub effects: LandEffectsUniform, // texture/rendering pipeline controls
    pub lighting: GlobalLightingUniforms, // global: grading, fog, gloom, tonemap, colors
    pub land_lighting: LandLightingUniforms, // land-specific: fill, rim, bent, diffuse intensities
    pub global_lighting: f32,        // scene-wide brightness scaler (maps to scene.global_lighting)
    pub sun: SunAngles,              // static light direction (maps to scene.light_direction)
    pub dirty: bool,                 // when true, push to GPU materials this frame
    /// Snapshot of the last values saved to disk (for undo). `None` until first save.
    pub saved_snapshot: Option<UniformSnapshot>,
    /// Active preset key, e.g. "kr.morning" — used to decide which slot to save into
    /// and to record which preset to restore on next launch.
    pub active_preset: String,
}

/// A lightweight snapshot of the values we persist (no dirty flag needed).
#[derive(Clone, Copy)]
pub struct UniformSnapshot {
    pub effects: LandEffectsUniform,
    pub lighting: GlobalLightingUniforms,
    pub land_lighting: LandLightingUniforms,
    pub global_lighting: f32,
    pub sun: SunAngles,
}

/// Immutable factory presets loaded from `assets/defaults/shader_presets.toml`.
///
/// The effective `LandShaderModePresets` resource may contain bundled or cached user
/// overrides, so the UI needs this separate resource for a real factory reset.
#[derive(Resource)]
pub struct FactoryShaderPresets(pub LandShaderModePresets);

impl UniformState {
    /// Take a snapshot of the current live values.
    pub fn snapshot(&self) -> UniformSnapshot {
        UniformSnapshot {
            effects: self.effects,
            lighting: self.lighting,
            land_lighting: self.land_lighting,
            global_lighting: self.global_lighting,
            sun: self.sun,
        }
    }

    /// Restore from a snapshot (used by Undo).
    pub fn restore(&mut self, snap: &UniformSnapshot) {
        self.effects = snap.effects;
        self.lighting = snap.lighting;
        self.land_lighting = snap.land_lighting;
        self.global_lighting = snap.global_lighting;
        self.sun = snap.sun;
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
        let exit = |error: eyre::Report| crate::configs::settings::exit_on_startup_config_error(&error);
        let factory_presets = load_factory_presets().unwrap_or_else(exit);
        let effective_presets = load_effective_presets(&factory_presets).unwrap_or_else(exit);

        app.insert_resource(effective_presets)
            .insert_resource(FactoryShaderPresets(factory_presets))
            .add_systems(Startup, setup_uniform_state);
    }
}

pub fn load_from_file() -> eyre::Result<LandShaderModePresets> {
    let factory_presets = load_factory_presets()?;
    load_effective_presets(&factory_presets)
}

fn load_effective_presets(factory_presets: &LandShaderModePresets) -> eyre::Result<LandShaderModePresets> {
    Ok(load_shader_settings(factory_presets)?.unwrap_or_else(|| clone_presets(factory_presets)))
}

/// The factory presets: hand-written and required, read strictly.
fn load_factory_presets() -> eyre::Result<LandShaderModePresets> {
    crate::configs::settings::load_bundled_settings(SHADER_PRESETS_FILE_NAME)
}

// ---------------------------------------------------------------------------
//  User shader settings persistence (land.toml)
// ---------------------------------------------------------------------------

/// Try to load user-saved shader settings from `settings/shaders/land.toml`.
///
/// The bundled copy under `assets/` is hand-written and read strictly; it is optional and stands in for the
/// factory presets when present. The cache copy the app writes is read over it leniently: keys it lacks keep the
/// bundled values, unknown keys are dropped, each with a warning, and the file is rewritten complete right away.
/// Returns `None` if neither copy exists.
pub fn load_shader_settings(
    factory_presets: &LandShaderModePresets,
) -> eyre::Result<Option<LandShaderModePresets>> {
    let bundled_path = crate::core::constants::valid_asset_dir().join(SHADER_SETTINGS_FILE_NAME);
    let bundled = if bundled_path.exists() {
        Some(crate::configs::settings::load_bundled_settings::<LandShaderModePresets>(SHADER_SETTINGS_FILE_NAME)?)
    } else {
        None
    };

    let cache_path = crate::core::constants::valid_cache_dir().join(SHADER_SETTINGS_FILE_NAME);
    let Ok(contents) = std::fs::read_to_string(&cache_path) else {
        return Ok(bundled);
    };
    let defaults = bundled.as_ref().unwrap_or(factory_presets);
    let contents = match migrate_cached_kr_to_painterly(&contents) {
        Some(migrated) => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Settings,
                &format!(
                    "{}: the saved KR-like presets are the UOD-Painterly presets now; KR-like takes its new defaults",
                    cache_path.display()
                ),
            );
            migrated
        }
        None => contents,
    };
    match uocf::utils::config_toml::parse_app_state(&cache_path.display().to_string(), &contents, defaults) {
        Ok(load) => {
            for warning in &load.warnings {
                console_logger::one(LogSev::Warn, LogAbout::Settings, warning);
            }
            if !load.warnings.is_empty() {
                save_shader_settings(&load.value);
            }
            Ok(Some(load.value))
        }
        Err(error) => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Settings,
                &format!("{error:#}; using the bundled shader presets, the file is rewritten on the next save"),
            );
            Ok(Some(clone_presets(defaults)))
        }
    }
}

/// A cached land.toml written before UOD-Painterly existed holds the painterly look under `kr`, which is the faithful
/// KR-like look now. Move that group, with the user's edits, to `painterly`; `kr` is then missing and the lenient load
/// fills it from the defaults. `default_preset` is kept, so a saved KR-like selection stays KR-like.
///
/// Returns the migrated text, or `None` when the file already has `painterly`, has no `kr`, or is not valid TOML (the
/// lenient load reports that).
fn migrate_cached_kr_to_painterly(contents: &str) -> Option<String> {
    let mut table: toml::Table = toml::from_str(contents).ok()?;
    if table.contains_key(PAINTERLY_PRESET_GROUP) {
        return None;
    }
    let kr = table.remove(KR_PRESET_GROUP)?;
    table.insert(PAINTERLY_PRESET_GROUP.to_string(), kr);
    toml::to_string(&table).ok()
}

/// Save the active shader settings to the cache dir's `settings/shaders/land.toml`
/// (never the read-only asset tree — the bundled `assets/` copy is the fallback default).
/// Overwrites the full preset structure (all modes × time-of-day) plus the active preset key.
pub fn save_shader_settings(presets: &LandShaderModePresets) {
    let path = crate::core::constants::valid_cache_dir().join(SHADER_SETTINGS_FILE_NAME);

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
    store_active_preset(&mut out, u);
    out
}

/// Capture the live shader values in their active mode × time-of-day slot.
///
/// The UI calls this before changing presets so edits are not discarded when the
/// next slot is applied. Disk persistence remains tied to closing the shader panel.
pub fn store_active_preset(presets: &mut LandShaderModePresets, u: &UniformState) {
    if let Some(slot) = get_preset_slot_mut(presets, &u.active_preset) {
        slot.effects = u.effects;
        slot.lighting = u.lighting;
        slot.land_lighting = u.land_lighting;
        slot.global_lighting = u.global_lighting;
        slot.sun = u.sun;
    }

    presets.default_preset = u.active_preset.clone();
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
        CLASSIC_PRESET_GROUP => &mut presets.classic,
        ENHANCED_PRESET_GROUP => &mut presets.enhanced,
        KR_PRESET_GROUP => &mut presets.kr,
        PAINTERLY_PRESET_GROUP => &mut presets.painterly,
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
pub fn get_preset_slot<'a>(
    presets: &'a LandShaderModePresets,
    key: &str,
) -> Option<&'a LandMaterialUniformsPresets> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() != 2 {
        return None;
    }
    let mode_group = match parts[0] {
        CLASSIC_PRESET_GROUP => &presets.classic,
        ENHANCED_PRESET_GROUP => &presets.enhanced,
        KR_PRESET_GROUP => &presets.kr,
        PAINTERLY_PRESET_GROUP => &presets.painterly,
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

/// Replace one mode × time-of-day slot with its immutable factory values.
pub fn reset_preset_to_factory(
    presets: &mut LandShaderModePresets,
    factory_presets: &LandShaderModePresets,
    key: &str,
) -> bool {
    let Some(factory_slot) = get_preset_slot(factory_presets, key) else {
        return false;
    };
    let Some(target_slot) = get_preset_slot_mut(presets, key) else {
        return false;
    };
    *target_slot = clone_slot(factory_slot);
    true
}

/// Clone a `LandShaderModePresets` (field-by-field since the inner Copy types make this easy).
fn clone_presets(p: &LandShaderModePresets) -> LandShaderModePresets {
    LandShaderModePresets {
        classic: clone_mode(&p.classic),
        enhanced: clone_mode(&p.enhanced),
        kr: clone_mode(&p.kr),
        painterly: clone_mode(&p.painterly),
        default_preset: p.default_preset.clone(),
    }
}

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
        sun: s.sun,
    }
}

// ---------------------------------------------------------------------------

fn setup_uniform_state(mut commands: Commands, shader_presets: Res<LandShaderModePresets>) {
    log_system_add_startup::<ShaderPresetsPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());

    let state = uniform_state_from_presets(&shader_presets);

    commands.insert_resource(state);
}

fn uniform_state_from_presets(source: &LandShaderModePresets) -> UniformState {
    let preset_key = source.default_preset.clone();
    let preset = get_preset_slot(source, &preset_key).unwrap_or_else(|| {
        let msg = format!(
            "Invalid active preset '{}', falling back to classic.morning",
            preset_key
        );
        console_logger::one(LogSev::Warn, LogAbout::Settings, &msg);
        &source.classic.morning
    });

    UniformState {
        effects: preset.effects,
        lighting: preset.lighting,
        land_lighting: preset.land_lighting,
        global_lighting: preset.global_lighting,
        sun: preset.sun,
        dirty: true,
        saved_snapshot: Some(UniformSnapshot {
            effects: preset.effects,
            lighting: preset.lighting,
            land_lighting: preset.land_lighting,
            global_lighting: preset.global_lighting,
            sun: preset.sun,
        }),
        active_preset: preset_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(mode: u32, global_lighting: f32, exposure: f32) -> LandMaterialUniformsPresets {
        LandMaterialUniformsPresets {
            global_lighting,
            effects: LandEffectsUniform {
                shading_mode: mode,
                ..Default::default()
            },
            lighting: GlobalLightingUniforms {
                exposure,
                ..Default::default()
            },
            land_lighting: LandLightingUniforms::default(),
            sun: SunAngles::default(),
        }
    }

    fn mode(base_mode: u32, base_global: f32) -> LandRenderStylePresetsPerMode {
        LandRenderStylePresetsPerMode {
            morning: slot(base_mode, base_global, 1.0),
            afternoon: slot(base_mode, base_global + 0.1, 1.1),
            night: slot(base_mode, base_global + 0.2, 1.2),
            cave: slot(base_mode, base_global + 0.3, 1.3),
        }
    }

    fn presets() -> LandShaderModePresets {
        LandShaderModePresets {
            classic: mode(0, 1.0),
            enhanced: mode(1, 2.0),
            kr: mode(2, 3.0),
            painterly: mode(2, 4.0),
            default_preset: "kr.afternoon".to_string(),
        }
    }

    #[test]
    fn uniform_state_uses_effective_preset_resource() {
        let source = presets();
        let state = uniform_state_from_presets(&source);

        assert_eq!(state.active_preset, "kr.afternoon");
        assert_eq!(state.effects.shading_mode, 2);
        assert_eq!(state.global_lighting, 3.1);
        assert_eq!(state.lighting.exposure, 1.1);
    }

    #[test]
    fn saving_active_slot_preserves_other_effective_preset_slots() {
        let source = presets();
        let mut state = uniform_state_from_presets(&source);
        state.active_preset = "classic.morning".to_string();
        state.global_lighting = 0.25;
        state.lighting.exposure = 0.75;

        let saved = build_save_presets(&state, &source);

        assert_eq!(saved.classic.morning.global_lighting, 0.25);
        assert_eq!(saved.classic.morning.lighting.exposure, 0.75);
        assert_eq!(saved.kr.afternoon.global_lighting, 3.1);
        assert_eq!(saved.kr.afternoon.lighting.exposure, 1.1);
        assert_eq!(saved.default_preset, "classic.morning");
    }

    #[test]
    fn painterly_presets_are_their_own_slots() {
        let mut effective = presets();
        let mut state = uniform_state_from_presets(&effective);
        state.active_preset = "painterly.night".to_string();
        state.global_lighting = 9.0;

        store_active_preset(&mut effective, &state);

        assert_eq!(effective.painterly.night.global_lighting, 9.0);
        assert_eq!(effective.kr.night.global_lighting, 3.2);
        assert_eq!(effective.default_preset, "painterly.night");
    }

    #[test]
    fn storing_active_preset_updates_only_its_combination() {
        let mut effective = presets();
        let mut state = uniform_state_from_presets(&effective);
        state.active_preset = "classic.cave".to_string();
        state.global_lighting = 9.0;
        state.lighting.exposure = 8.0;

        store_active_preset(&mut effective, &state);

        assert_eq!(effective.classic.cave.global_lighting, 9.0);
        assert_eq!(effective.classic.cave.lighting.exposure, 8.0);
        assert_eq!(effective.classic.morning.global_lighting, 1.0);
        assert_eq!(effective.enhanced.cave.global_lighting, 2.3);
        assert_eq!(effective.default_preset, "classic.cave");
    }

    #[test]
    fn resetting_preset_restores_only_the_active_factory_combination() {
        let factory = presets();
        let mut effective = presets();
        effective.classic.morning.global_lighting = 9.0;
        effective.classic.cave.lighting.exposure = 8.0;
        effective.enhanced.afternoon.global_lighting = 7.0;
        effective.default_preset = "classic.cave".to_string();

        assert!(reset_preset_to_factory(
            &mut effective,
            &factory,
            "classic.cave",
        ));

        assert_eq!(effective.classic.morning.global_lighting, 9.0);
        assert_eq!(effective.classic.cave.lighting.exposure, 1.3);
        assert_eq!(effective.enhanced.afternoon.global_lighting, 7.0);
        assert_eq!(effective.default_preset, "classic.cave");
    }

    const BUNDLED_FACTORY_PRESETS: &str = include_str!("../../assets/defaults/shader_presets.toml");
    const BUNDLED_LAND_SETTINGS: &str = include_str!("../../assets/settings/shaders/land.toml");

    fn parse(text: &str) -> eyre::Result<LandShaderModePresets> {
        uocf::utils::config_toml::parse_strict("shader_presets.toml", text)
    }

    #[test]
    fn bundled_shader_preset_files_parse_strictly() {
        parse(BUNDLED_FACTORY_PRESETS).unwrap_or_else(|error| panic!("{error}"));
        parse(BUNDLED_LAND_SETTINGS).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    fn shader_presets_reject_unknown_and_missing_keys() {
        let unknown = BUNDLED_FACTORY_PRESETS.replacen("exposure = ", "exposur = ", 1);
        let error = parse(&unknown).err().expect("unknown key").to_string();
        assert!(error.contains("unknown field `exposur`, expected one of `enable_fog`"), "{error}");
        assert!(error.contains("line "), "{error}");

        let padded = BUNDLED_FACTORY_PRESETS.replacen("exposure = ", "_pad0_ = 0.0\nexposure = ", 1);
        let error = parse(&padded).err().expect("layout padding is not a setting").to_string();
        assert!(error.contains("unknown field `_pad0_`"), "{error}");

        let missing = BUNDLED_FACTORY_PRESETS.replacen("kr_tonemap_params = ", "# kr_tonemap_params = ", 1);
        let error = parse(&missing).err().expect("missing key").to_string();
        assert!(error.contains("missing field `kr_tonemap_params`"), "{error}");
    }

    #[test]
    fn cached_shader_settings_drop_retired_keys_and_fill_new_ones() {
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let cache = BUNDLED_LAND_SETTINGS
            .replacen("[classic.morning.effects]\n", "[classic.morning.effects]\n_pad_eff0 = 0.0\n", 1)
            .replacen("kr_tonemap_params = [0.9, 0.0, 0.0, 0.0]\n", "", 1);
        let load = uocf::utils::config_toml::parse_app_state("land.toml", &cache, &bundled).expect("lenient load");
        assert_eq!(
            load.warnings,
            [
                "land.toml: missing key `classic.morning.lighting.kr_tonemap_params` filled from the default",
                "land.toml: unknown key `classic.morning.effects._pad_eff0` dropped",
            ]
        );
        let rewritten = toml::to_string_pretty(&load.value).expect("serialize");
        parse(&rewritten).expect("rewritten file is complete");
    }

    #[test]
    fn cached_kr_like_presets_from_before_painterly_become_painterly() {
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let painterly_start = BUNDLED_LAND_SETTINGS.find("[painterly.morning]\n").expect("painterly group");
        assert!(!BUNDLED_LAND_SETTINGS[painterly_start..].contains("\n[kr."), "painterly is the last group");
        let old_cache = BUNDLED_LAND_SETTINGS[..painterly_start]
            .replacen("default_preset = \"classic.morning\"", "default_preset = \"kr.night\"", 1)
            .replacen("[kr.night]\nglobal_lighting = ", "[kr.night]\nglobal_lighting = 0.125\n# ", 1);

        let migrated = migrate_cached_kr_to_painterly(&old_cache).expect("migrated");
        let load = uocf::utils::config_toml::parse_app_state("land.toml", &migrated, &bundled).expect("lenient load");

        assert_eq!(load.warnings, ["land.toml: missing key `kr` filled from the default"]);
        assert_eq!(load.value.painterly.night.global_lighting, 0.125);
        assert_eq!(load.value.kr.night.global_lighting, bundled.kr.night.global_lighting);
        assert_eq!(load.value.default_preset, "kr.night");
        assert!(migrate_cached_kr_to_painterly(BUNDLED_LAND_SETTINGS).is_none());
    }

    /// The KR and Enhanced clients' terrain light. Their D = (0, -0.7071, -0.7071) points from the light to the
    /// ground; towards the light that is (0, 0.7071, 0.7071) in our axes: from the south (+z) at 45 degrees.
    const CLIENT_SUN: SunAngles = SunAngles { azimuth_degrees: 180.0, elevation_degrees: 45.0 };
    const SUN_DIRECTION_TOLERANCE: f32 = 1e-6;

    #[test]
    fn default_sun_is_the_former_baked_light() {
        assert_eq!(
            SunAngles::default().light_direction(),
            crate::core::constants::BAKED_GLOBAL_LIGHT.normalize()
        );
    }

    #[test]
    fn client_sun_shines_from_the_south_at_45_degrees() {
        let towards_light = Vec3::new(0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
        let direction = CLIENT_SUN.light_direction();
        assert!(direction.abs_diff_eq(towards_light, SUN_DIRECTION_TOLERANCE), "{direction}");
    }

    #[test]
    fn bundled_presets_keep_the_baked_sun_but_kr_like_takes_the_client_sun() {
        for text in [BUNDLED_FACTORY_PRESETS, BUNDLED_LAND_SETTINGS] {
            let presets = parse(text).expect("bundled");
            for (group, expected) in [
                (&presets.classic, SunAngles::default()),
                (&presets.enhanced, SunAngles::default()),
                (&presets.kr, CLIENT_SUN),
                (&presets.painterly, SunAngles::default()),
            ] {
                for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                    assert_eq!(slot.sun, expected);
                }
            }
        }
    }

    #[test]
    fn only_kr_like_takes_the_client_texture_filter() {
        use crate::core::render::scene::world::land::mesh_material::{
            TEXTURE_FILTER_CLIENT_BILINEAR, TEXTURE_FILTER_PIXEL_ART,
        };
        for text in [BUNDLED_FACTORY_PRESETS, BUNDLED_LAND_SETTINGS] {
            let presets = parse(text).expect("bundled");
            for (group, expected) in [
                (&presets.classic, TEXTURE_FILTER_PIXEL_ART),
                (&presets.enhanced, TEXTURE_FILTER_PIXEL_ART),
                (&presets.kr, TEXTURE_FILTER_CLIENT_BILINEAR),
                (&presets.painterly, TEXTURE_FILTER_PIXEL_ART),
            ] {
                for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                    assert_eq!(slot.effects.texture_filter, expected);
                }
            }
        }
    }

    /// The clients' half-Lambert, (N.L + 1) / 2: the land shader's wrap gives N.L * (1 - wrap) + wrap.
    const CLIENT_HALF_LAMBERT_WRAP: f32 = 0.5;
    const CLIENT_DIFFUSE_STRENGTH: f32 = 1.0;

    #[test]
    fn enhanced_leaves_off_the_terms_and_static_shadows_the_client_lacks() {
        use crate::core::render::scene::world::land::mesh_material::{
            TERRAIN_SHADING_SMOOTH_PER_PIXEL, TONE_CURVE_IDENTITY,
        };
        for text in [BUNDLED_FACTORY_PRESETS, BUNDLED_LAND_SETTINGS] {
            let presets = parse(text).expect("bundled");
            let group = &presets.enhanced;
            for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                let effects = &slot.effects;
                assert_eq!(effects.enable_grunge, 0);
                assert_eq!(effects.light_decal_intensity, 0.0);
                assert_eq!(effects.static_light_decal_visibility, 0.0);
                assert_eq!(effects.art_highlight_strength, 0.0);
                assert_eq!(effects.art_depth_tint_strength, 0.0);
                // The Enhanced client draws no shadow for statics, items or foliage; only mobiles get one.
                assert_eq!(effects.art_shadow_strength, 0.0);
                assert_eq!(effects.art_contact_shadow_strength, 0.0);
                assert_eq!(effects.enable_art_projected_shadows, 0);
                assert_eq!(effects.art_projected_shadow_strength, 0.0);

                let lighting = &slot.lighting;
                assert_eq!(lighting.enable_grading, 0);
                assert_eq!(lighting.enable_gloom, 0);
                assert_eq!(lighting.enable_fog, 0);
                assert_eq!(lighting.tone_curve, TONE_CURVE_IDENTITY);
                assert_eq!(lighting.light_color, Vec3::ONE);
                assert_eq!(lighting.ambient_strength, 0.0);

                let land = &slot.land_lighting;
                assert_eq!(land.terrain_shading_model, TERRAIN_SHADING_SMOOTH_PER_PIXEL);
                assert_eq!(land.enable_bent, 0);
                assert_eq!(land.specular_strength, 0.0);
                assert_eq!(land.rim_strength, 0.0);
                assert_eq!(land.fill_strength, 0.0);
                assert_eq!(land.sharpness_mix, 0.0);
                assert_eq!(land.diffuse_wrap, CLIENT_HALF_LAMBERT_WRAP);
                assert_eq!(land.diffuse_strength, CLIENT_DIFFUSE_STRENGTH);
            }
        }
    }
}
