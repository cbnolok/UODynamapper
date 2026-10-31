//! Checks the map editions (`uocf::map_edition`) on the 2024 installs measured in
//! `scripts/dev-helpers/ec-research/ghidra-facets/results/facet_variants_data.md`: the Classic resolver reads
//! `mapNxLegacyMUL.uop`/`staidxNx`/`staticsNx` for the variant, and the Enhanced Client lookup picks `facetNx.uop`.
//! The standard and variant data differ exactly as measured there (CC-2024 map0/0x and map1/1x, EC-2024
//! facet0/0x and facet1/1x).
//!
//! The roots come from `UO_CLIENT_ROOTS` (a path list, `:`-separated on Unix); the installs are the folders named
//! [`CLASSIC_2024_FOLDER`] and [`EC_2024_FOLDER`] in one of them. Only reads. Run with `--ignored --release`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
use uocf::classic::map::{MapBlockRelPos, MapPlane};
use uocf::classic::statics::StaticsStore;
use uocf::enhanced::facet_sector::{facet_package_path, sector_entry_path, FacetGeometry};
use uocf::map_edition::MapEdition;
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::UopPackage;

const CLIENT_ROOTS_ENV: &str = "UO_CLIENT_ROOTS";
/// The Classic install with `map*LegacyMUL.uop` dated 2024-06-06 (CC-2024).
const CLASSIC_2024_FOLDER: &str = "_Ultima Online Classic";
/// The Enhanced Client install with the facet packages dated 2024-06-06 (EC-2024).
const EC_2024_FOLDER: &str = "_Ultima Online Enhanced";
const UOP_FIRST: [ClassicMapFormat; 2] = [ClassicMapFormat::Uop, ClassicMapFormat::Mul];

/// The measured difference of a Classic map and its variant: blocks whose land or statics differ, squares whose
/// statics differ, statics only in the variant and only in the standard map.
#[derive(Debug, Default, PartialEq, Eq)]
struct ClassicDiff {
    land_blocks: usize,
    static_blocks: usize,
    static_squares: usize,
    added: usize,
    removed: usize,
}

/// CC-2024 (`facet_variants_data.md`, Summary): statics only.
const CLASSIC_2024_DIFFS: [(u32, ClassicDiff); 2] = [
    (0, ClassicDiff { land_blocks: 0, static_blocks: 1_344, static_squares: 12_792, added: 5_754, removed: 14_075 }),
    (1, ClassicDiff { land_blocks: 0, static_blocks: 439, static_squares: 2_722, added: 3_138, removed: 885 }),
];
/// EC-2024 (`facet_variants_data.md`, Summary): sectors whose payload differs, of 7168.
const EC_2024_SECTOR_DIFFS: [(u8, usize); 2] = [(0, 191), (1, 209)];

fn install(folder: &str) -> PathBuf {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    std::env::split_paths(&roots)
        .map(|root| root.join(folder))
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| panic!("no {folder} under {CLIENT_ROOTS_ENV}"))
}

/// (graphic, x, y, z, hue) of each static of a block, sorted.
type StaticKey = (u16, u8, u8, i8, u16);

fn block_statics(store: &StaticsStore, block_x: u32, block_y: u32) -> Vec<StaticKey> {
    let mut statics: Vec<StaticKey> = store
        .block_tiles(block_x, block_y)
        .iter()
        .map(|tile| (tile.graphic, tile.x_offset(), tile.y_offset(), tile.z, tile.hue))
        .collect();
    statics.sort_unstable();
    statics
}

/// Counts of `a` minus `b` and `b` minus `a`, as multisets (both sorted).
fn multiset_difference(a: &[StaticKey], b: &[StaticKey]) -> (usize, usize) {
    let mut counts: HashMap<StaticKey, isize> = HashMap::new();
    for key in a {
        *counts.entry(*key).or_default() += 1;
    }
    for key in b {
        *counts.entry(*key).or_default() -= 1;
    }
    let only_a = counts.values().filter(|&&count| count > 0).map(|&count| count as usize).sum();
    let only_b = counts.values().filter(|&&count| count < 0).map(|&count| count.unsigned_abs()).sum();
    (only_a, only_b)
}

/// Squares of a block whose statics differ.
fn differing_squares(a: &[StaticKey], b: &[StaticKey]) -> usize {
    let square = |statics: &[StaticKey]| {
        let mut squares: HashMap<(u8, u8), Vec<StaticKey>> = HashMap::new();
        for key in statics {
            squares.entry((key.1, key.2)).or_default().push(*key);
        }
        squares
    };
    let (a, b) = (square(a), square(b));
    let mut positions: Vec<&(u8, u8)> = a.keys().chain(b.keys()).collect();
    positions.sort_unstable();
    positions.dedup();
    positions.into_iter().filter(|position| a.get(position) != b.get(position)).count()
}

