//! The Enhanced records that take Kingdom Reborn's world-art window (`kr_items::adopt_kr_world_windows`) on the real
//! clients: the 215 of `docs/client_research/world-art-windows`. The roots come from `UO_CLIENT_ROOTS` (a path list,
//! `:`-separated on Unix); the installs are the folders named [`KINGDOM_REBORN_FOLDER`] and [`ENHANCED_FOLDER`] in one of
//! them. Only their tileart and string dictionaries are read. Run with `--ignored`.

use std::path::PathBuf;

use udd_conv::kr_items::adopt_kr_world_windows;
use uocf::enhanced::tile_database::ArtDefinition;
use uocf::enhanced::tileart::TILEART_UOP_NAME;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const KINGDOM_REBORN_FOLDER: &str = "UO - Old KR";
const ENHANCED_FOLDER: &str = "_Ultima Online Enhanced";
const STRING_DICTIONARY: &str = "string_dictionary.uop";

/// The records whose Enhanced window was never authored, on the shipped clients.
const ADOPTED_WINDOWS: usize = 215;
/// Some of them: the Dog Frame, the desert ostard (drawn blank by its Enhanced window), the bird (on its perch with
/// Kingdom Reborn's), the blank wisp and death vortex.
const KNOWN_ADOPTED: [u16; 5] = [0x20D5, 0x2135, 0x20EE, 0x2100, 0x3789];

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

fn tileart(folder: &str) -> ArtDefinition {
    let dir = install(folder);
    ArtDefinition::load(&dir.join(TILEART_UOP_NAME), &dir.join(STRING_DICTIONARY)).unwrap()
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn the_enhanced_records_take_the_215_kingdom_reborn_windows() {
    let mut enhanced = tileart(ENHANCED_FOLDER);
    let kingdom_reborn = tileart(KINGDOM_REBORN_FOLDER);
    let adopted = adopt_kr_world_windows(&mut enhanced, &kingdom_reborn);
    assert_eq!(adopted.len(), ADOPTED_WINDOWS, "{adopted:x?}");
    for id in KNOWN_ADOPTED {
        assert!(adopted.contains(&id), "{id:#06x}");
        assert_eq!(enhanced.sprite_descriptors[&id].world_window, kingdom_reborn.sprite_descriptors[&id].world_window);
    }
    assert!(adopt_kr_world_windows(&mut enhanced, &kingdom_reborn).is_empty(), "once adopted, nothing is left");
}
