use udd_container::*;
use xxhash_rust::xxh64::xxh64;

#[test]
fn codec_bits_roundtrip_supports_only_none_and_zstd() {
    let none_bits = CodecBits::new(UddpCompression::None).expect("encode none");
    let zstd_bits = CodecBits::new(UddpCompression::Zstd).expect("encode zstd");

    assert_eq!(none_bits.raw(), 0);
    assert_eq!(zstd_bits.raw(), 1);
    assert_eq!(none_bits.compression().expect("decode none"), UddpCompression::None);
    assert_eq!(zstd_bits.compression().expect("decode zstd"), UddpCompression::Zstd);
}

#[test]
fn codec_bits_reject_unknown_compression_ids() {
    assert!(CodecBits::from_raw(2).compression().is_err());
    assert!(CodecBits::from_raw(3).compression().is_err());
    assert!(CodecBits::from_raw(0xFFFF).compression().is_err());
}

#[test]
fn uddf_roundtrip_preserves_raw_payload_without_compression() {
    let payload = b"raw wrapper payload";
    let uddf = UddfFile::wrap_bytes(payload, 0x5544_5432, 1, UddpCompression::None, 0)
        .expect("wrap raw uddf");

    let mut writer = std::io::Cursor::new(Vec::new());
    uddf.save_to_writer(&mut writer).expect("save raw uddf");

    let mut reader = std::io::Cursor::new(writer.into_inner());
    let loaded = UddfFile::load_from_reader(&mut reader).expect("load raw uddf");

    assert_eq!(loaded.unpack().expect("unpack raw uddf"), payload);
    assert_eq!(loaded.codec_bits().compression().expect("decode raw codec"), UddpCompression::None);
}

#[test]
fn path_hash_package_roundtrip_reads_entries() {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some("metadata/demo.bin"),
            path_hash64: None,
            id: None,
            data: b"metadata payload",
        })
        .expect("add metadata file");
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: Some("pages/0000.rgba"),
            path_hash64: None,
            id: None,
            data: b"raw texture bytes",
        })
        .expect("add texture file");

    let bytes = builder.build().expect("build package");
    let reader = UddpReader::open(bytes).expect("open package");

    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path("metadata/demo.bin"))
            .expect("read metadata"),
        b"metadata payload"
    );
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path("pages/0000.rgba"))
            .expect("read texture"),
        b"raw texture bytes"
    );
    assert_eq!(reader.stored_package_hash64(), reader.computed_package_hash64());
}

#[test]
fn consuming_builder_matches_reusable_builder_output() {
    let mut reusable_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    reusable_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some("metadata/demo.bin"),
            path_hash64: None,
            id: None,
            data: b"metadata payload metadata payload metadata payload",
        })
        .expect("add compressed metadata");
    reusable_builder
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::None,
            width: 0,
            height: 0,
            virtual_path: Some("pages/0000.rgba"),
            path_hash64: None,
            id: None,
            data: b"raw texture bytes",
        })
        .expect("add raw texture");

    let mut consuming_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    consuming_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some("metadata/demo.bin"),
            path_hash64: None,
            id: None,
            data: b"metadata payload metadata payload metadata payload",
        })
        .expect("add compressed metadata");
    consuming_builder
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::None,
            width: 0,
            height: 0,
            virtual_path: Some("pages/0000.rgba"),
            path_hash64: None,
            id: None,
            data: b"raw texture bytes",
        })
        .expect("add raw texture");

    let reusable_bytes = reusable_builder.build().expect("build reusable package");
    let consuming_bytes = consuming_builder.into_bytes().expect("build consuming package");
    assert_eq!(consuming_bytes, reusable_bytes);

    let reader = UddpReader::open(consuming_bytes).expect("open consuming package");
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path("metadata/demo.bin"))
            .expect("read metadata"),
        b"metadata payload metadata payload metadata payload"
    );
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path("pages/0000.rgba"))
            .expect("read texture"),
        b"raw texture bytes"
    );
}

