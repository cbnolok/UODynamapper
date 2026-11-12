use crate::{
    core::render::scene::world::effects::client_night::{
        MAX_CLIENT_LIGHT_LEVEL, MAX_NIGHT_OVERLAY_STRENGTH, MIN_NIGHT_OVERLAY_STRENGTH,
    },
    core::render::scene::world::land::mesh_material::{
        ClientNightLighting, GlobalLightingUniforms, LandEffectsUniform, LandLightingUniforms,
        LandMaterialUniformsPresets, LandRenderStylePresetsPerMode, LandShaderModePresets, SunAngles,
        TONE_CURVE_PROFILE,
    },
    core::system_sets::StartupSysSet,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::prelude::*;
use color_eyre::eyre;


const SHADER_PRESETS_FILE_NAME: &str = "defaults/shader_presets.toml";
const SHADER_SETTINGS_FILE_NAME: &str = "settings/shaders/land.toml";
/// The land.toml key of the selected preset (`LandShaderModePresets::default_preset`).
const DEFAULT_PRESET_KEY: &str = "default_preset";

/// Preset groups, the first part of a preset key such as "kr.morning". One group per render style.
pub const CLASSIC_PRESET_GROUP: &str = "classic";
pub const ENHANCED_PRESET_GROUP: &str = "enhanced";
pub const KR_PRESET_GROUP: &str = "kr";
pub const PAINTERLY_PRESET_GROUP: &str = "painterly";
/// Every preset group, in file order.
const PRESET_GROUPS: [&str; 4] = [CLASSIC_PRESET_GROUP, ENHANCED_PRESET_GROUP, KR_PRESET_GROUP, PAINTERLY_PRESET_GROUP];
/// The groups whose render style draws with the KR post-process profile (unit-tested against the render styles).
pub const KR_PROFILE_PRESET_GROUPS: [&str; 2] = [KR_PRESET_GROUP, PAINTERLY_PRESET_GROUP];
/// The KR profile curve's retired separate pre-scale, `lighting.kr_tonemap_params.x`, and the value the shader used
/// when a file left it unset (<= 0). The old hardcoded value was ≈ 0.09 (middle_gray(0.18) / luminance(2.0)), which
/// crushed KR ~11x darker than EC; this default sat near EC's own pre-scale (~1.0) so KR presets were legible.
const RETIRED_KR_TONEMAP_KEY: &str = "kr_tonemap_params";
const RETIRED_KR_TONEMAP_DEFAULT_GAIN: f64 = 0.9;
/// Lighting colours and parameters whose fourth component was never read; they hold three now.
const RETIRED_FOURTH_COMPONENT_KEYS: [&str; 3] = ["grade_warm_color", "grade_cool_color", "gloom_params"];
const THREE_COMPONENTS: usize = 3;

/// The scene multiplier while the clients' night composite does the dimming: the identity.
pub const CLIENT_LIT_SCENE_MULTIPLIER: f32 = 1.0;

/// The overlay colour's channels are 8-bit values in 0..1.
const OVERLAY_COLOR_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1.0;

/// The preset periods, the second part of a preset key such as "kr.morning".
const PRESET_PERIODS: [&str; 4] = ["morning", "afternoon", "night", "cave"];

/// KR-like and Enhanced dim at night through the clients' composite (`client_lighting`); Classic and UOD-Painterly
/// through the scene multiplier (`global_lighting`).
pub fn preset_group_uses_client_lighting(group: &str) -> bool {
    matches!(group, KR_PRESET_GROUP | ENHANCED_PRESET_GROUP)
}

/// KR-like draws the KR client's static shadows (`client_shadows`) in place of the projected and contact sprite
/// shadows; the Enhanced client draws none for statics, and Classic and UOD-Painterly keep their own.
pub fn preset_group_uses_client_shadows(group: &str) -> bool {
    group == KR_PRESET_GROUP
}

// Holds current values, a dirty flag, a saved snapshot for undo, and the active
// preset name (e.g. "kr.morning") so we know which mode × time to save into.
#[derive(Resource, Clone)]
pub struct UniformState {
    pub effects: LandEffectsUniform, // texture/rendering pipeline controls
    pub lighting: GlobalLightingUniforms, // global: grading, fog, gloom, tonemap, colors
    pub land_lighting: LandLightingUniforms, // land-specific: fill, rim, bent, diffuse intensities
    pub global_lighting: f32,        // scene-wide brightness scaler (maps to scene.global_lighting);
                                     // CLIENT_LIT_SCENE_MULTIPLIER while `client_lighting` is set
    /// The clients' night composite: `Some` in KR-like and Enhanced, which have no `global_lighting`.
    pub client_lighting: Option<ClientNightLighting>,
    /// The KR client's static shadows on or off: `Some` in KR-like only.
    pub client_shadows: Option<bool>,
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
    pub client_lighting: Option<ClientNightLighting>,
    pub client_shadows: Option<bool>,
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
            client_lighting: self.client_lighting,
            client_shadows: self.client_shadows,
            sun: self.sun,
        }
    }

    /// Restore from a snapshot (used by Undo).
    pub fn restore(&mut self, snap: &UniformSnapshot) {
        self.effects = snap.effects;
        self.lighting = snap.lighting;
        self.land_lighting = snap.land_lighting;
        self.global_lighting = snap.global_lighting;
        self.client_lighting = snap.client_lighting;
        self.client_shadows = snap.client_shadows;
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
    let presets = crate::configs::settings::load_bundled_settings(SHADER_PRESETS_FILE_NAME)?;
    validate_preset_lighting(&presets).map_err(|error| eyre::eyre!("{SHADER_PRESETS_FILE_NAME}: {error}"))?;
    Ok(presets)
}

