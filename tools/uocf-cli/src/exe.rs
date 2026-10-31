use std::path::{Path, PathBuf};

use clap::Subcommand;
use color_eyre::eyre::{self, WrapErr};
use uocf::client_exe::facet_data::{check_facet_data, FacetDataStatus};
use uocf::client_exe::facet_dimensions::{
    manifest_path, read_manifest, write_patched_client_exe, ClientExeFacets, FacetDimsPatch, SplitEdit,
    DISABLED_SPLIT_X, FACET_NAMES, STOCK_SPLIT_X,
};
use uocf::enhanced::facet_sector::CLIENT_FACET_DIMENSIONS;

#[derive(Subcommand, Debug)]
pub enum ExeCmd {
    /// Print the facet size table, the x = 5120 Felucca/Trammel split and the PE checksum of a client executable
    /// (UOSA*.exe, UOKR.exe). Only reads the file.
    Show {
        exe: PathBuf,
    },
    /// Write a copy of a client executable with a new size for one facet (and optionally a new split), and its
    /// manifest `<output>.facet-dims.json`. The input is only read; the output must be a new file outside the
    /// input's folder. Resize the facet data too (see `check-data`).
    Patch {
        exe: PathBuf,
        /// The patched copy.
        #[arg(long)]
        output: PathBuf,
        /// Facet index 0..6.
        #[arg(long)]
        facet: u8,
        /// New width in squares (a multiple of 8, at most 32704).
        #[arg(long)]
        width: u32,
        /// New height in squares (a multiple of 8, at most 32704).
        #[arg(long)]
        height: u32,
        /// Facets 0 and 1 only: move the x = 5120 split, across which chunks are not built, to this x.
        #[arg(long, conflicts_with = "no_split")]
        split: Option<u32>,
        /// Facets 0 and 1 only: disable the split (x = 0x8000).
        #[arg(long)]
        no_split: bool,
    },
    /// Check that the facet data of a client folder (facetN.uop / facetNx.uop sectors and data.map, facets.uop,
    /// facetNN.dds) has the sizes the executable's table holds. Only reads. Fails when anything disagrees.
    CheckData {
        exe: PathBuf,
        client_dir: PathBuf,
    },
}

pub fn run(cmd: ExeCmd) -> eyre::Result<()> {
    match cmd {
        ExeCmd::Show { exe } => show(&exe),
        ExeCmd::Patch { exe, output, facet, width, height, split, no_split } => {
            let split = match (split, no_split) {
                (Some(x), _) => SplitEdit::Set(x),
                (None, true) => SplitEdit::Disable,
                (None, false) => SplitEdit::Keep,
            };
            patch(&exe, &output, &FacetDimsPatch { facet, width, height, split })
        }
        ExeCmd::CheckData { exe, client_dir } => check_data(&exe, &client_dir),
    }
}

fn inspect(exe: &Path) -> eyre::Result<ClientExeFacets> {
    let bytes = std::fs::read(exe).wrap_err_with(|| format!("read {}", exe.display()))?;
    ClientExeFacets::inspect(&bytes).wrap_err_with(|| format!("inspect {}", exe.display()))
}

fn split_label(x: u32) -> &'static str {
    match x {
        STOCK_SPLIT_X => "stock",
        DISABLED_SPLIT_X => "disabled",
        _ => "custom",
    }
}

fn print_facets(facets: &ClientExeFacets) {
    let build = facets.known.map_or("unknown build (located by signature)", |build| build.name);
    println!("sha256    {}", facets.sha256);
    println!("build     {build}");
    println!("size      {} bytes", facets.file_size);
    let state = if facets.checksum_valid() { "valid" } else { "INVALID" };
    println!(
        "checksum  {:#010x} at {:#x}, computed {:#010x}: {state}",
        facets.pe.stored_checksum, facets.pe.checksum_offset, facets.computed_checksum
    );
    let signed = if facets.pe.is_signed() { "Authenticode signed (cannot be patched)" } else { "not signed" };
    println!("signature {signed}");
    println!("table     file {:#x}, VA {:#x}{}", facets.table_offset, facets.table_va, if facets.is_stock() { " (stock)" } else { "" });
    for (facet, (geometry, name)) in facets.dimensions.iter().zip(FACET_NAMES).enumerate() {
        let stock = if *geometry == CLIENT_FACET_DIMENSIONS[facet] { "" } else { "  (changed)" };
        println!(
            "  {facet} {name:<9} {:>5} x {:<5} {:>3} x {:<3} = {:>5} sectors{stock}",
            geometry.width,
            geometry.height,
            geometry.sector_columns(),
            geometry.sector_rows(),
            geometry.sector_count()
        );
    }
    match facets.split {
        None => println!("split     none (no x = 5120 Felucca/Trammel split in this build)"),
        Some(split) => {
            let at = split
                .offsets
                .iter()
                .map(|offset| format!("{offset:#x} (VA {:#x})", facets.pe.offset_to_va(*offset).unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(", ");
            println!("split     x = {} ({}), imm32 at file {at}", split.x, split_label(split.x));
        }
    }
}

fn show(exe: &Path) -> eyre::Result<()> {
    let facets = inspect(exe)?;
    print_facets(&facets);
    if let Some(manifest) = read_manifest(exe)? {
        let current = if manifest.patched_sha256 == facets.sha256 { "matches this file" } else { "STALE: another file" };
        println!("manifest  {} ({current})", manifest_path(exe).display());
        println!("  original {}", manifest.original_sha256);
        println!("  source   {}", manifest.source_sha256);
    }
    Ok(())
}

fn patch(exe: &Path, output: &Path, patch: &FacetDimsPatch) -> eyre::Result<()> {
    let patched = write_patched_client_exe(exe, output, patch)?;
    for warning in &patched.warnings {
        println!("warning: {warning}");
    }
    print_facets(&patched.after);
    println!("changed   {} bytes at {:x?}", patched.changed_offsets.len(), patched.changed_offsets);
    println!("wrote     {}", output.display());
    println!("manifest  {}", manifest_path(output).display());
    println!(
        "Resize the facet data too: facet{f}.uop sectors and data.map, facets.uop, facet{f:02}.dds (`uocf-cli exe check-data`).",
        f = patch.facet
    );
    Ok(())
}

fn check_data(exe: &Path, client_dir: &Path) -> eyre::Result<()> {
    let facets = inspect(exe)?;
    let reports = check_facet_data(&facets.dimensions, client_dir)?;
    let mut mismatches = 0;
    for report in &reports {
        let geometry = report.geometry;
        println!(
            "facet {} {} {}x{} ({} sectors)",
            report.facet,
            FACET_NAMES[usize::from(report.facet)],
            geometry.width,
            geometry.height,
            geometry.sector_count()
        );
        for item in &report.items {
            let status = match item.status {
                FacetDataStatus::Ok => "ok",
                FacetDataStatus::Mismatch => "MISMATCH",
                FacetDataStatus::Absent => "absent",
            };
            println!("  {status:<8} {:<28} {}", item.source, item.detail);
        }
        mismatches += report.mismatches().count();
    }
    eyre::ensure!(mismatches == 0, "{mismatches} files disagree with the executable's facet table");
    Ok(())
}
