//! Entry edits of UOP packages (add, remove, list) and the layout-preserving writer (`SaveLayout::Preserve`), on
//! hand-built packages laid out like the client's and on every package of a real client, and the zlib encoder on
//! every zlib entry of a real client.
//!
//! The client directories come from the variables in `common::CLIENT_DIR_ENVS`.

mod common;

use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use uocf::uop_container::data_block_header::{DataBlockHeader, DataBlockHeaderKind};
use rayon::prelude::*;
use uocf::uop_container::file::{CompressionFlag, UopFile};
use uocf::uop_container::hash::{hash_data_block, hash_file_name_single};
use uocf::uop_container::package::{FileReplacement, LoadMode, NewEntryCompression, SaveLayout, UopPackage};

const MAGIC: &[u8; 4] = b"MYP\0";
const PACKAGE_MISC: u32 = 0xFD23_EC43;
/// Size of the header fields up to `file_count`.
const FIXED_HEADER_SIZE: usize = 28;
/// Size of the header the compact writer emits.
const COMPACT_HEADER_SIZE: u64 = 32;
const BLOCK_HEADER_SIZE: usize = 12;
const FILE_RECORD_SIZE: usize = 34;
/// Version and header size of the hand-built package whose block table precedes its payloads, as in version-4
/// client packages.
const TABLE_FIRST_VERSION: u32 = 4;
const TABLE_FIRST_HEADER_SIZE: usize = 40;
/// Version and header size of the hand-built package whose payloads precede its block table, as in version-5
/// client packages; its header holds `u32 1`, `u32 1` after the fixed fields.
const PAYLOADS_FIRST_VERSION: u32 = 5;
const PAYLOADS_FIRST_HEADER_SIZE: usize = 64;
const PAYLOADS_FIRST_HEADER_WORDS: [u32; 2] = [1, 1];
/// Unused bytes between the first payload and the block table of the payloads-first package.
const PAYLOADS_FIRST_GAP: usize = 4;
/// Block size of the hand-built packages: the table-first one holds two records (one free), the payloads-first
/// one two entries and an empty slot.
const BLOCK_SIZE: u32 = 3;
/// FILETIME of the headers the hand-built entries start with.
const SOURCE_FILETIME: u64 = 0x01da_8abf_ed6a_8a60;
/// FILETIME injected as the edit time.
const EDIT_FILETIME: u64 = 0x01db_2a3b_4c5d_6e7f;
const ALPHA: &str = "build/test/alpha.bin";
const BRAVO: &str = "build/test/bravo.bin";
const CHARLIE: &str = "build/test/charlie.bin";
const DELTA: &str = "build/test/delta.bin";
const ALPHA_PAYLOAD: &[u8] = b"alpha payload";
const BRAVO_PAYLOAD: &[u8] = b"bravo payload, the longer one";
const SHORTER_PAYLOAD: &[u8] = b"short";
const LONGER_PAYLOAD: &[u8] = b"a payload longer than the alpha payload";
const CHARLIE_PAYLOAD: &[u8] = b"charlie payload";
const DELTA_PAYLOAD: &[u8] = b"delta payload";
/// Package file extension.
const UOP_EXTENSION: &str = "uop";
/// Bytes compared per read when comparing package files.
const COMPARE_CHUNK_SIZE: usize = 1 << 20;
/// Bits flipped in the last payload byte of the entry the real-client test replaces.
const FLIPPED_BITS: u8 = 0xFF;

/// Where the hand-built package puts its block table relative to its payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientLayout {
    TableFirst,
    PayloadsFirst,
}

/// One entry of a hand-built package: its timestamp header and uncompressed payload.
fn stored_entry(payload: &[u8]) -> Vec<u8> {
    let mut bytes = DataBlockHeader::from_timestamp(SOURCE_FILETIME).to_bytes();
    bytes.extend_from_slice(payload);
    bytes
}

