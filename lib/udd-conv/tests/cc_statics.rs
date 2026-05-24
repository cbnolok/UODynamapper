use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use byteorder::{LittleEndian, WriteBytesExt};
use udd_container::UddpReader;
use udd_conv::cc_statics::convert_statics_mul_to_uddp_from_sources;
use uocf::uop_container::{file::CompressionFlag, package::UopPackage};

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("uddconv_{test_name}_{timestamp}"))
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

#[test]
fn statics_pack_uses_legacy_map_uop_for_dimensions() {
    let dir = temp_dir("statics_map_uop_dimensions");
    fs::create_dir_all(&dir).expect("create temp dir");

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            &map_block(1, 0),
            "build/map2legacymul/00000000.dat",
            CompressionFlag::None,
        )
        .expect("add map chunk");
    package
        .finalize_and_save(dir.join("map2LegacyMUL.uop"))
        .expect("save map uop");

    let mut staidx = Vec::new();
    staidx.write_i32::<LittleEndian>(0).unwrap();
    staidx.write_i32::<LittleEndian>(7).unwrap();
    staidx.write_i32::<LittleEndian>(0).unwrap();
    for _ in 1..(288 * 200) {
        staidx.write_i32::<LittleEndian>(-1).unwrap();
        staidx.write_i32::<LittleEndian>(-1).unwrap();
        staidx.write_i32::<LittleEndian>(-1).unwrap();
    }
    fs::write(dir.join("staidx2.mul"), staidx).expect("write staidx");

    let mut statics = Vec::new();
    statics.write_u16::<LittleEndian>(0x4001).unwrap();
    statics.push(1);
    statics.push(2);
    statics.push(3);
    statics.write_u16::<LittleEndian>(0x0044).unwrap();
    fs::write(dir.join("statics2.mul"), statics).expect("write statics");

    let output = dir.join("statics2.uddp");
    let summary =
        convert_statics_mul_to_uddp_from_sources(std::slice::from_ref(&dir), &output, 2)
            .expect("pack statics");

    assert_eq!(summary.map_id, 2);
    assert_eq!(summary.total_statics, 1);

    let package = UddpReader::open(fs::read(output).expect("read statics package"))
        .expect("open statics package");
    assert_eq!(package.read_file_by_dense_id(0).unwrap().len(), 8);

    let _ = fs::remove_dir_all(dir);
}
