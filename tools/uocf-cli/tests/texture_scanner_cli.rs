use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ddsfile::{D3DFormat, Dds, NewD3dParams};

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

fn texture_scanner() -> Command {
    Command::new(env!("CARGO_BIN_EXE_texture-scanner"))
}

#[test]
fn texture_scanner_rejects_missing_input_directory() {
    let temp = TempDir::new("texture-scanner-missing-input");
    let output_path = temp.path().join("textures.csv");

    let output = texture_scanner()
        .arg(temp.path().join("missing"))
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run texture-scanner");

    assert!(!output.status.success());
    assert!(!output_path.exists());
}

#[test]
fn texture_scanner_writes_header_for_empty_directory() {
    let temp = TempDir::new("texture-scanner-empty");
    let input_dir = temp.path().join("input");
    let output_path = temp.path().join("textures.csv");
    fs::create_dir_all(&input_dir).expect("create input dir");

    let output = texture_scanner()
        .arg(&input_dir)
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run texture-scanner");

    assert!(output.status.success());
    let csv = fs::read_to_string(output_path).expect("read output csv");
    assert_eq!(
        csv,
        "path,vpath_hash,width,height,is_square,is_opaque,is_land_candidate,unused1,size\n"
    );
}

#[test]
fn texture_scanner_inventories_tiny_dds() {
    let temp = TempDir::new("texture-scanner-dds");
    let input_dir = temp.path().join("input");
    let output_path = temp.path().join("textures.csv");
    fs::create_dir_all(&input_dir).expect("create input dir");
    write_tiny_dds(&input_dir.join("sample.dds"));

    let output = texture_scanner()
        .arg(&input_dir)
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("run texture-scanner");

    assert!(output.status.success());
    let csv = fs::read_to_string(output_path).expect("read output csv");
    assert!(csv.contains("sample.dds"));
    assert!(csv.contains(",2,1,0,1,0,0,"));
}

fn write_tiny_dds(path: &Path) {
    let params = NewD3dParams {
        height: 1,
        width: 2,
        depth: None,
        format: D3DFormat::A8R8G8B8,
        mipmap_levels: None,
        caps2: None,
    };
    let mut dds = Dds::new_d3d(params).expect("create dds");
    dds.data = vec![
        255, 0, 0, 255,
        0, 255, 0, 255,
    ];
    let mut file = fs::File::create(path).expect("create dds file");
    dds.write(&mut file).expect("write dds");
}
