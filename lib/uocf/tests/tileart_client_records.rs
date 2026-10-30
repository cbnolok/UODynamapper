//! Parses every record of a real client `tileart.uop` (the Enhanced and Classic clients ship the same payloads)
//! and every entry of an Enhanced Client `EffectDefinitionCollection.uop`.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`; a directory without a file is
//! skipped. Every record must parse in any release. The expected effect counts are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-effects/verify_effects.py` on the reference release (the 2024
//! clients); they are asserted only when the record or entry count identifies that release.
//!
//! The tileart writer tests check that every record re-encodes byte for byte, and that a package rewritten with an edited, an added and a removed record changes only those entries. The
//! rewritten package goes to `std::env::temp_dir()` (set `TMPDIR` to choose it) and is deleted afterwards.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use uocf::enhanced::effect_definition::{
    parse_effect_definition_collection, EffectDefinition, EFFECT_TYPE_EARTHQUAKE, EFFECT_TYPE_LOD,
    EFFECT_TYPE_PARTICLE, EFFECT_TYPE_PROPERTY_FLAG, EFFECT_TYPE_SOUND, EFFECT_TYPE_WAIT,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::{tileart_entry_hash, write_tileart_package, TileArtChanges, TileArtEntry};
use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderRecord};
use uocf::uop_container::package::UopPackage;

const TILEART_UOP_NAME: &str = "tileart.uop";
const EFFECT_COLLECTION_UOP_NAME: &str = "EffectDefinitionCollection.uop";
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Particle name extensions: Nif scenes and Chaos effects.
const PARTICLE_NAME_EXTENSIONS: [&str; 2] = [".nif", ".ems"];

const EXPECTED_TILEART_RECORDS: usize = 37_756;
/// Tileart records per effect count, records without effects excluded.
const EXPECTED_TILEART_RECORDS_BY_EFFECT_COUNT: [(usize, usize); 2] = [(1, 313), (2, 16)];
const EXPECTED_TILEART_EFFECTS_ALL_LEVELS: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 572), (EFFECT_TYPE_LOD, 1), (EFFECT_TYPE_PROPERTY_FLAG, 23)];
const EXPECTED_TILEART_EFFECTS_TOP_LEVEL: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 321), (EFFECT_TYPE_LOD, 1), (EFFECT_TYPE_PROPERTY_FLAG, 23)];
const EXPECTED_TILEART_PARTICLE_NAMES: usize = 202;
/// Records with `alternate_animation` set (gargoyle two-handed weapons, halberds, creature placeholders).
const EXPECTED_TILEART_ALTERNATE_ANIMATION_RECORDS: usize = 30;

/// Records whose entry is named after another tile id than the one they store (all names are
/// `build/tileart/%08d.bin`, checked by brute force over the ids up to `u32::MAX / 8`).
const EXPECTED_TILEART_RECORDS_NAMED_AFTER_ANOTHER_TILE: usize = 25;
/// Tile id given to the record added to the rewritten package (a free id above the client's art ids).
const REWRITTEN_ADDED_TILE_ID: u32 = 0x10000 - 1;
/// Body type written into the record edited in the rewritten package.
const REWRITTEN_BODY_TYPE: u32 = 666;

const EXPECTED_COLLECTION_ENTRIES: usize = 269;
const EXPECTED_COLLECTION_TOP_LEVEL_DEFINITIONS: usize = 280;
const EXPECTED_COLLECTION_EFFECTS_ALL_LEVELS: [(u32, usize); 4] = [
    (EFFECT_TYPE_PARTICLE, 279), (EFFECT_TYPE_SOUND, 2), (EFFECT_TYPE_WAIT, 1), (EFFECT_TYPE_EARTHQUAKE, 1),
];
const EXPECTED_COLLECTION_EFFECTS_TOP_LEVEL: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 278), (EFFECT_TYPE_WAIT, 1), (EFFECT_TYPE_EARTHQUAKE, 1)];
const EXPECTED_COLLECTION_PARTICLE_NAMES: usize = 258;

/// Effect counts by type key and resolved particle names, over all nesting levels.
#[derive(Default)]
struct EffectTally {
    all_levels: BTreeMap<u32, usize>,
    top_level: BTreeMap<u32, usize>,
    particle_names: BTreeSet<String>,
    unresolved_names: Vec<u32>,
}

impl EffectTally {
    fn add(&mut self, definitions: &[EffectDefinition], dictionary: &UoStringDictionary) {
        for definition in definitions {
            *self.top_level.entry(definition.type_key()).or_default() += 1;
            definition.visit(&mut |nested| {
                *self.all_levels.entry(nested.type_key()).or_default() += 1;
                if let EffectDefinition::Particle(particle) = nested {
                    match particle.name(dictionary) {
                        Some(name) => { self.particle_names.insert(name.to_owned()); }
                        None => self.unresolved_names.push(particle.name_index),
                    }
                }
            });
        }
    }

