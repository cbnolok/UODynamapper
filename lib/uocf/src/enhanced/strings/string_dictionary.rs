//! Represents a string dictionary from a UOP file.
//!
//! Payload layout (`build/stringdictionary/string_dictionary.bin`), as the client's `UOStringDictionary::Load` reads it
//! (`docs/client_research/ghidra-tileart/results/uosa_tileart.md`):
//! - `u64 unk1` (looks like a FILETIME)
//! - `u32 strings_count`
//! - `u16 max_length`: the length of the longest string, in bytes
//! - `strings_count` strings, each `u16 length` and `length` UTF-8 bytes.
//!
//! String 0 is empty, and other files name a string by its index directly: index 0 names no string. The Enhanced
//! Client's 2024 dictionary holds 110856 strings (longest 72 bytes), Kingdom Reborn's 30221 (longest 59). A header of
//! a `u32` after the count (UOETE's reading) takes string 0's length into it and shifts every string down by one.
//!
//! The model keeps the header as read, so [`UoStringDictionary::to_payload_bytes`] writes the payload back byte for
//! byte.

use std::{io::Read, path::Path};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crate::uop_container::package::UopPackage;
crate::eyre_imports!();

/// Size of the payload header: `u64 unk1`, `u32 strings_count`, `u16 max_length`.
const PAYLOAD_HEADER_SIZE: usize = 14;

/// The index of the empty string, the one an index of no string names.
pub const EMPTY_STRING_INDEX: usize = 0;

