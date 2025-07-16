#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use image::{DynamicImage, ImageBuffer, RgbaImage};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{prelude::*, Cursor, SeekFrom};
use std::path::PathBuf;

use crate::generic_index;
use crate::utils::color::*;
use crate::utils::math::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureSize {
    Small,
    Big,
}
impl Default for TextureSize {
    fn default() -> Self {
        Self::Small
    }
}
impl TextureSize {
    pub const SMALL_X: u32 = 64;
    pub const SMALL_Y: u32 = 64;
    pub const BIG_X: u32 = 128;
    pub const BIG_Y: u32 = 128;
}

#[derive(Clone, Debug, Default)]
pub struct Texture2DElement {
    // Pixel data in TexMap.mul is stored as bgra5551 (u16), but we convert it to argb8888 (u32) before storing it.
    valid: bool,
    pub id: u32,
    size: TextureSize,
    pub pixel_data: Vec<u8>,
}
impl Texture2DElement {
    pub const TEXTURE_UNUSED: u32 = 0x007F; // NODRAW
    const PIXEL_DATA_CHANNELS: usize = 4; // R, G, B, A

    #[must_use]
    pub fn size_type_x(size: TextureSize) -> u32 {
        match size {
            TextureSize::Small => TextureSize::SMALL_X,
            TextureSize::Big => TextureSize::BIG_X,
        }
    }
    #[must_use]
    pub fn size_x(&self) -> u32 {
        Self::size_type_x(self.size)
    }

    #[must_use]
    pub fn size_type_y(size: TextureSize) -> u32 {
        match size {
            TextureSize::Small => TextureSize::SMALL_Y,
            TextureSize::Big => TextureSize::BIG_Y,
        }
    }
    #[must_use]
    pub fn size_y(&self) -> u32 {
        Self::size_type_y(self.size)
    }

    #[must_use]
    pub fn to_image(&self) -> eyre::Result<DynamicImage> {
        /*  // Less efficient way?
        let mut built_img = RgbImage::new(size.0, size.1);
        for y in 0..size.1 {
            for x in 0..size.0 {
                let pixel_data_index = (y*size.1 + x) as usize * PIXEL_DATA_CHANNELS;
                let pixel_data = self.pixel_data[pixel_data_index..pixel_data_index + PIXEL_DATA_CHANNELS];
                let pixel = image::Rgb::<u8>(pixel_data);
                println!("pixel data 0x{:X} {pixel_data}.  pixel {:?}", pixel_data, pixel);
                built_img.put_pixel(x, y, pixel);
            }
        }
        built_img.save("./test_built.png")?;
        */

        let img: image::ImageBuffer<image::Rgba<u8>, _> =
            ImageBuffer::from_vec(self.size_x(), self.size_y(), self.pixel_data.clone())
                .ok_or(eyre!("Invalid Texture Data"))?;
        //image::save_buffer("./test.png", &buf, size.0, size.1, image::ColorType::Rgba8);
        let img = DynamicImage::ImageRgba8(img);
        Ok(img)
    }
}

#[derive(Debug)]
pub struct TexMap2D {
    file_data: Vec<Texture2DElement> //HashMap<u32, Texture2DElement>,
}

impl TexMap2D {
    pub fn len(&self) -> usize {
        self.file_data.len()
    }

    pub fn element(&self, element_index: usize) -> Option<&Texture2DElement> {
        if element_index >= self.file_data.len() {
            /*return Err(eyre!(
                "TexMap2d: requested element with out of range index ({element_index})."
            ));*/
            return None;
        }
        //println!("Requested element {element_index} from texmap.mul.");
        let element = &self.file_data[element_index];
        if !element.valid {
            /*return Err(eyre!(
                "TexMap2d: requested invalid/uninitialized element ({element_index})."
            ));*/
            return None;
        }
        //Ok(element)
        Some(element)
    }

