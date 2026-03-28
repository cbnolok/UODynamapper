// ShaderType (from `encase`) generates internal `check()` functions per field for
// alignment validation at compile time — these appear as "function `check` is never used".
#![allow(dead_code)]

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use crate::console_logger::{self, LogAbout, LogSev};

/// A simple RGBA-like 16-bit unsigned integer pair used for packing tile metadata.
/// This matches the target texture format (Rg16Uint) in the shader.
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Rg16u {
    pub r: u16,
    pub g: u16,
}

impl Rg16u {
    /// Packs tile ID, height (Z), and texture size bit into the Rg16u format.
    /// This packing must be manually unrolled in the WGSL shader logic.
    pub fn pack(tile_id: u16, height_i8: i8, tex_size_bits: u16) -> Self {
        // g: low 8 bits: height_i8 + 128
        // g: high 8 bits: tex_size_bits (0 or 1)
        let height_biased = (height_i8 as i16 + 128).clamp(0, 255) as u8;
        let g = (height_biased as u16) | ((tex_size_bits & 0xFF) << 8);
        Self { r: tile_id, g }
    }
}

/// Uniform parameters passed to the terrain shader to resolve world coordinates into atlas samples.
/// This struct must be kept in sync with the shader's `AtlasParams` (including std140/std430 alignment).
#[repr(C, align(16))]
#[derive(Clone, Copy, ShaderType, PartialEq)]
pub struct AtlasParams {
    /// Dimension of a single page in texels.
    pub page_texels: UVec2,
    /// Number of tiles per page (e.g., 8x8 or 2048x2048).
    pub tiles_per_page: UVec2,
    /// Maximum number of physical layers available in the texture array.
    pub max_layers: u32,
    /// Number of world pages along the X axis, used for flat index calculation.
    pub world_pages_x: u32,
    pub _pad: UVec2,
    /// A flattened array mapping page indices to physical layer indices.
    /// Each u32 stores the layer index, or u32::MAX if not mapped.
    pub page_to_layer: [bevy::math::UVec4; 64], // Stores mapping for up to 256 pages!
}

impl Default for AtlasParams {
    fn default() -> Self {
        Self {
            page_texels: UVec2::ZERO,
            tiles_per_page: UVec2::ZERO,
            max_layers: 0,
            world_pages_x: 0,
            _pad: UVec2::ZERO,
            page_to_layer: [bevy::math::UVec4::MAX; 64],
        }
    }
}

use bevy::render::extract_resource::ExtractResource;

#[derive(Resource, Clone, ExtractResource)]
pub struct TileAtlasImageHandle(pub Handle<Image>);

#[derive(Clone)]
pub struct AtlasUpload {
    pub layer: u32,
    pub offset: UVec2,
    pub size: UVec2,
    pub data: Vec<u8>,
}

#[derive(Default, Clone, Copy)]
pub struct LayerDirtyRegion {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

/// Resource managing the paged metadata atlas.
/// It maintains a CPU-side cache and tracks pending uploads to the GPU.
#[derive(Resource)]
pub struct TileAtlas {
    /// Parameters shared with the GPU.
    pub params: AtlasParams,
    /// LRU Cache: Maps logical Page Coordinate (packed u64) to physical Layer Index (u32).
    /// Using a small Vec with linear search is faster than HashMap for the typical number of layers.
    /// mapping: page_u64 -> layer_idx. Sorted by page_u64 for binary search.
    page_to_layer: Vec<(u64, u32)>,
    /// Reverse mapping for eviction logic. Indexed by layer.
    layer_to_page: Vec<IVec2>,
    /// Tracks the 'last used' tick for each layer for LRU eviction. Indexed by layer.
    layer_access_tick: Vec<u64>,
    /// Monotonically increasing counter for LRU tracking.
    current_tick: u64,

