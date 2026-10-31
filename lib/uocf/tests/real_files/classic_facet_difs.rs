//! Checks the Classic facet resolver (`uocf::classic::facet_sources`) on a pre-7.0 client with dif files (UO Gold
//! or Mondain's Legacy, which ship no `map1.mul`), named by `UO_PRE7_CLIENT_DIR`:
//! - Trammel (facet 1) is built from `map0.mul`, `staidx0.mul` and `statics0.mul` with the facet-1 difs, and
//!   differs from Felucca exactly in the blocks `mapdifl1.mul` and `stadifl1.mul` list: those hold the dif's
//!   blocks (an invalid `stadifi1.mul` entry empties its block), every other block is Felucca's.
//! - Felucca read with its map and static difs asked for holds the `mapdif0.mul`/`stadif0.mul` blocks the same way.
//!
//! The dif files are read here straight from their bytes, apart from the readers under test. Run with
//! `--ignored --release`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat, FELUCCA, TRAMMEL};
use uocf::classic::map::{MapBlock, MapBlockRelPos, MapCell, MapPlane};
use uocf::classic::statics::{StaticTile, StaticsReader, StaticsStore};
use uocf::map_edition::MapEdition;

const CLIENT_DIR_ENV: &str = "UO_PRE7_CLIENT_DIR";
const MUL_FIRST: [ClassicMapFormat; 2] = [ClassicMapFormat::Mul, ClassicMapFormat::Uop];
const U32_SIZE: usize = std::mem::size_of::<u32>();
/// A `stadifiN.mul` entry: lookup, length, extra (u32 each).
const INDEX_ENTRY_SIZE: usize = 3 * U32_SIZE;
const INVALID: u32 = u32::MAX;
/// The block header before the 3-byte cells (id u16, z i8).
const MAP_BLOCK_HEADER_SIZE: usize = U32_SIZE;
const MAP_CELL_SIZE: usize = 3;

fn client_dir() -> PathBuf {
    PathBuf::from(std::env::var_os(CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {CLIENT_DIR_ENV}")))
}

fn read(dir: &Path, name: &str) -> Vec<u8> {
    let path = uocf::utils::path::find_file_ignoring_case(dir, name).unwrap_or_else(|| panic!("{name} in {}", dir.display()));
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + U32_SIZE].try_into().expect("u32"))
}

/// The block ids a dif lookup file lists, each with the index of its last patch (the one the clients keep).
fn last_patches(lookup: &[u8]) -> (usize, HashMap<u32, usize>) {
    assert_eq!(lookup.len() % U32_SIZE, 0, "lookup is a whole number of block ids");
    let listed = lookup.len() / U32_SIZE;
    let patches = (0..listed).map(|index| (u32_at(lookup, index * U32_SIZE), index)).collect();
    (listed, patches)
}

/// The (id, z) cells of a 196-byte map block.
fn dif_cells(blocks: &[u8], index: usize) -> Vec<(u16, i8)> {
    let block = &blocks[index * MapBlock::PACKED_SIZE..(index + 1) * MapBlock::PACKED_SIZE];
    block[MAP_BLOCK_HEADER_SIZE..]
        .chunks_exact(MAP_CELL_SIZE)
        .map(|cell| (u16::from_le_bytes([cell[0], cell[1]]), cell[2] as i8))
        .collect()
}

fn plane_cells(cells: &[MapCell]) -> Vec<(u16, i8)> {
    cells.iter().map(|cell| (cell.id, cell.z)).collect()
}

/// (graphic, x, y, z, hue) of each static.
type StaticKey = (u16, u8, u8, i8, u16);

fn dif_statics(index: &[u8], statics: &[u8], entry: usize) -> Vec<StaticKey> {
    let offset = entry * INDEX_ENTRY_SIZE;
    let (lookup, length) = (u32_at(index, offset), u32_at(index, offset + U32_SIZE));
    if lookup == INVALID || length == 0 || length == INVALID {
        return Vec::new();
    }
    let start = lookup as usize;
    statics[start..start + length as usize]
        .chunks_exact(StaticTile::RAW_SIZE)
        .map(|raw| (u16::from_le_bytes([raw[0], raw[1]]), raw[2] & 0x07, raw[3] & 0x07, raw[4] as i8, u16::from_le_bytes([raw[5], raw[6]])))
        .collect()
}

