use std::collections::HashMap;
use uocf::enhanced::tileart::ArtTexture;
use udd_container::{
    AddFileRequest, DataType, LookupMode, UddpBuilder, UddpReader,
};

use udd_conv::tex_art_ec::*;
use udd_conv::upscale::UpscaleFilter;
use udd_conv::AtlasPackingMode;
use udd_assets::{
    tex_art_ec::{TexArtEcCropAdjustment, PAGE_MANIFEST_ENTRY_PATH, SLOT_MANIFEST_ENTRY_PATH},
    tex_art_cc::{MISSING_PAGE_INDEX, page_entry_path, PagePixelFormat},
    TexArtEcPackage,
};
use udd_container::CompressionFlag;
use uocf::enhanced::tileart::{TaeFlag, TileType};

fn rgba_tile(art_id: u32, kind: ArtTileKind, width: u16, height: u16) -> DecodedArtTile {
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
    let options = TexArtEcAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Static, 4, 4),
        rgba_tile(3, ArtTileKind::Static, 4, 4),
    ];

    let (_pages, slots) = pack_tiles_into_pages(tiles, 5, &options).unwrap();

    assert!(slots[0].is_present());
    assert!(!slots[1].is_present());
    assert!(!slots[2].is_present());
    assert!(slots[3].is_present());
    assert_eq!(slots[4].page_index, MISSING_PAGE_INDEX);
}

#[test]
fn tileart_unused1_surfaces_pack_as_land_material_slots() {
    assert_eq!(
        art_tile_kind_for_tileart(TileType::Solid, TaeFlag::Unused1),
        ArtTileKind::Land
    );
    assert_eq!(
        art_tile_kind_for_tileart(TileType::Liquid, TaeFlag::None),
        ArtTileKind::Land
    );
    assert_eq!(
        art_tile_kind_for_tileart(TileType::Static, TaeFlag::None),
        ArtTileKind::Static
    );
}

#[test]
fn packer_spills_to_multiple_pages() {
    let options = TexArtEcAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Static, 4, 4),
        rgba_tile(1, ArtTileKind::Static, 4, 4),
    ];

    let (pages, slots) = pack_tiles_into_pages(tiles, 2, &options).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].record.tile_count, 1);
    assert_eq!(pages[1].record.tile_count, 1);
    assert_eq!(slots[0].page_index, 0);
    assert_eq!(slots[1].page_index, 1);
}

#[test]
fn later_ids_do_not_backfill_an_earlier_page() {
    let options = TexArtEcAtlasOptions {
        atlas_width: 10,
        atlas_height: 6,
        gutter: 0,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Static, 6, 6),
        rgba_tile(1, ArtTileKind::Static, 6, 6),
        rgba_tile(2, ArtTileKind::Static, 1, 1),
    ];

    let (pages, slots) = pack_tiles_into_pages(tiles, 0x10000, &options).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(slots[0].page_index, 0);
    assert_eq!(pages[0].record.tile_count, 1);
    assert_eq!(slots[1].page_index, 1);
    assert_eq!(slots[2].page_index, 1);
}

#[test]
fn full_width_static_tile_fits_when_page_is_4096_wide() {
    let options = TexArtEcAtlasOptions {
        atlas_width: 4096,
        atlas_height: 2048,
        gutter: 1,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![rgba_tile(41339, ArtTileKind::Static, 4096, 128)];

    let (pages, slots) = pack_tiles_into_pages(tiles, 0x10000, &options).unwrap();

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].record.tile_count, 1);
    assert_eq!(pages[0].placed_tiles[0].x, 0);
    assert_eq!(pages[0].placed_tiles[0].y, 1);
    assert_eq!(slots[41339].page_index, 0);
    assert_eq!(slots[41339].width, 4096);
    assert_eq!(slots[41339].height, 128);
}

#[test]
fn runtime_reader_can_unpack_page_and_slot_metadata() {
    let options = TexArtEcAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![rgba_tile(0, ArtTileKind::Static, 4, 4)];
    let (pages, slots) = pack_tiles_into_pages(tiles, 1, &options).unwrap();
    let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
    let slot_manifest = serialize_slot_manifest(&slots, &options).unwrap();

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(PAGE_MANIFEST_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &page_manifest,
        })
        .unwrap();
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(SLOT_MANIFEST_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &slot_manifest,
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
            compression: CompressionFlag::ZstdNoDict,
            width: 0, height: 0,
            virtual_path: Some(&page_path),
            path_hash64: None,
            id: None,
            data: &stored_page,
        })
        .unwrap();

    let package =
        TexArtEcPackage::from_uddp_package(UddpReader::open(package.build().unwrap()).unwrap())
            .unwrap();
    assert_eq!(package.pages().len(), 1);
    assert!(package.present_slot(0).unwrap().is_static());
    let page = &package.pages()[0];
    assert_eq!(package.read_page_bytes(0).unwrap().len(), (page.used_width * page.used_height * 4) as usize);
}

