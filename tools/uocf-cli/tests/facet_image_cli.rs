//! `facet patch-classic` and `facet convert` keep a real client's world map image (`facetNN.mul`) in step with
//! the map: a patched tile changes only the rows of the tiles it re-renders, and a convert of an unchanged map
//! writes the target's image byte for byte.
//!
//! `UO_CC_CLIENT_DIR` is a Classic client folder that ships `facet04.mul` (7.0.20 or later); the outputs go under
//! the temporary folder. Run with `--ignored`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use uocf::classic::facet_image::{facet_image_file_name, FacetImageFile};
use uocf::classic::target_client::CLASSIC_EXE;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
/// Tokuno: the smallest facet with an image.
const FACET: u32 = 4;
/// A tile inside Tokuno, and a land z it does not have.
const TILE: (u32, u32) = (724, 724);
const PATCHED_Z: &str = "100";

fn client_dir() -> PathBuf {
    PathBuf::from(std::env::var_os(CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {CLIENT_DIR_ENV}")))
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    std::env::temp_dir().join(format!("uocf-facet-image-cli-{name}-{}-{unique}", std::process::id()))
}

fn facet(args: &[&str], output_dir: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
        .arg("facet")
        .args(args)
        .arg("--output-dir")
        .arg(output_dir)
        .output()
        .expect("run uocf-cli");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    stdout
}

fn shipped_image(client: &Path) -> (PathBuf, FacetImageFile) {
    let path = uocf::utils::path::find_file_ignoring_case(client, &facet_image_file_name(FACET)).expect("facet04.mul");
    let image = FacetImageFile::load(&path).unwrap();
    (path, image)
}

#[test]
#[ignore = "needs a real client: set UO_CC_CLIENT_DIR and run with --ignored"]
fn a_folder_holding_a_copy_of_the_target_client_is_refused_as_the_output() {
    let client = client_dir();
    let target = temp_dir("target-output");
    fs::create_dir_all(&target).unwrap();
    let exe = target.join(CLASSIC_EXE);
    fs::copy(find_file_ignoring_case(&client, CLASSIC_EXE).expect("client.exe"), &exe).unwrap();
    let kept = target.join(format!("map{FACET}.mul"));
    fs::write(&kept, b"the target client's map").unwrap();
    for target_client in [&target, &exe] {
        let output = Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
            .args(["facet", "convert", "--client-dir", client.to_str().unwrap(), "--facet", &FACET.to_string(), "--to", "classic"])
            .args(["--container", "mul", "--target-client", target_client.to_str().unwrap(), "--output-dir", target.to_str().unwrap()])
            .output()
            .expect("run uocf-cli");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && stderr.contains("holds the target client's files"), "{stderr}");
        assert_eq!(fs::read(&kept).unwrap(), b"the target client's map");
    }
    fs::remove_dir_all(target).unwrap();
}

#[test]
#[ignore = "needs a real client: set UO_CC_CLIENT_DIR and run with --ignored"]
fn patching_one_tile_changes_only_the_rows_it_re_renders() {
    let client = client_dir();
    let (_, shipped) = shipped_image(&client);
    let out = temp_dir("patch");
    let (x, y) = TILE;
    let stdout = facet(
        &["patch-classic", "--client-dir", client.to_str().unwrap(), "--facet", &FACET.to_string(), "--x", &x.to_string(), "--y", &y.to_string(), "--z", PATCHED_Z],
        &out,
    );
    println!("{stdout}");
    let patched = FacetImageFile::load(&out.join(facet_image_file_name(FACET))).unwrap();
    let changed: Vec<u32> = (0..shipped.rows().len()).filter(|&row| shipped.rows()[row] != patched.rows()[row]).map(|row| row as u32).collect();
    // The tile and the one left of it are on row y, the one above on row y - 1.
    assert!(!changed.is_empty() && changed.iter().all(|&row| row == y || row == y - 1), "rows changed: {changed:?}");
    let width = usize::from(shipped.width());
    let (before, after) = (shipped.pixels(), patched.pixels());
    let touched = [(x, y), (x - 1, y), (x, y - 1)].map(|(x, y)| y as usize * width + x as usize);
    assert!((0..before.len()).filter(|&at| before[at] != after[at]).all(|at| touched.contains(&at)));
    fs::remove_dir_all(out).unwrap();
}

#[test]
#[ignore = "needs a real client: set UO_CC_CLIENT_DIR and run with --ignored"]
fn converting_an_unchanged_map_writes_the_image_byte_for_byte() {
    let client = client_dir();
    let (path, _) = shipped_image(&client);
    let out = temp_dir("convert");
    let client_arg = client.to_str().unwrap();
    let stdout = facet(&["convert", "--client-dir", client_arg, "--facet", &FACET.to_string(), "--to", "classic", "--target-client", client_arg], &out);
    println!("{stdout}");
    assert!(stdout.contains("unchanged"), "{stdout}");
    assert_eq!(fs::read(out.join(facet_image_file_name(FACET))).unwrap(), fs::read(path).unwrap());
    fs::remove_dir_all(out).unwrap();
}
