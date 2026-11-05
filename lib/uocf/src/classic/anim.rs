//! # UO Classic Animation Parser
//!
//! This module handles the parsing of `anim*.mul` and `anim*.idx` files.
//! These files contain the RLE-encoded animations for mobiles and effects.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use bytemuck::cast_slice_mut;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use wide::u16x8;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};
use crate::utils::path::find_file_ignoring_case;

pub const MAX_ANIM_FILES: u8 = 6; // anim, anim2, anim3, anim4, anim5
const MAX_CLASSIC_ANIM_FRAME_DIMENSION: u16 = 2048;
/// A frame offset of 0 is no frame: the client skips it (`UOClassicAnim::GetMulFrame`); 365 frames of the 7.0.103
/// `anim.mul` have it.
const MISSING_FRAME_OFFSET: usize = 0;
const MAX_CLASSIC_ANIM_FRAME_PIXELS: usize =
    MAX_CLASSIC_ANIM_FRAME_DIMENSION as usize * MAX_CLASSIC_ANIM_FRAME_DIMENSION as usize;
/// Ends a frame's runs.
const RUN_END_MARKER: u32 = 0x7FFF_7FFF;
const RUN_LENGTH_MASK: u32 = 0xFFF;
const RUN_HEADER_BYTES: usize = size_of::<u32>();
/// A run header's x offset is in bits 22..32 and its y offset in bits 12..22, both signed.
const RUN_X_OFFSET_SHIFT: u32 = 22;
const RUN_Y_OFFSET_LEFT_SHIFT: u32 = 10;

