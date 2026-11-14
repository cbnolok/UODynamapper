use std::path::{Path, PathBuf};
use std::time::Instant;

use uocf::client::login_servers::{CfgServerKind, LoginCfg};
use uocf::client::facet_data::FacetDataStatus;
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily, DISABLED_SPLIT_X, STOCK_SPLIT_X};
use uocf::client::login::{LoginHost, LoginServerEdit, LOGIN_HOST_SLOT_SIZE, MAX_LOGIN_HOST_LENGTH};
use uocf::client::patch::{patch_client_exe, ChangeCause, ExeEdit, ExePatch, SplitEdit};
use uocf::enhanced::facet_sector::{FacetGeometry, CLIENT_FACET_DIMENSIONS};

use super::edits::{EditState, SeamChoice};
use super::encryption::EncryptionStatus;
use super::login::{login_mode, login_site_rows, stock_login, ClassicLogin, LoginEdit, LoginMode, KR_IPV4_ONLY};
use super::opened::{classic_files, dif_lines, manifest_steps, seam_status, table_locations, ManifestState, OpenedExe};
use super::preview::{change_rows, preview, seam_warnings, DataCheck, Preview, PreviewCache};
use super::synthetic::{
    classic_stock_size, classic_stock_wrap, synthetic_classic, synthetic_classic_with_difs, synthetic_enhanced,
    synthetic_enhanced_login, synthetic_kingdom_reborn, synthetic_kingdom_reborn_ciphers, synthetic_kingdom_reborn_login,
    SyntheticLogin, TOL_HOSTS,
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
/// Login hosts and ports, short enough for the 12-byte slot (11 characters at most).
const SHARD_HOSTS: [&str; 2] = ["10.0.0.2", "10.0.0.4"];
const SHARD_PORTS: [u16; 2] = [2593, 2594];
/// The longest host of UOSA 4.0.115.0's 12-byte second slot.
const SHORT_SLOT_HOST_LENGTH: usize = 11;
/// 13 characters: fits a 16-byte slot, not a 12-byte one.
const LONG_HOST: &str = "192.168.100.1";
const NAMED_HOST: &str = "shard.example";

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
    assert_eq!(edits.build_patch().unwrap().edits, [size_edit.clone()]);
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
fn the_executable_section_says_whether_the_difs_apply() {
    assert_eq!(dif_lines(&opened_from(synthetic_enhanced(false), ManifestState::None).inspection), None);
    for (image, status, advice) in [
        (synthetic_classic(), "absent", "convert the map without"),
        (synthetic_classic_with_difs(false), "loaded, never applied", "convert the map without"),
        (synthetic_classic_with_difs(true), "applied", "convert the map with the dif files"),
    ] {
        let opened = opened_from(image, ManifestState::None);
        let lines = dif_lines(&opened.inspection).unwrap();
        let evidence = &opened.inspection.difs.as_ref().unwrap().evidence;
        assert_eq!(lines.len(), 3, "{lines:?}");
        for (line, kind) in lines.iter().zip(["land (mapdif): ", "statics (stadif): "]) {
            assert!(line.starts_with(&format!("{kind}{status}")), "{line} / {evidence:#?}");
        }
        assert!(lines[2].starts_with(advice), "{}", lines[2]);
        // The dif code does not disturb the rest of the inspection.
        assert!(opened.blockers().is_empty(), "{:?}", opened.blockers());
    }
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

/// Types `hosts` and `ports` into the login fields, one per site (`same_for_all` off); the other fields are empty.
fn type_login(login: &mut LoginEdit, hosts: &[&str], ports: &[u16]) {
    login.set_same_for_all(false);
    login.clear();
    for (field, host) in login.hosts.iter_mut().zip(hosts) {
        field.text = host.to_string();
    }
    for (field, port) in login.ports.iter_mut().zip(ports) {
        field.text = port.to_string();
    }
}

fn login_edit(hosts: &[&str], ports: &[u16]) -> ExeEdit {
    let hosts = hosts.iter().map(|host| LoginHost::new(host).unwrap()).collect();
    ExeEdit::LoginServer(LoginServerEdit { hosts, ports: ports.to_vec() })
}

#[test]
fn an_enhanced_login_edit_joins_the_facet_patch() {
    let opened = opened_from(synthetic_enhanced_login(SyntheticLogin::TimeOfLegends { short_slot: false }), ManifestState::None);
    assert_eq!(login_mode(&opened.inspection), LoginMode::PublicSites);
    let mut edits = EditState::new(&opened.inspection);
    assert_eq!(edits.login.hosts.len(), TOL_HOSTS.len());
    assert!(edits.login.same_for_all && edits.login.has_several_sites());
    assert_eq!(edits.login.hosts[1].capacity, Some(LOGIN_HOST_SLOT_SIZE));

    // One host and port typed with "same for all" go to every site, next to the facet edit.
    set_size(&mut edits, 0, FacetGeometry::new(NEW_WIDTH, NEW_HEIGHT));
    edits.login.hosts[0].text = SHARD_HOSTS[0].to_owned();
    edits.login.spread(false, 0);
    edits.login.ports[0].text = SHARD_PORTS[0].to_string();
    edits.login.spread(true, 0);
    let patch = edits.build_patch().unwrap();
    let uniform = login_edit(&[SHARD_HOSTS[0]; 2], &[SHARD_PORTS[0]; 2]);
    assert_eq!(patch.edits, [ExeEdit::FacetSize { facet: 0, width: NEW_WIDTH, height: NEW_HEIGHT }, uniform]);

    // The dry run lists the login bytes: two host slots and two ports, after the facet's.
    let plan = preview(&opened, &edits).plan().expect("planned").clone();
    let fields: Vec<String> = plan.changes.iter().filter(|change| change.cause == ChangeCause::Edit(1)).map(|change| change.field.clone()).collect();
    let slot = |number: usize| format!("login host {number} ({LOGIN_HOST_SLOT_SIZE}-byte slot)");
    assert_eq!(fields, [slot(1), slot(2), "login port 1 (imm16)".to_owned(), "login port 2 (imm16)".to_owned()]);
    let rows = change_rows(&plan.changes, &plan.patch);
    assert!(rows.iter().any(|row| row.cause == format!("edit 2: login server {}:{}", SHARD_HOSTS[0], SHARD_PORTS[0])), "{rows:?}");

    // A host and port per site.
    type_login(&mut edits.login, &SHARD_HOSTS, &SHARD_PORTS);
    assert_eq!(edits.build_patch().unwrap().edits[1], login_edit(&SHARD_HOSTS, &SHARD_PORTS));
    // An empty field keeps the file's value.
    edits.login.hosts[1].text.clear();
    edits.login.ports[1].text.clear();
    assert_eq!(
        edits.build_patch().unwrap().edits[1],
        login_edit(&[SHARD_HOSTS[0], TOL_HOSTS[1]], &[SHARD_PORTS[0], opened.inspection.login.as_ref().unwrap().ports[1].port])
    );

    // Written and reopened: the new values are the file's, and the manifest knows the stock ones.
    let temp = TempDir::new("login_write");
    let client = temp.folder("client");
    let exe = client.join(EC_EXE_NAME);
    std::fs::write(&exe, &opened.bytes).unwrap();
    let opened = OpenedExe::open(&exe).unwrap();
    type_login(&mut edits.login, &SHARD_HOSTS, &SHARD_PORTS);
    let copy = plan_write_and_reopen(&opened, &edits, &temp.folder("copies").join(EC_EXE_NAME));
    let reopened = LoginEdit::new(&copy.inspection);
    let current: Vec<&str> = reopened.hosts.iter().map(|field| field.current.as_str()).collect();
    assert_eq!(current, SHARD_HOSTS);
    let stock = stock_login(&copy.inspection, copy.current_manifest()).unwrap();
    assert_eq!(stock.hosts, TOL_HOSTS);
    let rows = login_site_rows(&copy.inspection, Some(&stock));
    assert!(rows[0].stock.starts_with("changed"), "{rows:?}");
    // Typing the file's own values is no edit.
    let mut same = EditState::new(&copy.inspection);
    type_login(&mut same.login, &SHARD_HOSTS, &SHARD_PORTS);
    assert!(same.login.is_edited() && same.build_patch().unwrap().edits.is_empty());
}

#[test]
fn login_hosts_validate_against_their_own_slot() {
    let opened = opened_from(synthetic_enhanced_login(SyntheticLogin::TimeOfLegends { short_slot: true }), ManifestState::None);
    let mut login = EditState::new(&opened.inspection).login;
    let lengths: Vec<Option<usize>> = login.hosts.iter().map(|field| field.max_host_length()).collect();
    assert_eq!(lengths, [Some(MAX_LOGIN_HOST_LENGTH), Some(SHORT_SLOT_HOST_LENGTH)]);
    // Same for all: the long host fits slot 1, not slot 2.
    login.hosts[0].text = LONG_HOST.to_owned();
    login.spread(false, 0);
    let (edit, validation) = login.check();
    assert!(edit.is_none());
    assert_eq!(validation.errors.len(), 1, "{:?}", validation.errors);
    assert!(validation.errors[0].starts_with("host 2:"), "{:?}", validation.errors);
    // Per slot: the long host on the 16-byte slot, a short one on the 12-byte slot.
    type_login(&mut login, &[LONG_HOST, SHARD_HOSTS[0]], &[]);
    let (edit, validation) = login.check();
    assert!(validation.is_ok(), "{:?}", validation.errors);
    assert!(edit.is_some());
    // A host name warns once; bad ports fail.
    type_login(&mut login, &[NAMED_HOST, NAMED_HOST], &[]);
    assert_eq!(login.check().1.warnings.len(), 1);
    type_login(&mut login, &[], &[0]);
    assert!(!login.check().1.is_ok());
    login.ports[1].text = "port".to_owned();
    assert_eq!(login.check().1.errors.len(), 2);
    login.clear();
    assert!(!login.is_edited() && login.check() == (None, Default::default()));

    // The one-host build before Time of Legends: one host and one port site.
    let one = LoginEdit::new(&ClientExeInspection::inspect(&synthetic_enhanced_login(SyntheticLogin::OneHost)).unwrap());
    assert_eq!((one.hosts.len(), one.ports.len()), (1, 1));
    assert!(!one.has_several_sites());
}

#[test]
fn kingdom_reborn_takes_an_ipv4_address() {
    let temp = TempDir::new("kr_login");
    let exe = temp.folder("client").join(KR_EXE_NAME);
    std::fs::write(&exe, synthetic_kingdom_reborn_login()).unwrap();
    let opened = OpenedExe::open(&exe).unwrap();
    assert_eq!(login_mode(&opened.inspection), LoginMode::FixedAddress);
    let mut edits = EditState::new(&opened.inspection);
    assert_eq!((edits.login.hosts.len(), edits.login.ports.len()), (1, 1));
    assert!(!edits.login.has_several_sites());
    edits.login.hosts[0].text = NAMED_HOST.to_owned();
    let (edit, validation) = edits.login.check();
    assert!(edit.is_none() && validation.errors[0].contains(KR_IPV4_ONLY), "{:?}", validation.errors);
    type_login(&mut edits.login, &[SHARD_HOSTS[0]], &[SHARD_PORTS[1]]);
    assert_eq!(edits.build_patch().unwrap().edits, [login_edit(&[SHARD_HOSTS[0]], &[SHARD_PORTS[1]])]);
    let plan = preview(&opened, &edits).plan().expect("planned").clone();
    let fields: Vec<&str> = plan.changes.iter().filter(|change| change.cause == ChangeCause::Edit(0)).map(|change| change.field.as_str()).collect();
    assert_eq!(fields, ["login address (ip imm32)", "login port (imm16)"]);
    let copy = plan_write_and_reopen(&opened, &edits, &temp.folder("copies").join(KR_EXE_NAME));
    let fixed = &copy.inspection.login.as_ref().unwrap().fixed;
    assert_eq!((fixed.ip.to_string(), fixed.port), (SHARD_HOSTS[0].to_owned(), SHARD_PORTS[1]));
    // Without its login code there is nothing to edit.
    assert_eq!(login_mode(&ClientExeInspection::inspect(&synthetic_kingdom_reborn()).unwrap()), LoginMode::NotLocated);
}

#[test]
fn the_encryption_controls_patch_their_selected_sites() {
    use uocf::client::encryption::{CipherSiteKind, EncryptionPart, EncryptionState};

    let temp = TempDir::new("kr_encryption");
    let exe = temp.folder("client").join(KR_EXE_NAME);
    std::fs::write(&exe, synthetic_kingdom_reborn_ciphers()).unwrap();
    let opened = OpenedExe::open(&exe).unwrap();
    let mut edits = EditState::new(&opened.inspection);
    assert_eq!(edits.encryption.status(), EncryptionStatus::Located(EncryptionState::Encrypted));
    assert!(edits.encryption.can_remove(EncryptionPart::Login));
    assert!(edits.encryption.can_remove(EncryptionPart::Game));
    assert!(edits.encryption.can_remove(EncryptionPart::KrHandshake));
    let rows = edits.encryption.rows(&opened.inspection);
    assert_eq!(rows.len(), CipherSiteKind::of_family(ClientFamily::KingdomReborn).len());
    assert!(rows.iter().all(|row| row.state.starts_with("on")));
    assert!(edits.build_patch().unwrap().edits.is_empty(), "off by default");

    edits.encryption.remove_login = true;
    edits.encryption.remove_game = true;
    edits.encryption.bypass_kr_handshake = true;
    assert_eq!(edits.build_patch().unwrap().edits, [ExeEdit::NoLoginEncryption, ExeEdit::NoGameEncryption, ExeEdit::BypassKrHandshake]);
    let plan = preview(&opened, &edits).plan().expect("planned").clone();
    let sites = &opened.inspection.encryption.as_ref().unwrap().sites;
    let offsets: Vec<usize> = plan.changes.iter().filter(|change| matches!(change.cause, ChangeCause::Edit(_))).map(|change| change.offset).collect();
    assert_eq!(offsets, sites.iter().map(|site| site.offset).collect::<Vec<_>>());
    let copy = plan_write_and_reopen(&opened, &edits, &temp.folder("copies").join(KR_EXE_NAME));
    let reopened = EditState::new(&copy.inspection);
    assert_eq!(reopened.encryption.status(), EncryptionStatus::Located(EncryptionState::Removed));
    assert!(!reopened.encryption.can_remove(EncryptionPart::Login));
    assert!(!reopened.encryption.can_remove(EncryptionPart::Game));
    assert!(!reopened.encryption.can_remove(EncryptionPart::KrHandshake));
    assert!(reopened.encryption.rows(&copy.inspection).iter().all(|row| row.state == "patched out"));

    // Without the cipher code: on is an error; Classic has none.
    let mut missing = EditState::new(&ClientExeInspection::inspect(&synthetic_kingdom_reborn()).unwrap());
    assert!(matches!(missing.encryption.status(), EncryptionStatus::NotLocated(_)));
    missing.encryption.remove_login = true;
    let errors = missing.build_patch().unwrap_err();
    assert!(errors[0].starts_with("encryption: login encryption: sites not all located"), "{errors:?}");
    let classic = EditState::new(&ClientExeInspection::inspect(&synthetic_classic()).unwrap());
    assert_eq!(classic.encryption.status(), EncryptionStatus::NotApplicable);
}

#[test]
fn encryption_controls_validate_only_their_selected_part() {
    use uocf::client::encryption::{CipherSiteKind, EncryptionPart, EncryptionState};

    const UNKNOWN_OPCODE: u8 = 0xcc;
    let mut image = synthetic_kingdom_reborn_ciphers();
    let inspected = ClientExeInspection::inspect(&image).unwrap();
    let handshake = inspected.encryption.as_ref().unwrap().sites.iter()
        .find(|site| site.kind == CipherSiteKind::GameLoginHandshake).unwrap();
    image[handshake.offset] = UNKNOWN_OPCODE;
    let inspection = ClientExeInspection::inspect(&image).unwrap();
    let mut edits = EditState::new(&inspection);
    assert!(matches!(edits.encryption.part_status(EncryptionPart::KrHandshake), EncryptionStatus::NotLocated(_)));
    assert!(edits.encryption.can_remove(EncryptionPart::Login));
    assert!(edits.encryption.can_remove(EncryptionPart::Game));
    edits.encryption.remove_login = true;
    assert_eq!(edits.build_patch().unwrap().edits, [ExeEdit::NoLoginEncryption]);
    edits.encryption.remove_game = true;
    assert_eq!(edits.build_patch().unwrap().edits, [ExeEdit::NoLoginEncryption, ExeEdit::NoGameEncryption]);
    edits.encryption.bypass_kr_handshake = true;
    let (patches, validation) = edits.encryption.check();
    assert!(patches.is_empty() && !validation.is_ok());

    let mut enhanced = EditState::new(&ClientExeInspection::inspect(&synthetic_enhanced(false)).unwrap());
    assert_eq!(enhanced.encryption.part_status(EncryptionPart::KrHandshake), EncryptionStatus::NotApplicable);
    enhanced.encryption.bypass_kr_handshake = true;
    assert!(enhanced.build_patch().unwrap_err()[0].contains("KR handshake is not applicable"));

    // Selecting the bypass alone leaves both cipher controls off.
    let mut bypass = EditState::new(&inspected);
    bypass.encryption.bypass_kr_handshake = true;
    assert_eq!(bypass.build_patch().unwrap().edits, [ExeEdit::BypassKrHandshake]);
    assert_eq!(bypass.encryption.part_status(EncryptionPart::Login), EncryptionStatus::Located(EncryptionState::Encrypted));
    assert_eq!(bypass.encryption.part_status(EncryptionPart::Game), EncryptionStatus::Located(EncryptionState::Encrypted));
}

/// A Classic `login.cfg` with a comment, a line the client does not know and both kinds of server.
const CLASSIC_LOGIN_CFG: &str = ";Loginservers for Ultima Online\r\nLoginServer=127.0.0.1,2593\r\nSomethingElse=1\r\nBackupLoginServer=10.0.0.9,2593\r\n";

#[test]
fn classic_login_lists_are_saved_as_a_copy_keeping_other_lines() {
    let classic = ClientExeInspection::inspect(&synthetic_classic()).unwrap();
    assert_eq!(login_mode(&classic), LoginMode::ConfigFile);
    assert!(LoginEdit::new(&classic).hosts.is_empty());

    let temp = TempDir::new("classic_login");
    let client = temp.folder("client");
    std::fs::write(client.join("login.cfg"), CLASSIC_LOGIN_CFG).unwrap();
    let mut login = ClassicLogin::load(&client);
    assert!(login.cfg_found && login.cfg_error().is_none());
    assert_eq!(login.list(CfgServerKind::Primary), ["127.0.0.1:2593"]);
    assert_eq!(login.list(CfgServerKind::Backup), ["10.0.0.9:2593"]);
    assert_eq!(login.edited_cfg().unwrap().to_text(), CLASSIC_LOGIN_CFG, "unedited: the file as it is");

    login.list_mut(CfgServerKind::Primary)[0] = format!("{}:{}", SHARD_HOSTS[0], SHARD_PORTS[0]);
    login.list_mut(CfgServerKind::Primary).push(format!("{NAMED_HOST}:{}", SHARD_PORTS[1]));
    assert!(login.is_edited());
    let text = login.edited_cfg().unwrap().to_text();
    let expected = format!(
        ";Loginservers for Ultima Online\r\nLoginServer={},{}\r\nLoginServer={NAMED_HOST},{}\r\nSomethingElse=1\r\nBackupLoginServer=10.0.0.9,2593\r\n",
        SHARD_HOSTS[0], SHARD_PORTS[0], SHARD_PORTS[1]
    );
    assert_eq!(text, expected);

    // Bad rows and an empty primary list are errors.
    login.list_mut(CfgServerKind::Backup).push("no port".to_owned());
    assert!(login.edited_cfg().unwrap_err()[0].starts_with("backup 2"));
    login.list_mut(CfgServerKind::Backup).pop();
    let primary = std::mem::take(login.list_mut(CfgServerKind::Primary));
    assert!(login.edited_cfg().is_err());
    *login.list_mut(CfgServerKind::Primary) = primary;

    // The copy goes outside the client folder; an existing one needs the overwrite.
    let client_dirs = [client.clone()];
    let inside = login.save_copy(&client.join("copy.cfg"), false, &client_dirs).unwrap_err();
    assert!(format!("{inside:#}").contains("inside the client folder"), "{inside:#}");
    let output = temp.folder("out").join("login.cfg");
    login.save_copy(&output, false, &client_dirs).unwrap();
    assert_eq!(std::fs::read_to_string(&output).unwrap(), expected);
    assert!(login.save_copy(&output, false, &client_dirs).is_err());
    login.reset();
    login.save_copy(&output, true, &client_dirs).unwrap();
    assert_eq!(LoginCfg::read(&output).unwrap().to_text(), CLASSIC_LOGIN_CFG);
    assert_eq!(std::fs::read_to_string(client.join("login.cfg")).unwrap(), CLASSIC_LOGIN_CFG, "the client's file is untouched");
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

        let same = opened.write_copy(&exe, &patch).unwrap_err();
        assert!(format!("{same:#}").contains("refusing to overwrite the input executable"), "{same:#}");
        let sibling = plan_write_and_reopen(&opened, &edits, &client.join("copy.exe"));
        assert_eq!(sibling.folder(), client);
        assert_eq!(std::fs::read(&exe).unwrap(), opened.bytes, "the source is unchanged");
        let copy = plan_write_and_reopen(&opened, &edits, &copies.join(name));
        std::fs::write(copies.join(name), b"old destination").unwrap();
        std::fs::write(uocf::client::patch::manifest_path(&copies.join(name)), b"old manifest").unwrap();
        let replacement = plan_write_and_reopen(&opened, &edits, &copies.join(name));
        assert_eq!(replacement.bytes, copy.bytes);
        assert_eq!(std::fs::read(&exe).unwrap(), opened.bytes, "replacing the destination preserves the source");

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
/// logic: inspect, edit facets 0 and 2, the split or seam and the login server (Enhanced Client: a host and port per
/// site; Kingdom Reborn: its IPv4 address), check the data folder against the edited sizes, dry run, write the copy
/// to the temp dir (`TMPDIR`), reopen it and patch it again. Classic: the login files of the folder are read and the
/// edited login file is saved as a copy in the temp dir.
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
        match login_mode(&opened.inspection) {
            LoginMode::PublicSites | LoginMode::FixedAddress => {
                type_login(&mut edits.login, &SHARD_HOSTS, &SHARD_PORTS);
                let (edit, validation) = edits.login.check();
                assert!(validation.is_ok() && edit.is_some(), "{}: {:?}", exe.display(), validation);
            }
            LoginMode::ConfigFile => {
                let mut login = ClassicLogin::load(&opened.folder());
                let report = login.report.as_ref().unwrap_or_else(|error| panic!("{}: {error}", exe.display()));
                let entries: Vec<String> = report.entries.iter().map(|entry| format!("{:?}", entry.address)).collect();
                eprintln!("{}: {} read={} {entries:?}", exe.display(), login.cfg_path.display(), login.cfg_found);
                assert!(login.cfg_error().is_none(), "{}", exe.display());
                login.list_mut(CfgServerKind::Primary).insert(0, format!("{}:{}", SHARD_HOSTS[0], SHARD_PORTS[0]));
                let output = temp.folder(&format!("{index}_login")).join(login.file_name());
                login.save_copy(&output, false, &[opened.folder()]).unwrap_or_else(|error| panic!("{}: {error:#}", exe.display()));
                let written = LoginCfg::read(&output).unwrap();
                let primary = written.servers().into_iter().find(|line| line.kind == CfgServerKind::Primary).unwrap();
                assert_eq!(primary.address.unwrap().host, SHARD_HOSTS[0]);
            }
            LoginMode::NotLocated => panic!("{}: login code not located", exe.display()),
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
