use crate::console_logger::{self, LogAbout, LogSev};
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
pub struct SectKeybindings {
    pub shader_settings: KeyCode,
    pub user_settings: KeyCode,
    pub keybindings_help: KeyCode,
    pub increase_z: KeyCode,
    pub decrease_z: KeyCode,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectCore {
    pub world: SectWorld,
}

// User-settings section: `#[serde(default)]` lets a cache file written by an older
// build (missing newly-added keys) load with defaults instead of panicking. Applied
// only to user settings + session state, not to bundled/system config.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct SectApp {
    pub input: SectInput,
    pub window: SectWindow,
    pub debug: SectDebug,
    pub performance: SectPerformance,
}

// Session state: relocated to the cache dir and made tolerant to missing keys.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct SectSessionState {
    pub world: SectSessionWorld,
    pub window: SectSessionWindow,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SectSessionWorld {
    pub last_p: UOVec4,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SectSessionWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectRuntimeAssets {
    pub udd_path: String,
    pub load_gumps: bool,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
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
pub struct SectWorld {
    pub start_p: UOVec4,
    pub hide_player: bool,
}

#[derive(Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct SectWorldRendering {
    #[serde(default = "default_enable_statics")]
    pub enable_statics: bool,
    #[serde(default = "default_enable_static_lights")]
    pub enable_static_lights: bool,
    #[serde(default = "default_enable_static_art_local_lights")]
    pub enable_static_art_local_lights: bool,
    /// Also draws the multi parts the client never draws (classic parts without the visible
    /// flag, EC `not_drawn` parts).
    #[serde(default)]
    pub show_hidden_multi_parts: bool,
    #[serde(default)]
    pub land_streaming: SectLandStreaming,
    #[serde(default)]
    pub diagnostics: SectWorldRenderingDiagnostics,
    #[serde(default)]
    pub shader_simplification: SectWorldRenderingShaderSimplification,
}

fn default_enable_statics() -> bool {
    false
}

fn default_enable_static_lights() -> bool {
    false
}

fn default_enable_static_art_local_lights() -> bool {
    false
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct SectWorldRenderingShaderSimplification {
    /// Forces a temporary minimal land shader configuration on the shared land material.
    #[serde(default)]
    pub force_minimal_shader: bool,
    /// Forces the land fragment shader to return a flat debug color immediately.
    #[serde(default)]
    pub force_flat_fragment_shader: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectWorldRenderingDiagnostics {
    /// Enables Bevy's low-level render diagnostics plugin.
    ///
    /// This gathers GPU pass timings and pipeline statistics. It is useful when profiling, but
    /// on some drivers it can perturb frame time noticeably. Restart required after changing it.
    #[serde(default = "default_worldmap_diagnostics_enable_render_diagnostics")]
    pub enable_render_diagnostics: bool,
    /// Enables compact periodic worldmap diagnostics in the console.
    #[serde(default)]
    pub dump_to_console: bool,
    /// Period between compact diagnostics dumps.
    #[serde(default = "default_worldmap_diagnostics_dump_interval_sec")]
    pub dump_interval_sec: f32,
    /// Include a compact render-pass GPU breakdown in the dump.
    #[serde(default = "default_worldmap_diagnostics_log_render_breakdown")]
    pub log_render_breakdown: bool,
    /// Include compact world/chunk state in the dump.
    #[serde(default = "default_worldmap_diagnostics_log_world_state")]
    pub log_world_state: bool,
    /// Include compact CPU-side system timing in the dump, plus auxiliary
    /// upload/extract helpers and a residual frame-gap estimate.
    #[serde(default = "default_worldmap_diagnostics_log_system_timers")]
    pub log_system_timers: bool,
    /// Draw a debug outline around the currently hovered static-world object.
    #[serde(default)]
    pub highlight_hovered_static_object: bool,
}

impl Default for SectWorldRenderingDiagnostics {
    fn default() -> Self {
        Self {
            enable_render_diagnostics: default_worldmap_diagnostics_enable_render_diagnostics(),
            dump_to_console: false,
            dump_interval_sec: default_worldmap_diagnostics_dump_interval_sec(),
            log_render_breakdown: default_worldmap_diagnostics_log_render_breakdown(),
            log_world_state: default_worldmap_diagnostics_log_world_state(),
            log_system_timers: default_worldmap_diagnostics_log_system_timers(),
            highlight_hovered_static_object: false,
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectLandStreaming {
    /// Main-world land preparation budget measured in equivalent 8x8 map blocks.
    ///
    /// This limits how many ready chunks we are allowed to expand, precache and enqueue
    /// into the atlas in a single frame. Set to 0 to disable the cap entirely.
    #[serde(default = "default_land_prepare_max_blocks_per_frame")]
    pub prepare_max_blocks_per_frame: usize,
    /// Maximum number of metadata-atlas `write_texture` uploads that may be submitted
    /// during a single render-world queue pass. Set to 0 to disable the cap.
    #[serde(default = "default_land_upload_max_ops_per_frame")]
    pub upload_max_ops_per_frame: usize,
    /// Maximum number of metadata-atlas bytes that may be uploaded in a single render
    /// frame. Set to 0 to disable the byte cap.
    #[serde(default = "default_land_upload_max_bytes_per_frame")]
    pub upload_max_bytes_per_frame: usize,
}

impl Default for SectLandStreaming {
    fn default() -> Self {
        Self {
            prepare_max_blocks_per_frame: default_land_prepare_max_blocks_per_frame(),
            upload_max_ops_per_frame: default_land_upload_max_ops_per_frame(),
            upload_max_bytes_per_frame: default_land_upload_max_bytes_per_frame(),
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SectDebug {
    pub map_render_wireframe: bool,
}

fn default_emit_true() -> bool {
    true
}

fn default_land_prepare_max_blocks_per_frame() -> usize {
    32_768
}

fn default_worldmap_diagnostics_dump_interval_sec() -> f32 {
    2.0
}

fn default_worldmap_diagnostics_enable_render_diagnostics() -> bool {
    true
}

fn default_worldmap_diagnostics_log_render_breakdown() -> bool {
    true
}

fn default_worldmap_diagnostics_log_world_state() -> bool {
    true
}

fn default_worldmap_diagnostics_log_system_timers() -> bool {
    true
}

fn default_land_upload_max_ops_per_frame() -> usize {
    8
}

fn default_land_upload_max_bytes_per_frame() -> usize {
    16 * 1024 * 1024
}

// Derived `Default` exists only to satisfy the `#[serde(default)]` on `SectApp`; in
// practice the `[performance]` table is always present (bundled default has it), so
// these zero/false defaults are never the effective values. Per-field `serde(default)`
// still governs individually-missing keys.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SectPerformance {
    /// Master toggle for all on-screen overlays. When off, every overlay is hidden
    /// regardless of the per-overlay flags below.
    pub show_overlay: bool,
    /// Per-overlay toggles, gated behind `show_overlay`. Defaulted to `true` so configs
    /// written before these existed keep showing every overlay (prior behavior).
    #[serde(default = "default_emit_true")]
    pub show_perf_stats_overlay: bool,
    #[serde(default = "default_emit_true")]
    pub show_player_position_overlay: bool,
    #[serde(default = "default_emit_true")]
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
#[serde(default)]
pub struct SectGraphics {
    pub reduce_unfocused_fps: bool,
    pub vsync: bool, // Added vsync control
    pub anti_aliasing: AntiAliasingMode,
    #[serde(default = "default_client_texture_source")]
    pub art_texture_source: ClientTextureSource,
    #[serde(default = "default_client_texture_source")]
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

fn default_client_texture_source() -> ClientTextureSource {
    ClientTextureSource::default()
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
pub struct SectLogging {
    pub min_severity: LogSev,
    #[serde(default = "default_emit_true")]
    pub emit_flat_plugin_build: bool,
    #[serde(default = "default_emit_true")]
    pub emit_tree_plugin_build: bool,
    pub filters: Vec<LogFilterSetting>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
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

/// Read a user-side settings file, preferring the cache copy (where the app writes
/// runtime changes) and falling back to the bundled default under `assets/`. Used
/// for user settings, session state, and user-saved shader settings; system/bundled
/// config reads `assets/` directly.
pub(crate) fn read_user_settings_file(rel: &str) -> std::io::Result<String> {
    let cache_path = crate::core::constants::valid_cache_dir().join(rel);
    match std::fs::read_to_string(&cache_path) {
        Ok(contents) => Ok(contents),
        Err(_) => std::fs::read_to_string(crate::core::constants::valid_asset_dir().join(rel)),
    }
}

pub fn load_from_files() -> Settings {
    let assets_path = crate::core::constants::valid_asset_dir();

    let core_path = assets_path.join(CORE_CONFIG_FILE);
    let runtime_assets_path = assets_path.join(RUNTIME_ASSETS_CONFIG_FILE);
    let world_rendering_path = assets_path.join(WORLD_RENDERING_CONFIG_FILE);

    let core_contents =
        std::fs::read_to_string(&core_path).expect("Failed to read settings/core.toml");

    // Core settings file contains top-level core and logging sections.
    #[derive(Deserialize)]
    struct CoreWrapper {
        core: SectCore,
        logging: SectLogging,
    }
    let core_data: CoreWrapper = toml::from_str(&core_contents).expect(
        "Failed to parse settings/core.toml — please fix the file in assets/settings/core.toml",
    );

    // Runtime asset package settings (assets/settings/runtime_assets.toml)
    let runtime_assets_contents = std::fs::read_to_string(&runtime_assets_path)
        .expect("Failed to read settings/runtime_assets.toml — please ensure assets/settings/runtime_assets.toml exists");
    let runtime_assets: SectRuntimeAssets = toml::from_str(&runtime_assets_contents)
        .expect("Failed to parse settings/runtime_assets.toml — please fix the file in assets/settings/runtime_assets.toml");

    // User preferences file contains SectApp fields directly at top level
    // (loaded from the cache dir if present, else the bundled default).
    let user_contents = read_user_settings_file(USER_PREFERENCES_CONFIG_FILE)
        .expect("Failed to read settings/user_preferences.toml — please ensure assets/settings/user_preferences.toml exists and is valid");

    let user_app: SectApp = toml::from_str(&user_contents)
        .expect("Failed to parse settings/user_preferences.toml — please fix the file in assets/settings/user_preferences.toml");

    let session_state_contents = read_user_settings_file(SESSION_STATE_CONFIG_FILE)
        .expect("Failed to read settings/session_state.toml — please ensure assets/settings/session_state.toml exists and is valid");

    let session_state: SectSessionState = toml::from_str(&session_state_contents)
        .expect("Failed to parse settings/session_state.toml — please fix the file in assets/settings/session_state.toml");

    // Graphics settings loader (cache dir if present, else the bundled default).
    let gfx_contents = read_user_settings_file(GRAPHICS_CONFIG_FILE).expect(
        "Failed to read settings/graphics.toml — please ensure assets/settings/graphics.toml exists",
    );
    let graphics = parse_graphics_settings(&gfx_contents);

    // Keybindings file
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);
    let kb_contents = std::fs::read_to_string(&kb_path)
        .expect("Failed to read keybindings.toml — please ensure assets/keybindings.toml exists and is valid");

    let keybindings: SectKeybindings = toml::from_str(&kb_contents).expect(
        "Failed to parse keybindings.toml — please fix the file in assets/keybindings.toml",
    );

    let world_rendering_contents = std::fs::read_to_string(&world_rendering_path).expect(
        "Failed to read settings/world_rendering.toml — please ensure assets/settings/world_rendering.toml exists",
    );

    let world_rendering: SectWorldRendering = toml::from_str(&world_rendering_contents)
        .expect(
            "Failed to parse settings/world_rendering.toml — please fix the file in assets/settings/world_rendering.toml",
        );

    Settings {
        core: core_data.core,
        graphics,
        runtime_assets,
        app: user_app,
        session_state,
        logging: core_data.logging,
        keybindings,
        world_rendering,
    }
}

fn parse_graphics_settings(contents: &str) -> SectGraphics {
    // Graphics settings live in a `[graphics]` table, as `save_graphics_settings` writes them.
    #[derive(Deserialize)]
    struct GraphicsWrapper {
        graphics: SectGraphics,
    }
    let wrapper: GraphicsWrapper = toml::from_str(contents).expect(
        "Failed to parse settings/graphics.toml — please fix the file in assets/settings/graphics.toml",
    );
    wrapper.graphics
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

    #[derive(Serialize)]
    struct GraphicsWrapper<'a> {
        graphics: &'a SectGraphics,
    }

    match toml::to_string_pretty(&GraphicsWrapper {
        graphics: &settings.graphics,
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
            .add_systems(
                FixedUpdate,
                sys_sync_resources_to_settings.before(sys_debounced_save),
            )
            .add_systems(FixedUpdate, sys_debounced_save)
            .add_systems(Last, sys_flush_settings_on_exit);
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

fn sys_startup_load_file(mut commands: Commands) {
    let data = load_from_files();

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

    const BUNDLED_GRAPHICS: &str = include_str!("../../assets/settings/graphics.toml");

    #[test]
    fn settings_files_load() {
        let settings = load_from_files();

        assert!(!settings.logging.filters.is_empty());
        assert!(settings.app.window.ui_scale > 0.0);
    }

    #[test]
    fn bundled_graphics_settings_parse() {
        let settings = parse_graphics_settings(BUNDLED_GRAPHICS);

        assert!(!settings.cc_terrain_transitions);
        assert!(settings.ec_terrain_transitions);
        assert_eq!(settings.land_texture_source, ClientTextureSource::Ec);
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
    }

    #[test]
    fn world_rendering_settings_default_static_art_local_lights_off() {
        let settings: SectWorldRendering =
            toml::from_str("enable_statics = true\nenable_static_lights = true\n")
                .expect("parse world rendering settings");

        assert!(settings.enable_statics);
        assert!(settings.enable_static_lights);
        assert!(!settings.enable_static_art_local_lights);
        assert!(!settings.show_hidden_multi_parts);
    }

    #[test]
    fn world_rendering_settings_accept_static_art_local_lights_opt_in() {
        let settings: SectWorldRendering = toml::from_str(
            "enable_statics = true\nenable_static_lights = true\nenable_static_art_local_lights = true\n",
        )
        .expect("parse world rendering settings");

        assert!(settings.enable_static_art_local_lights);
    }
}
