//! Edits of real client `AnimationSequence.uop` entries: every entry of each available client (Enhanced, Classic
//! and Kingdom Reborn) re-reads equal after encoding, also with its base frames renumbered as the editor does, and
//! an edited entry written to a new package (`write_sequence`) reads back as edited while the other entries keep
//! their bytes.
//!
//! The client directories come from `common::CLIENT_DIR_ENVS` and `common::KR_CLIENT_DIR_ENV`; each is optional and
//! a missing one is skipped.

use crate::common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use uocf::animation_sequence::{AnimationSequence, MoveActions, GROUP_PEACE, MOVE_WALK};
use uocf::enhanced::animationframe::package::{read_sequence, write_sequence, EntryChange};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
/// Entry ids searched for sequence names (body types).
const MAX_SEQUENCE_ID: u32 = 100_000;
/// The human sequence, in every client: the edited entry.
const EDITED_SEQUENCE: u32 = 400;
/// An entry the edit must leave byte for byte.
const UNTOUCHED_SEQUENCE: u32 = 200;
const EDITED_FPS: f32 = 7.5;

/// Every available `AnimationSequence.uop` with its variable and dialect.
fn sequence_packages() -> Vec<(&'static str, PathBuf, ClientDialect)> {
    let mut packages: Vec<_> = common::CLIENT_DIR_ENVS
        .into_iter()
        .filter_map(|env| std::env::var_os(env).map(|dir| (env, PathBuf::from(dir))))
        .chain(std::env::var_os(common::KR_CLIENT_DIR_ENV).map(|dir| (common::KR_CLIENT_DIR_ENV, PathBuf::from(dir))))
        .filter_map(|(env, dir)| {
            let path = dir.join(SEQUENCE_UOP_NAME);
            if path.is_file() {
                Some((env, path, ClientDialect::of_client_dir(&dir)))
            } else {
                println!("{env}: no {SEQUENCE_UOP_NAME}, skipped");
                None
            }
        })
        .collect();
    packages.dedup_by(|a, b| a.1 == b.1);
    if packages.is_empty() {
        println!("no client directory set ({} or {}): nothing to test", common::CLIENT_DIR_ENVS.join(", "), common::KR_CLIENT_DIR_ENV);
    }
    packages
}

fn payload(package: &UopPackage, id: u32) -> Option<Vec<u8>> {
    package.unpack_file_by_hash(hash_file_name_single(&AnimationSequence::entry_path(id))).unwrap()
}

fn load(path: &Path) -> UopPackage {
    UopPackage::load(path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()))
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR, UO_CC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn client_sequences_reread_equal_after_encoding_and_renumbering() {
    for (env, path, dialect) in sequence_packages() {
        let package = load(&path);
        let mut count = 0;
        for id in 0..MAX_SEQUENCE_ID {
            let Some(bytes) = payload(&package, id) else { continue };
            let sequence = AnimationSequence::parse_dialect(&bytes, dialect).unwrap_or_else(|error| panic!("{env} {id}: {error:#}"));
            assert!(sequence.edit_errors().is_empty(), "{env} {id}: {:?}", sequence.edit_errors());
            let encoded = sequence.to_bytes_dialect(dialect).unwrap();
            assert_eq!(AnimationSequence::parse_dialect(&encoded, dialect).unwrap(), sequence, "{env} {id}");
            let mut renumbered = sequence.clone();
            renumbered.renumber_base_frames();
            assert_eq!(renumbered, sequence, "{env} {id}: base frames differ from the reader's");
            count += 1;
        }
        assert!(count > 0, "{env}: no sequence entries");
        println!("{env}: {count} sequences ({})", dialect.label());
    }
}

#[test]
#[ignore = "needs real clients: set UO_EC_CLIENT_DIR, UO_CC_CLIENT_DIR and/or UO_KR_CLIENT_DIR and run with --ignored"]
fn edited_sequence_is_written_to_a_new_package() {
    for (env, path, dialect) in sequence_packages() {
        let source = load(&path);
        let mut sequence = read_sequence(&source, EDITED_SEQUENCE, dialect).unwrap().expect("sequence 400");
        let original = sequence.clone();
        let (&plain, record) = sequence.actions.iter().find(|(_, record)| record.alias.is_none()).unwrap();
        let frame_count = record.frame_count + 1;
        sequence.set_frame_count(plain, frame_count).unwrap();
        sequence.set_fps(plain, EDITED_FPS).unwrap();
        if dialect == ClientDialect::Enhanced {
            sequence.set_skipped_frames(plain, BTreeSet::from([0, frame_count - 1])).unwrap();
        }
        let other = *sequence.actions.keys().find(|&&action| action != plain).unwrap();
        sequence.set_alias(other, Some(plain)).unwrap();
        sequence.set_move_actions(GROUP_PEACE, MOVE_WALK, MoveActions { action: plain, alternate: plain }).unwrap();
        assert_ne!(sequence, original);

        let output = common::temp_dir("sequence_edit").join(SEQUENCE_UOP_NAME);
        assert!(write_sequence(&path, &path, EDITED_SEQUENCE, &sequence, dialect).is_err(), "the source is refused");
        assert_eq!(write_sequence(&path, &output, EDITED_SEQUENCE, &sequence, dialect).unwrap(), EntryChange::Replaced);
        let written = load(&output);
        assert_eq!(read_sequence(&written, EDITED_SEQUENCE, dialect).unwrap().unwrap(), sequence, "{env}");
        assert_eq!(payload(&written, UNTOUCHED_SEQUENCE), payload(&source, UNTOUCHED_SEQUENCE), "{env}");

        let mut broken = sequence.clone();
        broken.actions.get_mut(&plain).unwrap().fps = f32::NAN;
        assert!(write_sequence(&path, output.with_extension("broken.uop"), EDITED_SEQUENCE, &broken, dialect).is_err());
        std::fs::remove_dir_all(output.parent().unwrap()).unwrap();
        println!("{env}: sequence {EDITED_SEQUENCE} action {plain} edited and written");
    }
}
