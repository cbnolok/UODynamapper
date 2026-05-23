use udd_container::{
    AddFileRequest, DataType, LookupMode, UddpBuilder, UddpReader,
};

use udd_conv::tex_art_cc::*;
use udd_conv::upscale::UpscaleFilter;
use udd_conv::AtlasPackingMode;
use udd_assets::{
    tex_art_cc::{
        MISSING_PAGE_INDEX, page_entry_path, PagePixelFormat,
        PAGE_MANIFEST_ENTRY_PATH, SLOT_MANIFEST_ENTRY_PATH,
    },
    TexArtCcPackage,
};
use udd_container::CompressionFlag;

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
    let options = TexArtCcAtlasOptions {
        atlas_width: 16,
        atlas_height: 16,
        gutter: 1,
        compression: CompressionFlag::None,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
            bc7_rdo_lambda: udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Land, 4, 4),
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
fn packer_spills_to_multiple_pages() {
    let options = TexArtCcAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        compression: CompressionFlag::None,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
            bc7_rdo_lambda: udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Land, 4, 4),
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
    let options = TexArtCcAtlasOptions {
        atlas_width: 10,
        atlas_height: 6,
        gutter: 0,
        compression: CompressionFlag::None,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
            bc7_rdo_lambda: udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA,
    };
    let tiles = vec![
        rgba_tile(0, ArtTileKind::Static, 6, 6),
        rgba_tile(1, ArtTileKind::Static, 6, 6),
        rgba_tile(2, ArtTileKind::Static, 1, 1),
    ];

    let (pages, slots) = pack_tiles_into_pages(tiles, 3, &options).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(slots[0].page_index, 0);
    assert_eq!(pages[0].record.tile_count, 1);
    assert_eq!(slots[1].page_index, 1);
    assert_eq!(slots[2].page_index, 1);
}

#[test]
fn runtime_reader_can_unpack_page_and_slot_metadata() {
    let options = TexArtCcAtlasOptions {
        atlas_width: 8,
        atlas_height: 8,
        gutter: 1,
        compression: CompressionFlag::ZstdNoDict,
        upscale: UpscaleFilter::default(),
        pixel_format: PagePixelFormat::Rgba8888,
        packing_mode: AtlasPackingMode::MaximumPacking,
        filtering_ready: false,
            bc7_rdo_lambda: udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA,
    };
    let tiles = vec![rgba_tile(0, ArtTileKind::Land, 4, 4)];
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
        TexArtCcPackage::from_uddp_package(UddpReader::open(package.build().unwrap()).unwrap())
            .unwrap();
    assert_eq!(package.pages().len(), 1);
    assert!(package.present_slot(0).unwrap().is_land());
    let page = &package.pages()[0];
    assert_eq!(package.read_page_bytes(0).unwrap().len(), (page.used_width * page.used_height * 4) as usize);
}
