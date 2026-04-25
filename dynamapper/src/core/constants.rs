use bevy::prelude::Vec3;
use std::path::PathBuf;

pub fn default_asset_dir() -> PathBuf {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        PathBuf::from(manifest_dir).join("assets")
    } else {
        match std::env::current_exe() {
            Ok(exe_path) => {
                if let Some(parent) = exe_path.parent() {
                    parent.join("assets")
                } else {
                    PathBuf::from("assets")
                }
            }
            Err(_) => PathBuf::from("assets"),
        }
    }
}

pub const ASSET_FOLDER: &str = "assets/";


//------------------------------------
// World light
//------------------------------------

// /// Used by shaders to calculate lighting.

//#[derive(Resource, Deref)]
//pub struct LightDir(pub Vec3);

// Hardcoded light direction vector.
pub const BAKED_GLOBAL_LIGHT: Vec3 = Vec3::new(-1.0, 2.5, -1.0);

