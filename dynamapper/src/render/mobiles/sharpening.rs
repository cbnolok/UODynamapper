//! Each mobile's frame filter, from `runtime_specs/mobiles/Sharpening.kdl` (spec.rs): its CAS strength and its edge
//! treatment, by animation package, by the body's size class (its largest stored frame side) and by body. Each frame
//! drawn carries its body's filter in its instance (`MobileFrameFilter`, art/art_bindings.wesl), and the shader
//! multiplies the sharpening by the style's `mobile_sharpening` (`effects.mobile_sharpening`). The file is read at
//! startup and again within a second of a save of the bundled file or of the user's copy; a file that no longer reads
//! leaves the filters in force.

pub mod spec;

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};
use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use crate::console_logger::{self, LogAbout, LogSev};
use crate::settings::runtime_specs::SpecFiles;
use crate::world::mobile_motion::DIRECTION_COUNT;
use super::clips::{AnimPackages, AnimSource};
use super::upscales::package_of;
use spec::{FrameFilter, MobileSharpeningSpec, SHARPENING_FILE_NAME, SHARPENING_SPECS_FOLDER};

/// What the log calls the file.
const SHARPENING_WHAT: &str = "the mobile sharpening";
/// How often the files are checked for a save.
const SHARPENING_FILE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// The texels an outlined frame's card grows by on each side: room for the outline, as the frames are stored cropped
/// to their visible pixels.
pub const OUTLINE_PADDING_TEXELS: u32 = 1;
/// The widest outline the shader draws, in texels of the frame: one art pixel of a 2x frame (art/mobile_edges.wesl
/// `MOBILE_OUTLINE_WIDE_REACH`).
pub const OUTLINE_MAX_REACH_TEXELS: u32 = 2;

/// How wide a mobile's outline is on an upscaled frame (SPAN 2x at run time, or 2x frames packed ahead of time), whose
/// art pixel spans `Clip::texel_scale` texels. Kept in the app-written world rendering settings as
/// `world_rendering.mobile_outline_width`; the bundled settings/world_rendering.toml holds the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileOutlineWidth {
    /// One texel of the frame drawn: on a 2x frame half as wide on screen as at 1x.
    OneTexel,
    /// One art pixel: as many texels as the frame's upscale (two on a 2x frame), as wide on screen as at 1x.
    FollowsUpscale,
}

impl MobileOutlineWidth {
    pub const ALL: [Self; 2] = [Self::OneTexel, Self::FollowsUpscale];

    pub fn label(self) -> &'static str {
        match self {
            Self::OneTexel => "One Texel",
            Self::FollowsUpscale => "Follows Upscale",
        }
    }

    /// The texels an outline reaches past the silhouette of a frame of `texel_scale` texels to an art pixel, the card
    /// padded by as many: at most `OUTLINE_MAX_REACH_TEXELS`.
    pub fn reach_texels(self, texel_scale: u32) -> u32 {
        match self {
            Self::OneTexel => OUTLINE_PADDING_TEXELS,
            Self::FollowsUpscale => (OUTLINE_PADDING_TEXELS * texel_scale.max(1)).min(OUTLINE_MAX_REACH_TEXELS),
        }
    }
}

/// A mobile frame's filter, as the art sprite shader reads it after the frame's ground strips (`MobileSpriteInstance`,
/// art/art_bindings.wesl `MobileFrameFilter`). All zeros (an instance not filled) leaves the frame as it is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MobileFrameFilter {
    /// The CAS strength before the style's multiplier, 0..1.
    pub sharpening: f32,
    /// The share of the strength a partly transparent texel takes.
    pub fringe_sharpening: f32,
    /// The pull of the partial alpha towards 0 or 1 about `alpha_midpoint`, 0 off.
    pub alpha_contrast: f32,
    pub alpha_midpoint: f32,
    /// The darkening of the rim and the outermost opaque texels towards the contour colour, 0 off.
    pub contour: f32,
    /// The contour's sRGB-encoded colour, red in the lowest byte (WGSL `unpack4x8unorm`).
    pub contour_color: u32,
    /// The opacity of the outline outside the silhouette, 0 off.
    pub outline: f32,
    /// The texels the card grows by on each side beyond the frame (`OUTLINE_PADDING_TEXELS` with an outline, else
    /// 0): its taps outside the frame read transparent. The outline reaches as far past the silhouette
    /// (`for_frame`: two texels on an upscaled frame with `MobileOutlineWidth::FollowsUpscale`).
    pub padding_texels: u32,
    /// The strength of the interior lines (art/mobile_inner_edges.wesl), 0 off.
    pub inner_lines: f32,
    /// The strength of the contrast across the interior edges, 0 off.
    pub inner_contrast: f32,
    /// The colour step at which an interior edge starts.
    pub inner_threshold: f32,
    /// Keeps the struct a multiple of 16 bytes, as the instance array's stride.
    pub _padding: u32,
}

