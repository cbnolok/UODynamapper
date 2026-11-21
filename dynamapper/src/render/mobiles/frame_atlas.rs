//! The mobiles' frame atlas: the frames of the clips in use, packed on shelves into the pages of one 2D array texture.
//! A clip takes its frames' room on one page. When no page has room, the page used longest ago empties for it, and
//! the clips on it load again when drawn; a page unused for `IDLE_PAGE_RELEASE_SECS` empties too. A sequence larger
//! than a page (`reserve_sequence`) lies on several, in pieces wherever there is room; it is one entry all the same:
//! a page of it emptied, it leaves the others too. Placed frames upload to the render world rectangle by rectangle.

use std::hash::Hash;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;

/// The side of a page, in texels.
pub const MOBILE_ATLAS_PAGE_SIZE: u32 = 1024;
/// The pages of the atlas: 8 RGBA8 pages are 32 MiB, room for the movements of a few creatures and their riders in
/// every direction.
pub const MOBILE_ATLAS_PAGES: u32 = 8;
/// The transparent texels kept between frames, so a filtered tap at a frame's edge reads none of its neighbour.
pub const FRAME_GUTTER: u32 = 1;
/// A page no drawn clip used for this long empties.
pub const IDLE_PAGE_RELEASE_SECS: f64 = 300.0;
const BYTES_PER_TEXEL: u32 = 4;

/// Where a frame lies in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRect {
    /// The rectangle's corners in the page's 0..1 coordinates.
    pub fn uv(&self) -> (Vec2, Vec2) {
        let size = MOBILE_ATLAS_PAGE_SIZE as f32;
        (Vec2::new(self.x as f32, self.y as f32) / size,
            Vec2::new((self.x + self.width) as f32, (self.y + self.height) as f32) / size)
    }
}

/// A row of a page, filled left to right.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

#[derive(Clone, Debug)]
struct Page<K> {
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    /// The time a clip on the page was last drawn.
    last_used: f64,
    clips: Vec<K>,
}

impl<K> Page<K> {
    fn empty() -> Self {
        Self { shelves: Vec::new(), next_shelf_y: 0, last_used: f64::NEG_INFINITY, clips: Vec::new() }
    }

    fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Room for a `width` by `height` frame: on the first shelf it fits, else on a new one.
    fn place(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let (padded_width, padded_height) = (width + FRAME_GUTTER, height + FRAME_GUTTER);
        if padded_width > MOBILE_ATLAS_PAGE_SIZE || padded_height > MOBILE_ATLAS_PAGE_SIZE {
            return None;
        }
        if let Some(shelf) = self.shelves.iter_mut()
            .find(|shelf| shelf.height >= padded_height && shelf.next_x + padded_width <= MOBILE_ATLAS_PAGE_SIZE) {
            let x = shelf.next_x;
            shelf.next_x += padded_width;
            return Some((x, shelf.y));
        }
        if self.next_shelf_y + padded_height > MOBILE_ATLAS_PAGE_SIZE {
            return None;
        }
        let y = self.next_shelf_y;
        self.next_shelf_y += padded_height;
        self.shelves.push(Shelf { y, height: padded_height, next_x: padded_width });
        Some((0, y))
    }
}

/// A frame's pixels on their way to the GPU.
#[derive(Clone)]
pub struct FrameUpload {
    pub rect: AtlasRect,
    /// Straight-alpha RGBA8, `rect.width` by `rect.height`, rows top down.
    pub rgba: std::sync::Arc<[u8]>,
}

/// The frames' pages, keyed by clip `K`.
#[derive(Resource)]
pub struct MobileFrameAtlas<K> {
    image: Handle<Image>,
    pages: Vec<Page<K>>,
    /// The uploads placed since the last extraction, then the ones handed to it.
    staged: Vec<FrameUpload>,
    extract_staging: Vec<FrameUpload>,
}

impl<K: Copy + Eq + Hash> MobileFrameAtlas<K> {
    pub fn new(images: &mut Assets<Image>) -> Self {
        Self {
            image: images.add(atlas_image()),
            pages: (0..MOBILE_ATLAS_PAGES).map(|_| Page::empty()).collect(),
            staged: Vec::new(),
            extract_staging: Vec::new(),
        }
    }

    pub fn image(&self) -> &Handle<Image> {
        &self.image
    }

