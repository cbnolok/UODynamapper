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
