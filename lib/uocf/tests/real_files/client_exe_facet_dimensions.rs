//! Every client executable under the client roots is located by structure: each Enhanced Client and Kingdom Reborn
//! build has exactly one facet table, each Classic build one table or two agreeing copies with its startup globals,
//! wrap seam and file name table (the traced builds at their traced offsets). A multi-facet patched copy differs
//! from its executable only in the planned fields (records of every copy, split or seam imm32, Classic startup
//! globals, PE checksum), and the shipped facet data agrees with the stock tables. Other executables (launchers,
//! third-party clients) are only reported.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively for `*.exe`
//! files; they are only read. The copies go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted
//! afterwards.

use crate::common;

use std::ops::Range;
use std::path::{Path, PathBuf};

use uocf::client::classic_facets::{
    locate_classic_facet_tables, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET, CLASSIC_RECORD_WRAP_WIDTH_OFFSET,
    UNLOCATED_FILE_ID_LIMIT,
};
use uocf::client::facet_data::{check_client_facet_data, check_facet_data, FacetDataStatus};
use uocf::client::facet_dimensions::*;
use uocf::client::patch::{
    dry_run_client_exe, read_manifest, write_patched_client_exe, ExeEdit, ExePatch, SplitEdit,
};
use uocf::client::pe::PeImage;
use uocf::client::sha256::sha256_hex;
use uocf::client::version::read_exe_version;
use uocf::enhanced::facet_sector::{facet_package_file_name, FacetGeometry, CLIENT_FACET_DIMENSIONS};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const EXE_EXTENSION: &str = "exe";
/// Enhanced Client builds: `UOSA.exe`, `UOSA_TC.exe`, `UOSA_eclo.exe`, ...
const EC_EXE_PREFIX: &str = "uosa";
const KR_EXE_NAME: &str = "uokr.exe";
/// Classic builds: `client.exe`, `client_tc.exe`, and extracted copies named `<build>_client.exe`.
const CLASSIC_EXE_NAME: &str = "client.exe";
const CLASSIC_EXE_PREFIX: &str = "client_";
const CLASSIC_EXE_SUFFIX: &str = "_client.exe";
/// A Classic folder has map 0 as one of these.
const CLASSIC_MAP_0_FILES: [&str; 2] = ["map0.mul", "map0LegacyMUL.uop"];
/// Larger executables are installers, not clients (the clients are at most 11 MB).
const MAX_CLIENT_EXE_SIZE: u64 = 64 << 20;
/// The builds that ship a wrong PE checksum (`UOSA_eclo.exe`, Classic 6.0.14.3).
const WRONG_CHECKSUM_SHA256: [&str; 2] = [
    "5aaa654c517adc3aab88467f8bc869e9b048c78c0c195caf085723fecbb8fdf0",
    "9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d",
];
/// The stored checksum of the builds without one (Classic up to 5.0.9.1).
const NO_CHECKSUM: u32 = 0;
/// The patch: facet 0 to 8192 x height, facet 2 to width x 1608, the split or seam to 6144.
const PATCHED_FACET: u8 = 0;
const PATCHED_WIDTH: u32 = 8192;
const EC_PATCHED_HEIGHT: u32 = 4160;
const SECOND_PATCHED_FACET: u8 = 2;
const SECOND_PATCHED_HEIGHT: u32 = 1608;
const PATCHED_SPLIT: u32 = 6144;
/// The second patch of a chain, on the copy: the last facet.
const CHAINED_SIZE: FacetGeometry = FacetGeometry::new(512, 512);
const FIELD_SIZE: usize = size_of::<u32>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExeKind {
    Enhanced,
    KingdomReborn,
    Classic,
    Other,
}

pub(super) fn exe_kind(path: &Path) -> ExeKind {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if name.starts_with(EC_EXE_PREFIX) {
        ExeKind::Enhanced
    } else if name == KR_EXE_NAME {
        ExeKind::KingdomReborn
    } else if name == CLASSIC_EXE_NAME || name.starts_with(CLASSIC_EXE_PREFIX) || name.ends_with(CLASSIC_EXE_SUFFIX) {
        ExeKind::Classic
    } else {
        ExeKind::Other
    }
}

fn family_kind(family: ClientFamily) -> ExeKind {
    match family {
        ClientFamily::Enhanced => ExeKind::Enhanced,
        ClientFamily::KingdomReborn => ExeKind::KingdomReborn,
        ClientFamily::Classic => ExeKind::Classic,
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
pub(super) fn client_exes() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut exes = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect_exes(&root, &mut exes);
    }
    assert!(!exes.is_empty(), "no executables under {CLIENT_ROOTS_ENV}");
    exes
}

