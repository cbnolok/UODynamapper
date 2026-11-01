//! The output directory: the tiles, the state of a run, and atomic writes.
//!
//! ```text
//! <out>/index.html, leaflet/          the viewer
//! <out>/tiles/{z}/{x}/{y}.webp        the pyramid; tiles/empty.webp is the shared black tile
//! <out>/job.toml                      the frames still to render, rewritten by every run
//! <out>/spool/                        frames handed over by the renderer
//! <out>/state/plan.toml               the parameters, written once by `plan`
//! <out>/state/packages.toml           the package revisions of the first frame
//! <out>/state/frames.done             one frame id per line, appended after the frame's tiles are on disk
//! <out>/state/levels/{z}/{x}_{y}.rgb.zst   lossless tiles of the frame levels' lowest level and below
//! ```

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};

use crate::raster::{Rgb, RGB_CHANNELS};

pub const STATE_DIR: &str = "state";
pub const SPOOL_DIR: &str = "spool";
pub const TILES_DIR: &str = "tiles";
pub const LEVELS_DIR: &str = "levels";
pub const DONE_FILE: &str = "frames.done";
pub const PACKAGES_FILE: &str = "packages.toml";
pub const EMPTY_TILE_FILE: &str = "empty.webp";
pub const TILE_EXTENSION: &str = "webp";
const TEMP_SUFFIX: &str = ".tmp";
/// Zstd level of the lossless intermediate tiles: fast, and they are read once per level.
const INTERMEDIATE_ZSTD_LEVEL: i32 = 3;

/// Paths inside one output directory.
#[derive(Clone, Debug)]
pub struct OutputDir {
    pub root: PathBuf,
}

impl OutputDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn state(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }

    pub fn plan_file(&self) -> PathBuf {
        self.state().join(crate::formats::PLAN_FILE)
    }

    pub fn packages_file(&self) -> PathBuf {
        self.state().join(PACKAGES_FILE)
    }

    pub fn done_file(&self) -> PathBuf {
        self.state().join(DONE_FILE)
    }

    pub fn job_file(&self) -> PathBuf {
        self.root.join(crate::formats::JOB_FILE)
    }

    pub fn spool(&self) -> PathBuf {
        self.root.join(SPOOL_DIR)
    }

    pub fn tile(&self, level: u32, x: u32, y: u32) -> PathBuf {
        self.root.join(TILES_DIR).join(level.to_string()).join(x.to_string()).join(format!("{y}.{TILE_EXTENSION}"))
    }

    pub fn empty_tile(&self) -> PathBuf {
        self.root.join(TILES_DIR).join(EMPTY_TILE_FILE)
    }

    pub fn level_dir(&self, level: u32) -> PathBuf {
        self.state().join(LEVELS_DIR).join(level.to_string())
    }

    pub fn intermediate(&self, level: u32, x: u32, y: u32) -> PathBuf {
        self.level_dir(level).join(format!("{x}_{y}.rgb.zst"))
    }

    /// Frame ids recorded as done.
    pub fn read_done(&self) -> eyre::Result<BTreeSet<String>> {
        match fs::read_to_string(self.done_file()) {
            Ok(text) => Ok(text.lines().filter(|line| !line.is_empty()).map(str::to_string).collect()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
            Err(error) => Err(error).wrap_err_with(|| format!("read {}", self.done_file().display())),
        }
    }

    /// Records a frame as done; called after all its tiles are durably written.
    pub fn append_done(&self, id: &str) -> eyre::Result<()> {
        let path = self.done_file();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .wrap_err_with(|| format!("open {}", path.display()))?;
        writeln!(file, "{id}").and_then(|()| file.sync_all()).wrap_err_with(|| format!("append to {}", path.display()))
    }

    /// The lossless tiles stored for `level`, as `(x, y)`.
    pub fn intermediate_tiles(&self, level: u32) -> eyre::Result<Vec<(u32, u32)>> {
        let dir = self.level_dir(level);
        let mut tiles = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(tiles),
            Err(error) => return Err(error).wrap_err_with(|| format!("list {}", dir.display())),
        };
        for entry in entries {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".rgb.zst") else { continue };
            let parsed = stem.split_once('_').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)));
            let Some(tile) = parsed else { eyre::bail!("unexpected file {name} in {}", dir.display()) };
            tiles.push(tile);
        }
        tiles.sort_unstable();
        Ok(tiles)
    }

    pub fn write_intermediate(&self, level: u32, x: u32, y: u32, tile: &Rgb) -> eyre::Result<()> {
        let data = zstd::bulk::compress(&tile.pixels, INTERMEDIATE_ZSTD_LEVEL).wrap_err("compress tile")?;
        write_atomic(&self.intermediate(level, x, y), &data)
    }

    /// The lossless tile, or `None` where the level has no tile (all black).
    pub fn read_intermediate(&self, level: u32, x: u32, y: u32, tile_size: u32) -> eyre::Result<Option<Rgb>> {
        let path = self.intermediate(level, x, y);
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).wrap_err_with(|| format!("read {}", path.display())),
        };
        let size = tile_size as usize * tile_size as usize * RGB_CHANNELS;
        let pixels = zstd::bulk::decompress(&data, size).wrap_err_with(|| format!("decompress {}", path.display()))?;
        if pixels.len() != size {
            eyre::bail!("{} holds {} bytes, expected {size}", path.display(), pixels.len());
        }
        Ok(Some(Rgb { width: tile_size, height: tile_size, pixels }))
    }
}

/// Writes `data` to a temporary file next to `path`, syncs it and renames it over `path`, creating the parent
/// directories. A crash leaves either the old file or the new one, never a partial one.
pub fn write_atomic(path: &Path, data: &[u8]) -> eyre::Result<()> {
    let parent = path.parent().ok_or_else(|| eyre::eyre!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).wrap_err_with(|| format!("create {}", parent.display()))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(TEMP_SUFFIX);
    let temp = PathBuf::from(temp);
    let mut file = File::create(&temp).wrap_err_with(|| format!("create {}", temp.display()))?;
    file.write_all(data).and_then(|()| file.sync_all()).wrap_err_with(|| format!("write {}", temp.display()))?;
    drop(file);
    fs::rename(&temp, path).wrap_err_with(|| format!("rename {} to {}", temp.display(), path.display()))
}

/// Whether a spool file name is a temporary file the renderer has not committed yet.
pub fn is_temporary(name: &str) -> bool {
    name.ends_with(TEMP_SUFFIX)
}
