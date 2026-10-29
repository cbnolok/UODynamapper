//! Parses every multi entry and `housing.bin` of a real Enhanced Client `MultiCollection.uop`.
//!
//! The expected counts are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-housing/verify_housing.py`.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use uocf::enhanced::housing::{
    HousingPieces, HOUSING_TILE_TYPE_DOOR, HOUSING_TILE_TYPE_FLOOR, HOUSING_TILE_TYPE_MISC, HOUSING_TILE_TYPE_ROOF,
    HOUSING_TILE_TYPE_STAIRS, HOUSING_TILE_TYPE_TELEPORTER, HOUSING_TILE_TYPE_WALL,
};
use uocf::enhanced::multis::{MultiCollection, MULTI_COLLECTION_UOP_NAME};
use uocf::uop_container::package::UopPackage;

/// Environment variable naming the Enhanced Client directory that contains `MultiCollection.uop`.
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";

const EXPECTED_MULTI_ENTRIES: usize = 862;
const EXPECTED_MULTI_PARTS: usize = 178_808;
const EXPECTED_NOT_DRAWN_PARTS: usize = 4_026;
const EXPECTED_FLAG2_PARTS: usize = 1_359;
const EXPECTED_PART_STRING_REFERENCES: usize = 3_914;
const EXPECTED_PART_Z_RANGE: RangeInclusive<i16> = -8..=123;

const EXPECTED_PIECES_BY_TILE_TYPE: [(u32, usize); 7] = [
    (HOUSING_TILE_TYPE_STAIRS, 19),
    (HOUSING_TILE_TYPE_ROOF, 36),
    (HOUSING_TILE_TYPE_DOOR, 37),
    (HOUSING_TILE_TYPE_FLOOR, 58),
    (HOUSING_TILE_TYPE_WALL, 180),
    (HOUSING_TILE_TYPE_TELEPORTER, 1),
    (HOUSING_TILE_TYPE_MISC, 93),
];
const EXPECTED_HOUSING_PIECES: usize = 424;
/// Solid and window references together.
const EXPECTED_HOUSING_TILE_REFERENCES: usize = 3_567;
const EXPECTED_HOUSING_MULTI_REFERENCES: usize = 74;
const HOUSING_STAIR_MULTI_IDS: RangeInclusive<u32> = 7600..=7675;

fn multi_collection_path() -> PathBuf {
    let client_dir = std::env::var(EC_CLIENT_DIR_ENV)
        .unwrap_or_else(|_| panic!("{EC_CLIENT_DIR_ENV} must name the Enhanced Client directory"));
    PathBuf::from(client_dir).join(MULTI_COLLECTION_UOP_NAME)
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn every_multi_and_housing_bin_parse_exactly() {
    let package = UopPackage::load(multi_collection_path()).expect("load MultiCollection.uop");

    let collection = MultiCollection::from_package(&package).expect("every multi entry parses exactly");
    assert_eq!(collection.items.len(), EXPECTED_MULTI_ENTRIES);
    let parts: Vec<_> = collection.items.iter().flat_map(|item| &item.parts).collect();
    assert_eq!(parts.len(), EXPECTED_MULTI_PARTS);
    assert_eq!(parts.iter().filter(|part| part.not_drawn).count(), EXPECTED_NOT_DRAWN_PARTS);
    assert_eq!(parts.iter().filter(|part| part.flag2 != 0).count(), EXPECTED_FLAG2_PARTS);
    assert_eq!(parts.iter().map(|part| part.string_ids.len()).sum::<usize>(), EXPECTED_PART_STRING_REFERENCES);
    let out_of_range_z: Vec<_> = parts.iter().map(|part| part.z).filter(|z| !EXPECTED_PART_Z_RANGE.contains(z)).collect();
    assert!(out_of_range_z.is_empty(), "part z outside {EXPECTED_PART_Z_RANGE:?}: {out_of_range_z:?}");

    let housing = HousingPieces::from_package(&package)
        .expect("housing.bin parses exactly")
        .expect("MultiCollection.uop has housing.bin");
    let pieces_by_tile_type: Vec<_> = housing
        .tile_types
        .iter()
        .map(|tile_type| (tile_type.tile_type, tile_type.pieces.len()))
        .collect();
    assert_eq!(pieces_by_tile_type, EXPECTED_PIECES_BY_TILE_TYPE);
    assert_eq!(housing.pieces().count(), EXPECTED_HOUSING_PIECES);
    let tile_references: usize = housing.pieces().map(|(_, piece)| piece.solid.len() + piece.windows.len()).sum();
    assert_eq!(tile_references, EXPECTED_HOUSING_TILE_REFERENCES);

    let mut multi_references_by_tile_type = BTreeMap::new();
    for (tile_type, piece) in housing.pieces() {
        for multi in &piece.multis {
            *multi_references_by_tile_type.entry(tile_type).or_insert(0usize) += 1;
            assert!(HOUSING_STAIR_MULTI_IDS.contains(&multi.id), "multi reference {} out of range", multi.id);
            assert!(collection.get(multi.id).is_some(), "multi reference {} missing from the collection", multi.id);
        }
    }
    assert_eq!(
        multi_references_by_tile_type,
        BTreeMap::from([(HOUSING_TILE_TYPE_STAIRS, EXPECTED_HOUSING_MULTI_REFERENCES)])
    );
}
