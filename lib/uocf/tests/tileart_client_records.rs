//! Parses every record of a real Enhanced Client `tileart.uop`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use uocf::enhanced::tileart::TileArtEntry;
use uocf::uop_container::package::UopPackage;

/// Environment variable naming the Enhanced Client directory that contains `tileart.uop`.
const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const TILEART_UOP_NAME: &str = "tileart.uop";

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn tileart_parses_every_client_record() {
    let client_dir = std::env::var(EC_CLIENT_DIR_ENV)
        .unwrap_or_else(|_| panic!("{EC_CLIENT_DIR_ENV} must name the Enhanced Client directory"));
    let package = UopPackage::load(PathBuf::from(client_dir).join(TILEART_UOP_NAME))
        .expect("load tileart.uop");

    let mut records = 0usize;
    let mut with_sitting = 0usize;
    let mut with_effects = 0usize;
    let mut effect_definitions = 0usize;
    let mut appearance_by_subtype = BTreeMap::<u8, usize>::new();
    let mut failures = Vec::new();
    let mut leftovers = Vec::new();
    for file in package.iter_files() {
        records += 1;
        let entry = match TileArtEntry::parse_raw(file) {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!("{:016X}: {error}", file.filename_hash()));
                continue;
            }
        };
        with_sitting += usize::from(entry.has_sitting);
        for filter in &entry.appearance_vector {
            *appearance_by_subtype.entry(filter.subtype()).or_default() += 1;
        }
        if entry.effect_count == 0 {
            if !entry.effects_raw.is_empty() {
                leftovers.push(entry.tile_id);
            }
        } else {
            with_effects += 1;
            effect_definitions += usize::from(entry.effect_count);
        }
    }

    println!("records: {records}");
    println!("with sitting: {with_sitting}");
    println!("appearance filters by subtype: {appearance_by_subtype:?}");
    println!("with effects: {with_effects} ({effect_definitions} effect definitions)");
    assert!(failures.is_empty(), "{} records failed to parse: {failures:?}", failures.len());
    assert!(leftovers.is_empty(), "records without effects have leftover bytes: {leftovers:?}");
}
