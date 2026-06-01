//! # UO Classic AnimationFrame UOP Parser
//!
//! This module handles the parsing of `AnimationFrame*.uop` files for the Classic Client.
//! These files contain the RLE-encoded animations wrapped in a UOP container.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

use crate::classic::anim::{
    decode_classic_rle_frame, rgb555_palette_to_rgba_words, AnimFrame, AnimFrameInfo,
};

/// Represents the data inside a single CC AnimationFrame entry.
pub struct AnimationFrameCc {
    pub format_id: u32,
    pub version: u32,
    pub anim_id: u32,
    pub frame_count: u32,
    pub frames: Vec<AnimFrame>,
}

pub struct AnimationFrameCcMetadata {
    pub format_id: u32,
    pub version: u32,
    pub anim_id: u32,
    pub frame_count: u32,
    pub frames: Vec<AnimFrameInfo>,
}

impl AnimationFrameCc {
    /// Parses a single AnimationFrame CC entry from a byte slice (already decompressed from UOP).
    pub fn parse(data: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(data);

        let format_id = reader.read_u32::<LittleEndian>()?;
        let version = reader.read_u32::<LittleEndian>()?;
        let _decompressed_size = reader.read_u32::<LittleEndian>()?;
        let anim_id = reader.read_u32::<LittleEndian>()?;
        let _unk1 = reader.read_u64::<LittleEndian>()?;
        let _unk2 = reader.read_u16::<LittleEndian>()?;
        let _unk3 = reader.read_u16::<LittleEndian>()?;
        let _header_length = reader.read_u32::<LittleEndian>()?;
        let frame_count = reader.read_u32::<LittleEndian>()?;
        let first_frame_address = reader.read_u32::<LittleEndian>()? as usize;

        let mut frames = Vec::with_capacity(frame_count as usize);
        for frame_index in 0..frame_count as usize {
            let frame_start = frame_data_start(data, first_frame_address, frame_index)?;
            let mut frame_ptr = data
                .get(frame_start..)
                .ok_or_else(|| eyre!("Classic AnimationFrame pixel data offset out of bounds"))?;

            // Read Palette (256 colors, RGB555)
            let mut palette = [0u16; 256];
            frame_ptr.read_u16_into::<LittleEndian>(&mut palette)?;

            let rgba_palette = rgb555_palette_to_rgba_words(&palette);

            let center_x = frame_ptr.read_i16::<LittleEndian>()?;
            let center_y = frame_ptr.read_i16::<LittleEndian>()?;
            let width = frame_ptr.read_u16::<LittleEndian>()?;
            let height = frame_ptr.read_u16::<LittleEndian>()?;

            if width == 0 || height == 0 || width > 1024 || height > 1024 {
                frames.push(AnimFrame {
                    width: 0,
                    height: 0,
                    center_x,
                    center_y,
                    data: Vec::new(),
                });
                continue;
            }

            let mut pixel_data = vec![0u8; width as usize * height as usize * 4];

            decode_classic_rle_frame(
                &mut pixel_data,
                &mut frame_ptr,
                width,
                height,
                center_x,
                center_y,
                &rgba_palette,
            )?;

            frames.push(AnimFrame {
                width,
                height,
                center_x,
                center_y,
                data: pixel_data,
            });
        }

        Ok(Self {
            format_id,
            version,
            anim_id,
            frame_count,
            frames,
        })
    }

    pub fn parse_metadata(data: &[u8]) -> eyre::Result<AnimationFrameCcMetadata> {
        let mut reader = Cursor::new(data);

        let format_id = reader.read_u32::<LittleEndian>()?;
        let version = reader.read_u32::<LittleEndian>()?;
        let _decompressed_size = reader.read_u32::<LittleEndian>()?;
        let anim_id = reader.read_u32::<LittleEndian>()?;
        let _unk1 = reader.read_u64::<LittleEndian>()?;
        let _unk2 = reader.read_u16::<LittleEndian>()?;
        let _unk3 = reader.read_u16::<LittleEndian>()?;
        let _header_length = reader.read_u32::<LittleEndian>()?;
        let frame_count = reader.read_u32::<LittleEndian>()?;
        let first_frame_address = reader.read_u32::<LittleEndian>()? as usize;

        let mut frames = Vec::with_capacity(frame_count as usize);
        for frame_index in 0..frame_count as usize {
            let frame_start = frame_data_start(data, first_frame_address, frame_index)?;
            let metadata_start = frame_start
                .checked_add(512)
                .ok_or_else(|| eyre!("Classic AnimationFrame metadata offset overflows"))?;
            let mut frame_ptr = data
                .get(metadata_start..)
                .ok_or_else(|| eyre!("Classic AnimationFrame metadata offset out of bounds"))?;

            let center_x = frame_ptr.read_i16::<LittleEndian>()?;
            let center_y = frame_ptr.read_i16::<LittleEndian>()?;
            let width = frame_ptr.read_u16::<LittleEndian>()?;
            let height = frame_ptr.read_u16::<LittleEndian>()?;

            if width == 0 || height == 0 || width > 1024 || height > 1024 {
                frames.push(AnimFrameInfo {
                    width: 0,
                    height: 0,
                    center_x,
                    center_y,
                });
                continue;
            }

            frames.push(AnimFrameInfo {
                width,
                height,
                center_x,
                center_y,
            });
        }

        Ok(AnimationFrameCcMetadata {
            format_id,
            version,
            anim_id,
            frame_count,
            frames,
        })
    }
}

