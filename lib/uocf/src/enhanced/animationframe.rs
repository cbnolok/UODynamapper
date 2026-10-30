//! # UO Enhanced Client Animation Frame Parser
//!
//! This module handles the parsing of the `animationframe.bin` files from the enhanced client.
//! These files contain the animation data for creatures and characters.
//!

// TODO: the format should be the same also for KR!

pub const MAX_BODY_ID: u32 = 2048;
pub const MAX_ACTION_ID_FALLBACK: u16 = 100;

crate::eyre_imports!();
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

/// Code bytes below this value skip that many transparent pixels; from it up, the code starts a
/// coded run of `code - RLE_RUN_FLAG` solid pixels framed by optional edge pixels.
pub const RLE_RUN_FLAG: u8 = 0x80;
/// Bits of each edge-factor nibble in the byte that follows a run code.
pub const EDGE_FACTOR_BITS: u32 = 4;
/// Mask of the trailing edge factor (low nibble).
pub const EDGE_FACTOR_MASK: u8 = 0x0f;
/// Edge factors are coverage in sixteenths: a factor of `EDGE_FACTOR_ONE` would be full coverage.
pub const EDGE_FACTOR_ONE: u16 = 16;
/// Alpha of every palette colour and of every solid pixel.
pub const OPAQUE_ALPHA: u8 = 0xff;
/// Palette byte holding the "keep colour" flag: 0 means the colour takes the hue.
pub const PALETTE_KEEP_COLOUR_BYTE: usize = 3;
/// Hue mask value of a coded pixel whose palette colour takes the hue.
pub const HUE_MASK_HUEABLE: u8 = 0xff;
/// Hue mask value of a transparent pixel or of a coded pixel that keeps its colour.
pub const HUE_MASK_KEEP_COLOUR: u8 = 0;

const PALETTE_ENTRY_SIZE: usize = 4;
const MAX_PALETTE_COLOURS: usize = 1 << u8::BITS;
const RGBA_CHANNELS: usize = 4;
const RGB_CHANNELS: usize = 3;
const FRAME_ENTRY_SIZE: usize = std::mem::size_of::<RawFrameEntry>();

/// Represents a single decoded animation frame.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub width: u16,
    pub height: u16,
    /// Anchor inside the frame image, `(-x0, -y0)` of the frame box.
    pub center_x: i16,
    pub center_y: i16,
    /// RGBA8888 with straight alpha: solid pixels are opaque, edge pixels keep the palette RGB and
    /// carry their coverage `255 * f >> 4` in alpha.
    pub data: Vec<u8>,
    /// One byte per pixel: `HUE_MASK_HUEABLE` where a coded pixel takes the hue (its red byte is the
    /// hue table key), else `HUE_MASK_KEEP_COLOUR`. Empty for an empty frame.
    pub hue_mask: Vec<u8>,
}

impl DecodedFrame {
    /// Anchor x inside the horizontally flipped image of a mirrored direction: `x1` of the frame
    /// box. The client draws pixel x of a mirrored frame at `-1 - x` from the anchor.
    pub fn mirrored_center_x(&self) -> i16 {
        self.width as i16 - self.center_x
    }

    pub fn has_hueable_pixels(&self) -> bool {
        self.hue_mask.iter().any(|&value| value == HUE_MASK_HUEABLE)
    }
}

/// Represents a single frame entry optimized for memory layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameEntry {
    /// Action id of the frame; equal to the action of the entry name in every client file.
    pub action: u16,
    /// 1-based local frame number, `(direction - 3) * frame_count + frame + 1` for the stored
    /// directions 3..7 of the `AnimationSequence` action (confirmed by the client data). Numbers
    /// can have gaps: the number, not the entry position, gives the direction and frame.
    pub frame_number: u16,
    /// Frame box `x0, y0, x1, y1`, relative to the mobile anchor: pixel (0, 0) of the frame is
    /// drawn at `(x0, y0)` from the anchor.
    pub init_coords_x: i16,
    pub init_coords_y: i16,
    pub end_coords_x: i16,
    pub end_coords_y: i16,
    pub data_offset: u32,
}

