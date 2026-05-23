use std::fs;

use uocf::classic::map_statics_diff::StaticDiff;
use uocf::classic::statics::StaticsReader;

mod common;

#[test]
fn stadif_stadifi_and_stadifl_override_base_static_block() {
    let dir = common::temp_dir("classic_static_diff");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let lookup_path = dir.join("stadifl0.mul");
    let diff_idx_path = dir.join("stadifi0.mul");
    let diff_path = dir.join("stadif0.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    common::write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    common::write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, common::static_tile(100, 1, 2, 3, 4)).unwrap();

    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    let mut diff_idx = fs::File::create(&diff_idx_path).unwrap();
    common::write_index_entry(&mut diff_idx, 0, 7, 0);
    fs::write(&diff_path, common::static_tile(200, 2, 3, -4, 5)).unwrap();

    let diff = StaticDiff::load(&lookup_path, &diff_idx_path, &diff_path).unwrap();
    let reader = StaticsReader::new_with_patches(
        &idx_path,
        &mul_path,
        8,
        16,
        Some(diff),
        None,
    )
    .unwrap();

    let tiles = reader.read_block(0, 1).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].graphic, 200);
    assert_eq!(tiles[0].z, -4);

    let _ = fs::remove_dir_all(dir);
}
