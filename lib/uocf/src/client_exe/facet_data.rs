//! Checks that the facet data of a client folder has the sizes an executable's facet table holds
//! ([`super::facet_dimensions`]). Enhanced Client and Kingdom Reborn, per facet: the sectors of `facetN.uop` and
//! `facetNx.uop`, their `data.map`, the FacetDefinition in `facets.uop` (Enhanced Client) and the radar images
//! `facetNN.dds` / `facetNNx.dds`. Classic, per facet and table copy: the map (MUL, or `LegacyMUL.uop` blocks), the
//! `staidx` entries and the `statics` ranges they point to, in the files the record's ids name.

crate::eyre_imports!();
use std::path::Path;

use crate::classic::facet_sources::{map_file_names, ClassicMapFormat};
use crate::classic::generic_index::{IndexElement, IndexFile};
use crate::classic::map::{uop_map_data_size, MapBlock, UOP_MAP_TRAILING_BLOCKS};
use crate::client_exe::classic_facets::{classic_map_blocks, ClassicFileRole};
use crate::client_exe::facet_dimensions::{ClientExeInspection, ClientFamily};
use crate::uop_container::package::LoadMode;
use crate::utils::path::find_file_ignoring_case;

use crate::enhanced::facet_definition::{read_facet_definition, FACET_DEFINITIONS_UOP_NAME};
use crate::enhanced::facet_radar::{radar_dds_file_name, radar_mip_count, FacetRadarDds};
use crate::enhanced::facet_sector::{data_map_entry_path, facet_package_file_name, sector_entry_path, FacetGeometry};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

/// The extension of a Classic UOP map package.
const UOP_EXTENSION: &str = ".uop";

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

/// Checks the facet data of `client_dir` against the executable `inspection`: [`check_facet_data`] for the Enhanced
/// Client and Kingdom Reborn, [`check_classic_facet_data`] for Classic. Only reads.
pub fn check_client_facet_data(inspection: &ClientExeInspection, client_dir: &Path) -> eyre::Result<Vec<FacetDataReport>> {
    check_client_facet_data_with_sizes(inspection, &inspection.dimensions(), client_dir)
}

/// [`check_client_facet_data`] against the sizes `dimensions` (one per record of the table, e.g. edits not written
/// yet) instead of the table's; the Classic file ids still come from `inspection`'s records. Only reads.
pub fn check_client_facet_data_with_sizes(
    inspection: &ClientExeInspection,
    dimensions: &[FacetGeometry],
    client_dir: &Path,
) -> eyre::Result<Vec<FacetDataReport>> {
    eyre::ensure!(
        dimensions.len() == inspection.facet_count(),
        "{} sizes for a table of {} facets",
        dimensions.len(),
        inspection.facet_count()
    );
    match inspection.family {
        ClientFamily::Classic => classic_facet_data(inspection, dimensions, client_dir),
        ClientFamily::Enhanced | ClientFamily::KingdomReborn => check_facet_data(dimensions, client_dir),
    }
}

/// The map, `staidx` and `statics` files of `facet` in every table copy, without repeats: the names the record ids
/// give, or the usual names when the file name table was not located.
fn classic_facet_files(inspection: &ClientExeInspection, facet: usize, client_dir: &Path) -> Vec<(ClassicFileRole, String)> {
    let names = inspection.classic.as_ref().and_then(|layout| layout.file_names.as_ref());
    let roles = [ClassicFileRole::Map, ClassicFileRole::Staidx, ClassicFileRole::Statics];
    let mut files: Vec<(ClassicFileRole, String)> = Vec::new();
    for table in &inspection.tables {
        let record = table.records[facet].classic.expect("a Classic table");
        for role in roles {
            let name = match names.and_then(|names| names.name(record.file_id(role))) {
                Some(name) => name.to_owned(),
                None => fallback_file_name(role, facet as u32, client_dir),
            };
            if !files.iter().any(|(each_role, each)| *each_role == role && each.eq_ignore_ascii_case(&name)) {
                files.push((role, name));
            }
        }
    }
    files
}

/// The usual name of a Classic file of map `map_index`: the first map file found, else `mapN.mul`.
fn fallback_file_name(role: ClassicFileRole, map_index: u32, client_dir: &Path) -> String {
    match role {
        ClassicFileRole::Map => [ClassicMapFormat::Mul, ClassicMapFormat::Uop]
            .into_iter()
            .flat_map(|format| map_file_names(map_index, format))
            .find(|name| find_file_ignoring_case(client_dir, name).is_some())
            .unwrap_or_else(|| map_file_names(map_index, ClassicMapFormat::Mul).remove(0)),
        _ => format!("{}{map_index}.mul", role.label()),
    }
}

