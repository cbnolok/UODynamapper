//! Decoding for EC/CC `MultiCollection.uop` files.
//!
//! Layout and field meanings follow the client parser (`UOMultiCollection::Parse` in UOSA.exe), documented in
//! `scripts/dev-helpers/ec-research/ghidra-housing/results/uosa_housing.md` (section 4). The `housing.bin` entry
//! of the same package is decoded by [`super::housing`].
//!
//! Besides decoding, the module serves an editor: [`MultiItem::to_payload_bytes`] is the exact inverse of the
//! parser, [`MultiCollection::set`] and [`MultiCollection::remove`] add, replace and remove multis, and
//! [`MultiCollection::apply_to_package`] / [`MultiCollection::write_multi_collection`] write them back into a
//! `MultiCollection.uop`, leaving `housing.bin` and unchanged multis untouched.
//!
//! The Kingdom Reborn client (`UOKR.exe` reader 0x007bb4c3; `ClientDialect::KingdomReborn`, the `*_dialect`
//! functions) stores parts without `flag2`: `{u16 tile, i16 x, i16 y, i16 z, u8 not_drawn, u32 n, n x u32}`. That
//! the single flag is `not_drawn` rests on the data (it equals the EC `not_drawn` in all 48,428 parts of the multis
//! whose geometry is the same in both clients) and on its place; its KR readers were not traced
//! (`scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md`, answer 10).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::enhanced::client_dialect::ClientDialect;
use crate::enhanced::uop_entries::{self, EntryPut};
use crate::uop_container::file::CompressionFlag;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const MULTI_COLLECTION_UOP_NAME: &str = "MultiCollection.uop";
pub const HOUSING_PATH: &str = "build/multicollection/housing.bin";
/// Exclusive bound of the multi ids searched in the package: multi ids are 16-bit graphics. Recent clients
/// (2026) ship multi 9000, above the 0x2200 bound used before.
pub const MAX_MULTI_COLLECTION_ID: u32 = 0x10000;
/// Bytes of a part before its string list: tile id, x, y, z, `not_drawn`, `flag2` and the string count.
const MULTI_PART_FIXED_SIZE: usize = 14;
/// Bytes of a Kingdom Reborn part before its string list (no `flag2`).
const MULTI_KR_PART_FIXED_SIZE: usize = MULTI_PART_FIXED_SIZE - size_of::<u8>();
const STRING_ID_SIZE: usize = size_of::<u32>();
/// Bytes of an entry before its parts: the multi id and the part count.
const MULTI_ENTRY_HEADER_SIZE: usize = 2 * size_of::<u32>();
/// Compression of multis added to a package without multi entries (client entries are zlib-compressed).
const DEFAULT_MULTI_COMPRESSION: CompressionFlag = CompressionFlag::Zlib;

/// One part of a multi entry (client part, 0x1c bytes, written by `UOMultiCollection::SetPart`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiItemPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    /// Full signed 16-bit z (the client reads it with a sign-extending word load).
    pub z: i16,
    /// Byte +0xa: the part is not shown. The client skips it in the multi renderer (except for multis whose
    /// collection field +0x30 is 24..99, probably the new ships), in the house customization multi appearances
    /// and in placement tests. Classic `multi.mul` has flags = 0 for these parts. Confidence: high.
    pub not_drawn: bool,
    /// Byte +0xb. With 1 the renderer passes hue 0 instead of the multi hue ("ignore multi hue"); equals the
    /// extra dword of classic `multi.mul` records. Only that reader was found. Confidence: low-medium. Not in
    /// Kingdom Reborn parts (reads 0, and must be 0 to write).
    pub flag2: u8,
    /// String dictionary indices (`string_dictionary.uop`) of the part tags, such as `house_door`, `hueshadow`
    /// or `shipaccess`. The client copies each string and keeps only the non-empty ones; all ids are kept here.
    pub string_ids: Vec<u32>,
}

/// Inclusive coordinate range of all parts of a multi, drawn or not, like the client's bounds
/// (collection +0x30..+0x44).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiBounds {
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,
    pub min_z: i16,
    pub max_z: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiItem {
    pub id: u32,
    pub filename_hash: u64,
    pub path: String,
    pub parts: Vec<MultiItemPart>,
}

