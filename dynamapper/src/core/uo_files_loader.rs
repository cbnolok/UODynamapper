#![allow(unused)]

use crate::prelude::*;
use bevy::prelude::*;
use uocf::geo::{land_texture_2d, map};
use uocf::tiledata;
use uocf::eyre_imports; eyre_imports!();
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

pub struct UoInterfaceSettings {
    pub base_folder: PathBuf,
}

#[derive(Resource)]
pub struct UoFileData {
    pub settings:   RwLock<UoInterfaceSettings>,
    pub map_planes: RwLock<Vec<map::MapPlane>>,
    pub tiledata:   RwLock<tiledata::TileData>,
    pub texmap_2d:  RwLock<land_texture_2d::TexMap2D>,
}

pub struct UoFilesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(UoFilesPlugin);
impl Plugin for UoFilesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(OnEnter(AppState::LoadStartupFiles), sys_setup_uo_data);
    }
}

pub fn sys_setup_uo_data(
    mut commands: Commands,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let lg = |text| logger::one(None, logger::LogSev::Info, logger::LogAbout::UoFiles, text);
    let uo_path: PathBuf = "/mnt/dati/_proj_local/_uo_clients/Ultima Online Mondain's Legacy".into();

    lg("Start loading UO Data.");
    // TODO: inject a logger function to uocf crate calls.

    lg("Loading map plane 0 structure (map0)...");
    let map_plane =
        map::MapPlane::init(uo_path.join("map0.mul"), 0)
            .expect("Initialize map plane");
    let mut map_planes = Vec::<map::MapPlane>::new();
    map_planes.push(map_plane);

    // Test map loading.
    /*
        let map_rect_to_show = map::MapRectCells {
            x0: 0,
            y0: 0,
            width: 16,
            height: 16,
        };
        let blocks_to_load = map::MapPlane::calc_blocks_to_load(&mut map_plane, &map_rect_to_show);
        map_plane
            .load_blocks(blocks_to_load)
            .wrap_err("Load map blocks in the view area")?;
    */

    lg("Loading Tiledata");
    let tiledata =
        tiledata::TileData::load(uo_path.join("tiledata.mul"))
            .expect("Load tiledata");

    lg("Loading Texmaps...");
    let texmap_2d =
        land_texture_2d::TexMap2D::load(
            uo_path.join("texmaps.mul"),
            uo_path.join("texidx.mul"))
                .expect("Load texmap");

    lg("Done loading UO Data.");

    let data = UoFileData {
        settings: RwLock::new(UoInterfaceSettings {
            base_folder: uo_path,
        }),
        map_planes: RwLock::new(map_planes),
        tiledata: RwLock::new(tiledata),
        texmap_2d: RwLock::new(texmap_2d),
    };

    log_appstate_change("SetupScene");
    commands.insert_resource(data);
    next_state.set(AppState::SetupScene);
}
