use std::path::{Path, PathBuf};
use std::time::Instant;

use uocf::client_exe::facet_data::FacetDataStatus;
use uocf::client_exe::facet_dimensions::{ClientExeInspection, ClientFamily, DISABLED_SPLIT_X, STOCK_SPLIT_X};
use uocf::client_exe::patch::{patch_client_exe, ChangeCause, ExeEdit, ExePatch, SplitEdit};
use uocf::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS};

use super::edits::{EditState, SeamChoice};
use super::opened::{classic_files, manifest_steps, seam_status, table_locations, ManifestState, OpenedExe};
use super::preview::{change_rows, preview, seam_warnings, DataCheck, Preview, PreviewCache};
use super::synthetic::{
    classic_stock_size, classic_stock_wrap, synthetic_classic, synthetic_enhanced, synthetic_kingdom_reborn,
};

const NEW_WIDTH: u32 = 8192;
const NEW_HEIGHT: u32 = 4160;
/// Not a multiple of 64: the library warns of padded sectors.
const PADDED_HEIGHT: u32 = 1608;
const NEW_SPLIT: u32 = 6144;
/// Not a multiple of 8.
const UNALIGNED_SIDE: u32 = 8196;
const CHAINED_SIZE: FacetGeometry = FacetGeometry::new(512, 512);
const CLASSIC_ILSHENAR_WIDTH: u32 = 2400;
/// A Felucca narrower than its stock wrap (5120).
const CLASSIC_NARROW_FELUCCA: u32 = 4096;
const EC_EXE_NAME: &str = "UOSA.exe";
const KR_EXE_NAME: &str = "UOKR.exe";
const CLASSIC_EXE_NAME: &str = "client.exe";

fn opened_from(bytes: Vec<u8>, manifest: ManifestState) -> OpenedExe {
    let inspection = ClientExeInspection::inspect(&bytes).unwrap();
    OpenedExe { path: PathBuf::from(EC_EXE_NAME), bytes, inspection, manifest }
}

fn set_size(edits: &mut EditState, facet: usize, size: FacetGeometry) {
    edits.rows[facet].width = size.width.to_string();
    edits.rows[facet].height = size.height.to_string();
}

fn set_wrap(edits: &mut EditState, facet: usize, wrap: FacetGeometry) {
    edits.rows[facet].wrap_width = wrap.width.to_string();
    edits.rows[facet].wrap_height = wrap.height.to_string();
    edits.rows[facet].wrap_touched = true;
}

fn move_seam(edits: &mut EditState, x: u32) {
    edits.seam.choice = SeamChoice::Move;
    edits.seam.x = x.to_string();
}