/// The group and period names of each preset slot, in file order.
fn preset_slots(presets: &LandShaderModePresets) -> Vec<(&'static str, &'static str, &LandMaterialUniformsPresets)> {
    let groups = [
        (CLASSIC_PRESET_GROUP, &presets.classic),
        (ENHANCED_PRESET_GROUP, &presets.enhanced),
        (KR_PRESET_GROUP, &presets.kr),
        (PAINTERLY_PRESET_GROUP, &presets.painterly),
    ];
    groups
        .into_iter()
        .flat_map(|(group, slots)| {
            PRESET_PERIODS
                .into_iter()
                .zip([&slots.morning, &slots.afternoon, &slots.night, &slots.cave])
                .map(move |(period, slot)| (group, period, slot))
        })
        .collect()
}

/// The out-of-range parts of a client night lighting setting, described for an error or a warning.
fn client_lighting_range_problems(lighting: &ClientNightLighting) -> Vec<String> {
    let mut problems = Vec::new();
    if lighting.light_level > MAX_CLIENT_LIGHT_LEVEL {
        problems.push(format!("light_level {} is above {MAX_CLIENT_LIGHT_LEVEL}", lighting.light_level));
    }
    if !(MIN_NIGHT_OVERLAY_STRENGTH..=MAX_NIGHT_OVERLAY_STRENGTH).contains(&lighting.overlay_strength) {
        problems.push(format!(
            "overlay_strength {} is outside {MIN_NIGHT_OVERLAY_STRENGTH}..={MAX_NIGHT_OVERLAY_STRENGTH}",
            lighting.overlay_strength
        ));
    }
    if !lighting.overlay_color.to_array().iter().all(|channel| OVERLAY_COLOR_RANGE.contains(channel)) {
        problems.push(format!("overlay_color {} is outside 0..=1", lighting.overlay_color));
    }
    problems
}

/// The client night lighting with every value brought into its range.
fn clamp_client_lighting(lighting: ClientNightLighting) -> ClientNightLighting {
    ClientNightLighting {
        light_level: lighting.light_level.min(MAX_CLIENT_LIGHT_LEVEL),
        overlay_strength: lighting.overlay_strength.clamp(MIN_NIGHT_OVERLAY_STRENGTH, MAX_NIGHT_OVERLAY_STRENGTH),
        overlay_color: lighting
            .overlay_color
            .clamp(Vec3::splat(*OVERLAY_COLOR_RANGE.start()), Vec3::splat(*OVERLAY_COLOR_RANGE.end())),
    }
}

