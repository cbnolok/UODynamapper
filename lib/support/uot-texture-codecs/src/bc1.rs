//! BC1 blocks through the `dds` crate: 4x4 pixels in 8 bytes, colour in two RGB565 endpoints and one bit of alpha.
//!
//! A pixel with alpha below one half is transparent: its block takes BC1's three-colour mode, whose fourth index
//! decodes to transparent black. That is how Classic draws colour 0 of its gumps, so they lose nothing to the one
//! bit; Enhanced gumps keep straight (not premultiplied) colour, and any soft alpha edge becomes a hard one.
//!
//! The image keeps its true size: the blocks cover it rounded up to 4 on each side, and the decoder crops them back.

use dds::{ColorFormat, CompressionQuality, DecodeOptions, Dithering, EncodeOptions, Format, ImageView, ImageViewMut, Size};
use std::io::Cursor;

use crate::bc7::{ImageExtent, RawImageFormat, TextureError};

/// Pixels per side of a BC1 block, and bytes per block.
pub const BC1_BLOCK_EDGE: u32 = 4;
pub const BC1_BLOCK_BYTES: usize = 8;

/// The bytes of BC1 blocks covering `extent`, rounded up to whole blocks.
pub const fn expected_bc1_byte_len(extent: ImageExtent) -> usize {
    extent.blocks_wide() as usize * extent.blocks_high() as usize * BC1_BLOCK_BYTES
}

/// The encoder settings of the lossy gump measurements: the best quality, no dithering, straight colour. One image
/// encodes on the caller's thread; packers run images side by side.
fn bc1_encode_options() -> EncodeOptions {
    let mut options = EncodeOptions::default();
    options.quality = CompressionQuality::High;
    options.dithering = Dithering::None;
    options.parallel = false;
    options
}

/// Encodes RGBA8888 pixels of `extent` to BC1 blocks, row by row of blocks.
pub fn encode_bc1(rgba: &[u8], extent: ImageExtent) -> Result<Vec<u8>, TextureError> {
    let expected = extent.byte_len(RawImageFormat::Rgba8888);
    if rgba.len() != expected {
        return Err(TextureError::InvalidInputLength { expected, actual: rgba.len(), format: RawImageFormat::Rgba8888 });
    }
    let image = ImageView::new(rgba, Size::new(extent.width(), extent.height()), ColorFormat::RGBA_U8).ok_or_else(|| {
        backend_error("encode", "failed to construct the RGBA8 input view".to_string())
    })?;
    let mut blocks = Vec::with_capacity(expected_bc1_byte_len(extent));
    dds::encode(&mut blocks, image, Format::BC1_UNORM, None, &bc1_encode_options())
        .map_err(|error| backend_error("encode", error.to_string()))?;
    if blocks.len() != expected_bc1_byte_len(extent) {
        return Err(TextureError::InvalidBc1Length { expected: expected_bc1_byte_len(extent), actual: blocks.len() });
    }
    Ok(blocks)
}

/// Decodes BC1 blocks covering `extent` to its RGBA8888 pixels; transparent texels are (0, 0, 0, 0).
pub fn decode_bc1_to_rgba8888(blocks: &[u8], extent: ImageExtent) -> Result<Vec<u8>, TextureError> {
    let expected = expected_bc1_byte_len(extent);
    if blocks.len() != expected {
        return Err(TextureError::InvalidBc1Length { expected, actual: blocks.len() });
    }
    let mut rgba = vec![0u8; extent.byte_len(RawImageFormat::Rgba8888)];
    let image = ImageViewMut::new(&mut rgba, Size::new(extent.width(), extent.height()), ColorFormat::RGBA_U8)
        .ok_or_else(|| backend_error("decode", "failed to construct the RGBA8 output view".to_string()))?;
    dds::decode(&mut Cursor::new(blocks), image, Format::BC1_UNORM, &DecodeOptions::default())
        .map_err(|error| backend_error("decode", error.to_string()))?;
    Ok(rgba)
}

