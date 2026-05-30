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

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectApp {
    pub input: SectInput,
    pub window: SectWindow,
    pub debug: SectDebug,
    pub performance: SectPerformance,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectSessionState {
    pub world: SectSessionWorld,
    pub window: SectSessionWindow,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectSessionWorld {
    pub last_p: UOVec4,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectSessionWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectRuntimeAssets {
    pub udd_path: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
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

#[derive(Clone, Deserialize, Serialize, PartialEq)]
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

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectPerformance {
    pub show_overlay: bool,
    pub frame_limit_enabled: bool,
    pub target_fps: u32,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectGraphics {
    pub reduce_unfocused_fps: bool,
    pub vsync: bool, // Added vsync control
    pub anti_aliasing: AntiAliasingMode,
    #[serde(default = "default_client_texture_source")]
    pub art_texture_source: ClientTextureSource,
    #[serde(default = "default_client_texture_source")]
    pub land_texture_source: ClientTextureSource,
    pub sharpening_strength: f32, // 0.0 to 1.0
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

pub fn load_from_files() -> Settings {
    let assets_path = crate::core::constants::valid_asset_dir();

    let core_path = assets_path.join(CORE_CONFIG_FILE);
    let runtime_assets_path = assets_path.join(RUNTIME_ASSETS_CONFIG_FILE);
    let user_path = assets_path.join(USER_PREFERENCES_CONFIG_FILE);
    let session_state_path = assets_path.join(SESSION_STATE_CONFIG_FILE);
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
    let user_contents = std::fs::read_to_string(&user_path)
        .expect("Failed to read settings/user_preferences.toml — please ensure assets/settings/user_preferences.toml exists and is valid");

    let user_app: SectApp = toml::from_str(&user_contents)
        .expect("Failed to parse settings/user_preferences.toml — please fix the file in assets/settings/user_preferences.toml");

    let session_state_contents = std::fs::read_to_string(&session_state_path)
        .expect("Failed to read settings/session_state.toml — please ensure assets/settings/session_state.toml exists and is valid");

    let session_state: SectSessionState = toml::from_str(&session_state_contents)
        .expect("Failed to parse settings/session_state.toml — please fix the file in assets/settings/session_state.toml");

    // Graphics settings loader (assets/settings/graphics.toml)
    let gfx_path = assets_path.join(GRAPHICS_CONFIG_FILE);
    let gfx_contents = std::fs::read_to_string(&gfx_path).expect(
        "Failed to read settings/graphics.toml — please ensure assets/settings/graphics.toml exists",
    );

    // Try both [graphics] and [core.graphics] (legacy)
    let graphics: SectGraphics = if let Ok(val) = toml::from_str::<toml::Value>(&gfx_contents) {
        if let Some(g) = val.get("graphics") {
            g.clone()
                .try_into::<SectGraphics>()
                .expect("Failed to parse [graphics] in settings/graphics.toml")
        } else if let Some(c) = val.get("core").and_then(|c| c.get("graphics")) {
            c.clone()
                .try_into::<SectGraphics>()
                .expect("Failed to parse [core.graphics] in settings/graphics.toml")
        } else {
            // If neither table exists, try to parse the whole file as SectGraphics if it's flat (unlikely but possible)
            toml::from_str(&gfx_contents).expect("Failed to parse settings/graphics.toml — expected it to contain a [graphics] table")
        }
    } else {
        panic!("Failed to parse settings/graphics.toml as TOML");
    };

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
    let assets_path = crate::core::constants::valid_asset_dir();
    let user_path = assets_path.join(USER_PREFERENCES_CONFIG_FILE);

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
    let assets_path = crate::core::constants::valid_asset_dir();
    let path = assets_path.join(SESSION_STATE_CONFIG_FILE);

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
    let assets_path = crate::core::constants::valid_asset_dir();
    let graphics_path = assets_path.join(GRAPHICS_CONFIG_FILE);

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
            .add_systems(
                FixedUpdate,
                (
                    sys_evlisten_switch_wireframe,
                    sys_debounced_save,
                    sys_sync_resources_to_settings,
                ),
            );
    }
}

/// Syncs runtime resources back to Settings so they can be persisted by sys_debounced_save.
fn sys_sync_resources_to_settings(
    zoom: Res<RenderZoom>,
    mut settings: ResMut<Settings>,
    windows: Query<&Window>,
    player_q: Query<&Player>,
) {
    // Zoom
    if (settings.session_state.window.zoom - zoom.0).abs() > 0.001 {
        settings.session_state.window.zoom = zoom.0;
    }

    // Window size
    if let Some(window) = windows.iter().next() {
        let res = &window.resolution;
        if (settings.session_state.window.width - res.width()).abs() > 1.0 {
            settings.session_state.window.width = res.width();
        }
        if (settings.session_state.window.height - res.height()).abs() > 1.0 {
            settings.session_state.window.height = res.height();
        }
    }

    // Player position
    if let Some(player) = player_q.iter().next() {
        if let Some(pos) = player.current_pos {
            if settings.session_state.world.last_p != pos {
                settings.session_state.world.last_p = pos;
            }
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
