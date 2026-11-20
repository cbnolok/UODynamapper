use std::ops::{Deref, DerefMut};

use bevy::prelude::*;
use color_eyre::eyre;

use udd_assets::bc7::{self, ImageExtent, TextureUploadLayout, VramTextureFormat};
use udd_assets::{
    tex_art_cc::{TexArtCcPackage, PagePixelFormat},
    tex_art_ec::TexArtEcPackage,
    tex_aux_ec::TexAuxEcPackage,
    tex_land_ec::TexLandEcPackage,
};
use bevy::platform::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use bevy::tasks::{AsyncComputeTaskPool, TaskPool};
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::Extract;


#[derive(Clone, Copy, Debug)]
pub struct ResolvedArtSprite {
    /// The page the sprite is on, which gets its layer when the sprite is drawn (`ArtPageAtlas::bind_page`).
    pub page: ArtPageKey,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    pub logical_width: f32,
    pub logical_height: f32,
    pub offset_x: i16,
    pub offset_y: i16,
    pub source_pixel_ratio: f32,
    pub legacy_art: bool,
}

/// The page bytes uploaded in one frame, past the first page: an Enhanced page (32 MiB), whose copy into the queue
/// costs 6-10 ms. The pages decoded together, as a walk enters a town, came to 17-20 ms in one frame (2026-11-19).
const MAX_PAGE_UPLOAD_BYTES_PER_FRAME: usize = 32 << 20;

#[derive(Debug, Clone)]
pub struct ArtPageUpload {
    pub page_index: ArtPageKey,
    pub layer: u32,
    /// Shared, so that the extraction hands the render world the page without copying it: an Enhanced page is
    /// 32 MiB, whose copy cost 15-19 ms in the frame (2026-11-19).
    pub bytes: Arc<Vec<u8>>,
    pub upload_layout: TextureUploadLayout,
    pub upload_width: u32,
    pub upload_height: u32,
}

/// The resident layers of an atlas that grew, copied on the GPU from the image it replaced (`ArtPageAtlas::apply_resize`).
/// Holding `source` keeps the old image alive until the render world has copied it.
#[derive(Clone)]
pub struct ArtLayerCopy {
    pub source: Handle<Image>,
    pub destination: Handle<Image>,
    pub layers: u32,
}

/// A layer's source survives growth/reupload; equal page numbers from different
/// packages must never alias when Enhanced artwork falls back to Classic art.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtPageKey {
    Classic(u32),
    Enhanced(u32),
    Terrain(u32),
    /// The pages of the Kingdom Reborn item art and land (the sections of `tex_art_ec.uddp`, `tex_land_ec.uddp`), in the Enhanced
    /// layouts, which the statics that continue the Kingdom Reborn land draw.
    KrEnhanced(u32),
    KrTerrain(u32),
    Missing,
}

/// The packages the pages of `ArtPageKey` are read from (`ArtPageAtlas::bind_page`).
#[derive(Clone, Copy, Default)]
pub struct ArtPageSources<'a> {
    pub classic: Option<&'a Arc<TexArtCcPackage>>,
    pub enhanced: Option<&'a Arc<TexArtEcPackage>>,
    pub kr_enhanced: Option<&'a Arc<TexArtEcPackage>>,
    pub terrain: Option<&'a Arc<TexLandEcPackage>>,
    pub kr_terrain: Option<&'a Arc<TexLandEcPackage>>,
    pub missing: Option<&'a Arc<TexAuxEcPackage>>,
}

/// What uploading a page takes: its package's page size, the part of it used, its format and its read.
struct ArtPageRequest {
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
    read: Box<dyn FnOnce() -> eyre::Result<Vec<u8>> + Send>,
}

fn art_page_request(page: ArtPageKey, sources: ArtPageSources<'_>) -> Option<ArtPageRequest> {
    match page {
        ArtPageKey::Classic(index) => {
            let package = sources.classic?;
            let record = package.pages().get(index as usize)?;
            Some(ArtPageRequest {
                atlas_width: package.atlas_width(),
                atlas_height: package.atlas_height(),
                used_width: record.used_width,
                used_height: record.used_height,
                pixel_format: record.pixel_format,
                read: Box::new(read_page_off_frame(package, index, |package, page| package.read_page_bytes(page))),
            })
        }
        ArtPageKey::Enhanced(index) | ArtPageKey::KrEnhanced(index) => {
            let package = if matches!(page, ArtPageKey::Enhanced(_)) { sources.enhanced? } else { sources.kr_enhanced? };
            let record = package.pages().get(index as usize)?;
            Some(ArtPageRequest {
                atlas_width: package.atlas_width(),
                atlas_height: package.atlas_height(),
                used_width: record.used_width,
                used_height: record.used_height,
                pixel_format: record.pixel_format,
                read: Box::new(read_page_off_frame(package, index, |package, page| package.read_page_bytes(page))),
            })
        }
        ArtPageKey::Terrain(index) | ArtPageKey::KrTerrain(index) => {
            let package = if matches!(page, ArtPageKey::Terrain(_)) { sources.terrain? } else { sources.kr_terrain? };
            let record = package.pages().get(index as usize)?;
            Some(ArtPageRequest {
                atlas_width: package.atlas_width(),
                atlas_height: package.atlas_height(),
                used_width: record.used_width,
                used_height: record.used_height,
                pixel_format: record.pixel_format,
                read: Box::new(read_page_off_frame(package, index, |package, page| package.read_page_bytes(page))),
            })
        }
        ArtPageKey::Missing => {
            let aux = sources.missing?;
            let slot = aux.missing_texture_slot();
            let (width, height) = (u32::from(slot.width), u32::from(slot.height));
            Some(ArtPageRequest {
                atlas_width: width,
                atlas_height: height,
                used_width: width,
                used_height: height,
                pixel_format: PagePixelFormat::Rgba8888,
                read: Box::new(read_page_off_frame(aux, slot.payload_texture_id, |aux, id| aux.load_rgba(id)?
                    .ok_or_else(|| eyre::eyre!("missing-texture payload is unavailable")))),
            })
        }
    }
}

