//! The Classic -> Kingdom Reborn land mapping the KR facet encoder needs: for each Classic land id, the KR
//! material (`TerrainDefinition` id) and the `subtype` (alias of the material) a KR square stores with it.
//!
//! Decoding needs no mapping: KR squares store their Classic id ([`KrSquareLand::classic_land`]).
//!
//! The bundled `assets/kr/mappings.bin` is generated from the shipped KR facets (`UO - Old KR`, facets 0-4, 69,746,240 squares):
//! for each Classic id, the most frequent `material` and the most frequent `subtype` (ties to the smaller value).
//! It is exact for `material` on 99.9995% of the shipped squares (only Classic 2444 and 2446 take two KR ids) and
//! for `subtype` on 99.967% (64 Classic ids take more than one value); the ignored test
//! `kr_facet_sectors_reencode_identically_and_follow_the_rules` regenerates and checks it.
//! The separate `assets/kr/mappings_unverified.bin` adds the
//! Classic ids the KR facets never use, from Manawydan's KR converter table (`tile_mappings_builder`, feature
//! `tile_mappings_builder`); they are unverified.

use std::collections::{BTreeMap, HashMap};

use crate::enhanced::facet_decoder::{DecodedEcFacet, KrSquareLand};

crate::eyre_imports!();

const SHIPPED_KR_LAND_BINARY: &[u8] = include_bytes!("../../../assets/kr/mappings.bin");

const UNVERIFIED_KR_LAND_BINARY: &[u8] = include_bytes!("../../../assets/kr/mappings_unverified.bin");

/// How a KR square stores one Classic land id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KrLand {
    pub material: u16,
    pub subtype: u8,
}

/// A Classic -> KR land mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KrLandMap {
    map: HashMap<u16, KrLand>,
    /// What [`Self::square_land`] stores for a Classic id the map lacks; `None` fails instead.
    fallback: Option<KrLand>,
}

impl KrLandMap {
    /// The bundled verified binary plus the separate, explicitly unverified binary supplement.
    pub fn shipped() -> Self {
        let mut map = Self::shipped_verified();
        let supplement = Self::from_binary(UNVERIFIED_KR_LAND_BINARY)
            .expect("bundled unverified KR land mapping must be valid KRLM");
        for (classic, land) in supplement.map {
            map.map.entry(classic).or_insert(land);
        }
        map
    }

    /// Only the ids verified against the shipped facets, read from the bundled KRLM binary.
    pub fn shipped_verified() -> Self {
        Self::from_binary(SHIPPED_KR_LAND_BINARY).expect("bundled KR land mapping must be valid KRLM")
    }

    /// Reads a KRLM v1 binary. Rejects unsupported headers, length mismatches,
    /// duplicates and unsorted ids. No fallback is stored in this format.
    pub fn from_binary(bytes: &[u8]) -> eyre::Result<Self> {
        const HEADER_SIZE: usize = 12;
        const RECORD_SIZE: usize = 5;
        eyre::ensure!(bytes.len() >= HEADER_SIZE, "KRLM header is truncated");
        eyre::ensure!(&bytes[..4] == b"KRLM", "invalid KRLM magic");
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        eyre::ensure!(version == 1, "unsupported KRLM version {version}");
        let record_size = u16::from_le_bytes([bytes[6], bytes[7]]);
        eyre::ensure!(usize::from(record_size) == RECORD_SIZE, "invalid KRLM record size {record_size}");
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        eyre::ensure!(count <= 65_536, "KRLM record count exceeds the Classic id range");
        eyre::ensure!(bytes.len() == HEADER_SIZE + count as usize * RECORD_SIZE, "KRLM record count/length mismatch");
        let mut map = Self::default();
        let mut previous = None;
        for row in bytes[HEADER_SIZE..].chunks_exact(RECORD_SIZE) {
            let classic = u16::from_le_bytes([row[0], row[1]]);
            eyre::ensure!(previous.is_none_or(|id| id < classic), "KRLM ids must be strictly ascending: {classic}");
            let material = u16::from_le_bytes([row[2], row[3]]);
            map.insert(classic, KrLand { material, subtype: row[4] });
            previous = Some(classic);
        }
        Ok(map)
    }

