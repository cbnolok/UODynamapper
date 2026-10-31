//! Checks the Kingdom Reborn (`ClientDialect::KingdomReborn`) codecs of a real KR client (`UOKR.exe`, 2008) other
//! than the facets (`kr_facet_client_files.rs`): every entry of `EffectDefinitionCollection.uop`,
//! `AnimationSequence.uop`, `tileart.uop` and `MultiCollection.uop` decodes and re-encodes byte for byte, and the
//! package writers leave an unedited package unchanged. Every DDS of `Interface.uop` (X1R5G5B5 included) and every
//! TGA of `TileArtLegacy.uop` (the legacy art sources, one of them header-only) decodes.
//!
//! The KR client directory comes from `common::KR_CLIENT_DIR_ENV`. Run with `--release`.

use crate::common;

use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::effect_definition::{
    encode_effect_definition_collection_dialect, parse_effect_definition_collection_dialect,
    EffectDefinitionCollection, EFFECT_DEFINITION_COLLECTION_UOP_NAME,
};
use uocf::enhanced::multis::{
    multi_collection_hash, MultiCollection, MultiItem, MAX_MULTI_COLLECTION_ID, MULTI_COLLECTION_UOP_NAME,
};
use uocf::enhanced::textures::{decode_dds_rgba8, decode_tga_rgba8};
use uocf::enhanced::tileart::{TileArtEntry, TILEART_KR_RECORD_VERSION, TILEART_UOP_NAME};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const KR: ClientDialect = ClientDialect::KingdomReborn;
const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
/// Entry ids searched for sequence names (body types).
const MAX_SEQUENCE_ID: u32 = 100_000;

/// Entries of the shipped KR packages.
const KR_EFFECT_COLLECTION_ENTRIES: usize = 258;
const KR_SEQUENCES: usize = 252;
const KR_TILEART_RECORDS: usize = 17_086;
const KR_MULTIS: usize = 241;
const INTERFACE_UOP_NAME: &str = "Interface.uop";
const TILEART_LEGACY_UOP_NAME: &str = "TileArtLegacy.uop";
const DDS_MAGIC: &[u8] = b"DDS ";
/// DDS images of `Interface.uop`, and TGA entries of `TileArtLegacy.uop` (3 of them unnamed).
const KR_INTERFACE_DDS: usize = 809;
const KR_TILEART_LEGACY_TGAS: usize = 16_384;
/// Header-only TGAs of `TileArtLegacy.uop` (`data/tileartlegacy/16383.tga`).
const KR_EMPTY_TILEART_LEGACY_TGAS: usize = 1;

/// The non-empty entries of the KR package `name`, as (name hash, payload).
fn kr_payloads(name: &str) -> (UopPackage, Vec<(u64, Vec<u8>)>) {
    let path = common::kr_client_file(name);
    let package = UopPackage::load(&path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
    let payloads = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| (file.filename_hash(), file.unpack().unwrap_or_else(|error| panic!("unpack {name}: {error}"))))
        .collect();
    (package, payloads)
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_effect_collection_entries_reencode_identically() {
    let (mut package, payloads) = kr_payloads(EFFECT_DEFINITION_COLLECTION_UOP_NAME);
    assert_eq!(payloads.len(), KR_EFFECT_COLLECTION_ENTRIES);
    for (hash, payload) in &payloads {
        let definitions = parse_effect_definition_collection_dialect(payload, KR)
            .unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        let encoded = encode_effect_definition_collection_dialect(&definitions, KR).unwrap();
        assert!(&encoded == payload, "{hash:016X} re-encodes differently");
    }
    let collection = EffectDefinitionCollection::from_package_dialect(&package, KR).unwrap();
    let changes = collection.apply_to_package_dialect(&mut package, KR).unwrap();
    assert_eq!((changes.replaced, changes.added, changes.removed), (0, 0, 0));
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_animation_sequences_reencode_identically() {
    let (_, payloads) = kr_payloads(SEQUENCE_UOP_NAME);
    let named: std::collections::HashSet<u64> =
        (0..MAX_SEQUENCE_ID).map(|id| hash_file_name_single(&AnimationSequence::entry_path(id))).collect();
    assert_eq!(payloads.len(), KR_SEQUENCES);
    let mut repeated = 0usize;
    for (hash, payload) in &payloads {
        assert!(named.contains(hash), "{hash:016X}: no entry name");
        let sequence =
            AnimationSequence::parse_dialect(payload, KR).unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        repeated += sequence.repeated_actions.len();
        let encoded = sequence.to_bytes_dialect(KR).unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        assert!(&encoded == payload, "{hash:016X} re-encodes differently");
    }
    println!("{} sequences, {repeated} repeated actions", payloads.len());
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_tileart_records_reencode_identically() {
    let (_, payloads) = kr_payloads(TILEART_UOP_NAME);
    assert_eq!(payloads.len(), KR_TILEART_RECORDS);
    for (hash, payload) in &payloads {
        let entry = TileArtEntry::parse_bytes(payload).unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        assert_eq!(entry.version, TILEART_KR_RECORD_VERSION);
        let encoded = entry.to_bytes().unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        assert!(&encoded == payload, "{hash:016X} (tile {}) re-encodes differently", entry.tile_id);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_multis_reencode_identically() {
    let (mut package, _) = kr_payloads(MULTI_COLLECTION_UOP_NAME);
    let mut multis = 0usize;
    for id in 0..MAX_MULTI_COLLECTION_ID {
        let hash = multi_collection_hash(id);
        let Some(payload) = package.unpack_file_by_hash(hash).unwrap() else { continue };
        if payload.is_empty() {
            continue;
        }
        let path = uocf::enhanced::multis::multi_collection_path(id);
        let item = MultiItem::from_payload_bytes_dialect(hash, path, &payload, KR)
            .unwrap_or_else(|error| panic!("multi {id}: {error:#}"));
        assert!(item.to_payload_bytes_dialect(KR).unwrap() == payload, "multi {id} re-encodes differently");
        multis += 1;
    }
    assert_eq!(multis, KR_MULTIS);
    let collection = MultiCollection::from_package_dialect(&package, KR).unwrap();
    assert_eq!(collection.items.len(), KR_MULTIS);
    let changes = collection.apply_to_package(&mut package).unwrap();
    assert_eq!((changes.replaced, changes.added, changes.removed), (0, 0, 0));
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_interface_dds_and_tileart_legacy_tga_decode() {
    let (_, interface) = kr_payloads(INTERFACE_UOP_NAME);
    let mut dds = 0usize;
    for (hash, payload) in interface.iter().filter(|(_, payload)| payload.starts_with(DDS_MAGIC)) {
        decode_dds_rgba8(payload).unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        dds += 1;
    }
    assert_eq!(dds, KR_INTERFACE_DDS);

    let (_, legacy) = kr_payloads(TILEART_LEGACY_UOP_NAME);
    assert_eq!(legacy.len(), KR_TILEART_LEGACY_TGAS);
    let mut empty = 0usize;
    for (hash, payload) in &legacy {
        let image = decode_tga_rgba8(payload).unwrap_or_else(|error| panic!("{hash:016X}: {error:#}"));
        empty += usize::from(image.width() == 0);
    }
    assert_eq!(empty, KR_EMPTY_TILEART_LEGACY_TGAS);
}
