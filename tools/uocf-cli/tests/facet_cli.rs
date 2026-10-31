//! `uocf-cli facet from-classic --map-edition`: the variant edition reads the `x` files of the client directory and
//! writes `facetNx.uop`/`facetNNx.dds`; a missing variant fails naming the `x` files.
//! `--use-verdata` patches the standard `map0.mul` only, is off by default, and a missing `verdata.mul` is no error.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use uocf::classic::verdata::{VerFileId, VERDATA_FILE_NAME};
use uocf::enhanced::facet_package::EcFacetPackage;

/// A one-block-wide, two-block-tall map.
const MAP_WIDTH: u32 = 8;
const MAP_HEIGHT: u32 = 16;
const MAP_BLOCKS: usize = 2;
const MAP_CELLS_PER_BLOCK: usize = 64;
const STANDARD_LAND: u16 = 3;
const VARIANT_LAND: u16 = 9;
const VERDATA_LAND: u16 = 5;
/// `verdata.mul`: its entry count, then each entry's file id, index, lookup, length and extra.
const VERDATA_ENTRY_FIELDS: usize = 5;
const VERDATA_ENTRIES: usize = 1;
const VERDATA_HEADER_LEN: usize = size_of::<i32>() * (1 + VERDATA_ENTRIES * VERDATA_ENTRY_FIELDS);
const PATCHED_BLOCK: i32 = 0;
const VERDATA_EXTRA: i32 = 0;
const FACET: u8 = 0;
/// Radar colours for land ids up to the ones used here.
const RADARCOL_COLORS: usize = 16;

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path = std::env::temp_dir().join(format!("uocf-facet-cli-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// A map block of land `land` at z 0.
fn map_block(land: u16) -> Vec<u8> {
    let mut block = 0u32.to_le_bytes().to_vec();
    for _ in 0..MAP_CELLS_PER_BLOCK {
        block.extend_from_slice(&land.to_le_bytes());
        block.push(0);
    }
    block
}

/// `mapN{suffix}.mul` of land `land` at z 0, with empty `staidx`/`statics`.
fn write_facet_files(dir: &Path, suffix: &str, land: u16) {
    let map = map_block(land).repeat(MAP_BLOCKS);
    fs::write(dir.join(format!("map{FACET}{suffix}.mul")), map).unwrap();
    let empty_entry: Vec<u8> = [u32::MAX, 0, 0].iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(dir.join(format!("staidx{FACET}{suffix}.mul")), empty_entry.repeat(MAP_BLOCKS)).unwrap();
    fs::write(dir.join(format!("statics{FACET}{suffix}.mul")), b"").unwrap();
}

/// `verdata.mul` replacing map block [`PATCHED_BLOCK`] with land `land`.
fn write_map_verdata(dir: &Path, land: u16) {
    let block = map_block(land);
    let mut bytes = Vec::new();
    for field in [VERDATA_ENTRIES as i32, VerFileId::Map as i32, PATCHED_BLOCK, VERDATA_HEADER_LEN as i32, block.len() as i32, VERDATA_EXTRA] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(&block);
    fs::write(dir.join(VERDATA_FILE_NAME), bytes).unwrap();
}

fn from_classic_with(client_dir: &Path, output_dir: &Path, edition: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
        .args(["facet", "from-classic", "--client-dir"])
        .arg(client_dir)
        .args(["--facet", &FACET.to_string(), "--width", &MAP_WIDTH.to_string(), "--height", &MAP_HEIGHT.to_string()])
        .args(["--map-edition", edition, "--output-dir"])
        .arg(output_dir)
        .args(extra)
        .output()
        .expect("run uocf-cli")
}

fn from_classic(client_dir: &Path, output_dir: &Path, edition: &str) -> std::process::Output {
    from_classic_with(client_dir, output_dir, edition, &[])
}

/// Land of square (0, 0) of the `facet0{suffix}.uop` in `output_dir`.
fn first_land(output_dir: &Path, suffix: &str) -> u16 {
    let path = output_dir.join(format!("facet{FACET}{suffix}.uop"));
    let mut package = EcFacetPackage::open(&path, FACET).expect("open facet package");
    package.square(0, 0).unwrap().land_graphic
}

#[test]
fn from_classic_reads_the_variant_edition_into_facet_nx() {
    let dir = temp_dir("variant");
    let client = dir.join("client");
    let output = dir.join("out");
    fs::create_dir_all(&client).unwrap();
    write_facet_files(&client, "", STANDARD_LAND);
    fs::write(client.join("radarcol.mul"), vec![0u8; RADARCOL_COLORS * size_of::<u16>()]).unwrap();

    let missing = from_classic(&client, &output, "variant");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("variant edition of map0") && stderr.contains("map0x.mul"), "{stderr}");

    write_facet_files(&client, "x", VARIANT_LAND);
    let run = from_classic(&client, &output, "variant");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(output.join("facet00x.dds").is_file());
    assert!(!output.join("facet0.uop").exists());
    let mut package = EcFacetPackage::open(output.join("facet0x.uop"), FACET).expect("open facet0x.uop");
    assert_eq!(package.square(0, 0).unwrap().land_graphic, VARIANT_LAND);

    let run = from_classic(&client, &output, "standard");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let mut package = EcFacetPackage::open(output.join("facet0.uop"), FACET).expect("open facet0.uop");
    assert_eq!(package.square(0, 0).unwrap().land_graphic, STANDARD_LAND);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn from_classic_applies_verdata_to_the_standard_map_only_when_asked() {
    let dir = temp_dir("verdata");
    let client = dir.join("client");
    let output = dir.join("out");
    fs::create_dir_all(&client).unwrap();
    write_facet_files(&client, "", STANDARD_LAND);
    write_facet_files(&client, "x", VARIANT_LAND);
    fs::write(client.join("radarcol.mul"), vec![0u8; RADARCOL_COLORS * size_of::<u16>()]).unwrap();

    let missing = from_classic_with(&client, &output, "standard", &["--use-verdata"]);
    assert!(missing.status.success(), "{}", String::from_utf8_lossy(&missing.stderr));
    assert_eq!(first_land(&output, ""), STANDARD_LAND);

    write_map_verdata(&client, VERDATA_LAND);
    let run = from_classic(&client, &output, "standard");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(first_land(&output, ""), STANDARD_LAND);

    let run = from_classic_with(&client, &output, "standard", &["--use-verdata"]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(first_land(&output, ""), VERDATA_LAND);

    let run = from_classic_with(&client, &output, "variant", &["--use-verdata"]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(first_land(&output, "x"), VARIANT_LAND);

    let _ = fs::remove_dir_all(dir);
}
