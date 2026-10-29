//! Decoding for EC/CC `MultiCollection.uop` files.
//!
//! Layout and field meanings follow the client parser (`UOMultiCollection::Parse` in UOSA.exe), documented in
//! `scripts/dev-helpers/ec-research/ghidra-housing/results/uosa_housing.md` (section 4). The `housing.bin` entry
//! of the same package is decoded by [`super::housing`].

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const MULTI_COLLECTION_UOP_NAME: &str = "MultiCollection.uop";
pub const HOUSING_PATH: &str = "build/multicollection/housing.bin";
/// Exclusive bound of the multi ids searched in the package: multi ids are 16-bit graphics. Recent clients
/// (2026) ship multi 9000, above the 0x2200 bound used before.
pub const MAX_MULTI_COLLECTION_ID: u32 = 0x10000;
/// Bytes of a part before its string list: tile id, x, y, z, `not_drawn`, `flag2` and the string count.
const MULTI_PART_FIXED_SIZE: usize = 14;
const STRING_ID_SIZE: usize = size_of::<u32>();

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
    /// extra dword of classic `multi.mul` records. Only that reader was found. Confidence: low-medium.
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
}

impl MultiCollection {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
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
            let mut item = MultiItem::from_payload_bytes(hash, path, &bytes)?;
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
        })
    }

    pub fn get(&self, id: u32) -> Option<&MultiItem> {
        self.items_by_id
            .get(&id)
            .and_then(|index| self.items.get(*index))
    }
}

impl MultiItem {
    /// Decodes one `build/multicollection/%06d.bin` entry: u32 multi id, u32 part count, then the parts
    /// `{u16 tile, i16 x, i16 y, i16 z, u8 not_drawn, u8 flag2, u32 n, n x u32 string id}`. As in the client,
    /// the entry must end exactly after the last part.
    pub fn from_payload_bytes(filename_hash: u64, path: String, bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let id = reader.read_u32::<LittleEndian>()?;
        let count = reader.read_u32::<LittleEndian>()? as usize;
        let mut parts = Vec::with_capacity(count.min(remaining_len(&reader) / MULTI_PART_FIXED_SIZE));

        for _ in 0..count {
            if remaining_len(&reader) < MULTI_PART_FIXED_SIZE {
                eyre::bail!("truncated multicollection part in {path}");
            }

            let item_id = reader.read_u16::<LittleEndian>()?;
            let x = reader.read_i16::<LittleEndian>()?;
            let y = reader.read_i16::<LittleEndian>()?;
            let z = reader.read_i16::<LittleEndian>()?;
            let not_drawn = reader.read_u8()? != 0;
            let flag2 = reader.read_u8()?;
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
    fn empty_entry_has_no_bounds() {
        let bytes = entry_bytes(&[]);

        let item = MultiItem::from_payload_bytes(TEST_HASH, multi_collection_path(TEST_MULTI_ID), &bytes).unwrap();
        assert!(item.parts.is_empty());
        assert_eq!(item.bounds(), None);
    }
}
