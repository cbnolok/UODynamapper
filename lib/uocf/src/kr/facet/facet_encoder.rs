//! # UO Kingdom Reborn Facet Encoder
//!
//! Builds a Kingdom Reborn facet package (`facetN.uop`) and its radar image (`facetNN.dds`) from a Classic map
//! and statics, with the Enhanced Client builder (`crate::enhanced::facet_encoder`) in the KR square layout (see
//! `facet_decoder` for the format):
//!
//! - sectors numbered column-major over `ceil(w/64) x ceil(h/64)`, the squares past the map edge all zero bytes;
//! - the `u8 facet, u16 sector id` header, and the `data.map` entry (u32 height, u32 width) first, uncompressed;
//! - each square stores the KR material and `subtype` of its Classic id ([`KrLandMap`]) and the Classic id;
//! - delimiters (6 bytes, ascending direction) follow the shipped rule, holding the neighbour's z and KR terrain
//!   id; statics (9 bytes) keep their `statics.mul` order, and every static is written;
//! - a square with more than 255 statics fails the build (u8 count);
//! - the package is UOP version 4 with 100 entries per block, zlib sectors and timestamp data block headers, like
//!   the shipped KR packages.

use std::path::Path;

use super::kr_land_map::KrLandMap;
use crate::classic::map::MapPlane;
use crate::classic::statics::StaticsReader;
use crate::enhanced::client_dialect::ClientDialect;
use crate::classic::world::ClassicWorld;
use crate::enhanced::facet_encoder::{build_facet, ClassicFiles};
use crate::enhanced::facet_package::EcFacetPackage;
use crate::enhanced::facet_radar::{radar_dds_file_name, FacetRadarDds, RadarPalette};
use crate::enhanced::facet_sector::facet_package_file_name;

crate::eyre_imports!();

// region: --- Public API (Convenience & Application Use)

/// Encodes a Classic map plane and its statics into the KR `facet{map_index}.uop` and `facet{map_index:02}.dds`
/// in `output_dir`, with the radar colours of `radarcol_path` (Classic `radarcol.mul`).
pub fn encode_map_plane(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    radarcol_path: &Path,
    output_dir: &Path,
    map_index: u8,
    land_map: &KrLandMap,
) -> eyre::Result<()> {
    let palette = RadarPalette::load_radarcol(radarcol_path)?;
    let (mut package, radar) = build_kr_facet_from_classic(map_plane, statics, map_index, &palette, land_map)?;
    package.save(output_dir.join(facet_package_file_name(map_index, false)))?;
    radar.save(&output_dir.join(radar_dds_file_name(map_index)))
}

/// Builds the KR facet package (in memory) and the radar image of a Classic map plane and its statics. The facet
/// size is the map plane's. Fails on a Classic land id `land_map` lacks.
pub fn build_kr_facet_from_classic(
    map_plane: &mut MapPlane,
    statics: &StaticsReader,
    facet: u8,
    palette: &RadarPalette,
    land_map: &KrLandMap,
) -> eyre::Result<(EcFacetPackage, FacetRadarDds)> {
    let mut source = ClassicFiles { plane: map_plane, statics };
    let (package, radar) = build_facet(&mut source, facet, Some(palette), ClientDialect::KingdomReborn, &|classic| {
        land_map.square_land(classic)
    })?;
    Ok((package, radar.expect("a palette gives a radar image")))
}

/// Builds the KR facet package (in memory) of a facet held in memory, and its radar image when `palette` is
/// given. The facet size is the world's. Fails on a Classic land id `land_map` lacks.
pub fn build_kr_facet_from_world(
    world: &ClassicWorld,
    facet: u8,
    palette: Option<&RadarPalette>,
    land_map: &KrLandMap,
) -> eyre::Result<(EcFacetPackage, Option<FacetRadarDds>)> {
    build_facet(&mut &*world, facet, palette, ClientDialect::KingdomReborn, &|classic| land_map.square_land(classic))
}

// endregion: --- Public API
