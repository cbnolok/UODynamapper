use bevy::prelude::Vec3;
use std::path::PathBuf;

/// Determines the default directory for assets.
///
/// The search order is:
/// 1. Current Working Directory (CWD): looks for an "assets" folder in the same directory where the process was started.
/// 2. Cargo Manifest Directory: if running via `cargo run`, looks for "assets" in the crate root.
/// 3. Executable Directory: if running as a standalone binary, looks for "assets" next to the .exe.
/// 4. Fallback: returns "assets" as a relative path.
pub fn valid_asset_dir() -> PathBuf {
    // 1. Check Current Working Directory first.
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_assets = cwd.join("assets");
        if cwd_assets.is_dir() {
            return cwd_assets;
        }
    }

    // 2. Check Cargo Manifest directory (dev mode).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_assets = PathBuf::from(manifest_dir).join("assets");
        if manifest_assets.is_dir() {
            return manifest_assets;
        }
    }

    // 3. Check Executable directory.
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let exe_assets = parent.join("assets");
            if exe_assets.is_dir() {
                return exe_assets;
            }
        }
    }

    // 4. Fallback to relative "assets".
    PathBuf::from("assets")
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
