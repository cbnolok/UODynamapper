use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};
use bevy::{
    asset::{AssetLoader, LoadContext, io::Reader},
    prelude::*,
};
use serde::Deserialize;

const CONFIG_FILE_NAME: &'static str = "settings.toml";

#[derive(Asset, Clone, Debug, Deserialize, TypePath)]
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
    pub start_p: [i32; 3], // or [f32;3].
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
        app.init_asset_loader::<SettingsAssetLoader>() // Register custom loader
            .init_asset::<SettingsAsset>()
            .add_systems(Startup, sys_load_settings)
            .add_systems(Update, sys_settings_reloaded);
    }
}

// Asset wrapper
#[derive(Asset, TypePath, Debug, Clone)]
pub struct SettingsAsset(pub Settings);

#[derive(Resource, Clone)]
pub struct SettingsHandle(pub Handle<Settings>);

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

fn sys_load_settings(asset_server: Res<AssetServer>, mut commands: Commands) {
    let handle: Handle<Settings> = asset_server.load(CONFIG_FILE_NAME);
    commands.insert_resource(SettingsHandle(handle));
}

fn sys_settings_reloaded(
    mut events: EventReader<AssetEvent<Settings>>,
    handles: Res<SettingsHandle>,
    assets: Res<Assets<Settings>>,
) {
    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } if id == &handles.0.id() => {
                if let Some(settings) = assets.get(&handles.0) {
                    println!("Settings loaded (or hot reloaded): {settings:#?}");
                }
            }
            AssetEvent::Modified { id } if id == &handles.0.id() => {
                if let Some(settings) = assets.get(&handles.0) {
                    println!("Settings hot reloaded: {settings:#?}");
                }
            }
            _ => {}
        }
    }
}
