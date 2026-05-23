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

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};
    use std::io::Write;

    fn supported_file_ids() -> &'static [(i32, VerFileId)] {
        &[
            (0, VerFileId::Map),
            (1, VerFileId::StaIdx),
            (2, VerFileId::Statics),
            (3, VerFileId::ArtIdx),
            (4, VerFileId::Art),
            (5, VerFileId::AnimIdx),
            (6, VerFileId::Anim),
            (7, VerFileId::SoundIdx),
            (8, VerFileId::Sound),
            (9, VerFileId::TexIdx),
            (10, VerFileId::Texmaps),
            (11, VerFileId::GumpIdx),
            (12, VerFileId::Gumpart),
            (13, VerFileId::MultiIdx),
            (14, VerFileId::Multi),
            (15, VerFileId::SkillsIdx),
            (16, VerFileId::Skills),
            (22, VerFileId::LightIdx),
            (23, VerFileId::Light),
            (30, VerFileId::Tiledata),
            (31, VerFileId::Animdata),
        ]
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "uocf_verdata_test_{}_{}_{}",
            name,
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ))
    }

    fn write_verdata(path: &Path, records: &[(i32, i32, i32, i32, i32)], payload: &[u8]) -> eyre::Result<()> {
        let mut bytes = Vec::new();
        bytes.write_i32::<LittleEndian>(records.len() as i32)?;
        for &(file_id, index, lookup, length, extra) in records {
            bytes.write_i32::<LittleEndian>(file_id)?;
            bytes.write_i32::<LittleEndian>(index)?;
            bytes.write_i32::<LittleEndian>(lookup)?;
            bytes.write_i32::<LittleEndian>(length)?;
            bytes.write_i32::<LittleEndian>(extra)?;
        }
        bytes.write_all(payload)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    #[test]
    fn file_id_mapping_covers_every_supported_verdata_target() {
        for &(raw, file_id) in supported_file_ids() {
            assert_eq!(VerFileId::from_i32(raw), Some(file_id));
        }

        assert_eq!(VerFileId::from_i32(-1), None);
        assert_eq!(VerFileId::from_i32(17), None);
        assert_eq!(VerFileId::from_i32(32), None);
    }

    #[test]
    fn load_indexes_and_reads_patches_for_all_supported_file_ids() {
        let path = temp_path("all_ids.mul");
        let header_len = 4 + (supported_file_ids().len() + 1) * 20;
        let mut records = Vec::new();
        let mut payload = Vec::new();

        for (pos, &(raw, _file_id)) in supported_file_ids().iter().enumerate() {
            let data = [raw as u8, pos as u8, 0xA0 | pos as u8];
            records.push((
                raw,
                10_000 + pos as i32,
                (header_len + payload.len()) as i32,
                data.len() as i32,
                20_000 + pos as i32,
            ));
            payload.extend_from_slice(&data);
        }
        records.push((99, 123, (header_len + payload.len()) as i32, 2, 7));
        payload.extend_from_slice(&[0xFE, 0xED]);

        write_verdata(&path, &records, &payload).unwrap();
        let result = verify_all_supported_file_ids(&path);
        let _ = std::fs::remove_file(&path);

        result.unwrap();
    }

    fn verify_all_supported_file_ids(path: &Path) -> eyre::Result<()> {
        let verdata = Verdata::load(path)?;
        assert_eq!(verdata.len(), supported_file_ids().len());
        assert!(!verdata.is_empty());

        for (pos, &(raw, file_id)) in supported_file_ids().iter().enumerate() {
            let index = 10_000 + pos as i32;
            let entry = verdata.entry(file_id, index).unwrap();
            assert_eq!(entry.file_id, raw);
            assert_eq!(entry.index, index);
            assert_eq!(entry.length, 3);
            assert_eq!(entry.extra, 20_000 + pos as i32);
            assert_eq!(
                verdata.read_patch(file_id, index)?.unwrap(),
                vec![raw as u8, pos as u8, 0xA0 | pos as u8]
            );
            assert_eq!(
                verdata.index_patch(file_id, index),
                Some((entry.lookup as u32, entry.length as u32, entry.extra as u32))
            );
        }

        assert!(verdata.entry(VerFileId::Map, 123).is_none());
        assert!(verdata.read_patch(VerFileId::Map, 123)?.is_none());
        Ok(())
    }

    #[test]
    fn entries_for_returns_only_the_requested_file_id() {
        let path = temp_path("entries_for.mul");
        let header_len = 4 + 3 * 20;
        write_verdata(
            &path,
            &[
                (4, 7, header_len as i32, 1, 0),
                (4, 9, header_len as i32 + 1, 1, 0),
                (8, 7, header_len as i32 + 2, 1, 0),
            ],
            &[1, 2, 3],
        )
        .unwrap();

        let result = (|| -> eyre::Result<()> {
            let verdata = Verdata::load(&path)?;
            let mut art_indices = verdata
                .entries_for(VerFileId::Art)
                .map(|(index, _entry)| *index)
                .collect::<Vec<_>>();
            art_indices.sort_unstable();
            assert_eq!(art_indices, vec![7, 9]);
            assert_eq!(verdata.entries_for(VerFileId::Sound).count(), 1);
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);

        result.unwrap();
    }

    #[test]
    fn duplicate_entries_keep_the_last_patch() {
        let path = temp_path("duplicate.mul");
        let header_len = 4 + 2 * 20;
        write_verdata(
            &path,
            &[
                (4, 1, header_len as i32, 1, 11),
                (4, 1, header_len as i32 + 1, 2, 22),
            ],
            &[0x11, 0x22, 0x33],
        )
        .unwrap();

        let result = (|| -> eyre::Result<()> {
            let verdata = Verdata::load(&path)?;
            assert_eq!(verdata.len(), 1);
            assert_eq!(verdata.read_patch(VerFileId::Art, 1)?.unwrap(), vec![0x22, 0x33]);
            assert_eq!(verdata.entry(VerFileId::Art, 1).unwrap().extra, 22);
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);

        result.unwrap();
    }

    #[test]
    fn malformed_headers_are_rejected() {
        let negative_count = temp_path("negative_count.mul");
        std::fs::write(&negative_count, (-1i32).to_le_bytes()).unwrap();
        assert!(Verdata::load(&negative_count).is_err());
        let _ = std::fs::remove_file(&negative_count);

        let negative_lookup = temp_path("negative_lookup.mul");
        write_verdata(&negative_lookup, &[(4, 1, -1, 1, 0)], &[]).unwrap();
        assert!(Verdata::load(&negative_lookup).is_err());
        let _ = std::fs::remove_file(&negative_lookup);

        let negative_length = temp_path("negative_length.mul");
        write_verdata(&negative_length, &[(4, 1, 4, -1, 0)], &[]).unwrap();
        assert!(Verdata::load(&negative_length).is_err());
        let _ = std::fs::remove_file(&negative_length);
    }

    #[test]
    fn patch_read_reports_truncated_payload() {
        let path = temp_path("truncated_payload.mul");
        write_verdata(&path, &[(4, 1, 24, 4, 0)], &[0xAA]).unwrap();
        let result = (|| -> eyre::Result<()> {
            let verdata = Verdata::load(&path)?;
            assert!(verdata.read_patch(VerFileId::Art, 1).is_err());
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);

        result.unwrap();
    }
}
