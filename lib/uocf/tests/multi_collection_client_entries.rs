//! Parses every multi entry and `housing.bin` of a real client `MultiCollection.uop` (the Enhanced and Classic
//! clients ship the same payloads).
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`. Every entry must parse in any
//! release. The expected counts are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-housing/verify_housing.py` on the reference release (the 2024 clients);
//! they are asserted only when the multi count identifies that release, and printed otherwise.

mod common;

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use uocf::enhanced::housing::{
    HousingPieces, HOUSING_TILE_TYPE_DOOR, HOUSING_TILE_TYPE_FLOOR, HOUSING_TILE_TYPE_MISC, HOUSING_TILE_TYPE_ROOF,
    HOUSING_TILE_TYPE_STAIRS, HOUSING_TILE_TYPE_TELEPORTER, HOUSING_TILE_TYPE_WALL,
};
use uocf::enhanced::multis::{MultiCollection, MULTI_COLLECTION_UOP_NAME};
use uocf::uop_container::package::UopPackage;

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

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn every_multi_and_housing_bin_parse_exactly() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load MultiCollection.uop");

        let collection = MultiCollection::from_package(&package).expect("every multi entry parses exactly");
        let housing = HousingPieces::from_package(&package)
            .expect("housing.bin parses exactly")
            .expect("MultiCollection.uop has housing.bin");
        let entries = package.iter_files().filter(|file| file.has_size()).count();
        assert_eq!(entries, collection.items.len() + 1, "every entry is a multi or housing.bin");

        let parts: Vec<_> = collection.items.iter().flat_map(|item| &item.parts).collect();
        let not_drawn_parts = parts.iter().filter(|part| part.not_drawn).count();
        let flag2_parts = parts.iter().filter(|part| part.flag2 != 0).count();
        let part_string_references: usize = parts.iter().map(|part| part.string_ids.len()).sum();
        let part_z_range = parts.iter().map(|part| part.z).min().zip(parts.iter().map(|part| part.z).max());

        let pieces_by_tile_type: Vec<_> = housing
            .tile_types
            .iter()
            .map(|tile_type| (tile_type.tile_type, tile_type.pieces.len()))
            .collect();
        let tile_references: usize = housing.pieces().map(|(_, piece)| piece.solid.len() + piece.windows.len()).sum();
        let mut multi_references_by_tile_type = BTreeMap::new();
        let mut multi_reference_ids = Vec::new();
        for (tile_type, piece) in housing.pieces() {
            for multi in &piece.multis {
                *multi_references_by_tile_type.entry(tile_type).or_insert(0usize) += 1;
                assert!(collection.get(multi.id).is_some(), "multi reference {} missing from the collection", multi.id);
                multi_reference_ids.push(multi.id);
            }
        }

        println!(
            "{env}: {} multis, {} parts ({not_drawn_parts} not drawn, {flag2_parts} with flag2, \
             {part_string_references} string references, z range {part_z_range:?}); housing pieces by tile type \
             {pieces_by_tile_type:?}, {tile_references} tile references, multi references by tile type \
             {multi_references_by_tile_type:?}",
            collection.items.len(),
            parts.len()
        );
        if collection.items.len() != EXPECTED_MULTI_ENTRIES {
            println!("{env}: not the reference release, counts not checked");
            continue;
        }
        assert_eq!(parts.len(), EXPECTED_MULTI_PARTS);
        assert_eq!(not_drawn_parts, EXPECTED_NOT_DRAWN_PARTS);
        assert_eq!(flag2_parts, EXPECTED_FLAG2_PARTS);
        assert_eq!(part_string_references, EXPECTED_PART_STRING_REFERENCES);
        let out_of_range_z: Vec<_> = parts.iter().map(|part| part.z).filter(|z| !EXPECTED_PART_Z_RANGE.contains(z)).collect();
        assert!(out_of_range_z.is_empty(), "part z outside {EXPECTED_PART_Z_RANGE:?}: {out_of_range_z:?}");

        assert_eq!(pieces_by_tile_type, EXPECTED_PIECES_BY_TILE_TYPE);
        assert_eq!(housing.pieces().count(), EXPECTED_HOUSING_PIECES);
        assert_eq!(tile_references, EXPECTED_HOUSING_TILE_REFERENCES);
        let out_of_range_multis: Vec<_> =
            multi_reference_ids.iter().filter(|id| !HOUSING_STAIR_MULTI_IDS.contains(id)).collect();
        assert!(out_of_range_multis.is_empty(), "multi references outside {HOUSING_STAIR_MULTI_IDS:?}: {out_of_range_multis:?}");
        assert_eq!(
            multi_references_by_tile_type,
            BTreeMap::from([(HOUSING_TILE_TYPE_STAIRS, EXPECTED_HOUSING_MULTI_REFERENCES)])
        );
    }
}
