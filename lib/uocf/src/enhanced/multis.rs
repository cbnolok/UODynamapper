//! Decoding for EC/CC `MultiCollection.uop` files.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const MULTI_COLLECTION_UOP_NAME: &str = "MultiCollection.uop";
pub const HOUSING_PATH: &str = "build/multicollection/housing.bin";
pub const MAX_MULTI_COLLECTION_ID: u32 = 0x2200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiItemPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub flags: u16,
    pub cliloc_offsets: Vec<i32>,
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
    pub fn from_payload_bytes(filename_hash: u64, path: String, bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let id = reader.read_u32::<LittleEndian>()?;
        let count = reader.read_u32::<LittleEndian>()? as usize;
        let mut parts = Vec::with_capacity(count);

        for _ in 0..count {
            let remaining = bytes.len().saturating_sub(reader.position() as usize);
            if remaining < 14 {
                eyre::bail!("truncated multicollection part in {path}");
            }

            let item_id = reader.read_u16::<LittleEndian>()?;
            let x = reader.read_i16::<LittleEndian>()?;
            let y = reader.read_i16::<LittleEndian>()?;
            let z = reader.read_i16::<LittleEndian>()?;
            let flags = reader.read_u16::<LittleEndian>()?;
            let cliloc_count = reader.read_u32::<LittleEndian>()? as usize;
            let mut cliloc_offsets = Vec::with_capacity(cliloc_count.min(16));
            for _ in 0..cliloc_count {
                cliloc_offsets.push(reader.read_i32::<LittleEndian>()?);
            }

            parts.push(MultiItemPart {
                item_id,
                x,
                y,
                z: (z as i8) as i16,
                flags,
                cliloc_offsets,
            });
        }

        Ok(Self {
            id,
            filename_hash,
            path,
            parts,
        })
    }

    pub fn bounds(&self) -> Option<(i16, i16, i16, i16)> {
        let first = self.parts.first()?;
        let mut min_x = first.x;
        let mut max_x = first.x;
        let mut min_y = first.y;
        let mut max_y = first.y;
        for part in &self.parts {
            min_x = min_x.min(part.x);
            max_x = max_x.max(part.x);
            min_y = min_y.min(part.y);
            max_y = max_y.max(part.y);
        }
        Some((min_x, max_x, min_y, max_y))
    }
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
    use byteorder::WriteBytesExt;

    #[test]
    fn path_helpers_match_converter_pattern() {
        assert_eq!(multi_collection_path(0), "build/multicollection/000000.bin");
        assert_eq!(multi_collection_path(42), "build/multicollection/000042.bin");
        assert_eq!(housing_hash(), hash_file_name_single(HOUSING_PATH));
    }

    #[test]
    fn parses_variable_length_multi_collection_entry() {
        let mut bytes = Vec::new();
        bytes.write_u32::<LittleEndian>(42).unwrap();
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes.write_u16::<LittleEndian>(0x1234).unwrap();
        bytes.write_i16::<LittleEndian>(1).unwrap();
        bytes.write_i16::<LittleEndian>(-2).unwrap();
        bytes.write_i16::<LittleEndian>(255).unwrap();
        bytes.write_u16::<LittleEndian>(7).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_i32::<LittleEndian>(1000).unwrap();
        bytes.write_u16::<LittleEndian>(0x2345).unwrap();
        bytes.write_i16::<LittleEndian>(3).unwrap();
        bytes.write_i16::<LittleEndian>(4).unwrap();
        bytes.write_i16::<LittleEndian>(5).unwrap();
        bytes.write_u16::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();

        let item = MultiItem::from_payload_bytes(0xAA, multi_collection_path(42), &bytes).unwrap();

        assert_eq!(item.id, 42);
        assert_eq!(item.parts.len(), 2);
        assert_eq!(item.parts[0].z, -1);
        assert_eq!(item.parts[0].cliloc_offsets, vec![1000]);
        assert_eq!(item.bounds(), Some((1, 3, -2, 4)));
    }
}