fn frame_data_start(data: &[u8], first_frame_address: usize, frame_index: usize) -> eyre::Result<usize> {
    let entry_start = first_frame_address
        .checked_add(
            frame_index
                .checked_mul(16)
                .ok_or_else(|| eyre!("Classic AnimationFrame entry offset overflows"))?,
        )
        .ok_or_else(|| eyre!("Classic AnimationFrame entry offset overflows"))?;
    let pixel_offset_pos = entry_start
        .checked_add(12)
        .ok_or_else(|| eyre!("Classic AnimationFrame pixel data offset overflows"))?;
    let pixel_data_offset = read_u32_at(data, pixel_offset_pos)? as usize;
    let frame_start = entry_start
        .checked_add(pixel_data_offset)
        .ok_or_else(|| eyre!("Classic AnimationFrame pixel data offset overflows"))?;
    if frame_start > data.len() {
        eyre::bail!("Classic AnimationFrame pixel data offset out of bounds");
    }
    Ok(frame_start)
}

fn read_u32_at(data: &[u8], offset: usize) -> eyre::Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| eyre!("Classic AnimationFrame u32 offset overflows"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| eyre!("Classic AnimationFrame entry table truncated"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_frame_entries_without_cursor_seeks() {
        let data = cc_payload(2, 1, 2);

        let parsed = AnimationFrameCc::parse(&data).expect("payload should parse");
        let metadata = AnimationFrameCc::parse_metadata(&data).expect("metadata should parse");

        assert_eq!(parsed.frame_count, 2);
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(metadata.frames.len(), 2);
        assert_eq!(metadata.frames[0].width, parsed.frames[0].width);
        assert_eq!(metadata.frames[0].height, parsed.frames[0].height);
        assert_eq!(
            parsed.frames[0].data,
            vec![248, 0, 0, 255, 0, 248, 0, 255],
        );
        assert_eq!(parsed.frames[1].data, parsed.frames[0].data);
    }

    fn cc_payload(width: u16, height: u16, frame_count: u32) -> Vec<u8> {
        let header_size = 40usize;
        let entry_size = 16usize;
        let mut data = Vec::new();

        push_u32(&mut data, 1);
        push_u32(&mut data, 1);
        push_u32(&mut data, 0);
        push_u32(&mut data, 7);
        push_u64(&mut data, 0);
        push_u16(&mut data, 0);
        push_u16(&mut data, 0);
        push_u32(&mut data, header_size as u32);
        push_u32(&mut data, frame_count);
        push_u32(&mut data, header_size as u32);

        for frame_index in 0..frame_count {
            push_u16(&mut data, 0);
            push_u16(&mut data, frame_index as u16);
            push_u64(&mut data, 0);
            push_u32(&mut data, 0);
        }

        for frame_index in 0..frame_count as usize {
            let entry_start = header_size + frame_index * entry_size;
            let frame_start = data.len();
            data[entry_start + 12..entry_start + 16]
                .copy_from_slice(&((frame_start - entry_start) as u32).to_le_bytes());
            append_frame(&mut data, width, height);
        }

        let decompressed_size = data.len() as u32;
        data[8..12].copy_from_slice(&decompressed_size.to_le_bytes());
        data
    }

    fn append_frame(data: &mut Vec<u8>, width: u16, height: u16) {
        for index in 0..256u16 {
            let color: u16 = match index {
                1 => 0x7C00,
                2 => 0x03E0,
                _ => 0,
            };
            data.extend_from_slice(&color.to_le_bytes());
        }
        data.extend_from_slice(&0i16.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());

        let y_offset = ((0i32 - i32::from(height)) & 0x3FF) as u32;
        let header = u32::from(width) | (y_offset << 12);
        data.extend_from_slice(&header.to_le_bytes());
        data.push(1);
        data.push(2);
        data.extend_from_slice(&0x7FFF7FFFu32.to_le_bytes());
    }

    fn push_u16(data: &mut Vec<u8>, value: u16) {
        data.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(data: &mut Vec<u8>, value: u32) {
        data.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(data: &mut Vec<u8>, value: u64) {
        data.extend_from_slice(&value.to_le_bytes());
    }
}
