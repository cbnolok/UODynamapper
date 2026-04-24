//! Decoding for UO Enhanced Client texture files (`Texture.uop`).
//!
//! This module handles the parsing of texture metadata and extraction of texture image data
//! from the `Texture.uop` file. The textures can be in TGA or DDS format.

use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::tileart::{self, ArtData};
use crate::uop::hash::hash_file_name_single;
use crate::uop::package::UopPackage;
use byteorder::{LittleEndian, ReadBytesExt};
use image::{DynamicImage, ImageBuffer};
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

crate::eyre_imports!();

/// Represents the metadata for a texture file.
#[derive(Debug, Clone)]
pub struct TextureItem {
    pub texture_present: bool,
    pub unk1: u8,
    pub name_index: i32,
    pub images: Vec<TextureItemImage>,
    pub unk8: Vec<i32>,
    pub unk9: Vec<f32>,
}

/// Represents an image within a texture item's metadata.
#[derive(Debug, Clone)]
pub struct TextureItemImage {
    pub string_dictionary_offset: i32,
    pub unk4: u8,
    pub texture_repetition: f32,
    pub unk6: i32,
    pub unk7: i32,
}

impl TextureItem {
    /// Reads a `TextureItem` from a binary reader.
    pub fn read<R: Read>(reader: &mut R) -> eyre::Result<Self> {
        let texture_present = reader.read_u8()? != 0;
        if !texture_present {
            return Ok(Self {
                texture_present: false,
                unk1: 0,
                name_index: 0,
                images: vec![],
                unk8: vec![],
                unk9: vec![],
            });
        }

        let unk1 = reader.read_u8()?;
        let name_index = reader.read_i32::<LittleEndian>()?;
        let count1 = reader.read_u8()?;
        let mut images = Vec::with_capacity(count1 as usize);
        for _ in 0..count1 {
            images.push(TextureItemImage {
                string_dictionary_offset: reader.read_i32::<LittleEndian>()?,
                unk4: reader.read_u8()?,
                texture_repetition: reader.read_f32::<LittleEndian>()?,
                unk6: reader.read_i32::<LittleEndian>()?,
                unk7: reader.read_i32::<LittleEndian>()?,
            });
        }

        let count2 = reader.read_u32::<LittleEndian>()?;
        let mut unk8 = Vec::with_capacity(count2 as usize);
        for _ in 0..count2 {
            unk8.push(reader.read_i32::<LittleEndian>()?);
        }

        let count3 = reader.read_u32::<LittleEndian>()?;
        let mut unk9 = Vec::with_capacity(count3 as usize);
        for _ in 0..count3 {
            unk9.push(reader.read_f32::<LittleEndian>()?);
        }

        Ok(Self {
            texture_present: true,
            unk1,
            name_index,
            images,
            unk8,
            unk9,
        })
    }
}

/// Identifiers for the underlying raw container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ECImageFormat {
    DDS,
    TGA,
    Unknown,
}

/// Represents a fully mapped texture file containing both its metadata header
/// and a zero-copy thread-safe slice of its binary layout.
#[derive(Debug, Clone)]
pub struct TextureFile {
    pub metadata: TextureItem,
    pub is_ec: bool,
    pub format: ECImageFormat,
    /// Detailed Tile Art metadata if linked against `tileart.uop` definitions.
    pub props: Option<ArtData>,
    /// The unadultered bytes (such as DXT1/5 DDS byte-blocks or TGA blobs) extracted
    /// directly from `UopPackage` z-lib, without heavy CPU scalar decoding.
    /// This saves ~8x memory space and makes it extremely fast to upload to GPU VRAM mappings.
    pub raw_data: Arc<[u8]>,
}

impl TextureFile {
    /// Provides access to textures natively mapped in UOP layout without
    /// decompressing the format through `dds` to raw RGBA.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_data
    }

    /// CPU Software Decompression fallback primarily reserved for tooling or legacy UI extraction.
    /// Engine renderer (e.g., Bevy) should strictly utilize `raw_bytes` and forward the BC blocks payload
    /// directly to VRAM relying on `format`.
    pub fn decode_to_rgba(&self) -> eyre::Result<DynamicImage> {
        match self.format {
            ECImageFormat::DDS => {
                let cursor = Cursor::new(&self.raw_data[..]);
                let mut decoder = dds::Decoder::new(cursor).wrap_err("Failed reading DDS header")?;
                let size = decoder.main_size();
                let rgba_len = dds::ColorFormat::RGBA_U8
                    .buffer_size(size)
                    .ok_or_else(|| eyre::eyre!("DDS image is too large to decode"))?;
                let mut rgba = vec![0u8; rgba_len];
                let image = dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8)
                    .ok_or_else(|| eyre::eyre!("Could not create DDS decode buffer"))?;
                decoder
                    .read_surface(image)
                    .wrap_err("Failed decoding DDS image")?;
                let image_buffer = ImageBuffer::from_raw(size.width, size.height, rgba)
                    .ok_or_else(|| eyre::eyre!("Could not map buffer"))?;
                Ok(DynamicImage::ImageRgba8(image_buffer))
            }
            ECImageFormat::TGA => {
                image::load(Cursor::new(&self.raw_data[..]), image::ImageFormat::Tga)
                    .wrap_err("Failed reading TGA")
            }
            ECImageFormat::Unknown => {
                eyre::bail!("Unknown image format payload")
            }
        }
    }
}

/// Provides access to textures in a UOP file.
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
        let package = UopPackage::load(path)?;
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
        let is_ec_texture = path.to_string_lossy().ends_with("Texture.uop");
        Ok(Self {
            package,
            tileart,
            string_dictionary,
            is_ec_texture,
        })
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
        _full_tid: Option<&str>,
        format: ECImageFormat,
    ) -> eyre::Result<Option<TextureFile>> {
        if let Some(file) = self.package.get_file_by_hash(hash) {
            let data = file.unpack()?; // Returns Arc<[u8]>
            let mut cursor = Cursor::new(&*data);

            let metadata = TextureItem::read(&mut cursor)?;
            let image_data_pos = cursor.position() as usize;

            let sliced_image: Arc<[u8]> = data[image_data_pos..].into();

            let mut props = None;
            if let Some(full_tid) = _full_tid {
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
                raw_data: sliced_image,
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