impl FrameEntry {
    /// Frame size `(x1 - x0, y1 - y0)`; fails on an inverted or oversized box.
    pub fn size(&self) -> eyre::Result<(u16, u16)> {
        let width = i32::from(self.end_coords_x) - i32::from(self.init_coords_x);
        let height = i32::from(self.end_coords_y) - i32::from(self.init_coords_y);
        match (u16::try_from(width), u16::try_from(height)) {
            (Ok(width), Ok(height)) => Ok((width, height)),
            _ => eyre::bail!(
                "Invalid frame box ({}, {})..({}, {})",
                self.init_coords_x,
                self.init_coords_y,
                self.end_coords_x,
                self.end_coords_y
            ),
        }
    }

    /// Anchor inside the frame image: `(-x0, -y0)`.
    pub fn center(&self) -> (i16, i16) {
        (self.init_coords_x.wrapping_neg(), self.init_coords_y.wrapping_neg())
    }

    /// Anchor x inside the horizontally flipped image of a mirrored direction: `x1`.
    pub fn mirrored_center_x(&self) -> i16 {
        self.end_coords_x
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawFrameEntry {
    action: u16,
    frame_number: u16,
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
    bounds_x0: i16,
    bounds_y0: i16,
    bounds_x1: i16,
    bounds_y1: i16,
    colours_count: u32,
    colours_offset: u32,
    frames_count: u32,
    frames_offset: u32,
}

/// Header box at +0x10..+0x17. The client never reads it; it always contains every frame box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationBounds {
    pub x0: i16,
    pub y0: i16,
    pub x1: i16,
    pub y1: i16,
}

impl AnimationBounds {
    fn from_header(header: &AnimationFrameHeader) -> Self {
        Self {
            x0: i16::from_le(header.bounds_x0),
            y0: i16::from_le(header.bounds_y0),
            x1: i16::from_le(header.bounds_x1),
            y1: i16::from_le(header.bounds_y1),
        }
    }
}

/// Represents a single animation frame file.
#[derive(Debug, Clone)]
pub struct AnimationFrame {
    pub version: u32,
    pub total_size: u32,
    /// Header +0x0c: the body of the entry name in every client file. The client never reads it.
    pub animation_id: u32,
    pub bounds: AnimationBounds,
    pub colours_count: u32,
    pub colours_offset: u32,
    pub frames_count: u32,
    pub frames_offset: u32,
    /// Palette entries `[b0, b1, b2, keep_colour]`: RGB plus the "keep colour" flag (0 = the colour
    /// takes the hue). Every colour is opaque; byte 3 is never alpha.
    pub colours: Vec<[u8; 4]>,
    pub frames: Vec<FrameEntry>,
    /// The raw image data payload
    pub image_data: Arc<[u8]>,
}

#[derive(Debug, Clone)]
pub struct AnimationFrameMetadata {
    pub version: u32,
    pub total_size: u32,
    /// Header +0x0c: the body of the entry name in every client file. The client never reads it.
    pub animation_id: u32,
    pub bounds: AnimationBounds,
    pub frames_count: u32,
    pub frames_offset: u32,
    pub frames: Vec<FrameEntry>,
}

impl AnimationFrame {
    pub fn animationframe_path(body_id: u32, action_id: u16) -> String {
        format!("build/animationframe/{body_id:06}/{action_id:02}.bin")
    }

    pub fn animationframe_hash(body_id: u32, action_id: u16) -> u64 {
        crate::uop_container::hash::hash_file_name_single(&Self::animationframe_path(body_id, action_id))
    }