/// Represents a single decoded animation frame from a MUL file.
#[derive(Debug, Clone)]
pub struct AnimFrame {
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub data: Vec<u8>, // RGBA8888
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimFrameInfo {
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicAnimationIdentity {
    pub body_id: u16,
    pub action_id: u16,
    pub direction: u8,
    pub flags: u16,
}

pub const CLASSIC_ANIMATION_IDENTITY_UNMAPPED_SOURCE_INDEX: u16 = 0x0001;

/// Manages multiple animation MUL sources.
pub struct AnimMap {
    sources: Vec<Option<AnimSource>>,
    verdata: Option<Arc<Verdata>>,
}

struct AnimSource {
    idx: IndexFile,
    mul: memmap2::Mmap,
}

impl AnimMap {
    pub fn load(client_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let client_path = client_path.as_ref();
        let mut sources = Vec::with_capacity(MAX_ANIM_FILES as usize);

        for i in 0..MAX_ANIM_FILES {
            let suffix = if i == 0 {
                "".to_string()
            } else {
                (i + 1).to_string()
            };
            let idx_name = format!("anim{}.idx", suffix);
            let mul_name = format!("anim{}.mul", suffix);

            let idx_path = find_file_ignoring_case(client_path, &idx_name);
            let mul_path = find_file_ignoring_case(client_path, &mul_name);

            if let (Some(idx_path), Some(mul_path)) = (idx_path, mul_path) {
                let idx = IndexFile::load(idx_path)?;
                let mul_file = File::open(mul_path)?;
                let mul = unsafe { memmap2::Mmap::map(&mul_file)? };
                sources.push(Some(AnimSource { idx, mul }));
                log::debug!("uocf: Loaded animation source {}", idx_name);
            } else {
                sources.push(None);
            }
        }

        Ok(Self {
            sources,
            verdata: None,
        })
    }

    pub fn with_verdata(mut self, verdata: Arc<Verdata>) -> Self {
        self.verdata = Some(verdata);
        self
    }

    pub fn has_anim(&self, file_idx: u8, anim_id: u32) -> bool {
        if file_idx == 0 {
            if let Some(verdata) = &self.verdata {
                if verdata.entry(VerFileId::Anim, anim_id as i32).is_some()
                    || verdata.index_patch(VerFileId::AnimIdx, anim_id as i32).is_some()
                {
                    return true;
                }
            }
        }

        if let Some(Some(source)) = self.sources.get(file_idx as usize) {
            if let Ok(entry) = source.idx.element(anim_id as usize) {
                // Empty slots are `lookup 0, size 0` as often as `lookup -1` (13,795 in UO Gold's Anim2.idx).
                return entry.lookup().is_some() && entry.len().is_some_and(|len| len > 0);
            }
        }
        false
    }

    pub fn source_index_count(&self, file_idx: u8) -> Option<usize> {
        self.sources
            .get(file_idx as usize)
            .and_then(|source| source.as_ref())
            .map(|source| source.idx.element_count())
    }

    pub fn decode_animation_index(&self, file_idx: u8, index: u32) -> eyre::Result<Vec<AnimFrame>> {
        self.decode_animation(file_idx, index)
    }

    pub fn decode_animation_index_metadata(&self, file_idx: u8, index: u32) -> eyre::Result<Vec<AnimFrameInfo>> {
        self.decode_animation_metadata(file_idx, index)
    }

    /// Decodes an animation from a specific MUL file.
    /// Returns a list of frames.
    pub fn decode_animation(&self, file_idx: u8, anim_id: u32) -> eyre::Result<Vec<AnimFrame>> {
        self.read_animation_payload(file_idx, anim_id, |data| decode_animation_payload(data, 0))
    }

    pub fn decode_animation_metadata(&self, file_idx: u8, anim_id: u32) -> eyre::Result<Vec<AnimFrameInfo>> {
        self.read_animation_payload(file_idx, anim_id, |data| decode_animation_payload_metadata(data, 0))
    }

    fn read_animation_payload<T>(
        &self,
        file_idx: u8,
        anim_id: u32,
        decode: impl FnOnce(&[u8]) -> eyre::Result<T>,
    ) -> eyre::Result<T> {
        let source = self
            .sources
            .get(file_idx as usize)
            .and_then(|s| s.as_ref())
            .ok_or_else(|| eyre!("Animation source {} not loaded", file_idx))?;

        if file_idx == 0 {
            if let Some(verdata) = &self.verdata {
                if let Some(bytes) = verdata.read_patch(VerFileId::Anim, anim_id as i32)? {
                    return decode(&bytes);
                }
            }
        }

        let entry = source.idx.element(anim_id as usize)?;
        let index_patch = if file_idx == 0 {
            self.verdata
                .as_ref()
                .and_then(|verdata| verdata.index_patch(VerFileId::AnimIdx, anim_id as i32))
        } else {
            None
        };
        let (lookup, size) = index_patch
            .map(|(lookup, size, _extra)| (lookup, size))
            .or_else(|| entry.lookup().zip(entry.len()))
            .ok_or_else(|| eyre!("Animation {} not found in source {}", anim_id, file_idx))?;
        let lookup = lookup as usize;
        let size = size as usize;
        let end = lookup
            .checked_add(size)
            .ok_or_else(|| eyre!("Animation {} range overflows source {}", anim_id, file_idx))?;

        if size == 0 || lookup + 2 >= source.mul.len() || end > source.mul.len() {
            eyre::bail!(
                "Animation {} range {}..{} out of bounds for source {}",
                anim_id,
                lookup,
                end,
                file_idx
            );
        }

        decode(&source.mul[lookup..end])
    }
}

pub fn classic_animation_identity_from_source_index(
    file_index: u8,
    source_index: u32,
) -> ClassicAnimationIdentity {
    if let Some((body_id, action_id, direction)) =
        classic_animation_layout_from_source_index(file_index, source_index)
    {
        return ClassicAnimationIdentity {
            body_id,
            action_id,
            direction,
            flags: 0,
        };
    }

    ClassicAnimationIdentity {
        body_id: (source_index & 0xFFFF) as u16,
        action_id: (source_index >> 16) as u16,
        direction: file_index,
        flags: CLASSIC_ANIMATION_IDENTITY_UNMAPPED_SOURCE_INDEX,
    }
}

pub fn classic_animation_source_index_from_identity(
    file_index: u8,
    body_id: u16,
    action_id: u16,
    direction: u8,
) -> Option<u32> {
    if direction >= 5 {
        return None;
    }

    for &(body_start, body_end, source_start, action_count) in
        classic_animation_layout_groups(file_index)
    {
        if body_id < body_start || body_id >= body_end || action_id >= action_count {
            continue;
        }
        let body_offset = u32::from(body_id - body_start);
        let stride = u32::from(action_count) * 5;
        return Some(
            source_start
                + body_offset * stride
                + u32::from(action_id) * 5
                + u32::from(direction),
        );
    }

    None
}

fn classic_animation_layout_from_source_index(
    file_index: u8,
    source_index: u32,
) -> Option<(u16, u16, u8)> {
    for &(body_start, body_end, source_start, action_count) in
        classic_animation_layout_groups(file_index)
    {
        if let Some(layout) = classic_animation_layout_from_group(
            source_index,
            body_start,
            body_end,
            source_start,
            action_count,
        ) {
            return Some(layout);
        }
    }
    None
}

fn classic_animation_layout_groups(file_index: u8) -> &'static [(u16, u16, u32, u16)] {
    match file_index {
        0 => &[
            (0, 200, 0, 22),
            (200, 400, 22000, 13),
            (400, u16::MAX, 35000, 35),
        ],
        1 => &[
            (0, 200, 0, 22),
            (200, u16::MAX, 22000, 13),
        ],
        2 => &[
            (0, 300, 0, 13),
            (300, 400, 33000, 22),
            (400, u16::MAX, 35000, 35),
        ],
        _ => &[
            (0, 200, 0, 22),
            (200, 400, 22000, 13),
            (400, u16::MAX, 35000, 35),
        ],
    }
}

fn classic_animation_layout_from_group(
    source_index: u32,
    body_start: u16,
    body_end: u16,
    source_start: u32,
    action_count: u16,
) -> Option<(u16, u16, u8)> {
    if source_index < source_start || body_end <= body_start {
        return None;
    }
    let stride = u32::from(action_count) * 5;
    let body_count = u32::from(body_end - body_start);
    let rel = source_index - source_start;
    if rel >= body_count * stride {
        return None;
    }
    let body = u32::from(body_start) + rel / stride;
    let within_body = rel % stride;
    let action = within_body / 5;
    let direction = within_body % 5;
    Some((body as u16, action as u16, direction as u8))
}

fn decode_animation_payload(data: &[u8], lookup: usize) -> eyre::Result<Vec<AnimFrame>> {
        let mut mul_ptr = &data[lookup..];

        // Read Palette (256 colors, RGB555)
        let mut palette = [0u16; 256];
        mul_ptr.read_u16_into::<LittleEndian>(&mut palette)?;

        let rgba_palette = rgb555_palette_to_rgba_words(&palette);

        let frame_offset_base = lookup
            .checked_add(512)
            .ok_or_else(|| eyre!("Animation frame offset base overflows"))?;

        let frame_count = mul_ptr.read_u32::<LittleEndian>()?;
        if frame_count > 1000 {
            eyre::bail!("Suspiciously high frame count: {}", frame_count);
        }
        let frame_count = frame_count as usize;

        let offset_table_len = frame_count
            .checked_mul(4)
            .ok_or_else(|| eyre!("Animation frame offset table length overflows"))?;
        if mul_ptr.len() < offset_table_len {
            eyre::bail!("Animation frame offset table truncated");
        }
        let frame_offsets = &mul_ptr[..offset_table_len];

        let mut frames = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let offset = read_frame_offset(frame_offsets, i)? as usize;
            if offset == MISSING_FRAME_OFFSET {
                frames.push(AnimFrame { width: 0, height: 0, center_x: 0, center_y: 0, data: Vec::new() });
                continue;
            }
            let frame_start = frame_offset_base
                .checked_add(offset)
                .ok_or_else(|| eyre!("Frame offset {} overflows", offset))?;
            if frame_start >= data.len() {
                eyre::bail!("Frame offset {} out of bounds", frame_start);
            }

            let mut frame_ptr = &data[frame_start..];

            let center_x = frame_ptr.read_i16::<LittleEndian>()?;
            let center_y = frame_ptr.read_i16::<LittleEndian>()?;
            let width = frame_ptr.read_u16::<LittleEndian>()?;
            let height = frame_ptr.read_u16::<LittleEndian>()?;

            if width == 0 || height == 0 {
                frames.push(AnimFrame {
                    width: 0,
                    height: 0,
                    center_x,
                    center_y,
                    data: Vec::new(),
                });
                continue;
            }
            if width > MAX_CLASSIC_ANIM_FRAME_DIMENSION
                || height > MAX_CLASSIC_ANIM_FRAME_DIMENSION
                || width as usize * height as usize > MAX_CLASSIC_ANIM_FRAME_PIXELS
            {
                eyre::bail!("Suspicious animation frame dimensions: {}x{}", width, height);
            }

            let rect = frame_rect_covering_runs(frame_ptr, width, height, center_x, center_y)?;
            let pixel_len = frame_pixel_len(rect.width, rect.height)?;
            let mut pixel_data = vec![0u8; pixel_len];

            decode_classic_rle_frame(
                &mut pixel_data,
                &mut frame_ptr,
                rect.width,
                rect.height,
                rect.center_x,
                rect.center_y,
                &rgba_palette,
            )?;

            frames.push(AnimFrame {
                width: rect.width,
                height: rect.height,
                center_x: rect.center_x,
                center_y: rect.center_y,
                data: pixel_data,
            });
        }

