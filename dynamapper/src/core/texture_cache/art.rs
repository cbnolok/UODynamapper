#![allow(dead_code)]

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use bevy::prelude::*;
use color_eyre::eyre::{self, WrapErr};

use crate::{
    configs::settings::ClientTextureSource,
    core::{
        system_sets::StartupSysSet,
        uo_files_loader::{CcArtPackageRes, EcArtPackageRes, TileMetaPackageRes},
    },
    prelude::*,
};
use udd_conv::bc7::{self, ImageExtent, TextureUploadLayout, VramTextureFormat};
use udd_assets::{
    cc_art::{CcArtPackage, PagePixelFormat},
    ec_art::EcArtPackage,
    ec_land::EcLandPackage,
    tilemeta::TileMetaPackage,
};
use std::collections::HashMap;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;


#[derive(Debug, Clone)]
pub struct LoadedArtTexture {
    pub art_id: u32,
    pub source: ClientTextureSource,
    pub texture_id: u32,
    pub sample_start_x: i16,
    pub sample_start_y: i16,
    pub offset_x: i16,
    pub offset_y: i16,
    pub width: u16,
    pub height: u16,
    pub rgba8: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedArtSprite {
    pub layer: u32,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

#[derive(Debug, Clone)]
pub struct ArtPageUpload {
    pub page_index: u32,
    pub layer: u32,
    pub bytes: Vec<u8>,
    pub upload_layout: TextureUploadLayout,
    pub upload_width: u32,
    pub upload_height: u32,
}

pub struct ArtPageAtlas {
    pub gpu_handle: Handle<Image>,
    page_to_layer: HashMap<u32, u32>,
    layer_to_page: Vec<Option<u32>>,
    layer_access_tick: Vec<u64>,
    current_tick: u64,
    pub pending_uploads: Vec<ArtPageUpload>,
    pub extract_staging: Vec<ArtPageUpload>,
    pub page_width: u32,
    pub page_height: u32,
    pub active_layers: u32,
    pub max_layers: u32,
    pub pixel_format: PagePixelFormat,
    pub requested_resize_to: Option<u32>,
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
        Self {
            gpu_handle,
            page_to_layer: HashMap::new(),
            layer_to_page: vec![None; active_layers as usize],
            layer_access_tick: vec![0; active_layers as usize],
            current_tick: 0,
            pending_uploads: Vec::new(),
            extract_staging: Vec::new(),
            page_width,
            page_height,
            active_layers,
            max_layers,
            pixel_format,
            requested_resize_to: None,
        }
    }

    pub fn clear(&mut self) {
        self.page_to_layer.clear();
        self.layer_to_page.fill(None);
        self.layer_access_tick.fill(0);
        self.current_tick = 0;
        self.pending_uploads.clear();
        self.extract_staging.clear();
        self.requested_resize_to = None;
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

    pub fn resident_page_layers(&self) -> Vec<(u32, u32)> {
        self.page_to_layer
            .iter()
            .map(|(&page_index, &layer)| (page_index, layer))
            .collect()
    }

    pub fn queue_page_upload<F>(
        &mut self,
        page_index: u32,
        layer: u32,
        used_width: u32,
        used_height: u32,
        pixel_format: PagePixelFormat,
        read_page_bytes: F,
    ) where
        F: FnOnce() -> eyre::Result<Vec<u8>>,
    {
        let page_upload_pending = self
            .pending_uploads
            .iter()
            .any(|upload| upload.page_index == page_index)
            || self
                .extract_staging
                .iter()
                .any(|upload| upload.page_index == page_index);
        if page_upload_pending {
            return;
        }

        if let Ok(page_data) = read_page_bytes() {
            if let Some((bytes, upload_layout)) = prepare_page_upload_bytes(
                &page_data,
                used_width,
                used_height,
                pixel_format,
            ) {
                self.pending_uploads.push(ArtPageUpload {
                    page_index,
                    layer,
                    bytes,
                    upload_layout,
                    upload_width: used_width,
                    upload_height: used_height,
                });
            }
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
        self.current_tick = 0;
        self.requested_resize_to = None;
    }

    pub fn resolve_cc(&mut self, cc_art: &CcArtPackage, graphic: u16) -> Option<ResolvedArtSprite> {
        let art_id = graphic as u32;
        let slot = cc_art.present_slot(art_id)?;
        self.resolve_slot(
            slot.page_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            cc_art.atlas_width(),
            cc_art.atlas_height(),
            cc_art.pages().get(slot.page_index as usize)?.used_width,
            cc_art.pages().get(slot.page_index as usize)?.used_height,
            cc_art.pages().get(slot.page_index as usize)?.pixel_format,
            || cc_art.read_page_bytes(slot.page_index),
        )
    }

    pub fn resolve_ec(&mut self, ec_art: &EcArtPackage, art_id: u32) -> Option<ResolvedArtSprite> {
        let slot = ec_art.present_slot(art_id)?;
        self.resolve_slot(
            slot.page_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            ec_art.atlas_width(),
            ec_art.atlas_height(),
            ec_art.pages().get(slot.page_index as usize)?.used_width,
            ec_art.pages().get(slot.page_index as usize)?.used_height,
            ec_art.pages().get(slot.page_index as usize)?.pixel_format,
            || ec_art.read_page_bytes(slot.page_index),
        )
    }

    pub fn resolve_ec_land(
        &mut self,
        ec_land: &EcLandPackage,
        art_id: u32,
    ) -> Option<ResolvedArtSprite> {
        let slot = ec_land.present_slot(art_id)?;
        self.resolve_slot(
            slot.page_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
            ec_land.atlas_width(),
            ec_land.atlas_height(),
            ec_land.pages().get(slot.page_index as usize)?.used_width,
            ec_land.pages().get(slot.page_index as usize)?.used_height,
            ec_land.pages().get(slot.page_index as usize)?.pixel_format,
            || ec_land.read_page_bytes(slot.page_index),
        )
    }

    fn resolve_slot<F>(
        &mut self,
        page_index: u32,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        atlas_width: u32,
        atlas_height: u32,
        used_width: u32,
        used_height: u32,
        pixel_format: PagePixelFormat,
        read_page_bytes: F,
    ) -> Option<ResolvedArtSprite>
    where
        F: FnOnce() -> eyre::Result<Vec<u8>>,
    {
        if atlas_width > self.page_width || atlas_height > self.page_height {
            return None;
        }

        let page_upload_pending = self
            .pending_uploads
            .iter()
            .any(|upload| upload.page_index == page_index)
            || self
                .extract_staging
                .iter()
                .any(|upload| upload.page_index == page_index);

        self.current_tick += 1;

        let layer = if let Some(&layer) = self.page_to_layer.get(&page_index) {
            self.layer_access_tick[layer as usize] = self.current_tick;
            if page_upload_pending {
                return None;
            }
            layer
        } else {
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
                let lru_layer = self.layer_access_tick.iter().enumerate().min_by_key(|(_, &tick)| tick).map(|(i, _)| i).unwrap() as u32;
                if let Some(evicted_page) = self.layer_to_page[lru_layer as usize] {
                    self.page_to_layer.remove(&evicted_page);
                }
                lru_layer
            };

            self.queue_page_upload(
                page_index,
                layer,
                used_width,
                used_height,
                pixel_format,
                read_page_bytes,
            );

            self.page_to_layer.insert(page_index, layer);
            self.layer_to_page[layer as usize] = Some(page_index);
            self.layer_access_tick[layer as usize] = self.current_tick;

            return None; // Wait for upload
        };

        Some(ResolvedArtSprite {
            layer,
            uv_min: Vec2::new(x as f32 / self.page_width as f32, y as f32 / self.page_height as f32),
            uv_max: Vec2::new((x + width) as f32 / self.page_width as f32, (y + height) as f32 / self.page_height as f32),
            pixel_width: width,
            pixel_height: height,
        })
    }

    pub fn resident_page_count(&self) -> usize {
        self.page_to_layer.len()
    }

    pub fn pending_page_count(&self) -> usize {
        self.pending_uploads.len() + self.extract_staging.len()
    }
}

fn page_pixel_vram_format(pixel_format: PagePixelFormat) -> VramTextureFormat {
    match pixel_format {
        PagePixelFormat::Rgba8888 => VramTextureFormat::Rgba8UnormSrgb,
        PagePixelFormat::Bc7 => VramTextureFormat::Bc7RgbaUnormSrgb,
    }
}

fn prepare_page_upload_bytes(
    page_data: &[u8],
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
) -> Option<(Vec<u8>, TextureUploadLayout)> {
    let extent = ImageExtent::new(used_width.max(1), used_height.max(1)).ok()?;
    let format = page_pixel_vram_format(pixel_format);
    if page_data.len() != format.expected_byte_len(extent) {
        return None;
    }

    Some((page_data.to_vec(), format.upload_layout(extent)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        atlas.page_to_layer.insert(7, 1);
        atlas.layer_to_page[1] = Some(7);
        atlas.layer_access_tick[1] = 11;
        atlas.current_tick = 23;
        atlas.extract_staging.push(ArtPageUpload {
            page_index: 7,
            layer: 1,
            bytes: vec![1, 2, 3, 4],
            upload_layout: TextureUploadLayout {
                bytes_per_row: 4,
                rows_per_image: 1,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
            },
            upload_width: 1,
            upload_height: 1,
        });

        atlas.apply_resize(Handle::default(), 3);

        assert_eq!(atlas.active_layers, 3);
        assert_eq!(atlas.page_to_layer.get(&7), Some(&1));
        assert_eq!(atlas.layer_to_page[1], Some(7));
        assert_eq!(atlas.layer_to_page[2], None);
        assert_eq!(atlas.layer_access_tick[1], 11);
        assert_eq!(atlas.current_tick, 23);
        assert_eq!(atlas.pending_uploads.len(), 1);
        assert!(atlas.extract_staging.is_empty());
    }

    #[test]
    fn prepare_page_upload_bytes_uses_bc7_layout_for_bc7_pages() {
        let extent = ImageExtent::new(8, 8).unwrap();
        let bytes = vec![0; VramTextureFormat::Bc7RgbaUnormSrgb.expected_byte_len(extent)];

        let (_, layout) = prepare_page_upload_bytes(&bytes, 8, 8, PagePixelFormat::Bc7).unwrap();

        assert_eq!(layout.bytes_per_row, 32);
        assert_eq!(layout.rows_per_image, 2);
        assert_eq!(layout.format, wgpu::TextureFormat::Bc7RgbaUnormSrgb);
    }
}

#[derive(Resource)]
pub struct SpriteArtPageAtlas(pub ArtPageAtlas);

impl Deref for SpriteArtPageAtlas {
    type Target = ArtPageAtlas;

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
pub struct GroundArtPageAtlas(pub ArtPageAtlas);

impl Deref for GroundArtPageAtlas {
    type Target = ArtPageAtlas;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for GroundArtPageAtlas {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Resource, Default, Clone)]
pub struct ArtTextureLoader {
    cc_art: Option<Arc<CcArtPackage>>,
    ec_art: Option<Arc<EcArtPackage>>,
    tilemeta: Option<Arc<TileMetaPackage>>,
}

impl ArtTextureLoader {
    pub fn source_available(&self, source: ClientTextureSource) -> bool {
        match source {
            ClientTextureSource::Cc => self.cc_art.is_some(),
            ClientTextureSource::Ec => self.ec_art.is_some() && self.tilemeta.is_some(),
        }
    }

    pub fn resolve_effective_source(
        &self,
        requested_source: ClientTextureSource,
    ) -> Option<ClientTextureSource> {
        if self.source_available(requested_source) {
            return Some(requested_source);
        }

        ClientTextureSource::ALL.into_iter().find(|candidate| {
            *candidate != requested_source && self.source_available(*candidate)
        })
    }

    pub fn load_selected_texture(
        &self,
        art_id: u32,
        settings: &crate::configs::settings::SectGraphics,
    ) -> eyre::Result<Option<LoadedArtTexture>> {
        let Some(source) = self.resolve_effective_source(settings.art_texture_source) else {
            return Ok(None);
        };

        self.load_texture(art_id, source)
    }

    pub fn load_texture(
        &self,
        art_id: u32,
        source: ClientTextureSource,
    ) -> eyre::Result<Option<LoadedArtTexture>> {
        match source {
            ClientTextureSource::Cc => self.load_cc_texture(art_id),
            ClientTextureSource::Ec => self.load_ec_texture(art_id),
        }
    }

    fn load_cc_texture(&self, art_id: u32) -> eyre::Result<Option<LoadedArtTexture>> {
        let Some(package) = &self.cc_art else {
            return Ok(None);
        };
        let Some(slot) = package.present_slot(art_id) else {
            return Ok(None);
        };
        let page_data = package
            .read_page_bytes(slot.page_index)
            .wrap_err_with(|| format!("read cc_art page {}", slot.page_index))?;
        let page_meta = package
            .pages()
            .get(slot.page_index as usize)
            .ok_or_else(|| eyre::eyre!("cc_art missing page metadata for {}", slot.page_index))?;
        let rgba8 = extract_slot_rgba(
            &page_data,
            package.atlas_width(),
            package.atlas_height(),
            page_meta.used_width,
            page_meta.used_height,
            page_meta.pixel_format,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
        )?;

        let metadata = self
            .tilemeta
            .as_ref()
            .and_then(|tilemeta| tilemeta.item_tile(art_id));
        let texture_id = metadata
            .and_then(|item| (item.cc_texture_id != 0 || art_id == 0).then_some(item.cc_texture_id))
            .unwrap_or(art_id);
        Ok(Some(LoadedArtTexture {
            art_id,
            source: ClientTextureSource::Cc,
            texture_id,
            sample_start_x: metadata.map(|item| item.cc_start_x).unwrap_or(0),
            sample_start_y: metadata.map(|item| item.cc_start_y).unwrap_or(0),
            offset_x: metadata.map(|item| item.cc_offset_x).unwrap_or(0),
            offset_y: metadata.map(|item| item.cc_offset_y).unwrap_or(0),
            width: slot.width,
            height: slot.height,
            rgba8,
        }))
    }

    fn load_ec_texture(&self, art_id: u32) -> eyre::Result<Option<LoadedArtTexture>> {
        let Some(tilemeta) = &self.tilemeta else {
            return Ok(None);
        };
        let Some(package) = &self.ec_art else {
            return Ok(None);
        };
        let Some(metadata) = tilemeta.item_tile(art_id) else {
            return Ok(None);
        };
        let Some(slot) = package.present_slot(art_id) else {
            return Ok(None);
        };
        let page_data = package
            .read_page_bytes(slot.page_index)
            .wrap_err_with(|| format!("read ec_art page {}", slot.page_index))?;
        let page_meta = package
            .pages()
            .get(slot.page_index as usize)
            .ok_or_else(|| eyre::eyre!("ec_art missing page metadata for {}", slot.page_index))?;
        let rgba8 = extract_slot_rgba(
            &page_data,
            package.atlas_width(),
            package.atlas_height(),
            page_meta.used_width,
            page_meta.used_height,
            page_meta.pixel_format,
            slot.x,
            slot.y,
            slot.width,
            slot.height,
        )?;

        Ok(Some(LoadedArtTexture {
            art_id,
            source: ClientTextureSource::Ec,
            texture_id: metadata.ec_texture_id,
            sample_start_x: metadata.ec_start_x,
            sample_start_y: metadata.ec_start_y,
            offset_x: metadata.ec_offset_x,
            offset_y: metadata.ec_offset_y,
            width: slot.width,
            height: slot.height,
            rgba8,
        }))
    }
}

fn extract_slot_rgba(
    page_data: &[u8],
    _atlas_width: u32,
    _atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
) -> eyre::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    match pixel_format {
        PagePixelFormat::Rgba8888 => Ok(bc7::extract_rgba8888_subrect_arc(
            page_data,
            used_width,
            x as u32,
            y as u32,
            width as u32,
            height as u32,
        )
        .to_vec()),
        PagePixelFormat::Bc7 => {
            let page_extent = ImageExtent::new(used_width, used_height).map_err(|error| {
                eyre::eyre!("invalid stored page extent {used_width}x{used_height}: {error}")
            })?;
            let slot_extent = ImageExtent::new(width as u32, height as u32).map_err(|error| {
                eyre::eyre!("invalid slot extent {}x{}: {error}", width, height)
            })?;
            let blocks =
                bc7::extract_bc7_subrect(page_data, page_extent, x as u32, y as u32, slot_extent);
            bc7::decode_bc7_to_rgba8888(&blocks, slot_extent)
                .map_err(|error| eyre::eyre!("decode art BC7 subrect: {error}"))
        }
    }
}

pub struct ArtTextureLoaderPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(ArtTextureLoaderPlugin);

impl Plugin for ArtTextureLoaderPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_systems(
            Startup,
            sys_setup_art_texture_loader
                .in_set(StartupSysSet::SetupSceneStage1)
                .after(StartupSysSet::LoadStartupUOFiles),
        );
    }
}

fn sys_setup_art_texture_loader(
    mut commands: Commands,
    cc_art_r: Option<Res<CcArtPackageRes>>,
    ec_art_r: Option<Res<EcArtPackageRes>>,
    tilemeta_r: Option<Res<TileMetaPackageRes>>,
) {
    commands.insert_resource(ArtTextureLoader {
        cc_art: cc_art_r.map(|r| r.0.clone()),
        ec_art: ec_art_r.map(|r| r.0.clone()),
        tilemeta: tilemeta_r.map(|r| r.0.clone()),
    });
}

#[derive(Resource, Default)]
pub struct RenderSpriteArtPageUploads(pub Vec<ArtPageUpload>);

#[derive(Resource, Default)]
pub struct RenderGroundArtPageUploads(pub Vec<ArtPageUpload>);

pub fn sys_stage_sprite_art_page_uploads(mut atlas: ResMut<SpriteArtPageAtlas>) {
    atlas.extract_staging.clear();
    let mut pending = std::mem::take(&mut atlas.pending_uploads);
    atlas.extract_staging.append(&mut pending);
}

pub fn sys_stage_ground_art_page_uploads(mut atlas: ResMut<GroundArtPageAtlas>) {
    atlas.extract_staging.clear();
    let mut pending = std::mem::take(&mut atlas.pending_uploads);
    atlas.extract_staging.append(&mut pending);
}

pub fn sys_extract_sprite_art_page_uploads(
    atlas: Extract<Res<SpriteArtPageAtlas>>,
    mut render_uploads: ResMut<RenderSpriteArtPageUploads>,
) {
    if !atlas.extract_staging.is_empty() {
        render_uploads.0.extend(atlas.extract_staging.clone());
    }
}

pub fn sys_extract_ground_art_page_uploads(
    atlas: Extract<Res<GroundArtPageAtlas>>,
    mut render_uploads: ResMut<RenderGroundArtPageUploads>,
) {
    if !atlas.extract_staging.is_empty() {
        render_uploads.0.extend(atlas.extract_staging.clone());
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct SpriteArtPageAtlasHandle(pub Handle<Image>);

#[derive(Resource, Clone, ExtractResource)]
pub struct GroundArtPageAtlasHandle(pub Handle<Image>);

pub fn sys_render_upload_sprite_art_pages(
    mut uploads: ResMut<RenderSpriteArtPageUploads>,
    atlas_handle: Option<Res<SpriteArtPageAtlasHandle>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    render_art_page_uploads(&mut uploads.0, atlas_handle.as_ref().map(|handle| &handle.0), &gpu_images, &render_queue);
}

pub fn sys_render_upload_ground_art_pages(
    mut uploads: ResMut<RenderGroundArtPageUploads>,
    atlas_handle: Option<Res<GroundArtPageAtlasHandle>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    render_art_page_uploads(&mut uploads.0, atlas_handle.as_ref().map(|handle| &handle.0), &gpu_images, &render_queue);
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

    use wgpu::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo};

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
