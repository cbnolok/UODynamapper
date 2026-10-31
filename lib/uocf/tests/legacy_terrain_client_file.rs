//! Every LegacyTerrain document of a real client `LegacyTerrain.uop` parses and writes back byte for byte, edited
//! documents parse back to their definition, and a rewritten package changes only the edited entries.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`. The rewritten package goes to
//! `std::env::temp_dir()` (set `TMPDIR` to choose it) and is deleted afterwards.

mod common;

use std::collections::BTreeMap;

use uocf::enhanced::legacy_terrain::{
    legacy_terrain_entry_hash, read_legacy_terrain, read_legacy_terrain_documents, write_legacy_terrain_package,
    LegacyTerrainChanges, LegacyTerrainDocument, LegacyTerrainFlag, LegacyTerrainType, LEGACY_TERRAIN_UOP_NAME,
};
use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderRecord};
use uocf::uop_container::package::UopPackage;

/// Suffix appended to the name of the edited definition.
const REWRITTEN_NAME_SUFFIX: &str = " (edited)";

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn every_document_writes_back_byte_for_byte() {
    for (env, path) in common::client_files(LEGACY_TERRAIN_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load LegacyTerrain.uop");
        let documents = read_legacy_terrain_documents(&package).expect("every entry parses");
        let mut flag_counts = BTreeMap::<&str, usize>::new();
        for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
            let payload = file.unpack().unwrap();
            let document = LegacyTerrainDocument::parse(&payload).unwrap();
            let id = document.terrain.id;
            assert_eq!(file.filename_hash(), legacy_terrain_entry_hash(id), "{env}: entry of land tile {id}");
            assert!(document.to_bytes() == payload, "{env}: land tile {id} writes back differently");
            for flag in &document.terrain.flags {
                *flag_counts.entry(flag.element_name()).or_default() += 1;
            }
            for flag in LegacyTerrainFlag::ALL {
                let mut edited = document.clone();
                edited.terrain.set_flag(flag, !edited.terrain.has_flag(flag));
                edited.terrain.name.push_str(REWRITTEN_NAME_SUFFIX);
                let reparsed = LegacyTerrainDocument::parse(&edited.to_bytes()).unwrap();
                assert_eq!(reparsed.terrain, edited.terrain, "{env}: land tile {id} edited ({flag:?})");
            }
        }
        println!("{env}: {} documents write back byte for byte; flags {flag_counts:?}", documents.len());
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn rewritten_package_changes_only_the_edited_entries() {
    for (env, path) in common::client_files(LEGACY_TERRAIN_UOP_NAME) {
        let source = UopPackage::load(&path).expect("load LegacyTerrain.uop");
        let documents = read_legacy_terrain_documents(&source).expect("every entry parses");
        let mut ids = documents.keys().copied();
        let (edited_id, removed_id) = (ids.next().expect("a document"), ids.next().expect("a second document"));
        let added_id = (0..).find(|id| !documents.contains_key(id)).expect("a free land tile id");
        let mut edited = documents[&edited_id].clone();
        edited.terrain.name.push_str(REWRITTEN_NAME_SUFFIX);
        edited.terrain.set_flag(LegacyTerrainFlag::NoHouse, !edited.terrain.has_flag(LegacyTerrainFlag::NoHouse));
        let added = edited.with_terrain(LegacyTerrainType { id: added_id, ..edited.terrain.clone() });
        let dir = common::temp_dir("legacy_terrain_rewrite");
        let output = dir.join(LEGACY_TERRAIN_UOP_NAME);

        let changes = write_legacy_terrain_package(&path, &output, &[edited.clone(), added.clone()], &[removed_id])
            .expect("write the new package");

        assert_eq!(changes, LegacyTerrainChanges { replaced: 1, added: 1, removed: 1 }, "{env}");
        let written = UopPackage::load(&output).expect("reload the new package");
        let edited_hashes = [legacy_terrain_entry_hash(edited_id), legacy_terrain_entry_hash(removed_id)];
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
        assert_eq!(read_legacy_terrain(&written, removed_id).unwrap(), None, "{env}: removed document");
        for document in [&edited, &added] {
            let id = document.terrain.id;
            let file = written.get_file_by_hash(legacy_terrain_entry_hash(id)).expect("written document");
            assert_eq!(file.unpack().unwrap(), document.to_bytes(), "{env}: land tile {id}");
            let header = DataBlockHeader::parse(file.data_block_header().unwrap()).unwrap();
            assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{env}: header");
            assert_eq!(file.data_block_hash_matches_header().unwrap(), Some(true), "{env}: hash");
        }
        let source_len = std::fs::metadata(&path).unwrap().len();
        let written_len = std::fs::metadata(&output).unwrap().len();
        println!("{env}: land tile {edited_id} edited, {removed_id} removed, {added_id} added; {source_len} -> {written_len} bytes");
        std::fs::remove_dir_all(&dir).expect("delete the new package");
        assert!(write_legacy_terrain_package(&path, &path, &[], &[]).is_err(), "{env}: writing over the source must fail");
    }
}
