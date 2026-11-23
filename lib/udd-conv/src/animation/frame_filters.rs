//! Pack-time cleanup filters for mobile animation frames, applied to each decoded frame before any upscale and the
//! atlas blit. `MobileFrameFilters.kdl` picks the filter per body ([`super::frame_filter_table`]). The study behind
//! them is `docs/agent_notes/ec_frame_quantization.md`.
//!
//! Both filters are a selective 3x3 median: an interior texel is replaced by the channel median of its 8 neighbours
//! when it is an isolated outlier, at least [`OUTLIER_MIN_DISTANCE`] from that median (RGB distance) and
//! [`OUTLIER_SPREAD_FACTOR`] times farther from it than the neighbours' own median distance. Busy texture, whose
//! neighbours spread as far as the texel, stays.
//!
//! - [`FrameFilter::Highlight`] tests only where at least [`HIGHLIGHT_MIN_CLIPPED`] of the window's 9 texels have a
//!   channel at [`CLIP_LEVEL`] or above. In an overexposed render's clipped highlights only the other channels carry
//!   the texture's shading, so its brightness steps show as pale and saturated hue specks; elsewhere the frame stays
//!   as it is.
//! - [`FrameFilter::Median`] tests every interior texel. It removes more specks, and softens fine texture (scales,
//!   stone) with them.
//!
//! The filters keep the outline: only a solid texel (alpha 255) whose 8 neighbours are solid too can change, and only
//! its RGB, so transparent texels, the anti-aliased edge and alpha stay as they are. A window holding texels of two
//! classes (the hue mask: hueable or keeping its colour) is left alone, so a texel never takes a colour from the other
//! class, and the mask itself is not touched.

use color_eyre::eyre;

/// A channel at or above this is clipped: the render's highlight saturated it.
pub const CLIP_LEVEL: u8 = 250;
/// The clipped texels a 3x3 window needs for [`FrameFilter::Highlight`] to test its centre.
pub const HIGHLIGHT_MIN_CLIPPED: usize = 3;
/// The least RGB distance from the neighbours' channel median at which a texel is an outlier.
pub const OUTLIER_MIN_DISTANCE: f32 = 12.0;
/// How many times farther from the neighbours' median than their own median distance to it an outlier lies.
pub const OUTLIER_SPREAD_FACTOR: f32 = 2.5;

const RGBA_CHANNELS: usize = 4;
const RGB_CHANNELS: usize = 3;
const ALPHA_CHANNEL: usize = 3;
/// Alpha of a solid texel: full coverage, not the anti-aliased edge.
const SOLID_ALPHA: u8 = u8::MAX;
/// The 8 neighbours of a 3x3 window, by (dx, dy).
const NEIGHBOUR_OFFSETS: [(isize, isize); NEIGHBOUR_COUNT] =
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
const NEIGHBOUR_COUNT: usize = 8;

/// The cleanup a body's frames get at pack time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FrameFilter {
    /// The frames as the client stores them.
    #[default]
    None,
    /// The selective median in clipped highlights only.
    Highlight,
    /// The selective median everywhere.
    Median,
}

impl FrameFilter {
    pub const ALL: [FrameFilter; 3] = [FrameFilter::None, FrameFilter::Highlight, FrameFilter::Median];

    /// The name `MobileFrameFilters.kdl` uses.
    pub fn name(self) -> &'static str {
        match self {
            FrameFilter::None => "none",
            FrameFilter::Highlight => "highlight",
            FrameFilter::Median => "median",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|filter| filter.name() == name)
    }

    /// The clipped window texels the test needs; 0 tests every interior texel. `None` for no filter.
    fn min_clipped(self) -> Option<usize> {
        match self {
            FrameFilter::None => None,
            FrameFilter::Highlight => Some(HIGHLIGHT_MIN_CLIPPED),
            FrameFilter::Median => Some(0),
        }
    }
}