    /// CPU mirror of mapped layer data. Indexed by layer.
    pub cpu_mirror: Vec<Option<Vec<Rg16u>>>,
    /// Tracks modified bounding box per layer. Indexed by layer.
    pub dirty_regions: Vec<Option<LayerDirtyRegion>>,
    /// Staging buffer swapped into place by the clear system so that
    /// the extract system can take ownership without cloning.
    extract_staging: Vec<AtlasUpload>,

    /// When set, `sys_apply_tile_atlas_expansion` will create a new GPU image
    /// with this many layers, then clear all page mappings so they are
    /// re-populated by the mesh renderer on subsequent frames.
    pub requested_expansion: Option<u32>,
    /// Hard upper limit for atlas layers (from `TILE_ATLAS_MAX_LAYERS`).
    pub max_layers_limit: u32,
    /// Timestamp of the last time the atlas was at high utilisation (> 75%).
    /// Used by the shrink heuristic to avoid premature downsizing.
    pub last_high_usage_instant: std::time::Instant,
}

impl TileAtlas {
    pub fn new(params: AtlasParams, max_layers_limit: u32) -> Self {
        let max_layers = params.max_layers as usize;
        Self {
            params,
            page_to_layer: Vec::with_capacity(max_layers),
            layer_to_page: vec![IVec2::ZERO; max_layers],
            layer_access_tick: vec![0; max_layers],
            current_tick: 0,
            cpu_mirror: vec![None; max_layers],
            dirty_regions: vec![None; max_layers],
            extract_staging: Vec::new(),
            requested_expansion: None,
            max_layers_limit,
            last_high_usage_instant: std::time::Instant::now(),
        }
    }

    /// Assigns or retrieves a physical layer index for a given logical world page.
    /// If no layers are free, it evicts the least recently used (LRU) page.
    /// Returns the layer index and the optionally evicted page coordinate.
    pub fn ensure_layer_for_page(&mut self, page: IVec2) -> (u32, Option<IVec2>) {
        self.current_tick += 1;
        let page_u64 = (page.x as u32 as u64) | ((page.y as u32 as u64) << 32);

        // If it's already in the cache, return it
        if let Ok(pos) = self.page_to_layer.binary_search_by_key(&page_u64, |(p, _)| *p) {
            let layer = self.page_to_layer[pos].1;
            self.layer_access_tick[layer as usize] = self.current_tick;
            return (layer, None);
        }

        // Need to allocate a new layer
        let mut evicted_page = None;
        let layer = if (self.page_to_layer.len() as u32) < self.params.max_layers {
            // Unused layer available
            self.page_to_layer.len() as u32
        } else {
            // All layers occupied — request expansion so the next frame has
            // more headroom, then evict the LRU page for this frame.
            if self.params.max_layers < self.max_layers_limit {
                let target = (self.params.max_layers * 2).min(self.max_layers_limit);
                if self.requested_expansion.map_or(true, |r| target > r) {
                    self.requested_expansion = Some(target);
                }
            }

            // Evict least recently used layer
            let (lru_layer, _) = self.layer_access_tick.iter()
                .enumerate()
                .min_by_key(|&(_, tick)| tick)
                .expect("At least one layer must exist");
            let lru_layer = lru_layer as u32;

            let old_page = self.layer_to_page[lru_layer as usize];
            let old_page_u64 = (old_page.x as u32 as u64) | ((old_page.y as u32 as u64) << 32);
            if let Ok(pos) = self.page_to_layer.binary_search_by_key(&old_page_u64, |(p, _)| *p) {
                self.page_to_layer.remove(pos);
            }
            evicted_page = Some(old_page);
            lru_layer
        };

        // Insert new entry and keep sorted
        let insert_pos = self.page_to_layer.binary_search_by_key(&page_u64, |(p, _)| *p)
            .unwrap_err();
        self.page_to_layer.insert(insert_pos, (page_u64, layer));
        self.layer_to_page[layer as usize] = page;
        self.layer_access_tick[layer as usize] = self.current_tick;

        if let Some(old_page) = evicted_page {
            let evicted_page_index = (old_page.y as u32) * self.params.world_pages_x + (old_page.x as u32);
            if evicted_page_index < 256 {
                let idx = (evicted_page_index / 4) as usize;
                let comp = evicted_page_index % 4;
                let mut arr = self.params.page_to_layer[idx].to_array();
                arr[comp as usize] = u32::MAX;
                self.params.page_to_layer[idx] = bevy::math::UVec4::from_array(arr);
            }
        }

        let page_index = (page.y as u32) * self.params.world_pages_x + (page.x as u32);
        if page_index < 256 {
            let idx = (page_index / 4) as usize;
            let comp = page_index % 4;
            let mut arr = self.params.page_to_layer[idx].to_array();
            arr[comp as usize] = layer;
            self.params.page_to_layer[idx] = bevy::math::UVec4::from_array(arr);
        }

        // Track high utilisation for the shrink heuristic.
        let usage_ratio = self.page_to_layer.len() as f32 / self.params.max_layers.max(1) as f32;
        if usage_ratio > 0.75 {
            self.last_high_usage_instant = std::time::Instant::now();
        }

        (layer, evicted_page)
    }

