//! GPU texture array LRU cache supporting two texture sizes
//! Each texture_id can be either small or big and is mapped accordingly

#![allow(dead_code)]

use super::texture_array;
use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use uocf::geo::land_texture_2d::{LandTextureSize, TexMap2D};
use bevy::render::extract_resource::ExtractResource;
use bevy::render::renderer::RenderQueue;
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;

#[derive(Resource, Clone, ExtractResource)]
pub struct TextureArrayImageHandles {
    pub small: Handle<Image>,
    pub big: Handle<Image>,
}

#[derive(Clone)]
pub struct TextureArrayUpload {
    pub size: LandTextureSize,
    pub layer: u32,
    pub bytes: Vec<u8>,
    /// True if `bytes` contains BC7-compressed data instead of raw RGBA8.
    pub lossy_compressed: bool,
}

#[derive(Resource, Default)]
pub struct RenderTextureArrayUploads(pub Vec<TextureArrayUpload>);

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);

/// Runtime settings for the land texture cache, inserted at startup.
/// Holds values read from `settings.toml` that affect how tiles are uploaded to the GPU.
#[derive(Resource, Clone, Copy)]
pub struct LandTextureCacheSettings {
    /// When true, tiles are BC7-compressed on the CPU before uploading to the GPU.
    /// Reduces VRAM from ~160 MB to ~20 MB at the cost of near-lossless quality.
    pub lossy_texture_compression: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LandTextureEntry {
    pub layer: u32,
    pub last_touch: Instant,
}

/// A single TextureArray data (we use one for each size)
pub struct LandTextureArrayWrapper {
    pub image_handle: Handle<Image>,
    free_layers: Vec<u32>,
    lru: VecDeque<u16>, // texture_id queue
}
impl LandTextureArrayWrapper {
    fn new(image_handle: Handle<Image>, max_layers: u32) -> Self {
        Self {
            image_handle,
            free_layers: (0..max_layers).rev().collect(),
            lru: VecDeque::default(),
        }
    }
}

#[derive(Resource)]
pub struct LandTextureCache {
    pub small: LandTextureArrayWrapper,
    pub big: LandTextureArrayWrapper,
    entry_by_id: HashMap<u16, (LandTextureSize, LandTextureEntry)>,
    pub pending_uploads: Vec<TextureArrayUpload>,
}

impl LandTextureCache {
    pub fn new(small_tex_image_handle: Handle<Image>, big_tex_image_handle: Handle<Image>) -> Self {
        Self {
            small: LandTextureArrayWrapper::new(
                small_tex_image_handle,
                texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
            ),
            big: LandTextureArrayWrapper::new(
                big_tex_image_handle,
                texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
            ),
            entry_by_id: HashMap::default(),
            pending_uploads: Vec::new(),
        }
    }



    /// Gets the layer for a single texture. If not resident, it will be loaded, causing an async GPU upload.
    pub fn get_texture_size_layer(
        &mut self,
        texmap_2d: Arc<TexMap2D>,
        texture_id: u16,
        lossy_compression: bool,
    ) -> (LandTextureSize, u32) {
        // If texture is already resident, just return its info.
        if let Some(entry) = self.entry_by_id.get_mut(&texture_id) {
            entry.1.last_touch = Instant::now();
            return (entry.0, entry.1.layer);
        }

        let prepared = self.prepare_texture_residency(texture_id, &texmap_2d, lossy_compression).unwrap();

        self.pending_uploads.push(prepared.clone());

        // Update bookkeeping and return.
        self.update_bookkeeping(texture_id, prepared.size, prepared.layer);
        (prepared.size, prepared.layer)
    }

    /// Checks if a texture is resident. If not, allocates a layer and loads its data,
    /// returning a struct with all info needed to perform the upload and bookkeeping.
    fn prepare_texture_residency(
        &mut self,
        texture_id: u16,
        texmap_2d: &Arc<TexMap2D>,
        lossy_compression: bool,
    ) -> Option<TextureArrayUpload> {
        // If resident, touch timestamp and return None as no upload is needed.
        if let Some(entry) = self.entry_by_id.get_mut(&texture_id) {
            entry.1.last_touch = Instant::now();
            return None;
        }

        // --- If not resident, perform CPU-side work --- 

        // 1. Get the new texture data and metadata.
        let (texture_size, raw_rgba8) =
            texture_array::get_texmap_raw_data(texture_id, texmap_2d);

        // 2. Allocate a layer, evicting an old one if necessary.
        let layer = self.allocate_layer(texture_size);

        // 3. Optionally compress to BC7 before storing the upload bytes.
        //    Compression is done once per texture; the result is cached implicitly
        //    because the LRU keeps the entry alive until eviction.
        let tile_bytes: Vec<u8> = if lossy_compression {
            texture_array::compress_rgba8_to_bc7(raw_rgba8.as_slice(), texture_size)
        } else {
            raw_rgba8.to_vec()
        };

        Some(TextureArrayUpload {
            layer,
            size: texture_size,
            bytes: tile_bytes,
            lossy_compressed: lossy_compression,
        })
    }

