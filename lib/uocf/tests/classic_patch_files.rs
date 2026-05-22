use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use uocf::classic::body_def::BodyDef;
use uocf::classic::bodyconv_def::BodyConvDef;
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::map_statics_diff::{MapDiff, StaticDiff};
use uocf::classic::statics::StaticsReader;
use uocf::classic::verdata::{VerFileId, Verdata};

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("uocf_{test_name}_{timestamp}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn map_block(tile_id: u16, z: i8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(196);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..64 {
        bytes.extend_from_slice(&tile_id.to_le_bytes());
        bytes.push(z as u8);
    }
    bytes
}

fn static_tile(graphic: u16, x: u8, y: u8, z: i8, hue: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(7);
    bytes.extend_from_slice(&graphic.to_le_bytes());
    bytes.push(x);
    bytes.push(y);
    bytes.push(z as u8);
    bytes.extend_from_slice(&hue.to_le_bytes());
    bytes
}

fn write_index_entry(file: &mut fs::File, lookup: u32, size: u32, extra: u32) {
    file.write_all(&lookup.to_le_bytes()).unwrap();
    file.write_all(&size.to_le_bytes()).unwrap();
    file.write_all(&extra.to_le_bytes()).unwrap();
}

#[test]
fn body_def_and_bodyconv_def_parse_redirects() {
    let dir = temp_dir("classic_defs");
    let body_path = dir.join("Body.def");
    let bodyconv_path = dir.join("Bodyconv.def");

    fs::write(&body_path, b"0x00C0 { 1 2 0x03E8 } 44 # comment\n").unwrap();
    fs::write(&bodyconv_path, b"0x00C0 -1 400 -1 500\n").unwrap();

    let body = BodyDef::load(&body_path).unwrap();
    let body_entry = body.resolve(0x00C0).unwrap();
    assert_eq!(body_entry.graphic, 1000);
    assert_eq!(body_entry.hue, 44);

    let bodyconv = BodyConvDef::load(&bodyconv_path).unwrap();
    let bodyconv_entry = bodyconv.resolve(0x00C0).unwrap();
    assert_eq!(bodyconv_entry.file_index, 4);
    assert_eq!(bodyconv_entry.graphic, 500);
    assert_eq!(bodyconv_entry.mount_height, 0);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn map_diff_overrides_base_map_block() {
    let dir = temp_dir("classic_map_diff");
    let map_path = dir.join("map0.mul");
    let lookup_path = dir.join("mapdifl0.mul");
    let diff_path = dir.join("mapdif0.mul");

    let mut map_bytes = Vec::new();
    map_bytes.extend_from_slice(&map_block(1, 2));
    map_bytes.extend_from_slice(&map_block(3, 4));
    fs::write(&map_path, map_bytes).unwrap();
    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    fs::write(&diff_path, map_block(55, -6)).unwrap();

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
fn static_diff_overrides_base_static_block() {
    let dir = temp_dir("classic_static_diff");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let lookup_path = dir.join("stadifl0.mul");
    let diff_idx_path = dir.join("stadifi0.mul");
    let diff_path = dir.join("stadif0.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, static_tile(100, 1, 2, 3, 4)).unwrap();

    fs::write(&lookup_path, 1u32.to_le_bytes()).unwrap();
    let mut diff_idx = fs::File::create(&diff_idx_path).unwrap();
    write_index_entry(&mut diff_idx, 0, 7, 0);
    fs::write(&diff_path, static_tile(200, 2, 3, -4, 5)).unwrap();

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

#[test]
fn verdata_static_patch_overrides_base_static_block() {
    let dir = temp_dir("classic_verdata");
    let idx_path = dir.join("staidx0.mul");
    let mul_path = dir.join("statics0.mul");
    let verdata_path = dir.join("verdata.mul");

    let mut idx = fs::File::create(&idx_path).unwrap();
    write_index_entry(&mut idx, u32::MAX, 0, u32::MAX);
    write_index_entry(&mut idx, 0, 7, 0);
    fs::write(&mul_path, static_tile(100, 1, 2, 3, 4)).unwrap();

    let patch = static_tile(300, 4, 5, 6, 7);
    let mut verdata_bytes = Vec::new();
    verdata_bytes.extend_from_slice(&1i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&(VerFileId::Statics as i32).to_le_bytes());
    verdata_bytes.extend_from_slice(&1i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&24i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&(patch.len() as i32).to_le_bytes());
    verdata_bytes.extend_from_slice(&0i32.to_le_bytes());
    verdata_bytes.extend_from_slice(&patch);
    fs::write(&verdata_path, verdata_bytes).unwrap();

    let verdata = Arc::new(Verdata::load(&verdata_path).unwrap());
    let reader = StaticsReader::new_with_patches(
        &idx_path,
        &mul_path,
        8,
        16,
        None,
        Some(verdata),
    )
    .unwrap();

    let tiles = reader.read_block(0, 1).unwrap();
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].graphic, 300);
    assert_eq!(tiles[0].hue, 7);

    let _ = fs::remove_dir_all(dir);
}
