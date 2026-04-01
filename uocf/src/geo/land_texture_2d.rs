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
use indexmap::IndexMap;
use nohash_hasher::BuildNoHashHasher;
use std::borrow::Cow;
use std::fs::File;

use std::path::PathBuf;

use crate::generic_index;
use crate::utils::color::*;
use crate::utils::math::*;
use bytemuck;
use std::io::{prelude::*, BufReader, Cursor, SeekFrom};
use wide::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
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
    file_data: Vec<Texture2DElement>,
    shared_data: std::sync::Mutex<TexMapShared>,
}

#[derive(Debug)]
struct TexMapShared {
    file_reader: BufReader<File>,
    /// Cache stores the raw BGRA5551 file data (2 bytes/pixel).
    /// Using a non-hashing FastMap for rapid indexed lookups.
    cache: IndexMap<usize, (std::sync::Arc<[u8]>, std::time::Instant), BuildNoHashHasher<usize>>,
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
                cache: IndexMap::with_capacity_and_hasher(64, BuildNoHashHasher::default()),
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
                .expect(format!("Reading lookup value for element {i_idx_raw}").as_str());

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
            "uocf: Parsed {} (0x{:x}) Map Tile texture slots, loaded {} (0x{:x}) valid.",
            texidx.element_count(),
            texidx.element_count(),
            i_idx_valid,
            i_idx_valid
        );

        Ok(texmap)
    }

    /// Caches the raw BGRA5551 data file slice into memory, without decoding it to RGBA8888.
    /// This is strictly used by background preloader threads to warm up the OS filesystem.
    pub fn preload_pixel_data(&self, element_index: usize) -> Option<()> {
        let element: &Texture2DElement = self.element(element_index)?;
        let mut shared = self.shared_data.lock().unwrap();

        if let Some((_, time)) = shared.cache.get_mut(&element_index) {
            *time = std::time::Instant::now();
            return Some(());
        }

        let pixel_qty_bytes = element.pixel_qty * 2;
        let shared = &mut *shared;
        shared
            .file_reader
            .seek(SeekFrom::Start(element.file_offset))
            .ok()?;

        let mut raw_data = vec![0u8; pixel_qty_bytes];
        shared.file_reader.read_exact(&mut raw_data).ok()?;

        let arc_raw: std::sync::Arc<[u8]> = raw_data.into();
        shared.cache.insert(
            element_index,
            (std::sync::Arc::clone(&arc_raw), std::time::Instant::now()),
        );

        Some(())
    }

    pub fn get_pixel_data(
        &self,
        element_index: usize,
        now: std::time::Instant,
    ) -> Option<std::sync::Arc<[u8]>> {
        let element: &Texture2DElement = self.element(element_index)?;

        let raw_bgra5551: std::sync::Arc<[u8]> = {
            let mut shared = self.shared_data.lock().unwrap();

            // Check if the raw BGRA5551 data is already cached.
            if let Some((data, time)) = shared.cache.get_mut(&element_index) {
                *time = now;
                std::sync::Arc::clone(data)
            } else {
                // Read raw BGRA5551 from file and cache it (2 bytes/pixel).
                let pixel_qty_bytes = element.pixel_qty * 2;
                let shared = &mut *shared;
                shared
                    .file_reader
                    .seek(SeekFrom::Start(element.file_offset))
                    .ok()?;

                let mut raw_data = vec![0u8; pixel_qty_bytes];
                shared.file_reader.read_exact(&mut raw_data).ok()?;

                let arc_raw: std::sync::Arc<[u8]> = raw_data.into();
                shared
                    .cache
                    .insert(element_index, (std::sync::Arc::clone(&arc_raw), now));
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

                // PERFORMANCE NOTE:
                // We keep the entire decoding and packing process in SIMD registers to avoid
                // "SIMD-to-scalar spills". The original implementation moved data to a stack array
                // to loop over it, which forced the CPU to wait on high-latency extraction moves.
                //
                // ALGORITHM:
                // We extract the 5-6-5 bits into separate registers, then perform vectorized
                // bit-packing using wide bitwise shifts and ORs. This processes 8 pixels at
                // a time in parallel per register (16 per total chunk).

                let [lo, hi]: [u16x8; 2] = unsafe { std::mem::transmute(chunk) };

                // Extract components into u32 registers for bit-packing.
                // 1) Mask 5 bits (0x1F) for R, G, B channels.
                // 2) Shift right to extract (for G and R).
                // 3) Shift left by 3 to scale 5-bit to 8-bit (nearly, 0..31 -> 0..248).
                let b_u16_lo: u32x8 = u32x8::from((lo & u16x8::splat(0x1F)) << 3);
                let g_u16_lo: u32x8 = u32x8::from(((lo >> 5) & u16x8::splat(0x1F)) << 3);
                let r_u16_lo: u32x8 = u32x8::from(((lo >> 10) & u16x8::splat(0x1F)) << 3);
                let a_u16_lo: u32x8 = u32x8::splat(0xFF); // Full opacity for land tiles

                let b_u16_hi: u32x8 = u32x8::from((hi & u16x8::splat(0x1F)) << 3);
                let g_u16_hi: u32x8 = u32x8::from(((hi >> 5) & u16x8::splat(0x1F)) << 3);
                let r_u16_hi: u32x8 = u32x8::from(((hi >> 10) & u16x8::splat(0x1F)) << 3);
                let a_u16_hi: u32x8 = u32x8::splat(0xFF);

                // ENDIANNESS & PACKING:
                // Bit-packing as (A << 24 | B << 16 | G << 8 | R) results in memory bytes [R, G, B, A]
                // on Little-Endian systems (x86_64, aarch64), which is the standard RGBA8888
                // format expected by modern GPUs (Vulkan/Metal/DXR).
                #[allow(unused_mut)]
                let mut rgba_lo: u32x8 =
                    (a_u16_lo << 24) | (b_u16_lo << 16) | (g_u16_lo << 8) | r_u16_lo;
                #[allow(unused_mut)]
                let mut rgba_hi: u32x8 =
                    (a_u16_hi << 24) | (b_u16_hi << 16) | (g_u16_hi << 8) | r_u16_hi;

                // Handle host endianness: we want Little-Endian memory layout for RGBA [R, G, B, A]
                #[cfg(target_endian = "big")]
                {
                    rgba_lo = rgba_lo.swap_bytes();
                    rgba_hi = rgba_hi.swap_bytes();
                }

                // Efficient large stores (32-bytes at a time) instead of 16 individual 4-byte pushes.
                pixel_data.extend_from_slice(bytemuck::cast_slice(rgba_lo.as_array()));
                pixel_data.extend_from_slice(bytemuck::cast_slice(rgba_hi.as_array()));
            }

            for &p in pixel_data_u16_suffix {
                let mut pixel_16 = crate::utils::color::Bgra5551::new_from_val(p);
                pixel_16.set_a(1);
                pixel_data.extend_from_slice(pixel_16.as_rgba8888().value().to_le_bytes().as_ref());
            }
        }

        Some(pixel_data.into())
    }

    pub fn evict_idle_textures(&self, timeout: std::time::Duration) -> usize {
        let now = std::time::Instant::now();
        let mut shared = self.shared_data.lock().unwrap();
        let initial_len = shared.cache.len();

        shared
            .cache
            .retain(|_, (_, time)| now.duration_since(*time) <= timeout);

        initial_len - shared.cache.len()
    }
}
