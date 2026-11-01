//! Classic mobile animation frames: the frame codec shared by `anim*.mul` ([`mul`]) and the Classic Client's
//! `AnimationFrame*.uop` entries `build/animationlegacyframe/%06d/%02d.bin` ([`uop`]), plus the PNG folder both
//! export to and import from ([`folder`]).
//!
//! The rules come from the 7.0.103.0 client (`UOClassicAnim::DrawFrameRle`, `GetMulFrame`, `GetUopFrame`,
//! `FindFrameEntry`; `scripts/dev-helpers/ec-research/ghidra-classic-frames/`) and from the shipped files, which
//! this writer reproduces byte for byte (see `docs/agent_notes/cc_ec_shared_uop_formats.md`, Classic frames):
//!
//! - a palette of 256 RGB555 words; a frame's pixels are indices into it. The client draws `palette[index]` for
//!   every pixel of a run (no transparent index or colour), so transparency is only "not in a run";
//! - a frame: `i16` anchor x, `i16` anchor y, `u16` width, `u16` height, then runs. A run is a `u32` header (bits
//!   0..11 the length, 12..21 the signed y offset, 22..31 the signed x offset, both relative to the anchor: pixel
//!   `(x, y)` of the image has offsets `(x - anchor x, y - anchor y - height)`), then that many indices; the `u32`
//!   [`RLE_END_MARKER`] ends the frame;
//! - the shipped encoder (every shipped frame): runs top to bottom (the client stops at the first run below its
//!   clip box), left to right, each a maximal span of opaque pixels of one row (split at [`MAX_RUN_LENGTH`]),
//!   nothing after the marker. No shipped pixel uses index 0; some use a colour of 0 (the 7.0.103
//!   `AnimationFrame2.uop`), which the client draws black.
//!
//! From RGBA ([`index_rgba_frames`]), as the gump writer (`gump_write`): alpha 0 is transparent, 255 opaque, other
//! alphas are refused; channels round to the nearest 5-bit value. With a given palette each colour takes its lowest
//! index (index 0 last; bit 15 ignored), so black takes a word of 0 as in the shipped frames; without one, opaque
//! black is stored as [`OPAQUE_BLACK`] (the near black the shipped frames use most), since uocf's decoders (and
//! ClassicUO) draw a colour of 0 transparent, and the palette is every distinct
//! colour when there are at most [`USABLE_PALETTE_COLOURS`], else a deterministic median cut
//! ([`crate::enhanced::animationframe::quantize::median_cut`]); index 0 stays unused, as in the shipped palettes.

crate::eyre_imports!();

use std::collections::BTreeMap;

use crate::classic::anim::AnimFrame;
use crate::enhanced::animationframe::quantize::median_cut;

pub mod folder;
pub mod mul;
pub mod uop;

/// Colours in a frame palette.
pub const PALETTE_COLOURS: usize = 256;
/// A palette: RGB555 words as stored (bit 15 kept; the decoder ignores it).
pub type ClassicPalette = [u16; PALETTE_COLOURS];
/// Bytes of a stored palette.
pub const PALETTE_BYTES: usize = PALETTE_COLOURS * U16_BYTES;
/// Bytes of the frame header (anchor x, anchor y, width, height).
pub const FRAME_HEADER_BYTES: usize = 4 * U16_BYTES;
/// Ends a frame's runs.
pub const RLE_END_MARKER: u32 = 0x7FFF_7FFF;
/// Most pixels in one run (12 bits).
pub const MAX_RUN_LENGTH: usize = RUN_LENGTH_MASK as usize;
/// Range of a run's x and y offsets (signed 10 bits).
pub const MIN_RUN_OFFSET: i32 = -(1 << (RUN_OFFSET_BITS - 1));
pub const MAX_RUN_OFFSET: i32 = (1 << (RUN_OFFSET_BITS - 1)) - 1;
/// The index new palettes leave unused (no shipped pixel uses it).
pub const UNUSED_PALETTE_INDEX: u8 = 0;
/// Colours a new palette holds: every index but [`UNUSED_PALETTE_INDEX`].
pub const USABLE_PALETTE_COLOURS: usize = PALETTE_COLOURS - 1;
/// What opaque black is stored as (colour 0 decodes as transparent in uocf and ClassicUO).
pub const OPAQUE_BLACK: u16 = 0x0001;
/// The colour bits of a palette word; bit 15 is not a colour.
pub const RGB555_COLOUR_MASK: u16 = 0x7FFF;

