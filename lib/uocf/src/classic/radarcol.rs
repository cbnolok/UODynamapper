//! # UO Radar Color Parser (`radarcol.mul`)
//!
//! This module handles the loading of the `radarcol.mul` file, which contains the color palette
//! for the in-game radar map.
//!
//! ## File Format
//!
//! `radarcol.mul` is a simple file containing a sequence of 16-bit color values in RGB555 format.
//! Each color corresponds to a land tile type and is used to render the radar map.

use std::fs;
use std::io::Cursor;
use std::path::Path;
use byteorder::{LittleEndian, ReadBytesExt};
use crate::utils::color::Rgb555;

crate::eyre_imports!();

/// Loads the radarcol.mul file into a vector of 16-bit color values (RGB555).
pub fn load_radarcol(path: &Path) -> eyre::Result<Vec<Rgb555>> {
    let file_data: Vec<u8> = fs::read(path)?;
    let mut cursor: Cursor<Vec<u8>> = Cursor::new(file_data);
    let mut colors: Vec<Rgb555> = Vec::with_capacity(cursor.get_ref().len() / 2);
    while let Ok(color) = cursor.read_u16::<LittleEndian>() {
        colors.push(Rgb555::new_from_val(color));
    }
    Ok(colors)
}

/// The `radarcol.mul` bytes of `colors`, the inverse of [`load_radarcol`] (a file of an even length comes back
/// byte for byte).
pub fn encode_radarcol(colors: &[Rgb555]) -> Vec<u8> {
    colors.iter().flat_map(|color| color.value().to_le_bytes()).collect()
}

pub fn save_radarcol(path: &Path, colors: &[Rgb555]) -> eyre::Result<()> {
    fs::write(path, encode_radarcol(colors)).wrap_err_with(|| format!("Write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radarcol_round_trips_every_byte() {
        let bytes = [0x00, 0x00, 0xFF, 0x7F, 0x34, 0x12, 0x01, 0x80];
        let path = std::env::temp_dir().join(format!("uocf_radarcol_{}.mul", std::process::id()));
        fs::write(&path, bytes).unwrap();
        let colors = load_radarcol(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(colors.len(), bytes.len() / std::mem::size_of::<u16>());
        assert_eq!(encode_radarcol(&colors), bytes);
    }
}
