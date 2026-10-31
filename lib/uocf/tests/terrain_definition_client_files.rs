//! Every definition of a real client `TerrainDefinition.uop` (Enhanced Client and Kingdom Reborn) parses and
//! writes back byte for byte, passes the client checks, and covers every `(material, subtype)` its sectors and
//! `legacyterrainmap.csv` name; a rewritten package changes only the edited entry.
//!
//! The client directories come from `UO_EC_CLIENT_DIR` and `UO_KR_CLIENT_DIR`. The rewritten package goes to
//! `std::env::temp_dir()` (set `TMPDIR` to choose it) and is deleted afterwards.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
use uocf::enhanced::facet_sector::{facet_package_file_name, CLIENT_FACET_DIMENSIONS};
use uocf::enhanced::legacy_terrain_map::LegacyTerrainMap;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::terrain_definition::*;
use uocf::uop_container::package::UopPackage;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const EC_DEFINITION_COUNT: usize = 201;
const KR_DEFINITION_COUNT: usize = 191;
const STRING_DICTIONARY_UOP_NAME: &str = "string_dictionary.uop";
const GAME_DATA_UOP_NAME: &str = "GameData.uop";
/// Material 0 and its name in both clients.
const VOID_MATERIAL: u32 = 0;
const VOID_MATERIAL_NAME: &str = "void_no_draw";
/// The void definition's subtype-0 flags in the Enhanced Client: no-draw and ignored.
const VOID_FLAGS: u64 = TERRAIN_FLAG_NO_DRAW | TERRAIN_FLAG_IGNORED;
/// Added to the coverage exponent of the edited definition.
const EDITED_EXPONENT_DELTA: f32 = 1.0;

fn client_dir(env: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os(env).unwrap_or_else(|| panic!("set {env} to a client directory")))
}

/// Parses every entry of the package in `dir` in `dialect`, checks the byte-identical round trip and returns the
/// definitions by id.
fn round_trip(dir: &Path, dialect: ClientDialect, expected_count: usize) -> BTreeMap<u32, TerrainDefinitionEntry> {
    let path = dir.join(TERRAIN_DEFINITION_UOP_NAME);
    let package = UopPackage::load(&path).expect("load TerrainDefinition.uop");
    assert_eq!(detect_terrain_definition_dialect(&package).unwrap(), dialect);
    let dictionary = UoStringDictionary::load(&dir.join(STRING_DICTIONARY_UOP_NAME)).expect("load string_dictionary.uop");
    let definitions = read_terrain_definitions(&package, Some(&dictionary), dialect).expect("every entry parses");
    let mut identical = 0;
    for file in package.iter_files().filter(|file| file.has_size()) {
        let payload = file.unpack().unwrap();
        let entry = TerrainDefinitionEntry::from_bytes_dialect(&payload, dialect).unwrap();
        assert!(entry.to_bytes_dialect(dialect).unwrap() == payload, "material {} writes back differently", entry.id);
        identical += 1;
    }
    println!("{dialect:?}: {identical}/{} definitions write back byte for byte", definitions.len());
    assert_eq!(identical, expected_count);
    assert_eq!(definitions.len(), expected_count);
    assert_eq!(definitions[&VOID_MATERIAL].name.as_deref(), Some(VOID_MATERIAL_NAME));
    let issues = validate_terrain_definitions(&definitions);
    assert!(issues.is_empty(), "{dialect:?}: {issues:?}");

    let mut flag_bits = 0u64;
    let mut shaders = BTreeMap::<String, usize>::new();
    for definition in definitions.values() {
        flag_bits |= definition.aliases.iter().fold(0, |bits, alias| bits | alias.tile_flags);
        let texture = definition.texture.as_ref().unwrap();
        *shaders.entry(texture.shader_name.clone().unwrap_or_default()).or_default() += 1;
    }
    let named = TERRAIN_FLAG_NAMES.iter().fold(0, |bits, flag| bits | flag.mask_in(dialect));
    println!("{dialect:?}: flag bits {flag_bits:#x}, unnamed {:#x}; shaders {shaders:?}", flag_bits & !named);
    assert_eq!(flag_bits & !named, 0, "{dialect:?}: alias flags outside the named bits");
    definitions
}

