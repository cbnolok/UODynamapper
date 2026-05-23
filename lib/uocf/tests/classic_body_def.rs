use std::fs;

use uocf::classic::body_def::BodyDef;

mod common;

#[test]
fn body_def_parses_redirect_with_hue_and_comments() {
    let dir = common::temp_dir("classic_body_def");
    let body_path = dir.join("Body.def");

    fs::write(&body_path, b"0x00C0 { 1 2 0x03E8 } 44 # comment\n").unwrap();

    let body = BodyDef::load(&body_path).unwrap();
    let entry = body.resolve(0x00C0).unwrap();
    assert_eq!(entry.graphic, 1000);
    assert_eq!(entry.hue, 44);

    let _ = fs::remove_dir_all(dir);
}
