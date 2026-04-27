#![allow(unused)]

use crate::core::system_sets::StartupSysSet;
use crate::external_data::settings::Settings;
use crate::prelude::*;
use bevy::prelude::*;
//use parking_lot::RwLock;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use uocf::classic::tiledata;
use uocf::classic::{land_texture, map};

const MAX_MAP_INDEX: u32 = 5; // inclusive max, so map0..=map5

/// Arc is not yet needed (no cross-thread sharing), but kept for consistency
/// with other UO data resources and future background-thread access.
#[derive(Resource)]
pub struct UoFilesSettingsRes(pub Arc<UoFilesSettings>);

/// Not wrapped in Arc: owned exclusively by the Bevy main world.
/// Accessed only via Res<MapPlanesRes> on the main thread.
#[derive(Resource)]
pub struct MapPlanesRes(pub Vec<Option<map::MapPlane>>);

/// Arc: will be cloned and sent to background threads for tiledata lookups
/// (e.g. future item/static rendering, pathfinding).
#[derive(Resource)]
pub struct TileDataRes(pub Arc<tiledata::TileData>);

/// Arc: cloned into the chunk-loader OS thread (via LoadRequest.texmap_2d)
/// and passed to texture cache systems that warm pixel data off-thread.
#[derive(Resource)]
pub struct TexMap2DRes(pub Arc<land_texture::TexMap>);

/// Optional prepacked atlas package for static and land art.
#[derive(Resource)]
pub struct CcArtPackageRes(pub Arc<uddconv::cc_art::CcArtPackage>);

/// Optional prepacked atlas package for EC static art.
#[derive(Resource)]
pub struct EcArtPackageRes(pub Arc<uddconv::ec_art::EcArtPackage>);

/// Optional prepacked atlas package for EC land art.
#[derive(Resource)]
pub struct EcLandPackageRes(pub Arc<uddconv::ec_land::EcLandPackage>);

pub struct UoFilesSettings {
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

pub fn sys_setup_uo_data(mut commands: Commands, settings: Res<Settings>) {
    log_system_add_startup::<UOFilesPlugin>(StartupSysSet::LoadStartupUOFiles, fname!());
    let lg = |text: &str| {
        console_logger::one(
            console_logger::LogSev::Info,
            console_logger::LogAbout::UoFiles,
            text,
        )
    };
    let uo_path: PathBuf = settings.uo_files.folder.clone().into();
    let lossy = settings.graphics.lossy_texture_compression;

    lg("Start loading UO Data.");

    let mut map_planes: Vec<Option<map::MapPlane>> = std::iter::repeat_with(|| None)
        .take((MAX_MAP_INDEX + 1) as usize)
        .collect::<Vec<_>>();
    for map_plane_index in 0..=MAX_MAP_INDEX {
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
            map_planes[map_plane_index as usize] = Some(map_plane);
        }
    }

    lg("Loading Tiledata");
    let tiledata = tiledata::TileData::load(uo_path.join("tiledata.mul")).expect("Load tiledata");

    lg("Loading Texmaps...");
    let texmap_2d =
        land_texture::TexMap::load(uo_path.join("texmaps.mul"), uo_path.join("texidx.mul"))
            .expect("Load texmap");

    let cc_art_path = uo_path.join("cc_art.uddp");
    let cc_art_package = if cc_art_path.exists() {
        lg("Loading optional cc_art.uddp package...");
        Some(
            uddconv::cc_art::CcArtPackage::load(&cc_art_path)
                .unwrap_or_else(|_| panic!("Error loading {}", cc_art_path.display())),
        )
    } else {
        None
    };

    let ec_art_path = uo_path.join("ec_art.uddp");
    let ec_art_package = if ec_art_path.exists() {
        lg("Loading optional ec_art.uddp package...");
        Some(
            uddconv::ec_art::EcArtPackage::load(&ec_art_path)
                .unwrap_or_else(|_| panic!("Error loading {}", ec_art_path.display())),
        )
    } else {
        None
    };

    let ec_land_path = uo_path.join("ec_land.uddp");
    let ec_land_package = if ec_land_path.exists() {
        lg("Loading optional ec_land.uddp package...");
        Some(
            uddconv::ec_land::EcLandPackage::load(&ec_land_path)
                .unwrap_or_else(|_| panic!("Error loading {}", ec_land_path.display())),
        )
    } else {
        None
    };

    lg("Done loading UO Data.");

    commands.insert_resource(UoFilesSettingsRes(Arc::new(UoFilesSettings {
        base_folder: uo_path,
    })));
    commands.insert_resource(MapPlanesRes(map_planes));
    commands.insert_resource(TileDataRes(Arc::new(tiledata)));
    commands.insert_resource(TexMap2DRes(Arc::new(texmap_2d)));
    if let Some(cc_art_package) = cc_art_package {
        commands.insert_resource(CcArtPackageRes(Arc::new(cc_art_package)));
    }
    if let Some(ec_art_package) = ec_art_package {
        commands.insert_resource(EcArtPackageRes(Arc::new(ec_art_package)));
    }
    if let Some(ec_land_package) = ec_land_package {
        commands.insert_resource(EcLandPackageRes(Arc::new(ec_land_package)));
    }
}