/// The atlases of a set (`ArtPageAtlasSet`), one per page content: the Classic art pages, then the Enhanced ones
/// (the Enhanced art and land, and their Kingdom Reborn sections), each at its packages' page size and format.
pub const ART_ATLAS_SLOTS: usize = 2;
pub const CLASSIC_ATLAS_SLOT: usize = 0;
pub const ENHANCED_ATLAS_SLOT: usize = 1;

/// Set on the layer `ArtPageAtlasSet::bind_page` gives a page of the Enhanced atlas; the shaders read it as
/// `ART_LAYER_ENHANCED_ATLAS_BIT` (art/atlas_pages.wesl).
pub const ART_LAYER_ENHANCED_ATLAS_BIT: u32 = 1 << 31;

/// The atlas of a set that holds `page`. The missing texture's small page joins the Classic pages.
pub fn art_atlas_slot(page: ArtPageKey) -> usize {
    match page {
        ArtPageKey::Classic(_) | ArtPageKey::Missing => CLASSIC_ATLAS_SLOT,
        ArtPageKey::Enhanced(_) | ArtPageKey::Terrain(_) | ArtPageKey::KrEnhanced(_) | ArtPageKey::KrTerrain(_) => {
            ENHANCED_ATLAS_SLOT
        }
    }
}

/// The art pages one material draws from, in an atlas per page content (`art_atlas_slot`), each in its packages'
/// format, so a lossless Classic package and BC7 Enhanced ones each upload as stored. One atlas for both had the
/// largest package's page size and, with formats that differ, held every page in RGBA8: a Classic page, which the
/// Enhanced views draw where the Enhanced art lacks a sprite, took an RGBA layer as large as an Enhanced page, 32 MiB
/// for its 16, and every BC7 page was decoded on the CPU at four times its size.
pub struct ArtPageAtlasSet {
    pub atlases: [ArtPageAtlas; ART_ATLAS_SLOTS],
}

impl ArtPageAtlasSet {
    /// The layer of `page` in its atlas, with `ART_LAYER_ENHANCED_ATLAS_BIT` for the Enhanced one: an instance's
    /// `layer` (`ArtPageAtlas::bind_page`).
    pub fn bind_page(&mut self, page: ArtPageKey, sources: ArtPageSources<'_>) -> Option<u32> {
        let slot = art_atlas_slot(page);
        let layer = self.atlases[slot].bind_page(page, sources)?;
        Some(if slot == ENHANCED_ATLAS_SLOT { layer | ART_LAYER_ENHANCED_ATLAS_BIT } else { layer })
    }

    /// The size of each atlas's pages, which places the sprites on them.
    pub fn geometry(&self) -> ArtPageGeometry {
        ArtPageGeometry { page_sizes: self.atlases.each_ref().map(|atlas| UVec2::new(atlas.page_width, atlas.page_height)) }
    }

    pub fn handles(&self) -> [Handle<Image>; ART_ATLAS_SLOTS] {
        self.atlases.each_ref().map(|atlas| atlas.gpu_handle.clone())
    }

    /// Changes whenever a page of any of the atlases loses its layer (`ArtPageAtlas::mapping_revision`).
    pub fn mapping_revision(&self) -> u64 {
        self.atlases.iter().map(ArtPageAtlas::mapping_revision).fold(0, u64::wrapping_add)
    }

    pub fn resident_page_count(&self) -> usize {
        self.atlases.iter().map(ArtPageAtlas::resident_page_count).sum()
    }

    pub fn pending_page_count(&self) -> usize {
        self.atlases.iter().map(ArtPageAtlas::pending_page_count).sum()
    }

    /// The layers of all the atlases.
    pub fn active_layers(&self) -> u32 {
        self.atlases.iter().map(|atlas| atlas.active_layers).sum()
    }

    fn stage_for_extract(&mut self) {
        for atlas in &mut self.atlases {
            atlas.stage_for_extract();
        }
    }
}

#[cfg(test)]
impl ArtPageAtlasSet {
    /// Atlases of one small RGBA layer each, holding no page.
    pub fn blank_for_tests() -> Self {
        const PAGE_SIZE: u32 = 64;
        Self { atlases: std::array::from_fn(|_| ArtPageAtlas::new(Handle::default(), PAGE_SIZE, PAGE_SIZE, 1, 1,
            PagePixelFormat::Rgba8888)) }
    }
}

pub struct ArtPageAtlas {
    pub gpu_handle: Handle<Image>,
    page_to_layer: HashMap<ArtPageKey, u32>,
    layer_to_page: Vec<Option<ArtPageKey>>,
    layer_access_tick: Vec<u64>,
    current_tick: u64,
    mapping_revision: u64,
    pub pending_uploads: Vec<ArtPageUpload>,
    pub extract_staging: Vec<ArtPageUpload>,
    pending_layer_copies: Vec<ArtLayerCopy>,
    extract_layer_copies: Vec<ArtLayerCopy>,
    pub page_width: u32,
    pub page_height: u32,
    pub active_layers: u32,
    pub max_layers: u32,
    pub pixel_format: PagePixelFormat,
    pub requested_resize_to: Option<u32>,
    /// The pages being read and decoded off the frame (`queue_page_upload`); they wait like a staged upload.
    decoding: HashSet<ArtPageKey>,
    /// Bumped when the atlas drops its pages (`apply_resize` shrinking it), so the pages decoded for the old layers
    /// are dropped.
    decode_generation: u64,
    decoded_sender: mpsc::Sender<DecodedArtPage>,
    decoded_receiver: Mutex<mpsc::Receiver<DecodedArtPage>>,
}

/// A page decoded off the frame: its upload, or `None` when it could not be read.
struct DecodedArtPage {
    generation: u64,
    page_index: ArtPageKey,
    upload: Option<ArtPageUpload>,
}

