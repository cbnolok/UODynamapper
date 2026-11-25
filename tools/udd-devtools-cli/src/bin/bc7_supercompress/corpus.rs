//! The BC7 atlas pages of a mobile animation package, read as the container stores them and as they decode.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use clap::ValueEnum;
use color_eyre::eyre::{self, WrapErr};
use udd_assets::animation::{mobile_anim_cc, mobile_anim_ec};
use udd_assets::tex_art_cc::PagePixelFormat;
use udd_container::{Codec, FileKey, ResolvedFile, UddpReader, UddpReaderOptions, xxh64_virtual_path};

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

/// The clips of the package as the renderer loads them: each (body, action, direction) decodes every page its frames
/// lie on, once per clip.
#[derive(Debug, Clone, Default)]
pub struct ClipPages {
    pub clips: usize,
    pub page_reads: usize,
    pub most_pages: usize,
    pub clips_per_page: BTreeMap<u32, usize>,
}

pub struct Package {
    pub reader: UddpReader,
    pub pages: Vec<Page>,
    pub clip_pages: ClipPages,
    /// Path hashes of the hue masks (Enhanced pages with `PAGE_FLAG_HUE_MASK`).
    pub mask_hashes: BTreeSet<u64>,
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
        let clip_pages = match kind {
            Kind::Cc => {
                let package = mobile_anim_cc::MobileAnimCcPackage::load(path).wrap_err("read the Classic package")?;
                for page in package.pages() {
                    add(page.page_index, page.pixel_format,
                        [page.used_width, page.used_height, page.atlas_width, page.atlas_height], page.frame_count)?;
                }
                let frames = package.frames();
                let pages_of: Vec<(u64, Vec<u32>)> = package.animations().iter().map(|animation| {
                    let start = animation.frame_start as usize;
                    let range = start..start + usize::from(animation.frame_count);
                    let clip = (u64::from(animation.body_id) << 32) | (u64::from(animation.action_id) << 8)
                        | u64::from(animation.direction);
                    (clip, frames[range].iter().map(|frame| frame.page_index).collect())
                }).collect();
                clip_pages(pages_of.iter().map(|(clip, pages)| (*clip, pages.as_slice())))
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
                let pages_of: Vec<(u64, Vec<u32>)> = package.animations().iter().map(|animation| {
                    let start = animation.frame_start as usize;
                    let range = start..start + usize::from(animation.frame_count);
                    let clip = (u64::from(animation.body_id) << 32) | (u64::from(animation.action_id) << 8)
                        | u64::from(animation.direction);
                    (clip, frames[range].iter().map(|frame| frame.page_index).collect())
                }).collect();
                clip_pages(pages_of.iter().map(|(clip, pages)| (*clip, pages.as_slice())))
            }
        };
        Ok(Self { reader, pages, clip_pages, mask_hashes })
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
