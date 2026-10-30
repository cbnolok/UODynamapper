//! The mip levels the client's textures have, shared by the effects packer and the previews.
//!
//! Gamebryo 2.6 DX9 renderer, as UOSA.exe runs it (`uosa_effects.md`, "Effect textures and mipmaps"):
//! - `NiDX9SourceTextureData::Create`: a texture is mipmapped when its `NiSourceTexture` prefs say `YES`
//!   (`MIP_DEFAULT` would follow `NiTexture::ms_bMipmapByDefault`, which the client leaves false).
//! - `NiDevImageConverter::ConvertPixelData` (`NiDevImageConverter.cpp:446-470`): stored levels are kept as they
//!   are, mipmapped or not; a mipmapped single-level source with power-of-two sides and an RGB(A) destination
//!   gets a generated chain ([`generate_mip_chain`]).
//! - `NiDX9TextureManager::PrepareTextureForRendering` and `NiD3DTextureStage::ModifyFilterMode`: the mip filter
//!   of a texture with one level is off; with more levels the map's filter mode applies.
//! - `NiDDSReader::ReadHeader` (`NiDDSReader.cpp:313-323`): a DDS file's levels are kept only when
//!   `DDSCAPS_COMPLEX` and `DDSCAPS_MIPMAP` are set and the count is not 1 ([`dds_stored_levels`]).

/// Bytes of one RGBA8 pixel.
pub const RGBA8_BYTES_PER_PIXEL: usize = 4;
/// Byte offset of `dwMipMapCount` in a DDS file (after the 4-byte magic).
const DDS_MIP_COUNT_OFFSET: usize = 28;
/// Byte offset of `dwCaps` in a DDS file.
const DDS_CAPS_OFFSET: usize = 108;
/// `DDSCAPS_COMPLEX`.
const DDSCAPS_COMPLEX: u32 = 0x8;
/// `DDSCAPS_MIPMAP`.
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

/// The levels `NiDDSReader::ReadHeader` keeps of a DDS file: its mip count when `DDSCAPS_COMPLEX` and
/// `DDSCAPS_MIPMAP` are set and the count is not 1, else 1. `None` when the header is truncated.
pub fn dds_stored_levels(bytes: &[u8]) -> Option<u32> {
    let read_u32 = |offset: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
    };
    let caps = read_u32(DDS_CAPS_OFFSET)?;
    let count = read_u32(DDS_MIP_COUNT_OFFSET)?;
    Some(if caps & DDSCAPS_COMPLEX != 0 && caps & DDSCAPS_MIPMAP != 0 && count != 1 { count.max(1) } else { 1 })
}

/// A DDS file's levels as `NiDDSReader` keeps them ([`dds_stored_levels`]), decoded to RGBA8, level 0 first.
#[cfg(feature = "dds")]
pub struct DdsLevels {
    pub width: u32,
    pub height: u32,
    /// The file's pixel format.
    pub format: dds::Format,
    pub levels: Vec<Vec<u8>>,
}

#[cfg(feature = "dds")]
impl DdsLevels {
    /// Whether the client would generate a chain from a single level ([`generates_mip_chain`]): not for a DXT1 /
    /// DXT5 source, which stays compressed (chains are generated only for RGB(A) destinations).
    pub fn generatable(&self) -> bool {
        !matches!(self.format, dds::Format::BC1_UNORM | dds::Format::BC3_UNORM)
    }
}

/// Decodes the levels of a DDS file that `NiDDSReader::ReadHeader` keeps ([`dds_stored_levels`]) to RGBA8.
#[cfg(feature = "dds")]
pub fn decode_dds_levels(bytes: &[u8]) -> Result<DdsLevels, String> {
    let count = dds_stored_levels(bytes).ok_or_else(|| "truncated DDS header".to_owned())?;
    let mut decoder = dds::Decoder::new(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let size = decoder.main_size();
    let format = decoder.format();
    let mut levels = Vec::new();
    for _ in 0..count {
        let size = decoder.surface_info().ok_or_else(|| "fewer surfaces than mip levels".to_owned())?.size();
        let length = dds::ColorFormat::RGBA_U8.buffer_size(size).ok_or_else(|| "DDS too large".to_owned())?;
        let mut rgba = vec![0u8; length];
        let view =
            dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8).ok_or_else(|| "DDS buffer".to_owned())?;
        decoder.read_surface(view).map_err(|error| error.to_string())?;
        levels.push(rgba);
    }
    Ok(DdsLevels { width: size.width, height: size.height, format, levels })
}

