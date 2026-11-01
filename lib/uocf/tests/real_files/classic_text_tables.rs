//! The custom-house tables, `suppinfo.txt`, `prof.txt`, `keynames.txt`, `Tile256.txt`, `Tile1024.txt` and
//! `Arttable.cfg` of real clients of every era read strictly and write back byte for byte; an edit changes only
//! its own line. `palette.mul`, `animinfo.mul` and `sjis2uni.mul` write back byte for byte too.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::def_files::{DefDocument, DefRecord};
use uocf::classic::house_tables::{
    DOORS_FILE, DoorsRow, FLOORS_FILE, FloorsRow, MISC_FILE, MiscRow, ROOF_FILE, RoofRow, STAIRS_FILE, SUPPINFO_FILE,
    StairsRow, SuppInfoRecord, TELEPORTERS_FILE, TeleportersRow, WALLS_FILE, WallsRow,
};
use uocf::classic::key_names::{KEY_NAME_COUNT, KEY_NAMES_FILE, KeyNamesFile};
use uocf::classic::legacy_tables::{ARTTABLE_FILE, ArtTableFile, TILE256_FILE, TILE1024_FILE, TileTableRecord};
use uocf::classic::professions::{PROFESSIONS_FILE, ProfessionLine, ProfessionsFile};
use uocf::classic::small_binaries::{
    ANIMINFO_DEFAULT, ANIMINFO_FILE, AnimInfoFile, PALETTE_FILE, PaletteFile, SJIS_FILE, ShiftJisToUnicodeFile,
};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The name the key name edit gives scan code 1.
const EDITED_KEY_NAME: &str = "Escape";
/// The gump the profession edit sets.
const EDITED_GUMP: i32 = 1234;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The lines of `a` and `b` that differ.
fn changed_lines(a: &[u8], b: &[u8]) -> usize {
    let (a, b) = (a.split(|&byte| byte == b'\n').collect::<Vec<_>>(), b.split(|&byte| byte == b'\n').collect::<Vec<_>>());
    assert_eq!(a.len(), b.len(), "an edit keeps the line count");
    a.iter().zip(&b).filter(|(a, b)| a != b).count()
}

