//! # UO Classic Animation Parser
//!
//! This module handles the parsing of `anim*.mul` and `anim*.idx` files.
//! These files contain the RLE-encoded animations for mobiles and effects.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use crate::classic::generic_index::IndexFile;
use crate::classic::verdata::{VerFileId, Verdata};

pub const MAX_ANIM_FILES: u8 = 6; // anim, anim2, anim3, anim4, anim5
const MAX_CLASSIC_ANIM_FRAME_DIMENSION: u16 = 2048;
const MAX_CLASSIC_ANIM_FRAME_PIXELS: usize =
    MAX_CLASSIC_ANIM_FRAME_DIMENSION as usize * MAX_CLASSIC_ANIM_FRAME_DIMENSION as usize;

/// Represents a single decoded animation frame from a MUL file.
#[derive(Debug, Clone)]
pub struct AnimFrame {
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
    pub data: Vec<u8>, // RGBA8888
}

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

            let idx_path = client_path.join(&idx_name);
            let mul_path = client_path.join(&mul_name);

            if idx_path.exists() && mul_path.exists() {
                let idx = IndexFile::load(idx_path)?;
                let mul_file = File::open(mul_path)?;
                let mul = unsafe { memmap2::Mmap::map(&mul_file)? };
                sources.push(Some(AnimSource { idx, mul }));
                log::info!("uocf: Loaded animation source {}", idx_name);
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
                return entry.lookup().is_some();
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

    /// Decodes an animation from a specific MUL file.
    /// Returns a list of frames.
    pub fn decode_animation(&self, file_idx: u8, anim_id: u32) -> eyre::Result<Vec<AnimFrame>> {
        let source = self
            .sources
            .get(file_idx as usize)
            .and_then(|s| s.as_ref())
            .ok_or_else(|| eyre!("Animation source {} not loaded", file_idx))?;

        if file_idx == 0 {
            if let Some(verdata) = &self.verdata {
                if let Some(bytes) = verdata.read_patch(VerFileId::Anim, anim_id as i32)? {
                    return decode_animation_payload(&bytes, 0);
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

        decode_animation_payload(&source.mul[lookup..end], 0)
    }
}

fn decode_animation_payload(data: &[u8], lookup: usize) -> eyre::Result<Vec<AnimFrame>> {
        let mut mul_ptr = &data[lookup..];

        // Read Palette (256 colors, RGB555)
        let mut palette = [0u16; 256];
        for i in 0..256 {
            palette[i] = mul_ptr.read_u16::<LittleEndian>()?;
        }

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

        let frame_offset_base = lookup
            .checked_add(512)
            .ok_or_else(|| eyre!("Animation frame offset base overflows"))?;

        let frame_count = mul_ptr.read_u32::<LittleEndian>()?;
        if frame_count > 1000 {
            eyre::bail!("Suspiciously high frame count: {}", frame_count);
        }

        let mut frame_offsets = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            frame_offsets.push(mul_ptr.read_u32::<LittleEndian>()?);
        }

        let mut frames = Vec::with_capacity(frame_count as usize);
        for i in 0..frame_count {
            let offset = frame_offsets[i as usize] as usize;
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

            let pixel_len = frame_pixel_len(width, height)?;
            let mut pixel_data = vec![0u8; pixel_len];

            // RLE Decoding
            loop {
                let header = match frame_ptr.read_u32::<LittleEndian>() {
                    Ok(h) => h,
                    Err(_) => break,
                };

                if header == 0x7FFF7FFF {
                    break;
                }

                let x_run = (header & 0xFFF) as usize;
                let mut x_offset = ((header >> 22) & 0x3FF) as i32;
                let mut y_offset = ((header >> 12) & 0x3FF) as i32;

                // Sign-extend 10-bit values
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
                            let palette_index = frame_ptr.read_u8()? as usize;
                            let color = rgba_palette[palette_index];
                            let pixel_idx = frame_pixel_offset(y, final_x, width)?;
                            pixel_data[pixel_idx..pixel_idx + 4].copy_from_slice(&color);
                        } else {
                            // Skip the byte even if out of bounds
                            frame_ptr.read_u8()?;
                        }
                    }
                } else {
                    // Skip the bytes for this run
                    for _ in 0..x_run {
                        frame_ptr.read_u8()?;
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

        Ok(frames)
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

/// Handles AnimationDefinition.uop parsing for Body ID redirects (aliasing).
pub struct AnimationDefinition {
    pub redirects: std::collections::HashMap<u32, u32>,
}

impl AnimationDefinition {
    pub fn new() -> Self {
        Self {
            redirects: std::collections::HashMap::new(),
        }
    }

    /// Parses an AnimationDefinition entry from a byte slice (usually from UOP).
    pub fn parse(data: &[u8]) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(data);
        use byteorder::{LittleEndian, ReadBytesExt};

        // Format: u32 version (usually 1), u32 count, then count * (u32 original_id, u32 new_id)
        if data.len() < 8 {
            eyre::bail!("AnimationDefinition data too small");
        }
        let _version = reader.read_u32::<LittleEndian>()?;
        let count = reader.read_u32::<LittleEndian>()?;

        let mut redirects = std::collections::HashMap::with_capacity(count as usize);
        for _ in 0..count {
            let original_id = reader.read_u32::<LittleEndian>()?;
            let new_id = reader.read_u32::<LittleEndian>()?;
            redirects.insert(original_id, new_id);
        }

        Ok(Self { redirects })
    }

    /// Resolves a Body ID to its redirected ID, if any.
    pub fn resolve(&self, body_id: u32) -> u32 {
        *self.redirects.get(&body_id).unwrap_or(&body_id)
    }
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

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].width, 1);
        assert_eq!(frames[0].height, 1);
        assert_eq!(frames[0].data, vec![248, 248, 248, 255]);
    }
}
