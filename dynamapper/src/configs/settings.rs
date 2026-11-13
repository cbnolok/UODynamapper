use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::export_job::ExportJob;
use crate::core::render::dialogs::image_background::ImageBackground;
use crate::core::render::dialogs::paperdoll_ec::EnhancedPaperdoll;
use crate::core::render::scene::camera::RenderZoom;
use crate::core::render::scene::player::Player;
use crate::prelude::*;
use crate::util_lib::tracked_plugin::set_plugin_log_toggles;
use crate::util_lib::uo_coords::*;
use bevy::{
    //asset::{AssetLoader, LoadContext, io::Reader},
    pbr::wireframe::WireframeConfig,
    prelude::*,
    window::WindowResolution,
};
use color_eyre::eyre;
use serde::{Deserialize, Serialize};

#[derive(Asset, Clone, Deserialize, Serialize, Resource, TypePath)]
pub struct Settings {
    pub core: SectCore,
    pub graphics: SectGraphics,
    pub runtime_assets: SectRuntimeAssets,
    pub app: SectApp,
    pub session_state: SectSessionState,
    pub logging: SectLogging,
    pub keybindings: SectKeybindings,
    pub world_rendering: SectWorldRendering,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectKeybindings {
    pub shader_settings: KeyCode,
    pub user_settings: KeyCode,
    pub keybindings_help: KeyCode,
    pub increase_z: KeyCode,
    pub decrease_z: KeyCode,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectCore {
    pub world: SectWorld,
}

// User-settings section. The bundled file is hand-written and read strictly; the cache copy the app writes
// is read leniently (see `load_user_settings`), so a cache file written by an older build (missing
// newly-added keys) loads with the bundled values instead of stopping the app.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectApp {
    pub input: SectInput,
    pub window: SectWindow,
    pub debug: SectDebug,
    pub performance: SectPerformance,
}

// Session state: relocated to the cache dir, where it is read leniently like the user settings.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectSessionState {
    pub world: SectSessionWorld,
    pub window: SectSessionWindow,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectSessionWorld {
    pub last_p: UOVec4,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectSessionWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectRuntimeAssets {
    pub udd_path: String,
    pub load_gumps: bool,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectWindow {
    pub ui_scale: f32,
    /// Per-overlay scale for the player position overlay.
    pub player_position_scale: f32,
    /// Per-overlay scale for system messages rendered via egui.
    pub sysmessages_scale: f32,
    /// Per-overlay scale for the performance overlay.
    pub performance_overlay_scale: f32,
    /// Per-overlay scale for the cursor position overlay.
    pub cursor_position_scale: f32,
    pub free_camera: bool,
    pub perspective_camera: bool,
    /// What the gump and paperdoll windows draw behind their image; set in the Open Gump dialog.
    pub gump_background: ImageBackground,
    /// What Enhanced gumps and paperdoll (`graphics.gumps_and_paperdoll`) draw for a paperdoll: the client's 3D frames or
    /// the old-style gumps; set in the Open Gump dialog.
    pub enhanced_paperdoll: EnhancedPaperdoll,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectWorld {
    pub start_p: UOVec4,
    pub hide_player: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectWorldRendering {
    pub enable_statics: bool,
    pub enable_static_lights: bool,
    pub enable_static_art_local_lights: bool,
    /// Also draws the multi parts the client never draws (classic parts without the visible
    /// flag, EC `not_drawn` parts).
    pub show_hidden_multi_parts: bool,
    /// Hides what is above the player, as the client does: statics, multis and land at or above the cap the player's
    /// cell sets (a ceiling, a roof one cell south-east, or land over the player in a cave) are not drawn, and roofs are
    /// not drawn while the player is under one. A view setting shared by every render style.
    pub show_only_player_floor: bool,
    pub land_streaming: SectLandStreaming,
    pub diagnostics: SectWorldRenderingDiagnostics,
    pub shader_simplification: SectWorldRenderingShaderSimplification,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SectWorldRenderingShaderSimplification {
    /// Forces a temporary minimal land shader configuration on the shared land material.
    pub force_minimal_shader: bool,
    /// Forces the land fragment shader to return a flat debug color immediately.
    pub force_flat_fragment_shader: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectWorldRenderingDiagnostics {
    /// Enables Bevy's low-level render diagnostics plugin.
    ///
    /// This gathers GPU pass timings and pipeline statistics. It is useful when profiling, but
    /// on some drivers it can perturb frame time noticeably. Restart required after changing it.
    pub enable_render_diagnostics: bool,
    /// Enables compact periodic worldmap diagnostics in the console.
    pub dump_to_console: bool,
    /// Period between compact diagnostics dumps.
    pub dump_interval_sec: f32,
    /// Include a compact render-pass GPU breakdown in the dump.
    pub log_render_breakdown: bool,
    /// Include compact world/chunk state in the dump.
    pub log_world_state: bool,
    /// Include compact CPU-side system timing in the dump, plus auxiliary
    /// upload/extract helpers and a residual frame-gap estimate.
    pub log_system_timers: bool,
    /// Draw a debug outline around the currently hovered static-world object.
    pub highlight_hovered_static_object: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectLandStreaming {
    /// Main-world land preparation budget measured in equivalent 8x8 map blocks.
    ///
    /// This limits how many ready chunks we are allowed to expand, precache and enqueue
    /// into the atlas in a single frame. Set to 0 to disable the cap entirely.
    pub prepare_max_blocks_per_frame: usize,
    /// Maximum number of metadata-atlas `write_texture` uploads that may be submitted
    /// during a single render-world queue pass. Set to 0 to disable the cap.
    pub upload_max_ops_per_frame: usize,
    /// Maximum number of metadata-atlas bytes that may be uploaded in a single render
    /// frame. Set to 0 to disable the byte cap.
    pub upload_max_bytes_per_frame: usize,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectDebug {
    pub map_render_wireframe: bool,
}

// Derived `Default` exists only to satisfy the `Default` of `SectApp`; the effective
// values always come from the settings files.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectPerformance {
    /// Master toggle for all on-screen overlays. When off, every overlay is hidden
    /// regardless of the per-overlay flags below.
    pub show_overlay: bool,
    /// Per-overlay toggles, gated behind `show_overlay`.
    pub show_perf_stats_overlay: bool,
    pub show_player_position_overlay: bool,
    pub show_cursor_inspect_overlay: bool,
    pub frame_limit_enabled: bool,
    pub target_fps: u32,
}

impl SectPerformance {
    /// Effective visibility of the FPS / performance-stats overlay: master AND its own toggle.
    pub fn perf_stats_overlay_visible(&self) -> bool {
        self.show_overlay && self.show_perf_stats_overlay
    }
    /// Effective visibility of the player-position overlay: master AND its own toggle.
    pub fn player_position_overlay_visible(&self) -> bool {
        self.show_overlay && self.show_player_position_overlay
    }
    /// Effective visibility of the cursor / tile-inspector overlay: master AND its own toggle.
    pub fn cursor_inspect_overlay_visible(&self) -> bool {
        self.show_overlay && self.show_cursor_inspect_overlay
    }
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectGraphics {
    pub reduce_unfocused_fps: bool,
    pub vsync: bool, // Added vsync control
    pub anti_aliasing: AntiAliasingMode,
    pub art_texture_source: ClientTextureSource,
    pub land_texture_source: LandTextureSource,
    /// The profile of the route file (runtime_specs/land/LandRoutes.kdl) in force, such as a season; empty for none.
    pub route_profile: String,
    /// Which client's gumps and paperdoll the gump windows draw; like the art and land sources, never the preset's.
    pub gumps_and_paperdoll: GumpClient,
    pub cc_terrain_transitions: bool,
    pub ec_terrain_transitions: bool,
    pub sharpening_strength: f32, // 0.0 to 1.0
}

impl SectGraphics {
    /// Shared GPU word: bit 0 enables CC smoothing, bit 1 disables EC smoothing.
    /// Zero retains the established EC-on / CC-off rendering policy.
    pub fn terrain_transition_flags(&self) -> u32 {
        u32::from(self.cc_terrain_transitions) | (u32::from(!self.ec_terrain_transitions) << 1)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub enum ClientTextureSource {
    #[default]
    #[serde(rename = "cc")]
    Cc,
    #[serde(rename = "ec")]
    Ec,
}

impl ClientTextureSource {
    pub const ALL: [Self; 2] = [Self::Cc, Self::Ec];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cc => "Classic",
            Self::Ec => "Enhanced",
        }
    }

    pub const fn art_label(self) -> &'static str {
        match self {
            Self::Cc => "Classic (tex_art_cc.uddp)",
            Self::Ec => "Enhanced (tex_art_ec.uddp)",
        }
    }
}

/// The package the terrain is drawn from. Kingdom Reborn land (tex_land_kr.uddp) has the Enhanced land layout and is
/// drawn by the Enhanced terrain path; the art source stays Classic or Enhanced, since the art packages have no
/// Kingdom Reborn counterpart. The shader presets never choose it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub enum LandTextureSource {
    #[default]
    #[serde(rename = "cc")]
    Cc,
    #[serde(rename = "ec")]
    Ec,
    #[serde(rename = "kr")]
    Kr,
}

impl LandTextureSource {
    pub const ALL: [Self; 3] = [Self::Cc, Self::Ec, Self::Kr];

    pub const fn land_label(self) -> &'static str {
        match self {
            Self::Cc => "Classic (tex_land_cc.uddp)",
            Self::Ec => "Enhanced (tex_land_ec.uddp)",
            Self::Kr => "Kingdom Reborn (tex_land_kr.uddp)",
        }
    }

    /// Whether the terrain is drawn from a package in the Enhanced land layout (tex_land_ec.uddp or tex_land_kr.uddp).
    pub const fn uses_enhanced_land(self) -> bool {
        matches!(self, Self::Ec | Self::Kr)
    }
}

impl From<ClientTextureSource> for LandTextureSource {
    fn from(source: ClientTextureSource) -> Self {
        match source {
            ClientTextureSource::Cc => Self::Cc,
            ClientTextureSource::Ec => Self::Ec,
        }
    }
}

/// Which client's gumps and paperdoll are drawn: the "Gumps and paperdoll" preference, `graphics.gumps_and_paperdoll`
/// (the user, 2026-11-07). The shader preset never picks it: presets set shader settings only, never which assets are
/// drawn. Classic draws the Classic client's gumps and the gump paperdoll; Enhanced the Enhanced client's gumps and,
/// as `window.enhanced_paperdoll` asks, its 3D paperdoll.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum GumpClient {
    #[default]
    #[serde(rename = "classic")]
    Classic,
    #[serde(rename = "enhanced")]
    Enhanced,
}

impl GumpClient {
    pub const ALL: [Self; 2] = [Self::Classic, Self::Enhanced];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic (gumps_cc.uddp)",
            Self::Enhanced => "Enhanced (gumps_ec.uddp, paperdoll_ec.uddp)",
        }
    }

    /// The hover text of each choice in Preferences.
    pub const fn hover_text(self) -> &'static str {
        match self {
            Self::Classic => {
                "The Classic client's gumps, from gumps_cc.uddp or else the Classic client's gump files, and its \
                 paperdoll of gumps."
            }
            Self::Enhanced => {
                "The Enhanced client's gumps, from gumps_ec.uddp (the Classic ones for ids it lacks), and its 3D \
                 paperdoll from paperdoll_ec.uddp, or the paperdoll of gumps when the Open Gump dialog asks for it."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
pub enum AntiAliasingMode {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "fxaa")]
    Fxaa,
    #[serde(rename = "smaa")]
    Smaa,
    #[serde(rename = "msaa_2x")]
    Msaa2x,
    #[default]
    #[serde(rename = "msaa_4x")]
    Msaa4x,
}

impl AntiAliasingMode {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Fxaa,
        Self::Smaa,
        Self::Msaa2x,
        Self::Msaa4x,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
            Self::Msaa2x => "MSAA 2x",
            Self::Msaa4x => "MSAA 4x",
        }
    }
}

/// Resource used to debounce saving settings to disk.
#[derive(Resource)]
pub struct SettingsSaveTimer(pub Timer);

/// Tracks file modification times for hot-reload detection.
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SectLogging {
    pub min_severity: LogSev,
    pub emit_flat_plugin_build: bool,
    pub emit_tree_plugin_build: bool,
    pub filters: Vec<LogFilterSetting>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LogFilterSetting {
    pub sev: Option<LogSev>,
    pub about: Option<LogAbout>,
    pub suppress: bool,
}

// ----

#[derive(Message, Clone, Default, PartialEq)]
pub struct ToggleWireframe;

// ----

const CORE_CONFIG_FILE: &str = "settings/core.toml";
const RUNTIME_ASSETS_CONFIG_FILE: &str = "settings/runtime_assets.toml";
const USER_PREFERENCES_CONFIG_FILE: &str = "settings/user_preferences.toml";
const SESSION_STATE_CONFIG_FILE: &str = "settings/session_state.toml";
const KEYBINDINGS_CONFIG_FILE: &str = "settings/keybindings.toml";
const GRAPHICS_CONFIG_FILE: &str = "settings/graphics.toml";
const WORLD_RENDERING_CONFIG_FILE: &str = "settings/world_rendering.toml";

/// Process exit code when a settings file or a bundled spec file cannot be read.
pub const STARTUP_CONFIG_ERROR_EXIT_CODE: u8 = 2;

/// Shows a configuration error that stops the app: logged, and printed to stderr.
pub fn report_startup_config_error(error: &eyre::Report) {
    let message = format!("{error:#}");
    console_logger::one(LogSev::Error, LogAbout::Startup, &message);
    eprintln!("dynamapper: {message}");
}

/// Reports a configuration error and exits, for loaders that run inside the Bevy app (plugin build, startup
/// systems), where no error can be returned to `main`.
pub fn exit_on_startup_config_error(error: &eyre::Report) -> ! {
    report_startup_config_error(error);
    std::process::exit(i32::from(STARTUP_CONFIG_ERROR_EXIT_CODE));
}

/// Reads and strictly parses a hand-written file under `assets/`: unknown and missing keys are errors.
pub(crate) fn load_bundled_settings<T: serde::de::DeserializeOwned>(rel: &str) -> eyre::Result<T> {
    let path = crate::core::constants::valid_asset_dir().join(rel);
    let contents =
        std::fs::read_to_string(&path).map_err(|error| eyre::eyre!("{}: cannot read: {error}", path.display()))?;
    uocf::utils::config_toml::parse_strict(&path.display().to_string(), &contents)
}

/// Reads a user-side settings file: the bundled default under `assets/` (hand-written, strict), then the cache
/// copy the app writes, if any. The cache copy is lenient: keys it lacks keep the bundled values, unknown keys are
/// dropped and keys with a value this build rejects take the bundled value, each with a warning; the app rewrites it
/// complete on its next save. A cache copy that cannot be read at all is replaced by the bundled values, with a
/// warning.
pub(crate) fn load_user_settings<T>(rel: &str) -> eyre::Result<T>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    load_user_settings_migrated(rel, |_| None)
}

/// [`load_user_settings`], with `migrate` run first on the text of the cache copy: it returns the migrated text, or
/// `None` to read the text as it is.
fn load_user_settings_migrated<T>(rel: &str, migrate: impl FnOnce(&str) -> Option<String>) -> eyre::Result<T>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let bundled: T = load_bundled_settings(rel)?;
    let cache_path = crate::core::constants::valid_cache_dir().join(rel);
    let Ok(contents) = std::fs::read_to_string(&cache_path) else {
        return Ok(bundled);
    };
    let contents = migrate(&contents).unwrap_or(contents);
    match uocf::utils::config_toml::parse_app_state(&cache_path.display().to_string(), &contents, &bundled) {
        Ok(load) => {
            for warning in &load.warnings {
                console_logger::one(LogSev::Warn, LogAbout::Settings, warning);
            }
            Ok(load.value)
        }
        Err(error) => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Settings,
                &format!("{error:#}; using the bundled settings, the file is rewritten on the next save"),
            );
            Ok(bundled)
        }
    }
}

