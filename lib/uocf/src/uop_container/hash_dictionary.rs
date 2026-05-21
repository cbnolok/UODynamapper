use std::collections::{btree_map::Entry, BTreeMap};
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
        match self.entries.entry(hash) {
            Entry::Vacant(entry) => {
                entry.insert(None);
                true
            }
            Entry::Occupied(_) => false,
        }
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
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn hash_dictionary_reads_reference_dic_bytes() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"DIC\0");
        bytes.push(1);
        bytes.extend_from_slice(&0x818a15b51c6f3601u64.to_le_bytes());
        bytes.push(1);
        bytes.push(26);
        bytes.extend_from_slice(b"build/sectors/waypoint.bin");
        bytes.extend_from_slice(&0x947f5dd64c557dd7u64.to_le_bytes());
        bytes.push(0);

        let parsed = HashDictionary::from_bytes(&bytes).expect("parse DIC dictionary");

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.named_len(), 1);
        assert_eq!(parsed.resolve(0x818a15b51c6f3601), Some("build/sectors/waypoint.bin"));
        assert!(parsed.contains(0x947f5dd64c557dd7));
        assert_eq!(parsed.resolve(0x947f5dd64c557dd7), None);
    }

    #[test]
    fn hash_dictionary_writes_reference_dic_bytes() {
        let mut dictionary = HashDictionary::new();
        dictionary.set(0x818a15b51c6f3601, "build/sectors/waypoint.bin");
        dictionary.insert_unknown(0x947f5dd64c557dd7);

        let bytes = dictionary.to_bytes().expect("serialize DIC dictionary");

        let mut expected = Vec::new();
        expected.extend_from_slice(b"DIC\0");
        expected.push(1);
        expected.extend_from_slice(&0x818a15b51c6f3601u64.to_le_bytes());
        expected.push(1);
        expected.push(26);
        expected.extend_from_slice(b"build/sectors/waypoint.bin");
        expected.extend_from_slice(&0x947f5dd64c557dd7u64.to_le_bytes());
        expected.push(0);

        assert_eq!(bytes, expected);
    }

    #[test]
    fn hash_dictionary_roundtrips_long_dotnet_strings() {
        let mut dictionary = HashDictionary::new();
        let long_name = format!("build/{}", "a".repeat(130));
        dictionary.set(0x70efc7b90345eaab, &long_name);

        let bytes = dictionary.to_bytes().expect("serialize DIC dictionary");
        let parsed = HashDictionary::from_bytes(&bytes).expect("parse DIC dictionary");

        assert_eq!(parsed.resolve(0x70efc7b90345eaab), Some(long_name.as_str()));
    }

    #[test]
    fn hash_dictionary_save_and_load_preserves_entries() {
        let path = temp_dic_path("save_load");
        let mut dictionary = HashDictionary::new();
        dictionary.set(1001, "first_entry");
        dictionary.insert_unknown(2002);

        dictionary.save(&path).expect("save DIC dictionary");
        let loaded = HashDictionary::load(&path).expect("load DIC dictionary");

        let _ = fs::remove_file(path);

        assert_eq!(loaded.resolve(1001), Some("first_entry"));
        assert!(loaded.contains(2002));
        assert_eq!(loaded.resolve(2002), None);
    }

    #[test]
    fn hash_dictionary_updates_unknowns_without_overwriting_names() {
        let mut dictionary = HashDictionary::new();
        assert!(dictionary.insert_unknown(1));
        assert!(dictionary.set(1, "filled"));
        assert!(!dictionary.set(1, "ignored"));
        assert!(!dictionary.insert_unknown(1));

        assert_eq!(dictionary.resolve(1), Some("filled"));
    }

    #[test]
    fn hash_dictionary_merges_using_reference_semantics() {
        let mut base = HashDictionary::new();
        base.insert_unknown(1);
        base.set(2, "existing");
        base.set(4, "same");

        let mut incoming = HashDictionary::new();
        incoming.set(1, "filled");
        incoming.set(2, "ignored");
        incoming.set(3, "new");
        incoming.set(4, "same");
        incoming.insert_unknown(5);

        let report = base.merge(incoming);

        assert_eq!(base.resolve(1), Some("filled"));
        assert_eq!(base.resolve(2), Some("existing"));
        assert_eq!(base.resolve(3), Some("new"));
        assert_eq!(base.resolve(4), Some("same"));
        assert!(base.contains(5));
        assert_eq!(base.resolve(5), None);
        assert_eq!(report.new_hashes, 2);
        assert_eq!(report.new_file_names, 2);
        assert_eq!(report.duplicate_hashes, 3);
    }

    fn temp_dic_path(test_name: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("uocf_hash_dictionary_{test_name}_{timestamp}.dic"))
    }
}
