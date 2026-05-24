use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre;
use guillotiere::{size2, AtlasAllocator};
use udd_container::{AddFileRequest, CompressionFlag, DataType, UddpBuilder};

use crate::tex_art_cc::crop_rgba_page;
use crate::{merge_unplaced_tiles, resolve_packing_axis, AtlasPackingMode};

pub const GUMP_ATLAS_PAGE_MANIFEST_ID: u32 = 0xE000_0000;
pub const GUMP_ATLAS_SLOT_MANIFEST_ID: u32 = 0xE000_0001;
pub const GUMP_ATLAS_PAGE_ID_BASE: u32 = 0xF000_0000;
pub const GUMP_ATLAS_PAGE_MANIFEST_MAGIC: [u8; 4] = *b"GAPG";
pub const GUMP_ATLAS_SLOT_MANIFEST_MAGIC: [u8; 4] = *b"GASL";
pub const GUMP_ATLAS_METADATA_VERSION: u32 = 1;

pub const DEFAULT_GUMP_ATLAS_WIDTH: u32 = 2048;
pub const DEFAULT_GUMP_ATLAS_HEIGHT: u32 = 2048;
pub const DEFAULT_GUMP_ATLAS_GUTTER: u16 = 1;

pub const PAPERDOLL_EQUIPMENT_GUMP_ID_START: u32 = 50_001;
pub const PAPERDOLL_EQUIPMENT_GUMP_ID_END: u32 = 69_999;

#[derive(Debug, Clone, Copy)]
pub struct GumpAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
    pub compression: CompressionFlag,
}

impl Default for GumpAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_GUMP_ATLAS_WIDTH,
            atlas_height: DEFAULT_GUMP_ATLAS_HEIGHT,
            gutter: DEFAULT_GUMP_ATLAS_GUTTER,
            compression: CompressionFlag::ZstdNoDict,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedGump {
    pub gump_id: u32,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct GumpAtlasPageRecord {
    pub page_index: u32,
    pub gump_count: u32,
    pub used_width: u32,
    pub used_height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GumpAtlasSlotRecord {
    pub gump_id: u32,
    pub page_index: u32,
    pub page_gump_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy)]
struct PlacedGump {
    gump_id: u32,
    page_gump_index: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

#[derive(Debug, Clone)]
struct BuiltGumpAtlasPage {
    record: GumpAtlasPageRecord,
    pixels: Vec<u8>,
    placed_gumps: Vec<PlacedGump>,
}

pub fn is_paperdoll_equipment_gump_id(gump_id: u32) -> bool {
    (PAPERDOLL_EQUIPMENT_GUMP_ID_START..=PAPERDOLL_EQUIPMENT_GUMP_ID_END).contains(&gump_id)
}

pub fn add_gump_atlas_files(
    builder: &mut UddpBuilder,
    gumps: Vec<DecodedGump>,
    options: &GumpAtlasOptions,
) -> eyre::Result<u32> {
    if gumps.is_empty() {
        return Ok(0);
    }

    validate_options(options)?;
    let (pages, slots) = pack_gumps_into_pages(gumps, options)?;
    let page_manifest = serialize_page_manifest(options, &pages)?;
    let slot_manifest = serialize_slot_manifest(options, &slots)?;

    builder.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: options.compression,
        width: 0,
        height: 0,
        virtual_path: None,
        path_hash64: None,
        id: Some(GUMP_ATLAS_PAGE_MANIFEST_ID),
        data: &page_manifest,
    })?;
    builder.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: options.compression,
        width: 0,
        height: 0,
        virtual_path: None,
        path_hash64: None,
        id: Some(GUMP_ATLAS_SLOT_MANIFEST_ID),
        data: &slot_manifest,
    })?;

    for page in &pages {
        let payload = crop_rgba_page(
            &page.pixels,
            options.atlas_width,
            page.record.used_width,
            page.record.used_height,
        );
        builder.add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width: page.record.used_width,
            height: page.record.used_height,
            virtual_path: None,
            path_hash64: None,
            id: Some(GUMP_ATLAS_PAGE_ID_BASE + page.record.page_index),
            data: &payload,
        })?;
    }

    Ok(slots.len() as u32)
}

fn validate_options(options: &GumpAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("gump atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("gump atlas dimensions must fit into metadata u16 fields");
    }
    Ok(())
}

