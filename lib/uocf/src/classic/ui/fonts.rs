//! Classic client font loaders and writers for `fonts.mul` ([`AsciiFontsFile`]) and optional `unifont*.mul`
//! ([`UnicodeFontData`]); both write back byte for byte.
//!
//! `fonts.mul` stores a sequence of bitmap font faces. Each face begins with a
//! one-byte header followed by 224 glyph records for printable characters
//! 0x20..=0xff. A glyph record is `(width: u8, height: u8, unknown: u8)` plus
//! `width * height` little-endian `u16` pixels.
//!
//! Unicode font files use a dense 0x10000-entry `i32` offset table. Non-zero
//! offsets point to `(offset_x: i8, offset_y: i8, width: u8, height: u8)` plus a
//! one-bit-per-pixel bitmap, row-major with each row padded to a full byte.

crate::eyre_imports!();

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use memmap2::Mmap;

use crate::classic::gump_write::{rgb555_of_rgba, OPAQUE_BLACK, TRANSPARENT, TRANSPARENT_ALPHA};
use crate::utils::color::color_lut;

/// Bytes of an RGBA pixel.
const RGBA_CHANNELS: usize = 4;
const BITS_PER_BYTE: usize = 8;
/// From this alpha on, a pixel of an image is drawn in a Unicode glyph.
const DRAWN_ALPHA_MIN: u8 = 128;