impl ArtPageAtlas {
    pub fn new(
        gpu_handle: Handle<Image>,
        page_width: u32,
        page_height: u32,
        active_layers: u32,
        max_layers: u32,
        pixel_format: PagePixelFormat,
    ) -> Self {
        let (decoded_sender, decoded_receiver) = mpsc::channel();
        Self {
            gpu_handle,
            page_to_layer: HashMap::new(),
            layer_to_page: vec![None; active_layers as usize],
            layer_access_tick: vec![0; active_layers as usize],
            current_tick: 0,
            mapping_revision: 0,
            pending_uploads: Vec::new(),
            extract_staging: Vec::new(),
            pending_layer_copies: Vec::new(),
            extract_layer_copies: Vec::new(),
            page_width,
            page_height,
            active_layers,
            max_layers,
            pixel_format,
            requested_resize_to: None,
            decoding: HashSet::new(),
            decode_generation: 0,
            decoded_sender,
            decoded_receiver: Mutex::new(decoded_receiver),
        }
    }

    pub fn request_growth(&mut self) -> Option<u32> {
        if self.active_layers >= self.max_layers {
            return None;
        }

        let target_layers = (((self.active_layers as f32) * 1.5).ceil() as u32)
            .max(self.active_layers.saturating_add(1))
            .min(self.max_layers);
        self.requested_resize_to = Some(
            self.requested_resize_to
                .unwrap_or(target_layers)
                .max(target_layers),
        );
        self.requested_resize_to
    }

    pub fn take_resize_request(&mut self) -> Option<u32> {
        self.requested_resize_to.take()
    }

    /// Reads and decodes the page off the frame, on the async compute pool: it waits in `decoding` until
    /// `receive_decoded_pages` takes it, so a sprite on it resolves only once it is uploaded.
    pub fn queue_page_upload<F>(
        &mut self,
        page_index: ArtPageKey,
        layer: u32,
        atlas_width: u32,
        atlas_height: u32,
        used_width: u32,
        used_height: u32,
        pixel_format: PagePixelFormat,
        read_page_bytes: F,
    ) where
        F: FnOnce() -> eyre::Result<Vec<u8>> + Send + 'static,
    {
        if self.page_upload_pending(page_index) {
            return;
        }

        self.decoding.insert(page_index);
        let (sender, generation, atlas_pixel_format) = (self.decoded_sender.clone(), self.decode_generation, self.pixel_format);
        AsyncComputeTaskPool::get_or_init(TaskPool::default).spawn(async move {
            let upload = read_page_bytes().ok().and_then(|page_data| prepare_compatible_page_upload(
                page_data,
                atlas_width,
                atlas_height,
                used_width,
                used_height,
                pixel_format,
                atlas_pixel_format,
            )).map(|(bytes, upload_layout, upload_width, upload_height)| ArtPageUpload {
                page_index,
                layer,
                bytes: Arc::new(bytes),
                upload_layout,
                upload_width,
                upload_height,
            });
            let _ = sender.send(DecodedArtPage { generation, page_index, upload });
        }).detach();
    }

    /// Whether `page_index` is being decoded, or waits to be uploaded.
    fn page_upload_pending(&self, page_index: ArtPageKey) -> bool {
        self.decoding.contains(&page_index)
            || self.pending_uploads.iter().any(|upload| upload.page_index == page_index)
            || self.extract_staging.iter().any(|upload| upload.page_index == page_index)
    }

    /// Takes the pages decoded since the last call: each is uploaded when its layer is still its own; one decoded
    /// for layers the atlas dropped, or whose layer went to another page meanwhile, is dropped.
    fn receive_decoded_pages(&mut self) {
        let decoded: Vec<DecodedArtPage> = self.decoded_receiver.lock().unwrap().try_iter().collect();
        for page in decoded {
            self.accept_decoded_page(page);
        }
    }

    fn accept_decoded_page(&mut self, page: DecodedArtPage) {
        if page.generation != self.decode_generation {
            return;
        }
        self.decoding.remove(&page.page_index);
        if let Some(upload) = page.upload
            && self.layer_to_page.get(upload.layer as usize) == Some(&Some(page.page_index))
        {
            self.pending_uploads.push(upload);
        }
    }

    /// Waits for every page being decoded (tests only: the frame never waits).
    #[cfg(test)]
    fn finish_decodes(&mut self) {
        while !self.decoding.is_empty() {
            let page = self.decoded_receiver.lock().unwrap().recv().expect("the atlas holds a sender");
            self.accept_decoded_page(page);
        }
    }

    pub fn apply_resize(&mut self, new_handle: Handle<Image>, new_layers: u32) {
        if new_layers == self.active_layers {
            self.requested_resize_to = None;
            return;
        }

        let old_layers = self.active_layers;
        if new_layers > old_layers {
            let staged_uploads = std::mem::take(&mut self.extract_staging);
            self.pending_layer_copies.push(ArtLayerCopy {
                source: self.gpu_handle.clone(),
                destination: new_handle.clone(),
                layers: old_layers,
            });
            self.gpu_handle = new_handle;
            self.active_layers = new_layers;
            self.layer_to_page.resize(new_layers as usize, None);
            self.layer_access_tick.resize(new_layers as usize, 0);
            self.requested_resize_to = None;

            for upload in staged_uploads {
                if !self
                    .pending_uploads
                    .iter()
                    .any(|pending| pending.page_index == upload.page_index)
                {
                    self.pending_uploads.push(upload);
                }
            }
            return;
        }

        self.gpu_handle = new_handle;
        self.active_layers = new_layers;
        self.layer_to_page = vec![None; new_layers as usize];
        self.layer_access_tick = vec![0; new_layers as usize];
        self.page_to_layer.clear();
        self.pending_uploads.clear();
        self.extract_staging.clear();
        self.pending_layer_copies.clear();
        self.extract_layer_copies.clear();
        self.decoding.clear();
        self.decode_generation = self.decode_generation.wrapping_add(1);
        self.current_tick = 0;
        self.mapping_revision = self.mapping_revision.saturating_add(1);
        self.requested_resize_to = None;
    }

