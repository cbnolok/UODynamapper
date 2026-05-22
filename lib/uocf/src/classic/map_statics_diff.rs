//! Parsers for classic map/static diff patch files.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use crate::classic::generic_index::IndexFile;
use crate::classic::map::MapBlock;

pub struct MapDiff {
    lookup_table: HashMap<u32, u32>,
    diff_mmap: Mmap,
}

impl MapDiff {
    pub fn load(lookup_path: impl AsRef<Path>, diff_path: impl AsRef<Path>) -> eyre::Result<Self> {
        let lookup_table = load_lookup_table(lookup_path.as_ref())?;
        let diff_file = File::open(diff_path.as_ref())?;
        let diff_mmap = unsafe { Mmap::map(&diff_file)? };
        Ok(Self {
            lookup_table,
            diff_mmap,
        })
    }

    pub fn raw_block(&self, block_id: u32) -> eyre::Result<Option<&[u8]>> {
        let Some(diff_index) = self.lookup_table.get(&block_id).copied() else {
            return Ok(None);
        };

        let start = diff_index as usize * MapBlock::PACKED_SIZE;
        let end = start + MapBlock::PACKED_SIZE;
        if end > self.diff_mmap.len() {
            eyre::bail!("Map diff block {} points outside mapdif data", block_id);
        }

        Ok(Some(&self.diff_mmap[start..end]))
    }
}

pub struct StaticDiff {
    lookup_table: HashMap<u32, u32>,
    index: IndexFile,
    diff_mmap: Mmap,
}

impl StaticDiff {
    pub fn load(
        lookup_path: impl AsRef<Path>,
        index_path: impl AsRef<Path>,
        diff_path: impl AsRef<Path>,
    ) -> eyre::Result<Self> {
        let lookup_table = load_lookup_table(lookup_path.as_ref())?;
        let index = IndexFile::load(index_path.as_ref().to_path_buf())?;
        let diff_file = File::open(diff_path.as_ref())?;
        let diff_mmap = unsafe { Mmap::map(&diff_file)? };
        Ok(Self {
            lookup_table,
            index,
            diff_mmap,
        })
    }

    pub fn raw_block(&self, block_id: u32) -> eyre::Result<Option<&[u8]>> {
        let Some(diff_index) = self.lookup_table.get(&block_id).copied() else {
            return Ok(None);
        };

        let entry = self.index.element(diff_index as usize)?;
        let (Some(lookup), Some(length)) = (entry.lookup(), entry.len()) else {
            return Ok(None);
        };
        if length == 0 {
            return Ok(None);
        }

        let start = lookup as usize;
        let end = start + length as usize;
        if end > self.diff_mmap.len() {
            eyre::bail!("Static diff block {} points outside stadif data", block_id);
        }

        Ok(Some(&self.diff_mmap[start..end]))
    }
}

fn load_lookup_table(path: &Path) -> eyre::Result<HashMap<u32, u32>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len % 4 != 0 {
        eyre::bail!("Malformed diff lookup table '{}'", path.to_string_lossy());
    }

    let mut table = HashMap::with_capacity((len / 4) as usize);
    for diff_index in 0..(len / 4) as u32 {
        let block_id = file.read_u32::<LittleEndian>()?;
        table.insert(block_id, diff_index);
    }
    Ok(table)
}
