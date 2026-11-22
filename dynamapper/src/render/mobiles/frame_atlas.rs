//! The mobiles' frame atlases: the frames of the clips in use, packed on shelves into the pages of a 2D array texture,
//! one atlas per pixel format the animation packages store (`AtlasFormat`): an RGBA8888 package's frames upload as
//! RGBA8, a BC7 package's as its BC7 blocks, never decoded. With both packages of one format there is one atlas. An
//! entry (a clip, or a RIFE sequence) takes its frames' room in its source's atlas, on one page when they fit one,
//! else in pieces over several wherever there is room; it is one entry all the same: a page of it emptied, it leaves
//! the others too. When no page has room, the page used longest ago empties for it, and the clips on it load again
//! when drawn; a page unused for `IDLE_PAGE_RELEASE_SECS` empties too. Placed frames upload to the render world
//! rectangle by rectangle.
//!
//! While the SPAN upscales are on (upscales.rs), each atlas has `MOBILE_ATLAS_UPSCALED_PAGES` more pages after its
//! own, a pool of their own (`AtlasPool::Upscaled`): an upscaled sequence empties only pages of upscaled ones, never
//! the clips' or the in-betweens'. Turning them on or off remakes the atlases' textures, empty.

use std::hash::Hash;
use std::sync::Arc;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;
use udd_assets::tex_art_cc::PagePixelFormat;
use uot_gpu_compute::block_encode::BLOCK_SIDE;
use crate::render::assets::art::ART_LAYER_ENHANCED_ATLAS_BIT;

/// The side of a page, in texels.
pub const MOBILE_ATLAS_PAGE_SIZE: u32 = 1024;
/// The pages of an atlas: 8 RGBA8 pages are 32 MiB (8 BC7 ones 8 MiB), room for the movements of a few creatures and
/// their riders in every direction.
pub const MOBILE_ATLAS_PAGES: u32 = 8;
/// The transparent texels kept between frames of an RGBA8 atlas, so a filtered tap at a frame's edge reads none of its
/// neighbour. A BC7 atlas keeps a block (`AtlasFormat::gutter`).
pub const FRAME_GUTTER: u32 = 1;
/// The pages of an atlas the upscaled sequences take while the upscales are on, besides `MOBILE_ATLAS_PAGES`: 64 MiB
/// more for 16 RGBA8 pages (16 MiB for BC7 ones), as many sequences at 2x as the clips' pages hold at 1x.
pub const MOBILE_ATLAS_UPSCALED_PAGES: u32 = 16;
/// A page no drawn clip used for this long empties.
pub const IDLE_PAGE_RELEASE_SECS: f64 = 300.0;
const RGBA8_TEXEL_BYTES: u32 = 4;
const BC7_BLOCK_BYTES: u32 = 16;

/// The pixel format of a frame atlas, its packages' pages' (`PagePixelFormat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AtlasFormat {
    /// Straight-alpha RGBA8, sRGB.
    Rgba8,
    /// BC7 blocks, sRGB, as a BC7 package stores them: a frame's rectangle starts on a block corner and is padded to
    /// whole blocks.
    Bc7,
}

impl AtlasFormat {
    pub fn of(format: PagePixelFormat) -> Self {
        match format {
            PagePixelFormat::Rgba8888 => Self::Rgba8,
            PagePixelFormat::Bc7 => Self::Bc7,
        }
    }

    pub fn texture_format(self) -> TextureFormat {
        match self {
            Self::Rgba8 => TextureFormat::Rgba8UnormSrgb,
            Self::Bc7 => TextureFormat::Bc7RgbaUnormSrgb,
        }
    }

    /// The side a frame's rectangle starts on a multiple of and is padded to: a texel, or a block.
    pub const fn align(self) -> u32 {
        match self {
            Self::Rgba8 => 1,
            Self::Bc7 => BLOCK_SIDE,
        }
    }

    /// The room kept free after a frame, right and below: `FRAME_GUTTER`, or a block, so that a tap past a frame's
    /// blocks reads none of its neighbour's.
    pub const fn gutter(self) -> u32 {
        match self {
            Self::Rgba8 => FRAME_GUTTER,
            Self::Bc7 => BLOCK_SIDE,
        }
    }

    /// The extent a `width` by `height` frame's pixels cover: the frame, padded to whole blocks for BC7.
    pub const fn padded(self, width: u32, height: u32) -> (u32, u32) {
        (width.next_multiple_of(self.align()), height.next_multiple_of(self.align()))
    }

