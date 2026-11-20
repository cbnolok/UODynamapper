//! The export job: `job.toml`, which `webmap-export-cli run` writes, read strictly and checked against the renderer's
//! projection, and the settings the export draws with.

use crate::settings::shader_presets::{CLASSIC_PRESET_GROUP, PAINTERLY_PRESET_GROUP};
use crate::prelude::*;
use bevy::prelude::*;
use color_eyre::eyre::{self, WrapErr};
use serde::Deserialize;
use std::{fs, path::PathBuf};
use super::placement::{CAPTURE_PAD_PX, FramePlacement};

pub const EXPORT_JOB_ENV: &str = "DYNAMAPPER_EXPORT_JOB";
const JOB_FORMAT_VERSION: u32 = 1;
/// The projection this renderer draws at zoom 1: 44-pixel diamonds and 4 pixels per Z step.
pub(super) const TILE_HALF_WIDTH_PX: u32 = 22;
pub(super) const Z_STEP_PX: u32 = 4;
/// The texture size wgpu guarantees on every backend; the capture target, pad included, must fit it.
const MAX_FRAME_SIZE_PX: u32 = 8192;
/// The export draws at exactly one screen pixel per client pixel.
pub(super) const EXPORT_ZOOM: f32 = 1.0;
/// Fixed daylight: the period of the style preset the export uses.
const EXPORT_LIGHTING_PERIOD: &str = "afternoon";

/// `job.toml`, read strictly. Mirrors `webmap_export_cli::formats::Job`.
#[derive(Clone, Debug, Deserialize, Resource)]
#[serde(deny_unknown_fields)]
pub struct ExportJob {
    pub format_version: u32,
    pub facet: u32,
    pub map_width_tiles: u32,
    pub map_height_tiles: u32,
    pub texture_source: ClientTextureSource,
    pub tile_half_width_px: u32,
    pub z_step_px: u32,
    pub z_top_px: u32,
    pub frame_size_px: u32,
    pub frame_margin_px: u32,
    pub spool_dir: PathBuf,
    pub max_spool_frames: u32,
    pub frames: Vec<ExportJobFrame>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportJobFrame {
    pub id: String,
    /// Top-left corner of the rendered frame, in base-level pixels; may be negative.
    pub left_px: i64,
    pub top_px: i64,
}

/// Reads the job named by [`EXPORT_JOB_ENV`], if any.
pub fn load_requested_job() -> eyre::Result<Option<ExportJob>> {
    let Some(path) = std::env::var_os(EXPORT_JOB_ENV) else { return Ok(None) };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        eyre::bail!("{EXPORT_JOB_ENV} must be an absolute path, not {}", path.display());
    }
    let text = fs::read_to_string(&path).wrap_err_with(|| format!("read {}", path.display()))?;
    let job: ExportJob = uocf::utils::config_toml::parse_strict(&path.display().to_string(), &text)?;
    job.validate().wrap_err_with(|| format!("{}", path.display()))?;
    Ok(Some(job))
}

impl ExportJob {
    fn validate(&self) -> eyre::Result<()> {
        if self.format_version != JOB_FORMAT_VERSION {
            eyre::bail!("format_version {} is not {JOB_FORMAT_VERSION}", self.format_version);
        }
        if (self.tile_half_width_px, self.z_step_px) != (TILE_HALF_WIDTH_PX, Z_STEP_PX) {
            eyre::bail!(
                "the job's projection ({} px half width, {} px per Z) is not the renderer's ({TILE_HALF_WIDTH_PX}, {Z_STEP_PX})",
                self.tile_half_width_px,
                self.z_step_px
            );
        }
        if u8::try_from(self.facet).is_err() {
            eyre::bail!("facet {} is out of range", self.facet);
        }
        let max_frame_size_px = MAX_FRAME_SIZE_PX - 2 * CAPTURE_PAD_PX;
        if self.frame_size_px % 2 != 0 || self.frame_size_px > max_frame_size_px || self.frame_size_px <= 2 * self.frame_margin_px {
            eyre::bail!("frame_size_px {} (margin {}) is not an even size up to {max_frame_size_px} wider than both margins",
                self.frame_size_px, self.frame_margin_px);
        }
        if self.map_width_tiles == 0 || self.map_height_tiles == 0 {
            eyre::bail!("the map size is empty");
        }
        if !self.spool_dir.is_absolute() || !self.spool_dir.is_dir() {
            eyre::bail!("spool_dir {} is not an existing absolute directory", self.spool_dir.display());
        }
        if self.max_spool_frames == 0 {
            eyre::bail!("max_spool_frames must be at least 1");
        }
        if self.frames.is_empty() {
            eyre::bail!("the job has no frames");
        }
        for frame in &self.frames {
            let plain = !frame.id.is_empty()
                && !frame.id.starts_with('.')
                && frame.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !plain {
                eyre::bail!("frame id {:?} is not a plain file name", frame.id);
            }
        }
        Ok(())
    }

