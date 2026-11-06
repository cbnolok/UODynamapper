//! Writing TGA entries the way the entry they replace is written ([`TgaConvention`]): its image type, depth,
//! descriptor and ID field, the RLE packing of the tool that made it ([`RleStyle`]), and the bytes after its pixels
//! (a TGA 2.0 footer, the 3ds max extension area, the EC fp padding), with the footer's offsets moved by the new
//! pixel length.
//!
//! The shipped TGAs (EC, EC fp and KR `Interface`, `EffectTexture`, `SystemTextures`, KR `TileArtLegacy`) are types
//! 2 and 10 (24 and 32-bit) and 3 (8-bit grey), without a color map or ID field, the origin bottom-left but in one
//! image, never right-to-left. RLE packets never cross a row. Four packings, told apart by re-encoding each entry:
//!
//! - [`RleStyle::SinglePixelRaw`]: runs of 2 or more (128 at most, then the rest is looked at again), every other
//!   pixel a raw packet of its own. The EC Interface gumps without a footer (2,750) and KR `TileArtLegacy` (16,026).
//! - [`RleStyle::Lookahead`]: the 3ds max family, with a footer. A pixel starts a run when the pixel after it is
//!   the same; at the end of a row, "after" is the first pixel of the next row in the top-down source image
//!   ([`NextPixel::Memory`]), of the next stored row ([`NextPixel::File`]), or nothing ([`NextPixel::Row`]); a
//!   raw packet stops before a pixel equal to its next. Runs are packed by 128 with the rest packed as a run
//!   (`long_runs`) or looked at again, and a run of one is a run or a raw packet (`single_runs_raw`). Most
//!   (165 of 176) are `Memory`, not `long_runs`, `single_runs_raw`.
//! - [`RleStyle::RunsFromThree`]: runs of 3 or more within a row, raw packets of up to 128 otherwise (the one
//!   top-down 24-bit `SystemTextures` image of EC and KR).
//!
//! [`TgaConvention::of`] finds the style that gives the entry back; one that no style gives back is written with
//! the most common 3ds max style, and [`TgaConvention::reproduces`] says so.

use image::RgbaImage;

crate::eyre_imports!();

/// The TGA header; the 0x0 placeholder entries are a header alone.
pub const HEADER_BYTES: usize = 18;
const ID_LENGTH_OFFSET: usize = 0;
const COLOR_MAP_TYPE_OFFSET: usize = 1;
const IMAGE_TYPE_OFFSET: usize = 2;
const WIDTH_OFFSET: usize = 12;
const HEIGHT_OFFSET: usize = 14;
const DEPTH_OFFSET: usize = 16;
const DESCRIPTOR_OFFSET: usize = 17;
const NO_COLOR_MAP: u8 = 0;
pub const TRUE_COLOR: u8 = 2;
pub const GREY: u8 = 3;
pub const TRUE_COLOR_RLE: u8 = 10;
const WRITTEN_TYPES: [u8; 3] = [TRUE_COLOR, GREY, TRUE_COLOR_RLE];
const DEPTH_GREY: u8 = 8;
const DEPTH_24: u8 = 24;
const DEPTH_32: u8 = 32;
/// Descriptor bit 5: rows stored top to bottom.
const DESCRIPTOR_TOP_TO_BOTTOM: u8 = 0x20;
/// Descriptor bit 4: pixels stored right to left (no shipped TGA).
const DESCRIPTOR_RIGHT_TO_LEFT: u8 = 0x10;
const RUN_FLAG: u8 = 0x80;
const COUNT_MASK: u8 = 0x7f;
const MAX_PACKET_PIXELS: usize = 128;
const RUNS_FROM_THREE_MIN_RUN: usize = 3;
const FOOTER_SIGNATURE: &[u8] = b"TRUEVISION-XFILE.\0";
const FOOTER_OFFSET_BYTES: usize = size_of::<u32>();
const RGBA: usize = 4;
const OPAQUE: u8 = u8::MAX;

/// What is after a row's last pixel for [`RleStyle::Lookahead`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextPixel {
    /// The first pixel of the next row of the top-down source image (the previous stored row of a bottom-up TGA).
    Memory,
    /// The first pixel of the next stored row.
    File,
    /// Nothing: a row ends a raw packet.
    Row,
}

