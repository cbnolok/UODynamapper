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

/// Directory for runtime-mutable settings (user preferences + session state).
///
/// These are written here — a `cache/` folder placed next to the resolved asset
/// dir — instead of into the read-only `assets/` tree, so the app never dirties
/// its own source/asset files (which used to trigger a rebuild+relink and keep
/// `git status` dirty). System/bundled config stays authoritative in `assets/`.
/// The directory is created on demand.
pub fn valid_cache_dir() -> PathBuf {
    let asset_dir = valid_asset_dir();
    let base = asset_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&cache);
    cache
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