impl MobileFrameFilter {
    pub fn of(filter: FrameFilter) -> Self {
        let [red, green, blue] = filter.contour_color;
        Self {
            sharpening: filter.sharpening,
            fringe_sharpening: filter.fringe_sharpening,
            alpha_contrast: filter.alpha_contrast,
            alpha_midpoint: filter.alpha_midpoint,
            contour: filter.contour,
            contour_color: u32::from_le_bytes([red, green, blue, u8::MAX]),
            outline: filter.outline,
            padding_texels: if filter.outline > 0.0 { OUTLINE_PADDING_TEXELS } else { 0 },
            inner_lines: filter.inner_lines,
            inner_contrast: filter.inner_contrast,
            inner_threshold: filter.inner_threshold,
            _padding: 0,
        }
    }

    /// The filter of a frame of a clip of `texel_scale` texels to an art pixel: an outline reaches, and the card is
    /// padded, by `width`. A filter without an outline stays as it is.
    pub fn for_frame(self, width: MobileOutlineWidth, texel_scale: u32) -> Self {
        if self.padding_texels == 0 {
            return self;
        }
        Self { padding_texels: width.reach_texels(texel_scale), ..self }
    }
}

/// The filters of the bodies drawn, from the file, and the largest frame side of each body, read once from its
/// package's records.
#[derive(Resource)]
pub struct MobileSharpening {
    spec: MobileSharpeningSpec,
    files: SpecFiles,
    /// The modification times of the bundled file and of the user's copy when they were last read (`None`: absent).
    modified: [Option<SystemTime>; 2],
    sides: HashMap<(AnimSource, u32), Option<u32>>,
    filters: HashMap<(AnimSource, u32), MobileFrameFilter>,
}

impl MobileSharpening {
    /// Reads the file at startup: the bundled file is required, a missing or invalid one stops the app; a user copy
    /// that does not read leaves it in force, with a warning.
    pub fn load() -> Self {
        let files = SpecFiles::of(SHARPENING_SPECS_FOLDER, SHARPENING_FILE_NAME);
        let modified = files_modified(&files);
        let loaded = files.load(MobileSharpeningSpec::decode)
            .unwrap_or_else(|error| crate::settings::exit_on_startup_config_error(&error));
        loaded.report(SHARPENING_WHAT);
        Self { spec: loaded.value, files, modified, sides: HashMap::new(), filters: HashMap::new() }
    }

    /// The bodies' sizes are read again from `packages`' records as they are drawn.
    pub fn set_packages(&mut self) {
        self.sides.clear();
        self.filters.clear();
    }

    /// Reads the file again. The user's copy when it exists, else the bundled file; one that does not read leaves the
    /// filters in force, with a warning.
    fn reload(&mut self) {
        self.modified = files_modified(&self.files);
        let loaded = self.files.load(MobileSharpeningSpec::decode).and_then(|loaded| match loaded.user_error {
            Some(error) => Err(error),
            None => Ok(loaded),
        });
        match loaded {
            Ok(loaded) => {
                loaded.report(SHARPENING_WHAT);
                if loaded.value != self.spec {
                    self.spec = loaded.value;
                    self.filters.clear();
                }
            }
            Err(error) => {
                let message = format!("{error:#}: the mobile sharpening in force stays until it is fixed");
                console_logger::one(LogSev::Warn, LogAbout::Settings, &message);
                crate::ingame_sysmessage_logger::warning(message);
            }
        }
    }