/// How a TGA's RLE packets were cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RleStyle {
    SinglePixelRaw,
    Lookahead { next: NextPixel, long_runs: bool, single_runs_raw: bool },
    RunsFromThree,
}

impl RleStyle {
    /// The most common 3ds max style, for an entry no style gives back.
    pub const DEFAULT: Self = Self::Lookahead { next: NextPixel::Memory, long_runs: false, single_runs_raw: true };

    /// Every style, in the order [`TgaConvention::of`] tries them.
    pub fn all() -> Vec<Self> {
        let mut styles = vec![Self::SinglePixelRaw, Self::DEFAULT];
        for next in [NextPixel::Memory, NextPixel::File, NextPixel::Row] {
            for long_runs in [false, true] {
                for single_runs_raw in [true, false] {
                    let style = Self::Lookahead { next, long_runs, single_runs_raw };
                    if style != Self::DEFAULT {
                        styles.push(style);
                    }
                }
            }
        }
        styles.push(Self::RunsFromThree);
        styles
    }

    pub fn label(&self) -> String {
        match self {
            Self::SinglePixelRaw => "single-pixel raw packets".to_owned(),
            Self::Lookahead { next, long_runs, single_runs_raw } => format!(
                "3ds max ({next:?} lookahead{}{})",
                if *long_runs { ", long runs" } else { "" },
                if *single_runs_raw { ", single runs raw" } else { "" }
            ),
            Self::RunsFromThree => "runs of 3 or more".to_owned(),
        }
    }
}

/// Pixels as stored: rows in file order, `bytes` per pixel.
struct StoredPixels<'a> {
    data: &'a [u8],
    bytes: usize,
    width: usize,
    height: usize,
    top_to_bottom: bool,
}

impl StoredPixels<'_> {
    fn pixel(&self, row: usize, x: usize) -> &[u8] {
        let start = (row * self.width + x) * self.bytes;
        &self.data[start..start + self.bytes]
    }

    /// The pixel after `x` of stored row `row` for a lookahead.
    fn next(&self, row: usize, x: usize, next: NextPixel) -> Option<&[u8]> {
        if x + 1 < self.width {
            return Some(self.pixel(row, x + 1));
        }
        let following = match (next, self.top_to_bottom) {
            (NextPixel::Row, _) => None,
            (NextPixel::File, _) | (NextPixel::Memory, true) => Some(row + 1).filter(|&row| row < self.height),
            (NextPixel::Memory, false) => row.checked_sub(1),
        };
        following.map(|row| self.pixel(row, 0))
    }

    /// The length of the run of the pixel at `x`, within the row, up to `limit`.
    fn run(&self, row: usize, x: usize, limit: usize) -> usize {
        let pixel = self.pixel(row, x);
        (1..self.width - x).take_while(|&n| n < limit && self.pixel(row, x + n) == pixel).count() + 1
    }

    fn encode(&self, style: RleStyle) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len());
        let run = |out: &mut Vec<u8>, pixel: &[u8], count: usize| {
            out.push(RUN_FLAG | (count - 1) as u8);
            out.extend_from_slice(pixel);
        };
        let raw = |out: &mut Vec<u8>, row: usize, start: usize, end: usize| {
            out.push((end - start - 1) as u8);
            out.extend_from_slice(&self.data[(row * self.width + start) * self.bytes..(row * self.width + end) * self.bytes]);
        };
        for row in 0..self.height {
            let mut x = 0;
            while x < self.width {
                match style {
                    RleStyle::SinglePixelRaw => {
                        let count = self.run(row, x, MAX_PACKET_PIXELS);
                        if count > 1 {
                            run(&mut out, self.pixel(row, x), count);
                        } else {
                            raw(&mut out, row, x, x + 1);
                        }
                        x += count;
                    }
                    RleStyle::RunsFromThree => {
                        let count = self.run(row, x, MAX_PACKET_PIXELS);
                        if count >= RUNS_FROM_THREE_MIN_RUN {
                            run(&mut out, self.pixel(row, x), count);
                            x += count;
                            continue;
                        }
                        let start = x;
                        x += 1;
                        while x < self.width && x - start < MAX_PACKET_PIXELS && self.run(row, x, RUNS_FROM_THREE_MIN_RUN) < RUNS_FROM_THREE_MIN_RUN {
                            x += 1;
                        }
                        raw(&mut out, row, start, x);
                    }
                    RleStyle::Lookahead { next, long_runs, single_runs_raw } => {
                        if self.next(row, x, next) == Some(self.pixel(row, x)) {
                            let mut count = self.run(row, x, if long_runs { usize::MAX } else { MAX_PACKET_PIXELS });
                            if count == 1 && single_runs_raw {
                                raw(&mut out, row, x, x + 1);
                                x += 1;
                                continue;
                            }
                            while count > 0 {
                                let packet = count.min(MAX_PACKET_PIXELS);
                                run(&mut out, self.pixel(row, x), packet);
                                x += packet;
                                count -= packet;
                            }
                        } else {
                            let start = x;
                            while x < self.width && x - start < MAX_PACKET_PIXELS && self.next(row, x, next) != Some(self.pixel(row, x)) {
                                x += 1;
                            }
                            raw(&mut out, row, start, x);
                        }
                    }
                }
            }
        }
        out
    }
}

