//! Checks the Enhanced Client facet codecs on real client files: the sectors of every `facet*.uop` re-encode
//! byte for byte and their delimiters match the shipped rule, the radar images (`facetNN.dds`) parse and write
//! back identically and agree with the regeneration recipe, the `facets.uop` definitions re-encode identically,
//! and a facet built from Classic files reads back.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS` (the Classic one supplies
//! `radarcol.mul`, `map4LegacyMUL.uop`, `staidx4.mul` and `statics4.mul`). The facet tests decode every sector;
//! run them with `--release`. Files written here go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are
//! deleted.

use crate::common;

use std::path::PathBuf;

use rayon::prelude::*;
use uocf::classic::map::MapPlane;
use uocf::classic::statics::StaticsReader;
use uocf::enhanced::facet_decoder::decode_facet_bin_raw;
use uocf::enhanced::facet_definition::{
    facet_definition_entry_path, FacetDefinition, CLIENT_FACET_DEFINITION_IDS, FACET_DEFINITIONS_UOP_NAME,
};
use uocf::enhanced::facet_encoder::build_facet_from_classic;
use uocf::enhanced::facet_package::EcFacetPackage;
use uocf::enhanced::facet_radar::{
    radar_dds_file_name, radar_index_agreement, radar_mip_count, FacetRadarDds, FacetRadarSource, PixelRect,
    RadarPalette,
};
use uocf::enhanced::facet_sector::{
    data_map_entry_path, encode_ec_sector, expected_delimiters, facet_package_file_name, sector_entry_path,
    validate_ec_sector, FacetGeometry, CLIENT_FACET_DIMENSIONS, SECTOR_SIDE, VARIANT_PACKAGE_FACETS,
};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

/// Facets with a package and a radar image in the client (0..5; the Test facet 6 ships neither).
const SHIPPED_FACETS: std::ops::RangeInclusive<u8> = 0..=5;
/// Sectors in all shipped facet packages, base and variants.
const SHIPPED_SECTOR_TOTAL: u32 = 34_841;
const RADARCOL_NAME: &str = "radarcol.mul";
/// Lowest palette-index agreement of a regenerated radar image accepted (the research measured 0.977-0.992).
const MIN_RADAR_INDEX_AGREEMENT: f64 = 0.97;
/// Shipped `facets.uop` record sizes, by facet id.
const SHIPPED_DEFINITION_SIZES: [usize; 7] = [1284, 1380, 900, 148, 148, 836, 52];
/// The facet built from Classic files: Tokuno, the smallest, with partial sectors.
const CLASSIC_FACET: u8 = 4;
const CLASSIC_MAP_NAME: &str = "map4LegacyMUL.uop";
const CLASSIC_STAIDX_NAME: &str = "staidx4.mul";
const CLASSIC_STATICS_NAME: &str = "statics4.mul";
/// Square edited in the radar update check (on a sector corner of Tokuno, so the edit spans four sectors).
const EDITED_SQUARE: (u32, u32) = (SECTOR_SIDE * 10 - 1, SECTOR_SIDE * 7 - 1);
const EDITED_LAND_Z_RISE: i8 = 20;
/// A static graphic with a radar colour, added in the radar update check.
const EDITED_STATIC_GRAPHIC: u32 = 0x0001;
const EDITED_STATIC_Z_ABOVE_LAND: i8 = 1;

/// The facet packages of the client, with the facet whose entry names they use.
fn facet_packages() -> Vec<(String, u8, PathBuf)> {
    let mut packages = Vec::new();
    for facet in SHIPPED_FACETS {
        for variant in [false, true] {
            if variant && !VARIANT_PACKAGE_FACETS.contains(&facet) {
                continue;
            }
            let name = facet_package_file_name(facet, variant);
            for (env, path) in common::client_files(&name) {
                packages.push((format!("{env}/{name}"), facet, path));
            }
        }
    }
    assert!(!packages.is_empty(), "no facet*.uop found: set UO_EC_CLIENT_DIR");
    packages
}

fn radar_palette() -> RadarPalette {
    let (env, path) = common::client_files(RADARCOL_NAME)
        .into_iter()
        .next()
        .expect("no radarcol.mul: set UO_CC_CLIENT_DIR to a Classic client");
    println!("radar colours from {env}/{RADARCOL_NAME}");
    RadarPalette::load_radarcol(&path).expect("load radarcol.mul")
}