#[derive(Debug, Clone)]
pub struct MultiCollection {
    pub items: Vec<MultiItem>,
    items_by_id: BTreeMap<u32, usize>,
    pub housing_hash: Option<u64>,
    pub housing_byte_len: Option<u32>,
    /// The part layout the multis are read and written in.
    dialect: ClientDialect,
}

impl MultiCollection {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_dialect(path, ClientDialect::Enhanced)
    }

    /// [`Self::load`] for a package of `dialect`.
    pub fn load_dialect(path: impl AsRef<Path>, dialect: ClientDialect) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package_dialect(&package, dialect)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        Self::from_package_dialect(package, ClientDialect::Enhanced)
    }

    /// [`Self::from_package`] for a package of `dialect`; [`Self::apply_to_package`] writes the same layout.
    pub fn from_package_dialect(package: &UopPackage, dialect: ClientDialect) -> eyre::Result<Self> {
        let mut items = Vec::new();
        for id in 0..MAX_MULTI_COLLECTION_ID {
            let path = multi_collection_path(id);
            let hash = hash_file_name_single(&path);
            let Some(file) = package.get_file_by_hash(hash) else {
                continue;
            };
            if !file.has_size() {
                continue;
            }
            let bytes = file.unpack().wrap_err_with(|| format!("failed to unpack {path}"))?;
            let mut item = MultiItem::from_payload_bytes_dialect(hash, path, &bytes, dialect)?;
            if item.id != id {
                item.id = id;
            }
            items.push(item);
        }

        let housing_hash = hash_file_name_single(HOUSING_PATH);
        let housing_file = package.get_file_by_hash(housing_hash);
        let items_by_id = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id, index))
            .collect();

        Ok(Self {
            items,
            items_by_id,
            housing_hash: housing_file.map(|file| file.filename_hash()),
            housing_byte_len: housing_file.map(|file| file.decompressed_size()),
            dialect,
        })
    }

    /// The part layout of the collection.
    pub fn dialect(&self) -> ClientDialect {
        self.dialect
    }

    /// Sets the part layout [`Self::apply_to_package`] writes (to convert a collection to the other client).
    pub fn set_dialect(&mut self, dialect: ClientDialect) {
        self.dialect = dialect;
    }

    /// A collection of `items` without `housing.bin` information; the items are sorted by id, as
    /// [`Self::from_package`] lists them. Fails on two items with the same id.
    pub fn from_items(mut items: Vec<MultiItem>) -> eyre::Result<Self> {
        items.sort_by_key(|item| item.id);
        if let Some(pair) = items.windows(2).find(|pair| pair[0].id == pair[1].id) {
            eyre::bail!("multi {} appears twice", pair[0].id);
        }
        let mut collection = Self {
            items,
            items_by_id: BTreeMap::new(),
            housing_hash: None,
            housing_byte_len: None,
            dialect: ClientDialect::default(),
        };
        collection.rebuild_index();
        Ok(collection)
    }

    pub fn get(&self, id: u32) -> Option<&MultiItem> {
        self.items_by_id
            .get(&id)
            .and_then(|index| self.items.get(*index))
    }

    /// The parts of multi `id`, to edit them.
    pub fn parts_mut(&mut self, id: u32) -> Option<&mut Vec<MultiItemPart>> {
        let index = *self.items_by_id.get(&id)?;
        self.items.get_mut(index).map(|item| &mut item.parts)
    }

    /// The multi ids, ascending.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.items_by_id.keys().copied()
    }

    /// Stores `item` under its id, with the entry path and hash of that id; returns the multi it replaces. A new
    /// multi goes into id order.
    pub fn set(&mut self, mut item: MultiItem) -> Option<MultiItem> {
        item.path = multi_collection_path(item.id);
        item.filename_hash = multi_collection_hash(item.id);
        if let Some(&index) = self.items_by_id.get(&item.id) {
            return Some(std::mem::replace(&mut self.items[index], item));
        }
        let index = self.items.partition_point(|existing| existing.id < item.id);
        self.items.insert(index, item);
        self.rebuild_index();
        None
    }

    /// Removes multi `id`; returns it.
    pub fn remove(&mut self, id: u32) -> Option<MultiItem> {
        let index = self.items_by_id.remove(&id)?;
        let item = self.items.remove(index);
        self.rebuild_index();
        Some(item)
    }

    fn rebuild_index(&mut self) {
        self.items_by_id = self.items.iter().enumerate().map(|(index, item)| (item.id, index)).collect();
    }

    /// Makes the multi entries of `package` hold exactly these multis: unchanged entries are left untouched (same
    /// compressed bytes and data block header), changed ones are replaced keeping their compression flag, new ones
    /// are added with the compression of the package's multi entries; both get a fresh timestamp data block header
    /// (the package's edit time). Multi entries missing from the collection are removed; `housing.bin` and any
    /// other entry are kept. The multis are written in the collection's [`Self::dialect`]. Fails on an id outside
    /// [`MAX_MULTI_COLLECTION_ID`] or listed twice, or a part the dialect cannot hold.
    pub fn apply_to_package(&self, package: &mut UopPackage) -> eyre::Result<MultiCollectionChanges> {
        let ids_by_hash: BTreeMap<u64, u32> = (0..MAX_MULTI_COLLECTION_ID).map(|id| (multi_collection_hash(id), id)).collect();
        let compression = package
            .iter_files()
            .find(|file| file.has_size() && ids_by_hash.contains_key(&file.filename_hash()))
            .map_or(DEFAULT_MULTI_COMPRESSION, |file| file.compression());
        let mut changes = MultiCollectionChanges::default();
        let mut kept = BTreeSet::new();
        for item in &self.items {
            if item.id >= MAX_MULTI_COLLECTION_ID {
                eyre::bail!("multi id {} is not below {MAX_MULTI_COLLECTION_ID}", item.id);
            }
            if !kept.insert(item.id) {
                eyre::bail!("multi {} appears twice", item.id);
            }
            let hash = multi_collection_hash(item.id);
            let payload = item.to_payload_bytes_dialect(self.dialect)?;
            let path = multi_collection_path(item.id);
            let current = package.unpack_file_by_hash(hash).wrap_err_with(|| format!("failed to unpack {path}"))?;
            if current.as_deref() == Some(payload.as_slice()) {
                continue;
            }
            match uop_entries::put_entry(package, hash, &payload, compression)? {
                EntryPut::Replaced => changes.replaced += 1,
                EntryPut::Added => changes.added += 1,
            }
        }
        let stale: Vec<u64> = package
            .iter_files()
            .filter(|file| file.has_size())
            .filter(|file| ids_by_hash.get(&file.filename_hash()).is_some_and(|id| !kept.contains(id)))
            .map(|file| file.filename_hash())
            .collect();
        for hash in stale {
            uop_entries::remove_entry(package, hash);
            changes.removed += 1;
        }
        Ok(changes)
    }

    /// Writes a copy of the `source` `MultiCollection.uop` to `output` with these multis
    /// ([`Self::apply_to_package`], stamped with the current time): `housing.bin` and the unchanged multis keep
    /// their bytes and headers, the package keeps its version, block size and layout. `output` must not be
    /// `source`.
    pub fn write_multi_collection(
        &self,
        source: impl AsRef<Path>,
        output: impl AsRef<Path>,
    ) -> eyre::Result<MultiCollectionChanges> {
        let (source, output) = (source.as_ref(), output.as_ref());
        uop_entries::refuse_to_overwrite(source, output)?;
        let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
        let changes = self.apply_to_package(&mut package)?;
        uop_entries::save_package_as(&mut package, source, output)?;
        Ok(changes)
    }
}