const U16_BYTES: usize = size_of::<u16>();
const U32_BYTES: usize = size_of::<u32>();
const RUN_HEADER_BYTES: usize = U32_BYTES;
const RUN_LENGTH_MASK: u32 = 0xFFF;
const RUN_OFFSET_BITS: u32 = 10;
const RUN_OFFSET_MASK: u32 = (1 << RUN_OFFSET_BITS) - 1;
const RUN_Y_SHIFT: u32 = 12;
const RUN_X_SHIFT: u32 = RUN_Y_SHIFT + RUN_OFFSET_BITS;
/// Shift that sign-extends a 10-bit field placed in the top bits of a `u32`.
const RUN_OFFSET_SIGN_SHIFT: u32 = u32::BITS - RUN_OFFSET_BITS;
const CHANNEL_BITS: u32 = 5;
const CHANNEL_MAX: u16 = (1 << CHANNEL_BITS) - 1;
/// An 8-bit channel per 5-bit step, as the decoder expands them.
const CHANNEL_STEP: u16 = 1 << (u8::BITS - CHANNEL_BITS);
const RED_SHIFT: u32 = 2 * CHANNEL_BITS;
const GREEN_SHIFT: u32 = CHANNEL_BITS;
const RGBA_BYTES: usize = 4;
const ALPHA_BYTE: usize = 3;
const OPAQUE_ALPHA: u8 = u8::MAX;
const TRANSPARENT_ALPHA: u8 = 0;

/// A frame as palette indices: `None` for a pixel no run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicFrame {
    pub center_x: i16,
    pub center_y: i16,
    pub width: u16,
    pub height: u16,
    /// `width * height` pixels, row-major.
    pub pixels: Vec<Option<u8>>,
}

impl ClassicFrame {
    /// A frame without pixels (width and height 0).
    pub fn empty(center_x: i16, center_y: i16) -> Self {
        Self { center_x, center_y, width: 0, height: 0, pixels: Vec::new() }
    }

    /// Width or height 0: the client draws nothing (the UOP path returns no frame).
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Reads a frame from its header. Returns the frame and the bytes read: the header alone for an empty frame
    /// (the shipped UOP empty frames have nothing after it), else up to and including [`RLE_END_MARKER`]. Fails on
    /// a run outside the frame box (the client would draw it outside) or a stream without the marker.
    pub fn decode(data: &[u8]) -> eyre::Result<(Self, usize)> {
        eyre::ensure!(data.len() >= FRAME_HEADER_BYTES, "a frame header needs {FRAME_HEADER_BYTES} bytes, found {}", data.len());
        let read_i16 = |offset: usize| i16::from_le_bytes([data[offset], data[offset + 1]]);
        let read_u16 = |offset: usize| u16::from_le_bytes([data[offset], data[offset + 1]]);
        let (center_x, center_y) = (read_i16(0), read_i16(U16_BYTES));
        let (width, height) = (read_u16(2 * U16_BYTES), read_u16(3 * U16_BYTES));
        if width == 0 || height == 0 {
            return Ok((Self { center_x, center_y, width, height, pixels: Vec::new() }, FRAME_HEADER_BYTES));
        }
        let (w, h) = (i32::from(width), i32::from(height));
        let mut pixels = vec![None; usize::from(width) * usize::from(height)];
        let mut pos = FRAME_HEADER_BYTES;
        loop {
            let header = data
                .get(pos..pos + RUN_HEADER_BYTES)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                .ok_or_else(|| eyre!("the runs end at byte {pos} without the end marker"))?;
            pos += RUN_HEADER_BYTES;
            if header == RLE_END_MARKER {
                break;
            }
            let length = (header & RUN_LENGTH_MASK) as usize;
            let (x_offset, y_offset) = run_offsets(header);
            let x = x_offset + i32::from(center_x);
            let y = y_offset + i32::from(center_y) + h;
            let indices = data
                .get(pos..pos + length)
                .ok_or_else(|| eyre!("a run of {length} at byte {pos} passes the end of the data"))?;
            pos += length;
            eyre::ensure!(
                (0..h).contains(&y) && x >= 0 && x + length as i32 <= w,
                "a run of {length} at ({x}, {y}) is outside the {width}x{height} frame"
            );
            let start = y as usize * usize::from(width) + x as usize;
            for (pixel, &index) in pixels[start..start + length].iter_mut().zip(indices) {
                *pixel = Some(index);
            }
        }
        Ok((Self { center_x, center_y, width, height, pixels }, pos))
    }