    /// Places the frames of `clip`, `(width, height, rgba)` each, on one page, and stages their uploads. A page is
    /// emptied for them when none has room: the clips it held come back, to be dropped by their owner. `None` when
    /// the frames do not fit an empty page.
    pub fn place_clip(&mut self, clip: K, frames: &[(u32, u32, std::sync::Arc<[u8]>)], now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        let sizes = frames.iter().map(|(width, height, _)| (*width, *height)).collect::<Vec<_>>();
        let (rects, evicted) = self.reserve_clip(clip, &sizes, now)?;
        for (rect, (_, _, rgba)) in rects.iter().zip(frames) {
            self.staged.push(FrameUpload { rect: *rect, rgba: rgba.clone() });
        }
        Some((rects, evicted))
    }

    /// Makes room for the frames of `clip`, `(width, height)` each, on one page, as `place_clip` does, for pixels the
    /// GPU writes there itself.
    pub fn reserve_clip(&mut self, clip: K, sizes: &[(u32, u32)], now: f64) -> Option<(Vec<AtlasRect>, Vec<K>)> {
        self.reserve(clip, sizes, Placing::OnePage, now)
    }

    /// Makes room for a sequence of frames of `sizes` (the RIFE in-betweens) as `reserve_clip` does when they fit a
    /// page, else in pieces over several pages: each frame on the page of the one before while it has room, else on
    /// the first page with room for it, the fullest first, pages emptied as for a clip until all have room. `None`
    /// when a frame does not fit an empty page or the frames do not fit the atlas.
    pub fn reserve_sequence(&mut self, sequence: K, sizes: &[(u32, u32)], now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        self.reserve(sequence, sizes, Placing::Pieces, now)
    }

    /// Whether `reserve_sequence` would find room for frames of `sizes` without emptying a page used since `since`.
    /// Frames the atlas does not hold count as room: `reserve_sequence` turns them down.
    pub fn has_room_sparing(&self, sizes: &[(u32, u32)], since: f64) -> bool {
        reserve_on(&mut self.pages.clone(), sizes, Placing::Pieces, since).is_some()
            || reserve_on(&mut self.pages.clone(), sizes, Placing::Pieces, f64::INFINITY).is_none()
    }

    fn reserve(&mut self, entry: K, sizes: &[(u32, u32)], placing: Placing, now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        // On a copy of the pages, so that frames the atlas does not hold leave it as it was.
        let mut pages = self.pages.clone();
        let (rects, evicted) = reserve_on(&mut pages, sizes, placing, f64::INFINITY)?;
        for rect in &rects {
            let page = &mut pages[rect.layer as usize];
            if !page.clips.contains(&entry) {
                page.clips.push(entry);
            }
            page.last_used = now;
        }
        self.pages = pages;
        Some((rects, evicted))
    }

    /// Marks the page of a clip drawn at `now`.
    pub fn touch(&mut self, layer: u32, now: f64) {
        if let Some(page) = self.pages.get_mut(layer as usize) {
            page.last_used = now;
        }
    }

    /// Empties the pages no clip drawn since `now - IDLE_PAGE_RELEASE_SECS`, and returns their clips.
    pub fn release_idle(&mut self, now: f64) -> Vec<K> {
        let mut released = Vec::new();
        for layer in 0..self.pages.len() {
            let page = &self.pages[layer];
            if !page.is_empty() && now - page.last_used > IDLE_PAGE_RELEASE_SECS {
                released.extend(empty_page(&mut self.pages, layer));
            }
        }
        released
    }

    /// Hands the uploads staged since the last frame to the extraction.
    pub fn stage_for_extract(&mut self) {
        self.extract_staging = std::mem::take(&mut self.staged);
    }
}

/// How a reservation lays its frames out: all on one page, or in pieces over several when they do not fit one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placing {
    OnePage,
    Pieces,
}

