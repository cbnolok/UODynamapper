#![cfg(feature = "dev-tool-tests")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn cc_uop_mul_converter() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cc-uop-mul-converter"))
}

fn facet_evidence_tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_facet-evidence-tool"))
}

#[test]
fn cc_converter_rejects_missing_directory() {
    let temp = TempDir::new("cc-converter-missing-dir");
    let missing = temp.path().join("missing");

    let output = cc_uop_mul_converter()
        .arg("extract")
        .arg(&missing)
        .output()
        .expect("run cc-uop-mul-converter extract");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"));
}

#[test]
fn cc_converter_extract_empty_directory_counts_required_packages_only() {
    let temp = TempDir::new("cc-converter-empty-extract");

    let output = cc_uop_mul_converter()
        .arg("extract")
        .arg(temp.path())
        .output()
        .expect("run cc-uop-mul-converter extract");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Errors: 4"));
}

#[test]
fn cc_converter_pack_empty_directory_fails_attempted_actions() {
    let temp = TempDir::new("cc-converter-empty-pack");

    let output = cc_uop_mul_converter()
        .arg("pack")
        .arg(temp.path())
        .output()
        .expect("run cc-uop-mul-converter pack");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Errors: 16"));
}

#[test]
fn facet_evidence_tool_rejects_missing_evidence_sources() {
    let temp = TempDir::new("facet-evidence-no-source");
    let output_path = temp.path().join("evidence.kdl");

    let output = facet_evidence_tool()
        .arg("--client")
        .arg("ec")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run facet-evidence-tool");

    assert!(!output.status.success());
    assert!(!output_path.exists());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no evidence source selected"));
}
