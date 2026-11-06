//! `EffectDefinitionCollection.uop` as a whole: its entries by collection id, entry edits and the package writer.

use std::collections::HashMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use super::encode::encode_effect_definition_collection_dialect;
use super::{parse_effect_definition_collection_dialect, EffectDefinition};
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

/// File name of the package in the client directory.
pub const EFFECT_DEFINITION_COLLECTION_UOP_NAME: &str = "EffectDefinitionCollection.uop";
/// Directory of the entries in the package.
const ENTRY_PATH_PREFIX: &str = "build/effectdefinitioncollection/";
/// File name extension of the entries.
const ENTRY_PATH_EXTENSION: &str = ".bin";
/// Collection ids tried when resolving entry names. Packets name ids below 65535 (their u16 effect field minus
/// `EFFECT_ID_PACKET_BIAS`); client entries go up to 70006 (ids the client plays by itself). Two client entries
/// hash to no id in this range.
pub const EFFECT_COLLECTION_ID_SCAN_END: u32 = 0x2_0000;
/// Compression of entries added to a package without entries (client entries are zlib-compressed).
const DEFAULT_ENTRY_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// The package path of collection entry `id`: `build/effectdefinitioncollection/%08d.bin`.
pub fn effect_collection_entry_path(id: u32) -> String {
    format!("{ENTRY_PATH_PREFIX}{id:08}{ENTRY_PATH_EXTENSION}")
}

/// The name hash of [`effect_collection_entry_path`].
pub fn effect_collection_entry_hash(id: u32) -> u64 {
    hash_file_name_single(&effect_collection_entry_path(id))
}

/// How an entry is named: by collection id, or by its name hash when no id hashes to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EffectEntryKey {
    Id(u32),
    Hash(u64),
}

impl EffectEntryKey {
    /// The name hash of the entry in the package.
    pub fn hash(self) -> u64 {
        match self {
            Self::Id(id) => effect_collection_entry_hash(id),
            Self::Hash(hash) => hash,
        }
    }

    pub fn id(self) -> Option<u32> {
        match self {
            Self::Id(id) => Some(id),
            Self::Hash(_) => None,
        }
    }
}

/// One entry: the definitions played together for one collection id.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectCollectionEntry {
    pub key: EffectEntryKey,
    pub definitions: Vec<EffectDefinition>,
}

/// Every entry of `EffectDefinitionCollection.uop`, in package order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectDefinitionCollection {
    pub entries: Vec<EffectCollectionEntry>,
}

impl EffectDefinitionCollection {
    /// Reads every entry of a package; entry names are resolved by hashing the ids below
    /// [`EFFECT_COLLECTION_ID_SCAN_END`].
    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        Self::from_package_dialect(package, ClientDialect::Enhanced)
    }

    /// [`Self::from_package`] for a package in the effect grammar of `dialect`.
    pub fn from_package_dialect(package: &UopPackage, dialect: ClientDialect) -> eyre::Result<Self> {
        let ids_by_hash = ids_by_hash();
        let mut entries = Vec::new();
        for file in package.iter_files().filter(|file| file.has_size()) {
            let hash = file.filename_hash();
            let key = ids_by_hash.get(&hash).map_or(EffectEntryKey::Hash(hash), |&id| EffectEntryKey::Id(id));
            let payload = package
                .unpack_file_by_hash(hash)
                .wrap_err_with(|| format!("failed to unpack {}", describe(key)))?
                .unwrap_or_default();
            let definitions = parse_effect_definition_collection_dialect(&payload, dialect)
                .wrap_err_with(|| format!("failed to parse {}", describe(key)))?;
            entries.push(EffectCollectionEntry { key, definitions });
        }
        Ok(Self { entries })
    }

    /// Reads a package file ([`Self::from_package`]).
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let package = UopPackage::load(path).wrap_err_with(|| format!("failed to load {}", path.display()))?;
        Self::from_package(&package).wrap_err_with(|| format!("failed to read {}", path.display()))
    }

    /// The definitions of collection id `id`.
    pub fn get(&self, id: u32) -> Option<&[EffectDefinition]> {
        self.entry(EffectEntryKey::Id(id)).map(|entry| entry.definitions.as_slice())
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Vec<EffectDefinition>> {
        self.entries.iter_mut().find(|entry| entry.key == EffectEntryKey::Id(id)).map(|entry| &mut entry.definitions)
    }

    pub fn entry(&self, key: EffectEntryKey) -> Option<&EffectCollectionEntry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    /// Sets the definitions of `key`, adding the entry at the end when missing; returns the previous ones.
    pub fn set(&mut self, key: EffectEntryKey, definitions: Vec<EffectDefinition>) -> Option<Vec<EffectDefinition>> {
        match self.entries.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => Some(std::mem::replace(&mut entry.definitions, definitions)),
            None => {
                self.entries.push(EffectCollectionEntry { key, definitions });
                None
            }
        }
    }

    /// Removes the entry of `key`; returns its definitions.
    pub fn remove(&mut self, key: EffectEntryKey) -> Option<Vec<EffectDefinition>> {
        let index = self.entries.iter().position(|entry| entry.key == key)?;
        Some(self.entries.remove(index).definitions)
    }

    /// The collection ids, in package order.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().filter_map(|entry| entry.key.id())
    }

    /// Makes `package` hold exactly these entries. Entries whose payload is unchanged are left untouched (same
    /// compressed bytes); changed ones are replaced keeping their compression flag; new ones are added with the
    /// compression of the existing entries. Both get a fresh timestamp data block header (the package's edit
    /// time); entries missing from the collection are removed. Returns the number of entries replaced, added and removed.
    pub fn apply_to_package(&self, package: &mut UopPackage) -> eyre::Result<EffectCollectionChanges> {
        self.apply_to_package_dialect(package, ClientDialect::Enhanced)
    }

    /// [`Self::apply_to_package`] writing the effect grammar of `dialect`.
    pub fn apply_to_package_dialect(
        &self,
        package: &mut UopPackage,
        dialect: ClientDialect,
    ) -> eyre::Result<EffectCollectionChanges> {
        let mut changes = EffectCollectionChanges::default();
        let compression = package
            .iter_files()
            .find(|file| file.has_size())
            .map_or(DEFAULT_ENTRY_COMPRESSION, |file| file.compression());
        let mut seen = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let hash = entry.key.hash();
            if seen.contains(&hash) {
                eyre::bail!("{} appears twice in the collection", describe(entry.key));
            }
            seen.push(hash);
            let payload = encode_effect_definition_collection_dialect(&entry.definitions, dialect)
                .wrap_err_with(|| format!("failed to encode {}", describe(entry.key)))?;
            let current =
                package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack {}", describe(entry.key)))?;
            if current.as_deref() == Some(payload.as_slice()) {
                continue;
            }
            match uop_entries::put_entry(package, hash, &payload, compression)? {
                EntryPut::Replaced => changes.replaced += 1,
                EntryPut::Added => changes.added += 1,
            }
        }
        let stale = package
            .iter_files()
            .filter(|file| file.has_size() && !seen.contains(&file.filename_hash()))
            .map(|file| file.filename_hash())
            .collect::<Vec<_>>();
        for hash in stale {
            uop_entries::remove_entry(package, hash);
            changes.removed += 1;
        }
        Ok(changes)
    }

    /// Writes a copy of the `source` `EffectDefinitionCollection.uop` to `output` with these entries
    /// ([`Self::apply_to_package`]); the package keeps its version and block size. `output` must not be `source`.
    pub fn write_effect_definition_collection(
        &self,
        source: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> eyre::Result<EffectCollectionChanges> {
        let (source, output) = (source.as_ref(), output.as_ref());
        uop_entries::refuse_to_overwrite(source, output)?;
        let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
        let changes = self.apply_to_package(&mut package)?;
        uop_entries::save_package_as(&mut package, source, output)?;
        Ok(changes)
    }
}