#[test]
fn path_hash_cow_read_borrows_uncompressed_entries() {
    let path = "pages/raw.rgba";
    let payload = b"raw texture bytes";
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::None,
            width: 0,
            height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data: payload,
        })
        .expect("add raw texture file");

    let reader = UddpReader::open(builder.build().expect("build package")).expect("open package");
    let bytes = reader
        .read_file_by_path_hash_cow(xxh64_virtual_path(path))
        .expect("read raw texture through cow api");

    match bytes {
        std::borrow::Cow::Borrowed(bytes) => assert_eq!(bytes, payload),
        std::borrow::Cow::Owned(_) => panic!("raw entry should borrow package bytes"),
    }
}

#[test]
fn path_hash_package_roundtrip_reads_explicit_level_zstd_entry() {
    let payload = b"explicit-level zstd payload explicit-level zstd payload";
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDictLevel(5),
            width: 0,
            height: 0,
            virtual_path: Some("data/repeated.bin"),
            path_hash64: None,
            id: None,
            data: payload,
        })
        .expect("add explicit-level zstd file");

    let reader = UddpReader::open(builder.build().expect("build package")).expect("open package");
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path("data/repeated.bin"))
            .expect("read explicit-level zstd file"),
        payload
    );
}

#[test]
fn path_hash_cow_read_owns_compressed_entries() {
    let path = "data/repeated-cow.bin";
    let payload = b"explicit-level zstd payload explicit-level zstd payload";
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDictLevel(5),
            width: 0,
            height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data: payload,
        })
        .expect("add explicit-level zstd file");

    let reader = UddpReader::open(builder.build().expect("build package")).expect("open package");
    let bytes = reader
        .read_file_by_path_hash_cow(xxh64_virtual_path(path))
        .expect("read compressed payload through cow api");

    match bytes {
        std::borrow::Cow::Owned(bytes) => assert_eq!(bytes, payload),
        std::borrow::Cow::Borrowed(_) => panic!("compressed entry should decode into owned bytes"),
    }
}

#[test]
fn decoded_entry_cache_reads_repeated_compressed_entry() {
    let path = "data/cacheable.bin";
    let payload = b"cacheable zstd payload cacheable zstd payload cacheable zstd payload";
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data: payload,
        })
        .expect("add cacheable zstd file");

    let reader = UddpReader::open_with_options(
        builder.build().expect("build package"),
        UddpReaderOptions::enable_decoded_entry_cache(),
    )
    .expect("open package with decoded entry cache");

    assert!(reader.decoded_entry_cache_enabled());
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path(path))
            .expect("read cacheable file"),
        payload
    );
    assert_eq!(
        reader
            .read_file_by_path_hash(xxh64_virtual_path(path))
            .expect("read cacheable file again"),
        payload
    );
}

/// Where a path-hash index entry keeps its locator's `meta32`: after the u64 path hash and the u32 raw size.
const PATH_ENTRY_META32_OFFSET: usize = 8 + 4;
/// The 2-bit codec field of `meta32`.
const META32_CODEC_SHIFT: u32 = 6;
const META32_CODEC_MASK: u32 = 0x03;

/// The codec id no current codec uses, which the 2-bit codec field can still hold.
const UNKNOWN_CODEC_ID: u32 = 3;

/// A package whose one entry names [`UNKNOWN_CODEC_ID`].
fn package_with_unknown_codec() -> Vec<u8> {
    let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some("pages/0000.rgba8888"),
            path_hash64: None,
            id: None,
            data: &[0x11; 4096],
        })
        .expect("add page");
    let mut bytes = builder.build().expect("build package");
    let index_offset = UddpReader::open(bytes.clone()).expect("open package").header().index_offset as usize;
    let meta32_at = index_offset + PATH_ENTRY_META32_OFFSET;
    let meta32 = u32::from_le_bytes(bytes[meta32_at..meta32_at + 4].try_into().expect("meta32 bytes"));
    let meta32 = (meta32 & !(META32_CODEC_MASK << META32_CODEC_SHIFT)) | (UNKNOWN_CODEC_ID << META32_CODEC_SHIFT);
    bytes[meta32_at..meta32_at + 4].copy_from_slice(&meta32.to_le_bytes());
    bytes
}