        Ok(frames)
}

fn decode_animation_payload_metadata(data: &[u8], lookup: usize) -> eyre::Result<Vec<AnimFrameInfo>> {
        let frame_offset_base = lookup
            .checked_add(512)
            .ok_or_else(|| eyre!("Animation frame offset base overflows"))?;
        let mut mul_ptr = data
            .get(frame_offset_base..)
            .ok_or_else(|| eyre!("Animation palette truncated"))?;

        let frame_count = mul_ptr.read_u32::<LittleEndian>()?;
        if frame_count > 1000 {
            eyre::bail!("Suspiciously high frame count: {}", frame_count);
        }
        let frame_count = frame_count as usize;

        let offset_table_len = frame_count
            .checked_mul(4)
            .ok_or_else(|| eyre!("Animation frame offset table length overflows"))?;
        if mul_ptr.len() < offset_table_len {
            eyre::bail!("Animation frame offset table truncated");
        }
        let frame_offsets = &mul_ptr[..offset_table_len];

        let mut frames = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let offset = read_frame_offset(frame_offsets, i)? as usize;
            if offset == MISSING_FRAME_OFFSET {
                frames.push(AnimFrameInfo { width: 0, height: 0, center_x: 0, center_y: 0 });
                continue;
            }
            let frame_start = frame_offset_base
                .checked_add(offset)
                .ok_or_else(|| eyre!("Frame offset {} overflows", offset))?;
            if frame_start >= data.len() {
                eyre::bail!("Frame offset {} out of bounds", frame_start);
            }

            let mut frame_ptr = &data[frame_start..];
            let center_x = frame_ptr.read_i16::<LittleEndian>()?;
            let center_y = frame_ptr.read_i16::<LittleEndian>()?;
            let width = frame_ptr.read_u16::<LittleEndian>()?;
            let height = frame_ptr.read_u16::<LittleEndian>()?;

            if width > MAX_CLASSIC_ANIM_FRAME_DIMENSION
                || height > MAX_CLASSIC_ANIM_FRAME_DIMENSION
                || width as usize * height as usize > MAX_CLASSIC_ANIM_FRAME_PIXELS
            {
                eyre::bail!("Suspicious animation frame dimensions: {}x{}", width, height);
            }
            if width == 0 || height == 0 {
                frames.push(AnimFrameInfo { width, height, center_x, center_y });
                continue;
            }

            // Walking the runs here also means a frame that passes metadata can't fail at the pixel stage.
            let rect = frame_rect_covering_runs(frame_ptr, width, height, center_x, center_y)?;
            frames.push(AnimFrameInfo {
                width: rect.width,
                height: rect.height,
                center_x: rect.center_x,
                center_y: rect.center_y,
            });
        }

        Ok(frames)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClassicFrameRect {
    width: u16,
    height: u16,
    center_x: i16,
    center_y: i16,
}

