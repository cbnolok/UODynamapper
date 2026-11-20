//! The spool the tool reads: each frame's raw RGBA8 pixels, then its sidecar, each written as `*.tmp` and renamed, the
//! sidecar's rename committing the frame; and the provenance the sidecars record, the packages drawn from.

use crate::prelude::*;
use color_eyre::eyre::{self, WrapErr};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use udd_container::UddpReader;
use super::job::{ExportJob, ExportJobFrame};

const SIDECAR_FORMAT_VERSION: u32 = 1;
const TEMPORARY_SUFFIX: &str = ".tmp";
const PIXELS_EXTENSION: &str = "rgba";
const SIDECAR_EXTENSION: &str = "toml";

/// `<spool>/<id>.toml`. Mirrors `webmap_export_cli::formats::FrameSidecar`.
#[derive(Debug, Serialize)]
struct FrameSidecar<'a> {
    format_version: u32,
    id: &'a str,
    left_px: i64,
    top_px: i64,
    width_px: u32,
    height_px: u32,
    check_tile: [u32; 2],
    check_tile_px: [i64; 2],
    render_ms: u64,
    packages: &'a [PackageRevision],
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct PackageRevision {
    name: String,
    hash64: String,
}

/// The spool's committed frames: sidecars the tool has not consumed yet.
pub(super) fn count_sidecars(spool_dir: &Path) -> eyre::Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(spool_dir).wrap_err_with(|| format!("read {}", spool_dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some(SIDECAR_EXTENSION) {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn write_frame(
    job: &ExportJob,
    frame: &ExportJobFrame,
    pixels: &[u8],
    check_tile: (u32, u32),
    check_tile_px: [i64; 2],
    render_ms: u64,
    packages: &[PackageRevision],
) -> eyre::Result<()> {
    let sidecar = FrameSidecar {
        format_version: SIDECAR_FORMAT_VERSION,
        id: &frame.id,
        left_px: frame.left_px,
        top_px: frame.top_px,
        width_px: job.frame_size_px,
        height_px: job.frame_size_px,
        check_tile: [check_tile.0, check_tile.1],
        check_tile_px,
        render_ms,
        packages,
    };
    let sidecar = toml::to_string(&sidecar).wrap_err("serialize the sidecar")?;
    // The pixels first; the sidecar's rename is the commit marker.
    write_atomically(&job.spool_dir.join(format!("{}.{PIXELS_EXTENSION}", frame.id)), pixels)?;
    write_atomically(&job.spool_dir.join(format!("{}.{SIDECAR_EXTENSION}", frame.id)), sidecar.as_bytes())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(TEMPORARY_SUFFIX);
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).wrap_err_with(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, path).wrap_err_with(|| format!("rename {} into place", temporary.display()))
}

/// Every package the export draws from, with its embedded hash, in a fixed order.
pub(super) fn package_revisions(udd_path: &Path, job: &ExportJob) -> eyre::Result<Vec<PackageRevision>> {
    const SHARED_PACKAGES: [&str; 2] = ["tilemeta.uddp", "hues.uddp"];
    const CC_PACKAGES: [&str; 2] = ["tex_land_cc.uddp", "tex_art_cc.uddp"];
    // EC art and land fall back to the Classic packages, so an EC export reads both sets.
    const EC_PACKAGES: [&str; 3] = ["tex_land_ec.uddp", "tex_art_ec.uddp", "tex_aux_ec.uddp"];
    let facet_packages = [format!("map{}.uddp", job.facet), format!("statics{}.uddp", job.facet)];
    let textures: &[&str] = match job.texture_source {
        ClientTextureSource::Cc => &CC_PACKAGES,
        ClientTextureSource::Ec => &EC_PACKAGES,
    };
    let fallbacks: &[&str] = match job.texture_source {
        ClientTextureSource::Cc => &[],
        ClientTextureSource::Ec => &CC_PACKAGES,
    };
    let mut revisions = Vec::new();
    for name in facet_packages.iter().map(String::as_str).chain(SHARED_PACKAGES).chain(textures.iter().copied()).chain(fallbacks.iter().copied()) {
        let path = udd_path.join(name);
        if !path.exists() {
            if facet_packages.iter().any(|required| required == name) {
                eyre::bail!("{} is missing", path.display());
            }
            continue;
        }
        let package = UddpReader::load(&path).map_err(|error| eyre::eyre!("open {}: {error}", path.display()))?;
        revisions.push(PackageRevision { name: name.to_string(), hash64: format!("0x{:016x}", package.header().package_hash64) });
    }
    Ok(revisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use super::super::job::tests::{job_text, parse};

    #[test]
    fn spool_files_are_renamed_into_place_pixels_first() {
        let spool = std::env::temp_dir().join(format!("dynamapper-export-spool-{}", std::process::id()));
        fs::create_dir_all(&spool).unwrap();
        let mut job = parse(&job_text(&spool, "")).unwrap();
        job.frame_size_px = 2;
        let packages = [PackageRevision { name: "map0.uddp".into(), hash64: "0x0123456789abcdef".into() }];
        assert_eq!(count_sidecars(&spool).unwrap(), 0);
        write_frame(&job, &job.frames[0], &[0; 16], (10, 20), [89892, 1812], 5, &packages).unwrap();
        let names: HashSet<String> = fs::read_dir(&spool).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(names, HashSet::from(["f0021_0000.rgba".to_string(), "f0021_0000.toml".to_string()]));
        assert_eq!(count_sidecars(&spool).unwrap(), 1);
        let sidecar = fs::read_to_string(spool.join("f0021_0000.toml")).unwrap();
        assert!(sidecar.contains("check_tile = [10, 20]") && sidecar.contains("hash64 = \"0x0123456789abcdef\""), "{sidecar}");
        fs::remove_dir_all(&spool).unwrap();
    }
}
