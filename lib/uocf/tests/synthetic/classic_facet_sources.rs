//! The shared Classic facet resolver (`uocf::classic::facet_sources`): a Trammel with no map1 file is built from
//! map0 and the facet-1 difs; without those difs it fails naming them; a facet's own difs apply only when asked
//! for; the variant map edition reads the `x` files, never the standard ones, and difs apply to either edition.

use std::fs;
use std::path::{Path, PathBuf};

use uocf::classic::facet_sources::{
    ClassicDifOptions, ClassicFacetSources, ClassicMapFile, ClassicMapFormat, StaticsFiles, FELUCCA, TRAMMEL,
};
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::map_edition::MapEdition;
use uocf::uop_container::file::CompressionFlag;
use uocf::uop_container::package::UopPackage;

use crate::common;

const MUL_FIRST: [ClassicMapFormat; 2] = [ClassicMapFormat::Mul, ClassicMapFormat::Uop];
/// A one-block-wide, two-block-tall map: block 0 at (0, 0), block 1 at (0, 1).
const MAP_SIZE: MapSizeCells = MapSizeCells { width: 8, height: 16 };
const PATCHED_BLOCK: u32 = 1;
const PATCHED_POS: MapBlockRelPos = MapBlockRelPos { x: 0, y: 1 };
const UNPATCHED_POS: MapBlockRelPos = MapBlockRelPos { x: 0, y: 0 };

const BASE_LAND: [(u16, i8); 2] = [(1, 2), (3, 4)];
const DIF_LAND: (u16, i8) = (55, -6);
const BASE_STATIC_GRAPHICS: [u16; 2] = [100, 101];
const DIF_STATIC_GRAPHIC: u16 = 200;
const INVALID: u32 = u32::MAX;

fn write_base_map(dir: &Path, facet: u32) {
    let bytes: Vec<u8> = BASE_LAND.iter().flat_map(|&(id, z)| common::map_block(id, z)).collect();
    fs::write(dir.join(format!("map{facet}.mul")), bytes).unwrap();
}

/// One static in each of the two blocks.
fn write_base_statics(dir: &Path, facet: u32) {
    let mut index = fs::File::create(dir.join(format!("staidx{facet}.mul"))).unwrap();
    let mut statics = Vec::new();
    for graphic in BASE_STATIC_GRAPHICS {
        let tile = common::static_tile(graphic, 1, 2, 3, 0);
        common::write_index_entry(&mut index, statics.len() as u32, tile.len() as u32, 0);
        statics.extend(tile);
    }
    fs::write(dir.join(format!("statics{facet}.mul")), statics).unwrap();
}

fn write_map_difs(dir: &Path, facet: u32) {
    fs::write(dir.join(format!("mapdifl{facet}.mul")), PATCHED_BLOCK.to_le_bytes()).unwrap();
    fs::write(dir.join(format!("mapdif{facet}.mul")), common::map_block(DIF_LAND.0, DIF_LAND.1)).unwrap();
}

/// Patches block 1 with one static and block 0 with an invalid entry, which empties it.
fn write_static_difs(dir: &Path, facet: u32) {
    let ids: Vec<u8> = [PATCHED_BLOCK, 0].iter().flat_map(|id| id.to_le_bytes()).collect();
    fs::write(dir.join(format!("stadifl{facet}.mul")), ids).unwrap();
    let tile = common::static_tile(DIF_STATIC_GRAPHIC, 2, 3, -4, 5);
    let mut index = fs::File::create(dir.join(format!("stadifi{facet}.mul"))).unwrap();
    common::write_index_entry(&mut index, 0, tile.len() as u32, 0);
    common::write_index_entry(&mut index, INVALID, 0, INVALID);
    fs::write(dir.join(format!("stadif{facet}.mul")), tile).unwrap();
}

fn land(plane: &mut MapPlane, pos: MapBlockRelPos) -> (u16, i8) {
    plane.load_blocks(&mut [pos]).unwrap();
    let cell = plane.block_no_update(pos).unwrap().cell(0, 0).unwrap();
    (cell.id, cell.z)
}

