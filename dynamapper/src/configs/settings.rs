use std::time::SystemTime;

use crate::console_logger::{self, LogAbout, LogSev};
use crate::core::render::scene::camera::RenderZoom;
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
use uddconv::bc7::{is_bc7_encoder_backend_available, Bc7EncoderBackend};

#[derive(Asset, Clone, Deserialize, Serialize, Resource, TypePath)]
pub struct Settings {
    pub core: SectCore,
    pub graphics: SectGraphics,
    pub uo_files: SectUoFiles,
    pub app: SectApp,
    pub logging: SectLogging,
    pub keybindings: SectKeybindings,
    pub maps: SectMaps,
    pub worldmap_rendering: SectWorldMapRendering,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectKeybindings {
    pub shader_settings: KeyCode,
    pub user_settings: KeyCode,
    pub keybindings_help: KeyCode,
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

#[derive(Clone, Deserialize, Serialize)]
pub struct SectUoFiles {
    pub folder: String,
    #[serde(default)]
    pub udd_path: Option<String>,
    pub texmaps_preload_full_file: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
    pub smooth_movement: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
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

#[derive(Clone, Deserialize, Serialize)]
pub struct SectMaps {
    pub maps: Vec<SectMapSize>,
}

#[derive(Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct SectWorldMapRendering {
    #[serde(default = "default_enable_statics")]
    pub enable_statics: bool,
    #[serde(default)]
    pub land_streaming: SectLandStreaming,
    #[serde(default)]
    pub diagnostics: SectWorldMapDiagnostics,
    #[serde(default)]
    pub shader_simplification: SectWorldMapShaderSimplification,
}

fn default_enable_statics() -> bool {
    false
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct SectWorldMapShaderSimplification {
    /// Forces a temporary minimal terrain shader configuration on the shared land material.
    #[serde(default)]
    pub force_minimal_shader: bool,
    /// Forces the terrain fragment shader to return a flat debug color immediately.
    #[serde(default)]
    pub force_flat_fragment_shader: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectWorldMapDiagnostics {
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
}

impl Default for SectWorldMapDiagnostics {
    fn default() -> Self {
        Self {
            enable_render_diagnostics: default_worldmap_diagnostics_enable_render_diagnostics(),
            dump_to_console: false,
            dump_interval_sec: default_worldmap_diagnostics_dump_interval_sec(),
            log_render_breakdown: default_worldmap_diagnostics_log_render_breakdown(),
            log_world_state: default_worldmap_diagnostics_log_world_state(),
            log_system_timers: default_worldmap_diagnostics_log_system_timers(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectLandStreaming {
    /// Main-world terrain preparation budget measured in equivalent 8x8 map blocks.
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

impl SectMaps {
    pub fn map_size(&self, map_id: u32) -> Option<&SectMapSize> {
        self.maps.iter().find(|map| map.id == map_id)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SectMapSize {
    pub id: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct SectDebug {
    pub map_render_wireframe: bool,
    /// Whether settings hot-reload from disk is enabled. Disabled by default until
    /// all systems correctly respond to runtime changes.
    pub hot_reload_enabled: bool,
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
    pub lossy_texture_compression: bool,
    pub lossy_texture_compression_backend: LossyTextureCompressionBackend,
    pub reduce_unfocused_fps: bool,
    pub vsync: bool, // Added vsync control
    pub anti_aliasing: AntiAliasingMode,
    #[serde(default = "default_client_texture_source")]
    pub art_texture_source: ClientTextureSource,
    #[serde(default = "default_client_texture_source")]
    pub land_texture_source: ClientTextureSource,
    pub texture_filtering: u32,      // 0: Point, 1: Linear
    pub texture_reconstruction: u32, // 0: None, 1: Bicubic, 2: FSR
    pub sharpening_strength: f32,    // 0.0 to 1.0
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
            Self::Cc => "Classic (cc_art.uddp)",
            Self::Ec => "Enhanced (ec_art.uddp)",
        }
    }

    pub const fn land_label(self) -> &'static str {
        match self {
            Self::Cc => "Classic (texmaps.mul)",
            Self::Ec => "Enhanced (ec_land.uddp)",
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LossyTextureCompressionBackend {
    #[default]
    Dds,
    BlockCompression,
    Ispc,
}

impl LossyTextureCompressionBackend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dds => "dds",
            Self::BlockCompression => "block_compression",
            Self::Ispc => "ispc",
        }
    }
}

impl SectGraphics {
    pub fn active_lossy_texture_compression_backend(
        &self,
    ) -> Option<LossyTextureCompressionBackend> {
        if !self.lossy_texture_compression {
            return None;
        }

        match self.lossy_texture_compression_backend {
            LossyTextureCompressionBackend::Dds => Some(LossyTextureCompressionBackend::Dds),
            LossyTextureCompressionBackend::BlockCompression => {
                if is_bc7_encoder_backend_available(Bc7EncoderBackend::BlockCompression) {
                    Some(LossyTextureCompressionBackend::BlockCompression)
                } else {
                    Some(LossyTextureCompressionBackend::Dds)
                }
            }
            LossyTextureCompressionBackend::Ispc => {
                if is_bc7_encoder_backend_available(Bc7EncoderBackend::Ispc) {
                    Some(LossyTextureCompressionBackend::Ispc)
                } else {
                    Some(LossyTextureCompressionBackend::Dds)
                }
            }
        }
    }

    pub fn texture_compression_log_status(&self) -> String {
        match self.active_lossy_texture_compression_backend() {
            Some(backend) => {
                format!("ON. Utilizing BC7 backend: {}.", backend.label())
            }
            None => "OFF. Terrain textures will use uncompressed RGBA8.".to_string(),
        }
    }

    pub fn log_unavailable_texture_compression_backend_warning(&self) {
        if !self.lossy_texture_compression {
            return;
        }

        let warning = match self.lossy_texture_compression_backend {
            LossyTextureCompressionBackend::Dds => None,
            LossyTextureCompressionBackend::BlockCompression
                if !is_bc7_encoder_backend_available(Bc7EncoderBackend::BlockCompression) =>
            {
                Some(
                    "graphics.lossy_texture_compression_backend = \"block_compression\" requested, but this build was compiled without the `uddconv/block_compression` backend. Falling back to dds.",
                )
            }
            LossyTextureCompressionBackend::Ispc
                if !is_bc7_encoder_backend_available(Bc7EncoderBackend::Ispc) =>
            {
                Some(
                    "graphics.lossy_texture_compression_backend = \"ispc\" requested, but this build was compiled without the `uddconv/ispc` backend. Falling back to dds.",
                )
            }
            _ => None,
        };

        if let Some(message) = warning {
            console_logger::one(LogSev::Warn, LogAbout::General, message);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextureCompressionLogState {
    enabled: bool,
    active_backend: Option<LossyTextureCompressionBackend>,
}

impl TextureCompressionLogState {
    fn from_graphics(graphics: &SectGraphics) -> Self {
        Self {
            enabled: graphics.lossy_texture_compression,
            active_backend: graphics.active_lossy_texture_compression_backend(),
        }
    }
}

/// Resource used to debounce saving settings to disk.
#[derive(Resource)]
pub struct SettingsSaveTimer(pub Timer);

/// Tracks file modification times for hot-reload detection.
/// Checked at 1-second intervals so we never run read_dir or stat on every frame.
#[derive(Resource)]
pub struct SettingsFileWatcher {
    /// Polling timer — checked once per second to avoid expensive stat calls every frame.
    pub poll_timer: Timer,
    /// Last-known mtime for each watched file.
    pub core_mtime: Option<SystemTime>,
    pub graphics_mtime: Option<SystemTime>,
    pub user_mtime: Option<SystemTime>,
    pub kb_mtime: Option<SystemTime>,
    pub worldmap_rendering_mtime: Option<SystemTime>,
}

impl Default for SettingsFileWatcher {
    fn default() -> Self {
        let assets_path = crate::core::constants::valid_asset_dir();
        // Snapshot the initial mtimes so we don't trigger a reload immediately on startup.
        let mtime_of = |name: &str| -> Option<SystemTime> {
            std::fs::metadata(assets_path.join(name))
                .ok()
                .and_then(|m| m.modified().ok())
        };
        Self {
            poll_timer: Timer::from_seconds(1.0, TimerMode::Repeating),
            core_mtime: mtime_of(CORE_CONFIG_FILE),
            graphics_mtime: mtime_of(GRAPHICS_CONFIG_FILE),
            user_mtime: mtime_of(USER_CONFIG_FILE),
            kb_mtime: mtime_of(KEYBINDINGS_CONFIG_FILE),
            worldmap_rendering_mtime: mtime_of(WORLDMAP_RENDERING_CONFIG_FILE),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SectLogging {
    pub min_severity: LogSev,
    #[serde(default = "default_emit_true")]
    pub emit_flat_plugin_build: bool,
    #[serde(default = "default_emit_true")]
    pub emit_tree_plugin_build: bool,
    pub filters: Vec<LogFilterSetting>,
}

#[derive(Clone, Deserialize, Serialize)]
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
const UO_FILES_CONFIG_FILE: &str = "settings/uo_files.toml";
const USER_CONFIG_FILE: &str = "settings/preferences.toml";
const KEYBINDINGS_CONFIG_FILE: &str = "settings/keybindings.toml";
const GRAPHICS_CONFIG_FILE: &str = "settings/graphics.toml";
const MAPS_CONFIG_FILE: &str = "settings/maps.toml";
const WORLDMAP_RENDERING_CONFIG_FILE: &str = "settings/core_worldmap_rendering.toml";

pub fn load_from_files() -> Settings {
    let assets_path = crate::core::constants::valid_asset_dir();

    let core_path = assets_path.join(CORE_CONFIG_FILE);
    let uo_files_path = assets_path.join(UO_FILES_CONFIG_FILE);
    let user_path = assets_path.join(USER_CONFIG_FILE);
    let maps_path = assets_path.join(MAPS_CONFIG_FILE);
    let worldmap_rendering_path = assets_path.join(WORLDMAP_RENDERING_CONFIG_FILE);

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

    // UO files settings (assets/settings/uo_files.toml)
    let uo_files_contents = std::fs::read_to_string(&uo_files_path)
        .expect("Failed to read settings/uo_files.toml — please ensure assets/settings/uo_files.toml exists");
    let uo_files: SectUoFiles = toml::from_str(&uo_files_contents)
        .expect("Failed to parse settings/uo_files.toml — please fix the file in assets/settings/uo_files.toml");

    // User preferences file contains SectApp fields directly at top level
    let user_contents = std::fs::read_to_string(&user_path)
        .expect("Failed to read settings/preferences.toml — please ensure assets/settings/preferences.toml exists and is valid");

    let user_app: SectApp = toml::from_str(&user_contents)
        .expect("Failed to parse settings/preferences.toml — please fix the file in assets/settings/preferences.toml");

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

    let maps_contents = std::fs::read_to_string(&maps_path).expect(
        "Failed to read maps.toml — please ensure assets/settings/maps.toml exists and is valid",
    );

    let maps: SectMaps = toml::from_str(&maps_contents)
        .expect("Failed to parse maps.toml — please fix the file in assets/settings/maps.toml");

    let worldmap_rendering_contents = std::fs::read_to_string(&worldmap_rendering_path).expect(
        "Failed to read settings/core_worldmap_rendering.toml — please ensure assets/settings/core_worldmap_rendering.toml exists",
    );

    let worldmap_rendering: SectWorldMapRendering = toml::from_str(&worldmap_rendering_contents)
        .expect(
            "Failed to parse settings/core_worldmap_rendering.toml — please fix the file in assets/settings/core_worldmap_rendering.toml",
        );

    Settings {
        core: core_data.core,
        graphics,
        uo_files,
        app: user_app,
        logging: core_data.logging,
        keybindings,
        maps,
        worldmap_rendering,
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
    let user_path = assets_path.join(USER_CONFIG_FILE);

    match toml::to_string_pretty(&settings.app) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&user_path, toml_str) {
                paris::error!("Failed to save preferences.toml: {}", e);
            } else {
                console_logger::one(LogSev::Info, LogAbout::General, "Saved preferences.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize user preferences: {}", e);
        }
    }
}

pub fn save_keybindings(settings: &Settings) {
    let assets_path = crate::core::constants::valid_asset_dir();
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);

    match toml::to_string_pretty(&settings.keybindings) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&kb_path, toml_str) {
                paris::error!("Failed to save keybindings.toml: {}", e);
            } else {
                console_logger::one(LogSev::Info, LogAbout::General, "Saved keybindings.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize keybindings: {}", e);
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
                paris::error!("Failed to save graphics.toml: {}", e);
            } else {
                console_logger::one(LogSev::Info, LogAbout::General, "Saved graphics.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize graphics settings: {}", e);
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
                paris::error!("Failed to save core.toml: {}", e);
            } else {
                console_logger::one(LogSev::Info, LogAbout::General, "Saved core.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize core settings: {}", e);
        }
    }
}

pub fn save_worldmap_rendering_settings(settings: &Settings) {
    let assets_path = crate::core::constants::valid_asset_dir();
    let path = assets_path.join(WORLDMAP_RENDERING_CONFIG_FILE);

    match toml::to_string_pretty(&settings.worldmap_rendering) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&path, toml_str) {
                paris::error!("Failed to save core_worldmap_rendering.toml: {}", e);
            } else {
                console_logger::one(
                    LogSev::Info,
                    LogAbout::General,
                    "Saved core_worldmap_rendering.toml",
                );
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize worldmap rendering settings: {}", e);
        }
    }
}

// ----

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
            .add_systems(Update, sys_log_texture_compression_status)
            .insert_resource(SettingsSaveTimer({
                let mut t = Timer::from_seconds(1.0, TimerMode::Once);
                t.pause();
                t
            }))
            .init_resource::<SettingsFileWatcher>()
            .add_systems(
                FixedUpdate,
                (
                    sys_evlisten_switch_wireframe,
                    sys_debounced_save,
                    sys_hotreload_settings,
                ),
            );
    }
}

fn sys_startup_load_file(mut commands: Commands) {
    let data = load_from_files();

    // Initialize logger settings
    apply_logging_settings(&data.logging);
    data.graphics
        .log_unavailable_texture_compression_backend_warning();
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

fn sys_log_texture_compression_status(
    settings: Res<Settings>,
    mut last_logged_state: Local<Option<TextureCompressionLogState>>,
) {
    let current_state = TextureCompressionLogState::from_graphics(&settings.graphics);
    if last_logged_state.as_ref() == Some(&current_state) {
        return;
    }

    let prefix = if last_logged_state.is_none() {
        "Startup texture compression state"
    } else {
        "Texture compression state changed"
    };
    let message = format!(
        "{prefix}: {}",
        settings.graphics.texture_compression_log_status()
    );
    console_logger::one(LogSev::Info, LogAbout::Settings, &message);

    *last_logged_state = Some(current_state);
}

/// Hot-reload system: polls file modification times every second, and updates the
/// Settings resource in-place if any watched file has changed on disk.
/// This is intentionally a mtime poll rather than Bevy's AssetLoader because
/// settings are spread across three files with custom multi-file merging logic.
fn sys_hotreload_settings(
    time: Res<Time>,
    mut watcher: ResMut<SettingsFileWatcher>,
    mut settings: ResMut<Settings>,
) {
    // Respect Settings toggle: if hot-reload is globally disabled, skip checking.
    if !settings.app.debug.hot_reload_enabled {
        return;
    }
    // Only check once per second — stat syscalls are cheap but redundant every frame.
    watcher.poll_timer.tick(time.delta());
    if !watcher.poll_timer.just_finished() {
        return;
    }

    let assets_path = crate::core::constants::valid_asset_dir();
    let mtime_of = |name: &str| -> Option<SystemTime> {
        std::fs::metadata(assets_path.join(name))
            .ok()
            .and_then(|m| m.modified().ok())
    };

    let new_core = mtime_of(CORE_CONFIG_FILE);
    let new_graphics = mtime_of(GRAPHICS_CONFIG_FILE);
    let new_user = mtime_of(USER_CONFIG_FILE);
    let new_kb = mtime_of(KEYBINDINGS_CONFIG_FILE);
    let new_worldmap_rendering = mtime_of(WORLDMAP_RENDERING_CONFIG_FILE);

    let core_changed = new_core != watcher.core_mtime;
    let graphics_changed = new_graphics != watcher.graphics_mtime;
    let user_changed = new_user != watcher.user_mtime;
    let kb_changed = new_kb != watcher.kb_mtime;
    let worldmap_rendering_changed = new_worldmap_rendering != watcher.worldmap_rendering_mtime;

    if !(core_changed
        || graphics_changed
        || user_changed
        || kb_changed
        || worldmap_rendering_changed)
    {
        return;
    }

    // At least one file changed. Re-read the full settings bundle.
    let new_data = load_from_files();

    if core_changed {
        settings.core = new_data.core.clone();
        settings.logging = new_data.logging.clone();
        watcher.core_mtime = new_core;
        // Update plugin log toggles on settings hot-reload
        set_plugin_log_toggles(
            settings.logging.emit_flat_plugin_build,
            settings.logging.emit_tree_plugin_build,
        );
        console_logger::one(LogSev::Info, LogAbout::General, "Hot-reloaded: core.toml");
    }
    if graphics_changed {
        settings.graphics = new_data.graphics.clone();
        watcher.graphics_mtime = new_graphics;
        settings
            .graphics
            .log_unavailable_texture_compression_backend_warning();
        console_logger::one(
            LogSev::Info,
            LogAbout::General,
            "Hot-reloaded: graphics.toml",
        );
    }
    if user_changed {
        settings.app = new_data.app.clone();
        watcher.user_mtime = new_user;
        console_logger::one(
            LogSev::Info,
            LogAbout::General,
            "Hot-reloaded: preferences.toml",
        );
    }
    if kb_changed {
        settings.keybindings = new_data.keybindings.clone();
        watcher.kb_mtime = new_kb;
        console_logger::one(
            LogSev::Info,
            LogAbout::General,
            "Hot-reloaded: keybindings.toml",
        );
    }
    if worldmap_rendering_changed {
        settings.worldmap_rendering = new_data.worldmap_rendering.clone();
        watcher.worldmap_rendering_mtime = new_worldmap_rendering;
        console_logger::one(
            LogSev::Info,
            LogAbout::General,
            "Hot-reloaded: core_worldmap_rendering.toml",
        );
    }
}

fn sys_apply(
    settings_res: Res<Settings>,
    mut windows_q: Query<&mut Window>,
    mut zoom_res: ResMut<RenderZoom>,
) {
    let mut w = windows_q.single_mut().unwrap();
    w.resolution = WindowResolution::new(
        settings_res.app.window.width as u32,
        settings_res.app.window.height as u32,
    );

    zoom_res.write_val(settings_res.app.window.zoom);
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
    //mut commands: Commands,
    //query: Query<Entity, With<Wireframe>>,
) {
    log_system_add_update::<SettingsPlugin>(fname!());
    for _ in events.read() {
        // This disables global wireframe for all meshes immediately
        config.global = !config.global;

        /*
        // Optionally, remove the Wireframe component from all entities
        for entity in query.iter() {
            commands.entity(entity).remove::<Wireframe>();
        }
        */
    }
}

/// Automatically saves user preferences if they have been modified, with a 1s debounce timer.
fn sys_debounced_save(
    time: Res<Time>,
    settings: Res<Settings>,
    mut save_timer: ResMut<SettingsSaveTimer>,
    mut last_saved_app: Local<Option<SectApp>>,
    mut last_saved_graphics: Local<Option<SectGraphics>>,
    mut last_saved_keybindings: Local<Option<SectKeybindings>>,
    mut last_saved_core: Local<Option<SectCore>>,
    mut last_saved_worldmap_rendering: Local<Option<SectWorldMapRendering>>,
) {
    if settings.is_added() {
        *last_saved_app = Some(settings.app.clone());
        *last_saved_graphics = Some(settings.graphics.clone());
        *last_saved_keybindings = Some(settings.keybindings.clone());
        *last_saved_core = Some(settings.core.clone());
        *last_saved_worldmap_rendering = Some(settings.worldmap_rendering.clone());
        return;
    }

    let app_changed = last_saved_app
        .as_ref()
        .map_or(true, |last| last != &settings.app);
    let kb_changed = last_saved_keybindings
        .as_ref()
        .map_or(true, |last| last != &settings.keybindings);
    let graphics_changed = last_saved_graphics
        .as_ref()
        .map_or(true, |last| last != &settings.graphics);
    let core_changed = last_saved_core
        .as_ref()
        .map_or(true, |last| last != &settings.core);
    let worldmap_rendering_changed = last_saved_worldmap_rendering
        .as_ref()
        .map_or(true, |last| last != &settings.worldmap_rendering);

    if app_changed || graphics_changed || kb_changed || core_changed || worldmap_rendering_changed {
        // Reset timer whenever a change occurs
        save_timer.0.reset();
        save_timer.0.unpause();
    }

    if !save_timer.0.is_paused() {
        save_timer.0.tick(time.delta());
        if save_timer.0.just_finished() {
            if app_changed {
                save_app_settings(&settings);
                *last_saved_app = Some(settings.app.clone());
            }

            if graphics_changed {
                save_graphics_settings(&settings);
                *last_saved_graphics = Some(settings.graphics.clone());
            }

            if kb_changed {
                save_keybindings(&settings);
                *last_saved_keybindings = Some(settings.keybindings.clone());
            }

            if core_changed {
                save_core_settings(&settings);
                *last_saved_core = Some(settings.core.clone());
            }

            if worldmap_rendering_changed {
                save_worldmap_rendering_settings(&settings);
                *last_saved_worldmap_rendering = Some(settings.worldmap_rendering.clone());
            }

            save_timer.0.pause();
        }
    }
}
