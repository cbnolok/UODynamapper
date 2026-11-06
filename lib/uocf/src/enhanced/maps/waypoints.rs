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
//!
//! [`WaypointsPackage::to_payload_bytes`] is the exact inverse of the parser (unknown fields and trailing bytes
//! are kept verbatim), and [`WaypointsPackage::write_waypoints_package`] writes a copy of `waypoint.uop` with the
//! payload replaced.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use crate::enhanced::uop_entries;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

pub const WAYPOINTS_UOP_NAME: &str = "waypoint.uop";
pub const WAYPOINTS_PAYLOAD_PATH: &str = "build/sectors/waypoint.bin";
pub const KNOWN_WAYPOINTS_PAYLOAD_HASH: u64 = 0xE2818A15B51C6F36;
pub const WAYPOINTS_SUPPORTED_VERSION: u16 = 2;
/// Links per type definition.
pub const WAYPOINT_TYPE_LINK_COUNT: usize = 3;

/// Bytes of a section count (u16).
const SECTION_COUNT_SIZE: usize = size_of::<u16>();
/// Bytes of an icon or effect definition: u32 id, u32 name cliloc.
const CLILOC_DEFINITION_SIZE: usize = 2 * size_of::<u32>();
/// Bytes of a type link: u32 `value_1`, u8 `value_2`.
const TYPE_LINK_SIZE: usize = size_of::<u32>() + size_of::<u8>();
/// Bytes of a type definition: u32 name cliloc, u8 flags, the links.
const TYPE_DEFINITION_SIZE: usize = size_of::<u32>() + size_of::<u8>() + WAYPOINT_TYPE_LINK_COUNT * TYPE_LINK_SIZE;
/// Bytes of a waypoint: u32 x, u32 y, i8 z, u8 facet, u16 type, u16 `value_6`, u32 name cliloc.
const WAYPOINT_RECORD_SIZE: usize = 2 * size_of::<u32>() + 2 * size_of::<u8>() + 2 * size_of::<u16>() + size_of::<u32>();

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
    pub links: [WaypointTypeLink; WAYPOINT_TYPE_LINK_COUNT],
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
    /// Meaning unknown, kept verbatim. In the 2026 client it is 1 on every waypoint except the shops (type 13,
    /// "Shops"), whose 499 records hold 1 or one of 24 values in 10..=59, probably the shop kind. The consumer
    /// was not located.
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

    /// Encodes the `waypoint.bin` payload: the exact inverse of [`Self::from_payload_bytes`].
    ///
    /// # Panics
    /// When a section holds more records than its u16 count.
    pub fn to_payload_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            size_of::<u16>()
                + 4 * SECTION_COUNT_SIZE
                + (self.icon_definitions.len() + self.effect_definitions.len()) * CLILOC_DEFINITION_SIZE
                + self.type_definitions.len() * TYPE_DEFINITION_SIZE
                + self.waypoints.len() * WAYPOINT_RECORD_SIZE
                + self.trailing_bytes.len(),
        );
        bytes.extend_from_slice(&self.version.to_le_bytes());
        for definitions in [&self.icon_definitions, &self.effect_definitions] {
            push_count(&mut bytes, definitions.len());
            for definition in definitions {
                bytes.extend_from_slice(&definition.id.to_le_bytes());
                bytes.extend_from_slice(&definition.name_cliloc.to_le_bytes());
            }
        }
        push_count(&mut bytes, self.type_definitions.len());
        for definition in &self.type_definitions {
            bytes.extend_from_slice(&definition.name_cliloc.to_le_bytes());
            bytes.push(definition.flags);
            for link in &definition.links {
                bytes.extend_from_slice(&link.value_1.to_le_bytes());
                bytes.push(link.value_2);
            }
        }
        push_count(&mut bytes, self.waypoints.len());
        for waypoint in &self.waypoints {
            bytes.extend_from_slice(&waypoint.x.to_le_bytes());
            bytes.extend_from_slice(&waypoint.y.to_le_bytes());
            bytes.extend_from_slice(&waypoint.z.to_le_bytes());
            bytes.push(waypoint.facet);
            bytes.extend_from_slice(&waypoint.waypoint_type.to_le_bytes());
            bytes.extend_from_slice(&waypoint.value_6.to_le_bytes());
            bytes.extend_from_slice(&waypoint.name_cliloc.to_le_bytes());
        }
        bytes.extend_from_slice(&self.trailing_bytes);
        bytes
    }

    /// Replaces the `waypoint.bin` entry of a `waypoint.uop` package with this payload, keeping the entry's
    /// compression flag (`UopPackage::replace_file`): a changed entry gets a fresh timestamp data block header
    /// stamped with the package's edit FILETIME (also in this version-5 package), an unchanged one is left
    /// untouched. Fails when the package has no `waypoint.bin`.
    pub fn replace_in_package(&self, package: &mut UopPackage) -> eyre::Result<()> {
        let hash = [waypoints_payload_hash(), KNOWN_WAYPOINTS_PAYLOAD_HASH]
            .into_iter()
            .find(|hash| package.get_file_by_hash(*hash).is_some())
            .ok_or_else(|| eyre::eyre!("the package has no {WAYPOINTS_PAYLOAD_PATH}"))?;
        let compression = package.get_file_by_hash(hash).expect("the entry was just found").compression();
        package
            .replace_file(hash, &self.to_payload_bytes(), compression)
            .wrap_err_with(|| format!("failed to replace {WAYPOINTS_PAYLOAD_PATH}"))?;
        Ok(())
    }

    /// Writes a copy of the `source` `waypoint.uop` to `output` with `waypoint.bin` replaced
    /// ([`Self::replace_in_package`], stamped with the current time); the package keeps its version, block size and
    /// layout. `output` must not be `source`.
    pub fn write_waypoints_package(&self, source: impl AsRef<Path>, output: impl AsRef<Path>) -> eyre::Result<()> {
        let (source, output) = (source.as_ref(), output.as_ref());
        uop_entries::refuse_to_overwrite(source, output)?;
        let mut package = UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
        self.replace_in_package(&mut package)
            .wrap_err_with(|| format!("failed to replace {WAYPOINTS_PAYLOAD_PATH} in {}", source.display()))?;
        uop_entries::save_package_as(&mut package, source, output)
    }
}

