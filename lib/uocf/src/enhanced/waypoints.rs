//! Decoder for Enhanced Client `waypoint.uop`.
//!
//! The package is currently known to contain one payload at
//! `build/sectors/waypoint.bin`.
//!
//! The extracted EC UI `waypoints.lua` table describes waypoints as facet-local
//! `x`, `y`, `z`, `type`, `Name`, `Icon`, and `Scale` values. The binary package
//! is not a byte-for-byte copy of that Lua table: it contains definition
//! sections and a compact waypoint section. Cliloc-backed fields and waypoint
//! coordinates are named here; the remaining small fields keep neutral names
//! until their UI role is verified.
//!
//! EC `mapcommon.lua` identifies the waypoint type ids used by the client:
//! 1 corpse, 2 party, 4 quest giver, 5 new player quest, 6 wandering healer,
//! 7 danger, 9 city, 10 dungeon, 11 shrine, 12 moongate, 14 player, 15 custom.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const WAYPOINTS_UOP_NAME: &str = "waypoint.uop";
pub const WAYPOINTS_PAYLOAD_PATH: &str = "build/sectors/waypoint.bin";
pub const KNOWN_WAYPOINTS_PAYLOAD_HASH: u64 = 0xE2818A15B51C6F36;
pub const WAYPOINTS_SUPPORTED_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaypointsPackage {
    pub version: u16,
    pub icon_definitions: Vec<WaypointClilocDefinition>,
    pub effect_definitions: Vec<WaypointClilocDefinition>,
    pub type_definitions: Vec<WaypointTypeDefinition>,
    pub waypoints: Vec<WaypointRecord>,
    pub trailing_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointClilocDefinition {
    pub id: u32,
    pub name_cliloc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointTypeDefinition {
    pub name_cliloc: u32,
    pub flags: u8,
    pub links: [WaypointTypeLink; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointTypeLink {
    pub value_1: u32,
    pub value_2: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointRecord {
    pub x: u32,
    pub y: u32,
    pub z: i8,
    pub facet: u8,
    pub waypoint_type: u16,
    pub value_6: u16,
    pub name_cliloc: u32,
}

impl WaypointsPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let path_hash = hash_file_name_single(WAYPOINTS_PAYLOAD_PATH);
        let file = package
            .get_file_by_hash(path_hash)
            .or_else(|| package.get_file_by_hash(KNOWN_WAYPOINTS_PAYLOAD_HASH))
            .ok_or_else(|| {
                eyre::eyre!(
                    "waypoints payload not found at {} ({:#018x} or {:#018x})",
                    WAYPOINTS_PAYLOAD_PATH,
                    path_hash,
                    KNOWN_WAYPOINTS_PAYLOAD_HASH,
                )
            })?;

        let bytes = file
            .unpack()
            .wrap_err_with(|| format!("failed to unpack {WAYPOINTS_PAYLOAD_PATH}"))?;
        Self::from_payload_bytes(&bytes)
    }

    pub fn from_payload_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let version = reader.read_u16::<LittleEndian>()?;
        if version != WAYPOINTS_SUPPORTED_VERSION {
            eyre::bail!(
                "unsupported waypoints payload version: expected {}, got {}",
                WAYPOINTS_SUPPORTED_VERSION,
                version,
            );
        }

        let icon_definitions =
            read_cliloc_definition_section(&mut reader, bytes, "icon_definitions")?;
        let effect_definitions =
            read_cliloc_definition_section(&mut reader, bytes, "effect_definitions")?;
        let type_definitions = read_type_definition_section(&mut reader, bytes)?;
        let waypoints = read_waypoint_section(&mut reader, bytes)?;

        let mut trailing_bytes = Vec::new();
        reader.read_to_end(&mut trailing_bytes)?;

        Ok(Self {
            version,
            icon_definitions,
            effect_definitions,
            type_definitions,
            waypoints,
            trailing_bytes,
        })
    }
}

pub fn waypoints_payload_hash() -> u64 {
    hash_file_name_single(WAYPOINTS_PAYLOAD_PATH)
}

fn read_count(reader: &mut Cursor<&[u8]>, bytes: &[u8], section: &str) -> eyre::Result<usize> {
    require_remaining(reader, bytes, 2, section)?;
    Ok(reader.read_u16::<LittleEndian>()? as usize)
}

fn read_cliloc_definition_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
    section: &str,
) -> eyre::Result<Vec<WaypointClilocDefinition>> {
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 8, section)?;
        records.push(WaypointClilocDefinition {
            id: reader.read_u32::<LittleEndian>()?,
            name_cliloc: reader.read_u32::<LittleEndian>()?,
        });
    }
    Ok(records)
}

