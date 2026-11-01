//! Planning, consuming the renderer's spool, and building the pyramid.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

use color_eyre::eyre::{self, WrapErr};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::formats::{
    self, FRAME_BYTES_PER_PIXEL, FRAME_PIXELS_EXTENSION, FRAME_SIDECAR_EXTENSION, FrameSidecar, Job, PackageRevision,
    Plan,
};
use crate::geometry::{FrameIndex, Layout};
use crate::raster::Rgb;
use crate::storage::{self, OutputDir, write_atomic};
use crate::webp::{self, WebpSettings};

/// How often the spool is scanned while the renderer works.
pub const SPOOL_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Children per parent tile and per side.
const CHILDREN_PER_SIDE: u32 = 2;

/// `state/packages.toml`: the package revisions every frame must match.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PackagesFile {
    packages: Vec<PackageRevision>,
}

/// Creates the output directory and its plan. Refuses an existing plan: a run is resumed, never re-planned.
pub fn write_plan(out: &OutputDir, plan: &Plan) -> eyre::Result<()> {
    plan.validate()?;
    if out.plan_file().exists() {
        eyre::bail!("{} exists: resume with `run`, or plan into a new directory", out.plan_file().display());
    }
    fs::create_dir_all(out.spool()).wrap_err_with(|| format!("create {}", out.spool().display()))?;
    write_atomic(&out.plan_file(), formats::to_toml(plan)?.as_bytes())
}

pub fn read_plan(out: &OutputDir) -> eyre::Result<Plan> {
    let plan: Plan = formats::read_strict(&out.plan_file())?;
    plan.validate()?;
    Ok(plan)
}

/// The planned frames that are not done yet.
pub fn remaining_frames(out: &OutputDir, plan: &Plan, layout: &Layout) -> eyre::Result<Vec<FrameIndex>> {
    let done = out.read_done()?;
    Ok(layout.frames(plan.region).into_iter().filter(|frame| !done.contains(&frame.id())).collect())
}

/// Writes `job.toml` for the remaining frames and returns its absolute path.
pub fn write_job(out: &OutputDir, plan: &Plan, layout: &Layout, frames: &[FrameIndex]) -> eyre::Result<std::path::PathBuf> {
    let spool = fs::canonicalize(out.spool()).wrap_err_with(|| format!("resolve {}", out.spool().display()))?;
    let job = Job::new(plan, layout, &spool, frames);
    let path = out.job_file();
    write_atomic(&path, formats::to_toml(&job)?.as_bytes())?;
    fs::canonicalize(&path).wrap_err_with(|| format!("resolve {}", path.display()))
}

/// Tiles written for one frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FrameTiles {
    pub cut: usize,
    pub written: usize,
}

/// Cuts a frame's kept area into the tiles of the frame levels, encodes and writes them, and stores the lowest
/// frame level's tile losslessly for the pyramid.
pub fn process_frame(out: &OutputDir, layout: &Layout, settings: WebpSettings, frame: FrameIndex, rgba: &[u8]) -> eyre::Result<FrameTiles> {
    let grid = layout.grid;
    let size = grid.frame_size_px();
    let expected = u64::from(size) * u64::from(size) * FRAME_BYTES_PER_PIXEL;
    if rgba.len() as u64 != expected {
        eyre::bail!("frame {}: {} bytes, expected {expected} ({size}x{size} RGBA8)", frame.id(), rgba.len());
    }
    let mut image = Rgb::from_rgba_crop(rgba, size, grid.frame_margin_px, grid.frame_margin_px, grid.frame_keep_px());
    let mut tiles = Vec::new();
    let base = layout.base_level();
    for step in 0..grid.frame_levels() {
        if step > 0 {
            image = image.downsample_half();
        }
        let per_side = grid.frame_keep_tiles >> step;
        for b in 0..per_side {
            for a in 0..per_side {
                let tile = image.crop(a * grid.tile_size_px, b * grid.tile_size_px, grid.tile_size_px);
                tiles.push((base - step, frame.i * per_side + a, frame.j * per_side + b, tile));
            }
        }
    }
    let intermediate_level = layout.intermediate_level();
    let written: Vec<bool> = tiles
        .par_iter()
        .map(|(level, x, y, tile)| -> eyre::Result<bool> {
            if tile.is_black() {
                return Ok(false);
            }
            if *level == intermediate_level {
                out.write_intermediate(*level, *x, *y, tile)?;
            }
            let data = webp::encode_rgb(&tile.pixels, tile.width, tile.height, settings)?;
            write_atomic(&out.tile(*level, *x, *y), &data)?;
            Ok(true)
        })
        .collect::<eyre::Result<_>>()?;
    Ok(FrameTiles { cut: tiles.len(), written: written.iter().filter(|&&w| w).count() })
}

