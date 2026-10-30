//! Parses every multi entry and `housing.bin` of a real client `MultiCollection.uop` (the Enhanced and Classic
//! clients ship the same payloads).
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`. Every entry must parse in any
//! release. The expected counts are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-housing/verify_housing.py` on the reference release (the 2024 clients);
//! they are asserted only when the multi count identifies that release, and printed otherwise.
//!
//! The housing editing tests also check that `housing.bin` re-serializes byte for byte (directly and through
//! KDL), validates against the client's tileart, multis and clilocs, and that a rewritten package changes only
//! `housing.bin`. The rewritten package goes to `std::env::temp_dir()` (set `TMPDIR` to choose it) and is
//! deleted afterwards. The multi writer tests check that every multi entry re-encodes byte for byte and that a
//! package rewritten with an edited, an added and a removed multi changes only those entries.

mod common;

use std::collections::{BTreeMap, HashSet};
use std::ops::RangeInclusive;
use std::path::Path;
use uocf::classic::cliloc::Cliloc;
use uocf::enhanced::housing::{
    HousingIssue, HousingIssueSeverity, HousingPieces, HousingValidationContext, HOUSING_TILE_TYPE_DOOR,
    HOUSING_TILE_TYPE_FLOOR, HOUSING_TILE_TYPE_MISC, HOUSING_TILE_TYPE_ROOF, HOUSING_TILE_TYPE_STAIRS,
    HOUSING_TILE_TYPE_TELEPORTER, HOUSING_TILE_TYPE_WALL,
};
use uocf::enhanced::localized_strings::LocalizedStringsPackage;
use uocf::enhanced::multis::{
    multi_collection_hash, multi_collection_path, MultiCollection, MultiCollectionChanges, MultiItem, HOUSING_PATH,
    MAX_MULTI_COLLECTION_ID, MULTI_COLLECTION_UOP_NAME,
};
use uocf::enhanced::tileart::TileArtEntry;
use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderRecord};
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
/// Categories whose groups have different feature masks (walls 2..5 and misc 11).
const EXPECTED_MIXED_FEATURE_MASK_CATEGORIES: usize = 5;
/// Repeated slots: misc category 25 group 1 has solid slot 2 twice (tiles 17340 and 17342).
const EXPECTED_DUPLICATE_SLOTS: usize = 1;