    fn assert_names(&self) {
        assert!(self.unresolved_names.is_empty(), "unresolved particle names: {:?}", self.unresolved_names);
        let not_particles: Vec<_> = self.particle_names.iter()
            .filter(|name| !PARTICLE_NAME_EXTENSIONS.iter().any(|ext| name.to_ascii_lowercase().ends_with(ext)))
            .collect();
        assert!(not_particles.is_empty(), "particle names without a particle extension: {not_particles:?}");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn tileart_parses_every_client_record() {
    for (env, path) in common::client_files(TILEART_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load tileart.uop");
        let dictionary = UoStringDictionary::load(&path.with_file_name(STRING_DICTIONARY_UOP_NAME))
            .expect("load string_dictionary.uop");

        let mut records = 0usize;
        let mut with_sitting = 0usize;
        let mut with_alternate_animation = 0usize;
        let mut appearance_by_subtype = BTreeMap::<u8, usize>::new();
        let mut records_by_effect_count = BTreeMap::<usize, usize>::new();
        let mut effects = EffectTally::default();
        let mut failures = Vec::new();
        for file in package.iter_files() {
            records += 1;
            let entry = match TileArtEntry::parse_raw(file) {
                Ok(entry) => entry,
                Err(error) => {
                    failures.push(format!("{:016X}: {error}", file.filename_hash()));
                    continue;
                }
            };
            with_sitting += usize::from(entry.has_sitting);
            with_alternate_animation += usize::from(entry.alternate_animation);
            for filter in &entry.appearance_vector {
                *appearance_by_subtype.entry(filter.subtype()).or_default() += 1;
            }
            if !entry.effects.is_empty() {
                *records_by_effect_count.entry(entry.effects.len()).or_default() += 1;
            }
            effects.add(&entry.effects, &dictionary);
        }

        println!("{env}: records: {records}");
        println!("{env}: with sitting: {with_sitting}");
        println!("{env}: with alternate animation: {with_alternate_animation}");
        println!("{env}: appearance filters by subtype: {appearance_by_subtype:?}");
        println!("{env}: records by effect count: {records_by_effect_count:?}");
        println!("{env}: effects by type key, all levels: {:?}", effects.all_levels);
        println!("{env}: effects by type key, top level: {:?}", effects.top_level);
        println!("{env}: distinct particle names: {}", effects.particle_names.len());
        assert!(failures.is_empty(), "{} records failed to parse: {failures:?}", failures.len());
        effects.assert_names();
        if records != EXPECTED_TILEART_RECORDS {
            println!("{env}: not the reference release, counts not checked");
            continue;
        }
        assert_eq!(with_alternate_animation, EXPECTED_TILEART_ALTERNATE_ANIMATION_RECORDS);
        assert_eq!(records_by_effect_count, BTreeMap::from(EXPECTED_TILEART_RECORDS_BY_EFFECT_COUNT));
        assert_eq!(effects.all_levels, BTreeMap::from(EXPECTED_TILEART_EFFECTS_ALL_LEVELS));
        assert_eq!(effects.top_level, BTreeMap::from(EXPECTED_TILEART_EFFECTS_TOP_LEVEL));
        assert_eq!(effects.particle_names.len(), EXPECTED_TILEART_PARTICLE_NAMES);
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn tileart_reencodes_every_client_record() {
    for (env, path) in common::client_files(TILEART_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load tileart.uop");
        let mut records = 0usize;
        let mut other_names = 0usize;
        let mut failures = Vec::new();
        for file in package.iter_files().filter(|file| file.has_size()) {
            records += 1;
            let payload = file.unpack().expect("unpack");
            let entry = TileArtEntry::parse_bytes(&payload).expect("record parses");
            other_names += usize::from(tileart_entry_hash(entry.tile_id) != file.filename_hash());
            if entry.to_bytes().expect("record encodes") != payload {
                failures.push(format!("{:016X}: tile {} re-encodes differently", file.filename_hash(), entry.tile_id));
            }
        }
        println!("{env}: {records} records re-encode identically; {other_names} store another tile id than their name's");
        assert!(failures.is_empty(), "{} failures: {failures:?}", failures.len());
        if records != EXPECTED_TILEART_RECORDS {
            println!("{env}: not the reference release, counts not checked");
            continue;
        }
        assert_eq!(other_names, EXPECTED_TILEART_RECORDS_NAMED_AFTER_ANOTHER_TILE);
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn rewritten_tileart_changes_only_the_edited_records() {
    for (env, path) in common::client_files(TILEART_UOP_NAME) {
        let source = UopPackage::load(&path).expect("load tileart.uop");
        let mut sized = source.iter_files().filter(|file| file.has_size());
        let (first, second) = (sized.next().expect("a record"), sized.next().expect("a second record"));
        let mut edited = TileArtEntry::parse_raw(first).expect("record parses");
        let removed = TileArtEntry::parse_raw(second).expect("record parses").tile_id;
        let added = TileArtEntry { tile_id: REWRITTEN_ADDED_TILE_ID, ..TileArtEntry::parse_raw(first).expect("record parses") };
        assert!(source.get_file_by_hash(tileart_entry_hash(REWRITTEN_ADDED_TILE_ID)).is_none(), "{env}: id in use");
        edited.body_type = REWRITTEN_BODY_TYPE;
        let dir = common::temp_dir("tileart_rewrite");
        let output = dir.join(TILEART_UOP_NAME);

        let edits = [(first.filename_hash(), edited.clone()), (tileart_entry_hash(REWRITTEN_ADDED_TILE_ID), added.clone())];
        let changes = write_tileart_package(&path, &output, &edits, &[second.filename_hash()])
            .expect("write the new package");

        assert_eq!(changes, TileArtChanges { replaced: 1, added: 1, removed: 1 }, "{env}");
        let written = UopPackage::load(&output).expect("reload the new package");
        let edited_hashes = [first.filename_hash(), second.filename_hash()];
        for source_file in source.iter_files().filter(|file| file.has_size()) {
            let hash = source_file.filename_hash();
            if edited_hashes.contains(&hash) {
                continue;
            }
            let written_file =
                written.get_file_by_hash(hash).unwrap_or_else(|| panic!("{env}: entry {hash:016X} missing"));
            assert!(
                written_file.data() == source_file.data()
                    && written_file.data_block_header() == source_file.data_block_header()
                    && written_file.data_block_hash() == source_file.data_block_hash(),
                "{env}: entry {hash:016X} changed"
            );
        }
        assert!(written.get_file_by_hash(second.filename_hash()).is_none(), "{env}: removed record");
        for (hash, record) in &edits {
            let file = written.get_file_by_hash(*hash).expect("written record");
            assert_eq!(file.unpack().unwrap(), record.to_bytes().unwrap(), "{env}: tile {}", record.tile_id);
            let header = DataBlockHeader::parse(file.data_block_header().unwrap()).unwrap();
            assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{env}: header");
            assert_eq!(file.data_block_hash_matches_header().unwrap(), Some(true), "{env}: hash");
        }
        let source_len = std::fs::metadata(&path).unwrap().len();
        let written_len = std::fs::metadata(&output).unwrap().len();
        println!("{env}: tile {} edited, {removed} removed, {REWRITTEN_ADDED_TILE_ID} added; {source_len} -> {written_len} bytes", edited.tile_id);
        std::fs::remove_dir_all(&dir).expect("delete the new package");
        assert!(write_tileart_package(&path, &path, &[], &[]).is_err(), "{env}: writing over the source must fail");
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn effect_definition_collection_parses_every_client_entry() {
    for (env, path) in common::client_files(EFFECT_COLLECTION_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load EffectDefinitionCollection.uop");
        let dictionary = UoStringDictionary::load(&path.with_file_name(STRING_DICTIONARY_UOP_NAME))
            .expect("load string_dictionary.uop");

        let mut entries = 0usize;
        let mut top_level_definitions = 0usize;
        let mut effects = EffectTally::default();
        let mut failures = Vec::new();
        for file in package.iter_files().filter(|file| file.has_size()) {
            entries += 1;
            let definitions = match file.unpack_arc().map_err(Into::into)
                .and_then(|payload| parse_effect_definition_collection(&payload))
            {
                Ok(definitions) => definitions,
                Err(error) => {
                    failures.push(format!("{:016X}: {error}", file.filename_hash()));
                    continue;
                }
            };
            top_level_definitions += definitions.len();
            effects.add(&definitions, &dictionary);
        }

        println!("{env}: entries: {entries}");
        println!("{env}: top-level definitions: {top_level_definitions}");
        println!("{env}: effects by type key, all levels: {:?}", effects.all_levels);
        println!("{env}: effects by type key, top level: {:?}", effects.top_level);
        println!("{env}: distinct particle names: {}", effects.particle_names.len());
        assert!(failures.is_empty(), "{} entries failed to parse: {failures:?}", failures.len());
        effects.assert_names();
        if entries != EXPECTED_COLLECTION_ENTRIES {
            println!("{env}: not the reference release, counts not checked");
            continue;
        }
        assert_eq!(top_level_definitions, EXPECTED_COLLECTION_TOP_LEVEL_DEFINITIONS);
        assert_eq!(effects.all_levels, BTreeMap::from(EXPECTED_COLLECTION_EFFECTS_ALL_LEVELS));
        assert_eq!(effects.top_level, BTreeMap::from(EXPECTED_COLLECTION_EFFECTS_TOP_LEVEL));
        assert_eq!(effects.particle_names.len(), EXPECTED_COLLECTION_PARTICLE_NAMES);
    }
}