/// How a TGA entry is written, read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaConvention {
    header: [u8; HEADER_BYTES],
    id: Vec<u8>,
    /// For RLE: the style that gives the entry back, or `None` when none does ([`RleStyle::DEFAULT`] is used).
    style: Option<RleStyle>,
    /// The bytes after the pixels.
    tail: Vec<u8>,
    pixels_end: usize,
}

/// A TGA's fields, its pixels as stored, and where they end.
struct ReadTga {
    header: [u8; HEADER_BYTES],
    id: Vec<u8>,
    pixels: Vec<u8>,
    stream_start: usize,
    pixels_end: usize,
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_tga(bytes: &[u8]) -> eyre::Result<ReadTga> {
    let header: [u8; HEADER_BYTES] = bytes.get(..HEADER_BYTES).ok_or_else(|| eyre!("shorter than a TGA header"))?.try_into()?;
    let image_type = header[IMAGE_TYPE_OFFSET];
    let depth = header[DEPTH_OFFSET];
    if !WRITTEN_TYPES.contains(&image_type) || header[COLOR_MAP_TYPE_OFFSET] != NO_COLOR_MAP {
        eyre::bail!("TGA type {image_type} with color map type {}: the writer does types 2, 3 and 10 without a color map", header[COLOR_MAP_TYPE_OFFSET]);
    }
    let depth_ok = if image_type == GREY { depth == DEPTH_GREY } else { depth == DEPTH_24 || depth == DEPTH_32 };
    if !depth_ok || header[DESCRIPTOR_OFFSET] & DESCRIPTOR_RIGHT_TO_LEFT != 0 {
        eyre::bail!("TGA type {image_type}, {depth}-bit, descriptor {:#04x}: no shipped TGA is written so", header[DESCRIPTOR_OFFSET]);
    }
    let stream_start = HEADER_BYTES + usize::from(header[ID_LENGTH_OFFSET]);
    let id = bytes.get(HEADER_BYTES..stream_start).ok_or_else(|| eyre!("the ID field runs past the data"))?.to_vec();
    let bytes_per_pixel = usize::from(depth / u8::BITS as u8);
    let total = usize::from(read_u16(&header, WIDTH_OFFSET)) * usize::from(read_u16(&header, HEIGHT_OFFSET)) * bytes_per_pixel;
    let (pixels, pixels_end) = if image_type == TRUE_COLOR_RLE {
        let mut pixels = Vec::with_capacity(total);
        let mut offset = stream_start;
        while pixels.len() < total {
            let packet = *bytes.get(offset).ok_or_else(|| eyre!("the RLE data ends at byte {offset}"))?;
            offset += 1;
            let count = usize::from(packet & COUNT_MASK) + 1;
            let stored = if packet & RUN_FLAG != 0 { bytes_per_pixel } else { count * bytes_per_pixel };
            let data = bytes.get(offset..offset + stored).ok_or_else(|| eyre!("the RLE packet at byte {} runs past the data", offset - 1))?;
            if packet & RUN_FLAG != 0 {
                (0..count).for_each(|_| pixels.extend_from_slice(data));
            } else {
                pixels.extend_from_slice(data);
            }
            offset += stored;
        }
        if pixels.len() != total {
            eyre::bail!("the RLE packets hold more pixels than the image");
        }
        (pixels, offset)
    } else {
        let end = stream_start + total;
        (bytes.get(stream_start..end).ok_or_else(|| eyre!("the pixels run past the data"))?.to_vec(), end)
    };
    Ok(ReadTga { header, id, pixels, stream_start, pixels_end })
}

impl TgaConvention {
    /// The convention of the TGA `entry`.
    pub fn of(entry: &[u8]) -> eyre::Result<Self> {
        let read = read_tga(entry)?;
        let mut convention =
            Self { header: read.header, id: read.id, style: None, tail: entry[read.pixels_end..].to_vec(), pixels_end: read.pixels_end };
        if convention.image_type() == TRUE_COLOR_RLE {
            let stored = convention.stored(&read.pixels, convention.width(), convention.height());
            let stream = &entry[read.stream_start..read.pixels_end];
            convention.style = RleStyle::all().into_iter().find(|&style| stored.encode(style) == stream);
        }
        Ok(convention)
    }

