//! Decoding for UO Enhanced Client multi files (`Multi.uop`).
//!
//! This module handles the parsing of multi item data from the `Multi.uop` package
//! using zero-copy extraction directly from the decoded UOP blocks.

crate::eyre_imports!();
use crate::uop::package::UopPackage;
use bytemuck::{Pod, Zeroable};
use std::path::Path;

// region: --- Internal Raw File-Mapping Structs (Binary Format)

/// Raw, packed representation of a single multi component as stored in `Multi.uop`.
///
/// This structure matches the 16-byte aligned binary layout used by the file format.
/// It is used for zero-copy parsing before being converted to the public `MultiItemPart`.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RawMultiItemPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub flags: u32,
    pub cliloc_id: u32,
}

// endregion: --- Internal Raw File-Mapping Structs

// region: --- Public API (Convenience & Application Use)

/// Processed representation of a multi component with native byte ordering.
#[derive(Debug, Clone, Copy)]
pub struct MultiItemPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub flags: u32,
    pub cliloc_id: u32,
}

/// The type of a multi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiType {
    Boat,
    House,
    Decoration,
    Other,
}

/// Represents a single multi item.
#[derive(Debug, Clone)]
pub struct MultiItem {
    pub id: u32,
    pub name: String,
    pub multi_type: MultiType,
    pub parts: Vec<MultiItemPart>,
}

impl MultiItem {
    /// Reads a `MultiItem` from a zero-copy byte slice.
    pub fn load(data: &[u8], name: String) -> eyre::Result<Self> {
        if data.len() < 8 {
            eyre::bail!("Data too small to contain MultiItem header");
        }

        let id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;

        let parts_size = count * std::mem::size_of::<RawMultiItemPart>();
        if data.len() < 8 + parts_size {
            eyre::bail!("Data too small to contain declared MultiItem parts");
        }

        let parts_slice = &data[8..8 + parts_size];
        let raw_parts: &[RawMultiItemPart] = bytemuck::cast_slice(parts_slice);
        
        let mut parts = Vec::with_capacity(count);
        for raw in raw_parts {
            parts.push(MultiItemPart {
                item_id: u16::from_le(raw.item_id),
                x: i16::from_le(raw.x),
                y: i16::from_le(raw.y),
                z: i16::from_le(raw.z),
                flags: u32::from_le(raw.flags),
                cliloc_id: u32::from_le(raw.cliloc_id),
            });
        }

        let multi_type = if name.contains("boat") {
            MultiType::Boat
        } else if name.contains("house") {
            MultiType::House
        } else if name.contains("decoration") {
            MultiType::Decoration
        } else {
            MultiType::Other
        };

        Ok(Self {
            id,
            name,
            multi_type,
            parts,
        })
    }
}

/// Represents a collection of multi items.
#[derive(Debug, Clone)]
pub struct MultiCollection {
    pub items: Vec<MultiItem>,
}

impl MultiCollection {
    /// Loads a `MultiCollection` from a UOP file.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        let mut items = Vec::new();

        for file in package.iter_files() {
            let data = file.unpack()?;
            // Currently name must be retrieved from external string dictionary.
            let item = MultiItem::load(&data, String::new())?;
            items.push(item);
        }

        Ok(Self { items })
    }
}

// endregion: --- Public API
