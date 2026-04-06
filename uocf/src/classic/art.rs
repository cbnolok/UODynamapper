#![allow(dead_code)]
//! # UO Art Data Parser
//!
//! This module is responsible for loading and decoding Ultima Online art data.
//! Art data is divided into two main categories: land tiles and static items.
//!
//! ## File Formats
//!
//! ### `artidx.mul` and `art.mul`
//!
//! - **`artidx.mul`**: An index file containing records that point to art data in `art.mul`.
//! - **`art.mul`**: Contains the raw art data. The format of the data depends on whether
//!   it's a land tile or a static item.
//!
//! ### `artlegacymul.uop`
//!
//! - A UOP package that contains art data, primarily for legacy items. The data is stored
//!   as TGA files within the UOP package.
//!
//! ### Land Tiles (ID < 0x4000)
//!
//! - Land tiles are 44x44 pixels and are stored in a diamond shape. The data is a raw dump
//!   of 16-bit color values for each pixel in the diamond.
//!
//! ### Static Items (ID >= 0x4000)
//!
//! - Static items have variable dimensions and are stored using a run-length encoding (RLE)
//!   scheme. Each row of the image has a lookup table that points to the start of the RLE
//!   data for that row.
//!
//! ## Optimizations Applied
//!
//! - **Centralized Handle Caching**: The `ArtMap` structure keeps `artidx.mul` and `art.mul`
//!   descriptors, as well as `UopPackage` when present, actively open to avoid repeated OS I/O.
//! - **Allocation-Free Decode Hot Path**: Land tiles decode directly into the caller-provided
//!   output buffer, and static tiles decode straight into their final RGBA surface without
//!   per-row staging vectors.
//! - **Branch-Light Color Expansion**: A lazily built 16-bit to RGBA8888 lookup table removes
//!   repeated bit-extraction work from the inner loops.

crate::eyre_imports!();

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bytemuck::cast_slice_mut;

use crate::generic_index::IndexFile;
use crate::uop::package::UopPackage;
use crate::utils::color::color_lut;

const ART_ITEM_ID_OFFSET: u32 = 0x4000;
const LAND_DIMENSION: usize = 44;
const LAND_HALF_ROWS: usize = LAND_DIMENSION / 2;
const LAND_DIAMOND_PIXEL_COUNT: usize = 1936;
const LAND_DIAMOND_BYTE_COUNT: usize = LAND_DIAMOND_PIXEL_COUNT * 2;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const STATIC_HEADER_BYTES: usize = 8;
const LOOKUP_ENTRY_BYTES: usize = 2;