/// Core settings file contains top-level core and logging sections.
#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CoreFile {
    core: SectCore,
    logging: SectLogging,
}

/// Graphics settings live in a `[graphics]` table, as `save_graphics_settings` writes them.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GraphicsFile {
    graphics: SectGraphics,
}

/// The `[graphics]` table of graphics.toml and its "Gumps and paperdoll" key.
const GRAPHICS_TABLE: &str = "graphics";
const GUMPS_AND_PAPERDOLL_KEY: &str = "gumps_and_paperdoll";

/// A cached graphics.toml written before the "Gumps and paperdoll" preference, when the shader preset picked the gumps
/// and paperdoll: the Classic preset the Classic client's, every other preset the Enhanced client's. Sets the
/// preference to what `saved_preset`, the preset saved as selected, drew, so the gumps stay as they were.
///
/// Returns the migrated text and the choice, or `None` when the file has the key, has no `[graphics]` table or is not
/// valid TOML (the lenient load reports that).
fn migrate_cached_gumps_and_paperdoll(contents: &str, saved_preset: &str) -> Option<(String, GumpClient)> {
    let mut table: toml::Table = toml::from_str(contents).ok()?;
    let graphics = table.get_mut(GRAPHICS_TABLE)?.as_table_mut()?;
    if graphics.contains_key(GUMPS_AND_PAPERDOLL_KEY) {
        return None;
    }
    let client = gumps_and_paperdoll_of_preset(saved_preset);
    graphics.insert(GUMPS_AND_PAPERDOLL_KEY.to_string(), toml::Value::try_from(client).ok()?);
    Some((toml::to_string(&table).ok()?, client))
}

