use std::path::PathBuf;

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

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub overlay_scale: f32,
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

const CORE_CONFIG_FILE: &'static str = "core_settings.toml";
const USER_CONFIG_FILE: &'static str = "user_preferences.toml";
const KEYBINDINGS_CONFIG_FILE: &'static str = "keybindings.toml";

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
    let core_data: CoreWrapper = match toml::from_str(&core_contents) {
        Ok(s) => s,
        Err(_e) => {
            // If parsing fails, use defaults for core section
            CoreWrapper {
                core: SectCore {
                    uo_files: SectUoFiles { folder: "C:\\UO".to_string() },
                    world: SectWorld { start_p: UOVec4::default(), hide_player: false },
                    graphics: SectGraphics {
                        lossy_texture_compression: true,
                        reduce_unfocused_fps: true,
                        texture_filtering: 0,
                        texture_reconstruction: 0,
                        sharpening_strength: 0.0,
                    },
                },
                logging: SectLogging {
                    min_severity: LogSev::Info,
                    filters: vec![],
                },
            }
        }
    };

    // User preferences file contains SectApp fields directly at top level
    let user_contents = std::fs::read_to_string(&user_path)
        .unwrap_or_else(|_| "".to_string());
    
    let user_app: SectApp = match toml::from_str(&user_contents) {
        Ok(s) => s,
        Err(_e) => {
            // If it fails (maybe partial file), use defaults for app section
            SectApp {
                input: SectInput { movement_speed_multiplier: 1.0 },
                window: SectWindow { width: 1024.0, height: 768.0, zoom: 1.0, egui_scale: 1.0, overlay_scale: 1.0, free_camera: false },
                debug: SectDebug { map_render_wireframe: false },
                performance: SectPerformance { 
                    show_overlay: true,
                    frame_limit_enabled: true,
                    target_fps: 60,
                },
            }
        }
    };

    // Keybindings file
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);
    let kb_contents = std::fs::read_to_string(&kb_path)
        .unwrap_or_else(|_| "".to_string());
    
    let keybindings: SectKeybindings = match toml::from_str(&kb_contents) {
        Ok(s) => s,
        Err(_) => {
            SectKeybindings {
                shader_settings: KeyCode::F3,
                user_settings: KeyCode::F2,
                keybindings_help: KeyCode::F1,
            }
        }
    };

    Settings {
        core: core_data.core,
        app: user_app,
        logging: core_data.logging,
        keybindings,
    }
}

pub fn save_user_preferences(settings: &Settings) {
    let assets_path = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string());
    let user_path = assets_path.join(USER_CONFIG_FILE);
    let kb_path = assets_path.join(KEYBINDINGS_CONFIG_FILE);

    // Save user preferences
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

    // Save keybindings
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
            .insert_resource(SettingsSaveTimer(Timer::from_seconds(1.0, TimerMode::Once)))
            .add_systems(Update, (sys_evlisten_switch_wireframe, sys_debounced_save))
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
) {
    if settings.is_changed() {
        // Reset timer whenever a change occurs
        save_timer.0.reset();
        save_timer.0.unpause();
    }

    if !save_timer.0.is_paused() {
        save_timer.0.tick(time.delta());
        if save_timer.0.just_finished() {
            save_user_preferences(&settings);
            save_timer.0.pause();
        }
    }
}