/// Reads `name` of `root` as `R` lines, checks it writes back byte for byte, edits its first record with `edit`
/// and checks only that line changed; the record count, or `None` when the client lacks the file.
fn check_table<R: DefRecord>(root: &Path, name: &str, edit: impl Fn(&mut R)) -> Option<usize> {
    let client = label(root);
    let path = find_file_ignoring_case(root, name)?;
    let bytes = std::fs::read(&path).unwrap();
    let mut file = DefDocument::<R>::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {name}: {error}"));
    assert_eq!(file.to_bytes(), bytes, "{client} {name}");
    let records = file.records().count();
    assert!(records > 0, "{client} {name}: no record");
    edit(file.records_mut().next().unwrap());
    let edited = file.to_bytes();
    assert_eq!(changed_lines(&bytes, &edited), 1, "{client} {name}: only the edited line changes");
    let back = DefDocument::<R>::from_bytes(&edited).unwrap();
    assert_eq!(back.records().next(), file.records().next(), "{client} {name}: the edited record");
    println!("{client}: {name} ({records} records) written back byte for byte");
    Some(records)
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_house_tables_and_suppinfo_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let tables = [
            check_table::<WallsRow>(&root, WALLS_FILE, |row| row.values[0] += 1),
            check_table::<FloorsRow>(&root, FLOORS_FILE, |row| row.values[1] += 1),
            check_table::<DoorsRow>(&root, DOORS_FILE, |row| row.values[1] += 1),
            check_table::<MiscRow>(&root, MISC_FILE, |row| row.values[2] += 1),
            check_table::<StairsRow>(&root, STAIRS_FILE, |row| row.values[1] += 1),
            check_table::<TeleportersRow>(&root, TELEPORTERS_FILE, |row| row.values[2] += 1),
            check_table::<RoofRow>(&root, ROOF_FILE, |row| row.values[2] += 1),
            check_table::<SuppInfoRecord>(&root, SUPPINFO_FILE, |row| row.flags[0] = 1 - row.flags[0]),
        ];
        checked += tables.iter().flatten().count();
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships a house table");
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_professions_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let Some(path) = find_file_ignoring_case(&root, PROFESSIONS_FILE) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let mut file = ProfessionsFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {PROFESSIONS_FILE}: {error}"));
        assert_eq!(file.to_bytes(), bytes, "{client} {PROFESSIONS_FILE}");
        let professions = file.professions().unwrap();
        assert!(professions.iter().all(|profession| !profession.name.is_empty() && profession.name_id != 0 && !profession.skills.is_empty()));
        let names: Vec<&str> = professions.iter().map(|profession| profession.name.as_str()).collect();
        println!("{client}: {PROFESSIONS_FILE} ({names:?}) written back byte for byte");

        let gump = file.document.records_mut().find_map(|line| match line {
            ProfessionLine::Gump(gump) => Some(gump),
            _ => None,
        });
        *gump.unwrap() = EDITED_GUMP;
        let edited = file.to_bytes();
        assert_eq!(changed_lines(&bytes, &edited), 1, "{client} {PROFESSIONS_FILE}: only the edited line changes");
        assert_eq!(ProfessionsFile::from_bytes(&edited).unwrap().professions().unwrap()[0].gump, EDITED_GUMP);
        checked += 1;
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships {PROFESSIONS_FILE}");
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_key_names_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let Some(path) = find_file_ignoring_case(&root, KEY_NAMES_FILE) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let mut file = KeyNamesFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {KEY_NAMES_FILE}: {error}"));
        assert_eq!(file.to_bytes(), bytes, "{client} {KEY_NAMES_FILE}");
        assert_eq!(file.len(), KEY_NAME_COUNT, "{client}: one line per scan code");
        println!("{client}: {KEY_NAMES_FILE} ({} names, scan code 0 `{}`) written back byte for byte", file.len(), file.name(0).unwrap());
        file.set_name(1, EDITED_KEY_NAME).unwrap();
        let edited = file.to_bytes();
        assert_eq!(changed_lines(&bytes, &edited), 1, "{client} {KEY_NAMES_FILE}: only the edited line changes");
        assert_eq!(KeyNamesFile::from_bytes(&edited).unwrap().name(1), Some(EDITED_KEY_NAME));
        checked += 1;
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships {KEY_NAMES_FILE}");
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_unread_tables_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let tables = [
            check_table::<TileTableRecord>(&root, TILE256_FILE, |record| record.value += 1),
            check_table::<TileTableRecord>(&root, TILE1024_FILE, |record| record.value += 1),
        ];
        checked += tables.iter().flatten().count();
        if let Some(path) = find_file_ignoring_case(&root, ARTTABLE_FILE) {
            let bytes = std::fs::read(&path).unwrap();
            let file = ArtTableFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{} {ARTTABLE_FILE}: {error}", label(&root)));
            assert_eq!(file.to_bytes(), bytes, "{} {ARTTABLE_FILE}", label(&root));
            println!("{}: {ARTTABLE_FILE} ({} records) written back byte for byte", label(&root), file.records().count());
            checked += 1;
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships a tile table or {ARTTABLE_FILE}");
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_palette_animinfo_and_sjis_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let read = |name: &str| find_file_ignoring_case(&root, name).map(|path| std::fs::read(path).unwrap());
        if let Some(bytes) = read(PALETTE_FILE) {
            let file = PaletteFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client}: {error:#}"));
            assert_eq!(file.to_bytes(), bytes, "{client} {PALETTE_FILE}");
            checked += 1;
        }
        if let Some(bytes) = read(ANIMINFO_FILE) {
            let file = AnimInfoFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {ANIMINFO_FILE}");
            let defaults = file.entries.iter().filter(|&&entry| entry == ANIMINFO_DEFAULT).count();
            println!("{client}: {ANIMINFO_FILE} ({} entries, {defaults} the default) written back byte for byte", file.entries.len());
            checked += 1;
        }
        if let Some(bytes) = read(SJIS_FILE) {
            let file = ShiftJisToUnicodeFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {SJIS_FILE}");
            let mapped = file.code_units.iter().filter(|&&unit| unit != 0).count();
            println!("{client}: {SJIS_FILE} ({mapped} codes mapped) written back byte for byte");
            checked += 1;
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships {PALETTE_FILE}, {ANIMINFO_FILE} or {SJIS_FILE}");
}
