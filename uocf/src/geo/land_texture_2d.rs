#![allow(dead_code)]
//! # UO Land Texture Loading
//!
//! This module handles loading 2D map textures (texmaps) from UO mul files.
//!
//! ## Design Choices
//!
//! - **Centralized Texture Cache**: Instead of each texture element having its own `Mutex` (which costs
//!   significant memory when there are thousands of textures), we use a single `Mutex` protecting
//!   a centralized `HashMap` cache in `TexMap2D`. This reduces synchronization overhead and memory usage.
//! - **SIMD-Optimized Conversion**: UO textures are stored in `Bgra5551` format. We use `wide` crate SIMD
//!   intrinsics to process 16 pixels at a time, converting them to `Rgba8888` for the GPU. This
//!   path is highly optimized for modern CPUs.
//! - **Zero-Copy Intent**: Pixel data is stored in `Arc<Vec<u8>>` to allow O(1) sharing between
//!   the loading worker and the rendering system.

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
#[derive(Default)]
pub enum LandTextureSize {
    #[default]
    Small,
    Big,
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

#[derive(Debug, Default, Getters)]
pub struct Texture2DElement {
    // Pixel data in TexMap.mul is stored as bgra5551 (u16), but we convert it to argb8888 (u32) before storing it.
    valid: bool,
    #[get = "pub"]
    id: u32,
    #[get = "pub"]
    size: LandTextureSize,
    file_offset: u64,
    #[get = "pub"]
    pixel_qty: usize,
}

impl Clone for Texture2DElement {
    fn clone(&self) -> Self {
        Self {
            valid: self.valid,
            id: self.id,
            size: self.size,
            file_offset: self.file_offset,
            pixel_qty: self.pixel_qty,
        }
    }
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

}

#[derive(Debug)]
pub struct TexMap2D {
    file_data: Vec<Texture2DElement>, //HashMap<u32, Texture2DElement>,
    shared_data: std::sync::Mutex<TexMapShared>,
}

#[derive(Debug)]
struct TexMapShared {
    file_reader: BufReader<File>,
    scratch_buffer: Vec<u8>,
    /// Cache stores the raw BGRA5551 file data (2 bytes/pixel) rather than
    /// decoded RGBA8 (4 bytes/pixel), halving RAM usage.  Decode to RGBA8
    /// happens on-the-fly in `get_pixel_data` via SIMD and is very fast.
    cache: HashMap<usize, (std::sync::Arc<Vec<u8>>, std::time::Instant)>,
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
        // Do not use a local reader, use the struct's
        // let mut texmap_file_rdr = BufReader::new(texmap_file_handle);

        /* Open texidx.mul */
        let texidx: generic_index::IndexFile =
            generic_index::IndexFile::load(texmap_idx_file_path)?;

        /* Read whole texidx.mul to get texmap index data */
        const TEXMAP_MAX_ID: u32 = 0x1388;
        let mut texmap = TexMap2D {
            file_data: vec![Texture2DElement::default(); TEXMAP_MAX_ID as usize],
            shared_data: std::sync::Mutex::new(TexMapShared {
                file_reader: BufReader::new(texmap_file_handle),
                scratch_buffer: Vec::new(),
                cache: HashMap::new(),
            }),
        };

        // Loop on each entry of texidx
        let mut i_idx_valid: usize = 0;

        /*
        #[cfg(debug_assertions)]
        let _lut: Vec<[u8; 4]> = {
            let mut table = Vec::with_capacity(65536);
            for i in 0..=65535u16 {
                use crate::utils::color::Bgra5551;
                let mut p = Bgra5551::new_from_val(i);
                p.set_a(1);
                table.push(p.as_rgba8888().value().to_le_bytes());
            }
            table
        };
        */
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
            cur_texture.id = i_idx_raw; //i_idx_valid as u32;
            cur_texture.size = tex_size_type;

            let pixel_qty = match tex_size_type {
                LandTextureSize::Small => {
                    LandTextureSize::SMALL_X as usize * LandTextureSize::SMALL_Y as usize
                }
                LandTextureSize::Big => {
                    LandTextureSize::BIG_X as usize * LandTextureSize::BIG_Y as usize
                }
            };

            cur_texture.file_offset = tex_lookup as u64;
            cur_texture.pixel_qty = pixel_qty;

            cur_texture.valid = true;
            i_idx_valid += 1;
        }

        texmap.file_data.shrink_to_fit();

