//! Every client executable under the client roots is located by structure: each Enhanced Client and Kingdom Reborn
//! build has exactly one facet table (the traced builds at their traced offsets), a patched copy differs from its
//! executable only in the facet record, the split imm32 and the PE checksum, and the shipped facet data agrees with
//! the stock tables. Other executables (Classic clients, launchers, third-party clients) are only reported.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively for `*.exe`
//! files; they are only read. The copies go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted
//! afterwards.

mod common;

use std::path::{Path, PathBuf};

use uocf::client_exe::facet_data::{check_facet_data, FacetDataStatus};
use uocf::client_exe::facet_dimensions::*;
use uocf::client_exe::pe::PeImage;
use uocf::client_exe::sha256::sha256_hex;
use uocf::enhanced::facet_sector::{facet_package_file_name, FacetGeometry, CLIENT_FACET_DIMENSIONS};

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const EXE_EXTENSION: &str = "exe";
/// Enhanced Client builds: `UOSA.exe`, `UOSA_TC.exe`, `UOSA_eclo.exe`, ...
const EC_EXE_PREFIX: &str = "uosa";
const KR_EXE_NAME: &str = "uokr.exe";
/// Larger executables are installers, not clients (the clients are at most 11 MB).
const MAX_CLIENT_EXE_SIZE: u64 = 64 << 20;
/// The build that ships a wrong PE checksum.
const WRONG_CHECKSUM_EXE: &str = "UOSA_eclo.exe";
const PATCHED_FACET: u8 = 0;
const PATCHED_SIZE: FacetGeometry = FacetGeometry::new(8192, 4160);
const PATCHED_SPLIT: u32 = 6144;
/// The second patch of a chain, on the copy.
const CHAINED_FACET: u8 = 6;
const CHAINED_SIZE: FacetGeometry = FacetGeometry::new(512, 512);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExeKind {
    Enhanced,
    KingdomReborn,
    Other,
}

fn exe_kind(path: &Path) -> ExeKind {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if name.starts_with(EC_EXE_PREFIX) {
        ExeKind::Enhanced
    } else if name == KR_EXE_NAME {
        ExeKind::KingdomReborn
    } else {
        ExeKind::Other
    }
}

fn collect_exes(dir: &Path, exes: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_exes(&path, exes);
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXE_EXTENSION)) {
            exes.push(path);
        }
    }
}

/// Every `*.exe` under the client roots.
fn client_exes() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut exes = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect_exes(&root, &mut exes);
    }
    assert!(!exes.is_empty(), "no executables under {CLIENT_ROOTS_ENV}");
    exes
}

/// The Enhanced Client and Kingdom Reborn executables under the client roots.
fn ec_kr_exes() -> Vec<PathBuf> {
    let exes: Vec<PathBuf> = client_exes().into_iter().filter(|exe| exe_kind(exe) != ExeKind::Other).collect();
    assert!(!exes.is_empty(), "no UOSA*.exe or UOKR.exe under {CLIENT_ROOTS_ENV}");
    exes
}

fn inspect(path: &Path) -> ClientExeFacets {
    ClientExeFacets::inspect(&std::fs::read(path).unwrap()).unwrap_or_else(|error| panic!("{}: {error:#}", path.display()))
}

