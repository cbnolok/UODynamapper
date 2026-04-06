///! Represents a string dictionary from a UOP file.

use std::{io::Read, path::Path};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::uop::package::UopPackage;
crate::eyre_imports!();

/// Represents a string dictionary from a UOP file.
#[allow(dead_code)]
#[derive(Debug)]
pub struct UoStringDictionary {
    /// Unknown value.
    unk1: u64,
    /// The number of strings in the dictionary.
    strings_count: u32,
    /// Unknown value.
    unk2: u32,
    /// The strings in the dictionary.
    strings: Vec<String>,
}

impl UoStringDictionary {
    /// Loads a `UoStringDictionary` from a UOP file.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the UOP file.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        let file = package.get_file_by_hash(0)
            .ok_or_else(|| eyre::eyre!("File not found in package"))?;

        // Extracting decompressed payload using zero-copy extraction to a scratch buffer is optimal,
        // but since `unpack()` handles its own z-lib, we take the whole array
        let decompressed_data: Vec<u8> = file.unpack()?;
        let mut reader: std::io::Cursor<Vec<u8>> = std::io::Cursor::new(decompressed_data);

        let unk1: u64 = reader.read_u64::<LittleEndian>()?;
        let strings_count: u32 = reader.read_u32::<LittleEndian>()?;
        let unk2: u32 = reader.read_u32::<LittleEndian>()?;

        // Allocate string vector capacities in advance
        let mut strings: Vec<String> = Vec::with_capacity(strings_count as usize);
        let mut string_buffer = Vec::new();
        
        for _ in 0..strings_count {
            let string_len: usize = reader.read_u16::<LittleEndian>()? as usize;
            string_buffer.resize(string_len, 0); // O(N) but safely bounds checked, or can use read_exact with safe take
            reader.read_exact(&mut string_buffer)?;
            strings.push(String::from_utf8(string_buffer.clone()).map_err(|e| {
                eyre::eyre!("Invalid UTF-8 sequence in dictionary: {}", e)
            })?);
        }

        Ok(UoStringDictionary {
            unk1,
            strings_count,
            unk2,
            strings,
        })
    }

    /// Returns a string from the dictionary by its index.
    ///
    /// # Arguments
    ///
    /// * `string_index` - The index of the string to return.
    pub fn get_string(&self, string_index: usize) -> Option<&str> {
        self.strings.get(string_index).map(|s| s.as_str())
    }
}
