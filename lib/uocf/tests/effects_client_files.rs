//! Reads and rewrites the particle effect files of a real client: the Chaos scripts and the package of
//! `EffectTexture.uop`, and `EffectDefinitionCollection.uop`.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`; directories without the files
//! are skipped. Rewritten packages go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted
//! afterwards.

mod common;

use std::collections::BTreeMap;

use uocf::enhanced::chaos_script::{ChaosIssueSeverity, ChaosScript, ChaosValidationContext};
use uocf::enhanced::effect_definition::{
    encode_effect_definition_collection, EffectCollectionChanges, EffectDefinition, EffectDefinitionCollection,
    EffectEntryKey, EffectIssueKind, EffectValidationContext, SoundEffect, EFFECT_DEFINITION_COLLECTION_UOP_NAME,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::effect_texture::{
    effect_texture_hash, EffectFileKind, EffectTexturePackage, EntryPut, EFFECT_TEXTURE_UOP_NAME,
};
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;

/// Chaos scripts of the reference release (the 2024 clients).
const EXPECTED_CHAOS_SCRIPTS: usize = 61;
/// Size of the client's timestamp data block header (`u16 3`, `u16 8`, `u64` FILETIME).
const CLIENT_TIMESTAMP_HEADER_SIZE: u32 = 12;
/// The repo's UOP name dictionary, relative to this crate.
const REPO_HASH_DICTIONARY: &str = "../../tools/_shared_assets/Dictionary.dic";
/// File name of the string dictionary package.
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Entries of `EffectDefinitionCollection.uop` in the reference release, and how many hash to no id.
const EXPECTED_COLLECTION_ENTRIES: usize = 269;
const EXPECTED_UNNAMED_COLLECTION_ENTRIES: usize = 2;
/// Collection id the rewrite test adds (unused by the client).
const ADDED_COLLECTION_ID: u32 = 65_000;
/// Sound the rewrite test plays in its edits.
const TEST_SOUND_ID: u32 = 43;
/// Name of the entry the rewrite test adds.
const ADDED_EFFECT_FILE: &str = "uocf_test_added.ems";

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn chaos_scripts_round_trip() {
    for (env, path) in common::client_files(EFFECT_TEXTURE_UOP_NAME) {
        let mut package = EffectTexturePackage::load_with_names(path.parent().unwrap()).unwrap();
        let harvested = package.entries().iter().filter(|entry| entry.name.is_some()).count();
        let dictionary_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(REPO_HASH_DICTIONARY);
        if dictionary_path.is_file() {
            let dictionary = HashDictionary::load(&dictionary_path).unwrap();
            let resolved = package.resolve_names_from_dictionary(&dictionary);
            println!("{env}: {harvested} names without a dictionary, {resolved} more from {REPO_HASH_DICTIONARY}");
        }
        let entries = package.entries();
        let with_payload = entries.iter().filter(|entry| entry.has_payload()).count();
        let named = entries.iter().filter(|entry| entry.has_payload() && entry.name.is_some()).count();
        let mut kinds = BTreeMap::new();
        let mut scripts = 0;
        let mut issues = BTreeMap::new();
        for entry in entries.iter().filter(|entry| entry.has_payload()) {
            let payload = package.read_by_hash(entry.hash).unwrap().unwrap();
            let kind = EffectFileKind::detect(&payload);
            *kinds.entry(format!("{kind:?}")).or_insert(0) += 1;
            if kind != EffectFileKind::ChaosScript {
                continue;
            }
            scripts += 1;
            let label = entry.name.clone().unwrap_or_else(|| format!("{:016X}", entry.hash));
            let script = ChaosScript::parse_bytes(&payload).unwrap_or_else(|error| panic!("{env}: {label}: {error:?}"));
            assert_eq!(script.to_lossless_bytes().unwrap(), payload, "{env}: {label}: lossless output");
            let canonical = script.to_canonical_string();
            let reparsed = ChaosScript::parse(&canonical).unwrap_or_else(|error| panic!("{env}: {label}: {error:?}"));
            assert_eq!(reparsed, script, "{env}: {label}: canonical output reparses to the same script");
            let exists = |name: &str| package.contains(name);
            for issue in script.validate(&ChaosValidationContext { file_exists: Some(&exists) }) {
                println!("{env}: {label}: {:?}: {issue}", issue.severity());
                *issues.entry(issue.severity() == ChaosIssueSeverity::Error).or_insert(0) += 1;
            }
        }
        println!("{env}: {with_payload} entries with a payload, {named} named; kinds {kinds:?}");
        println!("{env}: {scripts} Chaos scripts round-trip; issues (error?: count) {issues:?}");
        assert_eq!(scripts, EXPECTED_CHAOS_SCRIPTS, "{env}");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn effect_texture_package_rewrite() {
    for (env, path) in common::client_files(EFFECT_TEXTURE_UOP_NAME) {
        let mut package = EffectTexturePackage::load_with_names(path.parent().unwrap()).unwrap();
        let entries = package.entries();
        let scripts = entries
            .iter()
            .filter(|entry| entry.name.as_deref().is_some_and(|name| name.ends_with(".ems")))
            .collect::<Vec<_>>();
        let (edited, removed) = (scripts[0].clone(), scripts[1].clone());
        let edited_name = edited.file_name().unwrap().to_owned();
        let removed_name = removed.file_name().unwrap().to_owned();

        let mut script = ChaosScript::parse_bytes(&package.read(&edited_name).unwrap().unwrap()).unwrap();
        script.name.push_str("_edited");
        let edited_payload = script.to_lossless_bytes().unwrap();
        let added_payload = script.to_canonical_bytes().unwrap();
        assert_eq!(package.put(&edited_name, &edited_payload).unwrap(), EntryPut::Replaced);
        assert_eq!(package.put(ADDED_EFFECT_FILE, &added_payload).unwrap(), EntryPut::Added);
        assert!(package.remove(&removed_name));
        assert!(package.save_as(&path).is_err(), "{env}: the source package is never overwritten");

        let dir = common::temp_dir("effect_texture_rewrite");
        let output = dir.join(EFFECT_TEXTURE_UOP_NAME);
        package.save_as(&output).unwrap();

        let source = UopPackage::load(&path).unwrap();
        let rewritten = UopPackage::load(&output).unwrap();
        assert_eq!(rewritten.file_count(), source.file_count(), "{env}: one added, one removed");
        // The added entry fills an empty slot when the source has one, else takes a new slot; the removed
        // entry leaves an empty slot.
        let new_slots = usize::from(!source.iter_files().any(|file| file.is_empty_slot()));
        assert_eq!(rewritten.iter_files().count(), source.iter_files().count() + new_slots, "{env}: block slots");
        for file in source.iter_files().filter(|file| !file.is_empty_slot()) {
            let hash = file.filename_hash();
            if hash == removed.hash {
                assert!(rewritten.get_file_by_hash(hash).is_none(), "{env}: removed entry");
                continue;
            }
            let copy = rewritten.get_file_by_hash(hash).unwrap();
            assert_eq!(copy.compression(), file.compression(), "{env}: {hash:016X}");
            assert_eq!(copy.data_block_header(), file.data_block_header(), "{env}: {hash:016X}");
            assert_eq!(copy.data_block_hash(), file.data_block_hash(), "{env}: {hash:016X}");
            if hash == edited.hash {
                assert_eq!(copy.unpack().unwrap(), edited_payload, "{env}: edited entry");
            } else {
                assert_eq!(copy.data(), file.data(), "{env}: {hash:016X} keeps its payload bytes");
            }
        }
        let added = rewritten.get_file_by_hash(effect_texture_hash(ADDED_EFFECT_FILE)).unwrap();
        assert_eq!(added.unpack().unwrap(), added_payload, "{env}: added entry");
        // New entries get a timestamp header only in packages with timestamp headers; EffectTexture.uop has the
        // longer (signature?) headers, which cannot be made, so its new entries have none.
        let timestamp_headers =
            source.iter_files().any(|file| file.data_block_length() == CLIENT_TIMESTAMP_HEADER_SIZE);
        let expected_header = if timestamp_headers { CLIENT_TIMESTAMP_HEADER_SIZE } else { 0 };
        assert_eq!(added.data_block_length(), expected_header, "{env}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn effect_definition_collection_round_trip() {
    for (env, path) in common::client_files(EFFECT_DEFINITION_COLLECTION_UOP_NAME) {
        let dir = path.parent().unwrap();
        let package = UopPackage::load(&path).unwrap();
        let collection = EffectDefinitionCollection::from_package(&package).unwrap();
        for entry in &collection.entries {
            let payload = package.unpack_file_by_hash(entry.key.hash()).unwrap().unwrap();
            assert_eq!(encode_effect_definition_collection(&entry.definitions), payload, "{env}: {:?}", entry.key);
        }
        let unnamed = collection.entries.iter().filter(|entry| entry.key.id().is_none()).count();
        println!("{env}: {} entries re-serialize byte for byte, {unnamed} without an id", collection.entries.len());
        assert_eq!(collection.entries.len(), EXPECTED_COLLECTION_ENTRIES, "{env}");
        assert_eq!(unnamed, EXPECTED_UNNAMED_COLLECTION_ENTRIES, "{env}");

        let strings = UoStringDictionary::load(&dir.join(STRING_DICTIONARY_UOP_NAME)).unwrap();
        let text = collection.to_kdl_string(Some(&strings));
        let parsed = EffectDefinitionCollection::from_kdl_str(&text, Some(&strings)).unwrap();
        assert_eq!(parsed, collection, "{env}: KDL round trip");
        let without_names = EffectDefinitionCollection::from_kdl_str(&collection.to_kdl_string(None), None).unwrap();
        assert_eq!(without_names, collection, "{env}: KDL round trip without names");
        println!("{env}: KDL round trip ok ({} lines)", text.lines().count());

        let textures = EffectTexturePackage::load_from_client_dir(dir).unwrap();
        let exists = |name: &str| textures.contains(name);
        let issues = collection
            .validate(&EffectValidationContext { strings: Some(&strings), particle_file_exists: Some(&exists) });
        for issue in &issues {
            println!("{env}: {:?}: {issue}", issue.severity());
        }
        let missing = issues.iter().filter(|issue| matches!(issue.kind, EffectIssueKind::MissingParticleFile { .. }));
        assert_eq!(missing.count(), issues.len(), "{env}: only missing particle files are reported");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn effect_definition_collection_rewrite() {
    for (env, path) in common::client_files(EFFECT_DEFINITION_COLLECTION_UOP_NAME) {
        let source = UopPackage::load(&path).unwrap();
        let mut collection = EffectDefinitionCollection::from_package(&source).unwrap();
        let dir = common::temp_dir("effect_collection_rewrite");

        let unchanged = dir.join("unchanged.uop");
        let changes = collection.write_effect_definition_collection(&path, &unchanged).unwrap();
        assert_eq!(changes, EffectCollectionChanges::default(), "{env}");
        let identical = std::fs::read(&unchanged).unwrap() == std::fs::read(&path).unwrap();
        println!("{env}: unchanged rewrite is byte-identical to the source: {identical}");

        let ids = collection.ids().collect::<Vec<_>>();
        let (edited, removed) = (ids[1], ids[2]);
        let sound = EffectDefinition::Sound(SoundEffect { sound_id: TEST_SOUND_ID, value_08: 1.0 });
        collection.get_mut(edited).unwrap().push(sound.clone());
        collection.set(EffectEntryKey::Id(ADDED_COLLECTION_ID), vec![sound]);
        collection.remove(EffectEntryKey::Id(removed)).unwrap();
        assert!(collection.write_effect_definition_collection(&path, &path).is_err(), "{env}: never overwrites");

        let output = dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME);
        let changes = collection.write_effect_definition_collection(&path, &output).unwrap();
        assert_eq!(changes, EffectCollectionChanges { replaced: 1, added: 1, removed: 1 }, "{env}");
        let rewritten = UopPackage::load(&output).unwrap();
        assert_eq!(EffectDefinitionCollection::from_package(&rewritten).unwrap(), collection, "{env}");
        for file in source.iter_files() {
            let hash = file.filename_hash();
            if hash == EffectEntryKey::Id(removed).hash() {
                assert!(rewritten.get_file_by_hash(hash).is_none(), "{env}: removed entry");
                continue;
            }
            let copy = rewritten.get_file_by_hash(hash).unwrap();
            assert_eq!(copy.data_block_header(), file.data_block_header(), "{env}: {hash:016X}");
            assert_eq!(copy.data_block_hash(), file.data_block_hash(), "{env}: {hash:016X}");
            if hash != EffectEntryKey::Id(edited).hash() {
                assert_eq!(copy.data(), file.data(), "{env}: {hash:016X} keeps its payload bytes");
            }
        }
        let added = rewritten.get_file_by_hash(EffectEntryKey::Id(ADDED_COLLECTION_ID).hash()).unwrap();
        assert_eq!(added.data_block_length(), CLIENT_TIMESTAMP_HEADER_SIZE, "{env}: the added entry has a header");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
