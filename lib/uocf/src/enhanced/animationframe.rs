//! # UO Enhanced Client Animation Frame Parser
//!
//! This module handles the parsing of the `animationframe.bin` files from the enhanced client.
//! These files contain the animation data for creatures and characters.
//!

// TODO: the format should be the same also for KR!

crate::eyre_imports!();
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

/// Represents a single decoded animation frame.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub data: Vec<u8>, // RGBA8888
}

/// Represents a single frame entry optimized for memory layout.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameEntry {
    pub frame: u16,
    pub unknown: u16,
    pub init_coords_x: i16,
    pub init_coords_y: i16,
    pub end_coords_x: i16,
    pub end_coords_y: i16,
    pub data_offset: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawFrameEntry {
    frame: u16,
    unknown: u16,
    init_coords_x: i16,
    init_coords_y: i16,
    end_coords_x: i16,
    end_coords_y: i16,
    data_offset: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AnimationFrameHeader {
    signature: [u8; 4],
    version: u32,
    total_size: u32,
    animation_id: u32,
    init_coords_x: i16,
    init_coords_y: i16,
    end_coords_x: i16,
    end_coords_y: i16,
    colours_count: u32,
    colours_offset: u32,
    frames_count: u32,
    frames_offset: u32,
}

/// Represents a single animation frame file.
#[derive(Debug, Clone)]
pub struct AnimationFrame {
    pub version: u32,
    pub total_size: u32,
    pub animation_id: u32,
    pub init_coords_x: i16,
    pub init_coords_y: i16,
    pub end_coords_x: i16,
    pub end_coords_y: i16,
    pub colours_count: u32,
    pub colours_offset: u32,
    pub frames_count: u32,
    pub frames_offset: u32,
    pub colours: Vec<[u8; 4]>,
    pub frames: Vec<FrameEntry>,
    /// The raw image data payload
    pub image_data: Arc<[u8]>,
}

impl AnimationFrame {
    /// Loads an animation frame from a zero-copy byte slice (e.g. from UOP).
    pub fn load(data: &[u8]) -> eyre::Result<Self> {
        if data.len() < std::mem::size_of::<AnimationFrameHeader>() {
            eyre::bail!("Data too small to contain AnimationFrame header");
        }

        let header: &AnimationFrameHeader =
            bytemuck::from_bytes(&data[0..std::mem::size_of::<AnimationFrameHeader>()]);

        if &header.signature[0..3] != b"AMO" {
            eyre::bail!("Invalid animation frame signature");
        }

        let colours_offset = header.colours_offset as usize;
        let colours_count = header.colours_count as usize;
        let expected_colours_end = colours_offset + colours_count * 4;
        if data.len() < expected_colours_end {
            eyre::bail!("Data too small for color palette");
        }

        let mut colours = Vec::with_capacity(colours_count);
        let palette_slice = &data[colours_offset..expected_colours_end];
        for chunk in palette_slice.chunks_exact(4) {
            colours.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        let frames_offset = header.frames_offset as usize;
        let frames_count = header.frames_count as usize;
        let expected_frames_end =
            frames_offset + frames_count * std::mem::size_of::<RawFrameEntry>();
        if data.len() < expected_frames_end {
            eyre::bail!("Data too small for frames entries");
        }

        let frames_slice = &data[frames_offset..expected_frames_end];
        let raw_frames: &[RawFrameEntry] = bytemuck::cast_slice(frames_slice);

        let mut frames = Vec::with_capacity(frames_count);
        for (i, raw) in raw_frames.iter().enumerate() {
            // Convert to native endianness and absolute offset
            frames.push(FrameEntry {
                frame: u16::from_le(raw.frame),
                unknown: u16::from_le(raw.unknown),
                init_coords_x: i16::from_le(raw.init_coords_x),
                init_coords_y: i16::from_le(raw.init_coords_y),
                end_coords_x: i16::from_le(raw.end_coords_x),
                end_coords_y: i16::from_le(raw.end_coords_y),
                data_offset: (header.frames_offset
                    + (i as u32) * 16
                    + u32::from_le(raw.data_offset)),
            });
        }

        let image_data_offset = expected_frames_end;
        let image_data: Arc<[u8]> = data[image_data_offset..].into();

        Ok(Self {
            version: u32::from_le(header.version),
            total_size: u32::from_le(header.total_size),
            animation_id: u32::from_le(header.animation_id),
            init_coords_x: i16::from_le(header.init_coords_x),
            init_coords_y: i16::from_le(header.init_coords_y),
            end_coords_x: i16::from_le(header.end_coords_x),
            end_coords_y: i16::from_le(header.end_coords_y),
            colours_count: u32::from_le(header.colours_count),
            colours_offset: u32::from_le(header.colours_offset),
            frames_count: u32::from_le(header.frames_count),
            frames_offset: u32::from_le(header.frames_offset),
            colours,
            frames,
            image_data,
        })
    }

    /// Decodes a single frame using RLE with alpha blending.
    pub fn decode_frame(&self, frame_entry: &FrameEntry) -> eyre::Result<DecodedFrame> {
        let width = (frame_entry.end_coords_x - frame_entry.init_coords_x).abs() as u16;
        let height = (frame_entry.end_coords_y - frame_entry.init_coords_y).abs() as u16;
        
        if width == 0 || height == 0 {
            return Ok(DecodedFrame {
                width: 0,
                height: 0,
                center_x: 0,
                center_y: 0,
                data: Vec::new(),
            });
        }

        let mut decoded_data = vec![0u8; width as usize * height as usize * 4];

        // Calculate offset relative to the image_data slice
        let start_pos = self.frames_offset + self.frames_count * 16;
        let relative_offset = frame_entry.data_offset.saturating_sub(start_pos) as usize;

        if relative_offset >= self.image_data.len() {
            eyre::bail!("Frame data offset out of bounds");
        }

        let mut data_cursor = &self.image_data[relative_offset..];
        let mut cur_x = 0i32;
        let mut cur_y = 0i32;

        let next_coord = |x: &mut i32, y: &mut i32, w: i32| {
            *x += 1;
            if *x >= w {
                *x = 0;
                *y += 1;
            }
        };

        let set_pixel = |data: &mut [u8], x: i32, y: i32, w: i32, h: i32, color: [u8; 4], factor: u8| {
            if x < 0 || x >= w || y < 0 || y >= h { return; }
            let idx = ((y * w + x) * 4) as usize;
            if factor == 16 {
                data[idx..idx+4].copy_from_slice(&color);
            } else if factor > 0 {
                // Alpha blend with existing pixel
                let f = factor as f32 / 16.0;
                let inv_f = 1.0 - f;
                
                let r = (color[0] as f32 * f + data[idx] as f32 * inv_f) as u8;
                let g = (color[1] as f32 * f + data[idx+1] as f32 * inv_f) as u8;
                let b = (color[2] as f32 * f + data[idx+2] as f32 * inv_f) as u8;
                let a = (color[3] as f32 * f + data[idx+3] as f32 * inv_f) as u8;
                
                data[idx] = r;
                data[idx+1] = g;
                data[idx+2] = b;
                data[idx+3] = a;
            }
        };

        while (cur_y as u16) < height && !data_cursor.is_empty() {
            let curr = data_cursor[0];
            data_cursor = &data_cursor[1..];

            if curr < 128 {
                // Skip pixels
                for _ in 0..curr {
                    next_coord(&mut cur_x, &mut cur_y, width as i32);
                }
            } else {
                // Run of pixels
                if data_cursor.is_empty() { break; }
                let next = data_cursor[0];
                data_cursor = &data_cursor[1..];

                let factor1 = next / 16;
                let factor2 = next % 16;

                // Handle first pixel with factor1
                if factor1 > 0 {
                    if data_cursor.is_empty() { break; }
                    let color_idx = data_cursor[0] as usize;
                    data_cursor = &data_cursor[1..];
                    if let Some(&color) = self.colours.get(color_idx) {
                        set_pixel(&mut decoded_data, cur_x, cur_y, width as i32, height as i32, color, factor1);
                    }
                    next_coord(&mut cur_x, &mut cur_y, width as i32);
                }

                // Solid run
                let count = curr - 128;
                for _ in 0..count {
                    if data_cursor.is_empty() { break; }
                    let color_idx = data_cursor[0] as usize;
                    data_cursor = &data_cursor[1..];
                    if let Some(&color) = self.colours.get(color_idx) {
                        set_pixel(&mut decoded_data, cur_x, cur_y, width as i32, height as i32, color, 16);
                    }
                    next_coord(&mut cur_x, &mut cur_y, width as i32);
                }

                // Handle last pixel with factor2
                if factor2 > 0 {
                    if data_cursor.is_empty() { break; }
                    let color_idx = data_cursor[0] as usize;
                    data_cursor = &data_cursor[1..];
                    if let Some(&color) = self.colours.get(color_idx) {
                        set_pixel(&mut decoded_data, cur_x, cur_y, width as i32, height as i32, color, factor2);
                    }
                    next_coord(&mut cur_x, &mut cur_y, width as i32);
                }
            }
        }

        Ok(DecodedFrame {
            width,
            height,
            center_x: self.init_coords_x - frame_entry.init_coords_x,
            center_y: self.init_coords_y - frame_entry.init_coords_y,
            data: decoded_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    fn single_frame_payload(
        header_init_x: i16,
        header_init_y: i16,
        frame_init_x: i16,
        frame_init_y: i16,
        frame_end_x: i16,
        frame_end_y: i16,
        frame_bytes: &[u8],
    ) -> Vec<u8> {
        let colours = [
            [255u8, 0, 0, 255],
            [0u8, 255, 0, 255],
        ];
        let header_size = 40u32;
        let colours_offset = header_size;
        let frames_offset = colours_offset + (colours.len() as u32 * 4);
        let image_offset = frames_offset + 16;
        let total_size = image_offset + frame_bytes.len() as u32;

        let mut bytes = Vec::with_capacity(total_size as usize);
        bytes.extend_from_slice(b"AMO\x04");
        bytes.write_u32::<LittleEndian>(4).unwrap();
        bytes.write_u32::<LittleEndian>(total_size).unwrap();
        bytes.write_u32::<LittleEndian>(42).unwrap();
        bytes.write_i16::<LittleEndian>(header_init_x).unwrap();
        bytes.write_i16::<LittleEndian>(header_init_y).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_y).unwrap();
        bytes.write_u32::<LittleEndian>(colours.len() as u32).unwrap();
        bytes.write_u32::<LittleEndian>(colours_offset).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(frames_offset).unwrap();

        for colour in colours {
            bytes.extend_from_slice(&colour);
        }

        bytes.write_u16::<LittleEndian>(9).unwrap();
        bytes.write_u16::<LittleEndian>(11).unwrap();
        bytes.write_i16::<LittleEndian>(frame_init_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_init_y).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_y).unwrap();
        bytes.write_u32::<LittleEndian>(image_offset - frames_offset).unwrap();
        bytes.extend_from_slice(frame_bytes);

        bytes
    }

    #[test]
    fn load_resolves_palette_frame_table_and_relative_image_offset() {
        let payload = single_frame_payload(5, 6, 3, 4, 5, 5, &[130, 0, 0, 1]);

        let animation = AnimationFrame::load(&payload).unwrap();

        assert_eq!(animation.version, 4);
        assert_eq!(animation.total_size, payload.len() as u32);
        assert_eq!(animation.animation_id, 42);
        assert_eq!(animation.colours, vec![[255, 0, 0, 255], [0, 255, 0, 255]]);
        assert_eq!(animation.frames.len(), 1);
        assert_eq!(animation.frames[0].frame, 9);
        assert_eq!(animation.frames[0].unknown, 11);
        assert_eq!(animation.frames[0].data_offset, 64);
    }

    #[test]
    fn decode_frame_expands_solid_rle_pixels() {
        let payload = single_frame_payload(5, 6, 3, 4, 5, 5, &[130, 0, 0, 1]);
        let animation = AnimationFrame::load(&payload).unwrap();

        let frame = animation.decode_frame(&animation.frames[0]).unwrap();

        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.center_x, 2);
        assert_eq!(frame.center_y, 2);
        assert_eq!(frame.data, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn decode_frame_handles_skips_and_partial_blend_pixels() {
        let payload = single_frame_payload(0, 0, 0, 0, 3, 1, &[1, 129, 0x80, 1, 0]);
        let animation = AnimationFrame::load(&payload).unwrap();

        let frame = animation.decode_frame(&animation.frames[0]).unwrap();

        assert_eq!(frame.width, 3);
        assert_eq!(frame.height, 1);
        assert_eq!(
            frame.data,
            vec![
                0, 0, 0, 0,
                0, 127, 0, 127,
                255, 0, 0, 255,
            ]
        );
    }
}