/// Without a cached graphics.toml, the gumps and paperdoll `saved_preset` drew before the "Gumps and paperdoll"
/// preference, when they differ from the bundled Classic ones: the Enhanced client's for a saved preset other than
/// Classic's. `None` for a Classic preset, or when no preset was saved (a fresh install).
fn gumps_and_paperdoll_without_a_cache_copy(saved_preset: Option<&str>) -> Option<GumpClient> {
    let client = gumps_and_paperdoll_of_preset(saved_preset?);
    (client != GumpClient::Classic).then_some(client)
}

/// The gumps and paperdoll a shader preset drew when the preset picked them: the Classic client's for the Classic
/// presets, the Enhanced client's for every other.
fn gumps_and_paperdoll_of_preset(preset: &str) -> GumpClient {
    if preset.split('.').next() == Some(crate::configs::shader_presets::CLASSIC_PRESET_GROUP) {
        GumpClient::Classic
    } else {
        GumpClient::Enhanced
    }
}

pub fn load_from_files() -> eyre::Result<Settings> {
    // Every file: the bundled default under assets/, then the cache copy the app saved, if any.
    let core_data: CoreFile = load_user_settings(CORE_CONFIG_FILE)?;
    let runtime_assets: SectRuntimeAssets = load_user_settings(RUNTIME_ASSETS_CONFIG_FILE)?;
    // User preferences file contains SectApp fields directly at top level.
    let user_app: SectApp = load_user_settings(USER_PREFERENCES_CONFIG_FILE)?;
    let session_state: SectSessionState = load_user_settings(SESSION_STATE_CONFIG_FILE)?;
    // Gumps and paperdoll the saved shader preset drew before they were a preference of their own.
    let saved_preset = crate::configs::shader_presets::saved_preset_key();
    let graphics_cached = crate::core::constants::valid_cache_dir().join(GRAPHICS_CONFIG_FILE).is_file();
    let mut kept_gumps = None;
    let mut graphics = load_user_settings_migrated::<GraphicsFile>(GRAPHICS_CONFIG_FILE, |contents| {
        let (migrated, client) = migrate_cached_gumps_and_paperdoll(contents, saved_preset.as_deref()?)?;
        kept_gumps = Some(client);
        Some(migrated)
    })?
    .graphics;
    if !graphics_cached && let Some(client) = gumps_and_paperdoll_without_a_cache_copy(saved_preset.as_deref()) {
        graphics.gumps_and_paperdoll = client;
        kept_gumps = Some(client);
    }
    if let (Some(client), Some(saved_preset)) = (kept_gumps, &saved_preset) {
        console_logger::one(
            LogSev::Warn,
            LogAbout::Settings,
            &format!(
                "{GRAPHICS_CONFIG_FILE}: the saved shader preset `{saved_preset}` drew the gumps and paperdoll; they are \
                 the \"Gumps and paperdoll\" preference now, set to {} to keep them",
                client.label()
            ),
        );
    }
    let keybindings: SectKeybindings = load_user_settings(KEYBINDINGS_CONFIG_FILE)?;
    let world_rendering: SectWorldRendering = load_user_settings(WORLD_RENDERING_CONFIG_FILE)?;

    let settings = Settings {
        core: core_data.core,
        graphics,
        runtime_assets,
        app: user_app,
        session_state,
        logging: core_data.logging,
        keybindings,
        world_rendering,
    };
    // Written at once, so the migration runs and is logged a single time.
    if kept_gumps.is_some() {
        save_graphics_settings(&settings);
    }
    Ok(settings)
}