fn static_graphics(sources: &ClassicFacetSources) -> [Vec<u16>; 2] {
    let reader = sources.load_statics_reader(MAP_SIZE.width, MAP_SIZE.height, None).unwrap();
    let store = reader.load_all().unwrap();
    [UNPATCHED_POS, PATCHED_POS].map(|pos| {
        let read: Vec<u16> = reader.read_block(pos.x, pos.y).unwrap().iter().map(|tile| tile.graphic).collect();
        let stored: Vec<u16> = store.block_tiles(pos.x, pos.y).iter().map(|tile| tile.graphic).collect();
        assert_eq!(read, stored, "read_block and load_all agree on block ({}, {})", pos.x, pos.y);
        read
    })
}

#[test]
fn trammel_without_map1_is_built_from_map0_and_the_facet_1_difs() {
    let dir = common::temp_dir("facet_sources_trammel");
    write_base_map(&dir, FELUCCA);
    write_base_statics(&dir, FELUCCA);
    write_map_difs(&dir, TRAMMEL);
    write_static_difs(&dir, TRAMMEL);
    let dirs = [dir.clone()];

    // No dif option asked for: the facet-1 difs apply anyway, since they are the only way to read Trammel.
    let sources = ClassicFacetSources::resolve(
        &dirs,
        TRAMMEL,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Standard,
    )
    .unwrap();
    assert_eq!(sources.base_facet, Some(FELUCCA));
    assert_eq!(sources.map.path, dir.join("map0.mul"));
    assert_eq!(sources.map.map_index, FELUCCA);
    assert_eq!(sources.statics.as_ref().unwrap().index, dir.join("staidx0.mul"));
    assert!(sources.map_difs.is_some() && sources.static_difs.is_some());
    let warning = sources.base_facet_warning().expect("a base facet warning");
    for name in ["map1.mul not found", "Trammel", "map0.mul", "staidx0.mul", "mapdif1.mul", "mapdifl1.mul", "stadif1.mul", "stadifl1.mul", "stadifi1.mul", "pre-7.0"] {
        assert!(warning.contains(name), "{warning:?} names {name}");
    }

    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(plane.index, TRAMMEL);
    assert_eq!(plane.file_map_index(), FELUCCA);
    assert_eq!(land(&mut plane, PATCHED_POS), DIF_LAND);
    assert_eq!(land(&mut plane, UNPATCHED_POS), BASE_LAND[0]);
    // Block 1 takes the dif's static; block 0, patched by an invalid entry, loses its static.
    assert_eq!(static_graphics(&sources), [vec![], vec![DIF_STATIC_GRAPHIC]]);

    // Felucca itself stays unpatched.
    let felucca = ClassicFacetSources::resolve(
        &dirs,
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Standard,
    )
    .unwrap();
    assert_eq!((felucca.base_facet, felucca.base_facet_warning()), (None, None));
    let mut plane = felucca.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), BASE_LAND[1]);
    assert_eq!(static_graphics(&felucca), BASE_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn trammel_from_a_map0_uop_reads_the_map0_entries() {
    let dir = common::temp_dir("facet_sources_trammel_uop");
    let chunk: Vec<u8> = BASE_LAND.iter().flat_map(|&(id, z)| common::map_block(id, z)).collect();
    let mut package = UopPackage::new_default();
    package.add_file_from_memory(&chunk, "build/map0legacymul/00000000.dat", CompressionFlag::None).unwrap();
    package.finalize_and_save(&dir.join("map0LegacyMUL.uop")).unwrap();
    write_map_difs(&dir, TRAMMEL);
    write_static_difs(&dir, TRAMMEL);

    let sources =
        ClassicFacetSources::resolve(
            &[dir.clone()],
            TRAMMEL,
            &MUL_FIRST,
            ClassicDifOptions::NONE,
            MapEdition::Standard,
        )
        .unwrap();
    assert_eq!(sources.map.format, ClassicMapFormat::Uop);
    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), DIF_LAND);
    assert_eq!(land(&mut plane, UNPATCHED_POS), BASE_LAND[0]);
    // There are no staidx0/statics0 here: the statics reader names them.
    let error = sources.load_statics_reader(MAP_SIZE.width, MAP_SIZE.height, None).err().unwrap();
    assert!(format!("{error:#}").contains("staidx0.mul and statics0.mul"), "{error:#}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn trammel_without_map1_or_its_difs_fails_naming_them() {
    let dir = common::temp_dir("facet_sources_trammel_missing");
    write_base_map(&dir, FELUCCA);
    write_base_statics(&dir, FELUCCA);
    write_map_difs(&dir, TRAMMEL);
    let dirs = [dir.clone()];

    let error = ClassicFacetSources::resolve(
        &dirs,
        TRAMMEL,
        &MUL_FIRST,
        ClassicDifOptions::ALL,
        MapEdition::Standard,
    )
    .unwrap_err();
    let message = format!("{error:#}");
    for name in ["Missing map data for map1", "map1.mul", "map1LegacyMUL.uop", "stadifl1.mul, stadifi1.mul, stadif1.mul are missing"] {
        assert!(message.contains(name), "{message:?} names {name}");
    }
    assert!(!message.contains("mapdif1.mul"), "{message:?} names only the missing files");

    // Without map0 either, the error names it too.
    fs::remove_file(dir.join("map0.mul")).unwrap();
    let error =
        ClassicFacetSources::resolve(&dirs, TRAMMEL, &MUL_FIRST, ClassicDifOptions::ALL, MapEdition::Standard).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("map0.mul, map0LegacyMUL.uop, stadifl1.mul"), "{message:?}");

    // A facet with no base facet names only its own map files.
    let error =
        ClassicFacetSources::resolve(&dirs, 2, &MUL_FIRST, ClassicDifOptions::ALL, MapEdition::Standard).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("Missing map data for map2: none of map2.mul, map2LegacyMUL.uop"), "{message:?}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_facets_own_difs_apply_only_when_asked_for() {
    let dir = common::temp_dir("facet_sources_own_difs");
    write_base_map(&dir, FELUCCA);
    write_base_statics(&dir, FELUCCA);
    write_map_difs(&dir, FELUCCA);
    write_static_difs(&dir, FELUCCA);
    let dirs = [dir.clone()];

    let unpatched = ClassicFacetSources::resolve(
        &dirs,
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Standard,
    )
    .unwrap();
    assert!(unpatched.map_difs.is_none() && unpatched.static_difs.is_none());
    let mut plane = unpatched.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), BASE_LAND[1]);
    assert_eq!(static_graphics(&unpatched), BASE_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    let map_only = ClassicDifOptions { map_difs: true, static_difs: false };
    let sources = ClassicFacetSources::resolve(&dirs, FELUCCA, &MUL_FIRST, map_only, MapEdition::Standard).unwrap();
    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), DIF_LAND);
    assert_eq!(static_graphics(&sources), BASE_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    let patched = ClassicFacetSources::resolve(
        &dirs,
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::ALL,
        MapEdition::Standard,
    )
    .unwrap();
    assert_eq!(static_graphics(&patched), [vec![], vec![DIF_STATIC_GRAPHIC]]);

    // Files named directly take their difs from the given directories.
    let map = ClassicMapFile { format: ClassicMapFormat::Mul, path: dir.join("map0.mul"), map_index: FELUCCA };
    let statics = StaticsFiles { index: dir.join("staidx0.mul"), statics: dir.join("statics0.mul") };
    let named = ClassicFacetSources::from_files(FELUCCA, map, Some(statics), &dirs, ClassicDifOptions::ALL);
    assert_eq!((named.map_difs, named.static_difs), (patched.map_difs, patched.static_difs));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn asked_for_difs_a_facet_lacks_are_no_error() {
    let dir = common::temp_dir("facet_sources_no_difs");
    write_base_map(&dir, FELUCCA);
    write_base_statics(&dir, FELUCCA);
    // An incomplete set (no mapdif0.mul) is not applied either.
    fs::write(dir.join("mapdifl0.mul"), PATCHED_BLOCK.to_le_bytes()).unwrap();
    let dirs: [PathBuf; 1] = [dir.clone()];

    let sources = ClassicFacetSources::resolve(
        &dirs,
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::ALL,
        MapEdition::Standard,
    )
    .unwrap();
    assert!(sources.map_difs.is_none() && sources.static_difs.is_none());
    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), BASE_LAND[1]);

    let _ = fs::remove_dir_all(dir);
}

