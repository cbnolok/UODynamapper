use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::eyre::{self, WrapErr};
use csv::{ReaderBuilder, StringRecord, Trim, WriterBuilder};
use udd_assets::{TexArtCcPackage, TexArtEcPackage, TexLandEcPackage};
use udd_assets::tex_art_cc::TexArtCcSlotRecord;
use udd_assets::tex_art_ec::TexArtEcSlotRecord;
use udd_assets::tex_land_ec::{TexLandEcSlotRecord, TexLandEcTerrainProvenanceRecord};
use udd_conv::tex_art_cc::encode_slot_manifest as encode_cc_slot_manifest;
use udd_assets::tex_art_cc::SLOT_MANIFEST_ENTRY_PATH as CC_SLOT_MANIFEST_ENTRY_PATH;
use udd_conv::tex_art_ec::encode_slot_manifest as encode_tex_art_ec_slot_manifest;
use udd_assets::tex_art_ec::SLOT_MANIFEST_ENTRY_PATH as TEX_ART_EC_SLOT_MANIFEST_ENTRY_PATH;
use udd_conv::tex_land_ec::{
    encode_slot_manifest as encode_tex_land_ec_slot_manifest, encode_terrain_provenance_manifest,
};
use udd_assets::tex_land_ec::{
    MISSING_SLOT_ID, MISSING_TERRAIN_LAYER_INDEX, MISSING_TEXTURE_ID,
    UDDP_SLOT_MANIFEST_ENTRY_VPATH as TEX_LAND_EC_SLOT_MANIFEST_ENTRY_PATH,
    UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH,
};
use udd_container::FileKey;
use udd_container::{xxh64_virtual_path, LookupMode, UddpReader};

use crate::extract;
use crate::package_edit;
use crate::package_info;

/// Inspect and edit UODynamapper package files.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show structural information about a .uddp or .uddpi package.
    Info { file: PathBuf },
    /// Extract package contents into a folder for inspection.
    Extract {
        file: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compute the xxh64 virtual-path hash used by path-addressed UDDP packages.
    HashPath { value: String },
    /// Replace one logical path-addressed file and write a rebuilt package.
    Replace {
        file: PathBuf,
        #[arg(long, required = true)]
        path: String,
        #[arg(long, required = true)]
        new_file: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Rebuild a package image while preserving its logical files.
    Rebuild {
        file: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Export editable CSV metadata from supported packages.
    ExportCsv {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = CsvKind::Auto)]
        kind: CsvKind,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Import edited CSV metadata into a rebuilt package.
    ImportCsv {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = CsvKind::Auto)]
        kind: CsvKind,
        #[arg(long, required = true)]
        csv: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare two packages or two metadata CSV files.
    Diff {
        left: PathBuf,
        right: PathBuf,
        #[arg(long, value_enum, default_value_t = DiffKind::Auto)]
        kind: DiffKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
enum CsvKind {
    #[default]
    Auto,
    Slots,
    TerrainProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum DiffKind {
    #[default]
    Auto,
    Slots,
    TerrainProvenance,
    Package,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SlotKind {
    Land,
    Static,
}

impl SlotKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Land => "land",
            Self::Static => "static",
        }
    }

    fn parse(value: &str, line_number: usize) -> eyre::Result<Self> {
        match value {
            "land" => Ok(Self::Land),
            "static" => Ok(Self::Static),
            _ => eyre::bail!("CSV line {line_number} has invalid kind '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AtlasSlotCsvRow {
    art_id: u32,
    kind: SlotKind,
    page_index: u32,
    page_tile_index: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

#[derive(Debug, Clone, Copy)]
struct AtlasPageBounds {
    tile_count: u32,
    used_width: u32,
    used_height: u32,
}

pub fn run() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    match Cli::parse().command {
        Commands::Info { file } => package_info::print_package_info(&file)?,
        Commands::Extract { file, output } => extract::extract_package(&file, output.as_deref())?,
        Commands::HashPath { value } => {
            println!(
                "Path hash for \"{value}\": 0x{:016x}",
                xxh64_virtual_path(&value)
            );
        }
        Commands::Replace {
            file,
            path,
            new_file,
            output,
        } => {
            let output = output.unwrap_or_else(|| derived_output_path(&file, "replaced"));
            let payload =
                fs::read(&new_file).wrap_err_with(|| format!("read {}", new_file.display()))?;
            package_edit::replace_virtual_path_file(&file, &output, &path, &payload)?;
            println!(
                "Replaced '{}' in '{}' and wrote '{}'.",
                path,
                file.display(),
                output.display()
            );
        }
        Commands::Rebuild { file, output } => {
            let output = output.unwrap_or_else(|| derived_output_path(&file, "rebuilt"));
            package_edit::rebuild_package(&file, &output)?;
            println!("Rebuilt '{}' into '{}'.", file.display(), output.display());
        }
        Commands::ExportCsv { file, kind, output } => match kind {
            CsvKind::Auto => export_csv_auto(&file, output.as_deref())?,
            CsvKind::Slots => export_slots_csv_auto(&file, output.as_deref())?,
            CsvKind::TerrainProvenance => export_terrain_provenance_csv(&file, output.as_deref())?,
        },
        Commands::ImportCsv {
            file,
            kind,
            csv,
            output,
        } => match kind {
            CsvKind::Auto => import_csv_auto(&file, &csv, output.as_deref())?,
            CsvKind::Slots => import_slots_csv_auto(&file, &csv, output.as_deref())?,
            CsvKind::TerrainProvenance => {
                import_terrain_provenance_csv(&file, &csv, output.as_deref())?
            }
        },
        Commands::Diff { left, right, kind } => diff_paths(&left, &right, kind)?,
    }

    Ok(())
}

fn export_csv_auto(file: &Path, output: Option<&Path>) -> eyre::Result<()> {
    if TexLandEcPackage::load(file).is_ok() {
        return export_terrain_provenance_csv(file, output);
    }
    export_slots_csv_auto(file, output)
}

fn import_csv_auto(file: &Path, csv: &Path, output: Option<&Path>) -> eyre::Result<()> {
    match detect_csv_kind(csv)? {
        CsvKind::TerrainProvenance => import_terrain_provenance_csv(file, csv, output),
        CsvKind::Slots | CsvKind::Auto => import_slots_csv_auto(file, csv, output),
    }
}

fn export_terrain_provenance_csv(file: &Path, output: Option<&Path>) -> eyre::Result<()> {
    let package = load_tex_land_ec_package(file)?;
    let output = output.map(Path::to_path_buf).unwrap_or_else(|| {
        file.with_file_name(format!(
            "{}.terrain_provenance.csv",
            file.file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("tex_land_ec")
        ))
    });
    write_tex_land_ec_terrain_provenance_csv(&output, package.terrain_provenance())?;
    println!(
        "Wrote tex_land_ec terrain provenance CSV with {} rows to '{}'.",
        package.terrain_provenance().len(),
        output.display()
    );
    Ok(())
}

fn import_terrain_provenance_csv(
    file: &Path,
    csv: &Path,
    output: Option<&Path>,
) -> eyre::Result<()> {
    let package = load_tex_land_ec_package(file)?;
    let records = read_tex_land_ec_terrain_provenance_csv(csv)?;
    validate_tex_land_ec_terrain_provenance(&package, &records)?;
    let manifest = encode_terrain_provenance_manifest(&records)?;
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| derived_output_path(file, "csv-imported"));
    package_edit::replace_virtual_path_file(
        file,
        &output,
        UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH,
        &manifest,
    )?;
    println!(
        "Imported {} tex_land_ec terrain provenance rows from '{}' into '{}'.",
        records.len(),
        csv.display(),
        output.display()
    );
    Ok(())
}

fn export_slots_csv_auto(file: &Path, output: Option<&Path>) -> eyre::Result<()> {
    if let Ok(package) = TexArtCcPackage::load(file) {
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_slots_output(file));
        write_atlas_slot_csv(&output, &cc_slot_rows(&package))?;
        println!("Wrote tex_art_cc slot CSV to '{}'.", output.display());
        return Ok(());
    }
    if let Ok(package) = TexArtEcPackage::load(file) {
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_slots_output(file));
        write_atlas_slot_csv(&output, &tex_art_ec_slot_rows(&package))?;
        println!("Wrote tex_art_ec slot CSV to '{}'.", output.display());
        return Ok(());
    }
    if let Ok(package) = TexLandEcPackage::load(file) {
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_slots_output(file));
        write_atlas_slot_csv(&output, &tex_land_ec_slot_rows(&package))?;
        println!("Wrote tex_land_ec slot CSV to '{}'.", output.display());
        return Ok(());
    }

    eyre::bail!(
        "{} is not a supported atlas package for slot CSV export",
        file.display()
    )
}