const TILEART_UOP_NAME: &str = "tileart.uop";
const CLASSIC_CLILOC_NAME: &str = "Cliloc.enu";
const LOCALIZED_STRINGS_FILE_NAME: &str = "LocalizedStrings.uop";
/// Tooltip cliloc changed in the rewritten package.
const REWRITTEN_TOOLTIP_CLILOC: u32 = 1061019;
/// Z offset given to the first part of the edited multi in the rewritten package.
const REWRITTEN_PART_Z_DELTA: i16 = 1;

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

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn housing_bin_reserializes_identically_directly_and_through_kdl() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load MultiCollection.uop");
        let original = package.get_file_by_name(HOUSING_PATH).expect("housing.bin entry").unpack().expect("unpack");

        let housing = HousingPieces::from_payload_bytes(&original).expect("housing.bin parses");
        assert!(housing.to_payload_bytes() == original, "{env}: re-serialized housing.bin differs");

        let kdl = housing.to_kdl_string();
        let from_kdl = HousingPieces::from_kdl_str(&kdl).expect("the written KDL parses");
        assert!(from_kdl == housing, "{env}: KDL round trip changed the pieces");
        assert!(from_kdl.to_payload_bytes() == original, "{env}: housing.bin from KDL differs");
        let (bytes, lines) = (original.len(), kdl.lines().count());
        println!("{env}: {bytes} bytes of housing.bin, {lines} KDL lines, {} KDL bytes", kdl.len());
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn housing_bin_validates_against_client_data() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load MultiCollection.uop");
        let housing = HousingPieces::from_package(&package).expect("housing.bin parses").expect("housing.bin entry");
        let multis: HashSet<u32> = MultiCollection::from_package(&package)
            .expect("multis parse")
            .items
            .iter()
            .map(|item| item.id)
            .collect();
        let tiles = load_tileart_ids(&path.with_file_name(TILEART_UOP_NAME));
        let clilocs = load_cliloc_ids(&path);
        println!(
            "{env}: {} multis, {} tileart records, {} clilocs",
            multis.len(),
            tiles.as_ref().map_or(0, HashSet::len),
            clilocs.as_ref().map_or(0, HashSet::len)
        );

        let multi_exists = |id| multis.contains(&id);
        let tile_exists = |id| tiles.as_ref().is_some_and(|tiles| tiles.contains(&id));
        let cliloc_exists = |id| clilocs.as_ref().is_some_and(|clilocs| clilocs.contains(&id));
        let context = HousingValidationContext {
            tile_exists: tiles.is_some().then_some(&tile_exists as &dyn Fn(u32) -> bool),
            multi_exists: Some(&multi_exists),
            cliloc_exists: clilocs.is_some().then_some(&cliloc_exists as &dyn Fn(u32) -> bool),
        };
        let issues = housing.validate(&context);

        for issue in &issues {
            println!("{env}: {:?}: {issue}", issue.severity());
        }
        let errors: Vec<_> = issues.iter().filter(|issue| issue.severity() == HousingIssueSeverity::Error).collect();
        assert!(errors.is_empty(), "{env}: {} errors: {errors:?}", errors.len());
        if multis.len() != EXPECTED_MULTI_ENTRIES {
            println!("{env}: not the reference release, warnings not checked");
            continue;
        }
        let mixed_masks = issues.iter().filter(|issue| matches!(issue, HousingIssue::MixedFeatureMask { .. })).count();
        let duplicate_slots = issues.iter().filter(|issue| matches!(issue, HousingIssue::DuplicateSlot { .. })).count();
        assert_eq!(mixed_masks, EXPECTED_MIXED_FEATURE_MASK_CATEGORIES);
        assert_eq!(duplicate_slots, EXPECTED_DUPLICATE_SLOTS);
        assert_eq!(mixed_masks + duplicate_slots, issues.len(), "{env}: no other warnings");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn rewritten_multi_collection_changes_only_housing_bin() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let source = UopPackage::load(&path).expect("load MultiCollection.uop");
        let mut housing = HousingPieces::from_package(&source).expect("housing.bin parses").expect("housing.bin entry");
        housing.tile_types[0].pieces[0].tooltip_cliloc = REWRITTEN_TOOLTIP_CLILOC;
        let dir = common::temp_dir("housing_rewrite");
        let output = dir.join(MULTI_COLLECTION_UOP_NAME);

        housing.write_multi_collection(&path, &output).expect("write the new package");

        let written = UopPackage::load(&output).expect("reload the new package");
        assert_eq!(HousingPieces::from_package(&written).unwrap(), Some(housing.clone()), "{env}: housing.bin");
        assert_eq!(written.file_count(), source.file_count(), "{env}: entry count");
        let housing_hash = uocf::enhanced::multis::housing_hash();
        for source_file in source.iter_files() {
            let hash = source_file.filename_hash();
            let written_file =
                written.get_file_by_hash(hash).unwrap_or_else(|| panic!("{env}: entry {hash:016X} missing"));
            assert_eq!(written_file.compression(), source_file.compression(), "{env}: entry {hash:016X} compression");
            if hash == housing_hash {
                // The edited entry gets a fresh timestamp header and its hash.
                let header = DataBlockHeader::parse(written_file.data_block_header().unwrap()).unwrap();
                assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{env}: housing header");
                assert_eq!(written_file.data_block_hash_matches_header().unwrap(), Some(true), "{env}: housing hash");
                continue;
            }
            assert!(
                written_file.data_block_header() == source_file.data_block_header()
                    && written_file.data_block_hash() == source_file.data_block_hash(),
                "{env}: entry {hash:016X} data block header differs"
            );
            if !source_file.has_size() {
                continue;
            }
            assert!(written_file.data() == source_file.data(), "{env}: entry {hash:016X} payload differs");
        }
        let source_multis = MultiCollection::from_package(&source).unwrap();
        let written_multis = MultiCollection::from_package(&written).unwrap();
        assert!(written_multis.items == source_multis.items, "{env}: multis differ");
        println!("{env}: rewrote {} entries", written.file_count());
        std::fs::remove_dir_all(&dir).expect("delete the new package");
        assert!(housing.write_multi_collection(&path, &path).is_err(), "{env}: writing over the source must fail");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn every_multi_reencodes_identically() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load MultiCollection.uop");
        let mut checked = 0usize;
        for id in 0..MAX_MULTI_COLLECTION_ID {
            let Some(file) = package.get_file_by_hash(multi_collection_hash(id)) else {
                continue;
            };
            let payload = file.unpack().expect("unpack");
            let item = MultiItem::from_payload_bytes(file.filename_hash(), multi_collection_path(id), &payload)
                .expect("multi parses");
            assert_eq!(item.id, id, "{env}: multi {id} stores another id");
            assert!(item.to_payload_bytes() == payload, "{env}: multi {id} re-encodes differently");
            checked += 1;
        }
        let entries = package.iter_files().filter(|file| file.has_size()).count();
        assert_eq!(checked + 1, entries, "{env}: every entry but housing.bin is a multi");
        println!("{env}: {checked} multis re-encode identically");
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn rewritten_multi_collection_changes_only_the_edited_multis() {
    for (env, path) in common::client_files(MULTI_COLLECTION_UOP_NAME) {
        let source = UopPackage::load(&path).expect("load MultiCollection.uop");
        let mut collection = MultiCollection::from_package(&source).expect("multis parse");
        let ids: Vec<u32> = collection.ids().collect();
        let (edited, removed) = (ids[0], ids[1]);
        let added = (0..MAX_MULTI_COLLECTION_ID).find(|id| collection.get(*id).is_none()).expect("a free multi id");
        collection.parts_mut(edited).expect("edited multi")[0].z += REWRITTEN_PART_Z_DELTA;
        let removed_item = collection.remove(removed).expect("removed multi");
        collection.set(MultiItem::new(added, removed_item.parts.clone()));
        let dir = common::temp_dir("multis_rewrite");
        let output = dir.join(MULTI_COLLECTION_UOP_NAME);

        let changes = collection.write_multi_collection(&path, &output).expect("write the new package");

        assert_eq!(changes, MultiCollectionChanges { replaced: 1, added: 1, removed: 1 }, "{env}");
        let written = UopPackage::load(&output).expect("reload the new package");
        let written_collection = MultiCollection::from_package(&written).expect("written multis parse");
        assert!(written_collection.items == collection.items, "{env}: written multis differ");
        assert_eq!(written.file_count(), source.file_count(), "{env}: one entry removed, one added");
        let edited_hashes = [multi_collection_hash(edited), multi_collection_hash(removed)];
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
        for hash in [multi_collection_hash(edited), multi_collection_hash(added)] {
            let file = written.get_file_by_hash(hash).expect("written entry");
            let header = DataBlockHeader::parse(file.data_block_header().unwrap()).unwrap();
            assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{env}: {hash:016X} header");
            assert_eq!(file.data_block_hash_matches_header().unwrap(), Some(true), "{env}: {hash:016X} hash");
        }
        let source_len = std::fs::metadata(&path).unwrap().len();
        let written_len = std::fs::metadata(&output).unwrap().len();
        println!("{env}: multi {edited} edited, {removed} removed, {added} added; {source_len} -> {written_len} bytes");
        std::fs::remove_dir_all(&dir).expect("delete the new package");
        assert!(collection.write_multi_collection(&path, &path).is_err(), "{env}: writing over the source must fail");
    }
}