/// A hand-written preset file must give KR-like and Enhanced `client_lighting` and no `global_lighting`, and the
/// other groups the reverse, with every value in range; and KR-like, alone, `client_shadows`.
fn validate_preset_lighting(presets: &LandShaderModePresets) -> eyre::Result<()> {
    let mut problems = Vec::new();
    for (group, period, slot) in preset_slots(presets) {
        let key = format!("{group}.{period}");
        if preset_group_uses_client_lighting(group) {
            if slot.global_lighting.is_some() {
                problems.push(format!("`{key}.global_lighting` is not a setting there: it dims through `client_lighting`"));
            }
            match &slot.client_lighting {
                None => problems.push(format!("missing `{key}.client_lighting`")),
                Some(lighting) => problems.extend(
                    client_lighting_range_problems(lighting)
                        .into_iter()
                        .map(|problem| format!("`{key}.client_lighting`: {problem}")),
                ),
            }
        } else {
            if slot.client_lighting.is_some() {
                problems.push(format!("`{key}.client_lighting` is not a setting there: it dims through `global_lighting`"));
            }
            if slot.global_lighting.is_none() {
                problems.push(format!("missing `{key}.global_lighting`"));
            }
        }
        match (preset_group_uses_client_shadows(group), slot.client_shadows) {
            (true, None) => problems.push(format!("missing `{key}.client_shadows`")),
            (false, Some(_)) => {
                problems.push(format!("`{key}.client_shadows` is not a setting there: only KR-like has the KR client's shadows"))
            }
            _ => {}
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(eyre::eyre!("{}", problems.join("; ")))
    }
}

/// Bring a leniently read land.toml in line with `validate_preset_lighting`: drop the dimming setting a group does not
/// use, fill the one it lacks from `defaults` and clamp out-of-range values; likewise `client_shadows`. Returns a
/// warning per change.
///
/// A land.toml saved before the night composite keeps `global_lighting` in KR-like and Enhanced; it is dropped there.
fn normalize_cached_preset_lighting(
    presets: &mut LandShaderModePresets,
    defaults: &LandShaderModePresets,
    file_name: &str,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (group, period, default_slot) in preset_slots(defaults) {
        let key = format!("{group}.{period}");
        let Some(slot) = get_preset_slot_mut(presets, &key) else {
            continue;
        };
        if preset_group_uses_client_lighting(group) {
            if slot.global_lighting.take().is_some() {
                warnings.push(format!(
                    "{file_name}: `{key}.global_lighting` dropped: this style dims through `client_lighting`"
                ));
            }
            match slot.client_lighting {
                None => {
                    slot.client_lighting = default_slot.client_lighting;
                    warnings.push(format!("{file_name}: missing key `{key}.client_lighting` filled from the default"));
                }
                Some(lighting) => {
                    let problems = client_lighting_range_problems(&lighting);
                    if !problems.is_empty() {
                        slot.client_lighting = Some(clamp_client_lighting(lighting));
                        warnings.push(format!("{file_name}: `{key}.client_lighting` clamped: {}", problems.join("; ")));
                    }
                }
            }
        } else {
            if slot.client_lighting.take().is_some() {
                warnings.push(format!(
                    "{file_name}: `{key}.client_lighting` dropped: this style dims through `global_lighting`"
                ));
            }
            if slot.global_lighting.is_none() {
                slot.global_lighting = default_slot.global_lighting;
                warnings.push(format!("{file_name}: missing key `{key}.global_lighting` filled from the default"));
            }
        }
        if preset_group_uses_client_shadows(group) {
            if slot.client_shadows.is_none() {
                slot.client_shadows = default_slot.client_shadows;
                warnings.push(format!("{file_name}: missing key `{key}.client_shadows` filled from the default"));
            }
        } else if slot.client_shadows.take().is_some() {
            warnings.push(format!(
                "{file_name}: `{key}.client_shadows` dropped: only KR-like has the KR client's shadows"
            ));
        }
    }
    warnings
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
        let bundled =
            crate::configs::settings::load_bundled_settings::<LandShaderModePresets>(SHADER_SETTINGS_FILE_NAME)?;
        validate_preset_lighting(&bundled).map_err(|error| eyre::eyre!("{SHADER_SETTINGS_FILE_NAME}: {error}"))?;
        Some(bundled)
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
    let retired = migrate_cached_retired_lighting(&contents, defaults);
    let migrated_retired = retired.is_some();
    let contents = match retired {
        Some(migrated) => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Settings,
                &format!(
                    "{}: KR Tonemap Brightness is folded into the exposure of the saved KR-profile presets; the unused \
                     fourth value of the grade colours and gloom parameters is dropped",
                    cache_path.display()
                ),
            );
            migrated
        }
        None => contents,
    };
    let file_name = cache_path.display().to_string();
    match uocf::utils::config_toml::parse_app_state(&file_name, &contents, defaults) {
        Ok(mut load) => {
            let lighting_warnings = normalize_cached_preset_lighting(&mut load.value, defaults, &file_name);
            load.warnings.extend(lighting_warnings);
            for warning in &load.warnings {
                console_logger::one(LogSev::Warn, LogAbout::Settings, warning);
            }
            if !load.warnings.is_empty() || migrated_retired {
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

/// The preset the app saved as selected, `default_preset` of the cached land.toml; `None` when there is no cache copy
/// (the shipped default preset is selected) or it names none.
pub fn saved_preset_key() -> Option<String> {
    let cache_path = crate::core::constants::valid_cache_dir().join(SHADER_SETTINGS_FILE_NAME);
    let table: toml::Table = toml::from_str(&std::fs::read_to_string(cache_path).ok()?).ok()?;
    table.get(DEFAULT_PRESET_KEY)?.as_str().map(str::to_owned)
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

/// A cached land.toml written before the KR tonemap gain and the unused fourth components were removed:
/// - drops `lighting.kr_tonemap_params`. Where its `.x` acted (a KR-profile group drawn with the profile tone curve)
///   it multiplied exactly like `exposure`, so the slot's exposure takes it, and the slot looks the same;
/// - keeps the first three values of `grade_warm_color`, `grade_cool_color` and `gloom_params`, which a three-value
///   field would otherwise reject, failing the whole file.
///
/// A slot's tone curve missing from the file is the one the lenient load fills in from `defaults`.
/// Returns the migrated text, or `None` when there is nothing to migrate or the text is not valid TOML (the lenient
/// load reports that). A migrated file has no `kr_tonemap_params`, so the fold is never applied twice.
fn migrate_cached_retired_lighting(contents: &str, defaults: &LandShaderModePresets) -> Option<String> {
    let mut table: toml::Table = toml::from_str(contents).ok()?;
    let mut migrated = false;
    for group in PRESET_GROUPS {
        for period in PRESET_PERIODS {
            let Some(lighting) = table
                .get_mut(group)
                .and_then(|value| value.get_mut(period))
                .and_then(|value| value.get_mut("lighting"))
                .and_then(toml::Value::as_table_mut)
            else {
                continue;
            };
            for key in RETIRED_FOURTH_COMPONENT_KEYS {
                if let Some(toml::Value::Array(values)) = lighting.get_mut(key) {
                    if values.len() > THREE_COMPONENTS {
                        values.truncate(THREE_COMPONENTS);
                        migrated = true;
                    }
                }
            }
            let Some(kr_tonemap) = lighting.remove(RETIRED_KR_TONEMAP_KEY) else {
                continue;
            };
            migrated = true;
            let tone_curve = lighting
                .get("tone_curve")
                .and_then(toml::Value::as_integer)
                .and_then(|curve| u32::try_from(curve).ok())
                .or_else(|| get_preset_slot(defaults, &format!("{group}.{period}")).map(|slot| slot.lighting.tone_curve));
            if !KR_PROFILE_PRESET_GROUPS.contains(&group) || tone_curve != Some(TONE_CURVE_PROFILE) {
                continue;
            }
            let gain = kr_tonemap
                .as_array()
                .and_then(|values| values.first())
                .and_then(toml_number)
                .filter(|gain| *gain > 0.0)
                .unwrap_or(RETIRED_KR_TONEMAP_DEFAULT_GAIN);
            if let Some(exposure) = lighting.get("exposure").and_then(toml_number) {
                lighting.insert("exposure".to_string(), toml::Value::Float(exposure * gain));
            }
        }
    }
    migrated.then(|| toml::to_string(&table).ok()).flatten()
}

/// A TOML float, or an integer written where a float is meant.
fn toml_number(value: &toml::Value) -> Option<f64> {
    value.as_float().or_else(|| value.as_integer().map(|integer| integer as f64))
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
        // A slot keeps the dimming setting its style uses: the light level, or the scene multiplier.
        slot.global_lighting = u.client_lighting.is_none().then_some(u.global_lighting);
        slot.client_lighting = u.client_lighting;
        slot.client_shadows = u.client_shadows;
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
        client_lighting: s.client_lighting,
        client_shadows: s.client_shadows,
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

    let global_lighting = preset_scene_multiplier(preset);
    UniformState {
        effects: preset.effects,
        lighting: preset.lighting,
        land_lighting: preset.land_lighting,
        global_lighting,
        client_lighting: preset.client_lighting,
        client_shadows: preset.client_shadows,
        sun: preset.sun,
        dirty: true,
        saved_snapshot: Some(UniformSnapshot {
            effects: preset.effects,
            lighting: preset.lighting,
            land_lighting: preset.land_lighting,
            global_lighting,
            client_lighting: preset.client_lighting,
            client_shadows: preset.client_shadows,
            sun: preset.sun,
        }),
        active_preset: preset_key,
    }
}

/// The scene multiplier a preset slot applies: its `global_lighting`, or the identity where the clients' night
/// composite dims instead.
pub fn preset_scene_multiplier(preset: &LandMaterialUniformsPresets) -> f32 {
    preset.global_lighting.unwrap_or(CLIENT_LIT_SCENE_MULTIPLIER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(mode: u32, global_lighting: f32, exposure: f32) -> LandMaterialUniformsPresets {
        LandMaterialUniformsPresets {
            global_lighting: Some(global_lighting),
            client_lighting: None,
            client_shadows: None,
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

        assert_eq!(saved.classic.morning.global_lighting, Some(0.25));
        assert_eq!(saved.classic.morning.lighting.exposure, 0.75);
        assert_eq!(saved.kr.afternoon.global_lighting, Some(3.1));
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

        assert_eq!(effective.painterly.night.global_lighting, Some(9.0));
        assert_eq!(effective.kr.night.global_lighting, Some(3.2));
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

        assert_eq!(effective.classic.cave.global_lighting, Some(9.0));
        assert_eq!(effective.classic.cave.lighting.exposure, 8.0);
        assert_eq!(effective.classic.morning.global_lighting, Some(1.0));
        assert_eq!(effective.enhanced.cave.global_lighting, Some(2.3));
        assert_eq!(effective.default_preset, "classic.cave");
    }

    #[test]
    fn resetting_preset_restores_only_the_active_factory_combination() {
        let factory = presets();
        let mut effective = presets();
        effective.classic.morning.global_lighting = Some(9.0);
        effective.classic.cave.lighting.exposure = 8.0;
        effective.enhanced.afternoon.global_lighting = Some(7.0);
        effective.default_preset = "classic.cave".to_string();

        assert!(reset_preset_to_factory(
            &mut effective,
            &factory,
            "classic.cave",
        ));

        assert_eq!(effective.classic.morning.global_lighting, Some(9.0));
        assert_eq!(effective.classic.cave.lighting.exposure, 1.3);
        assert_eq!(effective.enhanced.afternoon.global_lighting, Some(7.0));
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

    /// A fresh install starts from the shipped land.toml, and Reset to Factory from the factory presets: they must be
    /// the same values, or the first reset changes a preset the user never edited.
    #[test]
    fn bundled_land_settings_match_the_factory_presets() {
        let shipped: toml::Table = toml::from_str(BUNDLED_LAND_SETTINGS).expect("shipped");
        let factory: toml::Table = toml::from_str(BUNDLED_FACTORY_PRESETS).expect("factory");
        assert_eq!(shipped, factory);
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

        let missing = BUNDLED_FACTORY_PRESETS.replacen("tone_curve = ", "# tone_curve = ", 1);
        let error = parse(&missing).err().expect("missing key").to_string();
        assert!(error.contains("missing field `tone_curve`"), "{error}");
    }

    #[test]
    fn cached_shader_settings_drop_retired_keys_and_fill_new_ones() {
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let cache = BUNDLED_LAND_SETTINGS
            .replacen("[classic.morning.effects]\n", "[classic.morning.effects]\n_pad_eff0 = 0.0\n", 1)
            .replacen("tone_curve = 0\n", "", 1);
        let load = uocf::utils::config_toml::parse_app_state("land.toml", &cache, &bundled).expect("lenient load");
        assert_eq!(
            load.warnings,
            [
                "land.toml: missing key `classic.morning.lighting.tone_curve` filled from the default",
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
        // Before UOD-Painterly, `kr` held the painterly look: the bundled painterly group renamed to `kr`, with an edit.
        let painterly = &BUNDLED_LAND_SETTINGS[painterly_start..];
        let kr_start = BUNDLED_LAND_SETTINGS.find("[kr.morning").expect("kr group");
        let old_cache = format!(
            "{}{}",
            &BUNDLED_LAND_SETTINGS[..kr_start],
            painterly
                .replace("[painterly.", "[kr.")
                .replacen("[kr.night]\nglobal_lighting = ", "[kr.night]\nglobal_lighting = 0.125\n# ", 1)
        )
        .replacen("default_preset = \"classic.morning\"", "default_preset = \"kr.night\"", 1);

        let migrated = migrate_cached_kr_to_painterly(&old_cache).expect("migrated");
        let load = uocf::utils::config_toml::parse_app_state("land.toml", &migrated, &bundled).expect("lenient load");

        assert_eq!(load.warnings, ["land.toml: missing key `kr` filled from the default"]);
        assert_eq!(load.value.painterly.night.global_lighting, Some(0.125));
        assert_eq!(load.value.kr.night.client_lighting, bundled.kr.night.client_lighting);
        assert_eq!(load.value.default_preset, "kr.night");
        assert!(migrate_cached_kr_to_painterly(BUNDLED_LAND_SETTINGS).is_none());
    }

    #[test]
    fn bundled_presets_dim_kr_like_and_enhanced_through_the_client_night_lighting() {
        use crate::core::render::scene::world::effects::client_night::NIGHT_MAP_MEAN;
        const DAY: u32 = 0;
        const NIGHT: u32 = 10;
        const DUNGEON: u32 = 26;
        const CLIENT_STRENGTH: f32 = 1.0;
        for (name, text) in [("shader_presets.toml", BUNDLED_FACTORY_PRESETS), ("land.toml", BUNDLED_LAND_SETTINGS)] {
            let presets = parse(text).expect("bundled");
            validate_preset_lighting(&presets).unwrap_or_else(|error| panic!("{name}: {error}"));
            for group in [&presets.kr, &presets.enhanced] {
                for (slot, level) in
                    [(&group.morning, DAY), (&group.afternoon, DAY), (&group.night, NIGHT), (&group.cave, DUNGEON)]
                {
                    let expected = ClientNightLighting {
                        light_level: level,
                        overlay_strength: CLIENT_STRENGTH,
                        overlay_color: NIGHT_MAP_MEAN,
                    };
                    assert_eq!(slot.client_lighting, Some(expected), "{name}");
                }
            }
        }
    }

    #[test]
    fn hand_written_presets_must_dim_each_group_its_own_way() {
        let mut presets = parse(BUNDLED_FACTORY_PRESETS).expect("bundled");
        presets.kr.night.global_lighting = Some(1.0);
        presets.classic.cave.client_lighting = presets.enhanced.cave.client_lighting;
        presets.enhanced.morning.client_lighting = None;
        presets.painterly.cave.global_lighting = None;
        let error = validate_preset_lighting(&presets).expect_err("mixed dimming").to_string();
        for expected in [
            "`kr.night.global_lighting` is not a setting there",
            "`classic.cave.client_lighting` is not a setting there",
            "missing `enhanced.morning.client_lighting`",
            "missing `painterly.cave.global_lighting`",
        ] {
            assert!(error.contains(expected), "{error}");
        }

        let mut presets = parse(BUNDLED_FACTORY_PRESETS).expect("bundled");
        presets.kr.cave.client_lighting.as_mut().unwrap().light_level = MAX_CLIENT_LIGHT_LEVEL + 1;
        let error = validate_preset_lighting(&presets).expect_err("out of range").to_string();
        assert!(error.contains("`kr.cave.client_lighting`: light_level 31 is above 30"), "{error}");
    }

    #[test]
    fn only_kr_like_has_the_client_shadows_and_draws_them_alone() {
        for (name, text) in [("shader_presets.toml", BUNDLED_FACTORY_PRESETS), ("land.toml", BUNDLED_LAND_SETTINGS)] {
            let presets = parse(text).expect("bundled");
            for (group, slots) in [
                (CLASSIC_PRESET_GROUP, &presets.classic),
                (ENHANCED_PRESET_GROUP, &presets.enhanced),
                (KR_PRESET_GROUP, &presets.kr),
                (PAINTERLY_PRESET_GROUP, &presets.painterly),
            ] {
                for slot in [&slots.morning, &slots.afternoon, &slots.night, &slots.cave] {
                    let expected = preset_group_uses_client_shadows(group).then_some(true);
                    assert_eq!(slot.client_shadows, expected, "{name} {group}");
                }
            }
            // KR-like's client shadow replaces its sprite shadows.
            for slot in [&presets.kr.morning, &presets.kr.afternoon, &presets.kr.night, &presets.kr.cave] {
                let effects = &slot.effects;
                assert_eq!(effects.art_shadow_strength, 0.0, "{name}");
                assert_eq!(effects.art_contact_shadow_strength, 0.0, "{name}");
                assert_eq!(effects.enable_art_projected_shadows, 0, "{name}");
                assert_eq!(effects.art_projected_shadow_strength, 0.0, "{name}");
            }
        }

        let mut presets = parse(BUNDLED_FACTORY_PRESETS).expect("bundled");
        presets.kr.night.client_shadows = None;
        presets.painterly.morning.client_shadows = Some(true);
        let error = validate_preset_lighting(&presets).expect_err("misplaced client shadows").to_string();
        assert!(error.contains("missing `kr.night.client_shadows`"), "{error}");
        assert!(error.contains("`painterly.morning.client_shadows` is not a setting there"), "{error}");

        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let mut cached = clone_presets(&bundled);
        cached.kr.cave.client_shadows = None;
        cached.classic.night.client_shadows = Some(false);
        let warnings = normalize_cached_preset_lighting(&mut cached, &bundled, "land.toml");
        assert_eq!(
            warnings,
            [
                "land.toml: `classic.night.client_shadows` dropped: only KR-like has the KR client's shadows",
                "land.toml: missing key `kr.cave.client_shadows` filled from the default",
            ]
        );
        assert_eq!(cached.kr.cave.client_shadows, Some(true));
        assert_eq!(cached.classic.night.client_shadows, None);
    }

    #[test]
    fn cached_settings_from_before_the_night_composite_drop_kr_and_enhanced_global_lighting() {
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        // A cache from before: KR-like and Enhanced had a global_lighting and no client_lighting.
        let mut old = clone_presets(&bundled);
        old.kr.night.client_lighting = None;
        old.kr.night.global_lighting = Some(0.5);
        old.enhanced.cave.client_lighting.as_mut().unwrap().overlay_strength = MAX_NIGHT_OVERLAY_STRENGTH * 2.0;
        let text = toml::to_string_pretty(&old).expect("serialize");
        // The lenient merge fills the missing client_lighting and keeps global_lighting, a known key.
        let mut load = uocf::utils::config_toml::parse_app_state("land.toml", &text, &bundled).expect("lenient load");
        assert_eq!(load.warnings, ["land.toml: missing key `kr.night.client_lighting` filled from the default"]);

        let warnings = normalize_cached_preset_lighting(&mut load.value, &bundled, "land.toml");
        assert_eq!(
            warnings,
            [
                "land.toml: `enhanced.cave.client_lighting` clamped: overlay_strength 4 is outside 0..=2",
                "land.toml: `kr.night.global_lighting` dropped: this style dims through `client_lighting`",
            ]
        );
        assert_eq!(load.value.kr.night.global_lighting, None);
        assert_eq!(load.value.kr.night.client_lighting, bundled.kr.night.client_lighting);
        assert_eq!(
            load.value.enhanced.cave.client_lighting.map(|lighting| lighting.overlay_strength),
            Some(MAX_NIGHT_OVERLAY_STRENGTH)
        );
        validate_preset_lighting(&load.value).expect("normalized");
        let rewritten = toml::to_string_pretty(&load.value).expect("serialize");
        parse(&rewritten).expect("rewritten file is complete");
    }

    /// A cached file in the format before the KR tonemap gain was removed: every slot has `kr_tonemap_params` with
    /// `gain`, except `painterly.night`, left unset, and four-value grade colours and gloom parameters.
    fn cache_with_retired_lighting(gain: f64) -> toml::Table {
        let mut table: toml::Table = toml::from_str(BUNDLED_LAND_SETTINGS).expect("bundled");
        for group in PRESET_GROUPS {
            for period in PRESET_PERIODS {
                let lighting = table[group][period]["lighting"].as_table_mut().expect("lighting");
                let unset = group == PAINTERLY_PRESET_GROUP && period == "night";
                let x = if unset { 0.0 } else { gain };
                lighting.insert(RETIRED_KR_TONEMAP_KEY.to_string(), toml::Value::try_from([x, 0.0, 0.0, 0.0]).unwrap());
                for key in RETIRED_FOURTH_COMPONENT_KEYS {
                    lighting[key].as_array_mut().expect("array").push(toml::Value::Float(0.0));
                }
            }
        }
        table
    }

    #[test]
    fn cached_kr_tonemap_gain_folds_into_the_exposure_where_it_acted() {
        const GAIN: f64 = 0.5;
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let old = cache_with_retired_lighting(GAIN);
        let migrated = migrate_cached_retired_lighting(&toml::to_string(&old).unwrap(), &bundled).expect("migrated");
        let load = uocf::utils::config_toml::parse_app_state("land.toml", &migrated, &bundled).expect("lenient load");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        assert!(migrate_cached_retired_lighting(&migrated, &bundled).is_none(), "the fold applies once");
        assert!(migrate_cached_retired_lighting(BUNDLED_LAND_SETTINGS, &bundled).is_none());
        assert_eq!(bundled.painterly.morning.lighting.tone_curve, TONE_CURVE_PROFILE, "the fold is exercised");

        for group in PRESET_GROUPS {
            for period in PRESET_PERIODS {
                let key = format!("{group}.{period}");
                let before = get_preset_slot(&bundled, &key).unwrap();
                let after = get_preset_slot(&load.value, &key).unwrap();
                let acted = KR_PROFILE_PRESET_GROUPS.contains(&group) && before.lighting.tone_curve == TONE_CURVE_PROFILE;
                let gain = match (acted, key.as_str()) {
                    (false, _) => 1.0,
                    (true, "painterly.night") => RETIRED_KR_TONEMAP_DEFAULT_GAIN,
                    (true, _) => GAIN,
                };
                assert_eq!(after.lighting.exposure, (f64::from(before.lighting.exposure) * gain) as f32, "{key}");
                assert_eq!(after.lighting.grade_warm_color, before.lighting.grade_warm_color, "{key}");
                assert_eq!(after.lighting.grade_cool_color, before.lighting.grade_cool_color, "{key}");
                assert_eq!(after.lighting.gloom_params, before.lighting.gloom_params, "{key}");
            }
        }
    }

    #[test]
    fn cached_tone_curve_left_unset_takes_the_default_curve_for_the_fold() {
        let bundled = parse(BUNDLED_LAND_SETTINGS).expect("bundled");
        let mut old = cache_with_retired_lighting(RETIRED_KR_TONEMAP_DEFAULT_GAIN);
        for group in KR_PROFILE_PRESET_GROUPS {
            old[group]["morning"]["lighting"].as_table_mut().unwrap().remove("tone_curve");
        }
        let migrated = migrate_cached_retired_lighting(&toml::to_string(&old).unwrap(), &bundled).expect("migrated");
        let migrated: toml::Table = toml::from_str(&migrated).unwrap();
        let exposure = |table: &toml::Table, group: &str| table[group]["morning"]["lighting"]["exposure"].as_float().unwrap();
        assert_eq!(
            exposure(&migrated, PAINTERLY_PRESET_GROUP),
            exposure(&old, PAINTERLY_PRESET_GROUP) * RETIRED_KR_TONEMAP_DEFAULT_GAIN
        );
        assert_eq!(exposure(&migrated, KR_PRESET_GROUP), exposure(&old, KR_PRESET_GROUP));
    }

    /// The KR and Enhanced clients' terrain light, grey = (N.D + 1) / 2 with D = (0, -0.7071, -0.7071) (KR
    /// 0x00c0f3b8, EC 0x00dc18b8). UOTerrain::ComputeVertexNormal (KR 0x004488a0) builds N in x = cell column,
    /// y = -z * 0.12856454 (height negated) and z = -cell row, so flat ground has N = (0, -1, 0) and D points
    /// towards the light: up and to increasing rows. In our axes (y up, +z south) that is (0, 0.7071, 0.7071):
    /// from the south at 45 degrees.
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
    fn bundled_presets_keep_the_baked_sun_but_kr_like_and_enhanced_take_the_client_sun() {
        for text in [BUNDLED_FACTORY_PRESETS, BUNDLED_LAND_SETTINGS] {
            let presets = parse(text).expect("bundled");
            for (group, expected) in [
                (&presets.classic, SunAngles::default()),
                (&presets.enhanced, CLIENT_SUN),
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

    #[test]
    fn bundled_blur_radius_is_on_the_slider_from_the_shader_floor() {
        use crate::core::render::scene::world::land::mesh_material::{BLUR_RADIUS_MAX_PIXELS, BLUR_RADIUS_MIN_PIXELS};
        for text in [BUNDLED_FACTORY_PRESETS, BUNDLED_LAND_SETTINGS] {
            let presets = parse(text).expect("bundled");
            for group in [&presets.classic, &presets.enhanced, &presets.kr, &presets.painterly] {
                for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                    let radius = slot.effects.blur_radius;
                    assert!((BLUR_RADIUS_MIN_PIXELS..=BLUR_RADIUS_MAX_PIXELS).contains(&radius), "{radius}");
                }
            }
        }
    }

    /// Enhanced's per-pixel land light is a plain Lambert (shade_mode1_enhanced_fragment): only KR shading reads the
    /// wrap, N.L * (1 - wrap) + wrap, so Enhanced keeps it at its neutral value, as it does the rim colour and the
    /// headroom reserve, which only KR shading reads too.
    const ENHANCED_UNREAD_DIFFUSE_WRAP: f32 = 0.0;
    const ENHANCED_UNREAD_RIM_COLOR: Vec3 = Vec3::ONE;
    const ENHANCED_UNREAD_HEADROOM_RESERVE: f32 = 0.0;
    const CLIENT_DIFFUSE_STRENGTH: f32 = 1.0;

    #[test]
    fn enhanced_leaves_off_the_terms_and_static_shadows_the_client_lacks() {
        use crate::core::render::scene::world::land::mesh_material::{
            TERRAIN_SHADING_CLIENT_PER_VERTEX, TONE_CURVE_IDENTITY,
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
                // Both clients shade the terrain with the same per-vertex code.
                assert_eq!(land.terrain_shading_model, TERRAIN_SHADING_CLIENT_PER_VERTEX);
                assert_eq!(land.enable_bent, 0);
                assert_eq!(land.specular_strength, 0.0);
                assert_eq!(land.rim_strength, 0.0);
                assert_eq!(land.fill_strength, 0.0);
                assert_eq!(land.sharpness_mix, 0.0);
                assert_eq!(land.diffuse_wrap, ENHANCED_UNREAD_DIFFUSE_WRAP);
                assert_eq!(land.rim_color.truncate(), ENHANCED_UNREAD_RIM_COLOR);
                assert_eq!(lighting.grade_params.y, ENHANCED_UNREAD_HEADROOM_RESERVE);
                assert_eq!(land.diffuse_strength, CLIENT_DIFFUSE_STRENGTH);
            }
        }
    }
}