pub const ASCII_PRINTABLE_START: u8 = 32;
pub const ASCII_GLYPH_COUNT: usize = 224;
const ASCII_GLYPH_HEADER_BYTES: usize = 3;
const UNICODE_GLYPH_COUNT: usize = 0x10000;
const UNICODE_LOOKUP_BYTES: usize = UNICODE_GLYPH_COUNT * std::mem::size_of::<i32>();
const UNICODE_GLYPH_HEADER_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsciiFontGlyph {
    pub width: u8,
    pub height: u8,
    pub unknown: u8,
    pub pixels: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl AsciiFontGlyph {
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn to_rgba8(&self) -> eyre::Result<FontRgbaImage> {
        if self.pixels.len() != self.pixel_count() {
            eyre::bail!(
                "ASCII font glyph has {} pixels, expected {} for {}x{}.",
                self.pixels.len(),
                self.pixel_count(),
                self.width,
                self.height
            );
        }

        let lut = color_lut();
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for pixel in &self.pixels {
            rgba.extend_from_slice(&lut[*pixel as usize].to_le_bytes());
        }

        Ok(FontRgbaImage {
            width: self.width as u32,
            height: self.height as u32,
            rgba,
        })
    }

    /// As [`Self::to_rgba8`], with the pixels of value 0 transparent: what [`Self::set_rgba8`] reads back. Reads the
    /// pixels there are, so a glyph with too few is shorter.
    pub fn to_transparent_rgba8(&self) -> FontRgbaImage {
        let lut = color_lut();
        let mut rgba = Vec::with_capacity(self.pixels.len() * RGBA_CHANNELS);
        for &pixel in &self.pixels {
            if pixel == TRANSPARENT {
                rgba.extend_from_slice(&[0; RGBA_CHANNELS]);
            } else {
                rgba.extend_from_slice(&lut[usize::from(pixel)].to_le_bytes());
            }
        }
        FontRgbaImage { width: u32::from(self.width), height: u32::from(self.height), rgba }
    }

    /// The pixel of an opaque colour; black is the same as in art and gumps, since 0 is left for transparent.
    pub fn pixel_of_rgb([red, green, blue]: [u8; 3]) -> u16 {
        match rgb555_of_rgba(&[red, green, blue]) {
            TRANSPARENT => OPAQUE_BLACK,
            pixel => pixel,
        }
    }

    /// Replaces the size and pixels with an RGBA image: a transparent pixel is 0, any other its colour. `byte 3` stays.
    pub fn set_rgba8(&mut self, width: u8, height: u8, rgba: &[u8]) -> eyre::Result<()> {
        check_image_len(width, height, rgba)?;
        self.pixels = rgba
            .chunks_exact(RGBA_CHANNELS)
            .map(|pixel| if pixel[3] == TRANSPARENT_ALPHA { TRANSPARENT } else { Self::pixel_of_rgb([pixel[0], pixel[1], pixel[2]]) })
            .collect();
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Changes the size: the pixels both sizes have stay, the new ones are 0.
    pub fn resize(&mut self, width: u8, height: u8) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        let (old_width, new_width) = (usize::from(self.width), usize::from(width));
        let mut pixels = vec![TRANSPARENT; new_width * usize::from(height)];
        for y in 0..usize::from(self.height.min(height)) {
            for x in 0..old_width.min(new_width) {
                pixels[y * new_width + x] = self.pixels[y * old_width + x];
            }
        }
        self.pixels = pixels;
        self.width = width;
        self.height = height;
    }
}

fn check_image_len(width: u8, height: u8, rgba: &[u8]) -> eyre::Result<()> {
    let expected = usize::from(width) * usize::from(height) * RGBA_CHANNELS;
    if rgba.len() != expected {
        eyre::bail!("a {width}x{height} image has {expected} RGBA bytes, not {}", rgba.len());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsciiFont {
    pub header: u8,
    pub glyphs: Vec<AsciiFontGlyph>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeFontGlyph {
    pub offset_x: i8,
    pub offset_y: i8,
    pub width: u8,
    pub height: u8,
    pub bitmap: Vec<u8>,
}

impl UnicodeFontGlyph {
    /// A glyph with no pixel set and no offset.
    pub fn blank(width: u8, height: u8) -> Self {
        Self { offset_x: 0, offset_y: 0, width, height, bitmap: vec![0; unicode_bitmap_len(width, height)] }
    }

    /// Sets a pixel; false when it is outside the glyph.
    pub fn set_bit(&mut self, x: usize, y: usize, on: bool) -> bool {
        let Some(stride) = self.row_stride_bytes() else {
            return false;
        };
        if x >= usize::from(self.width) || y >= usize::from(self.height) {
            return false;
        }
        let Some(byte) = self.bitmap.get_mut(y * stride + x / BITS_PER_BYTE) else {
            return false;
        };
        let mask = 1u8 << (BITS_PER_BYTE - 1 - x % BITS_PER_BYTE);
        if on {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        true
    }

    /// Replaces the size and pixels with an RGBA image: a pixel is drawn when it is opaque and not black, so a light
    /// mark on either transparency or black reads the same. The offsets stay.
    pub fn set_rgba8(&mut self, width: u8, height: u8, rgba: &[u8]) -> eyre::Result<()> {
        check_image_len(width, height, rgba)?;
        let mut glyph = Self { offset_x: self.offset_x, offset_y: self.offset_y, ..Self::blank(width, height) };
        for (index, pixel) in rgba.chunks_exact(RGBA_CHANNELS).enumerate() {
            if pixel[3] >= DRAWN_ALPHA_MIN && pixel[..3].iter().any(|&channel| channel != 0) {
                glyph.set_bit(index % usize::from(width), index / usize::from(width), true);
            }
        }
        *self = glyph;
        Ok(())
    }

    /// Changes the size: the pixels both sizes have stay, the new ones are off.
    pub fn resize(&mut self, width: u8, height: u8) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        let mut glyph = Self { offset_x: self.offset_x, offset_y: self.offset_y, ..Self::blank(width, height) };
        for y in 0..usize::from(self.height.min(height)) {
            for x in 0..usize::from(self.width.min(width)) {
                if self.bit_at(x, y).unwrap_or(false) {
                    glyph.set_bit(x, y, true);
                }
            }
        }
        *self = glyph;
    }

    pub fn row_stride_bytes(&self) -> Option<usize> {
        if self.width == 0 || self.height == 0 {
            return None;
        }

        Some((self.width as usize + 7) / 8)
    }

    pub fn bit_at(&self, x: usize, y: usize) -> Option<bool> {
        let stride = self.row_stride_bytes()?;
        if x >= self.width as usize || y >= self.height as usize {
            return None;
        }

        let byte = self.bitmap.get(y * stride + x / 8)?;
        Some((byte & (1 << (7 - (x % 8)))) != 0)
    }

    pub fn to_rgba8(
        &self,
        foreground: [u8; 4],
        background: Option<[u8; 4]>,
    ) -> eyre::Result<FontRgbaImage> {
        let width = self.width as usize;
        let height = self.height as usize;
        let Some(stride) = self.row_stride_bytes() else {
            return Ok(FontRgbaImage {
                width: self.width as u32,
                height: self.height as u32,
                rgba: Vec::new(),
            });
        };
        let expected_len = stride
            .checked_mul(height)
            .ok_or_else(|| eyre!("unicode font glyph bitmap length overflowed"))?;
        if self.bitmap.len() != expected_len {
            eyre::bail!(
                "Unicode font glyph has {} bitmap bytes, expected {} for {}x{}.",
                self.bitmap.len(),
                expected_len,
                self.width,
                self.height
            );
        }

        let background = background.unwrap_or([0, 0, 0, 0]);
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                if self.bit_at(x, y).unwrap_or(false) {
                    rgba.extend_from_slice(&foreground);
                } else {
                    rgba.extend_from_slice(&background);
                }
            }
        }

        Ok(FontRgbaImage {
            width: self.width as u32,
            height: self.height as u32,
            rgba,
        })
    }
}

#[derive(Clone)]
pub struct UnicodeFontFile {
    path: PathBuf,
    mmap: Arc<Mmap>,
}

#[derive(Clone)]
pub struct ClassicFonts {
    ascii_fonts: Vec<AsciiFont>,
    unicode_fonts: Vec<Option<UnicodeFontFile>>,
}

fn first_existing(client_path: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|name| client_path.join(name))
        .find(|path| path.is_file())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> eyre::Result<u16> {
    if offset + std::mem::size_of::<u16>() > bytes.len() {
        eyre::bail!(
            "Unexpected end of fonts data while reading u16 at byte offset {}.",
            offset
        );
    }

    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> eyre::Result<i32> {
    if offset + std::mem::size_of::<i32>() > bytes.len() {
        eyre::bail!(
            "Unexpected end of unicode font data while reading i32 at byte offset {}.",
            offset
        );
    }

    Ok(i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

pub fn ascii_index_for_char(ch: char) -> usize {
    let byte = ch as u32 as u8;
    if byte < ASCII_PRINTABLE_START {
        0
    } else {
        (byte - ASCII_PRINTABLE_START) as usize
    }
}

pub fn load_ascii_fonts(path: impl AsRef<Path>) -> eyre::Result<Vec<AsciiFont>> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .wrap_err_with(|| format!("Failed to open classic fonts file {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .wrap_err_with(|| format!("Failed to read classic fonts file {}", path.display()))?;

    decode_ascii_fonts(&bytes)
}

pub fn decode_ascii_fonts(bytes: &[u8]) -> eyre::Result<Vec<AsciiFont>> {
    let mut offset = 0usize;
    let mut fonts = Vec::new();

    loop {
        let font_start = offset;
        if offset >= bytes.len() {
            break;
        }

        let header = bytes[offset];
        offset += 1;

        let mut glyphs = Vec::with_capacity(ASCII_GLYPH_COUNT);
        let mut complete = true;

        for _ in 0..ASCII_GLYPH_COUNT {
            if offset + ASCII_GLYPH_HEADER_BYTES > bytes.len() {
                complete = false;
                break;
            }

            let width = bytes[offset];
            let height = bytes[offset + 1];
            let unknown = bytes[offset + 2];
            offset += ASCII_GLYPH_HEADER_BYTES;

            let pixel_count = width as usize * height as usize;
            let pixel_bytes = pixel_count
                .checked_mul(std::mem::size_of::<u16>())
                .ok_or_else(|| eyre!("classic font glyph pixel byte length overflowed"))?;
            if offset + pixel_bytes > bytes.len() {
                complete = false;
                break;
            }

            let mut pixels = Vec::with_capacity(pixel_count);
            for pixel_index in 0..pixel_count {
                pixels.push(read_u16_le(bytes, offset + pixel_index * 2)?);
            }
            offset += pixel_bytes;

            glyphs.push(AsciiFontGlyph {
                width,
                height,
                unknown,
                pixels,
            });
        }

        if !complete {
            offset = font_start;
            break;
        }

        fonts.push(AsciiFont { header, glyphs });
    }

    if offset < bytes.len() {
        log::debug!(
            "uocf: ignored {} trailing byte(s) after {} complete classic font face(s)",
            bytes.len() - offset,
            fonts.len()
        );
    }

    Ok(fonts)
}

/// The `unifont*.mul` files the client may have: `unifont.mul`, then `unifont1.mul` to `unifont19.mul`.
pub const UNICODE_FONT_FILES_MAX: usize = 20;

/// The file name of Unicode font `index`: `unifont.mul` for 0, else `unifont{index}.mul`.
pub fn unicode_font_file_name(index: usize) -> String {
    if index == 0 { "unifont.mul".to_owned() } else { format!("unifont{index}.mul") }
}

fn ascii_glyph_bytes(glyph: &AsciiFontGlyph) -> usize {
    ASCII_GLYPH_HEADER_BYTES + glyph.pixels.len() * std::mem::size_of::<u16>()
}

/// `fonts.mul` for writing: the faces and the bytes after the last whole face, kept (every shipped file ends in
/// 143 such bytes: zeros, then a hexadecimal text between two `x`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsciiFontsFile {
    pub fonts: Vec<AsciiFont>,
    pub tail: Vec<u8>,
}

impl AsciiFontsFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let fonts = decode_ascii_fonts(bytes)?;
        let read: usize = fonts.iter().map(|font| 1 + font.glyphs.iter().map(ascii_glyph_bytes).sum::<usize>()).sum();
        Ok(Self { fonts, tail: bytes[read..].to_vec() })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    /// The faces, then the tail. Every face needs its [`ASCII_GLYPH_COUNT`] glyphs, and every glyph
    /// `width * height` pixels.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for (font_index, font) in self.fonts.iter().enumerate() {
            if font.glyphs.len() != ASCII_GLYPH_COUNT {
                eyre::bail!("font {font_index}: {} glyphs, a font has {ASCII_GLYPH_COUNT}", font.glyphs.len());
            }
            bytes.push(font.header);
            for (glyph_index, glyph) in font.glyphs.iter().enumerate() {
                if glyph.pixels.len() != glyph.pixel_count() {
                    eyre::bail!(
                        "font {font_index} glyph {glyph_index}: {} pixels for {}x{}",
                        glyph.pixels.len(),
                        glyph.width,
                        glyph.height
                    );
                }
                bytes.extend_from_slice(&[glyph.width, glyph.height, glyph.unknown]);
                glyph.pixels.iter().for_each(|pixel| bytes.extend_from_slice(&pixel.to_le_bytes()));
            }
        }
        bytes.extend_from_slice(&self.tail);
        Ok(bytes)
    }

    /// Writes [`Self::to_bytes`] through a temporary file in the folder of `path`.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save_bytes(path, &self.to_bytes()?)
    }
}

fn save_bytes(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    crate::utils::output_file::write_through_temp(path, |temp| {
        std::fs::write(temp, bytes).wrap_err_with(|| format!("write {}", temp.display()))
    })
}

/// One code point of a [`UnicodeFontData`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnicodeFontSlot {
    /// Offset 0: no glyph.
    Absent,
    /// A glyph, and the bytes after its bitmap up to the next glyph: the glyphs of width 0 of `unifont3.mul` on
    /// carry `height` bytes the client does not read.
    Glyph { glyph: UnicodeFontGlyph, padding: Vec<u8> },
    /// An offset to the end of the file, with no glyph there (U+FFE0 of `unifont3.mul` to `unifont12.mul`).
    PastEnd,
}