pub fn apply_logging_settings(logging: &SectLogging) {
    let mut filters = Vec::new();
    for f in &logging.filters {
        filters.push(console_logger::LogFilter {
            sev: f.sev.clone(),
            about: f.about.clone(),
            suppress: f.suppress,
        });
    }

    console_logger::set_log_settings(console_logger::LogSettings {
        min_severity: Some(logging.min_severity.clone()),
        filters,
    });
}

pub fn save_app_settings(settings: &Settings) {
    // User settings: written to the cache dir, never the read-only asset tree.
    let user_path = crate::core::constants::valid_cache_dir().join(USER_PREFERENCES_CONFIG_FILE);
    if let Some(parent) = user_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match toml::to_string_pretty(&settings.app) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&user_path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save user_preferences.toml: {}", e),
                );
            } else {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Settings,
                    "Saved user_preferences.toml",
                );
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize user preferences: {}", e),
            );
        }
    }
}

pub fn save_session_state_settings(settings: &Settings) {
    // Session state: written to the cache dir, never the read-only asset tree.
    let path = crate::core::constants::valid_cache_dir().join(SESSION_STATE_CONFIG_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match toml::to_string_pretty(&settings.session_state) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save session_state.toml: {}", e),
                );
            } else {
                console_logger::one(LogSev::Info, LogAbout::Settings, "Saved session_state.toml");
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize session state: {}", e),
            );
        }
    }
}

/// What the cache copy of the settings file `rel` should hold. There is none while the settings equal the bundled
/// file, so later edits of that file keep taking effect until the app has something of its own to keep.
fn cache_copy_contents<T>(rel: &str, value: &T) -> eyre::Result<Option<String>>
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq,
{
    if *value == load_bundled_settings::<T>(rel)? {
        return Ok(None);
    }
    Ok(Some(toml::to_string_pretty(value)?))
}

/// Writes or removes the cache copy of the settings file `rel`. The bundled file under `assets/` is never written.
fn save_cache_copy<T>(rel: &str, value: &T)
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq,
{
    let path = crate::core::constants::valid_cache_dir().join(rel);
    let saved = cache_copy_contents(rel, value).and_then(|contents| {
        let Some(contents) = contents else {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(false),
            };
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        Ok(true)
    });
    match saved {
        Ok(true) => console_logger::one(LogSev::Info, LogAbout::Settings, &format!("Saved {rel}")),
        Ok(false) => {}
        Err(error) => {
            console_logger::one(LogSev::Error, LogAbout::Settings, &format!("Failed to save {rel}: {error:#}"));
        }
    }
}

pub fn save_keybindings(settings: &Settings) {
    save_cache_copy(KEYBINDINGS_CONFIG_FILE, &settings.keybindings);
}

pub fn save_graphics_settings(settings: &Settings) {
    // User graphics prefs: written to the cache dir, never the read-only asset tree.
    let graphics_path = crate::core::constants::valid_cache_dir().join(GRAPHICS_CONFIG_FILE);
    if let Some(parent) = graphics_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match toml::to_string_pretty(&GraphicsFile {
        graphics: settings.graphics.clone(),
    }) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&graphics_path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save graphics.toml: {}", e),
                );
            } else {
                console_logger::one(LogSev::Info, LogAbout::Settings, "Saved graphics.toml");
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize graphics settings: {}", e),
            );
        }
    }
}

pub fn save_core_settings(settings: &Settings) {
    let core = CoreFile { core: settings.core.clone(), logging: settings.logging.clone() };
    save_cache_copy(CORE_CONFIG_FILE, &core);
}

pub fn save_world_rendering_settings(settings: &Settings) {
    save_cache_copy(WORLD_RENDERING_CONFIG_FILE, &settings.world_rendering);
}

pub fn save_runtime_assets_settings(settings: &Settings) {
    save_cache_copy(RUNTIME_ASSETS_CONFIG_FILE, &settings.runtime_assets);
}

#[derive(Clone, PartialEq)]
struct SettingsSaveSnapshot {
    app: SectApp,
    session_state: SectSessionState,
    graphics: SectGraphics,
    keybindings: SectKeybindings,
    core: SectCore,
    logging: SectLogging,
    runtime_assets: SectRuntimeAssets,
    world_rendering: SectWorldRendering,
}

impl SettingsSaveSnapshot {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            app: settings.app.clone(),
            session_state: settings.session_state.clone(),
            graphics: settings.graphics.clone(),
            keybindings: settings.keybindings.clone(),
            core: settings.core.clone(),
            logging: settings.logging.clone(),
            runtime_assets: settings.runtime_assets.clone(),
            world_rendering: settings.world_rendering.clone(),
        }
    }

    fn changes_from(&self, other: &Self) -> SettingsChangeSet {
        SettingsChangeSet {
            app: self.app != other.app,
            session_state: self.session_state != other.session_state,
            graphics: self.graphics != other.graphics,
            keybindings: self.keybindings != other.keybindings,
            core: self.core != other.core,
            logging: self.logging != other.logging,
            runtime_assets: self.runtime_assets != other.runtime_assets,
            world_rendering: self.world_rendering != other.world_rendering,
        }
    }
}

#[derive(Default)]
struct SettingsChangeSet {
    app: bool,
    session_state: bool,
    graphics: bool,
    keybindings: bool,
    core: bool,
    logging: bool,
    runtime_assets: bool,
    world_rendering: bool,
}

impl SettingsChangeSet {
    fn any(&self) -> bool {
        self.app
            || self.session_state
            || self.graphics
            || self.keybindings
            || self.core
            || self.logging
            || self.runtime_assets
            || self.world_rendering
    }
}

pub struct SettingsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(SettingsPlugin);
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_asset::<Settings>()
            //.init_asset::<SettingsAsset>()
            //.init_asset_loader::<SettingsAssetLoader>() // Register custom loader
            .add_message::<ToggleWireframe>()
            .add_systems(PreStartup, sys_startup_load_file)
            .add_systems(Startup, sys_apply)
            .insert_resource(SettingsSaveTimer({
                let mut t = Timer::from_seconds(1.0, TimerMode::Once);
                t.pause();
                t
            }))
            .add_systems(FixedUpdate, sys_evlisten_switch_wireframe)
            // Export mode overrides the settings in memory and must never save them.
            .add_systems(
                FixedUpdate,
                sys_sync_resources_to_settings
                    .before(sys_debounced_save)
                    .run_if(not(resource_exists::<ExportJob>)),
            )
            .add_systems(FixedUpdate, sys_debounced_save.run_if(not(resource_exists::<ExportJob>)))
            .add_systems(Last, sys_flush_settings_on_exit.run_if(not(resource_exists::<ExportJob>)));
    }
}

