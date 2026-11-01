//! The on-disk layout of a UDDP package and reads through the memory-mapped reader.
//!
//! Layout contract checked here: the header, the dictionary table and the index come first, back to back; the
//! blob region starts right after them and holds the dictionaries (by data type) and then every payload in key
//! order, with no padding, to the end of the file. Payloads are therefore not aligned: a reader that needs
//! aligned records must copy them (`read_file_by_*` returns owned bytes, `*_cow` may borrow at any offset).

use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;

use udd_container::*;

/// Files in the dictionary package: enough samples of one type for the builder to train a dictionary.
const DICT_FILE_COUNT: u32 = 256;
/// Bytes of each dictionary-package file.
const DICT_FILE_BYTES: usize = 1024;
/// Distinct record shapes the dictionary-package files are built from.
const DICT_RECORD_SHAPES: u32 = 7;
/// Serialized sizes of the head tables, from the format reference (`uddp/mod.rs`).
const HEADER_BYTES: u64 = 48;
const DICT_REF_BYTES: u64 = 20;
const DENSE_LOCATOR_BYTES: u64 = 16;
const PATH_ENTRY_BYTES: u64 = 24;
/// Odd payload sizes, so that later payloads start at unaligned offsets.
const ODD_PAYLOAD_SIZES: [usize; 5] = [1, 3, 7, 13, 29];

fn scratch_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("uddp_layout_{name}_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join("package.uddp")
}

/// Structured, repetitive bytes for file `id`, so a trained dictionary helps.
fn structured_payload(id: u32) -> Vec<u8> {
    let shape = id % DICT_RECORD_SHAPES;
    (0..DICT_FILE_BYTES)
        .map(|index| {
            let index = index as u32;
            ((index % (shape + 2)) * 17 + shape * 31 + (index / 64) * 3) as u8
        })
        .collect()
}

fn add_dense(builder: &mut UddpBuilder, id: u32, compression: CompressionFlag, data: &[u8]) {
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Sector as u8,
            compression,
            width: 0,
            height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(id),
            data,
        })
        .expect("add file");
}

/// (offset, stored size) of every payload, in index order.
fn payload_spans(reader: &UddpReader) -> Vec<(u64, u64)> {
    reader
        .records()
        .iter()
        .map(|record| {
            let locator = &record.locator;
            (
                unpack_offset40(locator.pos64),
                u64::from(reconstruct_stored_size(locator.raw_size, locator.meta32, locator.pos64)),
            )
        })
        .collect()
}

#[test]
fn the_blob_follows_the_head_and_holds_dictionaries_then_payloads_back_to_back() {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    let payloads: Vec<Vec<u8>> = (0..DICT_FILE_COUNT).map(structured_payload).collect();
    for (id, payload) in payloads.iter().enumerate() {
        add_dense(&mut builder, id as u32, CompressionFlag::ZstdDict, payload);
    }
    let bytes = builder.build().expect("build package");
    let reader = UddpReader::open(bytes.clone()).expect("open package");
    let header = *reader.header();

    let dictionaries = reader.dictionary_records();
    assert!(!dictionaries.is_empty(), "the builder trained no dictionary; the layout check needs one");
    assert_eq!(header.dict_table_offset, HEADER_BYTES);
    assert_eq!(header.index_offset, HEADER_BYTES + u64::from(header.dict_count) * DICT_REF_BYTES);
    assert_eq!(header.blob_offset, header.index_offset + u64::from(header.file_count) * DENSE_LOCATOR_BYTES);

    // The dictionaries open the blob, by data type.
    let dictionary_bytes: u64 = dictionaries.iter().map(|(_, _, size)| u64::from(*size)).sum();
    let mut types: Vec<u8> = dictionaries.iter().map(|(data_type, _, _)| *data_type).collect();
    let sorted = {
        let mut sorted = types.clone();
        sorted.sort_unstable();
        sorted
    };
    assert_eq!(types, sorted, "dictionaries are stored by data type");
    types.dedup();
    assert_eq!(types.len(), dictionaries.len(), "one dictionary per data type");

    // Then the payloads, in key order, with no gap and no padding, to the end of the file.
    let mut cursor = header.blob_offset + dictionary_bytes;
    for (id, (offset, stored)) in payload_spans(&reader).into_iter().enumerate() {
        assert_eq!(offset, cursor, "payload {id} starts right after the previous one");
        cursor += stored;
    }
    assert_eq!(cursor, bytes.len() as u64, "the last payload ends the file");
    assert!(
        reader.records().iter().all(|record| unpack_codec(record.locator.meta32) == Codec::ZstdTypeDict),
        "every forced-dictionary payload uses the dictionary"
    );
    for (id, payload) in payloads.iter().enumerate() {
        assert_eq!(&reader.read_file_by_dense_id(id as u32).expect("read payload"), payload);
    }
}