    pub fn enqueue_rg16u_block(&mut self, layer: u32, offset: UVec2, size: UVec2, texels: &[Rg16u]) {
        let page_width = self.params.page_texels.x;
        let layer_idx = layer as usize;
        
        if self.cpu_mirror[layer_idx].is_none() {
            self.cpu_mirror[layer_idx] = Some(vec![Rg16u { r: 0, g: 0 }; (page_width * self.params.page_texels.y) as usize]);
        }
        let mirror = self.cpu_mirror[layer_idx].as_mut().unwrap();
        
        for y in 0..size.y {
            let src_start = (y * size.x) as usize;
            let src_end = src_start + size.x as usize;
            let dst_start = ((offset.y + y) * page_width + offset.x) as usize;
            let dst_end = dst_start + size.x as usize;
            mirror[dst_start..dst_end].copy_from_slice(&texels[src_start..src_end]);
        }

        if self.dirty_regions[layer_idx].is_none() {
            self.dirty_regions[layer_idx] = Some(LayerDirtyRegion {
                min_x: u32::MAX,
                min_y: u32::MAX,
                max_x: 0,
                max_y: 0,
            });
        }
        let region = self.dirty_regions[layer_idx].as_mut().unwrap();
        region.min_x = region.min_x.min(offset.x);
        region.min_y = region.min_y.min(offset.y);
        region.max_x = region.max_x.max(offset.x + size.x);
        region.max_y = region.max_y.max(offset.y + size.y);
    }

    /// Number of currently mapped pages.
    pub fn mapped_page_count(&self) -> u32 {
        self.page_to_layer.len() as u32
    }

    /// Clears all page ↔ layer associations and resets the LRU state.
    /// Called after the atlas GPU image is replaced so mappings are rebuilt
    /// by the mesh renderer.
    pub fn clear_all_mappings(&mut self) {
        self.page_to_layer.clear();
        self.layer_to_page.fill(IVec2::ZERO);
        self.layer_access_tick.fill(0);
        self.current_tick = 0;
        self.params.page_to_layer = [bevy::math::UVec4::MAX; 64];
        self.cpu_mirror.fill(None);
        self.dirty_regions.fill(None);
    }

