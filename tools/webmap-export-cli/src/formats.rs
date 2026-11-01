//! The TOML files of an export: the plan (the tool's parameters), the job (read by the renderer's export mode)
//! and the frame sidecar (written by the renderer). All three are read strictly.
//!
//! The renderer contract: `docs/agent_notes/proposals/2026-11-01-web-map-export.md`, "Contract for the renderer
//! session".

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use serde::{Deserialize, Serialize};

use crate::geometry::{FrameIndex, Layout, PixelRect, TileGrid, Z_STEP_PX, TILE_HALF_PX};
use crate::webp::WebpSettings;

/// Version of the plan, job and sidecar formats.
pub const FORMAT_VERSION: u32 = 1;
/// Frames the renderer may leave in the spool before it waits for the tool.
pub const MAX_SPOOL_FRAMES: u32 = 2;
/// Bytes per pixel of a spooled frame (RGBA8).
pub const FRAME_BYTES_PER_PIXEL: u64 = 4;

pub const PLAN_FILE: &str = "plan.toml";
pub const JOB_FILE: &str = "job.toml";
pub const FRAME_PIXELS_EXTENSION: &str = "rgba";
pub const FRAME_SIDECAR_EXTENSION: &str = "toml";

/// Art and land textures the renderer draws with.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum TextureSource {
    Cc,
    Ec,
}

/// `state/plan.toml`: everything that fixes the output. A rerun reads it; it is never rewritten.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format_version: u32,
    pub facet: u32,
    pub map_width_tiles: u32,
    pub map_height_tiles: u32,
    pub max_sprite_height_px: u32,
    pub texture_source: TextureSource,
    pub tile_size_px: u32,
    pub frame_keep_tiles: u32,
    pub frame_margin_px: u32,
    pub webp_quality: f32,
    pub webp_method: i32,
    pub webp_sharp_yuv: bool,
    /// Tile region `[x0, y0, x1, y1)` to export instead of the whole facet (a pilot or a partial run). Optional by
    /// design: absent means the whole facet.
    pub region: Option<[u32; 4]>,
}

impl Plan {
    pub fn grid(&self) -> TileGrid {
        TileGrid {
            tile_size_px: self.tile_size_px,
            frame_keep_tiles: self.frame_keep_tiles,
            frame_margin_px: self.frame_margin_px,
        }
    }

    pub fn layout(&self) -> eyre::Result<Layout> {
        Layout::new(self.map_width_tiles, self.map_height_tiles, self.max_sprite_height_px, self.grid())
            .map_err(|error| eyre::eyre!("{PLAN_FILE}: {error}"))
    }

    pub fn webp(&self) -> WebpSettings {
        WebpSettings { quality: self.webp_quality, method: self.webp_method, sharp_yuv: self.webp_sharp_yuv }
    }

    pub fn validate(&self) -> eyre::Result<()> {
        if self.format_version != FORMAT_VERSION {
            eyre::bail!("{PLAN_FILE}: format_version {} is not {FORMAT_VERSION}", self.format_version);
        }
        self.webp().validate().map_err(|error| eyre::eyre!("{PLAN_FILE}: {error}"))?;
        self.layout().map(|_| ())
    }
}

/// `job.toml`: the frames the renderer is asked for.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub format_version: u32,
    pub facet: u32,
    pub map_width_tiles: u32,
    pub map_height_tiles: u32,
    pub texture_source: TextureSource,
    /// The projection, repeated for the renderer: the top vertex of tile `(x, y)` at height `z` is at
    /// `((x − y + map_height_tiles) · tile_half_width_px, (x + y) · tile_half_width_px − z_step_px · z + z_top_px)`.
    pub tile_half_width_px: u32,
    pub z_step_px: u32,
    pub z_top_px: u32,
    pub frame_size_px: u32,
    pub frame_margin_px: u32,
    /// Absolute path of the spool directory.
    pub spool_dir: PathBuf,
    pub max_spool_frames: u32,
    pub frames: Vec<JobFrame>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobFrame {
    pub id: String,
    /// Top-left corner of the rendered frame (margin included), in base-level pixels; may be negative.
    pub left_px: i64,
    pub top_px: i64,
}

impl Job {
    pub fn new(plan: &Plan, layout: &Layout, spool_dir: &Path, frames: &[FrameIndex]) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            facet: plan.facet,
            map_width_tiles: plan.map_width_tiles,
            map_height_tiles: plan.map_height_tiles,
            texture_source: plan.texture_source,
            tile_half_width_px: TILE_HALF_PX as u32,
            z_step_px: Z_STEP_PX as u32,
            z_top_px: layout.z_top_px,
            frame_size_px: layout.grid.frame_size_px(),
            frame_margin_px: layout.grid.frame_margin_px,
            spool_dir: spool_dir.to_path_buf(),
            max_spool_frames: MAX_SPOOL_FRAMES,
            frames: frames
                .iter()
                .map(|&frame| {
                    let rect = layout.frame_rect(frame);
                    JobFrame { id: frame.id(), left_px: rect.left, top_px: rect.top }
                })
                .collect(),
        }
    }
}