#[test]
fn a_package_with_an_unknown_codec_fails_to_open() {
    let error = UddpReader::open(package_with_unknown_codec()).err().expect("the unknown codec is refused");
    assert!(
        matches!(error, udd_container::uddp::FormatError::InvalidCodec(codec) if u32::from(codec) == UNKNOWN_CODEC_ID),
        "{error:?}"
    );
}

#[test]
fn dense_id_package_roundtrip_reads_entries() {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Sector as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(0),
            data: b"sector-0",
        })
        .expect("add id 0");
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Sector as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(1),
            data: b"sector-1-compressed",
        })
        .expect("add id 1");

    let reader = UddpReader::open(builder.build().expect("build dense package")).expect("open dense package");
    assert_eq!(reader.read_file_by_dense_id(0).expect("read id 0"), b"sector-0");
    assert_eq!(
        reader.read_file_by_dense_id(1).expect("read id 1"),
        b"sector-1-compressed"
    );
}

/// Byte offset of `version_major` in the serialized package header (after the magic).
const HEADER_VERSION_MAJOR_OFFSET: usize = 4;

#[test]
fn reader_rejects_other_package_versions() {
    let mut builder = UddpBuilder::new(LookupMode::DenseId);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Sector as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(0),
            data: b"sector-0",
        })
        .expect("add id 0");
    let mut bytes = builder.build().expect("build dense package");
    let other_major = UDDP_VERSION_MAJOR + 1;
    bytes[HEADER_VERSION_MAJOR_OFFSET..HEADER_VERSION_MAJOR_OFFSET + size_of::<u16>()]
        .copy_from_slice(&other_major.to_le_bytes());

    let message = UddpReader::open(bytes).err().expect("another version must be rejected").to_string();
    assert_eq!(
        message,
        format!("package version {other_major}.{UDDP_VERSION_MINOR} is not supported; repack with the current udd-cli")
    );
}

#[test]
fn sparse_id_package_roundtrip_reads_entries() {
    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Tile as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(3),
            data: b"tile-3",
        })
        .expect("add sparse id 3");
    builder
        .add_file(AddFileRequest {
            data_type: DataType::Tile as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: None,
            path_hash64: None,
            id: Some(9),
            data: b"tile-9",
        })
        .expect("add sparse id 9");

    let reader = UddpReader::open(builder.build().expect("build sparse package")).expect("open sparse package");
    assert_eq!(reader.read_file_by_sparse_id(3).expect("read id 3"), b"tile-3");
    assert_eq!(reader.read_file_by_sparse_id(9).expect("read id 9"), b"tile-9");
}

#[test]
fn uddpi_patch_replaces_target_file_and_recomputes_hash() {
    let path = "metadata/demo.bin";
    let old_payload = b"old payload";
    let new_payload = b"new payload";

    let mut base_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    base_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data: old_payload,
        })
        .expect("add base payload");
    base_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: Some("metadata/keep.bin"),
            path_hash64: None,
            id: None,
            data: b"keep me",
        })
        .expect("add untouched payload");
    let base_bytes = base_builder.build().expect("build base");

    let mut new_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    new_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(path),
            path_hash64: None,
            id: None,
            data: new_payload,
        })
        .expect("add new payload");
    new_builder
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::None,
            width: 0, height: 0,
            virtual_path: Some("metadata/keep.bin"),
            path_hash64: None,
            id: None,
            data: b"keep me",
        })
        .expect("add new untouched payload");
    let new_bytes = new_builder.build().expect("build new");

    let mut patch_builder = UddpiBuilder::new(
        LookupMode::VirtualPathHash,
        canonical_package_hash64(&base_bytes),
        canonical_package_hash64(&new_bytes),
    );
    patch_builder.add_path_replacement(
        xxh64_virtual_path(path),
        xxh64(old_payload, 0),
        xxh64(new_payload, 0),
        new_payload.to_vec(),
        DataType::Metadata as u8,
    );

    let patch_bytes = patch_builder.build().expect("build patch");
    let rebuilt = UddpiApplier::apply_patch(&base_bytes, &patch_bytes).expect("apply patch");
    let rebuilt_reader = UddpReader::open(rebuilt.clone()).expect("open rebuilt");

    assert_eq!(
        rebuilt_reader
            .read_file_by_path_hash(xxh64_virtual_path(path))
            .expect("read rebuilt payload"),
        new_payload
    );
    assert_eq!(canonical_package_hash64(&rebuilt), canonical_package_hash64(&new_bytes));
}

