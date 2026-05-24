//! # UO Enhanced Client Animation Frame Parser
//!
//! This module handles the parsing of the `animationframe.bin` files from the enhanced client.
//! These files contain the animation data for creatures and characters.
//!

// TODO: the format should be the same also for KR!

crate::eyre_imports!();
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use wide::u8x16;

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
        let width_usize = width as usize;
        let height_usize = height as usize;
        let mut pixel_index = 0usize;
        let pixel_count = width_usize * height_usize;

        while pixel_index < pixel_count && !data_cursor.is_empty() {
            let curr = data_cursor[0];
            data_cursor = &data_cursor[1..];

            if curr < 128 {
                pixel_index = pixel_index.saturating_add(curr as usize);
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
                        blend_pixel(&mut decoded_data, pixel_index, color, factor1);
                    }
                    pixel_index += 1;
                }

                // Solid run
                let count = curr - 128;
                let consumed = copy_solid_pixels(
                    &mut decoded_data,
                    pixel_index,
                    pixel_count,
                    &mut data_cursor,
                    count as usize,
                    &self.colours,
                );
                pixel_index += consumed;

                // Handle last pixel with factor2
                if factor2 > 0 {
                    if data_cursor.is_empty() { break; }
                    let color_idx = data_cursor[0] as usize;
                    data_cursor = &data_cursor[1..];
                    if let Some(&color) = self.colours.get(color_idx) {
                        blend_pixel(&mut decoded_data, pixel_index, color, factor2);
                    }
                    pixel_index += 1;
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

fn blend_pixel(data: &mut [u8], pixel_index: usize, color: [u8; 4], factor: u8) {
    if factor == 0 {
        return;
    }
    let idx = pixel_index * 4;
    if idx + 4 > data.len() {
        return;
    }
    if factor == 16 {
        data[idx..idx + 4].copy_from_slice(&color);
        return;
    }

    let factor = factor as u16;
    let inv_factor = 16 - factor;
    data[idx] = blend_channel(color[0], data[idx], factor, inv_factor);
    data[idx + 1] = blend_channel(color[1], data[idx + 1], factor, inv_factor);
    data[idx + 2] = blend_channel(color[2], data[idx + 2], factor, inv_factor);
    data[idx + 3] = blend_channel(color[3], data[idx + 3], factor, inv_factor);
}

fn blend_channel(src: u8, dst: u8, factor: u16, inv_factor: u16) -> u8 {
    ((src as u16 * factor + dst as u16 * inv_factor) >> 4) as u8
}

fn copy_solid_pixels(
    data: &mut [u8],
    pixel_index: usize,
    pixel_count: usize,
    cursor: &mut &[u8],
    count: usize,
    colours: &[[u8; 4]],
) -> usize {
    let available_count = count.min(cursor.len());
    let writable_count = available_count.min(pixel_count.saturating_sub(pixel_index));
    let (indices, rest) = cursor.split_at(available_count);
    *cursor = rest;

    let mut copied = 0usize;
    let mut dst = &mut data[pixel_index * 4..(pixel_index + writable_count) * 4];
    let mut remaining_indices = &indices[..writable_count];

    while remaining_indices.len() >= 4 && dst.len() >= 16 {
        let colors = [
            colours.get(remaining_indices[0] as usize).copied(),
            colours.get(remaining_indices[1] as usize).copied(),
            colours.get(remaining_indices[2] as usize).copied(),
            colours.get(remaining_indices[3] as usize).copied(),
        ];
        if let [Some(c0), Some(c1), Some(c2), Some(c3)] = colors {
            let packed = u8x16::from([
                c0[0], c0[1], c0[2], c0[3],
                c1[0], c1[1], c1[2], c1[3],
                c2[0], c2[1], c2[2], c2[3],
                c3[0], c3[1], c3[2], c3[3],
            ]);
            dst[..16].copy_from_slice(&packed.to_array());
            dst = &mut dst[16..];
            remaining_indices = &remaining_indices[4..];
            copied += 4;
        } else {
            break;
        }
    }

    for &color_idx in remaining_indices {
        if let Some(&color) = colours.get(color_idx as usize) {
            dst[..4].copy_from_slice(&color);
        }
        dst = &mut dst[4..];
        copied += 1;
    }

    copied + (available_count - writable_count)
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