/// What [`MultiCollection::apply_to_package`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MultiCollectionChanges {
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
}

impl MultiItem {
    /// A multi with the entry path and hash of `id`.
    pub fn new(id: u32, parts: Vec<MultiItemPart>) -> Self {
        Self { id, filename_hash: multi_collection_hash(id), path: multi_collection_path(id), parts }
    }

    /// Decodes one `build/multicollection/%06d.bin` entry: u32 multi id, u32 part count, then the parts
    /// `{u16 tile, i16 x, i16 y, i16 z, u8 not_drawn, u8 flag2, u32 n, n x u32 string id}`. As in the client,
    /// the entry must end exactly after the last part.
    pub fn from_payload_bytes(filename_hash: u64, path: String, bytes: &[u8]) -> eyre::Result<Self> {
        Self::from_payload_bytes_dialect(filename_hash, path, bytes, ClientDialect::Enhanced)
    }

    /// [`Self::from_payload_bytes`] for an entry of `dialect` (Kingdom Reborn parts have no `flag2`).
    pub fn from_payload_bytes_dialect(
        filename_hash: u64,
        path: String,
        bytes: &[u8],
        dialect: ClientDialect,
    ) -> eyre::Result<Self> {
        let kr = dialect.is_kingdom_reborn();
        let part_size = if kr { MULTI_KR_PART_FIXED_SIZE } else { MULTI_PART_FIXED_SIZE };
        let mut reader = Cursor::new(bytes);
        let id = reader.read_u32::<LittleEndian>()?;
        let count = reader.read_u32::<LittleEndian>()? as usize;
        let mut parts = Vec::with_capacity(count.min(remaining_len(&reader) / part_size));

        for _ in 0..count {
            if remaining_len(&reader) < part_size {
                eyre::bail!("truncated multicollection part in {path}");
            }

            let item_id = reader.read_u16::<LittleEndian>()?;
            let x = reader.read_i16::<LittleEndian>()?;
            let y = reader.read_i16::<LittleEndian>()?;
            let z = reader.read_i16::<LittleEndian>()?;
            let not_drawn = reader.read_u8()? != 0;
            let flag2 = if kr { 0 } else { reader.read_u8()? };
            let string_count = reader.read_u32::<LittleEndian>()? as usize;
            let mut string_ids = Vec::with_capacity(string_count.min(remaining_len(&reader) / STRING_ID_SIZE));
            for _ in 0..string_count {
                string_ids.push(
                    reader
                        .read_u32::<LittleEndian>()
                        .wrap_err_with(|| format!("truncated multicollection part strings in {path}"))?,
                );
            }

            parts.push(MultiItemPart {
                item_id,
                x,
                y,
                z,
                not_drawn,
                flag2,
                string_ids,
            });
        }

        let trailing = remaining_len(&reader);
        if trailing != 0 {
            eyre::bail!("{trailing} trailing bytes after {count} parts in {path}");
        }

        Ok(Self {
            id,
            filename_hash,
            path,
            parts,
        })
    }

