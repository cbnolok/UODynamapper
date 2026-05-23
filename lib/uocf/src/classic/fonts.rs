//! Classic client font loaders for `fonts.mul` and optional `unifont*.mul`.
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

use crate::utils::color::color_lut;

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
            log::info!(
                "uocf: Loaded classic fonts.mul with {} ASCII font face(s)",
                fonts.len()
            );
            fonts
        } else {
            Vec::new()
        };

        let mut unicode_fonts = Vec::with_capacity(20);
        for index in 0..20 {
            let file_name = if index == 0 {
                "unifont.mul".to_string()
            } else {
                format!("unifont{index}.mul")
            };
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
        let mut rgba = Vec::new();
        rgba.resize(width as usize * height as usize * 4, 0);
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