/// A fresh folder under the temp dir (`TMPDIR`), removed by [`TempDir::drop`].
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("uo_client_patcher_gui_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn folder(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_edit_state_builds_one_patch_of_every_section() {
    let opened = opened_from(synthetic_enhanced(false), ManifestState::None);
    let mut edits = EditState::new(&opened.inspection);
    assert_eq!(edits.rows.len(), CLIENT_FACET_DIMENSIONS.len());
    assert_eq!(edits.seam.current, Some(STOCK_SPLIT_X));
    assert_eq!(edits.build_patch().unwrap(), ExePatch::default());
    assert!(matches!(preview(&opened, &edits), Preview::Empty));

    set_size(&mut edits, 0, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    set_size(&mut edits, 2, FacetGeometry::new(CLIENT_FACET_DIMENSIONS[2].width, PADDED_HEIGHT));
    move_seam(&mut edits, NEW_SPLIT);
    let patch = edits.build_patch().unwrap();
    assert_eq!(
        patch.edits,
        [
            ExeEdit::FacetSize { facet: 0, width: NEW_WIDTH, height: NEW_HEIGHT },
            ExeEdit::FacetSize { facet: 2, width: CLIENT_FACET_DIMENSIONS[2].width, height: PADDED_HEIGHT },
            ExeEdit::Split(SplitEdit::Set(NEW_SPLIT)),
        ]
    );
    assert!(edits.rows[0].is_edited() && !edits.rows[1].is_edited());

    let preview = preview(&opened, &edits);
    let plan = preview.plan().expect("planned");
    assert_eq!(plan.patch, patch);
    assert_eq!(plan.warnings.len(), 1, "1608 is not a multiple of 64: {:?}", plan.warnings);
    let rows = change_rows(&plan.changes, &plan.patch);
    assert_eq!(rows.len(), plan.changes.len());
    let causes: Vec<ChangeCause> = plan.changes.iter().map(|change| change.cause).collect();
    let edit = ChangeCause::Edit;
    assert_eq!(causes, [edit(0), edit(0), edit(1), edit(2), edit(2), edit(2), ChangeCause::Checksum]);
    assert!(rows[0].cause.starts_with("edit 1: facet 0 size"), "{}", rows[0].cause);
    assert_eq!(rows.last().unwrap().va, "header");
    assert_eq!(rows[0].old.split(' ').count(), size_of::<u32>());

    // Moving the seam back to its x, or keeping it, is no edit; disabling it is one.
    move_seam(&mut edits, STOCK_SPLIT_X);
    assert_eq!(edits.build_patch().unwrap().edits.len(), 2);
    edits.seam.choice = SeamChoice::Disable;
    assert_eq!(edits.build_patch().unwrap().edits.last(), Some(&ExeEdit::Split(SplitEdit::Disable)));
}

#[test]
fn rows_reset_to_the_file_and_to_stock() {
    let image = synthetic_enhanced(false);
    let opened = opened_from(image.clone(), ManifestState::None);
    let mut edits = EditState::new(&opened.inspection);
    let unedited = edits.clone();
    set_size(&mut edits, 3, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    assert!(edits.rows[3].is_edited() && edits.can_reset_to_stock(3));
    edits.reset_to_current(3);
    assert_eq!(edits, unedited);
    assert!(!edits.can_reset_to_stock(3), "a stock file shows stock values");

    // A patched copy: its rows start at the patched size, and reset to the shipped one.
    let patch = ExePatch { edits: vec![ExeEdit::FacetSize { facet: 3, width: NEW_WIDTH, height: NEW_HEIGHT }] };
    let patched = patch_client_exe(&image, &patch, None).unwrap();
    let copy = opened_from(patched.bytes, ManifestState::Current(patched.manifest));
    assert!(copy.blockers().is_empty());
    let mut edits = EditState::new(&copy.inspection);
    assert_eq!(edits.rows[3].current, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    assert_eq!(edits.rows[3].stock, Some(CLIENT_FACET_DIMENSIONS[3]));
    assert!(edits.can_reset_to_stock(3));
    edits.reset_to_stock(3);
    let stock = CLIENT_FACET_DIMENSIONS[3];
    let expected = ExeEdit::FacetSize { facet: 3, width: stock.width, height: stock.height };
    assert_eq!(edits.build_patch().unwrap().edits, [expected]);
    assert!(preview(&copy, &edits).plan().is_some(), "the copy patches back with its manifest");
    let steps = manifest_steps(copy.current_manifest().unwrap());
    assert_eq!(steps.len(), 1);
    assert!(steps[0].contains("facet 3 size"), "{steps:?}");

    // Without its manifest the copy cannot be patched.
    let orphan = opened_from(copy.bytes.clone(), ManifestState::None);
    assert_eq!(orphan.blockers().len(), 1);
    assert!(matches!(preview(&orphan, &edits), Preview::Refused(_)));

    // Classic: reset to stock restores the wrap too.
    let classic = opened_from(synthetic_classic(), ManifestState::None);
    let mut edits = EditState::new(&classic.inspection);
    set_size(&mut edits, 1, FacetGeometry::new(NEW_WIDTH, classic_stock_size(1).height));
    set_wrap(&mut edits, 1, FacetGeometry::new(NEW_SPLIT, classic_stock_size(1).height));
    edits.reset_to_stock(1);
    assert_eq!(edits.build_patch().unwrap(), ExePatch::default());
    assert!(!edits.rows[1].wrap_touched);
}

#[test]
fn rows_and_the_seam_validate_live() {
    let opened = opened_from(synthetic_enhanced(false), ManifestState::None);
    let mut edits = EditState::new(&opened.inspection);
    edits.rows[0].width = "wide".to_owned();
    let check = edits.check_row(0);
    assert!(check.size.is_none() && check.validation.errors[0].contains("not a whole number"));
    assert!(edits.edited_dimensions().is_err());
    set_size(&mut edits, 0, FacetGeometry::new(UNALIGNED_SIDE, NEW_HEIGHT));
    assert!(edits.check_row(0).validation.errors[0].contains("multiple of"));
    let errors = edits.build_patch().unwrap_err();
    assert!(errors[0].starts_with("facet 0 (Felucca): "), "{errors:?}");
    set_size(&mut edits, 0, FacetGeometry::new(NEW_WIDTH, PADDED_HEIGHT));
    let check = edits.check_row(0);
    assert!(check.validation.is_ok() && check.validation.warnings.len() == 1, "{check:?}");
    move_seam(&mut edits, DISABLED_SPLIT_X + 1);
    assert!(!edits.check_seam().1.is_ok());
    edits.seam.x = "east".to_owned();
    assert!(!edits.check_seam().1.is_ok());
    assert!(edits.build_patch().unwrap_err()[0].starts_with("seam: "));

    // Kingdom Reborn: no seam; resizing facet 0 or 1 warns.
    let kr = opened_from(synthetic_kingdom_reborn(), ManifestState::None);
    assert_eq!(kr.inspection.family, ClientFamily::KingdomReborn);
    assert!(seam_status(&kr.inspection).starts_with("none"));
    let mut edits = EditState::new(&kr.inspection);
    assert_eq!(edits.seam.current, None);
    set_size(&mut edits, 1, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    assert_eq!(edits.check_row(1).validation.warnings.len(), 1);
    edits.seam.choice = SeamChoice::Disable;
    assert!(!edits.check_seam().1.is_ok(), "no seam to disable");

    // Classic: a narrower Felucca cuts its wrap until the wrap is set.
    let classic = opened_from(synthetic_classic(), ManifestState::None);
    let mut edits = EditState::new(&classic.inspection);
    let narrow = FacetGeometry::new(CLASSIC_NARROW_FELUCCA, classic_stock_size(0).height);
    set_size(&mut edits, 0, narrow);
    let check = edits.check_row(0);
    assert!(check.validation.errors[0].contains("set the wrap"), "{check:?}");
    set_wrap(&mut edits, 0, narrow);
    assert!(edits.check_row(0).validation.is_ok());
    set_wrap(&mut edits, 0, FacetGeometry::new(CLASSIC_NARROW_FELUCCA + 1, narrow.height));
    assert!(!edits.check_row(0).validation.is_ok());
    // Facet 0 needs the startup globals.
    edits.facet_0_patchable = false;
    set_wrap(&mut edits, 0, narrow);
    assert!(edits.check_row(0).validation.errors.iter().any(|error| error.contains("startup")));
}

#[test]
fn classic_wraps_follow_the_size_until_typed() {
    let opened = opened_from(synthetic_classic(), ManifestState::None);
    assert_eq!(opened.inspection.tables.len(), 2);
    assert_eq!(table_locations(&opened.inspection).len(), 2);
    let (files, felucca_rules) = classic_files(&opened.inspection, 0).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0].contains("map map0.mul") && files[1].contains("map map0x.mul"), "{files:?}");
    assert_eq!(felucca_rules, 1);
    let mut edits = EditState::new(&opened.inspection);
    let ilshenar = FacetGeometry::new(CLASSIC_ILSHENAR_WIDTH, classic_stock_size(2).height);
    set_size(&mut edits, 2, ilshenar);
    edits.sync_followed_wraps();
    assert_eq!(edits.rows[2].wrap_width, CLASSIC_ILSHENAR_WIDTH.to_string(), "the wrap followed");
    assert_eq!(edits.check_row(2).wrap, Some(ilshenar));
    let size_edit = ExeEdit::FacetSize { facet: 2, width: ilshenar.width, height: ilshenar.height };
    assert_eq!(edits.build_patch().unwrap().edits, [size_edit]);
    // Keeping the old wrap is a wrap edit.
    set_wrap(&mut edits, 2, classic_stock_wrap(2));
    edits.sync_followed_wraps();
    let wrap = classic_stock_wrap(2);
    let wrap_edit = ExeEdit::ClassicWrap { facet: 2, wrap_width: wrap.width, wrap_height: wrap.height };
    assert_eq!(edits.build_patch().unwrap().edits, [size_edit, wrap_edit]);

    // Felucca's wrap and the seam: one patch writes both, and the library warns while Trammel's wrap differs.
    edits.reset_to_current(2);
    set_wrap(&mut edits, 0, FacetGeometry::new(NEW_SPLIT, classic_stock_size(0).height));
    let felucca_wrap = edits.felucca_wrap_width().unwrap();
    move_seam(&mut edits, felucca_wrap);
    assert_eq!(edits.seam.x, NEW_SPLIT.to_string());
    let preview = preview(&opened, &edits);
    let plan = preview.plan().expect("planned");
    assert_eq!(plan.patch.edits.len(), 2);
    assert_eq!(seam_warnings(&plan.warnings).len(), 1, "{:?}", plan.warnings);
    assert_eq!(plan.expected.split_x, Some(NEW_SPLIT));
}

#[test]
fn the_preview_cache_replans_only_on_change() {
    let opened = opened_from(synthetic_enhanced(false), ManifestState::None);
    let mut edits = EditState::new(&opened.inspection);
    let mut cache = PreviewCache::default();
    assert!(matches!(cache.get(&opened, &edits), Preview::Empty));
    set_size(&mut edits, 4, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    assert!(cache.get(&opened, &edits).plan().is_some());
    edits.reset_to_current(4);
    assert!(matches!(cache.get(&opened, &edits), Preview::Empty));

    let signed = opened_from(synthetic_enhanced(true), ManifestState::None);
    assert_eq!(signed.blockers().len(), 1);
    let mut edits = EditState::new(&signed.inspection);
    set_size(&mut edits, 4, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    let mut cache = PreviewCache::default();
    assert!(matches!(cache.get(&signed, &edits), Preview::Refused(error) if error.contains("signed")));
}

#[test]
fn the_data_check_uses_the_edited_sizes() {
    let temp = TempDir::new("data_check");
    for image in [synthetic_enhanced(false), synthetic_classic()] {
        let opened = opened_from(image, ManifestState::None);
        let mut edits = EditState::new(&opened.inspection);
        let edited = FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT);
        set_size(&mut edits, 0, edited);
        let dimensions = edits.edited_dimensions().unwrap();
        let check = DataCheck::run(&opened.inspection, &dimensions, &temp.0);
        let reports = check.reports.as_ref().unwrap();
        assert_eq!(reports.len(), edits.rows.len());
        assert_eq!(reports[0].geometry, edited);
        assert!(reports.iter().flat_map(|report| &report.items).all(|item| item.status == FacetDataStatus::Absent));
        let counts = check.counts();
        assert!(counts.absent > 0 && counts.ok == 0 && counts.mismatch == 0);
        assert!(check.is_for(&dimensions, &temp.0));
        edits.reset_to_current(0);
        assert!(!check.is_for(&edits.edited_dimensions().unwrap(), &temp.0));
    }
}

/// Patches `exe` with `edits` through the logic: dry run, write to `copies`, reopen; the written file is the dry run
/// applied and the reopened copy has the planned values and a current manifest. Returns the reopened copy.
fn plan_write_and_reopen(opened: &OpenedExe, edits: &EditState, output: &Path) -> OpenedExe {
    let started = Instant::now();
    let preview = preview(opened, edits);
    let elapsed = started.elapsed();
    let plan = match &preview {
        Preview::Planned(plan) => plan,
        other => panic!("{}: {other:?}", opened.path.display()),
    };
    eprintln!("{}: dry run of {} edits in {elapsed:?}", opened.path.display(), plan.patch.edits.len());
    let written = opened.write_copy(output, &plan.patch).unwrap_or_else(|error| panic!("{error:#}"));
    assert_eq!(written.changes, plan.changes);
    assert_eq!(written.warnings, plan.warnings);
    assert_eq!(std::fs::read(output).unwrap(), plan.apply(&opened.bytes).unwrap());
    let reopened = OpenedExe::open(output).unwrap();
    assert!(matches!(reopened.manifest, ManifestState::Current(_)), "{:?}", reopened.manifest);
    assert_eq!(reopened.inspection.values(), plan.expected);
    assert!(reopened.inspection.checksum_valid() && reopened.blockers().is_empty());
    let reopened_edits = EditState::new(&reopened.inspection);
    for (row, expected) in reopened_edits.rows.iter().zip(&plan.expected.facets) {
        assert_eq!(row.current, FacetGeometry::new(expected.width, expected.height));
    }
    assert_eq!(reopened_edits.seam.current, plan.expected.split_x);
    reopened
}

#[test]
fn the_dry_run_is_what_is_written_and_copies_chain() {
    let temp = TempDir::new("write");
    for (name, image) in [(EC_EXE_NAME, synthetic_enhanced(false)), (CLASSIC_EXE_NAME, synthetic_classic())] {
        let client = temp.folder(&format!("client_{name}"));
        let copies = temp.folder(&format!("copies_{name}"));
        let exe = client.join(name);
        std::fs::write(&exe, image).unwrap();
        let opened = OpenedExe::open(&exe).unwrap();
        assert!(matches!(opened.manifest, ManifestState::None) && opened.blockers().is_empty());
        assert_eq!(opened.folder(), client);
        let mut edits = EditState::new(&opened.inspection);
        let height = edits.rows[0].current.height;
        set_size(&mut edits, 0, FacetGeometry::new(NEW_WIDTH, height));
        let ilshenar_height = edits.rows[2].current.height;
        set_size(&mut edits, 2, FacetGeometry::new(CLASSIC_ILSHENAR_WIDTH, ilshenar_height));
        edits.sync_followed_wraps();
        move_seam(&mut edits, NEW_SPLIT);
        let patch = edits.build_patch().unwrap();

        let inside = opened.write_copy(&client.join("copy.exe"), &patch).unwrap_err();
        assert!(format!("{inside:#}").contains("refusing to write inside the folder"), "{inside:#}");
        let copy = plan_write_and_reopen(&opened, &edits, &copies.join(name));
        let exists = opened.write_copy(&copies.join(name), &patch).unwrap_err();
        assert!(format!("{exists:#}").contains("exists"), "{exists:#}");

        // The reopened copy patches again; its manifest keeps the chain.
        let mut chained = EditState::new(&copy.inspection);
        let last = chained.rows.len() - 1;
        set_size(&mut chained, last, CHAINED_SIZE);
        chained.sync_followed_wraps();
        let second = plan_write_and_reopen(&copy, &chained, &temp.0.join(format!("second_{name}")));
        let manifest = second.current_manifest().unwrap();
        assert_eq!(manifest.steps.len(), 2);
        assert_eq!(manifest.original_sha256, opened.inspection.sha256);
    }
}

// ---- Real files.

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const EXE_EXTENSION: &str = "exe";
const EC_EXE_PREFIX: &str = "uosa";
const CLASSIC_EXE_PREFIX: &str = "client_";
const CLASSIC_EXE_SUFFIX: &str = "_client.exe";
/// Larger executables are installers, not clients.
const MAX_CLIENT_EXE_SIZE: u64 = 64 << 20;

/// An Enhanced Client, Kingdom Reborn or Classic executable, by its name.
fn is_client_exe(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let named = name.starts_with(EC_EXE_PREFIX)
        || name == KR_EXE_NAME.to_ascii_lowercase()
        || name == CLASSIC_EXE_NAME
        || name.starts_with(CLASSIC_EXE_PREFIX)
        || name.ends_with(CLASSIC_EXE_SUFFIX);
    named && std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= MAX_CLIENT_EXE_SIZE)
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
        } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXE_EXTENSION))
            && is_client_exe(&path)
        {
            exes.push(path);
        }
    }
}