/// Tile ids of every record of `tileart.uop`; `None` when the client has no such file.
fn load_tileart_ids(path: &Path) -> Option<HashSet<u32>> {
    if !path.is_file() {
        return None;
    }
    let package = UopPackage::load(path).expect("load tileart.uop");
    let tile_id = |file| TileArtEntry::parse_raw(file).expect("tileart record parses").tile_id;
    Some(package.iter_files().map(tile_id).collect())
}

/// Cliloc ids of the classic `Cliloc.enu`, or of the Enhanced Client `LocalizedStrings.uop`, next to `path`.
/// `None` (cliloc check skipped) when neither exists or `LocalizedStrings.uop` does not parse: this test checks
/// `housing.bin`, not the localized strings decoder.
fn load_cliloc_ids(path: &Path) -> Option<HashSet<u32>> {
    let classic = path.with_file_name(CLASSIC_CLILOC_NAME);
    if classic.is_file() {
        let cliloc = Cliloc::load(&classic).expect("load Cliloc.enu");
        return Some(cliloc.entries.iter().filter_map(|entry| u32::try_from(entry.number).ok()).collect());
    }
    let localized = path.with_file_name(LOCALIZED_STRINGS_FILE_NAME);
    if !localized.is_file() {
        return None;
    }
    match LocalizedStringsPackage::load(&localized) {
        Ok(package) => {
            Some(package.files.iter().flat_map(|file| file.strings.entries.iter().map(|entry| entry.id)).collect())
        }
        Err(error) => {
            println!("{}: {error:#}; tooltip clilocs not checked", localized.display());
            None
        }
    }
}
