//! The BC7 atlas pages of a mobile animation package, read as the container stores them and as they decode.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use clap::ValueEnum;
use color_eyre::eyre::{self, WrapErr};
use udd_assets::animation::{mobile_anim_cc, mobile_anim_ec};
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_container::{Codec, FileKey, ResolvedFile, UddpReader, UddpReaderOptions, xxh64_virtual_path};

use crate::transforms::BLOCK_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    /// `mobile_anim_cc.uddp`
    Cc,
    /// `mobile_anim_ec.uddp`
    Ec,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub index: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub frames: u32,
    pub entry: ResolvedFile,
}

/// Texels per side of a BC7 block.
pub const BLOCK_SIDE: u32 = 4;

impl Page {
    /// Blocks per row of the page's `raw_len` bytes of blocks: its used extent when the page stores that, else the
    /// atlas (as `udd_assets::common::bc7_payload_extent` reads it).
    pub fn blocks_wide(&self, raw_len: usize) -> usize {
        let used = self.used_width.div_ceil(BLOCK_SIDE) as usize * self.used_height.div_ceil(BLOCK_SIDE) as usize;
        if raw_len / BLOCK_BYTES == used {
            self.used_width.div_ceil(BLOCK_SIDE) as usize
        } else {
            self.atlas_width.div_ceil(BLOCK_SIDE) as usize
        }
    }
}

/// The clips of the package as the renderer loads them: each (body, action, direction) decodes every page its frames
/// lie on, once per clip.
#[derive(Debug, Clone, Default)]
pub struct ClipPages {
    pub clips: usize,
    pub page_reads: usize,
    pub most_pages: usize,
    pub clips_per_page: BTreeMap<u32, usize>,
}

/// A frame record as the package places it: its page and its pixel rectangle there (a block corner), the body whose
/// animation record owns it, and the packer's order of its placement: the frame index (Classic), or the body and the
/// source frame (Enhanced, whose records of one source frame share its placement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    pub order: u64,
    pub page: u32,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub owner_body: u32,
}

impl FrameInfo {
    /// Whether the frame has pixels on a page (placeholders and empty frames have none).
    pub fn placed(&self) -> bool {
        self.page != mobile_anim_cc::MISSING_PAGE_INDEX && self.width > 0 && self.height > 0
    }
}

/// An animation record: the clip the renderer loads, its body and its frames in the frame table.
#[derive(Debug, Clone, Copy)]
pub struct ClipInfo {
    pub body: u32,
    pub frame_start: u32,
    pub frame_count: u32,
}

/// The package as the game reads it, for the clips a body plays.
pub enum Anim {
    Cc(mobile_anim_cc::MobileAnimCcPackage),
    Ec(mobile_anim_ec::MobileAnimEcPackage),
}

pub struct Package {
    pub reader: UddpReader,
    pub anim: Anim,
    pub pages: Vec<Page>,
    pub clip_pages: ClipPages,
    /// Path hashes of the hue masks (Enhanced pages with `PAGE_FLAG_HUE_MASK`).
    pub mask_hashes: BTreeSet<u64>,
    /// Every frame record, in the package's order (the packer's global frame order).
    pub frames: Vec<FrameInfo>,
    /// Every animation record with frames.
    pub clips: Vec<ClipInfo>,
    /// The packer's gutter around each frame, in pixels.
    pub gutter: u16,
}

fn clip_pages<'a>(clips: impl Iterator<Item = (u64, &'a [u32])>) -> ClipPages {
    let mut summary = ClipPages::default();
    let mut by_clip: HashMap<u64, BTreeSet<u32>> = HashMap::new();
    for (clip, pages) in clips {
        by_clip.entry(clip).or_default().extend(pages.iter().copied());
    }
    for pages in by_clip.values() {
        let pages: Vec<u32> = pages.iter().copied().filter(|&page| page != mobile_anim_cc::MISSING_PAGE_INDEX).collect();
        if pages.is_empty() {
            continue;
        }
        summary.clips += 1;
        summary.page_reads += pages.len();
        summary.most_pages = summary.most_pages.max(pages.len());
        for page in pages {
            *summary.clips_per_page.entry(page).or_default() += 1;
        }
    }
    summary
}