    /// Loads an animation frame from a zero-copy byte slice (e.g. from UOP).
    pub fn load(data: &[u8]) -> eyre::Result<Self> {
        let header = parse_header(data)?;

        let colours_offset = header.colours_offset as usize;
        let colours_count = header.colours_count as usize;
        let expected_colours_end = colours_offset + colours_count * PALETTE_ENTRY_SIZE;
        if data.len() < expected_colours_end {
            eyre::bail!("Data too small for color palette");
        }

        let mut colours = Vec::with_capacity(colours_count);
        let palette_slice = &data[colours_offset..expected_colours_end];
        for chunk in palette_slice.chunks_exact(PALETTE_ENTRY_SIZE) {
            colours.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }

        let (frames, expected_frames_end) = parse_frame_entries(data, header)?;
        let image_data: Arc<[u8]> = data[expected_frames_end..].into();

        Ok(Self {
            version: u32::from_le(header.version),
            total_size: u32::from_le(header.total_size),
            animation_id: u32::from_le(header.animation_id),
            bounds: AnimationBounds::from_header(header),
            colours_count: u32::from_le(header.colours_count),
            colours_offset: u32::from_le(header.colours_offset),
            frames_count: u32::from_le(header.frames_count),
            frames_offset: u32::from_le(header.frames_offset),
            colours,
            frames,
            image_data,
        })
    }

    pub fn load_metadata(data: &[u8]) -> eyre::Result<AnimationFrameMetadata> {
        let header = parse_header(data)?;
        let (frames, _) = parse_frame_entries(data, header)?;

        Ok(AnimationFrameMetadata {
            version: u32::from_le(header.version),
            total_size: u32::from_le(header.total_size),
            animation_id: u32::from_le(header.animation_id),
            bounds: AnimationBounds::from_header(header),
            frames_count: u32::from_le(header.frames_count),
            frames_offset: u32::from_le(header.frames_offset),
            frames,
        })
    }

