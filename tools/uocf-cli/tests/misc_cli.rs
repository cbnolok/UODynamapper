use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use uocf::classic::multimap_rle::{self, BLACK_PIXEL, MultimapRleImage, WHITE_PIXEL};
use uocf::classic::sound::{SOUND_NAME_BYTES, WAV_HEADER_BYTES};
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::hash_dictionary::HashDictionary;
use uocf::uop_container::package::UopPackage;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "uocf-cli-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn multimap_tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_multimap-tool"))
}

fn sound_tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sound-tool"))
}

fn uop_dict_populator() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uop-dict-populator-cli"))
}

#[test]
fn multimap_tool_roundtrips_rle_through_png() {
    let temp = TempDir::new("multimap-cli");
    let rle_path = temp.path().join("multimap.rle");
    let png_path = temp.path().join("multimap.png");
    let roundtrip_path = temp.path().join("roundtrip.rle");
    let image = MultimapRleImage::new(
        3,
        2,
        vec![
            WHITE_PIXEL,
            BLACK_PIXEL,
            WHITE_PIXEL,
            BLACK_PIXEL,
            BLACK_PIXEL,
            WHITE_PIXEL,
        ],
    )
    .expect("build multimap image");
    multimap_rle::save_rle(&rle_path, &image).expect("write source rle");

    let decode = multimap_tool()
        .arg("rle-to-image")
        .arg("--input")
        .arg(&rle_path)
        .arg("--output")
        .arg(&png_path)
        .output()
        .expect("run multimap rle-to-image");
    assert!(decode.status.success());

    let encode = multimap_tool()
        .arg("image-to-rle")
        .arg("--input")
        .arg(&png_path)
        .arg("--output")
        .arg(&roundtrip_path)
        .output()
        .expect("run multimap image-to-rle");
    assert!(encode.status.success());

    let roundtrip = multimap_rle::load_rle(&roundtrip_path).expect("read roundtrip rle");
    assert_eq!(roundtrip, image);
}

#[test]
fn sound_tool_exports_slot_to_wav() {
    let temp = TempDir::new("sound-cli");
    let ccdir = temp.path();
    let mut sound_payload = vec![0u8; SOUND_NAME_BYTES];
    sound_payload[..5].copy_from_slice(b"clang");
    sound_payload.extend_from_slice(&[1u8, 2, 3, 4]);
    fs::write(ccdir.join("sound.mul"), &sound_payload).expect("write sound.mul");
    {
        let mut idx = fs::File::create(ccdir.join("soundidx.mul")).expect("create soundidx.mul");
        idx.write_all(&0u32.to_le_bytes()).expect("write lookup");
        idx.write_all(&(sound_payload.len() as u32).to_le_bytes())
            .expect("write length");
        idx.write_all(&1u32.to_le_bytes()).expect("write extra");
    }

    let output_path = ccdir.join("slot0.wav");
    let output = sound_tool()
        .arg("export")
        .arg("--ccdir")
        .arg(ccdir)
        .arg("--slot")
        .arg("0")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run sound-tool export");

    assert!(output.status.success());
    let wav = fs::read(output_path).expect("read wav");
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(&wav[WAV_HEADER_BYTES..], &[1u8, 2, 3, 4]);
}

#[test]
fn uop_dict_populator_finds_template_match() {
    let temp = TempDir::new("dict-populator-cli");
    let uop_dir = temp.path().join("uops");
    fs::create_dir_all(&uop_dir).expect("create uop dir");
    let config_path = temp.path().join("config.toml");
    let output_path = temp.path().join("Dictionary.dic");
    let packed_name = "build/worldart/00000042.dds";
    let hash = hash_file_name_single(packed_name);

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(b"DDS test payload", packed_name, CompressionFlag::None)
        .expect("add uop fixture file");
    package
        .finalize_and_save(uop_dir.join("Texture.uop"))
        .expect("save uop fixture");
    fs::write(
        &config_path,
        "[\"Texture.uop\"]\ncandidates = [\"build/worldart/{:08}.dds\"]\nrange = [42, 42]\n",
    )
    .expect("write config");

    let output = uop_dict_populator()
        .arg("--config")
        .arg(&config_path)
        .arg("--uop-dir")
        .arg(&uop_dir)
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uop-dict-populator-cli");

    assert!(output.status.success());
    let dictionary = HashDictionary::load(&output_path).expect("load output dictionary");
    assert_eq!(dictionary.resolve(hash), Some(packed_name));
}