impl Package {
    pub fn open(path: &Path, kind: Kind) -> eyre::Result<Self> {
        let reader = UddpReader::load_in_memory_with_options(path, UddpReaderOptions::disabled())
            .map_err(|error| eyre::eyre!("open {}: {error:?}", path.display()))?;
        let mut pages = Vec::new();
        let mut mask_hashes = BTreeSet::new();
        let mut add = |index: u32, format: PagePixelFormat, sizes: [u32; 4], frames: u32| -> eyre::Result<()> {
            eyre::ensure!(format == PagePixelFormat::Bc7, "page {index} is not BC7");
            let path = mobile_anim_cc::page_entry_path(index, format);
            let entry = reader.find_by_path_hash(xxh64_virtual_path(&path))
                .ok_or_else(|| eyre::eyre!("missing {path}"))?;
            let [used_width, used_height, atlas_width, atlas_height] = sizes;
            pages.push(Page { index, used_width, used_height, atlas_width, atlas_height, frames, entry });
            Ok(())
        };
        let mut frame_table = Vec::new();
        let mut clips = Vec::new();
        let (clip_pages, gutter, anim) = match kind {
            Kind::Cc => {
                let package = mobile_anim_cc::MobileAnimCcPackage::load(path).wrap_err("read the Classic package")?;
                for page in package.pages() {
                    add(page.page_index, page.pixel_format,
                        [page.used_width, page.used_height, page.atlas_width, page.atlas_height], page.frame_count)?;
                }
                let frames = package.frames();
                let animations = package.animations();
                frame_table.extend(frames.iter().enumerate().map(|(index, frame)| FrameInfo {
                    order: index as u64,
                    page: frame.page_index, x: frame.x, y: frame.y, width: frame.width, height: frame.height,
                    owner_body: animations.get(frame.animation_index as usize)
                        .map_or(u32::MAX, |animation| u32::from(animation.body_id)),
                }));
                clips.extend(animations.iter().filter(|animation| animation.frame_count > 0).map(|animation| ClipInfo {
                    body: u32::from(animation.body_id), frame_start: animation.frame_start, frame_count: u32::from(animation.frame_count),
                }));
                let pages_of: Vec<(u64, Vec<u32>)> = package.animations().iter().map(|animation| {
                    let start = animation.frame_start as usize;
                    let range = start..start + usize::from(animation.frame_count);
                    let clip = (u64::from(animation.body_id) << 32) | (u64::from(animation.action_id) << 8)
                        | u64::from(animation.direction);
                    (clip, frames[range].iter().map(|frame| frame.page_index).collect())
                }).collect();
                let summary = clip_pages(pages_of.iter().map(|(clip, pages)| (*clip, pages.as_slice())));
                (summary, package.gutter(), Anim::Cc(package))
            }
            Kind::Ec => {
                let package = mobile_anim_ec::MobileAnimEcPackage::load(path).wrap_err("read the Enhanced package")?;
                for page in package.pages() {
                    add(page.page_index, page.pixel_format,
                        [page.used_width, page.used_height, page.atlas_width, page.atlas_height], page.frame_count)?;
                    if page.has_hue_mask() {
                        mask_hashes.insert(xxh64_virtual_path(&mobile_anim_ec::hue_mask_entry_path(page.page_index)));
                    }
                }
                let frames = package.frames();
                let animations = package.animations();
                frame_table.extend(frames.iter().map(|frame| {
                    let owner_body =
                        animations.get(frame.animation_index as usize).map_or(u32::MAX, |animation| animation.body_id);
                    FrameInfo {
                        order: (u64::from(owner_body) << u16::BITS) | u64::from(frame.source_frame_index),
                        page: frame.page_index, x: frame.x, y: frame.y, width: frame.width, height: frame.height,
                        owner_body,
                    }
                }));
                clips.extend(animations.iter().filter(|animation| animation.frame_count > 0).map(|animation| ClipInfo {
                    body: animation.body_id, frame_start: animation.frame_start, frame_count: u32::from(animation.frame_count),
                }));
                let pages_of: Vec<(u64, Vec<u32>)> = package.animations().iter().map(|animation| {
                    let start = animation.frame_start as usize;
                    let range = start..start + usize::from(animation.frame_count);
                    let clip = (u64::from(animation.body_id) << 32) | (u64::from(animation.action_id) << 8)
                        | u64::from(animation.direction);
                    (clip, frames[range].iter().map(|frame| frame.page_index).collect())
                }).collect();
                let summary = clip_pages(pages_of.iter().map(|(clip, pages)| (*clip, pages.as_slice())));
                (summary, package.gutter(), Anim::Ec(package))
            }
        };
        Ok(Self { reader, anim, pages, clip_pages, mask_hashes, frames: frame_table, clips, gutter })
    }

