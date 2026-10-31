//! The shipped client executables are the traced stock builds, a patched copy differs from its executable only in
//! the facet record, the split imm32 and the PE checksum, and the shipped facet data agrees with the stock tables.
//!
//! The executables come from `UO_EC_CLIENT_DIR` (`UOSA.exe`, `UOSA_TC.exe`, `UOSA_eclo.exe`), the optional
//! `UO_EC_FP_CLIENT_DIR` (the same names) and `UO_KR_CLIENT_DIR` (`UOKR.exe`); they are only read. The copies go
//! to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted afterwards.

mod common;

use std::path::{Path, PathBuf};

use uocf::client_exe::facet_data::{check_facet_data, FacetDataStatus};
use uocf::client_exe::facet_dimensions::*;
use uocf::client_exe::sha256::sha256_hex;
use uocf::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS};

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const EC_FP_CLIENT_DIR_ENV: &str = "UO_EC_FP_CLIENT_DIR";
const EC_EXE_NAMES: [&str; 3] = ["UOSA.exe", "UOSA_TC.exe", "UOSA_eclo.exe"];
const KR_EXE_NAME: &str = "UOKR.exe";
/// The build that ships a wrong PE checksum.
const WRONG_CHECKSUM_EXE: &str = "UOSA_eclo.exe";
const PATCHED_FACET: u8 = 0;
const PATCHED_SIZE: FacetGeometry = FacetGeometry::new(8192, 4160);
const PATCHED_SPLIT: u32 = 6144;
/// The second patch of a chain, on the copy.
const CHAINED_FACET: u8 = 6;
const CHAINED_SIZE: FacetGeometry = FacetGeometry::new(512, 512);

fn env_dir(env: &str) -> Option<PathBuf> {
    std::env::var_os(env).map(PathBuf::from)
}

/// Every shipped executable of the client directories.
fn client_exes() -> Vec<PathBuf> {
    let mut exes = Vec::new();
    for env in [EC_CLIENT_DIR_ENV, EC_FP_CLIENT_DIR_ENV] {
        if let Some(dir) = env_dir(env) {
            exes.extend(EC_EXE_NAMES.iter().map(|name| dir.join(name)).filter(|path| path.is_file()));
        }
    }
    if let Some(dir) = env_dir(common::KR_CLIENT_DIR_ENV) {
        exes.push(dir.join(KR_EXE_NAME));
    }
    assert!(!exes.is_empty(), "set {EC_CLIENT_DIR_ENV}, {EC_FP_CLIENT_DIR_ENV} and/or {}", common::KR_CLIENT_DIR_ENV);
    exes
}

fn inspect(path: &Path) -> ClientExeFacets {
    ClientExeFacets::inspect(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR / UO_EC_FP_CLIENT_DIR / UO_KR_CLIENT_DIR and run with --ignored"]
fn shipped_executables_are_known_stock_builds() {
    for exe in client_exes() {
        let facets = inspect(&exe);
        let build = facets.known.unwrap_or_else(|| panic!("{}: unknown build {}", exe.display(), facets.sha256));
        assert!(facets.is_stock(), "{}", exe.display());
        assert!(!facets.pe.is_signed());
        let wrong = exe.file_name().unwrap() == WRONG_CHECKSUM_EXE;
        assert_eq!(facets.checksum_valid(), !wrong, "{}: checksum", exe.display());
        println!(
            "{}: {} checksum {:#x} stored {:#x}, split {:?}",
            exe.display(),
            build.name,
            facets.computed_checksum,
            facets.pe.stored_checksum,
            facets.split.map(|split| split.x)
        );
    }
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR / UO_EC_FP_CLIENT_DIR / UO_KR_CLIENT_DIR and run with --ignored"]
fn patched_copies_change_only_the_expected_bytes() {
    for exe in client_exes() {
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
        let record = before.table_offset + FACET_RECORD_SIZE * usize::from(PATCHED_FACET);
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
        assert_eq!(after.dimensions[usize::from(PATCHED_FACET)], PATCHED_SIZE);
        assert_eq!(after.dimensions[1..], CLIENT_FACET_DIMENSIONS[1..]);
        assert_eq!(after.split.map(|split| split.x), before.split.map(|_| PATCHED_SPLIT));
        let manifest = read_manifest(&output).unwrap().unwrap();
        assert_eq!(manifest.original_sha256, before.sha256);
        assert_eq!(manifest.patched_sha256, sha256_hex(&copy));
        assert_eq!(manifest.values, after.values());

        let chained = FacetDimsPatch { facet: CHAINED_FACET, width: CHAINED_SIZE.width, height: CHAINED_SIZE.height, split: SplitEdit::Keep };
        let second_dir = common::temp_dir("facet_dims_chain");
        let second = write_patched_client_exe(&output, &second_dir.join(exe.file_name().unwrap()), &chained).unwrap();
        assert_eq!(second.manifest.original_sha256, before.sha256);
        assert_eq!(second.after.dimensions[usize::from(PATCHED_FACET)], PATCHED_SIZE);
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
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR / UO_EC_FP_CLIENT_DIR / UO_KR_CLIENT_DIR and run with --ignored"]
fn shipped_facet_data_matches_the_stock_tables() {
    for exe in client_exes() {
        let facets = inspect(&exe);
        let reports = check_facet_data(&facets.dimensions, exe.parent().unwrap()).unwrap();
        let checked = reports.iter().flat_map(|report| &report.items).filter(|item| item.status == FacetDataStatus::Ok).count();
        let mismatches: Vec<_> = reports.iter().flat_map(|report| report.mismatches()).collect();
        println!("{}: {checked} files agree", exe.display());
        assert!(mismatches.is_empty(), "{}: {mismatches:?}", exe.display());

        let mut resized = facets.dimensions;
        resized[usize::from(PATCHED_FACET)] = PATCHED_SIZE;
        let reports = check_facet_data(&resized, exe.parent().unwrap()).unwrap();
        assert!(reports[usize::from(PATCHED_FACET)].mismatches().count() > 0, "stock data does not fit a resized facet");
    }
}