/// A `unifont*.mul` file for writing: one slot per code point.
///
/// The shipped files lay the glyphs out right after the offset table, in code point order, each once, so
/// [`Self::to_bytes`] writes them so; a glyph two code points share is written once per code point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeFontData {
    slots: Vec<UnicodeFontSlot>,
}

impl Default for UnicodeFontData {
    fn default() -> Self {
        Self { slots: vec![UnicodeFontSlot::Absent; UNICODE_GLYPH_COUNT] }
    }
}

fn unicode_bitmap_len(width: u8, height: u8) -> usize {
    if width == 0 || height == 0 { 0 } else { (width as usize).div_ceil(8) * height as usize }
}

impl UnicodeFontData {
    /// Reads every slot. An offset must be 0, address a glyph after the table, or the end of the file, and every
    /// byte after the table must belong to a glyph.
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < UNICODE_LOOKUP_BYTES {
            eyre::bail!("{} bytes, shorter than the {UNICODE_LOOKUP_BYTES}-byte offset table", bytes.len());
        }
        let offsets: Vec<i32> = (0..UNICODE_GLYPH_COUNT).map(|cp| read_i32_le(bytes, cp * std::mem::size_of::<i32>())).collect::<eyre::Result<_>>()?;
        let mut starts: Vec<usize> = Vec::new();
        for (cp, &offset) in offsets.iter().enumerate() {
            match usize::try_from(offset) {
                Ok(0) => {}
                Ok(start) if start == bytes.len() => {}
                Ok(start) if (UNICODE_LOOKUP_BYTES..bytes.len()).contains(&start) => starts.push(start),
                _ => eyre::bail!("U+{cp:04X}: offset {offset} is not 0, a glyph after the offset table, or the file end ({})", bytes.len()),
            }
        }
        starts.sort_unstable();
        starts.dedup();
        if let Some(&first) = starts.first().filter(|&&first| first != UNICODE_LOOKUP_BYTES) {
            eyre::bail!("{} bytes after the offset table belong to no glyph (the first is at {first})", first - UNICODE_LOOKUP_BYTES);
        }
        let mut slots = Vec::with_capacity(UNICODE_GLYPH_COUNT);
        for (cp, &offset) in offsets.iter().enumerate() {
            let start = offset as usize;
            if offset == 0 {
                slots.push(UnicodeFontSlot::Absent);
                continue;
            }
            if start == bytes.len() {
                slots.push(UnicodeFontSlot::PastEnd);
                continue;
            }
            let next = starts.partition_point(|&other| other <= start);
            let end = starts.get(next).copied().unwrap_or(bytes.len());
            if start + UNICODE_GLYPH_HEADER_BYTES > end {
                eyre::bail!("U+{cp:04X}: the glyph at {start} has {} bytes, fewer than its header", end - start);
            }
            let (width, height) = (bytes[start + 2], bytes[start + 3]);
            let bitmap_end = start + UNICODE_GLYPH_HEADER_BYTES + unicode_bitmap_len(width, height);
            if bitmap_end > end {
                eyre::bail!("U+{cp:04X}: the {width}x{height} bitmap at {start} runs into the next glyph or the file end at {end}");
            }
            let glyph = UnicodeFontGlyph {
                offset_x: bytes[start] as i8,
                offset_y: bytes[start + 1] as i8,
                width,
                height,
                bitmap: bytes[start + UNICODE_GLYPH_HEADER_BYTES..bitmap_end].to_vec(),
            };
            slots.push(UnicodeFontSlot::Glyph { glyph, padding: bytes[bitmap_end..end].to_vec() });
        }
        Ok(Self { slots })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    /// The offset table, then the glyphs in code point order; [`UnicodeFontSlot::PastEnd`] offsets address the
    /// file end.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = vec![0u8; UNICODE_LOOKUP_BYTES];
        let mut past_end = Vec::new();
        for (cp, slot) in self.slots.iter().enumerate() {
            let offset = match slot {
                UnicodeFontSlot::Absent => continue,
                UnicodeFontSlot::PastEnd => {
                    past_end.push(cp);
                    continue;
                }
                UnicodeFontSlot::Glyph { glyph, padding } => {
                    let offset = bytes.len();
                    bytes.extend_from_slice(&[glyph.offset_x as u8, glyph.offset_y as u8, glyph.width, glyph.height]);
                    bytes.extend_from_slice(&glyph.bitmap);
                    bytes.extend_from_slice(padding);
                    offset
                }
            };
            write_unicode_offset(&mut bytes, cp, offset)?;
        }
        let end = bytes.len();
        for cp in past_end {
            write_unicode_offset(&mut bytes, cp, end)?;
        }
        Ok(bytes)
    }

    /// Writes [`Self::to_bytes`] through a temporary file in the folder of `path`.
    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        save_bytes(path, &self.to_bytes()?)
    }

    pub fn slot(&self, codepoint: u16) -> &UnicodeFontSlot {
        &self.slots[codepoint as usize]
    }

    pub fn glyph(&self, codepoint: u16) -> Option<&UnicodeFontGlyph> {
        match self.slot(codepoint) {
            UnicodeFontSlot::Glyph { glyph, .. } => Some(glyph),
            _ => None,
        }
    }

    /// Sets or clears the glyph of `codepoint` (without padding); the bitmap must hold `ceil(width / 8) * height`
    /// bytes, none when the width or height is 0.
    pub fn set_glyph(&mut self, codepoint: u16, glyph: Option<UnicodeFontGlyph>) -> eyre::Result<()> {
        self.slots[codepoint as usize] = match glyph {
            None => UnicodeFontSlot::Absent,
            Some(glyph) => {
                let expected = unicode_bitmap_len(glyph.width, glyph.height);
                if glyph.bitmap.len() != expected {
                    eyre::bail!(
                        "U+{codepoint:04X}: a {}x{} glyph has {expected} bitmap bytes, not {}",
                        glyph.width,
                        glyph.height,
                        glyph.bitmap.len()
                    );
                }
                UnicodeFontSlot::Glyph { glyph, padding: Vec::new() }
            }
        };
        Ok(())
    }
}

