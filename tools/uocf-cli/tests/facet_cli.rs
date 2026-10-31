//! `uocf-cli facet from-classic --map-edition`: the variant edition reads the `x` files of the client directory and
//! writes `facetNx.uop`/`facetNNx.dds`; a missing variant fails naming the `x` files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use uocf::enhanced::facet_package::EcFacetPackage;

/// A one-block-wide, two-block-tall map.
const MAP_WIDTH: u32 = 8;
const MAP_HEIGHT: u32 = 16;
const MAP_BLOCKS: usize = 2;
const MAP_CELLS_PER_BLOCK: usize = 64;
const STANDARD_LAND: u16 = 3;
const VARIANT_LAND: u16 = 9;
const FACET: u8 = 0;
/// Radar colours for land ids up to the ones used here.
const RADARCOL_COLORS: usize = 16;

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path = std::env::temp_dir().join(format!("uocf-facet-cli-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// `mapN{suffix}.mul` of land `land` at z 0, with empty `staidx`/`statics`.
fn write_facet_files(dir: &Path, suffix: &str, land: u16) {
    let mut map = Vec::new();
    for _ in 0..MAP_BLOCKS {
        map.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..MAP_CELLS_PER_BLOCK {
            map.extend_from_slice(&land.to_le_bytes());
            map.push(0);
        }
    }
    fs::write(dir.join(format!("map{FACET}{suffix}.mul")), map).unwrap();
    let empty_entry: Vec<u8> = [u32::MAX, 0, 0].iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(dir.join(format!("staidx{FACET}{suffix}.mul")), empty_entry.repeat(MAP_BLOCKS)).unwrap();
    fs::write(dir.join(format!("statics{FACET}{suffix}.mul")), b"").unwrap();
}

fn from_classic(client_dir: &Path, output_dir: &Path, edition: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
        .args(["facet", "from-classic", "--client-dir"])
        .arg(client_dir)
        .args(["--facet", &FACET.to_string(), "--width", &MAP_WIDTH.to_string(), "--height", &MAP_HEIGHT.to_string()])
        .args(["--map-edition", edition, "--output-dir"])
        .arg(output_dir)
        .output()
        .expect("run uocf-cli")
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