    /// Encodes the entry payload: the exact inverse of [`Self::from_payload_bytes`] (`not_drawn` is written as
    /// 0 or 1, the only values in client data; the client tests the byte for non-zero).
    ///
    /// # Panics
    /// When a count does not fit the u32 of the format.
    pub fn to_payload_bytes(&self) -> Vec<u8> {
        self.to_payload_bytes_dialect(ClientDialect::Enhanced).expect("every part fits the Enhanced Client layout")
    }

    /// [`Self::to_payload_bytes`] in the layout of `dialect`; fails on a Kingdom Reborn part with a non-zero
    /// `flag2`, which that layout cannot hold.
    ///
    /// # Panics
    /// When a count does not fit the u32 of the format.
    pub fn to_payload_bytes_dialect(&self, dialect: ClientDialect) -> eyre::Result<Vec<u8>> {
        let kr = dialect.is_kingdom_reborn();
        let part_size = if kr { MULTI_KR_PART_FIXED_SIZE } else { MULTI_PART_FIXED_SIZE };
        let string_ids: usize = self.parts.iter().map(|part| part.string_ids.len()).sum();
        let mut bytes =
            Vec::with_capacity(MULTI_ENTRY_HEADER_SIZE + self.parts.len() * part_size + string_ids * STRING_ID_SIZE);
        bytes.extend_from_slice(&self.id.to_le_bytes());
        bytes.extend_from_slice(&count_u32(self.parts.len()).to_le_bytes());
        for part in &self.parts {
            bytes.extend_from_slice(&part.item_id.to_le_bytes());
            bytes.extend_from_slice(&part.x.to_le_bytes());
            bytes.extend_from_slice(&part.y.to_le_bytes());
            bytes.extend_from_slice(&part.z.to_le_bytes());
            bytes.push(u8::from(part.not_drawn));
            if kr {
                eyre::ensure!(
                    part.flag2 == 0,
                    "multi {}: part {} has flag2 {}, which Kingdom Reborn parts lack",
                    self.id,
                    part.item_id,
                    part.flag2
                );
            } else {
                bytes.push(part.flag2);
            }
            bytes.extend_from_slice(&count_u32(part.string_ids.len()).to_le_bytes());
            for string_id in &part.string_ids {
                bytes.extend_from_slice(&string_id.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    pub fn bounds(&self) -> Option<MultiBounds> {
        let first = self.parts.first()?;
        let mut bounds = MultiBounds {
            min_x: first.x,
            max_x: first.x,
            min_y: first.y,
            max_y: first.y,
            min_z: first.z,
            max_z: first.z,
        };
        for part in &self.parts {
            bounds.min_x = bounds.min_x.min(part.x);
            bounds.max_x = bounds.max_x.max(part.x);
            bounds.min_y = bounds.min_y.min(part.y);
            bounds.max_y = bounds.max_y.max(part.y);
            bounds.min_z = bounds.min_z.min(part.z);
            bounds.max_z = bounds.max_z.max(part.z);
        }
        Some(bounds)
    }
}

fn count_u32(count: usize) -> u32 {
    u32::try_from(count).expect("multi counts fit the u32 of the format")
}

fn remaining_len(reader: &Cursor<&[u8]>) -> usize {
    reader.get_ref().len().saturating_sub(reader.position() as usize)
}

pub fn multi_collection_path(id: u32) -> String {
    format!("build/multicollection/{id:06}.bin")
}

pub fn multi_collection_hash(id: u32) -> u64 {
    hash_file_name_single(&multi_collection_path(id))
}

pub fn housing_hash() -> u64 {
    hash_file_name_single(HOUSING_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uop_container::file::CompressionFlag;
    use byteorder::WriteBytesExt;

    #[test]
    fn path_helpers_match_converter_pattern() {
        assert_eq!(multi_collection_path(0), "build/multicollection/000000.bin");
        assert_eq!(multi_collection_path(42), "build/multicollection/000042.bin");
        assert_eq!(housing_hash(), hash_file_name_single(HOUSING_PATH));
    }

    const TEST_MULTI_ID: u32 = 42;
    const TEST_HASH: u64 = 0xAA;

    struct TestPart {
        item_id: u16,
        x: i16,
        y: i16,
        z: i16,
        not_drawn: u8,
        flag2: u8,
        string_ids: &'static [u32],
    }

    fn entry_bytes(parts: &[TestPart]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_u32::<LittleEndian>(TEST_MULTI_ID).unwrap();
        bytes.write_u32::<LittleEndian>(parts.len() as u32).unwrap();
        for part in parts {
            bytes.write_u16::<LittleEndian>(part.item_id).unwrap();
            bytes.write_i16::<LittleEndian>(part.x).unwrap();
            bytes.write_i16::<LittleEndian>(part.y).unwrap();
            bytes.write_i16::<LittleEndian>(part.z).unwrap();
            bytes.write_u8(part.not_drawn).unwrap();
            bytes.write_u8(part.flag2).unwrap();
            bytes.write_u32::<LittleEndian>(part.string_ids.len() as u32).unwrap();
            for id in part.string_ids {
                bytes.write_u32::<LittleEndian>(*id).unwrap();
            }
        }
        bytes
    }

    fn test_parts() -> [TestPart; 2] {
        [
            TestPart { item_id: 0x1234, x: 1, y: -2, z: 255, not_drawn: 1, flag2: 0, string_ids: &[1000, 0x8000_0000] },
            TestPart { item_id: 0x2345, x: 3, y: 4, z: -300, not_drawn: 0, flag2: 1, string_ids: &[] },
        ]
    }

    #[test]
    fn parses_variable_length_multi_collection_entry() {
        let bytes = entry_bytes(&test_parts());

        let item = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes).unwrap();

        assert_eq!(item.id, TEST_MULTI_ID);
        assert_eq!(item.parts.len(), 2);
        assert_eq!(item.parts[0].z, 255);
        assert_eq!(item.parts[1].z, -300);
        assert!(item.parts[0].not_drawn);
        assert_eq!(item.parts[0].flag2, 0);
        assert!(!item.parts[1].not_drawn);
        assert_eq!(item.parts[1].flag2, 1);
        assert_eq!(item.parts[0].string_ids, vec![1000, 0x8000_0000]);
        assert!(item.parts[1].string_ids.is_empty());
        assert_eq!(
            item.bounds(),
            Some(MultiBounds { min_x: 1, max_x: 3, min_y: -2, max_y: 4, min_z: -300, max_z: 255 })
        );
    }

    #[test]
    fn rejects_trailing_bytes_after_last_part() {
        let mut bytes = entry_bytes(&test_parts());
        bytes.push(0);

        let error = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes)
            .unwrap_err();
        assert!(error.to_string().contains("trailing"), "{error}");
    }

    #[test]
    fn rejects_truncated_string_list() {
        let mut bytes = entry_bytes(&test_parts());
        bytes.truncate(bytes.len() - MULTI_PART_FIXED_SIZE - STRING_ID_SIZE);

        assert!(MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes).is_err());
    }

    #[test]
    fn from_package_finds_the_highest_multi_id() {
        let highest_id = u32::from(u16::MAX);
        let mut package = UopPackage::new_default();
        package
            .add_file_from_memory(&entry_bytes(&test_parts()), &multi_collection_path(highest_id), CompressionFlag::None)
            .unwrap();

        let collection = MultiCollection::from_package(&package).unwrap();

        assert_eq!(collection.items.len(), 1);
        assert_eq!(collection.get(highest_id).map(|item| item.parts.len()), Some(test_parts().len()));
    }

    #[test]
    fn to_payload_bytes_is_the_inverse_of_the_parser() {
        let bytes = entry_bytes(&test_parts());
        let item = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes).unwrap();
        assert_eq!(item.to_payload_bytes(), bytes);
        assert_eq!(MultiItem::new(TEST_MULTI_ID, Vec::new()).to_payload_bytes(), entry_bytes(&[]));
    }

    #[test]
    fn kingdom_reborn_parts_have_no_flag2() {
        const KR: ClientDialect = ClientDialect::KingdomReborn;
        /// Offset of the first part's `flag2`: the entry header, then tile id, x, y, z and `not_drawn`.
        const FIRST_FLAG2_OFFSET: usize = MULTI_ENTRY_HEADER_SIZE + MULTI_KR_PART_FIXED_SIZE - size_of::<u32>();
        let [first, second] = test_parts();
        let parts = [first, TestPart { flag2: 0, ..second }];
        // The KR entry is the EC one without the flag2 byte of each part.
        let mut kr_bytes = entry_bytes(&parts);
        kr_bytes.remove(FIRST_FLAG2_OFFSET);
        let second_flag2 = FIRST_FLAG2_OFFSET + MULTI_KR_PART_FIXED_SIZE + parts[0].string_ids.len() * STRING_ID_SIZE;
        kr_bytes.remove(second_flag2);

        let path = multi_collection_path(TEST_MULTI_ID);
        let item = MultiItem::from_payload_bytes_dialect(TEST_HASH, path.clone(), &kr_bytes, KR).unwrap();
        let ec = MultiItem::from_payload_bytes(TEST_HASH, path, &entry_bytes(&parts)).unwrap();
        assert_eq!(item, ec);
        assert_eq!(item.to_payload_bytes_dialect(KR).unwrap(), kr_bytes);

        let flagged_bytes = entry_bytes(&test_parts());
        let flagged = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &flagged_bytes).unwrap();
        assert!(flagged.to_payload_bytes_dialect(KR).is_err());
        let mut collection = MultiCollection::from_items(vec![flagged]).unwrap();
        collection.set_dialect(KR);
        assert!(collection.apply_to_package(&mut UopPackage::new_default()).is_err());
    }

