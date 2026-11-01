//! The generic UOP entry editor over real clients: every shipped entry of every package of the Enhanced Client
//! (`UO_EC_CLIENT_DIR`) and Kingdom Reborn (`UO_KR_CLIENT_DIR`) passes its own replacement check, and an edit of
//! `GameData.uop` keeps the other entries, the flags and an unchanged save byte for byte; every text entry decodes
//! for the text editor and encodes back to its bytes. Copies go under the temporary directory (`TMPDIR`).

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use uocf::audio::validate::FormatPolicy;
use uocf::enhanced::entry_edit::{EntryCheck, EntryTarget, EntryWrite, UopEntryEditor};
use uocf::enhanced::entry_text::EntryText;
use uocf::enhanced::image_census::{classify_content, find_packages, ContentFormat};
use uocf::enhanced::legacy_terrain_map::{LegacyTerrainMap, LegacyTerrainRoute, UOP_ENTRY_PATH as LEGACY_TERRAIN_MAP_ENTRY};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;

/// The client folders, and the audio policy their shipped entries are checked with.
const CLIENT_DIR_ENVS: [(&str, FormatPolicy); 2] = [("UO_EC_CLIENT_DIR", EC_AUDIO_POLICY), ("UO_KR_CLIENT_DIR", KR_AUDIO_POLICY)];
const DICTIONARY: &str = "../../tools/_shared_assets/Dictionary.dic";
const STRING_DICTIONARY_FILE: &str = "string_dictionary.uop";
const GAME_DATA_FILE: &str = "GameData.uop";
/// Every shipped Enhanced Client file is one its decoder (Miles 7.2c) is verified to play.
const EC_AUDIO_POLICY: FormatPolicy = FormatPolicy::VerifiedOnly;
/// Kingdom Reborn plays with Miles 7.0x, and some of its music ends in a cut last frame, which 7.2c's handling is
/// not verified for: its shipped entries are checked with the unverified formats allowed (a file Miles refuses,
/// or of another kind, is refused under both policies).
const KR_AUDIO_POLICY: FormatPolicy = FormatPolicy::AllowUnverified;
/// How many failures the report prints.
const SHOWN_FAILURES: usize = 20;

/// The repository dictionary plus the strings of the client's string dictionary.
fn names(packages: &[PathBuf]) -> HashDictionary {
    let mut names = HashDictionary::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(DICTIONARY)).unwrap();
    for path in packages.iter().filter(|path| path.file_name().unwrap() == STRING_DICTIONARY_FILE) {
        for (_, string) in UoStringDictionary::load(path).unwrap().iter() {
            let hash = hash_file_name_single(string);
            if names.resolve(hash).is_none() {
                names.set(hash, string);
            }
        }
    }
    names
}

fn check_label(check: &EntryCheck) -> String {
    match check {
        EntryCheck::Parsed { reader, dialect: Some(dialect) } => format!("parsed: {reader} ({})", dialect.label()),
        EntryCheck::Parsed { reader, dialect: None } => format!("parsed: {reader}"),
        EntryCheck::Kind(kind) => format!("kind: {}", kind.label()),
        EntryCheck::Unchecked(kind) => format!("unchecked: {}", kind.label()),
    }
}

/// Every entry, given back as its own replacement, passes the kind, parse and audio checks; the checks applied
/// are printed per package.
#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn every_shipped_entry_passes_its_own_check() {
    let dirs: Vec<(&str, FormatPolicy, PathBuf)> = CLIENT_DIR_ENVS
        .into_iter()
        .filter_map(|(env, policy)| std::env::var_os(env).map(|dir| (env, policy, PathBuf::from(dir))))
        .collect();
    assert!(!dirs.is_empty(), "set {}", CLIENT_DIR_ENVS.map(|(env, _)| env).join(" and/or "));
    let mut failures = Vec::new();
    for (env, audio_policy, dir) in dirs {
        let packages = find_packages(&dir).unwrap();
        let names = names(&packages);
        for path in &packages {
            let editor = UopEntryEditor::load(path).unwrap();
            let hashes: Vec<u64> =
                editor.package().iter_files().filter(|file| !file.is_empty_slot()).map(|file| file.filename_hash()).collect();
            let results: Vec<(String, Result<EntryCheck, String>)> = hashes
                .par_iter()
                .map(|&hash| {
                    // The dictionary holds a few names under hashes they do not hash to (`da`): they name nothing.
                    let target = match names.resolve(hash).filter(|name| hash_file_name_single(name) == hash) {
                        Some(name) => EntryTarget::Name(name.to_owned()),
                        None => EntryTarget::Hash(hash),
                    };
                    let payload = editor.package().unpack_file_by_hash(hash).unwrap().unwrap();
                    let label = target.name().map_or_else(|| format!("{hash:016X}"), str::to_owned);
                    (label, editor.check_replacement(&target, &payload, audio_policy).map_err(|error| format!("{error:#}")))
                })
                .collect();
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for (label, result) in results {
                match result {
                    Ok(check) => *counts.entry(check_label(&check)).or_default() += 1,
                    Err(error) => failures.push(format!("{env} {}: {label}: {error}", path.display())),
                }
            }
            println!("{env} {}: {counts:?}", path.file_name().unwrap().to_string_lossy());
        }
    }
    for failure in failures.iter().take(SHOWN_FAILURES) {
        println!("FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} entries fail their own check", failures.len());
}