/// Checks that every frame was rendered from the same packages as the first.
fn check_packages(out: &OutputDir, sidecar: &FrameSidecar) -> eyre::Result<()> {
    let current = PackagesFile { packages: sidecar.packages.clone() };
    let path = out.packages_file();
    if !path.exists() {
        return write_atomic(&path, formats::to_toml(&current)?.as_bytes());
    }
    let first: PackagesFile = formats::read_strict(&path)?;
    if first != current {
        eyre::bail!(
            "frame {} was rendered from other packages than the first frame ({}): {:?}",
            sidecar.id,
            path.display(),
            current.packages
        );
    }
    Ok(())
}

/// Frames waiting in the spool: their sidecar is committed. Leftovers of frames already done are removed.
fn committed_frames(out: &OutputDir, pending: &BTreeMap<String, FrameIndex>) -> eyre::Result<Vec<String>> {
    let spool = out.spool();
    let mut ids = Vec::new();
    for entry in fs::read_dir(&spool).wrap_err_with(|| format!("list {}", spool.display()))? {
        let path = entry?.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if storage::is_temporary(&name) || path.extension().and_then(|e| e.to_str()) != Some(FRAME_SIDECAR_EXTENSION) {
            continue;
        }
        let id = name.trim_end_matches(&format!(".{FRAME_SIDECAR_EXTENSION}")).to_string();
        if pending.contains_key(&id) {
            ids.push(id);
        } else if out.read_done()?.contains(&id) {
            remove_spooled(out, &id)?;
        } else {
            eyre::bail!("{} is not a frame of this job", path.display());
        }
    }
    ids.sort();
    Ok(ids)
}

/// Removes a consumed frame: the pixels first, then the sidecar, which is what the renderer counts.
fn remove_spooled(out: &OutputDir, id: &str) -> eyre::Result<()> {
    for extension in [FRAME_PIXELS_EXTENSION, FRAME_SIDECAR_EXTENSION] {
        let path = out.spool().join(format!("{id}.{extension}"));
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).wrap_err_with(|| format!("remove {}", path.display())),
        }
    }
    Ok(())
}

