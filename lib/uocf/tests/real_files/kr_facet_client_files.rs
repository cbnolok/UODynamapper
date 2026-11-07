//! Checks the Kingdom Reborn facet codecs on a real KR client (`UOKR.exe`, 2008): every sector of `facet0..4.uop`
//! decodes and re-encodes byte for byte, its delimiters follow the shipped rule, the generated land map matches
//! the facets, and facets built from a Classic client compare with the shipped ones. `StaticChunk.uop` is checked
//! here as well.
//!
//! The KR client directory comes from `common::KR_CLIENT_DIR_ENV`, the Classic one (`map*.mul`, `staidx*.mul`,
//! `statics*.mul`, `radarcol.mul`) from `common::KR_CLASSIC_DIR_ENV`. Run with `--release`. Files written here go to
//! `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted.

use crate::common;

use std::path::PathBuf;

use rayon::prelude::*;
use uocf::classic::map::{MapPlane, MapSizeCells};
use uocf::classic::statics::StaticsReader;
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::facet_decoder::DecodedEcFacet;
use uocf::enhanced::facet_package::{
    detect_facet_dialect, EcFacetPackage, KR_FACET_PACKAGE_BLOCK_SIZE, KR_FACET_PACKAGE_VERSION,
};
use uocf::enhanced::facet_radar::RadarPalette;
use uocf::enhanced::facet_sector::{
    data_map_entry_path, encode_sector, expected_delimiters, facet_package_file_name, sector_entry_path,
    validate_sector, FacetGeometry, SECTOR_SIDE,
};
use uocf::kr::facet::facet_decoder::read_facet_block_raw;
use uocf::kr::facet::facet_encoder::build_kr_facet_from_classic;
use uocf::kr::facet::kr_land_map::{KrLandCounter, KrLandMap};
use uocf::kr::facet::static_chunk::{StaticChunkPackage, STATIC_CHUNK_UOP_NAME};
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

/// The facets the KR client ships (Felucca, Trammel, Ilshenar, Malas, Tokuno).
const KR_FACETS: std::ops::RangeInclusive<u8> = 0..=4;
/// Sectors of each shipped KR facet package.
const KR_SECTOR_COUNTS: [u32; 5] = [7168, 7168, 900, 1280, 529];
/// Entries and records of the shipped `StaticChunk.uop`.
const KR_STATIC_CHUNKS: usize = 145;
const KR_STATIC_CHUNK_RECORDS: usize = 60_713;
const RADARCOL_NAME: &str = "radarcol.mul";
/// The Classic id whose KR land the build stores for Classic ids newer than the KR land map (the shipped KR facets
/// store Classic 2, a no-draw tile, as KR terrain 0).
const CLASSIC_NODRAW_LAND: u16 = 2;

fn kr_facet_package(facet: u8) -> PathBuf {
    common::kr_client_file(&facet_package_file_name(facet, false))
}

