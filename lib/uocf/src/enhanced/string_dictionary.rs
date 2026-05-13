///! Represents a string dictionary from a UOP file.

use std::{io::Read, path::Path};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::uop_container::package::UopPackage;
crate::eyre_imports!();

/// Represents a string dictionary from a UOP file.
#[allow(dead_code)]
#[derive(Debug, Clone)]
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
        let file = if let Some(file) = package.get_file_by_hash(0) {
            file
        } else {
            let mut candidate_files = package.iter_files().filter(|file| file.has_size());
            let first_file = candidate_files
                .next()
                .ok_or_else(|| eyre::eyre!("File not found in package"))?;

            if let Some(second_file) = candidate_files.next() {
                return Err(eyre::eyre!(
                    "string dictionary package has no hash-0 entry and multiple payloads ({:#018x}, {:#018x}, ...)",
                    first_file.filename_hash(),
                    second_file.filename_hash(),
                ));
            }

            first_file
        };

        // Extracting decompressed payload using zero-copy extraction to a scratch buffer is optimal,
        // but since `unpack()` handles its own z-lib, we take the whole array
        let decompressed_data: Vec<u8> = file.unpack()?;
        Self::from_payload_bytes(&decompressed_data, &path.display().to_string())
    }

    pub fn from_payload_bytes(bytes: &[u8], source_label: &str) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);

        let unk1: u64 = reader.read_u64::<LittleEndian>()?;
        let declared_strings_count: u32 = reader.read_u32::<LittleEndian>()?;
        let unk2: u32 = reader.read_u32::<LittleEndian>()?;

        // Allocate string vector capacities in advance
        let mut strings: Vec<String> = Vec::with_capacity(declared_strings_count as usize);
        let mut string_buffer = Vec::new();
        let mut count_eof_mismatch = false;

        for _ in 0..declared_strings_count {
            if reader.position() as usize >= reader.get_ref().len() {
                count_eof_mismatch = true;
                break;
            }

            let string_len: usize = match reader.read_u16::<LittleEndian>() {
                Ok(value) => value as usize,
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    count_eof_mismatch = true;
                    break;
                }
                Err(error) => return Err(error.into()),
            };

            string_buffer.resize(string_len, 0);
            reader.read_exact(&mut string_buffer)?;
            strings.push(String::from_utf8(string_buffer.clone()).map_err(|e| {
                eyre::eyre!("Invalid UTF-8 sequence in dictionary: {}", e)
            })?);
        }

        if count_eof_mismatch {
            log::warn!(
                "uocf: string dictionary '{}' declared {} strings but reached EOF after {} complete entries",
                source_label,
                declared_strings_count,
                strings.len(),
            );
        }

        Ok(UoStringDictionary {
            unk1,
            strings_count: strings.len() as u32,
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