    /// Writes sorted KRLM v1 records. The optional fallback is not serialized.
    pub fn to_binary(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12 + self.len() * 5);
        bytes.extend_from_slice(b"KRLM");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for (classic, material, subtype) in self.rows() {
            bytes.extend_from_slice(&classic.to_le_bytes());
            bytes.extend_from_slice(&material.to_le_bytes());
            bytes.push(subtype);
        }
        bytes
    }

    /// Makes [`Self::square_land`] store `fallback` (with the Classic id itself) for the Classic ids the map lacks,
    /// such as the land tiles added after the KR release, instead of failing.
    pub fn with_fallback(mut self, fallback: KrLand) -> Self {
        self.fallback = Some(fallback);
        self
    }

    pub fn get(&self, classic_land: u16) -> Option<KrLand> {
        self.map.get(&classic_land).copied()
    }

    pub fn insert(&mut self, classic_land: u16, land: KrLand) -> Option<KrLand> {
        self.map.insert(classic_land, land)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The rows by Classic id, as `(classic land id, material, subtype)`.
    pub fn rows(&self) -> Vec<(u16, u16, u8)> {
        let mut rows: Vec<_> =
            self.map.iter().map(|(&classic, land)| (classic, land.material, land.subtype)).collect();
        rows.sort_unstable();
        rows
    }

    /// The land id and KR square bytes the encoder stores for `classic_land`. Fails on an id the map lacks, unless
    /// it has a fallback ([`Self::with_fallback`]).
    pub fn square_land(&self, classic_land: u16) -> eyre::Result<(u16, Option<KrSquareLand>)> {
        let land = self
            .get(classic_land)
            .or(self.fallback)
            .ok_or_else(|| eyre!("Classic land id {classic_land} has no Kingdom Reborn material in the land map"))?;
        Ok((land.material, Some(KrSquareLand { subtype: land.subtype, classic_land })))
    }
}

/// Counts the `(material, subtype)` stored with each Classic id over KR sectors, to derive a [`KrLandMap`].
#[derive(Debug, Clone, Default)]
pub struct KrLandCounter {
    material: BTreeMap<u16, BTreeMap<u16, u64>>,
    subtype: BTreeMap<u16, BTreeMap<u8, u64>>,
}

impl KrLandCounter {
    /// Counts the squares of `sector` for which `on_map(square index)` holds (pass the facet geometry check to
    /// leave out the padding squares).
    pub fn add_sector(&mut self, sector: &DecodedEcFacet, mut on_map: impl FnMut(usize) -> bool) {
        for (index, tile) in sector.tiles.iter().enumerate() {
            let Some(kr) = tile.kr else {
                continue;
            };
            if !on_map(index) {
                continue;
            }
            *self.material.entry(kr.classic_land).or_default().entry(tile.land_graphic).or_default() += 1;
            *self.subtype.entry(kr.classic_land).or_default().entry(kr.subtype).or_default() += 1;
        }
    }

    /// The most frequent `material` and `subtype` of each Classic id (ties to the smaller value).
    pub fn to_map(&self) -> KrLandMap {
        fn most_frequent<T: Copy + Ord>(counts: &BTreeMap<T, u64>) -> T {
            // BTreeMap iterates ascending, and max_by_key keeps the last maximum: reverse to keep the smallest.
            *counts.iter().rev().max_by_key(|(_, count)| **count).expect("counted ids have a value").0
        }
        let mut map = KrLandMap::default();
        for (&classic, kr_lands) in &self.material {
            let subtype = most_frequent(&self.subtype[&classic]);
            map.insert(classic, KrLand { material: most_frequent(kr_lands), subtype });
        }
        map
    }