    /// Appends the frame as the shipped encoder writes it: the header, then (unless empty) the runs and
    /// [`RLE_END_MARKER`]. Fails on a pixel count that does not match the size, or a run offset outside
    /// [`MIN_RUN_OFFSET`]..=[`MAX_RUN_OFFSET`].
    pub fn encode(&self, out: &mut Vec<u8>) -> eyre::Result<()> {
        out.extend_from_slice(&self.center_x.to_le_bytes());
        out.extend_from_slice(&self.center_y.to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        if self.is_empty() {
            eyre::ensure!(self.pixels.is_empty(), "an empty frame has {} pixels", self.pixels.len());
            return Ok(());
        }
        self.encode_runs(out)
    }

    /// Appends the runs and the end marker.
    pub fn encode_runs(&self, out: &mut Vec<u8>) -> eyre::Result<()> {
        let width = usize::from(self.width);
        let pixel_count = width * usize::from(self.height);
        eyre::ensure!(
            self.pixels.len() == pixel_count,
            "a {}x{} frame needs {pixel_count} pixels, found {}",
            self.width,
            self.height,
            self.pixels.len()
        );
        for (y, row) in self.pixels.chunks_exact(width.max(1)).enumerate() {
            let mut x = 0usize;
            while x < width {
                if row[x].is_none() {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < width && row[x].is_some() && x - start < MAX_RUN_LENGTH {
                    x += 1;
                }
                let x_offset = start as i32 - i32::from(self.center_x);
                let y_offset = y as i32 - i32::from(self.center_y) - i32::from(self.height);
                let range = MIN_RUN_OFFSET..=MAX_RUN_OFFSET;
                eyre::ensure!(
                    range.contains(&x_offset) && range.contains(&y_offset),
                    "pixel ({start}, {y}) is ({x_offset}, {y_offset}) from the anchor ({}, {}); a run offset is \
                     {MIN_RUN_OFFSET}..={MAX_RUN_OFFSET}",
                    self.center_x,
                    self.center_y
                );
                let header = (x - start) as u32
                    | (y_offset as u32 & RUN_OFFSET_MASK) << RUN_Y_SHIFT
                    | (x_offset as u32 & RUN_OFFSET_MASK) << RUN_X_SHIFT;
                out.extend_from_slice(&header.to_le_bytes());
                out.extend(row[start..x].iter().map(|pixel| pixel.unwrap_or(UNUSED_PALETTE_INDEX)));
            }
        }
        out.extend_from_slice(&RLE_END_MARKER.to_le_bytes());
        Ok(())
    }

    /// The frame as RGBA, as the client draws it: every pixel of a run opaque, a colour of 0 black. (uocf's
    /// decoders, [`crate::classic::anim`], make a colour of 0 transparent, as ClassicUO.)
    pub fn to_rgba(&self, palette: &ClassicPalette) -> AnimFrame {
        let data = self
            .pixels
            .iter()
            .flat_map(|pixel| match pixel {
                Some(index) => rgba_of_word(palette[usize::from(*index)]),
                None => [0; RGBA_BYTES],
            })
            .collect();
        let (width, height) = if self.is_empty() { (0, 0) } else { (self.width, self.height) };
        AnimFrame { width, height, center_x: self.center_x, center_y: self.center_y, data }
    }

    /// The palette indices the pixels use.
    pub fn used_indices(&self) -> impl Iterator<Item = u8> + '_ {
        self.pixels.iter().flatten().copied()
    }
}

/// A palette word as opaque RGBA (bit 15 ignored), each channel expanded as uocf's decoders do.
fn rgba_of_word(word: u16) -> [u8; RGBA_BYTES] {
    let channel = |shift: u32| (((word >> shift) & CHANNEL_MAX) << (u8::BITS - CHANNEL_BITS)) as u8;
    [channel(RED_SHIFT), channel(GREEN_SHIFT), channel(0), OPAQUE_ALPHA]
}

/// The x and y offsets of a run header (signed 10-bit fields).
fn run_offsets(header: u32) -> (i32, i32) {
    let x = (header as i32) >> RUN_X_SHIFT;
    let y = ((header << (RUN_OFFSET_SIGN_SHIFT - RUN_Y_SHIFT)) as i32) >> RUN_OFFSET_SIGN_SHIFT;
    (x, y)
}

/// Reads a stored palette.
pub fn read_palette(data: &[u8]) -> eyre::Result<ClassicPalette> {
    let bytes = data.get(..PALETTE_BYTES).ok_or_else(|| eyre!("a palette needs {PALETTE_BYTES} bytes, found {}", data.len()))?;
    let mut palette = [0u16; PALETTE_COLOURS];
    for (word, pair) in palette.iter_mut().zip(bytes.chunks_exact(U16_BYTES)) {
        *word = u16::from_le_bytes([pair[0], pair[1]]);
    }
    Ok(palette)
}

/// Appends a palette as stored.
pub fn write_palette(palette: &ClassicPalette, out: &mut Vec<u8>) {
    for word in palette {
        out.extend_from_slice(&word.to_le_bytes());
    }
}

/// A frame to encode: straight-alpha RGBA8888 plus its anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicRgbaFrame {
    pub center_x: i16,
    pub center_y: i16,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// What [`index_rgba_frames`] did besides exact matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColourReport {
    /// Distinct RGB555 colours of the opaque pixels.
    pub distinct_colours: usize,
    /// More than [`USABLE_PALETTE_COLOURS`] colours and no palette: median cut.
    pub quantized: bool,
    /// Pixels whose colour was missing from the palette and took the nearest entry.
    pub approximated: usize,
    /// Opaque pixels whose channels were not on the 5-bit grid, rounded to it.
    pub rounded: usize,
    /// Opaque black pixels, stored as [`OPAQUE_BLACK`].
    pub black: usize,
}

