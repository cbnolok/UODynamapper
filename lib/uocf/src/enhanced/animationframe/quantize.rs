//! RGBA frames to palette indices: alpha to the format's 16 coverage levels, colours to a shared palette of at
//! most 256 `[b0, b1, b2, keep_colour]` entries (one palette per AMOU entry, shared by all its frames).
//!
//! - With a given palette (the original one, for byte-identical round trips), each coded pixel takes the entry
//!   with its exact RGB and keep-colour flag; a colour missing from the palette takes the nearest entry with the
//!   same flag (squared RGB distance, lowest index on ties), and is counted in [`IndexingReport::approximated`].
//! - Without one, the palette is every distinct (RGB, flag) when there are at most 256, else a deterministic
//!   weighted median cut per flag ([`median_cut`]); slots are shared between the two flags by pixel weight.
//!
//! Pure Rust, no dependency: the result depends only on the input pixels.

use std::collections::BTreeMap;

use color_eyre::eyre;

use super::encode::{coverage_of_alpha, IndexedFrame, COVERAGE_SOLID, COVERAGE_TRANSPARENT};
use super::{
    HUE_MASK_HUEABLE, MAX_PALETTE_COLOURS, PALETTE_ENTRY_SIZE, PALETTE_KEEP_COLOUR_BYTE, RGBA_CHANNELS, RGB_CHANNELS,
};

/// Keep-colour flag byte the shipped palettes use for colours that ignore the hue (the other value is 0).
pub const PALETTE_KEEP_COLOUR_FLAG: u8 = 1;
/// Flag byte of a colour that takes the hue.
pub const PALETTE_HUEABLE_FLAG: u8 = 0;

const RGB: usize = RGB_CHANNELS;
/// Alpha byte of an RGBA pixel.
const ALPHA: usize = RGB_CHANNELS;

/// A frame to encode: straight-alpha RGBA8888 of the frame box plus its anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaFrame {
    pub width: u16,
    pub height: u16,
    /// Anchor inside the image, `(-x0, -y0)` of the frame box.
    pub center_x: i16,
    pub center_y: i16,
    pub rgba: Vec<u8>,
    /// One byte per pixel, [`HUE_MASK_HUEABLE`] where the pixel takes the hue; any other value keeps its colour.
    /// `None`: every pixel takes the hue.
    pub hue_mask: Option<Vec<u8>>,
}

impl RgbaFrame {
    fn pixel_count(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    fn check(&self) -> eyre::Result<()> {
        let pixels = self.pixel_count();
        eyre::ensure!(
            self.rgba.len() == pixels * RGBA_CHANNELS,
            "a {}x{} frame needs {} RGBA bytes, found {}",
            self.width,
            self.height,
            pixels * RGBA_CHANNELS,
            self.rgba.len()
        );
        if let Some(mask) = &self.hue_mask {
            eyre::ensure!(mask.len() == pixels, "the hue mask has {} bytes for {pixels} pixels", mask.len());
        }
        Ok(())
    }

    /// Coverage and palette key of pixel `i`; `None` for a transparent pixel.
    fn pixel(&self, i: usize) -> Option<(u8, [u8; PALETTE_ENTRY_SIZE])> {
        let rgba = &self.rgba[i * RGBA_CHANNELS..(i + 1) * RGBA_CHANNELS];
        let coverage = coverage_of_alpha(rgba[ALPHA]);
        if coverage == COVERAGE_TRANSPARENT {
            return None;
        }
        let hueable = self.hue_mask.as_ref().is_none_or(|mask| mask[i] == HUE_MASK_HUEABLE);
        let flag = if hueable { PALETTE_HUEABLE_FLAG } else { PALETTE_KEEP_COLOUR_FLAG };
        Some((coverage, [rgba[0], rgba[1], rgba[2], flag]))
    }
}

/// What indexing did besides exact matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexingReport {
    /// Distinct (RGB, flag) colours of the coded pixels.
    pub distinct_colours: usize,
    /// Coded pixels whose colour is not in the palette and took the nearest entry.
    pub approximated: usize,
    /// Whether the palette was built by [`median_cut`] (more than 256 distinct colours).
    pub quantized: bool,
}

