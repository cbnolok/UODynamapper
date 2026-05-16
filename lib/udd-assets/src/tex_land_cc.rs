use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::{AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba, extract_atlas_subrect_rgba, read_path_entry};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CTXP";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CTXS";
const TEX_LAND_CC_METADATA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandCcPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexLandCcSlotRecord {
    pub id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl TexLandCcSlotRecord {
    pub fn absent(id: u32) -> Self {
        Self {
            id,
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
}

pub struct TexLandCcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<TexLandCcPageRecord>,
    slots: Vec<TexLandCcSlotRecord>,
    page_cache: AtlasPageCache,
}

impl TexLandCcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_with_options(path: impl AsRef<Path>, options: AtlasCacheOptions) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        Self::from_uddp_package_with_options(package, AtlasCacheOptions::disabled())
    }

    pub fn from_uddp_package_with_options(
        package: UddpReader,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        let page_manifest = read_path_entry(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("tex_land_cc.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("tex_land_cc.uddp missing metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slot_packing_mode, slots) =
            parse_slot_manifest(&slot_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_land_cc metadata headers disagree on atlas dimensions or gutter");
        }
        if page_packing_mode != slot_packing_mode {
            eyre::bail!("tex_land_cc metadata headers disagree on atlas packing mode");
        }

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            packing_mode: page_packing_mode,
            pages,
            slots,
            page_cache: AtlasPageCache::new(options),
        })
    }

    pub fn package(&self) -> &UddpReader {
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

    pub fn packing_mode(&self) -> AtlasPackingMode {
        self.packing_mode
    }

    pub fn pages(&self) -> &[TexLandCcPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[TexLandCcSlotRecord] {
        &self.slots
    }

    pub fn atlas_cache_enabled(&self) -> bool {
        self.page_cache.is_enabled()
    }

    pub fn clear_atlas_cache(&self) {
        self.page_cache.clear();
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn get_texture_size(&self, id: u32) -> Option<uocf::classic::land_texture::LandTextureSize> {
        let slot = self.present_slot(id)?;
        uocf::classic::land_texture::LandTextureSize::from_dimensions(slot.width as u32, slot.height as u32)
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &page_entry_path(page_index, fmt))
                .wrap_err_with(|| format!("unpack atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .get(page_index as usize)
            .ok_or_else(|| eyre::eyre!("missing atlas page metadata for {page_index}"))?;
        let page_bytes = self.read_page_bytes(page_index)?;
        decode_atlas_page_rgba(
            &page_bytes,
            page.pixel_format,
            self.atlas_width,
            self.atlas_height,
            page.used_width,
            page.used_height,
        )
    }

    pub fn present_slot(&self, id: u32) -> Option<&TexLandCcSlotRecord> {
        self.slots
            .get(id as usize)
            .filter(|slot| slot.is_present())
    }

    pub fn get_pixel_data(&self, id: u32) -> Option<Vec<u8>> {
        let slot = self.present_slot(id)?;
        let page_bytes = self.page_cache.read_page_bytes_arc(slot.page_index, || {
            let fmt = self
                .pages
                .get(slot.page_index as usize)
                .map(|p| p.pixel_format)
                .unwrap_or(PagePixelFormat::Rgba8888);
            read_path_entry(&self.package, &page_entry_path(slot.page_index, fmt))
                .wrap_err_with(|| format!("unpack atlas page {}", slot.page_index))
        }).ok()?;

        let page_record = &self.pages[slot.page_index as usize];
        extract_atlas_subrect_rgba(
            &page_bytes,
            page_record.pixel_format,
            self.atlas_width,
            self.atlas_height,
            page_record.used_width,
            page_record.used_height,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
        )
        .ok()
    }

    pub fn get_pixel_data_arc(&self, id: u32) -> Option<std::sync::Arc<[u8]>> {
        self.get_pixel_data(id).map(|v| v.into())
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexLandCcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_CC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pf = PagePixelFormat::from_repr(cursor.read_u8()?).unwrap();
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        pages.push(TexLandCcPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            tile_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format: pf,
        });
    }
    Ok((w, h, g, packing_mode, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexLandCcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_LAND_CC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        slots.push(TexLandCcSlotRecord {
            id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_tile_index: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok((w, h, g, packing_mode, slots))
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}
