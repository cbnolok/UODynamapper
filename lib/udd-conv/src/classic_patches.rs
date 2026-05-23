use std::path::PathBuf;
use std::sync::Arc;

use log::{info, warn};
use uocf::classic::map_statics_diff::{MapDiff, StaticDiff};
use uocf::classic::verdata::Verdata;

use crate::source_paths::find_first_existing_file;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClassicPatchOptions {
    pub verdata: bool,
    pub map_difs: bool,
    pub static_difs: bool,
}

impl ClassicPatchOptions {
    pub const NONE: Self = Self {
        verdata: false,
        map_difs: false,
        static_difs: false,
    };

    pub fn any(self) -> bool {
        self.verdata || self.map_difs || self.static_difs
    }
}

pub(crate) fn load_verdata_if_enabled(
    source_dirs: &[PathBuf],
    options: &ClassicPatchOptions,
) -> color_eyre::eyre::Result<Option<Arc<Verdata>>> {
    if !options.verdata {
        return Ok(None);
    }

    let Some(path) = find_first_existing_file(source_dirs, &["verdata.mul", "Verdata.mul"]) else {
        warn!("Classic patch option verdata enabled, but verdata.mul was not found.");
        return Ok(None);
    };

    info!("Using verdata patch file: {}", path.display());
    Ok(Some(Arc::new(Verdata::load(path)?)))
}

pub(crate) fn load_map_diff_if_enabled(
    source_dirs: &[PathBuf],
    map_id: u32,
    options: &ClassicPatchOptions,
) -> color_eyre::eyre::Result<Option<MapDiff>> {
    if !options.map_difs {
        return Ok(None);
    }

    let lookup_names = [
        format!("mapdifl{}.mul", map_id),
        format!("Mapdifl{}.mul", map_id),
    ];
    let diff_names = [
        format!("mapdif{}.mul", map_id),
        format!("Mapdif{}.mul", map_id),
    ];

    let Some(lookup_path) = find_first_existing_file(source_dirs, &as_strs(&lookup_names)) else {
        warn!(
            "Classic patch option map_difs enabled, but mapdifl{}.mul was not found.",
            map_id
        );
        return Ok(None);
    };
    let Some(diff_path) = find_first_existing_file(source_dirs, &as_strs(&diff_names)) else {
        warn!(
            "Classic patch option map_difs enabled, but mapdif{}.mul was not found.",
            map_id
        );
        return Ok(None);
    };

    info!(
        "Using map diff patch files: {}, {}",
        lookup_path.display(),
        diff_path.display()
    );
    Ok(Some(MapDiff::load(lookup_path, diff_path)?))
}

pub(crate) fn load_static_diff_if_enabled(
    source_dirs: &[PathBuf],
    map_id: u32,
    options: &ClassicPatchOptions,
) -> color_eyre::eyre::Result<Option<StaticDiff>> {
    if !options.static_difs {
        return Ok(None);
    }

    let lookup_names = [
        format!("stadifl{}.mul", map_id),
        format!("Stadifl{}.mul", map_id),
    ];
    let index_names = [
        format!("stadifi{}.mul", map_id),
        format!("Stadifi{}.mul", map_id),
    ];
    let diff_names = [
        format!("stadif{}.mul", map_id),
        format!("Stadif{}.mul", map_id),
    ];

    let Some(lookup_path) = find_first_existing_file(source_dirs, &as_strs(&lookup_names)) else {
        warn!(
            "Classic patch option static_difs enabled, but stadifl{}.mul was not found.",
            map_id
        );
        return Ok(None);
    };
    let Some(index_path) = find_first_existing_file(source_dirs, &as_strs(&index_names)) else {
        warn!(
            "Classic patch option static_difs enabled, but stadifi{}.mul was not found.",
            map_id
        );
        return Ok(None);
    };
    let Some(diff_path) = find_first_existing_file(source_dirs, &as_strs(&diff_names)) else {
        warn!(
            "Classic patch option static_difs enabled, but stadif{}.mul was not found.",
            map_id
        );
        return Ok(None);
    };

    info!(
        "Using static diff patch files: {}, {}, {}",
        lookup_path.display(),
        index_path.display(),
        diff_path.display()
    );
    Ok(Some(StaticDiff::load(lookup_path, index_path, diff_path)?))
}