/// A patch keeps a payload of independent Zstd frames (strips) as strips, with the same strip size and Meta32 flag.
/// The patcher writes payloads again at the default level and checks the result's hash, so only a package written at
/// that level patches; this one is.
#[test]
fn uddpi_patch_keeps_strips_of_untouched_payloads() {
    /// The level a patch writes payloads at.
    const DEFAULT_ZSTD_LEVEL: i32 = 7;
    const STRIP_BYTES: u32 = 64 * 1024;
    const STRIPS: usize = 4;
    let replaced = "metadata/replaced.bin";
    let striped = "metadata/striped.bin";
    let striped_payload: Vec<u8> = (0..STRIPS * STRIP_BYTES as usize).map(|index| (index * 7 / 5 + index / 4096) as u8).collect();
    let package = |replaced_payload: &[u8]| {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, compression, data) in [
            (replaced, CompressionFlag::ZstdNoDict, replaced_payload),
            (striped, CompressionFlag::ZstdStripsLevel { level: DEFAULT_ZSTD_LEVEL, strip_bytes: STRIP_BYTES }, striped_payload.as_slice()),
        ] {
            builder
                .add_file(AddFileRequest {
                    data_type: DataType::Metadata as u8,
                    compression,
                    width: 0, height: 0,
                    virtual_path: Some(path),
                    path_hash64: None,
                    id: None,
                    data,
                })
                .expect("add payload");
        }
        builder.build().expect("build package")
    };
    let (old_payload, new_payload) = (b"old payload".as_slice(), b"new payload".as_slice());
    let base_bytes = package(old_payload);
    let new_bytes = package(new_payload);
    let mut patch_builder = UddpiBuilder::new(
        LookupMode::VirtualPathHash,
        canonical_package_hash64(&base_bytes),
        canonical_package_hash64(&new_bytes),
    );
    patch_builder.add_path_replacement(
        xxh64_virtual_path(replaced),
        xxh64(old_payload, 0),
        xxh64(new_payload, 0),
        new_payload.to_vec(),
        DataType::Metadata as u8,
    );
    let patch_bytes = patch_builder.build().expect("build patch");
    let rebuilt = UddpReader::open(UddpiApplier::apply_patch(&base_bytes, &patch_bytes).expect("apply patch")).expect("open rebuilt");

    assert_eq!(rebuilt.read_file_by_path_hash(xxh64_virtual_path(striped)).expect("read striped"), striped_payload);
    let record = rebuilt.records().into_iter().find(|record| record.key == FileKey::PathHash(xxh64_virtual_path(striped))).expect("striped record");
    assert!(unpack_independent_frames(record.locator.meta32));
    let file = rebuilt.find_by_path_hash(xxh64_virtual_path(striped)).expect("striped file");
    let mut stored = vec![0u8; file.stored_size as usize];
    rebuilt.read_entry(file.payload_offset, &mut stored).expect("read stored payload");
    let mut frames = 0;
    let mut rest = stored.as_slice();
    while !rest.is_empty() {
        rest = &rest[zstd::zstd_safe::find_frame_compressed_size(rest).expect("frame")..];
        frames += 1;
    }
    assert_eq!(frames, STRIPS);
}

