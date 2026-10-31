//! `AnimationDefinition.uop` as a whole: its entries by item id, entry edits, the package writer and the KDL
//! files of one entry each.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};

use super::kdl_text::{animation_definitions_to_kdl, load_animation_definitions_kdl};
use super::AnimationDefinition;
use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::string_dictionary::UoStringDictionary;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::package::UopPackage;

/// File name of the package in the client directory.
pub const ANIMATION_DEFINITION_UOP_NAME: &str = "AnimationDefinition.uop";
/// Item ids keyed like a mobile use this base plus the body (one client entry, 10000400).
pub const MOBILE_ANIMATION_DEFINITION_BASE: u32 = 10_000_000;
/// Item ids (16-bit tile ids) and mobile-keyed ids. The entry names of a package are resolved by hashing these;
/// the writer refuses other ids, whose entries it could not tell apart from foreign ones.
pub const ANIMATION_DEFINITION_ID_RANGES: [Range<u32>; 2] =
    [0..ITEM_ID_COUNT, MOBILE_ANIMATION_DEFINITION_BASE..MOBILE_ANIMATION_DEFINITION_BASE + ITEM_ID_COUNT];
const ITEM_ID_COUNT: u32 = 0x1_0000;
/// File name extension of the KDL files [`AnimationDefinitionCollection::export_kdl_files`] writes and
/// [`AnimationDefinitionCollection::import_kdl_paths`] reads from a folder.
pub const ANIMATION_DEFINITION_KDL_EXTENSION: &str = "kdl";
/// Compression of entries added to a package without entries (client entries are zlib-compressed).
const DEFAULT_ENTRY_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// The file name of the KDL file of item `item_id`: `<item id, 8 digits>.kdl`, like the entry name.
pub fn animation_definition_kdl_file_name(item_id: u32) -> String {
    format!("{item_id:08}.{ANIMATION_DEFINITION_KDL_EXTENSION}")
}

/// Every entry of `AnimationDefinition.uop` by item id, in the dialect of its client.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationDefinitionCollection {
    pub entries: BTreeMap<u32, AnimationDefinition>,
    /// The effect grammar of the package, used when writing it back.
    pub dialect: ClientDialect,
    /// Hashes of sized entries no id of [`ANIMATION_DEFINITION_ID_RANGES`] names (no client entry). They are not
    /// read and the writer keeps them as they are.
    pub unnamed_hashes: Vec<u64>,
}

/// What [`AnimationDefinitionCollection::apply_to_package`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationDefinitionChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

/// What [`AnimationDefinitionCollection::import_kdl_paths`] read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnimationDefinitionImport {
    /// The KDL files read, in reading order.
    pub files: Vec<PathBuf>,
    /// Item ids whose entry was replaced, then the ones added.
    pub replaced: Vec<u32>,
    pub added: Vec<u32>,
}

impl AnimationDefinitionCollection {
    /// An empty collection of `dialect`.
    pub fn new(dialect: ClientDialect) -> Self {
        Self { dialect, ..Self::default() }
    }

    /// Whether the writer can name the entry of `item_id` ([`ANIMATION_DEFINITION_ID_RANGES`]).
    pub fn is_valid_id(item_id: u32) -> bool {
        ANIMATION_DEFINITION_ID_RANGES.iter().any(|range| range.contains(&item_id))
    }

    /// Reads every entry of a package in the effect grammar of `dialect`. Fails on an entry that does not parse.
    pub fn from_package_dialect(package: &UopPackage, dialect: ClientDialect) -> eyre::Result<Self> {
        let ids_by_hash = ids_by_hash();
        let mut collection = Self::new(dialect);
        for file in package.iter_files().filter(|file| file.has_size()) {
            let hash = file.filename_hash();
            let Some(&item_id) = ids_by_hash.get(&hash) else {
                collection.unnamed_hashes.push(hash);
                continue;
            };
            let entry_path = AnimationDefinition::entry_path(item_id);
            let payload = file.unpack_arc().wrap_err_with(|| format!("failed to unpack {entry_path}"))?;
            let definition = AnimationDefinition::parse_dialect(&payload, dialect)
                .wrap_err_with(|| format!("failed to parse {entry_path}"))?;
            collection.entries.insert(item_id, definition);
        }
        Ok(collection)
    }

