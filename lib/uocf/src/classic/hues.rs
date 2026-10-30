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

/// Colours of one hue's table.
pub const HUE_COLOR_COUNT: usize = 32;
/// Bytes of a hue name.
pub const HUE_NAME_LEN: usize = 20;
/// Hue entries in one group of `hues.mul`.
pub const HUES_PER_GROUP: usize = HUES_PER_BLOCK;
/// Bytes of one group: the `u32` header, then `HUES_PER_GROUP` entries.
pub const HUE_GROUP_SIZE: usize = HUE_BLOCK_SIZE;
const HUE_GROUP_HEADER_SIZE: usize = 4;
const HUE_COLOR_SIZE: usize = 2;
const HUE_TABLE_START_OFFSET: usize = HUE_COLOR_COUNT * HUE_COLOR_SIZE;
const HUE_TABLE_END_OFFSET: usize = HUE_TABLE_START_OFFSET + HUE_COLOR_SIZE;
const HUE_NAME_OFFSET: usize = HUE_TABLE_END_OFFSET + HUE_COLOR_SIZE;

/// Represents a single hue entry from hues.mul, containing a color table and metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// One group of `hues.mul`: a `u32` header and `HUES_PER_GROUP` entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HueGroup {
    pub header: u32,
    pub entries: [HueEntry; HUES_PER_GROUP],
}

/// A whole `hues.mul`, kept bit for bit: every group header and entry field (colour bit 15 included) and the bytes
/// after the last whole group, so [`HuesMul::to_bytes`] gives back the parsed file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuesMul {
    pub groups: Vec<HueGroup>,
    /// Bytes after the last whole group (none in client files).
    pub trailing: Vec<u8>,
}

impl HueEntry {
    /// An all-zero entry for hue `id`.
    pub fn empty(id: u32) -> Self {
        Self {
            id,
            color_table: [0; HUE_COLOR_COUNT],
            table_start: 0,
            table_end: 0,
            name: [0; HUE_NAME_LEN],
        }
    }

    fn parse(id: u32, bytes: &[u8]) -> Self {
        let read_u16 = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        let mut color_table = [0u16; HUE_COLOR_COUNT];
        for (index, color) in color_table.iter_mut().enumerate() {
            *color = read_u16(index * HUE_COLOR_SIZE);
        }
        let mut name = [0u8; HUE_NAME_LEN];
        name.copy_from_slice(&bytes[HUE_NAME_OFFSET..HUE_NAME_OFFSET + HUE_NAME_LEN]);
        Self {
            id,
            color_table,
            table_start: read_u16(HUE_TABLE_START_OFFSET),
            table_end: read_u16(HUE_TABLE_END_OFFSET),
            name,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        for color in self.color_table {
            out.extend_from_slice(&color.to_le_bytes());
        }
        out.extend_from_slice(&self.table_start.to_le_bytes());
        out.extend_from_slice(&self.table_end.to_le_bytes());
        out.extend_from_slice(&self.name);
    }

    /// The name up to its first NUL byte.
    pub fn name_bytes(&self) -> &[u8] {
        let end = self.name.iter().position(|&byte| byte == 0).unwrap_or(HUE_NAME_LEN);
        &self.name[..end]
    }
}

impl HuesMul {
    /// `group_count` all-zero groups (hues `1..=group_count * HUES_PER_GROUP`).
    pub fn with_group_count(group_count: usize) -> Self {
        let groups = (0..group_count)
            .map(|group| HueGroup {
                header: 0,
                entries: std::array::from_fn(|entry| HueEntry::empty((group * HUES_PER_GROUP + entry + 1) as u32)),
            })
            .collect();
        Self { groups, trailing: Vec::new() }
    }

    /// Parses a `hues.mul`. Every byte is kept, so parsing cannot fail.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut groups = Vec::with_capacity(bytes.len() / HUE_GROUP_SIZE);
        let mut chunks = bytes.chunks_exact(HUE_GROUP_SIZE);
        for (group_index, chunk) in chunks.by_ref().enumerate() {
            let header = u32::from_le_bytes(chunk[..HUE_GROUP_HEADER_SIZE].try_into().expect("4-byte header"));
            let entries = std::array::from_fn(|entry_index| {
                let offset = HUE_GROUP_HEADER_SIZE + entry_index * HUE_ENTRY_SIZE;
                let id = (group_index * HUES_PER_GROUP + entry_index + 1) as u32;
                HueEntry::parse(id, &chunk[offset..offset + HUE_ENTRY_SIZE])
            });
            groups.push(HueGroup { header, entries });
        }
        Self { groups, trailing: chunks.remainder().to_vec() }
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Ok(Self::parse(&bytes))
    }

    /// The file bytes: the groups in order, then the trailing bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.groups.len() * HUE_GROUP_SIZE + self.trailing.len());
        for group in &self.groups {
            out.extend_from_slice(&group.header.to_le_bytes());
            for entry in &group.entries {
                entry.write(&mut out);
            }
        }
        out.extend_from_slice(&self.trailing);
        out
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        std::fs::write(path, self.to_bytes()).wrap_err_with(|| format!("failed to write {}", path.display()))
    }

    /// Number of hues (`HUES_PER_GROUP` per whole group).
    pub fn hue_count(&self) -> usize {
        self.groups.len() * HUES_PER_GROUP
    }

    /// Every hue in id order; hue ids start at 1.
    pub fn hues(&self) -> impl Iterator<Item = &HueEntry> {
        self.groups.iter().flat_map(|group| group.entries.iter())
    }

    /// Hue `id` (1-based).
    pub fn hue(&self, id: u32) -> Option<&HueEntry> {
        let index = (id as usize).checked_sub(1)?;
        self.groups.get(index / HUES_PER_GROUP).map(|group| &group.entries[index % HUES_PER_GROUP])
    }

    pub fn hue_mut(&mut self, id: u32) -> Option<&mut HueEntry> {
        let index = (id as usize).checked_sub(1)?;
        self.groups.get_mut(index / HUES_PER_GROUP).map(|group| &mut group.entries[index % HUES_PER_GROUP])
    }
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
    fn hues_mul_round_trips_every_byte() {
        let partial_group = HUE_GROUP_SIZE / 2;
        let bytes: Vec<u8> = (0..2 * HUE_GROUP_SIZE + partial_group).map(|index| (index * 7 + 3) as u8).collect();
        let hues = HuesMul::parse(&bytes);
        assert_eq!(hues.groups.len(), 2);
        assert_eq!(hues.trailing.len(), partial_group);
        assert_eq!(hues.hue_count(), 2 * HUES_PER_GROUP);
        assert_eq!(hues.to_bytes(), bytes);

        let second = hues.hue(2).expect("hue 2");
        assert_eq!(second.id, 2);
        let offset = HUE_GROUP_HEADER_SIZE + HUE_ENTRY_SIZE;
        assert_eq!(second.color_table[0], u16::from_le_bytes([bytes[offset], bytes[offset + 1]]));
        assert_eq!(hues.hue(0), None);
        assert_eq!(hues.hue(2 * HUES_PER_GROUP as u32 + 1), None);
    }

    #[test]
    fn hue_name_bytes_stop_at_the_first_nul() {
        let mut hue = HueEntry::empty(1);
        hue.name[..3].copy_from_slice(b"Red");
        hue.name[5] = b'x';
        assert_eq!(hue.name_bytes(), b"Red");
        hue.name = [b'a'; HUE_NAME_LEN];
        assert_eq!(hue.name_bytes().len(), HUE_NAME_LEN);
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
