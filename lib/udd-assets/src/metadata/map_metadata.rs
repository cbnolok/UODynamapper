//! Final dense-id metadata records of the map and statics packages.
//!
//! Both records version their whole package: a map record covers the 4096-byte `Rg16u` chunks
//! before it, a statics record covers the `StaticTile` chunk arrays before it.

use udd_container::{unpack_type, DataType, FileKey, UddpReader};

const MAP_METADATA_MAGIC: u32 = u32::from_le_bytes(*b"UMAP");
/// Version of the map package layout, chunks included.
pub const MAP_METADATA_VERSION: u32 = 1;
const MAP_METADATA_SIZE: usize = 24;

const STATICS_METADATA_MAGIC: u32 = u32::from_le_bytes(*b"USTA");
/// Version of the statics package layout, chunks included.
pub const STATICS_METADATA_VERSION: u32 = 1;
pub const STATICS_METADATA_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapPackageMetadata {
    pub map_id: u32,
    pub width_tiles: u32,
    pub height_tiles: u32,
    pub chunk_count: u32,
}

impl MapPackageMetadata {
    pub const fn new(map_id: u32, width_tiles: u32, height_tiles: u32, chunk_count: u32) -> Self {
        Self {
            map_id,
            width_tiles,
            height_tiles,
            chunk_count,
        }
    }
}

pub fn encode_map_package_metadata(metadata: MapPackageMetadata) -> [u8; MAP_METADATA_SIZE] {
    let mut bytes = [0u8; MAP_METADATA_SIZE];
    bytes[0..4].copy_from_slice(&MAP_METADATA_MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&MAP_METADATA_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&metadata.map_id.to_le_bytes());
    bytes[12..16].copy_from_slice(&metadata.width_tiles.to_le_bytes());
    bytes[16..20].copy_from_slice(&metadata.height_tiles.to_le_bytes());
    bytes[20..24].copy_from_slice(&metadata.chunk_count.to_le_bytes());
    bytes
}

pub fn decode_map_package_metadata(bytes: &[u8]) -> Result<MapPackageMetadata, String> {
    if bytes.len() != MAP_METADATA_SIZE {
        return Err(format!(
            "invalid map metadata size {}, expected {}",
            bytes.len(),
            MAP_METADATA_SIZE
        ));
    }

    let magic = u32::from_le_bytes(bytes[0..4].try_into().expect("slice length checked"));
    if magic != MAP_METADATA_MAGIC {
        return Err("invalid map metadata magic".to_string());
    }

    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked"));
    if version != MAP_METADATA_VERSION {
        return Err(format!(
            "map metadata: package version {version} is not supported (current is {MAP_METADATA_VERSION}); \
             repack with the current udd-cli"
        ));
    }

    Ok(MapPackageMetadata {
        map_id: u32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")),
        width_tiles: u32::from_le_bytes(bytes[12..16].try_into().expect("slice length checked")),
        height_tiles: u32::from_le_bytes(bytes[16..20].try_into().expect("slice length checked")),
        chunk_count: u32::from_le_bytes(bytes[20..24].try_into().expect("slice length checked")),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticsPackageMetadata {
    pub map_id: u32,
    pub chunk_count: u32,
}

pub fn encode_statics_package_metadata(metadata: StaticsPackageMetadata) -> [u8; STATICS_METADATA_SIZE] {
    let mut bytes = [0u8; STATICS_METADATA_SIZE];
    bytes[0..4].copy_from_slice(&STATICS_METADATA_MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&STATICS_METADATA_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&metadata.map_id.to_le_bytes());
    bytes[12..16].copy_from_slice(&metadata.chunk_count.to_le_bytes());
    bytes
}

pub fn decode_statics_package_metadata(bytes: &[u8]) -> Result<StaticsPackageMetadata, String> {
    let word = |index: usize| u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().expect("slice length checked"));
    if bytes.len() != STATICS_METADATA_SIZE || word(0) != STATICS_METADATA_MAGIC {
        return Err(format!(
            "statics metadata: no {STATICS_METADATA_SIZE}-byte USTA record; repack with the current udd-cli"
        ));
    }
    let version = word(1);
    if version != STATICS_METADATA_VERSION {
        return Err(format!(
            "statics metadata: package version {version} is not supported (current is {STATICS_METADATA_VERSION}); \
             repack with the current udd-cli"
        ));
    }
    Ok(StaticsPackageMetadata { map_id: word(2), chunk_count: word(3) })
}

/// Reads the final metadata record of a statics package and checks that it declares every chunk before it.
pub fn read_statics_package_metadata(package: &UddpReader) -> Result<StaticsPackageMetadata, String> {
    let records = package.records();
    let record = records.last().ok_or("statics package has no records")?;
    let FileKey::Id(id) = record.key else {
        return Err("statics package metadata record is not id addressed".into());
    };
    if unpack_type(record.locator.meta32) != DataType::Metadata as u8 || id as usize + 1 != records.len() {
        return Err("statics package has no final metadata record; repack with the current udd-cli".into());
    }
    let bytes = package.read_file_by_dense_id(id).map_err(|error| format!("read statics metadata: {error}"))?;
    let metadata = decode_statics_package_metadata(&bytes)?;
    if metadata.chunk_count != id {
        return Err(format!(
            "statics metadata declares {} chunks, the package has {id}",
            metadata.chunk_count
        ));
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_package_metadata_round_trips() {
        let metadata = MapPackageMetadata::new(0, 7168, 4096, 229_376);

        let bytes = encode_map_package_metadata(metadata);

        assert_eq!(decode_map_package_metadata(&bytes), Ok(metadata));
    }

    /// The u32 version field keeps the version 1 bytes of the former u16 version and zero reserved field.
    #[test]
    fn map_package_metadata_version_is_a_u32() {
        let bytes = encode_map_package_metadata(MapPackageMetadata::new(0, 8, 8, 1));
        assert_eq!(&bytes[4..8], &[1, 0, 0, 0]);
        let mut other = bytes;
        other[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_map_package_metadata(&other).unwrap_err().contains("repack with the current udd-cli"));
    }

    #[test]
    fn statics_package_metadata_round_trips_and_rejects_other_versions() {
        let metadata = StaticsPackageMetadata { map_id: 2, chunk_count: 3_200 };
        let bytes = encode_statics_package_metadata(metadata);
        assert_eq!(&bytes[0..4], b"USTA");
        assert_eq!(decode_statics_package_metadata(&bytes), Ok(metadata));

        let mut other = bytes;
        other[4..8].copy_from_slice(&(STATICS_METADATA_VERSION + 1).to_le_bytes());
        assert!(decode_statics_package_metadata(&other).unwrap_err().contains("repack with the current udd-cli"));
        assert!(decode_statics_package_metadata(&bytes[..8]).unwrap_err().contains("repack with the current udd-cli"));
    }
}
