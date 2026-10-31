//! Re-encodes and rewrites every `AnimationDefinition.uop` under the client roots (the Enhanced Client and Kingdom
//! Reborn ship it; no Classic client does).
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix), searched recursively; they are only
//! read. A package next to `UOKR.exe` is read in the Kingdom Reborn dialect. Every entry must re-encode byte for
//! byte, directly and through KDL (one text, and one file per entry), and validate without errors. The rewrite test
//! checks that an unchanged rewrite is byte-identical and that a rewrite with an edited, an added and a removed entry
//! changes only those. Rewritten packages and KDL files go to `std::env::temp_dir()` (set `TMPDIR` to choose it)
//! and are deleted afterwards.

use crate::common;

use std::path::{Path, PathBuf};

use uocf::enhanced::animation_definition::{
    animation_definitions_from_kdl, AnimationDefinition, AnimationDefinitionChanges, AnimationDefinitionCollection,
    AnimationIssueSeverity, ANIMATION_DEFINITION_UOP_NAME,
};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::effect_definition::EffectValidationContext;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderRecord};
use uocf::uop_container::file::UopFile;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Size of the client's timestamp data block header (`u16 3`, `u16 8`, `u64` FILETIME).
const CLIENT_TIMESTAMP_HEADER_SIZE: u32 = 12;
/// Item id the rewrite test adds (a tile id without an entry in any client).
const ADDED_ITEM_ID: u32 = 0xFFF0;
/// Seconds the rewrite test adds to the frame interval of the first set of the edited entry.
const EDITED_FRAME_INTERVAL_STEP: f32 = 0.125;

/// Every `AnimationDefinition.uop` under the client roots, in path order.
fn packages() -> Vec<PathBuf> {
    fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                collect(&path, found);
            } else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(ANIMATION_DEFINITION_UOP_NAME)) {
                found.push(path);
            }
        }
    }
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let mut found = Vec::new();
    for root in std::env::split_paths(&roots) {
        collect(&root, &mut found);
    }
    assert!(!found.is_empty(), "no {ANIMATION_DEFINITION_UOP_NAME} under {CLIENT_ROOTS_ENV}");
    found
}

fn strings_of(path: &Path) -> Option<UoStringDictionary> {
    let strings = find_file_ignoring_case(path.parent()?, STRING_DICTIONARY_UOP_NAME)?;
    Some(UoStringDictionary::load(&strings).expect("load string_dictionary.uop"))
}

