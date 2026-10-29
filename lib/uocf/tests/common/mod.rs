#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use uocf::classic::verdata::VerFileId;

pub fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("uocf_{test_name}_{timestamp}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn map_block(tile_id: u16, z: i8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(196);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..64 {
        bytes.extend_from_slice(&tile_id.to_le_bytes());
        bytes.push(z as u8);
    }
    bytes
}

pub fn static_tile(graphic: u16, x: u8, y: u8, z: i8, hue: u16) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(7);
    bytes.extend_from_slice(&graphic.to_le_bytes());
    bytes.push(x);
    bytes.push(y);
    bytes.push(z as u8);
    bytes.extend_from_slice(&hue.to_le_bytes());
    bytes
}

pub fn write_index_entry(file: &mut fs::File, lookup: u32, size: u32, extra: u32) {
    file.write_all(&lookup.to_le_bytes()).unwrap();
    file.write_all(&size.to_le_bytes()).unwrap();
    file.write_all(&extra.to_le_bytes()).unwrap();
}

pub fn write_verdata(path: &PathBuf, entries: &[(VerFileId, i32, i32, Vec<u8>)]) {
    let header_len = 4 + entries.len() * 20;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(entries.len() as i32).to_le_bytes());

    let mut lookup = header_len as i32;
    for (file_id, index, extra, payload) in entries {
        bytes.extend_from_slice(&(*file_id as i32).to_le_bytes());
        bytes.extend_from_slice(&index.to_le_bytes());
        bytes.extend_from_slice(&lookup.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&extra.to_le_bytes());
        lookup += payload.len() as i32;
    }

    for (_file_id, _index, _extra, payload) in entries {
        bytes.extend_from_slice(payload);
    }

    fs::write(path, bytes).unwrap();
}

/// Environment variables naming real client directories for the ignored client-data tests: an Enhanced Client
/// and a Classic Client directory. Each is optional; at least one must be set.
pub const CLIENT_DIR_ENVS: [&str; 2] = ["UO_EC_CLIENT_DIR", "UO_CC_CLIENT_DIR"];

/// The path of `file_name` in each client directory named by [`CLIENT_DIR_ENVS`], with the variable naming the
/// directory. Directories without the file are reported and skipped.
pub fn client_files(file_name: &str) -> Vec<(&'static str, PathBuf)> {
    let dirs: Vec<_> = CLIENT_DIR_ENVS
        .into_iter()
        .filter_map(|env| std::env::var_os(env).map(|dir| (env, PathBuf::from(dir))))
        .collect();
    assert!(!dirs.is_empty(), "set {} to a client directory", CLIENT_DIR_ENVS.join(" and/or "));
    dirs.into_iter()
        .filter_map(|(env, dir)| {
            let path = dir.join(file_name);
            if path.is_file() {
                Some((env, path))
            } else {
                println!("{env}: no {file_name}, skipped");
                None
            }
        })
        .collect()
}
