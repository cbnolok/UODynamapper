use std::fs;

use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::map_statics_diff::MapDiff;
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::package::UopPackage;

mod common;

#[test]
fn mapdif_and_mapdifl_override_base_map_block() {
    let dir = common::temp_dir("classic_map_diff");
    let map_path = dir.join("map0.mul");
    let lookup_path = dir.join("mapdifl0.mul");
    let diff_path = dir.join("mapdif0.mul");

    let mut map_bytes = Vec::new();
    map_bytes.extend_from_slice(&common::map_block(1, 2));
    map_bytes.extend_from_slice(&common::map_block(3, 4));
    fs::write(&map_path, map_bytes).unwrap();
    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    fs::write(&diff_path, common::map_block(55, -6)).unwrap();

    let diff = MapDiff::load(&lookup_path, &diff_path).unwrap();
    let mut plane = MapPlane::init_with_size_and_diff(
        map_path,
        0,
        Some(MapSizeCells {
            width: 8,
            height: 16,
        }),
        diff,
    )
    .unwrap();

    let mut blocks = [MapBlockRelPos { x: 0, y: 1 }];
    plane.load_blocks(&mut blocks).unwrap();
    let block = plane.block_no_update(MapBlockRelPos { x: 0, y: 1 }).unwrap();
    let cell = block.cell(0, 0).unwrap();
    assert_eq!(cell.id, 55);
    assert_eq!(cell.z, -6);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn mapx_legacy_uop_uses_base_map_internal_route() {
    let dir = common::temp_dir("classic_mapx_legacy_uop");
    let uop_path = dir.join("map0xLegacyMUL.uop");

    let mut chunk = Vec::new();
    chunk.extend_from_slice(&common::map_block(10, 1));
    chunk.extend_from_slice(&common::map_block(77, -3));

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            &chunk,
            "build/map0legacymul/00000000.dat",
            CompressionFlag::None,
        )
        .unwrap();
    package.finalize_and_save(&uop_path).unwrap();

    let mut plane = MapPlane::init_uop_with_size(
        uop_path,
        0,
        Some(MapSizeCells {
            width: 8,
            height: 16,
        }),
    )
    .unwrap();

    let mut blocks = [MapBlockRelPos { x: 0, y: 1 }];
    plane.load_blocks(&mut blocks).unwrap();
    let block = plane.block_no_update(MapBlockRelPos { x: 0, y: 1 }).unwrap();
    let cell = block.cell(0, 0).unwrap();
    assert_eq!(cell.id, 77);
    assert_eq!(cell.z, -3);

    let _ = fs::remove_dir_all(dir);
}
