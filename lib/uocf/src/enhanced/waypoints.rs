//! Decoder for Enhanced Client `waypoints.uop`.
//!
//! The package is currently known to contain one payload at
//! `build/sectors/waypoint.bin`. The record semantics are not yet verified, so
//! this module preserves the raw section layout instead of naming fields as
//! coordinates, map ids, names, or icon ids prematurely.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const WAYPOINTS_UOP_NAME: &str = "waypoints.uop";
pub const WAYPOINTS_PAYLOAD_PATH: &str = "build/sectors/waypoint.bin";
pub const KNOWN_WAYPOINTS_PAYLOAD_HASH: u64 = 0xE2818A15B51C6F36;
pub const WAYPOINTS_SUPPORTED_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaypointsPackage {
    pub version: u16,
    pub sub_24: Vec<WaypointPairRecord>,
    pub sub_24_2: Vec<WaypointPairRecord>,
    pub sub_24_3: Vec<WaypointLinkedRecord>,
    pub sub_24_4: Vec<WaypointDetailRecord>,
    pub trailing_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointPairRecord {
    pub value_1: u32,
    pub value_2: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointLinkedRecord {
    pub value_1: u32,
    pub value_2: u8,
    pub links: [WaypointLinkedValue; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointLinkedValue {
    pub value_1: u32,
    pub value_2: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaypointDetailRecord {
    pub value_1: u32,
    pub value_2: u32,
    pub value_3: u8,
    pub value_4: u8,
    pub value_5: u16,
    pub value_6: u16,
    pub value_7: u32,
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

        let sub_24 = read_pair_section(&mut reader, bytes, "sub_24")?;
        let sub_24_2 = read_pair_section(&mut reader, bytes, "sub_24_2")?;
        let sub_24_3 = read_linked_section(&mut reader, bytes)?;
        let sub_24_4 = read_detail_section(&mut reader, bytes)?;

        let mut trailing_bytes = Vec::new();
        reader.read_to_end(&mut trailing_bytes)?;

        Ok(Self {
            version,
            sub_24,
            sub_24_2,
            sub_24_3,
            sub_24_4,
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

fn read_pair_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
    section: &str,
) -> eyre::Result<Vec<WaypointPairRecord>> {
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 8, section)?;
        records.push(WaypointPairRecord {
            value_1: reader.read_u32::<LittleEndian>()?,
            value_2: reader.read_u32::<LittleEndian>()?,
        });
    }
    Ok(records)
}

fn read_linked_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
) -> eyre::Result<Vec<WaypointLinkedRecord>> {
    let section = "sub_24_3";
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 19, section)?;
        let value_1 = reader.read_u32::<LittleEndian>()?;
        let value_2 = reader.read_u8()?;
        let mut links = [WaypointLinkedValue {
            value_1: 0,
            value_2: 0,
        }; 3];
        for link in &mut links {
            link.value_1 = reader.read_u32::<LittleEndian>()?;
            link.value_2 = reader.read_u8()?;
        }
        records.push(WaypointLinkedRecord {
            value_1,
            value_2,
            links,
        });
    }
    Ok(records)
}

fn read_detail_section(
    reader: &mut Cursor<&[u8]>,
    bytes: &[u8],
) -> eyre::Result<Vec<WaypointDetailRecord>> {
    let section = "sub_24_4";
    let count = read_count(reader, bytes, section)?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        require_remaining(reader, bytes, 18, section)?;
        records.push(WaypointDetailRecord {
            value_1: reader.read_u32::<LittleEndian>()?,
            value_2: reader.read_u32::<LittleEndian>()?,
            value_3: reader.read_u8()?,
            value_4: reader.read_u8()?,
            value_5: reader.read_u16::<LittleEndian>()?,
            value_6: reader.read_u16::<LittleEndian>()?,
            value_7: reader.read_u32::<LittleEndian>()?,
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
        assert_eq!(package.sub_24, vec![WaypointPairRecord { value_1: 10, value_2: 20 }]);
        assert_eq!(package.sub_24_2, vec![WaypointPairRecord { value_1: 30, value_2: 40 }]);
        assert_eq!(package.sub_24_3[0].value_1, 50);
        assert_eq!(package.sub_24_3[0].value_2, 51);
        assert_eq!(
            package.sub_24_3[0].links,
            [
                WaypointLinkedValue { value_1: 60, value_2: 70 },
                WaypointLinkedValue { value_1: 61, value_2: 71 },
                WaypointLinkedValue { value_1: 62, value_2: 72 },
            ]
        );
        assert_eq!(
            package.sub_24_4,
            vec![WaypointDetailRecord {
                value_1: 80,
                value_2: 90,
                value_3: 100,
                value_4: 101,
                value_5: 110,
                value_6: 120,
                value_7: 130,
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
        assert!(error.to_string().contains("truncated waypoints payload in sub_24"));
    }
}
