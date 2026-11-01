//! The two `Paperdoll.uop` entries of the Enhanced Client and Kingdom Reborn: AMOU frame sets of the paperdoll
//! images (`build/paperdoll/000001/00.bin` male, `01.bin` female; the client loads them as assets 0x40 and 0x41
//! of type 0x12, `PaperdollFrameSet`, and looks a frame up by action = gender and frame number = the body or worn
//! item id, falling back to number 400 of the male set; see `scripts/dev-helpers/ec-research/ghidra-paperdoll/`).
//!
//! For each entry: the container parses and writes back byte for byte, every frame decodes and re-encodes from
//! its indices with the entry's palette to the same bytes and pixels, the frame table is sorted as the client's
//! binary search needs, and every frame has the action of its gender. The EC package comes from
//! `UO_EC_CLIENT_DIR`, the KR one from `UO_KR_CLIENT_DIR`; both are only read.

use crate::common;

use std::path::PathBuf;

use uocf::enhanced::animationframe::{AmouFile, AnimationFrame};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const PAPERDOLL_UOP_NAME: &str = "Paperdoll.uop";
/// The client's gender actions: 0 male (`00.bin`), 1 female (`01.bin`).
const GENDER_ENTRIES: [(&str, u16); 2] =
    [("build/paperdoll/000001/00.bin", 0), ("build/paperdoll/000001/01.bin", 1)];
/// The frame number `PaperdollTextures::FindPaperdollFrame` falls back to in the male set (the human male body).
const FALLBACK_FRAME_NUMBER: u16 = 400;
/// Animation id of both entries (the `000001` of their names).
const PAPERDOLL_ANIMATION_ID: u32 = 1;

fn packages() -> Vec<(&'static str, PathBuf)> {
    let mut found: Vec<_> = common::client_files(PAPERDOLL_UOP_NAME);
    if std::env::var_os(common::KR_CLIENT_DIR_ENV).is_some() {
        found.push((common::KR_CLIENT_DIR_ENV, common::kr_client_file(PAPERDOLL_UOP_NAME)));
    }
    assert!(!found.is_empty(), "no {PAPERDOLL_UOP_NAME} found");
    found
}

#[test]
#[ignore = "needs client files: set UO_EC_CLIENT_DIR and/or UO_KR_CLIENT_DIR, run with --ignored"]
fn paperdoll_frame_sets_round_trip() {
    for (env, path) in packages() {
        let package = UopPackage::load(&path).unwrap();
        let sized = package.iter_files().filter(|file| file.has_size()).count();
        assert_eq!(sized, GENDER_ENTRIES.len(), "{env}: entries");
        for (name, gender) in GENDER_ENTRIES {
            let payload = package.unpack_file_by_hash(hash_file_name_single(name)).unwrap().unwrap();
            let (file, layout) = AmouFile::parse(&payload).unwrap();
            assert_eq!(file.to_bytes().unwrap(), payload, "{env} {name}: container");
            assert_eq!(file.animation_id, PAPERDOLL_ANIMATION_ID, "{env} {name}: animation id");

            let animation = AnimationFrame::load(&payload).unwrap();
            let keys: Vec<_> = animation.frames.iter().map(|entry| (entry.action, entry.frame_number)).collect();
            assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{env} {name}: frame table sorted, unique");
            assert!(keys.iter().all(|&(action, _)| action == gender), "{env} {name}: gender action");
            if gender == GENDER_ENTRIES[0].1 {
                assert!(keys.contains(&(gender, FALLBACK_FRAME_NUMBER)), "{env} {name}: fallback frame");
            }

            let keyed: Vec<_> = animation
                .frames
                .iter()
                .map(|entry| (entry.action, entry.frame_number, animation.decode_frame_indexed(entry).unwrap()))
                .collect();
            let reencoded = AmouFile::from_indexed_frames(file.animation_id, file.colours.clone(), &keyed)
                .unwrap()
                .to_bytes()
                .unwrap();
            assert_eq!(reencoded, payload, "{env} {name}: frames re-encoded from their indices");
            let numbers = keys.iter().map(|&(_, number)| number);
            println!(
                "{env} {name}: {} frames (numbers {}..={}), {} colours, canonical layout {}; container and frame \
                 re-encode byte-identical ({} bytes)",
                keys.len(),
                numbers.clone().min().unwrap(),
                numbers.max().unwrap(),
                file.colours.len(),
                layout.is_canonical(),
                payload.len()
            );
        }
    }
}