/// Indexes `frames` against `palette`, or against a palette built from them; returns the palette, the frames and
/// the report. Fails on a malformed frame, an empty or oversized given palette, or a colour whose flag has no
/// entry in the given palette.
pub fn index_frames(
    frames: &[RgbaFrame],
    palette: Option<&[[u8; PALETTE_ENTRY_SIZE]]>,
) -> eyre::Result<(Vec<[u8; PALETTE_ENTRY_SIZE]>, Vec<IndexedFrame>, IndexingReport)> {
    for frame in frames {
        frame.check()?;
    }
    let mut weights: BTreeMap<[u8; PALETTE_ENTRY_SIZE], u64> = BTreeMap::new();
    for frame in frames {
        for i in 0..frame.pixel_count() {
            if let Some((coverage, key)) = frame.pixel(i) {
                *weights.entry(key).or_default() += u64::from(coverage);
            }
        }
    }
    let mut report = IndexingReport { distinct_colours: weights.len(), ..IndexingReport::default() };

    let palette = match palette {
        Some(palette) => {
            eyre::ensure!(
                !palette.is_empty() && palette.len() <= MAX_PALETTE_COLOURS,
                "a palette has 1..={MAX_PALETTE_COLOURS} colours, found {}",
                palette.len()
            );
            palette.to_vec()
        }
        None if weights.len() <= MAX_PALETTE_COLOURS => weights.keys().copied().collect(),
        None => {
            report.quantized = true;
            build_quantized_palette(&weights)
        }
    };

    let mut lookup: BTreeMap<[u8; PALETTE_ENTRY_SIZE], u8> = BTreeMap::new();
    for (index, colour) in palette.iter().enumerate().rev() {
        lookup.insert(*colour, index as u8);
    }
    let mut mapped: BTreeMap<[u8; PALETTE_ENTRY_SIZE], (u8, bool)> = BTreeMap::new();
    for &key in weights.keys() {
        let entry = match lookup.get(&key) {
            Some(&index) => (index, false),
            None => (nearest(&palette, key)?, true),
        };
        mapped.insert(key, entry);
    }

    let mut indexed = Vec::with_capacity(frames.len());
    for frame in frames {
        let pixels = frame.pixel_count();
        let mut indices = vec![0u8; pixels];
        let mut coverage = vec![COVERAGE_TRANSPARENT; pixels];
        for i in 0..pixels {
            if let Some((pixel_coverage, key)) = frame.pixel(i) {
                let (index, approximated) = mapped[&key];
                indices[i] = index;
                coverage[i] = pixel_coverage;
                report.approximated += usize::from(approximated);
            }
        }
        indexed.push(IndexedFrame {
            width: frame.width,
            height: frame.height,
            center_x: frame.center_x,
            center_y: frame.center_y,
            indices,
            coverage,
        });
    }
    Ok((palette, indexed, report))
}

/// The palette entry nearest to `key` among those with its flag.
fn nearest(palette: &[[u8; PALETTE_ENTRY_SIZE]], key: [u8; PALETTE_ENTRY_SIZE]) -> eyre::Result<u8> {
    palette
        .iter()
        .enumerate()
        .filter(|(_, colour)| (colour[PALETTE_KEEP_COLOUR_BYTE] == 0) == (key[PALETTE_KEEP_COLOUR_BYTE] == 0))
        .min_by_key(|(_, colour)| distance(colour, &key))
        .map(|(index, _)| index as u8)
        .ok_or_else(|| {
            let kind = if key[PALETTE_KEEP_COLOUR_BYTE] == 0 { "hueable" } else { "keep-colour" };
            eyre::eyre!("the palette has no {kind} colour for #{:02X}{:02X}{:02X}", key[0], key[1], key[2])
        })
}

fn distance(a: &[u8; PALETTE_ENTRY_SIZE], b: &[u8; PALETTE_ENTRY_SIZE]) -> u32 {
    (0..RGB).map(|c| (i32::from(a[c]) - i32::from(b[c])).unsigned_abs().pow(2)).sum()
}

