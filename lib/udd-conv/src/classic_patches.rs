use std::path::PathBuf;
use std::sync::Arc;

use log::{info, warn};
use uocf::classic::facet_sources::ClassicDifOptions;
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

    /// The dif files to apply to a Classic facet (`uocf::classic::facet_sources`).
    pub fn difs(self) -> ClassicDifOptions {
        ClassicDifOptions {
            map_difs: self.map_difs,
            static_difs: self.static_difs,
        }
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

    #[test]
    fn classic_patch_options_none_disables_all_sources() {
        let dir = temp_dir("none");
        fs::write(dir.join("verdata.mul"), b"invalid").expect("write invalid verdata");

        assert!(!ClassicPatchOptions::NONE.any());
        assert_eq!(ClassicPatchOptions::NONE.difs(), ClassicDifOptions::NONE);
        assert!(load_verdata_if_enabled(std::slice::from_ref(&dir), &ClassicPatchOptions::NONE)
            .expect("disabled verdata")
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
        assert_eq!(options.difs(), ClassicDifOptions::ALL);
        assert!(load_verdata_if_enabled(std::slice::from_ref(&dir), &options)
            .expect("missing verdata")
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
}