/// Replaces and adds `GameData.uop` entries: an unchanged save is byte-identical, a replaced entry keeps its flag,
/// an added one gets the flag of the package's text, the other entries keep their payloads, and a file of another
/// kind or a legacy terrain map that does not parse is refused.
#[test]
#[ignore = "needs the Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn game_data_entries_are_replaced_and_added() {
    let dir = PathBuf::from(std::env::var_os(CLIENT_DIR_ENVS[0].0).unwrap_or_else(|| panic!("set {}", CLIENT_DIR_ENVS[0].0)));
    let source = dir.join(GAME_DATA_FILE);
    let out = common::temp_dir("entry_editor_game_data");
    let policy = FormatPolicy::VerifiedOnly;

    let mut editor = UopEntryEditor::load(&source).unwrap();
    let map = EntryTarget::Name(LEGACY_TERRAIN_MAP_ENTRY.to_owned());
    let map_bytes = editor.package().unpack_file_by_hash(map.hash()).unwrap().unwrap();
    assert_eq!(editor.replace(&map, &map_bytes, policy).unwrap().write, EntryWrite::Unchanged);
    let unchanged = out.join("GameData.unchanged.uop");
    editor.save_as(&unchanged).unwrap();
    assert!(std::fs::read(&unchanged).unwrap() == std::fs::read(&source).unwrap(), "an unchanged save is byte-identical");

    let mut editor = UopEntryEditor::load(&source).unwrap();
    let before: BTreeMap<u64, Vec<u8>> = editor
        .package()
        .iter_files()
        .filter(|file| !file.is_empty_slot())
        .map(|file| (file.filename_hash(), editor.package().unpack_file_by_hash(file.filename_hash()).unwrap().unwrap()))
        .collect();
    let flag = editor.package().get_file_by_hash(map.hash()).unwrap().compression();
    let error = editor.replace(&map, b"not,a\r\nterrain map\r\n", policy).unwrap_err();
    assert!(format!("{error:#}").contains("does not parse"), "{error:#}");
    assert!(format!("{:#}", editor.replace(&map, b"<?xml version=\"1.0\"?><a/>", policy).unwrap_err()).contains("never stored"));
    let blank_line = [map_bytes.as_slice(), b"\r\n"].concat();
    assert!(format!("{:#}", editor.replace(&map, &blank_line, policy).unwrap_err()).contains("not in the shipped format"));
    let mut routes = LegacyTerrainMap::from_bytes(&map_bytes).unwrap();
    let first = *routes.routes.values().next().unwrap();
    routes.set_route(LegacyTerrainRoute { subtype: first.subtype + 1, ..first });
    let edited = routes.to_csv_bytes();
    let put = editor.replace(&map, &edited, policy).unwrap();
    assert!(matches!(put.check, EntryCheck::Parsed { .. }), "{:?}", put.check);
    assert_eq!((put.write, put.compression), (EntryWrite::Replaced, flag));
    let added_name = "data/gamedata/uocf_test_added.csv";
    let added = editor.add(&EntryTarget::Name(added_name.to_owned()), b"a,b\r\n1,2\r\n", None, policy).unwrap();
    assert_eq!((added.write, added.compression), (EntryWrite::Added, CompressionFlag::Zlib), "the text of GameData is zlib");
    let output = out.join("GameData.edited.uop");
    editor.save_as(&output).unwrap();

    let saved = UopPackage::load(&output).unwrap();
    for (hash, payload) in &before {
        let expected = if *hash == map.hash() { &edited } else { payload };
        assert!(saved.unpack_file_by_hash(*hash).unwrap().as_ref() == Some(expected), "entry {hash:016X}");
    }
    assert_eq!(saved.get_file_by_hash(map.hash()).unwrap().compression(), flag);
    assert_eq!(saved.unpack_file_by_hash(hash_file_name_single(added_name)).unwrap().unwrap(), b"a,b\r\n1,2\r\n");
    let _ = std::fs::remove_dir_all(&out);
}

/// Every text, XML and UTF-16 text entry, and every binary one that is Windows-1252 text, decodes as the text
/// editor shows it and encodes back to its bytes; the encodings and line endings are printed per client.
#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn every_text_entry_encodes_back_to_its_bytes() {
    const TEXT_KINDS: [ContentFormat; 3] = [ContentFormat::Text, ContentFormat::Xml, ContentFormat::Utf16Text];
    let dirs: Vec<(&str, PathBuf)> =
        CLIENT_DIR_ENVS.into_iter().filter_map(|(env, _)| std::env::var_os(env).map(|dir| (env, PathBuf::from(dir)))).collect();
    assert!(!dirs.is_empty(), "set {}", CLIENT_DIR_ENVS.map(|(env, _)| env).join(" and/or "));
    let mut failures = Vec::new();
    for (env, dir) in dirs {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for path in find_packages(&dir).unwrap() {
            let package = UopPackage::load(&path).unwrap();
            for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
                let payload = package.unpack_file_by_hash(file.filename_hash()).unwrap().unwrap();
                let kind = classify_content(&payload).format;
                if !TEXT_KINDS.contains(&kind) && kind != ContentFormat::Binary {
                    continue;
                }
                let label = format!("{env} {}: {:016X}", path.file_name().unwrap().to_string_lossy(), file.filename_hash());
                match EntryText::decode(&payload) {
                    None if kind == ContentFormat::Binary => {}
                    Some(text) if text.encode_unchanged() == payload => *counts.entry(text.label()).or_default() += 1,
                    Some(text) => failures.push(format!("{label}: {} does not encode back", text.label())),
                    None => failures.push(format!("{label}: does not decode")),
                }
            }
        }
        println!("{env}: {counts:?}");
    }
    for failure in failures.iter().take(SHOWN_FAILURES) {
        println!("FAIL {failure}");
    }
    assert!(failures.is_empty(), "{} text entries fail", failures.len());
}
