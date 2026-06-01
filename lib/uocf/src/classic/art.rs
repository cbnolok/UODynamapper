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
//! - A UOP package that contains art data, primarily for Classic Client items. The data is stored
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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::cast_slice_mut;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::uop_container::package::UopPackage;
use crate::utils::color::color_lut;

const ART_ITEM_ID_OFFSET: u32 = 0x4000;
pub const ART_LEGACY_UOP_MAX_ID_EXCLUSIVE: u32 = 0x14000;
pub const LAND_DIMENSION: usize = 44;
const LAND_HALF_ROWS: usize = LAND_DIMENSION / 2;
pub const LAND_DIAMOND_PIXEL_COUNT: usize = LAND_HALF_ROWS * (LAND_HALF_ROWS + 1) * 2;
const LAND_DIAMOND_BYTE_COUNT: usize = LAND_DIAMOND_PIXEL_COUNT * 2;
pub const RGBA_BYTES_PER_PIXEL: usize = 4;
const STATIC_HEADER_BYTES: usize = 8;
const LOOKUP_ENTRY_BYTES: usize = 2;

fn classic_art_payload_is_structurally_valid(art_id: u32, size: u32) -> bool {
    if art_id < ART_ITEM_ID_OFFSET {
        size as usize >= LAND_DIAMOND_BYTE_COUNT
    } else {
        size as usize >= STATIC_HEADER_BYTES
    }
}

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

fn read_unaligned_u16_le(bytes: &[u8], offset: usize) -> u16 {
    debug_assert!(offset + LOOKUP_ENTRY_BYTES <= bytes.len());

    let raw_ptr = unsafe { bytes.as_ptr().add(offset).cast::<u16>() };
    u16::from_le(unsafe { raw_ptr.read_unaligned() })
}

