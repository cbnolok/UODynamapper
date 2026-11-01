//! Checks the Classic map and statics writers (`classic::map_write`, `classic::statics_write`) and the facet
//! conversions (`classic::world`, `enhanced::facet_to_classic`, the facet builders) on real clients:
//!
//! - every `mapN.mul`, `mapNLegacyMUL.uop` and `staidxN.mul`/`staticsN.mul` (with their `x` editions) of the
//!   clients in `UO_CLASSIC_CLIENT_ROOTS` reads and writes back, MUL and UOP both ways;
//! - the difs baked into the map and statics match the dif-applying readers (UO Gold, Mondain's Legacy);
//! - an edit writes back one UOP entry and reads back through the client-file readers;
//! - the Enhanced Client facets (`UO_EC_CLIENT_DIR`) and the KR ones (`UO_KR_CLIENT_DIR`) convert to Classic and
//!   back, and the Classic facets (`UO_CC_CLIENT_DIR`, `UO_KR_CLASSIC_DIR`) to facets and back;
//! - resizing between the UO Gold and the Mondain's Legacy Felucca sizes.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists Classic client directories (separated like `PATH`), with UO Gold and Mondain's
//! Legacy for the dif and resize tests, named by their `CLIENT_ERA_*` markers below. Run with `--ignored --release`.
//! Files written here go to `std::env::temp_dir()` (set `TMPDIR` to choose it) and are deleted.