    pub fn load(
        texmap_file_path: PathBuf,
        texmap_idx_file_path: PathBuf,
    ) -> eyre::Result<TexMap2D> {
        /* Open texmap.mul */
        let texmap_file_name = texmap_file_path
            .file_name()
            .expect("Provided file path without filename.")
            .to_string_lossy();
        let texmap_file_path = texmap_file_path
            .canonicalize()
            .wrap_err_with(|| format!("Check {texmap_file_name} path"))?;

        let mut texmap_file_handle = File::open(&texmap_file_path)
            .wrap_err_with(|| format!("Open map textures mul file at '{texmap_file_name}'"))?;
        let texmap_file_metadata = texmap_file_handle
            .metadata()
            .wrap_err("Get {file_name} metadata")?;
        let texmap_file_size = downcast_ceil_usize(texmap_file_metadata.len());

        /* Open texidx.mul */
        let texidx = generic_index::IndexFile::load(texmap_idx_file_path)?;

        /* Read whole texidx.mul to get texmap index data */
        const TEXMAP_MAX_ID: u32 = 0x1388;
        let mut texmap = TexMap2D {
            //file_data: vec![Texture2DElement::default(); texidx.element_count()],
            file_data: vec![Texture2DElement::default(); TEXMAP_MAX_ID as usize],
        };

        let mut texmap_file_rdr = {
            let mut rdr_buf = vec![0; texmap_file_size];
            texmap_file_handle
                .read_exact(rdr_buf.as_mut())
                .wrap_err("Read index file")?;
            Cursor::new(rdr_buf)
        };

        // Loop on each entry of texidx
        let mut i_idx_valid: usize = 0;
        for i_idx_raw in 0..TEXMAP_MAX_ID { // 0..texidx.element_count() {
            // Fill texmap
            let cur_idx_elem = texidx
                .element(i_idx_raw as usize)
                .expect("Reading lookup value for element {i_idx}");

            let tex_lookup = match cur_idx_elem.lookup() {
                None => continue,
                Some(val) => {
                    if val as usize >= texmap_file_size {
                        continue;
                    }
                    val
                }
            };

            let tex_len = match cur_idx_elem.len() {
                None => continue,
                Some(val) => val,
            };

            let tex_size_type = match tex_len {
                0x2000 => {
                    // 0x2000 comes from 64*64 pixels = 0x1000. A single pixel is coded with a 16 bit (2 bytes) color value,
                    //  thus 0x1000 * 2 = 0x2000.
                    TextureSize::Small
                }
                0x8000 => {
                    // 0x8000 comes from 128*128 pixels * 2.
                    TextureSize::Big
                }
                _ => {
                    /*println!(
                        "Unknown texture size: {tex_len} (0x{:x}) for texture {i_idx} (0x{:x})",
                        tex_len, i_idx
                    );*/
                    continue;
                }
            };

            let cur_texture = &mut texmap.file_data[i_idx_raw as usize];
            cur_texture.id = i_idx_raw as u32; //i_idx_valid as u32;
            cur_texture.size = tex_size_type.clone();

            let pixel_qty = match tex_size_type {
                TextureSize::Small => TextureSize::SMALL_X as usize * TextureSize::SMALL_Y as usize,
                TextureSize::Big => TextureSize::BIG_X as usize * TextureSize::BIG_Y as usize,
            };

            texmap_file_rdr.seek(SeekFrom::Start(tex_lookup as u64))?;
            cur_texture.pixel_data = Vec::with_capacity(pixel_qty);
            for i_pixel in 0..pixel_qty {
                let mut pixel_16 = Bgra5551::new_from_val(
                    texmap_file_rdr
                        .read_u16::<LittleEndian>()
                        .wrap_err_with(|| {
                            format!("Read pixel {i_pixel} data for texture with raw idx {i_idx_raw}, idx in collection {i_idx_valid}")
                        })?,
                );
                pixel_16.set_a(1);
                cur_texture
                    .pixel_data
                    .extend_from_slice(pixel_16.as_rgba8888().value().to_le_bytes().as_ref());
            }

            cur_texture.valid = true;
            i_idx_valid += 1;
        }

        texmap.file_data.shrink_to_fit();

        println!(
            "Parsed {} (0x{:x}) Map Tile texture slots, loaded {} (0x{:x}) valid.",
            texidx.element_count(),
            texidx.element_count(),
            i_idx_valid,
            i_idx_valid
        );

        Ok(texmap)
    }
}
