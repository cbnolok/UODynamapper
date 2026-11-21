//! What a repack of a mobile animation package keeps of the strides measured on the package it replaces
//! (`udd_assets::animation::mobile_strides`): those whose clip's frames still hash the same. The others, and the new
//! clips, get none until measured again (Dynamapper's `measure-strides changed`).

use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use log::{info, warn};
use udd_assets::animation::mobile_strides::{
    read_package_strides, strides_still_matching, write_package_strides, MobileAnimClips, MobileStrideRecord,
};

/// The strides of the package at `path` (none without one, or when it does not read, which is logged): read before a
/// repack replaces it.
pub fn strides_before_repack(path: &Path) -> Vec<MobileStrideRecord> {
    if !path.is_file() {
        return Vec::new();
    }
    read_package_strides(path).unwrap_or_else(|error| {
        warn!("The strides of {} do not read, so the repack keeps none: {error:#}", path.display());
        Vec::new()
    })
}

/// Stores in the package at `path`, loaded by `load`, those of `previous` whose clip's frames hash the same in it,
/// over its own strides of the same clips. Gives how many were kept; the package is rewritten (its other entries
/// copied as stored) only when its strides change.
pub fn keep_matching_strides<P: MobileAnimClips + Sync>(path: &Path, load: impl FnOnce(&Path) -> eyre::Result<P>,
    previous: &[MobileStrideRecord]) -> eyre::Result<usize> {
    if previous.is_empty() {
        return Ok(0);
    }
    let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
    let package = load(path).wrap_err_with(|| format!("load {} to keep its strides", path.display()))?;
    let kept = strides_still_matching(&package, previous, threads);
    let own = package.stride_records().to_vec();
    drop(package);
    let mut records: BTreeMap<(u32, u16), MobileStrideRecord> = own.iter()
        .map(|record| ((record.body_id, record.action_id), *record)).collect();
    records.extend(kept.iter().map(|record| ((record.body_id, record.action_id), *record)));
    let records: Vec<MobileStrideRecord> = records.into_values().collect();
    info!("{}: kept {} of {} strides measured before; {} were measured on other frames or clips now gone and need \
        measuring again", path.display(), kept.len(), previous.len(), previous.len() - kept.len());
    if records != own {
        write_package_strides(path, path, &records)?;
    }
    Ok(kept.len())
}
