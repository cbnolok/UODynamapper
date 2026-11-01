//! Checks the owner-reference census (`pack audit-ec-material-refs`) on the real clients: it has one row per
//! tileart texture item and per terrain-definition layer, no key twice, and every tileart row agrees with the
//! ref `tilemeta.uddp` packs for the same item (texture id, support flag, stretch, source kind, sampler, UV set).
//! The census and tilemeta read the tileart through different code paths, so they check each other.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix); the installs are the folders
//! named [`CLASSIC_FOLDER`] and [`ENHANCED_FOLDER`] in one of them. The clients are only read; the outputs go
//! to `std::env::temp_dir()`. Run with `--ignored --release`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use udd_assets::TileMetaPackage;
use udd_conv::tex_art_ec::load_tex_art_ec_sources;
use udd_conv::tilemeta::{build_tilemeta_uddp_from_split_sources, TileMetaBuildOptions};
use udd_conv_cli::ec_material_audit::audit_ec_material_refs;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const CLASSIC_FOLDER: &str = "_Ultima Online Classic";
const ENHANCED_FOLDER: &str = "_Ultima Online Enhanced";

/// Census CSV columns read here.
const COL_OWNER_KIND: usize = 0;
const COL_OWNER_ID: usize = 1;
const COL_BLOCK: usize = 7;
const COL_ITEM: usize = 8;
const COL_TEXTURE_ID: usize = 9;
const COL_PHYSICAL_PACKAGE: usize = 12;
const COL_IS_SUPPORT: usize = 14;
const COL_STRETCH: usize = 15;
const COL_SOURCE_KIND: usize = 16;
const COL_SAMPLER: usize = 17;
const COL_UV_SET: usize = 18;
const OWNER_TILEART: &str = "tileart";
const OWNER_TERRAIN: &str = "terrain_definition";
/// Mismatches listed in the failure message.
const MAX_LISTED: usize = 16;

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

/// The census formats floats with six decimals.
fn census_float(value: f32) -> String {
    if value.is_finite() { format!("{value:.6}") } else { value.to_string() }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn the_owner_reference_census_covers_every_ref_once_and_agrees_with_tilemeta() {
    let classic = install(CLASSIC_FOLDER);
    let enhanced = install(ENHANCED_FOLDER);
    let dir = std::env::temp_dir().join(format!("udd_cli_ec_material_refs_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let census_path = dir.join("ec_material_owner_refs.csv");
    let tilemeta_path = dir.join("tilemeta.uddp");

    audit_ec_material_refs(&[enhanced.clone()], &census_path).unwrap();
    build_tilemeta_uddp_from_split_sources(&classic, &enhanced, &tilemeta_path, &TileMetaBuildOptions::default())
        .unwrap();
    let sources = load_tex_art_ec_sources(&[enhanced.clone()]).unwrap();
    let tilemeta = TileMetaPackage::load(&tilemeta_path).unwrap();

    let mut reader = csv::Reader::from_path(&census_path).unwrap();
    let rows: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
    fs::remove_dir_all(&dir).ok();

    // One row per tileart texture item and per terrain-definition layer.
    let tileart_items: usize = sources
        .art_definition
        .definitions
        .values()
        .map(|art| art.texture_items.iter().map(Vec::len).sum::<usize>())
        .sum();
    let terrain_layers: usize = sources
        .terrain_definition
        .entries
        .iter()
        .filter_map(|entry| entry.texture.as_ref())
        .map(|texture| texture.layers.len())
        .sum();
    let count = |kind: &str| rows.iter().filter(|row| &row[COL_OWNER_KIND] == kind).count();
    assert_eq!(count(OWNER_TILEART), tileart_items, "tileart rows");
    assert_eq!(count(OWNER_TERRAIN), terrain_layers, "terrain-definition rows");
    assert_eq!(rows.len(), tileart_items + terrain_layers, "rows of another owner kind");
    assert!(tileart_items > 0 && terrain_layers > 0, "the client gave an empty census");

    let mut keys = HashSet::new();
    let duplicates: Vec<_> = rows
        .iter()
        .map(|row| {
            (row[COL_OWNER_KIND].to_owned(), row[COL_OWNER_ID].to_owned(), row[COL_BLOCK].to_owned(), row[COL_ITEM].to_owned())
        })
        .filter(|key| !keys.insert(key.clone()))
        .collect();
    assert!(duplicates.is_empty(), "census keys seen twice: {:?}", &duplicates[..duplicates.len().min(MAX_LISTED)]);

    // Every tileart row agrees with the tilemeta ref of the same item.
    let tileart_rows: HashMap<(u32, u8, u8), &csv::StringRecord> = rows
        .iter()
        .filter(|row| &row[COL_OWNER_KIND] == OWNER_TILEART)
        .map(|row| ((row[COL_OWNER_ID].parse().unwrap(), row[COL_BLOCK].parse().unwrap(), row[COL_ITEM].parse().unwrap()), row))
        .collect();
    let mut compared = 0usize;
    let mut not_in_census = Vec::new();
    let mut disagreements = Vec::new();
    for item in tilemeta.item_tiles() {
        for tex_ref in tilemeta.item_texture_refs(item.tile_id) {
            let key = (item.tile_id, tex_ref.block_index, tex_ref.item_index);
            let Some(row) = tileart_rows.get(&key) else {
                not_in_census.push(key);
                continue;
            };
            compared += 1;
            let expected = [
                (COL_TEXTURE_ID, tex_ref.texture_id.to_string()),
                (COL_IS_SUPPORT, tex_ref.is_auxiliary().to_string()),
                (COL_STRETCH, census_float(tex_ref.texture_stretch)),
                (COL_SOURCE_KIND, tex_ref.source_kind.to_string()),
                (COL_SAMPLER, tex_ref.sampler_index.to_string()),
                (COL_UV_SET, tex_ref.uv_set.to_string()),
            ];
            for (column, value) in expected {
                if row[column] != value {
                    disagreements.push((key, column, row[column].to_owned(), value));
                }
            }
        }
    }
    let unknown_packages = rows.iter().filter(|row| &row[COL_PHYSICAL_PACKAGE] == "Unknown").count();
    println!(
        "census: {tileart_items} tileart rows, {terrain_layers} terrain rows ({unknown_packages} with an unknown \
         archive); {compared} tilemeta refs compared"
    );
    assert!(compared > 0, "no tilemeta ref was compared");
    assert!(not_in_census.is_empty(), "tilemeta refs missing from the census: {:?}", &not_in_census[..not_in_census.len().min(MAX_LISTED)]);
    assert!(
        disagreements.is_empty(),
        "{} census cells disagree with tilemeta ((tile, block, item), column, census, tilemeta): {:?}",
        disagreements.len(),
        &disagreements[..disagreements.len().min(MAX_LISTED)]
    );
}
