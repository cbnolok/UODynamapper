#![allow(dead_code)]
//! Classic client gump art loader and decoder.
//!
//! Classic gumps are stored in `gumpidx.mul`/`gumpart.mul`. The index `extra`
//! field packs dimensions as `(width << 16) | height`, and the art payload is
//! row-based RLE:
//!
//! - `height` little-endian `u32` lookup entries.
//! - Lookup values are offsets in 4-byte units from the start of the payload.
//! - RLE entries are `(color15: u16, run: u16)` pairs.
//! - Color `0` is transparent and advances the row cursor without writing.

crate::eyre_imports!();

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytemuck::cast_slice_mut;
use memmap2::Mmap;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::uop_container::package::{LoadMode, UopPackage};
use crate::utils::color::color_lut;

pub const RGBA_BYTES_PER_PIXEL: usize = 4;
const LOOKUP_ENTRY_BYTES: usize = 4;
const RLE_ENTRY_BYTES: usize = 4;
const UOP_GUMP_HEADER_BYTES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GumpDimensions {
    pub width: u16,
    pub height: u16,
}

fn read_u32_le(bytes: &[u8], offset: usize) -> eyre::Result<u32> {
    let end = offset + std::mem::size_of::<u32>();
    if end > bytes.len() {
        eyre::bail!(
            "Unexpected end of gump data while reading u32 at byte offset {}.",
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
fn read_u32_le_prechecked(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn dimensions_from_extra(extra: u32) -> eyre::Result<GumpDimensions> {
    let width = (extra >> 16) as u16;
    let height = (extra & 0xFFFF) as u16;
    if width == 0 || height == 0 {
        eyre::bail!("Invalid gump dimensions in index extra field.");
    }

    Ok(GumpDimensions { width, height })
}

fn classic_gump_payload_is_structurally_valid(size: u32, dimensions: GumpDimensions) -> bool {
    size as usize >= dimensions.height as usize * LOOKUP_ENTRY_BYTES
}

pub fn decode_gump_from_raw(
    raw_data: &[u8],
    width: u16,
    height: u16,
) -> eyre::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        eyre::bail!("Invalid gump dimensions.");
    }

    let width_usize = width as usize;
    let height_usize = height as usize;
    let lookup_table_bytes = height_usize * LOOKUP_ENTRY_BYTES;
    if lookup_table_bytes > raw_data.len() {
        eyre::bail!("Gump lookup table extends past the raw payload.");
    }

    let pixel_bytes = width_usize
        .checked_mul(height_usize)
        .and_then(|pixels| pixels.checked_mul(RGBA_BYTES_PER_PIXEL))
        .ok_or_else(|| eyre!("Gump dimensions overflow decoded RGBA size."))?;
    let mut pixel_data_out = vec![0u8; pixel_bytes];
    let pixel_words = cast_slice_mut::<u8, u32>(pixel_data_out.as_mut_slice());
    let lut = color_lut();

    for y in 0..height_usize {
        let lookup = read_u32_le_prechecked(raw_data, y * LOOKUP_ENTRY_BYTES) as usize;
        let mut row_offset = lookup
            .checked_mul(RLE_ENTRY_BYTES)
            .ok_or_else(|| eyre!("Gump row {} lookup offset overflowed.", y))?;
        if row_offset > raw_data.len() {
            eyre::bail!("Gump row {} starts outside the raw payload.", y);
        }

        let row_start = y * width_usize;
        let mut x = 0usize;

        while x < width_usize {
            if row_offset + RLE_ENTRY_BYTES > raw_data.len() {
                eyre::bail!("Gump row {} is truncated in the RLE stream.", y);
            }

            let rle_pair = read_u32_le_prechecked(raw_data, row_offset);
            row_offset += RLE_ENTRY_BYTES;
            let color = rle_pair as u16;
            let run = (rle_pair >> 16) as usize;

            if run == 0 {
                eyre::bail!("Gump row {} contains a zero-length run.", y);
            }

            let run_end = x
                .checked_add(run)
                .ok_or_else(|| eyre!("Gump row {} overflowed its run length.", y))?;
            if run_end > width_usize {
                eyre::bail!(
                    "Gump row {} run exceeds width (run_end {}, width {}).",
                    y,
                    run_end,
                    width_usize
                );
            }

            if color != 0 {
                let rgba = lut[color as usize];
                pixel_words[row_start + x..row_start + run_end].fill(rgba);
            }

            x = run_end;
        }
    }

    Ok(pixel_data_out)
}

#[derive(Clone)]
pub struct GumpMap {
    client_path: PathBuf,
    idx_file: Option<IndexFile>,
    gump_mmap: Option<Arc<Mmap>>,
    uop_package: Option<UopPackage>,
    verdata: Option<Arc<Verdata>>,
}

fn first_existing(client_path: &Path, candidates: &[&str]) -> Option<PathBuf> {
    for &name in candidates {
        let path = client_path.join(name);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn uop_gump_candidates(gump_id: u32) -> [String; 2] {
    [
        format!("build/gumpartlegacymul/{:08}.tga", gump_id),
        format!("build/gumpartlegacymul/{:07}.tga", gump_id),
    ]
}

fn decode_uop_gump_header(raw_data: &[u8]) -> eyre::Result<(GumpDimensions, &[u8])> {
    if raw_data.len() < UOP_GUMP_HEADER_BYTES {
        eyre::bail!("UOP gump payload is shorter than its width/height header.");
    }

    let width = read_u32_le(raw_data, 0)?;
    let height = read_u32_le(raw_data, 4)?;
    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        eyre::bail!("Invalid UOP gump dimensions ({width}x{height}).");
    }

    Ok((
        GumpDimensions {
            width: width as u16,
            height: height as u16,
        },
        &raw_data[UOP_GUMP_HEADER_BYTES..],
    ))
}

impl GumpMap {
    pub fn load_standalone_uop(uop: UopPackage) -> Self {
        Self {
            client_path: PathBuf::new(),
            idx_file: None,
            gump_mmap: None,
            uop_package: Some(uop),
            verdata: None,
        }
    }

    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref().to_path_buf();
        let idx_path = first_existing(&client_path, &["gumpidx.mul", "Gumpidx.mul"]);
        let mul_path = first_existing(&client_path, &["gumpart.mul", "Gumpart.mul"]);
        let uop_path = first_existing(
            &client_path,
            &[
                "gumpartLegacyMUL.uop",
                "GumpartLegacyMUL.uop",
                "gumpartlegacymul.uop",
            ],
        );

        let mut idx_file = None;
        let mut gump_mmap = None;
        let mut uop_package = None;

        if let (Some(idx_path), Some(mul_path)) = (idx_path, mul_path) {
            idx_file = Some(IndexFile::load(idx_path)?);

            let mul_handle = File::open(&mul_path)
                .wrap_err_with(|| format!("Failed to open {}", mul_path.display()))?;
            let mmap = unsafe { Mmap::map(&mul_handle)? };
            gump_mmap = Some(Arc::new(mmap));
            log::info!("uocf: Loaded classic gump MUL format (memory-mapped)");
        }

        if let Some(path) = uop_path {
            uop_package = Some(UopPackage::load_with_mode(&path, LoadMode::Lazy)?);
            log::info!("uocf: Loaded gump UOP format from {}", path.display());
        }

        if idx_file.is_none() && uop_package.is_none() {
            eyre::bail!(
                "Neither gumpidx.mul/gumpart.mul nor gumpartLegacyMUL.uop found in the client path."
            );
        }

        Ok(Self {
            client_path,
            idx_file,
            gump_mmap,
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

    pub fn get_raw_gump_data(
        &self,
        gump_id: u32,
        scratch_buffer: &mut Vec<u8>,
    ) -> eyre::Result<GumpDimensions> {
        scratch_buffer.clear();

        if let Some(verdata) = &self.verdata {
            if let Some(entry) = verdata.entry(VerFileId::Gumpart, gump_id as i32) {
                let dimensions = dimensions_from_extra(entry.extra as u32)?;
                if classic_gump_payload_is_structurally_valid(entry.length as u32, dimensions) {
                    let bytes = verdata.read_patch_data(entry)?;
                    scratch_buffer.extend_from_slice(&bytes);
                    return Ok(dimensions);
                }
            }
        }

        if let (Some(idx), Some(gump_mmap)) = (&self.idx_file, &self.gump_mmap) {
            if let Ok(entry) = idx.element(gump_id as usize) {
                let index_patch = self
                    .verdata
                    .as_ref()
                    .and_then(|verdata| verdata.index_patch(VerFileId::GumpIdx, gump_id as i32));
                let index_values = index_patch.or_else(|| {
                    Some((entry.lookup()?, entry.len()?, entry.extra()?))
                });
                if let Some((lookup, size, extra)) = index_values {
                    let dimensions = dimensions_from_extra(extra)?;
                    if classic_gump_payload_is_structurally_valid(size, dimensions) {
                        let lookup = lookup as usize;
                        let size = size as usize;
                        let end = lookup + size;

                        if end > gump_mmap.len() {
                            eyre::bail!(
                                "Gump index points outside mmap range for gump_id {}",
                                gump_id
                            );
                        }

                        scratch_buffer.extend_from_slice(&gump_mmap[lookup..end]);
                        return Ok(dimensions);
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            for file_name in uop_gump_candidates(gump_id) {
                let hash = crate::uop_container::hash::hash_file_name_single(&file_name);
                if let Some(uop_payload) = uop.unpack_file_by_hash(hash)? {
                    let (dimensions, rle_payload) = decode_uop_gump_header(&uop_payload)?;
                    scratch_buffer.extend_from_slice(rle_payload);
                    return Ok(dimensions);
                }
            }
        }

        eyre::bail!("Could not find gump data for ID {}", gump_id);
    }

    pub fn decode_gump(
        &self,
        gump_id: u32,
        scratch_raw_buffer: &mut Vec<u8>,
    ) -> eyre::Result<(u16, u16, Vec<u8>)> {
        let dimensions = self.get_raw_gump_data(gump_id, scratch_raw_buffer)?;
        let pixel_data =
            decode_gump_from_raw(scratch_raw_buffer, dimensions.width, dimensions.height)?;
        Ok((dimensions.width, dimensions.height, pixel_data))
    }

    pub fn max_id(&self) -> u32 {
        if let Some(idx) = &self.idx_file {
            idx.element_count() as u32
        } else {
            0xFFFF
        }
    }

    pub fn has_id(&self, gump_id: u32) -> bool {
        if let Some(verdata) = &self.verdata {
            if let Some(entry) = verdata.entry(VerFileId::Gumpart, gump_id as i32) {
                if let Ok(dimensions) = dimensions_from_extra(entry.extra as u32) {
                    if classic_gump_payload_is_structurally_valid(entry.length as u32, dimensions) {
                        return true;
                    }
                }
            }
        }

        if let Some(idx) = &self.idx_file {
            if let Ok(entry) = idx.element(gump_id as usize) {
                let index_patch = self
                    .verdata
                    .as_ref()
                    .and_then(|verdata| verdata.index_patch(VerFileId::GumpIdx, gump_id as i32));
                let index_values = index_patch.or_else(|| {
                    Some((entry.lookup()?, entry.len()?, entry.extra()?))
                });
                if let Some((_lookup, size, extra)) = index_values {
                    if let Ok(dimensions) = dimensions_from_extra(extra) {
                        if classic_gump_payload_is_structurally_valid(size, dimensions) {
                            return true;
                        }
                    }
                }
            }
        }

        if let Some(uop) = &self.uop_package {
            for name in uop_gump_candidates(gump_id) {
                if uop
                    .get_file_by_hash(crate::uop_container::hash::hash_file_name_single(&name))
                    .is_some()
                {
                    return true;
                }
            }
        }

        false
    }
}
