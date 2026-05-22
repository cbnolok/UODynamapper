//! Parser for classic `verdata.mul` patch containers.

crate::eyre_imports!();

use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum VerFileId {
    Map = 0,
    StaIdx = 1,
    Statics = 2,
    ArtIdx = 3,
    Art = 4,
    AnimIdx = 5,
    Anim = 6,
    SoundIdx = 7,
    Sound = 8,
    TexIdx = 9,
    Texmaps = 10,
    GumpIdx = 11,
    Gumpart = 12,
    MultiIdx = 13,
    Multi = 14,
    SkillsIdx = 15,
    Skills = 16,
    LightIdx = 22,
    Light = 23,
    Tiledata = 30,
    Animdata = 31,
}

impl VerFileId {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Map),
            1 => Some(Self::StaIdx),
            2 => Some(Self::Statics),
            3 => Some(Self::ArtIdx),
            4 => Some(Self::Art),
            5 => Some(Self::AnimIdx),
            6 => Some(Self::Anim),
            7 => Some(Self::SoundIdx),
            8 => Some(Self::Sound),
            9 => Some(Self::TexIdx),
            10 => Some(Self::Texmaps),
            11 => Some(Self::GumpIdx),
            12 => Some(Self::Gumpart),
            13 => Some(Self::MultiIdx),
            14 => Some(Self::Multi),
            15 => Some(Self::SkillsIdx),
            16 => Some(Self::Skills),
            22 => Some(Self::LightIdx),
            23 => Some(Self::Light),
            30 => Some(Self::Tiledata),
            31 => Some(Self::Animdata),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerdataEntry {
    pub file_id: i32,
    pub index: i32,
    pub lookup: i32,
    pub length: i32,
    pub extra: i32,
}

#[derive(Debug, Clone)]
pub struct Verdata {
    path: PathBuf,
    entries: HashMap<(VerFileId, i32), VerdataEntry>,
}

impl Verdata {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path).wrap_err_with(|| {
            format!("Open verdata file at '{}'", path.to_string_lossy())
        })?;

        let count = file.read_i32::<LittleEndian>()?;
        if count < 0 {
            eyre::bail!("Malformed verdata.mul: negative entry count {}", count);
        }

        let mut entries = HashMap::with_capacity(count as usize);
        for _ in 0..count {
            let file_id = file.read_i32::<LittleEndian>()?;
            let index = file.read_i32::<LittleEndian>()?;
            let lookup = file.read_i32::<LittleEndian>()?;
            let length = file.read_i32::<LittleEndian>()?;
            let extra = file.read_i32::<LittleEndian>()?;

            if lookup < 0 || length < 0 {
                eyre::bail!("Malformed verdata.mul: negative lookup or length");
            }

            if let Some(file_key) = VerFileId::from_i32(file_id) {
                entries.insert(
                    (file_key, index),
                    VerdataEntry {
                        file_id,
                        index,
                        lookup,
                        length,
                        extra,
                    },
                );
            }
        }

        Ok(Self { path, entries })
    }

    pub fn entry(&self, file_id: VerFileId, index: i32) -> Option<&VerdataEntry> {
        self.entries.get(&(file_id, index))
    }

    pub fn entries_for(
        &self,
        file_id: VerFileId,
    ) -> impl Iterator<Item = (&i32, &VerdataEntry)> {
        self.entries
            .iter()
            .filter_map(move |((entry_file_id, index), entry)| {
                if *entry_file_id == file_id {
                    Some((index, entry))
                } else {
                    None
                }
            })
    }

    pub fn read_patch_data(&self, entry: &VerdataEntry) -> eyre::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(entry.lookup as u64))?;
        let mut data = vec![0; entry.length as usize];
        file.read_exact(&mut data)?;
        Ok(data)
    }

    pub fn read_patch(&self, file_id: VerFileId, index: i32) -> eyre::Result<Option<Vec<u8>>> {
        match self.entry(file_id, index) {
            Some(entry) => Ok(Some(self.read_patch_data(entry)?)),
            None => Ok(None),
        }
    }

    pub fn index_patch(&self, file_id: VerFileId, index: i32) -> Option<(u32, u32, u32)> {
        let entry = self.entry(file_id, index)?;
        if entry.lookup < 0 || entry.length < 0 || entry.extra < 0 {
            return None;
        }
        Some((entry.lookup as u32, entry.length as u32, entry.extra as u32))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