/// Consumes spooled frames until every frame in `frames` is done. With `renderer`, an early exit of the renderer
/// is an error.
pub fn consume_spool(out: &OutputDir, plan: &Plan, layout: &Layout, frames: &[FrameIndex], mut renderer: Option<&mut Child>) -> eyre::Result<()> {
    let mut pending: BTreeMap<String, FrameIndex> = frames.iter().map(|&frame| (frame.id(), frame)).collect();
    let total = pending.len();
    let settings = plan.webp();
    while !pending.is_empty() {
        let ids = committed_frames(out, &pending)?;
        if ids.is_empty() {
            if let Some(child) = renderer.as_deref_mut() {
                if let Some(status) = child.try_wait().wrap_err("poll the renderer")? {
                    eyre::bail!("the renderer exited ({status}) with {} of {total} frames missing", pending.len());
                }
            }
            std::thread::sleep(SPOOL_POLL_INTERVAL);
            continue;
        }
        for id in ids {
            let frame = pending[&id];
            let sidecar_path = out.spool().join(format!("{id}.{FRAME_SIDECAR_EXTENSION}"));
            let sidecar: FrameSidecar = formats::read_strict(&sidecar_path)?;
            if sidecar.id != id {
                eyre::bail!("{}: id {} does not match the file name", sidecar_path.display(), sidecar.id);
            }
            sidecar.check(layout, layout.frame_rect(frame))?;
            check_packages(out, &sidecar)?;
            let pixels_path = out.spool().join(format!("{id}.{FRAME_PIXELS_EXTENSION}"));
            let rgba = fs::read(&pixels_path).wrap_err_with(|| format!("read {}", pixels_path.display()))?;
            let started = Instant::now();
            let tiles = process_frame(out, layout, settings, frame, &rgba)?;
            out.append_done(&id)?;
            remove_spooled(out, &id)?;
            pending.remove(&id);
            eprintln!(
                "frame {id}: {} of {} tiles written, render {} ms, tiles {} ms [{}/{total}]",
                tiles.written,
                tiles.cut,
                sidecar.render_ms,
                started.elapsed().as_millis(),
                total - pending.len()
            );
        }
    }
    Ok(())
}

/// Tiles written per pyramid level.
pub type LevelCounts = Vec<(u32, usize)>;

/// Builds every level below the frame levels from the lossless tiles, never from lossy ones. Requires every
/// planned frame to be done. Rerunning it rewrites the same files.
pub fn build_pyramid(out: &OutputDir, plan: &Plan, layout: &Layout) -> eyre::Result<LevelCounts> {
    let missing = remaining_frames(out, plan, layout)?;
    if !missing.is_empty() {
        eyre::bail!("{} planned frames are not done yet; finish them with `run`", missing.len());
    }
    let settings = plan.webp();
    let tile_size = layout.grid.tile_size_px;
    write_atomic(&out.empty_tile(), &webp::encode_rgb(&Rgb::black(tile_size, tile_size).pixels, tile_size, tile_size, settings)?)?;
    let mut counts = Vec::new();
    for level in (0..layout.intermediate_level()).rev() {
        let children = out.intermediate_tiles(level + 1)?;
        let mut parents: Vec<(u32, u32)> = children.iter().map(|&(x, y)| (x / CHILDREN_PER_SIDE, y / CHILDREN_PER_SIDE)).collect();
        parents.sort_unstable();
        parents.dedup();
        let written: Vec<bool> = parents
            .par_iter()
            .map(|&(x, y)| -> eyre::Result<bool> {
                let mut block = Rgb::black(CHILDREN_PER_SIDE * tile_size, CHILDREN_PER_SIDE * tile_size);
                for b in 0..CHILDREN_PER_SIDE {
                    for a in 0..CHILDREN_PER_SIDE {
                        let (cx, cy) = (x * CHILDREN_PER_SIDE + a, y * CHILDREN_PER_SIDE + b);
                        if let Some(child) = out.read_intermediate(level + 1, cx, cy, tile_size)? {
                            block.paste(&child, a * tile_size, b * tile_size);
                        }
                    }
                }
                let tile = block.downsample_half();
                if tile.is_black() {
                    return Ok(false);
                }
                out.write_intermediate(level, x, y, &tile)?;
                write_atomic(&out.tile(level, x, y), &webp::encode_rgb(&tile.pixels, tile_size, tile_size, settings)?)?;
                Ok(true)
            })
            .collect::<eyre::Result<_>>()?;
        let count = written.iter().filter(|&&w| w).count();
        eprintln!("level {level}: {count} tiles");
        counts.push((level, count));
    }
    Ok(counts)
}

/// Whether `path` is an existing, non-empty directory.
pub fn is_non_empty_dir(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}