/// Every Enhanced Client, Kingdom Reborn and Classic executable under `UO_CLIENT_ROOTS` (only read) goes through the
/// logic: inspect, edit facets 0 and 2 and the split or seam, check the data folder against the edited sizes, dry run,
/// write the copy to the temp dir (`TMPDIR`), reopen it and patch it again.
#[test]
#[ignore = "needs client executables under UO_CLIENT_ROOTS"]
fn real_clients_go_through_inspect_edit_plan_write() {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut exes = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect_exes(&root, &mut exes);
    }
    assert!(!exes.is_empty(), "no client executables under {CLIENT_ROOTS_ENV}");
    let temp = TempDir::new("real");
    let mut families = Vec::new();
    for (index, exe) in exes.iter().enumerate() {
        let opened = OpenedExe::open(exe).unwrap_or_else(|error| panic!("{error:#}"));
        let family = opened.inspection.family;
        families.push(family);
        assert!(opened.blockers().is_empty(), "{}: {:?}", exe.display(), opened.blockers());
        let mut edits = EditState::new(&opened.inspection);
        let felucca = FacetGeometry::new(NEW_WIDTH, edits.rows[0].current.height);
        let ilshenar = FacetGeometry::new(CLASSIC_ILSHENAR_WIDTH, edits.rows[2].current.height);
        set_size(&mut edits, 0, felucca);
        set_size(&mut edits, 2, ilshenar);
        edits.sync_followed_wraps();
        if edits.seam.current.is_some() {
            move_seam(&mut edits, NEW_SPLIT);
        }
        for row in 0..edits.rows.len() {
            let check = edits.check_row(row);
            assert!(check.validation.is_ok(), "{} facet {row}: {:?}", exe.display(), check.validation);
        }

        let dimensions = edits.edited_dimensions().unwrap();
        let started = Instant::now();
        let check = DataCheck::run(&opened.inspection, &dimensions, &opened.folder());
        let reports = check.reports.as_ref().unwrap_or_else(|error| panic!("{}: {error}", exe.display()));
        assert_eq!(reports[0].geometry, felucca);
        let counts = check.counts();
        eprintln!("{}: {} data check {counts:?} in {:?}", exe.display(), family.label(), started.elapsed());

        let copies = temp.folder(&index.to_string());
        let copy = plan_write_and_reopen(&opened, &edits, &copies.join(opened.file_name()));
        let mut chained = EditState::new(&copy.inspection);
        let last = chained.rows.len() - 1;
        set_size(&mut chained, last, CHAINED_SIZE);
        chained.sync_followed_wraps();
        let seconds = temp.folder(&format!("{index}_second"));
        let second = plan_write_and_reopen(&copy, &chained, &seconds.join(opened.file_name()));
        assert_eq!(second.current_manifest().unwrap().steps.len(), 2);
    }
    for family in [ClientFamily::Enhanced, ClientFamily::KingdomReborn, ClientFamily::Classic] {
        assert!(families.contains(&family), "no {} executable under {CLIENT_ROOTS_ENV}", family.label());
    }
}
