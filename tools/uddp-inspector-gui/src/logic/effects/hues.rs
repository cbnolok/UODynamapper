//! The hue atlas of hued NIF particles, from a `hues.uddp` package (with hue names) or from the client's
//! `Hues.uop` (`build/hues/hues.dds`). Both hold the same 1024 x 1024 strip layout `particle_sim::hued` samples.

use std::path::{Path, PathBuf};

use particle_sim::mips::decode_dds_levels;
use particle_sim::raster::RasterTexture;
use udd_assets::HuesPackage;
use udd_assets::hues::{HUES_TEXTURE_HEIGHT, HUES_TEXTURE_WIDTH};
use uocf::enhanced::hues::hues_atlas_hash;
use uocf::uop_container::package::{LoadMode, UopPackage};

/// File name of the hues package next to `effects_ec.uddp`.
pub const HUES_PACKAGE_FILE_NAME: &str = "hues.uddp";

/// A loaded atlas and where it came from.
pub struct HueAtlas {
    pub texture: std::sync::Arc<RasterTexture>,
    pub source: PathBuf,
    /// Hue names (only a `hues.uddp` has them).
    pub names: Option<HuesPackage>,
}

impl HueAtlas {
    pub fn hue_name(&self, hue: u32) -> Option<&str> {
        let hue = u16::try_from(hue).ok()?;
        self.names.as_ref()?.hue_name(hue)
    }
}

/// `hues.uddp` next to `package_path`, if there is one.
pub fn sibling_hues_package(package_path: &Path) -> Option<PathBuf> {
    let path = package_path.parent()?.join(HUES_PACKAGE_FILE_NAME);
    path.is_file().then_some(path)
}

pub fn load_hues_package(path: &Path) -> Result<HueAtlas, String> {
    let package = HuesPackage::load(path).map_err(|error| format!("{}: {error:#}", path.display()))?;
    let texture = RasterTexture::from_rgba8(
        HUES_TEXTURE_WIDTH as usize,
        HUES_TEXTURE_HEIGHT as usize,
        package.texture_bytes(),
    )
    .ok_or_else(|| format!("{}: the hue texture has the wrong size", path.display()))?;
    Ok(HueAtlas { texture: std::sync::Arc::new(texture), source: path.to_owned(), names: Some(package) })
}

/// The atlas of the client's `Hues.uop`.
pub fn load_hues_uop(path: &Path) -> Result<HueAtlas, String> {
    let package = UopPackage::load_with_mode(path, LoadMode::Lazy)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let payload = package
        .unpack_file_by_hash(hues_atlas_hash())
        .map_err(|error| format!("failed to unpack the hue atlas: {error}"))?
        .ok_or_else(|| format!("{} has no hue atlas", path.display()))?;
    let dds = decode_dds_levels(&payload)?;
    let level = dds.levels.first().ok_or_else(|| "the hue atlas has no level".to_owned())?;
    let texture = RasterTexture::from_rgba8(dds.width as usize, dds.height as usize, level)
        .ok_or_else(|| "the hue atlas has the wrong size".to_owned())?;
    Ok(HueAtlas { texture: std::sync::Arc::new(texture), source: path.to_owned(), names: None })
}
