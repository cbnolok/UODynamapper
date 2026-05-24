const MAP_METADATA_MAGIC: u32 = u32::from_le_bytes(*b"UMAP");
const MAP_METADATA_VERSION: u16 = 1;
const MAP_METADATA_SIZE: usize = 24;

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
    bytes[4..6].copy_from_slice(&MAP_METADATA_VERSION.to_le_bytes());
    bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
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

    let version = u16::from_le_bytes(bytes[4..6].try_into().expect("slice length checked"));
    if version != MAP_METADATA_VERSION {
        return Err(format!("unsupported map metadata version {version}"));
    }

    Ok(MapPackageMetadata {
        map_id: u32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")),
        width_tiles: u32::from_le_bytes(bytes[12..16].try_into().expect("slice length checked")),
        height_tiles: u32::from_le_bytes(bytes[16..20].try_into().expect("slice length checked")),
        chunk_count: u32::from_le_bytes(bytes[20..24].try_into().expect("slice length checked")),
    })
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
}
