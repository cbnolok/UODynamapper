use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use uocf::classic::michelangelo_uop_codec::{MichelangeloPatch, MichelangeloPatchEntry};
use uocf::classic::vd_codec::{VdFile, VERDATA_FILE_ID_ANIM};
use uocf::uop_container::file::CompressionFlag;
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
            "uocf-asset-cli-{name}-{}-{unique}",
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

fn asset_cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uocf-asset-cli"))
}

#[test]
fn export_anim_patch_writes_single_vd_entry_from_classic_mul() {
    let temp = TempDir::new("export-anim-vd");
    let (idx_path, mul_path) = write_anim_pair(temp.path(), &[(7, 77, b"anim payload".to_vec())]);
    let output_path = temp.path().join("body_7.vd");

    let output = asset_cli()
        .arg("export-anim-patch")
        .arg("--source")
        .arg("classic-mul")
        .arg("--idx")
        .arg(&idx_path)
        .arg("--mul")
        .arg(&mul_path)
        .arg("--block")
        .arg("7")
        .arg("--format")
        .arg("vd")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uocf-asset-cli export-anim-patch");

    assert!(output.status.success());
    let vd = VdFile::load(&output_path).expect("load exported vd");
    assert_eq!(vd.entry.file_id, VERDATA_FILE_ID_ANIM);
    assert_eq!(vd.entry.index, 7);
    assert_eq!(vd.entry.extra, 77);
    assert_eq!(vd.data, b"anim payload");
}

#[test]
fn export_anim_patch_writes_remapped_michelangelo_uop_from_classic_mul() {
    let temp = TempDir::new("export-anim-uop");
    let (idx_path, mul_path) = write_anim_pair(
        temp.path(),
        &[
            (1, 101, vec![2, 12]),
            (2, 102, vec![3, 13]),
        ],
    );
    let output_path = temp.path().join("patch.uop");

    let output = asset_cli()
        .arg("export-anim-patch")
        .arg("--source")
        .arg("classic-mul")
        .arg("--idx")
        .arg(&idx_path)
        .arg("--mul")
        .arg(&mul_path)
        .arg("--block")
        .arg("1")
        .arg("--block")
        .arg("2")
        .arg("--source-anim")
        .arg("0")
        .arg("--target-anim")
        .arg("200")
        .arg("--format")
        .arg("michelangelo-uop")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uocf-asset-cli export-anim-patch");

    assert!(output.status.success());
    let patch = MichelangeloPatch::load(&output_path).expect("load exported uop patch");
    assert_eq!(
        patch.entries,
        vec![
            MichelangeloPatchEntry::anim(22_001, 101, vec![2, 12]),
            MichelangeloPatchEntry::anim(22_002, 102, vec![3, 13]),
        ]
    );
}

#[test]
fn export_anim_patch_rejects_multi_block_vd() {
    let temp = TempDir::new("export-anim-vd-multi");
    let (idx_path, mul_path) = write_anim_pair(
        temp.path(),
        &[
            (1, 101, vec![2, 12]),
            (2, 102, vec![3, 13]),
        ],
    );
    let output_path = temp.path().join("bad.vd");

    let output = asset_cli()
        .arg("export-anim-patch")
        .arg("--source")
        .arg("classic-mul")
        .arg("--idx")
        .arg(&idx_path)
        .arg("--mul")
        .arg(&mul_path)
        .arg("--block")
        .arg("1")
        .arg("--block")
        .arg("2")
        .arg("--format")
        .arg("vd")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uocf-asset-cli export-anim-patch");

    assert!(!output.status.success());
    assert!(!output_path.exists());
}

#[test]
fn export_anim_patch_wraps_cc_animationframe_entry() {
    let temp = TempDir::new("export-cc-animationframe");
    let uop_path = temp.path().join("AnimationFrame1.uop");
    let output_path = temp.path().join("cc.vd");
    let payload = b"cc animationframe payload";
    write_uop_entry(
        &uop_path,
        "build/animationlegacyframe/000123/04.bin",
        payload,
    );

    let output = asset_cli()
        .arg("export-anim-patch")
        .arg("--source")
        .arg("cc-animation-frame")
        .arg("--uop")
        .arg(&uop_path)
        .arg("--body")
        .arg("123")
        .arg("--group")
        .arg("4")
        .arg("--format")
        .arg("vd")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uocf-asset-cli export-anim-patch");

    assert!(output.status.success());
    let vd = VdFile::load(&output_path).expect("load exported vd");
    assert_eq!(vd.entry.index, 123);
    assert_eq!(vd.entry.extra, 0);
    assert_eq!(vd.data, payload);
}

#[test]
fn export_anim_patch_wraps_ec_animationframe_entry() {
    let temp = TempDir::new("export-ec-animationframe");
    let uop_path = temp.path().join("AnimationFrame1.uop");
    let output_path = temp.path().join("ec.uop");
    let payload = b"ec animationframe payload";
    write_uop_entry(&uop_path, "data/animationframe/000321.bin", payload);

    let output = asset_cli()
        .arg("export-anim-patch")
        .arg("--source")
        .arg("ec-animation-frame")
        .arg("--uop")
        .arg(&uop_path)
        .arg("--body")
        .arg("321")
        .arg("--target-index")
        .arg("777")
        .arg("--extra")
        .arg("9")
        .arg("--format")
        .arg("michelangelo-uop")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run uocf-asset-cli export-anim-patch");

    assert!(output.status.success());
    let patch = MichelangeloPatch::load(&output_path).expect("load exported uop patch");
    assert_eq!(
        patch.entries,
        vec![MichelangeloPatchEntry::anim(777, 9, payload.to_vec())]
    );
}

fn write_anim_pair(dir: &Path, entries: &[(usize, u32, Vec<u8>)]) -> (PathBuf, PathBuf) {
    let idx_path = dir.join("anim.idx");
    let mul_path = dir.join("anim.mul");
    let count = entries.iter().map(|(index, _, _)| *index).max().unwrap_or(0) + 1;
    let mut index_entries = vec![(u32::MAX, 0u32, u32::MAX); count];
    let mut mul = Vec::new();

    for &(index, extra, ref data) in entries {
        let lookup = mul.len() as u32;
        mul.extend_from_slice(data);
        index_entries[index] = (lookup, data.len() as u32, extra);
    }

    let mut idx = Vec::new();
    for (lookup, size, extra) in index_entries {
        idx.extend_from_slice(&lookup.to_le_bytes());
        idx.extend_from_slice(&size.to_le_bytes());
        idx.extend_from_slice(&extra.to_le_bytes());
    }

    fs::write(&idx_path, idx).expect("write anim idx");
    fs::write(&mul_path, mul).expect("write anim mul");
    (idx_path, mul_path)
}

fn write_uop_entry(path: &Path, internal_path: &str, payload: &[u8]) {
    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(payload, internal_path, CompressionFlag::None)
        .expect("add uop fixture file");
    package.finalize_and_save(path).expect("save uop fixture");
}
