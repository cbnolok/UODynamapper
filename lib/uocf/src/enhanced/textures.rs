//! Decoding for UO Enhanced Client texture files (`Texture.uop`).
//!
//! This module handles the parsing of texture metadata and extraction of texture image data
//! from the `Texture.uop` file. The textures can be in TGA or DDS format.

use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::tileart::{self, ArtData};
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};
use byteorder::{LittleEndian, ReadBytesExt};
use image::{DynamicImage, ImageBuffer, RgbaImage};
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

crate::eyre_imports!();

// region: --- Internal Raw File-Mapping Structs (Binary Format)

/// Raw metadata header for a texture entry as stored in `Texture.uop`.
///
/// This structure mirrors the binary layout of the texture metadata, which is the
/// tileart shader block layout (see [`tileart::TaeTexture`]). It is used internally
/// for parsing and should not be used directly by high-level rendering code.
#[derive(Debug, Clone)]
pub struct TextureItem {
    pub texture_present: bool,
    pub field_04: u8,
    pub name_index: i32,
    pub images: Vec<TextureItemImage>,
    pub technique: Vec<i32>,
    pub tweaks: Vec<f32>,
}

/// Represents an image within a texture item's metadata.
#[derive(Debug, Clone)]
pub struct TextureItemImage {
    pub string_dictionary_offset: i32,
    pub source_kind: u8,
    pub texture_repetition: f32,
    pub sampler_index: i32,
    pub uv_set: i32,
}

impl TextureItem {
    pub fn absent() -> Self {
        Self {
            texture_present: false,
            field_04: 0,
            name_index: 0,
            images: vec![],
            technique: vec![],
            tweaks: vec![],
        }
    }

    /// Reads a `TextureItem` from a binary reader.
    pub fn read<R: Read>(reader: &mut R) -> eyre::Result<Self> {
        let texture_present = reader.read_u8()? != 0;
        if !texture_present {
            return Ok(Self::absent());
        }

        let field_04 = reader.read_u8()?;
        let name_index = reader.read_i32::<LittleEndian>()?;
        let count1 = reader.read_u8()?;
        let mut images = Vec::with_capacity(count1 as usize);
        for _ in 0..count1 {
            images.push(TextureItemImage {
                string_dictionary_offset: reader.read_i32::<LittleEndian>()?,
                source_kind: reader.read_u8()?,
                texture_repetition: reader.read_f32::<LittleEndian>()?,
                sampler_index: reader.read_i32::<LittleEndian>()?,
                uv_set: reader.read_i32::<LittleEndian>()?,
            });
        }

        let count2 = reader.read_u32::<LittleEndian>()?;
        let mut technique = Vec::with_capacity(count2 as usize);
        for _ in 0..count2 {
            technique.push(reader.read_i32::<LittleEndian>()?);
        }

        let count3 = reader.read_u32::<LittleEndian>()?;
        let mut tweaks = Vec::with_capacity(count3 as usize);
        for _ in 0..count3 {
            tweaks.push(reader.read_f32::<LittleEndian>()?);
        }

        Ok(Self {
            texture_present: true,
            field_04,
            name_index,
            images,
            technique,
            tweaks,
        })
    }
}

fn payload_starts_with_raw_image(data: &[u8], format: ECImageFormat) -> bool {
    match format {
        ECImageFormat::DDS => data.starts_with(b"DDS "),
        ECImageFormat::TGA => is_probably_tga(data),
        ECImageFormat::Unknown => data.starts_with(b"DDS ") || is_probably_tga(data),
    }
}

fn is_probably_tga(data: &[u8]) -> bool {
    if data.len() < 18 {
        return false;
    }

    let image_type = data[2];
    (image_type == 2 || image_type == 10) && data[1] <= 1
}

// endregion: --- Internal Raw File-Mapping Structs

// region: --- Public API (Convenience & Application Use)

/// Inferred image format used to decide how to handle the raw binary payload.
///
/// This is a convenience abstraction. The format is typically inferred from the
/// file extension within the UOP package or by inspecting the magic bytes of the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ECImageFormat {
    DDS,
    TGA,
    Unknown,
}

