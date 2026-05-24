//! # UO Classic AnimationFrame UOP Parser
//!
//! This module handles the parsing of `AnimationFrame*.uop` files for the Classic Client.
//! These files contain the RLE-encoded animations wrapped in a UOP container.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Seek, SeekFrom};

use crate::classic::anim::{decode_classic_rle_frame, rgb555_palette_to_rgba, AnimFrame};

/// Represents the data inside a single CC AnimationFrame entry.
pub struct AnimationFrameCc {
    pub format_id: u32,
    pub version: u32,
    pub anim_id: u32,
    pub frame_count: u32,
    pub frames: Vec<AnimFrame>,
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
        let first_frame_address = reader.read_u32::<LittleEndian>()?;

        reader.seek(SeekFrom::Start(first_frame_address as u64))?;

        let mut frame_entries = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            let data_start = reader.stream_position()?;
            let _anim_group = reader.read_u16::<LittleEndian>()?;
            let frame_id = reader.read_u16::<LittleEndian>()?;
            let _unk = reader.read_u64::<LittleEndian>()?;
            let pixel_data_offset = reader.read_u32::<LittleEndian>()?;

            frame_entries.push((data_start, frame_id, pixel_data_offset));
        }

        let mut frames = Vec::with_capacity(frame_count as usize);
        for (data_start, _frame_id, pixel_data_offset) in frame_entries {
            reader.seek(SeekFrom::Start(data_start + pixel_data_offset as u64))?;

            // Read Palette (256 colors, RGB555)
            let mut palette = [0u16; 256];
            reader.read_u16_into::<LittleEndian>(&mut palette)?;

            let rgba_palette = rgb555_palette_to_rgba(&palette);

            let center_x = reader.read_i16::<LittleEndian>()?;
            let center_y = reader.read_i16::<LittleEndian>()?;
            let width = reader.read_u16::<LittleEndian>()?;
            let height = reader.read_u16::<LittleEndian>()?;

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

            let rle_start = reader.position() as usize;
            if rle_start > data.len() {
                eyre::bail!("Classic AnimationFrame RLE offset out of bounds");
            }
            let mut frame_ptr = &data[rle_start..];
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
}