#[test]
fn uddf_roundtrip_preserves_flags_and_payload() {
    let payload = b"single wrapped payload";
    let uddf = UddfFile::wrap_bytes(
        payload,
        0x5544_5431,
        12,
        UddpCompression::Zstd,
        0x1122_3344_5566_7788u64,
    )
    .expect("wrap uddf");

    let mut writer = std::io::Cursor::new(Vec::new());
    uddf.save_to_writer(&mut writer).expect("save uddf");

    let bytes = writer.into_inner();
    let mut reader = std::io::Cursor::new(bytes);
    let loaded = UddfFile::load_from_reader(&mut reader).expect("load uddf");

    assert_eq!(loaded.unpack().expect("unpack uddf"), payload);
    assert_eq!(loaded.flags(), 0x1122_3344_5566_7788u64);
    assert_eq!(loaded.payload_offset() % UDDP_DEFAULT_ALIGNMENT, 0);
    assert_eq!(loaded.content_id(), 12);
    assert_eq!(loaded.typed_content_id(), Some(UddpContentId::Unknown));
    assert_eq!(loaded.codec_bits().raw(), 1);
}

/// Payload large enough to take the builder's multithreaded Zstd path.
const STREAMING_TEST_LARGE_PAYLOAD_BYTES: usize = 12 * 1024 * 1024;
const STREAMING_TEST_SMALL_FILE_COUNT: usize = 64;
const STREAMING_TEST_ZSTD_LEVEL: i32 = 9;

