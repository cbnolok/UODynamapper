//! Build-time converter for classic `art.mul`/`artidx.mul` into `cc_art.uddp`.
//!
//! Package layout produced by this module:
//! - `pages/{page_index}.rgba8888`: one fixed-size atlas page per UDDP entry.
//!   Payload bytes are raw RGBA8888 page pixels and are compressed with Zstd by
//!   the UDDP writer so runtime can decompress straight into a GPU upload buffer.
//! - `metadata/pages.bin`: page table describing how many tiles ended up in each
//!   atlas page and how much of the page is actually used.
//! - `metadata/slots.bin`: one record for every `art_id` slot from `artidx.mul`,
//!   including unused slots. This is the authoritative sparse index.
//!
//! Binary layout of `metadata/pages.bin`:
//! - `b"CAPG"`
//! - `u32 version`
//! - `u32 atlas_width`
//! - `u32 atlas_height`
//! - `u32 gutter`
//! - `u32 page_count`
//! - `page_count * CcArtPageRecord`
//!
//! Binary layout of `metadata/slots.bin`:
//! - `b"CASL"`
//! - `u32 version`
//! - `u32 atlas_width`
//! - `u32 atlas_height`
//! - `u32 gutter`
//! - `u32 slot_count`
//! - `slot_count * CcArtSlotRecord`

use std::path::Path;

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, ContextCompat, WrapErr};
use guillotiere::{AtlasAllocator, size2};

use uocf::{
    classic::art::ArtMap,
    generic_index::IndexFile,
    uop::{UddpCompression, UddpContentId, UddpPackage},
};

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CASL";
const CC_ART_METADATA_VERSION: u32 = 1;
const SLOT_FLAG_PRESENT: u16 = 1 << 0;
const SLOT_FLAG_LAND: u16 = 1 << 1;
const SLOT_FLAG_STATIC: u16 = 1 << 2;
const MISSING_PAGE_INDEX: u32 = u32::MAX;
const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Clone, Copy)]
pub struct CcArtAtlasOptions {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub gutter: u16,
}