fn push_count(bytes: &mut Vec<u8>, count: usize) {
    let count = u16::try_from(count).expect("waypoint section counts fit the u16 of the format");
    bytes.extend_from_slice(&count.to_le_bytes());
}

pub fn waypoints_payload_hash() -> u64 {
    hash_file_name_single(WAYPOINTS_PAYLOAD_PATH)
}

fn read_count(reader: &mut Cursor<&[u8]>, bytes: &[u8], section: &str) -> eyre::Result<usize> {
    require_remaining(reader, bytes, SECTION_COUNT_SIZE, section)?;
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
        require_remaining(reader, bytes, CLILOC_DEFINITION_SIZE, section)?;
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
        require_remaining(reader, bytes, TYPE_DEFINITION_SIZE, section)?;
        let name_cliloc = reader.read_u32::<LittleEndian>()?;
        let flags = reader.read_u8()?;
        let mut links = [WaypointTypeLink {
            value_1: 0,
            value_2: 0,
        }; WAYPOINT_TYPE_LINK_COUNT];
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
        require_remaining(reader, bytes, WAYPOINT_RECORD_SIZE, section)?;
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
    fn to_payload_bytes_is_the_inverse_of_the_parser() {
        let payload = sample_payload();
        let package = WaypointsPackage::from_payload_bytes(&payload).unwrap();
        assert_eq!(package.to_payload_bytes(), payload);

        let mut edited = package.clone();
        let mut added = edited.waypoints[0];
        added.name_cliloc += 1;
        edited.waypoints.push(added);
        edited.trailing_bytes.clear();
        let reparsed = WaypointsPackage::from_payload_bytes(&edited.to_payload_bytes()).unwrap();
        assert_eq!(reparsed, edited);
    }

    #[test]
    fn replace_in_package_keeps_the_compression_and_needs_the_entry() {
        use crate::uop_container::file::CompressionFlag;
        let mut package = UopPackage::new_default();
        package.add_file_from_memory(&sample_payload(), WAYPOINTS_PAYLOAD_PATH, CompressionFlag::Zlib).unwrap();
        let mut waypoints = WaypointsPackage::from_package(&package).unwrap();
        waypoints.waypoints[0].x += 1;
        waypoints.replace_in_package(&mut package).unwrap();
        let file = package.get_file_by_hash(waypoints_payload_hash()).unwrap();
        assert_eq!(file.compression(), CompressionFlag::Zlib);
        assert_eq!(WaypointsPackage::from_package(&package).unwrap(), waypoints);
        assert!(waypoints.replace_in_package(&mut UopPackage::new_default()).is_err());
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