fn streaming_test_dir(test_name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("udd-container-{test_name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// Mixed synthetic files: raw, Zstd at default and explicit level, a payload
/// Zstd cannot shrink, and one large enough for multithreaded Zstd.
fn streaming_test_files() -> Vec<AddOwnedFileRequest> {
    let mut files = Vec::new();
    for index in 0..STREAMING_TEST_SMALL_FILE_COUNT {
        let compression = match index % 3 {
            0 => CompressionFlag::None,
            1 => CompressionFlag::ZstdNoDict,
            _ => CompressionFlag::ZstdNoDictLevel(STREAMING_TEST_ZSTD_LEVEL),
        };
        files.push(AddOwnedFileRequest {
            data_type: DataType::Metadata as u8,
            compression,
            width: 0,
            height: 0,
            virtual_path: Some(format!("files/{index:04}.bin")),
            path_hash64: None,
            id: None,
            data: format!("payload {index} ").repeat(index + 1).into_bytes(),
        });
    }
    files.push(AddOwnedFileRequest {
        data_type: DataType::Texture as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some("pages/incompressible.bin".to_string()),
        path_hash64: None,
        id: None,
        data: vec![0x5A],
    });
    files.push(AddOwnedFileRequest {
        data_type: DataType::Texture as u8,
        compression: CompressionFlag::ZstdNoDict,
        width: 0,
        height: 0,
        virtual_path: Some("pages/large.bin".to_string()),
        path_hash64: None,
        id: None,
        data: (0..STREAMING_TEST_LARGE_PAYLOAD_BYTES).map(|index| (index % 251) as u8).collect(),
    });
    files
}

fn assert_canonical_package_hash(bytes: &[u8]) {
    const HEADER_PACKAGE_HASH_OFFSET: usize = 40;
    let stored = u64::from_le_bytes(
        bytes[HEADER_PACKAGE_HASH_OFFSET..HEADER_PACKAGE_HASH_OFFSET + 8]
            .try_into()
            .expect("hash bytes"),
    );
    assert_eq!(stored, canonical_package_hash64(bytes));
}

#[test]
fn builder_into_file_matches_into_bytes() {
    let dir = streaming_test_dir("into-file");
    let output = dir.join("package.uddp");

    let mut memory_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    let mut file_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    for file in streaming_test_files() {
        memory_builder.add_owned_file(file.clone()).expect("add memory file");
        file_builder.add_owned_file(file).expect("add file-backed file");
    }
    let expected = memory_builder.into_bytes().expect("build in memory");
    file_builder.into_file(&output).expect("write package file");

    let written = std::fs::read(&output).expect("read package file");
    assert_eq!(written, expected);
    assert_canonical_package_hash(&written);
    std::fs::remove_dir_all(&dir).expect("remove test dir");
}

#[test]
fn streaming_builder_matches_in_memory_builder() {
    let dir = streaming_test_dir("streaming");
    let output = dir.join("package.uddp");
    let files = streaming_test_files();

    let mut memory_builder = UddpBuilder::new(LookupMode::VirtualPathHash);
    for file in &files {
        memory_builder.add_owned_file(file.clone()).expect("add memory file");
    }
    let expected = memory_builder.build().expect("build in memory");

    let mut streaming_builder =
        UddpStreamingBuilder::create(LookupMode::VirtualPathHash, &output).expect("create streaming builder");
    let mut files = files.into_iter();
    let first = files.next().expect("first file");
    streaming_builder
        .add_file(AddFileRequest {
            data_type: first.data_type,
            compression: first.compression,
            width: first.width,
            height: first.height,
            virtual_path: first.virtual_path.as_deref(),
            path_hash64: None,
            id: None,
            data: &first.data,
        })
        .expect("add borrowed file");
    let second = files.next().expect("second file");
    streaming_builder.add_owned_file(second).expect("add owned file");
    streaming_builder
        .add_owned_files(files.collect())
        .expect("add owned file batch");

    let spill_path = dir.join("package.uddp.payloads.tmp");
    assert!(spill_path.exists());
    let mut assembled_steps = Vec::new();
    streaming_builder
        .finish_with_progress(|progress| {
            assert_eq!(progress.phase, BuildProgressPhase::Assembling);
            assembled_steps.push(progress.completed);
        })
        .expect("finish streaming package");
    assert!(!spill_path.exists());
    assert_eq!(assembled_steps.first(), Some(&0));
    assert_eq!(
        assembled_steps.last().copied(),
        Some(2 + STREAMING_TEST_SMALL_FILE_COUNT + 2)
    );

    let written = std::fs::read(&output).expect("read streamed package");
    assert_eq!(written, expected);
    assert_canonical_package_hash(&written);

    let reader = UddpReader::open(written).expect("open streamed package");
    for file in streaming_test_files() {
        let path = file.virtual_path.as_deref().expect("test files have paths");
        assert_eq!(
            reader
                .read_file_by_path_hash(xxh64_virtual_path(path))
                .expect("read streamed file"),
            file.data,
            "{path}"
        );
    }
    std::fs::remove_dir_all(&dir).expect("remove test dir");
}

#[test]
fn streaming_builder_assigns_dense_ids_like_in_memory_builder() {
    let dir = streaming_test_dir("streaming-dense");
    let output = dir.join("package.uddp");
    let payloads = [b"sector-0".as_slice(), b"sector-1-sector-1-sector-1", b"sector-2"];

    let mut memory_builder = UddpBuilder::new(LookupMode::DenseId);
    let mut streaming_builder =
        UddpStreamingBuilder::create(LookupMode::DenseId, &output).expect("create streaming builder");
    for payload in payloads {
        let request = AddFileRequest {
            data_type: DataType::Sector as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: None,
            path_hash64: None,
            id: None,
            data: payload,
        };
        memory_builder.add_file(request.clone()).expect("add memory file");
        streaming_builder.add_file(request).expect("add streamed file");
    }
    streaming_builder.finish().expect("finish streaming package");

    assert_eq!(
        std::fs::read(&output).expect("read streamed package"),
        memory_builder.build().expect("build in memory")
    );
    std::fs::remove_dir_all(&dir).expect("remove test dir");
}

#[test]
fn streaming_builder_rejects_dictionary_compression_and_drops_spill() {
    let dir = streaming_test_dir("streaming-dict");
    let output = dir.join("package.uddp");
    let spill_path = dir.join("package.uddp.payloads.tmp");
    let mut streaming_builder =
        UddpStreamingBuilder::create(LookupMode::VirtualPathHash, &output).expect("create streaming builder");
    for compression in [CompressionFlag::ZstdDict, CompressionFlag::Auto] {
        let result = streaming_builder.add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression,
            width: 0,
            height: 0,
            virtual_path: Some("metadata/dict.bin"),
            path_hash64: None,
            id: None,
            data: b"dictionary candidate",
        });
        assert!(matches!(
            result,
            Err(udd_container::uddp::BuildError::DictionaryCompressionWhileStreaming(rejected)) if rejected == compression
        ));
    }
    assert!(spill_path.exists());
    drop(streaming_builder);
    assert!(!spill_path.exists());
    assert!(!output.exists());
    std::fs::remove_dir_all(&dir).expect("remove test dir");
}

