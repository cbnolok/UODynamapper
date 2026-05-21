use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

crate::eyre_imports!();

const MAGIC: &[u8; 4] = b"DIC\0";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashDictionary {
    entries: BTreeMap<u64, Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashDictionaryMergeReport {
    pub new_hashes: usize,
    pub new_file_names: usize,
    pub duplicate_hashes: usize,
}

impl HashDictionary {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let data = fs::read(path)
            .wrap_err_with(|| format!("failed to read dictionary {}", path.display()))?;
        Self::from_bytes(&data)
            .wrap_err_with(|| format!("failed to parse dictionary {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        fs::write(path, self.to_bytes()?)
            .wrap_err_with(|| format!("failed to write dictionary {}", path.display()))
    }

    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < MAGIC.len() + 1 || &bytes[..MAGIC.len()] != MAGIC {
            eyre::bail!("not a DIC hash dictionary");
        }

        let mut reader = Cursor::new(&bytes[MAGIC.len()..]);
        let version = reader.read_u8()?;
        if version != VERSION {
            eyre::bail!("unsupported DIC hash dictionary version: {}", version);
        }

        let mut dictionary = Self::new();
        while (reader.position() as usize) < reader.get_ref().len() {
            let hash = reader.read_u64::<LittleEndian>()?;
            let name = match reader.read_u8()? {
                0 => None,
                1 => Some(read_dotnet_string(&mut reader)?),
                value => eyre::bail!("invalid DIC name flag: {}", value),
            };
            dictionary.entries.entry(hash).or_insert(name);
        }

        Ok(dictionary)
    }

    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.write_u8(VERSION)?;

        for (hash, name) in &self.entries {
            bytes.write_u64::<LittleEndian>(*hash)?;
            match name {
                Some(name) => {
                    bytes.write_u8(1)?;
                    write_dotnet_string(&mut bytes, name)?;
                }
                None => bytes.write_u8(0)?,
            }
        }

        Ok(bytes)
    }

    pub fn merge(&mut self, other: Self) -> HashDictionaryMergeReport {
        let mut report = HashDictionaryMergeReport {
            new_hashes: 0,
            new_file_names: 0,
            duplicate_hashes: 0,
        };

        for (hash, incoming_name) in other.entries {
            match self.entries.get_mut(&hash) {
                Some(existing_name) => {
                    report.duplicate_hashes += 1;
                    if existing_name.is_none() && incoming_name.is_some() {
                        *existing_name = incoming_name;
                        report.new_file_names += 1;
                    }
                }
                None => {
                    if incoming_name.is_some() {
                        report.new_file_names += 1;
                    }
                    self.entries.insert(hash, incoming_name);
                    report.new_hashes += 1;
                }
            }
        }

        report
    }

    pub fn set(&mut self, hash: u64, name: impl Into<String>) -> bool {
        match self.entries.get_mut(&hash) {
            Some(existing_name) if existing_name.is_none() => {
                *existing_name = Some(name.into());
                true
            }
            Some(_) => false,
            None => {
                self.entries.insert(hash, Some(name.into()));
                true
            }
        }
    }

    pub fn insert_unknown(&mut self, hash: u64) -> bool {
        self.entries.insert(hash, None).is_none()
    }

    pub fn contains(&self, hash: u64) -> bool {
        self.entries.contains_key(&hash)
    }

    pub fn resolve(&self, hash: u64) -> Option<&str> {
        self.entries.get(&hash).and_then(|name| name.as_deref())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn named_len(&self) -> usize {
        self.entries.values().filter(|name| name.is_some()).count()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, Option<&str>)> {
        self.entries
            .iter()
            .map(|(hash, name)| (*hash, name.as_deref()))
    }
}

impl Default for HashDictionary {
    fn default() -> Self {
        Self::new()
    }
}

fn read_dotnet_string(reader: &mut Cursor<&[u8]>) -> eyre::Result<String> {
    let len = read_7bit_encoded_usize(reader)?;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8(bytes)?)
}

fn write_dotnet_string(writer: &mut Vec<u8>, value: &str) -> eyre::Result<()> {
    write_7bit_encoded_usize(writer, value.len())?;
    writer.write_all(value.as_bytes())?;
    Ok(())
}

fn read_7bit_encoded_usize(reader: &mut Cursor<&[u8]>) -> eyre::Result<usize> {
    let mut value = 0usize;
    let mut shift = 0usize;
    loop {
        let byte = reader.read_u8()?;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= usize::BITS as usize {
            eyre::bail!("DIC string length is too large");
        }
    }
}

fn write_7bit_encoded_usize(writer: &mut Vec<u8>, mut value: usize) -> eyre::Result<()> {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        writer.write_u8(byte)?;
        if value == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_dictionary_roundtrips_dic_bytes() {
        let mut dictionary = HashDictionary::new();
        dictionary.set(0x818a15b51c6f3601, "build/sectors/waypoint.bin");
        dictionary.set(0x70efc7b90345eaab, "build/animationdefinition/00000060.bin");
        dictionary.insert_unknown(0x947f5dd64c557dd7);

        let bytes = dictionary.to_bytes().expect("serialize DIC dictionary");
        let parsed = HashDictionary::from_bytes(&bytes).expect("parse DIC dictionary");

        assert_eq!(parsed.resolve(0x818a15b51c6f3601), Some("build/sectors/waypoint.bin"));
        assert_eq!(
            parsed.resolve(0x70efc7b90345eaab),
            Some("build/animationdefinition/00000060.bin")
        );
        assert!(parsed.contains(0x947f5dd64c557dd7));
        assert_eq!(parsed.resolve(0x947f5dd64c557dd7), None);
    }

    #[test]
    fn hash_dictionary_merge_matches_reference_semantics() {
        let mut base = HashDictionary::new();
        base.insert_unknown(1);
        base.set(2, "existing");

        let mut incoming = HashDictionary::new();
        incoming.set(1, "filled");
        incoming.set(2, "ignored");
        incoming.set(3, "new");

        let report = base.merge(incoming);

        assert_eq!(base.resolve(1), Some("filled"));
        assert_eq!(base.resolve(2), Some("existing"));
        assert_eq!(base.resolve(3), Some("new"));
        assert_eq!(report.new_hashes, 1);
        assert_eq!(report.new_file_names, 2);
        assert_eq!(report.duplicate_hashes, 2);
    }
}