fn open(dir: &Path, facet: u32, edition: MapEdition) -> (ClassicFacetSources, MapPlane, StaticsStore) {
    let sources = ClassicFacetSources::resolve(
        &[dir.to_path_buf()],
        facet,
        &UOP_FIRST,
        ClassicDifOptions::NONE,
        edition,
    )
    .unwrap_or_else(|error| panic!("resolve map{facet} {edition}: {error:#}"));
    let plane = sources.load_map_plane(None, None).expect("open map");
    let size = plane.size_cells();
    let store = sources
        .load_statics_reader(size.width, size.height, None)
        .expect("open statics")
        .load_all()
        .expect("load statics");
    (sources, plane, store)
}

fn classic_diff(dir: &Path, facet: u32) -> ClassicDiff {
    let (standard, mut standard_plane, standard_statics) = open(dir, facet, MapEdition::Standard);
    let (variant, mut variant_plane, variant_statics) = open(dir, facet, MapEdition::Variant);
    let x = MapEdition::Variant.file_suffix();
    assert_eq!(variant.edition, MapEdition::Variant);
    assert!(
        variant.map.path.ends_with(format!("map{facet}{x}LegacyMUL.uop")),
        "{}",
        variant.map.path.display()
    );
    assert!(variant.statics_files().unwrap().index.ends_with(format!("staidx{facet}{x}.mul")));
    assert!(standard.map.path.ends_with(format!("map{facet}LegacyMUL.uop")), "{}", standard.map.path.display());

    let (width, height) = (standard_plane.size_blocks.width, standard_plane.size_blocks.height);
    assert_eq!((variant_plane.size_blocks.width, variant_plane.size_blocks.height), (width, height), "map size");
    let mut diff = ClassicDiff::default();
    for block_x in 0..width {
        let mut column: Vec<_> = (0..height).map(|y| MapBlockRelPos { x: block_x, y }).collect();
        standard_plane.load_blocks(&mut column).expect("load standard column");
        variant_plane.load_blocks(&mut column).expect("load variant column");
        for pos in column {
            let cells = |plane: &MapPlane| -> Vec<(u16, i8)> {
                plane.block_no_update(pos).expect("block").cells.iter().map(|cell| (cell.id, cell.z)).collect()
            };
            diff.land_blocks += usize::from(cells(&standard_plane) != cells(&variant_plane));
            let (a, b) = (block_statics(&standard_statics, pos.x, pos.y), block_statics(&variant_statics, pos.x, pos.y));
            if a != b {
                let (removed, added) = multiset_difference(&a, &b);
                diff.static_blocks += 1;
                diff.static_squares += differing_squares(&a, &b);
                diff.added += added;
                diff.removed += removed;
            }
        }
        standard_plane.evict_idle_blocks(std::time::Duration::ZERO);
        variant_plane.evict_idle_blocks(std::time::Duration::ZERO);
    }
    diff
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn classic_2024_variant_maps_differ_as_measured() {
    let dir = install(CLASSIC_2024_FOLDER);
    for (facet, expected) in CLASSIC_2024_DIFFS {
        let diff = classic_diff(&dir, facet);
        println!("{}: map{facet} / map{facet}x: {diff:?}", dir.display());
        assert_eq!(diff, expected, "map{facet} / map{facet}x");
    }
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored --release"]
fn ec_2024_variant_packages_differ_as_measured() {
    let dir = install(EC_2024_FOLDER);
    for (facet, expected) in EC_2024_SECTOR_DIFFS {
        let standard_path = facet_package_path(&dir, facet, MapEdition::Standard).expect("standard package");
        let variant_path = facet_package_path(&dir, facet, MapEdition::Variant).expect("variant package");
        assert!(variant_path.ends_with(format!("facet{facet}x.uop")), "{}", variant_path.display());
        let standard = UopPackage::load(&standard_path).expect("load standard package");
        let variant = UopPackage::load(&variant_path).expect("load variant package");
        let geometry = FacetGeometry::client_facet(facet).expect("client facet");
        let differing = (0..geometry.sector_count())
            .filter(|&sector| {
                let hash = hash_file_name_single(&sector_entry_path(facet, sector));
                let payload = |package: &UopPackage| package.unpack_file_by_hash(hash).expect("unpack").expect("sector");
                payload(&standard) != payload(&variant)
            })
            .count();
        println!(
            "{}: facet{facet} / facet{facet}x: {differing} of {} sectors differ",
            dir.display(),
            geometry.sector_count()
        );
        assert_eq!(differing, expected, "facet{facet} / facet{facet}x");
    }
}