/// What [`EffectDefinitionCollection::apply_to_package`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectCollectionChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

fn ids_by_hash() -> HashMap<u64, u32> {
    (0..EFFECT_COLLECTION_ID_SCAN_END).map(|id| (effect_collection_entry_hash(id), id)).collect()
}

fn describe(key: EffectEntryKey) -> String {
    match key {
        EffectEntryKey::Id(id) => effect_collection_entry_path(id),
        EffectEntryKey::Hash(hash) => format!("entry {hash:016X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::effect_definition::encode_effect_definition_collection;
    use crate::enhanced::effect_definition::SoundEffect;

    fn sound(sound_id: u32) -> EffectDefinition {
        EffectDefinition::Sound(SoundEffect { sound_id, value_08: 1.0 })
    }

    #[test]
    fn entry_paths() {
        assert_eq!(effect_collection_entry_path(666), "build/effectdefinitioncollection/00000666.bin");
    }

    #[test]
    fn edits_and_applies_to_a_package() {
        let mut package = UopPackage::new_default();
        for id in [1, 2, 3] {
            let payload = encode_effect_definition_collection(&[sound(id)]);
            package.add_file_from_memory(&payload, &effect_collection_entry_path(id), CompressionFlag::Zlib).unwrap();
        }
        let unknown_hash = hash_file_name_single("build/effectdefinitioncollection/unknown.bin");
        let unknown_payload = encode_effect_definition_collection(&[sound(9)]);
        package.add_file_from_memory(&unknown_payload, "build/effectdefinitioncollection/unknown.bin", CompressionFlag::Zlib).unwrap();

        let mut collection = EffectDefinitionCollection::from_package(&package).unwrap();
        assert_eq!(collection.ids().collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(collection.entries[3].key, EffectEntryKey::Hash(unknown_hash));
        assert_eq!(collection.get(2), Some([sound(2)].as_slice()));

        collection.get_mut(2).unwrap().push(sound(20));
        assert_eq!(collection.set(EffectEntryKey::Id(4), vec![sound(4)]), None);
        assert_eq!(collection.remove(EffectEntryKey::Id(3)), Some(vec![sound(3)]));
        let changes = collection.apply_to_package(&mut package).unwrap();
        assert_eq!(changes, EffectCollectionChanges { replaced: 1, added: 1, removed: 1 });
        assert_eq!(EffectDefinitionCollection::from_package(&package).unwrap(), collection);
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let mut collection = EffectDefinitionCollection::default();
        collection.entries.push(EffectCollectionEntry { key: EffectEntryKey::Id(1), definitions: Vec::new() });
        collection.entries.push(EffectCollectionEntry {
            key: EffectEntryKey::Hash(effect_collection_entry_hash(1)),
            definitions: Vec::new(),
        });
        let mut package = UopPackage::new_default();
        assert!(collection.apply_to_package(&mut package).is_err());
    }
}
