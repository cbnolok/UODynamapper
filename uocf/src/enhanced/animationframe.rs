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

    /// Decodes a single frame using RLE.
    pub fn decode_frame(&self, frame_entry: &FrameEntry) -> eyre::Result<DecodedFrame> {
        let width = (frame_entry.end_coords_x - frame_entry.init_coords_x).abs() as u16;
        let height = (frame_entry.end_coords_y - frame_entry.init_coords_y).abs() as u16;
        let mut decoded_data = vec![0; width as usize * height as usize * 4];

        // Calculate offset relative to the image_data slice
        let start_pos = self.frames_offset + self.frames_count * 16;
        let relative_offset = frame_entry.data_offset.saturating_sub(start_pos) as usize;

        if relative_offset >= self.image_data.len() {
            eyre::bail!("Frame data offset out of bounds");
        }

        let mut data_cursor = &self.image_data[relative_offset..];

        let mut x = 0;
        let mut y = 0;

        while y < height && !data_cursor.is_empty() {
            let rle_tag = data_cursor[0];
            data_cursor = &data_cursor[1..];

            if rle_tag < 128 {
                // Transparent run: skip pixels
                x += rle_tag as u16;
                while x >= width {
                    x -= width;
                    y += 1;
                }
            } else {
                // Color run: draw pixels
                let count = rle_tag - 128;
                for _ in 0..count {
                    if data_cursor.is_empty() {
                        break;
                    }
                    let color_index = data_cursor[0] as usize;
                    data_cursor = &data_cursor[1..];

                    if color_index < self.colours.len() {
                        let color = self.colours[color_index];
                        let index = ((y * width + x) * 4) as usize;
                        if index + 4 <= decoded_data.len() {
                            decoded_data[index..index + 4].copy_from_slice(&color);
                        }
                    }

                    x += 1;
                    if x >= width {
                        x = 0;
                        y += 1;
                    }
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