    /// Hands this frame's uploads and layer copies to the extraction, in place of the last frame's: the uploads in
    /// the order they came, up to `MAX_PAGE_UPLOAD_BYTES_PER_FRAME` (always the first), the rest waiting for the next
    /// frames. An upload whose layer went to another page meanwhile is dropped.
    fn stage_for_extract(&mut self) {
        self.receive_decoded_pages();
        self.extract_staging.clear();
        let layer_to_page = &self.layer_to_page;
        self.pending_uploads.retain(|upload| layer_to_page.get(upload.layer as usize) == Some(&Some(upload.page_index)));
        let mut staged_bytes = 0;
        let staged = self.pending_uploads.iter()
            .take_while(|upload| {
                let first = staged_bytes == 0;
                staged_bytes += upload.bytes.len();
                first || staged_bytes <= MAX_PAGE_UPLOAD_BYTES_PER_FRAME
            })
            .count();
        self.extract_staging.extend(self.pending_uploads.drain(..staged));
        self.extract_layer_copies = std::mem::take(&mut self.pending_layer_copies);
    }

    pub fn mapping_revision(&self) -> u64 {
        self.mapping_revision
    }
}

/// The size of the pages of a set's atlases, by slot (`ArtPageAtlasSet::geometry`): the sprites' places on them, read
/// without the atlases, as a build off the main thread does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArtPageGeometry {
    pub page_sizes: [UVec2; ART_ATLAS_SLOTS],
}

impl ArtPageGeometry {
    /// The sprite of `art_id` in `tex_art_cc`, its page not yet bound to a layer (`ArtPageAtlas::bind_page`).
    pub fn sprite_cc(&self, tex_art_cc: &TexArtCcPackage, art_id: u32) -> Option<ResolvedArtSprite> {
        let slot = tex_art_cc.present_slot(art_id)?;
        let mut resolved = self.slot_sprite(
            ArtPageKey::Classic(slot.page_index),
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            slot.logical_width(),
            slot.logical_height(),
            slot.draw_offset_x,
            slot.draw_offset_y,
            tex_art_cc.atlas_width(),
            tex_art_cc.atlas_height(),
        )?;
        resolved.legacy_art = true;
        Some(resolved)
    }

    /// The sprite of `art_id` in a package in the Enhanced art layout whose pages are `page_key`: `tex_art_ec.uddp`
    /// (`ArtPageKey::Enhanced`) or its Kingdom Reborn art (`ArtPageKey::KrEnhanced`).
    pub fn sprite_ec_pages(&self, tex_art_ec: &TexArtEcPackage, art_id: u32, page_key: fn(u32) -> ArtPageKey)
        -> Option<ResolvedArtSprite> {
        let slot = tex_art_ec.present_slot(art_id)?;
        let mut resolved = self.slot_sprite(
            page_key(slot.page_index),
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            slot.logical_width(),
            slot.logical_height(),
            slot.draw_offset_x,
            slot.draw_offset_y,
            tex_art_ec.atlas_width(),
            tex_art_ec.atlas_height(),
        )?;
        let geometry = tex_art_ec.sprite_geometry(art_id)?;
        resolved.source_pixel_ratio = geometry.legacy_pixel_ratio();
        Some(resolved)
    }

    /// The sprite of `art_id` in a package in the Enhanced land layout whose pages are `page_key`: `tex_land_ec.uddp`
    /// (`ArtPageKey::Terrain`) or its Kingdom Reborn land (`ArtPageKey::KrTerrain`).
    pub fn sprite_tex_land_ec_pages(
        &self,
        tex_land_ec: &TexLandEcPackage,
        art_id: u32,
        page_key: fn(u32) -> ArtPageKey,
    ) -> Option<ResolvedArtSprite> {
        let slot = tex_land_ec.present_slot(art_id)?;
        self.slot_sprite(
            page_key(slot.page_index),
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            slot.width as f32,
            slot.height as f32,
            0,
            0,
            tex_land_ec.atlas_width(),
            tex_land_ec.atlas_height(),
        )
    }

    /// The missing texture's sprite (`TexAuxEcPackage::missing_texture_slot`).
    pub fn sprite_missing(&self, aux: &TexAuxEcPackage) -> Option<ResolvedArtSprite> {
        let slot = aux.missing_texture_slot();
        self.slot_sprite(ArtPageKey::Missing, 0, 0, slot.width, slot.height,
            f32::from(slot.width), f32::from(slot.height), -(slot.width as i32 / 2) as i16, 0,
            u32::from(slot.width), u32::from(slot.height))
    }

    /// The sprite at (`x`, `y`) of `page`, placed on the pages of its atlas (`art_atlas_slot`); `None` when its
    /// package's pages (`atlas_width`×`atlas_height`) are larger.
    fn slot_sprite(
        &self,
        page: ArtPageKey,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        logical_width: f32,
        logical_height: f32,
        offset_x: i16,
        offset_y: i16,
        atlas_width: u32,
        atlas_height: u32,
    ) -> Option<ResolvedArtSprite> {
        let page_size = self.page_sizes[art_atlas_slot(page)];
        if atlas_width > page_size.x || atlas_height > page_size.y {
            return None;
        }
        let page_size = page_size.as_vec2();
        Some(ResolvedArtSprite {
            page,
            uv_min: Vec2::new(f32::from(x), f32::from(y)) / page_size,
            uv_max: Vec2::new(f32::from(x + width), f32::from(y + height)) / page_size,
            logical_width,
            logical_height,
            offset_x,
            offset_y,
            source_pixel_ratio: 1.0,
            legacy_art: false,
        })
    }
}

