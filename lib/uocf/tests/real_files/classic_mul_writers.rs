//! The Classic MUL writers on real clients of every era: `tiledata.mul` (every layout), `radarcol.mul`,
//! `animdata.mul` and `Multimap.rle` read and write back byte for byte, and a `tiledata.mul` edit changes only the
//! edited fields' bytes.
//!
//! `UO_CLASSIC_CLIENT_ROOTS` lists the client directories (separated like `PATH`); a file a client lacks is
//! skipped. Nothing is written to disk. Run with `--ignored --release`.

use std::path::{Path, PathBuf};

use uocf::classic::animdata::AnimData;
use uocf::classic::multimap_rle::{decode_rle_bytes, encode_rle_bytes};
use uocf::classic::radarcol::{encode_radarcol, load_radarcol};
use uocf::classic::tiledata::TileData;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const TILEDATA_FILE: &str = "tiledata.mul";
const RADARCOL_FILE: &str = "radarcol.mul";
const ANIMDATA_FILE: &str = "animdata.mul";
const MULTIMAP_FILE: &str = "Multimap.rle";
/// The item tile and the land tile the edit test changes.
const EDITED_ITEM: usize = 0x0E7D;
const EDITED_LAND: usize = 3;
/// The weight byte and the height byte are distinct single bytes of the item record; the land texture id is two.
const EDITED_ITEM_BYTES: usize = 2;
const EDITED_LAND_BYTES: usize = 2;

fn client_roots() -> Vec<PathBuf> {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    let roots: Vec<_> = std::env::split_paths(&roots).filter(|root| !root.as_os_str().is_empty()).collect();
    assert!(!roots.is_empty(), "{CLIENT_ROOTS_ENV} names no directory");
    roots
}

fn label(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

#[test]
#[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored --release"]
fn classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte() {
    let mut checked = 0usize;
    for root in client_roots() {
        let name = label(&root);
        if let Some(path) = find_file_ignoring_case(&root, TILEDATA_FILE) {
            let source = std::fs::read(&path).unwrap();
            let mut tiledata = TileData::load(path.clone()).unwrap();
            assert_eq!(tiledata.to_bytes().unwrap(), source, "{name}: {TILEDATA_FILE}");
            let (land_size, item_size) = tiledata.record_sizes();
            println!("{name}: {TILEDATA_FILE} ({land_size}/{item_size}-byte records, {} items) written back byte for byte", tiledata.item_tiles().len());
            let item = &mut tiledata.item_tiles_mut()[EDITED_ITEM];
            item.weight = item.weight.wrapping_add(1);
            item.height = item.height.wrapping_add(1);
            let land = &mut tiledata.land_tiles_mut()[EDITED_LAND];
            land.texture_id = !land.texture_id;
            let edited = tiledata.to_bytes().unwrap();
            assert_eq!(differing_bytes(&edited, &source), EDITED_ITEM_BYTES + EDITED_LAND_BYTES, "{name}: edit");
            checked += 1;
        }
        if let Some(path) = find_file_ignoring_case(&root, RADARCOL_FILE) {
            let source = std::fs::read(&path).unwrap();
            assert_eq!(encode_radarcol(&load_radarcol(&path).unwrap()), source, "{name}: {RADARCOL_FILE}");
            println!("{name}: {RADARCOL_FILE} written back byte for byte");
            checked += 1;
        }
        if let Some(path) = find_file_ignoring_case(&root, ANIMDATA_FILE) {
            let source = std::fs::read(&path).unwrap();
            assert_eq!(AnimData::from_bytes(&source).unwrap().to_bytes().unwrap(), source, "{name}: {ANIMDATA_FILE}");
            println!("{name}: {ANIMDATA_FILE} written back byte for byte");
            checked += 1;
        }
        if let Some(path) = find_file_ignoring_case(&root, MULTIMAP_FILE) {
            let source = std::fs::read(&path).unwrap();
            assert_eq!(encode_rle_bytes(&decode_rle_bytes(&source).unwrap()), source, "{name}: {MULTIMAP_FILE}");
            println!("{name}: {MULTIMAP_FILE} written back byte for byte");
            checked += 1;
        }
    }
    assert!(checked > 0, "no client in {CLIENT_ROOTS_ENV} has any of the files");
}