/// The RGB555 colour of an opaque pixel (0 for black), `None` for a transparent one; fails on a partial alpha.
fn rgb555_of(pixel: &[u8], report: &mut ColourReport) -> eyre::Result<Option<u16>> {
    match pixel[ALPHA_BYTE] {
        TRANSPARENT_ALPHA => return Ok(None),
        OPAQUE_ALPHA => {}
        alpha => eyre::bail!("alpha {alpha}: a Classic frame pixel is opaque (255) or transparent (0)"),
    }
    let channel = |value: u8| ((u16::from(value) + CHANNEL_STEP / 2) / CHANNEL_STEP).min(CHANNEL_MAX);
    if pixel[..ALPHA_BYTE].iter().any(|&value| u16::from(value) % CHANNEL_STEP != 0) {
        report.rounded += 1;
    }
    Ok(Some(channel(pixel[0]) << RED_SHIFT | channel(pixel[1]) << GREEN_SHIFT | channel(pixel[2])))
}

fn channels(colour: u16) -> [u8; 3] {
    let mask = CHANNEL_MAX;
    [((colour >> RED_SHIFT) & mask) as u8, ((colour >> GREEN_SHIFT) & mask) as u8, (colour & mask) as u8]
}

fn colour_of(channels: [u8; 3]) -> u16 {
    u16::from(channels[0]) << RED_SHIFT | u16::from(channels[1]) << GREEN_SHIFT | u16::from(channels[2])
}

fn distance(a: u16, b: u16) -> u32 {
    let (a, b) = (channels(a), channels(b));
    a.iter().zip(&b).map(|(&x, &y)| (i32::from(x) - i32::from(y)).pow(2) as u32).sum()
}

