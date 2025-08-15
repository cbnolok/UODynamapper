#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use getset::Getters;
use image::{DynamicImage, ImageBuffer, RgbaImage};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;

use std::path::PathBuf;

use crate::generic_index;
use crate::utils::color::*;
use crate::utils::math::*;
use bytemuck;
use std::io::{BufReader, Cursor, SeekFrom, prelude::*};
use wide::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LandTextureSize {
    Small,
    Big,
}
impl Default for LandTextureSize {
    fn default() -> Self {
        Self::Small
    }
}
impl LandTextureSize {
    pub const SMALL_X: u32 = 64;
    pub const SMALL_Y: u32 = 64;
    pub const BIG_X: u32 = 128;
    pub const BIG_Y: u32 = 128;

    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            LandTextureSize::Small => (LandTextureSize::SMALL_X, LandTextureSize::SMALL_Y),
            LandTextureSize::Big => (LandTextureSize::BIG_X, LandTextureSize::BIG_Y),
        }
    }
    pub fn from_dimensions(width: u32, height: u32) -> Option<Self> {
        match (width, height) {
            (Self::SMALL_X, Self::SMALL_Y) => Some(Self::Small),
            (Self::BIG_X, Self::BIG_Y) => Some(Self::Big),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Getters)]
pub struct Texture2DElement {
    // Pixel data in TexMap.mul is stored as bgra5551 (u16), but we convert it to argb8888 (u32) before storing it.
    valid: bool,
    #[get = "pub"]
    id: u32,
    #[get = "pub"]
    size: LandTextureSize,
    #[get = "pub"]
    pixel_data: Vec<u8>,
}
impl Texture2DElement {
    pub const TEXTURE_UNUSED: u32 = 0x007F; // NODRAW
    const PIXEL_DATA_CHANNELS: usize = 4; // R, G, B, A

    #[must_use]
    pub fn size_type_x(size: LandTextureSize) -> u32 {
        match size {
            LandTextureSize::Small => LandTextureSize::SMALL_X,
            LandTextureSize::Big => LandTextureSize::BIG_X,
        }
    }
    #[must_use]
    pub fn size_x(&self) -> u32 {
        Self::size_type_x(self.size)
    }

    #[must_use]
    pub fn size_type_y(size: LandTextureSize) -> u32 {
        match size {
            LandTextureSize::Small => LandTextureSize::SMALL_Y,
            LandTextureSize::Big => LandTextureSize::BIG_Y,
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
        //Image::from_dynamic
        Ok(img)
    }
}

#[derive(Debug)]
pub struct TexMap2D {
    file_data: Vec<Texture2DElement>, //HashMap<u32, Texture2DElement>,
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
        let element: &Texture2DElement = &self.file_data[element_index];
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

        let texmap_file_handle = File::open(&texmap_file_path)
            .wrap_err_with(|| format!("Open map textures mul file at '{texmap_file_name}'"))?;
        let texmap_file_metadata = texmap_file_handle
            .metadata()
            .wrap_err_with(|| format!("Get {texmap_file_name} metadata"))?;
        let texmap_file_size = downcast_ceil_usize(texmap_file_metadata.len());
        let mut texmap_file_rdr = BufReader::new(texmap_file_handle);

        /* Open texidx.mul */
        let texidx: generic_index::IndexFile =
            generic_index::IndexFile::load(texmap_idx_file_path)?;

        /* Read whole texidx.mul to get texmap index data */
        const TEXMAP_MAX_ID: u32 = 0x1388;
        let mut texmap = TexMap2D {
            //file_data: vec![Texture2DElement::default(); texidx.element_count()],
            file_data: vec![Texture2DElement::default(); TEXMAP_MAX_ID as usize],
        };

        // Loop on each entry of texidx
        let mut i_idx_valid: usize = 0;
        for i_idx_raw in 0..TEXMAP_MAX_ID {
            // 0..texidx.element_count() {
            // Fill texmap
            let cur_idx_elem: &generic_index::IndexElement = texidx
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

            let tex_size_type: LandTextureSize = match tex_len {
                0x2000 => {
                    // 0x2000 comes from 64*64 pixels = 0x1000. A single pixel is coded with a 16 bit (2 bytes) color value,
                    //  thus 0x1000 * 2 = 0x2000.
                    LandTextureSize::Small
                }
                0x8000 => {
                    // 0x8000 comes from 128*128 pixels * 2.
                    LandTextureSize::Big
                }
                _ => {
                    /*println!(
                        "Unknown texture size: {tex_len} (0x{:x}) for texture {i_idx} (0x{:x})",
                        tex_len, i_idx
                    );*/
                    continue;
                }
            };

            let cur_texture: &mut Texture2DElement = &mut texmap.file_data[i_idx_raw as usize];
            cur_texture.id = i_idx_raw as u32; //i_idx_valid as u32;
            cur_texture.size = tex_size_type.clone();

            let pixel_qty = match tex_size_type {
                LandTextureSize::Small => {
                    LandTextureSize::SMALL_X as usize * LandTextureSize::SMALL_Y as usize
                }
                LandTextureSize::Big => {
                    LandTextureSize::BIG_X as usize * LandTextureSize::BIG_Y as usize
                }
            };

            texmap_file_rdr.seek(SeekFrom::Start(tex_lookup as u64))?;
            let pixel_qty_bytes = pixel_qty * 2; // Each u16 is 2 bytes
            let mut pixel_data_bytes = vec![0u8; pixel_qty_bytes];
            texmap_file_rdr.read_exact(&mut pixel_data_bytes)?;

            cur_texture.pixel_data = Vec::with_capacity(pixel_qty * 4);

            let (pixel_data_u16_prefix, pixel_data_u16_suffix) =
                bytemuck::cast_slice(&pixel_data_bytes).as_chunks::<16>();

            for &chunk_array in pixel_data_u16_prefix {
                #[allow(unused_mut)]
                let mut chunk = u16x16::new(chunk_array);

                #[cfg(target_endian = "big")]
                {
                    chunk = chunk.swap_bytes();
                }

                let b_u16: u16x16 = (chunk & u16x16::splat(0x1F)) << 3;
                let g_u16: u16x16 = ((chunk >> 5) & u16x16::splat(0x1F)) << 3;
                let r_u16: u16x16 = ((chunk >> 10) & u16x16::splat(0x1F)) << 3;
                let a_u16: u16x16 = u16x16::splat(0xFF); // Alpha is set to 255

                // Now convert u16x16 to [u32; 16]
                let mut rgba_u32_array = [0u32; 16];
                for i in 0..16 {
                    let r_val = r_u16.as_array_ref()[i] as u32;
                    let g_val = g_u16.as_array_ref()[i] as u32;
                    let b_val = b_u16.as_array_ref()[i] as u32;
                    let a_val = a_u16.as_array_ref()[i] as u32;
                    rgba_u32_array[i] = (a_val << 24) | (b_val << 16) | (g_val << 8) | r_val;
                }
                cur_texture
                    .pixel_data
                    .extend_from_slice(bytemuck::cast_slice(&rgba_u32_array));
            }

            for &pixel_16_val in pixel_data_u16_suffix {
                #[allow(unused_mut)]
                let mut pixel_16 = Bgra5551::new_from_val(pixel_16_val);
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
