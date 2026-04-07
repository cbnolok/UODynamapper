//! Build-time and runtime support for `cc_art.uddp`.
//!
//! Package layout:
//! - `pages/{page_index}.rgba8888`: fixed-size atlas page payloads. The page bytes are
//!   raw RGBA8888 pixels compressed with Zstd by the UDDP container.
//! - `metadata/pages.bin`: page table with atlas dimensions and per-page occupancy.
//! - `metadata/slots.bin`: sparse slot table with one record per `art_id`, including
//!   empty slots from `artidx.mul`.

use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
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
const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

pub const DEFAULT_ATLAS_PAGE_WIDTH: u32 = 2048;
pub const DEFAULT_ATLAS_PAGE_HEIGHT: u32 = 2048;
pub const DEFAULT_ATLAS_GUTTER: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcArtPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcArtSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
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

    pub fn is_present(self) -> bool {
        (self.flags & SLOT_FLAG_PRESENT) != 0
    }

    pub fn is_land(self) -> bool {
        (self.flags & SLOT_FLAG_LAND) != 0
    }

    pub fn is_static(self) -> bool {
        (self.flags & SLOT_FLAG_STATIC) != 0
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

#[derive(Debug, Clone)]
pub struct CcArtPackage {
    package: UddpPackage,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    pages: Vec<CcArtPageRecord>,
    slots: Vec<CcArtSlotRecord>,
}

impl CcArtPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpPackage::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpPackage) -> eyre::Result<Self> {
        let page_manifest = package
            .get_entry_by_path(PAGE_MANIFEST_ENTRY_PATH)
            .context("cc_art.uddp missing metadata/pages.bin")?
            .unpack()
            .wrap_err("unpack metadata/pages.bin")?;
        let slot_manifest = package
            .get_entry_by_path(SLOT_MANIFEST_ENTRY_PATH)
            .context("cc_art.uddp missing metadata/slots.bin")?
            .unpack()
            .wrap_err("unpack metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, pages) = parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slots) = parse_slot_manifest(&slot_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("cc_art metadata headers disagree on atlas dimensions or gutter");
        }

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            pages,
            slots,
        })
    }

    pub fn package(&self) -> &UddpPackage {
        &self.package
    }

    pub fn atlas_width(&self) -> u32 {
        self.atlas_width
    }

    pub fn atlas_height(&self) -> u32 {
        self.atlas_height
    }

    pub fn gutter(&self) -> u16 {
        self.gutter
    }

    pub fn pages(&self) -> &[CcArtPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[CcArtSlotRecord] {
        &self.slots
    }

    pub fn slot_record(&self, art_id: u32) -> Option<&CcArtSlotRecord> {
        self.slots.get(art_id as usize)
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&CcArtSlotRecord> {
        self.slot_record(art_id).filter(|slot| slot.is_present())
    }

    pub fn read_page_rgba8888(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        self.package
            .get_entry_by_path(&page_entry_path(page_index))
            .with_context(|| format!("cc_art.uddp missing page {page_index}"))?
            .unpack()
            .wrap_err_with(|| format!("unpack atlas page {page_index}"))
    }
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
        PAGE_MANIFEST_ENTRY_PATH,
        UddpContentId::Metadata,
        UddpCompression::Zstd,
    )?;
    package.add_typed_entry_from_memory(
        &slot_manifest,
        SLOT_MANIFEST_ENTRY_PATH,
        UddpContentId::Metadata,
        UddpCompression::Zstd,
    )?;

    for page in &pages {
        package.add_typed_entry_from_memory(
            &page.pixels,
            &page_entry_path(page.record.page_index),
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
            eyre::bail!(
                "could not fit any art tile into atlas page {}x{}",
                options.atlas_width,
                options.atlas_height
            );
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

    Ok((
        BuiltPage {
            record: CcArtPageRecord {
                page_index,
                tile_count: placed_tiles.len() as u32,
                used_width,
                used_height,
            },
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

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<CcArtPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC {
        eyre::bail!("invalid cc_art page manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CC_ART_METADATA_VERSION {
        eyre::bail!("unsupported cc_art page manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let page_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        pages.push(CcArtPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            tile_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok((atlas_width, atlas_height, gutter, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, Vec<CcArtSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC {
        eyre::bail!("invalid cc_art slot manifest magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CC_ART_METADATA_VERSION {
        eyre::bail!("unsupported cc_art slot manifest version {version}");
    }
    let atlas_width = cursor.read_u32::<LittleEndian>()?;
    let atlas_height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let slot_count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        slots.push(CcArtSlotRecord {
            art_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_tile_index: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok((atlas_width, atlas_height, gutter, slots))
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
    fn runtime_reader_can_unpack_page_and_slot_metadata() {
        let options = CcArtAtlasOptions {
            atlas_width: 8,
            atlas_height: 8,
            gutter: 1,
        };
        let tiles = vec![rgba_tile(0, ArtTileKind::Land, 4, 4)];
        let (pages, slots) = pack_tiles_into_pages(tiles, 1, &options).unwrap();
        let page_manifest = serialize_page_manifest(&pages, &options).unwrap();
        let slot_manifest = serialize_slot_manifest(&slots, &options).unwrap();

        let mut package = UddpPackage::new();
        package
            .add_typed_entry_from_memory(
                &page_manifest,
                PAGE_MANIFEST_ENTRY_PATH,
                UddpContentId::Metadata,
                UddpCompression::Zstd,
            )
            .unwrap();
        package
            .add_typed_entry_from_memory(
                &slot_manifest,
                SLOT_MANIFEST_ENTRY_PATH,
                UddpContentId::Metadata,
                UddpCompression::Zstd,
            )
            .unwrap();
        package
            .add_typed_entry_from_memory(
                &pages[0].pixels,
                &page_entry_path(0),
                UddpContentId::Rgba8888,
                UddpCompression::Zstd,
            )
            .unwrap();

        let package = CcArtPackage::from_uddp_package(package).unwrap();
        assert_eq!(package.pages().len(), 1);
        assert!(package.present_slot(0).unwrap().is_land());
        assert_eq!(package.read_page_rgba8888(0).unwrap().len(), 8 * 8 * 4);
    }
}
