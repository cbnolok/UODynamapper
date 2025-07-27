use std::path::PathBuf;

use crate::util_lib::uo_coords::*;
use crate::logger::{self, LogAbout, LogSev};
use crate::{impl_tracked_plugin, util_lib::tracked_plugin::*};
use bevy::{
    //asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;

const CONFIG_FILE_NAME: &'static str = "settings.toml";

#[derive(Asset, Clone, Debug, Deserialize, Resource, TypePath)]
pub struct Settings {
    pub uo_files: UoFiles,
    pub input: Input,
    pub window: Window,
    pub world: World,
    pub debug: Debug,
    // pub logger: Option<Logger>, // For the commented section
}

#[derive(Clone, Debug, Deserialize)]
pub struct UoFiles {
    pub folder: String, // or PathBuf for extra fanciness
}

#[derive(Clone, Debug, Deserialize)]
pub struct Input {
    pub movement_speed_multiplier: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Window {
    pub height: f32,
    pub width: f32,
    pub zoom: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct World {
    pub start_p: UOVec4, //[i32; 4], // or [f32;4].
}

#[derive(Clone, Debug, Deserialize)]
pub struct Debug {
    pub map_render_wireframe: bool,
}

pub struct SettingsPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(SettingsPlugin);
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app
            .init_asset::<SettingsAsset>()
            .init_asset::<Settings>()
            //.init_asset_loader::<SettingsAssetLoader>() // Register custom loader
            .add_systems(PreStartup, sys_load_settings)
            //.add_systems(Update, sys_settings_reloaded)
            ;
    }
}

// Wrappers
#[derive(Asset, TypePath, Debug, Clone)]
pub struct SettingsAsset(pub Settings);

#[derive(Resource, Clone)]
pub struct SettingsHandle(pub Handle<Settings>);

// ----

fn sys_load_settings(/*asset_server: Res<AssetServer>,*/ mut commands: Commands) {
    let settings_with_rel_path: PathBuf = PathBuf::from(crate::core::constants::ASSET_FOLDER.to_string() + CONFIG_FILE_NAME);

    let contents = std::fs::read_to_string(&settings_with_rel_path)
        .expect("Failed to read settings file");
    let settings: Settings = toml::from_str(&contents)
        .expect("Failed to parse settings TOML");

    commands.insert_resource(settings);

    logger::one(
        None,
        LogSev::Info,
        LogAbout::Startup,
        "Loaded settings file.",
    );

    // TODO: reset window size, read zoom and wireframe value from settings.

    /*
    // TODO: disable hot reloading for now. We would need every system to fetch the updated settings and react.

    // Ttrack for changes and update it asynchronously.
    let handle: Handle<Settings> = asset_server.load(CONFIG_FILE_NAME);
    commands.insert_resource(SettingsHandle(handle));
    */
}

/*
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
