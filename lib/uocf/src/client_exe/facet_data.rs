//! Checks that the facet data of a client folder has the sizes an executable's facet table holds
//! ([`super::facet_dimensions`]): per facet, the sectors of `facetN.uop` and `facetNx.uop`, their `data.map`, the
//! FacetDefinition in `facets.uop` (Enhanced Client) and the radar images `facetNN.dds` / `facetNNx.dds`.

crate::eyre_imports!();
use std::path::Path;

use crate::enhanced::facet_definition::{read_facet_definition, FACET_DEFINITIONS_UOP_NAME};
use crate::enhanced::facet_radar::{radar_dds_file_name, radar_mip_count, FacetRadarDds};
use crate::enhanced::facet_sector::{data_map_entry_path, facet_package_file_name, sector_entry_path, FacetGeometry};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

/// The radar file of a variant package: `facetNNx.dds` (shipped for facet 0 in the `fp` client).
const VARIANT_RADAR_SUFFIX: &str = "x.dds";
const RADAR_EXTENSION: &str = ".dds";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacetDataStatus {
    /// The file agrees with the table.
    Ok,
    /// The file disagrees with the table: the client would misread or crash.
    Mismatch,
    /// The file does not exist.
    Absent,
}

/// One checked file or entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetDataItem {
    pub source: String,
    pub status: FacetDataStatus,
    pub detail: String,
}

/// The checks of one facet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetDataReport {
    pub facet: u8,
    pub geometry: FacetGeometry,
    pub items: Vec<FacetDataItem>,
}

impl FacetDataReport {
    pub fn mismatches(&self) -> impl Iterator<Item = &FacetDataItem> {
        self.items.iter().filter(|item| item.status == FacetDataStatus::Mismatch)
    }
}

fn item(source: impl Into<String>, ok: bool, detail: String) -> FacetDataItem {
    let status = if ok { FacetDataStatus::Ok } else { FacetDataStatus::Mismatch };
    FacetDataItem { source: source.into(), status, detail }
}

fn absent(source: impl Into<String>) -> FacetDataItem {
    FacetDataItem { source: source.into(), status: FacetDataStatus::Absent, detail: String::new() }
}

/// The sectors and `data.map` of one facet package.
fn check_package(items: &mut Vec<FacetDataItem>, path: &Path, facet: u8, geometry: FacetGeometry) -> eyre::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let package = UopPackage::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    let sized = |hash: u64| package.get_file_by_hash(hash).is_some_and(|file| file.has_size());
    let expected = geometry.sector_count();
    let missing: Vec<u32> = (0..expected).filter(|&sector| !sized(hash_file_name_single(&sector_entry_path(facet, sector)))).collect();
    let data_map_hash = hash_file_name_single(&data_map_entry_path(facet));
    let entries = package.iter_files().filter(|file| file.has_size()).count();
    let other = entries - (expected as usize - missing.len()) - usize::from(sized(data_map_hash));
    let mut detail = format!("{}/{expected} sectors", expected as usize - missing.len());
    if let Some(first) = missing.first() {
        detail.push_str(&format!(", {} missing (first {first})", missing.len()));
    }
    if other != 0 {
        detail.push_str(&format!(", {other} entries outside the {}x{} sector grid", geometry.sector_columns(), geometry.sector_rows()));
    }
    items.push(item(name.clone(), missing.is_empty() && other == 0, detail));

    let data_map = format!("{name} data.map");
    match package.unpack_file_by_hash(data_map_hash)? {
        None => items.push(absent(data_map)),
        Some(payload) => {
            let stored = FacetGeometry::from_data_map_payload(&payload)?;
            items.push(item(data_map, stored == geometry, format!("{}x{}", stored.width, stored.height)));
        }
    }
    Ok(())
}

/// The size and mips of one radar image.
fn check_radar(items: &mut Vec<FacetDataItem>, path: &Path, geometry: FacetGeometry) {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match FacetRadarDds::load(path) {
        Ok(radar) => {
            let mips = radar_mip_count(geometry.width, geometry.height);
            let ok = radar.width() == geometry.width && radar.height() == geometry.height && radar.mip_count() == mips;
            let detail = format!("{}x{}, {} mips (expected {mips})", radar.width(), radar.height(), radar.mip_count());
            items.push(item(name, ok, detail));
        }
        Err(error) => items.push(item(name, false, format!("{error:#}"))),
    }
}

/// Checks every facet of `dimensions` (an executable's facet table, record order) against the data in `client_dir`.
/// Only reads.
pub fn check_facet_data(dimensions: &[FacetGeometry], client_dir: &Path) -> eyre::Result<Vec<FacetDataReport>> {
    let definitions_path = client_dir.join(FACET_DEFINITIONS_UOP_NAME);
    let definitions = if definitions_path.is_file() {
        Some(UopPackage::load(&definitions_path).wrap_err_with(|| format!("load {}", definitions_path.display()))?)
    } else {
        None
    };
    let mut reports = Vec::with_capacity(dimensions.len());
    for (facet, geometry) in dimensions.iter().copied().enumerate() {
        let facet = facet as u8;
        let mut items = Vec::new();
        for variant in [false, true] {
            let path = client_dir.join(facet_package_file_name(facet, variant));
            if path.is_file() {
                check_package(&mut items, &path, facet, geometry)?;
            } else if !variant {
                items.push(absent(facet_package_file_name(facet, variant)));
            }
        }
        let source = format!("{FACET_DEFINITIONS_UOP_NAME} facet {facet}");
        match &definitions {
            None => items.push(absent(source)),
            Some(package) => match read_facet_definition(package, u32::from(facet))? {
                None => items.push(absent(source)),
                Some(definition) => {
                    let ok = definition.width == geometry.width && definition.height == geometry.height;
                    items.push(item(source, ok, format!("{}x{}", definition.width, definition.height)));
                }
            },
        }
        let radar = radar_dds_file_name(facet);
        let variant_radar = radar.replace(RADAR_EXTENSION, VARIANT_RADAR_SUFFIX);
        for (name, required) in [(radar, true), (variant_radar, false)] {
            let path = client_dir.join(&name);
            if path.is_file() {
                check_radar(&mut items, &path, geometry);
            } else if required {
                items.push(absent(name));
            }
        }
        reports.push(FacetDataReport { facet, geometry, items });
    }
    Ok(reports)
}