/// The Enhanced Client, Kingdom Reborn and Classic executables under the client roots.
fn known_kind_exes() -> Vec<PathBuf> {
    let exes: Vec<PathBuf> = client_exes().into_iter().filter(|exe| exe_kind(exe) != ExeKind::Other).collect();
    assert!(!exes.is_empty(), "no UOSA*.exe, UOKR.exe or client*.exe under {CLIENT_ROOTS_ENV}");
    exes
}

fn inspect(path: &Path) -> ClientExeInspection {
    ClientExeInspection::inspect(&std::fs::read(path).unwrap())
        .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()))
}

fn describe_tables(tables: &[FacetTable]) -> String {
    let runs: Vec<String> = tables
        .iter()
        .map(|table| {
            let records: Vec<String> = table
                .records
                .iter()
                .map(|record| {
                    let wrap = record
                        .classic
                        .map(|classic| format!(" wrap {}x{}", classic.wrap.width, classic.wrap.height))
                        .unwrap_or_default();
                    format!("{} {}x{}{wrap}", record.name, record.geometry.width, record.geometry.height)
                })
                .collect();
            format!("{} records in {} at {:#x} [{}]", table.records.len(), table.section, table.offset, records.join(", "))
        })
        .collect();
    format!("{} table candidates {}", tables.len(), runs.join("; "))
}

