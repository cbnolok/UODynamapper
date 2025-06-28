#![allow(dead_code)]

//mod startup;

use dynamapper::uocf::geo::{land_texture_2d, map};
use dynamapper::uocf::tiledata;

//use color_eyre;
use once_cell::sync::{Lazy, OnceCell};
use std::path::PathBuf;
use std::sync::RwLock;

pub struct UoFileData {
    pub startup_inited: bool,
    pub map_planes: RwLock<Vec<map::MapPlane>>,
    pub tiledata: RwLock<tiledata::TileData>,
    pub texmap_2d: RwLock<land_texture_2d::TexMap2D>,
}

static UO_PATH: OnceCell<PathBuf> = OnceCell::new();
static DATA: Lazy<UoFileData> =
    Lazy::new(|| startup::load_essential_uo_files().expect("Load UO Files"));

pub fn init(uo_path: PathBuf) {
    UO_PATH.get_or_init(|| uo_path);

    // Force loading UO Data here.
    Lazy::force(&DATA);
}

pub fn get_ref() -> &'static UoFileData {
    if !DATA.startup_inited {
        panic!("Trying to retrieve uninitialized UO File Data");
    }
    &DATA
}

/*
pub fn get_mut() -> &'static UoFileData {
    if !DATA.startup_inited {
        panic!("Trying to retrieve uninitialized UO File Data");
    }
    &mut DATA
}
*/