const VARIANT_LAND: [(u16, i8); 2] = [(7, 8), (9, 10)];
const VARIANT_STATIC_GRAPHICS: [u16; 2] = [300, 301];
/// A facet without a variant edition in any client (Malas).
const FACET_WITHOUT_VARIANT: u32 = 3;

fn write_variant_map(dir: &Path, facet: u32) {
    let bytes: Vec<u8> = VARIANT_LAND.iter().flat_map(|&(id, z)| common::map_block(id, z)).collect();
    fs::write(dir.join(format!("map{facet}x.mul")), bytes).unwrap();
}

fn write_variant_statics(dir: &Path, facet: u32) {
    let mut index = fs::File::create(dir.join(format!("staidx{facet}x.mul"))).unwrap();
    let mut statics = Vec::new();
    for graphic in VARIANT_STATIC_GRAPHICS {
        let tile = common::static_tile(graphic, 1, 2, 3, 0);
        common::write_index_entry(&mut index, statics.len() as u32, tile.len() as u32, 0);
        statics.extend(tile);
    }
    fs::write(dir.join(format!("statics{facet}x.mul")), statics).unwrap();
}

#[test]
fn the_variant_edition_reads_the_x_files() {
    let dir = common::temp_dir("facet_sources_variant");
    for facet in [FELUCCA, TRAMMEL] {
        write_base_map(&dir, facet);
        write_base_statics(&dir, facet);
        write_variant_map(&dir, facet);
        write_variant_statics(&dir, facet);
    }
    let dirs = [dir.clone()];

    let sources = ClassicFacetSources::resolve(&dirs, TRAMMEL, &MUL_FIRST, ClassicDifOptions::NONE, MapEdition::Variant)
        .unwrap();
    assert_eq!(sources.edition, MapEdition::Variant);
    assert_eq!(sources.map.path, dir.join("map1x.mul"));
    assert_eq!(sources.map.map_index, TRAMMEL);
    assert_eq!(sources.statics.as_ref().unwrap().index, dir.join("staidx1x.mul"));
    assert_eq!(sources.statics.as_ref().unwrap().statics, dir.join("statics1x.mul"));
    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, UNPATCHED_POS), VARIANT_LAND[0]);
    assert_eq!(land(&mut plane, PATCHED_POS), VARIANT_LAND[1]);
    assert_eq!(static_graphics(&sources), VARIANT_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    // The standard edition ignores the x files.
    let standard =
        ClassicFacetSources::resolve(&dirs, TRAMMEL, &MUL_FIRST, ClassicDifOptions::NONE, MapEdition::Standard)
            .unwrap();
    assert_eq!((standard.edition, standard.map.path.clone()), (MapEdition::Standard, dir.join("map1.mul")));
    assert_eq!(static_graphics(&standard), BASE_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    // A facet no client has a variant of reads its standard files.
    write_base_map(&dir, FACET_WITHOUT_VARIANT);
    let malas = ClassicFacetSources::resolve(
        &dirs,
        FACET_WITHOUT_VARIANT,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Variant,
    )
    .unwrap();
    assert_eq!((malas.edition, malas.map.path), (MapEdition::Standard, dir.join("map3.mul")));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_variant_edition_of_a_uop_map_keeps_the_standard_entry_names() {
    let dir = common::temp_dir("facet_sources_variant_uop");
    let chunk: Vec<u8> = VARIANT_LAND.iter().flat_map(|&(id, z)| common::map_block(id, z)).collect();
    let mut package = UopPackage::new_default();
    package.add_file_from_memory(&chunk, "build/map0legacymul/00000000.dat", CompressionFlag::None).unwrap();
    package.finalize_and_save(&dir.join("map0xLegacyMUL.uop")).unwrap();
    write_base_map(&dir, FELUCCA);

    let uop_first = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];
    let sources =
        ClassicFacetSources::resolve(&[dir.clone()], FELUCCA, &uop_first, ClassicDifOptions::NONE, MapEdition::Variant)
            .unwrap();
    assert_eq!((sources.map.format, sources.map.path.clone()), (ClassicMapFormat::Uop, dir.join("map0xLegacyMUL.uop")));
    let mut plane = sources.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), VARIANT_LAND[1]);
    // No staidx0x/statics0x: the statics reader names them.
    let error = sources.load_statics_reader(MAP_SIZE.width, MAP_SIZE.height, None).err().unwrap();
    assert!(format!("{error:#}").contains("staidx0x.mul and statics0x.mul"), "{error:#}");

    // The standard edition never takes the x package.
    fs::remove_file(dir.join("map0.mul")).unwrap();
    let error =
        ClassicFacetSources::resolve(&[dir.clone()], FELUCCA, &uop_first, ClassicDifOptions::NONE, MapEdition::Standard)
            .unwrap_err();
    assert!(format!("{error:#}").contains("Missing map data for map0"), "{error:#}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_missing_variant_is_an_error_naming_the_x_files() {
    let dir = common::temp_dir("facet_sources_variant_missing");
    write_base_map(&dir, TRAMMEL);
    write_base_statics(&dir, TRAMMEL);
    let dirs = [dir.clone()];

    let error = ClassicFacetSources::resolve(&dirs, TRAMMEL, &MUL_FIRST, ClassicDifOptions::NONE, MapEdition::Variant)
        .unwrap_err();
    let message = format!("{error:#}");
    for name in ["variant edition of map1", "map1x.mul", "map1xLegacyMUL.uop", "map1x.uop", "staidx1x.mul", "statics1x.mul"] {
        assert!(message.contains(name), "{message:?} names {name}");
    }

    // A pre-7.0 folder with no map1 at all does not build the variant from map0 and the facet-1 difs.
    let pre7 = common::temp_dir("facet_sources_variant_pre7");
    write_base_map(&pre7, FELUCCA);
    write_base_statics(&pre7, FELUCCA);
    write_map_difs(&pre7, TRAMMEL);
    write_static_difs(&pre7, TRAMMEL);
    let error =
        ClassicFacetSources::resolve(&[pre7.clone()], TRAMMEL, &MUL_FIRST, ClassicDifOptions::NONE, MapEdition::Variant)
            .unwrap_err();
    assert!(format!("{error:#}").contains("variant edition of map1"), "{error:#}");

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(pre7);
}

