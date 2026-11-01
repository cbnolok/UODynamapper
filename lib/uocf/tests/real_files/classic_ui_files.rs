//! The localized UI string tables (`Intloc*`, `Chat.*`, `Tilehelp.*` and the rest) and `Langcode.iff` of real
//! clients of every era read strictly and write back byte for byte; an edited string reads back.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Run with `--ignored`.

use std::path::{Path, PathBuf};

use uocf::classic::localized_text::{LanguageCodesFile, LocalizedTextFile, is_localized_table_file};
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The text the edit gives the first string.
const EDITED_STRING: &str = "Edited string";

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The files of `root` whose names `keep` accepts, sorted.
fn files(root: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && path.file_name().and_then(|name| name.to_str()).is_some_and(&keep))
        .collect();
    paths.sort();
    paths
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn classic_localized_text_and_language_codes_write_back_byte_for_byte() {
    let mut checked = 0;
    for root in client_roots() {
        let client = label(&root);
        let tables = files(&root, is_localized_table_file);
        let mut strings = 0;
        for path in &tables {
            let name = path.file_name().unwrap().to_string_lossy();
            let bytes = std::fs::read(path).unwrap();
            let mut file = LocalizedTextFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} {name}: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} {name}");
            strings += file.strings.len();
            file.strings[0] = EDITED_STRING.to_owned();
            let back = LocalizedTextFile::from_bytes(&file.to_bytes().unwrap()).unwrap();
            assert_eq!(back, file, "{client} {name}: the edited string");
        }
        println!("{client}: {} string tables ({strings} strings) written back byte for byte", tables.len());
        checked += tables.len();
        if let Some(path) = find_file_ignoring_case(&root, LanguageCodesFile::FILE_NAME) {
            let bytes = std::fs::read(&path).unwrap();
            let file = LanguageCodesFile::from_bytes(&bytes).unwrap_or_else(|error| panic!("{client} Langcode.iff: {error:#}"));
            assert_eq!(file.to_bytes().unwrap(), bytes, "{client} Langcode.iff");
            println!("{client}: {} ({} codes) written back byte for byte", LanguageCodesFile::FILE_NAME, file.codes.len());
            checked += 1;
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} ships a string table");
}
