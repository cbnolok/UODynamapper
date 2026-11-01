//! What the plan reads from the installed packages: the facet size and the tallest static sprite.

use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use udd_assets::map_metadata::decode_map_package_metadata;
use udd_assets::tex_art_cc::TexArtCcPackage;
use udd_container::{DataType, FileKey, UddpReader, unpack_type};

pub const ART_PACKAGE_FILE: &str = "tex_art_cc.uddp";

pub fn map_package_file(facet: u32) -> String {
    format!("map{facet}.uddp")
}

/// The facet's size in tiles, from the metadata record that ends `map{facet}.uddp`.
pub fn read_facet_size(packages_dir: &Path, facet: u32) -> eyre::Result<(u32, u32)> {
    let path = packages_dir.join(map_package_file(facet));
    let package = UddpReader::load(&path).map_err(|error| eyre::eyre!("{error}")).wrap_err_with(|| format!("open {}", path.display()))?;
    let records = package.records();
    let record = records.last().ok_or_else(|| eyre::eyre!("{} has no records", path.display()))?;
    if unpack_type(record.locator.meta32) != DataType::Metadata as u8 {
        eyre::bail!("{} does not end with its metadata record", path.display());
    }
    let FileKey::Id(id) = record.key else { eyre::bail!("{}: the metadata record has no dense id", path.display()) };
    let bytes = package.read_file_by_dense_id(id).map_err(|error| eyre::eyre!("{error}")).wrap_err("read map metadata")?;
    let metadata = decode_map_package_metadata(&bytes).map_err(|error| eyre::eyre!("{}: {error}", path.display()))?;
    if metadata.map_id != facet {
        eyre::bail!("{} holds map {}, not {facet}", path.display(), metadata.map_id);
    }
    Ok((metadata.width_tiles, metadata.height_tiles))
}

/// The most any static sprite rises above its tile's top vertex, at client scale: its height plus its vertical
/// draw offset, which bounds the rise whichever way the offset points.
pub fn read_max_sprite_height(packages_dir: &Path) -> eyre::Result<u32> {
    let path = packages_dir.join(ART_PACKAGE_FILE);
    let package = TexArtCcPackage::load(&path).wrap_err_with(|| format!("open {}", path.display()))?;
    let tallest = package
        .slots()
        .iter()
        .filter(|slot| slot.is_present() && slot.is_static())
        .map(|slot| slot.logical_height().ceil() as u32 + u32::from(slot.draw_offset_y.unsigned_abs()))
        .max();
    tallest.ok_or_else(|| eyre::eyre!("{} holds no static sprites", path.display()))
}