/// The byte ranges a patch of `edits` on `before` may change, derived from the inspection alone.
pub(super) fn allowed_ranges(before: &ClientExeInspection, edits: &[ExeEdit]) -> Vec<Range<usize>> {
    let field = |offset: usize| offset..offset + FIELD_SIZE;
    let mut ranges = vec![field(before.pe.checksum_offset)];
    for edit in edits {
        match *edit {
            ExeEdit::FacetSize { facet, .. } | ExeEdit::ClassicWrap { facet, .. } => {
                for table in &before.tables {
                    let record = table.record_offset(usize::from(facet));
                    ranges.extend([FACET_RECORD_WIDTH_OFFSET, FACET_RECORD_HEIGHT_OFFSET].map(|at| field(record + at)));
                    if before.family == ClientFamily::Classic {
                        let wraps = [CLASSIC_RECORD_WRAP_WIDTH_OFFSET, CLASSIC_RECORD_WRAP_HEIGHT_OFFSET];
                        ranges.extend(wraps.map(|at| field(record + at)));
                    }
                }
                if facet == 0
                    && let Some(startup) = before.classic.as_ref().and_then(|layout| layout.startup)
                {
                    ranges.extend(startup.field_offsets().map(field));
                }
            }
            ExeEdit::Split(_) => ranges.extend(before.split.iter().flat_map(|split| split.offsets.iter().copied().map(field))),
            ExeEdit::LoginServer(_) => {
                if let Some(login) = &before.login {
                    if login.uses_fixed_address() {
                        ranges.push(field(login.fixed.ip_offset));
                        ranges.push(login.fixed.port_offset..login.fixed.port_offset + size_of::<u16>());
                    }
                    ranges.extend(login.hosts.iter().map(|slot| slot.offset..slot.offset + slot.capacity));
                    ranges.extend(login.ports.iter().map(|site| site.offset..site.offset + size_of::<u16>()));
                }
            }
        }
    }
    ranges
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_client_exe_is_surveyed_and_each_client_has_its_tables() {
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
        let classic_tables = locate_classic_facet_tables(&bytes, &pe, UNLOCATED_FILE_ID_LIMIT);
        let splits = locate_split_candidates(&bytes, &pe).map(|splits| {
            let values: Vec<String> =
                splits.iter().map(|split| format!("{:#x} {:?}", split.signature_offset, split.values)).collect();
            format!("{} split candidates [{}]", splits.len(), values.join(", "))
        });
        println!(
            "{}: {}; Classic {}; {}",
            exe.display(),
            describe_tables(&tables),
            describe_tables(&classic_tables),
            splits.unwrap_or_else(|error| format!("{error:#}"))
        );
        match read_exe_version(&bytes, &pe) {
            Some(version) => println!("  version {version}"),
            None => println!("  version none (no version resource)"),
        }
        if kind == ExeKind::Other {
            continue;
        }
        let inspection = inspect(&exe);
        assert_eq!(family_kind(inspection.family), kind, "{}: family", exe.display());
        assert!(inspection.is_stock() && !inspection.pe.is_signed(), "{}", exe.display());
        let wrong = WRONG_CHECKSUM_SHA256.contains(&inspection.sha256.as_str()) || inspection.pe.stored_checksum == NO_CHECKSUM;
        assert_eq!(inspection.checksum_valid(), !wrong, "{}: checksum", exe.display());
        match inspection.family {
            ClientFamily::Classic => {
                assert!(tables.is_empty(), "{}", exe.display());
                assert!((1..=2).contains(&inspection.tables.len()), "{}", exe.display());
                let layout = inspection.classic.as_ref().unwrap();
                let startup = layout.startup.unwrap_or_else(|| panic!("{}: no startup globals", exe.display()));
                let names = layout.file_names.as_ref().unwrap_or_else(|| panic!("{}: no file names", exe.display()));
                assert_eq!(inspection.split.as_ref().map(|split| split.x), Some(STOCK_SPLIT_X), "{}: seam", exe.display());
                println!(
                    "  Classic: {} copies of {} records at {:x?}; startup {:#x}; seam {:x?}; {} file names of {} bytes at {:#x}",
                    inspection.tables.len(),
                    inspection.facet_count(),
                    inspection.tables.iter().map(|table| table.offset).collect::<Vec<_>>(),
                    startup.offset,
                    inspection.split.as_ref().unwrap().offsets,
                    names.names.len(),
                    names.stride,
                    names.offset
                );
            }
            family => {
                assert_eq!(tables.len(), 1, "{}", exe.display());
                assert!(classic_tables.is_empty(), "{}", exe.display());
                assert_eq!(inspection.facet_count(), CLIENT_FACET_DIMENSIONS.len(), "{}", exe.display());
                assert_eq!(inspection.split.is_some(), family == ClientFamily::Enhanced, "{}: split", exe.display());
            }
        }
        if let Some(build) = inspection.known {
            let table_offsets: Vec<usize> = inspection.tables.iter().map(|table| table.offset).collect();
            assert_eq!(build.family, inspection.family, "{}: {}", exe.display(), build.name);
            assert_eq!(table_offsets, build.table_offsets, "{}: {}", exe.display(), build.name);
            let split = inspection.split.as_ref().map(|split| split.offsets.clone()).unwrap_or_default();
            assert_eq!(split, build.split_offsets, "{}", exe.display());
            let startup = inspection.classic.as_ref().and_then(|layout| layout.startup).map(|startup| startup.offset);
            assert_eq!(startup, build.startup_offset, "{}", exe.display());
            assert_eq!(inspection.pe.checksum_offset, build.checksum_offset, "{}", exe.display());
        }
        println!("  {}; known build: {}", inspection.family.label(), inspection.known.map_or("no", |build| build.name));
        located += 1;
    }
    assert!(located > 0, "no client executables under {CLIENT_ROOTS_ENV}");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn multi_facet_patched_copies_change_only_the_expected_bytes() {
    for exe in known_kind_exes() {
        let original = std::fs::read(&exe).unwrap();
        let before = ClientExeInspection::inspect(&original).unwrap();
        let classic = before.family == ClientFamily::Classic;
        let first_height = if classic { before.dimensions()[0].height } else { EC_PATCHED_HEIGHT };
        let second_width = before.dimensions()[usize::from(SECOND_PATCHED_FACET)].width;
        let mut edits = vec![
            ExeEdit::FacetSize { facet: PATCHED_FACET, width: PATCHED_WIDTH, height: first_height },
            ExeEdit::FacetSize { facet: SECOND_PATCHED_FACET, width: second_width, height: SECOND_PATCHED_HEIGHT },
        ];
        if before.split.is_some() {
            edits.push(ExeEdit::Split(SplitEdit::Set(PATCHED_SPLIT)));
        }
        let patch = ExePatch { edits: edits.clone() };
        let dir = common::temp_dir("client_exe_patch");
        let output = dir.join(exe.file_name().unwrap());
        assert!(write_patched_client_exe(&exe, &exe, &patch).is_err(), "the input is refused as the output");
        let plan = dry_run_client_exe(&exe, &patch).unwrap();
        let patched = write_patched_client_exe(&exe, &output, &patch).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), original, "the input is unchanged");
        assert_eq!(plan.changes, patched.changes, "the dry run is the write");

        let copy = std::fs::read(&output).unwrap();
        assert_eq!(copy.len(), original.len());
        let allowed = allowed_ranges(&before, &edits);
        let changed: Vec<usize> = (0..copy.len()).filter(|&offset| copy[offset] != original[offset]).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|offset| allowed.iter().any(|range| range.contains(offset))), "{}: {changed:x?}", exe.display());
        let planned: Vec<usize> = patched
            .changes
            .iter()
            .flat_map(|change| (0..change.new.len()).filter(|at| change.old[*at] != change.new[*at]).map(|at| change.offset + at))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(changed, planned);

        let after = ClientExeInspection::inspect(&copy).unwrap();
        assert!(after.checksum_valid() && after.known.is_none());
        assert_eq!(after.family, before.family);
        for (table, before_table) in after.tables.iter().zip(&before.tables) {
            assert_eq!(table.offset, before_table.offset);
            assert_eq!(table.records[0].geometry, FacetGeometry::new(PATCHED_WIDTH, first_height));
            let second = &table.records[usize::from(SECOND_PATCHED_FACET)];
            assert_eq!(second.geometry, FacetGeometry::new(second_width, SECOND_PATCHED_HEIGHT));
            if let Some(classic) = second.classic {
                assert_eq!(classic.wrap, second.geometry, "{}: the wrap followed the size", exe.display());
            }
            for facet in [1, 3] {
                assert_eq!(table.records[facet].geometry, before_table.records[facet].geometry);
            }
        }
        if let Some(startup) = after.classic.as_ref().and_then(|layout| layout.startup) {
            assert_eq!(startup.size, FacetGeometry::new(PATCHED_WIDTH, first_height));
        }
        assert_eq!(after.split.as_ref().map(|split| split.x), before.split.as_ref().map(|_| PATCHED_SPLIT));
        let manifest = read_manifest(&output).unwrap().unwrap();
        assert_eq!(manifest.original_sha256, before.sha256);
        assert_eq!(manifest.source_sha256, before.sha256);
        assert_eq!(manifest.patched_sha256, sha256_hex(&copy));
        assert_eq!(manifest.build, before.known.map(|build| build.name.to_owned()));
        assert_eq!(manifest.family, Some(before.family));
        assert_eq!(manifest.values, after.values());
        assert_eq!(manifest.steps.len(), 1);
        assert_eq!(manifest.steps[0].edits, edits);

        let last = (after.facet_count() - 1) as u8;
        let chained = ExePatch { edits: vec![ExeEdit::FacetSize { facet: last, width: CHAINED_SIZE.width, height: CHAINED_SIZE.height }] };
        let second_dir = common::temp_dir("client_exe_chain");
        let second = write_patched_client_exe(&output, &second_dir.join(exe.file_name().unwrap()), &chained).unwrap();
        assert_eq!(second.manifest.original_sha256, before.sha256);
        assert_eq!(second.manifest.source_sha256, manifest.patched_sha256);
        assert_eq!(second.manifest.steps.len(), 2);
        assert_eq!(second.after.dimensions()[usize::from(last)], CHAINED_SIZE);
        let orphan_dir = common::temp_dir("client_exe_orphan");
        let orphan = orphan_dir.join(exe.file_name().unwrap());
        std::fs::copy(&output, &orphan).unwrap();
        let refused = write_patched_client_exe(&orphan, &second_dir.join("orphan.exe"), &chained);
        assert!(refused.is_err(), "a patched copy without its manifest is refused");
        println!("{}: {} bytes changed in {} fields, warnings {:?}", exe.display(), changed.len(), patched.changes.len(), patched.warnings);
        for dir in [dir, second_dir, orphan_dir] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn shipped_facet_data_matches_the_stock_tables() {
    for exe in known_kind_exes() {
        let client_dir = exe.parent().unwrap();
        let has_data = match exe_kind(&exe) {
            ExeKind::Classic => CLASSIC_MAP_0_FILES.iter().any(|name| find_file_ignoring_case(client_dir, name).is_some()),
            _ => client_dir.join(facet_package_file_name(0, false)).is_file(),
        };
        if !has_data {
            println!("{}: no facet data next to it, skipped", exe.display());
            continue;
        }
        let inspection = inspect(&exe);
        let reports = check_client_facet_data(&inspection, client_dir).unwrap();
        let checked = reports.iter().flat_map(|report| &report.items).filter(|item| item.status == FacetDataStatus::Ok).count();
        let absent: Vec<_> = reports
            .iter()
            .flat_map(|report| &report.items)
            .filter(|item| item.status == FacetDataStatus::Absent)
            .map(|item| item.source.as_str())
            .collect();
        let mismatches: Vec<_> = reports.iter().flat_map(|report| report.mismatches()).collect();
        println!("{}: {checked} files agree, absent {absent:?}", exe.display());
        assert!(checked > 0, "{}: nothing checked", exe.display());
        assert!(mismatches.is_empty(), "{}: {mismatches:?}", exe.display());

        if inspection.family != ClientFamily::Classic {
            let mut resized = inspection.dimensions();
            resized[usize::from(PATCHED_FACET)] = FacetGeometry::new(PATCHED_WIDTH, EC_PATCHED_HEIGHT);
            let reports = check_facet_data(&resized, client_dir).unwrap();
            assert!(reports[usize::from(PATCHED_FACET)].mismatches().count() > 0, "stock data does not fit a resized facet");
        }
    }
}
