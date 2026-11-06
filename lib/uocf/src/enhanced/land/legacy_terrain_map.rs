//! Original EC land routing from `GameData.uop`, including authored subtypes.
//!
//! The shipped table (`data/gamedata/legacyterrainmap.csv`, 2,705 rows) is the header `legacyId,newId,newSubType`
//! and one `legacyId,newId,newSubType` row per land id, plain decimal, every line ending in CRLF, no BOM, no blank
//! line. Its rows are in authored order (ascending with four out-of-order runs, e.g. 1309 after 16379), so the model
//! keeps that order next to the routes. [`LegacyTerrainMap::to_csv_bytes`] writes that format and gives the
//! shipped table back byte for byte; [`check_canonical`] tells whether a CSV is already in it.

use std::collections::BTreeMap;
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use serde::Deserialize;
use crate::uop_container::{hash::hash_file_name_single, package::UopPackage};

pub const UOP_ENTRY_PATH: &str = "data/gamedata/legacyterrainmap.csv";

/// The header line of the shipped table, without its line ending.
pub const CSV_HEADER: &str = "legacyId,newId,newSubType";
/// The line ending of every shipped line.
pub const CSV_LINE_ENDING: &str = "\r\n";

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
    /// Land ids in row order: the source order, then ids added by [`Self::set_route`]. Routes missing here are
    /// written after these, ascending; ids here without a route are skipped.
    pub row_order: Vec<u16>,
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
        let mut row_order = Vec::new();
        for row in reader.deserialize::<LegacyTerrainRoute>() {
            let route = row.wrap_err("invalid legacyterrainmap.csv row")?;
            match routes.insert(route.legacy_id, route) {
                Some(previous) if previous != route => {
                    eyre::bail!("conflicting legacyterrainmap.csv routes for land id {}", route.legacy_id);
                }
                Some(_) => {}
                None => row_order.push(route.legacy_id),
            }
        }
        if routes.is_empty() {
            eyre::bail!("legacyterrainmap.csv contains no routes");
        }
        Ok(Self { routes, row_order, source_csv: bytes.to_vec() })
    }

    /// Sets the route of its land id; a new id is appended to the row order. Returns the previous route.
    pub fn set_route(&mut self, route: LegacyTerrainRoute) -> Option<LegacyTerrainRoute> {
        let previous = self.routes.insert(route.legacy_id, route);
        if previous.is_none() && !self.row_order.contains(&route.legacy_id) {
            self.row_order.push(route.legacy_id);
        }
        previous
    }

    /// Removes the route of `legacy_id`; returns it.
    pub fn remove_route(&mut self, legacy_id: u16) -> Option<LegacyTerrainRoute> {
        self.row_order.retain(|&id| id != legacy_id);
        self.routes.remove(&legacy_id)
    }

    /// The routes in row order: [`Self::row_order`], then the routes it misses, ascending; each id once.
    pub fn ordered_routes(&self) -> Vec<LegacyTerrainRoute> {
        let mut seen = std::collections::BTreeSet::new();
        let listed = self.row_order.iter().filter_map(|id| self.routes.get(id));
        let rest = self.routes.values().filter(|route| !self.row_order.contains(&route.legacy_id));
        listed.chain(rest).filter(|route| seen.insert(route.legacy_id)).copied().collect()
    }

    /// The table in the shipped format: [`CSV_HEADER`], then `legacyId,newId,newSubType` per route in
    /// [`Self::ordered_routes`] order, every line ending in [`CSV_LINE_ENDING`]. Gives the shipped table back byte
    /// for byte.
    pub fn to_csv_bytes(&self) -> Vec<u8> {
        let mut text = String::with_capacity(self.source_csv.len().max(CSV_HEADER.len() + CSV_LINE_ENDING.len()));
        text.push_str(CSV_HEADER);
        text.push_str(CSV_LINE_ENDING);
        for route in self.ordered_routes() {
            use std::fmt::Write as _;
            write!(text, "{},{},{}{CSV_LINE_ENDING}", route.legacy_id, route.material_id, route.subtype).unwrap();
        }
        text.into_bytes()
    }
}

/// Checks that `bytes` parse as a land routing table and are in the shipped format (the bytes
/// [`LegacyTerrainMap::to_csv_bytes`] writes for them): fails naming the first line that differs, e.g. for spaces,
/// LF-only line endings, a BOM, leading zeros, a repeated row or a missing final line ending.
pub fn check_canonical(bytes: &[u8]) -> eyre::Result<LegacyTerrainMap> {
    let map = LegacyTerrainMap::from_bytes(bytes)?;
    let canonical = map.to_csv_bytes();
    if canonical != bytes {
        let source_lines = bytes.split_inclusive(|&byte| byte == b'\n');
        let canonical_lines = canonical.split_inclusive(|&byte| byte == b'\n');
        let (line, found, expected) = source_lines
            .map(Some)
            .chain(std::iter::repeat(None))
            .zip(canonical_lines.map(Some).chain(std::iter::repeat(None)))
            .enumerate()
            .find(|(_, (found, expected))| found != expected)
            .map(|(index, (found, expected))| (index + 1, found, expected))
            .expect("the bytes differ");
        let show = |line: Option<&[u8]>| line.map_or_else(|| "nothing".to_string(), |line| format!("{:?}", String::from_utf8_lossy(line)));
        eyre::bail!(
            "legacyterrainmap.csv is not in the shipped format at line {line}: found {}, expected {}",
            show(found),
            show(expected)
        );
    }
    Ok(map)
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
    fn writer_keeps_the_row_order_and_the_shipped_format() {
        let bytes = b"legacyId,newId,newSubType\r\n1,0,1\r\n16379,4,0\r\n1309,69,1\r\n";
        let mut map = check_canonical(bytes).unwrap();
        assert_eq!(map.to_csv_bytes(), bytes);
        map.set_route(LegacyTerrainRoute { legacy_id: 5, material_id: 7, subtype: 0 });
        map.set_route(LegacyTerrainRoute { legacy_id: 1, material_id: 2, subtype: 0 });
        map.remove_route(16379);
        assert_eq!(map.to_csv_bytes(), b"legacyId,newId,newSubType\r\n1,2,0\r\n1309,69,1\r\n5,7,0\r\n");
        map.row_order.clear();
        assert_eq!(map.to_csv_bytes(), b"legacyId,newId,newSubType\r\n1,2,0\r\n5,7,0\r\n1309,69,1\r\n");
    }

    #[test]
    fn canonical_check_names_the_first_differing_line() {
        let cases: [(&[u8], &str); 5] = [
            (b"legacyId,newId,newSubType\n1,0,1\n", "line 1"),
            (b"legacyId,newId,newSubType\r\n1, 0,1\r\n", "line 2"),
            (b"legacyId,newId,newSubType\r\n1,0,1\r\n2,00,1\r\n", "line 3"),
            (b"legacyId,newId,newSubType\r\n1,0,1\r\n1,0,1\r\n", "line 3: found \"1,0,1\\r\\n\", expected nothing"),
            (b"legacyId,newId,newSubType\r\n1,0,1", "line 2"),
        ];
        for (bytes, expected) in cases {
            let error = check_canonical(bytes).unwrap_err().to_string();
            assert!(error.contains(expected), "{:?}: {error}", String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn conflicting_or_malformed_routes_are_rejected() {
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n3,1,0\n3,4,0\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n65536,1,0\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId,newSubType\n").is_err());
        assert!(LegacyTerrainMap::from_bytes(b"legacyId,newId\n3,1\n").is_err());
    }
}
