// ShaderType (from `encase`) generates internal `check()` functions per field for
// alignment validation at compile time — these appear as "function `check` is never used".
#![allow(dead_code)]

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use crate::console_logger::{self, LogAbout, LogSev};

/// A simple RGBA-like 16-bit unsigned integer pair used for packing tile metadata.
/// This matches the target texture format (Rg16Uint) in the shader.
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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
#[derive(Debug, Clone, Copy, ShaderType, PartialEq)]
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

/// Resource managing the paged metadata atlas. 
/// It maintains a CPU-side cache and tracks pending uploads to the GPU.
#[derive(Resource)]
pub struct TileAtlas {
    /// Parameters shared with the GPU.
    pub params: AtlasParams,
    /// LRU Cache: Maps logical Page Coordinate (IVec2) to physical Layer Index (u32).
    page_to_layer: std::collections::HashMap<IVec2, u32>,
    /// Reverse mapping for eviction logic.
    layer_to_page: std::collections::HashMap<u32, IVec2>,
    /// Tracks the 'last used' tick for each layer for LRU eviction.
    layer_access_tick: std::collections::HashMap<u32, u64>,
    /// Monotonically increasing counter for LRU tracking.
    current_tick: u64,
    
    /// Collects dirty regions to be uploaded to the GPU via `write_texture`.
    pending_uploads: Vec<AtlasUpload>,
}

impl TileAtlas {
    pub fn new(params: AtlasParams) -> Self {
        Self {
            params,
            page_to_layer: Default::default(),
            layer_to_page: Default::default(),
            layer_access_tick: Default::default(),
            current_tick: 0,
            pending_uploads: Vec::new(),
        }
    }

    /// Assigns or retrieves a physical layer index for a given logical world page.
    /// If no layers are free, it evicts the least recently used (LRU) page.
    /// Returns the layer index and the optionally evicted page coordinate.
    pub fn ensure_layer_for_page(&mut self, page: IVec2) -> (u32, Option<IVec2>) {
        self.current_tick += 1;

        // If it's already in the cache, return it
        if let Some(&layer) = self.page_to_layer.get(&page) {
            self.layer_access_tick.insert(layer, self.current_tick);
            return (layer, None);
        }

        // Need to allocate a new layer
        let mut evicted_page = None;
        let layer = if (self.page_to_layer.len() as u32) < self.params.max_layers {
            // Unused layer available
            self.page_to_layer.len() as u32
        } else {
            // Evict least recently used layer
            let lru_layer = *self.layer_access_tick
                .iter()
                .min_by_key(|&(_, &tick)| tick)
                .map(|(layer, _)| layer)
                .unwrap();
            
            let old_page = self.layer_to_page.remove(&lru_layer).unwrap();
            self.page_to_layer.remove(&old_page);
            evicted_page = Some(old_page);
            lru_layer
        };

        // Insert new association
        self.page_to_layer.insert(page, layer);
        self.layer_to_page.insert(layer, page);
        self.layer_access_tick.insert(layer, self.current_tick);

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

        (layer, evicted_page)
    }

    /// Enqueues a block of Rg16u metadata for upload to a specific layer and offset.
    /// This registers a `write_texture` operation that will be executed in the render world.
    pub fn enqueue_rg16u_block(&mut self, layer: u32, offset: UVec2, size: UVec2, texels: &[Rg16u]) {
        let size_bytes = std::mem::size_of_val(texels);
        let mut data = vec![0u8; size_bytes];
        data.copy_from_slice(bytemuck::cast_slice(texels));
        
        self.pending_uploads.push(AtlasUpload {
            layer,
            offset,
            size,
            data,
        });
    }

    pub fn drain_pending_uploads(&mut self) -> Vec<AtlasUpload> {
        std::mem::take(&mut self.pending_uploads)
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
    if !tile_atlas.pending_uploads.is_empty() {
        let count = tile_atlas.pending_uploads.len();
        console_logger::one(
            None,
            LogSev::Debug,
            LogAbout::Performance,
            &format!("[DBG-extract] Extracting {count} texture array uploads"),
        );
        render_uploads.0.extend(tile_atlas.pending_uploads.clone());
    }
}

/// System that clears the main world's pending uploads after they have been extracted.
pub fn sys_clear_atlas_uploads(mut tile_atlas: ResMut<TileAtlas>) {
    tile_atlas.pending_uploads.clear();
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