    /// The filter of server `body`'s frames from `source`.
    pub fn filter(&mut self, source: AnimSource, body: u32, packages: &AnimPackages) -> MobileFrameFilter {
        if let Some(filter) = self.filters.get(&(source, body)) {
            return *filter;
        }
        let side = *self.sides.entry((source, body)).or_insert_with(|| body_largest_side(packages, source, body));
        let filter = MobileFrameFilter::of(self.spec.resolve(source, body, side));
        debug!("Mobile sharpening: {source:?} body {body}, largest frame side {side:?}: {filter:?}");
        self.filters.insert((source, body), filter);
        filter
    }
}

/// Reads the file again when the bundled file or the user's copy is saved, or the copy created or deleted.
pub fn sys_watch_mobile_sharpening(mut sharpening: ResMut<MobileSharpening>, time: Res<Time<Real>>,
    mut timer: Local<Option<Timer>>) {
    let timer = timer.get_or_insert_with(|| Timer::new(SHARPENING_FILE_CHECK_INTERVAL, TimerMode::Repeating));
    let checked = timer.tick(time.delta()).just_finished();
    if checked && files_modified(&sharpening.files) != sharpening.modified {
        sharpening.reload();
    }
}

fn files_modified(files: &SpecFiles) -> [Option<SystemTime>; 2] {
    [modified(&files.bundled), modified(&files.user)]
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// The larger of the width and the height over every frame `source`'s package stores of server `body`, every action
/// and facing at 1x, from its records: none without frames.
fn body_largest_side(packages: &AnimPackages, source: AnimSource, body: u32) -> Option<u32> {
    let package = package_of(packages, source)?;
    package.body_actions(body).into_iter()
        .flat_map(|action| (0..DIRECTION_COUNT).filter_map(move |direction| package.clip_extents(body, action, direction)))
        .flat_map(|clip| clip.frames.into_iter().map(|frame| u32::from(frame.width.max(frame.height))))
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_reads_a_filter_as_wgsl_lays_it_out() {
        const WGSL_MOBILE_FRAME_FILTER_SIZE: usize = 48;
        assert_eq!(std::mem::size_of::<MobileFrameFilter>(), WGSL_MOBILE_FRAME_FILTER_SIZE);
    }

    #[test]
    fn an_outline_pads_the_card_and_the_colour_packs_red_first() {
        let filter = MobileFrameFilter::of(FrameFilter { outline: 0.5, contour_color: [1, 2, 3], ..FrameFilter::OFF });
        assert_eq!(filter.padding_texels, OUTLINE_PADDING_TEXELS);
        assert_eq!(filter.contour_color.to_le_bytes(), [1, 2, 3, u8::MAX]);
        assert_eq!(MobileFrameFilter::of(FrameFilter::OFF).padding_texels, 0);
    }

    #[test]
    fn an_outline_follows_the_upscale_only_when_chosen() {
        const UPSCALED: u32 = 2;
        let outlined = MobileFrameFilter::of(FrameFilter { outline: 1.0, ..FrameFilter::OFF });
        for scale in [1, UPSCALED] {
            assert_eq!(outlined.for_frame(MobileOutlineWidth::OneTexel, scale).padding_texels, OUTLINE_PADDING_TEXELS);
        }
        assert_eq!(outlined.for_frame(MobileOutlineWidth::FollowsUpscale, 1).padding_texels, OUTLINE_PADDING_TEXELS);
        assert_eq!(outlined.for_frame(MobileOutlineWidth::FollowsUpscale, UPSCALED).padding_texels, UPSCALED);
        // Past the shader's widest outline the reach stays at it.
        assert_eq!(outlined.for_frame(MobileOutlineWidth::FollowsUpscale, 2 * UPSCALED).padding_texels,
            OUTLINE_MAX_REACH_TEXELS);
        // The rest of the filter stays; a frame without an outline is not padded.
        let wide = outlined.for_frame(MobileOutlineWidth::FollowsUpscale, UPSCALED);
        assert_eq!(MobileFrameFilter { padding_texels: OUTLINE_PADDING_TEXELS, ..wide }, outlined);
        let plain = MobileFrameFilter::of(FrameFilter::OFF);
        assert_eq!(plain.for_frame(MobileOutlineWidth::FollowsUpscale, UPSCALED), plain);
    }
}