/// Indexes RGBA frames against `palette`, or against a palette built from them (one palette for all the frames:
/// a UOP entry or a MUL direction). Fails on a malformed frame or a partial alpha.
pub fn index_rgba_frames(
    frames: &[ClassicRgbaFrame],
    palette: Option<&ClassicPalette>,
) -> eyre::Result<(ClassicPalette, Vec<ClassicFrame>, ColourReport)> {
    let mut report = ColourReport::default();
    let mut colours: Vec<Vec<Option<u16>>> = Vec::with_capacity(frames.len());
    let mut weights: BTreeMap<u16, u64> = BTreeMap::new();
    for (number, frame) in frames.iter().enumerate() {
        let pixel_count = usize::from(frame.width) * usize::from(frame.height);
        eyre::ensure!(
            frame.rgba.len() == pixel_count * RGBA_BYTES,
            "frame {number}: a {}x{} frame needs {} RGBA bytes, found {}",
            frame.width,
            frame.height,
            pixel_count * RGBA_BYTES,
            frame.rgba.len()
        );
        let mut pixels = frame
            .rgba
            .chunks_exact(RGBA_BYTES)
            .map(|pixel| rgb555_of(pixel, &mut report))
            .collect::<eyre::Result<Vec<_>>>()
            .wrap_err_with(|| format!("frame {number}"))?;
        for colour in pixels.iter_mut().flatten() {
            // A new palette stores black as OPAQUE_BLACK.
            if palette.is_none() && *colour == 0 {
                report.black += 1;
                *colour = OPAQUE_BLACK;
            }
            *weights.entry(*colour).or_default() += 1;
        }
        colours.push(pixels);
    }
    report.distinct_colours = weights.len();

    let given = palette.is_some();
    let palette = match palette {
        Some(palette) => *palette,
        None => {
            let mut built = [0u16; PALETTE_COLOURS];
            let chosen: Vec<u16> = if weights.len() <= USABLE_PALETTE_COLOURS {
                weights.keys().copied().collect()
            } else {
                report.quantized = true;
                let boxes = weights.iter().map(|(&colour, &weight)| (channels(colour), weight)).collect();
                let mut cut: Vec<u16> = median_cut(boxes, USABLE_PALETTE_COLOURS)
                    .into_iter()
                    .map(|rgb| match colour_of(rgb) {
                        0 => OPAQUE_BLACK,
                        colour => colour,
                    })
                    .collect();
                cut.sort_unstable();
                cut.dedup();
                cut
            };
            for (slot, colour) in built.iter_mut().skip(usize::from(UNUSED_PALETTE_INDEX) + 1).zip(chosen) {
                *slot = colour;
            }
            built
        }
    };

    // Lowest index per colour (bit 15 ignored), index 0 last. Black takes a word of 0 (or 0x8000) of a given
    // palette, except at index 0, as the shipped frames that draw black with it; such words are never the nearest
    // colour, and a new palette's empty slots are none of these.
    let mut lookup: BTreeMap<u16, u8> = BTreeMap::new();
    let order = (1..PALETTE_COLOURS).chain(std::iter::once(usize::from(UNUSED_PALETTE_INDEX)));
    for index in order {
        let colour = palette[index] & RGB555_COLOUR_MASK;
        if colour != 0 || given && index != usize::from(UNUSED_PALETTE_INDEX) {
            lookup.entry(colour).or_insert(index as u8);
        }
    }
    let mut nearest: BTreeMap<u16, u8> = BTreeMap::new();
    let mut index_of = |colour: u16, report: &mut ColourReport| -> u8 {
        let colour = if colour == 0 && !lookup.contains_key(&0) {
            report.black += 1;
            OPAQUE_BLACK
        } else {
            colour
        };
        if let Some(&index) = lookup.get(&colour) {
            return index;
        }
        report.approximated += 1;
        *nearest.entry(colour).or_insert_with(|| {
            lookup
                .iter()
                .filter(|&(&candidate, _)| candidate != 0)
                .min_by_key(|&(&candidate, &index)| (distance(candidate, colour), index))
                .map(|(_, &index)| index)
                .unwrap_or(UNUSED_PALETTE_INDEX)
        })
    };

    let mut indexed = Vec::with_capacity(frames.len());
    for (frame, pixels) in frames.iter().zip(colours) {
        let pixels = pixels.into_iter().map(|colour| colour.map(|colour| index_of(colour, &mut report))).collect();
        indexed.push(ClassicFrame {
            center_x: frame.center_x,
            center_y: frame.center_y,
            width: frame.width,
            height: frame.height,
            pixels,
        });
    }
    Ok((palette, indexed, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> ClassicFrame {
        // 4x2: row 0 ".AB.", row 1 "C..D".
        ClassicFrame {
            center_x: 2,
            center_y: -3,
            width: 4,
            height: 2,
            pixels: vec![None, Some(1), Some(2), None, Some(3), None, None, Some(4)],
        }
    }

    #[test]
    fn frame_round_trips_through_runs() {
        let frame = frame();
        let mut bytes = Vec::new();
        frame.encode(&mut bytes).unwrap();
        // Header, three runs of 2, 1, 1 pixels, the marker.
        assert_eq!(bytes.len(), FRAME_HEADER_BYTES + 3 * RUN_HEADER_BYTES + 4 + U32_BYTES);
        let (decoded, read) = ClassicFrame::decode(&bytes).unwrap();
        assert_eq!(read, bytes.len());
        assert_eq!(decoded, frame);
        // Offsets relative to the anchor: pixel (1, 0) is (1 - 2, 0 + 3 - 2).
        let first = u32::from_le_bytes(bytes[FRAME_HEADER_BYTES..FRAME_HEADER_BYTES + 4].try_into().unwrap());
        assert_eq!(first & RUN_LENGTH_MASK, 2);
        assert_eq!(run_offsets(first), (-1, 1));
    }

    #[test]
    fn far_pixels_fail_and_wide_rows_fit() {
        // The widest row whose start fits the offsets: 1024 pixels anchored at its middle, one run.
        let width = (MAX_RUN_OFFSET - MIN_RUN_OFFSET + 1) as u16;
        let row = vec![Some(1); usize::from(width)];
        let far = ClassicFrame { center_x: 0, center_y: -1, width, height: 1, pixels: row.clone() };
        let mut bytes = Vec::new();
        assert!(far.encode(&mut bytes).is_ok(), "one run starting at offset 0");
        let mut gap = row;
        gap[usize::from(width) - 2] = None;
        let far = ClassicFrame { pixels: gap, ..far };
        bytes.clear();
        assert!(far.encode(&mut bytes).is_err(), "the last run starts past {MAX_RUN_OFFSET}");
        let frame = ClassicFrame { center_x: -MIN_RUN_OFFSET as i16, ..far };
        bytes.clear();
        frame.encode(&mut bytes).unwrap();
        let (decoded, _) = ClassicFrame::decode(&bytes).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn empty_frames_are_a_header() {
        let mut bytes = Vec::new();
        ClassicFrame::empty(0, 0).encode(&mut bytes).unwrap();
        assert_eq!(bytes, [0; FRAME_HEADER_BYTES]);
        assert_eq!(ClassicFrame::decode(&bytes).unwrap(), (ClassicFrame::empty(0, 0), FRAME_HEADER_BYTES));
    }

    #[test]
    fn rgba_round_trips_with_and_without_the_palette() {
        let mut palette = [0u16; PALETTE_COLOURS];
        palette[0] = 0xdc18;
        palette[1] = 0x7C00;
        palette[2] = 0x03E0;
        palette[3] = 0x001F;
        palette[4] = 0x03E0; // a duplicate: the lowest index wins
        let frame = ClassicFrame { pixels: vec![None, Some(1), Some(2), None, Some(3), None, None, Some(2)], ..frame() };
        let rgba = frame.to_rgba(&palette);
        let input = ClassicRgbaFrame { center_x: 2, center_y: -3, width: 4, height: 2, rgba: rgba.data.clone() };
        let (same, frames, report) = index_rgba_frames(std::slice::from_ref(&input), Some(&palette)).unwrap();
        assert_eq!((same, &frames[0]), (palette, &frame));
        assert_eq!(report, ColourReport { distinct_colours: 3, ..ColourReport::default() });

        let (built, frames, _) = index_rgba_frames(std::slice::from_ref(&input), None).unwrap();
        assert_eq!(&built[..4], &[0, 0x001F, 0x03E0, 0x7C00]);
        assert_eq!(frames[0].to_rgba(&built).data, rgba.data);
    }

    #[test]
    fn black_partial_alpha_and_quantization() {
        let black = ClassicRgbaFrame { center_x: 0, center_y: 0, width: 1, height: 1, rgba: vec![0, 0, 0, 255] };
        let (palette, frames, report) = index_rgba_frames(&[black.clone()], None).unwrap();
        assert_eq!((palette[1], frames[0].pixels[0], report.black), (OPAQUE_BLACK, Some(1), 1));
        let partial = ClassicRgbaFrame { rgba: vec![0, 0, 0, 128], ..black };
        assert!(index_rgba_frames(&[partial], None).is_err());

        let width = 64u16;
        let rgba = (0..usize::from(width) * usize::from(width))
            .flat_map(|i| [(i % 32 * 8) as u8, (i / 32 % 32 * 8) as u8, (i / 1024 * 64) as u8, 255])
            .collect();
        let many = ClassicRgbaFrame { center_x: 0, center_y: 0, width, height: width, rgba };
        let (palette, frames, report) = index_rgba_frames(std::slice::from_ref(&many), None).unwrap();
        assert!(report.quantized && report.distinct_colours > USABLE_PALETTE_COLOURS);
        assert_eq!(palette[usize::from(UNUSED_PALETTE_INDEX)], 0);
        assert!(frames[0].used_indices().all(|index| index != UNUSED_PALETTE_INDEX));
        assert_eq!(index_rgba_frames(&[many], None).unwrap().1, frames, "deterministic");
    }
}