    /// The bytes of a row of a `width` wide frame's pixels: a row of texels, or of blocks.
    pub const fn row_bytes(self, width: u32) -> u32 {
        match self {
            Self::Rgba8 => width * RGBA8_TEXEL_BYTES,
            Self::Bc7 => width.div_ceil(BLOCK_SIDE) * BC7_BLOCK_BYTES,
        }
    }

    /// The bytes of a `width` by `height` frame's pixels.
    pub const fn frame_bytes(self, width: u32, height: u32) -> usize {
        let rows = height.div_ceil(self.align());
        self.row_bytes(width) as usize * rows as usize
    }
}

/// Where a frame lies: its atlas, its page there and its rectangle (`width` by `height` its own size, its pixels
/// covering `padded_size`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRect {
    pub format: AtlasFormat,
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

    /// The extent of the frame's pixels: whole blocks in a BC7 atlas.
    pub fn padded_size(&self) -> (u32, u32) {
        self.format.padded(self.width, self.height)
    }

    /// The layer as an instance of the art sprite shader names it: a BC7 atlas binds as the material's second atlas
    /// (`atlas_enhanced`), whose pages carry `ART_LAYER_ENHANCED_ATLAS_BIT` (art/atlas_pages.wesl).
    pub fn instance_layer(&self) -> u32 {
        match self.format {
            AtlasFormat::Rgba8 => self.layer,
            AtlasFormat::Bc7 => self.layer | ART_LAYER_ENHANCED_ATLAS_BIT,
        }
    }
}

/// The pages of an atlas an entry takes room in, each emptied only for entries of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasPool {
    /// The clips and their in-betweens: the first `MOBILE_ATLAS_PAGES`.
    Base,
    /// The upscaled sequences: the pages after, while the upscales are on.
    Upscaled,
}