/// Places frames of `sizes` on `pages`, emptying the page used longest ago until they have room, but none used since
/// `spare_since`: the frames' rectangles and the entries of the pages emptied. None, the pages possibly changed, when
/// they do not get room. The pages do not list the frames' entry yet.
fn reserve_on<K: Copy + Eq>(pages: &mut [Page<K>], sizes: &[(u32, u32)], placing: Placing, spare_since: f64)
    -> Option<(Vec<AtlasRect>, Vec<K>)> {
    let one_page = fits(&mut Page::<K>::empty(), sizes);
    let each_fits_a_page = || sizes.iter().all(|&size| fits(&mut Page::<K>::empty(), &[size]));
    if !one_page && (placing == Placing::OnePage || !each_fits_a_page()) {
        return None;
    }
    let mut evicted = Vec::new();
    loop {
        let layers = if one_page {
            page_with_room(pages, sizes).map(|layer| vec![layer; sizes.len()])
        } else {
            pages_with_room(pages, sizes)
        };
        if let Some(layers) = layers {
            let rects = sizes.iter().zip(layers).map(|(&(width, height), layer)| {
                let (x, y) = pages[layer].place(width, height).expect("the page was checked to hold the frame");
                AtlasRect { layer: layer as u32, x, y, width, height }
            }).collect();
            return Some((rects, evicted));
        }
        let layer = (0..pages.len()).filter(|&layer| !pages[layer].is_empty())
            .min_by(|&a, &b| pages[a].last_used.total_cmp(&pages[b].last_used))?;
        if pages[layer].last_used >= spare_since {
            return None;
        }
        evicted.extend(empty_page(pages, layer));
    }
}

/// The pages in the order they fill: those in use, the fullest first, then the empty ones, so that the pages fill
/// one by one.
fn fill_order<K>(pages: &[Page<K>]) -> Vec<usize> {
    let mut order = (0..pages.len()).filter(|&layer| !pages[layer].is_empty()).collect::<Vec<_>>();
    order.sort_by_key(|&layer| std::cmp::Reverse(pages[layer].next_shelf_y));
    order.extend((0..pages.len()).filter(|&layer| pages[layer].is_empty()));
    order
}

/// The first page in `fill_order` with room for all frames of `sizes`.
fn page_with_room<K>(pages: &[Page<K>], sizes: &[(u32, u32)]) -> Option<usize> {
    // A trial on a copy of each page's shelves.
    fill_order(pages).into_iter().find(|&layer| {
        let page = &pages[layer];
        let mut trial = Page::<K> { shelves: page.shelves.clone(), next_shelf_y: page.next_shelf_y,
            last_used: page.last_used, clips: Vec::new() };
        fits(&mut trial, sizes)
    })
}

/// The pages of the frames of `sizes` in pieces, tried on a copy: each frame on the page of the one before while it
/// has room, else on the first page in `fill_order` with room for it. None when a frame finds none.
fn pages_with_room<K: Clone>(pages: &[Page<K>], sizes: &[(u32, u32)]) -> Option<Vec<usize>> {
    let mut trial = pages.to_vec();
    let order = fill_order(pages);
    let mut current: Option<usize> = None;
    sizes.iter().map(|&(width, height)| {
        let layer = current.filter(|&layer| trial[layer].place(width, height).is_some())
            .or_else(|| order.iter().copied().find(|&layer| trial[layer].place(width, height).is_some()))?;
        current = Some(layer);
        Some(layer)
    }).collect()
}

/// Empties page `layer` and returns its entries. An entry is one unit, though a sequence's frames may lie on several
/// pages: it leaves them all, and a page left without entries empties as well.
fn empty_page<K: Copy + Eq>(pages: &mut [Page<K>], layer: usize) -> Vec<K> {
    let evicted = std::mem::take(&mut pages[layer].clips);
    pages[layer] = Page::empty();
    for page in pages.iter_mut().filter(|page| !page.is_empty()) {
        page.clips.retain(|entry| !evicted.contains(entry));
        if page.is_empty() {
            *page = Page::empty();
        }
    }
    evicted
}

fn fits<K>(page: &mut Page<K>, sizes: &[(u32, u32)]) -> bool {
    sizes.iter().all(|&(width, height)| page.place(width, height).is_some())
}

fn atlas_image() -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor,
        TextureViewDimension};
    // No CPU data: the GPU zero-fills the pages, transparent, and the frames upload into them.
    let mut image = Image::new_uninit(
        Extent3d { width: MOBILE_ATLAS_PAGE_SIZE, height: MOBILE_ATLAS_PAGE_SIZE, depth_or_array_layers: MOBILE_ATLAS_PAGES },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image
}

/// Hands the frames placed this frame to the extraction.
pub fn sys_stage_mobile_frame_uploads(atlas: Option<ResMut<MobileFrameAtlas<super::clips::AtlasEntry>>>) {
    if let Some(mut atlas) = atlas {
        atlas.stage_for_extract();
    }
}

/// The render world's uploads, and the atlas image they go to.
#[derive(Resource, Default)]
pub struct RenderMobileFrameUploads {
    image: Option<Handle<Image>>,
    uploads: Vec<FrameUpload>,
}

