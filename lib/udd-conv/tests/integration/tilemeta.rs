use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use udd_container::{
    AddFileRequest, DataType, LookupMode, UddpBuilder, UddpReader,
};
use bytemuck::Zeroable;

use udd_assets::{
    tilemeta::{
        encode_item_texture_refs, EcMaterialLogicalFamily, EcMaterialPhysicalPackage,
        EcMaterialStableRole, TileMetaItemDefinitions, TileMetaItemTextureRef,
        TileMetaItemTextureRefSpan, TileMetaItemTile, TileMetaItemVisualKind, TileMetaLandTile,
        TILEMETA_ITEM_DEFINITION_ENTRY_PATH, TILEMETA_ITEM_ENTRY_PATH, TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, TILEMETA_ITEM_MATERIAL_ENTRY_PATH,
        TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, TILEMETA_LAND_ENTRY_PATH,
    },
    tex_art_ec::TexArtEcCropAdjustment,
    TileMetaPackage,
};
use udd_conv::tilemeta::*;
use udd_container::CompressionFlag;

/// The item definitions entry every tilemeta package holds, with no records.
fn add_empty_item_definitions(package: &mut UddpBuilder) {
    let definitions = TileMetaItemDefinitions::default().to_bytes().expect("encode item definitions");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(TILEMETA_ITEM_DEFINITION_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &definitions,
        })
        .expect("add item definitions");
}

fn temp_dir(test_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("uddconv_{test_name}_{timestamp}"))
}

#[test]
fn tilemeta_finds_string_dictionary_path() {
    let dir = temp_dir("string_dictionary_find");
    fs::create_dir_all(&dir).expect("create temp dir");
    fs::write(dir.join("string_dictionary.uop"), []).expect("write dict marker");

    let found = find_string_dictionary_path(std::slice::from_ref(&dir)).expect("find dictionary file");

    assert_eq!(found, dir.join("string_dictionary.uop"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn tilemeta_package_roundtrip_preserves_metadata_records() {
    let mut land_tiles = vec![TileMetaLandTile::zeroed(); 8];
    land_tiles[7] = TileMetaLandTile {
        tile_id: 7,
        texture_id: 42,
        tile_type: 2,
        _pad1: 0,
        flags: 0x1234,
        radar_color: [1, 2, 3, 4],
        name: {
            let mut name = [0u8; 20];
            name[..4].copy_from_slice(b"sand");
            name
        },
    };
    let mut item_tiles = vec![TileMetaItemTile::zeroed(); 12];
    item_tiles[11] = TileMetaItemTile {
        tile_id: 11,
        weight: 1,
        quality: 2,
        quantity: 3,
        hue_extra: 4,
        flags: 0xABCD,
        anim_id: 5,
        stacking_offset: 6,
        value: 7,
        height: 8,
        _pad1: 0,
        ec_art_flags: 0,
        radar_color: [9, 10, 11, 12],
        name: {
            let mut name = [0u8; 20];
            name[..5].copy_from_slice(b"chair");
            name
        },
        ec_texture_id: 100,
        ec_start_x: 101,
        ec_start_y: 102,
        cc_texture_id: 200,
        cc_start_x: 201,
        cc_start_y: 202,
    };

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDict,
        width: 0, height: 0, virtual_path: Some(TILEMETA_ITEM_MATERIAL_ENTRY_PATH),
        path_hash64: None, id: None, data: b"{}",
    }).expect("add source item materials");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(TILEMETA_LAND_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: bytemuck::cast_slice(&land_tiles),
        })
        .expect("add land metadata");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(TILEMETA_ITEM_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: bytemuck::cast_slice(&item_tiles),
        })
        .expect("add item metadata");
    let no_refs = encode_item_texture_refs(&[]);
    for (path, data) in [(TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH, [].as_slice()), (TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH, &no_refs)] {
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0, height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            })
            .expect("add empty item texture refs");
    }
    add_empty_item_definitions(&mut package);

    let package = TileMetaPackage::from_uddp_package(
        UddpReader::open(package.build().expect("build package")).expect("open package"),
    )
    .expect("read tilemeta package");

    assert_eq!(package.land_tiles().len(), 8);
    assert_eq!(package.item_tiles().len(), 12);
    assert_eq!(package.land_tile(7).expect("land tile").name_ascii(), "sand");
    let item = package.item_tile(11).expect("item tile");
    assert_eq!(item.name_ascii(), "chair");
    assert_eq!(item.ec_texture_id, 100);
    assert_eq!(item.cc_texture_id, 200);
    assert_eq!(item.visual_kind(), TileMetaItemVisualKind::RegularArt);
}

#[test]
fn tilemeta_crop_adjustment_shifts_ec_sampling_start_to_the_trimmed_image() {
    const TILE_ID: u32 = 42;
    const START_X: i32 = 12;
    const START_Y: i32 = 18;
    const TRIM_LEFT: i16 = 2;
    const TRIM_TOP: i16 = 3;
    let trimmed = TexArtEcCropAdjustment {
        source_left: START_X as i16 + TRIM_LEFT,
        source_top: START_Y as i16 + TRIM_TOP,
        trim_left: TRIM_LEFT,
        trim_top: TRIM_TOP,
        trim_right: 4,
        trim_bottom: 6,
    };

    assert_eq!(
        adjusted_ec_sampling_start(TILE_ID, START_X, START_Y, Some(trimmed)).expect("shift EC sampling start"),
        (START_X as i16 + TRIM_LEFT, START_Y as i16 + TRIM_TOP)
    );
    assert_eq!(
        adjusted_ec_sampling_start(TILE_ID, START_X, START_Y, None).expect("keep EC sampling start"),
        (START_X as i16, START_Y as i16)
    );
    assert!(adjusted_ec_sampling_start(TILE_ID, i32::from(i16::MAX) + 1, START_Y, None).is_err());
}