/// Syncs runtime resources back to Settings so they can be persisted by sys_debounced_save.
fn sys_sync_resources_to_settings(
    zoom: Res<RenderZoom>,
    mut settings: ResMut<Settings>,
    windows: Query<&Window>,
    player_q: Query<&Player>,
) {
    sync_runtime_resources_to_settings(
        &mut settings,
        Some(zoom.0),
        first_window_size(&windows),
        first_player_position(&player_q),
    );
}

fn sys_flush_settings_on_exit(
    mut exits: MessageReader<AppExit>,
    zoom: Option<Res<RenderZoom>>,
    mut settings: ResMut<Settings>,
    windows: Query<&Window>,
    player_q: Query<&Player>,
) {
    if exits.read().next().is_none() {
        return;
    }

    sync_runtime_resources_to_settings(
        &mut settings,
        zoom.as_ref().map(|zoom| zoom.0),
        first_window_size(&windows),
        first_player_position(&player_q),
    );

    save_app_settings(&settings);
    save_session_state_settings(&settings);
    save_graphics_settings(&settings);
    save_keybindings(&settings);
    save_core_settings(&settings);
    save_runtime_assets_settings(&settings);
    save_world_rendering_settings(&settings);
}

fn first_window_size(windows: &Query<&Window>) -> Option<(f32, f32)> {
    windows
        .iter()
        .next()
        .map(|window| (window.resolution.width(), window.resolution.height()))
}

fn first_player_position(player_q: &Query<&Player>) -> Option<UOVec4> {
    player_q.iter().next().and_then(|player| player.current_pos)
}

fn sync_runtime_resources_to_settings(
    settings: &mut Settings,
    zoom: Option<f32>,
    window_size: Option<(f32, f32)>,
    player_pos: Option<UOVec4>,
) {
    if let Some(zoom) = zoom {
        if (settings.session_state.window.zoom - zoom).abs() > 0.001 {
            settings.session_state.window.zoom = zoom;
        }
    }

    if let Some((width, height)) = window_size {
        if (settings.session_state.window.width - width).abs() > 1.0 {
            settings.session_state.window.width = width;
        }
        if (settings.session_state.window.height - height).abs() > 1.0 {
            settings.session_state.window.height = height;
        }
    }

    if let Some(pos) = player_pos {
        if settings.session_state.world.last_p != pos {
            settings.session_state.world.last_p = pos;
        }
    }
}

fn sys_startup_load_file(mut commands: Commands, export_job: Option<Res<ExportJob>>) {
    let mut data = load_from_files().unwrap_or_else(|error| exit_on_startup_config_error(&error));
    if let Some(job) = export_job {
        crate::core::export_job::apply_settings_overrides(&mut data, &job);
    }

    // Initialize logger settings
    apply_logging_settings(&data.logging);
    // Ensure plugin log toggles follow configured settings
    set_plugin_log_toggles(
        data.logging.emit_flat_plugin_build,
        data.logging.emit_tree_plugin_build,
    );

    commands.insert_resource(data);
    console_logger::one(
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file for global access.",
    );
}

fn sys_apply(
    settings_res: Res<Settings>,
    mut windows_q: Query<&mut Window>,
    mut zoom_res: ResMut<RenderZoom>,
) {
    let mut w = windows_q.single_mut().unwrap();
    w.resolution = WindowResolution::new(
        settings_res.session_state.window.width as u32,
        settings_res.session_state.window.height as u32,
    );

    zoom_res.write_val(settings_res.session_state.window.zoom);
}

// ----

/*

// Wrappers
#[derive(Asset, TypePath, Debug, Clone)]
pub struct SettingsAsset(pub Settings);

#[derive(Resource, Clone)]
pub struct SettingsHandle(pub Handle<Settings>);

fn sys_settings_watcher_loader(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // Ttrack for changes and update it asynchronously.
    let handle: Handle<Settings> = asset_server.load(CONFIG_FILE_NAME);
    commands.insert_resource(SettingsHandle(handle));
}

#[derive(Default)]
pub struct SettingsAssetLoader;

impl AssetLoader for SettingsAssetLoader {
    type Asset = Settings;
    type Settings = ();
    type Error = anyhow::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let s = std::str::from_utf8(&bytes)?;
        let config = toml::from_str(s)?;
        Ok(config)
    }

    fn extensions(&self) -> &[&str] {
        &["toml"]
    }
}

fn sys_settings_reloaded(
    mut commands: Commands,
    mut events: EventReader<AssetEvent<Settings>>,
    handles: Res<SettingsHandle>,
    assets: Res<Assets<Settings>>,
) {
    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } if id == &handles.0.id() => {
                if let Some(settings) = assets.get(&handles.0) {
                    println!("Settings loaded (or hot reloaded): {settings:#?}");
                    commands.insert_resource(settings.clone());
                }
            }
            AssetEvent::Modified { id } if id == &handles.0.id() => {
                if let Some(settings) = assets.get(&handles.0) {
                    println!("Settings hot reloaded: {settings:#?}");
                    commands.insert_resource(settings.clone());
                }
            }
            _ => {}
        }
    }
}

*/

// ----

fn sys_evlisten_switch_wireframe(
    mut events: MessageReader<ToggleWireframe>,
    mut config: ResMut<WireframeConfig>,
    mut settings: ResMut<Settings>,
) {
    log_system_add_update::<SettingsPlugin>(fname!());
    for _ in events.read() {
        // This disables global wireframe for all meshes immediately
        config.global = !config.global;
        settings.app.debug.map_render_wireframe = config.global;
    }
}

