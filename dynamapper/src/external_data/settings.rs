use std::path::PathBuf;
use std::time::SystemTime;

use crate::prelude::*;
use crate::core::render::scene::camera::RenderZoom;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::util_lib::uo_coords::*;
use bevy::{
    //asset::{AssetLoader, LoadContext, io::Reader},
    pbr::wireframe::WireframeConfig,
    prelude::*,
    window::WindowResolution
};
use serde::{Deserialize, Serialize};


#[derive(Asset, Clone, Debug, Deserialize, Serialize, Resource, TypePath)]
pub struct Settings {
    pub core: SectCore,
    pub app: SectApp,
    pub logging: SectLogging,
    pub keybindings: SectKeybindings,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SectKeybindings {
    pub shader_settings: KeyCode,
    pub user_settings: KeyCode,
    pub keybindings_help: KeyCode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectCore {
    pub uo_files: SectUoFiles,
    pub world: SectWorld,
    pub graphics: SectGraphics,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectApp {
    pub input: SectInput,
    pub window: SectWindow,
    pub debug: SectDebug,
    pub performance: SectPerformance,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectUoFiles {
    pub folder: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
    pub egui_scale: f32,
    /// Per-overlay scale for the player position overlay.
    pub player_position_scale: f32,
    /// Per-overlay scale for system messages rendered via egui.
    pub sysmessages_scale: f32,
    /// Per-overlay scale for the performance overlay.
    pub performance_overlay_scale: f32,
    pub free_camera: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectWorld {
    pub start_p: UOVec4,
    pub hide_player: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectDebug {
    pub map_render_wireframe: bool,
    /// Whether settings hot-reload from disk is enabled. Disabled by default until
    /// all systems correctly respond to runtime changes.
    pub hot_reload_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectPerformance {
    pub show_overlay: bool,
    pub frame_limit_enabled: bool,
    pub target_fps: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectGraphics {
    pub lossy_texture_compression: bool,
    pub reduce_unfocused_fps: bool,
    pub texture_filtering: u32,       // 0: Point, 1: Linear
    pub texture_reconstruction: u32,  // 0: None, 1: Bicubic, 2: FSR
    pub sharpening_strength: f32,     // 0.0 to 1.0
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
    pub user_mtime: Option<SystemTime>,
    pub kb_mtime: Option<SystemTime>,
}

impl Default for SettingsFileWatcher {
    fn default() -> Self {
        let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());
        // Snapshot the initial mtimes so we don't trigger a reload immediately on startup.
        let mtime_of = |name: &str| -> Option<SystemTime> {
            std::fs::metadata(assets_path.join(name))
                .ok()
                .and_then(|m| m.modified().ok())
        };
        Self {
            poll_timer: Timer::from_seconds(1.0, TimerMode::Repeating),
            core_mtime: mtime_of(CORE_CONFIG_FILE),
            user_mtime: mtime_of(USER_CONFIG_FILE),
            kb_mtime: mtime_of(KEYBINDINGS_CONFIG_FILE),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SectLogging {
    pub min_severity: LogSev,
    pub filters: Vec<LogFilterSetting>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogFilterSetting {
    pub sev: Option<LogSev>,
    pub about: Option<LogAbout>,
    pub suppress: bool,
}

// ----

#[derive(Message, Debug, Clone, Default, PartialEq)]
pub struct ToggleWireframe;

// ----

const CORE_CONFIG_FILE: &str = "core_settings.toml";
const USER_CONFIG_FILE: &str = "user_preferences.toml";
const KEYBINDINGS_CONFIG_FILE: &str = "keybindings.toml";

pub fn load_from_files() -> Settings {
    let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());

    let core_path = assets_path.join(CORE_CONFIG_FILE);
    let user_path = assets_path.join(USER_CONFIG_FILE);

    let core_contents = std::fs::read_to_string(&core_path)
        .expect("Failed to read core_settings.toml");

    // Core settings file contains top-level core and logging sections
    #[derive(Deserialize)]
    struct CoreWrapper {
        core: SectCore,
        logging: SectLogging,
    }
    let core_data: CoreWrapper = toml::from_str(&core_contents)
        .expect("Failed to parse core_settings.toml — please fix the file in assets/core_settings.toml");

    // User preferences file contains SectApp fields directly at top level
    let user_contents = std::fs::read_to_string(&user_path)
        .expect("Failed to read user_preferences.toml — please ensure assets/user_preferences.toml exists and is valid");

    let user_app: SectApp = toml::from_str(&user_contents)
        .expect("Failed to parse user_preferences.toml — please fix the file in assets/user_preferences.toml");

    // Keybindings file
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);
    let kb_contents = std::fs::read_to_string(&kb_path)
        .expect("Failed to read keybindings.toml — please ensure assets/keybindings.toml exists and is valid");

    let keybindings: SectKeybindings = toml::from_str(&kb_contents)
        .expect("Failed to parse keybindings.toml — please fix the file in assets/keybindings.toml");

    Settings {
        core: core_data.core,
        app: user_app,
        logging: core_data.logging,
        keybindings,
    }
}

pub fn save_app_settings(settings: &Settings) {
    let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());
    let user_path = assets_path.join(USER_CONFIG_FILE);

    match toml::to_string_pretty(&settings.app) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&user_path, toml_str) {
                paris::error!("Failed to save user_preferences.toml: {}", e);
            } else {
                console_logger::one(None, LogSev::Info, LogAbout::General, "Saved user_preferences.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize user preferences: {}", e);
        }
    }
}

pub fn save_keybindings(settings: &Settings) {
    let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);

    match toml::to_string_pretty(&settings.keybindings) {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(&kb_path, toml_str) {
                paris::error!("Failed to save keybindings.toml: {}", e);
            } else {
                console_logger::one(None, LogSev::Info, LogAbout::General, "Saved keybindings.toml");
            }
        }
        Err(e) => {
            paris::error!("Failed to serialize keybindings: {}", e);
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
            .insert_resource(SettingsSaveTimer({
                let mut t = Timer::from_seconds(1.0, TimerMode::Once);
                t.pause();
                t
            }))
            .init_resource::<SettingsFileWatcher>()
            .add_systems(Update, (sys_evlisten_switch_wireframe, sys_debounced_save, sys_hotreload_settings))
            ;
    }
}

fn sys_startup_load_file(mut commands: Commands) {
    let data = load_from_files();

    // Initialize logger settings
    let mut filters = Vec::new();
    for f in &data.logging.filters {
        filters.push(console_logger::LogFilter {
            sev: f.sev.clone(),
            about: f.about.clone(),
            suppress: f.suppress,
        });
    }
    console_logger::set_log_settings(console_logger::LogSettings {
        min_severity: Some(data.logging.min_severity.clone()),
        filters,
    });

    commands.insert_resource(data);
    console_logger::one(
        None,
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file for global access.",
    );
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

    let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());
    let mtime_of = |name: &str| -> Option<SystemTime> {
        std::fs::metadata(assets_path.join(name))
            .ok()
            .and_then(|m| m.modified().ok())
    };

    let new_core = mtime_of(CORE_CONFIG_FILE);
    let new_user = mtime_of(USER_CONFIG_FILE);
    let new_kb   = mtime_of(KEYBINDINGS_CONFIG_FILE);

    let core_changed = new_core != watcher.core_mtime;
    let user_changed = new_user != watcher.user_mtime;
    let kb_changed   = new_kb   != watcher.kb_mtime;

    if !(core_changed || user_changed || kb_changed) {
        return;
    }

    // At least one file changed. Re-read the full settings bundle.
    let new_data = load_from_files();

    if core_changed {
        settings.core = new_data.core.clone();
        settings.logging = new_data.logging.clone();
        watcher.core_mtime = new_core;
        console_logger::one(None, LogSev::Info, LogAbout::General,
            "Hot-reloaded: core_settings.toml");
    }
    if user_changed {
        settings.app = new_data.app.clone();
        watcher.user_mtime = new_user;
        console_logger::one(None, LogSev::Info, LogAbout::General,
            "Hot-reloaded: user_preferences.toml");
    }
    if kb_changed {
        settings.keybindings = new_data.keybindings.clone();
        watcher.kb_mtime = new_kb;
        console_logger::one(None, LogSev::Info, LogAbout::General,
            "Hot-reloaded: keybindings.toml");
    }
}


fn sys_apply(
    settings_res: Res<Settings>,
    mut windows_q: Query<&mut Window>,
    mut zoom_res: ResMut<RenderZoom>,
){
    let mut w = windows_q.single_mut().unwrap();
    w.resolution = WindowResolution::new(settings_res.app.window.width as u32, settings_res.app.window.height as u32);

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
    // TODO: disable hot reloading for now. We would need every system to fetch the updated settings and react.

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

// TODO: Make something actually emit this event:
/*
fn keyboard_toggle_wireframe(
    input: Res<Input<KeyCode>>,
    mut writer: EventWriter<ToggleWireframe>,
) {
    if input.just_pressed(KeyCode::W) {
        writer.send(ToggleWireframe);
    }
}
     */

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
    mut last_saved_keybindings: Local<Option<SectKeybindings>>,
) {
    if settings.is_changed() && !settings.is_added() {
        // Reset timer whenever a change occurs
        save_timer.0.reset();
        save_timer.0.unpause();
    }

    if !save_timer.0.is_paused() {
        save_timer.0.tick(time.delta());
        if save_timer.0.just_finished() {
            save_app_settings(&settings);

            // Only save keybindings when they have actually changed
            let kb_changed = last_saved_keybindings
                .as_ref()
                .map_or(true, |last| last != &settings.keybindings);
            if kb_changed {
                save_keybindings(&settings);
                *last_saved_keybindings = Some(settings.keybindings.clone());
            }

            save_timer.0.pause();
        }
    }
}