/// Splits the 256 slots between the hueable and the keep-colour colours by weight (at least one slot for a flag
/// that has colours, no more slots than it has colours), then median-cuts each group.
fn build_quantized_palette(weights: &BTreeMap<[u8; PALETTE_ENTRY_SIZE], u64>) -> Vec<[u8; PALETTE_ENTRY_SIZE]> {
    let group = |hueable: bool| -> Vec<([u8; RGB], u64)> {
        weights
            .iter()
            .filter(|(key, _)| (key[PALETTE_KEEP_COLOUR_BYTE] == 0) == hueable)
            .map(|(key, &weight)| ([key[0], key[1], key[2]], weight))
            .collect()
    };
    let (hueable, keep) = (group(true), group(false));
    let weight = |colours: &[([u8; RGB], u64)]| colours.iter().map(|colour| colour.1).sum::<u64>();
    let total = weight(&hueable) + weight(&keep);
    let slots = MAX_PALETTE_COLOURS as u64;
    let mut keep_slots = if keep.is_empty() {
        0
    } else {
        ((slots * weight(&keep) + total / 2) / total).clamp(1, slots - u64::from(!hueable.is_empty())) as usize
    };
    keep_slots = keep_slots.min(keep.len());
    let mut hueable_slots = (MAX_PALETTE_COLOURS - keep_slots).min(hueable.len());
    // Slots a group cannot use go to the other one.
    keep_slots = (MAX_PALETTE_COLOURS - hueable_slots).min(keep.len());
    hueable_slots = (MAX_PALETTE_COLOURS - keep_slots).min(hueable.len());

    let mut palette = Vec::with_capacity(MAX_PALETTE_COLOURS);
    for (colours, count, flag) in
        [(hueable, hueable_slots, PALETTE_HUEABLE_FLAG), (keep, keep_slots, PALETTE_KEEP_COLOUR_FLAG)]
    {
        palette.extend(median_cut(colours, count).into_iter().map(|[r, g, b]| [r, g, b, flag]));
    }
    palette
}

/// Deterministic weighted median cut: starting from one box holding every colour, repeatedly splits the box with
/// the largest weighted squared spread on its widest channel at the weighted median, until there are `count`
/// boxes or no box can be split; each box gives its weighted mean colour. The colours are distinct; the result has
/// distinct entries, sorted.
pub fn median_cut(colours: Vec<([u8; RGB], u64)>, count: usize) -> Vec<[u8; RGB]> {
    if colours.is_empty() || count == 0 {
        return Vec::new();
    }
    if colours.len() <= count {
        let mut exact: Vec<[u8; RGB]> = colours.into_iter().map(|colour| colour.0).collect();
        exact.sort_unstable();
        return exact;
    }
    let mut boxes = vec![colours];
    while boxes.len() < count {
        let Some((index, _)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, colours)| colours.len() > 1)
            .map(|(index, colours)| (index, spread(colours)))
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        else {
            break;
        };
        let mut colours = boxes.swap_remove(index);
        let channel = widest_channel(&colours);
        colours.sort_unstable_by_key(|(rgb, _)| (rgb[channel], *rgb));
        let half = colours.iter().map(|colour| u128::from(colour.1.max(1))).sum::<u128>().div_ceil(2);
        let mut acc = 0u128;
        let mut split = colours.len() - 1;
        for (i, colour) in colours.iter().enumerate() {
            acc += u128::from(colour.1.max(1));
            if acc >= half {
                split = (i + 1).min(colours.len() - 1);
                break;
            }
        }
        let upper = colours.split_off(split);
        boxes.push(colours);
        boxes.push(upper);
    }
    let mut palette: Vec<[u8; RGB]> = boxes.iter().map(|colours| mean(colours)).collect();
    palette.sort_unstable();
    palette.dedup();
    palette
}

fn widest_channel(colours: &[([u8; RGB], u64)]) -> usize {
    (0..RGB)
        .max_by_key(|&c| {
            let (low, high) = colours.iter().fold((u8::MAX, u8::MIN), |(low, high), colour| {
                (low.min(colour.0[c]), high.max(colour.0[c]))
            });
            // Ties go to the first channel.
            (high - low, std::cmp::Reverse(c))
        })
        .unwrap_or(0)
}

/// Weighted sum of squared distances to the box mean.
fn spread(colours: &[([u8; RGB], u64)]) -> u128 {
    let centre = mean(colours);
    colours.iter().map(|(rgb, weight)| u128::from((*weight).max(1)) * u128::from(distance3(rgb, &centre))).sum()
}

fn distance3(a: &[u8; RGB], b: &[u8; RGB]) -> u32 {
    (0..RGB).map(|c| (i32::from(a[c]) - i32::from(b[c])).unsigned_abs().pow(2)).sum()
}

