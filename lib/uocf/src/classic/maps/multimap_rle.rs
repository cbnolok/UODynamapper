//! Codec for Classic Client `multimap.rle`.
//!
//! The file stores a monochrome treasure-map style drawing. The first eight
//! bytes are little-endian width and height. The remaining bytes are RLE runs:
//! bit 7 selects black, and bits 0..6 store the run length.

crate::eyre_imports!();

use image::{ColorType, ImageFormat};
use std::fs;
use std::path::Path;

pub const WHITE_PIXEL: u8 = 0;
pub const BLACK_PIXEL: u8 = 1;
pub const DEFAULT_WIDTH: u32 = 2560;
pub const DEFAULT_HEIGHT: u32 = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultimapRleImage {
    pub width: u32,
    pub height: u32,
    /// One byte per pixel: `WHITE_PIXEL` or `BLACK_PIXEL`.
    pub pixels: Vec<u8>,
}

impl MultimapRleImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> eyre::Result<Self> {
        let expected_len = pixel_len(width, height)?;
        if pixels.len() != expected_len {
            eyre::bail!(
                "multimap pixel buffer has {} pixels, expected {} for {}x{}",
                pixels.len(),
                expected_len,
                width,
                height
            );
        }
        if pixels.iter().any(|&pixel| pixel != WHITE_PIXEL && pixel != BLACK_PIXEL) {
            eyre::bail!("multimap pixel buffer contains values other than 0/1");
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> eyre::Result<Self> {
        let expected_pixels = pixel_len(width, height)?;
        let expected_bytes = expected_pixels
            .checked_mul(4)
            .ok_or_else(|| eyre::eyre!("multimap RGBA byte length overflow"))?;
        if rgba.len() != expected_bytes {
            eyre::bail!(
                "RGBA buffer has {} bytes, expected {} for {}x{}",
                rgba.len(),
                expected_bytes,
                width,
                height
            );
        }

        let mut pixels = Vec::with_capacity(expected_pixels);
        for chunk in rgba.chunks_exact(4) {
            pixels.push(classify_rgba_pixel(chunk[0], chunk[1], chunk[2], chunk[3]));
        }
        Self::new(width, height, pixels)
    }

    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &pixel in &self.pixels {
            if pixel == BLACK_PIXEL {
                rgba.extend_from_slice(&[0, 0, 0, 255]);
            } else {
                rgba.extend_from_slice(&[255, 255, 255, 255]);
            }
        }
        rgba
    }
}

pub fn decode_rle_bytes(bytes: &[u8]) -> eyre::Result<MultimapRleImage> {
    if bytes.len() < 8 {
        eyre::bail!("multimap.rle is too short: {} bytes", bytes.len());
    }

    let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let expected_len = pixel_len(width, height)?;
    let mut pixels = Vec::with_capacity(expected_len);

    for &run in &bytes[8..] {
        let count = (run & 0x7f) as usize;
        if count == 0 {
            continue;
        }

        let pixel = if (run & 0x80) != 0 {
            BLACK_PIXEL
        } else {
            WHITE_PIXEL
        };
        if pixels.len() + count > expected_len {
            eyre::bail!(
                "multimap.rle run data exceeds image size: {} > {} pixels",
                pixels.len() + count,
                expected_len
            );
        }
        pixels.extend(std::iter::repeat_n(pixel, count));
    }

    if pixels.len() != expected_len {
        eyre::bail!(
            "multimap.rle decoded {} pixels, expected {} for {}x{}",
            pixels.len(),
            expected_len,
            width,
            height
        );
    }

    MultimapRleImage::new(width, height, pixels)
}

pub fn encode_rle_bytes(image: &MultimapRleImage) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + image.pixels.len() / 8);
    bytes.extend_from_slice(&image.width.to_le_bytes());
    bytes.extend_from_slice(&image.height.to_le_bytes());

    let Some((&first, rest)) = image.pixels.split_first() else {
        return bytes;
    };

    let mut current = first;
    let mut count = 1u8;
    for &pixel in rest {
        if pixel == current && count < 0x7f {
            count += 1;
            continue;
        }

        push_run(&mut bytes, current, count);
        current = pixel;
        count = 1;
    }
    push_run(&mut bytes, current, count);

    bytes
}

pub fn load_rle(path: impl AsRef<Path>) -> eyre::Result<MultimapRleImage> {
    let path = path.as_ref();
    let bytes = fs::read(path)
        .wrap_err_with(|| format!("Failed to read multimap RLE {}", path.display()))?;
    decode_rle_bytes(&bytes)
}

pub fn save_rle(path: impl AsRef<Path>, image: &MultimapRleImage) -> eyre::Result<()> {
    let path = path.as_ref();
    fs::write(path, encode_rle_bytes(image))
        .wrap_err_with(|| format!("Failed to write multimap RLE {}", path.display()))
}

pub fn load_bitmap_or_png(path: impl AsRef<Path>) -> eyre::Result<MultimapRleImage> {
    let path = path.as_ref();
    let image = image::open(path)
        .wrap_err_with(|| format!("Failed to load multimap image {}", path.display()))?
        .to_rgba8();
    MultimapRleImage::from_rgba8(image.width(), image.height(), image.as_raw())
}

pub fn save_bitmap_or_png(path: impl AsRef<Path>, image: &MultimapRleImage) -> eyre::Result<()> {
    let path = path.as_ref();
    let format = output_format(path)?;
    let rgba = image.to_rgba8();
    image::save_buffer_with_format(path, &rgba, image.width, image.height, ColorType::Rgba8, format)
        .wrap_err_with(|| format!("Failed to write multimap image {}", path.display()))
}

fn pixel_len(width: u32, height: u32) -> eyre::Result<usize> {
    if width == 0 || height == 0 {
        eyre::bail!("multimap dimensions must be non-zero, got {}x{}", width, height);
    }
    width
        .checked_mul(height)
        .and_then(|pixels| usize::try_from(pixels).ok())
        .ok_or_else(|| eyre::eyre!("multimap dimensions are too large: {}x{}", width, height))
}

fn classify_rgba_pixel(r: u8, g: u8, b: u8, a: u8) -> u8 {
    if a < 128 {
        return WHITE_PIXEL;
    }

    let luminance = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114;
    if luminance >= 128_000 {
        WHITE_PIXEL
    } else {
        BLACK_PIXEL
    }
}

fn push_run(bytes: &mut Vec<u8>, pixel: u8, count: u8) {
    let mask = if pixel == BLACK_PIXEL { 0x80 } else { 0 };
    bytes.push(mask | count);
}

fn output_format(path: &Path) -> eyre::Result<ImageFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("bmp") => Ok(ImageFormat::Bmp),
        Some("png") => Ok(ImageFormat::Png),
        _ => eyre::bail!("output image extension must be .bmp or .png: {}", path.display()),
    }
}
