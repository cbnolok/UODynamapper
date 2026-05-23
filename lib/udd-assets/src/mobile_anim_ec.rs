use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::common::{decode_atlas_page_rgba, read_path_entry, AtlasCacheOptions, AtlasPageCache};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MEPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MEAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MEFR";
const MOBILE_ANIM_EC_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimEcPageRecord {
    pub page_index: u32,
    pub frame_count: u32,
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

pub struct MobileAnimEcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<MobileAnimEcPageRecord>,
    animations: Vec<MobileAnimEcAnimationRecord>,
    frames: Vec<MobileAnimEcFrameRecord>,
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
        let page_manifest = read_path_entry(&package, PAGE_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/pages.bin")?;
        let animation_manifest = read_path_entry(&package, ANIMATION_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/animations.bin")?;
        let frame_manifest = read_path_entry(&package, FRAME_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_ec.uddp missing metadata/frames.bin")?;

        let (atlas_width, atlas_height, gutter, packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;

        Ok(Self {
            package,
            atlas_width,
            atlas_height,
            gutter,
            packing_mode,
            pages,
            animations,
            frames,
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

    pub fn read_page_bytes(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let fmt = self
            .pages
            .get(page_index as usize)
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
            .get(page_index as usize)
            .ok_or_else(|| eyre::eyre!("missing EC mobile animation atlas page metadata for {page_index}"))?;
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

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<MobileAnimEcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid EC mobile animation page magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != MOBILE_ANIM_EC_METADATA_VERSION { eyre::bail!("invalid EC mobile animation page version"); }
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
    if version != MOBILE_ANIM_EC_METADATA_VERSION { eyre::bail!("invalid EC mobile animation manifest version"); }
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
    if version != MOBILE_ANIM_EC_METADATA_VERSION { eyre::bail!("invalid EC mobile animation frame version"); }
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

pub fn page_entry_path(page_index: u32, fmt: PagePixelFormat) -> String {
    format!("pages/{page_index:05}.{}", fmt.extension())
}
