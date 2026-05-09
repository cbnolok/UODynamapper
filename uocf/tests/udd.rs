use uocf::udd::*;
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
