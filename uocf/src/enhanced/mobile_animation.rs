//! # UO Enhanced Client Mobile Animation Parser
//!
//! This module handles the parsing of the `.mobileanimation` files from the enhanced client.
//! These files contain the animation data for creatures, characters, and equipment.

crate::eyre_imports!();
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// Represents a single 32-bit ARGB color entry in the palette.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct ColourEntry {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

/// Represents a single frame entry from the frame table.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameEntry {
    pub id: u16,
    pub frame_num: u16,
    pub init_coords_x: i16,
    pub init_coords_y: i16,
    pub end_coords_x: i16,
    pub end_coords_y: i16,
    pub data_offset: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawFrameEntry {
    id: u16,
    frame_num: u16,
    init_coords_x: i16,
    init_coords_y: i16,
    end_coords_x: i16,
    end_coords_y: i16,
    data_offset: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MobileAnimationHeader {
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

/// Represents a fully decoded animation frame.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub width: i32,
    pub height: i32,
    pub center_x: i16,
    pub center_y: i16,
    pub data: Vec<u8>, // RGBA
}

/// Represents a single EC animation action.
#[derive(Debug, Clone)]
pub struct UoAnimation {
    pub animation_id: u32,
    pub colours: Vec<ColourEntry>,
    pub frames: Vec<DecodedFrame>,
}

impl UoAnimation {
    /// Loads and decodes an animation from a zero-copy byte slice.
    pub fn load(data: &[u8]) -> eyre::Result<Self> {
        if data.len() < std::mem::size_of::<MobileAnimationHeader>() {
            eyre::bail!("Data too small to contain MobileAnimation header");
        }

        let header: &MobileAnimationHeader = bytemuck::from_bytes(&data[0..std::mem::size_of::<MobileAnimationHeader>()]);

        if &header.signature != b"AMO\x04" {
            eyre::bail!("Invalid mobile animation signature");
        }

        let colours_offset = u32::from_le(header.colours_offset) as usize;
        let colours_count = u32::from_le(header.colours_count) as usize;
        let expected_colours_end = colours_offset + colours_count * std::mem::size_of::<ColourEntry>();

        if data.len() < expected_colours_end {
            eyre::bail!("Data too small for color palette");
        }

        let palette_slice = &data[colours_offset..expected_colours_end];
        let raw_colours: &[ColourEntry] = bytemuck::cast_slice(palette_slice);
        let colours = raw_colours.to_vec();

        let frames_offset = u32::from_le(header.frames_offset) as usize;
        let frames_count = u32::from_le(header.frames_count) as usize;
        let expected_frames_end = frames_offset + frames_count * std::mem::size_of::<RawFrameEntry>();

        if data.len() < expected_frames_end {
            eyre::bail!("Data too small for frames entries");
        }

        let frames_slice = &data[frames_offset..expected_frames_end];
        let raw_frames: &[RawFrameEntry] = bytemuck::cast_slice(frames_slice);

        let mut frame_entries = Vec::with_capacity(frames_count);
        for (i, raw) in raw_frames.iter().enumerate() {
            frame_entries.push(FrameEntry {
                id: u16::from_le(raw.id),
                frame_num: u16::from_le(raw.frame_num),
                init_coords_x: i16::from_le(raw.init_coords_x),
                init_coords_y: i16::from_le(raw.init_coords_y),
                end_coords_x: i16::from_le(raw.end_coords_x),
                end_coords_y: i16::from_le(raw.end_coords_y),
                data_offset: frames_offset as u32 + (i as u32 * 16) + u32::from_le(raw.data_offset),
            });
        }

        let image_data_offset = expected_frames_end as u32;
        let frames = Self::decode_frames(
            &frame_entries,
            &colours,
            &data[image_data_offset as usize..],
            image_data_offset,
        )?;

        Ok(Self {
            animation_id: u32::from_le(header.animation_id),
            colours,
            frames,
        })
    }

    fn decode_frames(
        frame_entries: &[FrameEntry],
        colours: &[ColourEntry],
        image_data: &[u8],
        image_data_offset: u32,
    ) -> eyre::Result<Vec<DecodedFrame>> {
        let mut decoded_frames = Vec::with_capacity(frame_entries.len());

        for entry in frame_entries {
            let width = (entry.end_coords_x - entry.init_coords_x).abs() as i32;
            let height = (entry.end_coords_y - entry.init_coords_y).abs() as i32;
            let center_x = width as i16 - entry.end_coords_x;
            let center_y = -entry.end_coords_y;

            let mut frame_pixel_data = vec![0; (width * height * 4) as usize];
            let mut cursor_pos = entry
                .data_offset
                .checked_sub(image_data_offset)
                .ok_or_else(|| eyre!("Frame data offset points before image data section"))?
                as usize;

            let mut x = 0;
            let mut y = 0;

            while y < height {
                if cursor_pos >= image_data.len() {
                    break;
                }
                let header = image_data[cursor_pos];
                cursor_pos += 1;

                if header < 0x80 {
                    x += header as i32;
                    while x >= width {
                        x -= width;
                        y += 1;
                    }
                } else {
                    if cursor_pos >= image_data.len() { break; }
                    let next = image_data[cursor_pos];
                    cursor_pos += 1;

                    let factor1 = next / 16;
                    let factor2 = next % 16;

                    if factor1 > 0 {
                        if cursor_pos >= image_data.len() { break; }
                        let source_color = colours[image_data[cursor_pos] as usize];
                        cursor_pos += 1;
                        let index = ((y * width + x) * 4) as usize;
                        if index + 4 <= frame_pixel_data.len() {
                            let target_color = ColourEntry {
                                r: frame_pixel_data[index],
                                g: frame_pixel_data[index + 1],
                                b: frame_pixel_data[index + 2],
                                a: frame_pixel_data[index + 3],
                            };
                            let blended_color = Self::combine_colors(source_color, target_color, factor1 as i32);
                            frame_pixel_data[index..index + 4].copy_from_slice(&[
                                blended_color.r, blended_color.g, blended_color.b, blended_color.a
                            ]);
                        }
                        x += 1;
                        if x >= width { x = 0; y += 1; }
                    }

                    for _ in 0..(header - 0x80) {
                        if cursor_pos >= image_data.len() { break; }
                        let color = colours[image_data[cursor_pos] as usize];
                        cursor_pos += 1;
                        let index = ((y * width + x) * 4) as usize;
                        if index + 4 <= frame_pixel_data.len() {
                            frame_pixel_data[index..index + 4].copy_from_slice(&[
                                color.r, color.g, color.b, color.a
                            ]);
                        }
                        x += 1;
                        if x >= width { x = 0; y += 1; }
                    }

                    if factor2 > 0 {
                        if cursor_pos >= image_data.len() { break; }
                        let source_color = colours[image_data[cursor_pos] as usize];
                        cursor_pos += 1;
                        let index = ((y * width + x) * 4) as usize;
                        if index + 4 <= frame_pixel_data.len() {
                            let target_color = ColourEntry {
                                r: frame_pixel_data[index],
                                g: frame_pixel_data[index + 1],
                                b: frame_pixel_data[index + 2],
                                a: frame_pixel_data[index + 3],
                            };
                            let blended_color = Self::combine_colors(source_color, target_color, factor2 as i32);
                            frame_pixel_data[index..index + 4].copy_from_slice(&[
                                blended_color.r, blended_color.g, blended_color.b, blended_color.a
                            ]);
                        }
                        x += 1;
                        if x >= width { x = 0; y += 1; }
                    }
                }
            }

            decoded_frames.push(DecodedFrame {
                width,
                height,
                center_x,
                center_y,
                data: frame_pixel_data,
            });
        }

        Ok(decoded_frames)
    }

    fn combine_colors(source: ColourEntry, target: ColourEntry, factor: i32) -> ColourEntry {
        let r = (((source.r as i32 * factor) + (target.r as i32 * (16 - factor))) / 16) as u8;
        let g = (((source.g as i32 * factor) + (target.g as i32 * (16 - factor))) / 16) as u8;
        let b = (((source.b as i32 * factor) + (target.b as i32 * (16 - factor))) / 16) as u8;
        let a = (((source.a as i32 * factor) + (target.a as i32 * (16 - factor))) / 16) as u8;
        ColourEntry { b, g, r, a }
    }
}

/// Represents a mobile, which is a collection of animations.
#[derive(Debug, Clone)]
pub struct Mobile {
    pub body_id: i32,
    pub actions: HashMap<u16, UoAnimation>,
}

impl Mobile {
    pub fn new(body_id: i32) -> Self {
        Self {
            body_id,
            actions: HashMap::new(),
        }
    }
}
