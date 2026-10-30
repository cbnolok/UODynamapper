//! Re-encodes the `waypoint.bin` of a real client `waypoint.uop` and rewrites the package with an edited waypoint.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`. The rewritten package goes to
//! `std::env::temp_dir()` (set `TMPDIR` to choose it) and is deleted afterwards.

mod common;

use uocf::enhanced::waypoints::{waypoints_payload_hash, WaypointsPackage, WAYPOINTS_UOP_NAME};
use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderRecord};
use uocf::uop_container::package::UopPackage;

/// X offset given to the first waypoint in the rewritten package.
const REWRITTEN_X_DELTA: u32 = 1;

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn waypoints_reencode_identically() {
    for (env, path) in common::client_files(WAYPOINTS_UOP_NAME) {
        let package = UopPackage::load(&path).expect("load waypoint.uop");
        let payload = package
            .get_file_by_hash(waypoints_payload_hash())
            .expect("waypoint.bin")
            .unpack()
            .expect("unpack");
        let waypoints = WaypointsPackage::from_payload_bytes(&payload).expect("waypoints parse");
        assert!(waypoints.to_payload_bytes() == payload, "{env}: waypoint.bin re-encodes differently");
        println!(
            "{env}: {} waypoints, {} types, {} icons, {} effects, {} trailing bytes re-encode identically",
            waypoints.waypoints.len(),
            waypoints.type_definitions.len(),
            waypoints.icon_definitions.len(),
            waypoints.effect_definitions.len(),
            waypoints.trailing_bytes.len()
        );
    }
}

#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn rewritten_waypoints_package_holds_the_edit() {
    for (env, path) in common::client_files(WAYPOINTS_UOP_NAME) {
        let source = UopPackage::load(&path).expect("load waypoint.uop");
        let mut waypoints = WaypointsPackage::from_package(&source).expect("waypoints parse");
        waypoints.waypoints[0].x += REWRITTEN_X_DELTA;
        let dir = common::temp_dir("waypoints_rewrite");
        let output = dir.join(WAYPOINTS_UOP_NAME);

        waypoints.write_waypoints_package(&path, &output).expect("write the new package");

        let written = UopPackage::load(&output).expect("reload the new package");
        assert_eq!(written.version(), source.version(), "{env}: package version");
        assert_eq!(WaypointsPackage::from_package(&written).expect("written waypoints parse"), waypoints, "{env}");
        let file = written.get_file_by_hash(waypoints_payload_hash()).expect("written entry");
        let header = DataBlockHeader::parse(file.data_block_header().unwrap()).unwrap();
        assert!(matches!(header.records(), [DataBlockHeaderRecord::Timestamp(_)]), "{env}: header");
        assert_eq!(file.data_block_hash_matches_header().unwrap(), Some(true), "{env}: hash");
        std::fs::remove_dir_all(&dir).expect("delete the new package");
        assert!(waypoints.write_waypoints_package(&path, &path).is_err(), "{env}: writing over the source must fail");
    }
}