    #[test]
    fn set_and_remove_keep_the_id_order_and_index() {
        let parts = MultiItem::from_payload_bytes(TEST_HASH, String::new(), &entry_bytes(&test_parts())).unwrap().parts;
        let mut collection = MultiCollection::from_items(vec![
            MultiItem::new(5, parts.clone()),
            MultiItem::new(1, Vec::new()),
        ])
        .unwrap();
        assert_eq!(collection.ids().collect::<Vec<_>>(), [1, 5]);
        assert!(collection.set(MultiItem::new(3, parts.clone())).is_none());
        assert_eq!(collection.ids().collect::<Vec<_>>(), [1, 3, 5]);
        assert_eq!(collection.items.iter().map(|item| item.id).collect::<Vec<_>>(), [1, 3, 5]);
        let replaced = collection.set(MultiItem::new(1, parts.clone())).unwrap();
        assert!(replaced.parts.is_empty());
        assert_eq!(collection.get(3).unwrap().filename_hash, multi_collection_hash(3));
        collection.parts_mut(5).unwrap().pop();
        assert_eq!(collection.get(5).unwrap().parts.len(), parts.len() - 1);
        assert_eq!(collection.remove(3).map(|item| item.id), Some(3));
        assert!(collection.remove(3).is_none());
        assert_eq!(collection.get(5).map(|item| item.id), Some(5), "the index follows the removal");
        assert!(MultiCollection::from_items(vec![MultiItem::new(1, Vec::new()), MultiItem::new(1, Vec::new())]).is_err());
    }

