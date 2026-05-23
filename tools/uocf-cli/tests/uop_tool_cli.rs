use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn uop_tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uop-tool"))
}

#[test]
fn hash_command_prints_expected_uop_hash() {
    let output = uop_tool()
        .args(["hash", "build/worldart/00000042.dds"])
        .output()
        .expect("run uop-tool hash");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("hash stdout is utf-8");
    let expected_hash = hash_file_name_single("build/worldart/00000042.dds");
    assert!(stdout.contains(&format!("0x{expected_hash:016x}")));
}

#[test]
fn crack_rejects_empty_charset() {
    let output = uop_tool()
        .args([
            "crack",
            "0x0000000000000000",
            "--charset",
            "",
            "--min-len",
            "1",
            "--max-len",
            "1",
        ])
        .output()
        .expect("run uop-tool crack");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Charset must not be empty"));
}

#[test]
fn crack_rejects_min_len_greater_than_max_len() {
    let output = uop_tool()
        .args([
            "crack",
            "0x0000000000000000",
            "--min-len",
            "2",
            "--max-len",
            "1",
        ])
        .output()
        .expect("run uop-tool crack");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Minimum length"));
}

#[test]
fn merge_dic_merges_input_dictionaries() {
    let temp = TempDir::new("merge-dic");
    let first_path = temp.path().join("first.dic");
    let second_path = temp.path().join("second.dic");
    let output_path = temp.path().join("merged.dic");

    let mut first = HashDictionary::new();
    first.set(0x10, "build/a.bin");
    first.insert_unknown(0x20);
    first.save(&first_path).expect("save first dictionary");

    let mut second = HashDictionary::new();
    second.set(0x20, "build/b.bin");
    second.insert_unknown(0x30);
    second.save(&second_path).expect("save second dictionary");

    let output = uop_tool()
        .arg("merge-dic")
        .arg("--output")
        .arg(&output_path)
        .arg(&first_path)
        .arg(&second_path)
        .output()
        .expect("run uop-tool merge-dic");

    assert!(output.status.success());
    let merged = HashDictionary::load(&output_path).expect("load merged dictionary");
    assert_eq!(merged.len(), 3);
    assert_eq!(merged.resolve(0x10), Some("build/a.bin"));
    assert_eq!(merged.resolve(0x20), Some("build/b.bin"));
    assert_eq!(merged.resolve(0x30), None);
}

#[test]
fn extract_writes_known_worldart_path() {
    let temp = TempDir::new("extract");
    let uop_path = temp.path().join("Texture.uop");
    let out_dir = temp.path().join("out");
    let payload = b"DDS test payload";
    let packed_name = "build/worldart/00000042.dds";

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(payload, packed_name, CompressionFlag::None)
        .expect("add uop fixture file");
    package.finalize_and_save(&uop_path).expect("save uop fixture");

    let output = uop_tool()
        .arg("extract")
        .arg(&uop_path)
        .arg(&out_dir)
        .output()
        .expect("run uop-tool extract");

    assert!(output.status.success());
    assert_eq!(
        fs::read(out_dir.join("build").join("worldart").join("00000042.dds"))
            .expect("read extracted payload"),
        payload
    );
}

#[test]
fn extract_rejects_malformed_uop() {
    let temp = TempDir::new("extract-malformed");
    let uop_path = temp.path().join("bad.uop");
    let out_dir = temp.path().join("out");
    fs::write(&uop_path, b"not a uop").expect("write malformed uop");

    let output = uop_tool()
        .arg("extract")
        .arg(&uop_path)
        .arg(&out_dir)
        .output()
        .expect("run uop-tool extract");

    assert!(!output.status.success());
}

#[test]
fn extract_rejects_bad_dictionary_argument() {
    let temp = TempDir::new("extract-bad-dictionary");
    let uop_path = temp.path().join("Texture.uop");
    let out_dir = temp.path().join("out");
    let dictionary_path = temp.path().join("bad-dictionary.uop");

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            b"DDS test payload",
            "build/worldart/00000042.dds",
            CompressionFlag::None,
        )
        .expect("add uop fixture file");
    package.finalize_and_save(&uop_path).expect("save uop fixture");
    fs::write(&dictionary_path, b"not a dictionary").expect("write bad dictionary");

    let output = uop_tool()
        .arg("extract")
        .arg(&uop_path)
        .arg(&out_dir)
        .arg("--dictionary")
        .arg(&dictionary_path)
        .output()
        .expect("run uop-tool extract");

    assert!(!output.status.success());
}

#[test]
fn replace_updates_payload_without_fixed_temp_file() {
    let temp = TempDir::new("replace");
    let uop_path = temp.path().join("package.uop");
    let replacement_path = temp.path().join("replacement.bin");
    let packed_name = "build/worldart/00000042.dds";
    let hash = hash_file_name_single(packed_name);

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(b"old payload", packed_name, CompressionFlag::None)
        .expect("add uop fixture file");
    package.finalize_and_save(&uop_path).expect("save uop fixture");
    fs::write(&replacement_path, b"new payload").expect("write replacement");

    let output = uop_tool()
        .arg("replace")
        .arg(&uop_path)
        .arg(format!("0x{hash:016x}"))
        .arg(&replacement_path)
        .output()
        .expect("run uop-tool replace");

    assert!(output.status.success());
    assert!(!uop_path.with_extension("uop.temp").exists());
    let loaded = UopPackage::load(&uop_path).expect("load replaced package");
    let file = loaded.get_file_by_hash(hash).expect("find replaced file");
    assert_eq!(file.unpack().expect("unpack replaced file"), b"new payload");
}

#[test]
fn rebuild_rewrites_package_without_fixed_temp_file() {
    let temp = TempDir::new("rebuild");
    let uop_path = temp.path().join("package.uop");
    let packed_name = "build/worldart/00000042.dds";
    let hash = hash_file_name_single(packed_name);

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(b"payload", packed_name, CompressionFlag::None)
        .expect("add uop fixture file");
    package.finalize_and_save(&uop_path).expect("save uop fixture");

    let output = uop_tool()
        .arg("rebuild")
        .arg(&uop_path)
        .output()
        .expect("run uop-tool rebuild");

    assert!(output.status.success());
    assert!(!uop_path.with_extension("uop.temp").exists());
    let loaded = UopPackage::load(&uop_path).expect("load rebuilt package");
    let file = loaded.get_file_by_hash(hash).expect("find rebuilt file");
    assert_eq!(file.unpack().expect("unpack rebuilt file"), b"payload");
}