    /// Allocates a layer for a new texture, handling LRU eviction if the array is full.
    fn allocate_layer(&mut self, texture_size: LandTextureSize) -> u32 {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        if let Some(l) = array.free_layers.pop() {
            l
        } else {
            let victim_id = loop {
                let oldest = array
                    .lru
                    .pop_front()
                    .expect("LRU should not be empty at this stage");
                if let Some(still) = self.entry_by_id.get(&oldest) {
                    if Instant::now() - still.1.last_touch >= CACHE_EVICT_AFTER {
                        break oldest;
                    }
                }
                array.lru.push_back(oldest);
            };
            let victim_entry: (LandTextureSize, LandTextureEntry) =
                self.entry_by_id.remove(&victim_id).unwrap();
            victim_entry.1.layer
        }
    }

    /// Updates the cache's internal maps after a texture has been uploaded.
    fn update_bookkeeping(&mut self, texture_id: u16, texture_size: LandTextureSize, layer: u32) {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        self.entry_by_id.insert(
            texture_id,
            (
                texture_size,
                LandTextureEntry {
                    layer,
                    last_touch: Instant::now(),
                },
            ),
        );
        array.lru.push_back(texture_id);
    }

    fn free_layer_for_entry(&mut self, texture_size: LandTextureSize, entry: LandTextureEntry) {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };
        array.free_layers.push(entry.layer);
        // Removal from LRU performed implicitly (by removing the entry entirely or letting it fall off on reset)
    }
}

/*
use image::{ImageBuffer, RgbaImage};
use std::path::Path;
fn dump_texture_array_layer(
    images: &ResMut<Assets<Image>>,
    array_handle: &Handle<Image>,
    layer_index: u32,
    tile_size: u32, // e.g. 44
    output_file: &str,
) {
    // Get the bevy Image (texture array)
    if let Some(array_img) = images.get(array_handle) {
        let pixel_data = array_img.data.as_ref().unwrap();
        let depth = array_img.texture_descriptor.size.depth_or_array_layers;
        assert!(
            layer_index < depth,
            "Requested layer {} out of bounds",
            layer_index
        );

        let layer_size = (tile_size * tile_size * 4) as usize; // RGBA8
        let layer_offset = layer_index as usize * layer_size;

        // Make an ImageBuffer from the raw data
        let layer_data = &pixel_data[layer_offset..layer_offset + layer_size];
        let img_buf: RgbaImage = ImageBuffer::from_raw(tile_size, tile_size, layer_data.to_vec())
            .expect("Failed to create buffer from raw tile data.");

        // Save as PNG
        img_buf.save(Path::new(output_file)).unwrap();

        println!("Dumped layer {layer_index} to {output_file}");
    } else {
        eprintln!("Couldn't find texture array handle");
    }
}
*/

pub fn sys_extract_texture_array_uploads(
    cache: Extract<Res<LandTextureCache>>,
    mut render_uploads: ResMut<RenderTextureArrayUploads>,
) {
    if !cache.pending_uploads.is_empty() {
        eprintln!("[DBG-extract] Extracting {} texture array uploads", cache.pending_uploads.len());
        render_uploads.0.extend(cache.pending_uploads.iter().cloned());
    }
}

pub fn sys_clear_texture_array_uploads(mut cache: ResMut<LandTextureCache>) {
    cache.pending_uploads.clear();
}

pub fn sys_render_upload_texture_array(
    mut uploads: ResMut<RenderTextureArrayUploads>,
    handles: Res<TextureArrayImageHandles>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    if uploads.0.is_empty() {
        return;
    }

    eprintln!("[DBG-render] Processing {} texture array uploads", uploads.0.len());

    let small_gpu = gpu_images.get(&handles.small);
    let big_gpu = gpu_images.get(&handles.big);

    if small_gpu.is_none() {
        eprintln!("[DBG-render] small_gpu image NOT FOUND in RenderAssets");
    }
    if big_gpu.is_none() {
        eprintln!("[DBG-render] big_gpu image NOT FOUND in RenderAssets");
    }

    if small_gpu.is_none() && big_gpu.is_none() {
        uploads.0.clear();
        return;
    }

    use wgpu::{TexelCopyTextureInfo, TexelCopyBufferLayout, Origin3d, Extent3d};

    for upload in uploads.0.drain(..) {
        let gpu_image = match upload.size {
            LandTextureSize::Small => small_gpu,
            LandTextureSize::Big => big_gpu,
        };
        let Some(gpu_image) = gpu_image else { continue; };
        
        let (width, height) = upload.size.dimensions();

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

        // bytes_per_row must match the data format:
        //  - Uncompressed RGBA8: each row is `width * 4` bytes.
        //  - BC7 (block-compressed): each "row" in wgpu terms is one row of 4×4 blocks.
        //    A single block covers 4 pixels horizontally and costs 16 bytes.
        //    So bytes_per_row = ceil(width / 4) * 16.
        let bytes_per_row = if upload.lossy_compressed {
            let blocks_wide = (width + 3) / 4;
            blocks_wide * 16
        } else {
            width * 4
        };

        let data_layout = TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        };

        let extent = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        render_queue.write_texture(destination, &upload.bytes, data_layout, extent);
    }
}
