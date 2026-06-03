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
            let text = decode_cliloc_text(&bytes[start..start + len]);
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

fn decode_cliloc_text(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if bytes.starts_with(&[0xFF, 0xFE]) && bytes.len() % 2 == 0 {
        return decode_utf16_bytes(&bytes[2..], LittleEndianMarker);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() % 2 == 0 {
        return decode_utf16_bytes(&bytes[2..], BigEndianMarker);
    }
    if looks_like_utf16le_text(bytes) {
        return decode_utf16_bytes(bytes, LittleEndianMarker);
    }

    String::from_utf8_lossy(bytes).into_owned()
}

struct LittleEndianMarker;
struct BigEndianMarker;

trait Utf16Endian {
    fn read(bytes: [u8; 2]) -> u16;
}

impl Utf16Endian for LittleEndianMarker {
    fn read(bytes: [u8; 2]) -> u16 {
        u16::from_le_bytes(bytes)
    }
}

impl Utf16Endian for BigEndianMarker {
    fn read(bytes: [u8; 2]) -> u16 {
        u16::from_be_bytes(bytes)
    }
}

fn decode_utf16_bytes<E: Utf16Endian>(bytes: &[u8], _endian: E) -> String {
    let words = bytes
        .chunks_exact(2)
        .map(|chunk| E::read([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&words)
}

fn looks_like_utf16le_text(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }

    let units = bytes.len() / 2;
    let odd_zero_bytes = bytes.chunks_exact(2).filter(|chunk| chunk[1] == 0).count();
    if odd_zero_bytes * 3 >= units {
        return utf16le_decodes_as_text(bytes);
    }

    let raw_control_bytes = bytes
        .iter()
        .filter(|byte| matches!(byte, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F))
        .count();
    if raw_control_bytes * 2 >= units && utf16le_decodes_as_text(bytes) {
        return true;
    }

    if std::str::from_utf8(bytes).is_err() && utf16le_script_char_count(bytes) >= 2 {
        return utf16le_decodes_as_text(bytes);
    }

    false
}

fn utf16le_decodes_as_text(bytes: &[u8]) -> bool {
    let mut replacements = 0usize;
    let mut disallowed_controls = 0usize;

    for item in std::char::decode_utf16(
        bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])),
    ) {
        match item {
            Ok('\t' | '\n' | '\r') => {}
            Ok(ch) if ch.is_control() => disallowed_controls += 1,
            Ok(_) => {}
            Err(_) => replacements += 1,
        }
    }

    replacements == 0 && disallowed_controls == 0
}

fn utf16le_script_char_count(bytes: &[u8]) -> usize {
    std::char::decode_utf16(
        bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])),
    )
    .filter_map(Result::ok)
    .filter(|ch| {
        matches!(
            *ch as u32,
            0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF
        )
    })
    .count()
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

    fn cliloc_payload_with_text(text: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(1).unwrap();
        bytes.write_i16::<LittleEndian>(2).unwrap();
        bytes.write_i32::<LittleEndian>(100).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_i16::<LittleEndian>(text.len() as i16).unwrap();
        bytes.extend_from_slice(text);
        bytes
    }

    fn utf16le_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|word| word.to_le_bytes())
            .collect()
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

    #[test]
    fn parses_utf8_cliloc_text() {
        let cliloc = Cliloc::from_payload_bytes(&cliloc_payload_with_text("café grève".as_bytes()))
            .unwrap();

        assert_eq!(cliloc.get(100), Some("café grève"));
    }

    #[test]
    fn parses_utf16le_cliloc_text_with_ascii_range_characters() {
        let cliloc = Cliloc::from_payload_bytes(&cliloc_payload_with_text(&utf16le_bytes("hello")))
            .unwrap();

        assert_eq!(cliloc.get(100), Some("hello"));
    }

    #[test]
    fn parses_utf16le_cliloc_text_with_non_latin_characters() {
        let cliloc = Cliloc::from_payload_bytes(&cliloc_payload_with_text(&utf16le_bytes("銀行取引")))
            .unwrap();

        assert_eq!(cliloc.get(100), Some("銀行取引"));
    }
}
