//! Decoding for Classic UO multi files (`multi.mul` and `multi.idx`).

crate::eyre_imports!();
use crate::classic::generic_index::IndexFile;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct MultiPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub flags: u32,
}

pub struct MultiMap {
    index: IndexFile,
    mul_path: std::path::PathBuf,
}

impl MultiMap {
    pub fn load(client_path: &Path) -> eyre::Result<Self> {
        let idx_path = client_path.join("multi.idx");
        let mul_path = client_path.join("multi.mul");

        if !idx_path.exists() || !mul_path.exists() {
            eyre::bail!(
                "multi.idx or multi.mul not found in {}",
                client_path.display()
            );
        }

        let index = IndexFile::load(idx_path)?;
        Ok(Self { index, mul_path })
    }

    pub fn get_parts(&self, multi_id: u32) -> eyre::Result<Vec<MultiPart>> {
        let entry = self
            .index
            .element(multi_id as usize)
            .wrap_err_with(|| format!("Multi ID {} not found in index", multi_id))?;

        let lookup = match entry.lookup() {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };
        let length = match entry.len() {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };

        let file = File::open(&self.mul_path)?;
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(lookup as u64))?;

        let count = length as usize / 12; // Each part is 12 bytes
        let mut parts = Vec::with_capacity(count);

        for _ in 0..count {
            let mut buf = [0u8; 12];
            reader.read_exact(&mut buf)?;

            parts.push(MultiPart {
                item_id: u16::from_le_bytes([buf[0], buf[1]]),
                x: i16::from_le_bytes([buf[2], buf[3]]),
                y: i16::from_le_bytes([buf[4], buf[5]]),
                z: i16::from_le_bytes([buf[6], buf[7]]),
                flags: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            });
        }

        Ok(parts)
    }

    pub fn max_id(&self) -> u32 {
        self.index.element_count() as u32
    }
}
