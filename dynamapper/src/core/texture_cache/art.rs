#![allow(dead_code)]

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
use uddconv::{
    bc7::{self, ImageExtent},
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

pub struct ResolvedArtSprite {
    pub layer: u32,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

#[derive(Debug, Clone)]
pub struct ArtPageUpload {
    pub cache_key: u64,
    pub page_index: u32,
    pub layer: u32,
    pub rgba: Vec<u8>,
}

#[derive(Resource)]
pub struct ArtPageAtlas {
    pub gpu_handle: Handle<Image>,
    page_to_layer: HashMap<u64, u32>,
    layer_to_page: Vec<Option<u64>>,
    layer_access_tick: Vec<u64>,
    current_tick: u64,
    pub pending_uploads: Vec<ArtPageUpload>,
    pub extract_staging: Vec<ArtPageUpload>,
    pub page_width: u32,
    pub page_height: u32,
    pub max_layers: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ArtPageSource {
    Cc,
    EcArt,
    EcLand,
}

impl ArtPageAtlas {
    pub fn new(gpu_handle: Handle<Image>, page_width: u32, page_height: u32, max_layers: u32) -> Self {
        Self {
            gpu_handle,
            page_to_layer: HashMap::new(),
            layer_to_page: vec![None; max_layers as usize],
            layer_access_tick: vec![0; max_layers as usize],
            current_tick: 0,
            pending_uploads: Vec::new(),
            extract_staging: Vec::new(),
            page_width,
            page_height,
            max_layers,
        }
    }

    pub fn clear(&mut self) {
        self.page_to_layer.clear();
        self.layer_to_page.fill(None);
        self.layer_access_tick.fill(0);
        self.current_tick = 0;
        self.pending_uploads.clear();
        self.extract_staging.clear();
    }

    fn cache_key_for_page_source(source: ArtPageSource, page_index: u32) -> u64 {
        let source_bits = match source {
            ArtPageSource::Cc => 0u64,
            ArtPageSource::EcArt => 1u64,
            ArtPageSource::EcLand => 2u64,
        };
        (source_bits << 32) | page_index as u64
    }

    pub fn cache_key_for_page(source: ClientTextureSource, page_index: u32) -> u64 {
        let page_source = match source {
            ClientTextureSource::Cc => ArtPageSource::Cc,
            ClientTextureSource::Ec => ArtPageSource::EcArt,
        };
        Self::cache_key_for_page_source(page_source, page_index)
    }

    pub fn cache_key_for_ec_land_page(page_index: u32) -> u64 {
        Self::cache_key_for_page_source(ArtPageSource::EcLand, page_index)
    }

    pub fn resolve_cc(&mut self, cc_art: &CcArtPackage, graphic: u16) -> Option<ResolvedArtSprite> {
        let art_id = graphic as u32;
        let slot = cc_art.present_slot(art_id)?;
        self.resolve_slot(
            ArtPageSource::Cc,
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
            ArtPageSource::EcArt,
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
            ArtPageSource::EcLand,
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
        source: ArtPageSource,
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

        let cache_key = Self::cache_key_for_page_source(source, page_index);
        let page_upload_pending = self
            .pending_uploads
            .iter()
            .any(|upload| upload.cache_key == cache_key)
            || self
                .extract_staging
                .iter()
                .any(|upload| upload.cache_key == cache_key);

        self.current_tick += 1;

        let layer = if let Some(&layer) = self.page_to_layer.get(&cache_key) {
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
            } else {
                let lru_layer = self.layer_access_tick.iter().enumerate().min_by_key(|(_, &tick)| tick).map(|(i, _)| i).unwrap() as u32;
                if let Some(evicted_page) = self.layer_to_page[lru_layer as usize] {
                    self.page_to_layer.remove(&evicted_page);
                }
                lru_layer
            };

            // Queue the page upload
            if let Ok(page_data) = read_page_bytes() {
                if let Ok(rgba) = extract_slot_rgba(
                    &page_data,
                    atlas_width,
                    atlas_height,
                    used_width,
                    used_height,
                    pixel_format,
                    0,
                    0,
                    used_width as u16,
                    used_height as u16,
                ) {
                    let mut full_page = vec![0; (self.page_width * self.page_height * 4) as usize];
                    let copy_width = used_width as usize;
                    let copy_height = used_height as usize;
                    for y in 0..copy_height {
                        let src_start = y * used_width as usize * 4;
                        let src_end = src_start + copy_width * 4;
                        let dst_start = y * self.page_width as usize * 4;
                        let dst_end = dst_start + copy_width * 4;
                        full_page[dst_start..dst_end].copy_from_slice(&rgba[src_start..src_end]);
                    }
                    self.pending_uploads.push(ArtPageUpload {
                        cache_key,
                        page_index,
                        layer,
                        rgba: full_page,
                    });
                }
            }

            self.page_to_layer.insert(cache_key, layer);
            self.layer_to_page[layer as usize] = Some(cache_key);
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
pub struct RenderArtPageUploads(pub Vec<ArtPageUpload>);

pub fn sys_stage_art_page_uploads(mut atlas: ResMut<ArtPageAtlas>) {
    atlas.extract_staging.clear();
    let mut pending = std::mem::take(&mut atlas.pending_uploads);
    atlas.extract_staging.append(&mut pending);
}

pub fn sys_extract_art_page_uploads(
    atlas: Extract<Res<ArtPageAtlas>>,
    mut render_uploads: ResMut<RenderArtPageUploads>,
) {
    if !atlas.extract_staging.is_empty() {
        render_uploads.0.extend(atlas.extract_staging.clone());
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct ArtPageAtlasHandle(pub Handle<Image>);

pub fn sys_render_upload_art_pages(
    mut uploads: ResMut<RenderArtPageUploads>,
    atlas_handle: Option<Res<ArtPageAtlasHandle>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    if uploads.0.is_empty() {
        return;
    }
    let Some(atlas_handle) = atlas_handle else { return };
    let Some(gpu_image) = gpu_images.get(&atlas_handle.0) else { return };

    use wgpu::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo};

    let submitted = uploads.0.len();
    for upload in uploads.0.iter() {
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

        let width = gpu_image.size.width;
        let height = gpu_image.size.height;
        let data_layout = TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        };

        let extent = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        render_queue.write_texture(destination, &upload.rgba, data_layout, extent);
    }

    if submitted > 0 {
        uploads.0.drain(0..submitted);
    }
}