/// Per facet package: (sectors, squares whose delimiters follow the rule, squares, sectors all of whose squares
/// do). Asserts that every sector re-encodes byte for byte and has the expected header.
fn check_package(label: &str, facet: u8, path: &PathBuf) -> (u32, u64, u64, u32) {
    let package = UopPackage::load(path).expect("load facet package");
    let data_map = package
        .unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))
        .expect("unpack data.map")
        .expect("data.map entry");
    let geometry = FacetGeometry::from_data_map_payload(&data_map).expect("data.map");
    assert_eq!(Some(geometry), FacetGeometry::client_facet(facet), "{label}: data.map size");

    let squares = geometry.width as usize * geometry.height as usize;
    let at = |x: u32, y: u32| y as usize * geometry.width as usize + x as usize;
    let decode = |sector: u32| {
        let path = sector_entry_path(facet, sector);
        let payload = package.unpack_file_by_hash(hash_file_name_single(&path)).expect("unpack").expect(&path);
        (decode_facet_bin_raw(&payload).unwrap_or_else(|error| panic!("{label} {path}: {error:#}")), payload)
    };

    // Pass 1: byte-identical re-encoding, and the land of the whole facet for the delimiters.
    let lands: Vec<Vec<(u32, u32, i8, u16)>> = (0..geometry.sector_count())
        .into_par_iter()
        .map(|sector| {
            let (decoded, payload) = decode(sector);
            let issues = validate_ec_sector(&decoded, Some((facet, sector)));
            assert!(issues.is_empty(), "{label} sector {sector}: {issues:?}");
            assert!(encode_ec_sector(&decoded).unwrap() == payload, "{label} sector {sector} re-encodes differently");
            let (ox, oy) = geometry.sector_origin(sector);
            decoded
                .tiles
                .iter()
                .enumerate()
                .map(|(index, tile)| (ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE, tile.z, tile.land_graphic))
                .filter(|&(x, y, _, _)| geometry.contains(x, y))
                .collect()
        })
        .collect();
    let mut land = vec![(0i8, 0u16); squares];
    for (x, y, z, id) in lands.into_iter().flatten() {
        land[at(x, y)] = (z, id);
    }

    // Pass 2: the shipped delimiters against the rule.
    let (matching, checked, clean_sectors) = (0..geometry.sector_count())
        .into_par_iter()
        .map(|sector| {
            let (decoded, _) = decode(sector);
            let (ox, oy) = geometry.sector_origin(sector);
            let (mut matching, mut checked) = (0u64, 0u64);
            for (index, tile) in decoded.tiles.iter().enumerate() {
                let (x, y) = (ox + index as u32 / SECTOR_SIDE, oy + index as u32 % SECTOR_SIDE);
                if !geometry.contains(x, y) {
                    continue;
                }
                checked += 1;
                matching += u64::from(expected_delimiters(&geometry, x, y, |nx, ny| land[at(nx, ny)]) == tile.delimiters);
            }
            (matching, checked, u32::from(matching == checked))
        })
        .reduce(|| (0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
    (geometry.sector_count(), matching, checked, clean_sectors)
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn facet_data_maps_reencode_identically() {
    for (label, facet, path) in facet_packages() {
        let package = UopPackage::load(&path).expect("load facet package");
        let data_map = package
            .unpack_file_by_hash(hash_file_name_single(&data_map_entry_path(facet)))
            .expect("unpack data.map")
            .expect("data.map entry");
        let geometry = FacetGeometry::from_data_map_payload(&data_map).expect("data.map");
        assert_eq!(Some(geometry), FacetGeometry::client_facet(facet), "{label}: data.map size");
        assert_eq!(geometry.data_map_payload().as_slice(), data_map.as_slice(), "{label}: data.map re-encodes differently");
        println!("{label}: data.map {}x{} re-encodes identically", geometry.width, geometry.height);
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored --release"]
fn facet_sectors_reencode_identically_and_delimiters_follow_the_rule() {
    let mut total = 0;
    for (label, facet, path) in facet_packages() {
        let (sectors, matching, checked, clean) = check_package(&label, facet, &path);
        total += sectors;
        println!(
            "{label}: {sectors} sectors re-encode identically; delimiters of {matching}/{checked} squares and \
             {clean}/{sectors} sectors match the rule"
        );
    }
    println!("{total} sectors in all");
    if total != SHIPPED_SECTOR_TOTAL {
        println!("(the research release has {SHIPPED_SECTOR_TOTAL})");
    }
}

#[test]
#[ignore = "needs a real Enhanced and Classic Client: set UO_EC_CLIENT_DIR and UO_CC_CLIENT_DIR and run with --ignored --release"]
fn radar_images_round_trip_and_agree_with_the_recipe() {
    let palette = radar_palette();
    for facet in SHIPPED_FACETS {
        let name = radar_dds_file_name(facet);
        for (env, path) in common::client_files(&name) {
            let bytes = std::fs::read(&path).unwrap();
            let shipped = FacetRadarDds::from_bytes(&bytes).expect("parse radar image");
            assert!(shipped.to_bytes() == bytes, "{env}/{name} writes back differently");
            assert_eq!(shipped.mip_count(), radar_mip_count(shipped.width(), shipped.height()), "{env}/{name}");
            let package_path = path.with_file_name(facet_package_file_name(facet, false));
            let package = EcFacetPackage::open(&package_path, facet).expect("open facet package");
            let geometry = package.geometry();
            assert_eq!((geometry.width, geometry.height), (shipped.width(), shipped.height()), "{env}/{name}");

            let mut uncoloured = 0u64;
            package
                .for_each_sector(|_, sector| {
                    uncoloured += sector
                        .tiles
                        .iter()
                        .flat_map(|tile| tile.statics.iter().filter(move |item| item.z >= tile.z))
                        .filter(|item| palette.static_color(item.graphic_id).is_none())
                        .count() as u64;
                    Ok(())
                })
                .unwrap();
            let source = FacetRadarSource::from_facet(&package, &palette).unwrap();
            let rgb = source.rgb8();
            let agreement = radar_index_agreement(&shipped, &rgb).unwrap();
            println!(
                "{env}/{name}: {}x{}, {} mips, writes back identically; palette-index agreement {agreement:.4}; \
                 {uncoloured} statics at or above the land without a radarcol entry",
                shipped.width(),
                shipped.height(),
                shipped.mip_count()
            );
            assert!(agreement >= MIN_RADAR_INDEX_AGREEMENT, "{env}/{name}: agreement {agreement}");
        }
    }
}

#[test]
#[ignore = "needs a real Enhanced and Classic Client: set UO_EC_CLIENT_DIR and UO_CC_CLIENT_DIR and run with --ignored --release"]
fn radar_region_update_matches_a_full_rebuild_on_a_real_facet() {
    let palette = radar_palette();
    let name = facet_package_file_name(CLASSIC_FACET, false);
    for (env, path) in common::client_files(&name) {
        let mut facet = EcFacetPackage::open(&path, CLASSIC_FACET).unwrap();
        let mut radar = FacetRadarDds::build(&facet, &palette).unwrap();
        let shipped = FacetRadarDds::load(&path.with_file_name(radar_dds_file_name(CLASSIC_FACET))).unwrap();
        let encoded_agreement = radar_index_agreement(&shipped, &radar.decode_level_rgb8(0)).unwrap();

        let (x, y) = EDITED_SQUARE;
        let tile = facet.square(x, y).unwrap().clone();
        facet.set_land(x, y, tile.land_graphic, tile.z.saturating_add(EDITED_LAND_Z_RISE)).unwrap();
        let (sx, sy) = (x + 1, y + 1);
        let static_z = facet.square(sx, sy).unwrap().z.saturating_add(EDITED_STATIC_Z_ABOVE_LAND);
        facet.add_static(sx, sy, EDITED_STATIC_GRAPHIC, static_z, 0).unwrap();
        let rewritten = radar
            .update_region(&mut facet, &palette, PixelRect::square(x, y).union(&PixelRect::square(sx, sy)))
            .unwrap();
        assert!(radar == FacetRadarDds::build(&facet, &palette).unwrap(), "{env}/{name}: update differs from a rebuild");
        println!(
            "{env}/{name}: our DXT1 mip 0 decodes at {encoded_agreement:.4} agreement; an edit at ({x}, {y}) rewrote \
             mip 0 pixels {}..{} x {}..{}, identical to a full rebuild at every level",
            rewritten.x0, rewritten.x1, rewritten.y0, rewritten.y1
        );
    }
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
fn facet_definitions_reencode_identically() {
    for (env, path) in common::client_files(FACET_DEFINITIONS_UOP_NAME) {
        let package = UopPackage::load(&path).unwrap();
        for id in CLIENT_FACET_DEFINITION_IDS {
            let entry = facet_definition_entry_path(id);
            let bytes = package.unpack_file_by_hash(hash_file_name_single(&entry)).unwrap().expect(&entry);
            let definition = FacetDefinition::from_bytes(&bytes).unwrap();
            assert!(definition.to_bytes().unwrap() == bytes, "{env} {entry} re-encodes differently");
            assert_eq!(bytes.len(), SHIPPED_DEFINITION_SIZES[id as usize], "{env} {entry}");
            assert_eq!(u32::from(definition.facet_id), id);
            let client = CLIENT_FACET_DIMENSIONS[id as usize];
            assert_eq!((definition.width, definition.height), (client.width, client.height), "{env} {entry}");
            println!(
                "{env} {entry}: {} bytes, {}x{}, unknown {:02x?}, {} regions, {} rects, issues {:?}",
                bytes.len(),
                definition.width,
                definition.height,
                definition.unknown,
                definition.regions.len(),
                definition.regions.iter().map(|region| region.rects.len()).sum::<usize>(),
                definition.validate()
            );
        }
    }
}

#[test]
#[ignore = "needs a real Enhanced and Classic Client: set UO_EC_CLIENT_DIR and UO_CC_CLIENT_DIR and run with --ignored --release"]
fn classic_facet_builds_and_compares_with_the_shipped_one() {
    let palette = radar_palette();
    let (env, map_path) = common::client_files(CLASSIC_MAP_NAME).into_iter().next().expect("map4LegacyMUL.uop");
    let dir = map_path.parent().unwrap();
    let geometry = FacetGeometry::client_facet(CLASSIC_FACET).unwrap();
    let size = uocf::classic::map::MapSizeCells { width: geometry.width, height: geometry.height };
    let mut plane = MapPlane::init_uop_with_size(map_path.clone(), u32::from(CLASSIC_FACET), Some(size)).unwrap();
    let statics = StaticsReader::new(
        &dir.join(CLASSIC_STAIDX_NAME),
        &dir.join(CLASSIC_STATICS_NAME),
        geometry.width,
        geometry.height,
    )
    .unwrap();
    let (mut built, radar) = build_facet_from_classic(&mut plane, &statics, CLASSIC_FACET, &palette).unwrap();
    let out = common::temp_dir("facet_from_classic");
    let built_path = out.join(facet_package_file_name(CLASSIC_FACET, false));
    built.save(&built_path).unwrap();
    radar.save(&out.join(radar_dds_file_name(CLASSIC_FACET))).unwrap();
    let mut reopened = EcFacetPackage::open(&built_path, CLASSIC_FACET).unwrap();
    assert_eq!(reopened.geometry(), geometry);

    let (mut same_z, mut same_land, mut same_statics, mut same_delimiters, mut squares) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut identical_sectors = 0u32;
    for (ec_env, shipped_path) in common::client_files(&facet_package_file_name(CLASSIC_FACET, false)) {
        let shipped = EcFacetPackage::open(&shipped_path, CLASSIC_FACET).unwrap();
        shipped
            .for_each_sector(|sector, expected| {
                let ours = reopened.sector(sector)?;
                identical_sectors += u32::from(ours == expected);
                for (a, b) in ours.tiles.iter().zip(&expected.tiles) {
                    squares += 1;
                    same_z += u64::from(a.z == b.z);
                    same_land += u64::from(a.land_graphic == b.land_graphic);
                    same_statics += u64::from(a.statics == b.statics);
                    same_delimiters += u64::from(a.delimiters == b.delimiters);
                }
                Ok(())
            })
            .unwrap();
        println!(
            "{env}/{CLASSIC_MAP_NAME} vs {ec_env}: of {squares} squares, z equal {same_z}, land id equal {same_land}, \
             statics equal {same_statics}, delimiters equal {same_delimiters}; {identical_sectors}/{} sectors identical",
            geometry.sector_count()
        );
    }
    std::fs::remove_dir_all(out).unwrap();
}
