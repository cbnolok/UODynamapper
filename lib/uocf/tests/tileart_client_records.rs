//! Parses every record of a real Enhanced Client `tileart.uop` and every entry of its
//! `EffectDefinitionCollection.uop`.
//!
//! The expected effect counts are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-effects/verify_effects.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use uocf::enhanced::effect_definition::{
    parse_effect_definition_collection, EffectDefinition, EFFECT_TYPE_EARTHQUAKE, EFFECT_TYPE_LOD,
    EFFECT_TYPE_PARTICLE, EFFECT_TYPE_PROPERTY_FLAG, EFFECT_TYPE_SOUND, EFFECT_TYPE_WAIT,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::uop_container::package::UopPackage;

/// Environment variable naming the Enhanced Client directory that contains `tileart.uop`.
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const TILEART_UOP_NAME: &str = "tileart.uop";
const EFFECT_COLLECTION_UOP_NAME: &str = "EffectDefinitionCollection.uop";
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
/// Particle name extensions: Nif scenes and Chaos effects.
const PARTICLE_NAME_EXTENSIONS: [&str; 2] = [".nif", ".ems"];

/// Tileart records per effect count, records without effects excluded.
const EXPECTED_TILEART_RECORDS_BY_EFFECT_COUNT: [(usize, usize); 2] = [(1, 313), (2, 16)];
const EXPECTED_TILEART_EFFECTS_ALL_LEVELS: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 572), (EFFECT_TYPE_LOD, 1), (EFFECT_TYPE_PROPERTY_FLAG, 23)];
const EXPECTED_TILEART_EFFECTS_TOP_LEVEL: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 321), (EFFECT_TYPE_LOD, 1), (EFFECT_TYPE_PROPERTY_FLAG, 23)];
const EXPECTED_TILEART_PARTICLE_NAMES: usize = 202;

const EXPECTED_COLLECTION_ENTRIES: usize = 269;
const EXPECTED_COLLECTION_TOP_LEVEL_DEFINITIONS: usize = 280;
const EXPECTED_COLLECTION_EFFECTS_ALL_LEVELS: [(u32, usize); 4] = [
    (EFFECT_TYPE_PARTICLE, 279), (EFFECT_TYPE_SOUND, 2), (EFFECT_TYPE_WAIT, 1), (EFFECT_TYPE_EARTHQUAKE, 1),
];
const EXPECTED_COLLECTION_EFFECTS_TOP_LEVEL: [(u32, usize); 3] =
    [(EFFECT_TYPE_PARTICLE, 278), (EFFECT_TYPE_WAIT, 1), (EFFECT_TYPE_EARTHQUAKE, 1)];
const EXPECTED_COLLECTION_PARTICLE_NAMES: usize = 258;

fn client_dir() -> PathBuf {
    let client_dir = std::env::var(EC_CLIENT_DIR_ENV)
        .unwrap_or_else(|_| panic!("{EC_CLIENT_DIR_ENV} must name the Enhanced Client directory"));
    PathBuf::from(client_dir)
}

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

    fn assert_names(&self, expected_distinct: usize) {
        assert!(self.unresolved_names.is_empty(), "unresolved particle names: {:?}", self.unresolved_names);
        let not_particles: Vec<_> = self.particle_names.iter()
            .filter(|name| !PARTICLE_NAME_EXTENSIONS.iter().any(|ext| name.to_ascii_lowercase().ends_with(ext)))
            .collect();
        assert!(not_particles.is_empty(), "particle names without a particle extension: {not_particles:?}");
        assert_eq!(self.particle_names.len(), expected_distinct);
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn tileart_parses_every_client_record() {
    let client_dir = client_dir();
    let package = UopPackage::load(client_dir.join(TILEART_UOP_NAME)).expect("load tileart.uop");
    let dictionary = UoStringDictionary::load(&client_dir.join(STRING_DICTIONARY_UOP_NAME))
        .expect("load string_dictionary.uop");

    let mut records = 0usize;
    let mut with_sitting = 0usize;
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
        for filter in &entry.appearance_vector {
            *appearance_by_subtype.entry(filter.subtype()).or_default() += 1;
        }
        if !entry.effects.is_empty() {
            *records_by_effect_count.entry(entry.effects.len()).or_default() += 1;
        }
        effects.add(&entry.effects, &dictionary);
    }

    println!("records: {records}");
    println!("with sitting: {with_sitting}");
    println!("appearance filters by subtype: {appearance_by_subtype:?}");
    println!("records by effect count: {records_by_effect_count:?}");
    println!("effects by type key, all levels: {:?}", effects.all_levels);
    println!("effects by type key, top level: {:?}", effects.top_level);
    println!("distinct particle names: {}", effects.particle_names.len());
    assert!(failures.is_empty(), "{} records failed to parse: {failures:?}", failures.len());
    assert_eq!(records_by_effect_count, BTreeMap::from(EXPECTED_TILEART_RECORDS_BY_EFFECT_COUNT));
    assert_eq!(effects.all_levels, BTreeMap::from(EXPECTED_TILEART_EFFECTS_ALL_LEVELS));
    assert_eq!(effects.top_level, BTreeMap::from(EXPECTED_TILEART_EFFECTS_TOP_LEVEL));
    effects.assert_names(EXPECTED_TILEART_PARTICLE_NAMES);
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn effect_definition_collection_parses_every_client_entry() {
    let client_dir = client_dir();
    let package = UopPackage::load(client_dir.join(EFFECT_COLLECTION_UOP_NAME))
        .expect("load EffectDefinitionCollection.uop");
    let dictionary = UoStringDictionary::load(&client_dir.join(STRING_DICTIONARY_UOP_NAME))
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

    println!("entries: {entries}");
    println!("top-level definitions: {top_level_definitions}");
    println!("effects by type key, all levels: {:?}", effects.all_levels);
    println!("effects by type key, top level: {:?}", effects.top_level);
    println!("distinct particle names: {}", effects.particle_names.len());
    assert!(failures.is_empty(), "{} entries failed to parse: {failures:?}", failures.len());
    assert_eq!(entries, EXPECTED_COLLECTION_ENTRIES);
    assert_eq!(top_level_definitions, EXPECTED_COLLECTION_TOP_LEVEL_DEFINITIONS);
    assert_eq!(effects.all_levels, BTreeMap::from(EXPECTED_COLLECTION_EFFECTS_ALL_LEVELS));
    assert_eq!(effects.top_level, BTreeMap::from(EXPECTED_COLLECTION_EFFECTS_TOP_LEVEL));
    effects.assert_names(EXPECTED_COLLECTION_PARTICLE_NAMES);
}