/// `<spool>/<id>.toml`, written by the renderer after `<id>.rgba` and renamed into place last.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrameSidecar {
    pub format_version: u32,
    pub id: String,
    pub left_px: i64,
    pub top_px: i64,
    pub width_px: u32,
    pub height_px: u32,
    /// A tile the renderer placed, and the base-level pixel where it put that tile's top vertex at z = 0: the
    /// tool checks it against its own projection.
    pub check_tile: [u32; 2],
    pub check_tile_px: [i64; 2],
    pub render_ms: u64,
    pub packages: Vec<PackageRevision>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageRevision {
    /// File name, e.g. `map0.uddp`.
    pub name: String,
    /// The package hash64, `0x` and 16 hex digits.
    pub hash64: String,
}

impl FrameSidecar {
    /// Checks the sidecar against the frame the job asked for and the plan's projection.
    pub fn check(&self, layout: &Layout, expected: PixelRect) -> eyre::Result<()> {
        if self.format_version != FORMAT_VERSION {
            eyre::bail!("frame {}: format_version {} is not {FORMAT_VERSION}", self.id, self.format_version);
        }
        let got = (self.left_px, self.top_px, i64::from(self.width_px), i64::from(self.height_px));
        let want = (expected.left, expected.top, expected.width, expected.height);
        if got != want {
            eyre::bail!("frame {}: rectangle {got:?} is not the job's {want:?}", self.id);
        }
        let [x, y] = self.check_tile;
        let projected = layout.project(i64::from(x), i64::from(y), 0);
        if projected != (self.check_tile_px[0], self.check_tile_px[1]) {
            eyre::bail!(
                "frame {}: the renderer put tile {x},{y} at {:?}, the projection says {projected:?}",
                self.id,
                self.check_tile_px
            );
        }
        Ok(())
    }
}

/// Reads a TOML file strictly; errors name the file.
pub fn read_strict<T: serde::de::DeserializeOwned>(path: &Path) -> eyre::Result<T> {
    let text = std::fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
    uocf::utils::config_toml::parse_strict(&path.display().to_string(), &text)
}

pub fn to_toml<T: Serialize>(value: &T) -> eyre::Result<String> {
    toml::to_string(value).wrap_err("serialize TOML")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sidecar_text() -> String {
        r#"format_version = 1
id = "f0001_0002"
left_px = 4064
top_px = 8160
width_px = 4160
height_px = 4160
check_tile = [10, 20]
check_tile_px = [90332, 1172]
render_ms = 850
packages = [{ name = "map0.uddp", hash64 = "0x0123456789abcdef" }]
"#
        .to_string()
    }

    #[test]
    fn sidecar_is_read_strictly() {
        let sidecar: FrameSidecar = uocf::utils::config_toml::parse_strict("f.toml", &sidecar_text()).unwrap();
        assert_eq!(sidecar.check_tile, [10, 20]);
        let unknown = sidecar_text() + "extra = 1\n";
        let error = uocf::utils::config_toml::parse_strict::<FrameSidecar>("f.toml", &unknown).unwrap_err();
        assert!(error.to_string().contains("extra"), "{error}");
        let missing = sidecar_text().replace("render_ms = 850\n", "");
        assert!(uocf::utils::config_toml::parse_strict::<FrameSidecar>("f.toml", &missing).is_err());
    }

    #[test]
    fn sidecar_check_rejects_a_wrong_rectangle_or_projection() {
        let layout = Layout::new(7168, 4096, 0, TileGrid::EXPORT).unwrap();
        let frame = FrameIndex { i: 1, j: 2 };
        let rect = layout.frame_rect(frame);
        let mut sidecar: FrameSidecar = uocf::utils::config_toml::parse_strict("f.toml", &sidecar_text()).unwrap();
        let (px, py) = layout.project(10, 20, 0);
        sidecar.check_tile_px = [px, py];
        sidecar.check(&layout, rect).unwrap();
        sidecar.check_tile_px[1] += 1;
        assert!(sidecar.check(&layout, rect).is_err());
        sidecar.check_tile_px[1] -= 1;
        sidecar.left_px += 1;
        assert!(sidecar.check(&layout, rect).is_err());
    }

    #[test]
    fn job_round_trips_through_strict_toml() {
        let plan = Plan {
            format_version: FORMAT_VERSION,
            facet: 0,
            map_width_tiles: 7168,
            map_height_tiles: 4096,
            max_sprite_height_px: 0,
            texture_source: TextureSource::Cc,
            tile_size_px: 512,
            frame_keep_tiles: 8,
            frame_margin_px: 32,
            webp_quality: 85.0,
            webp_method: 4,
            webp_sharp_yuv: true,
            region: None,
        };
        plan.validate().unwrap();
        let layout = plan.layout().unwrap();
        let frames = layout.frames(Some([1400, 1600, 1500, 1700]));
        let job = Job::new(&plan, &layout, Path::new("/spool"), &frames);
        let text = to_toml(&job).unwrap();
        let back: Job = uocf::utils::config_toml::parse_strict("job.toml", &text).unwrap();
        assert_eq!(back, job);
        assert_eq!(back.frames[0].left_px % 4096, 4096 - 32);
        let plan_text = to_toml(&plan).unwrap();
        assert_eq!(uocf::utils::config_toml::parse_strict::<Plan>("plan.toml", &plan_text).unwrap(), plan);
    }
}
