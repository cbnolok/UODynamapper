//! The BC7 field split codecs (4 and 5) through the builder and the reader, and the codec bits a reader refuses.

use udd_container::bc7_fields::{self, BLOCK_BYTES};
use udd_container::*;

/// Distinct blocks of the test page, repeated to fill it so that both entropy coders shrink it.
const DISTINCT_BLOCKS: usize = 64;
const PAGE_BLOCKS: usize = 4096;
const SPLIT_ZSTD_PATH: &str = "pages/fields_zstd.bin";
const SPLIT_LZ4_PATH: &str = "pages/fields_lz4.bin";
const PLAIN_PATH: &str = "pages/plain.bin";
const TEST_ZSTD_LEVEL: i32 = 3;
/// The first reserved bit of a locator's metadata word.
const META32_FIRST_RESERVED_BIT: u32 = 18;

/// A page of BC7-sized blocks of every mode, with the reserved mode's blocks among them: any bytes round-trip.
fn page() -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
    };
    let distinct: Vec<u8> = (0..DISTINCT_BLOCKS * BLOCK_BYTES).map(|_| next()).collect();
    let mut page = Vec::with_capacity(PAGE_BLOCKS * BLOCK_BYTES);
    for block in 0..PAGE_BLOCKS {
        let at = (block % DISTINCT_BLOCKS) * BLOCK_BYTES;
        page.extend_from_slice(&distinct[at..at + BLOCK_BYTES]);
    }
    // One block of each mode (the lowest set bit of the first byte) and a reserved one (first byte 0).
    for mode in 0..=8u32 {
        page[mode as usize * BLOCK_BYTES] = 1u8.checked_shl(mode).unwrap_or(0);
    }
    page
}

fn package(page: &[u8]) -> Vec<u8> {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    for (path, compression) in [
        (SPLIT_ZSTD_PATH, CompressionFlag::Bc7FieldsZstdLevel(TEST_ZSTD_LEVEL)),
        (SPLIT_LZ4_PATH, CompressionFlag::Bc7FieldsLz4Hc),
        (PLAIN_PATH, CompressionFlag::ZstdNoDictLevel(TEST_ZSTD_LEVEL)),
    ] {
        builder
            .add_owned_file(AddOwnedFileRequest {
                data_type: DataType::Texture as u8,
                compression,
                width: 0,
                height: 0,
                virtual_path: Some(path.to_owned()),
                path_hash64: None,
                id: None,
                data: page.to_vec(),
            })
            .expect("add page");
    }
    builder.build().expect("build package")
}

fn codec_of(reader: &UddpReader, path: &str) -> Codec {
    let key = FileKey::PathHash(xxh64_virtual_path(path));
    let record = reader.records().into_iter().find(|record| record.key == key).expect("record");
    unpack_codec(record.locator.meta32)
}

#[test]
fn split_pages_read_back_whole_and_as_their_split() {
    let page = page();
    let reader = UddpReader::open(package(&page)).expect("open package");
    for (path, codec) in [(SPLIT_ZSTD_PATH, Codec::Bc7FieldsZstd), (SPLIT_LZ4_PATH, Codec::Bc7FieldsLz4Hc)] {
        assert_eq!(codec_of(&reader, path), codec, "{path}");
        let hash = xxh64_virtual_path(path);
        assert_eq!(reader.read_file_by_path_hash(hash).expect("read whole"), page, "{path}");
        let split = reader.read_bc7_fields_by_path_hash(hash).expect("read split").expect("a split entry");
        assert_eq!(split.raw_len, page.len(), "{path}");
        let mut joined = Vec::new();
        bc7_fields::inverse(&split.stream, split.raw_len, &mut joined).expect("join");
        assert_eq!(joined, page, "{path}");
    }
    assert_eq!(codec_of(&reader, PLAIN_PATH), Codec::ZstdNoDict);
    assert!(reader.read_bc7_fields_by_path_hash(xxh64_virtual_path(PLAIN_PATH)).expect("read plain").is_none());
}

#[test]
fn codec_ids_take_the_high_bit_and_reserved_bits_are_refused() {
    for codec in [Codec::None, Codec::ZstdNoDict, Codec::ZstdTypeDict, Codec::Lz4Hc, Codec::Bc7FieldsZstd,
        Codec::Bc7FieldsLz4Hc] {
        let meta32 = pack_meta32(DataType::Texture as u8, codec, u32::MAX, true);
        assert_eq!(try_unpack_codec(meta32).expect("a known codec"), codec);
        assert!(meta32 >> META32_FIRST_RESERVED_BIT == 0, "{codec:?} sets a reserved bit");
    }
    let bc7_fields_lz4 = pack_meta32(DataType::Texture as u8, Codec::Bc7FieldsLz4Hc, 0, false);
    // Codec 6 and 7 aren't defined.
    assert!(try_unpack_codec(bc7_fields_lz4 + (1 << 6)).is_err());
    assert!(try_unpack_codec(bc7_fields_lz4 + (2 << 6)).is_err());
    assert!(matches!(try_unpack_codec(bc7_fields_lz4 | 1 << META32_FIRST_RESERVED_BIT),
        Err(udd_container::uddp::FormatError::ReservedBits(_))));
}

#[test]
fn a_package_with_reserved_locator_bits_does_not_open() {
    let page = page();
    let mut bytes = package(&page);
    let reader = UddpReader::open(bytes.clone()).expect("open package");
    let key = FileKey::PathHash(xxh64_virtual_path(SPLIT_LZ4_PATH));
    let meta32 = reader.records().into_iter().find(|record| record.key == key).expect("record").locator.meta32;
    let word = meta32.to_le_bytes();
    let at: Vec<usize> = bytes.windows(word.len()).enumerate().filter(|(_, window)| *window == word).map(|(at, _)| at)
        .collect();
    assert_eq!(at.len(), 1, "the locator's metadata word must be found once to patch it");
    bytes[at[0]..at[0] + word.len()].copy_from_slice(&(meta32 | 1 << META32_FIRST_RESERVED_BIT).to_le_bytes());
    let error = UddpReader::open(bytes).err().expect("reserved bits must be refused").to_string();
    assert!(error.contains("reserved bits"), "{error}");
}
