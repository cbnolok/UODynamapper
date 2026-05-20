use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::zlib_bwt_codec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClilocEntry {
    pub number: i32,
    pub flag: u8,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Cliloc {
    pub header1: i32,
    pub header2: i16,
    pub entries: Vec<ClilocEntry>,
    entries_by_number: BTreeMap<i32, usize>,
}

impl Cliloc {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let bytes = fs::read(path.as_ref())
            .wrap_err_with(|| format!("failed to read {}", path.as_ref().display()))?;
        Self::from_file_bytes(&bytes)
    }

    pub fn from_file_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let payload = if bytes.get(3).copied() == Some(0x8E) {
            zlib_bwt_codec::decompress(bytes).wrap_err("failed to decompress compressed cliloc")?
        } else {
            bytes.to_vec()
        };
        Self::from_payload_bytes(&payload)
    }

    pub fn from_payload_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = std::io::Cursor::new(bytes);
        let header1 = reader.read_i32::<LittleEndian>()?;
        let header2 = reader.read_i16::<LittleEndian>()?;
        let mut entries = Vec::new();
        let mut entries_by_number = BTreeMap::new();

        while (reader.position() as usize) < bytes.len() {
            let remaining = bytes.len() - reader.position() as usize;
            if remaining < 7 {
                eyre::bail!("truncated cliloc entry header");
            }

            let number = reader.read_i32::<LittleEndian>()?;
            let flag = reader.read_u8()?;
            let len = reader.read_i16::<LittleEndian>()?;
            if len < 0 {
                eyre::bail!("negative cliloc string length for {number}");
            }
            let len = len as usize;
            if bytes.len() - (reader.position() as usize) < len {
                eyre::bail!("truncated cliloc string for {number}");
            }

            let start = reader.position() as usize;
            let text = String::from_utf8_lossy(&bytes[start..start + len]).into_owned();
            reader.set_position((start + len) as u64);
            entries_by_number.insert(number, entries.len());
            entries.push(ClilocEntry { number, flag, text });
        }

        Ok(Self {
            header1,
            header2,
            entries,
            entries_by_number,
        })
    }

    pub fn get(&self, number: i32) -> Option<&str> {
        self.entries_by_number
            .get(&number)
            .and_then(|index| self.entries.get(*index))
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
    use byteorder::WriteBytesExt;

    fn cliloc_payload() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(1).unwrap();
        bytes.write_i16::<LittleEndian>(2).unwrap();
        bytes.write_i32::<LittleEndian>(100).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_i16::<LittleEndian>(5).unwrap();
        bytes.extend_from_slice(b"hello");
        bytes.write_i32::<LittleEndian>(101).unwrap();
        bytes.write_u8(3).unwrap();
        bytes.write_i16::<LittleEndian>(0).unwrap();
        bytes
    }

    #[test]
    fn parses_classic_cliloc_payload() {
        let cliloc = Cliloc::from_payload_bytes(&cliloc_payload()).unwrap();

        assert_eq!(cliloc.header1, 1);
        assert_eq!(cliloc.header2, 2);
        assert_eq!(cliloc.len(), 2);
        assert_eq!(cliloc.get(100), Some("hello"));
        assert_eq!(cliloc.entries[1].flag, 3);
        assert_eq!(cliloc.get(101), Some(""));
    }
}