impl Default for CcArtAtlasOptions {
    fn default() -> Self {
        Self {
            atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
            atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
            gutter: DEFAULT_ATLAS_GUTTER,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CcArtBuildSummary {
    pub slot_count: u32,
    pub populated_slot_count: u32,
    pub page_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtTileKind {
    Land,
    Static,
}

impl ArtTileKind {
    fn slot_flags(self) -> u16 {
        match self {
            Self::Land => SLOT_FLAG_PRESENT | SLOT_FLAG_LAND,
            Self::Static => SLOT_FLAG_PRESENT | SLOT_FLAG_STATIC,
        }
    }
}

#[derive(Debug, Clone)]
struct DecodedArtTile {
    art_id: u32,
    kind: ArtTileKind,
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct CcArtPageRecord {
    page_index: u32,
    tile_count: u32,
    used_width: u32,
    used_height: u32,
}

#[derive(Debug, Clone, Copy)]
struct CcArtSlotRecord {
    art_id: u32,
    page_index: u32,
    page_tile_index: u16,
    flags: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl CcArtSlotRecord {
    fn absent(art_id: u32) -> Self {
        Self {
            art_id,
            page_index: MISSING_PAGE_INDEX,
            page_tile_index: MISSING_PAGE_TILE_INDEX,
            flags: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }

    #[cfg(test)]
    fn is_present(self) -> bool {
        (self.flags & SLOT_FLAG_PRESENT) != 0
    }
}

#[derive(Debug, Clone)]
struct PlacedTile {
    art_id: u32,
    kind: ArtTileKind,
    page_tile_index: u16,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

#[derive(Debug, Clone)]
struct BuiltPage {
    record: CcArtPageRecord,
    pixels: Vec<u8>,
    placed_tiles: Vec<PlacedTile>,
}

pub fn convert_art_mul_to_cc_art_uddp(
    client_dir: &Path,
    out_file: &Path,
    options: &CcArtAtlasOptions,
) -> eyre::Result<CcArtBuildSummary> {
    validate_options(options)?;

    let art_idx_path = client_dir.join("artidx.mul");
    let art_mul_path = client_dir.join("art.mul");
    if !art_idx_path.is_file() {
        eyre::bail!("missing required file: {}", art_idx_path.display());
    }
    if !art_mul_path.is_file() {
        eyre::bail!("missing required file: {}", art_mul_path.display());
    }

    let idx = IndexFile::load(art_idx_path.clone())
        .wrap_err_with(|| format!("load {}", art_idx_path.display()))?;
    let art_map = ArtMap::load(client_dir)
        .wrap_err_with(|| format!("load art sources from {}", client_dir.display()))?;

    let slot_count = idx.element_count() as u32;
    let decoded_tiles = decode_present_tiles(&art_map, &idx)?;
    let populated_slot_count = decoded_tiles.len() as u32;

    let (pages, slot_records) = pack_tiles_into_pages(decoded_tiles, slot_count, options)?;
    let page_manifest = serialize_page_manifest(&pages, options)?;
    let slot_manifest = serialize_slot_manifest(&slot_records, options)?;

    let mut package = UddpPackage::new();
    package.add_typed_entry_from_memory(
        &page_manifest,
        "metadata/pages.bin",
        UddpContentId::Metadata,
        UddpCompression::Zstd,
    )?;
    package.add_typed_entry_from_memory(
        &slot_manifest,
        "metadata/slots.bin",
        UddpContentId::Metadata,
        UddpCompression::Zstd,
    )?;

    for page in &pages {
        let entry_path = page_entry_path(page.record.page_index);
        package.add_typed_entry_from_memory(
            &page.pixels,
            &entry_path,
            UddpContentId::Rgba8888,
            UddpCompression::Zstd,
        )?;
    }

    package
        .save(out_file)
        .wrap_err_with(|| format!("save {}", out_file.display()))?;

    Ok(CcArtBuildSummary {
        slot_count,
        populated_slot_count,
        page_count: pages.len() as u32,
        atlas_width: options.atlas_width,
        atlas_height: options.atlas_height,
    })
}

fn validate_options(options: &CcArtAtlasOptions) -> eyre::Result<()> {
    if options.atlas_width == 0 || options.atlas_height == 0 {
        eyre::bail!("atlas dimensions must be greater than zero");
    }
    if options.atlas_width > u16::MAX as u32 || options.atlas_height > u16::MAX as u32 {
        eyre::bail!("atlas dimensions must fit into metadata u16 fields");
    }
    Ok(())
}

fn decode_present_tiles(art_map: &ArtMap, idx: &IndexFile) -> eyre::Result<Vec<DecodedArtTile>> {
    let mut decoded_tiles = Vec::new();
    let mut scratch_raw = Vec::new();

    for art_id in 0..idx.element_count() as u32 {
        let element = idx
            .element(art_id as usize)
            .wrap_err_with(|| format!("read artidx slot {art_id}"))?;

        let has_payload = match (element.lookup(), element.len()) {
            (Some(_), Some(size)) => size > 0,
            _ => false,
        };
        if !has_payload {
            continue;
        }

        if art_id < 0x4000 {
            let mut rgba = [0u8; 44 * 44 * 4];
            art_map
                .decode_land_tile(art_id, &mut scratch_raw, &mut rgba)
                .wrap_err_with(|| format!("decode land art tile {art_id}"))?;
            decoded_tiles.push(DecodedArtTile {
                art_id,
                kind: ArtTileKind::Land,
                width: 44,
                height: 44,
                rgba: rgba.to_vec(),
            });
        } else {
            let (width, height, rgba) = art_map
                .decode_static_tile(art_id, &mut scratch_raw)
                .wrap_err_with(|| format!("decode static art tile {art_id}"))?;
            decoded_tiles.push(DecodedArtTile {
                art_id,
                kind: ArtTileKind::Static,
                width,
                height,
                rgba,
            });
        }
    }

    decoded_tiles.sort_by(|left, right| {
        let left_area = left.width as u32 * left.height as u32;
        let right_area = right.width as u32 * right.height as u32;
        right_area
            .cmp(&left_area)
            .then_with(|| left.art_id.cmp(&right.art_id))
    });

    Ok(decoded_tiles)
}

fn pack_tiles_into_pages(
    tiles: Vec<DecodedArtTile>,
    slot_count: u32,
    options: &CcArtAtlasOptions,
) -> eyre::Result<(Vec<BuiltPage>, Vec<CcArtSlotRecord>)> {
    let mut pages = Vec::new();
    let mut slot_records = (0..slot_count).map(CcArtSlotRecord::absent).collect::<Vec<_>>();
    let mut remaining = tiles;
    let mut page_index = 0u32;

    while !remaining.is_empty() {
        let (page, leftovers) = build_page(page_index, remaining, options)?;
        if page.placed_tiles.is_empty() {
            eyre::bail!("could not fit any art tile into atlas page {}x{}", options.atlas_width, options.atlas_height);
        }

        for placed in &page.placed_tiles {
            let slot = slot_records
                .get_mut(placed.art_id as usize)
                .context("placed tile art_id outside slot table")?;
            *slot = CcArtSlotRecord {
                art_id: placed.art_id,
                page_index,
                page_tile_index: placed.page_tile_index,
                flags: placed.kind.slot_flags(),
                x: placed.x,
                y: placed.y,
                width: placed.width,
                height: placed.height,
            };
        }

        pages.push(page);
        remaining = leftovers;
        page_index += 1;
    }

    Ok((pages, slot_records))
}

fn build_page(
    page_index: u32,
    tiles: Vec<DecodedArtTile>,
    options: &CcArtAtlasOptions,
) -> eyre::Result<(BuiltPage, Vec<DecodedArtTile>)> {
    let mut allocator = AtlasAllocator::new(size2(
        options.atlas_width as i32,
        options.atlas_height as i32,
    ));
    let mut pixels = vec![0u8; options.atlas_width as usize * options.atlas_height as usize * 4];
    let mut placed_tiles = Vec::new();
    let mut leftovers = Vec::new();
    let mut used_width = 0u32;
    let mut used_height = 0u32;
    let gutter = i32::from(options.gutter);

    for tile in tiles {
        let alloc_width = tile.width as i32 + gutter * 2;
        let alloc_height = tile.height as i32 + gutter * 2;
        if alloc_width > options.atlas_width as i32 || alloc_height > options.atlas_height as i32 {
            eyre::bail!(
                "art tile {} ({}x{}) does not fit into atlas page {}x{} with gutter {}",
                tile.art_id,
                tile.width,
                tile.height,
                options.atlas_width,
                options.atlas_height,
                options.gutter
            );
        }

        if let Some(allocation) = allocator.allocate(size2(alloc_width, alloc_height)) {
            let inner_x = allocation.rectangle.min.x + gutter;
            let inner_y = allocation.rectangle.min.y + gutter;
            blit_rgba_tile(
                &mut pixels,
                options.atlas_width,
                inner_x as u32,
                inner_y as u32,
                tile.width as u32,
                tile.height as u32,
                &tile.rgba,
            )?;

            used_width = used_width.max(inner_x as u32 + tile.width as u32);
            used_height = used_height.max(inner_y as u32 + tile.height as u32);
            placed_tiles.push(PlacedTile {
                art_id: tile.art_id,
                kind: tile.kind,
                page_tile_index: placed_tiles.len() as u16,
                x: inner_x as u16,
                y: inner_y as u16,
                width: tile.width,
                height: tile.height,
            });
        } else {
            leftovers.push(tile);
        }
    }

    let record = CcArtPageRecord {
        page_index,
        tile_count: placed_tiles.len() as u32,
        used_width,
        used_height,
    };

    Ok((
        BuiltPage {
            record,
            pixels,
            placed_tiles,
        },
        leftovers,
    ))
}

fn blit_rgba_tile(
    dst: &mut [u8],
    dst_width: u32,
    dst_x: u32,
    dst_y: u32,
    tile_width: u32,
    tile_height: u32,
    src: &[u8],
) -> eyre::Result<()> {
    let expected_len = tile_width as usize * tile_height as usize * 4;
    if src.len() != expected_len {
        eyre::bail!(
            "invalid RGBA payload length for tile {}x{}: expected {}, got {}",
            tile_width,
            tile_height,
            expected_len,
            src.len()
        );
    }

    let dst_stride = dst_width as usize * 4;
    let src_stride = tile_width as usize * 4;
    for row in 0..tile_height as usize {
        let src_start = row * src_stride;
        let dst_start = ((dst_y as usize + row) * dst_stride) + dst_x as usize * 4;
        let dst_end = dst_start + src_stride;
        dst[dst_start..dst_end].copy_from_slice(&src[src_start..src_start + src_stride]);
    }

    Ok(())
}

fn serialize_page_manifest(pages: &[BuiltPage], options: &CcArtAtlasOptions) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + pages.len() * 16);
    bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(CC_ART_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.write_u32::<LittleEndian>(pages.len() as u32)?;
    for page in pages {
        bytes.write_u32::<LittleEndian>(page.record.page_index)?;
        bytes.write_u32::<LittleEndian>(page.record.tile_count)?;
        bytes.write_u32::<LittleEndian>(page.record.used_width)?;
        bytes.write_u32::<LittleEndian>(page.record.used_height)?;
    }
    Ok(bytes)
}

fn serialize_slot_manifest(
    slots: &[CcArtSlotRecord],
    options: &CcArtAtlasOptions,
) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(24 + slots.len() * 20);
    bytes.extend_from_slice(&SLOT_MANIFEST_MAGIC);
    bytes.write_u32::<LittleEndian>(CC_ART_METADATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(options.atlas_width)?;
    bytes.write_u32::<LittleEndian>(options.atlas_height)?;
    bytes.write_u32::<LittleEndian>(options.gutter as u32)?;
    bytes.write_u32::<LittleEndian>(slots.len() as u32)?;
    for slot in slots {
        bytes.write_u32::<LittleEndian>(slot.art_id)?;
        bytes.write_u32::<LittleEndian>(slot.page_index)?;
        bytes.write_u16::<LittleEndian>(slot.page_tile_index)?;
        bytes.write_u16::<LittleEndian>(slot.flags)?;
        bytes.write_u16::<LittleEndian>(slot.x)?;
        bytes.write_u16::<LittleEndian>(slot.y)?;
        bytes.write_u16::<LittleEndian>(slot.width)?;
        bytes.write_u16::<LittleEndian>(slot.height)?;
    }
    Ok(bytes)
}

fn page_entry_path(page_index: u32) -> String {
    format!("pages/{page_index:05}.rgba8888")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let options = CcArtAtlasOptions {
            atlas_width: 16,
            atlas_height: 16,
            gutter: 1,
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
        let options = CcArtAtlasOptions {
            atlas_width: 8,
            atlas_height: 8,
            gutter: 1,
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
    fn slot_manifest_contains_header_and_all_slots() {
        let options = CcArtAtlasOptions::default();
        let slots = vec![CcArtSlotRecord::absent(0), CcArtSlotRecord::absent(1)];
        let bytes = serialize_slot_manifest(&slots, &options).unwrap();

        assert_eq!(&bytes[..4], b"CASL");
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 2);
        assert_eq!(bytes.len(), 24 + 2 * 20);
    }
}