fn pack_gumps_into_pages(
    gumps: Vec<DecodedGump>,
    options: &GumpAtlasOptions,
) -> eyre::Result<(Vec<BuiltGumpAtlasPage>, Vec<GumpAtlasSlotRecord>)> {
    let mut pages = Vec::new();
    let mut slots = Vec::new();
    let mut remaining = gumps;
    remaining.sort_by_key(|gump| gump.gump_id);
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page_gumps, leftovers) = take_page_gump_prefix(remaining, options)?;
        let (page, unplaced) = build_page(page_index, page_gumps, options)?;
        if page.placed_gumps.is_empty() {
            eyre::bail!(
                "could not fit any paperdoll gump into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
        }

        slots.extend(page.placed_gumps.iter().map(|placed| GumpAtlasSlotRecord {
            gump_id: placed.gump_id,
            page_index,
            page_gump_index: placed.page_gump_index,
            x: placed.x,
            y: placed.y,
            width: placed.width,
            height: placed.height,
        }));

        pages.push(page);
        remaining = merge_unplaced_tiles(leftovers, unplaced, |gump| gump.gump_id);
        page_index += 1;
    }

    Ok((pages, slots))
}

fn take_page_gump_prefix(
    gumps: Vec<DecodedGump>,
    options: &GumpAtlasOptions,
) -> eyre::Result<(Vec<DecodedGump>, Vec<DecodedGump>)> {
    let prefix_len = max_fitting_page_prefix_len(&gumps, options)?;
    if prefix_len == 0 {
        eyre::bail!(
            "could not fit any paperdoll gump into atlas page {}x{}",
            options.atlas_width,
            options.atlas_height
        );
    }

    let mut leftovers = gumps;
    let selected = leftovers.drain(..prefix_len).collect::<Vec<_>>();
    Ok((selected, leftovers))
}

fn max_fitting_page_prefix_len(gumps: &[DecodedGump], options: &GumpAtlasOptions) -> eyre::Result<usize> {
    let mut low = 1usize;
    let mut high = gumps.len();
    let mut best = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        if page_prefix_fits(&gumps[..mid], options)? {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }

    Ok(best)
}

fn page_prefix_fits(gumps: &[DecodedGump], options: &GumpAtlasOptions) -> eyre::Result<bool> {
    let mut to_pack = gumps.to_vec();
    sort_gumps_within_page(&mut to_pack);
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));

    for gump in &to_pack {
        let width_axis = resolve_packing_axis(
            gump.width as u32,
            options.atlas_width,
            options.gutter,
            AtlasPackingMode::MaximumPacking,
            false,
        );
        let height_axis = resolve_packing_axis(
            gump.height as u32,
            options.atlas_height,
            options.gutter,
            AtlasPackingMode::MaximumPacking,
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!(
                "paperdoll gump {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                gump.gump_id,
                gump.width,
                gump.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        };

        if allocator
            .allocate(size2(width_axis.alloc_extent as i32, height_axis.alloc_extent as i32))
            .is_none()
        {
            return Ok(false);
        }
    }

    Ok(true)
}

fn build_page(
    page_index: u32,
    mut gumps: Vec<DecodedGump>,
    options: &GumpAtlasOptions,
) -> eyre::Result<(BuiltGumpAtlasPage, Vec<DecodedGump>)> {
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut placed_gumps = Vec::new();
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;

    sort_gumps_within_page(&mut gumps);

    for gump in gumps {
        let width_axis = resolve_packing_axis(
            gump.width as u32,
            options.atlas_width,
            options.gutter,
            AtlasPackingMode::MaximumPacking,
            false,
        );
        let height_axis = resolve_packing_axis(
            gump.height as u32,
            options.atlas_height,
            options.gutter,
            AtlasPackingMode::MaximumPacking,
            false,
        );
        let (Some(width_axis), Some(height_axis)) = (width_axis, height_axis) else {
            eyre::bail!(
                "paperdoll gump {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                gump.gump_id,
                gump.width,
                gump.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        };

        if let Some(allocation) = allocator.allocate(size2(
            width_axis.alloc_extent as i32,
            height_axis.alloc_extent as i32,
        )) {
            let inner_x = allocation.rectangle.min.x + width_axis.leading_padding as i32;
            let inner_y = allocation.rectangle.min.y + height_axis.leading_padding as i32;
            blit_rgba_gump(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                gump.width as u32,
                gump.height as u32,
                &gump.rgba,
            )?;
            used_width = used_width.max(inner_x as u32 + width_axis.used_extent);
            used_height = used_height.max(inner_y as u32 + height_axis.used_extent);
            placed_gumps.push(PlacedGump {
                gump_id: gump.gump_id,
                page_gump_index: placed_gumps.len() as u16,
                x: inner_x as u16,
                y: inner_y as u16,
                width: gump.width,
                height: gump.height,
            });
        } else {
            leftovers.push(gump);
        }
    }

    Ok((
        BuiltGumpAtlasPage {
            record: GumpAtlasPageRecord {
                page_index,
                gump_count: placed_gumps.len() as u32,
                used_width,
                used_height,
            },
            pixels,
            placed_gumps,
        },
        leftovers,
    ))
}

fn sort_gumps_within_page(gumps: &mut [DecodedGump]) {
    gumps.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.gump_id.cmp(&right.gump_id))
    });
}