/// An entry a writer replaced or added: one timestamp record hashed with Adler-32.
fn assert_fresh_header(file: &UopFile, context: &str) {
    assert_eq!(file.data_block_length(), CLIENT_TIMESTAMP_HEADER_SIZE, "{context}");
    let header = DataBlockHeader::parse(file.data_block_header().unwrap()).unwrap();
    assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{context}");
    assert_eq!(file.data_block_hash_matches_header().unwrap(), Some(true), "{context}");
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn every_entry_round_trips_binary_and_kdl() {
    for path in packages() {
        let name = path.display();
        let dialect = ClientDialect::of_package_path(&path);
        let package = UopPackage::load(&path).unwrap();
        let collection = AnimationDefinitionCollection::from_package_dialect(&package, dialect).unwrap();
        assert!(collection.unnamed_hashes.is_empty(), "{name}: entries without an item id");
        let mut effects = 0usize;
        for (&item_id, definition) in &collection.entries {
            let payload = package.unpack_file_by_hash(AnimationDefinition::entry_hash(item_id)).unwrap().unwrap();
            assert!(definition.to_bytes_dialect(dialect).unwrap() == payload, "{name}: item {item_id} re-encodes");
            effects += definition.sets.values().flat_map(|set| &set.frames).map(|frame| frame.effects.len()).sum::<usize>();
        }

        let strings = strings_of(&path);
        for strings in [strings.as_ref(), None] {
            let text = collection.to_kdl_string(strings);
            let parsed = animation_definitions_from_kdl(&text, strings).unwrap();
            assert_eq!(parsed.len(), collection.entries.len(), "{name}");
            for (item_id, definition) in &parsed {
                let payload = package.unpack_file_by_hash(AnimationDefinition::entry_hash(*item_id)).unwrap().unwrap();
                assert!(definition.to_bytes_dialect(dialect).unwrap() == payload, "{name}: item {item_id} through KDL");
            }
        }

        let context = EffectValidationContext { strings: strings.as_ref(), particle_file_exists: None };
        let (mut errors, mut warnings) = (0usize, 0usize);
        for (&item_id, definition) in &collection.entries {
            for mut issue in definition.validate(&context) {
                issue.item_id = Some(item_id);
                match issue.severity() {
                    AnimationIssueSeverity::Error => {
                        errors += 1;
                        println!("{name}: error: {issue}");
                    }
                    AnimationIssueSeverity::Warning => warnings += 1,
                }
            }
        }
        let sets: usize = collection.entries.values().map(|definition| definition.sets.len()).sum();
        let frames: usize =
            collection.entries.values().flat_map(|definition| definition.sets.values()).map(|set| set.frames.len()).sum();
        println!(
            "{name} ({}): {} entries, {sets} frame sets, {frames} frames, {effects} frame effects re-encode byte for \
             byte, directly and through KDL (names {}); {warnings} warnings",
            dialect.label(),
            collection.entries.len(),
            if strings.is_some() { "resolved" } else { "unavailable" }
        );
        assert_eq!(errors, 0, "{name}: validation errors");
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn package_rewrite_changes_only_the_edited_entries() {
    for path in packages() {
        let name = path.display();
        let source = UopPackage::load(&path).unwrap();
        let mut collection = AnimationDefinitionCollection::load(&path).unwrap();
        let strings = strings_of(&path);
        let dir = common::temp_dir("animation_definition_rewrite");

        let kdl_dir = dir.join("kdl");
        assert_eq!(collection.export_kdl_files(&kdl_dir, strings.as_ref()).unwrap(), collection.entries.len());
        let mut from_files = AnimationDefinitionCollection::new(collection.dialect);
        let import = from_files.import_kdl_paths(&[kdl_dir], strings.as_ref()).unwrap();
        assert_eq!(import.added.len(), collection.entries.len(), "{name}");
        assert_eq!(from_files.entries, collection.entries, "{name}: KDL files round trip");

        let unchanged = dir.join("unchanged.uop");
        assert_eq!(collection.write_package(&path, &unchanged).unwrap(), AnimationDefinitionChanges::default());
        let identical = std::fs::read(&unchanged).unwrap() == std::fs::read(&path).unwrap();
        assert!(identical, "{name}: an unchanged rewrite is byte-identical to the source");

        let ids: Vec<u32> = collection.ids().collect();
        let (edited, removed) = (ids[0], ids[1]);
        assert!(collection.get(ADDED_ITEM_ID).is_none(), "{name}: the added id is free");
        let definition = collection.get_mut(edited).unwrap();
        definition.sets.values_mut().next().unwrap().frame_interval += EDITED_FRAME_INTERVAL_STEP;
        let added = collection.get(edited).unwrap().clone();
        collection.set(ADDED_ITEM_ID, added);
        collection.remove(removed).unwrap();
        assert!(collection.write_package(&path, &path).is_err(), "{name}: never overwrites");

        let output = dir.join(ANIMATION_DEFINITION_UOP_NAME);
        let changes = collection.write_package(&path, &output).unwrap();
        assert_eq!(changes, AnimationDefinitionChanges { replaced: 1, added: 1, removed: 1 }, "{name}");
        let rewritten = UopPackage::load(&output).unwrap();
        let reread = AnimationDefinitionCollection::from_package_dialect(&rewritten, collection.dialect).unwrap();
        assert_eq!(reread, collection, "{name}");
        let mut kept = 0usize;
        for file in source.iter_files().filter(|file| file.has_size()) {
            let hash = file.filename_hash();
            if hash == AnimationDefinition::entry_hash(removed) {
                assert!(rewritten.get_file_by_hash(hash).is_none(), "{name}: removed entry");
                continue;
            }
            let copy = rewritten.get_file_by_hash(hash).unwrap();
            if hash == AnimationDefinition::entry_hash(edited) {
                assert_fresh_header(copy, &format!("{name}: edited entry"));
                continue;
            }
            assert_eq!(copy.data_block_header(), file.data_block_header(), "{name}: {hash:016X}");
            assert_eq!(copy.data_block_hash(), file.data_block_hash(), "{name}: {hash:016X}");
            assert_eq!(copy.data(), file.data(), "{name}: {hash:016X} keeps its payload bytes");
            kept += 1;
        }
        let added = rewritten.get_file_by_hash(AnimationDefinition::entry_hash(ADDED_ITEM_ID)).unwrap();
        assert_fresh_header(added, &format!("{name}: added entry"));
        println!("{name}: rewrite replaced item {edited}, added {ADDED_ITEM_ID}, removed {removed}, kept {kept} entries");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