/// A run's first pixel in the image of a frame `height` rows tall.
fn run_image_position(header: u32, center_x: i16, center_y: i16, height: i32) -> (i32, i32) {
    let x_offset = (header as i32) >> RUN_X_OFFSET_SHIFT;
    let y_offset = ((header << RUN_Y_OFFSET_LEFT_SHIFT) as i32) >> RUN_X_OFFSET_SHIFT;
    (x_offset + i32::from(center_x), y_offset + i32::from(center_y) + height)
}

/// The declared rectangle grown to hold every run. The client places runs from the anchor and clips them only to the
/// screen, never reading the frame's width or height (`UOClassicAnim::DrawFrameRle`), so a run a custom tool wrote
/// outside the declared size is still drawn. Growing keeps each pixel at its offset from the anchor.
fn frame_rect_covering_runs(
    mut runs: &[u8],
    width: u16,
    height: u16,
    center_x: i16,
    center_y: i16,
) -> eyre::Result<ClassicFrameRect> {
    let height_i32 = i32::from(height);
    let (mut left, mut right, mut top, mut bottom) = (0, i32::from(width), 0, height_i32);
    while let Some((header_bytes, rest)) = runs.split_first_chunk::<RUN_HEADER_BYTES>() {
        let header = u32::from_le_bytes(*header_bytes);
        if header == RUN_END_MARKER {
            break;
        }
        let run_len = (header & RUN_LENGTH_MASK) as usize;
        runs = rest.get(run_len..).ok_or_else(|| eyre!("Animation RLE run exceeds remaining frame data"))?;
        if run_len == 0 {
            continue;
        }
        let (x, y) = run_image_position(header, center_x, center_y, height_i32);
        left = left.min(x);
        right = right.max(x + run_len as i32);
        top = top.min(y);
        bottom = bottom.max(y + 1);
    }

    let (grown_width, grown_height) = (right - left, bottom - top);
    let max_dimension = i32::from(MAX_CLASSIC_ANIM_FRAME_DIMENSION);
    if grown_width > max_dimension || grown_height > max_dimension {
        eyre::bail!(
            "Animation frame runs reach {grown_width}x{grown_height}, past the {max_dimension} limit (declared {width}x{height})"
        );
    }
    // Rows added below the declared rectangle lower the anchor's row, which counts from the bottom edge.
    let rows_below = bottom - height_i32;
    Ok(ClassicFrameRect {
        width: grown_width as u16,
        height: grown_height as u16,
        center_x: i16::try_from(i32::from(center_x) - left)
            .map_err(|_| eyre!("Animation frame anchor x overflows after growing to its runs"))?,
        center_y: i16::try_from(i32::from(center_y) - rows_below)
            .map_err(|_| eyre!("Animation frame anchor y overflows after growing to its runs"))?,
    })
}