/// Represents a string dictionary from a UOP file.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct UoStringDictionary {
    /// Unknown value.
    unk1: u64,
    /// The number of strings the payload header declares, which may differ from `strings.len()`.
    strings_count: u32,
    /// The length of the longest string the payload header declares, in bytes.
    max_length: u16,
    /// The strings in the dictionary, the empty string 0 first.
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

    /// A dictionary of the empty string 0 and `strings` (at indices 1 on), declaring their count and longest length.
    pub fn from_strings<S: Into<String>>(strings: impl IntoIterator<Item = S>) -> Self {
        let strings: Vec<String> = std::iter::once(String::new()).chain(strings.into_iter().map(Into::into)).collect();
        let max_length = strings.iter().map(String::len).max().unwrap_or(0);
        UoStringDictionary {
            unk1: 0,
            strings_count: u32::try_from(strings.len()).unwrap_or(u32::MAX),
            max_length: u16::try_from(max_length).unwrap_or(u16::MAX),
            strings,
        }
    }

    pub fn from_payload_bytes(bytes: &[u8], source_label: &str) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);

        let unk1: u64 = reader.read_u64::<LittleEndian>()?;
        let declared_strings_count: u32 = reader.read_u32::<LittleEndian>()?;
        let max_length: u16 = reader.read_u16::<LittleEndian>()?;

        // Allocate string vector capacities in advance
        let mut strings: Vec<String> = Vec::with_capacity(declared_strings_count as usize);
        let mut string_buffer = Vec::new();
        let mut count_eof_mismatch = false;

        // Every string up to the end of the payload, also past the declared count, so the payload is written back
        // whole.
        while (reader.position() as usize) < reader.get_ref().len() {
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

        if strings.len() < declared_strings_count as usize {
            count_eof_mismatch = true;
        } else if strings.len() > declared_strings_count as usize {
            log::warn!(
                "uocf: string dictionary '{}' declared {} strings but holds {}",
                source_label,
                declared_strings_count,
                strings.len(),
            );
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
            max_length,
            strings,
        })
    }

    /// The payload bytes: the header with the declared count and the longest length (the declared one, or a longer
    /// string's), then every string. Gives back the bytes [`Self::from_payload_bytes`] parsed. Fails for a string
    /// longer than `u16::MAX` bytes.
    pub fn to_payload_bytes(&self) -> eyre::Result<Vec<u8>> {
        let strings_length: usize =
            self.strings.iter().map(|string| std::mem::size_of::<u16>() + string.len()).sum();
        let mut bytes = Vec::with_capacity(PAYLOAD_HEADER_SIZE + strings_length);
        bytes.write_u64::<LittleEndian>(self.unk1)?;
        bytes.write_u32::<LittleEndian>(self.strings_count)?;
        let lengths = self.strings.iter().enumerate().map(|(index, string)| u16::try_from(string.len()).map_err(|_| {
            eyre::eyre!("string {index} is {} bytes long; the dictionary stores at most {}", string.len(), u16::MAX)
        })).collect::<eyre::Result<Vec<u16>>>()?;
        bytes.write_u16::<LittleEndian>(lengths.iter().copied().fold(self.max_length, u16::max))?;
        for (length, string) in lengths.into_iter().zip(&self.strings) {
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

    /// Replaces the string at `string_index`; false when there is none, or for the empty string 0 that an index of no
    /// string names. Other files name a string by its index, so strings keep their places and none can be removed.
    pub fn set_string(&mut self, string_index: usize, string: String) -> bool {
        if string_index == EMPTY_STRING_INDEX {
            return false;
        }
        match self.strings.get_mut(string_index) {
            Some(slot) => {
                *slot = string;
                true
            }
            None => false,
        }
    }

    /// Returns a string from the dictionary by its index, as other files name it: index 0 is the empty string.
    ///
    /// # Arguments
    ///
    /// * `string_index` - The index of the string to return.
    pub fn get_string(&self, string_index: usize) -> Option<&str> {
        self.strings.get(string_index).map(|s| s.as_str())
    }

    /// The string `string_index` names in another file (a tileart record, a terrain or effect definition): `None` for
    /// index 0, which names no string, and past the end.
    pub fn named_string(&self, string_index: u32) -> Option<&str> {
        let string_index = usize::try_from(string_index).ok().filter(|&index| index != EMPTY_STRING_INDEX)?;
        self.get_string(string_index)
    }

    pub fn unk1(&self) -> u64 {
        self.unk1
    }

    /// The length of the longest string the payload header declares, in bytes.
    pub fn max_length(&self) -> u16 {
        self.max_length
    }

    /// The number of stored strings, the empty string 0 included.
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

    /// The declared longest length of the test payloads.
    const MAX_LENGTH: u16 = 0x48;

    /// A payload declaring `declared` strings and holding the empty string 0 and `values`.
    fn payload_declaring(declared: u32, values: &[&str]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x1122334455667788u64.to_le_bytes());
        payload.extend_from_slice(&declared.to_le_bytes());
        payload.extend_from_slice(&MAX_LENGTH.to_le_bytes());
        for value in std::iter::once(&"").chain(values) {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }
        payload
    }

    fn payload_with_strings(values: &[&str]) -> Vec<u8> {
        payload_declaring(values.len() as u32 + 1, values)
    }

    #[test]
    fn reads_the_header_and_indexes_strings_directly() {
        let dictionary =
            UoStringDictionary::from_payload_bytes(&payload_with_strings(&["Data\\WorldArt\\1.tga", "foo"]), "test")
                .unwrap();

        assert_eq!(dictionary.unk1(), 0x1122334455667788);
        assert_eq!(dictionary.max_length(), MAX_LENGTH);
        assert_eq!(dictionary.len(), 3);
        assert_eq!(dictionary.declared_count(), 3);
        assert_eq!(dictionary.get_string(EMPTY_STRING_INDEX), Some(""));
        assert_eq!(dictionary.get_string(2), Some("foo"));
        assert_eq!(
            dictionary.iter().collect::<Vec<_>>(),
            vec![(0, ""), (1, "Data\\WorldArt\\1.tga"), (2, "foo")]
        );
    }

    #[test]
    fn keeps_the_declared_count_and_writes_the_payload_back() {
        let declared_with_one_missing = 4u32;
        let payload = payload_declaring(declared_with_one_missing, &["alpha", "beta"]);
        let mut dictionary = UoStringDictionary::from_payload_bytes(&payload, "test").unwrap();
        assert_eq!(dictionary.len(), 3);
        assert_eq!(dictionary.declared_count(), declared_with_one_missing);
        assert_eq!(dictionary.to_payload_bytes().unwrap(), payload);

        assert!(!dictionary.set_string(3, "none".to_owned()));
        assert!(!dictionary.set_string(EMPTY_STRING_INDEX, "none".to_owned()));
        assert!(dictionary.set_string(2, "BETA".to_owned()));
        assert_eq!(dictionary.get_string(2), Some("BETA"));
        assert!(dictionary.set_string(2, "beta".to_owned()));

        assert_eq!(dictionary.push_string("gamma".to_owned()), 3);
        assert_eq!(dictionary.declared_count(), declared_with_one_missing + 1);
        let reparsed = UoStringDictionary::from_payload_bytes(&dictionary.to_payload_bytes().unwrap(), "test").unwrap();
        assert_eq!(reparsed.iter().collect::<Vec<_>>(), vec![(0, ""), (1, "alpha"), (2, "beta"), (3, "gamma")]);
        assert_eq!(reparsed.declared_count(), declared_with_one_missing + 1);
    }

    #[test]
    fn keeps_strings_past_the_declared_count() {
        let payload = payload_declaring(2, &["alpha", "beta"]);
        let dictionary = UoStringDictionary::from_payload_bytes(&payload, "test").unwrap();
        assert_eq!((dictionary.len(), dictionary.get_string(2)), (3, Some("beta")));
        assert_eq!(dictionary.to_payload_bytes().unwrap(), payload);
    }

    #[test]
    fn a_longer_string_raises_the_declared_longest_length() {
        let mut dictionary = UoStringDictionary::from_payload_bytes(&payload_with_strings(&["alpha"]), "test").unwrap();
        let longer = "x".repeat(usize::from(MAX_LENGTH) + 1);
        dictionary.push_string(longer.clone());
        let reparsed = UoStringDictionary::from_payload_bytes(&dictionary.to_payload_bytes().unwrap(), "test").unwrap();
        assert_eq!(usize::from(reparsed.max_length()), longer.len());
    }

    #[test]
    fn rejects_strings_too_long_to_store() {
        let mut dictionary = UoStringDictionary::from_payload_bytes(&payload_with_strings(&[]), "test").unwrap();
        dictionary.push_string("x".repeat(usize::from(u16::MAX) + 1));
        assert!(dictionary.to_payload_bytes().is_err());
    }
}
