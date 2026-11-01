//! Every Classic gump of the clients in `UO_CLASSIC_CLIENT_ROOTS` (directories separated like `PATH`; each with
//! `gumpartLegacyMUL.uop` or `gumpidx.mul` and `gumpart.mul`) encodes back byte for byte, and the UOP entries that do
//! not decode are the 8-byte 0x0 placeholders.

use std::path::{Path, PathBuf};

use uocf::classic::generic_index::IndexFile;
use uocf::classic::gump_write::GumpColors;
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
const UOP_FILE: &str = "gumpartLegacyMUL.uop";
const INDEX_FILE: &str = "gumpidx.mul";
const MUL_FILE: &str = "gumpart.mul";
/// A placeholder entry: a zero width and height, no rows.
const PLACEHOLDER: [u8; 8] = [0; 8];
const INDEX_WIDTH_SHIFT: u32 = u16::BITS;
const INDEX_HEIGHT_MASK: u32 = u16::MAX as u32;

/// Encoded back, placeholders.
fn check_uop(path: &Path) -> (usize, usize) {
    let package = UopPackage::load(path).unwrap();
    let (mut encoded, mut placeholders) = (0, 0);
    for file in package.iter_files().filter(|file| !file.is_empty_slot()) {
        let payload = package.unpack_file_by_hash(file.filename_hash()).unwrap().unwrap();
        match GumpColors::decode_uop(&payload) {
            Ok(gump) => {
                assert!(gump.encode_uop().unwrap() == payload, "{} {:016X} encodes back byte for byte", path.display(), file.filename_hash());
                encoded += 1;
            }
            Err(error) => {
                assert_eq!(payload, PLACEHOLDER, "{} {:016X}: {error:#}", path.display(), file.filename_hash());
                placeholders += 1;
            }
        }
    }
    (encoded, placeholders)
}

fn check_mul(index: &Path, mul: &Path) -> usize {
    let index = IndexFile::load(index.to_path_buf()).unwrap();
    let data = std::fs::read(mul).unwrap();
    let mut encoded = 0;
    for slot in 0..index.element_count() {
        let entry = index.element(slot).unwrap();
        // Empty slots: no index entry, or 0 bytes (7.0.20 lists some with a 0x0 extra).
        let (Some(offset), Some(length @ 1..), Some(extra)) = (entry.lookup(), entry.len(), entry.extra()) else { continue };
        let rows = &data[offset as usize..offset as usize + length as usize];
        let gump = GumpColors::decode_rows(rows, (extra >> INDEX_WIDTH_SHIFT) as u16, (extra & INDEX_HEIGHT_MASK) as u16)
            .unwrap_or_else(|error| panic!("{} slot {slot}: {error:#}", mul.display()));
        assert!(gump.encode_rows().unwrap() == rows, "{} slot {slot} encodes back byte for byte", mul.display());
        assert_eq!(gump.index_extra(), extra);
        encoded += 1;
    }
    encoded
}

#[test]
#[ignore = "needs real clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
fn every_shipped_gump_encodes_back() {
    let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
    for root in std::env::split_paths(&roots).collect::<Vec<PathBuf>>() {
        if let Some(uop) = find_file_ignoring_case(&root, UOP_FILE) {
            let (encoded, placeholders) = check_uop(&uop);
            println!("{}: {encoded} UOP gumps encoded back byte for byte; {placeholders} 0x0 placeholders", root.display());
        }
        if let (Some(index), Some(mul)) = (find_file_ignoring_case(&root, INDEX_FILE), find_file_ignoring_case(&root, MUL_FILE)) {
            println!("{}: {} MUL gumps encoded back byte for byte", root.display(), check_mul(&index, &mul));
        }
    }
}