/// Automatically saves user preferences if they have been modified, with a 1s debounce timer.
fn sys_debounced_save(
    time: Res<Time>,
    settings: Res<Settings>,
    mut save_timer: ResMut<SettingsSaveTimer>,
    mut last_saved: Local<Option<SettingsSaveSnapshot>>,
    mut pending_save: Local<Option<SettingsSaveSnapshot>>,
) {
    let current = SettingsSaveSnapshot::from_settings(&settings);

    if settings.is_added() {
        *last_saved = Some(current);
        pending_save.take();
        return;
    }

    if last_saved.is_none() {
        *last_saved = Some(current.clone());
    }

    let changes = current.changes_from(last_saved.as_ref().unwrap());

    if changes.any() {
        // Debounce: reset only when the pending settings snapshot changes.
        // If the snapshot stays stable, the timer is allowed to finish and persist it.
        if pending_save.as_ref() != Some(&current) {
            if changes.app {
                console_logger::one(LogSev::Debug, LogAbout::Settings, "App settings changed");
            }
            if changes.session_state {
                console_logger::one(
                    LogSev::DebugVerbose,
                    LogAbout::Settings,
                    "Session state changed",
                );
            }
            if changes.graphics {
                console_logger::one(
                    LogSev::Debug,
                    LogAbout::Settings,
                    "Graphics settings changed",
                );
            }
            if changes.keybindings {
                console_logger::one(LogSev::Debug, LogAbout::Settings, "Keybindings changed");
            }
            if changes.core {
                console_logger::one(LogSev::Debug, LogAbout::Settings, "Core settings changed");
            }
            if changes.world_rendering {
                console_logger::one(
                    LogSev::Debug,
                    LogAbout::Settings,
                    "World rendering settings changed",
                );
            }
            *pending_save = Some(current.clone());
            save_timer.0.reset();
            save_timer.0.unpause();
        }
    } else {
        pending_save.take();
        save_timer.0.pause();
    }

    if !save_timer.0.is_paused() {
        save_timer.0.tick(time.delta());
        if save_timer.0.just_finished() {
            let pending = pending_save.as_ref().unwrap_or(&current);
            let changes = pending.changes_from(last_saved.as_ref().unwrap());

            if changes.app {
                save_app_settings(&settings);
            }

            if changes.session_state {
                save_session_state_settings(&settings);
            }

            if changes.graphics {
                save_graphics_settings(&settings);
            }

            if changes.keybindings {
                save_keybindings(&settings);
            }

            if changes.core || changes.logging {
                save_core_settings(&settings);
            }

            if changes.runtime_assets {
                save_runtime_assets_settings(&settings);
            }

            if changes.world_rendering {
                save_world_rendering_settings(&settings);
            }

            *last_saved = Some(pending.clone());
            pending_save.take();
            save_timer.0.pause();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::utils::config_toml::{parse_app_state, parse_strict};

    const BUNDLED_GRAPHICS: &str = include_str!("../../assets/settings/graphics.toml");
    const BUNDLED_USER_PREFERENCES: &str = include_str!("../../assets/settings/user_preferences.toml");
    const BUNDLED_WORLD_RENDERING: &str = include_str!("../../assets/settings/world_rendering.toml");
    const BUNDLED_CORE: &str = include_str!("../../assets/settings/core.toml");

    fn parse_graphics_settings(contents: &str) -> SectGraphics {
        parse_strict::<GraphicsFile>(GRAPHICS_CONFIG_FILE, contents).unwrap_or_else(|error| panic!("{error}")).graphics
    }

    #[test]
    fn settings_files_load() {
        let settings = load_from_files().unwrap_or_else(|error| panic!("{error:#}"));

        assert!(!settings.logging.filters.is_empty());
        assert!(settings.app.window.ui_scale > 0.0);
    }

    #[test]
    fn only_settings_that_differ_from_the_bundled_file_get_a_cache_copy() {
        fn unchanged<T>(rel: &str)
        where
            T: Serialize + serde::de::DeserializeOwned + PartialEq,
        {
            let bundled = load_bundled_settings::<T>(rel).unwrap_or_else(|error| panic!("{error:#}"));
            assert!(cache_copy_contents(rel, &bundled).unwrap().is_none(), "{rel}");
        }
        unchanged::<CoreFile>(CORE_CONFIG_FILE);
        unchanged::<SectRuntimeAssets>(RUNTIME_ASSETS_CONFIG_FILE);
        unchanged::<SectKeybindings>(KEYBINDINGS_CONFIG_FILE);
        unchanged::<SectWorldRendering>(WORLD_RENDERING_CONFIG_FILE);

        const CHANGED_PATH: &str = "elsewhere/";
        let mut runtime: SectRuntimeAssets = load_bundled_settings(RUNTIME_ASSETS_CONFIG_FILE).unwrap();
        runtime.udd_path = CHANGED_PATH.to_string();
        let contents = cache_copy_contents(RUNTIME_ASSETS_CONFIG_FILE, &runtime).unwrap().expect("a cache copy");
        let saved: SectRuntimeAssets = toml::from_str(&contents).unwrap();
        assert_eq!(saved.udd_path, CHANGED_PATH);
    }

    #[test]
    fn bundled_settings_files_parse_strictly() {
        fn check<T: serde::de::DeserializeOwned>(rel: &str) {
            load_bundled_settings::<T>(rel).unwrap_or_else(|error| panic!("{error:#}"));
        }
        check::<CoreFile>(CORE_CONFIG_FILE);
        check::<SectRuntimeAssets>(RUNTIME_ASSETS_CONFIG_FILE);
        check::<SectApp>(USER_PREFERENCES_CONFIG_FILE);
        check::<SectSessionState>(SESSION_STATE_CONFIG_FILE);
        check::<GraphicsFile>(GRAPHICS_CONFIG_FILE);
        check::<SectKeybindings>(KEYBINDINGS_CONFIG_FILE);
        check::<SectWorldRendering>(WORLD_RENDERING_CONFIG_FILE);
    }

    #[test]
    fn bundled_graphics_settings_parse() {
        let settings = parse_graphics_settings(BUNDLED_GRAPHICS);

        assert!(!settings.cc_terrain_transitions);
        assert!(settings.ec_terrain_transitions);
        assert_eq!(settings.land_texture_source, LandTextureSource::Ec);
    }

    /// A saved Kingdom Reborn land source reads back. A land source this build does not know (as `kr` is to builds
    /// before it) takes the bundled value alone, with one warning naming the key and the value; the other saved
    /// graphics settings keep their values.
    #[test]
    fn a_saved_kingdom_reborn_land_source_reads_back_and_an_unknown_one_falls_back_alone() {
        let bundled = parse_strict::<GraphicsFile>(GRAPHICS_CONFIG_FILE, BUNDLED_GRAPHICS).expect("bundled graphics");
        let saved = BUNDLED_GRAPHICS.replace("land_texture_source = \"ec\"", "land_texture_source = \"kr\"");
        let load = parse_app_state(GRAPHICS_CONFIG_FILE, &saved, &bundled).expect("lenient load");
        assert_eq!(load.value.graphics.land_texture_source, LandTextureSource::Kr);
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);

        let newer = BUNDLED_GRAPHICS
            .replace("land_texture_source = \"ec\"", "land_texture_source = \"newer\"")
            .replace("art_texture_source = \"ec\"", "art_texture_source = \"cc\"")
            .replace("gumps_and_paperdoll = \"classic\"", "gumps_and_paperdoll = \"enhanced\"")
            .replace("vsync = false", "vsync = true");
        let load = parse_app_state(GRAPHICS_CONFIG_FILE, &newer, &bundled).expect("lenient load");
        let graphics = &load.value.graphics;
        assert_eq!(graphics.land_texture_source, bundled.graphics.land_texture_source);
        assert_eq!(graphics.art_texture_source, ClientTextureSource::Cc);
        assert_eq!(graphics.gumps_and_paperdoll, GumpClient::Enhanced);
        assert!(graphics.vsync);
        assert_eq!(load.warnings.len(), 1, "{:?}", load.warnings);
        let warning = &load.warnings[0];
        assert!(warning.contains("invalid value \"newer\" of key `graphics.land_texture_source`"), "{warning}");
    }

    /// The bundled settings draw the Classic gumps and paperdoll, as the default Classic preset drew them before the
    /// preference; a cache file from before the preference takes them, and Enhanced, once picked, is written and read
    /// back.
    #[test]
    fn gumps_and_paperdoll_are_filled_from_the_bundled_file_and_a_pick_round_trips() {
        let bundled = parse_graphics_settings(BUNDLED_GRAPHICS);
        assert_eq!(bundled.gumps_and_paperdoll, GumpClient::Classic);
        let bundled_file = GraphicsFile { graphics: bundled.clone() };
        let older = BUNDLED_GRAPHICS
            .lines()
            .filter(|line| !line.starts_with("gumps_and_paperdoll"))
            .collect::<Vec<_>>()
            .join("\n");
        let load = parse_app_state("graphics.toml", &older, &bundled_file).expect("lenient load");
        assert_eq!(load.value.graphics.gumps_and_paperdoll, GumpClient::Classic);
        assert_eq!(load.warnings, ["graphics.toml: missing key `graphics.gumps_and_paperdoll` filled from the default"]);

        let mut picked = bundled;
        picked.gumps_and_paperdoll = GumpClient::Enhanced;
        let written = toml::to_string_pretty(&GraphicsFile { graphics: picked }).expect("serialize");
        let load = parse_app_state("graphics.toml", &written, &bundled_file).expect("lenient load");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        assert_eq!(load.value.graphics.gumps_and_paperdoll, GumpClient::Enhanced);
    }

    /// A graphics.toml from before the "Gumps and paperdoll" preference keeps the gumps its saved preset drew: Enhanced
    /// under every preset but Classic's, with or without a cached graphics.toml. A file with the key, migrated or
    /// picked, is left as it is.
    #[test]
    fn a_graphics_file_from_before_the_gumps_preference_keeps_the_saved_presets_gumps() {
        let bundled = parse_strict::<GraphicsFile>(GRAPHICS_CONFIG_FILE, BUNDLED_GRAPHICS).expect("bundled graphics");
        let older = BUNDLED_GRAPHICS
            .lines()
            .filter(|line| !line.starts_with(GUMPS_AND_PAPERDOLL_KEY))
            .collect::<Vec<_>>()
            .join("\n");
        let cases = [
            ("classic.morning", GumpClient::Classic),
            ("enhanced.afternoon", GumpClient::Enhanced),
            ("kr.night", GumpClient::Enhanced),
            ("painterly.cave", GumpClient::Enhanced),
        ];
        for (saved_preset, expected) in cases {
            let (migrated, client) = migrate_cached_gumps_and_paperdoll(&older, saved_preset).expect("migrated");
            assert_eq!(client, expected, "{saved_preset}");
            let load = parse_app_state(GRAPHICS_CONFIG_FILE, &migrated, &bundled).expect("lenient load");
            assert!(load.warnings.is_empty(), "{saved_preset}: {:?}", load.warnings);
            assert_eq!(load.value.graphics.gumps_and_paperdoll, expected, "{saved_preset}");
            // The other saved values stay.
            assert_eq!(load.value.graphics.art_texture_source, bundled.graphics.art_texture_source);
            // Once the key is written, the migration does not run again.
            assert!(migrate_cached_gumps_and_paperdoll(&migrated, "classic.morning").is_none(), "{saved_preset}");
        }
        assert!(migrate_cached_gumps_and_paperdoll(BUNDLED_GRAPHICS, "kr.night").is_none());
        assert!(migrate_cached_gumps_and_paperdoll("not toml = ", "kr.night").is_none());

        // Without a graphics.toml, a saved preset other than Classic's keeps the Enhanced gumps; a Classic one and a
        // fresh install (no saved preset) keep the bundled Classic ones.
        assert_eq!(bundled.graphics.gumps_and_paperdoll, GumpClient::Classic);
        for (saved_preset, expected) in cases {
            let kept = gumps_and_paperdoll_without_a_cache_copy(Some(saved_preset));
            assert_eq!(kept, (expected != GumpClient::Classic).then_some(expected), "{saved_preset}");
        }
        assert!(gumps_and_paperdoll_without_a_cache_copy(None).is_none());
    }

    #[test]
    fn bundled_settings_reject_unknown_keys_with_their_position() {
        let graphics = BUNDLED_GRAPHICS.replace("vsync", "v_sync");
        let error = parse_strict::<GraphicsFile>(GRAPHICS_CONFIG_FILE, &graphics).err().expect("unknown key").to_string();
        assert!(error.starts_with("settings/graphics.toml: TOML parse error at line"), "{error}");
        assert!(error.contains("unknown field `v_sync`, expected one of `reduce_unfocused_fps`, `vsync`"), "{error}");

        let preferences = format!("{BUNDLED_USER_PREFERENCES}\n[debug2]\nmap_render_wireframe = false\n");
        let error = parse_strict::<SectApp>(USER_PREFERENCES_CONFIG_FILE, &preferences)
            .err().expect("unknown table")
            .to_string();
        assert!(error.contains("unknown field `debug2`, expected one of `input`, `window`, `debug`, `performance`"), "{error}");

        let core = BUNDLED_CORE.replace("hide_player", "hide_players");
        let error = parse_strict::<CoreFile>(CORE_CONFIG_FILE, &core).err().expect("unknown key").to_string();
        assert!(error.contains("unknown field `hide_players`, expected `start_p` or `hide_player`"), "{error}");
    }

    #[test]
    fn bundled_settings_reject_missing_keys() {
        let error = parse_strict::<SectWorldRendering>(
            WORLD_RENDERING_CONFIG_FILE,
            "enable_statics = true\nenable_static_lights = true\n",
        )
        .err().expect("missing keys")
        .to_string();
        assert!(error.contains("missing field `enable_static_art_local_lights`"), "{error}");

        let graphics = BUNDLED_GRAPHICS.lines().filter(|line| !line.starts_with("vsync")).collect::<Vec<_>>().join("\n");
        let error = parse_strict::<GraphicsFile>(GRAPHICS_CONFIG_FILE, &graphics).err().expect("missing key").to_string();
        assert!(error.contains("missing field `vsync`"), "{error}");
    }

    #[test]
    fn cached_user_settings_are_filled_from_the_bundled_file_and_rewritten_complete() {
        let bundled: SectApp = parse_strict(USER_PREFERENCES_CONFIG_FILE, BUNDLED_USER_PREFERENCES).expect("bundled");
        // A cache file from another build: one key missing, one key this build does not know.
        let cache = BUNDLED_USER_PREFERENCES
            .lines()
            .filter(|line| !line.starts_with("target_fps"))
            .chain(["retired_key = true"])
            .collect::<Vec<_>>()
            .join("\n")
            .replace("ui_scale = 1.15", "ui_scale = 1.5");
        let load = parse_app_state("user_preferences.toml", &cache, &bundled).expect("lenient load");

        assert_eq!(load.value.window.ui_scale, 1.5);
        assert_eq!(load.value.performance.target_fps, bundled.performance.target_fps);
        assert_eq!(
            load.warnings,
            [
                "user_preferences.toml: missing key `performance.target_fps` filled from the default",
                "user_preferences.toml: unknown key `performance.retired_key` dropped",
            ]
        );

        let rewritten = toml::to_string_pretty(&load.value).expect("serialize");
        let reread: SectApp = parse_strict(USER_PREFERENCES_CONFIG_FILE, &rewritten).expect("rewritten file is complete");
        assert!(reread == load.value);
    }

    /// A cache file from before the gump background takes the bundled one, the window's own; a picked colour is
    /// written and read back.
    #[test]
    fn the_gump_background_is_filled_from_the_bundled_file_and_a_picked_colour_round_trips() {
        let bundled: SectApp = parse_strict(USER_PREFERENCES_CONFIG_FILE, BUNDLED_USER_PREFERENCES).expect("bundled");
        assert_eq!(bundled.window.gump_background, ImageBackground::ViewerDefault);
        let older = BUNDLED_USER_PREFERENCES
            .lines()
            .filter(|line| !line.starts_with("gump_background"))
            .collect::<Vec<_>>()
            .join("\n");
        let load = parse_app_state("user_preferences.toml", &older, &bundled).expect("lenient load");
        assert_eq!(load.value.window.gump_background, ImageBackground::ViewerDefault);
        assert_eq!(load.warnings, ["user_preferences.toml: missing key `window.gump_background` filled from the default"]);

        let mut picked = bundled.clone();
        picked.window.gump_background =
            ImageBackground::Color(crate::core::render::dialogs::image_background::MAGENTA_BACKGROUND);
        let written = toml::to_string_pretty(&picked).expect("serialize");
        let load = parse_app_state("user_preferences.toml", &written, &bundled).expect("lenient load");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        assert_eq!(load.value.window.gump_background, picked.window.gump_background);
    }

    /// The bundled settings draw the 3D paperdoll with Enhanced gumps and paperdoll; a cache file from before the
    /// setting takes it, and the gump paperdoll, once picked, is written and read back.
    #[test]
    fn the_enhanced_paperdoll_is_filled_from_the_bundled_file_and_a_pick_round_trips() {
        let bundled: SectApp = parse_strict(USER_PREFERENCES_CONFIG_FILE, BUNDLED_USER_PREFERENCES).expect("bundled");
        assert_eq!(bundled.window.enhanced_paperdoll, EnhancedPaperdoll::Frames);
        let older = BUNDLED_USER_PREFERENCES
            .lines()
            .filter(|line| !line.starts_with("enhanced_paperdoll"))
            .collect::<Vec<_>>()
            .join("\n");
        let load = parse_app_state("user_preferences.toml", &older, &bundled).expect("lenient load");
        assert_eq!(load.value.window.enhanced_paperdoll, EnhancedPaperdoll::Frames);
        assert_eq!(
            load.warnings,
            ["user_preferences.toml: missing key `window.enhanced_paperdoll` filled from the default"]
        );

        let mut picked = bundled.clone();
        picked.window.enhanced_paperdoll = EnhancedPaperdoll::Gumps;
        let written = toml::to_string_pretty(&picked).expect("serialize");
        let load = parse_app_state("user_preferences.toml", &written, &bundled).expect("lenient load");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        assert_eq!(load.value.window.enhanced_paperdoll, EnhancedPaperdoll::Gumps);
    }

    #[test]
    fn terrain_transition_controls_round_trip_independently_of_source_and_presets() {
        for (cc, ec, flags) in [(false, true, 0), (true, true, 1), (false, false, 2), (true, false, 3)] {
            let mut settings = parse_graphics_settings(BUNDLED_GRAPHICS);
            settings.cc_terrain_transitions = cc;
            settings.ec_terrain_transitions = ec;
            let serialized = format!("[graphics]\n{}", toml::to_string(&settings).unwrap());
            let loaded = parse_graphics_settings(&serialized);
            assert_eq!((loaded.cc_terrain_transitions, loaded.ec_terrain_transitions), (cc, ec));
            assert_eq!(loaded.terrain_transition_flags(), flags);
        }
    }

    #[test]
    fn runtime_assets_settings_require_explicit_gump_loading_opt_in() {
        let settings: SectRuntimeAssets =
            toml::from_str("udd_path = \"/tmp/uddp\"\nload_gumps = false\n")
                .expect("parse runtime asset settings");

        assert_eq!(settings.udd_path, "/tmp/uddp");
        assert!(!settings.load_gumps);

        let error = toml::from_str::<SectRuntimeAssets>("udd_path = \"/tmp/uddp\"\n").err().expect("missing key");
        assert!(error.to_string().contains("missing field `load_gumps`"), "{error}");
    }

    #[test]
    fn world_rendering_settings_default_static_art_local_lights_off() {
        let settings: SectWorldRendering =
            parse_strict(WORLD_RENDERING_CONFIG_FILE, BUNDLED_WORLD_RENDERING).expect("parse world rendering settings");

        assert!(settings.enable_statics);
        assert!(settings.enable_static_lights);
        assert!(!settings.enable_static_art_local_lights);
        assert!(!settings.show_hidden_multi_parts);
        assert!(!settings.show_only_player_floor);
    }

    /// A cache copy written before the player-floor view existed loads with it off, and a saved opt-in round-trips.
    #[test]
    fn world_rendering_cache_without_player_floor_view_loads_it_off() {
        let bundled: SectWorldRendering =
            parse_strict(WORLD_RENDERING_CONFIG_FILE, BUNDLED_WORLD_RENDERING).expect("bundled");
        let older = BUNDLED_WORLD_RENDERING
            .lines()
            .filter(|line| !line.starts_with("show_only_player_floor"))
            .collect::<Vec<_>>()
            .join("\n");
        let load = parse_app_state(WORLD_RENDERING_CONFIG_FILE, &older, &bundled).expect("lenient load");
        assert!(!load.value.show_only_player_floor);

        let mut picked = bundled.clone();
        picked.show_only_player_floor = true;
        let written = toml::to_string_pretty(&picked).expect("serialize");
        let load = parse_app_state(WORLD_RENDERING_CONFIG_FILE, &written, &bundled).expect("lenient load");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        assert!(load.value.show_only_player_floor);
    }

    #[test]
    fn world_rendering_settings_accept_static_art_local_lights_opt_in() {
        let text = BUNDLED_WORLD_RENDERING
            .replace("enable_static_art_local_lights = false", "enable_static_art_local_lights = true");
        let settings: SectWorldRendering =
            parse_strict(WORLD_RENDERING_CONFIG_FILE, &text).expect("parse world rendering settings");

        assert!(settings.enable_static_art_local_lights);
    }
}
