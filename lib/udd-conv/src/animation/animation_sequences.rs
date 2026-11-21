//! `AnimationSequence.uop` loading and the body records' move actions, shared by the Classic and the Enhanced
//! mobile animation packers.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use uocf::animation_sequence::AnimationSequence;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use udd_assets::animation::move_actions::{
    MOVE_ACTION_GROUPS, MOVE_ACTION_GROUP_COUNT, MOVE_ACTION_MOVEMENTS, MOVE_ACTION_MOVEMENT_COUNT, NO_MOVE_ACTION,
};

/// Highest body type whose `AnimationSequence` entry name is looked up (body ids are u16).
const MAX_SEQUENCE_BODY_TYPE: u32 = u16::MAX as u32;
const SEQUENCE_FILE_NAMES: [&str; 2] = ["AnimationSequence.uop", "animationsequence.uop"];

/// Loads every `AnimationSequence` entry of `client_dir`, keyed by body type (the entry name). Returns the
/// sequences and the number of entries that failed to parse. `client` names the client in the log ("EC", "CC"),
/// `missing_fallback` what the packer does without the file.
pub(crate) fn load_animation_sequences(
    client_dir: &Path,
    client: &str,
    missing_fallback: &str,
) -> eyre::Result<(BTreeMap<u32, AnimationSequence>, u32)> {
    let sequence_path = SEQUENCE_FILE_NAMES
        .iter()
        .map(|name| client_dir.join(name))
        .find(|path| path.is_file());
    let Some(sequence_path) = sequence_path else {
        warn!("AnimationSequence.uop not found in {}; {missing_fallback}", client_dir.display());
        return Ok((BTreeMap::new(), 0));
    };
    let mut package = UopPackage::load_with_mode(&sequence_path, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", sequence_path.display()))?;
    let entry_hashes = package
        .iter_files()
        .filter(|file| file.has_size())
        .map(|file| file.filename_hash())
        .collect::<HashSet<_>>();
    let mut sequences = BTreeMap::new();
    let mut named_entry_count = 0usize;
    let mut parse_error_count = 0u32;
    for body_type in 0..=MAX_SEQUENCE_BODY_TYPE {
        let hash = hash_file_name_single(&AnimationSequence::entry_path(body_type));
        if !entry_hashes.contains(&hash) {
            continue;
        }
        named_entry_count += 1;
        let Some(data) = package.unpack_file_by_hash_cached(hash)? else {
            continue;
        };
        match AnimationSequence::parse(&data) {
            Ok(sequence) => {
                sequences.insert(body_type, sequence);
            }
            Err(error) => {
                parse_error_count += 1;
                warn!("{client} animation sequence of body {body_type} failed to parse: {error}");
            }
        }
    }
    let unnamed_entry_count = entry_hashes.len() - named_entry_count;
    if unnamed_entry_count != 0 {
        warn!(
            "{unnamed_entry_count} AnimationSequence.uop entries have no body type name up to {MAX_SEQUENCE_BODY_TYPE}"
        );
    }
    info!("{client} mobile animation sequences loaded ({})", sequences.len());
    Ok((sequences, parse_error_count))
}

/// Move actions of a body record: `GetMoveAction(group, movement, false)` of `sequence` for each
/// group and movement, `NO_MOVE_ACTION` where the group is missing or there is no sequence.
pub(crate) fn move_actions(
    sequence: Option<&AnimationSequence>,
) -> eyre::Result<[[u16; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT]> {
    let mut actions = [[NO_MOVE_ACTION; MOVE_ACTION_MOVEMENT_COUNT]; MOVE_ACTION_GROUP_COUNT];
    let Some(sequence) = sequence else {
        return Ok(actions);
    };
    for (group_actions, &group) in actions.iter_mut().zip(&MOVE_ACTION_GROUPS) {
        for (action, &movement) in group_actions.iter_mut().zip(&MOVE_ACTION_MOVEMENTS) {
            if let Some(move_action) = sequence.move_action(group, movement, false) {
                *action = u16::try_from(move_action)
                    .ok()
                    .filter(|&action| action != NO_MOVE_ACTION)
                    .ok_or_else(|| eyre::eyre!("move action {move_action} does not fit the package action ids"))?;
            }
        }
    }
    Ok(actions)
}
