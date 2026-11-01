//! The string tables of real clients written back byte for byte: every `LocalizedStrings.uop` table (Enhanced Client
//! and Kingdom Reborn), every Classic `Cliloc.*` file (same table format), directly and through KDL, and every
//! `string_dictionary.uop` payload. An edit writes a copy of the package that changes only the edited table.
//!
//! Client directories: `UO_EC_CLIENT_DIR` and/or `UO_CC_CLIENT_DIR` (`common::client_files`), and
//! `UO_KR_CLIENT_DIR` (`common::KR_CLIENT_DIR_ENV`). Temporary packages go to `std::env::temp_dir()` (`TMPDIR`)
//! and are deleted. Run with `--release`.

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::enhanced::localized_strings::{
    language_of_hash, localized_strings_entry_hash, write_tables, LocalizedStringTable, TablesWrite,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::package::UopPackage;

const LOCALIZED_STRINGS_FILE_NAME: &str = "LocalizedStrings.uop";
const STRING_DICTIONARY_FILE_NAME: &str = "string_dictionary.uop";
/// File-name prefix of the Classic tables (`Cliloc.enu`, ...), matched ignoring ASCII case.
const CLASSIC_CLILOC_PREFIX: &str = "cliloc.";
/// Tables of the reference clients: eight Enhanced Client languages, three Kingdom Reborn ones.
const EC_TABLES: usize = 8;
const KR_TABLES: usize = 3;
/// The string the edit test changes: cliloc 500000 of the English table (language id 1).
const ENGLISH_LANGUAGE_ID: u32 = 1;
const EDITED_STRING_ID: u32 = 500_000;

/// Every sized entry of `path`, by hash.
fn payloads(path: &Path) -> BTreeMap<u64, Vec<u8>> {
    let package = UopPackage::load(path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
    package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| (file.filename_hash(), file.unpack().unwrap()))
        .collect()
}

/// Asserts that `bytes` parse into a table that writes them back, directly and through KDL; returns the strings.
fn assert_table_round_trips(label: &str, bytes: &[u8]) -> usize {
    let table = LocalizedStringTable::from_payload_bytes(bytes).unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(table.to_payload_bytes().unwrap() == bytes, "{label}: the table is written back byte for byte");
    let kdl = table.to_kdl_string();
    let back = LocalizedStringTable::from_kdl_str(label, &kdl).unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(back.to_payload_bytes().unwrap() == bytes, "{label}: the table comes back from KDL byte for byte");
    println!("{label}: {} strings, {} bytes ({} KDL bytes), byte for byte directly and through KDL", table.len(), bytes.len(), kdl.len());
    table.len()
}

fn check_localized_strings_package(label: &str, path: &Path, expected_tables: usize) {
    let tables = payloads(path);
    for (&hash, bytes) in &tables {
        let language = language_of_hash(hash).unwrap_or_else(|| panic!("{label}: table {hash:016X} has no known language"));
        assert_table_round_trips(&format!("{label} {:03} ({})", language.id, language.code), bytes);
    }
    assert_eq!(tables.len(), expected_tables, "{label}: tables");

    // An unchanged write is a copy; an edit changes only its table.
    let out = common::temp_dir(&format!("localized_strings_{label}"));
    let english_hash = localized_strings_entry_hash(ENGLISH_LANGUAGE_ID);
    let mut english = LocalizedStringTable::from_payload_bytes(&tables[&english_hash]).unwrap();
    let unchanged = out.join("unchanged.uop");
    let write = write_tables(path, &unchanged, &[(english_hash, &english)]).unwrap();
    assert_eq!(write, TablesWrite { unchanged: 1, ..TablesWrite::default() });
    assert!(std::fs::read(&unchanged).unwrap() == std::fs::read(path).unwrap(), "{label}: an unchanged write is a copy");

    let edited_text = format!("{} (edited)", english.get(EDITED_STRING_ID).unwrap_or_default());
    english.set(EDITED_STRING_ID, edited_text.clone());
    let edited = out.join("edited.uop");
    assert_eq!(write_tables(path, &edited, &[(english_hash, &english)]).unwrap().replaced, 1);
    let after = payloads(&edited);
    assert_eq!(after.len(), tables.len());
    for (hash, bytes) in &tables {
        if *hash != english_hash {
            assert!(&after[hash] == bytes, "{label}: table {hash:016X} is untouched");
        }
    }
    let reread = LocalizedStringTable::from_payload_bytes(&after[&english_hash]).unwrap();
    assert_eq!(reread.get(EDITED_STRING_ID), Some(edited_text.as_str()));
    assert_eq!(reread, english);
    println!("{label}: cliloc {EDITED_STRING_ID} edited; the other {} tables untouched", tables.len() - 1);
    for path in [unchanged, edited] {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(&out).unwrap();
}

fn check_string_dictionary(label: &str, path: &Path) {
    for (hash, bytes) in payloads(path) {
        let dictionary = UoStringDictionary::from_payload_bytes(&bytes, label).unwrap();
        assert!(dictionary.to_payload_bytes().unwrap() == bytes, "{label} {hash:016X}: written back byte for byte");
        println!(
            "{label} {hash:016X}: {} strings ({} declared), {} bytes written back byte for byte",
            dictionary.len(),
            dictionary.declared_count(),
            bytes.len()
        );
    }
}

/// The Classic `Cliloc.*` files of `dir`, sorted by name.
fn classic_clilocs(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_file()
                && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                    name.len() > CLASSIC_CLILOC_PREFIX.len()
                        && name[..CLASSIC_CLILOC_PREFIX.len()].eq_ignore_ascii_case(CLASSIC_CLILOC_PREFIX)
                })
        })
        .collect();
    files.sort();
    files
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored --release"]
fn localized_strings_tables_write_back_and_edit() {
    for (env, path) in common::client_files(LOCALIZED_STRINGS_FILE_NAME) {
        check_localized_strings_package(env, &path, EC_TABLES);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_localized_strings_tables_write_back_and_edit() {
    check_localized_strings_package("KR", &common::kr_client_file(LOCALIZED_STRINGS_FILE_NAME), KR_TABLES);
}

#[test]
#[ignore = "needs a real Classic client: set UO_CC_CLIENT_DIR and run with --ignored --release"]
fn classic_cliloc_files_write_back_through_the_table_format() {
    let dir = std::env::var_os(common::CLIENT_DIR_ENVS[1]).unwrap_or_else(|| panic!("set {}", common::CLIENT_DIR_ENVS[1]));
    let files = classic_clilocs(Path::new(&dir));
    assert!(!files.is_empty(), "no Cliloc.* files in {}", Path::new(&dir).display());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_table_round_trips(&name, &std::fs::read(&path).unwrap());
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR (and/or UO_CC_CLIENT_DIR) and run with --ignored --release"]
fn string_dictionaries_write_back_byte_for_byte() {
    for (env, path) in common::client_files(STRING_DICTIONARY_FILE_NAME) {
        check_string_dictionary(env, &path);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_string_dictionary_writes_back_byte_for_byte() {
    check_string_dictionary("KR", &common::kr_client_file(STRING_DICTIONARY_FILE_NAME));
}