    /// Decodes a single frame as `UOAnimations::DecodeFrameRle` does over a zeroed canvas, with
    /// straight alpha, plus the per-pixel hue mask.
    ///
    /// The stream is walked over the linear pixel index `0..width * height`: a skip may carry
    /// over rows (and past the end), a coded run may not pass the box. A run past the box, a
    /// stream that ends early or a palette index past the colour count is an error.
    pub fn decode_frame(&self, frame_entry: &FrameEntry) -> eyre::Result<DecodedFrame> {
        let (width, height) = frame_entry.size()?;
        let (center_x, center_y) = frame_entry.center();

        if width == 0 || height == 0 {
            return Ok(DecodedFrame {
                width: 0,
                height: 0,
                center_x,
                center_y,
                data: Vec::new(),
                hue_mask: Vec::new(),
            });
        }

        let pixel_count = width as usize * height as usize;
        let mut decoded_data = vec![0u8; pixel_count * RGBA_CHANNELS];
        let mut hue_mask = vec![HUE_MASK_KEEP_COLOUR; pixel_count];

        // Calculate offset relative to the image_data slice
        let start_pos = self.frames_offset as usize + self.frames_count as usize * FRAME_ENTRY_SIZE;
        let relative_offset = (frame_entry.data_offset as usize)
            .checked_sub(start_pos)
            .filter(|&offset| offset < self.image_data.len())
            .ok_or_else(|| eyre::eyre!("Frame data offset out of bounds"))?;

        let palette = DecodePalette::new(&self.colours);
        let mut stream = RleStream {
            source: &self.image_data[relative_offset..],
            pos: 0,
        };
        let mut pixel_index = 0usize;

        while pixel_index < pixel_count {
            let code = stream.next_byte()?;
            if code < RLE_RUN_FLAG {
                pixel_index += code as usize;
                continue;
            }

            let factors = stream.next_byte()?;
            let leading_factor = factors >> EDGE_FACTOR_BITS;
            let trailing_factor = factors & EDGE_FACTOR_MASK;
            let solid_count = (code - RLE_RUN_FLAG) as usize;
            let run_end = pixel_index
                + solid_count
                + usize::from(leading_factor != 0)
                + usize::from(trailing_factor != 0);
            if run_end > pixel_count {
                eyre::bail!(
                    "RLE run writes pixel {} past the {}x{} frame box",
                    run_end,
                    width,
                    height
                );
            }

            if leading_factor != 0 {
                let colour = stream.next_byte()?;
                palette.write_edge(&mut decoded_data, &mut hue_mask, pixel_index, colour, leading_factor)?;
                pixel_index += 1;
            }

            let indices = stream.take(solid_count)?;
            palette.write_solid(&mut decoded_data, &mut hue_mask, pixel_index, indices)?;
            pixel_index += solid_count;

            if trailing_factor != 0 {
                let colour = stream.next_byte()?;
                palette.write_edge(&mut decoded_data, &mut hue_mask, pixel_index, colour, trailing_factor)?;
                pixel_index += 1;
            }
        }

        Ok(DecodedFrame {
            width,
            height,
            center_x,
            center_y,
            data: decoded_data,
            hue_mask,
        })
    }
}

fn parse_header(data: &[u8]) -> eyre::Result<&AnimationFrameHeader> {
    if data.len() < std::mem::size_of::<AnimationFrameHeader>() {
        eyre::bail!("Data too small to contain AnimationFrame header");
    }

    let header: &AnimationFrameHeader =
        bytemuck::from_bytes(&data[0..std::mem::size_of::<AnimationFrameHeader>()]);

    if &header.signature[0..3] != b"AMO" {
        eyre::bail!("Invalid animation frame signature");
    }
    Ok(header)
}

/// Frame entries with absolute data offsets, and the end of the frame table.
fn parse_frame_entries(
    data: &[u8],
    header: &AnimationFrameHeader,
) -> eyre::Result<(Vec<FrameEntry>, usize)> {
    let frames_offset = u32::from_le(header.frames_offset);
    let frames_count = u32::from_le(header.frames_count) as usize;
    let expected_frames_end = frames_offset as usize + frames_count * FRAME_ENTRY_SIZE;
    if data.len() < expected_frames_end {
        eyre::bail!("Data too small for frames entries");
    }

    let frames_slice = &data[frames_offset as usize..expected_frames_end];
    let raw_frames: &[RawFrameEntry] = bytemuck::cast_slice(frames_slice);

    let mut frames = Vec::with_capacity(frames_count);
    for (i, raw) in raw_frames.iter().enumerate() {
        // Convert to native endianness and absolute offset
        frames.push(FrameEntry {
            action: u16::from_le(raw.action),
            frame_number: u16::from_le(raw.frame_number),
            init_coords_x: i16::from_le(raw.init_coords_x),
            init_coords_y: i16::from_le(raw.init_coords_y),
            end_coords_x: i16::from_le(raw.end_coords_x),
            end_coords_y: i16::from_le(raw.end_coords_y),
            data_offset: frames_offset
                .wrapping_add((i * FRAME_ENTRY_SIZE) as u32)
                .wrapping_add(u32::from_le(raw.data_offset)),
        });
    }
    Ok((frames, expected_frames_end))
}

struct RleStream<'a> {
    source: &'a [u8],
    pos: usize,
}

impl<'a> RleStream<'a> {
    fn next_byte(&mut self) -> eyre::Result<u8> {
        let byte = *self
            .source
            .get(self.pos)
            .ok_or_else(|| eyre::eyre!("RLE stream ends early at byte {}", self.pos))?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, count: usize) -> eyre::Result<&'a [u8]> {
        let bytes = self
            .source
            .get(self.pos..self.pos + count)
            .ok_or_else(|| eyre::eyre!("RLE stream ends early at byte {}", self.pos))?;
        self.pos += count;
        Ok(bytes)
    }
}

/// Palette as opaque RGBA8888 words plus the hue mask value of each colour.
struct DecodePalette {
    colours: [[u8; RGBA_CHANNELS]; MAX_PALETTE_COLOURS],
    hue_mask: [u8; MAX_PALETTE_COLOURS],
    count: usize,
}

