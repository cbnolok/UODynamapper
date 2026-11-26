//! The mobile animation packer's page layout, replayed on a chosen list of frames: which frames go on each page, at
//! which bucket size and where, as `udd_conv::animation::mobile_anim_cc::pack_planned_frames_into_package` decides it
//! (`select_planned_page_bucket`, `longest_fitting_prefix`, `build_planned_page`). The packer encodes each 4 × 4
//! block of a page on its own and each allocation holds only its frame and the frame's extruded gutter, so a page laid
//! out here is built from the blocks of the stored pages, copied slot by slot, and equals what the packer would encode
//! (`check` measures that against the package's own layout).

use std::cmp::Reverse;
use std::collections::HashMap;

use guillotiere::{AtlasAllocator, size2};
use rayon::prelude::*;

use crate::corpus::{BLOCK_SIDE, FrameInfo};
use crate::transforms::BLOCK_BYTES;

/// The page sizes the packer tries (`atlas_page_buckets`), before sorting by area; all fit the 2048 × 2048 atlas.
const PAGE_BUCKETS: [(u32, u32); 7] =
    [(512, 512), (1024, 512), (512, 1024), (1024, 1024), (2048, 1024), (1024, 2048), (2048, 2048)];
/// Growth of the prefix search's doubling probes (`prefix_search.rs`, `GROWTH`).
const PREFIX_GROWTH: usize = 2;

/// A frame to place: its index in the package's frame table, the packer's order of it (`FrameInfo::order`) and its
/// content size in pixels.
#[derive(Debug, Clone, Copy)]
pub struct SlotFrame {
    pub frame: usize,
    pub key: u64,
    pub width: u16,
    pub height: u16,
}

/// A frame placed on a planned page: its content corner in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedFrame {
    pub frame: usize,
    pub x: u32,
    pub y: u32,
}

/// A page as the packer would lay it out: the used extent (the right and bottom edges of its allocations) and its
/// frames.
#[derive(Debug, Clone)]
pub struct PlannedPage {
    pub used_width: u32,
    pub used_height: u32,
    pub frames: Vec<PlacedFrame>,
}

impl PlannedPage {
    pub fn blocks_wide(&self) -> usize {
        (self.used_width / BLOCK_SIDE) as usize
    }

    pub fn blocks_high(&self) -> usize {
        (self.used_height / BLOCK_SIDE) as usize
    }
}

fn align_block(extent: u32) -> u32 {
    extent.next_multiple_of(BLOCK_SIDE)
}

/// The allocation of a side of `extent` pixels with a gutter of `gutter` pixels (`resolve_packing_axis`,
/// `Bc7Oriented`): the content padded to blocks, and the gutter padded to blocks on both sides.
fn alloc_extent(extent: u16, gutter: u16) -> u32 {
    align_block(u32::from(extent)) + 2 * align_block(u32::from(gutter))
}

#[derive(Clone, Copy)]
struct Candidate {
    frame: usize,
    key: u64,
    width: u32,
    height: u32,
}

impl Candidate {
    fn area(&self) -> u32 {
        self.width * self.height
    }
}

/// The packer's placement order on a page: the largest allocations first, then in the packer's order.
fn sorted(candidates: &[Candidate]) -> Vec<Candidate> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by_key(|candidate| (Reverse(candidate.area()), candidate.key));
    sorted
}

/// Places `candidates` on a page of `bucket` in the packer's order; the allocations' corners, or none when one does not
/// fit.
fn place(candidates: &[Candidate], bucket: (u32, u32)) -> Option<Vec<(Candidate, i32, i32)>> {
    let mut allocator = AtlasAllocator::new(size2(bucket.0 as i32, bucket.1 as i32));
    sorted(candidates).into_iter().map(|candidate| {
        let allocation = allocator.allocate(size2(candidate.width as i32, candidate.height as i32))?;
        Some((candidate, allocation.rectangle.min.x, allocation.rectangle.min.y))
    }).collect()
}

