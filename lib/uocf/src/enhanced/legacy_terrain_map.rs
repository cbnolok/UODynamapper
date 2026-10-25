//! Original EC land routing from `GameData.uop`, including authored subtypes.

use std::collections::BTreeMap;
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use serde::Deserialize;
use crate::uop_container::{hash::hash_file_name_single, package::UopPackage};

pub const UOP_ENTRY_PATH: &str = "data/gamedata/legacyterrainmap.csv";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct LegacyTerrainRoute {
    #[serde(rename = "legacyId")]
    pub legacy_id: u16,
    #[serde(rename = "newId")]
    pub material_id: u32,
    #[serde(rename = "newSubType")]
    pub subtype: u32,
}

#[derive(Debug, Clone)]
pub struct LegacyTerrainMap {
    pub routes: BTreeMap<u16, LegacyTerrainRoute>,
    /// Retained verbatim for package provenance, including the source header.
    pub source_csv: Vec<u8>,
}

impl LegacyTerrainMap {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load(path.as_ref())?;
        let file = package.get_file_by_hash(hash_file_name_single(UOP_ENTRY_PATH))
            .ok_or_else(|| eyre::eyre!("{} missing from {}", UOP_ENTRY_PATH, path.as_ref().display()))?;
        Self::from_bytes(&file.unpack()?)
    }

    pub fn from_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(bytes);
        let mut routes = BTreeMap::new();
        for row in reader.deserialize::<LegacyTerrainRoute>() {
            let route = row.wrap_err("invalid legacyterrainmap.csv row")?;
            if let Some(previous) = routes.insert(route.legacy_id, route) {
                if previous != route {
                    eyre::bail!("conflicting legacyterrainmap.csv routes for land id {}", route.legacy_id);
                }
            }
        }
        if routes.is_empty() {
            eyre::bail!("legacyterrainmap.csv contains no routes");
        }
        Ok(Self { routes, source_csv: bytes.to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_routes_preserve_void_material_and_subtypes() {
        let bytes = b"legacyId,newId,newSubType\r\n1,0,1\r\n203,4,0\r\n15853,69,1\r\n";
        let map = LegacyTerrainMap::from_bytes(bytes).unwrap();
        assert_eq!(map.routes[&1].material_id, 0);
        assert_eq!(map.routes[&1].subtype, 1);
        assert_eq!(map.routes[&203].material_id, 4);
        assert_eq!(map.routes[&15853].subtype, 1);
        assert_eq!(map.source_csv, bytes);
    }

    #[test]
    fn conflicting_or_malformed_routes_are_rejected() {
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n3,1,0\n3,4,0\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n65536,1,0\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId\n3,1\n").is_err());
    }
}