impl DecodePalette {
    fn new(entries: &[[u8; PALETTE_ENTRY_SIZE]]) -> Self {
        let mut colours = [[0u8; RGBA_CHANNELS]; MAX_PALETTE_COLOURS];
        let mut hue_mask = [HUE_MASK_KEEP_COLOUR; MAX_PALETTE_COLOURS];
        for ((colour, mask), entry) in colours
            .iter_mut()
            .zip(hue_mask.iter_mut())
            .zip(entries.iter())
        {
            *colour = [entry[0], entry[1], entry[2], OPAQUE_ALPHA];
            if entry[PALETTE_KEEP_COLOUR_BYTE] == 0 {
                *mask = HUE_MASK_HUEABLE;
            }
        }
        Self {
            colours,
            hue_mask,
            count: entries.len().min(MAX_PALETTE_COLOURS),
        }
    }

    fn check_index(&self, index: u8) -> eyre::Result<usize> {
        let index = index as usize;
        if index >= self.count {
            eyre::bail!("Palette index {} past the {} colours", index, self.count);
        }
        Ok(index)
    }

    fn write_solid(
        &self,
        data: &mut [u8],
        hue_mask: &mut [u8],
        pixel_index: usize,
        indices: &[u8],
    ) -> eyre::Result<()> {
        let end = pixel_index + indices.len();
        let pixels = data[pixel_index * RGBA_CHANNELS..end * RGBA_CHANNELS].chunks_exact_mut(RGBA_CHANNELS);
        for ((pixel, mask), &index) in pixels.zip(hue_mask[pixel_index..end].iter_mut()).zip(indices) {
            let index = self.check_index(index)?;
            pixel.copy_from_slice(&self.colours[index]);
            *mask = self.hue_mask[index];
        }
        Ok(())
    }

    /// Edge pixel with coverage `factor / 16`: palette RGB, alpha `255 * factor >> 4`.
    fn write_edge(
        &self,
        data: &mut [u8],
        hue_mask: &mut [u8],
        pixel_index: usize,
        index: u8,
        factor: u8,
    ) -> eyre::Result<()> {
        let index = self.check_index(index)?;
        let pixel = &mut data[pixel_index * RGBA_CHANNELS..(pixel_index + 1) * RGBA_CHANNELS];
        pixel[..RGB_CHANNELS].copy_from_slice(&self.colours[index][..RGB_CHANNELS]);
        pixel[RGB_CHANNELS] = edge_alpha(factor);
        hue_mask[pixel_index] = self.hue_mask[index];
        Ok(())
    }
}

