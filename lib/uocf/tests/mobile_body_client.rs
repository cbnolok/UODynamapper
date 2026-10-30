//! Maps every server body of a real Enhanced Client through its `tileart.uop` creature placeholder, and
//! decodes real `AnimationFrame*.uop` pixels.
//!
//! The expected figures are the ones verified by
//! `scripts/dev-helpers/ec-research/ghidra-animation/verify_animation.py --tileart --pixels` on the reference
//! release (the 2024 Enhanced Client); they are asserted only when the placeholder count identifies that
//! release, and printed otherwise.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::{AnimationFrame, MAX_ACTION_ID_FALLBACK, MAX_BODY_ID, OPAQUE_ALPHA};
use uocf::enhanced::hues::{
    EcHueTableRef, EcHueTables, ShadowPaletteStats, SHADOW_ALPHA_MIN, SHADOW_ALPHA_RANGE, SHADOW_HUE,
};
use uocf::enhanced::mobile_body::{MobileBodyMap, FALLBACK_FRAMES_BODY, GHOST_HUE};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const TILEART_UOP_NAME: &str = "tileart.uop";
const HUES_UOP_NAME: &str = "Hues.uop";
const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
const FRAME_UOP_NAMES: [&str; 6] = [
    "AnimationFrame1.uop",
    "AnimationFrame2.uop",
    "AnimationFrame3.uop",
    "AnimationFrame4.uop",
    "AnimationFrame5.uop",
    "AnimationFrame6.uop",
];
/// Sequence entry ids searched.
const MAX_SEQUENCE_ID: u32 = 100_000;

const EXPECTED_PLACEHOLDERS: usize = 1143;
const EXPECTED_OWN_SEQUENCE: usize = 363;
const EXPECTED_APPEARANCE_DIFFERS: usize = 172;
const EXPECTED_FALLBACK_BODIES: usize = 52;
const EXPECTED_FRAME_BODIES: usize = 926;
const EXPECTED_REACHED_FRAME_BODIES: usize = 919;
/// Frame bodies without a sequence of their own, and those still without a placeholder.
const EXPECTED_FRAME_ONLY_BODIES: usize = 559;
const EXPECTED_FRAME_ONLY_WITHOUT_PLACEHOLDER: [u32; 2] = [344, 827];

const HUMAN_MALE: u32 = 400;
const HUMAN_FEMALE: u32 = 401;
const MALE_GHOST: u32 = 402;
/// props1 key 10 of the female human placeholder.
const FEMALE_GENDER: u32 = 1;
const MALE_GHOST_SEQUENCE: u32 = 400;
const MALE_GHOST_FRAMES: u32 = 400;
const GIANT_SERPENT: u32 = 146;
const GIANT_SERPENT_SEQUENCE: u32 = 124;
const STAND_ACTION: u16 = 0;

fn sequence_ids(path: &Path) -> BTreeSet<u32> {
    let package = UopPackage::load(path).unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
    let names: HashMap<u64, u32> =
        (0..MAX_SEQUENCE_ID).map(|id| (hash_file_name_single(&AnimationSequence::entry_path(id)), id)).collect();
    package
        .iter_files()
        .filter(|file| file.has_size())
        .filter_map(|file| names.get(&file.filename_hash()).copied())
        .collect()
}

fn frame_packages(dir: &Path) -> Vec<UopPackage> {
    FRAME_UOP_NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .map(|path| UopPackage::load(&path).unwrap_or_else(|error| panic!("load {}: {error}", path.display())))
        .collect()
}