fn import_slots_csv_auto(file: &Path, csv: &Path, output: Option<&Path>) -> eyre::Result<()> {
    if let Ok(package) = TexArtCcPackage::load(file) {
        let rows = read_atlas_slot_csv(csv)?;
        validate_atlas_slot_rows(
            "tex_art_cc",
            &cc_slot_rows(&package),
            &cc_page_bounds(&package),
            &rows,
        )?;
        let mut slots = package.slots().to_vec();
        for row in &rows {
            let existing = slots[row.art_id as usize];
            slots[row.art_id as usize] = TexArtCcSlotRecord {
                art_id: row.art_id,
                page_index: row.page_index,
                page_tile_index: row.page_tile_index,
                flags: existing.flags,
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            };
        }
        let manifest = encode_cc_slot_manifest(
            &slots,
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
        )?;
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| derived_output_path(file, "csv-imported"));
        package_edit::replace_virtual_path_file(
            file,
            &output,
            CC_SLOT_MANIFEST_ENTRY_PATH,
            &manifest,
        )?;
        println!(
            "Imported {} tex_art_cc slot rows into '{}'.",
            rows.len(),
            output.display()
        );
        return Ok(());
    }
    if let Ok(package) = TexArtEcPackage::load(file) {
        let rows = read_atlas_slot_csv(csv)?;
        validate_atlas_slot_rows(
            "tex_art_ec",
            &tex_art_ec_slot_rows(&package),
            &tex_art_ec_page_bounds(&package),
            &rows,
        )?;
        let mut slots = package.slots().to_vec();
        for row in &rows {
            let existing = slots[row.art_id as usize];
            slots[row.art_id as usize] = TexArtEcSlotRecord {
                art_id: row.art_id,
                page_index: row.page_index,
                page_tile_index: row.page_tile_index,
                flags: existing.flags,
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            };
        }
        let manifest = encode_tex_art_ec_slot_manifest(
            &slots,
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
        )?;
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| derived_output_path(file, "csv-imported"));
        package_edit::replace_virtual_path_file(
            file,
            &output,
            TEX_ART_EC_SLOT_MANIFEST_ENTRY_PATH,
            &manifest,
        )?;
        println!(
            "Imported {} tex_art_ec slot rows into '{}'.",
            rows.len(),
            output.display()
        );
        return Ok(());
    }
    if let Ok(package) = TexLandEcPackage::load(file) {
        let rows = read_atlas_slot_csv(csv)?;
        validate_atlas_slot_rows(
            "tex_land_ec",
            &tex_land_ec_slot_rows(&package),
            &tex_land_ec_page_bounds(&package),
            &rows,
        )?;
        let mut slots = package.slots().to_vec();
        for row in &rows {
            let existing = slots[row.art_id as usize];
            slots[row.art_id as usize] = TexLandEcSlotRecord {
                art_id: row.art_id,
                page_index: row.page_index,
                page_tile_index: row.page_tile_index,
                flags: existing.flags,
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            };
        }
        let manifest = encode_tex_land_ec_slot_manifest(
            &slots,
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
        )?;
        let output = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| derived_output_path(file, "csv-imported"));
        package_edit::replace_virtual_path_file(
            file,
            &output,
            TEX_LAND_EC_SLOT_MANIFEST_ENTRY_PATH,
            &manifest,
        )?;
        println!(
            "Imported {} tex_land_ec slot rows into '{}'.",
            rows.len(),
            output.display()
        );
        return Ok(());
    }

    eyre::bail!(
        "{} is not a supported atlas package for slot CSV import",
        file.display()
    )
}