fn file_record(address: usize, name: &str, payload: &[u8]) -> Vec<u8> {
    let header = DataBlockHeader::from_timestamp(SOURCE_FILETIME).to_bytes();
    let mut record = Vec::with_capacity(FILE_RECORD_SIZE);
    record.extend_from_slice(&(address as u64).to_le_bytes());
    record.extend_from_slice(&(header.len() as u32).to_le_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&hash_file_name_single(name).to_le_bytes());
    record.extend_from_slice(&hash_data_block(&header).unwrap().to_le_bytes());
    record.extend_from_slice(&(CompressionFlag::None as i16).to_le_bytes());
    record
}

fn header_fields(version: u32, start_address: usize, file_count: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FIXED_HEADER_SIZE);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&PACKAGE_MISC.to_le_bytes());
    bytes.extend_from_slice(&(start_address as u64).to_le_bytes());
    bytes.extend_from_slice(&BLOCK_SIZE.to_le_bytes());
    bytes.extend_from_slice(&file_count.to_le_bytes());
    bytes
}

/// A package holding `ALPHA` and `BRAVO` laid out like client packages of the version `layout` stands for.
fn client_like_package(layout: ClientLayout) -> Vec<u8> {
    let (alpha, bravo) = (stored_entry(ALPHA_PAYLOAD), stored_entry(BRAVO_PAYLOAD));
    let table_capacity = BLOCK_HEADER_SIZE + FILE_RECORD_SIZE * BLOCK_SIZE as usize;
    match layout {
        ClientLayout::TableFirst => {
            let table = TABLE_FIRST_HEADER_SIZE;
            let alpha_address = table + table_capacity;
            let bravo_address = alpha_address + alpha.len();
            let mut bytes = header_fields(TABLE_FIRST_VERSION, table, 2);
            bytes.resize(TABLE_FIRST_HEADER_SIZE, 0);
            bytes.extend_from_slice(&2u32.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            bytes.extend(file_record(alpha_address, ALPHA, ALPHA_PAYLOAD));
            bytes.extend(file_record(bravo_address, BRAVO, BRAVO_PAYLOAD));
            bytes.resize(alpha_address, 0);
            bytes.extend(alpha);
            bytes.extend(bravo);
            bytes
        }
        ClientLayout::PayloadsFirst => {
            let alpha_address = PAYLOADS_FIRST_HEADER_SIZE;
            let table = alpha_address + alpha.len() + PAYLOADS_FIRST_GAP;
            let bravo_address = table + table_capacity;
            let mut bytes = header_fields(PAYLOADS_FIRST_VERSION, table, 2);
            for word in PAYLOADS_FIRST_HEADER_WORDS {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            bytes.resize(PAYLOADS_FIRST_HEADER_SIZE, 0);
            bytes.extend(alpha);
            bytes.resize(table, 0);
            bytes.extend_from_slice(&BLOCK_SIZE.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            bytes.extend(file_record(alpha_address, ALPHA, ALPHA_PAYLOAD));
            bytes.extend(file_record(bravo_address, BRAVO, BRAVO_PAYLOAD));
            bytes.resize(bravo_address, 0);
            bytes.extend(bravo);
            bytes
        }
    }
}

fn write_client_like_package(dir: &Path, layout: ClientLayout) -> (PathBuf, Vec<u8>) {
    let bytes = client_like_package(layout);
    let path = dir.join(format!("{layout:?}.uop"));
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

/// The bytes the entry `name` stores in `package_bytes`: header and payload.
fn stored_bytes<'a>(package_bytes: &'a [u8], package: &UopPackage, name: &str) -> &'a [u8] {
    let file = package.get_file_by_name(name).unwrap();
    let start = file.data_block_address() as usize;
    &package_bytes[start..start + file.stored_length() as usize]
}

fn assert_zero(bytes: &[u8], context: &str) {
    assert!(bytes.iter().all(|&byte| byte == 0), "{context}: expected zeros");
}

#[test]
fn unchanged_client_layouts_resave_byte_for_byte() {
    let dir = common::temp_dir("uop_preserve_noop");
    for layout in [ClientLayout::TableFirst, ClientLayout::PayloadsFirst] {
        let (source, bytes) = write_client_like_package(&dir, layout);
        for mode in [LoadMode::Eager, LoadMode::Lazy] {
            let output = dir.join(format!("{layout:?}_{mode:?}_noop.uop"));
            UopPackage::load_with_mode(&source, mode).unwrap().finalize_and_save(&output).unwrap();
            assert_eq!(fs::read(&output).unwrap(), bytes, "{layout:?} {mode:?}");
        }

        let compact = dir.join(format!("{layout:?}_compact.uop"));
        UopPackage::load(&source).unwrap().finalize_and_save_with_layout(&compact, SaveLayout::Compact).unwrap();
        let reloaded = UopPackage::load(&compact).unwrap();
        assert_eq!(reloaded.blocks()[0].source().unwrap().address, COMPACT_HEADER_SIZE, "{layout:?}: compact layout");
        for (name, payload) in [(ALPHA, ALPHA_PAYLOAD), (BRAVO, BRAVO_PAYLOAD)] {
            assert_eq!(reloaded.get_file_by_name(name).unwrap().unpack().unwrap(), payload, "{layout:?} compact");
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

/// Table-first: a replaced entry that fits stays, a removed one leaves zeros and an empty slot, and an added
/// entry extends the table into its reserved room and goes after the source file.
#[test]
fn table_first_edits_keep_untouched_bytes_in_place() {
    let dir = common::temp_dir("uop_preserve_table_first");
    let (source_path, source_bytes) = write_client_like_package(&dir, ClientLayout::TableFirst);
    let source = UopPackage::load(&source_path).unwrap();
    let mut package = UopPackage::load(&source_path).unwrap();
    package.set_edit_filetime(Some(EDIT_FILETIME));
    assert_eq!(package.replace_file(hash_file_name_single(ALPHA), SHORTER_PAYLOAD, CompressionFlag::None).unwrap(), FileReplacement::Replaced);
    package.add_entry_by_name(CHARLIE, CHARLIE_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::Zlib)).unwrap();
    assert!(package.remove_entry(hash_file_name_single(BRAVO)));
    let output = dir.join("edited.uop");
    package.finalize_and_save(&output).unwrap();

    let bytes = fs::read(&output).unwrap();
    let written = UopPackage::load(&output).unwrap();
    assert_eq!(written.file_count(), 2);
    assert_eq!(written.blocks().len(), 1);
    assert_eq!(written.blocks()[0].source().unwrap().address, TABLE_FIRST_HEADER_SIZE as u64, "the table stays");
    assert_eq!(&bytes[FIXED_HEADER_SIZE..TABLE_FIRST_HEADER_SIZE], &source_bytes[FIXED_HEADER_SIZE..TABLE_FIRST_HEADER_SIZE]);

    let (alpha_before, alpha) = (source.get_file_by_name(ALPHA).unwrap(), written.get_file_by_name(ALPHA).unwrap());
    assert_eq!(alpha.data_block_address(), alpha_before.data_block_address(), "a replaced entry that fits stays");
    assert_eq!(alpha.unpack().unwrap(), SHORTER_PAYLOAD);
    assert_zero(&bytes[alpha.source_extent().unwrap().end() as usize..alpha_before.source_extent().unwrap().end() as usize], "the rest of the replaced entry");
    let bravo_before = source.get_file_by_name(BRAVO).unwrap().source_extent().unwrap();
    assert_zero(&bytes[bravo_before.address as usize..bravo_before.end() as usize], "the removed entry");
    assert!(written.blocks()[0].files()[1].is_empty_slot(), "the removed entry leaves an empty slot");

    let charlie = written.get_file_by_name(CHARLIE).unwrap();
    assert_eq!(written.blocks()[0].files().len(), 3, "the added entry extends the table in its reserved room");
    assert_eq!(charlie.data_block_address(), source_bytes.len() as u64, "the added payload goes after the source file");
    assert_eq!(bytes.len() as u64, charlie.source_extent().unwrap().end());
    assert_eq!(charlie.compression(), CompressionFlag::Zlib);
    assert_eq!(charlie.unpack().unwrap(), CHARLIE_PAYLOAD);
    fs::remove_dir_all(dir).unwrap();
}

/// Payloads-first: an added entry fills the empty slot, a longer replacement moves to the end and zeroes its old
/// bytes, and an entry beyond the block size opens a new block after the source file.
#[test]
fn payloads_first_edits_move_only_what_does_not_fit() {
    let dir = common::temp_dir("uop_preserve_payloads_first");
    let (source_path, source_bytes) = write_client_like_package(&dir, ClientLayout::PayloadsFirst);
    let source = UopPackage::load(&source_path).unwrap();
    let mut package = UopPackage::load_with_mode(&source_path, LoadMode::Lazy).unwrap();
    package.set_edit_filetime(Some(EDIT_FILETIME));
    package.replace_file(hash_file_name_single(ALPHA), LONGER_PAYLOAD, CompressionFlag::None).unwrap();
    package.add_entry_by_name(CHARLIE, CHARLIE_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
    package.add_entry_by_name(DELTA, DELTA_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
    let output = dir.join("edited.uop");
    package.finalize_and_save(&output).unwrap();

    let bytes = fs::read(&output).unwrap();
    let written = UopPackage::load(&output).unwrap();
    assert_eq!(written.file_count(), 4);
    assert_eq!(&bytes[FIXED_HEADER_SIZE..PAYLOADS_FIRST_HEADER_SIZE], &source_bytes[FIXED_HEADER_SIZE..PAYLOADS_FIRST_HEADER_SIZE]);
    assert_eq!(stored_bytes(&bytes, &written, BRAVO), stored_bytes(&source_bytes, &source, BRAVO), "untouched");
    assert_eq!(written.get_file_by_name(BRAVO).unwrap().data_block_address(), source.get_file_by_name(BRAVO).unwrap().data_block_address());

    let block = &written.blocks()[0];
    assert_eq!(block.source().unwrap().address, source.blocks()[0].source().unwrap().address, "the table stays");
    assert_eq!(block.files()[2].filename_hash(), hash_file_name_single(CHARLIE), "the added entry fills the empty slot");

    // Moved and added bytes follow the source file in block chain and slot order: alpha, charlie, then the new
    // block's table with room for the block size, then delta.
    let alpha_before = source.get_file_by_name(ALPHA).unwrap().source_extent().unwrap();
    assert_zero(&bytes[alpha_before.address as usize..alpha_before.end() as usize], "the moved entry's old bytes");
    let alpha = written.get_file_by_name(ALPHA).unwrap();
    assert_eq!(alpha.data_block_address(), source_bytes.len() as u64, "a longer replacement moves to the end");
    assert_eq!(alpha.unpack().unwrap(), LONGER_PAYLOAD);
    let charlie = written.get_file_by_name(CHARLIE).unwrap();
    assert_eq!(charlie.data_block_address(), alpha.source_extent().unwrap().end());
    let new_block = &written.blocks()[1];
    let new_block_address = new_block.source().unwrap().address;
    assert_eq!(new_block_address, charlie.source_extent().unwrap().end(), "the new block follows");
    let delta = written.get_file_by_name(DELTA).unwrap();
    let reserved_table = (BLOCK_HEADER_SIZE + FILE_RECORD_SIZE * BLOCK_SIZE as usize) as u64;
    assert_eq!(delta.data_block_address(), new_block_address + reserved_table, "a new table reserves the block size");
    assert_eq!(bytes.len() as u64, delta.source_extent().unwrap().end());
    assert_eq!(delta.unpack().unwrap(), DELTA_PAYLOAD);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn entries_are_added_removed_and_listed() {
    let dir = common::temp_dir("uop_entry_edits");
    let (source_path, _) = write_client_like_package(&dir, ClientLayout::PayloadsFirst);
    let mut package = UopPackage::load_with_mode(&source_path, LoadMode::Lazy).unwrap();
    package.set_edit_filetime(Some(EDIT_FILETIME));

    let error = package.add_entry_by_name(ALPHA, CHARLIE_PAYLOAD, NewEntryCompression::LikePackage).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "duplicate hash");
    let error = package.add_entry_by_name(CHARLIE, &[], NewEntryCompression::LikePackage).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "empty payload");

    let charlie = package.add_entry_by_name(CHARLIE, CHARLIE_PAYLOAD, NewEntryCompression::LikePackage).unwrap();
    assert_eq!(charlie, hash_file_name_single(CHARLIE));
    assert_eq!(package.file_count(), 3);
    let added = package.get_file_by_hash(charlie).unwrap();
    assert_eq!(added.compression(), CompressionFlag::None, "like the package's uncompressed entries");
    assert_eq!(added.unpack().unwrap(), CHARLIE_PAYLOAD);

    let names = |hash: u64| (hash == hash_file_name_single(ALPHA)).then(|| ALPHA.to_owned());
    let entries = package.list_entries(names).unwrap();
    let summary: Vec<_> = entries
        .iter()
        .map(|entry| (entry.filename_hash, entry.name.as_deref(), entry.header, entry.compression, entry.block_index, entry.slot_index))
        .collect();
    assert_eq!(
        summary,
        vec![
            (hash_file_name_single(ALPHA), Some(ALPHA), DataBlockHeaderKind::Timestamp(SOURCE_FILETIME), CompressionFlag::None, 0, 0),
            (hash_file_name_single(BRAVO), None, DataBlockHeaderKind::Timestamp(SOURCE_FILETIME), CompressionFlag::None, 0, 1),
            (charlie, None, DataBlockHeaderKind::Timestamp(EDIT_FILETIME), CompressionFlag::None, 0, 2),
        ]
    );
    assert_eq!(entries[2].data_block_hash, added.data_block_hash());
    assert_eq!(entries[2].decompressed_size as usize, CHARLIE_PAYLOAD.len());

    assert!(package.remove_entry(hash_file_name_single(ALPHA)));
    assert!(!package.remove_entry(hash_file_name_single(ALPHA)));
    assert!(package.get_file_by_name(ALPHA).is_none());
    assert_eq!(package.file_count(), 2);
    assert!(package.blocks()[0].files()[0].is_empty_slot());
    package.add_entry_by_name(DELTA, DELTA_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::Zlib)).unwrap();
    assert_eq!(package.blocks()[0].files()[0].filename_hash(), hash_file_name_single(DELTA), "fills the freed slot");
    assert_eq!(package.blocks().len(), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn new_entries_take_the_compression_of_most_entries() {
    let mut package = UopPackage::new_default();
    package.add_entry_by_name(ALPHA, ALPHA_PAYLOAD, NewEntryCompression::LikePackage).unwrap();
    assert_eq!(package.get_file_by_name(ALPHA).unwrap().compression(), CompressionFlag::Zlib, "an empty package");
    package.add_entry_by_name(BRAVO, BRAVO_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
    package.add_entry_by_name(CHARLIE, CHARLIE_PAYLOAD, NewEntryCompression::LikePackage).unwrap();
    assert_eq!(package.get_file_by_name(CHARLIE).unwrap().compression(), CompressionFlag::Zlib, "a tie");
    package.add_entry_by_name(DELTA, DELTA_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
    package.add_entry_by_name("build/test/echo.bin", ALPHA_PAYLOAD, NewEntryCompression::Flag(CompressionFlag::None)).unwrap();
    package.add_entry_by_name("build/test/foxtrot.bin", ALPHA_PAYLOAD, NewEntryCompression::LikePackage).unwrap();
    assert_eq!(package.get_file_by_name("build/test/foxtrot.bin").unwrap().compression(), CompressionFlag::None);
}

/// The byte ranges where two files differ, merged into runs, plus a run past the shorter file's end.
fn differing_runs(left: &Path, right: &Path) -> Vec<(u64, u64)> {
    let mut readers = [left, right].map(|path| BufReader::with_capacity(COMPARE_CHUNK_SIZE, fs::File::open(path).unwrap()));
    let lengths = [left, right].map(|path| fs::metadata(path).unwrap().len());
    let common_length = lengths[0].min(lengths[1]);
    let (mut buffers, mut runs, mut offset) = ([vec![0u8; COMPARE_CHUNK_SIZE], vec![0u8; COMPARE_CHUNK_SIZE]], Vec::new(), 0u64);
    let mut current: Option<(u64, u64)> = None;
    while offset < common_length {
        let chunk = (common_length - offset).min(COMPARE_CHUNK_SIZE as u64) as usize;
        for (reader, buffer) in readers.iter_mut().zip(buffers.iter_mut()) {
            reader.read_exact(&mut buffer[..chunk]).unwrap();
        }
        if buffers[0][..chunk] != buffers[1][..chunk] {
            for index in 0..chunk {
                if buffers[0][index] != buffers[1][index] {
                    let at = offset + index as u64;
                    current = match current {
                        Some((start, end)) if end == at => Some((start, at + 1)),
                        Some(run) => {
                            runs.push(run);
                            Some((at, at + 1))
                        }
                        None => Some((at, at + 1)),
                    };
                }
            }
        }
        offset += chunk as u64;
    }
    runs.extend(current);
    if lengths[0] != lengths[1] {
        runs.push((common_length, lengths[0].max(lengths[1])));
    }
    runs
}

fn client_packages(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("read client directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(UOP_EXTENSION)))
        .collect();
    paths.sort();
    paths
}

/// Every package of the client directories re-saves byte for byte, and replacing its largest entry changes only
/// that entry's record, its old bytes and, when it no longer fits, the bytes appended after the source file.
/// Set `TMPDIR` to a disk with room for the largest package written twice.
#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn client_packages_resave_byte_for_byte_and_replace_one_entry_in_place() {
    let mut checked = 0usize;
    for env in common::CLIENT_DIR_ENVS {
        let Some(dir) = std::env::var_os(env) else { continue };
        let out = common::temp_dir("uop_client_resave");
        for path in client_packages(Path::new(&dir)) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source_length = fs::metadata(&path).unwrap().len();
            let mut package = UopPackage::load(&path).unwrap();
            package.set_edit_filetime(Some(EDIT_FILETIME));

            let noop = out.join(format!("noop_{name}"));
            package.finalize_and_save(&noop).unwrap();
            let noop_runs = differing_runs(&path, &noop);
            fs::remove_file(&noop).unwrap();
            assert!(noop_runs.is_empty(), "{env} {name}: an unchanged re-save differs at {:?}", &noop_runs[..noop_runs.len().min(4)]);

            let (block_index, slot_index, target) = package
                .blocks()
                .iter()
                .enumerate()
                .flat_map(|(block_index, block)| {
                    block.files().iter().enumerate().map(move |(slot_index, file)| (block_index, slot_index, file))
                })
                .filter(|(_, _, file)| !file.is_empty_slot())
                .max_by_key(|(_, _, file)| file.decompressed_size())
                .map(|(block_index, slot_index, file)| (block_index, slot_index, file.clone()))
                .unwrap();
            let record = package.blocks()[block_index].source().unwrap().address
                + (BLOCK_HEADER_SIZE + FILE_RECORD_SIZE * slot_index) as u64;
            let old_extent = target.source_extent().unwrap();
            let mut payload = target.unpack().unwrap();
            *payload.last_mut().unwrap() ^= FLIPPED_BITS;
            let hash = target.filename_hash();
            assert_eq!(package.replace_file(hash, &payload, target.compression()).unwrap(), FileReplacement::Replaced);
            let new_length = package.get_file_by_hash(hash).unwrap().stored_length();
            let edited = out.join(format!("edited_{name}"));
            package.finalize_and_save(&edited).unwrap();

            let in_place = new_length <= old_extent.length;
            let runs = differing_runs(&path, &edited);
            let allowed =
                [(record, record + FILE_RECORD_SIZE as u64), (old_extent.address, old_extent.end()), (source_length, u64::MAX)];
            for &(start, end) in &runs {
                assert!(
                    allowed.iter().any(|&(low, high)| low <= start && end <= high),
                    "{env} {name}: bytes {start}..{end} changed outside the edited entry"
                );
            }
            let edited_length = fs::metadata(&edited).unwrap().len();
            let expected_length = if in_place { source_length } else { source_length + new_length };
            assert_eq!(edited_length, expected_length, "{env} {name}");
            let mut reloaded = UopPackage::load_with_mode(&edited, LoadMode::Lazy).unwrap();
            reloaded.ensure_file_data_loaded_by_hash(hash).unwrap();
            let file = reloaded.get_file_by_hash(hash).unwrap();
            let expected_address = if in_place { old_extent.address } else { source_length };
            assert_eq!(file.data_block_address(), expected_address, "{env} {name}");
            assert_eq!(file.unpack().unwrap(), payload, "{env} {name}");
            assert_eq!(file.parse_data_block_header().unwrap().unwrap().timestamp(), Some(EDIT_FILETIME), "{env} {name}");
            fs::remove_file(&edited).unwrap();

            let changed: u64 = runs.iter().map(|(start, end)| end - start).sum();
            println!(
                "{env}\t{name}\tv{}\t{} entries\tno-op identical\treplaced {hash:016X} ({} -> {new_length} bytes)\t{}\t{} runs, {changed} bytes changed\tsize {source_length} -> {edited_length}",
                package.version(),
                package.file_count(),
                old_extent.length,
                if in_place { "in place" } else { "moved to the end" },
                runs.len(),
            );
            checked += 1;
        }
        fs::remove_dir_all(out).unwrap();
    }
    assert!(checked > 0, "set {} to a client directory", common::CLIENT_DIR_ENVS.join(" and/or "));
}

/// Every zlib entry of every package of the client directories re-compresses (`encode_payload`, classic zlib level
/// 6) to its stored bytes.
#[test]
#[ignore = "needs a real client: set UO_EC_CLIENT_DIR and/or UO_CC_CLIENT_DIR and run with --ignored"]
fn client_zlib_entries_recompress_byte_for_byte() {
    let mut checked = 0usize;
    for env in common::CLIENT_DIR_ENVS {
        let Some(dir) = std::env::var_os(env) else { continue };
        for path in client_packages(Path::new(&dir)) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let package = UopPackage::load(&path).unwrap();
            let files: Vec<&UopFile> = package
                .iter_files()
                .filter(|file| !file.is_empty_slot() && file.compression() == CompressionFlag::Zlib)
                .collect();
            let differing: Vec<u64> = files
                .par_iter()
                .filter_map(|file| {
                    let recompressed = UopFile::new()
                        .create_file_from_bytes(&file.unpack().unwrap(), file.filename_hash(), CompressionFlag::Zlib)
                        .unwrap();
                    (recompressed.data() != file.data()).then_some(file.filename_hash())
                })
                .collect();
            println!("{env}\t{name}\t{} zlib entries\t{} differ", files.len(), differing.len());
            assert!(differing.is_empty(), "{env} {name}: entries {differing:016X?} re-compress to other bytes");
            checked += files.len();
        }
    }
    assert!(checked > 0, "no zlib entries: set {} to a client directory", common::CLIENT_DIR_ENVS.join(" and/or "));
}