/// Represents a fully mapped texture file containing both its metadata header
/// and a zero-copy thread-safe slice of its binary layout.
/// A fully resolved texture asset ready for consumption by the engine.
///
/// This is a public convenience structure that aggregates the raw binary payload
/// with its associated metadata and inferred format. It provides a clean interface
/// for the renderer to access raw bytes for GPU upload or perform CPU decoding.
#[derive(Debug, Clone)]
pub struct TextureFile {
    pub metadata: TextureItem,
    pub is_ec: bool,
    pub format: ECImageFormat,
    /// Detailed Tile Art metadata if linked against `tileart.uop` definitions.
    pub props: Option<ArtData>,
    /// The unpacked UOP payload. `image_data_offset` skips optional texture metadata
    /// so `raw_bytes()` can expose the DDS/TGA bytes without copying them.
    /// This saves ~8x memory space and makes it extremely fast to upload to GPU VRAM mappings.
    pub raw_data: Arc<[u8]>,
    pub image_data_offset: usize,
}

impl TextureFile {
    /// Provides access to textures natively mapped in UOP layout without
    /// decompressing the format through `dds` to raw RGBA.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_data[self.image_data_offset..]
    }

    /// CPU Software Decompression fallback primarily reserved for tooling or legacy UI extraction.
    /// Engine renderer (e.g., Bevy) should strictly utilize `raw_bytes` and forward the BC blocks payload
    /// directly to VRAM relying on `format`.
    pub fn decode_to_rgba(&self) -> eyre::Result<DynamicImage> {
        Ok(DynamicImage::ImageRgba8(self.decode_to_rgba8()?))
    }

    pub fn decode_to_rgba8(&self) -> eyre::Result<RgbaImage> {
        match self.format {
            ECImageFormat::DDS => decode_dds_rgba8(self.raw_bytes()),
            ECImageFormat::TGA => decode_tga_rgba8(self.raw_bytes()),
            ECImageFormat::Unknown => {
                eyre::bail!("Unknown image format payload")
            }
        }
    }
}

/// DX9 bit count of X1R5G5B5 DDS images (four 2048x1024 Kingdom Reborn `Interface.uop` images).
const X1R5G5B5_BIT_COUNT: dds::header::RgbBitCount = dds::header::RgbBitCount::Count16;
/// Red, green and blue masks of X1R5G5B5; the top bit is unused (no alpha mask).
const X1R5G5B5_RGB_MASKS: [u32; 3] = [0x7C00, 0x03E0, 0x001F];
/// Size of a TGA header; the width and height are the u16 at [`TGA_WIDTH_OFFSET`] and [`TGA_HEIGHT_OFFSET`].
const TGA_HEADER_SIZE: usize = 18;
const TGA_WIDTH_OFFSET: usize = 12;
const TGA_HEIGHT_OFFSET: usize = 14;
/// Alpha of an opaque RGBA pixel.
const OPAQUE_ALPHA: u8 = u8::MAX;

/// Whether `header` is X1R5G5B5, which the `dds` crate does not detect: it reads as B5G5R5A1 with the alpha bit
/// ignored.
fn is_x1r5g5b5(header: &dds::header::Header) -> bool {
    let dds::header::Header::Dx9(header) = header else { return false };
    let dds::header::Dx9PixelFormat::Mask(mask) = &header.pixel_format else { return false };
    mask.rgb_bit_count == X1R5G5B5_BIT_COUNT
        && [mask.r_bit_mask, mask.g_bit_mask, mask.b_bit_mask] == X1R5G5B5_RGB_MASKS
        && mask.a_bit_mask == 0
}

/// Decodes the main surface of a DDS file to RGBA, including X1R5G5B5 (opaque).
pub fn decode_dds_rgba8(bytes: &[u8]) -> eyre::Result<RgbaImage> {
    let mut cursor = Cursor::new(bytes);
    let header = dds::header::Header::read(&mut cursor, &dds::header::ParseOptions::default())
        .wrap_err("Failed reading DDS header")?;
    let opaque = is_x1r5g5b5(&header);
    let mut decoder = if opaque {
        dds::Decoder::from_header_with(cursor, header, dds::Format::B5G5R5A1_UNORM)
    } else {
        dds::Decoder::from_header(cursor, header)
    }
    .wrap_err("Failed reading DDS header")?;
    let size = decoder.main_size();
    let rgba_len = dds::ColorFormat::RGBA_U8
        .buffer_size(size)
        .ok_or_else(|| eyre::eyre!("DDS image is too large to decode"))?;
    let mut rgba = vec![0u8; rgba_len];
    let image = dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8)
        .ok_or_else(|| eyre::eyre!("Could not create DDS decode buffer"))?;
    decoder.read_surface(image).wrap_err("Failed decoding DDS image")?;
    if opaque {
        rgba.chunks_exact_mut(dds::ColorFormat::RGBA_U8.bytes_per_pixel() as usize)
            .for_each(|pixel| pixel[pixel.len() - 1] = OPAQUE_ALPHA);
    }
    ImageBuffer::from_raw(size.width, size.height, rgba).ok_or_else(|| eyre::eyre!("Could not map buffer"))
}

