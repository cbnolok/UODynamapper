use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use knuffel::Decode;
use udd_container::UddpReader;

use crate::common::{
    decode_atlas_page_rgba, extract_atlas_subrect_rgba, read_path_entry, read_path_entry_cow, AtlasCacheOptions,
    AtlasPageCache,
};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";
pub const ITEM_MANIFEST_ENTRY_PATH: &str = "metadata/items.bin";
pub const SOURCE_HINT_MANIFEST_ENTRY_PATH: &str = "metadata/source_hints.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const ITEM_MANIFEST_MAGIC: [u8; 4] = *b"MEIT";
const SOURCE_HINT_MANIFEST_MAGIC: [u8; 4] = *b"MESH";
const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 2;
const MOBILE_ANIM_EC_METADATA_VERSION_DYNAMIC_PAGES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcPageRecord {
    pub page_index: u32,
    pub frame_count: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcAnimationRecord {
    pub body_id: u32,
    pub action_id: u16,
    pub direction: u8,
    pub frame_start: u32,
    pub frame_count: u16,
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcFrameRecord {
    pub animation_index: u32,
    pub frame_index: u16,
    pub source_frame_index: u16,
    pub page_index: u32,
    pub page_frame_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAnimEcItemRecord {
    pub item_id: i32,
    pub item_type: i16,
    pub layer: i16,
    pub flags: u8,
    pub name: String,
    pub source_hint_start: u32,
    pub source_hint_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcSourceHintRecord {
    pub item_id: i32,
    pub action_id: u16,
    pub uop_index: u8,
    pub block_index: u32,
    pub file_index: u32,
}

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationsKdl {
    #[knuffel(children(name = "item"))]
    pub items: Vec<EcMobileAnimationItemKdl>,
}

impl EcMobileAnimationsKdl {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .wrap_err_with(|| format!("Failed to read KDL file: {:?}", path.as_ref()))?;
        Self::parse(
            path.as_ref().to_str().unwrap_or("EcMobileAnimations.kdl"),
            &content,
        )
    }

    pub fn parse(name: &str, content: &str) -> eyre::Result<Self> {
        knuffel::parse(name, content).wrap_err("Failed to parse EcMobileAnimations KDL")
    }
}

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationItemKdl {
    #[knuffel(argument)]
    pub id: i32,
    #[knuffel(property)]
    pub name: String,
    #[knuffel(property(name = "type"))]
    pub item_type: i16,
    #[knuffel(property)]
    pub layer: i16,
    #[knuffel(property)]
    pub male: Option<bool>,
    #[knuffel(property)]
    pub female: Option<bool>,
    #[knuffel(property)]
    pub gargoyle: Option<bool>,
    #[knuffel(children(name = "anim"))]
    pub animations: Vec<EcMobileAnimationSourceHintKdl>,
}

impl EcMobileAnimationItemKdl {
    pub fn flags(&self) -> u8 {
        u8::from(self.male.unwrap_or(false))
            | (u8::from(self.female.unwrap_or(false)) << 1)
            | (u8::from(self.gargoyle.unwrap_or(false)) << 2)
    }
}

#[derive(Decode, Debug, Clone)]
pub struct EcMobileAnimationSourceHintKdl {
    #[knuffel(argument)]
    pub action_id: u16,
    #[knuffel(property)]
    pub uop: String,
    #[knuffel(property)]
    pub block: u32,
    #[knuffel(property)]
    pub file: u32,
}

pub struct MobileAnimEcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<MobileAnimEcPageRecord>,
    animations: Vec<MobileAnimEcAnimationRecord>,
    frames: Vec<MobileAnimEcFrameRecord>,
    items: Vec<MobileAnimEcItemRecord>,
    source_hints: Vec<MobileAnimEcSourceHintRecord>,
    page_cache: AtlasPageCache,
}

impl MobileAnimEcPackage {
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
        let page_manifest = read_path_entry_cow(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/pages.bin")?;
        let animation_manifest = read_path_entry_cow(&package, ANIMATION_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/animations.bin")?;
        let frame_manifest = read_path_entry_cow(&package, FRAME_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/frames.bin")?;
        let item_manifest = read_path_entry_cow(&package, ITEM_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/items.bin")?;
        let source_hint_manifest = read_path_entry_cow(&package, SOURCE_HINT_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/source_hints.bin")?;

        let (atlas_width, atlas_height, gutter, packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;
        let items = parse_item_manifest(&item_manifest)?;
        let source_hints = parse_source_hint_manifest(&source_hint_manifest)?;

        Ok(Self {
            package,
            atlas_width,
            atlas_height,
            gutter,
            packing_mode,
            pages,
            animations,
            frames,
            items,
            source_hints,
            page_cache: AtlasPageCache::new(options),
        })
    }

    pub fn atlas_width(&self) -> u32 { self.atlas_width }
    pub fn atlas_height(&self) -> u32 { self.atlas_height }
    pub fn gutter(&self) -> u16 { self.gutter }
    pub fn packing_mode(&self) -> AtlasPackingMode { self.packing_mode }
    pub fn pages(&self) -> &[MobileAnimEcPageRecord] { &self.pages }
    pub fn animations(&self) -> &[MobileAnimEcAnimationRecord] { &self.animations }
    pub fn frames(&self) -> &[MobileAnimEcFrameRecord] { &self.frames }
    pub fn items(&self) -> &[MobileAnimEcItemRecord] { &self.items }
    pub fn source_hints(&self) -> &[MobileAnimEcSourceHintRecord] { &self.source_hints }

    pub fn animation(&self, body_id: u32, action_id: u16, direction: u8) -> Option<&MobileAnimEcAnimationRecord> {
        self.animations.iter().find(|animation| {
            animation.body_id == body_id
                && animation.action_id == action_id
                && animation.direction == direction
        })
    }

    pub fn animation_frames(&self, animation: &MobileAnimEcAnimationRecord) -> &[MobileAnimEcFrameRecord] {
        let start = animation.frame_start as usize;
        let end = start + animation.frame_count as usize;
        self.frames.get(start..end).unwrap_or(&[])
    }

    pub fn item(&self, item_id: i32) -> Option<&MobileAnimEcItemRecord> {
        self.items.iter().find(|item| item.item_id == item_id)
    }

    pub fn item_source_hints(&self, item: &MobileAnimEcItemRecord) -> &[MobileAnimEcSourceHintRecord] {
        let start = item.source_hint_start as usize;
        let end = start + item.source_hint_count as usize;
        self.source_hints.get(start..end).unwrap_or(&[])
    }

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .map(|page| page.pixel_format)
            .unwrap_or(PagePixelFormat::Rgba8888);
        self.page_cache.read_page_bytes(page_index, || {
            read_path_entry(&self.package, &page_entry_path(page_index, fmt))
                .wrap_err_with(|| format!("unpack EC mobile animation atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
            .ok_or_else(|| eyre::eyre!("missing EC mobile animation atlas page metadata for {page_index}"))?;
        let page_bytes = self.read_page_bytes(page_index)?;
        decode_atlas_page_rgba(
            &page_bytes,
            page.pixel_format,
            page.atlas_width,
            page.atlas_height,
            page.used_width,
            page.used_height,
        )
    }

    pub fn read_frame_rgba(&self, frame: &MobileAnimEcFrameRecord) -> eyre::Result<Vec<u8>> {
        if frame.page_index == MISSING_PAGE_INDEX || frame.width == 0 || frame.height == 0 {
            return Ok(Vec::new());
        }
        let page = self
            .pages
            .iter()
            .find(|page| page.page_index == frame.page_index)
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing EC mobile animation atlas page metadata for {}",
                    frame.page_index
                )
            })?;
        let page_bytes = self.read_page_bytes(frame.page_index)?;
        extract_atlas_subrect_rgba(
            &page_bytes,
            page.pixel_format,
            page.atlas_width,
            page.atlas_height,
            page.used_width,
            page.used_height,
            frame.x,
            frame.y,
            frame.width,
            frame.height,
        )
    }
}

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<MobileAnimEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation page magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !is_supported_metadata_version(version) { eyre::bail!("invalid EC mobile animation page version"); }
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid EC mobile animation pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid EC mobile animation packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        pages.push(MobileAnimEcPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u32::<LittleEndian>()?,
            atlas_width: if version >= MOBILE_ANIM_EC_METADATA_VERSION_DYNAMIC_PAGES {
                cursor.read_u32::<LittleEndian>()?
            } else {
                width
            },
            atlas_height: if version >= MOBILE_ANIM_EC_METADATA_VERSION_DYNAMIC_PAGES {
                cursor.read_u32::<LittleEndian>()?
            } else {
                height
            },
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((width, height, gutter, packing_mode, pages))
}

fn parse_animation_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcAnimationRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ANIMATION_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation manifest magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !is_supported_metadata_version(version) { eyre::bail!("invalid EC mobile animation manifest version"); }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut animations = Vec::with_capacity(count);
    for _ in 0..count {
        animations.push(MobileAnimEcAnimationRecord {
            body_id: cursor.read_u32::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            direction: cursor.read_u8()?,
            frame_start: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok(animations)
}

fn parse_frame_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcFrameRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != FRAME_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation frame magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !is_supported_metadata_version(version) { eyre::bail!("invalid EC mobile animation frame version"); }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        frames.push(MobileAnimEcFrameRecord {
            animation_index: cursor.read_u32::<LittleEndian>()?,
            frame_index: cursor.read_u16::<LittleEndian>()?,
            source_frame_index: cursor.read_u16::<LittleEndian>()?,
            page_index: cursor.read_u32::<LittleEndian>()?,
            page_frame_index: cursor.read_u16::<LittleEndian>()?,
            x: cursor.read_u16::<LittleEndian>()?,
            y: cursor.read_u16::<LittleEndian>()?,
            width: cursor.read_u16::<LittleEndian>()?,
            height: cursor.read_u16::<LittleEndian>()?,
            center_x: cursor.read_i16::<LittleEndian>()?,
            center_y: cursor.read_i16::<LittleEndian>()?,
        });
    }
    Ok(frames)
}

fn parse_item_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcItemRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ITEM_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation item magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !is_supported_metadata_version(version) { eyre::bail!("invalid EC mobile animation item version"); }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let string_len = cursor.read_u32::<LittleEndian>()? as usize;
    let mut raw_records = Vec::with_capacity(count);
    for _ in 0..count {
        raw_records.push((
            cursor.read_i32::<LittleEndian>()?,
            cursor.read_i16::<LittleEndian>()?,
            cursor.read_i16::<LittleEndian>()?,
            cursor.read_u8()?,
            cursor.read_u32::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
            cursor.read_u32::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
        ));
    }
    let mut strings = vec![0u8; string_len];
    cursor.read_exact(&mut strings)?;
    let mut items = Vec::with_capacity(count);
    for (item_id, item_type, layer, flags, name_start, name_len, source_hint_start, source_hint_count) in raw_records {
        let start = name_start as usize;
        let end = start + name_len as usize;
        let name = std::str::from_utf8(strings.get(start..end).unwrap_or(&[]))
            .unwrap_or("")
            .to_string();
        items.push(MobileAnimEcItemRecord {
            item_id,
            item_type,
            layer,
            flags,
            name,
            source_hint_start,
            source_hint_count,
        });
    }
    Ok(items)
}

fn parse_source_hint_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimEcSourceHintRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != SOURCE_HINT_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation source hint magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if !is_supported_metadata_version(version) { eyre::bail!("invalid EC mobile animation source hint version"); }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut source_hints = Vec::with_capacity(count);
    for _ in 0..count {
        source_hints.push(MobileAnimEcSourceHintRecord {
            item_id: cursor.read_i32::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            uop_index: cursor.read_u8()?,
            block_index: cursor.read_u32::<LittleEndian>()?,
            file_index: cursor.read_u32::<LittleEndian>()?,
        });
    }
    Ok(source_hints)
}

fn is_supported_metadata_version(version: u32) -> bool {
    version == 1 || version == MOBILE_ANIM_EC_METADATA_VERSION
}

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ec_mobile_animations_kdl_asset() {
        let metadata = EcMobileAnimationsKdl::parse(
            "EcMobileAnimations.kdl",
            include_str!("../../../dynamapper/assets/cc_ec_convtables/EcMobileAnimations.kdl"),
        )
        .unwrap();

        let male = metadata.items.iter().find(|item| item.id == 400).unwrap();
        assert_eq!(male.name, "Human Male");
        assert_eq!(male.item_type, 3);
        assert_eq!(male.layer, 0);
        assert_eq!(male.male, Some(true));
        assert!(male.animations.iter().any(|animation| {
            animation.action_id == 0
                && animation.uop == "AnimationFrame1.uop"
                && animation.block == 0
                && animation.file == 1213
        }));
    }
}