/// The packer's `longest_fitting_prefix` without its speculation (which gives the same result): doubling from 1 while
/// the prefix fits and is not the whole list, then bisecting between the last prefix that fit and the first that did
/// not. `fits` need not be monotone, which is why the search order matters.
fn longest_fitting_prefix(len: usize, fits: impl Fn(usize) -> bool) -> usize {
    let mut best = 0;
    let mut high = 1;
    while high <= len {
        if !fits(high) {
            break;
        }
        best = high;
        if high == len {
            return best;
        }
        high = (high * PREFIX_GROWTH).min(len);
    }
    if best == 0 {
        return 0;
    }
    let (mut low, mut high) = (best + 1, high - 1);
    while low <= high {
        let mid = low + (high - low) / 2;
        if fits(mid) {
            best = mid;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    best
}

/// Lays out `frames` (in any order: the packer sorts them by their key) on pages as the packer would, with a
/// `gutter`-pixel gutter. Errors when a frame fits no page.
pub fn plan(frames: &[SlotFrame], gutter: u16) -> Result<Vec<PlannedPage>, String> {
    let mut remaining: Vec<Candidate> = frames.iter().map(|frame| Candidate {
        frame: frame.frame,
        key: frame.key,
        width: alloc_extent(frame.width, gutter),
        height: alloc_extent(frame.height, gutter),
    }).collect();
    remaining.sort_by_key(|candidate| candidate.key);
    let mut buckets = PAGE_BUCKETS.to_vec();
    buckets.sort_by_key(|&(width, height)| width * height);
    let leading = align_block(u32::from(gutter));
    let mut pages = Vec::new();
    let mut start = 0;
    while start < remaining.len() {
        let rest = &remaining[start..];
        // Each bucket's longest prefix (it ends at the first frame the bucket cannot hold) and its allocated area.
        let results: Vec<Option<((u32, u32), usize, u64)>> = buckets.par_iter().map(|&bucket| {
            let usable = rest.iter().position(|candidate| candidate.width > bucket.0 || candidate.height > bucket.1)
                .unwrap_or(rest.len());
            let len = longest_fitting_prefix(usable, |count| place(&rest[..count], bucket).is_some());
            (len > 0).then(|| (bucket, len, rest[..len].iter().map(|candidate| u64::from(candidate.area())).sum()))
        }).collect();
        let mut best: Option<((u32, u32), usize, u64)> = None;
        for (bucket, len, area) in results.into_iter().flatten() {
            let page_area = u64::from(bucket.0 * bucket.1);
            let replace = best.is_none_or(|(best_bucket, best_len, best_area)| {
                let best_page_area = u64::from(best_bucket.0 * best_bucket.1);
                area * best_page_area > best_area * page_area
                    || (area * best_page_area == best_area * page_area
                        && (len > best_len || (len == best_len && page_area < best_page_area)))
            });
            if replace {
                best = Some((bucket, len, area));
            }
        }
        let (bucket, len, _) = best.ok_or_else(|| format!("frame {} fits no page", rest[0].frame))?;
        let placed = place(&rest[..len], bucket).ok_or("a prefix that fit no longer fits")?;
        let mut page = PlannedPage { used_width: 0, used_height: 0, frames: Vec::with_capacity(len) };
        for (candidate, x, y) in placed {
            let (x, y) = (x as u32, y as u32);
            page.used_width = page.used_width.max(x + candidate.width);
            page.used_height = page.used_height.max(y + candidate.height);
            page.frames.push(PlacedFrame { frame: candidate.frame, x: x + leading, y: y + leading });
        }
        pages.push(page);
        start += len;
    }
    Ok(pages)
}

/// The block rectangle of a frame's content at its corner, in blocks.
pub fn content_blocks(x: u32, y: u32, width: u16, height: u16) -> crate::transforms::BlockRect {
    crate::transforms::BlockRect {
        x: (x / BLOCK_SIDE) as usize,
        y: (y / BLOCK_SIDE) as usize,
        width: align_block(u32::from(width)) as usize / BLOCK_SIDE as usize,
        height: align_block(u32::from(height)) as usize / BLOCK_SIDE as usize,
    }
}

/// A stored page's blocks, as the layout's source.
pub struct SourcePage<'a> {
    pub blocks: &'a [u8],
    pub blocks_wide: usize,
}

/// The blocks of `page`, each frame's slot (its content and gutter) copied from where `frames` places it on its
/// source page, the rest `background`. `source` gives the source page of a frame record's page index.
pub fn build<'a>(
    page: &PlannedPage,
    frames: &[FrameInfo],
    gutter: u16,
    background: &[u8; BLOCK_BYTES],
    source: impl Fn(u32) -> Option<SourcePage<'a>>,
) -> Result<Vec<u8>, String> {
    let (wide, high) = (page.blocks_wide(), page.blocks_high());
    let mut out = background.repeat(wide * high);
    let gutter_blocks = (align_block(u32::from(gutter)) / BLOCK_SIDE) as usize;
    for placed in &page.frames {
        let frame = frames[placed.frame];
        let from = content_blocks(u32::from(frame.x), u32::from(frame.y), frame.width, frame.height);
        let to = content_blocks(placed.x, placed.y, frame.width, frame.height);
        let source = source(frame.page).ok_or_else(|| format!("no source page {}", frame.page))?;
        let source_high = source.blocks.len() / BLOCK_BYTES / source.blocks_wide;
        if from.x < gutter_blocks || from.y < gutter_blocks || from.x + from.width + gutter_blocks > source.blocks_wide
            || from.y + from.height + gutter_blocks > source_high {
            return Err(format!("frame {} slot leaves its page {}", placed.frame, frame.page));
        }
        let row_blocks = from.width + 2 * gutter_blocks;
        for row in 0..from.height + 2 * gutter_blocks {
            let source_start = ((from.y - gutter_blocks + row) * source.blocks_wide + from.x - gutter_blocks) * BLOCK_BYTES;
            let target_start = ((to.y - gutter_blocks + row) * wide + to.x - gutter_blocks) * BLOCK_BYTES;
            out[target_start..target_start + row_blocks * BLOCK_BYTES]
                .copy_from_slice(&source.blocks[source_start..source_start + row_blocks * BLOCK_BYTES]);
        }
    }
    Ok(out)
}