#[inline(always)]
fn read_u16_le(bytes: &[u8], offset: usize) -> eyre::Result<u16> {
    let end = offset + LOOKUP_ENTRY_BYTES;
    if end > bytes.len() {
        eyre::bail!(
            "Unexpected end of art data while reading u16 at byte offset {}.",
            offset
        );
    }

    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

#[inline(always)]
fn read_u32_le(bytes: &[u8], offset: usize) -> eyre::Result<u32> {
    let end = offset + std::mem::size_of::<u32>();
    if end > bytes.len() {
        eyre::bail!(
            "Unexpected end of art data while reading u32 at byte offset {}.",
            offset
        );
    }

    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

#[inline(always)]
fn read_unaligned_u16_le(bytes: &[u8], offset: usize) -> u16 {
    debug_assert!(offset + LOOKUP_ENTRY_BYTES <= bytes.len());

    let raw_ptr = unsafe { bytes.as_ptr().add(offset).cast::<u16>() };
    u16::from_le(unsafe { raw_ptr.read_unaligned() })
}

#[inline(always)]
fn decode_land_tile_from_raw(
    raw_data: &[u8],
    pixel_data_out: &mut [u8; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL],
) -> eyre::Result<()> {
    if raw_data.len() < LAND_DIAMOND_BYTE_COUNT {
        eyre::bail!("Read buffer length below structural requirements for land diamond.");
    }

    pixel_data_out.fill(0);

    let pixel_words = cast_slice_mut::<u8, u32>(pixel_data_out.as_mut_slice());
    let lut = color_lut();
    let mut source_offset = 0usize;

    // The diamond shape is stored as a series of horizontal lines.
    // The first half of the diamond grows from a width of 2 to 44.
    let mut x = LAND_DIMENSION / 2;
    let mut y = 0usize;
    let mut line_width = 2usize;

    for _ in 0..LAND_HALF_ROWS {
        x -= 1;
        let row_start = y * LAND_DIMENSION + x;
        let row_pixels = &mut pixel_words[row_start..row_start + line_width];

        for pixel in row_pixels {
            *pixel = lut[read_unaligned_u16_le(raw_data, source_offset) as usize];
            source_offset += LOOKUP_ENTRY_BYTES;
        }

        y += 1;
        line_width += 2;
    }

    // The second half of the diamond shrinks from a width of 44 to 2.
    x = 0;
    line_width = LAND_DIMENSION;

    for _ in 0..LAND_HALF_ROWS {
        let row_start = y * LAND_DIMENSION + x;
        let row_pixels = &mut pixel_words[row_start..row_start + line_width];

        for pixel in row_pixels {
            *pixel = lut[read_unaligned_u16_le(raw_data, source_offset) as usize];
            source_offset += LOOKUP_ENTRY_BYTES;
        }

        x += 1;
        y += 1;
        line_width -= 2;
    }

    Ok(())
}

#[inline(always)]
fn decode_static_tile_from_raw(raw_data: &[u8]) -> eyre::Result<(u16, u16, Vec<u8>)> {
    let _flags = read_u32_le(raw_data, 0)?;
    let width = read_u16_le(raw_data, 4)?;
    let height = read_u16_le(raw_data, 6)?;

    if width == 0 || height == 0 {
        eyre::bail!("Invalid static tile dimensions.");
    }

    let lookup_table_bytes = height as usize * LOOKUP_ENTRY_BYTES;
    let data_start = STATIC_HEADER_BYTES + lookup_table_bytes;
    if data_start > raw_data.len() {
        eyre::bail!("Static tile lookup table extends past the raw art payload.");
    }

    let width_usize = width as usize;
    let mut pixel_data_out = vec![0u8; width_usize * height as usize * RGBA_BYTES_PER_PIXEL];
    let pixel_words = cast_slice_mut::<u8, u32>(pixel_data_out.as_mut_slice());
    let lut = color_lut();

    // The lookup table contains the offset to the start of each row's RLE data.
    for y in 0..height as usize {
        let lookup_offset = STATIC_HEADER_BYTES + y * LOOKUP_ENTRY_BYTES;
        let lookup = read_u16_le(raw_data, lookup_offset)? as usize;
        let mut row_offset = data_start + lookup * LOOKUP_ENTRY_BYTES;
        if row_offset > raw_data.len() {
            eyre::bail!("Static tile row {} starts outside the raw art payload.", y);
        }

        let row_start = y * width_usize;
        let mut x = 0usize;

        // Each row is a series of RLE chunks.
        loop {
            if row_offset + 2 * LOOKUP_ENTRY_BYTES > raw_data.len() {
                eyre::bail!("Static tile row {} is truncated in the RLE stream.", y);
            }

            let x_offset = read_unaligned_u16_le(raw_data, row_offset) as usize;
            row_offset += LOOKUP_ENTRY_BYTES;
            let x_run = read_unaligned_u16_le(raw_data, row_offset) as usize;
            row_offset += LOOKUP_ENTRY_BYTES;

            // An offset and run of 0 marks the end of the row.
            if x_offset == 0 && x_run == 0 {
                break;
            }

            x = x
                .checked_add(x_offset)
                .ok_or_else(|| eyre!("Static tile row {} overflowed its x cursor.", y))?;

            let run_end = x
                .checked_add(x_run)
                .ok_or_else(|| eyre!("Static tile row {} overflowed its run length.", y))?;

            if run_end > width_usize {
                eyre::bail!(
                    "Static tile row {} run exceeds width (run_end {}, width {}).",
                    y,
                    run_end,
                    width_usize
                );
            }

            let row_pixels = &mut pixel_words[row_start + x..row_start + run_end];
            let run_byte_count = x_run * LOOKUP_ENTRY_BYTES;

            if row_offset + run_byte_count > raw_data.len() {
                eyre::bail!("Static tile row {} color payload is truncated.", y);
            }

            let mut color_offset = row_offset;
            for pixel in row_pixels {
                *pixel = lut[read_unaligned_u16_le(raw_data, color_offset) as usize];
                color_offset += LOOKUP_ENTRY_BYTES;
            }

            row_offset = color_offset;
            x = run_end;
        }
    }

    Ok((width, height, pixel_data_out))
}

pub struct ArtMap {
    client_path: PathBuf,
    idx_file: Option<IndexFile>,
    art_file: Option<Mutex<BufReader<File>>>,
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

            let mul_handle = File::open(&mul_path)
                .wrap_err_with(|| format!("Failed to open {}", mul_path.display()))?;
            art_file = Some(Mutex::new(BufReader::new(mul_handle)));
            log::info!("uocf: Loaded classic Art.mul format");
        }

        if uop_path.exists() {
            uop_package = Some(UopPackage::load(&uop_path)?);
            log::info!("uocf: Loaded newer artlegacymul.uop format");
        }

        if idx_file.is_none() && uop_package.is_none() {
            eyre::bail!(
                "Neither artidx.mul/art.mul nor artlegacymul.uop found in the client path."
            );
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
    pub fn get_raw_art_data(&self, art_id: u32, scratch_buffer: &mut Vec<u8>) -> eyre::Result<()> {
        scratch_buffer.clear();

        // Attempt the classic mul path first to preserve the existing compatibility order.
        if let (Some(idx), Some(art_mutex)) = (&self.idx_file, &self.art_file) {
            if let Ok(entry) = idx.element(art_id as usize) {
                if let (Some(lookup), Some(size)) = (entry.lookup(), entry.len()) {
                    if size > 0 {
                        let target_size = size as usize;
                        scratch_buffer.resize(target_size, 0);

                        let mut art_reader = art_mutex.lock().unwrap();
                        art_reader.seek(SeekFrom::Start(lookup as u64))?;
                        art_reader.read_exact(scratch_buffer)?;

                        return Ok(());
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            let file_name = format!("build/artlegacymul/{:08}.tga", art_id);
            let hash = crate::uop::hash::hash_file_name_single(&file_name);
            if let Some(file) = uop.get_file_by_hash(hash) {
                file.unpack_to(scratch_buffer)?;
                return Ok(());
            }
        }

        eyre::bail!("Could not find art data for ID {}", art_id);
    }

    /// Reads and parses an Art Land tile (44x44 isometric diamond surface).
    /// Outputs precisely the 44x44 4-byte pixels straight into `pixel_data_out`.
    pub fn decode_land_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
        pixel_data_out: &mut [u8; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL],
    ) -> eyre::Result<()> {
        self.get_raw_art_data(art_id, scratch_raw_buffer)?;
        decode_land_tile_from_raw(scratch_raw_buffer, pixel_data_out)
    }

    /// Reads and parses an Art Static tile (RLE encoded image).
    /// Modifies the `scratch_raw_buffer` and returns `(width, height, pixel_data)`.
    pub fn decode_static_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
    ) -> eyre::Result<(u16, u16, Vec<u8>)> {
        self.get_raw_art_data(art_id, scratch_raw_buffer)?;
        decode_static_tile_from_raw(scratch_raw_buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_land_tile_from_raw, decode_static_tile_from_raw, LAND_DIMENSION};

    fn rgba_at(pixel_data: &[u8], x: usize, y: usize, width: usize) -> [u8; 4] {
        let offset = (y * width + x) * 4;
        [
            pixel_data[offset],
            pixel_data[offset + 1],
            pixel_data[offset + 2],
            pixel_data[offset + 3],
        ]
    }

    #[test]
    fn land_decode_clears_transparent_corners() {
        let raw_data = vec![0xFFu8; 1936 * 2];
        let mut pixel_data = [0x7Fu8; LAND_DIMENSION * LAND_DIMENSION * 4];

        decode_land_tile_from_raw(&raw_data, &mut pixel_data).unwrap();

        assert_eq!(rgba_at(&pixel_data, 0, 0, LAND_DIMENSION), [0, 0, 0, 0]);
        assert_eq!(rgba_at(&pixel_data, LAND_DIMENSION - 1, 0, LAND_DIMENSION), [0, 0, 0, 0]);
        assert_eq!(
            rgba_at(&pixel_data, LAND_DIMENSION / 2, LAND_DIMENSION / 2, LAND_DIMENSION),
            [248, 248, 248, 255]
        );
    }

    #[test]
    fn static_decode_preserves_rle_positions() {
        let raw_data = vec![
            0, 0, 0, 0, // flags
            4, 0, // width
            1, 0, // height
            0, 0, // lookup for row 0
            1, 0, // x_offset
            2, 0, // x_run
            0x00, 0x7C, // red
            0xE0, 0x03, // green
            0, 0, // row terminator
            0, 0,
        ];

        let (width, height, pixel_data) = decode_static_tile_from_raw(&raw_data).unwrap();

        assert_eq!((width, height), (4, 1));
        assert_eq!(rgba_at(&pixel_data, 0, 0, width as usize), [0, 0, 0, 0]);
        assert_eq!(rgba_at(&pixel_data, 1, 0, width as usize), [248, 0, 0, 255]);
        assert_eq!(rgba_at(&pixel_data, 2, 0, width as usize), [0, 248, 0, 255]);
        assert_eq!(rgba_at(&pixel_data, 3, 0, width as usize), [0, 0, 0, 0]);
    }
}
