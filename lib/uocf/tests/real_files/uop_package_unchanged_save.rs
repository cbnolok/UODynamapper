//! Loads every UOP package of a real Enhanced Client, Kingdom Reborn client and Classic clients and saves it
//! unchanged (`SaveLayout::Preserve`, the default of `UopPackage::finalize_and_save`): every saved file must be
//! byte-identical to its source.
//!
//! The client directories come from `UO_EC_CLIENT_DIR`, `common::KR_CLIENT_DIR_ENV` and `UO_CLASSIC_CLIENT_ROOTS`
//! (Classic client directories, separated like `PATH`; those without packages are skipped). Packages
//! are written one at a time to `std::env::temp_dir()` (set `TMPDIR` to choose it; the largest is about 570 MB) and
//! each is deleted after the comparison. Run with `--release`.

use crate::common;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use uocf::uop_container::package::UopPackage;

const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
const CLASSIC_CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const UOP_EXTENSION: &str = "uop";
/// Packages of the reference clients: the 2024 Enhanced Client and the 2008 Kingdom Reborn client.
const EC_PACKAGES: usize = 40;
const KR_PACKAGES: usize = 34;
/// Packages of the canonical Classic client (2024, `_Ultima Online Classic`).
const CLASSIC_PACKAGES: usize = 22;
/// Read buffer of the file comparison.
const COMPARE_CHUNK: usize = 1 << 20;

/// The `*.uop` files of `dir` (extension matched ignoring ASCII case), sorted by name.
fn packages_in(dir: &Path) -> Vec<PathBuf> {
    let mut packages: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case(UOP_EXTENSION))
        })
        .collect();
    packages.sort();
    packages
}

/// The offset of the first differing byte, or `None` when the files are identical.
fn first_difference(a: &Path, b: &Path) -> Option<u64> {
    let (len_a, len_b) = (a.metadata().unwrap().len(), b.metadata().unwrap().len());
    let (mut ra, mut rb) = (BufReader::new(File::open(a).unwrap()), BufReader::new(File::open(b).unwrap()));
    let (mut ba, mut bb) = (vec![0u8; COMPARE_CHUNK], vec![0u8; COMPARE_CHUNK]);
    let mut offset = 0u64;
    loop {
        let na = read_full(&mut ra, &mut ba);
        let nb = read_full(&mut rb, &mut bb);
        if let Some(at) = ba[..na.min(nb)].iter().zip(&bb[..na.min(nb)]).position(|(x, y)| x != y) {
            return Some(offset + at as u64);
        }
        if na != nb {
            return Some(offset + na.min(nb) as u64);
        }
        if na == 0 {
            return (len_a != len_b).then_some(len_a.min(len_b));
        }
        offset += na as u64;
    }
}

fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> usize {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]).unwrap() {
            0 => break,
            n => filled += n,
        }
    }
    filled
}

/// Saves every package of `dir` unchanged and asserts that each comes back byte for byte.
fn check_client(label: &str, dir: &Path, expected_packages: usize) {
    let packages = packages_in(dir);
    let out = common::temp_dir(&format!("unchanged_save_{label}"));
    let mut differing = Vec::new();
    for path in &packages {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let output = out.join(&name);
        let mut package = UopPackage::load(path).unwrap_or_else(|error| panic!("{label} {name}: load: {error}"));
        package.finalize_and_save(&output).unwrap_or_else(|error| panic!("{label} {name}: save: {error}"));
        drop(package);
        match first_difference(path, &output) {
            None => println!("{label} {name}: {} bytes written back byte for byte", path.metadata().unwrap().len()),
            Some(at) => {
                println!("{label} {name}: differs at byte {at:#x}");
                differing.push(name);
            }
        }
        std::fs::remove_file(&output).unwrap();
    }
    std::fs::remove_dir(&out).unwrap();
    assert!(differing.is_empty(), "{label}: packages not written back byte for byte: {differing:?}");
    println!("{label}: {} packages written back byte for byte", packages.len());
    if packages.len() != expected_packages {
        println!("{label}: (the reference client has {expected_packages} packages)");
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored --release"]
fn every_enhanced_client_package_saves_unchanged_byte_for_byte() {
    let dir = std::env::var_os(EC_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {EC_CLIENT_DIR_ENV}"));
    check_client("EC", Path::new(&dir), EC_PACKAGES);
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn every_kingdom_reborn_package_saves_unchanged_byte_for_byte() {
    let dir = std::env::var_os(common::KR_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {}", common::KR_CLIENT_DIR_ENV));
    check_client("KR", Path::new(&dir), KR_PACKAGES);
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn every_classic_client_package_saves_unchanged_byte_for_byte() {
    let roots =
        std::env::var_os(CLASSIC_CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLASSIC_CLIENT_ROOTS_ENV}"));
    let mut checked = 0usize;
    for root in std::env::split_paths(&roots) {
        if packages_in(&root).is_empty() {
            println!("{}: no packages", root.display());
            continue;
        }
        let label = root.file_name().unwrap().to_string_lossy().into_owned();
        check_client(&label, &root, CLASSIC_PACKAGES);
        checked += 1;
    }
    assert!(checked > 0, "no Classic client in {CLASSIC_CLIENT_ROOTS_ENV} has packages");
}
