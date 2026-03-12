use std::path::PathBuf;

use crate::prelude::*;
use crate::core::render::scene::camera::RenderZoom;
use crate::logger::{self, LogAbout, LogSev};
use crate::util_lib::uo_coords::*;
use bevy::{
    //asset::{AssetLoader, LoadContext, io::Reader},
    pbr::wireframe::WireframeConfig,
    prelude::*,
    window::WindowResolution
};
use serde::Deserialize;

const CONFIG_FILE_NAME: &'static str = "settings.toml";

#[derive(Asset, Clone, Debug, Deserialize, Resource, TypePath)]
pub struct Settings {
    pub uo_files: SectUoFiles,
    pub input: SectInput,
    pub window: SectWindow,
    pub world: SectWorld,
    pub debug: SectDebug,
    pub graphics: SectGraphics,
    pub logging: SectLogging,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectUoFiles {
    pub folder: String, // or PathBuf for extra fanciness
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectInput {
    pub movement_speed_multiplier: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectWindow {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectWorld {
    pub start_p: UOVec4, //[i32; 4], // or [f32;4].
    pub hide_player: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectDebug {
    pub map_render_wireframe: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectGraphics {
    /// When true, terrain textures are BC7-compressed on the CPU before uploading to the GPU.
    /// This saves ~8x VRAM (160 MB -> ~20 MB) at the cost of near-lossless quality.
    /// Requires BC texture compression GPU support (most desktop GPUs support this).
    pub lossy_texture_compression: bool,
    /// When true, the application significantly reduces FPS/updates when the window is unfocused.
    pub reduce_unfocused_fps: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SectLogging {
    pub min_severity: LogSev,
    pub filters: Vec<LogFilterSetting>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LogFilterSetting {
    pub sev: Option<LogSev>,
    pub about: Option<LogAbout>,
    pub suppress: bool,
}

// ----

#[derive(Event)]
pub struct ToggleWireframe;

// ----

pub fn load_from_file() -> Settings {
    let settings_with_rel_path: PathBuf =
        PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string() + CONFIG_FILE_NAME);

    let contents =
        std::fs::read_to_string(&settings_with_rel_path).expect("Failed to read settings file");
    let settings: Settings = match toml::from_str(&contents) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\n<red><bold>!! TOML PARSING ERROR !!</></bold>");
            eprintln!("<red>File: {:?}</>", settings_with_rel_path);
            eprintln!("<red>Error: {}</>\n", e);
            panic!("Configuration failure. Please check your settings.toml");
        }
    };

    settings
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
            .add_event::<ToggleWireframe>()
            .add_systems(PreStartup, sys_startup_load_file)
            .add_systems(Startup, sys_apply)
            .add_systems(Update, sys_evlisten_switch_wireframe)
            ;
    }
}

fn sys_startup_load_file(mut commands: Commands) {
    let data = load_from_file();
    
    // Initialize logger settings
    let mut filters = Vec::new();
    for f in &data.logging.filters {
        filters.push(logger::LogFilter {
            sev: f.sev.clone(),
            about: f.about.clone(),
            suppress: f.suppress,
        });
    }
    logger::set_log_settings(logger::LogSettings {
        min_severity: Some(data.logging.min_severity.clone()),
        filters,
    });

    commands.insert_resource(data);
    logger::one(
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
    w.resolution = WindowResolution::new(settings_res.window.width, settings_res.window.height);

    zoom_res.write_val(settings_res.window.zoom);
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
    mut events: EventReader<ToggleWireframe>,
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