    /// Applies a new layer count after the GPU image has been replaced.
    pub fn apply_resize(&mut self, new_max_layers: u32) {
        self.params.max_layers = new_max_layers;
        self.requested_expansion = None;

        let new_len = new_max_layers as usize;
        self.layer_to_page.resize(new_len, IVec2::ZERO);
        self.layer_access_tick.resize(new_len, 0);
        self.cpu_mirror.resize(new_len, None);
        self.dirty_regions.resize(new_len, None);

        self.clear_all_mappings();
    }
}

use bevy::render::renderer::RenderQueue;
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;

/// Middle-man resource that holds uploaded data during the transition from Main world to Render world.
#[derive(Resource, Default)]
pub struct RenderAtlasUploads(pub Vec<AtlasUpload>);

/// System that extracts pending uploads from the `TileAtlas` resource in the Main world
/// and moves them into the `RenderAtlasUploads` resource in the Render world.
pub fn sys_extract_atlas_uploads(
    tile_atlas: Extract<Res<TileAtlas>>,
    mut render_uploads: ResMut<RenderAtlasUploads>,
) {
    if !tile_atlas.extract_staging.is_empty() {
        let count = tile_atlas.extract_staging.len();
        console_logger::one(
            None,
            LogSev::Debug,
            LogAbout::Performance,
            &format!("[DBG-extract] Extracting {count} texture array uploads"),
        );
        render_uploads.0.extend_from_slice(&tile_atlas.extract_staging);
    }
}

pub fn sys_clear_atlas_uploads(mut tile_atlas: ResMut<TileAtlas>) {
    let mut extracted_uploads = std::mem::take(&mut tile_atlas.extract_staging);
    extracted_uploads.clear();

    let page_width = tile_atlas.params.page_texels.x;
    
    for (layer, opt_region) in tile_atlas.dirty_regions.iter().enumerate() {
        let Some(region) = opt_region else { continue; };
        let layer = layer as u32;
        
        let width = region.max_x.saturating_sub(region.min_x);
        let height = region.max_y.saturating_sub(region.min_y);
        if width == 0 || height == 0 { continue; }
        
        let mirror = tile_atlas.cpu_mirror[layer as usize].as_ref().unwrap();
        let size_bytes = (width * height * 4) as usize;
        let mut data = Vec::with_capacity(size_bytes);
        
        for y in region.min_y..region.max_y {
            let start = (y * page_width + region.min_x) as usize;
            let end = start + width as usize;
            let row_slice: &[u8] = bytemuck::cast_slice(&mirror[start..end]);
            data.extend_from_slice(row_slice);
        }
        
        extracted_uploads.push(AtlasUpload {
            layer,
            offset: UVec2::new(region.min_x, region.min_y),
            size: UVec2::new(width, height),
            data,
        });
    }

    tile_atlas.dirty_regions.fill(None);
    tile_atlas.extract_staging = extracted_uploads;
}

/// System running in the Render world that drains `RenderAtlasUploads` and issues
/// `write_texture` commands to the GPU queue to update the metadata atlas.
pub fn sys_render_upload_tile_atlas(
    mut uploads: ResMut<RenderAtlasUploads>,
    atlas_handle: Res<TileAtlasImageHandle>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    if uploads.0.is_empty() {
        return;
    }

    let count = uploads.0.len();
    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::Performance,
        &format!("[DBG-render] Processing {count} texture array uploads"),
    );

    let Some(gpu_image) = gpu_images.get(&atlas_handle.0) else {
        uploads.0.clear();
        return;
    };

    use wgpu::{TexelCopyTextureInfo, TexelCopyBufferLayout, Origin3d, Extent3d};

    for upload in uploads.0.drain(..) {
        let destination = TexelCopyTextureInfo {
            texture: &*gpu_image.texture,
            mip_level: 0,
            origin: Origin3d {
                x: upload.offset.x,
                y: upload.offset.y,
                z: upload.layer,
            },
            aspect: bevy::render::render_resource::TextureAspect::All,
        };

        let data_layout = TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(upload.size.x * 4),
            rows_per_image: Some(upload.size.y),
        };

        let extent = Extent3d {
            width: upload.size.x,
            height: upload.size.y,
            depth_or_array_layers: 1,
        };

        render_queue.write_texture(destination, &upload.data, data_layout, extent);
    }
}