    pub fn image_type(&self) -> u8 {
        self.header[IMAGE_TYPE_OFFSET]
    }

    pub fn depth(&self) -> u8 {
        self.header[DEPTH_OFFSET]
    }

    pub fn width(&self) -> u16 {
        read_u16(&self.header, WIDTH_OFFSET)
    }

    pub fn height(&self) -> u16 {
        read_u16(&self.header, HEIGHT_OFFSET)
    }

    /// The RLE style, for an RLE entry.
    pub fn style(&self) -> Option<RleStyle> {
        self.style
    }

    /// Whether the entry is given back byte for byte: uncompressed, or RLE of a known style.
    pub fn reproduces(&self) -> bool {
        self.image_type() != TRUE_COLOR_RLE || self.style.is_some()
    }

    fn top_to_bottom(&self) -> bool {
        self.header[DESCRIPTOR_OFFSET] & DESCRIPTOR_TOP_TO_BOTTOM != 0
    }

    fn bytes_per_pixel(&self) -> usize {
        usize::from(self.depth() / u8::BITS as u8)
    }

    fn stored<'a>(&self, pixels: &'a [u8], width: u16, height: u16) -> StoredPixels<'a> {
        StoredPixels { data: pixels, bytes: self.bytes_per_pixel(), width: usize::from(width), height: usize::from(height), top_to_bottom: self.top_to_bottom() }
    }

    /// "TGA type 10, 32-bit, single-pixel raw packets, footer".
    pub fn label(&self) -> String {
        let style = match (self.image_type(), self.style) {
            (TRUE_COLOR_RLE, Some(style)) => format!(", {}", style.label()),
            (TRUE_COLOR_RLE, None) => format!(", RLE of no known style (written as {})", RleStyle::DEFAULT.label()),
            _ => String::new(),
        };
        let ending = if self.tail.is_empty() {
            ""
        } else if self.footer_at().is_some() {
            ", footer"
        } else {
            ", trailing bytes"
        };
        format!("TGA type {}, {}-bit{style}{ending}", self.image_type(), self.depth())
    }

    /// Where the footer starts in the tail.
    fn footer_at(&self) -> Option<usize> {
        let signature = self.tail.windows(FOOTER_SIGNATURE.len()).rposition(|window| window == FOOTER_SIGNATURE)?;
        signature.checked_sub(2 * FOOTER_OFFSET_BYTES)
    }

    /// A TGA of `pixels` (`width` x `height`, as stored: in file row order, the entry's bytes per pixel) written
    /// as the entry is.
    pub fn encode_stored(&self, pixels: &[u8], width: u16, height: u16) -> eyre::Result<Vec<u8>> {
        let expected = usize::from(width) * usize::from(height) * self.bytes_per_pixel();
        if pixels.len() != expected {
            eyre::bail!("{} pixel bytes for {width}x{height} at {} bits", pixels.len(), self.depth());
        }
        let mut out = self.header.to_vec();
        out[WIDTH_OFFSET..WIDTH_OFFSET + 2].copy_from_slice(&width.to_le_bytes());
        out[HEIGHT_OFFSET..HEIGHT_OFFSET + 2].copy_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&self.id);
        if self.image_type() == TRUE_COLOR_RLE {
            out.extend_from_slice(&self.stored(pixels, width, height).encode(self.style.unwrap_or(RleStyle::DEFAULT)));
        } else {
            out.extend_from_slice(pixels);
        }
        let pixels_end = out.len();
        let mut tail = self.tail.clone();
        if let Some(footer) = self.footer_at() {
            for field in [footer, footer + FOOTER_OFFSET_BYTES] {
                let offset = u32::from_le_bytes(tail[field..field + FOOTER_OFFSET_BYTES].try_into().expect("4 bytes")) as usize;
                if offset >= self.pixels_end {
                    let moved = u32::try_from(offset - self.pixels_end + pixels_end).wrap_err("the TGA is too large for its footer offsets")?;
                    tail[field..field + FOOTER_OFFSET_BYTES].copy_from_slice(&moved.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&tail);
        Ok(out)
    }

    /// A TGA of `image` written as the entry is. A 24-bit or grey entry takes opaque pixels only, a grey one grey
    /// pixels only; refused pixels are named.
    pub fn encode_rgba(&self, image: &RgbaImage) -> eyre::Result<Vec<u8>> {
        let (Ok(width), Ok(height)) = (u16::try_from(image.width()), u16::try_from(image.height())) else {
            eyre::bail!("an image of {}x{} is larger than a TGA holds", image.width(), image.height());
        };
        let bytes = self.bytes_per_pixel();
        let mut stored = vec![0; usize::from(width) * usize::from(height) * bytes];
        let rows: Vec<&[u8]> = image.as_raw().chunks_exact(usize::from(width).max(1) * RGBA).collect();
        for (index, row) in rows.iter().enumerate() {
            let file_row = if self.top_to_bottom() { index } else { rows.len() - 1 - index };
            let out = &mut stored[file_row * usize::from(width) * bytes..(file_row + 1) * usize::from(width) * bytes];
            for (x, (pixel, target)) in row.chunks_exact(RGBA).zip(out.chunks_exact_mut(bytes)).enumerate() {
                let [red, green, blue, alpha] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                let refuse = |what: &str| eyre!("pixel x {x}, y {index} is {what}; the entry is a {}-bit TGA", self.depth());
                match self.depth() {
                    DEPTH_32 => target.copy_from_slice(&[blue, green, red, alpha]),
                    _ if alpha != OPAQUE => return Err(refuse(&format!("partly transparent (alpha {alpha})"))),
                    DEPTH_24 => target.copy_from_slice(&[blue, green, red]),
                    _ if red != green || green != blue => return Err(refuse("not grey")),
                    _ => target[0] = red,
                }
            }
        }
        self.encode_stored(&stored, width, height)
    }
}

/// The TGA `entry` written again from its own pixels with its convention: the entry itself when
/// [`TgaConvention::reproduces`].
pub fn reencode(entry: &[u8]) -> eyre::Result<Vec<u8>> {
    let convention = TgaConvention::of(entry)?;
    let read = read_tga(entry)?;
    convention.encode_stored(&read.pixels, convention.width(), convention.height())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::textures::decode_tga_rgba8;

    const RED: [u8; 4] = [0, 0, 255, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];

    fn header(image_type: u8, depth: u8, descriptor: u8, width: u16, height: u16) -> Vec<u8> {
        let mut header = vec![0; HEADER_BYTES];
        header[IMAGE_TYPE_OFFSET] = image_type;
        header[WIDTH_OFFSET..WIDTH_OFFSET + 2].copy_from_slice(&width.to_le_bytes());
        header[HEIGHT_OFFSET..HEIGHT_OFFSET + 2].copy_from_slice(&height.to_le_bytes());
        header[DEPTH_OFFSET] = depth;
        header[DESCRIPTOR_OFFSET] = descriptor;
        header
    }

    #[test]
    fn finds_the_style_that_gives_the_entry_back() {
        // A row RED RED BLUE RED: single-pixel raw packets.
        let mut single = header(TRUE_COLOR_RLE, DEPTH_32, 0, 4, 1);
        single.extend([RUN_FLAG | 1].iter().chain(&RED).chain(&[0]).chain(&BLUE).chain(&[0]).chain(&RED));
        let convention = TgaConvention::of(&single).unwrap();
        assert_eq!(convention.style(), Some(RleStyle::SinglePixelRaw));
        assert_eq!(reencode(&single).unwrap(), single);

        // BLUE RED BLUE as one raw packet, then a footer pointing at the extension area after the pixels.
        let mut packed = header(TRUE_COLOR_RLE, DEPTH_32, 8, 3, 1);
        packed.extend([2].iter().chain(&BLUE).chain(&RED).chain(&BLUE));
        let pixels_end = packed.len() as u32;
        let extension = [7u8; 4];
        packed.extend_from_slice(&extension);
        packed.extend_from_slice(&pixels_end.to_le_bytes());
        packed.extend_from_slice(&0u32.to_le_bytes());
        packed.extend_from_slice(FOOTER_SIGNATURE);
        let convention = TgaConvention::of(&packed).unwrap();
        assert!(matches!(convention.style(), Some(RleStyle::Lookahead { .. })), "{:?}", convention.style());
        assert_eq!(reencode(&packed).unwrap(), packed);

        // A wider image moves the extension offset with the pixels; the image crate reads it back.
        let image = RgbaImage::from_raw(4, 1, [RED, RED, RED, BLUE].iter().flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]]).collect()).unwrap();
        let written = convention.encode_rgba(&image).unwrap();
        let moved = u32::from_le_bytes(written[written.len() - 26..written.len() - 22].try_into().unwrap()) as usize;
        assert_eq!(&written[moved..moved + extension.len()], extension);
        assert_eq!(decode_tga_rgba8(&written).unwrap(), image);
    }

    #[test]
    fn writes_uncompressed_and_refuses_what_the_depth_cannot_hold() {
        let mut grey = header(GREY, DEPTH_GREY, 0, 2, 2);
        grey.extend([10, 20, 30, 40]);
        assert_eq!(reencode(&grey).unwrap(), grey);
        let convention = TgaConvention::of(&grey).unwrap();
        let image = RgbaImage::from_raw(2, 2, vec![30, 30, 30, 255, 40, 40, 40, 255, 10, 10, 10, 255, 20, 20, 20, 255]).unwrap();
        assert_eq!(convention.encode_rgba(&image).unwrap(), grey, "rows are stored bottom up");
        let colored = RgbaImage::from_raw(1, 1, vec![1, 2, 3, 255]).unwrap();
        assert!(convention.encode_rgba(&colored).unwrap_err().to_string().contains("not grey"));

        let mut opaque = header(TRUE_COLOR, DEPTH_24, DESCRIPTOR_TOP_TO_BOTTOM, 1, 1);
        opaque.extend([1, 2, 3]);
        let convention = TgaConvention::of(&opaque).unwrap();
        let clear = RgbaImage::from_raw(1, 1, vec![3, 2, 1, 0]).unwrap();
        assert!(convention.encode_rgba(&clear).unwrap_err().to_string().contains("x 0, y 0 is partly transparent (alpha 0)"));
        assert!(TgaConvention::of(&header(1, 8, 0, 1, 1)).is_err(), "color-mapped");
    }

    #[test]
    fn every_style_reads_back() {
        let pixels: Vec<u8> = [RED, RED, RED, BLUE, RED, BLUE, BLUE, RED, RED, RED, RED, BLUE].concat();
        for descriptor in [0, DESCRIPTOR_TOP_TO_BOTTOM] {
            let stored = StoredPixels { data: &pixels, bytes: RGBA, width: 4, height: 3, top_to_bottom: descriptor != 0 };
            for style in RleStyle::all() {
                let mut tga = header(TRUE_COLOR_RLE, DEPTH_32, descriptor, 4, 3);
                tga.extend(stored.encode(style));
                assert_eq!(read_tga(&tga).unwrap().pixels, pixels, "{style:?}");
            }
        }
    }
}
