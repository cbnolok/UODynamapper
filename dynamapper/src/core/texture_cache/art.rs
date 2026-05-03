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

#[derive(Resource)]
pub struct ArtPageAtlas {
    pub gpu_handle: Handle<Image>,
    page_to_layer: HashMap<u32, u32>,
    layer_to_page: Vec<Option<u32>>,
    layer_access_tick: Vec<u64>,
    current_tick: u64,
    pub pending_uploads: Vec<(u32, Vec<u8>)>,
    pub extract_staging: Vec<(u32, Vec<u8>)>,
    pub page_width: u32,
    pub page_height: u32,
    pub max_layers: u32,
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

    pub fn resolve(&mut self, cc_art: &CcArtPackage, graphic: u16) -> Option<ResolvedArtSprite> {
        let art_id = graphic as u32;
        let slot = cc_art.present_slot(art_id)?;
        let page_index = slot.page_index;
        
        self.current_tick += 1;
        
        let layer = if let Some(&layer) = self.page_to_layer.get(&page_index) {
            self.layer_access_tick[layer as usize] = self.current_tick;
            layer
        } else {
            // Need to allocate a layer for this page. Check if we already requested it.
            if self.pending_uploads.iter().any(|(p, _)| *p == page_index) {
                return None; // Still uploading
            }
            if self.extract_staging.iter().any(|(p, _)| *p == page_index) {
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
            if let Ok(page_data) = cc_art.read_page_bytes(page_index) {
                if let Some(page_meta) = cc_art.pages().get(page_index as usize) {
                    // Extract full RGBA page
                    if let Ok(rgba) = extract_slot_rgba(
                        &page_data,
                        cc_art.atlas_width(),
                        cc_art.atlas_height(),
                        page_meta.used_width,
                        page_meta.used_height,
                        page_meta.pixel_format,
                        0,
                        0,
                        page_meta.used_width as u16,
                        page_meta.used_height as u16,
                    ) {
                        let mut full_page = vec![0; (self.page_width * self.page_height * 4) as usize];
                        let copy_width = (page_meta.used_width as usize).min(self.page_width as usize);
                        let copy_height = (page_meta.used_height as usize).min(self.page_height as usize);
                        for y in 0..copy_height {
                            let src_start = y * (page_meta.used_width as usize) * 4;
                            let src_end = src_start + copy_width * 4;
                            let dst_start = y * (self.page_width as usize) * 4;
                            let dst_end = dst_start + copy_width * 4;
                            full_page[dst_start..dst_end].copy_from_slice(&rgba[src_start..src_end]);
                        }
                        self.pending_uploads.push((page_index, full_page));
                    }
                }
            }

            self.page_to_layer.insert(page_index, layer);
            self.layer_to_page[layer as usize] = Some(page_index);
            self.layer_access_tick[layer as usize] = self.current_tick;
            
            return None; // Wait for upload
        };

        Some(ResolvedArtSprite {
            layer,
            uv_min: Vec2::new(slot.x as f32 / self.page_width as f32, slot.y as f32 / self.page_height as f32),
            uv_max: Vec2::new((slot.x + slot.width) as f32 / self.page_width as f32, (slot.y + slot.height) as f32 / self.page_height as f32),
            pixel_width: slot.width,
            pixel_height: slot.height,
        })
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

    pub fn load_selected_texture(
        &self,
        art_id: u32,
        settings: &crate::configs::settings::SectGraphics,
    ) -> eyre::Result<Option<LoadedArtTexture>> {
        self.load_texture(art_id, settings.art_texture_source)
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
pub struct RenderArtPageUploads(pub Vec<(u32, Vec<u8>)>);

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

    let mut submitted = 0;
    for (layer, data) in uploads.0.iter() {
        let destination = TexelCopyTextureInfo {
            texture: &*gpu_image.texture,
            mip_level: 0,
            origin: Origin3d { x: 0, y: 0, z: *layer },
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

        render_queue.write_texture(destination, data, data_layout, extent);
        submitted += 1;
        
        // Budget cap: max 1 page per frame (16MB each)
        if submitted >= 1 {
            break;
        }
    }

    if submitted > 0 {
        uploads.0.drain(0..submitted);
    }
}