/// Decodes a TGA file to RGBA. A header-only TGA of width or height 0 (the last Kingdom Reborn
/// `TileArtLegacy.uop` entry) is an empty image.
pub fn decode_tga_rgba8(bytes: &[u8]) -> eyre::Result<RgbaImage> {
    if bytes.len() >= TGA_HEADER_SIZE {
        let dimension = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if dimension(TGA_WIDTH_OFFSET) == 0 || dimension(TGA_HEIGHT_OFFSET) == 0 {
            return Ok(RgbaImage::new(0, 0));
        }
    }
    image::load(Cursor::new(bytes), image::ImageFormat::Tga)
        .wrap_err("Failed reading TGA")
        .map(|image| image.into_rgba8())
}

/// The primary public-facing orchestrator for Enhanced Client textures.
///
/// This structure manages the loading and lookup of textures from UOP packages.
/// It handles path hashing and optional cross-referencing with `tileart.uop`
/// to provide fully decorated `TextureFile` results.
#[derive(Clone)]
pub struct Textures {
    package: UopPackage,
    tileart: Option<UopPackage>, // to be integrated later in tileart.rs port
    string_dictionary: Option<UoStringDictionary>,
    is_ec_texture: bool,
}

impl Textures {
    pub fn new(
        path: &Path,
        tileart_path: Option<&Path>,
        string_dictionary_path: Option<&Path>,
    ) -> eyre::Result<Self> {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy)?;
        let tileart = if let Some(p) = tileart_path {
            Some(UopPackage::load(p)?)
        } else {
            None
        };
        let string_dictionary = if let Some(dict_path) = string_dictionary_path {
            Some(UoStringDictionary::load(dict_path)?)
        } else {
            None
        };
        let is_ec_texture = path.file_name().is_some_and(|n| n == "Texture.uop");
        Ok(Self {
            package,
            tileart,
            string_dictionary,
            is_ec_texture,
        })
    }

    pub fn package(&self) -> &UopPackage {
        &self.package
    }

    // Deprecated: used purely for returning String names. Replaced internally by stack buffers.
    pub fn get_texture_name_from_id(&self, tid: u32) -> String {
        if self.is_ec_texture {
            format!("build/worldart/{:08}.dds", tid)
        } else {
            format!("build/tileartlegacy/{:08}.dds", tid)
        }
    }

    pub fn get_from_name(&self, full_tid: &str) -> eyre::Result<Option<TextureFile>> {
        let hash = hash_file_name_single(full_tid);
        let format = if full_tid.ends_with(".dds") {
            ECImageFormat::DDS
        } else if full_tid.ends_with(".tga") {
            ECImageFormat::TGA
        } else {
            ECImageFormat::Unknown
        };
        self.get_from_hash(hash, Some(full_tid), format)
    }

    pub fn get_from_hash(
        &self,
        hash: u64,
        full_tid: Option<&str>,
        format: ECImageFormat,
    ) -> eyre::Result<Option<TextureFile>> {
        if let Some(data) = self.package.unpack_file_arc_by_hash(hash)? {
            let (metadata, image_data_offset) = if payload_starts_with_raw_image(&data, format) {
                // The EC texture UOPs typically store the DDS/TGA payload directly.
                // Treat the whole unpacked file as image data in that common case.
                (TextureItem::absent(), 0)
            } else {
                let mut cursor = Cursor::new(&*data);
                match TextureItem::read(&mut cursor) {
                    Ok(metadata) => {
                        let image_data_pos = cursor.position() as usize;
                        (metadata, image_data_pos)
                    }
                    Err(_) => (TextureItem::absent(), 0),
                }
            };

            let mut props = None;
            if let Some(full_tid) = full_tid {
                if let (Some(tileart), Some(string_dictionary)) =
                    (&self.tileart, &self.string_dictionary)
                {
                    let parts: Vec<&str> = full_tid.split('/').collect();
                    if parts.len() >= 3 {
                        if let Some(id_part) = parts.last() {
                            let tid = id_part.split('.').next().unwrap_or("");
                            if let Ok(item_id) = tid.parse::<u32>() {
                                use std::io::Write;
                                let mut path_buf = [0u8; 64];
                                let mut slice = &mut path_buf[..];
                                write!(slice, "build/tileart/{:08}.dat", item_id).unwrap();
                                let len = 64 - slice.len();
                                let path =
                                    unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
                                let tileart_hash = hash_file_name_single(path);
                                if let Some(tileart_file) = tileart.get_file_by_hash(tileart_hash) {
                                    // Parse properties from `uop/file.rs -> unpacking`
                                    props = tileart::load(tileart_file, string_dictionary).ok();
                                }
                            }
                        }
                    }
                }
            }

            return Ok(Some(TextureFile {
                metadata,
                format,
                props,
                raw_data: data,
                image_data_offset,
                is_ec: self.is_ec_texture,
            }));
        }

        Ok(None)
    }

    pub fn get_from_id(&self, tid: u32) -> eyre::Result<Option<TextureFile>> {
        use std::io::Write;
        let mut path_buf = [0u8; 64];
        let mut slice = &mut path_buf[..];
        if self.is_ec_texture {
            write!(slice, "build/worldart/{:08}.dds", tid).unwrap();
        } else {
            write!(slice, "build/tileartlegacy/{:08}.dds", tid).unwrap();
        }
        let len = 64 - slice.len();
        let path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
        self.get_from_name(path)
    }
}

