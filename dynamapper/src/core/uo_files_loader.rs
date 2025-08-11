#![allow(unused)]

use crate::core::system_sets::StartupSysSet;
use crate::prelude::*;
use bevy::prelude::*;
use dashmap::DashMap;
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
pub struct MapPlanesRes(pub Arc<DashMap<u32, map::MapPlane>>);

#[derive(Resource)]
pub struct TileDataRes(pub Arc<tiledata::TileData>);

#[derive(Resource)]
pub struct TexMap2DRes(pub Arc<land_texture_2d::TexMap2D>);

pub struct UoInterfaceSettings {
    pub base_folder: PathBuf,
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

pub fn sys_setup_uo_data(mut commands: Commands) {
    let lg = |text: &str| logger::one(None, logger::LogSev::Info, logger::LogAbout::UoFiles, text);
    let uo_path: PathBuf =
        "/mnt/dati/_proj_local/_uo_clients/Ultima Online Mondain's Legacy".into();

    lg("Start loading UO Data.");

    let map_plane_index = 0_u32;
    lg(
        &format!("Loading map plane {map_plane_index} structure (map{map_plane_index}.mul)...")
            .as_str(),
    );
    let map_plane = map::MapPlane::init(
        uo_path.join(&format!("map{map_plane_index}.mul")),
        map_plane_index,
    )
    .expect(&format!("Error initializing map plane {map_plane_index}"));
    let mut map_planes = DashMap::<u32, map::MapPlane>::new();
    map_planes.insert(map_plane_index, map_plane);

    lg("Loading Tiledata");
    let tiledata = tiledata::TileData::load(uo_path.join("tiledata.mul")).expect("Load tiledata");

    lg("Loading Texmaps...");
    let texmap_2d =
        land_texture_2d::TexMap2D::load(uo_path.join("texmaps.mul"), uo_path.join("texidx.mul"))
            .expect("Load texmap");

    lg("Done loading UO Data.");

    commands.insert_resource(UoInterfaceSettingsRes(Arc::new(UoInterfaceSettings {
        base_folder: uo_path,
    })));
    commands.insert_resource(MapPlanesRes(Arc::new(map_planes)));
    commands.insert_resource(TileDataRes(Arc::new(tiledata)));
    commands.insert_resource(TexMap2DRes(Arc::new(texmap_2d)));
}