/// A package added uncompressed to another one is read in place, the other's entries stay as they were stored
/// (dictionary-compressed ones too), and a compressed entry is refused as a section.
#[test]
fn an_extended_package_keeps_its_entries_and_opens_the_added_one_in_place() {
    const SAMPLES: usize = 64;
    const SAMPLE_LINES: usize = 200;
    const NESTED_PATH: &str = "nested/package.uddp";
    let path_request = |path: String, compression, data: Vec<u8>| AddOwnedFileRequest {
        data_type: DataType::Metadata as u8,
        compression,
        width: 0,
        height: 0,
        virtual_path: Some(path),
        path_hash64: None,
        id: None,
        data,
    };
    let mut nested = UddpBuilder::new(LookupMode::VirtualPathHash);
    nested.add_owned_file(path_request("inner.bin".into(), CompressionFlag::ZstdNoDict, b"inner payload".to_vec())).unwrap();
    let nested = nested.into_bytes().unwrap();

    let mut base = UddpBuilder::new(LookupMode::VirtualPathHash);
    for index in 0..SAMPLES {
        // Enough similar samples (16 or more, 128 KiB in all) for the builder to train a dictionary.
        let data: String = (0..SAMPLE_LINES).map(|line| format!("record {index}:{line} value {}\n", line * index % 97)).collect();
        let data = data.into_bytes();
        base.add_owned_file(path_request(format!("samples/{index}.bin"), CompressionFlag::ZstdDict, data)).unwrap();
    }
    base.add_owned_file(path_request("compressed.bin".into(), CompressionFlag::ZstdNoDict, nested.clone())).unwrap();
    let base = UddpReader::open(base.into_bytes().unwrap()).unwrap();

    let extended = UddpBuilder::extend_package(
        &base,
        vec![(xxh64_virtual_path(NESTED_PATH), DataType::Metadata as u8, nested.clone())],
    )
    .unwrap();
    let extended = UddpReader::open(extended).unwrap();
    assert_eq!(extended.computed_package_hash64(), extended.stored_package_hash64());
    assert_eq!(extended.dictionary_records(), base.dictionary_records());
    for record in base.records() {
        let FileKey::PathHash(hash) = record.key else { panic!("path package") };
        assert_eq!(extended.read_file_by_path_hash(hash).unwrap(), base.read_file_by_path_hash(hash).unwrap());
        let (was, is) = (base.find_by_path_hash(hash).unwrap(), extended.find_by_path_hash(hash).unwrap());
        assert_eq!((is.codec, is.stored_size), (was.codec, was.stored_size), "stored as it was");
    }

    let section = extended.open_section(xxh64_virtual_path(NESTED_PATH), UddpReaderOptions::disabled()).unwrap();
    assert_eq!(section.read_file_by_path_hash(xxh64_virtual_path("inner.bin")).unwrap(), b"inner payload");
    assert!(matches!(
        extended.open_section(xxh64_virtual_path("compressed.bin"), UddpReaderOptions::disabled()),
        Err(udd_container::uddp::FormatError::CompressedSection)
    ));
    assert!(matches!(
        extended.open_section(xxh64_virtual_path("absent.bin"), UddpReaderOptions::disabled()),
        Err(udd_container::uddp::FormatError::FileNotFound)
    ));
}