/// The `(material, subtype)` references of every facet package in `dir` and of its `legacyterrainmap.csv`.
fn client_references(dir: &Path, map: Option<&LegacyTerrainMap>, dialect: ClientDialect) -> TerrainReferences {
    let mut references = TerrainReferences::default();
    if let Some(map) = map {
        references.add_legacy_terrain_map(map);
    }
    let empty_map = LegacyTerrainMap { routes: BTreeMap::new(), source_csv: Vec::new() };
    for facet in 0..CLIENT_FACET_DIMENSIONS.len() as u8 {
        let path = dir.join(facet_package_file_name(facet, false));
        if !path.is_file() {
            continue;
        }
        let package = UopPackage::load(&path).unwrap();
        assert_eq!(detect_facet_dialect(&package, facet).unwrap(), dialect, "facet {facet}");
        let facet_package = EcFacetPackage::from_package_dialect(package, facet, None, dialect).unwrap();
        facet_package
            .for_each_sector(|_, sector| {
                references.add_sector(sector, map.unwrap_or(&empty_map));
                Ok(())
            })
            .unwrap();
    }
    references
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn enhanced_definitions_write_back_byte_for_byte() {
    let dir = client_dir(EC_CLIENT_DIR_ENV);
    let definitions = round_trip(&dir, ClientDialect::Enhanced, EC_DEFINITION_COUNT);
    assert_eq!(definitions[&VOID_MATERIAL].alias(BASE_SUBTYPE).unwrap().tile_flags, VOID_FLAGS);
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored"]
fn kingdom_reborn_definitions_write_back_byte_for_byte() {
    let dir = client_dir(common::KR_CLIENT_DIR_ENV);
    let definitions = round_trip(&dir, ClientDialect::KingdomReborn, KR_DEFINITION_COUNT);
    let void = definitions[&VOID_MATERIAL].kr_to_enhanced();
    assert_eq!(void.alias(BASE_SUBTYPE).unwrap().tile_flags, VOID_FLAGS);
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn enhanced_sectors_and_csv_name_only_defined_terrain() {
    let dir = client_dir(EC_CLIENT_DIR_ENV);
    let package = UopPackage::load(dir.join(TERRAIN_DEFINITION_UOP_NAME)).unwrap();
    let definitions = read_terrain_definitions(&package, None, ClientDialect::Enhanced).unwrap();
    let map = LegacyTerrainMap::load(dir.join(GAME_DATA_UOP_NAME)).unwrap();
    let references = client_references(&dir, Some(&map), ClientDialect::Enhanced);
    let issues = check_terrain_references(&definitions, &references);
    println!(
        "{} (material, subtype) pairs, unrouted Classic land ids (id: squares) {:?}; issues: {issues:?}",
        references.uses.len(),
        references.unrouted_legacy_land
    );
    assert!(issues.is_empty());
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored"]
fn kingdom_reborn_sectors_name_only_defined_terrain() {
    let dir = client_dir(common::KR_CLIENT_DIR_ENV);
    let package = UopPackage::load(dir.join(TERRAIN_DEFINITION_UOP_NAME)).unwrap();
    let definitions = read_terrain_definitions(&package, None, ClientDialect::KingdomReborn).unwrap();
    let references = client_references(&dir, None, ClientDialect::KingdomReborn);
    let issues = check_terrain_references(&definitions, &references);
    println!("{} (material, subtype) pairs; issues: {issues:?}", references.uses.len());
    assert!(issues.is_empty());
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn rewritten_package_changes_only_the_edited_definition() {
    let dir = client_dir(EC_CLIENT_DIR_ENV);
    let source = dir.join(TERRAIN_DEFINITION_UOP_NAME);
    let package = UopPackage::load(&source).unwrap();
    let definitions = read_terrain_definitions(&package, None, ClientDialect::Enhanced).unwrap();
    let mut edited = definitions.values().last().unwrap().clone();
    edited.set_coverage_exponent(edited.coverage_exponent() + EDITED_EXPONENT_DELTA);
    let output = common::temp_dir("terrain_definition_rewrite").join(TERRAIN_DEFINITION_UOP_NAME);

    let unchanged = definitions.values().cloned().collect::<Vec<_>>();
    assert!(write_terrain_definition_package(&source, &source, &unchanged, &[], ClientDialect::Enhanced).is_err());
    let changes =
        write_terrain_definition_package(&source, &output, &[edited.clone()], &[], ClientDialect::Enhanced).unwrap();
    assert_eq!(changes, TerrainDefinitionChanges { replaced: 1, added: 0, removed: 0 });

    let rewritten = UopPackage::load(&output).unwrap();
    let edited_hash = terrain_definition_entry_hash(edited.id);
    for file in package.iter_files().filter(|file| file.has_size()) {
        let hash = file.filename_hash();
        let after = rewritten.unpack_file_by_hash(hash).unwrap().unwrap();
        if hash == edited_hash {
            assert_eq!(TerrainDefinitionEntry::from_bytes(&after).unwrap(), edited);
        } else {
            assert!(after == file.unpack().unwrap(), "entry {hash:016X} changed");
        }
    }
    let _ = std::fs::remove_dir_all(output.parent().unwrap());
}