/// The most common block of `blocks`: the fully transparent block on these pages.
pub fn most_common_block(blocks: &[u8]) -> [u8; BLOCK_BYTES] {
    let mut counts: HashMap<[u8; BLOCK_BYTES], usize> = HashMap::new();
    for block in blocks.chunks_exact(BLOCK_BYTES) {
        *counts.entry(block.try_into().expect("a block")).or_default() += 1;
    }
    counts.into_iter().max_by_key(|&(_, count)| count).map(|(block, _)| block).unwrap_or([0; BLOCK_BYTES])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_search_doubles_then_bisects() {
        // Monotone: the longest prefix.
        assert_eq!(longest_fitting_prefix(100, |count| count <= 37), 37);
        assert_eq!(longest_fitting_prefix(100, |count| count <= 100), 100);
        assert_eq!(longest_fitting_prefix(0, |_| true), 0);
        assert_eq!(longest_fitting_prefix(5, |_| false), 0);
        // Not monotone: 1, 2, 4, 8 fit, 16 does not; the bisection of 9..=15 tries 12 (no), then 10 (yes), 11 (no).
        assert_eq!(longest_fitting_prefix(100, |count| count != 12 && count != 11 && count < 16), 10);
    }

    #[test]
    fn frames_fill_a_small_page_before_a_larger_one() {
        let frames: Vec<SlotFrame> =
            (0..8).map(|frame| SlotFrame { frame, key: frame as u64, width: 100, height: 100 }).collect();
        let pages = plan(&frames, 4).unwrap();
        assert_eq!(pages.iter().map(|page| page.frames.len()).sum::<usize>(), frames.len());
        for page in &pages {
            for placed in &page.frames {
                assert_eq!((placed.x % BLOCK_SIDE, placed.y % BLOCK_SIDE), (0, 0));
                assert!(placed.x + 100 + 4 <= page.used_width && placed.y + 100 + 4 <= page.used_height);
            }
        }
    }
}
