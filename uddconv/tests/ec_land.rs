use uocf::udd::{
    AddFileRequest, DataType, LookupMode, CompressionFlag as UddCompressionFlag, UddpBuilder, UddpReader,
};

use uddconv::ec_land::*;
use uddconv::cc_art::PagePixelFormat;
use uddconv::upscale::{UpscaleConfig};

pub fn rgba_tile(art_id: u32, kind: ArtTileKind, width: u16, height: u16) -> DecodedArtTile {
    DecodedArtTile {
        art_id,
        kind,
        width,
        height,
        rgba: vec![255u8; width as usize * height as usize * 4],
    }
}

#[test]
fn sparse_slots_keep_absent_records() {
    let options = EcLandAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        compression: UddCompressionFlag::ZstdNoDict,
        upscale_64: UpscaleConfig::default(),
        upscale_128: UpscaleConfig::default(),
        upscale_256: UpscaleConfig::default(),
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Land, 4, 4),
        rgba_tile(3, ArtTileKind::Land, 4, 4),
    ];

    let (_pages, slots) = pack_tiles_into_pages(tiles, 5, &options).unwrap();

    assert!(slots[0].is_present());
    assert!(!slots[1].is_present());
    assert!(!slots[2].is_present());
    assert!(slots[3].is_present());
    assert_eq!(slots[4].page_index, MISSING_PAGE_INDEX);
}

#[test]
fn packer_spills_to_multiple_pages() {
    let options = EcLandAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        compression: UddCompressionFlag::ZstdNoDict,
        upscale_64: UpscaleConfig::default(),
        upscale_128: UpscaleConfig::default(),
        upscale_256: UpscaleConfig::default(),
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Land, 4, 4),
        rgba_tile(1, ArtTileKind::Land, 4, 4),
    ];

    let (pages, slots) = pack_tiles_into_pages(tiles, 2, &options).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].record.tile_count, 1);
    assert_eq!(pages[1].record.tile_count, 1);
    assert_eq!(slots[0].page_index, 0);
    assert_eq!(slots[1].page_index, 1);
}

#[test]
fn runtime_reader_can_unpack_page_and_slot_metadata() {
    let options = EcLandAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        compression: UddCompressionFlag::ZstdNoDict,
        upscale_64: UpscaleConfig::default(),
        upscale_128: UpscaleConfig::default(),
        upscale_256: UpscaleConfig::default(),
    };
    let tiles = vec![rgba_tile(0, ArtTileKind::Land, 4, 4)];
    let (pages, slots) = pack_tiles_into_pages(tiles, 1, &options).unwrap();
    let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
    let slot_manifest = serialize_slot_manifest(&slots, &options).unwrap();
    let terrain_provenance = vec![EcLandTerrainProvenanceRecord {
        material_id: 12,
        material_name_id: 34,
        alias_count_index: 0,
        alias_slot_id: 0,
        alias_tile_flags: 0x55,
        selected_texture_id: 2_000_540,
        canonical_slot_id: 0,
    }];
    let terrain_provenance_manifest =
        serialize_terrain_provenance_manifest(&terrain_provenance).unwrap();

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_PAGE_MANIFEST_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &page_manifest,
        })
        .unwrap();
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_SLOT_MANIFEST_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &slot_manifest,
        })
        .unwrap();
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &terrain_provenance_manifest,
        })
        .unwrap();
    let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
    let stored_page = crop_rgba_page(
        &pages[0].pixels,
        options.atlas_width,
        pages[0].record.used_width,
        pages[0].record.used_height,
    );
    package
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(&page_path),
            path_hash64: None,
            id: None,
            data: &stored_page,
        })
        .unwrap();

    let package =
        EcLandPackage::from_uddp_package(UddpReader::open(package.build().unwrap()).unwrap())
            .unwrap();
    assert_eq!(package.pages().len(), 1);
    assert!(package.present_slot(0).unwrap().is_land());
    assert_eq!(package.terrain_provenance(), terrain_provenance);
    let page = &package.pages()[0];
    assert_eq!(
        package.read_page_bytes(0).unwrap().len(),
        (page.used_width * page.used_height * 4) as usize
    );
}

#[test]
fn land_alias_slots_reuse_canonical_page_location() {
    let options = EcLandAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        compression: UddCompressionFlag::ZstdNoDict,
        upscale_64: UpscaleConfig::default(),
        upscale_128: UpscaleConfig::default(),
        upscale_256: UpscaleConfig::default(),
    };
    let tiles = vec![rgba_tile(7, ArtTileKind::Land, 4, 4)];

    let (_pages, mut slots) = pack_tiles_into_pages(tiles, 16, &options).unwrap();
    apply_slot_aliases(
        &mut slots,
        &[SlotAlias {
            art_id: 9,
            canonical_art_id: 7,
        }],
    )
    .unwrap();

    assert!(slots[7].is_present());
    assert!(slots[9].is_present());
    assert_eq!(slots[9].art_id, 9);
    assert_eq!(slots[9].page_index, slots[7].page_index);
    assert_eq!(slots[9].page_tile_index, slots[7].page_tile_index);
    assert_eq!(slots[9].x, slots[7].x);
    assert_eq!(slots[9].y, slots[7].y);
    assert_eq!(slots[9].width, slots[7].width);
    assert_eq!(slots[9].height, slots[7].height);
}