/// The land of every on-map square of `sector`, as `(x, y, z, material)`.
fn sector_land(geometry: &FacetGeometry, sector: u32, decoded: &DecodedEcFacet) -> Vec<(u32, u32, i8, u16)> {
    let (ox, oy) = geometry.sector_origin(sector);
    decoded
        .tiles
        .iter()
        .enumerate()
        .map(|(index, tile)| (ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE, tile.z, tile.land_graphic))
        .filter(|&(x, y, _, _)| geometry.contains(x, y))
        .collect()
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_facet_sectors_reencode_identically_and_follow_the_rules() {
    let mut counter = KrLandCounter::default();
    for facet in KR_FACETS {
        let path = kr_facet_package(facet);
        let package = UopPackage::load(&path).expect("load KR facet package");
        assert_eq!((package.version(), package.block_size()), (KR_FACET_PACKAGE_VERSION, KR_FACET_PACKAGE_BLOCK_SIZE));
        let data_map_hash = hash_file_name_single(&data_map_entry_path(facet));
        let first = package.iter_files().next().expect("an entry");
        assert_eq!((first.filename_hash(), first.compression()), (data_map_hash, CompressionFlag::None), "data.map first");
        let data_map = package.unpack_file_by_hash(data_map_hash).unwrap().expect("data.map");
        let geometry = FacetGeometry::from_data_map_payload(&data_map).unwrap();
        assert_eq!(geometry.sector_count(), KR_SECTOR_COUNTS[usize::from(facet)], "facet {facet} sectors");
        assert_eq!(detect_facet_dialect(&package, facet).unwrap(), ClientDialect::KingdomReborn);

        // Pass 1: decoding with the header check, byte-identical re-encoding, and the land for the delimiters.
        let decoded: Vec<DecodedEcFacet> = (0..geometry.sector_count())
            .into_par_iter()
            .map(|sector| {
                let entry = sector_entry_path(facet, sector);
                let payload = package.unpack_file_by_hash(hash_file_name_single(&entry)).unwrap().expect(&entry);
                let file = package.get_file_by_hash(hash_file_name_single(&entry)).unwrap();
                assert_eq!(file.compression(), CompressionFlag::Zlib, "{entry}");
                let decoded = read_facet_block_raw(&package, facet, sector).unwrap_or_else(|e| panic!("{entry}: {e:#}"));
                let issues = validate_sector(&decoded, Some((facet, sector)), ClientDialect::KingdomReborn);
                assert!(issues.is_empty(), "{entry}: {issues:?}");
                assert!(encode_sector(&decoded, ClientDialect::KingdomReborn).unwrap() == payload, "{entry} differs");
                decoded
            })
            .collect();
        let width = geometry.width as usize;
        let mut land = vec![(0i8, 0u16); width * geometry.height as usize];
        let mut padding_zero = true;
        for (sector, decoded) in decoded.iter().enumerate() {
            let sector = sector as u32;
            for (x, y, z, id) in sector_land(&geometry, sector, decoded) {
                land[y as usize * width + x as usize] = (z, id);
            }
            let (ox, oy) = geometry.sector_origin(sector);
            counter.add_sector(decoded, |index| {
                let on_map = geometry.contains(ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE);
                if !on_map {
                    let tile = &decoded.tiles[index];
                    padding_zero &= tile.z == 0
                        && tile.land_graphic == 0
                        && tile.kr.is_some_and(|kr| kr.subtype == 0 && kr.classic_land == 0)
                        && tile.delimiters.is_empty()
                        && tile.statics.is_empty();
                }
                on_map
            });
        }
        assert!(padding_zero, "facet {facet}: off-map squares are not all zero");

        // Pass 2: delimiter presence and order against the rule, and their (z, material) against the neighbours.
        let (mut presence, mut exact, mut squares) = (0u64, 0u64, 0u64);
        for (sector, decoded) in decoded.iter().enumerate() {
            let (ox, oy) = geometry.sector_origin(sector as u32);
            for (index, tile) in decoded.tiles.iter().enumerate() {
                let (x, y) = (ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE);
                if !geometry.contains(x, y) {
                    continue;
                }
                squares += 1;
                let expected = expected_delimiters(&geometry, x, y, |nx, ny| land[ny as usize * width + nx as usize]);
                presence += u64::from(
                    expected.iter().map(|d| d.direction).eq(tile.delimiters.iter().map(|d| d.direction)),
                );
                exact += u64::from(expected == tile.delimiters);
            }
        }
        println!(
            "facet {facet}: {} sectors ({}x{}) re-encode identically; delimiter presence and order match the rule on \
             {presence}/{squares} squares, (z, material) as well on {exact}",
            geometry.sector_count(),
            geometry.width,
            geometry.height
        );
        assert_eq!(presence, squares, "facet {facet}: delimiter presence");
    }

    let derived = counter.to_map();
    let shipped = KrLandMap::shipped_verified();
    assert_eq!(derived.rows(), shipped.rows(), "bundled mappings.bin differs from the facets: regenerate it");
    let (total, material_ok, subtype_ok) = counter.agreement(&shipped);
    println!(
        "land map: {} Classic ids; material exact on {material_ok}/{total} squares, subtype on {subtype_ok}/{total}",
        shipped.len()
    );
}

/// Per-square agreement of a facet built from Classic files with the shipped one.
#[derive(Default)]
struct Agreement {
    squares: u64,
    classic_land: u64,
    z: u64,
    statics: u64,
    material: u64,
    subtype: u64,
    delimiters: u64,
    identical_squares: u64,
    identical_sectors: u32,
    /// Squares whose Classic id the verified land map lacks.
    unverified_land: u64,
}

fn percent(part: u64, whole: u64) -> f64 {
    const PERCENT: f64 = 100.0;
    part as f64 * PERCENT / whole as f64
}

#[test]
#[ignore = "needs a real Kingdom Reborn and Classic client: set UO_KR_CLIENT_DIR and UO_KR_CLASSIC_DIR and run with \
            --ignored --release"]
fn kr_facets_build_from_classic_and_compare_with_the_shipped_ones() {
    let palette = RadarPalette::load_radarcol(&common::required_client_file(common::KR_CLASSIC_DIR_ENV, RADARCOL_NAME))
        .expect("radarcol.mul");
    let verified = KrLandMap::shipped_verified();
    let land_map = KrLandMap::shipped().with_fallback(verified.get(CLASSIC_NODRAW_LAND).unwrap());
    let out = common::temp_dir("kr_facet_from_classic");
    for facet in KR_FACETS {
        let shipped = EcFacetPackage::open_dialect(kr_facet_package(facet), facet, ClientDialect::KingdomReborn).unwrap();
        let geometry = shipped.geometry();
        let classic = |name: String| common::required_client_file(common::KR_CLASSIC_DIR_ENV, &name);
        let size = MapSizeCells { width: geometry.width, height: geometry.height };
        let mut plane = MapPlane::init_with_size(classic(format!("map{facet}.mul")), u32::from(facet), Some(size)).unwrap();
        let statics = StaticsReader::new(
            &classic(format!("staidx{facet}.mul")),
            &classic(format!("statics{facet}.mul")),
            geometry.width,
            geometry.height,
        )
        .unwrap();
        let (mut built, _radar) = build_kr_facet_from_classic(&mut plane, &statics, facet, &palette, &land_map).unwrap();
        let built_path = out.join(facet_package_file_name(facet, false));
        built.save(&built_path).unwrap();
        let reloaded = UopPackage::load(&built_path).unwrap();
        assert_eq!((reloaded.version(), reloaded.block_size()), (KR_FACET_PACKAGE_VERSION, KR_FACET_PACKAGE_BLOCK_SIZE));
        let mut reopened = EcFacetPackage::from_package_dialect(reloaded, facet, None, ClientDialect::KingdomReborn).unwrap();
        assert_eq!(reopened.geometry(), geometry);

        let mut a = Agreement::default();
        shipped
            .for_each_sector(|sector, expected| {
                let ours = reopened.sector(sector)?;
                a.identical_sectors += u32::from(ours == expected);
                let (ox, oy) = geometry.sector_origin(sector);
                for (index, (o, e)) in ours.tiles.iter().zip(&expected.tiles).enumerate() {
                    if !geometry.contains(ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE) {
                        continue;
                    }
                    let (ok, ek) = (o.kr.unwrap_or_default(), e.kr.unwrap_or_default());
                    a.squares += 1;
                    a.classic_land += u64::from(ok.classic_land == ek.classic_land);
                    a.z += u64::from(o.z == e.z);
                    a.statics += u64::from(o.statics == e.statics);
                    a.material += u64::from(o.land_graphic == e.land_graphic);
                    a.subtype += u64::from(ok.subtype == ek.subtype);
                    a.delimiters += u64::from(o.delimiters == e.delimiters);
                    a.identical_squares += u64::from(o == e);
                    a.unverified_land += u64::from(verified.get(ok.classic_land).is_none());
                }
                Ok(())
            })
            .unwrap();
        println!(
            "facet {facet} from Classic: land {:.3}% / z {:.3}% / statics {:.3}%; material {:.3}%, subtype {:.3}%, \
             delimiters {:.3}%, whole square {:.3}%; {}/{} sectors identical; {} squares with a Classic id outside \
             the verified land map",
            percent(a.classic_land, a.squares),
            percent(a.z, a.squares),
            percent(a.statics, a.squares),
            percent(a.material, a.squares),
            percent(a.subtype, a.squares),
            percent(a.delimiters, a.squares),
            percent(a.identical_squares, a.squares),
            a.identical_sectors,
            geometry.sector_count(),
            a.unverified_land
        );
        std::fs::remove_file(&built_path).unwrap();
    }
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored"]
fn kr_facet_data_maps_reencode_identically() {
    for facet in KR_FACETS {
        let package = UopPackage::load(kr_facet_package(facet)).expect("load KR facet package");
        let data_map = package
            .unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))
            .unwrap()
            .expect("data.map");
        let geometry = FacetGeometry::from_data_map_payload(&data_map).unwrap();
        assert_eq!(Some(geometry), FacetGeometry::client_facet(facet), "facet {facet}: data.map size");
        assert_eq!(geometry.data_map_payload().as_slice(), data_map.as_slice(), "facet {facet}: data.map re-encodes differently");
        println!("facet {facet}: data.map {}x{} re-encodes identically", geometry.width, geometry.height);
    }
}

#[test]
#[ignore = "needs a real Kingdom Reborn client: set UO_KR_CLIENT_DIR and run with --ignored"]
fn kr_static_chunks_reencode_identically() {
    let path = common::kr_client_file(STATIC_CHUNK_UOP_NAME);
    let mut package = UopPackage::load(&path).unwrap();
    let chunks = StaticChunkPackage::from_package(&package).unwrap();
    assert_eq!(chunks.chunks.len(), KR_STATIC_CHUNKS);
    let records: usize = chunks.chunks.iter().map(|(_, chunk)| chunk.records.len()).sum();
    assert_eq!(records, KR_STATIC_CHUNK_RECORDS);
    for (hash, chunk) in &chunks.chunks {
        let payload = package.unpack_file_by_hash(*hash).unwrap().unwrap();
        assert!(chunk.encode().unwrap() == payload, "static chunk {hash:016X} re-encodes differently");
    }
    assert_eq!(chunks.apply_to_package(&mut package).unwrap(), 0, "unchanged chunks replaced entries");
    println!("{STATIC_CHUNK_UOP_NAME}: {} chunks, {records} records, all re-encode identically", chunks.chunks.len());
}
