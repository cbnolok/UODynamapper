use std::io::{Cursor, Read};
use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use byteorder::{LittleEndian, ReadBytesExt};
use udd_container::UddpReader;
use crate::common::{
    AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba, ensure_manifest_version, read_path_entry, read_path_entry_cow,
};
use crate::tex_art_cc::{upscale_algorithm_name, AtlasPackingMode, PagePixelFormat};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const SLOT_MANIFEST_ENTRY_PATH: &str = "metadata/slots.bin";
pub const SPRITE_GEOMETRY_ENTRY_PATH: &str = "metadata/sprite_geometry.json";

pub const SLOT_FLAG_PRESENT: u16 = 1 << 0;
pub const SLOT_FLAG_LAND: u16 = 1 << 1;
pub const SLOT_FLAG_STATIC: u16 = 1 << 2;
pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_TILE_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"EAPG";
const SLOT_MANIFEST_MAGIC: [u8; 4] = *b"EASL";
/// Bump version when the binary layout of either manifest changes.
/// Required source-geometry metadata is also part of this package contract.
pub const TEX_ART_EC_METADATA_VERSION: u32 = 1;

/// The original window builder's legacy pixel unit, against which the descriptor scales are given.
pub const LEGACY_PIXEL_UNIT: f32 = 1.5;

/// Source geometry of a packed sprite. Its source is always the block-0 WorldArt image.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TexArtEcSpriteGeometry {
    pub texture_id: u32,
    pub descriptor: uocf::enhanced::tile_database::ArtSpriteDescriptor,
}

