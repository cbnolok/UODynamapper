use std::fs;
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

pub const ANIMDATA_RECORDS_PER_CHUNK: usize = 8;
pub const ANIMDATA_FRAME_COUNT: usize = 64;
pub const ANIMDATA_RECORD_SIZE: usize = ANIMDATA_FRAME_COUNT + 4;
pub const ANIMDATA_CHUNK_SIZE: usize = 4 + ANIMDATA_RECORDS_PER_CHUNK * ANIMDATA_RECORD_SIZE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimDataEntry {
    pub id: u32,
    pub chunk_header: i32,
    pub frames: [i8; ANIMDATA_FRAME_COUNT],
    pub unknown: u8,
    pub frame_count: u8,
    pub frame_interval: u8,
    pub frame_start: u8,
}

impl AnimDataEntry {
    pub fn is_active(&self) -> bool {
        self.frame_count > 0 && self.frame_interval > 0
    }

    pub fn frame_offset(&self, index: usize) -> Option<i8> {
        if index >= self.frame_count as usize || index >= self.frames.len() {
            return None;
        }
        Some(self.frames[index])
    }

    pub fn frame_tile_id(&self, index: usize) -> Option<i32> {
        self.frame_offset(index)
            .map(|offset| self.id as i32 + offset as i32)
    }
}

#[derive(Debug, Clone)]
pub struct AnimData {
    pub entries: Vec<AnimDataEntry>,
    pub chunk_headers: Vec<i32>,
    pub trailing_bytes: Vec<u8>,
}

impl AnimData {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let bytes = fs::read(path.as_ref())
            .wrap_err_with(|| format!("failed to read {}", path.as_ref().display()))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let chunk_count = bytes.len() / ANIMDATA_CHUNK_SIZE;
        let mut reader = std::io::Cursor::new(bytes);
        let mut entries = Vec::with_capacity(chunk_count * ANIMDATA_RECORDS_PER_CHUNK);
        let mut chunk_headers = Vec::with_capacity(chunk_count);
        let mut id = 0u32;

        for _ in 0..chunk_count {
            let chunk_header = reader.read_i32::<LittleEndian>()?;
            chunk_headers.push(chunk_header);

            for _ in 0..ANIMDATA_RECORDS_PER_CHUNK {
                let mut frames = [0i8; ANIMDATA_FRAME_COUNT];
                for frame in &mut frames {
                    *frame = reader.read_i8()?;
                }
                entries.push(AnimDataEntry {
                    id,
                    chunk_header,
                    frames,
                    unknown: reader.read_u8()?,
                    frame_count: reader.read_u8()?,
                    frame_interval: reader.read_u8()?,
                    frame_start: reader.read_u8()?,
                });
                id += 1;
            }
        }

        let trailing_start = reader.position() as usize;
        let trailing_bytes = bytes[trailing_start..].to_vec();
        Ok(Self {
            entries,
            chunk_headers,
            trailing_bytes,
        })
    }

    pub fn get(&self, id: u32) -> Option<&AnimDataEntry> {
        self.entries.get(id as usize)
    }

    pub fn active_entries(&self) -> impl Iterator<Item = &AnimDataEntry> {
        self.entries.iter().filter(|entry| entry.is_active())
    }

    pub fn active_count(&self) -> usize {
        self.active_entries().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    #[test]
    fn parses_animdata_chunk_records_and_trailing_bytes() {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(0x11223344).unwrap();
        for record in 0..ANIMDATA_RECORDS_PER_CHUNK {
            for frame in 0..ANIMDATA_FRAME_COUNT {
                bytes.write_i8(if record == 1 { frame as i8 - 2 } else { 0 }).unwrap();
            }
            bytes.write_u8(9).unwrap();
            bytes.write_u8(if record == 1 { 3 } else { 0 }).unwrap();
            bytes.write_u8(if record == 1 { 5 } else { 0 }).unwrap();
            bytes.write_u8(7).unwrap();
        }
        bytes.extend_from_slice(b"tail");

        let animdata = AnimData::from_bytes(&bytes).unwrap();

        assert_eq!(animdata.chunk_headers, vec![0x11223344]);
        assert_eq!(animdata.entries.len(), 8);
        assert_eq!(animdata.trailing_bytes, b"tail");
        assert_eq!(animdata.active_count(), 1);
        assert_eq!(animdata.get(1).unwrap().frame_tile_id(0), Some(-1));
        assert_eq!(animdata.get(1).unwrap().frame_tile_id(2), Some(1));
        assert_eq!(animdata.get(1).unwrap().frame_tile_id(3), None);
    }
}
