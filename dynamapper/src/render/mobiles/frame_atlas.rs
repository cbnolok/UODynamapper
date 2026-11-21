//! The mobiles' frame atlas: the frames of the clips in use, packed on shelves into the pages of one 2D array texture.
//! A clip takes its frames' room on one page. When no page has room, the page used longest ago empties for it, and
//! the clips on it load again when drawn; a page unused for `IDLE_PAGE_RELEASE_SECS` empties too. Placed frames upload
//! to the render world rectangle by rectangle.

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

#[derive(Debug)]
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
    /// GPU writes there itself (the RIFE in-betweens).
    pub fn reserve_clip(&mut self, clip: K, sizes: &[(u32, u32)], now: f64) -> Option<(Vec<AtlasRect>, Vec<K>)> {
        let mut evicted = Vec::new();
        let layer = match self.page_with_room(sizes) {
            Some(layer) => layer,
            None => {
                if !fits(&mut Page::<K>::empty(), sizes) {
                    return None;
                }
                let layer = self.least_recently_used_page();
                evicted = std::mem::take(&mut self.pages[layer].clips);
                self.pages[layer] = Page::empty();
                layer
            }
        };
        let page = &mut self.pages[layer];
        let rects = sizes.iter().map(|&(width, height)| {
            let (x, y) = page.place(width, height).expect("the page was checked to hold the clip");
            AtlasRect { layer: layer as u32, x, y, width, height }
        }).collect();
        page.clips.push(clip);
        page.last_used = now;
        Some((rects, evicted))
    }

    /// Whether `reserve_clip` would find room for frames of `sizes` without emptying a page used since `since`.
    /// Frames no page holds count as room: `reserve_clip` turns them down.
    pub fn has_room_sparing(&self, sizes: &[(u32, u32)], since: f64) -> bool {
        self.page_with_room(sizes).is_some() || self.pages[self.least_recently_used_page()].last_used < since
            || !fits(&mut Page::<K>::empty(), sizes)
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
        for page in &mut self.pages {
            if !page.is_empty() && now - page.last_used > IDLE_PAGE_RELEASE_SECS {
                released.append(&mut page.clips);
                *page = Page::empty();
            }
        }
        released
    }

    fn page_with_room(&self, sizes: &[(u32, u32)]) -> Option<usize> {
        // A trial on a copy of each page's shelves, the fullest first, so that the pages fill one by one.
        let mut order = (0..self.pages.len()).filter(|&layer| !self.pages[layer].is_empty()).collect::<Vec<_>>();
        order.sort_by_key(|&layer| std::cmp::Reverse(self.pages[layer].next_shelf_y));
        order.extend((0..self.pages.len()).filter(|&layer| self.pages[layer].is_empty()));
        order.into_iter().find(|&layer| {
            let page = &self.pages[layer];
            let mut trial = Page::<K> { shelves: page.shelves.clone(), next_shelf_y: page.next_shelf_y,
                last_used: page.last_used, clips: Vec::new() };
            fits(&mut trial, sizes)
        })
    }

    fn least_recently_used_page(&self) -> usize {
        (0..self.pages.len())
            .min_by(|&a, &b| self.pages[a].last_used.total_cmp(&self.pages[b].last_used))
            .expect("the atlas has pages")
    }

    /// Hands the uploads staged since the last frame to the extraction.
    pub fn stage_for_extract(&mut self) {
        self.extract_staging = std::mem::take(&mut self.staged);
    }
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
}
