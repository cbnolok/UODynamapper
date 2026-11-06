//! `speech.mul`: the keywords a server is told the player said, written back byte for byte.
//!
//! Records to the end of the file: a big-endian `u16` keyword id, a big-endian `u16` byte length and that many
//! bytes of UTF-8 text (`*` is a wildcard). Ids repeat (one per phrasing) and are not sorted; some records are
//! empty. UO Gold and Mondain's Legacy have 5,560 records, 7.0.20 6,125, the 2024 client and fp 6,128.

crate::eyre_imports!();
use std::path::Path;

use crate::utils::output_file::write_through_temp;

pub const SPEECH_FILE: &str = "speech.mul";
/// Bytes of a record header: the id and the text length.
const HEADER_BYTES: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechEntry {
    pub id: u16,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeechFile {
    /// In file order.
    pub entries: Vec<SpeechEntry>,
}

impl SpeechFile {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut entries = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let header = bytes.get(at..at + HEADER_BYTES).ok_or_else(|| eyre!("record {} at {at}: a header cut short", entries.len()))?;
            let id = u16::from_be_bytes([header[0], header[1]]);
            let len = usize::from(u16::from_be_bytes([header[2], header[3]]));
            let start = at + HEADER_BYTES;
            let text = bytes
                .get(start..start + len)
                .ok_or_else(|| eyre!("record {} at {at}: {len} bytes of text past the end of the file", entries.len()))?;
            let text = std::str::from_utf8(text).wrap_err_with(|| format!("record {} at {at}: the text is not UTF-8", entries.len()))?;
            entries.push(SpeechEntry { id, text: text.to_owned() });
            at = start + len;
        }
        Ok(Self { entries })
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let len = u16::try_from(entry.text.len())
                .map_err(|_| eyre!("record {index}: {} bytes of text; a record holds up to {}", entry.text.len(), u16::MAX))?;
            bytes.extend_from_slice(&entry.id.to_be_bytes());
            bytes.extend_from_slice(&len.to_be_bytes());
            bytes.extend_from_slice(entry.text.as_bytes());
        }
        Ok(bytes)
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let bytes = self.to_bytes()?;
        write_through_temp(path, |temp| std::fs::write(temp, &bytes).wrap_err_with(|| format!("write {}", temp.display())))
    }

    /// The texts of keyword `id`.
    pub fn texts(&self, id: u16) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(move |entry| entry.id == id).map(|entry| entry.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_back() {
        let bytes = [0, 1, 0, 3, b'*', b'h', b'i', 0, 0, 0, 0, 0, 1, 0, 2, b'y', b'o'];
        let file = SpeechFile::from_bytes(&bytes).unwrap();
        assert_eq!(file.texts(1).collect::<Vec<_>>(), ["*hi", "yo"]);
        assert_eq!(file.entries[1].text, "");
        assert_eq!(file.to_bytes().unwrap(), bytes);
        assert!(SpeechFile::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(SpeechFile::from_bytes(&[0, 1, 0, 1, 0xFF]).is_err());
    }
}