impl ArtPageAtlas {
    /// The layer of `page`, for a sprite drawn on it now (marked used, for the LRU eviction); `None` while the page is
    /// read, decoded or uploaded, which this starts when the page has no layer yet: in a free layer, else by growing
    /// the atlas, else in the least recently used page's layer. `None` also when `sources` lack the page's package.
    pub fn bind_page(&mut self, page: ArtPageKey, sources: ArtPageSources<'_>) -> Option<u32> {
        if let Some(&layer) = self.page_to_layer.get(&page) {
            self.current_tick += 1;
            self.layer_access_tick[layer as usize] = self.current_tick;
            return (!self.page_upload_pending(page)).then_some(layer);
        }
        if self.page_upload_pending(page) {
            return None;
        }
        let request = art_page_request(page, sources)?;
        // A page larger than the atlas's never binds (`ArtPageGeometry::slot_sprite` places no sprite on it), nor a
        // package page in another format: a Kingdom Reborn section packed apart from its Enhanced package does not load
        // (`content::kr_section_page_mismatch`). The missing texture takes the atlas's format.
        if request.atlas_width > self.page_width || request.atlas_height > self.page_height
            || (request.pixel_format != self.pixel_format && page != ArtPageKey::Missing) {
            return None;
        }
        self.page_layer(page, request.atlas_width, request.atlas_height, request.used_width, request.used_height,
            request.pixel_format, request.read)
    }

    fn page_layer<F>(
        &mut self,
        page_index: ArtPageKey,
        atlas_width: u32,
        atlas_height: u32,
        used_width: u32,
        used_height: u32,
        pixel_format: PagePixelFormat,
        read_page_bytes: F,
    ) -> Option<u32>
    where
        F: FnOnce() -> eyre::Result<Vec<u8>> + Send + 'static,
    {
        let page_upload_pending = self.page_upload_pending(page_index);

        self.current_tick += 1;

        if let Some(&layer) = self.page_to_layer.get(&page_index) {
            self.layer_access_tick[layer as usize] = self.current_tick;
            return (!page_upload_pending).then_some(layer);
        }
        // Need to allocate a layer for this page. Check if we already requested it.
        if page_upload_pending {
            return None; // Still uploading
        }

        // Find free layer or evict LRU
        let layer = if let Some(free_layer) = self.layer_to_page.iter().position(|p| p.is_none()) {
            free_layer as u32
        } else if self.active_layers < self.max_layers {
            self.request_growth();
            return None;
        } else {
            let lru_layer = self.layer_access_tick.iter().enumerate().min_by_key(|&(_, &tick)| tick).map(|(i, _)| i).unwrap() as u32;
            if let Some(evicted_page) = self.layer_to_page[lru_layer as usize] {
                self.page_to_layer.remove(&evicted_page);
                self.mapping_revision = self.mapping_revision.saturating_add(1);
            }
            lru_layer
        };

        self.queue_page_upload(
            page_index,
            layer,
            atlas_width,
            atlas_height,
            used_width,
            used_height,
            pixel_format,
            read_page_bytes,
        );

        self.page_to_layer.insert(page_index, layer);
        self.layer_to_page[layer as usize] = Some(page_index);
        self.layer_access_tick[layer as usize] = self.current_tick;

        None // Wait for upload
    }

    pub fn resident_page_count(&self) -> usize {
        self.page_to_layer.len()
    }

    /// The pages being decoded or waiting to be uploaded.
    pub fn pending_page_count(&self) -> usize {
        self.decoding.len() + self.pending_uploads.len() + self.extract_staging.len()
    }
}

/// The read of page `page` of `package`, to run on the async compute pool (`ArtPageAtlas::queue_page_upload`).
fn read_page_off_frame<P: Send + Sync + 'static, K: Send + 'static>(
    package: &Arc<P>,
    page: K,
    read: fn(&P, K) -> eyre::Result<Vec<u8>>,
) -> impl FnOnce() -> eyre::Result<Vec<u8>> + Send + 'static {
    let package = Arc::clone(package);
    move || read(&package, page)
}

fn page_pixel_vram_format(pixel_format: PagePixelFormat) -> VramTextureFormat {
    match pixel_format {
        PagePixelFormat::Rgba8888 => VramTextureFormat::Rgba8UnormSrgb,
        PagePixelFormat::Bc7 => VramTextureFormat::Bc7RgbaUnormSrgb,
    }
}

/// The page's upload as stored: its bytes, layout and extent, the part used of an RGBA page, a BC7 page whole.
fn prepare_page_upload_bytes(
    page_data: Vec<u8>,
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
) -> Option<(Vec<u8>, TextureUploadLayout, u32, u32)> {
    let (width, height) = match pixel_format {
        PagePixelFormat::Rgba8888 => (used_width.max(1), used_height.max(1)),
        PagePixelFormat::Bc7 => (atlas_width.max(1), atlas_height.max(1)),
    };
    let extent = ImageExtent::new(width, height).ok()?;
    let format = page_pixel_vram_format(pixel_format);
    if page_data.len() != format.expected_byte_len(extent) {
        return None;
    }

    Some((page_data, format.upload_layout(extent), width, height))
}

