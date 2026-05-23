use std::fs;

use uocf::classic::bodyconv_def::BodyConvDef;

mod common;

#[test]
fn bodyconv_def_picks_highest_available_animation_file() {
    let dir = common::temp_dir("classic_bodyconv_def");
    let bodyconv_path = dir.join("Bodyconv.def");

    fs::write(&bodyconv_path, b"0x00C0 -1 400 -1 500\n").unwrap();

    let bodyconv = BodyConvDef::load(&bodyconv_path).unwrap();
    let entry = bodyconv.resolve(0x00C0).unwrap();
    assert_eq!(entry.file_index, 4);
    assert_eq!(entry.graphic, 500);
    assert_eq!(entry.mount_height, 0);

    let _ = fs::remove_dir_all(dir);
}