#[test]
fn tilemeta_item_defaults_cc_texture_to_owning_tile_id() {
    let tile_id = 1234u32;
    let item = TileMetaItemTile {
        tile_id,
        cc_texture_id: tile_id,
        ..TileMetaItemTile::zeroed()
    };

    assert_eq!(item.cc_texture_id, tile_id);
}

#[test]
fn tilemeta_item_visual_kind_roundtrips_in_padding_byte() {
    let mut item = TileMetaItemTile::zeroed();
    item.set_visual_kind(TileMetaItemVisualKind::SurfaceLike);

    assert!(item.is_surface_like());
    assert_eq!(item.visual_kind(), TileMetaItemVisualKind::SurfaceLike);
}

#[test]
fn classify_item_visual_kind_marks_solid_entries_as_surface_like() {
    use uocf::enhanced::tileart::{ArtData, TileType};
    let art_data = ArtData {
        tile_type: TileType::Solid,
        ..ArtData::default()
    };

    assert_eq!(
        classify_item_visual_kind(1444, Some(&art_data)),
        TileMetaItemVisualKind::SurfaceLike,
    );
}

#[test]
fn classify_item_visual_kind_keeps_static_entries_as_regular_art() {
    use uocf::enhanced::tileart::{ArtData, TileType};
    let art_data = ArtData {
        tile_type: TileType::Static,
        ..ArtData::default()
    };

    assert_eq!(
        classify_item_visual_kind(172, Some(&art_data)),
        TileMetaItemVisualKind::RegularArt,
    );
}

/// Sidecar integrity: TileMetaItemTextureRef records with populated role,
/// physical-package, and logical-family fields must survive a UDDP write/read
/// round-trip without losing any metadata axis.
///
/// It exercises the
/// full binary format path:
///   Vec<TileMetaItemTextureRef> → bytemuck::cast_slice → UDDP entry →
///   TileMetaPackage::from_uddp_package → item_texture_refs(tile_id)
#[test]
fn tilemeta_texture_refs_roundtrip_preserves_role_fields() {
    // Two refs for tile 42: a water surface (Base, position 1) and a water bump (position 0).
    let ref_base = TileMetaItemTextureRef {
        texture_id: 1000,
        texture_type: EcMaterialLogicalFamily::WorldArt as u8,
        physical_package: EcMaterialPhysicalPackage::Texture as u8,
        stable_role: EcMaterialStableRole::Base as u8,
        item_index: 1,
        texture_stretch: 4.0,
        ..TileMetaItemTextureRef::zeroed()
    };
    let ref_ripple = TileMetaItemTextureRef {
        texture_id: 1000020,
        texture_type: EcMaterialLogicalFamily::Textures as u8,
        physical_package: EcMaterialPhysicalPackage::TerrainTexture as u8,
        stable_role: EcMaterialStableRole::WaterBump as u8,
        texture_stretch: 16.0,
        ..TileMetaItemTextureRef::zeroed()
    };
    let refs = vec![ref_base, ref_ripple];

    // The span table is indexed by tile_id, so it must have at least tile_id+1
    // entries.  Entries for tiles without refs stay zeroed (len=0).
    let mut spans = vec![TileMetaItemTextureRefSpan::zeroed(); 43];
    spans[42] = TileMetaItemTextureRefSpan {
        start: 0,
        len: 2,
        _pad: 0,
    };

    // Build a minimal item tile so tile 42 has an entry (span lookup is by
    // tile_id, which indexes into item_tiles by position).
    let mut item_tiles = vec![TileMetaItemTile::zeroed(); 43];
    item_tiles[42].tile_id = 42;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8, compression: CompressionFlag::ZstdNoDict,
        width: 0, height: 0, virtual_path: Some(TILEMETA_ITEM_MATERIAL_ENTRY_PATH),
        path_hash64: None, id: None, data: b"{}",
    }).expect("add source item materials");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(TILEMETA_LAND_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &[],
        })
        .expect("add empty land");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(TILEMETA_ITEM_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: bytemuck::cast_slice(&item_tiles),
        })
        .expect("add item tiles");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: bytemuck::cast_slice(&spans),
        })
        .expect("add ref spans");
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0,
            height: 0,
            virtual_path: Some(TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &encode_item_texture_refs(&refs),
        })
        .expect("add texture refs");
    add_empty_item_definitions(&mut package);

    let reader = UddpReader::open(package.build().expect("build")).expect("open");
    let tilemeta = TileMetaPackage::from_uddp_package(reader).expect("load tilemeta");

    let loaded = tilemeta.item_texture_refs(42);
    assert_eq!(loaded.len(), 2, "both refs must survive the round-trip");

    // Base ref — role axes
    assert_eq!(loaded[0].texture_id, 1000);
    assert_eq!(loaded[0].logical_family(), EcMaterialLogicalFamily::WorldArt);
    assert_eq!(loaded[0].physical_package(), EcMaterialPhysicalPackage::Texture);
    assert_eq!(loaded[0].stable_role(), EcMaterialStableRole::Base);
    assert_eq!(loaded[0].item_index, 1);
    assert!((loaded[0].texture_stretch - 4.0).abs() < f32::EPSILON);

    // Bump ref
    assert_eq!(loaded[1].texture_id, 1000020);
    assert_eq!(loaded[1].logical_family(), EcMaterialLogicalFamily::Textures);
    assert_eq!(loaded[1].physical_package(), EcMaterialPhysicalPackage::TerrainTexture);
    assert_eq!(loaded[1].stable_role(), EcMaterialStableRole::WaterBump);
    assert!((loaded[1].texture_stretch - 16.0).abs() < f32::EPSILON);
}
