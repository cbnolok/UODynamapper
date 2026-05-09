//! # UO Classic AnimationFrame UOP Parser
//!
//! This module handles the parsing of `AnimationFrame*.uop` files for the Classic Client.
//! These files contain the RLE-encoded animations wrapped in a UOP container.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Seek, SeekFrom};

use crate::classic::anim::AnimFrame;

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

            // Convert palette to RGBA8888
            let mut rgba_palette = [[0u8; 4]; 256];
            for i in 0..256 {
                let c = palette[i];
                if c != 0 {
                    let r = (((c >> 10) & 0x1F) << 3) as u8;
                    let g = (((c >> 5) & 0x1F) << 3) as u8;
                    let b = ((c & 0x1F) << 3) as u8;
                    rgba_palette[i] = [r, g, b, 255];
                }
            }

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

            // RLE Decoding (Same as MUL)
            loop {
                let header = match reader.read_u32::<LittleEndian>() {
                    Ok(h) => h,
                    Err(_) => break,
                };

                if header == 0x7FFF7FFF {
                    break;
                }

                let x_run = (header & 0xFFF) as usize;
                let mut x_offset = ((header >> 22) & 0x3FF) as i32;
                let mut y_offset = ((header >> 12) & 0x3FF) as i32;

                if (x_offset & 0x200) != 0 {
                    x_offset |= !0x3FF;
                }
                if (y_offset & 0x200) != 0 {
                    y_offset |= !0x3FF;
                }

                let x = (x_offset + center_x as i32) as i32;
                let y = (y_offset + center_y as i32 + height as i32) as i32;

                if y >= 0 && y < height as i32 {
                    for k in 0..x_run {
                        let final_x = x + k as i32;
                        if final_x >= 0 && final_x < width as i32 {
                            let palette_index = reader.read_u8()? as usize;
                            let color = rgba_palette[palette_index];
                            let pixel_idx = ((y * width as i32 + final_x) * 4) as usize;
                            pixel_data[pixel_idx..pixel_idx + 4].copy_from_slice(&color);
                        } else {
                            reader.read_u8()?;
                        }
                    }
                } else {
                    for _ in 0..x_run {
                        reader.read_u8()?;
                    }
                }
            }

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
