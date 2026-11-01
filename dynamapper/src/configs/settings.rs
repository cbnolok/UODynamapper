use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::export_job::ExportJob;
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
    pub land_texture_source: ClientTextureSource,
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

    pub const fn land_label(self) -> &'static str {
        match self {
            Self::Cc => "Classic (tex_land_cc.uddp)",
            Self::Ec => "Enhanced (tex_land_ec.uddp)",
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
/// dropped, each with a warning; the app rewrites it complete on its next save. A cache copy that cannot be read
/// at all is replaced by the bundled values, with a warning.
pub(crate) fn load_user_settings<T>(rel: &str) -> eyre::Result<T>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let bundled: T = load_bundled_settings(rel)?;
    let cache_path = crate::core::constants::valid_cache_dir().join(rel);
    let Ok(contents) = std::fs::read_to_string(&cache_path) else {
        return Ok(bundled);
    };
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
#[derive(Deserialize)]
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

pub fn load_from_files() -> eyre::Result<Settings> {
    let core_data: CoreFile = load_bundled_settings(CORE_CONFIG_FILE)?;
    // Runtime asset package settings (assets/settings/runtime_assets.toml)
    let runtime_assets: SectRuntimeAssets = load_bundled_settings(RUNTIME_ASSETS_CONFIG_FILE)?;
    // User preferences file contains SectApp fields directly at top level
    // (loaded from the cache dir if present, over the bundled default).
    let user_app: SectApp = load_user_settings(USER_PREFERENCES_CONFIG_FILE)?;
    let session_state: SectSessionState = load_user_settings(SESSION_STATE_CONFIG_FILE)?;
    // Graphics settings loader (cache dir if present, over the bundled default).
    let graphics = load_user_settings::<GraphicsFile>(GRAPHICS_CONFIG_FILE)?.graphics;
    let keybindings: SectKeybindings = load_bundled_settings(KEYBINDINGS_CONFIG_FILE)?;
    let world_rendering: SectWorldRendering = load_bundled_settings(WORLD_RENDERING_CONFIG_FILE)?;

    Ok(Settings {
        core: core_data.core,
        graphics,
        runtime_assets,
        app: user_app,
        session_state,
        logging: core_data.logging,
        keybindings,
        world_rendering,
    })
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

pub fn save_keybindings(settings: &Settings) {
    let assets_path = crate::core::constants::valid_asset_dir();
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);

    match toml::to_string_pretty(&settings.keybindings) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&kb_path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save keybindings.toml: {}", e),
                );
            } else {
                console_logger::one(LogSev::Info, LogAbout::Settings, "Saved keybindings.toml");
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize keybindings: {}", e),
            );
        }
    }
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
    let assets_path = crate::core::constants::valid_asset_dir();
    let core_path = assets_path.join(CORE_CONFIG_FILE);

    #[derive(Serialize)]
    struct CoreWrapper<'a> {
        core: &'a SectCore,
        logging: &'a SectLogging,
    }

    match toml::to_string_pretty(&CoreWrapper {
        core: &settings.core,
        logging: &settings.logging,
    }) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&core_path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save core.toml: {}", e),
                );
            } else {
                console_logger::one(LogSev::Info, LogAbout::Settings, "Saved core.toml");
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize core settings: {}", e),
            );
        }
    }
}

pub fn save_world_rendering_settings(settings: &Settings) {
    let assets_path = crate::core::constants::valid_asset_dir();
    let path = assets_path.join(WORLD_RENDERING_CONFIG_FILE);

    match toml::to_string_pretty(&settings.world_rendering) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save world_rendering.toml: {}", e),
                );
            } else {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Settings,
                    "Saved world_rendering.toml",
                );
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize world rendering settings: {}", e),
            );
        }
    }
}

pub fn save_runtime_assets_settings(settings: &Settings) {
    let assets_path = crate::core::constants::valid_asset_dir();
    let path = assets_path.join(RUNTIME_ASSETS_CONFIG_FILE);

    match toml::to_string_pretty(&settings.runtime_assets) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&path, toml_str) {
                console_logger::one(
                    LogSev::Error,
                    LogAbout::Settings,
                    &format!("Failed to save runtime_assets.toml: {}", e),
                );
            } else {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Settings,
                    "Saved runtime_assets.toml",
                );
            }
        }
        Err(e) => {
            console_logger::one(
                LogSev::Error,
                LogAbout::Settings,
                &format!("Failed to serialize runtime asset settings: {}", e),
            );
        }
    }
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
        assert_eq!(settings.land_texture_source, ClientTextureSource::Ec);
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
