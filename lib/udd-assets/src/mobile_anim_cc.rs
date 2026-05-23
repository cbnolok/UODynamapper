use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};
use udd_container::UddpReader;

use crate::common::{read_path_entry, AtlasCacheOptions, AtlasPageCache, decode_atlas_page_rgba};
use crate::tex_art_cc::{AtlasPackingMode, PagePixelFormat};

pub const PAGE_MANIFEST_ENTRY_PATH: &str = "metadata/pages.bin";
pub const ANIMATION_MANIFEST_ENTRY_PATH: &str = "metadata/animations.bin";
pub const FRAME_MANIFEST_ENTRY_PATH: &str = "metadata/frames.bin";

pub const MISSING_PAGE_INDEX: u32 = u32::MAX;
pub const MISSING_PAGE_FRAME_INDEX: u16 = u16::MAX;

const PAGE_MANIFEST_MAGIC: [u8; 4] = *b"MAPG";
const ANIMATION_MANIFEST_MAGIC: [u8; 4] = *b"MAAN";
const FRAME_MANIFEST_MAGIC: [u8; 4] = *b"MAFR";
const MOBILE_ANIM_CC_METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcPageRecord {
    pub page_index: u32,
    pub frame_count: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: PagePixelFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcAnimationRecord {
    pub body_id: u16,
    pub action_id: u16,
    pub direction: u8,
    pub file_index: u8,
    pub source_index: u32,
    pub frame_start: u32,
    pub frame_count: u16,
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MobileAnimCcFrameRecord {
    pub animation_index: u32,
    pub frame_index: u16,
    pub page_index: u32,
    pub page_frame_index: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub center_x: i16,
    pub center_y: i16,
}

pub struct MobileAnimCcPackage {
    package: UddpReader,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    packing_mode: AtlasPackingMode,
    pages: Vec<MobileAnimCcPageRecord>,
    animations: Vec<MobileAnimCcAnimationRecord>,
    frames: Vec<MobileAnimCcFrameRecord>,
    page_cache: AtlasPageCache,
}

impl MobileAnimCcPackage {
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
            .context("mobile_anim_cc.uddp missing metadata/pages.bin")?;
        let animation_manifest = read_path_entry(&package, ANIMATION_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/animations.bin")?;
        let frame_manifest = read_path_entry(&package, FRAME_MANIFEST_ENTRY_PATH)
            .context("mobile_anim_cc.uddp missing metadata/frames.bin")?;

        let (page_width, page_height, page_gutter, page_packing_mode, pages) =
            parse_page_manifest(&page_manifest)?;
        let animations = parse_animation_manifest(&animation_manifest)?;
        let frames = parse_frame_manifest(&frame_manifest)?;

        Ok(Self {
            package,
            atlas_width: page_width,
            atlas_height: page_height,
            gutter: page_gutter,
            packing_mode: page_packing_mode,
            pages,
            animations,
            frames,
            page_cache: AtlasPageCache::new(options),
        })
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

    pub fn pages(&self) -> &[MobileAnimCcPageRecord] {
        &self.pages
    }

    pub fn animations(&self) -> &[MobileAnimCcAnimationRecord] {
        &self.animations
    }

    pub fn frames(&self) -> &[MobileAnimCcFrameRecord] {
        &self.frames
    }

    pub fn animation(&self, body_id: u16, action_id: u16, direction: u8) -> Option<&MobileAnimCcAnimationRecord> {
        self.animations.iter().find(|animation| {
            animation.body_id == body_id
                && animation.action_id == action_id
                && animation.direction == direction
        })
    }

    pub fn animation_frames(&self, animation: &MobileAnimCcAnimationRecord) -> &[MobileAnimCcFrameRecord] {
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
                .wrap_err_with(|| format!("unpack mobile animation atlas page {page_index}"))
        })
    }

    pub fn read_page_rgba(&self, page_index: u32) -> eyre::Result<Vec<u8>> {
        let page = self
            .pages
            .get(page_index as usize)
            .ok_or_else(|| eyre::eyre!("missing mobile animation atlas page metadata for {page_index}"))?;
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

fn parse_page_manifest(bytes: &[u8]) -> eyre::Result<(u32, u32, u16, AtlasPackingMode, Vec<MobileAnimCcPageRecord>)> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != PAGE_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation page magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != MOBILE_ANIM_CC_METADATA_VERSION {
        eyre::bail!("invalid mobile animation page manifest version");
    }
    let width = cursor.read_u32::<LittleEndian>()?;
    let height = cursor.read_u32::<LittleEndian>()?;
    let gutter = cursor.read_u32::<LittleEndian>()? as u16;
    let pixel_format = PagePixelFormat::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid mobile animation page pixel format"))?;
    let packing_mode = AtlasPackingMode::from_repr(cursor.read_u8()?)
        .ok_or_else(|| eyre::eyre!("invalid mobile animation atlas packing mode"))?;
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut pages = Vec::with_capacity(count);
    for _ in 0..count {
        pages.push(MobileAnimCcPageRecord {
            page_index: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u32::<LittleEndian>()?,
            used_width: cursor.read_u32::<LittleEndian>()?,
            used_height: cursor.read_u32::<LittleEndian>()?,
            pixel_format,
        });
    }
    Ok((width, height, gutter, packing_mode, pages))
}

fn parse_animation_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcAnimationRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ANIMATION_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation manifest magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != MOBILE_ANIM_CC_METADATA_VERSION {
        eyre::bail!("invalid mobile animation manifest version");
    }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut animations = Vec::with_capacity(count);
    for _ in 0..count {
        animations.push(MobileAnimCcAnimationRecord {
            body_id: cursor.read_u16::<LittleEndian>()?,
            action_id: cursor.read_u16::<LittleEndian>()?,
            direction: cursor.read_u8()?,
            file_index: cursor.read_u8()?,
            source_index: cursor.read_u32::<LittleEndian>()?,
            frame_start: cursor.read_u32::<LittleEndian>()?,
            frame_count: cursor.read_u16::<LittleEndian>()?,
            flags: cursor.read_u16::<LittleEndian>()?,
        });
    }
    Ok(animations)
}

fn parse_frame_manifest(bytes: &[u8]) -> eyre::Result<Vec<MobileAnimCcFrameRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != FRAME_MANIFEST_MAGIC { eyre::bail!("invalid mobile animation frame magic"); }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != MOBILE_ANIM_CC_METADATA_VERSION {
        eyre::bail!("invalid mobile animation frame manifest version");
    }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        frames.push(MobileAnimCcFrameRecord {
            animation_index: cursor.read_u32::<LittleEndian>()?,
            frame_index: cursor.read_u16::<LittleEndian>()?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};

    #[test]
    fn package_reader_loads_metadata_and_page_payload() {
        let page_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&PAGE_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 8).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            bytes.push(PagePixelFormat::Rgba8888 as u8);
            bytes.push(AtlasPackingMode::Bc7Oriented as u8);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 4).unwrap();
            bytes
        };
        let animation_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&ANIMATION_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 3).unwrap();
            bytes.push(4);
            bytes.push(0);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 20).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            bytes
        };
        let frame_manifest = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&FRAME_MANIFEST_MAGIC);
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, MOBILE_ANIM_CC_METADATA_VERSION).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u32::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 0).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 4).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 4).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_u16::<LittleEndian>(&mut bytes, 2).unwrap();
            byteorder::WriteBytesExt::write_i16::<LittleEndian>(&mut bytes, 1).unwrap();
            byteorder::WriteBytesExt::write_i16::<LittleEndian>(&mut bytes, -1).unwrap();
            bytes
        };
        let page = vec![255u8; 4 * 4 * 4];
        let page_path = page_entry_path(0, PagePixelFormat::Rgba8888);
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        for (path, data, data_type) in [
            (PAGE_MANIFEST_ENTRY_PATH, page_manifest.as_slice(), DataType::Metadata),
            (ANIMATION_MANIFEST_ENTRY_PATH, animation_manifest.as_slice(), DataType::Metadata),
            (FRAME_MANIFEST_ENTRY_PATH, frame_manifest.as_slice(), DataType::Metadata),
            (page_path.as_str(), page.as_slice(), DataType::Texture),
        ] {
            builder.add_file(AddFileRequest {
                data_type: data_type as u8,
                compression: CompressionFlag::ZstdNoDict,
                width: 0,
                height: 0,
                virtual_path: Some(path),
                path_hash64: None,
                id: None,
                data,
            }).unwrap();
        }

        let package = MobileAnimCcPackage::from_uddp_package(
            UddpReader::open(builder.build().unwrap()).unwrap(),
        ).unwrap();

        assert_eq!(package.pages().len(), 1);
        assert_eq!(package.animation(2, 3, 4).unwrap().source_index, 20);
        assert_eq!(package.animation_frames(package.animation(2, 3, 4).unwrap()).len(), 1);
        assert_eq!(package.read_page_bytes(0).unwrap().len(), page.len());
    }
}