/// The page's upload into an atlas of `target` format: as stored, the format of every package page its atlas binds
/// (`ArtPageAtlas::bind_page`), or for the missing texture, which has none, its RGBA8 encoded to the atlas's BC7.
fn prepare_compatible_page_upload(data: Vec<u8>, atlas_width: u32, atlas_height: u32,
    used_width: u32, used_height: u32, source: PagePixelFormat, target: PagePixelFormat,
) -> Option<(Vec<u8>, TextureUploadLayout, u32, u32)> {
    match (source, target) {
        _ if source == target => prepare_page_upload_bytes(data, atlas_width, atlas_height, used_width, used_height,
            source),
        (PagePixelFormat::Rgba8888, PagePixelFormat::Bc7) => {
            let extent = ImageExtent::new(used_width, used_height).ok()?;
            let texture = bc7::encode_for_vram(&data, extent, bc7::RawImageFormat::Rgba8888,
                bc7::VramTextureEncoding::Bc7(bc7::preferred_bc7_encoder_backend())).ok()?;
            Some((texture.bytes().to_vec(), texture.upload_layout(), used_width, used_height))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_sprite_pages_do_not_alias_and_keep_source_keys_after_growth() {
        let mut atlas = ArtPageAtlas::new(Handle::default(), 4, 4, 2, 4, PagePixelFormat::Rgba8888);
        for (key, value) in [(ArtPageKey::Classic(0), 31), (ArtPageKey::Enhanced(0), 127)] {
            assert!(atlas.page_layer(key, 4, 4, 4, 4, PagePixelFormat::Rgba8888, move || Ok(vec![value; 64])).is_none());
        }
        atlas.finish_decodes();
        assert_eq!(atlas.pending_uploads.len(), 2);
        atlas.pending_uploads.sort_by_key(|upload| upload.bytes[0]);
        assert_ne!(atlas.pending_uploads[0].layer, atlas.pending_uploads[1].layer);
        assert_eq!(*atlas.pending_uploads[0].bytes, vec![31; 64]);
        assert_eq!(*atlas.pending_uploads[1].bytes, vec![127; 64]);
        let keys = atlas.page_to_layer.clone();
        atlas.apply_resize(Handle::default(), 3);
        assert_eq!(atlas.page_to_layer.len(), 2);
        for (key, layer) in keys { assert_eq!(atlas.page_to_layer[&key], layer); }
    }

    #[test]
    fn page_reads_leave_the_frame_and_a_reset_drops_their_pages() {
        let mut atlas = ArtPageAtlas::new(Handle::default(), 4, 4, 2, 4, PagePixelFormat::Rgba8888);
        let (release, wait) = mpsc::channel::<()>();
        let resolve = |atlas: &mut ArtPageAtlas, key, wait: mpsc::Receiver<()>| atlas.page_layer(key, 4, 4, 4, 4,
            PagePixelFormat::Rgba8888, move || { let _ = wait.recv(); Ok(vec![7; 64]) });
        // The read blocks until released, yet the resolve returns: the page waits as pending, not on the frame.
        assert!(resolve(&mut atlas, ArtPageKey::Enhanced(0), wait).is_none());
        assert_eq!(atlas.pending_page_count(), 1);
        release.send(()).unwrap();
        atlas.finish_decodes();
        assert_eq!((atlas.pending_uploads.len(), atlas.pending_page_count()), (1, 1));

        // A page still decoding when the atlas drops its pages is not uploaded into the new layers.
        let (release, wait) = mpsc::channel::<()>();
        assert!(resolve(&mut atlas, ArtPageKey::Enhanced(1), wait).is_none());
        atlas.apply_resize(Handle::default(), 1);
        release.send(()).unwrap();
        let page = atlas.decoded_receiver.lock().unwrap().recv().expect("the atlas holds a sender");
        atlas.accept_decoded_page(page);
        assert_eq!(atlas.pending_page_count(), 0);
    }

    #[test]
    fn request_growth_uses_ceil_1p5x_and_clamps_to_max_layers() {
        let mut atlas = ArtPageAtlas::new(
            Handle::default(),
            64,
            64,
            4,
            5,
            PagePixelFormat::Rgba8888,
        );

        assert_eq!(atlas.request_growth(), Some(5));
        assert_eq!(atlas.take_resize_request(), Some(5));
    }

    #[test]
    fn apply_resize_growth_preserves_resident_pages_and_staged_uploads() {
        let mut atlas = ArtPageAtlas::new(
            Handle::default(),
            64,
            64,
            2,
            8,
            PagePixelFormat::Rgba8888,
        );
        atlas.page_to_layer.insert(ArtPageKey::Enhanced(7), 1);
        atlas.layer_to_page[1] = Some(ArtPageKey::Enhanced(7));
        atlas.layer_access_tick[1] = 11;
        atlas.current_tick = 23;
        atlas.extract_staging.push(ArtPageUpload {
            page_index: ArtPageKey::Enhanced(7),
            layer: 1,
            bytes: vec![1, 2, 3, 4].into(),
            upload_layout: TextureUploadLayout {
                bytes_per_row: 4,
                rows_per_image: 1,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
            },
            upload_width: 1,
            upload_height: 1,
        });

        let (old, grown) = (atlas.gpu_handle.clone(), bevy::asset::uuid_handle!("6d1a2f0e-58c4-4b7e-9f3a-0c2b7e91d4a5"));
        atlas.apply_resize(grown.clone(), 3);

        assert_eq!(atlas.active_layers, 3);
        assert_eq!(atlas.pending_layer_copies.len(), 1);
        let copy = &atlas.pending_layer_copies[0];
        assert_eq!((&copy.source, &copy.destination, copy.layers), (&old, &grown, 2));
        assert_eq!(atlas.page_to_layer.get(&ArtPageKey::Enhanced(7)), Some(&1));
        assert_eq!(atlas.layer_to_page[1], Some(ArtPageKey::Enhanced(7)));
        assert_eq!(atlas.layer_to_page[2], None);
        assert_eq!(atlas.layer_access_tick[1], 11);
        assert_eq!(atlas.current_tick, 23);
        assert_eq!(atlas.pending_uploads.len(), 1);
        assert!(atlas.extract_staging.is_empty());
    }

    #[test]
    fn staging_uploads_a_frame_budget_in_order_and_drops_evicted_layers() {
        let mut atlas = ArtPageAtlas::new(Handle::default(), 64, 64, 4, 4, PagePixelFormat::Rgba8888);
        let upload = |page: u32, layer: u32, bytes: usize| ArtPageUpload {
            page_index: ArtPageKey::Enhanced(page),
            layer,
            bytes: vec![0; bytes].into(),
            upload_layout: TextureUploadLayout { bytes_per_row: 4, rows_per_image: 1, format: wgpu::TextureFormat::Rgba8UnormSrgb },
            upload_width: 1,
            upload_height: 1,
        };
        let half = MAX_PAGE_UPLOAD_BYTES_PER_FRAME / 2;
        for layer in 0..4 {
            atlas.layer_to_page[layer as usize] = Some(ArtPageKey::Enhanced(layer));
        }
        // Layer 1 went to page 9 after its upload was queued; a page over the budget alone still goes.
        atlas.layer_to_page[1] = Some(ArtPageKey::Enhanced(9));
        atlas.pending_uploads = vec![upload(0, 0, half), upload(1, 1, half), upload(2, 2, half), upload(3, 3, 3 * half)];

        let staged_pages = |atlas: &ArtPageAtlas| atlas.extract_staging.iter().map(|upload| upload.page_index).collect::<Vec<_>>();
        atlas.stage_for_extract();
        assert_eq!(staged_pages(&atlas), [ArtPageKey::Enhanced(0), ArtPageKey::Enhanced(2)]);
        atlas.stage_for_extract();
        assert_eq!(staged_pages(&atlas), [ArtPageKey::Enhanced(3)]);
        atlas.stage_for_extract();
        assert!(staged_pages(&atlas).is_empty());
    }

    #[test]
    fn prepare_page_upload_bytes_uses_bc7_layout_for_bc7_pages() {
        let extent = ImageExtent::new(8, 8).unwrap();
        let bytes = vec![0; VramTextureFormat::Bc7RgbaUnormSrgb.expected_byte_len(extent)];

        let (_, layout, width, height) = prepare_page_upload_bytes(bytes, 8, 8, 5, 7, PagePixelFormat::Bc7).unwrap();

        assert_eq!((width, height), (8, 8));
        assert_eq!(layout.bytes_per_row, 32);
        assert_eq!(layout.rows_per_image, 2);
        assert_eq!(layout.format, wgpu::TextureFormat::Bc7RgbaUnormSrgb);
    }

    /// A sprite lands on the pages of its page's atlas, its uv over their size; a package of larger pages has none.
    #[test]
    fn sprites_are_placed_on_their_atlas_pages() {
        let geometry = ArtPageGeometry { page_sizes: [UVec2::new(2048, 2048), UVec2::new(4096, 2048)] };
        let place = |page, x, atlas_width| geometry.slot_sprite(page, x, 0, 16, 16, 16.0, 16.0, 0, 0, atlas_width, 2048)
            .map(|sprite| sprite.uv_min.x);
        assert_eq!(place(ArtPageKey::KrEnhanced(0), 2048, 4096), Some(0.5));
        assert_eq!(place(ArtPageKey::Classic(0), 1024, 2048), Some(0.5));
        assert_eq!(place(ArtPageKey::Classic(0), 0, 4096), None);
        for (page, slot) in [(ArtPageKey::Classic(3), CLASSIC_ATLAS_SLOT), (ArtPageKey::Missing, CLASSIC_ATLAS_SLOT),
            (ArtPageKey::Enhanced(3), ENHANCED_ATLAS_SLOT), (ArtPageKey::Terrain(3), ENHANCED_ATLAS_SLOT),
            (ArtPageKey::KrEnhanced(3), ENHANCED_ATLAS_SLOT), (ArtPageKey::KrTerrain(3), ENHANCED_ATLAS_SLOT)] {
            assert_eq!(art_atlas_slot(page), slot, "{page:?}");
        }
    }

    #[test]
    fn pages_upload_as_stored_and_only_the_missing_texture_is_encoded() {
        let extent = ImageExtent::new(8, 8).unwrap();
        let rgba: Vec<_> = (0..64).flat_map(|i| [i * 3, i, 117, 255]).collect();
        let encoded = bc7::encode_for_vram(&rgba, extent, bc7::RawImageFormat::Rgba8888,
            bc7::VramTextureEncoding::Bc7(bc7::preferred_bc7_encoder_backend())).unwrap();
        let (native, layout, width, height) = prepare_compatible_page_upload(encoded.bytes().to_vec(),
            8, 8, 5, 7, PagePixelFormat::Bc7, PagePixelFormat::Bc7).unwrap();
        assert_eq!(native, encoded.bytes());
        assert_eq!((width, height), (8, 8));
        assert_eq!(layout.format, wgpu::TextureFormat::Bc7RgbaUnormSrgb);
        // No BC7 page is decoded into an RGBA atlas.
        assert!(prepare_compatible_page_upload(encoded.bytes().to_vec(), 8, 8, 5, 7, PagePixelFormat::Bc7,
            PagePixelFormat::Rgba8888).is_none());
        let (missing, layout, _, _) = prepare_compatible_page_upload(rgba,
            8, 8, 8, 8, PagePixelFormat::Rgba8888, PagePixelFormat::Bc7).unwrap();
        assert_eq!(missing, encoded.bytes());
        assert_eq!(layout.format, wgpu::TextureFormat::Bc7RgbaUnormSrgb);
    }
}

#[derive(Resource)]
pub struct SpriteArtPageAtlas(pub ArtPageAtlasSet);

impl Deref for SpriteArtPageAtlas {
    type Target = ArtPageAtlasSet;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for SpriteArtPageAtlas {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Resource)]
pub struct GroundArtPageAtlas(pub ArtPageAtlasSet);

impl Deref for GroundArtPageAtlas {
    type Target = ArtPageAtlasSet;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GroundArtPageAtlas {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// The render world's share of an atlas of a set: the layer copies of its growths, then the page uploads.
#[derive(Default)]
pub struct RenderArtPageWork {
    uploads: Vec<ArtPageUpload>,
    layer_copies: Vec<ArtLayerCopy>,
}

impl RenderArtPageWork {
    fn extend_from(&mut self, atlas: &ArtPageAtlas) {
        self.uploads.extend(atlas.extract_staging.iter().cloned());
        self.layer_copies.extend(atlas.extract_layer_copies.iter().cloned());
    }

    /// The share of each atlas of `set`, by slot.
    fn extend_from_set(works: &mut [Self; ART_ATLAS_SLOTS], set: &ArtPageAtlasSet) {
        for (work, atlas) in works.iter_mut().zip(&set.atlases) {
            work.extend_from(atlas);
        }
    }
}

#[derive(Resource, Default)]
pub struct RenderSpriteArtPageUploads(pub [RenderArtPageWork; ART_ATLAS_SLOTS]);

#[derive(Resource, Default)]
pub struct RenderGroundArtPageUploads(pub [RenderArtPageWork; ART_ATLAS_SLOTS]);

pub fn sys_stage_sprite_art_page_uploads(mut atlas: ResMut<SpriteArtPageAtlas>) {
    atlas.stage_for_extract();
}

pub fn sys_stage_ground_art_page_uploads(mut atlas: ResMut<GroundArtPageAtlas>) {
    atlas.stage_for_extract();
}

pub fn sys_extract_sprite_art_page_uploads(
    atlas: Extract<Res<SpriteArtPageAtlas>>,
    mut render_uploads: ResMut<RenderSpriteArtPageUploads>,
) {
    RenderArtPageWork::extend_from_set(&mut render_uploads.0, &atlas);
}

pub fn sys_extract_ground_art_page_uploads(
    atlas: Extract<Res<GroundArtPageAtlas>>,
    mut render_uploads: ResMut<RenderGroundArtPageUploads>,
) {
    RenderArtPageWork::extend_from_set(&mut render_uploads.0, &atlas);
}

/// The images of the sprite atlases, by slot (`ArtPageAtlasSet::handles`).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct SpriteArtPageAtlasHandles(pub [Handle<Image>; ART_ATLAS_SLOTS]);

/// The images of the ground atlases, by slot (`ArtPageAtlasSet::handles`).
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct GroundArtPageAtlasHandles(pub [Handle<Image>; ART_ATLAS_SLOTS]);

pub fn sys_render_upload_sprite_art_pages(
    mut uploads: ResMut<RenderSpriteArtPageUploads>,
    atlas_handles: Option<Res<SpriteArtPageAtlasHandles>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    render_art_page_work(&mut uploads.0, atlas_handles.as_ref().map(|handles| &handles.0), &gpu_images,
        &render_device, &render_queue);
}

pub fn sys_render_upload_ground_art_pages(
    mut uploads: ResMut<RenderGroundArtPageUploads>,
    atlas_handles: Option<Res<GroundArtPageAtlasHandles>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    render_art_page_work(&mut uploads.0, atlas_handles.as_ref().map(|handles| &handles.0), &gpu_images,
        &render_device, &render_queue);
}

/// Each atlas's layer copies, then its page uploads once they are done.
fn render_art_page_work(
    works: &mut [RenderArtPageWork; ART_ATLAS_SLOTS],
    atlas_handles: Option<&[Handle<Image>; ART_ATLAS_SLOTS]>,
    gpu_images: &RenderAssets<GpuImage>,
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
) {
    for (slot, work) in works.iter_mut().enumerate() {
        if copy_grown_art_layers(&mut work.layer_copies, gpu_images, render_device, render_queue) {
            render_art_page_uploads(&mut work.uploads, atlas_handles.map(|handles| &handles[slot]), gpu_images,
                render_queue);
        }
    }
}

/// Copies the resident layers of each grown atlas from the image it replaced, in growth order, and says whether all are
/// done. The page uploads wait for them: a page staged before a growth targets a layer the copy would overwrite.
fn copy_grown_art_layers(
    copies: &mut Vec<ArtLayerCopy>,
    gpu_images: &RenderAssets<GpuImage>,
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
) -> bool {
    use bevy::render::render_resource::{CommandEncoderDescriptor, Extent3d, Origin3d, TexelCopyTextureInfo, TextureAspect};

    let mut encoder = None;
    let mut copied = 0;
    for copy in copies.iter() {
        let (Some(source), Some(destination)) = (gpu_images.get(&copy.source), gpu_images.get(&copy.destination)) else {
            break;
        };
        let encoder = encoder.get_or_insert_with(|| render_device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("art_atlas_growth_copy"),
        }));
        let size = source.texture_descriptor.size;
        let layer_start = |texture| TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All };
        encoder.copy_texture_to_texture(
            layer_start(&*source.texture),
            layer_start(&*destination.texture),
            Extent3d { width: size.width, height: size.height, depth_or_array_layers: copy.layers },
        );
        copied += 1;
    }
    if let Some(encoder) = encoder {
        render_queue.submit([encoder.finish()]);
    }
    copies.drain(..copied);
    copies.is_empty()
}

fn render_art_page_uploads(
    uploads: &mut Vec<ArtPageUpload>,
    atlas_handle: Option<&Handle<Image>>,
    gpu_images: &RenderAssets<GpuImage>,
    render_queue: &RenderQueue,
) {
    if uploads.is_empty() {
        return;
    }
    let Some(atlas_handle) = atlas_handle else { return };
    let Some(gpu_image) = gpu_images.get(atlas_handle) else { return };

    use bevy::render::render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo};

    let submitted = uploads.len();
    for upload in uploads.iter() {
        let destination = TexelCopyTextureInfo {
            texture: &*gpu_image.texture,
            mip_level: 0,
            origin: Origin3d {
                x: 0,
                y: 0,
                z: upload.layer,
            },
            aspect: bevy::render::render_resource::TextureAspect::All,
        };

        let data_layout = TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(upload.upload_layout.bytes_per_row),
            rows_per_image: Some(upload.upload_layout.rows_per_image),
        };

        let extent = Extent3d {
            width: upload.upload_width,
            height: upload.upload_height,
            depth_or_array_layers: 1,
        };

        render_queue.write_texture(destination, &upload.bytes, data_layout, extent);
    }

    if submitted > 0 {
        uploads.drain(0..submitted);
    }
}