/// The blocks of the Classic map `map_index` in `path` and the bytes past the last whole block: its size (MUL), or
/// its entries' bytes (`LegacyMUL.uop` and its `x` variant, entries `build/mapNlegacymul/NNNNNNNN.dat`), over the
/// block size.
fn classic_map_file_blocks(path: &Path, map_index: u32) -> eyre::Result<(u64, u64)> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let size = if name.ends_with(UOP_EXTENSION) {
        let package =
            UopPackage::load_with_mode(path, LoadMode::Lazy).wrap_err_with(|| format!("load {}", path.display()))?;
        uop_map_data_size(&package, map_index)
    } else {
        std::fs::metadata(path).wrap_err_with(|| format!("read {}", path.display()))?.len()
    };
    Ok((size / MapBlock::PACKED_SIZE as u64, size % MapBlock::PACKED_SIZE as u64))
}

/// One Classic file of a facet of size `geometry`.
fn check_classic_file(
    items: &mut Vec<FacetDataItem>,
    role: ClassicFileRole,
    path: &Path,
    map_index: u32,
    geometry: FacetGeometry,
) -> eyre::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let blocks = classic_map_blocks(geometry);
    match role {
        ClassicFileRole::Map => {
            let (found, remainder) = classic_map_file_blocks(path, map_index)?;
            let trailing = if name.to_ascii_lowercase().ends_with(UOP_EXTENSION) { UOP_MAP_TRAILING_BLOCKS } else { 0 };
            let ok = remainder == 0 && (blocks..=blocks + trailing).contains(&found);
            items.push(item(name, ok, format!("{found}/{blocks} blocks")));
        }
        ClassicFileRole::Staidx => {
            let size = std::fs::metadata(path).wrap_err_with(|| format!("read {}", path.display()))?.len();
            let expected = blocks * u64::from(IndexElement::PACKED_SIZE);
            items.push(item(name, size == expected, format!("{} entries, {size}/{expected} bytes", size / u64::from(IndexElement::PACKED_SIZE))));
        }
        ClassicFileRole::Statics => {
            let size = std::fs::metadata(path).wrap_err_with(|| format!("read {}", path.display()))?.len();
            let index_name = name.to_ascii_lowercase().replacen(ClassicFileRole::Statics.label(), ClassicFileRole::Staidx.label(), 1);
            let Some(index_path) = path.parent().and_then(|dir| find_file_ignoring_case(dir, &index_name)) else {
                items.push(item(name, true, format!("{size} bytes (no {index_name} to check the ranges)")));
                return Ok(());
            };
            let index = IndexFile::load(index_path)?;
            let outside = (0..index.element_count())
                .filter_map(|element| index.element(element).ok())
                .filter_map(|element| Some(u64::from(element.lookup()?) + u64::from(element.len()?)))
                .filter(|end| *end > size)
                .count();
            items.push(item(name, outside == 0, format!("{size} bytes, {outside} {index_name} ranges outside")));
        }
        ClassicFileRole::Mapdif | ClassicFileRole::Stadif | ClassicFileRole::Stadifi => {}
    }
    Ok(())
}

/// Checks every facet of the Classic `inspection` (both table copies) against the data in `client_dir`: map blocks,
/// `staidx` entries and `statics` ranges. Files the folder does not have are reported absent. Only reads.
pub fn check_classic_facet_data(inspection: &ClientExeInspection, client_dir: &Path) -> eyre::Result<Vec<FacetDataReport>> {
    classic_facet_data(inspection, &inspection.dimensions(), client_dir)
}

fn classic_facet_data(
    inspection: &ClientExeInspection,
    dimensions: &[FacetGeometry],
    client_dir: &Path,
) -> eyre::Result<Vec<FacetDataReport>> {
    eyre::ensure!(inspection.family == ClientFamily::Classic, "not a Classic client");
    let mut reports = Vec::with_capacity(inspection.facet_count());
    for (facet, geometry) in dimensions.iter().copied().enumerate() {
        let mut items = Vec::new();
        for (role, name) in classic_facet_files(inspection, facet, client_dir) {
            match find_file_ignoring_case(client_dir, &name) {
                Some(path) => check_classic_file(&mut items, role, &path, facet as u32, geometry)?,
                None => items.push(absent(name)),
            }
        }
        reports.push(FacetDataReport { facet: facet as u8, geometry, items });
    }
    Ok(reports)
}
