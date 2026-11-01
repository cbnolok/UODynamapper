//! `credits.dat`, `helpindx.dat` (Classic) and `data/assetmap.xml` (the `MainMisc.uop` entry the clients read, and
//! the loose `Data/assetmap.xml` the 2024 and fp clients also ship) of real clients read strictly and written back
//! byte for byte; an edit reads back and undoing it restores the file.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`; Classic, EC and KR directories
//! all work); a file a client lacks is skipped. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::asset_map::{ASSET_MAP_FILE, ASSET_MAP_UOP_PATH, AssetMap};
use uocf::classic::credits::{CREDITS_FILE, CreditsDirective, CreditsFile, CreditsLine};
use uocf::classic::help_index::{HELP_INDEX_FILE, HelpIndexFile, HelpIndexLine};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const MAIN_MISC_UOP: &str = "MainMisc.uop";
/// The song the credits edit adds.
const ADDED_SONG: i32 = 1;
/// The padding the asset map edit sets.
const EDITED_PADDING: &str = "7";
const PADDING: &str = "padding";

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn credits_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let Some(path) = find_file_ignoring_case(&root, CREDITS_FILE) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let mut file = CreditsFile::load(&path).unwrap_or_else(|error| panic!("{client} {CREDITS_FILE}: {error:#}"));
        assert_eq!(file.to_bytes(), bytes, "{client}: written back byte for byte");
        let directives = file.lines().iter().filter(|line| matches!(line, CreditsLine::Directives(_))).count();
        let line = CreditsLine::directive_line(&[CreditsDirective::Song(ADDED_SONG)]);
        file.insert_line(0, &line).unwrap();
        let back = CreditsFile::from_bytes(&file.to_bytes()).unwrap();
        assert_eq!(back.lines()[0], CreditsLine::Directives(vec![CreditsDirective::Song(ADDED_SONG)]), "{client}");
        file.remove_line(0).unwrap();
        assert_eq!(file.to_bytes(), bytes, "{client}: removing the line restores the file");
        println!("{client}: {CREDITS_FILE} ({} lines, {directives} directive lines) written back byte for byte", file.lines().len());
        checked += 1;
    }
    assert!(checked > 0, "no client ships {CREDITS_FILE}");
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn help_index_writes_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let Some(path) = find_file_ignoring_case(&root, HELP_INDEX_FILE) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let mut file = HelpIndexFile::load(&path).unwrap_or_else(|error| panic!("{client} {HELP_INDEX_FILE}: {error:#}"));
        assert_eq!(file.to_bytes(), bytes, "{client}: written back byte for byte");
        let topics = file.topics().count();
        let mut topic = file.topics().next().expect("a topic").clone();
        let index = file.lines().iter().position(|line| matches!(line, HelpIndexLine::Topic(_))).unwrap();
        let original = topic.clone();
        topic.links.push(original.id);
        file.set_line(index, HelpIndexLine::Topic(topic.clone())).unwrap();
        let back = HelpIndexFile::from_bytes(&file.to_bytes()).unwrap();
        assert_eq!(back.lines()[index], HelpIndexLine::Topic(topic), "{client}");
        file.set_line(index, HelpIndexLine::Topic(original)).unwrap();
        assert_eq!(file.to_bytes(), bytes, "{client}: undoing the edit restores the file");
        println!("{client}: {HELP_INDEX_FILE} ({} lines, {topics} topics) written back byte for byte", file.lines().len());
        checked += 1;
    }
    assert!(checked > 0, "no client ships {HELP_INDEX_FILE}");
}

/// Reads `bytes` as an asset map, checks the round trip and an edit; the mapping count.
fn check_asset_map(source: &str, bytes: &[u8]) -> usize {
    let mut map = AssetMap::from_bytes(bytes).unwrap_or_else(|error| panic!("{source}: {error}"));
    assert_eq!(map.to_bytes(), bytes, "{source}: written back byte for byte");
    let count = map.mappings().len();
    map.set_attribute(0, None, PADDING, EDITED_PADDING).unwrap();
    let back = AssetMap::from_bytes(&map.to_bytes()).unwrap();
    assert_eq!(back.mappings().len(), count, "{source}");
    let restored = AssetMap::from_bytes(bytes).unwrap();
    assert_ne!(back.mappings()[0], restored.mappings()[0], "{source}: the edit reads back");
    println!("{source}: {count} mappings written back byte for byte");
    count
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn asset_maps_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        if let Some(path) = find_file_ignoring_case(&root, MAIN_MISC_UOP) {
            let package = UopPackage::load(&path).unwrap_or_else(|error| panic!("{client} {MAIN_MISC_UOP}: {error}"));
            if let Some(entry) = package.get_file_by_hash(hash_file_name_single(ASSET_MAP_UOP_PATH)) {
                check_asset_map(&format!("{client} {MAIN_MISC_UOP} {ASSET_MAP_UOP_PATH}"), &entry.unpack().unwrap());
                checked += 1;
            }
        }
        if let Some(path) = find_file_ignoring_case(&root, ASSET_MAP_FILE) {
            check_asset_map(&format!("{client} {ASSET_MAP_FILE}"), &std::fs::read(&path).unwrap());
            checked += 1;
        }
    }
    assert!(checked > 0, "no client ships an asset map");
}
