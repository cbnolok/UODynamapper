//! Parses every entry of a real Enhanced Client `AnimationSequence.uop` and
//! `AnimationDefinition.uop`.
//!
//! The expected figures are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-animation/verify_animation.py`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animation_definition::AnimationDefinition;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

/// Environment variable naming the Enhanced Client directory.
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
const DEFINITION_UOP_NAME: &str = "AnimationDefinition.uop";

/// Entry ids searched for sequence names (body types).
const MAX_SEQUENCE_ID: u32 = 100_000;
/// Entry ids searched for definition names: item ids, and item ids above the mobile base.
const MAX_ITEM_ID: u32 = 0x10000;
/// Definitions keyed like a mobile use this base + body (one entry, 10000400).
const MOBILE_DEFINITION_BASE: u32 = 10_000_000;

const EXPECTED_SEQUENCES: usize = 370;
const EXPECTED_SEQUENCE_REDIRECTS: [(u32, u32); 3] = [(146, 704), (1308, 29), (1309, 1254)];
const EXPECTED_PLAIN_ACTIONS: usize = 5546;
const EXPECTED_ALIAS_ACTIONS: usize = 5368;
/// Aliases whose target is another alias; the client resolves them to the lowest plain action.
const EXPECTED_ALIAS_OF_ALIAS: usize = 78;
const EXPECTED_EFFECT_FRAMES: usize = 56;
const EXPECTED_SKIPPED_FRAME_ACTIONS: usize = 69;

const EXPECTED_DEFINITIONS: usize = 1102;
const EXPECTED_DEFINITION_REDIRECTS: [(u32, u32); 3] = [(7905, 7897), (7923, 7916), (18153, 18150)];

fn client_dir() -> PathBuf {
    let client_dir = std::env::var(EC_CLIENT_DIR_ENV)
        .unwrap_or_else(|_| panic!("{EC_CLIENT_DIR_ENV} must name the Enhanced Client directory"));
    PathBuf::from(client_dir)
}

/// Parses every non-empty entry of `uop_name`, keyed by the id its name hash resolves to.
fn parse_entries<T>(
    uop_name: &str,
    ids: impl Iterator<Item = u32>,
    entry_path: fn(u32) -> String,
    parse: fn(&[u8]) -> color_eyre::eyre::Result<T>,
) -> BTreeMap<u32, T> {
    let package = UopPackage::load(client_dir().join(uop_name)).unwrap_or_else(|error| panic!("load {uop_name}: {error}"));
    let names: HashMap<u64, u32> = ids.map(|id| (hash_file_name_single(&entry_path(id)), id)).collect();
    let mut entries = BTreeMap::new();
    let mut failures = Vec::new();
    for file in package.iter_files().filter(|file| file.has_size()) {
        let Some(&id) = names.get(&file.filename_hash()) else {
            failures.push(format!("{:016X}: no entry name", file.filename_hash()));
            continue;
        };
        match file.unpack_arc().map_err(Into::into).and_then(|payload| parse(&payload)) {
            Ok(entry) => { entries.insert(id, entry); }
            Err(error) => failures.push(format!("{id}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{} entries failed: {failures:?}", failures.len());
    entries
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn animation_sequence_parses_every_client_entry() {
    let sequences = parse_entries(
        SEQUENCE_UOP_NAME,
        0..MAX_SEQUENCE_ID,
        AnimationSequence::entry_path,
        AnimationSequence::parse,
    );
    assert_eq!(sequences.len(), EXPECTED_SEQUENCES);

    let redirects: BTreeMap<u32, u32> = sequences
        .iter()
        .filter(|(id, sequence)| sequence.stored_id != **id)
        .map(|(&id, sequence)| (id, sequence.stored_id))
        .collect();
    assert_eq!(redirects, BTreeMap::from(EXPECTED_SEQUENCE_REDIRECTS));

    let mut plain = 0usize;
    let mut aliases = 0usize;
    let mut alias_of_alias = 0usize;
    let mut effect_frames = 0usize;
    let mut skipped_frame_actions = 0usize;
    for sequence in sequences.values() {
        for (&action_id, action) in &sequence.actions {
            effect_frames += action.frame_effects.len();
            skipped_frame_actions += usize::from(!action.skipped_frames.is_empty());
            let Some(target) = action.alias else {
                plain += 1;
                assert_eq!(sequence.resolve_action(action_id, true), Some(action_id));
                continue;
            };
            aliases += 1;
            let target_record = sequence.actions.get(&target).expect("no alias target is missing");
            alias_of_alias += usize::from(target_record.alias.is_some());
        }
        for group in sequence.groups.values() {
            for choices in group.typed.values() {
                assert!(choices.iter().all(|choice| sequence.actions.contains_key(choice)));
            }
        }
    }
    assert_eq!(plain, EXPECTED_PLAIN_ACTIONS);
    assert_eq!(aliases, EXPECTED_ALIAS_ACTIONS);
    assert_eq!(alias_of_alias, EXPECTED_ALIAS_OF_ALIAS);
    assert_eq!(effect_frames, EXPECTED_EFFECT_FRAMES);
    assert_eq!(skipped_frame_actions, EXPECTED_SKIPPED_FRAME_ACTIONS);
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn animation_definition_parses_every_client_entry() {
    let ids = (0..MAX_ITEM_ID).chain((0..MAX_ITEM_ID).map(|id| MOBILE_DEFINITION_BASE + id));
    let definitions = parse_entries(
        DEFINITION_UOP_NAME,
        ids,
        AnimationDefinition::entry_path,
        AnimationDefinition::parse,
    );
    assert_eq!(definitions.len(), EXPECTED_DEFINITIONS);

    let redirects: BTreeMap<u32, u32> = definitions
        .iter()
        .filter(|(id, definition)| definition.stored_id != **id)
        .map(|(&id, definition)| (id, definition.stored_id))
        .collect();
    assert_eq!(redirects, BTreeMap::from(EXPECTED_DEFINITION_REDIRECTS));
    assert!(definitions.values().all(|definition| definition.sets.values().all(|set| !set.frames.is_empty())));
}
