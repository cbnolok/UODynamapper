//! Codec for the old Michelangelo/UOAnimTool `.uop` patch format.
//!
//! This is not Mythic/EC UOP. It is the compact patch stream written by
//! UOAnimTool `QuickExport.UOPExport`: magic, entry count, then raw verdata-like
//! animation payload records.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::classic::generic_index::IndexFile;
use crate::classic::vd_codec::VERDATA_FILE_ID_ANIM;

pub const MICHELANGELO_UOP_MAGIC: i32 = 72_372_053;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MichelangeloPatchEntry {
    pub file_id: u8,
    pub index: i32,
    pub extra: i32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MichelangeloPatch {
    pub entries: Vec<MichelangeloPatchEntry>,
}

impl MichelangeloPatchEntry {
    pub fn anim(index: i32, extra: i32, data: Vec<u8>) -> Self {
        Self {
            file_id: VERDATA_FILE_ID_ANIM as u8,
            index,
            extra,
            data,
        }
    }
}

impl MichelangeloPatch {
    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let magic = reader.read_i32::<LittleEndian>()?;
        if magic != MICHELANGELO_UOP_MAGIC {
            eyre::bail!("invalid Michelangelo UOP magic {magic}");
        }

        let entry_count = reader.read_i64::<LittleEndian>()?;
        if entry_count < 0 {
            eyre::bail!("Michelangelo UOP entry count cannot be negative");
        }

        let mut entries = Vec::with_capacity(entry_count as usize);
        for _ in 0..entry_count {
            let file_id = reader.read_u8()?;
            let index = reader.read_i32::<LittleEndian>()?;
            let length = reader.read_i32::<LittleEndian>()?;
            let extra = reader.read_i32::<LittleEndian>()?;
            if length < 0 {
                eyre::bail!("Michelangelo UOP entry length cannot be negative");
            }

            let mut data = vec![0; length as usize];
            reader.read_exact(&mut data)?;
            entries.push(MichelangeloPatchEntry {
                file_id,
                index,
                extra,
                data,
            });
        }

        if reader.position() != bytes.len() as u64 {
            eyre::bail!("Michelangelo UOP has trailing bytes");
        }

        Ok(Self { entries })
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(MICHELANGELO_UOP_MAGIC)?;
        bytes.write_i64::<LittleEndian>(
            i64::try_from(self.entries.len()).wrap_err("too many Michelangelo UOP entries")?,
        )?;

        for entry in &self.entries {
            let length =
                i32::try_from(entry.data.len()).wrap_err("Michelangelo UOP entry too large")?;
            bytes.write_u8(entry.file_id)?;
            bytes.write_i32::<LittleEndian>(entry.index)?;
            bytes.write_i32::<LittleEndian>(length)?;
            bytes.write_i32::<LittleEndian>(entry.extra)?;
            bytes.write_all(&entry.data)?;
        }

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

pub fn anim_block_index(anim_id: i32) -> i32 {
    if anim_id < 200 {
        anim_id * 110
    } else if anim_id < 400 {
        22_000 + ((anim_id - 200) * 65)
    } else {
        35_000 + ((anim_id - 400) * 175)
    }
}

pub fn export_anim_blocks_from_mul(
    idx_path: impl Into<PathBuf>,
    mul_path: impl AsRef<Path>,
    block_ids: &[i32],
    source_anim_id: i32,
    target_anim_id: i32,
) -> eyre::Result<MichelangeloPatch> {
    let index = IndexFile::load(idx_path.into())?;
    let mut mul = File::open(mul_path)?;
    let source_start = anim_block_index(source_anim_id);
    let target_start = anim_block_index(target_anim_id);
    let mut entries = Vec::with_capacity(block_ids.len());

    for &block_id in block_ids {
        if block_id < 0 {
            eyre::bail!("animation block id cannot be negative: {block_id}");
        }
        let element = index.element(block_id as usize)?;
        let lookup = element
            .lookup()
            .ok_or_else(|| eyre!("animation block {block_id} has no lookup"))?;
        let length = element
            .len()
            .ok_or_else(|| eyre!("animation block {block_id} has no length"))?;
        let extra = element
            .extra()
            .ok_or_else(|| eyre!("animation block {block_id} has no extra"))?;

        let mut data = vec![0; length as usize];
        mul.seek(SeekFrom::Start(lookup as u64))?;
        mul.read_exact(&mut data)?;

        let target_block_id = (block_id - source_start) + target_start;
        entries.push(MichelangeloPatchEntry::anim(
            target_block_id,
            extra as i32,
            data,
        ));
    }

    Ok(MichelangeloPatch { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn michelangelo_patch_roundtrip_matches_quickexport_layout() {
        let patch = MichelangeloPatch {
            entries: vec![MichelangeloPatchEntry::anim(77, 9, vec![1, 2, 3])],
        };

        let bytes = patch.to_bytes().unwrap();

        assert_eq!(
            bytes,
            vec![
                0x55, 0x4F, 0x50, 0x04,
                1, 0, 0, 0, 0, 0, 0, 0,
                6,
                77, 0, 0, 0,
                3, 0, 0, 0,
                9, 0, 0, 0,
                1, 2, 3,
            ]
        );
        assert_eq!(MichelangeloPatch::from_bytes(&bytes).unwrap(), patch);
    }

    #[test]
    fn anim_block_index_matches_uoanimtool_mapping() {
        assert_eq!(anim_block_index(0), 0);
        assert_eq!(anim_block_index(199), 21_890);
        assert_eq!(anim_block_index(200), 22_000);
        assert_eq!(anim_block_index(399), 34_935);
        assert_eq!(anim_block_index(400), 35_000);
        assert_eq!(anim_block_index(401), 35_175);
    }

    #[test]
    fn michelangelo_patch_rejects_trailing_bytes() {
        let mut bytes = MichelangeloPatch::default().to_bytes().unwrap();
        bytes.push(0);

        assert!(MichelangeloPatch::from_bytes(&bytes).is_err());
    }
}