/// Filters a `width` x `height` straight-alpha RGBA8888 frame in place. `classes` holds one byte per texel (the hue
/// mask) or is empty when every texel is of one class. Returns how many texels changed.
pub fn apply_frame_filter(
    filter: FrameFilter,
    width: usize,
    height: usize,
    rgba: &mut [u8],
    classes: &[u8],
) -> eyre::Result<usize> {
    let texel_count = width * height;
    eyre::ensure!(
        rgba.len() == texel_count * RGBA_CHANNELS,
        "frame filter: {width}x{height} frame has {} RGBA bytes, expected {}",
        rgba.len(),
        texel_count * RGBA_CHANNELS
    );
    eyre::ensure!(
        classes.is_empty() || classes.len() == texel_count,
        "frame filter: {width}x{height} frame has {} hue mask bytes, expected {texel_count}",
        classes.len()
    );
    let Some(min_clipped) = filter.min_clipped() else {
        return Ok(0);
    };
    let source = rgba.to_vec();
    let solid = |i: usize| source[i * RGBA_CHANNELS + ALPHA_CHANNEL] == SOLID_ALPHA;
    let clipped = |i: usize| source[i * RGBA_CHANNELS..i * RGBA_CHANNELS + RGB_CHANNELS].iter().any(|&v| v >= CLIP_LEVEL);
    let mut changed = 0;
    // Interior texels only: every neighbour inside the frame and solid.
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let centre = y * width + x;
            let neighbours = NEIGHBOUR_OFFSETS
                .map(|(dx, dy)| (y as isize + dy) as usize * width + (x as isize + dx) as usize);
            if !solid(centre) || !neighbours.iter().all(|&j| solid(j)) {
                continue;
            }
            if !classes.is_empty() && neighbours.iter().any(|&j| classes[j] != classes[centre]) {
                continue;
            }
            if min_clipped > 0
                && usize::from(clipped(centre)) + neighbours.iter().filter(|&&j| clipped(j)).count() < min_clipped
            {
                continue;
            }
            let Some(median) = outlier_median(&source, centre, &neighbours) else {
                continue;
            };
            let at = centre * RGBA_CHANNELS;
            let replacement = median.map(|value| value.round().clamp(0.0, f32::from(u8::MAX)) as u8);
            if rgba[at..at + RGB_CHANNELS] != replacement {
                rgba[at..at + RGB_CHANNELS].copy_from_slice(&replacement);
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// The channel median of the neighbours when the centre texel is an isolated outlier from it.
fn outlier_median(rgba: &[u8], centre: usize, neighbours: &[usize; NEIGHBOUR_COUNT]) -> Option<[f32; RGB_CHANNELS]> {
    let colour = |i: usize| {
        let at = i * RGBA_CHANNELS;
        [f32::from(rgba[at]), f32::from(rgba[at + 1]), f32::from(rgba[at + 2])]
    };
    let colours = neighbours.map(colour);
    let median = std::array::from_fn(|channel| median_of_8(colours.map(|c| c[channel])));
    let distance = rgb_distance(&colour(centre), &median);
    if distance < OUTLIER_MIN_DISTANCE {
        return None;
    }
    let spread = median_of_8(colours.map(|c| rgb_distance(&c, &median)));
    (distance >= OUTLIER_SPREAD_FACTOR * spread).then_some(median)
}

/// The median of 8 values: the mean of the middle two.
fn median_of_8(mut values: [f32; NEIGHBOUR_COUNT]) -> f32 {
    values.sort_by(f32::total_cmp);
    let middle = NEIGHBOUR_COUNT / 2;
    0.5 * (values[middle - 1] + values[middle])
}

fn rgb_distance(a: &[f32; RGB_CHANNELS], b: &[f32; RGB_CHANNELS]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 5;
    const CENTRE: usize = 2 * SIZE + 2;
    const FIELD: [u8; 3] = [254, 180, 130];
    const SPECK: [u8; 3] = [254, 120, 80];
    const DULL_FIELD: [u8; 3] = [200, 180, 130];
    const DULL_SPECK: [u8; 3] = [200, 120, 80];
    const HUEABLE: u8 = 1;

    /// A solid `SIZE` x `SIZE` frame of `field` with `speck` at its centre.
    fn frame(field: [u8; 3], speck: [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for i in 0..SIZE * SIZE {
            let colour = if i == CENTRE { speck } else { field };
            rgba.extend_from_slice(&colour);
            rgba.push(SOLID_ALPHA);
        }
        rgba
    }

    fn rgb(rgba: &[u8], i: usize) -> [u8; 3] {
        rgba[i * RGBA_CHANNELS..i * RGBA_CHANNELS + RGB_CHANNELS].try_into().unwrap()
    }

    fn filtered(filter: FrameFilter, mut rgba: Vec<u8>, classes: &[u8]) -> (Vec<u8>, usize) {
        let changed = apply_frame_filter(filter, SIZE, SIZE, &mut rgba, classes).unwrap();
        (rgba, changed)
    }

    #[test]
    fn highlight_removes_a_speck_in_a_clipped_highlight() {
        let (out, changed) = filtered(FrameFilter::Highlight, frame(FIELD, SPECK), &[]);
        assert_eq!(changed, 1);
        assert_eq!(rgb(&out, CENTRE), FIELD);
        assert_eq!(out, frame(FIELD, FIELD));
    }

    #[test]
    fn highlight_leaves_unclipped_areas_and_median_does_not() {
        let source = frame(DULL_FIELD, DULL_SPECK);
        assert_eq!(filtered(FrameFilter::Highlight, source.clone(), &[]), (source.clone(), 0));
        let (out, changed) = filtered(FrameFilter::Median, source, &[]);
        assert_eq!(changed, 1);
        assert_eq!(rgb(&out, CENTRE), DULL_FIELD);
    }

    #[test]
    fn none_changes_nothing() {
        let source = frame(FIELD, SPECK);
        assert_eq!(filtered(FrameFilter::None, source.clone(), &[]), (source, 0));
    }

    #[test]
    fn small_differences_and_busy_texture_stay() {
        let near = [FIELD[0], FIELD[1] - 8, FIELD[2] - 6];
        let source = frame(FIELD, near);
        assert_eq!(filtered(FrameFilter::Median, source.clone(), &[]), (source, 0));

        // A checkerboard: every texel's neighbours spread as far as the centre does. Texture, not a speck.
        let mut busy = frame(DULL_FIELD, DULL_SPECK);
        for i in (0..SIZE * SIZE).filter(|&i| i != CENTRE && (i % SIZE + i / SIZE) % 2 == 1) {
            busy[i * RGBA_CHANNELS + 1] = DULL_SPECK[1];
        }
        assert_eq!(filtered(FrameFilter::Median, busy.clone(), &[]), (busy, 0));
    }

    #[test]
    fn the_edge_and_transparent_texels_stay() {
        // A speck next to an anti-aliased texel, and one on the frame's border.
        let mut source = frame(DULL_FIELD, DULL_SPECK);
        source[(CENTRE + 1) * RGBA_CHANNELS + ALPHA_CHANNEL] = SOLID_ALPHA / 2;
        source[..RGB_CHANNELS].copy_from_slice(&DULL_SPECK);
        assert_eq!(filtered(FrameFilter::Median, source.clone(), &[]), (source, 0));

        let mut transparent = frame(DULL_FIELD, DULL_SPECK);
        transparent[CENTRE * RGBA_CHANNELS + ALPHA_CHANNEL] = 0;
        assert_eq!(filtered(FrameFilter::Median, transparent.clone(), &[]), (transparent, 0));
    }

    #[test]
    fn a_window_mixing_hue_classes_stays_and_the_classes_do_not_change() {
        let source = frame(DULL_FIELD, DULL_SPECK);
        let mut classes = vec![0u8; SIZE * SIZE];
        classes[CENTRE - 1] = HUEABLE;
        assert_eq!(filtered(FrameFilter::Median, source.clone(), &classes), (source.clone(), 0));

        let all_hueable = vec![HUEABLE; SIZE * SIZE];
        let (out, changed) = filtered(FrameFilter::Median, source, &all_hueable);
        assert_eq!((rgb(&out, CENTRE), changed), (DULL_FIELD, 1));
        assert_eq!(all_hueable, vec![HUEABLE; SIZE * SIZE]);
    }

    #[test]
    fn mismatched_buffers_are_errors() {
        let mut rgba = frame(FIELD, SPECK);
        assert!(apply_frame_filter(FrameFilter::Median, SIZE, SIZE + 1, &mut rgba, &[]).is_err());
        assert!(apply_frame_filter(FrameFilter::Median, SIZE, SIZE, &mut rgba, &[0; 3]).is_err());
    }

    #[test]
    fn names_round_trip() {
        for filter in FrameFilter::ALL {
            assert_eq!(FrameFilter::from_name(filter.name()), Some(filter));
        }
        assert_eq!(FrameFilter::from_name("blur"), None);
    }
}