fn store_statics(store: &StaticsStore, block_x: u32, block_y: u32) -> Vec<StaticKey> {
    store
        .block_tiles(block_x, block_y)
        .iter()
        .map(|tile| (tile.graphic, tile.x_offset(), tile.y_offset(), tile.z, tile.hue))
        .collect()
}

#[derive(Debug, Default)]
struct DifReport {
    listed: usize,
    distinct: usize,
    /// Listed blocks whose content differs from the base facet's.
    changed: usize,
    /// Static blocks the dif empties.
    emptied: usize,
}

/// Compares every land block of `patched` with `base`: blocks the `mapdifl{dif_facet}` lists hold the dif's
/// cells, the others the base's.
fn check_land(patched: &mut MapPlane, base: &mut MapPlane, dir: &Path, dif_facet: u32) -> DifReport {
    let (listed, patches) = last_patches(&read(dir, &format!("mapdifl{dif_facet}.mul")));
    let blocks = read(dir, &format!("mapdif{dif_facet}.mul"));
    let (width, height) = (base.size_blocks.width, base.size_blocks.height);
    assert_eq!((patched.size_blocks.width, patched.size_blocks.height), (width, height), "map size");
    let mut report = DifReport { listed, distinct: patches.len(), ..DifReport::default() };
    for x in 0..width {
        let mut column: Vec<_> = (0..height).map(|y| MapBlockRelPos { x, y }).collect();
        patched.load_blocks(&mut column).expect("load patched column");
        base.load_blocks(&mut column).expect("load base column");
        for pos in column {
            let block_id = x * height + pos.y;
            let got = plane_cells(&patched.block_no_update(pos).expect("patched block").cells);
            let base_cells = plane_cells(&base.block_no_update(pos).expect("base block").cells);
            match patches.get(&block_id) {
                Some(&index) => {
                    assert_eq!(got, dif_cells(&blocks, index), "block {block_id} holds mapdif{dif_facet} entry {index}");
                    report.changed += usize::from(got != base_cells);
                }
                None => assert_eq!(got, base_cells, "unlisted block {block_id} is the base facet's"),
            }
        }
        patched.evict_idle_blocks(std::time::Duration::ZERO);
        base.evict_idle_blocks(std::time::Duration::ZERO);
    }
    report
}

/// Compares every static block of `patched` with `base` as [`check_land`] does, with the `stadif{dif_facet}` files.
fn check_statics(patched: &StaticsStore, base: &StaticsStore, dir: &Path, dif_facet: u32) -> DifReport {
    let (listed, patches) = last_patches(&read(dir, &format!("stadifl{dif_facet}.mul")));
    let index = read(dir, &format!("stadifi{dif_facet}.mul"));
    let statics = read(dir, &format!("stadif{dif_facet}.mul"));
    let mut report = DifReport { listed, distinct: patches.len(), ..DifReport::default() };
    for x in 0..base.block_width {
        for y in 0..base.block_height {
            let block_id = x * base.block_height + y;
            let (got, base_statics) = (store_statics(patched, x, y), store_statics(base, x, y));
            match patches.get(&block_id) {
                Some(&entry) => {
                    let expected = dif_statics(&index, &statics, entry);
                    assert_eq!(got, expected, "block {block_id} holds stadif{dif_facet} entry {entry}");
                    report.changed += usize::from(got != base_statics);
                    report.emptied += usize::from(expected.is_empty() && !base_statics.is_empty());
                }
                None => assert_eq!(got, base_statics, "unlisted block {block_id} is the base facet's"),
            }
        }
    }
    report
}

fn base_felucca(dir: &Path) -> (MapPlane, StaticsStore) {
    let felucca = ClassicFacetSources::resolve(
        &[dir.to_path_buf()],
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Standard,
    )
    .expect("resolve Felucca");
    assert!(felucca.map_difs.is_none() && felucca.static_difs.is_none(), "Felucca unpatched without the options");
    let plane = felucca.load_map_plane(None, None).expect("open map0");
    let size = plane.size_cells();
    let statics = felucca.load_statics_reader(size.width, size.height, None).expect("open statics0");
    let store = statics.load_all().expect("load statics0");
    (plane, store)
}