    /// [`Self::from_package_dialect`] for the Enhanced Client.
    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        Self::from_package_dialect(package, ClientDialect::Enhanced)
    }

    /// Reads a package file; the dialect is the one of its folder ([`ClientDialect::of_package_path`]: Kingdom
    /// Reborn next to `UOKR.exe`).
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        Self::load_dialect(path, ClientDialect::of_package_path(path))
    }

    /// Reads a package file in `dialect`.
    pub fn load_dialect(path: impl AsRef<Path>, dialect: ClientDialect) -> eyre::Result<Self> {
        let path = path.as_ref();
        let package = UopPackage::load(path).wrap_err_with(|| format!("failed to load {}", path.display()))?;
        Self::from_package_dialect(&package, dialect).wrap_err_with(|| format!("failed to read {}", path.display()))
    }

    pub fn get(&self, item_id: u32) -> Option<&AnimationDefinition> {
        self.entries.get(&item_id)
    }

    pub fn get_mut(&mut self, item_id: u32) -> Option<&mut AnimationDefinition> {
        self.entries.get_mut(&item_id)
    }

    /// Sets the definition of `item_id`; returns the previous one.
    pub fn set(&mut self, item_id: u32, definition: AnimationDefinition) -> Option<AnimationDefinition> {
        self.entries.insert(item_id, definition)
    }

    /// Removes the entry of `item_id`; returns its definition.
    pub fn remove(&mut self, item_id: u32) -> Option<AnimationDefinition> {
        self.entries.remove(&item_id)
    }

    /// The item ids, ascending.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }

    /// Makes the named entries of `package` exactly these entries, in [`Self::dialect`]. Entries whose payload is
    /// unchanged are left untouched (same compressed bytes and data block header); changed ones are replaced
    /// keeping their compression flag; new ones are added with the compression of the existing entries. Both get
    /// a fresh timestamp data block header (the package's edit time). Named entries missing from the collection
    /// are removed; unnamed ones are kept. Fails before changing anything on an id outside
    /// [`ANIMATION_DEFINITION_ID_RANGES`] or an entry the writer refuses.
    pub fn apply_to_package(&self, package: &mut UopPackage) -> eyre::Result<AnimationDefinitionChanges> {
        let mut payloads = Vec::with_capacity(self.entries.len());
        for (&item_id, definition) in &self.entries {
            eyre::ensure!(
                Self::is_valid_id(item_id),
                "item {item_id} is outside the animation definition ids ({ANIMATION_DEFINITION_ID_RANGES:?})"
            );
            let payload = definition
                .to_bytes_dialect(self.dialect)
                .wrap_err_with(|| format!("failed to encode {}", AnimationDefinition::entry_path(item_id)))?;
            payloads.push((AnimationDefinition::entry_hash(item_id), item_id, payload));
        }

        let mut changes = AnimationDefinitionChanges::default();
        let compression = package
            .iter_files()
            .find(|file| file.has_size())
            .map_or(DEFAULT_ENTRY_COMPRESSION, |file| file.compression());
        for (hash, item_id, payload) in &payloads {
            let current = package
                .unpack_file_by_hash(*hash)
                .wrap_err_with(|| format!("failed to unpack {}", AnimationDefinition::entry_path(*item_id)))?;
            if current.as_deref() == Some(payload.as_slice()) {
                continue;
            }
            match uop_entries::put_entry(package, *hash, payload, compression)? {
                EntryPut::Replaced => changes.replaced += 1,
                EntryPut::Added => changes.added += 1,
            }
        }
        let ids_by_hash = ids_by_hash();
        let stale = package
            .iter_files()
            .filter(|file| file.has_size())
            .filter_map(|file| ids_by_hash.get(&file.filename_hash()).map(|&item_id| (file.filename_hash(), item_id)))
            .filter(|(_, item_id)| !self.entries.contains_key(item_id))
            .map(|(hash, _)| hash)
            .collect::<Vec<_>>();
        for hash in stale {
            uop_entries::remove_entry(package, hash);
            changes.removed += 1;
        }
        Ok(changes)
    }

    /// Writes a copy of the `source` `AnimationDefinition.uop` to `output` with these entries
    /// ([`Self::apply_to_package`]); the package keeps its version and block size. `output` must not be `source`.
    pub fn write_package(
        &self,
        source: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> eyre::Result<AnimationDefinitionChanges> {
        let (source, output) = (source.as_ref(), output.as_ref());
        uop_entries::refuse_to_overwrite(source, output)?;
        let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
        let changes = self.apply_to_package(&mut package)?;
        uop_entries::save_package_as(&mut package, source, output)?;
        Ok(changes)
    }

    /// Every entry as KDL in one text ([`animation_definitions_to_kdl`]).
    pub fn to_kdl_string(&self, strings: Option<&UoStringDictionary>) -> String {
        animation_definitions_to_kdl(self.entries.iter().map(|(&item_id, definition)| (item_id, definition)), strings)
    }

    /// Writes the KDL file of entry `item_id` to `path`.
    pub fn export_kdl_file(&self, item_id: u32, path: impl AsRef<Path>, strings: Option<&UoStringDictionary>) -> eyre::Result<()> {
        let path = path.as_ref();
        let definition = self.get(item_id).ok_or_else(|| eyre::eyre!("no animation definition for item {item_id}"))?;
        std::fs::write(path, animation_definitions_to_kdl([(item_id, definition)], strings))
            .wrap_err_with(|| format!("failed to write {}", path.display()))
    }

    /// Writes one KDL file per entry into `dir` (created when missing), named
    /// [`animation_definition_kdl_file_name`]; returns how many.
    pub fn export_kdl_files(&self, dir: impl AsRef<Path>, strings: Option<&UoStringDictionary>) -> eyre::Result<usize> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        for &item_id in self.entries.keys() {
            self.export_kdl_file(item_id, dir.join(animation_definition_kdl_file_name(item_id)), strings)?;
        }
        Ok(self.entries.len())
    }

    /// Reads KDL files and sets every entry they hold. A path is a KDL file or a folder, whose `.kdl` files are
    /// read in name order. Reads every file before changing anything; fails on a file that does not parse, an id
    /// outside [`ANIMATION_DEFINITION_ID_RANGES`] and an item given by two files.
    pub fn import_kdl_paths(
        &mut self,
        paths: &[PathBuf],
        strings: Option<&UoStringDictionary>,
    ) -> eyre::Result<AnimationDefinitionImport> {
        let files = kdl_files(paths)?;
        let mut read: BTreeMap<u32, (AnimationDefinition, &Path)> = BTreeMap::new();
        for file in &files {
            for (item_id, definition) in load_animation_definitions_kdl(file, strings)? {
                eyre::ensure!(
                    Self::is_valid_id(item_id),
                    "{}: item {item_id} is outside the animation definition ids ({ANIMATION_DEFINITION_ID_RANGES:?})",
                    file.display()
                );
                if let Some((_, first)) = read.insert(item_id, (definition, file)) {
                    eyre::bail!("item {item_id} is in both {} and {}", first.display(), file.display());
                }
            }
        }
        let mut import = AnimationDefinitionImport::default();
        let read: Vec<_> = read.into_iter().map(|(item_id, (definition, _))| (item_id, definition)).collect();
        for (item_id, definition) in read {
            match self.set(item_id, definition) {
                Some(_) => import.replaced.push(item_id),
                None => import.added.push(item_id),
            }
        }
        import.files = files;
        Ok(import)
    }
}