pub fn decode_land_tile_from_raw(
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

pub fn decode_static_tile_from_raw(raw_data: &[u8]) -> eyre::Result<(u16, u16, Vec<u8>)> {
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

use memmap2::Mmap;

#[derive(Clone)]
pub struct ArtMap {
    client_path: PathBuf,
    idx_file: Option<IndexFile>,
    art_mmap: Option<Arc<Mmap>>,
    uop_package: Option<UopPackage>,
    verdata: Option<Arc<Verdata>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtSource {
    Mul,
    CcUop,
    EcUop,
    Any,
}

fn uop_art_candidate_hashes(art_id: u32, source: ArtSource) -> ([u64; 6], usize) {
    let mut hashes = [0u64; 6];
    let mut count = 0usize;
    if source == ArtSource::CcUop || source == ArtSource::Any {
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/artlegacymul/", art_id, ".tga");
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/artlegacy/", art_id, ".dat");
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/art/", art_id, ".tga");
    }
    if source == ArtSource::EcUop || source == ArtSource::Any {
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/tileartlegacy/", art_id, ".dds");
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/tileartlegacy/", art_id, ".tga");
        push_uop_art_candidate_hash(&mut hashes, &mut count, "build/legacytexture/", art_id, ".tga");
    }
    (hashes, count)
}

fn push_uop_art_candidate_hash(
    hashes: &mut [u64; 6],
    count: &mut usize,
    prefix: &str,
    art_id: u32,
    suffix: &str,
) {
    let mut path_buf = [0u8; 64];
    let len = {
        let full_len = path_buf.len();
        let mut slice = &mut path_buf[..];
        write!(slice, "{prefix}{art_id:08}{suffix}").expect("art UOP candidate path fits stack buffer");
        full_len - slice.len()
    };
    let path = unsafe { std::str::from_utf8_unchecked(&path_buf[..len]) };
    hashes[*count] = crate::uop_container::hash::hash_file_name_single(path);
    *count += 1;
}

impl ArtMap {
    pub fn load_standalone_uop(uop: UopPackage) -> Self {
        Self {
            client_path: PathBuf::new(),
            idx_file: None,
            art_mmap: None,
            uop_package: Some(uop),
            verdata: None,
        }
    }

    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref().to_path_buf();
        let idx_path = client_path.join("artidx.mul");
        let mul_path = client_path.join("art.mul");

        // Find artlegacymul.uop case-insensitively
        let uop_candidates = ["artlegacymul.uop", "artLegacyMUL.uop", "LegacyTexture.uop"];
        let mut uop_path = None;
        for &name in &uop_candidates {
            let path = client_path.join(name);
            if path.exists() {
                uop_path = Some(path);
                break;
            }
        }

        let mut idx_file = None;
        let mut art_mmap = None;
        let mut uop_package = None;

        if idx_path.exists() && mul_path.exists() {
            idx_file = Some(IndexFile::load(idx_path.clone())?);

            let mul_handle = File::open(&mul_path)
                .wrap_err_with(|| format!("Failed to open {}", mul_path.display()))?;
            let mmap = unsafe { Mmap::map(&mul_handle)? };
            art_mmap = Some(Arc::new(mmap));
            log::info!("uocf: Loaded classic Art.mul format (memory-mapped)");
        }

        if let Some(path) = uop_path {
            uop_package = Some(UopPackage::load(&path)?);
            log::info!("uocf: Loaded newer art UOP format from {}", path.display());
        }

        if idx_file.is_none() && uop_package.is_none() {
            eyre::bail!(
                "Neither artidx.mul/art.mul nor artlegacymul.uop found in the client path."
            );
        }

        Ok(Self {
            client_path,
            idx_file,
            art_mmap,
            uop_package,
            verdata: None,
        })
    }

    pub fn with_uop(mut self, uop: UopPackage) -> Self {
        self.uop_package = Some(uop);
        self
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    /// Fetches the raw compressed bytes for a given `art_id` from a specific source.
    pub fn get_raw_art_data_from_source(
        &self,
        art_id: u32,
        source: ArtSource,
        scratch_buffer: &mut Vec<u8>,
    ) -> eyre::Result<()> {
        scratch_buffer.clear();

        if source == ArtSource::Mul || source == ArtSource::Any {
            if let Some(verdata) = &self.verdata {
                if let Some(bytes) = verdata.read_patch(VerFileId::Art, art_id as i32)? {
                    if classic_art_payload_is_structurally_valid(art_id, bytes.len() as u32) {
                        scratch_buffer.extend_from_slice(&bytes);
                        return Ok(());
                    }
                }
            }

            if let (Some(idx), Some(art_mmap)) = (&self.idx_file, &self.art_mmap) {
                if let Ok(entry) = idx.element(art_id as usize) {
                    let index_patch = self
                        .verdata
                        .as_ref()
                        .and_then(|verdata| verdata.index_patch(VerFileId::ArtIdx, art_id as i32));
                    let index_values = index_patch.or_else(|| {
                        Some((entry.lookup()?, entry.len()?, entry.extra().unwrap_or(0)))
                    });
                    if let Some((lookup, size, _extra)) = index_values {
                        if classic_art_payload_is_structurally_valid(art_id, size) {
                            let lookup = lookup as usize;
                            let size = size as usize;
                            let end = lookup + size;

                            if end > art_mmap.len() {
                                eyre::bail!(
                                    "Art index points outside mmap range for art_id {}",
                                    art_id
                                );
                            }

                            scratch_buffer.extend_from_slice(&art_mmap[lookup..end]);
                            return Ok(());
                        }
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            let (hashes, hash_count) = uop_art_candidate_hashes(art_id, source);
            for &hash in &hashes[..hash_count] {
                if let Some(file) = uop.get_file_by_hash(hash) {
                    file.unpack_to(scratch_buffer)?;
                    return Ok(());
                }
            }
        }

        eyre::bail!(
            "Could not find art data for ID {} in source {:?}",
            art_id,
            source
        );
    }

    /// Fetches the raw compressed bytes for a given `art_id` without parsing its internal format.
    /// Puts the result into the provided `scratch_buffer` to avoid memory allocation.
    pub fn get_raw_art_data(&self, art_id: u32, scratch_buffer: &mut Vec<u8>) -> eyre::Result<()> {
        self.get_raw_art_data_from_source(art_id, ArtSource::Any, scratch_buffer)
    }

    /// Reads and parses an Art Land tile (44x44 isometric diamond surface) from a specific source.
    pub fn decode_land_tile_from_source(
        &self,
        art_id: u32,
        source: ArtSource,
        scratch_raw_buffer: &mut Vec<u8>,
        pixel_data_out: &mut [u8; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL],
    ) -> eyre::Result<()> {
        self.get_raw_art_data_from_source(art_id, source, scratch_raw_buffer)?;
        decode_land_tile_from_raw(scratch_raw_buffer, pixel_data_out)
    }

    /// Reads and parses an Art Land tile (44x44 isometric diamond surface).
    /// Outputs precisely the 44x44 4-byte pixels straight into `pixel_data_out`.
    pub fn decode_land_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
        pixel_data_out: &mut [u8; LAND_DIMENSION * LAND_DIMENSION * RGBA_BYTES_PER_PIXEL],
    ) -> eyre::Result<()> {
        self.decode_land_tile_from_source(
            art_id,
            ArtSource::Any,
            scratch_raw_buffer,
            pixel_data_out,
        )
    }

    /// Reads and parses an Art Static tile (RLE encoded image) from a specific source.
    pub fn decode_static_tile_from_source(
        &self,
        art_id: u32,
        source: ArtSource,
        scratch_raw_buffer: &mut Vec<u8>,
    ) -> eyre::Result<(u16, u16, Vec<u8>)> {
        self.get_raw_art_data_from_source(art_id, source, scratch_raw_buffer)?;

        // Check if it's a TGA (common in UOP versions)
        if scratch_raw_buffer.len() >= 18
            && (scratch_raw_buffer.ends_with(b"TRUEVISION-XFILE.\0")
                || is_probably_tga(scratch_raw_buffer))
        {
            let img =
                image::load_from_memory_with_format(scratch_raw_buffer, image::ImageFormat::Tga)?;
            let rgba = img.to_rgba8();
            return Ok((rgba.width() as u16, rgba.height() as u16, rgba.into_raw()));
        }

        decode_static_tile_from_raw(scratch_raw_buffer)
    }

    /// Reads and parses an Art Static tile (RLE encoded image).
    /// Modifies the `scratch_raw_buffer` and returns `(width, height, pixel_data)`.
    pub fn decode_static_tile(
        &self,
        art_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
    ) -> eyre::Result<(u16, u16, Vec<u8>)> {
        self.decode_static_tile_from_source(art_id, ArtSource::Any, scratch_raw_buffer)
    }

    pub fn max_id(&self) -> u32 {
        self.max_id_for_source(ArtSource::Any)
    }

    pub fn max_id_for_source(&self, source: ArtSource) -> u32 {
        let mul_max_id = self
            .idx_file
            .as_ref()
            .map(|idx| idx.element_count() as u32)
            .unwrap_or(0);
        let uop_max_id = if self.uop_package.is_some()
            && matches!(source, ArtSource::CcUop | ArtSource::EcUop | ArtSource::Any)
        {
            ART_LEGACY_UOP_MAX_ID_EXCLUSIVE
        } else {
            0
        };

        match source {
            ArtSource::Mul => mul_max_id,
            ArtSource::CcUop | ArtSource::EcUop => uop_max_id,
            ArtSource::Any => mul_max_id.max(uop_max_id),
        }
    }

    pub fn has_id(&self, art_id: u32) -> bool {
        self.has_id_from_source(art_id, ArtSource::Any)
    }

    pub fn has_id_from_source(&self, art_id: u32, source: ArtSource) -> bool {
        if let Some(verdata) = &self.verdata {
            if source == ArtSource::Mul || source == ArtSource::Any {
                if let Some(entry) = verdata.entry(VerFileId::Art, art_id as i32) {
                    if classic_art_payload_is_structurally_valid(art_id, entry.length as u32) {
                        return true;
                    }
                }
            }
        }

        if (source == ArtSource::Mul || source == ArtSource::Any) && self.idx_file.is_some() {
            let idx = self.idx_file.as_ref().expect("checked above");
            if let Ok(entry) = idx.element(art_id as usize) {
                let index_patch = self
                    .verdata
                    .as_ref()
                    .and_then(|verdata| verdata.index_patch(VerFileId::ArtIdx, art_id as i32));
                let index_values = index_patch.or_else(|| {
                    Some((entry.lookup()?, entry.len()?, entry.extra().unwrap_or(0)))
                });
                if let Some((_lookup, size, _extra)) = index_values {
                    if classic_art_payload_is_structurally_valid(art_id, size) {
                        return true;
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            let (hashes, hash_count) = uop_art_candidate_hashes(art_id, source);
            for &hash in &hashes[..hash_count] {
                if uop.get_file_by_hash(hash).is_some() {
                    return true;
                }
            }
            false
        } else {
            false
        }
    }
}

fn is_probably_tga(data: &[u8]) -> bool {
    if data.len() < 18 {
        return false;
    }
    // TGA header: id_len(1), color_map_type(1), image_type(1), ...
    // image_type 2 is uncompressed RGB/RGBA, 10 is RLE RGB/RGBA
    let image_type = data[2];
    (image_type == 2 || image_type == 10) && data[1] <= 1
}