pub fn diff_paths(left: &Path, right: &Path, kind: DiffKind) -> eyre::Result<()> {
    let left_is_csv = left.extension().and_then(|value| value.to_str()) == Some("csv");
    let right_is_csv = right.extension().and_then(|value| value.to_str()) == Some("csv");

    if left_is_csv && right_is_csv {
        return match kind {
            DiffKind::Auto => match detect_csv_kind(left)? {
                CsvKind::TerrainProvenance => diff_terrain_provenance_csv(left, right),
                CsvKind::Slots | CsvKind::Auto => diff_slot_csv(left, right),
            },
            DiffKind::TerrainProvenance => diff_terrain_provenance_csv(left, right),
            DiffKind::Slots => diff_slot_csv(left, right),
            DiffKind::Package => eyre::bail!("package diff cannot be used with CSV inputs"),
        };
    }

    match kind {
        DiffKind::TerrainProvenance => diff_terrain_provenance_packages(left, right),
        DiffKind::Slots => diff_slot_packages(left, right),
        DiffKind::Package => diff_package_payloads(left, right),
        DiffKind::Auto => {
            if TexLandEcPackage::load(left).is_ok() && TexLandEcPackage::load(right).is_ok() {
                diff_terrain_provenance_packages(left, right)
            } else if is_atlas_package(left) && is_atlas_package(right) {
                diff_slot_packages(left, right)
            } else {
                diff_package_payloads(left, right)
            }
        }
    }
}

fn diff_terrain_provenance_csv(left: &Path, right: &Path) -> eyre::Result<()> {
    let left_rows = read_tex_land_ec_terrain_provenance_csv(left)?
        .into_iter()
        .map(terrain_record_key)
        .collect::<BTreeSet<_>>();
    let right_rows = read_tex_land_ec_terrain_provenance_csv(right)?
        .into_iter()
        .map(terrain_record_key)
        .collect::<BTreeSet<_>>();
    print_set_diff("terrain provenance rows", &left_rows, &right_rows);
    Ok(())
}

fn diff_slot_csv(left: &Path, right: &Path) -> eyre::Result<()> {
    let left_rows = read_atlas_slot_csv(left)?
        .into_iter()
        .map(slot_row_key)
        .collect::<BTreeSet<_>>();
    let right_rows = read_atlas_slot_csv(right)?
        .into_iter()
        .map(slot_row_key)
        .collect::<BTreeSet<_>>();
    print_set_diff("atlas slot rows", &left_rows, &right_rows);
    Ok(())
}

fn diff_terrain_provenance_packages(left: &Path, right: &Path) -> eyre::Result<()> {
    let left_package = load_tex_land_ec_package(left)?;
    let right_package = load_tex_land_ec_package(right)?;
    let left_rows = left_package
        .terrain_provenance()
        .iter()
        .copied()
        .map(terrain_record_key)
        .collect::<BTreeSet<_>>();
    let right_rows = right_package
        .terrain_provenance()
        .iter()
        .copied()
        .map(terrain_record_key)
        .collect::<BTreeSet<_>>();
    println!(
        "Left: rows={}, populated_slots={} | Right: rows={}, populated_slots={}",
        left_package.terrain_provenance().len(),
        left_package
            .slots()
            .iter()
            .filter(|slot| slot.is_present())
            .count(),
        right_package.terrain_provenance().len(),
        right_package
            .slots()
            .iter()
            .filter(|slot| slot.is_present())
            .count(),
    );
    print_set_diff("terrain provenance rows", &left_rows, &right_rows);
    Ok(())
}

