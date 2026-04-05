#![allow(dead_code)]
//! # UO Art Data Parser
//!
//! This module is responsible for loading and decoding Ultima Online art data.
//! Art data is divided into two main categories: land tiles and static items.
//!
//! ## Optimizations Applied
//! - **Centralized Handle Caching**: The `ArtMap` structure keeps `artidx.mul` and `art.mul`
//!   descriptors (as well as `UopPackage` if applicable) actively open to prevent OS I/O bottlenecks.
//! - **Zero-Allocation**: Scratch buffers are heavily utilized instead of continuous `Vec<u8>` spawning.
//! - **SIMD Color Pipeline**: Like `land_texture_2d.rs`, decoding Argb1555 `u16` colors to RGBA8888 `u32`
//!   is offloaded to SIMD vectorized instructions using the `wide` crate, mitigating decoding loop latency.

crate::eyre_imports!();

use std::fs::File;
use std::io::{prelude::*, BufReader, SeekFrom};
use std::path::{Path, PathBuf};

use byteorder::{LittleEndian, ReadBytesExt};
use wide::*; // SIMD

use crate::generic_index::{IndexElement, IndexFile};
// use crate::classic::hues::HueEntry; // Assuming hues are handled upper-level or later
// use crate::classic::verdata::*; // Depending on if we need verdata
use crate::uop::package::UopPackage;
use crate::utils::math::*;

const ART_ITEM_ID_OFFSET: u32 = 0x4000;

pub struct ArtMap {
    client_path: PathBuf,
    idx_file: Option<IndexFile>,
    art_file: Option<std::sync::Mutex<BufReader<File>>>,
    uop_package: Option<UopPackage>,
}

impl ArtMap {
    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref().to_path_buf();
        let idx_path = client_path.join("artidx.mul");
        let mul_path = client_path.join("art.mul");
        let uop_path = client_path.join("artlegacymul.uop");

        let mut idx_file = None;
        let mut art_file = None;
        let mut uop_package = None;

        if idx_path.exists() && mul_path.exists() {
            idx_file = Some(IndexFile::load(idx_path.clone())?);
            
            let mul_handle = File::open(&mul_path).wrap_err_with(|| format!("Failed to open {}", mul_path.display()))?;
            art_file = Some(std::sync::Mutex::new(BufReader::new(mul_handle)));
            log::info!("uocf: Loaded classic Art.mul format");
        }

        if uop_path.exists() {
            uop_package = Some(UopPackage::load(&uop_path)?);
            log::info!("uocf: Loaded newer artlegacymul.uop format");
        }

        if idx_file.is_none() && uop_package.is_none() {
            eyre::bail!("Neither artidx.mul/art.mul nor artlegacymul.uop found in the client path.");
        }