#[test]
fn uncompressed_payloads_are_not_padded_and_borrow_at_any_offset() {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    let payloads: Vec<(String, Vec<u8>)> = ODD_PAYLOAD_SIZES
        .iter()
        .enumerate()
        .map(|(index, &size)| (format!("odd/{index}.bin"), (0..size).map(|byte| (byte + index) as u8).collect()))
        .collect();
    for (path, data) in &payloads {
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            })
            .expect("add file");
    }
    let path = scratch_path("odd");
    builder.into_file(&path).expect("write package");
    let file_len = fs::metadata(&path).expect("package metadata").len();

    let reader = UddpReader::load(&path).expect("map package");
    let header = *reader.header();
    assert_eq!(header.blob_offset, HEADER_BYTES + u64::from(header.file_count) * PATH_ENTRY_BYTES);
    let total: u64 = ODD_PAYLOAD_SIZES.iter().map(|&size| size as u64).sum();
    assert_eq!(header.blob_offset + total, file_len, "uncompressed payloads carry no padding");
    let mut offsets: Vec<u64> = payload_spans(&reader).into_iter().map(|(offset, _)| offset).collect();
    offsets.sort_unstable();
    assert!(offsets.iter().any(|offset| offset % 2 == 1), "some payload starts at an odd offset: {offsets:?}");

    for (virtual_path, data) in &payloads {
        let read = reader.read_file_by_path_hash_cow(xxh64_virtual_path(virtual_path)).expect("read payload");
        assert!(matches!(read, Cow::Borrowed(_)), "{virtual_path}: an uncompressed payload borrows the mapping");
        assert_eq!(read.as_ref(), data.as_slice(), "{virtual_path}");
    }
    fs::remove_dir_all(path.parent().expect("scratch dir")).ok();
}

#[test]
fn the_mapped_reader_reads_what_the_in_memory_reader_reads() {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    let payloads: Vec<Vec<u8>> = (0..DICT_FILE_COUNT).map(structured_payload).collect();
    let compressions = [CompressionFlag::None, CompressionFlag::ZstdNoDict, CompressionFlag::ZstdDict, CompressionFlag::Auto];
    for (id, payload) in payloads.iter().enumerate() {
        add_dense(&mut builder, id as u32, compressions[id % compressions.len()], payload);
    }
    let bytes = builder.build().expect("build package");
    let path = scratch_path("mapped");
    fs::write(&path, &bytes).expect("write package");

    let mapped = UddpReader::load(&path).expect("map package");
    let in_memory = UddpReader::open(bytes).expect("open package");
    assert_eq!(mapped.package_size_bytes(), in_memory.package_size_bytes());
    assert_eq!(mapped.stored_package_hash64(), mapped.computed_package_hash64());
    assert_eq!(mapped.computed_package_hash64(), in_memory.computed_package_hash64());
    assert_eq!(payload_spans(&mapped), payload_spans(&in_memory));
    for (id, payload) in payloads.iter().enumerate() {
        let id = id as u32;
        let from_map = mapped.read_file_by_dense_id_cow(id).expect("read mapped");
        let from_memory = in_memory.read_file_by_dense_id_cow(id).expect("read in memory");
        assert_eq!(from_map.as_ref(), payload.as_slice(), "entry {id}");
        assert_eq!(from_map.as_ref(), from_memory.as_ref(), "entry {id}");
        assert_eq!(matches!(from_map, Cow::Borrowed(_)), matches!(from_memory, Cow::Borrowed(_)), "entry {id}");
    }
    fs::remove_dir_all(path.parent().expect("scratch dir")).ok();
}

#[test]
fn a_truncated_mapped_package_fails_to_open_or_to_read_but_never_panics() {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    let payloads: Vec<Vec<u8>> = (0..ODD_PAYLOAD_SIZES.len() as u32).map(structured_payload).collect();
    for (id, payload) in payloads.iter().enumerate() {
        add_dense(&mut builder, id as u32, CompressionFlag::None, payload);
    }
    let bytes = builder.build().expect("build package");
    let path = scratch_path("truncated");

    for len in 0..bytes.len() {
        fs::write(&path, &bytes[..len]).expect("write truncated package");
        let Ok(reader) = UddpReader::load(&path) else { continue };
        let all_read = (0..payloads.len() as u32).all(|id| reader.read_file_by_dense_id(id).is_ok());
        assert!(!all_read, "a package cut at {len} of {} bytes read every entry", bytes.len());
    }
    fs::remove_dir_all(path.parent().expect("scratch dir")).ok();
}