fn as_strs(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(test_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("uddconv_patch_options_{test_name}_{timestamp}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_verdata(path: PathBuf) {
        fs::write(path, 0i32.to_le_bytes()).expect("write verdata");
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

    fn static_tile(graphic: u16, x: u8, y: u8, z: i8, hue: u16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(7);
        bytes.extend_from_slice(&graphic.to_le_bytes());
        bytes.push(x);
        bytes.push(y);
        bytes.push(z as u8);
        bytes.extend_from_slice(&hue.to_le_bytes());
        bytes
    }

    fn static_index_entry(lookup: u32, size: u32, extra: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&lookup.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&extra.to_le_bytes());
        bytes
    }

    #[test]
    fn classic_patch_options_none_disables_all_sources() {
        let dir = temp_dir("none");
        fs::write(dir.join("verdata.mul"), b"invalid").expect("write invalid verdata");
        fs::write(dir.join("mapdifl0.mul"), b"invalid").expect("write invalid map lookup");
        fs::write(dir.join("mapdif0.mul"), b"invalid").expect("write invalid map diff");
        fs::write(dir.join("stadifl0.mul"), b"invalid").expect("write invalid static lookup");
        fs::write(dir.join("stadifi0.mul"), b"invalid").expect("write invalid static index");
        fs::write(dir.join("stadif0.mul"), b"invalid").expect("write invalid static diff");

        assert!(!ClassicPatchOptions::NONE.any());
        assert!(load_verdata_if_enabled(std::slice::from_ref(&dir), &ClassicPatchOptions::NONE)
            .expect("disabled verdata")
            .is_none());
        assert!(load_map_diff_if_enabled(std::slice::from_ref(&dir), 0, &ClassicPatchOptions::NONE)
            .expect("disabled map dif")
            .is_none());
        assert!(load_static_diff_if_enabled(std::slice::from_ref(&dir), 0, &ClassicPatchOptions::NONE)
            .expect("disabled static dif")
            .is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn enabled_missing_patch_files_return_none() {
        let dir = temp_dir("missing");
        let options = ClassicPatchOptions {
            verdata: true,
            map_difs: true,
            static_difs: true,
        };

        assert!(options.any());
        assert!(load_verdata_if_enabled(std::slice::from_ref(&dir), &options)
            .expect("missing verdata")
            .is_none());
        assert!(load_map_diff_if_enabled(std::slice::from_ref(&dir), 0, &options)
            .expect("missing map dif")
            .is_none());
        assert!(load_static_diff_if_enabled(std::slice::from_ref(&dir), 0, &options)
            .expect("missing static dif")
            .is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn enabled_verdata_loads_case_insensitive_filename_variant() {
        let dir = temp_dir("verdata_case");
        write_verdata(dir.join("Verdata.mul"));

        let loaded = load_verdata_if_enabled(
            std::slice::from_ref(&dir),
            &ClassicPatchOptions {
                verdata: true,
                map_difs: false,
                static_difs: false,
            },
        )
        .expect("load verdata");

        assert!(loaded.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn enabled_map_diff_loads_matching_pair() {
        let dir = temp_dir("map_diff");
        fs::write(dir.join("mapdifl0.mul"), 1u32.to_le_bytes()).expect("write map lookup");
        fs::write(dir.join("mapdif0.mul"), map_block(44, -2)).expect("write map diff");

        let loaded = load_map_diff_if_enabled(
            std::slice::from_ref(&dir),
            0,
            &ClassicPatchOptions {
                verdata: false,
                map_difs: true,
                static_difs: false,
            },
        )
        .expect("load map diff");

        assert!(loaded.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn enabled_static_diff_loads_lookup_index_and_payload() {
        let dir = temp_dir("static_diff");
        fs::write(dir.join("stadifl0.mul"), 1u32.to_le_bytes()).expect("write static lookup");
        fs::write(dir.join("stadifi0.mul"), static_index_entry(0, 7, 0)).expect("write static index");
        fs::write(dir.join("stadif0.mul"), static_tile(200, 1, 2, -3, 4)).expect("write static diff");

        let loaded = load_static_diff_if_enabled(
            std::slice::from_ref(&dir),
            0,
            &ClassicPatchOptions {
                verdata: false,
                map_difs: false,
                static_difs: true,
            },
        )
        .expect("load static diff");

        assert!(loaded.is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