    /// `(squares, squares whose material, squares whose subtype equal the map's)` over the counted squares.
    pub fn agreement(&self, map: &KrLandMap) -> (u64, u64, u64) {
        let (mut total, mut material_ok, mut subtype_ok) = (0, 0, 0);
        for (classic, kr_lands) in &self.material {
            let land = map.get(*classic);
            for (material, count) in kr_lands {
                total += count;
                if land.is_some_and(|land| land.material == *material) {
                    material_ok += count;
                }
            }
            for (subtype, count) in &self.subtype[classic] {
                if land.is_some_and(|land| land.subtype == *subtype) {
                    subtype_ok += count;
                }
            }
        }
        (total, material_ok, subtype_ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_binary_matches_observation_evidence() {
        let mut counter = KrLandCounter::default();
        let evidence = include_str!("../../../assets/kr/observations.csv");
        let mut reader = csv::Reader::from_reader(evidence.as_bytes());
        for row in reader.deserialize::<(u16, u16, u8, u64)>() {
            let (classic, material, subtype, count) = row.unwrap();
            *counter.material.entry(classic).or_default().entry(material).or_default() += count;
            *counter.subtype.entry(classic).or_default().entry(subtype).or_default() += count;
        }
        let verified = KrLandMap::shipped_verified();
        assert_eq!(verified, counter.to_map());
        assert_eq!(counter.agreement(&verified), (69_746_240, 69_745_894, 69_723_228));
        assert_eq!(verified.to_binary(), SHIPPED_KR_LAND_BINARY);
    }

    #[test]
    fn binary_round_trip_preserves_ids_and_excludes_fallback() {
        let mut map = KrLandMap::default();
        map.insert(u16::MAX, KrLand { material: u16::MAX, subtype: u8::MAX });
        map.insert(0, KrLand { material: 7, subtype: 2 });
        let bytes = map.to_binary();
        assert_eq!(KrLandMap::from_binary(&bytes).unwrap(), map);
        assert_eq!(KrLandMap::default(), KrLandMap::from_binary(&KrLandMap::default().to_binary()).unwrap());
        map = map.with_fallback(KrLand { material: 3, subtype: 1 });
        assert_eq!(map.to_binary(), bytes);
        assert!(KrLandMap::from_binary(&bytes).unwrap().square_land(1).is_err());
    }

    #[test]
    fn binary_rejects_invalid_headers_lengths_and_order() {
        let bytes = SHIPPED_KR_LAND_BINARY;
        for len in [0, 4, 11, bytes.len() - 1] {
            assert!(KrLandMap::from_binary(&bytes[..len]).is_err());
        }
        for (offset, value) in [(0, b'X'), (4, 2), (6, 6), (8, 0), (11, 1)] {
            let mut invalid = bytes.to_vec();
            invalid[offset] = value;
            assert!(KrLandMap::from_binary(&invalid).is_err(), "header byte {offset}");
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(KrLandMap::from_binary(&trailing).is_err());
        let mut duplicate = bytes.to_vec();
        duplicate[17..19].copy_from_slice(&bytes[12..14]);
        assert!(KrLandMap::from_binary(&duplicate).is_err());
        let mut unsorted = bytes.to_vec();
        unsorted[12..17].copy_from_slice(&bytes[17..22]);
        unsorted[17..22].copy_from_slice(&bytes[12..17]);
        assert!(KrLandMap::from_binary(&unsorted).is_err());
    }

    #[test]
    fn bundled_binaries_are_disjoint_and_preserve_coverage() {
        let verified = KrLandMap::shipped_verified();
        let unverified = KrLandMap::from_binary(UNVERIFIED_KR_LAND_BINARY).unwrap();
        assert_eq!(verified.len(), 1556);
        assert_eq!(unverified.len(), 1133);
        assert_eq!(unverified.to_binary(), UNVERIFIED_KR_LAND_BINARY);
        let combined = KrLandMap::shipped();
        for (classic, material, subtype) in unverified.rows() {
            assert!(verified.get(classic).is_none());
            assert_eq!(combined.get(classic), Some(KrLand { material, subtype }));
        }
        for (classic, material, subtype) in verified.rows() {
            assert_eq!(combined.get(classic), Some(KrLand { material, subtype }));
        }
        assert_eq!(combined.len(), verified.len() + unverified.len());
    }

    #[test]
    fn square_land_stores_the_classic_id() {
        const CLASSIC_GRASS: u16 = 3;
        let map = KrLandMap::shipped();
        let (material, kr) = map.square_land(CLASSIC_GRASS).unwrap();
        assert_eq!(material, map.get(CLASSIC_GRASS).unwrap().material);
        assert_eq!(kr.unwrap().classic_land, CLASSIC_GRASS);
        assert!(KrLandMap::default().square_land(CLASSIC_GRASS).is_err());
        let fallback = KrLand { material: 0, subtype: 0 };
        let (material, kr) = KrLandMap::default().with_fallback(fallback).square_land(CLASSIC_GRASS).unwrap();
        assert_eq!((material, kr.unwrap().classic_land), (fallback.material, CLASSIC_GRASS));
    }

    #[test]
    fn counter_takes_the_most_frequent_value() {
        let mut sector = crate::enhanced::facet_sector::empty_sector(
            0,
            0,
            crate::enhanced::client_dialect::ClientDialect::KingdomReborn,
        )
        .unwrap();
        for (index, tile) in sector.tiles.iter_mut().take(3).enumerate() {
            tile.land_graphic = if index == 0 { 9 } else { 8 };
            tile.kr = Some(KrSquareLand { subtype: if index == 2 { 1 } else { 2 }, classic_land: 5 });
        }
        let mut counter = KrLandCounter::default();
        counter.add_sector(&sector, |index| index < 3);
        let map = counter.to_map();
        assert_eq!(map.get(5), Some(KrLand { material: 8, subtype: 2 }));
        assert_eq!(counter.agreement(&map), (3, 2, 2));
    }
}
