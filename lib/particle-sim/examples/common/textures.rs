//! Texture loading shared by the preview examples: the client's texture files decoded with the levels its
//! readers keep, then the mip levels the client builds from them ([`particle_sim::mips`], the packer's rules).
//!
//! - DDS: the levels `NiDDSReader::ReadHeader` keeps ([`decode_dds_levels`]); a BC1 / BC3 source is compressed,
//!   so no chain is generated for it (the packer's rule).
//! - `.nif`: one `NiPixelData` with its stored levels ([`decode_nif_image`], `NiNIFImageReader`).
//! - Anything else: TGA, one level.

use particle_sim::mips::{client_mip_levels, decode_dds_levels};
use particle_sim::nif::compile::decode_nif_image;
use particle_sim::raster::RasterTexture;

const DDS_MAGIC: &[u8] = b"DDS ";
const NIF_EXTENSION: &str = ".nif";

/// Decoded RGBA8 levels, level 0 first, and whether the client would generate a chain from a single level of
/// this source (an uncompressed one).
struct Levels {
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
    generatable: bool,
}

fn decode_levels(name: &str, payload: &[u8]) -> Result<Levels, String> {
    if payload.starts_with(DDS_MAGIC) {
        let dds = decode_dds_levels(payload)?;
        let generatable = dds.generatable();
        return Ok(Levels { width: dds.width, height: dds.height, levels: dds.levels, generatable });
    }
    if name.to_ascii_lowercase().ends_with(NIF_EXTENSION) {
        let levels = decode_nif_image(payload)?;
        let (width, height) = (levels[0].0, levels[0].1);
        return Ok(Levels { width, height, levels: levels.into_iter().map(|(_, _, rgba)| rgba).collect(), generatable: true });
    }
    let image = image::load_from_memory_with_format(payload, image::ImageFormat::Tga)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok(Levels { width: image.width(), height: image.height(), levels: vec![image.into_raw()], generatable: true })
}

/// A texture file of the client's packages with the client's mip levels; `mipmapped`: the referencing
/// `NiSourceTexture` prefs say `YES` (Chaos textures are not mipmapped).
pub fn load_texture(name: &str, payload: &[u8], mipmapped: bool) -> Result<RasterTexture, String> {
    let Levels { width, height, levels, generatable } = decode_levels(name, payload)?;
    raster_texture(width, height, levels, mipmapped, generatable)
}

/// A texture of RGBA8 `levels` with the client's mip levels.
pub fn raster_texture(
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
    mipmapped: bool,
    generatable: bool,
) -> Result<RasterTexture, String> {
    let (levels, _) = client_mip_levels(width, height, levels, mipmapped, generatable);
    RasterTexture::from_rgba8_levels(width as usize, height as usize, &levels).ok_or_else(|| "empty texture".to_owned())
}
