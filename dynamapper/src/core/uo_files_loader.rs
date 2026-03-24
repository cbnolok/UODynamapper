#![allow(unused)]

use crate::core::system_sets::StartupSysSet;
use crate::external_data::settings::Settings;
use crate::prelude::*;
use bevy::prelude::*;
//use dashmap::DashMap;
//use parking_lot::RwLock;
use uocf::eyre_imports;
use uocf::geo::{land_texture_2d, map};
use uocf::tiledata;
eyre_imports!();
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Resource)]
pub struct UoInterfaceSettingsRes(pub Arc<UoInterfaceSettings>);

#[derive(Resource)]
pub struct MapPlanesRes(pub Vec<(u32, map::MapPlane)>);

#[derive(Resource)]
pub struct TileDataRes(pub Arc<tiledata::TileData>);

#[derive(Resource)]
pub struct TexMap2DRes(pub Arc<land_texture_2d::TexMap2D>);

pub struct UoInterfaceSettings {
    pub base_folder: PathBuf,
    pub lossy_texture_compression: bool,
}

pub struct UOFilesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(UOFilesPlugin);
impl Plugin for UOFilesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_uo_data.in_set(StartupSysSet::LoadStartupUOFiles),
        );
    }
}

pub fn sys_setup_uo_data(mut commands: Commands, settings: Res<Settings>) {
    log_system_add_startup::<UOFilesPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());
    let lg = |text: &str| {
        console_logger::one(
            None,
            console_logger::LogSev::Info,
            console_logger::LogAbout::UoFiles,
            text,
        )
    };
    let uo_path: PathBuf = settings.core.uo_files.folder.clone().into();
    let lossy = settings.core.graphics.lossy_texture_compression;

    lg("Start loading UO Data.");

    let mut map_planes = Vec::<(u32, map::MapPlane)>::new();
    for map_plane_index in 0..6 {
        let map_file = uo_path.join(format!("map{map_plane_index}.mul"));
        if map_file.exists() {
            lg(&format!(
                "Loading map plane {map_plane_index} structure (map{map_plane_index}.mul)..."
            ));
            let map_size_override =
                settings
                    .maps
                    .map_size(map_plane_index)
                    .map(|map_size| map::MapSizeCells {
                        width: map_size.width,
                        height: map_size.height,
                    });
            let map_plane =
                map::MapPlane::init_with_size(map_file, map_plane_index, map_size_override)
                    .unwrap_or_else(|_| panic!("Error initializing map plane {map_plane_index}"));
            map_planes.push((map_plane_index, map_plane));
        }
    }

    lg("Loading Tiledata");
    let tiledata = tiledata::TileData::load(uo_path.join("tiledata.mul")).expect("Load tiledata");

    lg("Loading Texmaps...");
    let texmap_2d =
        land_texture_2d::TexMap2D::load(uo_path.join("texmaps.mul"), uo_path.join("texidx.mul"))
            .expect("Load texmap");

    lg("Done loading UO Data.");

    commands.insert_resource(UoInterfaceSettingsRes(Arc::new(UoInterfaceSettings {
        base_folder: uo_path,
        lossy_texture_compression: lossy,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    // TODO: Do re really need to encapsulate with Arc those Bevy Resources?
    commands.insert_resource(TileDataRes(Arc::new(tiledata)));
    commands.insert_resource(TexMap2DRes(Arc::new(texmap_2d)));
}
