//! Textures: images decoded as the effect previews decode them and embedded as PNG (level 0), with samplers
//! from the Gamebryo filter and wrap modes. Images, samplers and textures are shared by every part of an export:
//! external files by lower-case name, embedded pixels by content.
//!
//! - DDS (by its magic): the stored level 0 (`particle_sim::mips::decode_dds_levels`).
//! - `.nif` (an `NiPixelData` image file): level 0 (`particle_sim::nif::compile::decode_nif_image`).
//! - Anything else: TGA.
//! - Embedded `NiPixelData` (`NifTextureSource::Embedded`): level 0 as compiled.
//!
//! The mip levels are not exported: viewers build their own chain when the sampler asks for one.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use particle_sim::NifEffectDesc;
use particle_sim::mips::decode_dds_levels;
use particle_sim::nif::compile::decode_nif_image;
use particle_sim::nif::desc::{MipFilter, NifTextureSource, TexelFilter, TextureAddressing, TextureFilter};
use serde_json::json;

use crate::glb::Builder;

/// The magic of a DDS file.
const DDS_MAGIC: &[u8] = b"DDS ";
/// The extension of a NIF image file.
const NIF_EXTENSION: &str = ".nif";
/// Bytes per RGBA8 pixel.
const RGBA_BYTES: usize = 4;

/// Sampler wrap mode `REPEAT`.
pub const WRAP_REPEAT: u32 = 10497;
/// Sampler wrap mode `CLAMP_TO_EDGE`.
pub const WRAP_CLAMP_TO_EDGE: u32 = 33071;
/// Sampler filter `NEAREST`.
pub const FILTER_NEAREST: u32 = 9728;
/// Sampler filter `LINEAR`.
pub const FILTER_LINEAR: u32 = 9729;
/// Sampler minification filter `NEAREST_MIPMAP_NEAREST`.
pub const FILTER_NEAREST_MIPMAP_NEAREST: u32 = 9984;
/// Sampler minification filter `LINEAR_MIPMAP_NEAREST`.
pub const FILTER_LINEAR_MIPMAP_NEAREST: u32 = 9985;
/// Sampler minification filter `NEAREST_MIPMAP_LINEAR`.
pub const FILTER_NEAREST_MIPMAP_LINEAR: u32 = 9986;
/// Sampler minification filter `LINEAR_MIPMAP_LINEAR`.
pub const FILTER_LINEAR_MIPMAP_LINEAR: u32 = 9987;