// endregion: --- Public API

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x1 X1R5G5B5 DDS: one pure red pixel with the unused top bit clear, one pure blue with it set.
    fn x1r5g5b5_dds() -> Vec<u8> {
        const DDS_HEADER_SIZE: u32 = 124;
        const DDS_PIXEL_FORMAT_SIZE: u32 = 32;
        /// DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT.
        const DDS_FLAGS: u32 = 0x1007;
        const DDPF_RGB: u32 = 0x40;
        const DDSCAPS_TEXTURE: u32 = 0x1000;
        const HEADER_RESERVED_WORDS: usize = 11;
        const TRAILING_WORDS: usize = 4;
        const RED: u16 = 0x7C00;
        const BLUE_WITH_TOP_BIT: u16 = 0x801F;
        let mut bytes = b"DDS ".to_vec();
        let mut put = |value: u32| bytes.extend_from_slice(&value.to_le_bytes());
        // size, flags, height, width, pitch, depth, mip count
        [DDS_HEADER_SIZE, DDS_FLAGS, 1, 2, 0, 0, 0].into_iter().for_each(&mut put);
        (0..HEADER_RESERVED_WORDS).for_each(|_| put(0));
        [DDS_PIXEL_FORMAT_SIZE, DDPF_RGB, 0, u32::from(X1R5G5B5_BIT_COUNT)].into_iter().for_each(&mut put);
        X1R5G5B5_RGB_MASKS.into_iter().for_each(&mut put);
        put(0);
        put(DDSCAPS_TEXTURE);
        (0..TRAILING_WORDS).for_each(|_| put(0));
        bytes.extend_from_slice(&RED.to_le_bytes());
        bytes.extend_from_slice(&BLUE_WITH_TOP_BIT.to_le_bytes());
        bytes
    }

    #[test]
    fn x1r5g5b5_dds_decodes_opaque() {
        let image = decode_dds_rgba8(&x1r5g5b5_dds()).unwrap();
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.get_pixel(0, 0).0, [u8::MAX, 0, 0, OPAQUE_ALPHA]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, u8::MAX, OPAQUE_ALPHA]);
    }

    #[test]
    fn header_only_tga_is_an_empty_image() {
        /// Uncompressed true-colour TGA, 32 bits per pixel, 0x0.
        const TGA_TRUE_COLOUR: u8 = 2;
        const TGA_BITS_PER_PIXEL_OFFSET: usize = 16;
        const TGA_BITS_PER_PIXEL: u8 = 32;
        let mut header = [0u8; TGA_HEADER_SIZE];
        header[2] = TGA_TRUE_COLOUR;
        header[TGA_BITS_PER_PIXEL_OFFSET] = TGA_BITS_PER_PIXEL;
        assert_eq!(decode_tga_rgba8(&header).unwrap().dimensions(), (0, 0));
        assert!(decode_tga_rgba8(&header[..TGA_HEADER_SIZE - 1]).is_err());
    }
}