pub fn sys_extract_mobile_frame_uploads<K: Copy + Eq + Hash + Send + Sync + 'static>(
    atlas: Extract<Option<Res<MobileFrameAtlas<K>>>>,
    mut render_uploads: ResMut<RenderMobileFrameUploads>,
) {
    let Some(atlas) = atlas.as_ref() else { return };
    render_uploads.image = Some(atlas.image.clone());
    render_uploads.uploads.extend(atlas.extract_staging.iter().cloned());
}

/// Writes the uploads into the atlas once its image is on the GPU.
pub fn sys_render_upload_mobile_frames(
    mut render_uploads: ResMut<RenderMobileFrameUploads>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    use bevy::render::render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect};
    if render_uploads.uploads.is_empty() {
        return;
    }
    let Some(gpu_image) = render_uploads.image.as_ref().and_then(|image| gpu_images.get(image)) else { return };
    for upload in render_uploads.uploads.iter() {
        let rect = upload.rect;
        render_queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu_image.texture,
                mip_level: 0,
                origin: Origin3d { x: rect.x, y: rect.y, z: rect.layer },
                aspect: TextureAspect::All,
            },
            &upload.rgba,
            TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(rect.width * BYTES_PER_TEXEL), rows_per_image: None },
            Extent3d { width: rect.width, height: rect.height, depth_or_array_layers: 1 },
        );
    }
    render_uploads.uploads.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(count: usize, side: u32) -> Vec<(u32, u32, std::sync::Arc<[u8]>)> {
        (0..count).map(|_| (side, side, std::sync::Arc::from(vec![0u8; (side * side * BYTES_PER_TEXEL) as usize]))).collect()
    }

    fn atlas() -> MobileFrameAtlas<u32> {
        MobileFrameAtlas { image: Handle::default(), pages: (0..MOBILE_ATLAS_PAGES).map(|_| Page::empty()).collect(),
            staged: Vec::new(), extract_staging: Vec::new() }
    }

    #[test]
    fn frames_of_a_clip_share_a_page_without_overlapping() {
        let mut atlas = atlas();
        let (rects, evicted) = atlas.place_clip(1, &frames(40, 100), 0.0).unwrap();
        assert!(evicted.is_empty());
        assert!(rects.iter().all(|rect| rect.layer == rects[0].layer));
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let apart = a.x + a.width < b.x || b.x + b.width < a.x || a.y + a.height < b.y || b.y + b.height < a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(atlas.staged.len(), 40);
    }

    #[test]
    fn a_full_atlas_empties_its_least_recently_used_page() {
        let mut atlas = atlas();
        // A clip of 100 frames of 100 texels fills a page (10 by 10 per 1024, with their gutters).
        const OLDEST_PAGE: u32 = 1;
        for clip in 0..MOBILE_ATLAS_PAGES {
            let now = if clip == OLDEST_PAGE { 0.0 } else { 1.0 + f64::from(clip) };
            atlas.place_clip(clip, &frames(100, 100), now).unwrap();
        }
        let (rects, evicted) = atlas.place_clip(99, &frames(1, 100), f64::from(MOBILE_ATLAS_PAGES) + 1.0).unwrap();
        assert_eq!(evicted, vec![OLDEST_PAGE]);
        assert_eq!(rects[0].layer, OLDEST_PAGE);
    }

    #[test]
    fn idle_pages_empty() {
        let mut atlas = atlas();
        atlas.place_clip(1, &frames(1, 10), 0.0).unwrap();
        atlas.place_clip(2, &frames(1, 10), 0.0).unwrap();
        assert!(atlas.release_idle(IDLE_PAGE_RELEASE_SECS).is_empty());
        assert_eq!(atlas.release_idle(IDLE_PAGE_RELEASE_SECS + 1.0), vec![1, 2]);
    }

    /// The sizes of `count` frames of `side` texels.
    fn sizes(count: usize, side: u32) -> Vec<(u32, u32)> {
        vec![(side, side); count]
    }

    /// A page holds 100 frames of 100 texels (`a_full_atlas_empties_its_least_recently_used_page`).
    const FRAMES_PER_PAGE: usize = 100;

    fn assert_apart(rects: &[AtlasRect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in rects[i + 1..].iter().filter(|b| b.layer == a.layer) {
                let apart = a.x + a.width < b.x || b.x + b.width < a.x || a.y + a.height < b.y || b.y + b.height < a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn a_sequence_larger_than_a_page_lies_in_pieces_and_leaves_as_one() {
        const CLIP: u32 = 1;
        const SEQUENCE: u32 = 7;
        const CLIP_FRAMES: usize = 30;
        let mut atlas = atlas();
        let (clip_rects, _) = atlas.place_clip(CLIP, &frames(CLIP_FRAMES, 100), 0.0).unwrap();
        let sequence = sizes(FRAMES_PER_PAGE * 2 - CLIP_FRAMES, 100);
        // A clip keeps to one page; a sequence fills the free room of the clip's page, then an empty one.
        assert!(atlas.reserve_clip(SEQUENCE, &sequence, 1.0).is_none());
        let (rects, evicted) = atlas.reserve_sequence(SEQUENCE, &sequence, 1.0).unwrap();
        assert!(evicted.is_empty());
        let split = FRAMES_PER_PAGE - CLIP_FRAMES;
        let (first_layer, second_layer) = (clip_rects[0].layer, rects[split].layer);
        assert!(rects[..split].iter().all(|rect| rect.layer == first_layer));
        assert!(rects[split..].iter().all(|rect| rect.layer == second_layer));
        assert_ne!(first_layer, second_layer);
        assert_apart(&[clip_rects.as_slice(), rects.as_slice()].concat());
        assert_eq!(atlas.pages[first_layer as usize].clips, vec![CLIP, SEQUENCE]);
        assert_eq!(atlas.pages[second_layer as usize].clips, vec![SEQUENCE]);

        // The other pages full, clips drawn after; the clip's page drawn since. The sequence's second page is the
        // least recently used: emptied for a new clip, the sequence leaves the clip's page too, the clip stays.
        for clip in 10..10 + MOBILE_ATLAS_PAGES - 2 {
            atlas.place_clip(clip, &frames(FRAMES_PER_PAGE, 100), 2.0 + f64::from(clip)).unwrap();
        }
        atlas.touch(first_layer, 50.0);
        let (rects, evicted) = atlas.place_clip(99, &frames(1, 100), 60.0).unwrap();
        assert_eq!(evicted, vec![SEQUENCE]);
        assert_eq!(rects[0].layer, second_layer);
        assert_eq!(atlas.pages[first_layer as usize].clips, vec![CLIP]);
    }

    #[test]
    fn a_page_left_without_entries_empties() {
        const SEQUENCE: u32 = 7;
        let mut atlas = atlas();
        let (rects, _) = atlas.reserve_sequence(SEQUENCE, &sizes(FRAMES_PER_PAGE * 2, 100), 0.0).unwrap();
        let (first_layer, second_layer) = (rects[0].layer, rects[FRAMES_PER_PAGE].layer);
        assert_ne!(first_layer, second_layer);
        // Its first page idle, its second drawn: both empty, as the sequence is one entry.
        atlas.touch(second_layer, IDLE_PAGE_RELEASE_SECS);
        assert_eq!(atlas.release_idle(IDLE_PAGE_RELEASE_SECS + 1.0), vec![SEQUENCE]);
        assert!(atlas.pages.iter().all(|page| page.is_empty() && page.shelves.is_empty()));
    }

    #[test]
    fn a_sequence_spares_the_pages_drawn_and_one_too_large_is_turned_down() {
        const NOW: f64 = 10.0;
        let mut atlas = atlas();
        for clip in 0..MOBILE_ATLAS_PAGES {
            atlas.place_clip(clip, &frames(FRAMES_PER_PAGE, 100), NOW).unwrap();
        }
        let sequence = sizes(FRAMES_PER_PAGE * 2, 100);
        assert!(!atlas.has_room_sparing(&sequence, NOW));
        assert!(atlas.has_room_sparing(&sequence, NOW + 1.0));
        // Larger than the atlas, or a frame larger than a page: no room ever, and the atlas stays as it was.
        let too_many = sizes(FRAMES_PER_PAGE * (MOBILE_ATLAS_PAGES as usize + 1), 100);
        let too_large = sizes(1, MOBILE_ATLAS_PAGE_SIZE);
        for sizes in [too_many, too_large] {
            assert!(atlas.has_room_sparing(&sizes, NOW));
            assert!(atlas.reserve_sequence(99, &sizes, NOW + 1.0).is_none());
            assert!(atlas.pages.iter().enumerate().all(|(layer, page)| page.clips == vec![layer as u32]));
        }
    }
}