#[test]
fn alias_slots_reuse_canonical_page_location() {
    let options = TexArtEcAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        crop_transparent_bounds: false,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
    };
    let tiles = vec![rgba_tile(7, ArtTileKind::Static, 4, 4)];

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
fn transparent_border_crop_trims_to_opaque_bounds() {
    let mut rgba = vec![0u8; 4 * 4 * 4];
    for y in 1..=2usize {
        for x in 1..=2usize {
            let index = (y * 4 + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[10, 20, 30, 255]);
        }
    }

    let (width, height, cropped, adjustment) = crop_rgba_tile_to_bounds(4, 4, rgba, None)
        .expect("crop rgba tile");

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(cropped.len(), 2 * 2 * 4);
    assert_eq!(adjustment, TexArtEcCropAdjustment { left: 1, top: 1 });
}

#[test]
fn clip_rect_is_applied_before_alpha_trim() {
    let mut rgba = vec![0u8; 6 * 6 * 4];
    for y in 0..6usize {
        let index = (y * 6 + 5) * 4;
        rgba[index..index + 4].copy_from_slice(&[10, 20, 30, 255]);
    }
    for y in 2..=3usize {
        for x in 2..=3usize {
            let index = (y * 6 + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[40, 50, 60, 255]);
        }
    }

    let (width, height, cropped, adjustment) = crop_rgba_tile_to_bounds(
        6,
        6,
        rgba,
        Some(SourceClipRect {
            left: 1,
            top: 1,
            right: 5,
            bottom: 5,
        }),
    )
    .expect("crop clipped rgba tile");

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(cropped.len(), 2 * 2 * 4);
    assert_eq!(adjustment, TexArtEcCropAdjustment { left: 2, top: 2 });
}

#[test]
fn requested_clip_rect_is_applied_without_alpha_trim() {
    let mut rgba = vec![0u8; 6 * 6 * 4];
    for y in 0..6usize {
        for x in 0..6usize {
            let index = (y * 6 + x) * 4;
            rgba[index..index + 4].copy_from_slice(&[x as u8, y as u8, 99, 255]);
        }
    }

    let (width, height, cropped, adjustment) = apply_requested_clip_rect(
        6,
        6,
        rgba,
        Some(SourceClipRect {
            left: 2,
            top: 1,
            right: 4,
            bottom: 3,
        }),
    )
    .expect("apply clip rect without alpha trim");

    assert_eq!(width, 2);
    assert_eq!(height, 2);
    assert_eq!(adjustment, TexArtEcCropAdjustment { left: 2, top: 1 });
    assert_eq!(cropped.len(), 2 * 2 * 4);
    assert_eq!(&cropped[0..4], &[2, 1, 99, 255]);
    assert_eq!(&cropped[4..8], &[3, 1, 99, 255]);
    assert_eq!(&cropped[8..12], &[2, 2, 99, 255]);
    assert_eq!(&cropped[12..16], &[3, 2, 99, 255]);
}

#[test]
fn normalized_source_clip_rect_ignores_empty_rects() {
    let texture = ArtTexture {
        texture_id: 1,
        start_x: 0,
        start_y: 1,
        end_x: 0,
        end_y: 1,
        offset_x: 0,
        offset_y: 0,
    };

    assert_eq!(normalized_source_clip_rect(8, 8, &texture), None);
}

#[test]
fn crop_adjustment_lookup_copies_canonical_adjustment_to_aliases() {
    let lookup = build_crop_adjustment_lookup(
        16,
        &HashMap::from([(7, TexArtEcCropAdjustment { left: 3, top: 4 })]),
        &[SlotAlias {
            art_id: 9,
            canonical_art_id: 7,
        }],
    );

    assert_eq!(lookup[7], Some(TexArtEcCropAdjustment { left: 3, top: 4 }));
    assert_eq!(lookup[9], Some(TexArtEcCropAdjustment { left: 3, top: 4 }));
    assert_eq!(lookup[6], None);
}

#[test]
fn canonical_tile_key_distinguishes_different_sampling_windows() {
    let source = TextureSourceKey::World(77);
    let canonical = CanonicalTileKey {
        source,
        window: requested_source_window(&ArtTexture {
            texture_id: 77,
            start_x: 0,
            start_y: 0,
            end_x: 32,
            end_y: 32,
            offset_x: 0,
            offset_y: 0,
        }),
    };
    let different_window = CanonicalTileKey {
        source,
        window: requested_source_window(&ArtTexture {
            texture_id: 77,
            start_x: 8,
            start_y: 4,
            end_x: 40,
            end_y: 36,
            offset_x: 0,
            offset_y: 0,
        }),
    };

    assert_ne!(canonical, different_window);
}

#[test]
fn identical_final_payloads_alias_by_rendered_pixels() {
    let mut canonical_by_rendered_tile = HashMap::new();
    let rgba = vec![
        10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
    ];

    assert_eq!(
        register_rendered_tile_alias(7, 2, 2, &rgba, &mut canonical_by_rendered_tile),
        None
    );
    assert_eq!(
        register_rendered_tile_alias(9, 2, 2, &rgba, &mut canonical_by_rendered_tile),
        Some(7)
    );
    assert_eq!(canonical_by_rendered_tile.len(), 1);
}
