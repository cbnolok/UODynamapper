//! # UO Hue Data Parser (`hues.mul`)
//!
//! This module handles the loading and application of hues from the `hues.mul` file.
//! Hues are used in Ultima Online to colorize items and creatures. Each hue contains a
//! color table that maps grayscale intensities to specific colors.
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
//! When a hue is applied to an asset, the grayscale intensity of each pixel in the asset is
//! calculated. This intensity is then used as an index into the hue's color table to determine
//! the new color for that pixel. This allows for a wide range of color variations for a single
//! base asset.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use byteorder::{LittleEndian, ReadBytesExt};

crate::eyre_imports!();

const HUES_MUL_ENTRY_SIZE: usize = 88;

/// Represents a single hue entry from hues.mul, containing a color table and metadata.
#[derive(Debug, Clone, Copy)]
pub struct HueEntry {
    pub color_table: [u16; 32],
    pub table_start: u16,
    pub table_end: u16,
    pub name: [u8; 20],
}

/// Loads the hues.mul file into a vector of HueEntry structs.
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
    let file_data: Vec<u8> = fs::read(path)?;
    let num_entries: usize = file_data.len() / HUES_MUL_ENTRY_SIZE;
    let mut hues: Vec<HueEntry> = Vec::with_capacity(num_entries);
    let mut cursor: Cursor<Vec<u8>> = Cursor::new(file_data);

    for _ in 0..num_entries {
        let mut entry = HueEntry {
            color_table: [0; 32],
            table_start: 0,
            table_end: 0,
            name: [0; 20]
        };
        for i in 0..32 {
            entry.color_table[i] = cursor.read_u16::<LittleEndian>()?;
        }
        entry.table_start = cursor.read_u16::<LittleEndian>()?;
        entry.table_end = cursor.read_u16::<LittleEndian>()?;
        cursor.read_exact(&mut entry.name)?;
        hues.push(entry);
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

        // The color table in hues.mul is a lookup for the grayscale intensity.
        // We find the intensity of the pixel (average of R, G, B) and use that
        // as an index into our 32-entry color table.
        let intensity: u16 = (r5 + g5 + b5) / 3; // Simple average for intensity
        let color_index: u16 = if intensity > 31 { 31 } else { intensity };

        self.color_table[color_index as usize] | 0x8000 // Preserve alpha bit
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

        // Convert to 5-bit components to find intensity, similar to 16-bit version
        let r5: u32 = r8 >> 3;
        let g5: u32 = g8 >> 3;
        let b5: u32 = b8 >> 3;

        let intensity: u32 = (r5 + g5 + b5) / 3;
        let color_index: u32 = if intensity > 31 { 31 } else { intensity };

        let hued_color16: u16 = self.color_table[color_index as usize];

        // Convert the 16-bit hued color back to 32-bit ARGB
        let hr5: u16 = (hued_color16 >> 10) & 0x1F;
        let hg5: u16 = (hued_color16 >> 5) & 0x1F;
        let hb5: u16 = hued_color16 & 0x1F;

        let hr8: u16 = (hr5 << 3) | (hr5 >> 2);
        let hg8: u16 = (hg5 << 3) | (hg5 >> 2);
        let hb8: u16 = (hb5 << 3) | (hb5 >> 2);

        (a << 24) | ((hr8 as u32) << 16) | ((hg8 as u32) << 8) | (hb8 as u32)
    }
}