/// The KDL files of `paths`: files as given, folders expanded to their `.kdl` files in name order.
fn kdl_files(paths: &[PathBuf]) -> eyre::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if !path.is_dir() {
            files.push(path.clone());
            continue;
        }
        let mut found = std::fs::read_dir(path)
            .wrap_err_with(|| format!("failed to list {}", path.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .wrap_err_with(|| format!("failed to list {}", path.display()))?;
        found.retain(|file| {
            file.is_file()
                && file
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(ANIMATION_DEFINITION_KDL_EXTENSION))
        });
        found.sort();
        eyre::ensure!(!found.is_empty(), "{} holds no .{ANIMATION_DEFINITION_KDL_EXTENSION} files", path.display());
        files.extend(found);
    }
    Ok(files)
}

fn ids_by_hash() -> HashMap<u64, u32> {
    ANIMATION_DEFINITION_ID_RANGES
        .iter()
        .flat_map(Clone::clone)
        .map(|item_id| (AnimationDefinition::entry_hash(item_id), item_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{DefinitionFrame, FrameSet};
    use super::*;
    use crate::uop_container::hash::hash_file_name_single;

    const UNNAMED_PATH: &str = "build/animationdefinition/unknown.bin";
    const MOBILE_ID: u32 = MOBILE_ANIMATION_DEFINITION_BASE + 400;

    fn definition(tile_id: u32) -> AnimationDefinition {
        let frames = vec![DefinitionFrame { tile_id, effects: Vec::new() }];
        let sets = BTreeMap::from([(0, FrameSet { next: Some(0), frame_interval: 0.25, frames })]);
        AnimationDefinition { stored_id: tile_id, start_set: 0, second_set: 0, value: 0.0, sets }
    }

    fn package(ids: &[u32]) -> UopPackage {
        let mut package = UopPackage::new_default();
        for &id in ids {
            let payload = definition(id).to_bytes().unwrap();
            package.add_file_from_memory(&payload, &AnimationDefinition::entry_path(id), CompressionFlag::Zlib).unwrap();
        }
        let unnamed = definition(1).to_bytes().unwrap();
        package.add_file_from_memory(&unnamed, UNNAMED_PATH, CompressionFlag::Zlib).unwrap();
        package
    }

    #[test]
    fn edits_and_applies_to_a_package() {
        let mut package = package(&[1, 2, 3, MOBILE_ID]);
        let mut collection = AnimationDefinitionCollection::from_package(&package).unwrap();
        assert_eq!(collection.ids().collect::<Vec<_>>(), [1, 2, 3, MOBILE_ID]);
        assert_eq!(collection.unnamed_hashes, [hash_file_name_single(UNNAMED_PATH)]);

        assert_eq!(collection.apply_to_package(&mut package).unwrap(), AnimationDefinitionChanges::default());
        collection.get_mut(2).unwrap().sets.get_mut(&0).unwrap().frame_interval = 0.5;
        assert_eq!(collection.set(4, definition(4)), None);
        assert!(collection.remove(3).is_some());
        let changes = collection.apply_to_package(&mut package).unwrap();
        assert_eq!(changes, AnimationDefinitionChanges { replaced: 1, added: 1, removed: 1 });
        assert_eq!(AnimationDefinitionCollection::from_package(&package).unwrap(), collection);
        assert!(package.get_file_by_hash(hash_file_name_single(UNNAMED_PATH)).is_some(), "unnamed entries are kept");
    }

    #[test]
    fn refuses_invalid_entries_before_changing_anything() {
        let mut package = package(&[1]);
        let mut collection = AnimationDefinitionCollection::from_package(&package).unwrap();
        collection.set(2, definition(2));
        collection.set(ITEM_ID_COUNT, definition(5));
        assert!(collection.apply_to_package(&mut package).unwrap_err().to_string().contains("outside"));
        collection.remove(ITEM_ID_COUNT);
        collection.get_mut(1).unwrap().start_set = 9;
        let error = collection.apply_to_package(&mut package).unwrap_err();
        assert!(format!("{error:#}").contains("start frame set 9"), "{error:#}");
        let unchanged = AnimationDefinitionCollection::from_package(&package).unwrap();
        assert_eq!(unchanged.ids().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn exports_and_imports_kdl_files() {
        let dir = std::env::temp_dir().join(format!("uocf_animdef_collection_{}", std::process::id()));
        let collection = AnimationDefinitionCollection::from_package(&package(&[1, 2, MOBILE_ID])).unwrap();
        assert_eq!(collection.export_kdl_files(&dir, None).unwrap(), 3);
        assert!(dir.join("10000400.kdl").is_file());

        let mut imported = AnimationDefinitionCollection::new(ClientDialect::Enhanced);
        imported.set(1, definition(9));
        let import = imported.import_kdl_paths(&[dir.clone()], None).unwrap();
        assert_eq!(import.files.len(), 3);
        assert_eq!((import.replaced, import.added), (vec![1], vec![2, MOBILE_ID]));
        assert_eq!(imported.entries, collection.entries);

        let twice = imported.import_kdl_paths(&[dir.join("00000001.kdl"), dir.join("00000001.kdl")], None);
        assert!(twice.unwrap_err().to_string().contains("item 1 is in both"));
        std::fs::write(dir.join("00000003.kdl"), "animation 3 {\n    set 0 interval=1 next=7\n}\n").unwrap();
        let error = imported.import_kdl_paths(&[dir.clone()], None).unwrap_err().to_string();
        assert!(error.contains("00000003.kdl: line 2, column 27: frame set 0: next 7"), "{error}");
        assert_eq!(imported.entries, collection.entries, "a failed import changes nothing");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
