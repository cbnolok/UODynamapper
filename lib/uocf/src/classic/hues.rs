//! # UO Hue Data Parser (`hues.mul`)
//!
//! This module handles the loading and application of hues from the `hues.mul` file.
//! Hues are used in Ultima Online to colorize items and creatures. Each hue contains a
//! color table that maps source red-channel shade steps to specific colors.
//!
//! ## File Format
//!
//! `hues.mul` is a collection of hue entries. Each entry is 88 bytes long and has the
//! following structure:
//!
//! - **Color Table** (64 bytes, 32 x u16, little-endian): A table of 32 16-bit colors (ARGB1555).
//!   This table represents a gradient of colors that will replace the grayscale tones of an asset.
//! - **Table Start** (2 bytes, u16, little-endian): The starting color of the hue range.
//! - **Table End** (2 bytes, u16, little-endian): The ending color of the hue range.
//! - **Name** (20 bytes, ASCII string): The name of the hue.
//!
//! ## Hue Application
//!
//! When a hue is applied to an asset, the 5-bit red component of each pixel is used as an index
//! into the hue's color table to determine the new color for that pixel. This matches the
//! UOFiddler/Punt-style lookup used by existing UO tooling.

use std::io::Read;
use std::fs::File;
use std::path::Path;

crate::eyre_imports!();

const HUE_ENTRY_SIZE: usize = 88;
const HUE_BLOCK_SIZE: usize = 708; // 4 byte header + 8 * 88 byte entries
const HUES_PER_BLOCK: usize = 8;

/// Represents a single hue entry from hues.mul, containing a color table and metadata.
#[derive(Debug, Clone, Copy)]
pub struct HueEntry {
    pub id: u32,
    pub color_table: [u16; 32],
    pub table_start: u16,
    pub table_end: u16,
    pub name: [u8; 20],
}

/// Loads the hues.mul file into memory.
///
/// # Arguments
///
/// * `path` - Path to the `hues.mul` file.
///
/// # Returns
///
/// A `Result` containing a `Vec<HueEntry>`, or an `eyre::Report` on failure.
///
pub fn load_hues(path: &Path) -> eyre::Result<Vec<HueEntry>> {
    let mut file = File::open(path).wrap_err("Failed to open hues.mul")?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;

    let num_blocks = buffer.len() / HUE_BLOCK_SIZE;
    let mut hues = Vec::with_capacity(num_blocks * HUES_PER_BLOCK);

    for block_idx in 0..num_blocks {
        let block_offset = block_idx * HUE_BLOCK_SIZE;
        // Skip 4-byte header
        let entries_offset = block_offset + 4;

        for entry_idx in 0..HUES_PER_BLOCK {
            let offset = entries_offset + entry_idx * HUE_ENTRY_SIZE;
            if offset + HUE_ENTRY_SIZE > buffer.len() {
                break;
            }

            let entry_data = &buffer[offset..offset + HUE_ENTRY_SIZE];
            let mut color_table = [0u16; 32];
            for i in 0..32 {
                color_table[i] = u16::from_le_bytes([entry_data[i * 2], entry_data[i * 2 + 1]]);
            }

            let table_start = u16::from_le_bytes([entry_data[64], entry_data[65]]);
            let table_end = u16::from_le_bytes([entry_data[66], entry_data[67]]);
            let mut name = [0u8; 20];
            name.copy_from_slice(&entry_data[68..88]);

            hues.push(HueEntry {
                id: (block_idx * HUES_PER_BLOCK + entry_idx + 1) as u32,
                color_table,
                table_start,
                table_end,
                name,
            });
        }
    }

    Ok(hues)
}


impl HueEntry {
    /// Applies the hue to a 16-bit color value (ARGB1555).
    ///
    /// # Arguments
    ///
    /// * `color` - The original 16-bit color.
    /// * `partial_hue` - If true, only applies hue to grayscale pixels.
    ///
    /// # Returns
    ///
    /// The new 16-bit color value with the hue applied.
    pub fn apply_to_color16(&self, color: u16, partial_hue: bool) -> u16 {
        if color & 0x8000 == 0 { // No alpha, so no hue
            return color;
        }

        let r5: u16 = (color >> 10) & 0x1F;
        let g5: u16 = (color >> 5) & 0x1F;
        let b5: u16 = color & 0x1F;

        // Grayscale check for partial hueing
        if partial_hue && (r5 != g5 || r5 != b5) {
            return color;
        }

        self.color_table[r5 as usize] | 0x8000 // Preserve alpha bit
    }

    /// Applies the hue to a 32-bit color value (ARGB8888).
    ///
    /// # Arguments
    ///
    /// * `color` - The original 32-bit ARGB color.
    /// * `partial_hue` - If true, only applies hue to grayscale pixels.
    ///
    /// # Returns
    ///
    /// The new 32-bit color value with the hue applied.
    pub fn apply_to_color32(&self, color: u32, partial_hue: bool) -> u32 {
        let a = (color >> 24) & 0xFF;
        if a == 0 { // Transparent, no hue
            return color;
        }

        let r8: u32 = (color >> 16) & 0xFF;
        let g8: u32 = (color >> 8) & 0xFF;
        let b8: u32 = color & 0xFF;

        // Grayscale check for partial hueing
        if partial_hue && (r8 != g8 || r8 != b8) {
            return color;
        }

        let color_index = (r8 >> 3).min(31) as usize;
        let hued_color16: u16 = self.color_table[color_index];

        // Convert the 16-bit hued color back to 32-bit ARGB
        let hr5: u16 = (hued_color16 >> 10) & 0x1F;
        let hg5: u16 = (hued_color16 >> 5) & 0x1F;
        let hb5: u16 = hued_color16 & 0x1F;

        let hr8: u16 = hr5 * 8;
        let hg8: u16 = hg5 * 8;
        let hb8: u16 = hb5 * 8;

        (a << 24) | ((hr8 as u32) << 16) | ((hg8 as u32) << 8) | (hb8 as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hue() -> HueEntry {
        let mut color_table = [0u16; 32];
        for (index, color) in color_table.iter_mut().enumerate() {
            let r = index as u16;
            let g = (31 - index) as u16;
            let b = (index / 2) as u16;
            *color = (r << 10) | (g << 5) | b;
        }
        HueEntry {
            id: 0,
            color_table,
            table_start: color_table[0],
            table_end: color_table[31],
            name: [0; 20],
        }
    }

    #[test]
    fn hue_application_uses_red_channel_index_for_16_bit_colors() {
        let hue = test_hue();
        let source = 0x8000 | (12 << 10) | (4 << 5) | 4;

        assert_eq!(hue.apply_to_color16(source, false), hue.color_table[12] | 0x8000);
    }

    #[test]
    fn hue_application_uses_red_channel_index_for_32_bit_colors() {
        let hue = test_hue();
        let source = 0xCC_60_20_20;
        let expected = hue.color_table[12];
        let r = ((expected >> 10) & 0x1F) as u32 * 8;
        let g = ((expected >> 5) & 0x1F) as u32 * 8;
        let b = (expected & 0x1F) as u32 * 8;

        assert_eq!(hue.apply_to_color32(source, false), 0xCC00_0000 | (r << 16) | (g << 8) | b);
    }
}