fn read_type_definition_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
) -> eyre::Result<Vec<WaypointTypeDefinition>> {
    let section = "type_definitions";
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 19, section)?;
        let name_cliloc = reader.read_u32::<LittleEndian>()?;
        let flags = reader.read_u8()?;
        let mut links = [WaypointTypeLink {
            value_1: 0,
            value_2: 0,
        }; 3];
        for link in &mut links {
            link.value_1 = reader.read_u32::<LittleEndian>()?;
            link.value_2 = reader.read_u8()?;
        }
        records.push(WaypointTypeDefinition {
            name_cliloc,
            flags,
            links,
        });
    }
    Ok(records)
}

fn read_waypoint_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
) -> eyre::Result<Vec<WaypointRecord>> {
    let section = "waypoints";
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 18, section)?;
        records.push(WaypointRecord {
            x: reader.read_u32::<LittleEndian>()?,
            y: reader.read_u32::<LittleEndian>()?,
            z: reader.read_i8()?,
            facet: reader.read_u8()?,
            waypoint_type: reader.read_u16::<LittleEndian>()?,
            value_6: reader.read_u16::<LittleEndian>()?,
            name_cliloc: reader.read_u32::<LittleEndian>()?,
        });
    }
    Ok(records)
}

fn require_remaining(
    reader: &Cursor<&[u8]>,
    bytes: &[u8],
    needed: usize,
    section: &str,
) -> eyre::Result<()> {
    let remaining = bytes.len().saturating_sub(reader.position() as usize);
    if remaining < needed {
        eyre::bail!(
            "truncated waypoints payload in {section}: needed {needed} bytes, had {remaining}",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    fn sample_payload() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_u16::<LittleEndian>(2).unwrap();

        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(10).unwrap();
        bytes.write_u32::<LittleEndian>(20).unwrap();

        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(30).unwrap();
        bytes.write_u32::<LittleEndian>(40).unwrap();

        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(50).unwrap();
        bytes.write_u8(51).unwrap();
        for index in 0..3 {
            bytes.write_u32::<LittleEndian>(60 + index).unwrap();
            bytes.write_u8(70 + index as u8).unwrap();
        }

        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(80).unwrap();
        bytes.write_u32::<LittleEndian>(90).unwrap();
        bytes.write_u8(100).unwrap();
        bytes.write_u8(101).unwrap();
        bytes.write_u16::<LittleEndian>(110).unwrap();
        bytes.write_u16::<LittleEndian>(120).unwrap();
        bytes.write_u32::<LittleEndian>(130).unwrap();

        bytes.extend_from_slice(&[0xAA, 0xBB]);
        bytes
    }

    #[test]
    fn path_hash_matches_known_waypoints_hash() {
        assert_eq!(waypoints_payload_hash(), KNOWN_WAYPOINTS_PAYLOAD_HASH);
    }

    #[test]
    fn parses_known_waypoints_layout() {
        let package = WaypointsPackage::from_payload_bytes(&sample_payload()).unwrap();

        assert_eq!(package.version, 2);
        assert_eq!(
            package.icon_definitions,
            vec![WaypointClilocDefinition { id: 10, name_cliloc: 20 }]
        );
        assert_eq!(
            package.effect_definitions,
            vec![WaypointClilocDefinition { id: 30, name_cliloc: 40 }]
        );
        assert_eq!(package.type_definitions[0].name_cliloc, 50);
        assert_eq!(package.type_definitions[0].flags, 51);
        assert_eq!(
            package.type_definitions[0].links,
            [
                WaypointTypeLink { value_1: 60, value_2: 70 },
                WaypointTypeLink { value_1: 61, value_2: 71 },
                WaypointTypeLink { value_1: 62, value_2: 72 },
            ]
        );
        assert_eq!(
            package.waypoints,
            vec![WaypointRecord {
                x: 80,
                y: 90,
                z: 100,
                facet: 101,
                waypoint_type: 110,
                value_6: 120,
                name_cliloc: 130,
            }]
        );
        assert_eq!(package.trailing_bytes, vec![0xAA, 0xBB]);
    }

    #[test]
    fn rejects_truncated_section_record() {
        let mut bytes = Vec::new();
        bytes.write_u16::<LittleEndian>(2).unwrap();
        bytes.write_u16::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(10).unwrap();

        let error = WaypointsPackage::from_payload_bytes(&bytes).unwrap_err();
        assert!(error
            .to_string()
            .contains("truncated waypoints payload in icon_definitions"));
    }
}
