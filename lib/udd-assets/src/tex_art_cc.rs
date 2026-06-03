use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::{AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba, read_path_entry, read_path_entry_cow};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"CAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"CASL";
const TEX_ART_CC_METADATA_VERSION: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PagePixelFormat {
    Rgba8888 = 0,
    Bc7 = 1,
}

impl PagePixelFormat {
    pub(crate) fn from_repr(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Rgba8888),
            1 => Some(Self::Bc7),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Rgba8888 => "rgba8888",
            Self::Bc7 => "bc7",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AtlasPackingMode {
    MaximumPacking = 0,
    Bc7Oriented = 1,
}

impl AtlasPackingMode {
    pub(crate) fn from_repr(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::MaximumPacking),
            1 => Some(Self::Bc7Oriented),
            _ => None,
        }
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtCcPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtCcSlotRecord {
    pub art_id: u32,
    pub page_index: u32,
    pub page_tile_index: u16,
    pub flags: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub upscale_factor: u16,
    pub upscale_algorithm: u16,
    pub draw_offset_x: i16,
    pub draw_offset_y: i16,
}

impl TexArtCcSlotRecord {
    pub fn absent(art_id: u32) -> Self {
        Self {
            art_id,
            page_index: MISSING_PAGE_INDEX,
            page_tile_index: MISSING_PAGE_TILE_INDEX,
            flags: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            upscale_factor: 1,
            upscale_algorithm: 0,
            draw_offset_x: 0,
            draw_offset_y: 0,
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

    pub fn logical_width(self) -> f32 {
        self.width as f32 / f32::from(self.upscale_factor.max(1))
    }

    pub fn logical_height(self) -> f32 {
        self.height as f32 / f32::from(self.upscale_factor.max(1))
    }

    pub fn upscale_algorithm_name(self) -> &'static str {
        upscale_algorithm_name(self.upscale_algorithm)
    }
}

pub fn upscale_algorithm_name(code: u16) -> &'static str {
    match code {
        0 => "None",
        1 => "Nearest",
        2 => "Bilinear",
        3 => "CatmullRom",
        4 => "Lanczos3",
        5 => "SuperSai",
        6 => "FsrEasu",
        7 => "FsrEasuRcas",
        8 => "Depixelize",
        9 => "Nedi",
        10 => "TwoSai",
        11 => "SuperEagle",
        12 => "Lq",
        13 => "Hq",
        14 => "HqTrue",
        15 => "Epx",
        16 => "Xbr",
        17 => "Mmpx",
        18 => "SuperXbr",
        _ => "Unknown",
    }
}

pub struct TexArtCcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<TexArtCcPageRecord>,
    slots: Vec<TexArtCcSlotRecord>,
    page_cache: AtlasPageCache,
}

impl TexArtCcPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_with_options(path: impl AsRef<Path>, options: AtlasCacheOptions) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        Self::load_in_memory_with_options(path, AtlasCacheOptions::disabled())
    }

    pub fn load_in_memory_with_options(
        path: impl AsRef<Path>,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package_with_options(package, options)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        Self::from_uddp_package_with_options(package, AtlasCacheOptions::disabled())
    }

    pub fn from_uddp_package_with_options(
        package: UddpReader,
        options: AtlasCacheOptions,
    ) -> eyre::Result<Self> {
        let page_manifest = read_path_entry_cow(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("tex_art_cc.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry_cow(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("tex_art_cc.uddp missing metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slot_packing_mode, slots) =
            parse_slot_manifest(&slot_manifest)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_art_cc metadata headers disagree on atlas dimensions or gutter");
        }
        if page_packing_mode != slot_packing_mode {
            eyre::bail!("tex_art_cc metadata headers disagree on atlas packing mode");
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

    pub fn pages(&self) -> &[TexArtCcPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[TexArtCcSlotRecord] {
        &self.slots
    }

    pub fn atlas_cache_enabled(&self) -> bool {
        self.page_cache.is_enabled()
    }

    pub fn clear_atlas_cache(&self) {
        self.page_cache.clear();
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

    pub fn present_slot(&self, art_id: u32) -> Option<&TexArtCcSlotRecord> {
        self.slots
            .get(art_id as usize)
            .filter(|slot| slot.is_present())
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexArtCcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_ART_CC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?).unwrap();
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        let page_index = cursor.read_u32::<LittleEndian>()?;
        let tile_count = cursor.read_u32::<LittleEndian>()?;
        let used_width = cursor.read_u32::<LittleEndian>()?;
        let used_height = cursor.read_u32::<LittleEndian>()?;
        pages.push(TexArtCcPageRecord {
            page_index,
            tile_count,
            used_width,
            used_height,
            pixel_format,
        });
    }
    Ok((w, h, g, packing_mode, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexArtCcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != TEX_ART_CC_METADATA_VERSION {
        eyre::bail!("invalid version");
    }
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?).unwrap();
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        slots.push(TexArtCcSlotRecord {
            art_id: cursor.read_u32::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_tile_index: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            upscale_factor: cursor.read_u16::<LittleEndian>()?.max(1),
            upscale_algorithm: cursor.read_u16::<LittleEndian>()?,
            draw_offset_x: cursor.read_i16::<LittleEndian>()?,
            draw_offset_y: cursor.read_i16::<LittleEndian>()?,
        });
    }
    Ok((w, h, g, packing_mode, slots))
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}