/// Reads a texture file of the client's packages by the name a NIF or a Chaos script gives.
pub type TextureLoader<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// RGBA8 level 0 of a texture file.
pub fn decode_texture_file(name: &str, payload: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if payload.starts_with(DDS_MAGIC) {
        let mut dds = decode_dds_levels(payload)?;
        if dds.levels.is_empty() {
            return Err("DDS without levels".to_owned());
        }
        return Ok((dds.width, dds.height, dds.levels.swap_remove(0)));
    }
    if name.to_ascii_lowercase().ends_with(NIF_EXTENSION) {
        let mut levels = decode_nif_image(payload)?;
        if levels.is_empty() {
            return Err("NIF image without levels".to_owned());
        }
        return Ok(levels.swap_remove(0));
    }
    let image = image::load_from_memory_with_format(payload, image::ImageFormat::Tga)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

fn encode_png(width: u32, height: u32, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    if rgba.len() != width as usize * height as usize * RGBA_BYTES {
        return Err(format!("{} bytes for {width}x{height} RGBA pixels", rgba.len()));
    }
    let image = image::RgbaImage::from_raw(width, height, rgba).ok_or("pixel buffer size mismatch")?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

/// The glTF sampler of a Gamebryo filter mode and addressing.
fn sampler_json(filter: u32, addressing: TextureAddressing) -> serde_json::Value {
    let TextureFilter { mag, min, mip } = TextureFilter::from_raw(filter);
    let mag_filter = match mag {
        TexelFilter::Point => FILTER_NEAREST,
        TexelFilter::Linear => FILTER_LINEAR,
    };
    let min_filter = match (min, mip) {
        (TexelFilter::Point, MipFilter::None) => FILTER_NEAREST,
        (TexelFilter::Linear, MipFilter::None) => FILTER_LINEAR,
        (TexelFilter::Point, MipFilter::Point) => FILTER_NEAREST_MIPMAP_NEAREST,
        (TexelFilter::Linear, MipFilter::Point) => FILTER_LINEAR_MIPMAP_NEAREST,
        (TexelFilter::Point, MipFilter::Linear) => FILTER_NEAREST_MIPMAP_LINEAR,
        (TexelFilter::Linear, MipFilter::Linear) => FILTER_LINEAR_MIPMAP_LINEAR,
    };
    let wrap = |repeat: bool| if repeat { WRAP_REPEAT } else { WRAP_CLAMP_TO_EDGE };
    json!({
        "magFilter": mag_filter,
        "minFilter": min_filter,
        "wrapS": wrap(addressing.wrap_s),
        "wrapT": wrap(addressing.wrap_t),
    })
}

/// The images, samplers and textures of one export.
#[derive(Default)]
pub struct TextureCache {
    /// Image key → image index, or `None` when it failed (reported once).
    images: HashMap<String, Option<usize>>,
    samplers: HashMap<String, usize>,
    textures: HashMap<(usize, usize), usize>,
}

impl TextureCache {
    fn sampler(&mut self, builder: &mut Builder, filter: u32, addressing: TextureAddressing) -> usize {
        let sampler = sampler_json(filter, addressing);
        let key = sampler.to_string();
        *self.samplers.entry(key).or_insert_with(|| {
            builder.samplers.push(sampler);
            builder.samplers.len() - 1
        })
    }

    fn texture(&mut self, builder: &mut Builder, image: usize, sampler: usize, name: &str) -> usize {
        *self.textures.entry((image, sampler)).or_insert_with(|| {
            builder.textures.push(json!({ "name": name, "source": image, "sampler": sampler }));
            builder.textures.len() - 1
        })
    }

    /// The image of a texture file read through `loader`.
    pub fn file_image(
        &mut self,
        builder: &mut Builder,
        loader: TextureLoader<'_>,
        name: &str,
        warnings: &mut Vec<String>,
    ) -> Option<usize> {
        let key = format!("file:{}", name.to_ascii_lowercase());
        if let Some(image) = self.images.get(&key) {
            return *image;
        }
        let image = match loader(name) {
            None => {
                warnings.push(format!("texture {name}: not found"));
                None
            }
            Some(payload) => match decode_texture_file(name, &payload).and_then(|(w, h, rgba)| encode_png(w, h, rgba)) {
                Ok(png) => Some(builder.png_image(name, &png)),
                Err(error) => {
                    warnings.push(format!("texture {name}: {error}"));
                    None
                }
            },
        };
        self.images.insert(key, image);
        image
    }

    /// The image of texture `index` of `desc`.
    pub fn nif_image(
        &mut self,
        builder: &mut Builder,
        loader: TextureLoader<'_>,
        desc: &NifEffectDesc,
        index: usize,
        warnings: &mut Vec<String>,
    ) -> Option<usize> {
        let texture = desc.textures.get(index)?;
        match &texture.source {
            NifTextureSource::External { file_name } => self.file_image(builder, loader, file_name, warnings),
            NifTextureSource::Embedded { width, height, levels } => {
                let level = levels.first()?;
                let mut hasher = DefaultHasher::new();
                (width, height, level).hash(&mut hasher);
                let key = format!("embedded:{:016x}", hasher.finish());
                if let Some(image) = self.images.get(&key) {
                    return *image;
                }
                let name = format!("{}{}", desc.name, texture.name());
                let image = match encode_png(*width, *height, level.clone()) {
                    Ok(png) => Some(builder.png_image(&name, &png)),
                    Err(error) => {
                        warnings.push(format!("{name}: {error}"));
                        None
                    }
                };
                self.images.insert(key, image);
                image
            }
            NifTextureSource::Undecoded { format } => {
                warnings.push(format!("{}: embedded texture {index} has an undecoded pixel format {format}", desc.name));
                None
            }
        }
    }

    /// The glTF texture of texture `index` of `desc` sampled with a map's filter and addressing.
    pub fn nif_texture(
        &mut self,
        builder: &mut Builder,
        loader: TextureLoader<'_>,
        desc: &NifEffectDesc,
        index: Option<usize>,
        filter: u32,
        addressing: TextureAddressing,
        warnings: &mut Vec<String>,
    ) -> Option<usize> {
        let index = index?;
        let image = self.nif_image(builder, loader, desc, index, warnings)?;
        let sampler = self.sampler(builder, filter, addressing);
        let name = desc.textures[index].name();
        Some(self.texture(builder, image, sampler, &name))
    }

    /// The glTF texture of a texture file with a sampler of `filter` and `addressing` (baked sprites).
    pub fn file_texture(
        &mut self,
        builder: &mut Builder,
        loader: TextureLoader<'_>,
        name: &str,
        filter: u32,
        addressing: TextureAddressing,
        warnings: &mut Vec<String>,
    ) -> Option<usize> {
        let image = self.file_image(builder, loader, name, warnings)?;
        let sampler = self.sampler(builder, filter, addressing);
        Some(self.texture(builder, image, sampler, name))
    }
}