/// The full mip chain of a power-of-two RGBA8 image as `NiDevImageConverter::GenerateMipmapLevels` builds it
/// (`CoreLibs/NiMain/NiDevImageConverter.cpp:1108-1290`): each level from the previous one by a truncating 2x2
/// box (`(a + b + c + d) >> 2`), or `(a + b) >> 1` when the previous level is one pixel wide or high, down to
/// 1x1. Level 0 first.
pub fn generate_mip_chain(width: u32, height: u32, level0: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![level0];
    let (mut fine_width, mut fine_height) = (width as usize, height as usize);
    while fine_width > 1 || fine_height > 1 {
        let fine = levels.last().expect("level 0");
        let (coarse_width, coarse_height) = ((fine_width / 2).max(1), (fine_height / 2).max(1));
        let pixel = |x: usize, y: usize| &fine[(y * fine_width + x) * RGBA8_BYTES_PER_PIXEL..][..RGBA8_BYTES_PER_PIXEL];
        let mut coarse = Vec::with_capacity(coarse_width * coarse_height * RGBA8_BYTES_PER_PIXEL);
        for y in 0..coarse_height {
            for x in 0..coarse_width {
                for channel in 0..RGBA8_BYTES_PER_PIXEL {
                    let value = |x: usize, y: usize| u32::from(pixel(x, y)[channel]);
                    let average = if fine_width == 1 {
                        (value(0, 2 * y) + value(0, 2 * y + 1)) >> 1
                    } else if fine_height == 1 {
                        (value(2 * x, 0) + value(2 * x + 1, 0)) >> 1
                    } else {
                        (value(2 * x, 2 * y) + value(2 * x + 1, 2 * y) + value(2 * x, 2 * y + 1) + value(2 * x + 1, 2 * y + 1))
                            >> 2
                    };
                    coarse.push(average as u8);
                }
            }
        }
        levels.push(coarse);
        (fine_width, fine_height) = (coarse_width, coarse_height);
    }
    levels
}

/// Whether `ConvertPixelData` generates a chain: a mipmapped single-level source with power-of-two sides whose
/// destination format is RGB(A) (`generatable`: an uncompressed source, which the client converts to RGB(A)).
pub fn generates_mip_chain(width: u32, height: u32, stored_levels: usize, mipmapped: bool, generatable: bool) -> bool {
    stored_levels == 1 && mipmapped && generatable && width.is_power_of_two() && height.is_power_of_two()
}

/// The RGBA8 levels the client's texture has: the stored `levels` as they are, or the generated chain of a
/// single level ([`generates_mip_chain`]). Returns the levels and whether they were generated.
pub fn client_mip_levels(
    width: u32,
    height: u32,
    mut levels: Vec<Vec<u8>>,
    mipmapped: bool,
    generatable: bool,
) -> (Vec<Vec<u8>>, bool) {
    if !generates_mip_chain(width, height, levels.len(), mipmapped, generatable) {
        return (levels, false);
    }
    let level0 = levels.pop().expect("one level");
    (generate_mip_chain(width, height, level0), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_halve_down_to_one_pixel_with_truncating_boxes() {
        let level0 = [10u8, 20, 30, 40, 11, 21, 31, 41, 12, 22, 32, 42, 13, 23, 33, 43].to_vec();
        let (levels, generated) = client_mip_levels(4, 1, vec![level0], true, true);
        assert!(generated);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[1], [10, 20, 30, 40, 12, 22, 32, 42]);
        assert_eq!(levels[2], [11, 21, 31, 41]);
    }

    #[test]
    fn stored_levels_and_unmipmapped_sources_are_kept() {
        let one = vec![vec![0u8; 16]];
        assert_eq!(client_mip_levels(2, 2, one.clone(), false, true), (one.clone(), false));
        assert_eq!(client_mip_levels(2, 2, one.clone(), true, false), (one.clone(), false));
        let not_pow2 = vec![vec![0u8; 12]];
        assert_eq!(client_mip_levels(3, 1, not_pow2.clone(), true, true), (not_pow2, false));
        let stored = vec![vec![0u8; 16], vec![9u8; 4]];
        assert_eq!(client_mip_levels(2, 2, stored.clone(), true, true), (stored, false));
    }

    #[test]
    fn dds_levels_need_both_caps() {
        let header = |caps: u32, count: u32| {
            let mut bytes = vec![0u8; DDS_CAPS_OFFSET + 4];
            bytes[DDS_MIP_COUNT_OFFSET..][..4].copy_from_slice(&count.to_le_bytes());
            bytes[DDS_CAPS_OFFSET..][..4].copy_from_slice(&caps.to_le_bytes());
            bytes
        };
        assert_eq!(dds_stored_levels(&header(DDSCAPS_COMPLEX | DDSCAPS_MIPMAP, 7)), Some(7));
        assert_eq!(dds_stored_levels(&header(DDSCAPS_MIPMAP, 7)), Some(1));
        assert_eq!(dds_stored_levels(&header(DDSCAPS_COMPLEX | DDSCAPS_MIPMAP, 0)), Some(1));
        assert_eq!(dds_stored_levels(&[0u8; 8]), None);
    }
}