/// Bodies with at least one frame file.
fn frame_bodies(packages: &[UopPackage]) -> BTreeSet<u32> {
    let mut bodies = BTreeSet::new();
    for body in 0..MAX_BODY_ID {
        let found = (0..MAX_ACTION_ID_FALLBACK).any(|action| {
            let hash = AnimationFrame::animationframe_hash(body, action);
            packages.iter().any(|package| package.get_file_by_hash(hash).is_some_and(|file| file.has_size()))
        });
        if found {
            bodies.insert(body);
        }
    }
    bodies
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn every_placeholder_maps_body_to_sequence_and_frames() {
    for (env, tileart_path) in common::client_files(TILEART_UOP_NAME) {
        let dir = tileart_path.parent().unwrap();
        let map = MobileBodyMap::load(&tileart_path).unwrap();
        let sequences = sequence_ids(&dir.join(SEQUENCE_UOP_NAME));
        let packages = frame_packages(dir);
        let frames = frame_bodies(&packages);

        let definitions = &map.definitions;
        let own_sequence = definitions.values().filter(|definition| definition.sequence_id == definition.body).count();
        let appearance_differs =
            definitions.values().filter(|definition| definition.appearance != definition.body).count();
        let animation_appearances =
            definitions.values().filter(|definition| definition.animation_appearance.is_some()).count();
        let missing_sequences: Vec<u32> = definitions
            .values()
            .filter(|definition| !sequences.contains(&definition.sequence_id))
            .map(|definition| definition.body)
            .collect();
        let resolved: Vec<_> = definitions
            .values()
            .map(|definition| (definition, definition.resolve_frames_body(|body| frames.contains(&body))))
            .collect();
        let fallback = resolved.iter().filter(|(_, frames)| frames.is_some_and(|frames| frames.fallback)).count();
        let unresolved = resolved.iter().filter(|(_, frames)| frames.is_none()).count();
        let reached: BTreeSet<u32> = definitions.values().map(|definition| definition.requested_frames_body()).collect();
        let reached_frame_bodies = frames.intersection(&reached).count();
        let frame_only: BTreeSet<u32> = frames.difference(&sequences).copied().collect();
        let frame_only_without_placeholder: Vec<u32> =
            frame_only.iter().copied().filter(|body| !definitions.contains_key(body)).collect();

        println!(
            "{env}: {} placeholders, {own_sequence} with their own sequence, {appearance_differs} appearances \
             differ, {animation_appearances} animation appearances, {} missing sequences, {fallback} fall back \
             to body {FALLBACK_FRAMES_BODY}, {unresolved} unresolved; {} frame bodies, {reached_frame_bodies} \
             reached; {} frame-only bodies, without placeholder {frame_only_without_placeholder:?}",
            definitions.len(),
            missing_sequences.len(),
            frames.len(),
            frame_only.len(),
        );

        let with_race = definitions.values().filter(|definition| definition.race.is_some()).count();
        let with_gender = definitions.values().filter(|definition| definition.gender.is_some()).count();
        println!("{env}: {with_race} placeholders with a race, {with_gender} with a gender");

        let female = map.definition(HUMAN_FEMALE).unwrap();
        assert_eq!((female.sequence_id, female.requested_frames_body()), (HUMAN_MALE, HUMAN_FEMALE));
        assert_eq!(female.gender, Some(FEMALE_GENDER));
        let ghost = map.definition(MALE_GHOST).unwrap();
        assert_eq!((ghost.sequence_id, ghost.requested_frames_body()), (MALE_GHOST_SEQUENCE, MALE_GHOST_FRAMES));
        assert_eq!(ghost.forced_hue(), Some(GHOST_HUE));
        let serpent = map.definition(GIANT_SERPENT).unwrap();
        assert_eq!((serpent.sequence_id, serpent.requested_frames_body()), (GIANT_SERPENT_SEQUENCE, GIANT_SERPENT_SEQUENCE));
        assert!(missing_sequences.is_empty(), "placeholders without a sequence: {missing_sequences:?}");
        assert_eq!(unresolved, 0);

        if definitions.len() != EXPECTED_PLACEHOLDERS {
            println!("{env}: not the reference release, counts not checked");
            continue;
        }
        assert_eq!(own_sequence, EXPECTED_OWN_SEQUENCE);
        assert_eq!(appearance_differs, EXPECTED_APPEARANCE_DIFFERS);
        assert_eq!(fallback, EXPECTED_FALLBACK_BODIES);
        assert_eq!(frames.len(), EXPECTED_FRAME_BODIES);
        assert_eq!(reached_frame_bodies, EXPECTED_REACHED_FRAME_BODIES);
        assert_eq!(frame_only.len(), EXPECTED_FRAME_ONLY_BODIES);
        assert_eq!(frame_only_without_placeholder, EXPECTED_FRAME_ONLY_WITHOUT_PLACEHOLDER);
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn real_frames_decode_opaque_with_soft_edges_and_hue_masks() {
    for (env, tileart_path) in common::client_files(TILEART_UOP_NAME) {
        let packages = frame_packages(tileart_path.parent().unwrap());
        let hash = AnimationFrame::animationframe_hash(HUMAN_MALE, STAND_ACTION);
        let payload = packages
            .iter()
            .find_map(|package| package.unpack_file_by_hash(hash).unwrap())
            .expect("body 400 stand frames");
        let animation = AnimationFrame::load(&payload).unwrap();

        let mut opaque = 0usize;
        let mut edges = 0usize;
        let mut hueable = 0usize;
        for entry in &animation.frames {
            let frame = animation.decode_frame(entry).unwrap();
            assert_eq!(frame.hue_mask.len(), frame.width as usize * frame.height as usize);
            assert_eq!((frame.center_x, frame.center_y), entry.center());
            for (pixel, &mask) in frame.data.chunks_exact(4).zip(&frame.hue_mask) {
                opaque += usize::from(pixel[3] == OPAQUE_ALPHA);
                edges += usize::from(pixel[3] != 0 && pixel[3] != OPAQUE_ALPHA);
                hueable += usize::from(mask != 0);
            }
        }
        println!("{env}: body {HUMAN_MALE} stand: {opaque} opaque, {edges} edge and {hueable} hueable pixels");
        assert!(opaque > 0 && edges > 0 && hueable > 0);

        let stats = ShadowPaletteStats::from_palette(&animation.colours);
        let alphas: BTreeSet<u8> = animation.colours.iter().map(|c| stats.alpha([c[0], c[1], c[2]])).collect();
        println!("{env}: body {HUMAN_MALE} stand shadow {stats:?}, alphas {:?}..{:?}", alphas.first(), alphas.last());
        assert!(stats.min_inverse <= stats.mean_inverse && stats.mean_inverse <= stats.max_inverse);
        let alpha_range = SHADOW_ALPHA_MIN as u8..=(SHADOW_ALPHA_MIN + SHADOW_ALPHA_RANGE) as u8;
        assert!(alphas.iter().all(|alpha| alpha_range.contains(alpha)));
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn real_hue_tables_include_the_ghost_hue() {
    for (env, hues_path) in common::client_files(HUES_UOP_NAME) {
        let tables = EcHueTables::load(&hues_path).unwrap();
        println!("{env}: {} hue tables", tables.tables.len());
        assert!(matches!(tables.hue_table(GHOST_HUE), EcHueTableRef::Table(_)));
        assert_eq!(tables.hue_table(0), EcHueTableRef::None);
        assert_eq!(tables.hue_table(SHADOW_HUE), EcHueTableRef::Shadow);
    }
}