fn patched_store(sources: &ClassicFacetSources, plane: &MapPlane) -> StaticsStore {
    let size = plane.size_cells();
    let reader: StaticsReader = sources.load_statics_reader(size.width, size.height, None).expect("open statics");
    reader.load_all().expect("load statics")
}

#[test]
#[ignore = "needs a pre-7.0 Classic client with dif files: set UO_PRE7_CLIENT_DIR and run with --ignored --release"]
fn trammel_is_felucca_with_the_facet_1_difs() {
    let dir = client_dir();
    let (mut felucca_plane, felucca_statics) = base_felucca(&dir);

    let trammel = ClassicFacetSources::resolve(
        &[dir.clone()],
        TRAMMEL,
        &MUL_FIRST,
        ClassicDifOptions::NONE,
        MapEdition::Standard,
    )
    .expect("resolve Trammel");
    assert_eq!(trammel.base_facet, Some(FELUCCA), "{} has a map1 file", dir.display());
    println!("{}", trammel.base_facet_warning().expect("warning"));
    let mut trammel_plane = trammel.load_map_plane(None, None).expect("open Trammel");
    let trammel_statics = patched_store(&trammel, &trammel_plane);

    let land = check_land(&mut trammel_plane, &mut felucca_plane, &dir, TRAMMEL);
    let statics = check_statics(&trammel_statics, &felucca_statics, &dir, TRAMMEL);
    let size = trammel_plane.size_cells();
    println!(
        "{}: Trammel {}x{} ({} blocks). Land: mapdifl1 lists {} blocks ({} distinct), {} differ from Felucca. \
         Statics: stadifl1 lists {} blocks ({} distinct), {} differ from Felucca, {} emptied; {} statics \
         (Felucca {}).",
        dir.display(),
        size.width,
        size.height,
        trammel_plane.size_blocks.width * trammel_plane.size_blocks.height,
        land.listed,
        land.distinct,
        land.changed,
        statics.listed,
        statics.distinct,
        statics.changed,
        statics.emptied,
        trammel_statics.tiles.len(),
        felucca_statics.tiles.len()
    );
    assert!(land.changed > 0 && statics.changed > 0, "the facet-1 difs change Felucca");
}

#[test]
#[ignore = "needs a pre-7.0 Classic client with dif files: set UO_PRE7_CLIENT_DIR and run with --ignored --release"]
fn felucca_with_its_difs_asked_for_holds_the_facet_0_difs() {
    let dir = client_dir();
    let (mut base_plane, base_statics) = base_felucca(&dir);

    let felucca = ClassicFacetSources::resolve(
        &[dir.clone()],
        FELUCCA,
        &MUL_FIRST,
        ClassicDifOptions::ALL,
        MapEdition::Standard,
    )
    .expect("resolve Felucca with difs");
    assert!(felucca.map_difs.is_some() && felucca.static_difs.is_some(), "{} has the facet-0 difs", dir.display());
    let mut plane = felucca.load_map_plane(None, None).expect("open patched Felucca");
    let statics = patched_store(&felucca, &plane);

    let land = check_land(&mut plane, &mut base_plane, &dir, FELUCCA);
    let static_report = check_statics(&statics, &base_statics, &dir, FELUCCA);
    println!(
        "{}: Felucca with difs. Land: mapdifl0 lists {} blocks ({} distinct), {} differ from map0. Statics: \
         stadifl0 lists {} blocks ({} distinct), {} differ, {} emptied; {} statics (unpatched {}).",
        dir.display(),
        land.listed,
        land.distinct,
        land.changed,
        static_report.listed,
        static_report.distinct,
        static_report.changed,
        static_report.emptied,
        statics.tiles.len(),
        base_statics.tiles.len()
    );
    assert!(land.changed > 0 && static_report.changed > 0, "the facet-0 difs change map0");
}
