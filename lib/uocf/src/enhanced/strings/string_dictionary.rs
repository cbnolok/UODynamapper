//! Represents a string dictionary from a UOP file.
//!
//! Payload layout (`build/stringdictionary/string_dictionary.bin`):
//! - `u64 unk1` (looks like a FILETIME)
//! - `u32 strings_count`: the declared count
//! - `u32 unk2`
//! - strings, each `u16 length` and `length` UTF-8 bytes, up to the end of the payload.
//!
//! The declared count need not match the stored strings: the reference Enhanced Client declares 110856 and stores
//! 110855. The model keeps both, so [`UoStringDictionary::to_payload_bytes`] writes the payload back byte for byte.

use std::{io::Read, path::Path};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crate::uop_container::package::UopPackage;
crate::eyre_imports!();

/// Size of the payload header: `u64 unk1`, `u32 strings_count`, `u32 unk2`.
const PAYLOAD_HEADER_SIZE: usize = 16;

/// Represents a string dictionary from a UOP file.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct UoStringDictionary {
    /// Unknown value.
    unk1: u64,
    /// The number of strings the payload header declares, which may differ from `strings.len()`.
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

        let decompressed_data = file.unpack_arc()?;
        Self::from_payload_bytes(decompressed_data.as_ref(), &path.display().to_string())
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
            strings_count: declared_strings_count,
            unk2,
            strings,
        })
    }

    /// The payload bytes: the header with the declared count, then every string. Gives back the bytes
    /// [`Self::from_payload_bytes`] parsed. Fails for a string longer than `u16::MAX` bytes.
    pub fn to_payload_bytes(&self) -> eyre::Result<Vec<u8>> {
        let strings_length: usize =
            self.strings.iter().map(|string| std::mem::size_of::<u16>() + string.len()).sum();
        let mut bytes = Vec::with_capacity(PAYLOAD_HEADER_SIZE + strings_length);
        bytes.write_u64::<LittleEndian>(self.unk1)?;
        bytes.write_u32::<LittleEndian>(self.strings_count)?;
        bytes.write_u32::<LittleEndian>(self.unk2)?;
        for (index, string) in self.strings.iter().enumerate() {
            let length = u16::try_from(string.len()).map_err(|_| {
                eyre::eyre!("string {index} is {} bytes long; the dictionary stores at most {}", string.len(), u16::MAX)
            })?;
            bytes.write_u16::<LittleEndian>(length)?;
            bytes.extend_from_slice(string.as_bytes());
        }
        Ok(bytes)
    }

    /// Appends `string` and counts it in the declared count; returns its index.
    pub fn push_string(&mut self, string: String) -> usize {
        self.strings.push(string);
        self.strings_count = self.strings_count.wrapping_add(1);
        self.strings.len() - 1
    }

    /// Replaces the string at `string_index`; false when there is none. Other files name a string by its index, so
    /// strings keep their places and none can be removed.
    pub fn set_string(&mut self, string_index: usize, string: String) -> bool {
        match self.strings.get_mut(string_index) {
            Some(slot) => {
                *slot = string;
                true
            }
            None => false,
        }
    }

    /// Returns a string from the dictionary by its index.
    ///
    /// # Arguments
    ///
    /// * `string_index` - The index of the string to return.
    pub fn get_string(&self, string_index: usize) -> Option<&str> {
        self.strings.get(string_index).map(|s| s.as_str())
    }

    pub fn unk1(&self) -> u64 {
        self.unk1
    }

    pub fn unk2(&self) -> u32 {
        self.unk2
    }

    /// The number of stored strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// The number of strings the payload header declares, which may differ from [`Self::len`].
    pub fn declared_count(&self) -> u32 {
        self.strings_count
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &str)> {
        self.strings
            .iter()
            .enumerate()
            .map(|(index, value)| (index, value.as_str()))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with_strings(values: &[&str]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        payload.extend_from_slice(&(values.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0x99AABBCCu32.to_le_bytes());
        for value in values {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }
        payload
    }

    #[test]
    fn exposes_header_count_and_iterated_strings() {
        let dictionary =
            UoStringDictionary::from_payload_bytes(&payload_with_strings(&["Data\\WorldArt\\1.tga", "foo"]), "test")
                .unwrap();

        assert_eq!(dictionary.unk1(), 0x1122334455667788);
        assert_eq!(dictionary.unk2(), 0x99AABBCC);
        assert_eq!(dictionary.len(), 2);
        assert_eq!(dictionary.declared_count(), 2);
        assert_eq!(dictionary.get_string(1), Some("foo"));
        assert_eq!(
            dictionary.iter().collect::<Vec<_>>(),
            vec![(0, "Data\\WorldArt\\1.tga"), (1, "foo")]
        );
    }

    #[test]
    fn keeps_the_declared_count_and_writes_the_payload_back() {
        let mut payload = payload_with_strings(&["alpha", "beta"]);
        let declared_with_one_missing = 3u32;
        payload[std::mem::size_of::<u64>()..PAYLOAD_HEADER_SIZE - std::mem::size_of::<u32>()]
            .copy_from_slice(&declared_with_one_missing.to_le_bytes());
        let mut dictionary = UoStringDictionary::from_payload_bytes(&payload, "test").unwrap();
        assert_eq!(dictionary.len(), 2);
        assert_eq!(dictionary.declared_count(), declared_with_one_missing);
        assert_eq!(dictionary.to_payload_bytes().unwrap(), payload);

        assert!(!dictionary.set_string(2, "none".to_owned()));
        assert!(dictionary.set_string(1, "BETA".to_owned()));
        assert_eq!(dictionary.get_string(1), Some("BETA"));
        assert!(dictionary.set_string(1, "beta".to_owned()));

        assert_eq!(dictionary.push_string("gamma".to_owned()), 2);
        assert_eq!(dictionary.declared_count(), declared_with_one_missing + 1);
        let reparsed = UoStringDictionary::from_payload_bytes(&dictionary.to_payload_bytes().unwrap(), "test").unwrap();
        assert_eq!(reparsed.iter().collect::<Vec<_>>(), vec![(0, "alpha"), (1, "beta"), (2, "gamma")]);
        assert_eq!(reparsed.declared_count(), declared_with_one_missing + 1);
    }

    #[test]
    fn rejects_strings_too_long_to_store() {
        let mut dictionary = UoStringDictionary::from_payload_bytes(&payload_with_strings(&[]), "test").unwrap();
        dictionary.push_string("x".repeat(usize::from(u16::MAX) + 1));
        assert!(dictionary.to_payload_bytes().is_err());
    }
}