#[test]
fn runtime_slot_resolution_uses_direct_slots_before_provenance_fallback() {
    let options = EcLandAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        compression: UddCompressionFlag::ZstdNoDict,
        upscale_64: UpscaleConfig::default(),
        upscale_128: UpscaleConfig::default(),
        upscale_256: UpscaleConfig::default(),
    };
    let tiles = vec![
        rgba_tile(2, ArtTileKind::Land, 4, 4),
        rgba_tile(26, ArtTileKind::Land, 4, 4),
        rgba_tile(196, ArtTileKind::Land, 4, 4),
        rgba_tile(172, ArtTileKind::Land, 4, 4),
        rgba_tile(581, ArtTileKind::Land, 4, 4),
        rgba_tile(16_110, ArtTileKind::Land, 4, 4),
        rgba_tile(16_111, ArtTileKind::Land, 4, 4),
    ];
    let (pages, slots) = pack_tiles_into_pages(tiles, 16_112, &options).unwrap();
    let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
    let slot_manifest = serialize_slot_manifest(&slots, &options).unwrap();
    let terrain_provenance = vec![
        EcLandTerrainProvenanceRecord {
            material_id: 54,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 26,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_540,
            canonical_slot_id: 26,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 171,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 586,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_131,
            canonical_slot_id: 581,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 4,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 0,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_000,
            canonical_slot_id: 2,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 4,
            material_name_id: 0,
            alias_count_index: 1,
            alias_slot_id: 196,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_040,
            canonical_slot_id: 196,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 172,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 0,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_000,
            canonical_slot_id: 0,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 172,
            material_name_id: 0,
            alias_count_index: 1,
            alias_slot_id: 10_172,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_131,
            canonical_slot_id: 581,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 197,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 0,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_000,
            canonical_slot_id: 2,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 197,
            material_name_id: 0,
            alias_count_index: 1,
            alias_slot_id: 196,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_040,
            canonical_slot_id: 196,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 198,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 0,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_000,
            canonical_slot_id: 2,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 198,
            material_name_id: 0,
            alias_count_index: 1,
            alias_slot_id: 16_110,
            alias_tile_flags: 256,
            selected_texture_id: 16_110,
            canonical_slot_id: 16_110,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 199,
            material_name_id: 0,
            alias_count_index: 0,
            alias_slot_id: 0,
            alias_tile_flags: 0,
            selected_texture_id: 2_000_000,
            canonical_slot_id: 2,
        },
        EcLandTerrainProvenanceRecord {
            material_id: 199,
            material_name_id: 0,
            alias_count_index: 1,
            alias_slot_id: 16_111,
            alias_tile_flags: 256,
            selected_texture_id: 16_111,
            canonical_slot_id: 16_111,
        },
    ];
    let terrain_provenance_manifest =
        serialize_terrain_provenance_manifest(&terrain_provenance).unwrap();

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_PAGE_MANIFEST_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &page_manifest,
        })
        .unwrap();
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_SLOT_MANIFEST_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &slot_manifest,
        })
        .unwrap();
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH),
            path_hash64: None,
            id: None,
            data: &terrain_provenance_manifest,
        })
        .unwrap();

    for (page_index, page) in pages.iter().enumerate() {
        let page_path = page_entry_path(page_index as u32, PagePixelFormat::Rgba8888);
        let stored_page = crop_rgba_page(
            &page.pixels,
            options.atlas_width,
            page.record.used_width,
            page.record.used_height,
        );
        package
            .add_file(AddFileRequest {
                data_type: DataType::Texture as u8,
                compression: UddCompressionFlag::ZstdNoDict,
            width: 0, height: 0,
                virtual_path: Some(&page_path),
                path_hash64: None,
                id: None,
                data: &stored_page,
            })
            .unwrap();
    }

    let mut package =
        EcLandPackage::from_uddp_package(UddpReader::open(package.build().unwrap()).unwrap())
            .unwrap();

    // Mock the runtime transcode table injection that the dynamapper does:
    package.transcode.insert(54, 54);
    package.transcode.insert(171, 171);
    package.transcode.insert(4, 4);
    package.transcode.insert(197, 197);
    package.transcode.insert(198, 198);
    package.transcode.insert(199, 199);

    assert_eq!(package.resolve_runtime_slot_id(54), Some(26));
    assert_eq!(package.resolve_runtime_slot_id(171), Some(581));
    assert_eq!(package.resolve_runtime_slot_id(4), Some(196));
    assert_eq!(package.resolve_runtime_slot_id(197), Some(196));
    assert_eq!(package.resolve_runtime_slot_id(198), Some(16_110));
    assert_eq!(package.resolve_runtime_slot_id(199), Some(16_111));
    assert_eq!(package.resolve_runtime_slot_id(172), Some(172));
    assert_eq!(package.resolve_runtime_slot_id(26), Some(26));
}
