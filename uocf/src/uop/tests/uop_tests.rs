use crate::uop::file::CompressionFlag;
use crate::uop::hash::hash_file_name_single;
use crate::uop::package::{LoadMode, UopPackage};
use flate2::Compression;
use std::io::Write;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_uop_path(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("uocf_{test_name}_{timestamp}.uop"))
}

#[test]
fn package_roundtrip_preserves_raw_and_zlib_entries() {
    let path = temp_uop_path("uop_roundtrip");

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            b"metadata payload",
            "build/metadata/00000000.bin",
            CompressionFlag::Zlib,
        )
        .expect("add compressed entry");
    package
        .add_file_from_memory(
            b"raw texture bytes",
            "build/textures/00000000.dds",
            CompressionFlag::None,
        )
        .expect("add raw entry");
    package.finalize_and_save(&path).expect("save package");

    let loaded = UopPackage::load(&path).expect("load package");
    let metadata_hash = hash_file_name_single("build/metadata/00000000.bin");
    let texture_hash = hash_file_name_single("build/textures/00000000.dds");

    assert_eq!(loaded.file_count(), 2);
    assert_eq!(loaded.iter_files().count(), 2);
    assert_eq!(loaded.files_by_hash().len(), 2);
    assert_eq!(
        loaded
            .get_file_by_hash(metadata_hash)
            .expect("metadata entry")
            .unpack()
            .expect("metadata payload"),
        b"metadata payload"
    );
    assert_eq!(
        loaded
            .get_file_by_hash(texture_hash)
            .expect("texture entry")
            .unpack()
            .expect("texture payload"),
        b"raw texture bytes"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn package_roundtrip_supports_multiple_blocks() {
    let path = temp_uop_path("uop_multiblock");

    let mut package = UopPackage::new(5, 1).expect("small block package");
    package
        .add_file_from_memory(b"first", "build/block/00000000.bin", CompressionFlag::None)
        .expect("add first entry");
    package
        .add_file_from_memory(b"second", "build/block/00000001.bin", CompressionFlag::Zlib)
        .expect("add second entry");
    package.finalize_and_save(&path).expect("save package");

    let loaded = UopPackage::load(&path).expect("load package");
    let first_hash = hash_file_name_single("build/block/00000000.bin");
    let second_hash = hash_file_name_single("build/block/00000001.bin");

    assert_eq!(loaded.blocks().len(), 2);
    assert_eq!(loaded.file_count(), 2);
    assert_eq!(
        loaded
            .get_file_by_hash(first_hash)
            .expect("first file")
            .unpack()
            .expect("first payload"),
        b"first"
    );
    assert_eq!(
        loaded
            .get_file_by_hash(second_hash)
            .expect("second file")
            .unpack()
            .expect("second payload"),
        b"second"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn package_load_mode_can_defer_payload_loading() {
    let path = temp_uop_path("uop_lazy_load");

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(b"lazy payload", "build/lazy.bin", CompressionFlag::Zlib)
        .expect("add lazy entry");
    package.finalize_and_save(&path).expect("save package");

    let file_hash = hash_file_name_single("build/lazy.bin");
    let mut loaded = UopPackage::load_with_mode(&path, LoadMode::Lazy).expect("load package lazily");

    assert_eq!(loaded.load_mode(), LoadMode::Lazy);
    assert_eq!(loaded.files_by_hash().len(), 1);
    assert!(loaded
        .get_file_by_hash(file_hash)
        .expect("lazy file metadata")
        .data()
        .is_none());

    loaded
        .ensure_file_data_loaded_by_hash(file_hash)
        .expect("materialize payload");
    assert_eq!(
        loaded
            .get_file_by_hash(file_hash)
            .expect("lazy file after load")
            .unpack()
            .expect("lazy payload"),
        b"lazy payload"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn package_recompress_preserves_mythic_and_zlib_bwt_entries() {
    let path = temp_uop_path("uop_recompress_codecs");

    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(
            b"mythic payload",
            "build/codecs/00000000.bin",
            CompressionFlag::Mythic,
        )
        .expect("add mythic entry");
    package
        .add_file_from_memory(
            b"zlib bwt payload",
            "build/codecs/00000001.bin",
            CompressionFlag::ZlibBwt,
        )
        .expect("add zlib+bwt entry");

    package
        .recompress(Compression::default())
        .expect("recompress package");
    package.finalize_and_save(&path).expect("save package");

    let loaded = UopPackage::load(&path).expect("load package");
    let mythic_hash = hash_file_name_single("build/codecs/00000000.bin");
    let zlib_bwt_hash = hash_file_name_single("build/codecs/00000001.bin");

    assert_eq!(
        loaded
            .get_file_by_hash(mythic_hash)
            .expect("mythic entry")
            .compression(),
        CompressionFlag::Mythic
    );
    assert_eq!(
        loaded
            .get_file_by_hash(mythic_hash)
            .expect("mythic payload")
            .unpack()
            .expect("decode mythic payload"),
        b"mythic payload"
    );
    assert_eq!(
        loaded
            .get_file_by_hash(zlib_bwt_hash)
            .expect("zlib+bwt entry")
            .compression(),
        CompressionFlag::ZlibBwt
    );
    assert_eq!(
        loaded
            .get_file_by_hash(zlib_bwt_hash)
            .expect("zlib+bwt payload")
            .unpack()
            .expect("decode zlib+bwt payload"),
        b"zlib bwt payload"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn get_file_by_hash_returns_none_for_missing_entry() {
    let mut package = UopPackage::new_default();
    package
        .add_file_from_memory(b"known", "build/known.bin", CompressionFlag::None)
        .expect("add known entry");

    assert!(package
        .get_file_by_hash(hash_file_name_single("build/missing.bin"))
        .is_none());
}

#[test]
fn load_rejects_invalid_magic() {
    let path = temp_uop_path("uop_invalid_magic");
    let mut file = fs::File::create(&path).expect("create temp file");
    file.write_all(b"NOPE").expect("write bad magic");
    file.flush().expect("flush bad file");

    let error = UopPackage::load(&path)
        .err()
        .expect("invalid magic must fail");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

    let _ = fs::remove_file(path);
}
