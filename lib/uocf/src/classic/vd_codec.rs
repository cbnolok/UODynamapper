//! Codec for single-entry UO `.vd` patch files.
//!
//! This is the small wrapper used by old animation tools for one `verdata.mul`
//! entry: a 20-byte verdata entry header followed by the raw patched payload.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Read, Write};
use std::path::Path;

pub const VERDATA_FILE_ID_ANIM: i32 = 6;
const VDD_HEADER_LEN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerdataEntry {
    pub file_id: i32,
    pub index: i32,
    pub lookup: i32,
    pub length: i32,
    pub extra: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdFile {
    pub entry: VerdataEntry,
    pub data: Vec<u8>,
}

impl VdFile {
    pub fn new(entry: VerdataEntry, data: Vec<u8>) -> eyre::Result<Self> {
        if entry.length < 0 {
            eyre::bail!(".vd entry length cannot be negative");
        }
        if entry.length as usize != data.len() {
            eyre::bail!(
                ".vd entry length {} does not match payload length {}",
                entry.length,
                data.len()
            );
        }
        Ok(Self { entry, data })
    }

    pub fn for_anim(index: i32, extra: i32, data: Vec<u8>) -> eyre::Result<Self> {
        let length = i32::try_from(data.len()).wrap_err(".vd payload too large")?;
        Self::new(
            VerdataEntry {
                file_id: VERDATA_FILE_ID_ANIM,
                index,
                lookup: 0,
                length,
                extra,
            },
            data,
        )
    }

    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < VDD_HEADER_LEN {
            eyre::bail!(".vd payload too short for header");
        }

        let mut reader = Cursor::new(bytes);
        let entry = VerdataEntry {
            file_id: reader.read_i32::<LittleEndian>()?,
            index: reader.read_i32::<LittleEndian>()?,
            lookup: reader.read_i32::<LittleEndian>()?,
            length: reader.read_i32::<LittleEndian>()?,
            extra: reader.read_i32::<LittleEndian>()?,
        };

        if entry.length < 0 {
            eyre::bail!(".vd entry length cannot be negative");
        }
        let expected_len = VDD_HEADER_LEN + entry.length as usize;
        if bytes.len() != expected_len {
            eyre::bail!(
                ".vd payload length {} does not match header length {}",
                bytes.len() - VDD_HEADER_LEN,
                entry.length
            );
        }

        let mut data = Vec::with_capacity(entry.length as usize);
        reader.read_to_end(&mut data)?;
        Ok(Self { entry, data })
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        if self.entry.length < 0 || self.entry.length as usize != self.data.len() {
            eyre::bail!(
                ".vd entry length {} does not match payload length {}",
                self.entry.length,
                self.data.len()
            );
        }

        let mut bytes = Vec::with_capacity(VDD_HEADER_LEN + self.data.len());
        bytes.write_i32::<LittleEndian>(self.entry.file_id)?;
        bytes.write_i32::<LittleEndian>(self.entry.index)?;
        bytes.write_i32::<LittleEndian>(self.entry.lookup)?;
        bytes.write_i32::<LittleEndian>(self.entry.length)?;
        bytes.write_i32::<LittleEndian>(self.entry.extra)?;
        bytes.write_all(&self.data)?;
        Ok(bytes)
    }

    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> eyre::Result<()> {
        std::fs::write(path, self.to_bytes()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vd_roundtrip_preserves_verdata_header_and_payload() {
        let vd = VdFile::for_anim(123, 456, vec![1, 2, 3]).unwrap();

        let bytes = vd.to_bytes().unwrap();

        assert_eq!(
            bytes,
            vec![
                6, 0, 0, 0,
                123, 0, 0, 0,
                0, 0, 0, 0,
                3, 0, 0, 0,
                200, 1, 0, 0,
                1, 2, 3,
            ]
        );
        assert_eq!(VdFile::from_bytes(&bytes).unwrap(), vd);
    }

    #[test]
    fn vd_rejects_length_mismatch() {
        let mut bytes = VdFile::for_anim(1, 0, vec![1, 2, 3])
            .unwrap()
            .to_bytes()
            .unwrap();
        bytes.pop();

        assert!(VdFile::from_bytes(&bytes).is_err());
    }
}