fn blit_rgba_gump(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    gump_width: u32,
    gump_height: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let expected_len = gump_width as usize * gump_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for gump {}x{}: expected {}, got {}",
            gump_width,
            gump_height,
            expected_len,
            src.len()
        );
    }

    let dst_stride = dst_width as usize * 4;
    let src_stride = gump_width as usize * 4;
    for row in 0..gump_height as usize {
        let src_start = row * src_stride;
        let dst_start = ((dst_y as usize + row) * dst_stride) + dst_x as usize * 4;
        let dst_end = dst_start + src_stride;
        dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + src_stride]);
    }

    Ok(())
}

fn serialize_page_manifest(
    options: &GumpAtlasOptions,
    pages: &[BuiltGumpAtlasPage],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + pages.len() * 16);
    bytes.extend_from_slice(&GUMP_ATLAS_PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(GUMP_ATLAS_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(u32::from(options.gutter))?;
    bytes.write_u32::<LittleEndian>(pages.len() as u32)?;
    for page in pages {
        bytes.write_u32::<LittleEndian>(page.record.page_index)?;
        bytes.write_u32::<LittleEndian>(page.record.gump_count)?;
        bytes.write_u32::<LittleEndian>(page.record.used_width)?;
        bytes.write_u32::<LittleEndian>(page.record.used_height)?;
    }
    Ok(bytes)
}

fn serialize_slot_manifest(
    options: &GumpAtlasOptions,
    slots: &[GumpAtlasSlotRecord],
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + slots.len() * 20);
    bytes.extend_from_slice(&GUMP_ATLAS_SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(GUMP_ATLAS_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(u32::from(options.gutter))?;
    bytes.write_u32::<LittleEndian>(slots.len() as u32)?;
    for slot in slots {
        bytes.write_u32::<LittleEndian>(slot.gump_id)?;
        bytes.write_u32::<LittleEndian>(slot.page_index)?;
        bytes.write_u16::<LittleEndian>(slot.page_gump_index)?;
        bytes.write_u16::<LittleEndian>(slot.x)?;
        bytes.write_u16::<LittleEndian>(slot.y)?;
        bytes.write_u16::<LittleEndian>(slot.width)?;
        bytes.write_u16::<LittleEndian>(slot.height)?;
        bytes.write_u16::<LittleEndian>(0)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::ReadBytesExt;
    use std::io::Cursor;
    use udd_container::{LookupMode, UddpReader};

    #[test]
    fn paperdoll_equipment_range_matches_profile_offsets() {
        assert!(!is_paperdoll_equipment_gump_id(50_000));
        assert!(is_paperdoll_equipment_gump_id(50_001));
        assert!(is_paperdoll_equipment_gump_id(60_001));
        assert!(is_paperdoll_equipment_gump_id(69_999));
        assert!(!is_paperdoll_equipment_gump_id(70_000));
    }

    #[test]
    fn gump_atlas_packs_and_serializes_metadata() {
        let gumps = vec![
            DecodedGump {
                gump_id: 50_001,
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            },
            DecodedGump {
                gump_id: 60_001,
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            },
        ];
        let options = GumpAtlasOptions {
            atlas_width: 8,
            atlas_height: 8,
            gutter: 1,
            compression: CompressionFlag::None,
        };

        let (pages, slots) = pack_gumps_into_pages(gumps, &options).unwrap();
        let page_manifest = serialize_page_manifest(&options, &pages).unwrap();
        let slot_manifest = serialize_slot_manifest(&options, &slots).unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(slots.len(), 2);
        assert_eq!(&page_manifest[..4], &GUMP_ATLAS_PAGE_MANIFEST_MAGIC);
        assert_eq!(&slot_manifest[..4], &GUMP_ATLAS_SLOT_MANIFEST_MAGIC);
    }

    #[test]
    fn gump_atlas_writes_reserved_sparse_entries() {
        let options = GumpAtlasOptions {
            atlas_width: 8,
            atlas_height: 8,
            gutter: 1,
            compression: CompressionFlag::None,
        };
        let mut builder = UddpBuilder::new(LookupMode::SparseId);

        let packed_count = add_gump_atlas_files(
            &mut builder,
            vec![
                solid_gump(50_001, 2, 2),
                solid_gump(60_001, 1, 1),
            ],
            &options,
        )
        .expect("add atlas files");
        let package = UddpReader::open(builder.build().expect("build package")).expect("open package");

        assert_eq!(packed_count, 2);
        assert_eq!(package.lookup_mode(), LookupMode::SparseId);
        assert_eq!(
            package
                .find_by_sparse_id(GUMP_ATLAS_PAGE_MANIFEST_ID)
                .expect("page manifest")
                .data_type,
            DataType::Metadata as u8
        );
        assert_eq!(
            package
                .find_by_sparse_id(GUMP_ATLAS_SLOT_MANIFEST_ID)
                .expect("slot manifest")
                .data_type,
            DataType::Metadata as u8
        );
        assert_eq!(
            package
                .find_by_sparse_id(GUMP_ATLAS_PAGE_ID_BASE)
                .expect("atlas page")
                .data_type,
            DataType::Texture as u8
        );

        let page_manifest = package
            .read_file_by_sparse_id(GUMP_ATLAS_PAGE_MANIFEST_ID)
            .expect("read page manifest");
        let slot_manifest = package
            .read_file_by_sparse_id(GUMP_ATLAS_SLOT_MANIFEST_ID)
            .expect("read slot manifest");

        assert_eq!(page_manifest_header(&page_manifest), (8, 8, 1, 1));
        assert_eq!(slot_manifest_header(&slot_manifest), (8, 8, 1, 2));
    }

    #[test]
    fn gump_atlas_splits_across_pages_when_needed() {
        let options = GumpAtlasOptions {
            atlas_width: 4,
            atlas_height: 4,
            gutter: 0,
            compression: CompressionFlag::None,
        };

        let (pages, slots) = pack_gumps_into_pages(
            vec![solid_gump(50_001, 3, 3), solid_gump(50_002, 3, 3)],
            &options,
        )
        .expect("pack gumps");

        assert_eq!(pages.len(), 2);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].page_index, 0);
        assert_eq!(slots[1].page_index, 1);
    }

    #[test]
    fn gump_atlas_rejects_gumps_larger_than_page() {
        let options = GumpAtlasOptions {
            atlas_width: 4,
            atlas_height: 4,
            gutter: 0,
            compression: CompressionFlag::None,
        };

        let error = pack_gumps_into_pages(vec![solid_gump(50_001, 5, 4)], &options)
            .expect_err("oversized gump should not fit");

        assert!(error.to_string().contains("does not fit into atlas page"));
    }

    fn solid_gump(gump_id: u32, width: u16, height: u16) -> DecodedGump {
        DecodedGump {
            gump_id,
            width,
            height,
            rgba: vec![255; width as usize * height as usize * 4],
        }
    }

    fn page_manifest_header(bytes: &[u8]) -> (u32, u32, u32, u32) {
        assert_eq!(&bytes[..4], &GUMP_ATLAS_PAGE_MANIFEST_MAGIC);
        read_manifest_common_header(bytes)
    }

    fn slot_manifest_header(bytes: &[u8]) -> (u32, u32, u32, u32) {
        assert_eq!(&bytes[..4], &GUMP_ATLAS_SLOT_MANIFEST_MAGIC);
        read_manifest_common_header(bytes)
    }

    fn read_manifest_common_header(bytes: &[u8]) -> (u32, u32, u32, u32) {
        let mut cursor = Cursor::new(&bytes[4..]);
        let version = cursor.read_u32::<LittleEndian>().expect("version");
        let width = cursor.read_u32::<LittleEndian>().expect("width");
        let height = cursor.read_u32::<LittleEndian>().expect("height");
        let gutter = cursor.read_u32::<LittleEndian>().expect("gutter");
        let count = cursor.read_u32::<LittleEndian>().expect("count");

        assert_eq!(version, GUMP_ATLAS_METADATA_VERSION);
        (width, height, gutter, count)
    }
}