impl AtlasPool {
    /// The layers of the pool among `pages` layers.
    fn layers(self, pages: usize) -> std::ops::Range<usize> {
        let base = (MOBILE_ATLAS_PAGES as usize).min(pages);
        match self {
            Self::Base => 0..base,
            Self::Upscaled => base..pages,
        }
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

    /// Room for a `width` by `height` frame of `format`: on the first shelf it fits, else on a new one. Every room
    /// taken is a multiple of `format`'s alignment, so every frame starts on one.
    fn place(&mut self, format: AtlasFormat, width: u32, height: u32) -> Option<(u32, u32)> {
        let (width, height) = format.padded(width, height);
        let (padded_width, padded_height) = (width + format.gutter(), height + format.gutter());
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
    /// The pixels of `rect.padded_size()` in `rect.format`, rows top down: straight-alpha RGBA8, or rows of BC7
    /// blocks.
    pub pixels: Arc<[u8]>,
}

/// The pages of one format.
struct FormatAtlas<K> {
    format: AtlasFormat,
    image: Handle<Image>,
    pages: Vec<Page<K>>,
}

/// The frames' atlases, one per format, keyed by clip `K`.
#[derive(Resource)]
pub struct MobileFrameAtlas<K> {
    atlases: Vec<FormatAtlas<K>>,
    /// The uploads placed since the last extraction, then the ones handed to it.
    staged: Vec<FrameUpload>,
    extract_staging: Vec<FrameUpload>,
}

impl<K: Copy + Eq + Hash> MobileFrameAtlas<K> {
    /// The atlases of `formats`, one each.
    pub fn new(images: &mut Assets<Image>, formats: impl IntoIterator<Item = AtlasFormat>) -> Self {
        let mut atlases: Vec<FormatAtlas<K>> = Vec::new();
        for format in formats {
            if atlases.iter().all(|atlas| atlas.format != format) {
                atlases.push(FormatAtlas { format, image: images.add(atlas_image(format, MOBILE_ATLAS_PAGES)),
                    pages: empty_pages(MOBILE_ATLAS_PAGES) });
            }
        }
        Self { atlases, staged: Vec::new(), extract_staging: Vec::new() }
    }

    /// The image of the atlas of `format`, if there is one.
    pub fn image(&self, format: AtlasFormat) -> Option<&Handle<Image>> {
        self.atlases.iter().find(|atlas| atlas.format == format).map(|atlas| &atlas.image)
    }

    /// The atlases' formats and images.
    pub fn images(&self) -> impl Iterator<Item = (AtlasFormat, &Handle<Image>)> {
        self.atlases.iter().map(|atlas| (atlas.format, &atlas.image))
    }

    fn pages_of(&mut self, format: AtlasFormat) -> Option<&mut Vec<Page<K>>> {
        self.atlases.iter_mut().find(|atlas| atlas.format == format).map(|atlas| &mut atlas.pages)
    }

    /// The pages of the upscaled pool of each atlas.
    pub fn upscaled_pages(&self) -> u32 {
        self.atlases.first().map_or(0, |atlas| atlas.pages.len() as u32 - MOBILE_ATLAS_PAGES)
    }

    /// Gives each atlas `upscaled` pages for upscaled sequences after its own. An atlas that changes is remade, its
    /// texture too, empty: the entries it held come back, to be dropped by their owners, and the uploads staged go.
    pub fn set_upscaled_pages(&mut self, images: &mut Assets<Image>, upscaled: u32) -> Vec<K> {
        let pages = MOBILE_ATLAS_PAGES + upscaled;
        let mut evicted = Vec::new();
        for atlas in self.atlases.iter_mut().filter(|atlas| atlas.pages.len() != pages as usize) {
            for page in std::mem::replace(&mut atlas.pages, empty_pages(pages)) {
                for entry in page.clips {
                    if !evicted.contains(&entry) {
                        evicted.push(entry);
                    }
                }
            }
            let _ = images.insert(&atlas.image, atlas_image(atlas.format, pages));
            self.staged.clear();
            self.extract_staging.clear();
        }
        evicted
    }

    /// Places the frames of `clip` in the atlas of `format`, `(width, height, pixels)` each (`FrameUpload::pixels`),
    /// as `reserve` does, and stages their uploads.
    pub fn place_clip(&mut self, clip: K, format: AtlasFormat, frames: &[(u32, u32, Arc<[u8]>)], now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        let sizes = frames.iter().map(|(width, height, _)| (*width, *height)).collect::<Vec<_>>();
        let (rects, evicted) = self.reserve(clip, format, &sizes, now)?;
        for (rect, (_, _, pixels)) in rects.iter().zip(frames) {
            self.staged.push(FrameUpload { rect: *rect, pixels: pixels.clone() });
        }
        Some((rects, evicted))
    }

    /// Makes room in the atlas of `format` for the frames of `entry`, `(width, height)` each, for pixels the GPU
    /// writes there itself: on the first page with room for all, the fullest first; when none has, in pieces, each
    /// frame on the page of the one before while it has room, else on the first page with room for it. Pages are
    /// emptied until the frames have room: the entries they held come back, to be dropped by their owner. `None` when
    /// a frame does not fit an empty page, the frames do not fit the atlas, or there is no atlas of `format`.
    pub fn reserve(&mut self, entry: K, format: AtlasFormat, sizes: &[(u32, u32)], now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        self.reserve_in(entry, format, AtlasPool::Base, sizes, now)
    }

    /// `reserve` on the pages of `pool`: those of another pool neither take the frames nor empty for them.
    pub fn reserve_in(&mut self, entry: K, format: AtlasFormat, pool: AtlasPool, sizes: &[(u32, u32)], now: f64)
        -> Option<(Vec<AtlasRect>, Vec<K>)> {
        let atlas_pages = self.pages_of(format)?;
        // On a copy of the pages, so that frames the atlas does not hold leave it as it was.
        let mut pages = atlas_pages.clone();
        let layers = pool.layers(pages.len());
        let (mut rects, evicted) = reserve_on(&mut pages[layers.clone()], format, sizes, f64::INFINITY)?;
        for rect in &mut rects {
            rect.layer += layers.start as u32;
        }
        for rect in &rects {
            let page = &mut pages[rect.layer as usize];
            if !page.clips.contains(&entry) {
                page.clips.push(entry);
            }
            page.last_used = now;
        }
        *atlas_pages = pages;
        Some((rects, evicted))
    }

    /// Whether `reserve` would find room for frames of `sizes` in the atlas of `format` without emptying a page used
    /// since `since`. Frames the atlas does not hold count as room: `reserve` turns them down.
    pub fn has_room_sparing(&self, format: AtlasFormat, sizes: &[(u32, u32)], since: f64) -> bool {
        self.has_room_sparing_in(format, AtlasPool::Base, sizes, since)
    }

    /// `has_room_sparing` on the pages of `pool`.
    pub fn has_room_sparing_in(&self, format: AtlasFormat, pool: AtlasPool, sizes: &[(u32, u32)], since: f64)
        -> bool {
        let Some(atlas) = self.atlases.iter().find(|atlas| atlas.format == format) else { return true };
        let layers = pool.layers(atlas.pages.len());
        reserve_on(&mut atlas.pages[layers.clone()].to_vec(), format, sizes, since).is_some()
            || reserve_on(&mut atlas.pages[layers].to_vec(), format, sizes, f64::INFINITY).is_none()
    }

    /// Marks the page of a frame of a clip drawn at `now`.
    pub fn touch(&mut self, rect: AtlasRect, now: f64) {
        if let Some(page) = self.pages_of(rect.format).and_then(|pages| pages.get_mut(rect.layer as usize)) {
            page.last_used = now;
        }
    }

    /// Empties the pages no clip drawn since `now - IDLE_PAGE_RELEASE_SECS`, and returns their clips.
    pub fn release_idle(&mut self, now: f64) -> Vec<K> {
        let mut released = Vec::new();
        for atlas in &mut self.atlases {
            for layer in 0..atlas.pages.len() {
                let page = &atlas.pages[layer];
                if !page.is_empty() && now - page.last_used > IDLE_PAGE_RELEASE_SECS {
                    released.extend(empty_page(&mut atlas.pages, layer));
                }
            }
        }
        released
    }

    /// Hands the uploads staged since the last frame to the extraction.
    pub fn stage_for_extract(&mut self) {
        self.extract_staging = std::mem::take(&mut self.staged);
    }
}

fn empty_pages<K>(pages: u32) -> Vec<Page<K>> {
    (0..pages).map(|_| Page::empty()).collect()
}

/// Places frames of `sizes` of `format` on `pages`, emptying the page used longest ago until they have room, but none
/// used since `spare_since`: the frames' rectangles and the entries of the pages emptied. None, the pages possibly
/// changed, when they do not get room. The pages do not list the frames' entry yet.
fn reserve_on<K: Copy + Eq>(pages: &mut [Page<K>], format: AtlasFormat, sizes: &[(u32, u32)], spare_since: f64)
    -> Option<(Vec<AtlasRect>, Vec<K>)> {
    let one_page = fits(&mut Page::<K>::empty(), format, sizes);
    let each_fits_a_page = || sizes.iter().all(|&size| fits(&mut Page::<K>::empty(), format, &[size]));
    if !one_page && !each_fits_a_page() {
        return None;
    }
    let mut evicted = Vec::new();
    loop {
        let layers = if one_page {
            page_with_room(pages, format, sizes).map(|layer| vec![layer; sizes.len()])
        } else {
            pages_with_room(pages, format, sizes)
        };
        if let Some(layers) = layers {
            let rects = sizes.iter().zip(layers).map(|(&(width, height), layer)| {
                let (x, y) =
                    pages[layer].place(format, width, height).expect("the page was checked to hold the frame");
                AtlasRect { format, layer: layer as u32, x, y, width, height }
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
fn page_with_room<K>(pages: &[Page<K>], format: AtlasFormat, sizes: &[(u32, u32)]) -> Option<usize> {
    // A trial on a copy of each page's shelves.
    fill_order(pages).into_iter().find(|&layer| {
        let page = &pages[layer];
        let mut trial = Page::<K> { shelves: page.shelves.clone(), next_shelf_y: page.next_shelf_y,
            last_used: page.last_used, clips: Vec::new() };
        fits(&mut trial, format, sizes)
    })
}

/// The pages of the frames of `sizes` in pieces, tried on a copy: each frame on the page of the one before while it
/// has room, else on the first page in `fill_order` with room for it. None when a frame finds none.
fn pages_with_room<K: Clone>(pages: &[Page<K>], format: AtlasFormat, sizes: &[(u32, u32)]) -> Option<Vec<usize>> {
    let mut trial = pages.to_vec();
    let order = fill_order(pages);
    let mut current: Option<usize> = None;
    sizes.iter().map(|&(width, height)| {
        let layer = current.filter(|&layer| trial[layer].place(format, width, height).is_some())
            .or_else(|| order.iter().copied().find(|&layer| trial[layer].place(format, width, height).is_some()))?;
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

fn fits<K>(page: &mut Page<K>, format: AtlasFormat, sizes: &[(u32, u32)]) -> bool {
    sizes.iter().all(|&(width, height)| page.place(format, width, height).is_some())
}

fn atlas_image(format: AtlasFormat, pages: u32) -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureViewDescriptor, TextureViewDimension};
    // No CPU data: the GPU zero-fills the pages, transparent (a zero BC7 block decodes to transparent black), and the
    // frames upload into them.
    let mut image = Image::new_uninit(
        Extent3d { width: MOBILE_ATLAS_PAGE_SIZE, height: MOBILE_ATLAS_PAGE_SIZE, depth_or_array_layers: pages },
        TextureDimension::D2,
        format.texture_format(),
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

/// The render world's uploads, and the atlas images they go to.
#[derive(Resource, Default)]
pub struct RenderMobileFrameUploads {
    images: Vec<(AtlasFormat, Handle<Image>)>,
    uploads: Vec<FrameUpload>,
}

pub fn sys_extract_mobile_frame_uploads<K: Copy + Eq + Hash + Send + Sync + 'static>(
    atlas: Extract<Option<Res<MobileFrameAtlas<K>>>>,
    mut render_uploads: ResMut<RenderMobileFrameUploads>,
) {
    let Some(atlas) = atlas.as_ref() else { return };
    if render_uploads.images.len() != atlas.atlases.len() {
        render_uploads.images = atlas.images().map(|(format, image)| (format, image.clone())).collect();
    }
    render_uploads.uploads.extend(atlas.extract_staging.iter().cloned());
}

/// The copy layout of `rect`'s pixels: the bytes of a row (of texels, or of blocks) and the extent they cover.
fn upload_layout(rect: AtlasRect) -> (u32, (u32, u32)) {
    (rect.format.row_bytes(rect.width), rect.padded_size())
}

/// Writes the uploads into their atlases once the images are on the GPU: RGBA8 rows, or BC7 blocks as they are.
pub fn sys_render_upload_mobile_frames(
    mut render_uploads: ResMut<RenderMobileFrameUploads>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    use bevy::render::render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect};
    if render_uploads.uploads.is_empty() {
        return;
    }
    let atlas = |format| render_uploads.images.iter().find(|(atlas, _)| *atlas == format)
        .and_then(|(_, image)| gpu_images.get(image));
    if render_uploads.uploads.iter().any(|upload| atlas(upload.rect.format).is_none()) {
        return;
    }
    for upload in render_uploads.uploads.iter() {
        let rect = upload.rect;
        let gpu_image = atlas(rect.format).expect("checked above");
        let (row_bytes, (width, height)) = upload_layout(rect);
        debug_assert_eq!(upload.pixels.len(), rect.format.frame_bytes(rect.width, rect.height));
        render_queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu_image.texture,
                mip_level: 0,
                origin: Origin3d { x: rect.x, y: rect.y, z: rect.layer },
                aspect: TextureAspect::All,
            },
            &upload.pixels,
            TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row_bytes), rows_per_image: None },
            Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
    render_uploads.uploads.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames_of(format: AtlasFormat, count: usize, side: u32) -> Vec<(u32, u32, Arc<[u8]>)> {
        (0..count).map(|_| (side, side, Arc::from(vec![0u8; format.frame_bytes(side, side)]))).collect()
    }

    fn frames(count: usize, side: u32) -> Vec<(u32, u32, Arc<[u8]>)> {
        frames_of(AtlasFormat::Rgba8, count, side)
    }

    fn atlas_of(formats: &[AtlasFormat]) -> MobileFrameAtlas<u32> {
        MobileFrameAtlas::new(&mut Assets::<Image>::default(), formats.iter().copied())
    }

    /// An RGBA8 atlas, as the tests before BC7 knew it.
    struct Rgba8Atlas(MobileFrameAtlas<u32>);

    impl Rgba8Atlas {
        fn place_clip(&mut self, clip: u32, frames: &[(u32, u32, Arc<[u8]>)], now: f64)
            -> Option<(Vec<AtlasRect>, Vec<u32>)> {
            self.0.place_clip(clip, AtlasFormat::Rgba8, frames, now)
        }

        fn reserve(&mut self, entry: u32, sizes: &[(u32, u32)], now: f64) -> Option<(Vec<AtlasRect>, Vec<u32>)> {
            self.0.reserve(entry, AtlasFormat::Rgba8, sizes, now)
        }

        fn has_room_sparing(&self, sizes: &[(u32, u32)], since: f64) -> bool {
            self.0.has_room_sparing(AtlasFormat::Rgba8, sizes, since)
        }

        fn touch(&mut self, layer: u32, now: f64) {
            self.0.touch(AtlasRect { format: AtlasFormat::Rgba8, layer, x: 0, y: 0, width: 0, height: 0 }, now);
        }

        fn release_idle(&mut self, now: f64) -> Vec<u32> {
            self.0.release_idle(now)
        }

        fn pages(&self) -> &[Page<u32>] {
            &self.0.atlases[0].pages
        }
    }

    fn atlas() -> Rgba8Atlas {
        Rgba8Atlas(atlas_of(&[AtlasFormat::Rgba8]))
    }

    #[test]
    fn the_upscaled_pool_takes_its_own_pages_and_remaking_the_atlas_empties_it() {
        let mut images = Assets::<Image>::default();
        let mut atlas = MobileFrameAtlas::<u32>::new(&mut images, [AtlasFormat::Rgba8, AtlasFormat::Bc7]);
        assert_eq!(atlas.upscaled_pages(), 0);
        // Without upscaled pages, there is no room for them.
        assert!(atlas.reserve_in(1, AtlasFormat::Rgba8, AtlasPool::Upscaled, &[(8, 8)], 0.0).is_none());
        assert!(atlas.set_upscaled_pages(&mut images, MOBILE_ATLAS_UPSCALED_PAGES).is_empty());
        let layers = MOBILE_ATLAS_PAGES + MOBILE_ATLAS_UPSCALED_PAGES;
        for (_, image) in atlas.images() {
            assert_eq!(images.get(image).unwrap().texture_descriptor.size.depth_or_array_layers, layers);
        }
        let (base, _) = atlas.reserve(1, AtlasFormat::Rgba8, &[(8, 8)], 0.0).unwrap();
        let (upscaled, _) = atlas.reserve_in(2, AtlasFormat::Rgba8, AtlasPool::Upscaled, &[(16, 16)], 0.0).unwrap();
        assert!(base[0].layer < MOBILE_ATLAS_PAGES);
        assert!((MOBILE_ATLAS_PAGES..layers).contains(&upscaled[0].layer));
        // Filling the upscaled pool empties its own pages only: the one of 2, used longest ago.
        let page = MOBILE_ATLAS_PAGE_SIZE - FRAME_GUTTER;
        let mut evicted = Vec::new();
        for entry in 3..3 + MOBILE_ATLAS_UPSCALED_PAGES {
            evicted.extend(atlas.reserve_in(entry, AtlasFormat::Rgba8, AtlasPool::Upscaled, &[(page, page)],
                f64::from(entry)).unwrap().1);
        }
        assert_eq!(evicted, vec![2]);
        // Remade, both atlases come back to their own pages, empty.
        let evicted = atlas.set_upscaled_pages(&mut images, 0);
        assert!(evicted.contains(&1) && evicted.contains(&3) && !evicted.contains(&2));
        for (_, image) in atlas.images() {
            assert_eq!(images.get(image).unwrap().texture_descriptor.size.depth_or_array_layers, MOBILE_ATLAS_PAGES);
        }
        assert!(atlas.atlases.iter().all(|atlas| atlas.pages.iter().all(Page::is_empty)));
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
        assert_eq!(atlas.0.staged.len(), 40);
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
        // The sequence fills the free room of the clip's page, then an empty one.
        let (rects, evicted) = atlas.reserve(SEQUENCE, &sequence, 1.0).unwrap();
        assert!(evicted.is_empty());
        let split = FRAMES_PER_PAGE - CLIP_FRAMES;
        let (first_layer, second_layer) = (clip_rects[0].layer, rects[split].layer);
        assert!(rects[..split].iter().all(|rect| rect.layer == first_layer));
        assert!(rects[split..].iter().all(|rect| rect.layer == second_layer));
        assert_ne!(first_layer, second_layer);
        assert_apart(&[clip_rects.as_slice(), rects.as_slice()].concat());
        assert_eq!(atlas.pages()[first_layer as usize].clips, vec![CLIP, SEQUENCE]);
        assert_eq!(atlas.pages()[second_layer as usize].clips, vec![SEQUENCE]);

        // The other pages full, clips drawn after; the clip's page drawn since. The sequence's second page is the
        // least recently used: emptied for a new clip, the sequence leaves the clip's page too, the clip stays.
        for clip in 10..10 + MOBILE_ATLAS_PAGES - 2 {
            atlas.place_clip(clip, &frames(FRAMES_PER_PAGE, 100), 2.0 + f64::from(clip)).unwrap();
        }
        atlas.touch(first_layer, 50.0);
        let (rects, evicted) = atlas.place_clip(99, &frames(1, 100), 60.0).unwrap();
        assert_eq!(evicted, vec![SEQUENCE]);
        assert_eq!(rects[0].layer, second_layer);
        assert_eq!(atlas.pages()[first_layer as usize].clips, vec![CLIP]);
    }

    #[test]
    fn a_clip_keeps_to_one_page_when_it_fits_one_else_lies_in_pieces() {
        const PARTLY_FILLED: u32 = 1;
        const FITS: u32 = 2;
        const LARGER: u32 = 3;
        let mut atlas = atlas();
        let (partly_filled, _) = atlas.place_clip(PARTLY_FILLED, &frames(FRAMES_PER_PAGE / 2, 100), 0.0).unwrap();
        // Too many for the room left on the first page, but not for a page: whole, on an empty one.
        let (fits, _) = atlas.place_clip(FITS, &frames(FRAMES_PER_PAGE * 3 / 4, 100), 0.0).unwrap();
        assert!(fits.iter().all(|rect| rect.layer == fits[0].layer));
        assert_ne!(fits[0].layer, partly_filled[0].layer);
        // More than a page: in pieces, the free room first.
        let (larger, evicted) = atlas.place_clip(LARGER, &frames(FRAMES_PER_PAGE * 3 / 2, 100), 0.0).unwrap();
        assert!(evicted.is_empty());
        let layers = larger.iter().map(|rect| rect.layer).collect::<std::collections::HashSet<_>>();
        assert!(layers.len() > 1);
        assert!(layers.contains(&fits[0].layer) || layers.contains(&partly_filled[0].layer));
        assert_apart(&[partly_filled, fits, larger].concat());
    }

    #[test]
    fn a_page_left_without_entries_empties() {
        const SEQUENCE: u32 = 7;
        let mut atlas = atlas();
        let (rects, _) = atlas.reserve(SEQUENCE, &sizes(FRAMES_PER_PAGE * 2, 100), 0.0).unwrap();
        let (first_layer, second_layer) = (rects[0].layer, rects[FRAMES_PER_PAGE].layer);
        assert_ne!(first_layer, second_layer);
        // Its first page idle, its second drawn: both empty, as the sequence is one entry.
        atlas.touch(second_layer, IDLE_PAGE_RELEASE_SECS);
        assert_eq!(atlas.release_idle(IDLE_PAGE_RELEASE_SECS + 1.0), vec![SEQUENCE]);
        assert!(atlas.pages().iter().all(|page| page.is_empty() && page.shelves.is_empty()));
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
            assert!(atlas.reserve(99, &sizes, NOW + 1.0).is_none());
            assert!(atlas.pages().iter().enumerate().all(|(layer, page)| page.clips == vec![layer as u32]));
        }
    }

    /// Whether the padded rectangles of `rects`, each with `gutter` free texels right and below, stay apart.
    fn padded_apart(rects: &[AtlasRect], gutter: u32) -> bool {
        rects.iter().enumerate().all(|(i, a)| rects[i + 1..].iter().filter(|b| b.layer == a.layer).all(|b| {
            let ((a_width, a_height), (b_width, b_height)) = (a.padded_size(), b.padded_size());
            a.x + a_width + gutter <= b.x || b.x + b_width + gutter <= a.x
                || a.y + a_height + gutter <= b.y || b.y + b_height + gutter <= a.y
        }))
    }

    /// A BC7 frame starts on a block corner, its pixels cover whole blocks, and a block stays free between frames;
    /// its rectangle keeps the frame's own size, for its coordinates.
    #[test]
    fn bc7_frames_start_on_blocks_padded_to_whole_blocks_a_block_apart() {
        const ODD_SIZES: [(u32, u32); 5] = [(1, 1), (5, 7), (13, 3), (16, 16), (37, 61)];
        let mut atlas = atlas_of(&[AtlasFormat::Bc7]);
        let frames: Vec<_> = (0..20).map(|index| {
            let (width, height) = ODD_SIZES[index % ODD_SIZES.len()];
            (width, height, Arc::from(vec![0u8; AtlasFormat::Bc7.frame_bytes(width, height)]))
        }).collect();
        let (rects, _) = atlas.place_clip(1, AtlasFormat::Bc7, &frames, 0.0).unwrap();
        for (rect, (width, height, _)) in rects.iter().zip(&frames) {
            assert_eq!(rect.format, AtlasFormat::Bc7);
            assert_eq!((rect.x % BLOCK_SIDE, rect.y % BLOCK_SIDE), (0, 0), "{rect:?}");
            assert_eq!((rect.width, rect.height), (*width, *height));
            assert_eq!(rect.padded_size(), (width.next_multiple_of(BLOCK_SIDE), height.next_multiple_of(BLOCK_SIDE)));
        }
        assert!(padded_apart(&rects, BLOCK_SIDE));
        assert_eq!(AtlasFormat::Bc7.gutter(), BLOCK_SIDE);
        // An RGBA8 atlas keeps its texel alignment and one texel of gutter.
        assert_eq!((AtlasFormat::Rgba8.align(), AtlasFormat::Rgba8.gutter()), (1, FRAME_GUTTER));
        assert_eq!(AtlasFormat::Rgba8.padded(5, 7), (5, 7));
    }

    /// A BC7 frame's pixels are rows of 16-byte blocks covering its padded size, copied to its block-aligned corner;
    /// an RGBA8 frame's are rows of its own width, as before.
    #[test]
    fn bc7_uploads_copy_whole_blocks_and_rgba8_ones_their_texels() {
        let bc7 = AtlasRect { format: AtlasFormat::Bc7, layer: 2, x: 8, y: 12, width: 13, height: 6 };
        // 13 by 6 is 4 by 2 blocks: rows of 64 bytes, over 16 by 8 texels.
        assert_eq!(upload_layout(bc7), (4 * BC7_BLOCK_BYTES, (16, 8)));
        assert_eq!(AtlasFormat::Bc7.frame_bytes(13, 6), 4 * 2 * BC7_BLOCK_BYTES as usize);
        let rgba = AtlasRect { format: AtlasFormat::Rgba8, ..bc7 };
        assert_eq!(upload_layout(rgba), (13 * RGBA8_TEXEL_BYTES, (13, 6)));
        assert_eq!(AtlasFormat::Rgba8.frame_bytes(13, 6), (13 * 6 * RGBA8_TEXEL_BYTES) as usize);
        // The layer an instance names: the BC7 atlas binds as the second one.
        assert_eq!(rgba.instance_layer(), 2);
        assert_eq!(bc7.instance_layer(), 2 | ART_LAYER_ENHANCED_ATLAS_BIT);
    }

    /// Each package's format has its atlas; a format both share has one. Entries of one atlas never evict another's.
    #[test]
    fn each_format_has_its_atlas_and_the_formats_do_not_evict_each_other() {
        assert_eq!(AtlasFormat::of(PagePixelFormat::Rgba8888), AtlasFormat::Rgba8);
        assert_eq!(AtlasFormat::of(PagePixelFormat::Bc7), AtlasFormat::Bc7);
        assert_eq!(atlas_of(&[AtlasFormat::Rgba8, AtlasFormat::Rgba8]).atlases.len(), 1);
        let mut atlas = atlas_of(&[AtlasFormat::Rgba8, AtlasFormat::Bc7]);
        assert_eq!(atlas.images().map(|(format, _)| format).collect::<Vec<_>>(),
            vec![AtlasFormat::Rgba8, AtlasFormat::Bc7]);
        assert_ne!(atlas.image(AtlasFormat::Rgba8), atlas.image(AtlasFormat::Bc7));
        assert!(atlas_of(&[AtlasFormat::Rgba8]).place_clip(1, AtlasFormat::Bc7, &frames_of(AtlasFormat::Bc7, 1, 8),
            0.0).is_none());
        // Both full, Classic RGBA8 drawn earlier: a new Enhanced BC7 clip evicts a BC7 page only.
        for clip in 0..MOBILE_ATLAS_PAGES {
            atlas.place_clip(clip, AtlasFormat::Rgba8, &frames(FRAMES_PER_PAGE, 100), 0.0).unwrap();
            atlas.place_clip(100 + clip, AtlasFormat::Bc7, &frames_of(AtlasFormat::Bc7, 81, 100), 1.0 + f64::from(clip))
                .unwrap();
        }
        let (rects, evicted) =
            atlas.place_clip(999, AtlasFormat::Bc7, &frames_of(AtlasFormat::Bc7, 1, 100), 20.0).unwrap();
        assert_eq!(evicted, vec![100]);
        assert_eq!(rects[0].format, AtlasFormat::Bc7);
        assert!(atlas.atlases[0].pages.iter().all(|page| page.clips.len() == 1));
        // Idle pages empty in every atlas.
        assert_eq!(atlas.release_idle(IDLE_PAGE_RELEASE_SECS + 0.5).len(), MOBILE_ATLAS_PAGES as usize);
    }
}