fn diff_slot_packages(left: &Path, right: &Path) -> eyre::Result<()> {
    if let (Ok(left_package), Ok(right_package)) =
        (TexArtCcPackage::load(left), TexArtCcPackage::load(right))
    {
        let left_rows = cc_slot_rows(&left_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        let right_rows = cc_slot_rows(&right_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        print_set_diff("tex_art_cc slot rows", &left_rows, &right_rows);
        return Ok(());
    }
    if let (Ok(left_package), Ok(right_package)) =
        (TexArtEcPackage::load(left), TexArtEcPackage::load(right))
    {
        let left_rows = tex_art_ec_slot_rows(&left_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        let right_rows = tex_art_ec_slot_rows(&right_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        print_set_diff("tex_art_ec slot rows", &left_rows, &right_rows);
        return Ok(());
    }
    if let (Ok(left_package), Ok(right_package)) =
        (TexLandEcPackage::load(left), TexLandEcPackage::load(right))
    {
        let left_rows = tex_land_ec_slot_rows(&left_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        let right_rows = tex_land_ec_slot_rows(&right_package)
            .into_iter()
            .map(slot_row_key)
            .collect::<BTreeSet<_>>();
        print_set_diff("tex_land_ec slot rows", &left_rows, &right_rows);
        return Ok(());
    }

    eyre::bail!("slot diff requires matching atlas package types on both sides")
}

fn diff_package_payloads(left: &Path, right: &Path) -> eyre::Result<()> {
    let left_reader = UddpReader::load(left).wrap_err_with(|| format!("load {}", left.display()))?;
    let right_reader = UddpReader::load(right).wrap_err_with(|| format!("load {}", right.display()))?;
    println!(
        "Left: lookup_mode={:?}, files={} | Right: lookup_mode={:?}, files={}",
        left_reader.lookup_mode(),
        left_reader.records().len(),
        right_reader.lookup_mode(),
        right_reader.records().len(),
    );
    let left_files = package_file_fingerprints(&left_reader)?;
    let right_files = package_file_fingerprints(&right_reader)?;
    print_map_diff("package payloads", &left_files, &right_files);
    Ok(())
}

fn detect_csv_kind(path: &Path) -> eyre::Result<CsvKind> {
    let mut reader = ReaderBuilder::new()
        .trim(Trim::All)
        .from_path(path)
        .wrap_err_with(|| format!("read {}", path.display()))?;
    let headers = reader.headers()?.clone();
    if headers.iter().any(|header| header == "material_id") {
        return Ok(CsvKind::TerrainProvenance);
    }
    if headers.iter().any(|header| header == "art_id") {
        return Ok(CsvKind::Slots);
    }
    Ok(CsvKind::Auto)
}

fn is_atlas_package(path: &Path) -> bool {
    TexArtCcPackage::load(path).is_ok()
        || TexArtEcPackage::load(path).is_ok()
        || TexLandEcPackage::load(path).is_ok()
}

fn load_tex_land_ec_package(file: &Path) -> eyre::Result<TexLandEcPackage> {
    TexLandEcPackage::load(file)
        .wrap_err_with(|| format!("{} is not a supported tex_land_ec package", file.display()))
}

fn default_slots_output(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    file.with_file_name(format!("{stem}.slots.csv"))
}

fn derived_output_path(file: &Path, suffix: &str) -> PathBuf {
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    match file.extension().and_then(|value| value.to_str()) {
        Some(ext) => file.with_file_name(format!("{stem}.{suffix}.{ext}")),
        None => file.with_file_name(format!("{stem}.{suffix}")),
    }
}

fn cc_slot_rows(package: &TexArtCcPackage) -> Vec<AtlasSlotCsvRow> {
    package
        .slots()
        .iter()
        .copied()
        .filter(|slot| slot.is_present())
        .map(|slot| AtlasSlotCsvRow {
            art_id: slot.art_id,
            kind: if slot.is_land() {
                SlotKind::Land
            } else {
                SlotKind::Static
            },
            page_index: slot.page_index,
            page_tile_index: slot.page_tile_index,
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        })
        .collect()
}

fn tex_art_ec_slot_rows(package: &TexArtEcPackage) -> Vec<AtlasSlotCsvRow> {
    package
        .slots()
        .iter()
        .copied()
        .filter(|slot| slot.is_present())
        .map(|slot| AtlasSlotCsvRow {
            art_id: slot.art_id,
            kind: if slot.is_land() {
                SlotKind::Land
            } else {
                SlotKind::Static
            },
            page_index: slot.page_index,
            page_tile_index: slot.page_tile_index,
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        })
        .collect()
}

fn tex_land_ec_slot_rows(package: &TexLandEcPackage) -> Vec<AtlasSlotCsvRow> {
    package
        .slots()
        .iter()
        .copied()
        .filter(|slot| slot.is_present())
        .map(|slot| AtlasSlotCsvRow {
            art_id: slot.art_id,
            kind: SlotKind::Land,
            page_index: slot.page_index,
            page_tile_index: slot.page_tile_index,
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        })
        .collect()
}

fn cc_page_bounds(package: &TexArtCcPackage) -> HashMap<u32, AtlasPageBounds> {
    package
        .pages()
        .iter()
        .map(|page| {
            (
                page.page_index,
                AtlasPageBounds {
                    tile_count: page.tile_count,
                    used_width: page.used_width,
                    used_height: page.used_height,
                },
            )
        })
        .collect()
}

fn tex_art_ec_page_bounds(package: &TexArtEcPackage) -> HashMap<u32, AtlasPageBounds> {
    package
        .pages()
        .iter()
        .map(|page| {
            (
                page.page_index,
                AtlasPageBounds {
                    tile_count: page.tile_count,
                    used_width: page.used_width,
                    used_height: page.used_height,
                },
            )
        })
        .collect()
}

fn tex_land_ec_page_bounds(package: &TexLandEcPackage) -> HashMap<u32, AtlasPageBounds> {
    package
        .pages()
        .iter()
        .map(|page| {
            (
                page.page_index,
                AtlasPageBounds {
                    tile_count: page.tile_count,
                    used_width: page.used_width,
                    used_height: page.used_height,
                },
            )
        })
        .collect()
}

fn write_atlas_slot_csv(path: &Path, rows: &[AtlasSlotCsvRow]) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| format!("create {}", parent.display()))?;
    }
    let mut writer = WriterBuilder::new()
        .has_headers(true)
        .from_path(path)
        .wrap_err_with(|| format!("write {}", path.display()))?;
    writer.write_record([
        "art_id",
        "kind",
        "page_index",
        "page_tile_index",
        "x",
        "y",
        "width",
        "height",
    ])?;
    for row in rows {
        writer.write_record([
            row.art_id.to_string(),
            row.kind.as_str().to_string(),
            row.page_index.to_string(),
            row.page_tile_index.to_string(),
            row.x.to_string(),
            row.y.to_string(),
            row.width.to_string(),
            row.height.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn read_atlas_slot_csv(path: &Path) -> eyre::Result<Vec<AtlasSlotCsvRow>> {
    let mut reader = ReaderBuilder::new()
        .trim(Trim::All)
        .from_path(path)
        .wrap_err_with(|| format!("read {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let art_id = csv_header_index(&headers, "art_id")?;
    let kind = csv_header_index(&headers, "kind")?;
    let page_index = csv_header_index(&headers, "page_index")?;
    let page_tile_index = csv_header_index(&headers, "page_tile_index")?;
    let x = csv_header_index(&headers, "x")?;
    let y = csv_header_index(&headers, "y")?;
    let width = csv_header_index(&headers, "width")?;
    let height = csv_header_index(&headers, "height")?;

    let mut rows = Vec::new();
    for (row_index, row) in reader.records().enumerate() {
        let row = row?;
        let line_number = row_index + 2;
        rows.push(AtlasSlotCsvRow {
            art_id: parse_u32_field(&row, art_id, "art_id", line_number)?,
            kind: SlotKind::parse(
                row.get(kind)
                    .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field 'kind'"))?,
                line_number,
            )?,
            page_index: parse_u32_field(&row, page_index, "page_index", line_number)?,
            page_tile_index: parse_u16_field(
                &row,
                page_tile_index,
                "page_tile_index",
                line_number,
            )?,
            x: parse_u16_field(&row, x, "x", line_number)?,
            y: parse_u16_field(&row, y, "y", line_number)?,
            width: parse_u16_field(&row, width, "width", line_number)?,
            height: parse_u16_field(&row, height, "height", line_number)?,
        });
    }
    rows.sort();
    Ok(rows)
}

fn validate_atlas_slot_rows(
    package_name: &str,
    current_rows: &[AtlasSlotCsvRow],
    pages: &HashMap<u32, AtlasPageBounds>,
    imported_rows: &[AtlasSlotCsvRow],
) -> eyre::Result<()> {
    if imported_rows.len() != current_rows.len() {
        eyre::bail!(
            "{package_name} slot CSV row count changed from {} to {}. Adding or removing present slots is not allowed.",
            current_rows.len(),
            imported_rows.len()
        );
    }

    let current_by_id = current_rows
        .iter()
        .copied()
        .map(|row| (row.art_id, row))
        .collect::<HashMap<_, _>>();
    let imported_ids = imported_rows
        .iter()
        .map(|row| row.art_id)
        .collect::<HashSet<_>>();
    let current_ids = current_rows
        .iter()
        .map(|row| row.art_id)
        .collect::<HashSet<_>>();
    if imported_ids != current_ids {
        eyre::bail!("{package_name} slot CSV must keep the exact set of present art_id rows");
    }

    let mut seen = HashSet::new();
    for row in imported_rows {
        let current = current_by_id.get(&row.art_id).ok_or_else(|| {
            eyre::eyre!(
                "{package_name} slot CSV art_id {} is not present in the package",
                row.art_id
            )
        })?;
        if !seen.insert(row.art_id) {
            eyre::bail!(
                "{package_name} slot CSV contains duplicate art_id {}",
                row.art_id
            );
        }
        if row.kind != current.kind {
            eyre::bail!(
                "{package_name} slot CSV cannot change art_id {} kind from {} to {}",
                row.art_id,
                current.kind.as_str(),
                row.kind.as_str()
            );
        }
        if row.width == 0 || row.height == 0 {
            eyre::bail!(
                "{package_name} slot CSV art_id {} must keep non-zero width and height",
                row.art_id
            );
        }
        let page = pages.get(&row.page_index).ok_or_else(|| {
            eyre::eyre!(
                "{package_name} slot CSV art_id {} targets missing page {}",
                row.art_id,
                row.page_index
            )
        })?;
        if u32::from(row.page_tile_index) >= page.tile_count {
            eyre::bail!(
                "{package_name} slot CSV art_id {} uses page_tile_index {} outside page {} tile_count {}",
                row.art_id,
                row.page_tile_index,
                row.page_index,
                page.tile_count
            );
        }
        if u32::from(row.x) + u32::from(row.width) > page.used_width
            || u32::from(row.y) + u32::from(row.height) > page.used_height
        {
            eyre::bail!(
                "{package_name} slot CSV art_id {} rectangle ({},{} {}x{}) exceeds page {} used bounds {}x{}",
                row.art_id,
                row.x,
                row.y,
                row.width,
                row.height,
                row.page_index,
                page.used_width,
                page.used_height
            );
        }
    }

    Ok(())
}

fn write_tex_land_ec_terrain_provenance_csv(
    path: &Path,
    records: &[TexLandEcTerrainProvenanceRecord],
) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| format!("create {}", parent.display()))?;
    }

    let mut writer = WriterBuilder::new()
        .has_headers(true)
        .from_path(path)
        .wrap_err_with(|| format!("write {}", path.display()))?;
    writer.write_record([
        "material_id",
        "material_name_id",
        "alias_count_index",
        "alias_slot_id",
        "alias_tile_flags",
        "selected_texture_id",
        "canonical_slot_id",
        "selected_layer_index",
        "selected_texture_repetition",
        "primary_texture_id",
        "primary_layer_index",
        "primary_selection_reason",
        "primary_selection_flags",
    ])?;
    for record in records {
        writer.write_record([
            record.material_id.to_string(),
            record.material_name_id.to_string(),
            record.alias_count_index.to_string(),
            record.alias_slot_id.to_string(),
            record.alias_tile_flags.to_string(),
            optional_u32_string(record.selected_texture_id, MISSING_TEXTURE_ID),
            optional_u32_string(record.canonical_slot_id, MISSING_SLOT_ID),
            optional_u32_string(record.selected_layer_index, MISSING_TERRAIN_LAYER_INDEX),
            record.selected_texture_repetition.to_string(),
            optional_u32_string(record.primary_texture_id, MISSING_TEXTURE_ID),
            optional_u32_string(record.primary_layer_index, MISSING_TERRAIN_LAYER_INDEX),
            record.primary_selection_reason.to_string(),
            record.primary_selection_flags.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn read_tex_land_ec_terrain_provenance_csv(
    path: &Path,
) -> eyre::Result<Vec<TexLandEcTerrainProvenanceRecord>> {
    let mut reader = ReaderBuilder::new()
        .trim(Trim::All)
        .from_path(path)
        .wrap_err_with(|| format!("read {}", path.display()))?;
    let headers = reader.headers()?.clone();

    let material_id = csv_header_index(&headers, "material_id")?;
    let material_name_id = csv_header_index(&headers, "material_name_id")?;
    let alias_count_index = csv_header_index(&headers, "alias_count_index")?;
    let alias_slot_id = csv_header_index(&headers, "alias_slot_id")?;
    let alias_tile_flags = csv_header_index(&headers, "alias_tile_flags")?;
    let selected_texture_id = csv_header_index(&headers, "selected_texture_id")?;
    let canonical_slot_id = csv_header_index(&headers, "canonical_slot_id")?;
    let selected_layer_index = optional_csv_header_index(&headers, "selected_layer_index");
    let selected_texture_repetition =
        optional_csv_header_index(&headers, "selected_texture_repetition");
    let primary_texture_id = optional_csv_header_index(&headers, "primary_texture_id");
    let primary_layer_index = optional_csv_header_index(&headers, "primary_layer_index");
    let primary_selection_reason = optional_csv_header_index(&headers, "primary_selection_reason");
    let primary_selection_flags = optional_csv_header_index(&headers, "primary_selection_flags");

    let mut records = Vec::new();
    for (row_index, row) in reader.records().enumerate() {
        let row = row?;
        let line_number = row_index + 2;
        records.push(TexLandEcTerrainProvenanceRecord {
            material_id: parse_u32_field(&row, material_id, "material_id", line_number)?,
            material_name_id: parse_i32_field(
                &row,
                material_name_id,
                "material_name_id",
                line_number,
            )?,
            alias_count_index: parse_u32_field(
                &row,
                alias_count_index,
                "alias_count_index",
                line_number,
            )?,
            alias_slot_id: parse_u32_field(&row, alias_slot_id, "alias_slot_id", line_number)?,
            alias_tile_flags: parse_u64_field(
                &row,
                alias_tile_flags,
                "alias_tile_flags",
                line_number,
            )?,
            selected_texture_id: parse_optional_u32_field(
                &row,
                selected_texture_id,
                "selected_texture_id",
                line_number,
                MISSING_TEXTURE_ID,
            )?,
            canonical_slot_id: parse_optional_u32_field(
                &row,
                canonical_slot_id,
                "canonical_slot_id",
                line_number,
                MISSING_SLOT_ID,
            )?,
            selected_layer_index: parse_optional_u32_field_or_default(
                &row,
                selected_layer_index,
                "selected_layer_index",
                line_number,
                MISSING_TERRAIN_LAYER_INDEX,
            )?,
            selected_texture_repetition: parse_f32_field_or_default(
                &row,
                selected_texture_repetition,
                "selected_texture_repetition",
                line_number,
                0.0,
            )?,
            primary_texture_id: parse_optional_u32_field_or_default(
                &row,
                primary_texture_id,
                "primary_texture_id",
                line_number,
                MISSING_TEXTURE_ID,
            )?,
            primary_layer_index: parse_optional_u32_field_or_default(
                &row,
                primary_layer_index,
                "primary_layer_index",
                line_number,
                MISSING_TERRAIN_LAYER_INDEX,
            )?,
            primary_selection_reason: parse_u8_field_or_default(
                &row,
                primary_selection_reason,
                "primary_selection_reason",
                line_number,
                0,
            )?,
            primary_selection_flags: parse_u16_field_or_default(
                &row,
                primary_selection_flags,
                "primary_selection_flags",
                line_number,
                0,
            )?,
        });
    }
    records.sort_by_key(|record| {
        (
            record.alias_slot_id,
            record.material_id,
            record.alias_count_index,
        )
    });
    Ok(records)
}

fn validate_tex_land_ec_terrain_provenance(
    package: &TexLandEcPackage,
    records: &[TexLandEcTerrainProvenanceRecord],
) -> eyre::Result<()> {
    let current_records = package.terrain_provenance();
    if records.len() != current_records.len() {
        eyre::bail!(
            "tex_land_ec terrain provenance row count changed from {} to {}. Adding or removing provenance rows is not allowed.",
            current_records.len(),
            records.len()
        );
    }

    let slot_count = package.slots().len() as u32;
    let current_keys = current_records
        .iter()
        .map(|record| {
            (
                record.material_id,
                record.alias_slot_id,
                record.alias_count_index,
            )
        })
        .collect::<HashSet<_>>();
    let imported_keys = records
        .iter()
        .map(|record| {
            (
                record.material_id,
                record.alias_slot_id,
                record.alias_count_index,
            )
        })
        .collect::<HashSet<_>>();
    if current_keys != imported_keys {
        eyre::bail!(
            "tex_land_ec terrain provenance CSV must keep the exact set of material/alias rows"
        );
    }

    let mut seen = HashSet::new();
    let mut selection_by_alias = HashMap::<u32, (u32, u32)>::new();
    let canonical_targets = records
        .iter()
        .filter(|record| record.selected_texture_id != MISSING_TEXTURE_ID)
        .map(|record| {
            (
                record.alias_slot_id,
                record.selected_texture_id,
                record.canonical_slot_id,
            )
        })
        .collect::<HashSet<_>>();

    for record in records {
        if record.alias_slot_id >= slot_count {
            eyre::bail!(
                "alias_slot_id {} is outside tex_land_ec slot table size {}",
                record.alias_slot_id,
                slot_count
            );
        }
        if record.canonical_slot_id != MISSING_SLOT_ID && record.canonical_slot_id >= slot_count {
            eyre::bail!(
                "canonical_slot_id {} is outside tex_land_ec slot table size {}",
                record.canonical_slot_id,
                slot_count
            );
        }
        if !seen.insert((
            record.material_id,
            record.alias_slot_id,
            record.alias_count_index,
        )) {
            eyre::bail!(
                "duplicate tex_land_ec provenance row for material_id={}, alias_slot_id={}, alias_count_index={}",
                record.material_id,
                record.alias_slot_id,
                record.alias_count_index
            );
        }

        match selection_by_alias.entry(record.alias_slot_id) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                if *entry.get() != (record.selected_texture_id, record.canonical_slot_id) {
                    eyre::bail!(
                        "alias_slot_id {} has conflicting selected_texture_id/canonical_slot_id values across rows",
                        record.alias_slot_id
                    );
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((record.selected_texture_id, record.canonical_slot_id));
            }
        }

        if record.selected_texture_id == MISSING_TEXTURE_ID {
            if record.canonical_slot_id != MISSING_SLOT_ID {
                eyre::bail!(
                    "alias_slot_id {} sets canonical_slot_id without selected_texture_id",
                    record.alias_slot_id
                );
            }
            continue;
        }

        if record.canonical_slot_id == MISSING_SLOT_ID {
            eyre::bail!(
                "alias_slot_id {} must set canonical_slot_id when selected_texture_id is present",
                record.alias_slot_id
            );
        }

        if !canonical_targets.contains(&(
            record.canonical_slot_id,
            record.selected_texture_id,
            record.canonical_slot_id,
        )) {
            eyre::bail!(
                "alias_slot_id {} references canonical_slot_id {} without a matching canonical row for selected_texture_id {}",
                record.alias_slot_id,
                record.canonical_slot_id,
                record.selected_texture_id
            );
        }
    }

    Ok(())
}

fn package_file_fingerprints(package: &UddpReader) -> eyre::Result<BTreeMap<String, u64>> {
    let mut result = BTreeMap::new();
    for record in package.records() {
        let key = file_key_name(record.key, package.lookup_mode());
        let bytes = match record.key {
            FileKey::PathHash(hash) => package.read_file_by_path_hash(hash)?,
            FileKey::Id(id) => match package.lookup_mode() {
                LookupMode::DenseId => package.read_file_by_dense_id(id)?,
                LookupMode::SparseId => package.read_file_by_sparse_id(id)?,
                LookupMode::VirtualPathHash => {
                    unreachable!("path hash packages must use path-hash keys")
                }
            },
        };
        result.insert(key, fingerprint_bytes(&bytes));
    }
    Ok(result)
}

fn file_key_name(key: FileKey, lookup_mode: LookupMode) -> String {
    match key {
        FileKey::PathHash(hash) => format!("path_hash:{hash:016x}"),
        FileKey::Id(id) => match lookup_mode {
            LookupMode::DenseId => format!("dense_id:{id}"),
            LookupMode::SparseId => format!("sparse_id:{id}"),
            LookupMode::VirtualPathHash => {
                unreachable!("path hash packages must use path-hash keys")
            }
        },
    }
}

fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn terrain_record_key(record: TexLandEcTerrainProvenanceRecord) -> String {
    format!(
        "material_id={}|material_name_id={}|alias_count_index={}|alias_slot_id={}|alias_tile_flags={}|selected_texture_id={}|canonical_slot_id={}",
        record.material_id,
        record.material_name_id,
        record.alias_count_index,
        record.alias_slot_id,
        record.alias_tile_flags,
        optional_u32_string(record.selected_texture_id, MISSING_TEXTURE_ID),
        optional_u32_string(record.canonical_slot_id, MISSING_SLOT_ID),
    )
}

fn slot_row_key(row: AtlasSlotCsvRow) -> String {
    format!(
        "art_id={}|kind={}|page_index={}|page_tile_index={}|x={}|y={}|width={}|height={}",
        row.art_id,
        row.kind.as_str(),
        row.page_index,
        row.page_tile_index,
        row.x,
        row.y,
        row.width,
        row.height,
    )
}

fn print_set_diff<T>(label: &str, left: &BTreeSet<T>, right: &BTreeSet<T>)
where
    T: Ord + std::fmt::Display,
{
    let only_left = left.difference(right).collect::<Vec<_>>();
    let only_right = right.difference(left).collect::<Vec<_>>();
    println!(
        "{}: left_only={}, right_only={}",
        label,
        only_left.len(),
        only_right.len()
    );
    for value in only_left.iter().take(20) {
        println!("  - {}", value);
    }
    for value in only_right.iter().take(20) {
        println!("  + {}", value);
    }
}

fn print_map_diff(label: &str, left: &BTreeMap<String, u64>, right: &BTreeMap<String, u64>) {
    let left_keys = left.keys().cloned().collect::<BTreeSet<_>>();
    let right_keys = right.keys().cloned().collect::<BTreeSet<_>>();
    let mut changed = Vec::new();
    for key in left_keys.intersection(&right_keys) {
        if left.get(key) != right.get(key) {
            changed.push(key.clone());
        }
    }

    println!(
        "{}: added={}, removed={}, changed={}",
        label,
        right_keys.difference(&left_keys).count(),
        left_keys.difference(&right_keys).count(),
        changed.len()
    );
    for key in changed.iter().take(20) {
        println!("  * {}", key);
    }
    for key in left_keys.difference(&right_keys).take(20) {
        println!("  - {}", key);
    }
    for key in right_keys.difference(&left_keys).take(20) {
        println!("  + {}", key);
    }
}

fn csv_header_index(headers: &StringRecord, name: &str) -> eyre::Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .ok_or_else(|| eyre::eyre!("missing CSV header '{name}'"))
}

fn optional_csv_header_index(headers: &StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|header| header == name)
}

fn parse_u32_field(
    row: &StringRecord,
    index: usize,
    field_name: &str,
    line_number: usize,
) -> eyre::Result<u32> {
    row.get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?
        .parse::<u32>()
        .map_err(|error| eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}"))
}

fn parse_u16_field(
    row: &StringRecord,
    index: usize,
    field_name: &str,
    line_number: usize,
) -> eyre::Result<u16> {
    row.get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?
        .parse::<u16>()
        .map_err(|error| eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}"))
}

fn parse_u8_field_or_default(
    row: &StringRecord,
    index: Option<usize>,
    field_name: &str,
    line_number: usize,
    default: u8,
) -> eyre::Result<u8> {
    let Some(index) = index else {
        return Ok(default);
    };
    let value = row
        .get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?;
    if value.is_empty() {
        Ok(default)
    } else {
        value.parse::<u8>().map_err(|error| {
            eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}")
        })
    }
}

fn parse_u16_field_or_default(
    row: &StringRecord,
    index: Option<usize>,
    field_name: &str,
    line_number: usize,
    default: u16,
) -> eyre::Result<u16> {
    let Some(index) = index else {
        return Ok(default);
    };
    let value = row
        .get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?;
    if value.is_empty() {
        Ok(default)
    } else {
        value.parse::<u16>().map_err(|error| {
            eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}")
        })
    }
}

fn parse_f32_field_or_default(
    row: &StringRecord,
    index: Option<usize>,
    field_name: &str,
    line_number: usize,
    default: f32,
) -> eyre::Result<f32> {
    let Some(index) = index else {
        return Ok(default);
    };
    let value = row
        .get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?;
    if value.is_empty() {
        Ok(default)
    } else {
        value.parse::<f32>().map_err(|error| {
            eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}")
        })
    }
}

fn parse_i32_field(
    row: &StringRecord,
    index: usize,
    field_name: &str,
    line_number: usize,
) -> eyre::Result<i32> {
    row.get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?
        .parse::<i32>()
        .map_err(|error| eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}"))
}

fn parse_u64_field(
    row: &StringRecord,
    index: usize,
    field_name: &str,
    line_number: usize,
) -> eyre::Result<u64> {
    row.get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?
        .parse::<u64>()
        .map_err(|error| eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}"))
}

fn parse_optional_u32_field(
    row: &StringRecord,
    index: usize,
    field_name: &str,
    line_number: usize,
    missing: u32,
) -> eyre::Result<u32> {
    let value = row
        .get(index)
        .ok_or_else(|| eyre::eyre!("CSV line {line_number} missing field '{field_name}'"))?;
    if value.is_empty() {
        Ok(missing)
    } else {
        value.parse::<u32>().map_err(|error| {
            eyre::eyre!("CSV line {line_number} has invalid {field_name}: {error}")
        })
    }
}

fn parse_optional_u32_field_or_default(
    row: &StringRecord,
    index: Option<usize>,
    field_name: &str,
    line_number: usize,
    missing: u32,
) -> eyre::Result<u32> {
    let Some(index) = index else {
        return Ok(missing);
    };
    parse_optional_u32_field(row, index, field_name, line_number, missing)
}

fn optional_u32_string(value: u32, missing: u32) -> String {
    if value == missing {
        String::new()
    } else {
        value.to_string()
    }
}
