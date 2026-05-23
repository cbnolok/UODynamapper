use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::package::UopPackage;

pub const LOCALIZED_STRINGS_UOP_NAME: &str = "localizedstrings.uop";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedStringEntry {
    pub id: u32,
    pub unk: u8,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringTable {
    pub header1: u16,
    pub header2: u32,
    pub entries: Vec<LocalizedStringEntry>,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringsFile {
    pub filename_hash: u64,
    pub byte_len: u32,
    pub strings: LocalizedStringTable,
}

#[derive(Debug, Clone)]
pub struct LocalizedStringsPackage {
    pub files: Vec<LocalizedStringsFile>,
}

impl LocalizedStringsPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut files = Vec::new();
        for file in package.iter_files() {
            if !file.has_size() {
                continue;
            }
            let bytes = file.unpack().wrap_err_with(|| {
                format!("failed to unpack localized strings file {:016X}", file.filename_hash())
            })?;
            let strings = LocalizedStringTable::from_payload_bytes(&bytes).wrap_err_with(|| {
                format!("failed to parse localized strings file {:016X}", file.filename_hash())
            })?;
            files.push(LocalizedStringsFile {
                filename_hash: file.filename_hash(),
                byte_len: file.decompressed_size(),
                strings,
            });
        }
        files.sort_by_key(|file| file.filename_hash);
        Ok(Self { files })
    }

    pub fn len(&self) -> usize {
        self.files.iter().map(|file| file.strings.len()).sum()
    }
}

impl LocalizedStringTable {
    pub fn from_payload_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);
        let header1 = reader.read_u16::<LittleEndian>()?;
        let header2 = reader.read_u32::<LittleEndian>()?;
        let mut entries = Vec::new();

        while (reader.position() as usize) < bytes.len() {
            let remaining = bytes.len() - reader.position() as usize;
            if remaining < 7 {
                eyre::bail!("truncated localized string entry header");
            }

            let id = reader.read_u32::<LittleEndian>()?;
            let unk = reader.read_u8()?;
            let len = reader.read_u16::<LittleEndian>()? as usize;
            if bytes.len() - (reader.position() as usize) < len {
                eyre::bail!("truncated localized string for {id}");
            }

            let start = reader.position() as usize;
            let text = String::from_utf8_lossy(&bytes[start..start + len]).into_owned();
            reader.set_position((start + len) as u64);
            entries.push(LocalizedStringEntry { id, unk, text });
        }

        Ok(Self {
            header1,
            header2,
            entries,
        })
    }

    pub fn get(&self, id: u32) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.text.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};

    #[test]
    fn parses_localized_payload_with_reference_layout() {
        let mut bytes = Vec::new();
        bytes.write_u16::<LittleEndian>(0x1122).unwrap();
        bytes.write_u32::<LittleEndian>(0x33445566).unwrap();
        bytes.write_u32::<LittleEndian>(200).unwrap();
        bytes.write_u8(0xAB).unwrap();
        bytes.write_u16::<LittleEndian>(4).unwrap();
        bytes.extend_from_slice(b"text");

        let strings = LocalizedStringTable::from_payload_bytes(&bytes).unwrap();

        assert_eq!(strings.header1, 0x1122);
        assert_eq!(strings.header2, 0x33445566);
        assert_eq!(strings.len(), 1);
        assert_eq!(strings.entries[0].id, 200);
        assert_eq!(strings.entries[0].unk, 0xAB);
        assert_eq!(strings.get(200), Some("text"));
    }

    #[test]
    fn rejects_truncated_localized_entry() {
        let mut bytes = Vec::new();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(200).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u16::<LittleEndian>(5).unwrap();
        bytes.extend_from_slice(b"abc");

        let error = LocalizedStringTable::from_payload_bytes(&bytes).unwrap_err();
        assert!(error.to_string().contains("truncated localized string for 200"));
    }
}