fn write_unicode_offset(bytes: &mut [u8], codepoint: usize, offset: usize) -> eyre::Result<()> {
    let offset = i32::try_from(offset).map_err(|_| eyre!("U+{codepoint:04X}: offset {offset} does not fit 32 bits"))?;
    let at = codepoint * std::mem::size_of::<i32>();
    bytes[at..at + std::mem::size_of::<i32>()].copy_from_slice(&offset.to_le_bytes());
    Ok(())
}

impl UnicodeFontFile {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .wrap_err_with(|| format!("Failed to open unicode font file {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < UNICODE_LOOKUP_BYTES {
            eyre::bail!(
                "unicode font file {} is shorter than its lookup table",
                path.display()
            );
        }

        Ok(Self {
            path: path.to_path_buf(),
            mmap: Arc::new(mmap),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn glyph(&self, codepoint: u16) -> eyre::Result<Option<UnicodeFontGlyph>> {
        let lookup_offset = codepoint as usize * std::mem::size_of::<i32>();
        let payload_offset = read_i32_le(&self.mmap, lookup_offset)?;
        if payload_offset <= 0 {
            return Ok(None);
        }

        let payload_offset = payload_offset as usize;
        if payload_offset + UNICODE_GLYPH_HEADER_BYTES > self.mmap.len() {
            eyre::bail!(
                "unicode font glyph U+{codepoint:04X} points outside {}",
                self.path.display()
            );
        }

        let offset_x = self.mmap[payload_offset] as i8;
        let offset_y = self.mmap[payload_offset + 1] as i8;
        let width = self.mmap[payload_offset + 2];
        let height = self.mmap[payload_offset + 3];
        if width == 0 || height == 0 {
            return Ok(Some(UnicodeFontGlyph {
                offset_x,
                offset_y,
                width,
                height,
                bitmap: Vec::new(),
            }));
        }

        let stride = (width as usize + 7) / 8;
        let bitmap_len = stride
            .checked_mul(height as usize)
            .ok_or_else(|| eyre!("unicode font glyph bitmap length overflowed"))?;
        let bitmap_start = payload_offset + UNICODE_GLYPH_HEADER_BYTES;
        let bitmap_end = bitmap_start + bitmap_len;
        if bitmap_end > self.mmap.len() {
            eyre::bail!(
                "unicode font glyph U+{codepoint:04X} bitmap extends outside {}",
                self.path.display()
            );
        }

        Ok(Some(UnicodeFontGlyph {
            offset_x,
            offset_y,
            width,
            height,
            bitmap: self.mmap[bitmap_start..bitmap_end].to_vec(),
        }))
    }
}

impl ClassicFonts {
    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref();
        let fonts_path = first_existing(client_path, &["fonts.mul", "Fonts.mul"]);
        let ascii_fonts = if let Some(path) = fonts_path {
            let fonts = load_ascii_fonts(&path)?;
            log::debug!(
                "uocf: Loaded classic fonts.mul with {} ASCII font face(s)",
                fonts.len()
            );
            fonts
        } else {
            Vec::new()
        };

        let mut unicode_fonts = Vec::with_capacity(UNICODE_FONT_FILES_MAX);
        for index in 0..UNICODE_FONT_FILES_MAX {
            let file_name = unicode_font_file_name(index);
            let title_file_name = if index == 0 {
                "Unifont.mul".to_string()
            } else {
                format!("Unifont{index}.mul")
            };
            let path = first_existing(client_path, &[file_name.as_str(), title_file_name.as_str()]);
            let unicode_font = match path {
                Some(path) => Some(UnicodeFontFile::load(path)?),
                None => None,
            };
            unicode_fonts.push(unicode_font);
        }

        if unicode_fonts.get(1).and_then(|font| font.as_ref()).is_none() {
            if let Some(font0) = unicode_fonts.first().and_then(|font| font.as_ref()).cloned() {
                unicode_fonts[1] = Some(font0);
            }
        }

        if ascii_fonts.is_empty() && unicode_fonts.iter().all(Option::is_none) {
            eyre::bail!("No fonts.mul or unifont*.mul files found in {}", client_path.display());
        }

        Ok(Self {
            ascii_fonts,
            unicode_fonts,
        })
    }

    pub fn ascii_font_count(&self) -> usize {
        self.ascii_fonts.len()
    }

    pub fn ascii_font(&self, font: usize) -> Option<&AsciiFont> {
        self.ascii_fonts.get(font)
    }

    pub fn ascii_glyph(&self, font: usize, ch: char) -> Option<&AsciiFontGlyph> {
        self.ascii_font(font)?.glyphs.get(ascii_index_for_char(ch))
    }

    pub fn ascii_text_width(&self, font: usize, text: &str) -> u32 {
        text.chars()
            .filter_map(|ch| self.ascii_glyph(font, ch))
            .map(|glyph| glyph.width as u32)
            .sum()
    }

    pub fn unicode_font_exists(&self, font: usize) -> bool {
        self.unicode_fonts
            .get(font)
            .and_then(|font| font.as_ref())
            .is_some()
    }

    pub fn unicode_glyph(
        &self,
        font: usize,
        codepoint: u16,
    ) -> eyre::Result<Option<UnicodeFontGlyph>> {
        let Some(font) = self.unicode_fonts.get(font).and_then(|font| font.as_ref()) else {
            return Ok(None);
        };

        font.glyph(codepoint)
    }

    pub fn unicode_text_width(&self, font: usize, text: &str) -> eyre::Result<i32> {
        let mut width = 0i32;
        for ch in text.chars() {
            if let Some(glyph) = self.unicode_glyph(font, ch as u16)? {
                width += glyph.width as i32 + glyph.offset_x as i32;
            }
        }

        Ok(width)
    }

    pub fn unicode_text_height(&self, font: usize, text: &str) -> eyre::Result<i32> {
        let mut height = 0i32;
        for ch in text.chars() {
            if let Some(glyph) = self.unicode_glyph(font, ch as u16)? {
                height = height.max(glyph.height as i32 + glyph.offset_y as i32);
            }
        }

        Ok(height)
    }

    pub fn unicode_text_rgba8(
        &self,
        font: usize,
        text: &str,
        foreground: [u8; 4],
        background: Option<[u8; 4]>,
    ) -> eyre::Result<FontRgbaImage> {
        let mut glyphs = Vec::new();
        let mut pen_x = 0i32;
        let mut min_x = 0i32;
        let mut min_y = 0i32;
        let mut max_x = 0i32;
        let mut max_y = 0i32;

        for ch in text.chars() {
            let Some(glyph) = self.unicode_glyph(font, ch as u16)? else {
                continue;
            };

            let draw_x = pen_x + glyph.offset_x as i32;
            let draw_y = glyph.offset_y as i32;
            let glyph_width = glyph.width as i32;
            let glyph_height = glyph.height as i32;

            min_x = min_x.min(draw_x);
            min_y = min_y.min(draw_y);
            max_x = max_x.max(draw_x + glyph_width);
            max_y = max_y.max(draw_y + glyph_height);
            pen_x = draw_x + glyph_width;
            glyphs.push((draw_x, draw_y, glyph));
        }

        let width = (max_x - min_x).max(0) as u32;
        let height = (max_y - min_y).max(0) as u32;
        let background = background.unwrap_or([0, 0, 0, 0]);
        let mut rgba = vec![0; width as usize * height as usize * 4];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.copy_from_slice(&background);
        }

        for (draw_x, draw_y, glyph) in glyphs {
            let glyph_image = glyph.to_rgba8(foreground, None)?;
            let dst_x = (draw_x - min_x) as usize;
            let dst_y = (draw_y - min_y) as usize;
            blit_rgba(
                &mut rgba,
                width as usize,
                dst_x,
                dst_y,
                glyph_image.width as usize,
                glyph_image.height as usize,
                &glyph_image.rgba,
            );
        }

        Ok(FontRgbaImage {
            width,
            height,
            rgba,
        })
    }
}

fn blit_rgba(
    dst: &mut [u8],
    dst_width: usize,
    dst_x: usize,
    dst_y: usize,
    src_width: usize,
    src_height: usize,
    src: &[u8],
) {
    for y in 0..src_height {
        for x in 0..src_width {
            let src_start = (y * src_width + x) * 4;
            if src[src_start + 3] == 0 {
                continue;
            }

            let dst_start = ((dst_y + y) * dst_width + dst_x + x) * 4;
            dst[dst_start..dst_start + 4].copy_from_slice(&src[src_start..src_start + 4]);
        }
    }
}
