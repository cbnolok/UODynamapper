//! Texture payloads of `effects_ec.uddp` decoded to 8-bit RGBA levels: for the texture view (one level over a
//! checker background) and for the preview (every level as a particle-sim [`RasterTexture`]).
//!
//! The package stores RGBA8, BC1, BC3 and BC7 levels back to back, level 0 first (`texture_payload_len`). The BC
//! formats decode with the `dds` crate, the decoder udd-assets and udd-conv use for BC7.

use std::io::Cursor;

use particle_sim::raster::RasterTexture;
use udd_assets::effects_ec::{
    RGBA8_BYTES_PER_PIXEL, TEXTURE_FORMAT_BC1, TEXTURE_FORMAT_BC3, TEXTURE_FORMAT_BC7, TEXTURE_FORMAT_NONE,
    TEXTURE_FORMAT_RGBA8, TextureData, mip_extent, texture_level_len,
};

/// Side of a checker square of the texture view, pixels.
pub const CHECKER_SQUARE_PX: u32 = 8;
/// Light checker colour (RGB).
const CHECKER_LIGHT: [u8; 3] = [204, 204, 204];
/// Dark checker colour (RGB).
const CHECKER_DARK: [u8; 3] = [140, 140, 140];
/// Index of the alpha channel of an RGBA8 pixel.
const ALPHA: usize = 3;
/// Opaque 8-bit alpha.
const OPAQUE_ALPHA: u8 = u8::MAX;

/// One decoded mip level, 8-bit RGBA rows, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Display name of a `TEXTURE_FORMAT_*`.
pub fn format_name(format: u8) -> &'static str {
    match format {
        TEXTURE_FORMAT_NONE => "none",
        TEXTURE_FORMAT_RGBA8 => "RGBA8",
        TEXTURE_FORMAT_BC7 => "BC7",
        TEXTURE_FORMAT_BC1 => "BC1",
        TEXTURE_FORMAT_BC3 => "BC3",
        _ => "unknown",
    }
}

/// The `dds` format of a block-compressed `TEXTURE_FORMAT_*`.
fn block_format(format: u8) -> Option<dds::Format> {
    match format {
        TEXTURE_FORMAT_BC1 => Some(dds::Format::BC1_UNORM),
        TEXTURE_FORMAT_BC3 => Some(dds::Format::BC3_UNORM),
        TEXTURE_FORMAT_BC7 => Some(dds::Format::BC7_UNORM),
        _ => None,
    }
}

/// Decodes one level of `format` to RGBA8.
pub fn decode_level(format: u8, width: u32, height: u32, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let expected = texture_level_len(format, width, height)
        .ok_or_else(|| format!("texture format {format} has no pixels"))?;
    if bytes.len() != expected {
        return Err(format!("{width}x{height} {} level: {} bytes, expected {expected}", format_name(format), bytes.len()));
    }
    if format == TEXTURE_FORMAT_RGBA8 {
        return Ok(bytes.to_vec());
    }
    let block = block_format(format).ok_or_else(|| format!("texture format {format} does not decode"))?;
    let mut rgba = vec![0u8; width as usize * height as usize * RGBA8_BYTES_PER_PIXEL];
    let image = dds::ImageViewMut::new(&mut rgba, dds::Size::new(width, height), dds::ColorFormat::RGBA_U8)
        .ok_or_else(|| format!("{width}x{height}: no RGBA8 view"))?;
    dds::decode(&mut Cursor::new(bytes), image, block, &dds::DecodeOptions::default())
        .map_err(|error| format!("{} decode: {error}", format_name(format)))?;
    Ok(rgba)
}

/// Every level of a payload decoded to RGBA8, level 0 first.
pub fn decode_levels(texture: &TextureData) -> Result<Vec<DecodedLevel>, String> {
    let mut offset = 0usize;
    let mut levels = Vec::with_capacity(texture.mip_count as usize);
    for level in 0..texture.mip_count {
        let (width, height) = mip_extent(texture.width, texture.height, level);
        let len = texture_level_len(texture.format, width, height)
            .ok_or_else(|| format!("texture format {} has no pixels", texture.format))?;
        let bytes = offset
            .checked_add(len)
            .and_then(|end| texture.data.get(offset..end))
            .ok_or_else(|| format!("level {level} is past the end of the payload ({} bytes)", texture.data.len()))?;
        levels.push(DecodedLevel { width, height, rgba: decode_level(texture.format, width, height, bytes)? });
        offset += len;
    }
    Ok(levels)
}