fn mean(colours: &[([u8; RGB], u64)]) -> [u8; RGB] {
    let total: u128 = colours.iter().map(|colour| u128::from(colour.1.max(1))).sum();
    let mut out = [0u8; RGB];
    for (c, value) in out.iter_mut().enumerate() {
        let sum: u128 = colours.iter().map(|(rgb, weight)| u128::from(rgb[c]) * u128::from((*weight).max(1))).sum();
        *value = ((sum + total / 2) / total) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::animationframe::encode::alpha_of_coverage;

    fn rgba_frame(pixels: &[[u8; RGBA_CHANNELS]]) -> RgbaFrame {
        RgbaFrame {
            width: pixels.len() as u16,
            height: 1,
            center_x: 0,
            center_y: 0,
            rgba: pixels.iter().flatten().copied().collect(),
            hue_mask: None,
        }
    }

    #[test]
    fn few_colours_give_an_exact_sorted_palette_and_alpha_levels() {
        let frame = rgba_frame(&[[9, 9, 9, 255], [1, 2, 3, alpha_of_coverage(5)], [7, 7, 7, 0], [9, 9, 9, 255]]);
        let (palette, frames, report) = index_frames(&[frame], None).unwrap();
        assert_eq!(palette, [[1, 2, 3, 0], [9, 9, 9, 0]]);
        assert_eq!(frames[0].indices, [1, 0, 0, 1]);
        assert_eq!(frames[0].coverage, [COVERAGE_SOLID, 5, COVERAGE_TRANSPARENT, COVERAGE_SOLID]);
        assert_eq!(report, IndexingReport { distinct_colours: 2, approximated: 0, quantized: false });
    }

    #[test]
    fn a_given_palette_is_matched_exactly_then_by_nearest_colour_with_the_same_flag() {
        let mut frame = rgba_frame(&[[10, 10, 10, 255], [12, 10, 10, 255], [10, 10, 10, 255]]);
        frame.hue_mask = Some(vec![HUE_MASK_HUEABLE, HUE_MASK_HUEABLE, 0]);
        let palette = [[10, 10, 10, 1], [200, 0, 0, 0], [10, 10, 10, 0]];
        let (_, frames, report) = index_frames(&[frame.clone()], Some(&palette)).unwrap();
        assert_eq!(frames[0].indices, [2, 2, 0]);
        assert_eq!(report.approximated, 1);
        // No keep-colour entry for the third pixel.
        assert!(index_frames(&[frame], Some(&[[10, 10, 10, 0]])).is_err());
    }

    #[test]
    fn many_colours_are_quantized_to_256_deterministically() {
        let pixels: Vec<[u8; RGBA_CHANNELS]> =
            (0..1024u32).map(|i| [(i % 32 * 8) as u8, (i / 32 * 8) as u8, (i % 7 * 30) as u8, 255]).collect();
        let mut frame = rgba_frame(&pixels);
        frame.hue_mask = Some((0..pixels.len()).map(|i| if i % 5 == 0 { 0 } else { HUE_MASK_HUEABLE }).collect());
        let (palette, frames, report) = index_frames(&[frame.clone()], None).unwrap();
        assert!(report.quantized);
        assert!(palette.len() <= MAX_PALETTE_COLOURS && palette.len() > MAX_PALETTE_COLOURS / 2);
        assert!(palette.iter().any(|colour| colour[PALETTE_KEEP_COLOUR_BYTE] == PALETTE_KEEP_COLOUR_FLAG));
        for (i, &index) in frames[0].indices.iter().enumerate() {
            let flag = palette[usize::from(index)][PALETTE_KEEP_COLOUR_BYTE];
            assert_eq!(flag == PALETTE_KEEP_COLOUR_FLAG, i % 5 == 0);
        }
        let again = index_frames(&[frame], None).unwrap();
        assert_eq!((again.0, again.1), (palette, frames));
    }

    #[test]
    fn median_cut_keeps_distinct_colours_when_they_fit() {
        let colours = vec![([5, 5, 5], 1), ([1, 1, 1], 3)];
        assert_eq!(median_cut(colours, 4), [[1, 1, 1], [5, 5, 5]]);
        let two = median_cut(vec![([0, 0, 0], 1), ([2, 0, 0], 1), ([250, 0, 0], 1), ([252, 0, 0], 1)], 2);
        assert_eq!(two, [[1, 0, 0], [251, 0, 0]]);
    }
}
