//! Checks a real `mobile_anim_ec.uddp` against the Enhanced Client files it was packed from: the creature fields
//! of the body records against the `tileart.uop` placeholders and `AnimationSequence.uop`, and the shadow palette
//! statistics of the frame records against the `AnimationFrame*.uop` palettes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use udd_assets::mobile_anim_ec::{MobileAnimEcPackage, ShadowPaletteStats};
use uocf::animation_sequence::AnimationSequence;
use uocf::enhanced::animationframe::AnimationFrame;
use uocf::enhanced::mobile_body::MobileBodyMap;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
/// Path of the package to check.
const PACKAGE_ENV: &str = "UDD_MOBILE_ANIM_EC_UDDP";
const TILEART_UOP_NAME: &str = "tileart.uop";
const SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
const FRAME_UOP_NAMES: [&str; 6] = [
    "AnimationFrame1.uop",
    "AnimationFrame2.uop",
    "AnimationFrame3.uop",
    "AnimationFrame4.uop",
    "AnimationFrame5.uop",
    "AnimationFrame6.uop",
];
/// Bodies whose frame palettes are compared: human male and female, and the giant serpent frames.
const SHADOW_BODIES: [u32; 3] = [400, 401, 124];
/// Actions compared per body: the stand action and a walk action.
const SHADOW_ACTIONS: [u16; 2] = [0, 1];
/// Directions whose frames are compared.
const DIRECTIONS: std::ops::Range<u8> = 0..udd_assets::mobile_anim_ec::DIRECTION_COUNT as u8;

fn paths() -> Option<(PathBuf, PathBuf)> {
    let client = std::env::var_os(EC_CLIENT_DIR_ENV).map(PathBuf::from);
    let package = std::env::var_os(PACKAGE_ENV).map(PathBuf::from);
    match (client, package) {
        (Some(client), Some(package)) => Some((client, package)),
        _ => {
            println!("set {EC_CLIENT_DIR_ENV} and {PACKAGE_ENV}; skipped");
            None
        }
    }
}

#[test]
#[ignore = "needs a real Enhanced Client and package: set UO_EC_CLIENT_DIR and UDD_MOBILE_ANIM_EC_UDDP, run with --ignored"]
fn real_package_bodies_carry_the_placeholder_creature_fields() {
    let Some((client, package_path)) = paths() else { return };
    let package = MobileAnimEcPackage::load(&package_path).unwrap();
    let map = MobileBodyMap::load(client.join(TILEART_UOP_NAME)).unwrap();
    let sequences = UopPackage::load(client.join(SEQUENCE_UOP_NAME)).unwrap();

    let mut stored_ids = BTreeMap::new();
    let mut races = BTreeMap::<Option<u8>, usize>::new();
    let mut genders = BTreeMap::<Option<u8>, usize>::new();
    for definition in map.definitions.values() {
        let body = package.body(definition.body).unwrap_or_else(|| panic!("body {} missing", definition.body));
        let race = definition.race.map(|race| u8::try_from(race).unwrap());
        let gender = definition.gender.map(|gender| u8::try_from(gender).unwrap());
        assert_eq!((body.race(), body.gender()), (race, gender), "body {}", definition.body);
        *races.entry(race).or_default() += 1;
        *genders.entry(gender).or_default() += 1;

        let stored_id = *stored_ids.entry(definition.sequence_id).or_insert_with(|| {
            let hash = hash_file_name_single(&AnimationSequence::entry_path(definition.sequence_id));
            sequences
                .unpack_file_by_hash(hash)
                .unwrap()
                .map(|data| AnimationSequence::parse(&data).unwrap().stored_id)
        });
        assert_eq!(body.sequence_stored_id, stored_id.unwrap_or(definition.sequence_id), "body {}", definition.body);
    }
    let differing = package.bodies().iter().filter(|body| body.sequence_stored_id != body.sequence_id).count();
    println!(
        "{} bodies; races {races:?}; genders {genders:?}; {differing} with a stored sequence id other than the \
         sequence id",
        package.bodies().len()
    );
}

#[test]
#[ignore = "needs a real Enhanced Client and package: set UO_EC_CLIENT_DIR and UDD_MOBILE_ANIM_EC_UDDP, run with --ignored"]
fn real_package_frames_carry_their_palette_shadow_statistics() {
    let Some((client, package_path)) = paths() else { return };
    let package = MobileAnimEcPackage::load(&package_path).unwrap();
    let frame_packages: Vec<UopPackage> = FRAME_UOP_NAMES
        .iter()
        .map(|name| client.join(name))
        .filter(|path| path.is_file())
        .map(|path| UopPackage::load(&path).unwrap())
        .collect();

    let mut compared = 0usize;
    for body in SHADOW_BODIES {
        // Frame records do not name their source body: compare only bodies that use their own frames.
        if package.body(body).is_none_or(|record| record.frames_body_id != body) {
            println!("body {body} does not use its own frames, skipped");
            continue;
        }
        for action in SHADOW_ACTIONS {
            let hash = AnimationFrame::animationframe_hash(body, action);
            let Some(payload) = frame_packages.iter().find_map(|package| package.unpack_file_by_hash(hash).unwrap())
            else {
                continue;
            };
            let expected = ShadowPaletteStats::from_palette(&AnimationFrame::load(&payload).unwrap().colours);
            for direction in DIRECTIONS {
                let Some(animation) = package.animation(body, action, direction) else { continue };
                for frame in package.animation_frames(animation) {
                    if frame.source_action == action {
                        assert_eq!(frame.shadow, expected, "body {body} action {action} direction {direction}");
                        compared += 1;
                    }
                }
            }
            println!("body {body} action {action}: shadow {expected:?}");
        }
    }
    assert!(compared > 0, "no frames compared");
    println!("{compared} frame records compared");
}