#[test]
fn difs_apply_to_the_variant_edition_too() {
    let dir = common::temp_dir("facet_sources_variant_difs");
    write_base_map(&dir, FELUCCA);
    write_base_statics(&dir, FELUCCA);
    write_variant_map(&dir, FELUCCA);
    write_variant_statics(&dir, FELUCCA);
    write_map_difs(&dir, FELUCCA);
    write_static_difs(&dir, FELUCCA);
    let dirs = [dir.clone()];

    let unpatched =
        ClassicFacetSources::resolve(&dirs, FELUCCA, &MUL_FIRST, ClassicDifOptions::NONE, MapEdition::Variant).unwrap();
    assert!(unpatched.map_difs.is_none() && unpatched.static_difs.is_none());

    // The difs patch the x base: block 1 takes the dif's land and static, block 0 keeps the variant land and loses
    // its static to the invalid entry.
    let patched =
        ClassicFacetSources::resolve(&dirs, FELUCCA, &MUL_FIRST, ClassicDifOptions::ALL, MapEdition::Variant).unwrap();
    assert_eq!((patched.edition, patched.map.path.clone()), (MapEdition::Variant, dir.join("map0x.mul")));
    assert!(patched.map_difs.is_some() && patched.static_difs.is_some());
    let mut plane = patched.load_map_plane(Some(MAP_SIZE), None).unwrap();
    assert_eq!(land(&mut plane, PATCHED_POS), DIF_LAND);
    assert_eq!(land(&mut plane, UNPATCHED_POS), VARIANT_LAND[0]);
    assert_eq!(static_graphics(&patched), [vec![], vec![DIF_STATIC_GRAPHIC]]);

    // Map difs only: the variant statics stay as they are.
    let map_only = ClassicDifOptions { map_difs: true, static_difs: false };
    let sources = ClassicFacetSources::resolve(&dirs, FELUCCA, &MUL_FIRST, map_only, MapEdition::Variant).unwrap();
    assert_eq!(static_graphics(&sources), VARIANT_STATIC_GRAPHICS.map(|graphic| vec![graphic]));

    // A variant map named directly is read as the variant edition, with its difs as asked.
    let map = ClassicMapFile { format: ClassicMapFormat::Mul, path: dir.join("MAP0X.MUL"), map_index: FELUCCA };
    let statics = StaticsFiles { index: dir.join("staidx0x.mul"), statics: dir.join("statics0x.mul") };
    let named = ClassicFacetSources::from_files(FELUCCA, map, Some(statics), &dirs, ClassicDifOptions::ALL);
    assert_eq!(named.edition, MapEdition::Variant);
    assert_eq!((named.map_difs, named.static_difs), (patched.map_difs, patched.static_difs));

    let _ = fs::remove_dir_all(dir);
}
