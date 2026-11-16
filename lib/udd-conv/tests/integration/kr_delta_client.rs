//! Makes the Kingdom Reborn delta from the real clients and checks that it is the one shipped in
//! `lib/uocf/assets/kr`, and that the shipped one rebuilds the Kingdom Reborn files from the Enhanced Client: the data
//! files byte for byte, every texture as Kingdom Reborn's or the Enhanced one it stands for, nothing replaced.
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix); the installs are the folders named
//! [`KINGDOM_REBORN_FOLDER`] and [`ENHANCED_FOLDER`] in one of them. The clients are only read; the rebuilt files go to
//! Cargo's target temp folder and are removed. Run with `--ignored --release`.

use std::fs;
use std::path::{Path, PathBuf};

use udd_conv::kr_delta::{load_kr_delta, make_kr_delta, rebuild_kr_client, verify_rebuilt_client};

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
const KINGDOM_REBORN_FOLDER: &str = "UO - Old KR";
const ENHANCED_FOLDER: &str = "_Ultima Online Enhanced";

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn the_shipped_delta_is_the_clients_delta_and_rebuilds_kingdom_reborn() {
    let kingdom_reborn = install(KINGDOM_REBORN_FOLDER);
    let enhanced = install(ENHANCED_FOLDER);

    let (made, _) = make_kr_delta(&kingdom_reborn, &enhanced).unwrap();
    let shipped = load_kr_delta(None).unwrap();
    assert!(made.files == shipped.files, "the shipped kr_delta.uddp is not the delta of these clients: run pack make-kr-delta");

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("kr_delta_client_{}", std::process::id()));
    let replaced = rebuild_kr_client(&shipped, &enhanced, &dir).unwrap();
    let verified = verify_rebuilt_client(&shipped, &kingdom_reborn, &enhanced, &dir);
    fs::remove_dir_all(&dir).unwrap();
    assert!(replaced.is_empty(), "entries replaced: {replaced:?}");
    verified.unwrap();
}
