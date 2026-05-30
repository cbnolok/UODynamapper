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
        TileMetaItemTile, TileMetaLandTile, TileMetaItemVisualKind,
        TILEMETA_LAND_ENTRY_PATH, TILEMETA_ITEM_ENTRY_PATH,
    },
    tex_art_ec::TexArtEcCropAdjustment,
    TileMetaPackage,
};
use udd_conv::tilemeta::*;
use udd_container::CompressionFlag;

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
        _pad2: 0,
        radar_color: [9, 10, 11, 12],
        name: {
            let mut name = [0u8; 20];
            name[..5].copy_from_slice(b"chair");
            name
        },
        ec_texture_id: 100,
        ec_start_x: 101,
        ec_start_y: 102,
        ec_offset_x: 103,
        ec_offset_y: 104,
        cc_texture_id: 200,
        cc_start_x: 201,
        cc_start_y: 202,
        cc_offset_x: 203,
        cc_offset_y: 204,
    };

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
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
fn tilemeta_crop_adjustment_shifts_ec_sampling_start_only() {
    let adjusted = adjusted_ec_sampling_start(
        42,
        12,
        18,
        Some(TexArtEcCropAdjustment {
            source_left: 5,
            source_top: 7,
            trim_left: 2,
            trim_top: 3,
            trim_right: 4,
            trim_bottom: 6,
        }),
    )
    .expect("adjust EC sampling start");

    assert_eq!(adjusted, (7, 11));
}

#[test]
fn tilemeta_crop_adjustment_shifts_ec_draw_offset_to_trimmed_visual_bounds() {
    let adjusted = adjusted_ec_draw_offset(
        42,
        12,
        18,
        Some(TexArtEcCropAdjustment {
            source_left: 5,
            source_top: 7,
            trim_left: 2,
            trim_top: 3,
            trim_right: 4,
            trim_bottom: 6,
        }),
    )
    .expect("adjust EC draw offset");

    assert_eq!(adjusted, (14, 12));
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
