//! The conversion tables the converters read: kept with the Dynamapper assets in the source checkout, and shipped in
//! the tools release's `shared/` folder. [`uocf::utils::bundled_files::resolve`] finds them.

use uocf::utils::bundled_files::BundledFile;

/// Folder of the conversion tables, below the release's shared folder and below the Dynamapper assets.
pub const CONVERSION_TABLES_DIR: &str = "cc_ec_convtables";
/// The Dynamapper assets in the source checkout, relative to this crate's folder.
const DYNAMAPPER_ASSETS_FROM_CRATE: &str = "../../dynamapper/assets";

const fn conversion_table(name: &'static str) -> BundledFile {
    BundledFile {
        name,
        subdir: CONVERSION_TABLES_DIR,
        crate_dir: env!("CARGO_MANIFEST_DIR"),
        checkout_dir_from_crate: DYNAMAPPER_ASSETS_FROM_CRATE,
    }
}

/// Classic land id to EC material routing.
pub const TERRAIN_TRANSCODE_KDL: BundledFile = conversion_table("TerrainTranscode.kdl");
/// Reviewed EC terrain layer substitutions.
pub const EC_TERRAIN_OVERRIDES_KDL: BundledFile = conversion_table("EcTerrainOverrides.kdl");
/// Reviewed EC surface art redirections.
pub const EC_SURFACE_OVERRIDES_KDL: BundledFile = conversion_table("EcSurfaceOverrides.kdl");
/// EC mobile animation item metadata.
pub const EC_MOBILE_ANIMATIONS_KDL: BundledFile = conversion_table("EcMobileAnimations.kdl");

/// Every table above, for the checks that the source checkout holds them.
pub const ALL_CONVERSION_TABLES: &[BundledFile] =
    &[TERRAIN_TRANSCODE_KDL, EC_TERRAIN_OVERRIDES_KDL, EC_SURFACE_OVERRIDES_KDL, EC_MOBILE_ANIMATIONS_KDL];

#[cfg(test)]
mod tests {
    use super::*;
    use uocf::utils::bundled_files::resolve;

    /// The checkout this crate builds from holds every table where the release packaging copies it from, and a
    /// development build finds it there.
    #[test]
    fn the_source_checkout_holds_every_conversion_table() {
        for table in ALL_CONVERSION_TABLES {
            let in_checkout = table.checkout_dir().join(table.relative_path());
            assert!(in_checkout.is_file(), "{}", in_checkout.display());
            assert_eq!(resolve(None, *table).unwrap(), in_checkout);
        }
    }
}