/// Straight alpha of an edge pixel: the client's `255 * f >> 4` over black.
fn edge_alpha(factor: u8) -> u8 {
    ((u16::from(OPAQUE_ALPHA) * u16::from(factor)) >> EDGE_FACTOR_BITS).min(u16::from(OPAQUE_ALPHA)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    /// Red keeps its colour (flag 1), green and grey take the hue (flag 0).
    const TEST_COLOURS: [[u8; 4]; 3] = [[255, 0, 0, 1], [0, 255, 0, 0], [90, 90, 90, 0]];
    const HEADER_SIZE: u32 = 40;
    const TEST_ANIMATION_ID: u32 = 42;
    const TEST_VERSION: u32 = 4;
    const TEST_ACTION: u16 = 9;
    const TEST_FRAME_NUMBER: u16 = 11;

    fn single_frame_payload(
        header_init_x: i16,
        header_init_y: i16,
        frame_init_x: i16,
        frame_init_y: i16,
        frame_end_x: i16,
        frame_end_y: i16,
        frame_bytes: &[u8],
    ) -> Vec<u8> {
        let colours_offset = HEADER_SIZE;
        let frames_offset = colours_offset + (TEST_COLOURS.len() * PALETTE_ENTRY_SIZE) as u32;
        let image_offset = frames_offset + FRAME_ENTRY_SIZE as u32;
        let total_size = image_offset + frame_bytes.len() as u32;

        let mut bytes = Vec::with_capacity(total_size as usize);
        bytes.extend_from_slice(b"AMO\x04");
        bytes.write_u32::<LittleEndian>(TEST_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(total_size).unwrap();
        bytes.write_u32::<LittleEndian>(TEST_ANIMATION_ID).unwrap();
        bytes.write_i16::<LittleEndian>(header_init_x).unwrap();
        bytes.write_i16::<LittleEndian>(header_init_y).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_y).unwrap();
        bytes.write_u32::<LittleEndian>(TEST_COLOURS.len() as u32).unwrap();
        bytes.write_u32::<LittleEndian>(colours_offset).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(frames_offset).unwrap();

        for colour in TEST_COLOURS {
            bytes.extend_from_slice(&colour);
        }

        bytes.write_u16::<LittleEndian>(TEST_ACTION).unwrap();
        bytes.write_u16::<LittleEndian>(TEST_FRAME_NUMBER).unwrap();
        bytes.write_i16::<LittleEndian>(frame_init_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_init_y).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_x).unwrap();
        bytes.write_i16::<LittleEndian>(frame_end_y).unwrap();
        bytes.write_u32::<LittleEndian>(image_offset - frames_offset).unwrap();
        bytes.extend_from_slice(frame_bytes);

        bytes
    }

    fn decode(frame_init: (i16, i16), frame_end: (i16, i16), frame_bytes: &[u8]) -> eyre::Result<DecodedFrame> {
        let payload = single_frame_payload(0, 0, frame_init.0, frame_init.1, frame_end.0, frame_end.1, frame_bytes);
        let animation = AnimationFrame::load(&payload).unwrap();
        animation.decode_frame(&animation.frames[0])
    }

    #[test]
    fn load_resolves_palette_frame_table_and_relative_image_offset() {
        let payload = single_frame_payload(5, 6, 3, 4, 5, 5, &[130, 0, 0, 1]);

        let animation = AnimationFrame::load(&payload).unwrap();
        let metadata = AnimationFrame::load_metadata(&payload).unwrap();

        assert_eq!(animation.version, TEST_VERSION);
        assert_eq!(animation.total_size, payload.len() as u32);
        assert_eq!(animation.animation_id, TEST_ANIMATION_ID);
        assert_eq!(animation.bounds, AnimationBounds { x0: 5, y0: 6, x1: 5, y1: 5 });
        assert_eq!(animation.colours, TEST_COLOURS.to_vec());
        assert_eq!(animation.frames.len(), 1);
        assert_eq!(animation.frames[0].action, TEST_ACTION);
        assert_eq!(animation.frames[0].frame_number, TEST_FRAME_NUMBER);
        assert_eq!(animation.frames[0].data_offset, 68);
        assert_eq!(metadata.version, animation.version);
        assert_eq!(metadata.total_size, animation.total_size);
        assert_eq!(metadata.animation_id, animation.animation_id);
        assert_eq!(metadata.bounds, animation.bounds);
        assert_eq!(metadata.frames, animation.frames);
    }

    #[test]
    fn decode_frame_expands_solid_pixels_as_opaque_and_ignores_the_header_box() {
        let payload = single_frame_payload(5, 6, 3, 4, 5, 5, &[130, 0, 0, 1]);
        let animation = AnimationFrame::load(&payload).unwrap();

        let frame = animation.decode_frame(&animation.frames[0]).unwrap();

        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!((frame.center_x, frame.center_y), (-3, -4));
        assert_eq!(frame.data, vec![255, 0, 0, OPAQUE_ALPHA, 0, 255, 0, OPAQUE_ALPHA]);
        assert_eq!(frame.hue_mask, vec![HUE_MASK_KEEP_COLOUR, HUE_MASK_HUEABLE]);
        assert!(frame.has_hueable_pixels());
    }

    #[test]
    fn decode_frame_writes_edges_with_straight_coverage_alpha() {
        // One leading edge at 8/16, one solid, one trailing edge at 4/16, then the other rows.
        let frame = decode((-2, -3), (1, 0), &[129, 0x84, 1, 0, 2, 6]).unwrap();

        assert_eq!((frame.width, frame.height), (3, 3));
        assert_eq!((frame.center_x, frame.center_y), (2, 3));
        assert_eq!(&frame.data[..12], &[0, 255, 0, 127, 255, 0, 0, OPAQUE_ALPHA, 90, 90, 90, 63]);
        assert_eq!(
            &frame.hue_mask[..3],
            &[HUE_MASK_HUEABLE, HUE_MASK_KEEP_COLOUR, HUE_MASK_HUEABLE]
        );
        assert!(frame.data[12..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn decode_frame_carries_skips_across_rows() {
        // 2x2 box: skip 3 pixels (row 0 and the first of row 1), then one solid pixel.
        let frame = decode((0, 0), (2, 2), &[3, 129, 0, 2]).unwrap();

        assert_eq!(&frame.data[..12], &[0; 12]);
        assert_eq!(&frame.data[12..], &[90, 90, 90, OPAQUE_ALPHA]);
        assert_eq!(frame.hue_mask, vec![0, 0, 0, HUE_MASK_HUEABLE]);
    }

    #[test]
    fn decode_frame_allows_a_trailing_skip_past_the_box() {
        let frame = decode((0, 0), (2, 1), &[129, 0, 0, 100]).unwrap();
        assert_eq!(frame.data, vec![255, 0, 0, OPAQUE_ALPHA, 0, 0, 0, 0]);
        assert!(!frame.has_hueable_pixels());
    }

    #[test]
    fn decoded_frame_reports_the_mirrored_anchor() {
        // Body 400 action 0 frame 1 box: x -15..15, y -74..-10, all transparent.
        let (x0, y0, x1, y1) = (-15i16, -74i16, 15i16, -10i16);
        let pixel_count = (x1 - x0) as usize * (y1 - y0) as usize;
        let max_skip = RLE_RUN_FLAG as usize - 1;
        let skips = vec![max_skip as u8; pixel_count.div_ceil(max_skip)];
        let frame = decode((x0, y0), (x1, y1), &skips).unwrap();

        assert_eq!((frame.center_x, frame.center_y), (-x0, -y0));
        assert_eq!(frame.mirrored_center_x(), x1);

        let payload = single_frame_payload(0, 0, x0, y0, x1, y1, &skips);
        let animation = AnimationFrame::load(&payload).unwrap();
        assert_eq!(animation.frames[0].center(), (-x0, -y0));
        assert_eq!(animation.frames[0].mirrored_center_x(), x1);
    }

    #[test]
    fn decode_frame_rejects_runs_past_the_box() {
        let error = decode((0, 0), (2, 1), &[131, 0, 0, 1, 2]).unwrap_err();
        assert!(error.to_string().contains("past the 2x1 frame box"), "{error}");
        // An edge pixel counts towards the run.
        assert!(decode((0, 0), (2, 1), &[130, 0x10, 0, 1, 2]).is_err());
    }

    #[test]
    fn decode_frame_rejects_truncated_streams() {
        assert!(decode((0, 0), (2, 1), &[130, 0, 0]).is_err());
        assert!(decode((0, 0), (2, 1), &[1]).is_err());
        assert!(decode((0, 0), (2, 1), &[129]).is_err());
    }

    #[test]
    fn decode_frame_rejects_palette_indices_past_the_colour_count() {
        let error = decode((0, 0), (1, 1), &[129, 0, 3]).unwrap_err();
        assert!(error.to_string().contains("Palette index 3"), "{error}");
    }

    #[test]
    fn decode_frame_rejects_inverted_boxes() {
        assert!(decode((2, 0), (0, 1), &[]).is_err());
    }
}