fn read_frame_offset(frame_offsets: &[u8], index: usize) -> eyre::Result<u32> {
    let start = index
        .checked_mul(4)
        .ok_or_else(|| eyre!("Animation frame offset index overflows"))?;
    let end = start
        .checked_add(4)
        .ok_or_else(|| eyre!("Animation frame offset index overflows"))?;
    let bytes = frame_offsets
        .get(start..end)
        .ok_or_else(|| eyre!("Animation frame offset table truncated"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// The palette as opaque RGBA words, bit 15 ignored. A word of 0 is opaque black: the client draws
/// `palette[index]` for every pixel of a run, whatever the word (`UOClassicAnim::DrawFrameRle`,
/// `scripts/dev-helpers/ec-research/ghidra-classic-frames/`); only pixels outside the runs are transparent.
pub(crate) fn rgb555_palette_to_rgba_words(palette: &[u16; 256]) -> [u32; 256] {
    let mut rgba_palette = [0u32; 256];
    let mask = u16x8::splat(0x1F);
    for (chunk_index, chunk) in palette.chunks_exact(8).enumerate() {
        let colors = u16x8::from([
            chunk[0], chunk[1], chunk[2], chunk[3],
            chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        let r = (((colors >> 10_u32) & mask) << 3_u32).to_array();
        let g = (((colors >> 5_u32) & mask) << 3_u32).to_array();
        let b = ((colors & mask) << 3_u32).to_array();
        let base = chunk_index * 8;
        for lane in 0..8 {
            rgba_palette[base + lane] = u32::from_le_bytes([
                r[lane] as u8,
                g[lane] as u8,
                b[lane] as u8,
                u8::MAX,
            ]);
        }
    }
    rgba_palette
}

pub(crate) fn decode_classic_rle_frame(
    pixel_data: &mut [u8],
    data: &mut &[u8],
    width: u16,
    height: u16,
    center_x: i16,
    center_y: i16,
    rgba_palette: &[u32; 256],
) -> eyre::Result<()> {
    let pixel_words = cast_slice_mut::<u8, u32>(pixel_data);
    let width_usize = width as usize;
    let height_i32 = height as i32;
    let width_i32 = width as i32;

    loop {
        if data.len() < 4 {
            break;
        }
        let header = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        *data = &data[4..];

        if header == RUN_END_MARKER {
            break;
        }

        let x_run = (header & RUN_LENGTH_MASK) as usize;
        if data.len() < x_run {
            eyre::bail!("Animation RLE run exceeds remaining frame data");
        }
        let (run, rest) = data.split_at(x_run);
        *data = rest;

        let (x, y) = run_image_position(header, center_x, center_y, height_i32);

        if y < 0 || y >= height_i32 {
            continue;
        }

        let run_end = x + x_run as i32;
        if x >= 0 && run_end <= width_i32 {
            let dst_start = y as usize * width_usize + x as usize;
            write_palette_run_words(
                &mut pixel_words[dst_start..dst_start + x_run],
                run,
                rgba_palette,
            );
            continue;
        }

        let start_x = x.max(0);
        let end_x = run_end.min(width_i32);
        if start_x >= end_x {
            continue;
        }

        let src_start = (start_x - x) as usize;
        let src_end = (end_x - x) as usize;
        let dst_start = y as usize * width_usize + start_x as usize;
        write_palette_run_words(
            &mut pixel_words[dst_start..dst_start + (src_end - src_start)],
            &run[src_start..src_end],
            rgba_palette,
        );
    }
    Ok(())
}

fn write_palette_run_words(dst: &mut [u32], indices: &[u8], rgba_palette: &[u32; 256]) {
    debug_assert_eq!(dst.len(), indices.len());

    if indices.len() < 4 {
        for (pixel, &palette_index) in dst.iter_mut().zip(indices) {
            *pixel = rgba_palette[palette_index as usize];
        }
        return;
    }

    let mut dst_chunks = dst.chunks_exact_mut(4);
    let mut index_chunks = indices.chunks_exact(4);
    for (dst_chunk, index_chunk) in dst_chunks.by_ref().zip(index_chunks.by_ref()) {
        let packed = [
            rgba_palette[index_chunk[0] as usize],
            rgba_palette[index_chunk[1] as usize],
            rgba_palette[index_chunk[2] as usize],
            rgba_palette[index_chunk[3] as usize],
        ];
        dst_chunk.copy_from_slice(&packed);
    }

    for (pixel, &palette_index) in dst_chunks
        .into_remainder()
        .iter_mut()
        .zip(index_chunks.remainder())
    {
        *pixel = rgba_palette[palette_index as usize];
    }
}

fn frame_pixel_len(width: u16, height: u16) -> eyre::Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| eyre!("Animation frame dimensions {}x{} overflow RGBA size", width, height))
}

fn frame_pixel_offset(y: i32, x: i32, width: u16) -> eyre::Result<usize> {
    let y = usize::try_from(y).map_err(|_| eyre!("negative animation frame y offset {}", y))?;
    let x = usize::try_from(x).map_err(|_| eyre!("negative animation frame x offset {}", x))?;
    y.checked_mul(width as usize)
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
        .ok_or_else(|| eyre!("Animation frame pixel offset overflow at {},{}", x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_pixel_offset_handles_i32_overflow_boundary() {
        let offset = frame_pixel_offset(32768, 0, u16::MAX)
            .expect("large decoded coordinates should use usize arithmetic");

        assert_eq!(offset, 32768usize * u16::MAX as usize * 4);
    }

    #[test]
    fn decode_animation_payload_uses_palette_relative_frame_offsets() {
        let mut data = vec![0u8; 512 + 8 + 8 + 4 + 1 + 4];
        data[2..4].copy_from_slice(&0x7FFFu16.to_le_bytes());
        data[512..516].copy_from_slice(&1u32.to_le_bytes());
        data[516..520].copy_from_slice(&8u32.to_le_bytes());
        data[520..522].copy_from_slice(&0i16.to_le_bytes());
        data[522..524].copy_from_slice(&0i16.to_le_bytes());
        data[524..526].copy_from_slice(&1u16.to_le_bytes());
        data[526..528].copy_from_slice(&1u16.to_le_bytes());

        let header = 1u32 | (0x3FFu32 << 12);
        data[528..532].copy_from_slice(&header.to_le_bytes());
        data[532] = 1;
        data[533..537].copy_from_slice(&0x7FFF7FFFu32.to_le_bytes());

        let frames = decode_animation_payload(&data, 0).expect("payload should decode");
        let metadata = decode_animation_payload_metadata(&data, 0).expect("metadata should decode");

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].width, 1);
        assert_eq!(frames[0].height, 1);
        assert_eq!(frames[0].data, vec![248, 248, 248, 255]);
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].width, frames[0].width);
        assert_eq!(metadata[0].height, frames[0].height);
        assert_eq!(metadata[0].center_x, frames[0].center_x);
        assert_eq!(metadata[0].center_y, frames[0].center_y);
    }

    fn run_header(x_offset: i32, y_offset: i32, len: u32) -> u32 {
        ((x_offset as u32 & 0x3FF) << 22) | ((y_offset as u32 & 0x3FF) << 12) | len
    }

    /// An entry with one 2x2 frame anchored at (0, 0), holding the given runs, each of colour index 1.
    fn entry_with_runs(runs: &[(i32, i32, u32)]) -> Vec<u8> {
        let mut data = vec![0u8; 512];
        data[2..4].copy_from_slice(&0x7FFFu16.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        for &(x_offset, y_offset, len) in runs {
            data.extend_from_slice(&run_header(x_offset, y_offset, len).to_le_bytes());
            data.extend(std::iter::repeat_n(1u8, len as usize));
        }
        data.extend_from_slice(&RUN_END_MARKER.to_le_bytes());
        data
    }

    #[test]
    fn frame_grows_to_runs_outside_the_declared_size_and_keeps_pixels_at_their_anchor_offsets() {
        // Declared 2x2 anchored at (0, 0): image pixel (x, y) has offsets (x, y - 2). These reach one column left,
        // one row above, two columns right and two rows below.
        let data = entry_with_runs(&[(-1, -3, 1), (0, -2, 2), (3, 1, 1)]);

        let frames = decode_animation_payload(&data, 0).expect("payload should decode");
        let metadata = decode_animation_payload_metadata(&data, 0).expect("metadata should decode");

        let frame = &frames[0];
        assert_eq!((frame.width, frame.height), (5, 5));
        assert_eq!((frame.center_x, frame.center_y), (1, -2));
        assert_eq!(
            (metadata[0].width, metadata[0].height, metadata[0].center_x, metadata[0].center_y),
            (frame.width, frame.height, frame.center_x, frame.center_y),
        );
        let opaque: Vec<(i32, i32)> = (0..5)
            .flat_map(|y| (0..5).map(move |x| (x, y)))
            .filter(|&(x, y)| frame.data[((y * 5 + x) * 4 + 3) as usize] != 0)
            .collect();
        let to_offsets = |(x, y): (i32, i32)| (x - i32::from(frame.center_x), y - i32::from(frame.center_y) - 5);
        assert_eq!(
            opaque.into_iter().map(to_offsets).collect::<Vec<_>>(),
            vec![(-1, -3), (0, -2), (1, -2), (3, 1)],
        );
    }

    #[test]
    fn frame_inside_its_declared_size_is_unchanged() {
        let data = entry_with_runs(&[(0, -2, 2), (0, -1, 2)]);
        let metadata = decode_animation_payload_metadata(&data, 0).expect("metadata should decode");
        assert_eq!((metadata[0].width, metadata[0].height), (2, 2));
        assert_eq!((metadata[0].center_x, metadata[0].center_y), (0, 0));
    }

    #[test]
    fn frame_whose_run_passes_the_data_end_fails_at_metadata() {
        let mut data = entry_with_runs(&[(0, -2, 2)]);
        data.truncate(data.len() - RUN_HEADER_BYTES - 1);
        assert!(decode_animation_payload_metadata(&data, 0).is_err());
    }

    #[test]
    fn classic_animation_identity_maps_body_action_direction_to_source_index() {
        assert_eq!(classic_animation_source_index_from_identity(0, 0, 0, 0), Some(0));
        assert_eq!(classic_animation_source_index_from_identity(0, 0, 0, 4), Some(4));
        assert_eq!(classic_animation_source_index_from_identity(0, 0, 1, 0), Some(5));
        assert_eq!(classic_animation_source_index_from_identity(0, 200, 0, 0), Some(22000));
        assert_eq!(classic_animation_source_index_from_identity(0, 400, 0, 0), Some(35000));
        assert_eq!(classic_animation_source_index_from_identity(0, 0, 22, 0), None);
        assert_eq!(classic_animation_source_index_from_identity(0, 0, 0, 5), None);
    }

    #[test]
    fn classic_animation_identity_roundtrips_known_layout_indices() {
        for (file_index, source_index, body_id, action_id, direction) in [
            (0, 0, 0, 0, 0),
            (0, 22000, 200, 0, 0),
            (0, 35000, 400, 0, 0),
            (1, 22000, 200, 0, 0),
            (2, 33000, 300, 0, 0),
        ] {
            let identity = classic_animation_identity_from_source_index(file_index, source_index);
            assert_eq!(identity.body_id, body_id);
            assert_eq!(identity.action_id, action_id);
            assert_eq!(identity.direction, direction);
            assert_eq!(identity.flags, 0);
            assert_eq!(
                classic_animation_source_index_from_identity(
                    file_index,
                    body_id,
                    action_id,
                    direction,
                ),
                Some(source_index)
            );
        }
    }
}