        log::info!(
            "Parsed {} (0x{:x}) Map Tile texture slots, loaded {} (0x{:x}) valid.",
            texidx.element_count(),
            texidx.element_count(),
            i_idx_valid,
            i_idx_valid
        );

        Ok(texmap)
    }

    pub fn get_pixel_data(&self, element_index: usize) -> Option<std::sync::Arc<Vec<u8>>> {
        let element: &Texture2DElement = self.element(element_index)?;

        let raw_bgra5551: std::sync::Arc<Vec<u8>> = {
            let mut shared = self.shared_data.lock().unwrap();

            // Check if the raw BGRA5551 data is already cached.
            if let Some((data, time)) = shared.cache.get_mut(&element_index) {
                *time = std::time::Instant::now();
                std::sync::Arc::clone(data)
            } else {
                // Read raw BGRA5551 from file and cache it (2 bytes/pixel).
                let pixel_qty_bytes = element.pixel_qty * 2;
                let shared = &mut *shared;
                shared.file_reader.seek(SeekFrom::Start(element.file_offset)).ok()?;
                shared.scratch_buffer.resize(pixel_qty_bytes, 0);
                shared.file_reader.read_exact(&mut shared.scratch_buffer).ok()?;

                let arc_raw = std::sync::Arc::new(shared.scratch_buffer.clone());
                shared.cache.insert(
                    element_index,
                    (std::sync::Arc::clone(&arc_raw), std::time::Instant::now()),
                );
                arc_raw
            }
        };

        // Decode BGRA5551 → RGBA8888 (done outside the lock so other threads
        // can access the cache concurrently).
        let mut pixel_data: Vec<u8> = Vec::with_capacity(element.pixel_qty * 4);

        #[cfg(debug_assertions)]
        {
            let pixels_u16: &[u16] = bytemuck::cast_slice(&raw_bgra5551);
            for &p in pixels_u16 {
                let mut pixel_16: Bgra5551 = crate::utils::color::Bgra5551::new_from_val(p);
                pixel_16.set_a(1);
                pixel_data.extend_from_slice(pixel_16.as_rgba8888().value().to_le_bytes().as_ref());
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let (pixel_data_u16_prefix, pixel_data_u16_suffix) =
                bytemuck::cast_slice(&raw_bgra5551).as_chunks::<16>();

            for &chunk_array in pixel_data_u16_prefix {
                let chunk = u16x16::new(chunk_array);

                #[cfg(target_endian = "big")]
                let chunk = chunk.swap_bytes();

                let b_u16: u16x16 = (chunk          & u16x16::splat(0x1F)) << 3;
                let g_u16: u16x16 = ((chunk >> 5)   & u16x16::splat(0x1F)) << 3;
                let r_u16: u16x16 = ((chunk >> 10)  & u16x16::splat(0x1F)) << 3;
                let a_u16: u16x16 = u16x16::splat(0xFF);

                let b_u16: &[u16; 16] = b_u16.as_array();
                let g_u16: &[u16; 16] = g_u16.as_array();
                let r_u16: &[u16; 16] = r_u16.as_array();
                let a_u16: &[u16; 16] = a_u16.as_array();

                let mut rgba_u32_array = [0u32; 16];
                for i in 0..16 {
                    let r_val = r_u16[i] as u32;
                    let g_val = g_u16[i] as u32;
                    let b_val = b_u16[i] as u32;
                    let a_val = a_u16[i] as u32;
                    rgba_u32_array[i] = (a_val << 24) | (b_val << 16) | (g_val << 8) | r_val;
                }
                pixel_data.extend_from_slice(bytemuck::cast_slice(&rgba_u32_array));
            }

            for &p in pixel_data_u16_suffix {
                let mut pixel_16 = crate::utils::color::Bgra5551::new_from_val(p);
                pixel_16.set_a(1);
                pixel_data.extend_from_slice(pixel_16.as_rgba8888().value().to_le_bytes().as_ref());
            }
        }

        Some(std::sync::Arc::new(pixel_data))
    }

    pub fn evict_idle_textures(&self, timeout: std::time::Duration) -> usize {
        let now = std::time::Instant::now();
        let mut shared = self.shared_data.lock().unwrap();
        let initial_len = shared.cache.len();

        shared.cache.retain(|_, (_, time)| {
            now.duration_since(*time) <= timeout
        });

        initial_len - shared.cache.len()
    }
}