use crate::common;

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
use uocf::classic::map::{MapBlockRelPos, MapPlane, MapSizeCells};
use uocf::classic::map_write::ClassicMapBlocks;
use uocf::classic::statics::{StaticTile, StaticsReader};
use uocf::classic::statics_write::ClassicStaticsBlocks;
use uocf::classic::world::{ClassicWorld, LandFill, VOID_LAND_ID, VOID_LAND_Z};
use uocf::enhanced::facet_encoder::build_facet_from_world;
use uocf::enhanced::facet_package::{detect_facet_dialect, EcFacetPackage};
use uocf::enhanced::facet_sector::facet_package_file_name;
use uocf::enhanced::facet_to_classic::facet_to_classic;
use uocf::kr::facet::facet_encoder::build_kr_facet_from_world;
use uocf::kr::facet::kr_land_map::KrLandMap;
use uocf::map_edition::MapEdition;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
/// The Classic client the Enhanced Client facets are compared with (optional).
const CLASSIC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
/// The client directories the dif and resize tests use, by a part of their name.
const CLIENT_ERA_GOLD: &str = "Gold";
const CLIENT_ERA_ML: &str = "Mondain";
/// The clients whose files are in the old layout (block headers other than the block index, statics in another
/// order and other index fillers): read exactly, written in the modern layout.
const OLD_LAYOUT_ERAS: [&str; 2] = [CLIENT_ERA_GOLD, CLIENT_ERA_ML];
/// The Classic facets (map indices) a client may ship.
const CLASSIC_FACETS: std::ops::RangeInclusive<u32> = 0..=5;
/// The facets UO Gold and Mondain's Legacy have difs for.
const DIF_FACETS: std::ops::RangeInclusive<u32> = 0..=2;
/// The Enhanced Client and KR facet packages.
const FACET_PACKAGES: std::ops::RangeInclusive<u8> = 0..=5;
/// Felucca, and its widths before and since Mondain's Legacy.
const FELUCCA: u32 = 0;
const GOLD_FELUCCA: MapSizeCells = MapSizeCells { width: 6144, height: 4096 };
const ML_FELUCCA: MapSizeCells = MapSizeCells { width: 7168, height: 4096 };
/// The Classic id the KR build stores for Classic land ids newer than the KR land map (see
/// `kr_facet_client_files`).
const CLASSIC_NODRAW_LAND: u16 = 2;
/// The edit test: Tokuno, a cell and the static added there.
const EDITED_FACET: u32 = 4;
const EDITED_CELL: (u32, u32) = (700, 700);
const EDITED_LAND: (u16, i8) = (0x0003, 7);
const EDITED_STATIC: (u16, i8, u16) = (0x0E75, 9, 0x0021);

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn era_root(marker: &str) -> PathBuf {
    client_roots()
        .into_iter()
        .find(|root| label(root).contains(marker))
        .unwrap_or_else(|| panic!("{CLIENT_ROOTS_ENV} names no directory with {marker} in its name"))
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    if a.len() != b.len() {
        return a.len().max(b.len());
    }
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// The facet `facet` of `dirs` as the client reads it, from the first container found.
fn resolve(dirs: &[PathBuf], facet: u32, difs: ClassicDifOptions, edition: MapEdition) -> Option<ClassicFacetSources> {
    ClassicFacetSources::resolve(dirs, facet, &[ClassicMapFormat::Uop, ClassicMapFormat::Mul], difs, edition).ok()
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn classic_maps_and_statics_write_back_mul_and_uop() {
    let out = common::temp_dir("classic_map_writers");
    for root in client_roots() {
        let name = label(&root);
        let old_layout = OLD_LAYOUT_ERAS.iter().any(|marker| name.contains(marker));
        for facet in CLASSIC_FACETS {
            for x in ["", "x"] {
                let mul = find_file_ignoring_case(&root, &format!("map{facet}{x}.mul"));
                let uop = find_file_ignoring_case(&root, &format!("map{facet}{x}LegacyMUL.uop"));
                let map = if let Some(path) = &mul {
                    let bytes = std::fs::read(path).unwrap();
                    let map = ClassicMapBlocks::from_mul_bytes(&bytes, facet, None).unwrap();
                    let written = map.to_mul_bytes();
                    println!(
                        "{name}: map{facet}{x}.mul {}x{}: {} bytes of {} differ when written back",
                        map.size_cells().width,
                        map.size_cells().height,
                        differing_bytes(&written, &bytes),
                        bytes.len()
                    );
                    assert_eq!(ClassicMapBlocks::from_mul_bytes(&written, facet, None).unwrap(), map);
                    if !old_layout {
                        assert!(written == bytes, "{name}: map{facet}{x}.mul is not written back byte for byte");
                    }
                    Some(map)
                } else {
                    None
                };
                let map = if let Some(path) = &uop {
                    let original = std::fs::read(path).unwrap();
                    let map = ClassicMapBlocks::read_uop(path, facet, None).unwrap();
                    // UOP -> MUL -> UOP over the shipped package as the template.
                    let through_mul = ClassicMapBlocks::from_mul_bytes(&map.to_mul_bytes(), facet, None).unwrap();
                    let rewritten = out.join(format!("{name}_map{facet}{x}.uop"));
                    let report = through_mul.write_uop(&rewritten, facet, Some(UopPackage::load(path).unwrap())).unwrap();
                    let same = std::fs::read(&rewritten).unwrap() == original;
                    // A fresh package: the same entry payloads.
                    let fresh = out.join(format!("{name}_map{facet}{x}_fresh.uop"));
                    through_mul.write_uop(&fresh, facet, None).unwrap();
                    let (shipped, ours) = (UopPackage::load(path).unwrap(), UopPackage::load(&fresh).unwrap());
                    let payloads_equal = shipped.file_count() == ours.file_count()
                        && (0..through_mul.uop_entry_payloads().len() as u32).all(|entry| {
                            let hash = uocf::uop_container::hash::hash_file_name_single(
                                &uocf::classic::map::uop_map_entry_name(facet, entry),
                            );
                            shipped.unpack_file_by_hash(hash).unwrap() == ours.unpack_file_by_hash(hash).unwrap()
                        });
                    assert_eq!(ClassicMapBlocks::read_uop(&fresh, facet, None).unwrap(), map);
                    println!(
                        "{name}: map{facet}{x}LegacyMUL.uop {}x{} via MUL: byte for byte {same} ({report:?}); fresh \
                         package with the same entries {payloads_equal}",
                        map.size_cells().width,
                        map.size_cells().height
                    );
                    if let Some(mul_map) = &map_from(&mul, facet) {
                        println!("{name}: map{facet}{x}: the MUL and the UOP hold the same blocks: {}", *mul_map == map);
                    }
                    // The shipped package saved unchanged: the same bytes, except that the writer zeroes bytes no
                    // entry or table uses (some patched packages hold stale data there).
                    let unchanged = out.join(format!("{name}_map{facet}{x}_unchanged.uop"));
                    UopPackage::load(path).unwrap().finalize_and_save(&unchanged).unwrap();
                    let unchanged_bytes = std::fs::read(&unchanged).unwrap();
                    assert!(std::fs::read(&rewritten).unwrap() == unchanged_bytes, "{name}: map{facet}{x} rewrite");
                    if !same {
                        println!(
                            "{name}: map{facet}{x}LegacyMUL.uop: {} bytes no entry or table uses differ (zeroed by \
                             any save)",
                            differing_bytes(&unchanged_bytes, &original)
                        );
                    }
                    for file in [rewritten, fresh, unchanged] {
                        std::fs::remove_file(file).unwrap();
                    }
                    Some(map)
                } else {
                    map
                };
                let Some(map) = map else { continue };
                let (Some(index), Some(data)) = (
                    find_file_ignoring_case(&root, &format!("staidx{facet}{x}.mul")),
                    find_file_ignoring_case(&root, &format!("statics{facet}{x}.mul")),
                ) else {
                    continue;
                };
                let (index_bytes, data_bytes) = (std::fs::read(&index).unwrap(), std::fs::read(&data).unwrap());
                let statics = ClassicStaticsBlocks::from_mul_bytes(&index_bytes, &data_bytes, map.size_cells()).unwrap();
                let (index_out, data_out) = statics.to_mul_bytes();
                println!(
                    "{name}: staidx{facet}{x}/statics{facet}{x}.mul ({} statics): {} index and {} data bytes differ when \
                     written back",
                    statics.static_count(),
                    differing_bytes(&index_out, &index_bytes),
                    differing_bytes(&data_out, &data_bytes)
                );
                assert_eq!(
                    ClassicStaticsBlocks::from_mul_bytes(&index_out, &data_out, map.size_cells()).unwrap(),
                    statics
                );
                if !old_layout {
                    assert!(index_out == index_bytes && data_out == data_bytes, "{name}: statics{facet}{x}");
                }
            }
        }
    }
    std::fs::remove_dir_all(out).unwrap();
}

fn tile_key(tile: StaticTile) -> (u16, u8, u8, i8, u16) {
    (tile.graphic, tile.x_offset, tile.y_offset, tile.z, tile.hue)
}

fn map_from(mul: &Option<PathBuf>, facet: u32) -> Option<ClassicMapBlocks> {
    mul.as_ref().map(|path| ClassicMapBlocks::read_mul(path, facet, None).unwrap())
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn baked_difs_match_the_dif_applying_readers() {
    for marker in [CLIENT_ERA_GOLD, CLIENT_ERA_ML] {
        let root = era_root(marker);
        let dirs = [root.clone()];
        for facet in DIF_FACETS {
            let sources = resolve(&dirs, facet, ClassicDifOptions::ALL, MapEdition::Standard).unwrap();
            let (world, map_bake, static_bake) = ClassicWorld::load(&sources, None).unwrap();
            let size = world.size_cells();
            let mut plane = sources.load_map_plane(None, None).unwrap();
            let reader = sources.load_statics_reader(size.width, size.height, None).unwrap();
            let height_blocks = world.map.size_blocks().height;
            let (mut cells_differ, mut blocks_differ) = (0u64, 0u64);
            for bx in 0..world.map.size_blocks().width {
                let mut column: Vec<_> = (0..height_blocks).map(|by| MapBlockRelPos { x: bx, y: by }).collect();
                plane.load_blocks(&mut column).unwrap();
                for by in 0..height_blocks {
                    let read = plane.block_no_update(MapBlockRelPos { x: bx, y: by }).unwrap();
                    let ours = world.map.map_block(bx, by).unwrap();
                    cells_differ +=
                        read.cells.iter().zip(&ours.cells).filter(|(a, b)| (a.id, a.z) != (b.id, b.z)).count() as u64;
                    let block = (bx * height_blocks + by) as usize;
                    let ours: Vec<_> = world.statics.block(block).iter().map(|record| tile_key(record.to_tile())).collect();
                    let read: Vec<_> = reader.read_block(bx, by).unwrap().into_iter().map(tile_key).collect();
                    blocks_differ += u64::from(read != ours);
                }
                plane.evict_idle_blocks(std::time::Duration::ZERO);
            }
            println!(
                "{}: map{facet} ({}x{}) with difs baked (map {map_bake:?}, statics {static_bake:?}): {cells_differ} \
                 cells and {blocks_differ} static blocks differ from the dif-applying readers",
                label(&root),
                size.width,
                size.height
            );
            assert_eq!((cells_differ, blocks_differ), (0, 0));
        }
    }
}

#[test]
#[ignore = "needs a real Classic client: set UO_CC_CLIENT_DIR and run with --ignored --release"]
fn an_edit_writes_one_entry_and_reads_back() {
    let (_, path) = common::client_files(&format!("map{EDITED_FACET}LegacyMUL.uop")).into_iter().next().unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let sources = resolve(&[dir], EDITED_FACET, ClassicDifOptions::NONE, MapEdition::Standard).unwrap();
    let (mut world, _, _) = ClassicWorld::load(&sources, None).unwrap();
    let (x, y) = EDITED_CELL;
    world.map.set_cell(x, y, EDITED_LAND.0, EDITED_LAND.1).unwrap();
    let before = world.statics.cell_statics(x, y).unwrap().len();
    let (graphic, z, hue) = EDITED_STATIC;
    assert_eq!(world.statics.add_static(x, y, graphic, z, hue).unwrap(), before);

    let out = common::temp_dir("classic_map_edit");
    let (uop, mul, index, data) =
        (out.join("map.uop"), out.join("map.mul"), out.join("staidx.mul"), out.join("statics.mul"));
    let report = world.map.write_uop(&uop, EDITED_FACET, Some(UopPackage::load(&path).unwrap())).unwrap();
    assert_eq!((report.written, report.removed), (1, 0), "{report:?}");
    world.map.write_mul(&mul).unwrap();
    world.statics.write_mul(&index, &data).unwrap();
    let size = world.size_cells();
    for mut plane in [
        MapPlane::init_uop_with_size(uop.clone(), EDITED_FACET, Some(size)).unwrap(),
        MapPlane::init_with_size(mul.clone(), EDITED_FACET, Some(size)).unwrap(),
    ] {
        let block = MapBlockRelPos { x: x / 8, y: y / 8 };
        plane.load_blocks(&mut [block]).unwrap();
        let cell = *plane.block(block).unwrap().cell(x % 8, y % 8).unwrap();
        assert_eq!((cell.id, cell.z), EDITED_LAND);
    }
    let reader = StaticsReader::new(&index, &data, size.width, size.height).unwrap();
    let tiles = reader.read_block(x / 8, y / 8).unwrap();
    assert!(tiles.iter().any(|tile| (tile.graphic, tile.z, tile.hue) == EDITED_STATIC));
    println!("edited ({x}, {y}): {report:?}; read back from the UOP, the MUL and the statics");
    std::fs::remove_dir_all(out).unwrap();
}

/// Converts a facet package to Classic and back, and compares the sectors. Returns the Classic facet.
fn facet_round_trip(label: &str, package: &EcFacetPackage, kr_land: Option<&KrLandMap>) -> ClassicWorld {
    let converted = facet_to_classic(package, kr_land).unwrap();
    let losses = converted.losses.describe();
    let (rebuilt, _) = match kr_land {
        Some(land) => build_kr_facet_from_world(&converted.world, package.facet(), None, land).unwrap(),
        None => build_facet_from_world(&converted.world, package.facet(), None).unwrap(),
    };
    let geometry = package.geometry();
    let (identical, squares_differ) = (0..geometry.sector_count())
        .into_par_iter()
        .map(|sector| {
            let (Some(a), Some(b)) =
                (package.read_sector_from_package(sector).unwrap(), rebuilt.read_sector_from_package(sector).unwrap())
            else {
                return (0u32, 0u64);
            };
            let differ = a.tiles.iter().zip(&b.tiles).filter(|(x, y)| x != y).count() as u64;
            (u32::from(differ == 0), differ)
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    println!(
        "{label} ({}x{}) -> Classic ({} statics) -> facet: {identical}/{} sectors identical, {squares_differ} squares \
         differ; losses: {}",
        geometry.width,
        geometry.height,
        converted.world.statics.static_count(),
        geometry.sector_count(),
        if losses.is_empty() { "none".to_string() } else { losses.join("; ") }
    );
    converted.world
}

#[test]
#[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored --release"]
fn enhanced_facets_convert_to_classic_and_back() {
    for facet in FACET_PACKAGES {
        for variant in [false, true] {
            let name = facet_package_file_name(facet, variant);
            for (env, path) in common::client_files(&name) {
                let package = EcFacetPackage::open(&path, facet).unwrap();
                let world = facet_round_trip(&format!("{env}/{name}"), &package, None);
                // The same facet of the Classic client, when one is given.
                let edition = if variant { MapEdition::Variant } else { MapEdition::Standard };
                let Some(dir) = std::env::var_os(CLASSIC_CLIENT_DIR_ENV) else { continue };
                let Some(sources) = resolve(&[PathBuf::from(dir)], u32::from(facet), ClassicDifOptions::NONE, edition)
                else {
                    continue;
                };
                let (classic, _, _) = ClassicWorld::load(&sources, None).unwrap();
                let size = world.size_cells();
                let cells_differ = (0..size.width)
                    .flat_map(|x| (0..size.height).map(move |y| (x, y)))
                    .filter(|&(x, y)| classic.map.cell(x, y).ok() != world.map.cell(x, y).ok())
                    .count();
                let blocks_differ = (0..world.statics.block_count())
                    .filter(|&block| classic.statics.block(block) != world.statics.block(block))
                    .count();
                println!(
                    "  against {CLASSIC_CLIENT_DIR_ENV}'s {}: {cells_differ} cells and {blocks_differ} static blocks differ",
                    sources.map.path.display()
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a real KR client: set UO_KR_CLIENT_DIR and run with --ignored --release"]
fn kr_facets_convert_to_classic_and_back() {
    let land = KrLandMap::shipped();
    for facet in FACET_PACKAGES {
        let name = facet_package_file_name(facet, false);
        let Some(dir) = std::env::var_os(common::KR_CLIENT_DIR_ENV) else { panic!("set {}", common::KR_CLIENT_DIR_ENV) };
        let path = PathBuf::from(dir).join(&name);
        if !path.is_file() {
            continue;
        }
        let package = UopPackage::load(&path).unwrap();
        let dialect = detect_facet_dialect(&package, facet).unwrap();
        let package = EcFacetPackage::from_package_dialect(package, facet, None, dialect).unwrap();
        facet_round_trip(&format!("KR/{name}"), &package, Some(&land));
    }
}

/// Builds a facet of `world` (Enhanced Client, or KR with `kr_land`), converts it back and compares.
fn classic_round_trip(label: &str, world: &ClassicWorld, facet: u8, kr_land: Option<&KrLandMap>) {
    let (package, _) = match kr_land {
        Some(land) => build_kr_facet_from_world(world, facet, None, land).unwrap(),
        None => build_facet_from_world(world, facet, None).unwrap(),
    };
    let back = facet_to_classic(&package, kr_land).unwrap();
    let unsorted = world.unsorted_static_blocks();
    let cells_equal = back.world.map == world.map;
    let blocks_differ = (0..world.statics.block_count())
        .filter(|&block| back.world.statics.block(block) != world.statics.block(block))
        .count();
    println!(
        "{label} ({}x{}, {} statics) -> facet -> Classic: map equal {cells_equal}, {blocks_differ} static blocks differ \
         ({unsorted} blocks of the source are not in (y, x) order); losses: {:?}",
        world.size_cells().width,
        world.size_cells().height,
        world.statics.static_count(),
        back.losses.describe()
    );
    assert!(cells_equal, "{label}: the map differs");
    assert!(blocks_differ <= unsorted, "{label}: {blocks_differ} static blocks differ");
    assert!(back.losses.is_lossless(), "{label}: {:?}", back.losses);
}

#[test]
#[ignore = "needs a real Classic client: set UO_CC_CLIENT_DIR (and UO_KR_CLASSIC_DIR) and run with --ignored --release"]
fn classic_facets_convert_to_facets_and_back() {
    let kr_land = KrLandMap::shipped()
        .with_fallback(KrLandMap::shipped_verified().get(CLASSIC_NODRAW_LAND).expect("the no-draw land"));
    let mut dirs: Vec<(&str, PathBuf)> = Vec::new();
    for env in ["UO_CC_CLIENT_DIR", common::KR_CLASSIC_DIR_ENV] {
        if let Some(dir) = std::env::var_os(env) {
            dirs.push((env, PathBuf::from(dir)));
        }
    }
    assert!(!dirs.is_empty(), "set UO_CC_CLIENT_DIR or {}", common::KR_CLASSIC_DIR_ENV);
    for (env, dir) in dirs {
        for facet in CLASSIC_FACETS {
            let Some(sources) = resolve(&[dir.clone()], facet, ClassicDifOptions::NONE, MapEdition::Standard) else {
                continue;
            };
            let (world, _, _) = ClassicWorld::load(&sources, None).unwrap();
            classic_round_trip(&format!("{env} map{facet} -> EC"), &world, facet as u8, None);
            classic_round_trip(&format!("{env} map{facet} -> KR"), &world, facet as u8, Some(&kr_land));
        }
    }
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn felucca_resizes_between_the_gold_and_mondains_legacy_sizes() {
    let load = |marker: &str| {
        let sources = resolve(&[era_root(marker)], FELUCCA, ClassicDifOptions::ALL, MapEdition::Standard).unwrap();
        ClassicWorld::load(&sources, None).unwrap().0
    };
    let (gold, ml) = (load(CLIENT_ERA_GOLD), load(CLIENT_ERA_ML));
    assert_eq!(gold.size_cells(), GOLD_FELUCCA);
    assert_eq!(ml.size_cells(), ML_FELUCCA);

    // Gold -> ML size -> Gold.
    let (grown, report) = gold.resized(ML_FELUCCA, LandFill::default(), false).unwrap();
    let (back, back_report) = grown.resized(GOLD_FELUCCA, LandFill::default(), false).unwrap();
    assert!(!back_report.drops_content());
    assert!(back == gold, "Gold -> 7168 -> Gold differs");
    // The filled strip against Mondain's Legacy's.
    let (mut same, mut cells, mut ml_void) = (0u64, 0u64, 0u64);
    for x in GOLD_FELUCCA.width..ML_FELUCCA.width {
        for y in 0..ML_FELUCCA.height {
            let ml_cell = ml.map.cell(x, y).unwrap();
            same += u64::from(grown.map.cell(x, y).unwrap() == ml_cell);
            ml_void += u64::from(ml_cell == (VOID_LAND_ID, VOID_LAND_Z));
            cells += 1;
        }
    }
    println!(
        "Gold Felucca grown to {}x{}: {} cells filled; the strip equals Mondain's Legacy's in {same}/{cells} cells \
         (Mondain's Legacy has the fill in {ml_void}); shrunk back equal",
        ML_FELUCCA.width, ML_FELUCCA.height, report.filled_cells
    );
    assert_eq!(same, ml_void);

    // Mondain's Legacy -> Gold size: refused, then cropped.
    let refusal = ml.resized(GOLD_FELUCCA, LandFill::default(), false).unwrap_err();
    println!("Mondain's Legacy Felucca to {}x{}: {refusal}", GOLD_FELUCCA.width, GOLD_FELUCCA.height);
    let (cropped, crop) = ml.resized(GOLD_FELUCCA, LandFill::default(), true).unwrap();
    println!("cropped: {}", crop.describe().join("\n"));
    assert!(crop.drops_content());
    assert_eq!(cropped.size_cells(), GOLD_FELUCCA);
}