    /// The page with index `index`.
    pub fn page(&self, index: u32) -> Option<&Page> {
        // The packers number the pages in order, so the index is usually the position.
        self.pages.get(index as usize).filter(|page| page.index == index)
            .or_else(|| self.pages.iter().find(|page| page.index == index))
    }

    /// The page's BC7 blocks, decoded by the container.
    pub fn raw(&self, page: &Page) -> eyre::Result<Vec<u8>> {
        let path = mobile_anim_cc::page_entry_path(page.index, PagePixelFormat::Bc7);
        self.reader.read_file_by_path_hash(xxh64_virtual_path(&path))
            .map_err(|error| eyre::eyre!("decode page {}: {error:?}", page.index))
    }

    /// Every entry of the package by kind: (entries, raw bytes, stored bytes) per (kind, codec).
    pub fn entry_summary(&self) -> BTreeMap<(&'static str, String), (usize, u64, u64)> {
        let page_hashes: BTreeSet<u64> = self.pages.iter()
            .map(|page| xxh64_virtual_path(&mobile_anim_cc::page_entry_path(page.index, PagePixelFormat::Bc7)))
            .collect();
        let mut summary: BTreeMap<(&'static str, String), (usize, u64, u64)> = BTreeMap::new();
        for record in self.reader.records() {
            let (kind, entry) = match record.key {
                FileKey::PathHash(hash) => {
                    let kind = if page_hashes.contains(&hash) {
                        "BC7 page"
                    } else if self.mask_hashes.contains(&hash) {
                        "hue mask"
                    } else {
                        "other (metadata, 2x pages)"
                    };
                    (kind, self.reader.find_by_path_hash(hash))
                }
                FileKey::Id(id) => ("other (metadata, 2x pages)", self.reader.find_by_dense_id(id)),
            };
            let Some(entry) = entry else { continue };
            let codec = match entry.codec {
                Codec::None => "none".to_string(),
                Codec::ZstdNoDict => "zstd".to_string(),
                Codec::ZstdTypeDict => "zstd+dict".to_string(),
                Codec::Lz4Hc => "lz4hc".to_string(),
                Codec::Bc7FieldsZstd => "fields+zstd".to_string(),
                Codec::Bc7FieldsLz4Hc => "fields+lz4hc".to_string(),
            };
            let slot = summary.entry((kind, codec)).or_default();
            slot.0 += 1;
            slot.1 += u64::from(entry.raw_size);
            slot.2 += u64::from(entry.stored_size);
        }
        summary
    }

    /// Every `every`-th page, starting at `offset`.
    pub fn sample(&self, every: usize, offset: usize) -> Vec<&Page> {
        self.pages.iter().skip(offset).step_by(every.max(1)).collect()
    }
}