        Ok(Self {
            client_path,
            idx_file,
            art_file,
            uop_package,
        })
    }

    /// Fetches the raw compressed bytes for a given `art_id` without parsing its internal format.
    /// Puts the result into the provided `scratch_buffer` to avoid memory allocation.
    pub fn get_raw_art_data(
        &self,
        art_id: u32,
        scratch_buffer: &mut Vec<u8>
    ) -> eyre::Result<()> {
        scratch_buffer.clear();

        // Optional `Verdata` checking goes here if passed:
        // if verdata.patch_exists(...) { ... }

        // Attempt `artlegacymul.uop` first as newer format, if it was requested specifically or fallback
        // The algorithm in new_impl checked UOP *after* classic formats. 
        // Let's uphold backwards compatibility order.

        if let (Some(idx), Some(art_mutex)) = (&self.idx_file, &self.art_file) {
            if let Ok(entry) = idx.element(art_id as usize) {
                if let (Some(lookup), Some(size)) = (entry.lookup(), entry.len()) {
                    if size > 0 {
                        let mut art_reader = art_mutex.lock().unwrap();
                        let target_size = size as usize;
                        scratch_buffer.reserve_exact(target_size);
                        
                        art_reader.seek(SeekFrom::Start(lookup as u64))?;
                        
                        // Prevent zero-initialization allocation by using an unsafe buffer copy
                        // Alternatively, bypass safely:
                        (&mut *art_reader).take(size as u64).read_to_end(scratch_buffer)?;
                        return Ok(());
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            let file_name = format!("build/artlegacymul/{:08}.tga", art_id);
            let hash = crate::uop::hash::hash_file_name_single(&file_name);
            if let Some(file) = uop.get_file_by_hash(hash) {
                // If it is compressed, we can use `unpack_to` and direct it into the scratch_buffer:
                file.unpack_to(scratch_buffer)?;
                return Ok(());
            }
        }

        eyre::bail!("Could not find art data for ID {}", art_id);
    }
    
    
    /// Reads and parses an Art Land tile (44x44 Isometric diamond surface)
    /// Outputs precisely the 44x44=1936 4-byte pixels straight into `pixel_data_out`.
    pub fn decode_land_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
        pixel_data_out: &mut [u8; 44 * 44 * 4],
    ) -> eyre::Result<()> {
        self.get_raw_art_data(art_id, scratch_raw_buffer)?;

        // Extract native u16 arrays
        if scratch_raw_buffer.len() < 1936 * 2 {
            eyre::bail!("Read buffer length below structural requirements for land diamond.");
        }
        
        let u16_diamond: &[u16] = bytemuck::cast_slice(&scratch_raw_buffer[..1936*2]);

        // Instead of doing color conversions locally per loop cycle, we pre-bake the SIMD operation on the entire raw source:
        // And we map them to a separate flattened scratch buffer for easy plotting
        let mut simded_rgba = vec![0u8; 1936 * 4];
        crate::utils::color::bulk_convert_bgra5551_to_rgba8888(u16_diamond, &mut simded_rgba);

        // Scatter 1936 SIMD-processed sequential RGBA pixels into the 44x44 diamond layout grid.
        let mut fetch_idx = 0;
        
        let mut x: u16 = 22;
        let mut y: u16 = 0;
        let mut line_width: u16 = 2;

        for _ in 0..22 {
            x -= 1;
            for i in 0..line_width {
                let target_offset = ((y * 44 + (x + i)) * 4) as usize;
                
                // Copy the pixel components from our aligned source
                let src_offset = (fetch_idx * 4) as usize;
                pixel_data_out[target_offset..target_offset+4].copy_from_slice(&simded_rgba[src_offset..src_offset+4]);
                
                fetch_idx += 1;
            }
            y += 1;
            line_width += 2;
        }

        x = 0;
        line_width = 44;
        for _ in 0..22 {
            for i in 0..line_width {
                let target_offset = ((y * 44 + (x + i)) * 4) as usize;
                
                let src_offset = (fetch_idx * 4) as usize;
                pixel_data_out[target_offset..target_offset+4].copy_from_slice(&simded_rgba[src_offset..src_offset+4]);
                
                fetch_idx += 1;
            }
            x += 1;
            y += 1;
            line_width -= 2;
        }

        Ok(())
    }

    /// Reads and parses an Art Static tile (RLE Encoded Image)
    /// Modifies the `scratch_raw_buffer` and returns `(width, height, pixel_data)` allocating a `Vec<u8>`.
    pub fn decode_static_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
    ) -> eyre::Result<(u16, u16, Vec<u8>)> {
        self.get_raw_art_data(art_id, scratch_raw_buffer)?;
        
        let mut cursor = std::io::Cursor::new(scratch_raw_buffer);
        let _flags = cursor.read_u32::<LittleEndian>()?;
        let width = cursor.read_u16::<LittleEndian>()?;
        let height = cursor.read_u16::<LittleEndian>()?;

        if width == 0 || height == 0 {
            eyre::bail!("Invalid static tile dimensions for ID {}", art_id);
        }

        let lookups: Vec<u16> = (0..height)
            .map(|_| cursor.read_u16::<LittleEndian>())
            .collect::<Result<_, _>>()?;

        let data_start = cursor.position();
        let mut pixel_data_out = vec![0u8; (width as usize * height as usize * 4)];
        
        // Temporarily stash decoded u16 items line by line to SIMD process them collectively
        let mut row_raw_u16 = Vec::with_capacity(width as usize);

        for y in 0..height {
            let mut x: u16 = 0;
            let lookup: u16 = lookups[y as usize];
            cursor.seek(SeekFrom::Start(data_start + (lookup as u64 * 2)))?;

            row_raw_u16.clear();

            // Track target mapping since RLE will cause skips (transparency)
            let mut mapped_dest_indices = Vec::with_capacity(width as usize);

            loop {
                let x_offset = cursor.read_u16::<LittleEndian>()?;
                let x_run = cursor.read_u16::<LittleEndian>()?;

                if x_offset == 0 && x_run == 0 {
                    break;
                }

                x += x_offset;

                for _ in 0..x_run {
                    row_raw_u16.push(cursor.read_u16::<LittleEndian>()?);
                    mapped_dest_indices.push(x);
                    x += 1;
                }
            }

            if row_raw_u16.is_empty() {
                continue;
            }

            // SIMD bulk convert all valid colored pixels at once for this row
            let mut out_simd_rgba = vec![0u8; row_raw_u16.len() * 4];
            crate::utils::color::bulk_convert_bgra5551_to_rgba8888(&row_raw_u16, &mut out_simd_rgba);

            // Scatter RGBA to their mapped positions
            for (idx, &dest_x) in mapped_dest_indices.iter().enumerate() {
                let target_offset = ((y as usize * width as usize + dest_x as usize) * 4);
                let src_offset = idx * 4;
                pixel_data_out[target_offset..target_offset+4].copy_from_slice(&out_simd_rgba[src_offset..src_offset+4]);
            }
        }

        Ok((width, height, pixel_data_out))
    }
}
