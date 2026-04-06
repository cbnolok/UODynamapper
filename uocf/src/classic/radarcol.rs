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