    #[test]
    fn apply_to_package_replaces_adds_and_removes_only_multis() {
        const KEPT: u32 = 1;
        const EDITED: u32 = 2;
        const REMOVED: u32 = 3;
        const ADDED: u32 = 4;
        const HOUSING_PAYLOAD: &[u8] = b"housing pieces";
        let parts = MultiItem::from_payload_bytes(TEST_HASH, String::new(), &entry_bytes(&test_parts())).unwrap().parts;
        let mut package = UopPackage::new_default();
        for id in [KEPT, EDITED, REMOVED] {
            let payload = MultiItem::new(id, parts.clone()).to_payload_bytes();
            package.add_file_from_memory(&payload, &multi_collection_path(id), CompressionFlag::Zlib).unwrap();
        }
        package.add_file_from_memory(HOUSING_PAYLOAD, HOUSING_PATH, CompressionFlag::None).unwrap();
        let kept_before = package.get_file_by_hash(multi_collection_hash(KEPT)).unwrap().data().cloned();

        let mut collection = MultiCollection::from_package(&package).unwrap();
        assert_eq!(collection.housing_hash, Some(housing_hash()));
        collection.parts_mut(EDITED).unwrap().truncate(1);
        collection.remove(REMOVED);
        collection.set(MultiItem::new(ADDED, Vec::new()));
        let changes = collection.apply_to_package(&mut package).unwrap();

        assert_eq!(changes, MultiCollectionChanges { replaced: 1, added: 1, removed: 1 });
        assert_eq!(package.get_file_by_hash(multi_collection_hash(KEPT)).unwrap().data().cloned(), kept_before);
        assert_eq!(package.get_file_by_hash(multi_collection_hash(ADDED)).unwrap().compression(), CompressionFlag::Zlib);
        assert!(package.get_file_by_hash(multi_collection_hash(REMOVED)).is_none());
        assert_eq!(package.get_file_by_hash(housing_hash()).unwrap().unpack().unwrap(), HOUSING_PAYLOAD);
        let written = MultiCollection::from_package(&package).unwrap();
        assert_eq!(written.items, collection.items);
        assert_eq!(collection.apply_to_package(&mut package).unwrap(), MultiCollectionChanges::default());

        collection.set(MultiItem::new(MAX_MULTI_COLLECTION_ID, Vec::new()));
        assert!(collection.apply_to_package(&mut package).is_err(), "ids above the scanned range are refused");
    }

    #[test]
    fn empty_entry_has_no_bounds() {
        let bytes = entry_bytes(&[]);

        let item = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes).unwrap();
        assert!(item.parts.is_empty());
        assert_eq!(item.bounds(), None);
    }
}