fn describe_tables(tables: &[FacetTable]) -> String {
    let runs: Vec<String> = tables
        .iter()
        .map(|table| {
            let records: Vec<String> = table
                .records
                .iter()
                .map(|record| format!("{} {}x{}", record.name, record.geometry.width, record.geometry.height))
                .collect();
            format!("{} records in {} at {:#x} [{}]", table.records.len(), table.section, table.offset, records.join(", "))
        })
        .collect();
    format!("{} table candidates {}", tables.len(), runs.join("; "))
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_client_exe_is_surveyed_and_ec_kr_builds_have_one_table() {
    let mut located = 0;
    for exe in client_exes() {
        let kind = exe_kind(&exe);
        let size = std::fs::metadata(&exe).unwrap().len();
        if size > MAX_CLIENT_EXE_SIZE {
            assert_eq!(kind, ExeKind::Other, "{}", exe.display());
            println!("{}: {size} bytes, skipped (an installer)", exe.display());
            continue;
        }
        let bytes = std::fs::read(&exe).unwrap();
        let pe = match PeImage::parse(&bytes) {
            Ok(pe) => pe,
            Err(error) => {
                assert_eq!(kind, ExeKind::Other, "{}: {error:#}", exe.display());
                println!("{}: {error:#}", exe.display());
                continue;
            }
        };
        let tables = locate_facet_tables(&bytes, &pe);
        let splits = locate_split_candidates(&bytes, &pe).map(|splits| {
            let values: Vec<String> =
                splits.iter().map(|split| format!("{:#x} {:?}", split.signature_offset, split.values)).collect();
            format!("{} split candidates [{}]", splits.len(), values.join(", "))
        });
        println!("{}: {}; {}", exe.display(), describe_tables(&tables), splits.unwrap_or_else(|error| format!("{error:#}")));
        if kind == ExeKind::Other {
            continue;
        }
        let facets = inspect(&exe);
        assert_eq!(tables.len(), 1, "{}", exe.display());
        assert!(facets.is_stock() && !facets.pe.is_signed(), "{}", exe.display());
        assert_eq!(facets.facet_count(), CLIENT_FACET_DIMENSIONS.len(), "{}", exe.display());
        assert_eq!(facets.split.is_some(), kind == ExeKind::Enhanced, "{}: split", exe.display());
        let wrong = exe.file_name().unwrap() == WRONG_CHECKSUM_EXE;
        assert_eq!(facets.checksum_valid(), !wrong, "{}: checksum", exe.display());
        if let Some(build) = facets.known {
            assert_eq!(facets.table.offset, build.table_offset, "{}: {}", exe.display(), build.name);
            assert_eq!(facets.split.map(|split| split.offsets), build.split_offsets, "{}", exe.display());
            assert_eq!(facets.pe.checksum_offset, build.checksum_offset, "{}", exe.display());
        }
        println!("  known build: {}", facets.known.map_or("no", |build| build.name));
        located += 1;
    }
    assert!(located > 0, "no UOSA*.exe or UOKR.exe under {CLIENT_ROOTS_ENV}");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn patched_copies_change_only_the_expected_bytes() {
    for exe in ec_kr_exes() {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeFacets::inspect(&original).unwrap();
        let split = if before.split.is_some() { SplitEdit::Set(PATCHED_SPLIT) } else { SplitEdit::Keep };
        let patch = FacetDimsPatch { facet: PATCHED_FACET, width: PATCHED_SIZE.width, height: PATCHED_SIZE.height, split };
        let dir = common::temp_dir("facet_dims_patch");
        let output = dir.join(exe.file_name().unwrap());
        assert!(write_patched_client_exe(&exe, &exe, &patch).is_err(), "the input is refused as the output");
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), original, "the input is unchanged");

        let copy = std::fs::read(&output).unwrap();
        let record = before.table.offset + FACET_RECORD_SIZE * usize::from(PATCHED_FACET);
        let checksum = before.pe.checksum_offset;
        let mut expected = vec![record + size_of::<u32>()..record + FACET_RECORD_SIZE, checksum..checksum + size_of::<u32>()];
        if let Some(split) = before.split {
            expected.extend(split.offsets.map(|offset| offset..offset + size_of::<u32>()));
        }
        assert_eq!(copy.len(), original.len());
        let changed: Vec<usize> = (0..copy.len()).filter(|&offset| copy[offset] != original[offset]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|offset| expected.iter().any(|range| range.contains(offset))), "{}: {changed:x?}", exe.display());
        assert_eq!(changed, patched.changed_offsets);

        let after = ClientExeFacets::inspect(&copy).unwrap();
        assert!(after.checksum_valid() && after.known.is_none());
        assert_eq!(after.table.offset, before.table.offset);
        assert_eq!(after.dimensions()[usize::from(PATCHED_FACET)], PATCHED_SIZE);
        assert_eq!(after.dimensions()[1..], CLIENT_FACET_DIMENSIONS[1..]);
        assert_eq!(after.split.map(|split| split.x), before.split.map(|_| PATCHED_SPLIT));
        let manifest = read_manifest(&output).unwrap().unwrap();
        assert_eq!(manifest.original_sha256, before.sha256);
        assert_eq!(manifest.source_sha256, before.sha256);
        assert_eq!(manifest.patched_sha256, sha256_hex(&copy));
        assert_eq!(manifest.build, before.known.map(|build| build.name.to_owned()));
        assert_eq!(manifest.values, after.values());

        let chained = FacetDimsPatch { facet: CHAINED_FACET, width: CHAINED_SIZE.width, height: CHAINED_SIZE.height, split: SplitEdit::Keep };
        let second_dir = common::temp_dir("facet_dims_chain");
        let second = write_patched_client_exe(&output, &second_dir.join(exe.file_name().unwrap()), &chained).unwrap();
        assert_eq!(second.manifest.original_sha256, before.sha256);
        assert_eq!(second.manifest.source_sha256, manifest.patched_sha256);
        assert_eq!(second.after.dimensions()[usize::from(PATCHED_FACET)], PATCHED_SIZE);
        let orphan_dir = common::temp_dir("facet_dims_orphan");
        let orphan = orphan_dir.join(exe.file_name().unwrap());
        std::fs::copy(&output, &orphan).unwrap();
        let refused = write_patched_client_exe(&orphan, &second_dir.join("orphan.exe"), &chained);
        assert!(refused.is_err(), "a patched copy without its manifest is refused");
        println!("{}: {} bytes changed at {:x?}", exe.display(), changed.len(), changed);
        for dir in [dir, second_dir, orphan_dir] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn shipped_facet_data_matches_the_stock_tables() {
    for exe in ec_kr_exes() {
        let client_dir = exe.parent().unwrap();
        if !client_dir.join(facet_package_file_name(0, false)).is_file() {
            println!("{}: no facet data next to it, skipped", exe.display());
            continue;
        }
        let facets = inspect(&exe);
        let reports = check_facet_data(&facets.dimensions(), client_dir).unwrap();
        let checked = reports.iter().flat_map(|report| &report.items).filter(|item| item.status == FacetDataStatus::Ok).count();
        let mismatches: Vec<_> = reports.iter().flat_map(|report| report.mismatches()).collect();
        println!("{}: {checked} files agree", exe.display());
        assert!(mismatches.is_empty(), "{}: {mismatches:?}", exe.display());

        let mut resized = facets.dimensions();
        resized[usize::from(PATCHED_FACET)] = PATCHED_SIZE;
        let reports = check_facet_data(&resized, client_dir).unwrap();
        assert!(reports[usize::from(PATCHED_FACET)].mismatches().count() > 0, "stock data does not fit a resized facet");
    }
}