/// A payload as the preview samples it: every stored level (the package holds the client's mip levels).
pub fn raster_texture(texture: &TextureData) -> Result<RasterTexture, String> {
    let levels = decode_levels(texture)?;
    let rgba: Vec<&[u8]> = levels.iter().map(|level| level.rgba.as_slice()).collect();
    RasterTexture::from_rgba8_levels(texture.width as usize, texture.height as usize, &rgba)
        .ok_or_else(|| format!("{}x{}: bad texture size", texture.width, texture.height))
}

/// A level drawn over a checker background (straight alpha), or with its alpha ignored: opaque RGBA8.
pub fn checker_composite(level: &DecodedLevel, show_alpha: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(level.rgba.len());
    for (index, pixel) in level.rgba.chunks_exact(RGBA8_BYTES_PER_PIXEL).enumerate() {
        let (x, y) = (index as u32 % level.width.max(1), index as u32 / level.width.max(1));
        let background = if (x / CHECKER_SQUARE_PX + y / CHECKER_SQUARE_PX) % 2 == 0 { CHECKER_LIGHT } else { CHECKER_DARK };
        let alpha = if show_alpha { u32::from(pixel[ALPHA]) } else { u32::from(OPAQUE_ALPHA) };
        let max = u32::from(OPAQUE_ALPHA);
        for channel in 0..ALPHA {
            let blended = (u32::from(pixel[channel]) * alpha + u32::from(background[channel]) * (max - alpha) + max / 2) / max;
            out.push(blended as u8);
        }
        out.push(OPAQUE_ALPHA);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BC1 block whose two colours are white (0xffff) and black, every index 0: all white, opaque.
    const WHITE_BC1_BLOCK: [u8; 8] = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
    /// Side of the BC test textures.
    const BLOCK_SIDE: u32 = 4;

    #[test]
    fn rgba8_levels_split_in_order() {
        let level0 = vec![10u8; 2 * 2 * RGBA8_BYTES_PER_PIXEL];
        let level1 = vec![20u8; RGBA8_BYTES_PER_PIXEL];
        let data = [level0.clone(), level1.clone()].concat();
        let texture = TextureData { width: 2, height: 2, format: TEXTURE_FORMAT_RGBA8, mip_count: 2, data };
        let levels = decode_levels(&texture).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[0].width, levels[0].height, &levels[0].rgba), (2, 2, &level0));
        assert_eq!((levels[1].width, levels[1].height, &levels[1].rgba), (1, 1, &level1));
        assert_eq!(raster_texture(&texture).unwrap().level_count(), 2);
    }

    #[test]
    fn bc1_decodes_and_short_payloads_fail() {
        let texture = TextureData {
            width: BLOCK_SIDE,
            height: BLOCK_SIDE,
            format: TEXTURE_FORMAT_BC1,
            mip_count: 1,
            data: WHITE_BC1_BLOCK.to_vec(),
        };
        let levels = decode_levels(&texture).unwrap();
        assert!(levels[0].rgba.iter().all(|&byte| byte == u8::MAX), "white and opaque");
        let short = TextureData { data: WHITE_BC1_BLOCK[..4].to_vec(), ..texture.clone() };
        assert!(decode_levels(&short).is_err());
        let two_levels = TextureData { mip_count: 2, ..texture };
        assert!(decode_levels(&two_levels).is_err(), "level 1 is missing");
        assert!(decode_level(TEXTURE_FORMAT_NONE, 1, 1, &[]).is_err());
    }

    #[test]
    fn checker_shows_through_transparent_pixels() {
        let level = DecodedLevel { width: 1, height: 1, rgba: vec![255, 0, 0, 0] };
        assert_eq!(checker_composite(&level, true), [CHECKER_LIGHT[0], CHECKER_LIGHT[1], CHECKER_LIGHT[2], OPAQUE_ALPHA]);
        assert_eq!(checker_composite(&level, false), [255, 0, 0, OPAQUE_ALPHA]);
    }
}
