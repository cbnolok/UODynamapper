//! Early Classic network patching works without pretending that a facet table was found.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[path = "../../../lib/uocf/tests/common/classic_cipher.rs"]
mod classic_cipher;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
}

fn text(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn early_classic_keys_and_shared_patch_work_without_facet_capabilities() {
    let dir = TempDir(std::env::temp_dir().join(format!("uocf-early-classic-cli-{}", std::process::id())));
    fs::create_dir_all(&dir.0).unwrap();
    let source = dir.0.join("client.exe");
    let destination = dir.0.join("client-plaintext.exe");
    let original = classic_cipher::image();
    fs::write(&source, &original).unwrap();

    let show = command().args(["exe", "show"]).arg(&source).arg("--crypt-keys").output().unwrap();
    let shown = text(&show);
    assert!(show.status.success(), "{shown}");
    for expected in ["family    Classic", "facet editing and data checks are unsupported", "shared login/game master keys", "raw four-byte connection seed", "no 0xEF protocol version is sent"] {
        assert!(shown.contains(expected), "missing {expected}: {shown}");
    }
    assert!(!shown.contains("wire version not located"), "an absent 0xEF writer is expected: {shown}");
    let original_keys = shown.lines().find(|line| line.starts_with("cryptkeys ")).unwrap();

    let patch = command().args(["exe", "patch"]).arg(&source).arg("--output").arg(&destination)
        .arg("--no-encryption").output().unwrap();
    assert!(patch.status.success(), "{}", text(&patch));
    assert_eq!(fs::read(&source).unwrap(), original, "the original stays intact beside the output");
    let patched = command().args(["exe", "show"]).arg(&destination).arg("--crypt-keys").output().unwrap();
    let patched_text = text(&patched);
    assert!(patched.status.success(), "{patched_text}");
    assert!(patched_text.contains("shared cipher patched out"), "{patched_text}");
    assert_eq!(patched_text.lines().find(|line| line.starts_with("cryptkeys ")).unwrap(), original_keys);

    for flag in ["--no-login-encryption", "--no-game-encryption"] {
        let scoped = command().args(["exe", "patch"]).arg(&source).args(["--dry-run", flag]).output().unwrap();
        assert!(!scoped.status.success(), "a shared routine cannot be changed for one phase");
        assert!(text(&scoped).contains("--no-encryption"), "{}", text(&scoped));
    }
    let size = command().args(["exe", "patch"]).arg(&source).args(["--dry-run", "--size", "0=8192x4096"]).output().unwrap();
    assert!(!size.status.success(), "facet editing is unavailable");
    let data = command().args(["exe", "check-data"]).arg(&source).arg(&dir.0).output().unwrap();
    assert!(!data.status.success(), "an empty data check cannot claim success");
    assert!(text(&data).contains("no supported facet table"), "{}", text(&data));
}