    pub(super) fn lighting_preset_key(&self) -> String {
        // EC exports keep the painterly look they had before KR-like became the faithful client look.
        let style = match self.texture_source {
            ClientTextureSource::Cc => CLASSIC_PRESET_GROUP,
            ClientTextureSource::Ec => PAINTERLY_PRESET_GROUP,
        };
        format!("{style}.{EXPORT_LIGHTING_PERIOD}")
    }
}

/// The export preset, applied in memory to the loaded settings; export mode never saves them.
pub fn apply_settings_overrides(settings: &mut Settings, job: &ExportJob) {
    settings.core.world.hide_player = true;
    settings.world_rendering.enable_statics = true;
    settings.world_rendering.enable_static_lights = false;
    settings.world_rendering.enable_static_art_local_lights = false;
    settings.world_rendering.show_hidden_multi_parts = false;
    settings.world_rendering.show_only_player_floor = false;
    settings.world_rendering.caves = crate::world::caves::CaveView::HiddenUntilEntered;
    settings.world_rendering.diagnostics.highlight_hovered_static_object = false;
    settings.graphics.vsync = false;
    settings.graphics.reduce_unfocused_fps = false;
    settings.graphics.anti_aliasing = AntiAliasingMode::Off;
    settings.graphics.art_texture_source = job.texture_source;
    settings.graphics.land_texture_source = job.texture_source.into();
    settings.app.window.free_camera = false;
    settings.app.window.perspective_camera = false;
    settings.app.performance.show_overlay = false;
    settings.app.performance.frame_limit_enabled = false;
    settings.app.debug.map_render_wireframe = false;
    settings.session_state.window.zoom = EXPORT_ZOOM;
    let placement = FramePlacement::new(job, &job.frames[0]);
    settings.session_state.world.last_p = placement.player_position(job);
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::path::Path;

    pub(in crate::export) const BRITANNIA_WIDTH: u32 = 7168;
    pub(in crate::export) const BRITANNIA_HEIGHT: u32 = 4096;
    const PILOT_Z_TOP_PX: u32 = 1152;
    const PILOT_FRAME_SIZE_PX: u32 = 4160;
    const PILOT_MARGIN_PX: u32 = 32;

    pub(in crate::export) fn job_text(spool: &Path, extra: &str) -> String {
        format!(
            "format_version = 1\nfacet = 0\nmap_width_tiles = {BRITANNIA_WIDTH}\nmap_height_tiles = {BRITANNIA_HEIGHT}\n\
             texture_source = \"cc\"\ntile_half_width_px = 22\nz_step_px = 4\nz_top_px = {PILOT_Z_TOP_PX}\n\
             frame_size_px = {PILOT_FRAME_SIZE_PX}\nframe_margin_px = {PILOT_MARGIN_PX}\nspool_dir = {:?}\nmax_spool_frames = 2\n{extra}\
             [[frames]]\nid = \"f0021_0000\"\nleft_px = 85984\ntop_px = -32\n",
            spool.display().to_string()
        )
    }

    pub(in crate::export) fn parse(text: &str) -> eyre::Result<ExportJob> {
        let job: ExportJob = uocf::utils::config_toml::parse_strict("job.toml", text)?;
        job.validate()?;
        Ok(job)
    }

    #[test]
    fn job_parses_strictly_and_checks_the_projection() {
        let spool = std::env::temp_dir();
        let job = parse(&job_text(&spool, "")).unwrap();
        assert_eq!(job.frames[0].left_px, 85984);
        assert_eq!(job.lighting_preset_key(), "classic.afternoon");
        assert!(parse(&job_text(&spool, "unknown_key = 1\n")).is_err());
        assert!(parse(&job_text(&spool, "").replace("z_step_px = 4", "z_step_px = 5")).is_err());
        assert!(parse(&job_text(&spool, "").replace("\"f0021_0000\"", "\"../escape\"")).is_err());
        assert!(parse(&job_text(Path::new("relative/spool"), "")).is_err());
    }
}