fn backend_error(operation: &'static str, message: String) -> TextureError {
    TextureError::BackendOperationFailed { backend: "dds BC1", operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPAQUE: u8 = u8::MAX;
    const TRANSPARENT: u8 = 0;
    /// The largest error a flat colour takes through RGB565 endpoints: 8 levels in red and blue.
    const MAX_FLAT_COLOUR_ERROR: u8 = 8;

    fn flat(extent: ImageExtent, pixel: [u8; 4]) -> Vec<u8> {
        pixel.repeat(extent.width() as usize * extent.height() as usize)
    }

    fn close(left: &[u8], right: &[u8]) -> bool {
        left.iter().zip(right).all(|(left, right)| left.abs_diff(*right) <= MAX_FLAT_COLOUR_ERROR)
    }

    #[test]
    fn a_flat_opaque_image_round_trips_within_rgb565() {
        const SIDE: u32 = 8;
        const COLOUR: [u8; 4] = [200, 120, 40, OPAQUE];
        let extent = ImageExtent::new(SIDE, SIDE).unwrap();
        let rgba = flat(extent, COLOUR);

        let blocks = encode_bc1(&rgba, extent).unwrap();
        assert_eq!(blocks.len(), expected_bc1_byte_len(extent));
        let decoded = decode_bc1_to_rgba8888(&blocks, extent).unwrap();
        assert_eq!(decoded.len(), rgba.len());
        assert!(close(&decoded, &rgba));
    }

    /// Classic's colour 0 and Enhanced's clear texels, whatever colour they hold, decode to transparent black; the
    /// painted texels of the same blocks stay opaque.
    #[test]
    fn transparent_texels_decode_to_transparent_black() {
        const SIDE: u32 = 4;
        const PAINTED: [u8; 4] = [90, 180, 30, OPAQUE];
        const CLASSIC_COLOUR_ZERO: [u8; 4] = [0, 0, 0, TRANSPARENT];
        const STRAIGHT_CLEAR: [u8; 4] = [250, 10, 10, 100];
        let extent = ImageExtent::new(SIDE, SIDE).unwrap();
        let mut rgba = flat(extent, PAINTED);
        rgba[..4].copy_from_slice(&CLASSIC_COLOUR_ZERO);
        rgba[4..8].copy_from_slice(&STRAIGHT_CLEAR);

        let decoded = decode_bc1_to_rgba8888(&encode_bc1(&rgba, extent).unwrap(), extent).unwrap();
        assert_eq!(decoded[..8], [0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(decoded[8..].chunks_exact(4).all(|pixel| pixel[3] == OPAQUE && close(pixel, &PAINTED)));
    }

    /// An image whose sides aren't multiples of 4 takes whole blocks and decodes back to its own size.
    #[test]
    fn odd_sizes_round_up_to_whole_blocks() {
        const COLOUR: [u8; 4] = [10, 220, 140, OPAQUE];
        for (width, height) in [(1, 1), (3, 5), (5, 3), (7, 9), (13, 2)] {
            let extent = ImageExtent::new(width, height).unwrap();
            let rgba = flat(extent, COLOUR);
            let blocks = encode_bc1(&rgba, extent).unwrap();
            assert_eq!(
                blocks.len(),
                width.div_ceil(BC1_BLOCK_EDGE) as usize * height.div_ceil(BC1_BLOCK_EDGE) as usize * BC1_BLOCK_BYTES
            );
            let decoded = decode_bc1_to_rgba8888(&blocks, extent).unwrap();
            assert_eq!(decoded.len(), rgba.len(), "{width}x{height}");
            assert!(close(&decoded, &rgba), "{width}x{height}");
        }
    }

    #[test]
    fn wrong_lengths_are_errors() {
        let extent = ImageExtent::new(4, 4).unwrap();
        assert!(encode_bc1(&[0; 3], extent).is_err());
        assert!(decode_bc1_to_rgba8888(&[0; BC1_BLOCK_BYTES + 1], extent).is_err());
    }
}