impl TexArtEcSpriteGeometry {
    /// The WorldArt scale over the legacy pixel unit (1.5) the original window builder uses.
    pub fn legacy_pixel_ratio(&self) -> f32 {
        self.descriptor.world_art_scale / LEGACY_PIXEL_UNIT
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TexArtEcCropAdjustment {
    pub source_left: i16,
    pub source_top: i16,
    pub trim_left: i16,
    pub trim_top: i16,
    pub trim_right: i16,
    pub trim_bottom: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtEcPageRecord {
    pub page_index: u32,
    pub tile_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexArtEcSlotRecord {
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

impl TexArtEcSlotRecord {
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

pub struct TexArtEcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<TexArtEcPageRecord>,
    slots: Vec<TexArtEcSlotRecord>,
    sprite_geometry: std::collections::BTreeMap<u32, TexArtEcSpriteGeometry>,
    page_cache: AtlasPageCache,
}

impl TexArtEcPackage {
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
            .context("tex_art_ec.uddp missing metadata/pages.bin")?;
        let slot_manifest = read_path_entry_cow(&package, SLOT_MANIFEST_ENTRY_PATH)
            .context("tex_art_ec.uddp missing metadata/slots.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let (slot_width, slot_height, slot_gutter, slot_packing_mode, slots) =
            parse_slot_manifest(&slot_manifest)?;
        let geometry_bytes = read_path_entry_cow(&package, SPRITE_GEOMETRY_ENTRY_PATH)
            .context("tex_art_ec missing source sprite geometry; rebuild tex_art_ec.uddp")?;
        let sprite_geometry = parse_sprite_geometry(&geometry_bytes, &slots)?;

        if (page_width, page_height, page_gutter) != (slot_width, slot_height, slot_gutter) {
            eyre::bail!("tex_art_ec metadata headers disagree on atlas dimensions or gutter");
        }
        if page_packing_mode != slot_packing_mode {
            eyre::bail!("tex_art_ec metadata headers disagree on atlas packing mode");
        }

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            packing_mode: page_packing_mode,
            pages,
            slots,
            sprite_geometry,
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

    pub fn pages(&self) -> &[TexArtEcPageRecord] {
        &self.pages
    }

    pub fn slots(&self) -> &[TexArtEcSlotRecord] {
        &self.slots
    }

    pub fn sprite_geometry(&self, art_id: u32) -> Option<&TexArtEcSpriteGeometry> {
        self.sprite_geometry.get(&art_id)
    }

    pub fn atlas_cache_enabled(&self) -> bool {
        self.page_cache.is_enabled()
    }

    pub fn clear_atlas_cache(&self) {
        self.page_cache.clear();
    }

    pub fn slot_record(&self, art_id: u32) -> Option<&TexArtEcSlotRecord> {
        self.slots.get(art_id as usize)
    }

    pub fn present_slot(&self, art_id: u32) -> Option<&TexArtEcSlotRecord> {
        self.slot_record(art_id).filter(|slot| slot.is_present())
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
            .map(|p| p.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &crate::tex_art_cc::page_entry_path(page_index, fmt))
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
}

fn parse_sprite_geometry(bytes: &[u8], slots: &[TexArtEcSlotRecord])
    -> eyre::Result<std::collections::BTreeMap<u32, TexArtEcSpriteGeometry>> {
    let records: std::collections::BTreeMap<u32, TexArtEcSpriteGeometry> = serde_json::from_slice(bytes)
        .context("parse EC sprite geometry")?;
    for slot in slots.iter().filter(|slot| slot.is_present()) {
        let geometry = records.get(&slot.art_id)
            .ok_or_else(|| eyre::eyre!("art {} missing source sprite geometry; rebuild tex_art_ec.uddp", slot.art_id))?;
        let ratio = geometry.legacy_pixel_ratio();
        if !ratio.is_finite() || ratio <= 0.0 {
            eyre::bail!("art {} has invalid authored sprite scale", slot.art_id);
        }
    }
    Ok(records)
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexArtEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("tex_art_ec page manifest", version, TEX_ART_EC_METADATA_VERSION)?;
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_art_ec page pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_art_ec atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        let page_index = cursor.read_u32::<LittleEndian>()?;
        let tile_count = cursor.read_u32::<LittleEndian>()?;
        let used_width = cursor.read_u32::<LittleEndian>()?;
        let used_height = cursor.read_u32::<LittleEndian>()?;
        pages.push(TexArtEcPageRecord {
            page_index,
            tile_count,
            used_width,
            used_height,
            pixel_format,
        });
    }
    Ok((w, h, g, packing_mode, pages))
}

fn parse_slot_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<TexArtEcSlotRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SLOT_MANIFEST_MAGIC { eyre::bail!("invalid magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    ensure_manifest_version("tex_art_ec slot manifest", version, TEX_ART_EC_METADATA_VERSION)?;
    let w = cursor.read_u32::<LittleEndian>()?;
    let h = cursor.read_u32::<LittleEndian>()?;
    let g = cursor.read_u32::<LittleEndian>()? as u16;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid tex_art_ec atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        slots.push(TexArtEcSlotRecord {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A pixel format byte this reader doesn't know is an error, not a panic.
    #[test]
    fn unknown_page_pixel_format_is_an_error() {
        const UNKNOWN_PIXEL_FORMAT: u8 = u8::MAX;
        const NO_PAGES: u32 = 0;
        let mut bytes = PAGE_MANIFEST_MAGIC.to_vec();
        bytes.extend_from_slice(&TEX_ART_EC_METADATA_VERSION.to_le_bytes());
        for width_height_gutter in [0u32; 3] {
            bytes.extend_from_slice(&width_height_gutter.to_le_bytes());
        }
        bytes.push(UNKNOWN_PIXEL_FORMAT);
        bytes.push(AtlasPackingMode::MaximumPacking as u8);
        bytes.extend_from_slice(&NO_PAGES.to_le_bytes());

        let error = parse_page_manifest(&bytes).unwrap_err();
        assert!(error.to_string().contains("pixel format"), "{error}");
    }

    #[test]
    fn present_sprites_require_complete_positive_source_geometry() {
        let mut slot = TexArtEcSlotRecord::absent(77);
        slot.flags = SLOT_FLAG_PRESENT;
        assert!(parse_sprite_geometry(b"{}", &[slot]).is_err());
        let mut manifest = serde_json::json!({"77":{
            "texture_id":123, "descriptor":{
                "source_tile_id":77, "world_art_scale":0.8, "legacy_art_scale":1.5,
                "world_window":[0,0,100,200,-50,0], "legacy_window":[0,0,44,44,-22,0],
            }
        }});
        let records = parse_sprite_geometry(&serde_json::to_vec(&manifest).unwrap(), &[slot]).unwrap();
        assert!((records[&77].legacy_pixel_ratio() - 0.8 / 1.5).abs() < 0.000001);
        for scale in [0.0, -0.8] {
            manifest["77"]["descriptor"]["world_art_scale"] = serde_json::json!(scale);
            assert!(parse_sprite_geometry(&serde_json::to_vec(&manifest).unwrap(), &[slot]).is_err());
        }
        manifest["77"]["descriptor"].as_object_mut().unwrap().remove("world_art_scale");
        assert!(parse_sprite_geometry(&serde_json::to_vec(&manifest).unwrap(), &[slot]).is_err());
    }
}
