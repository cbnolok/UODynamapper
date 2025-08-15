//! GPU texture array LRU cache supporting two texture sizes
//! Each texture_id can be either small or big and is mapped accordingly

#![allow(dead_code)]

use super::texture_array;
use bevy::prelude::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use uocf::geo::land_texture_2d::{LandTextureSize, TexMap2D};

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);
const TEXTURE_BYTES_PER_PIXEL: usize = 4; // RGBA8888

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
}

struct PreparedTextureUpload {
    texture_id: u16,
    layer: u32,
    size: LandTextureSize,
    bytes: Vec<u8>,
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
        }
    }

    /// Preloads a set of textures into the cache, performing one batched GPU upload.
    pub fn preload_textures(
        &mut self,
        images_resmut: &mut ResMut<Assets<Image>>,
        texmap_2d: Arc<TexMap2D>,
        texture_ids: &HashSet<u16>,
    ) {
        let mut pending_uploads = Vec::new();

        // --- Stage 1: Collection --- 
        // For each texture, prepare it for upload without actually modifying the GPU asset.
        for &texture_id in texture_ids {
            if let Some(prepared) = self.prepare_texture_residency(texture_id, images_resmut, &texmap_2d) {
                pending_uploads.push(prepared);
            }
        }

        if pending_uploads.is_empty() {
            return;
        }

        // --- Stage 2: Batched Upload --- 
        // Separate uploads by texture array size to avoid mutable borrow conflicts.
        let mut small_uploads = Vec::new();
        let mut big_uploads = Vec::new();
        for upload in pending_uploads {
            match upload.size {
                LandTextureSize::Small => small_uploads.push(upload),
                LandTextureSize::Big => big_uploads.push(upload),
            }
        }

        if !small_uploads.is_empty() {
            if let Some(data) = &mut images_resmut.get_mut(&self.small.image_handle).unwrap().data {
                for upload in &small_uploads {
                    let (width, height) = upload.size.dimensions();
                    let layer_byte_size = (width * height) as usize * TEXTURE_BYTES_PER_PIXEL;
                    let offset = upload.layer as usize * layer_byte_size;
                    data[offset..offset + layer_byte_size].copy_from_slice(&upload.bytes);
                }
            }
        }

        if !big_uploads.is_empty() {
            if let Some(data) = &mut images_resmut.get_mut(&self.big.image_handle).unwrap().data {
                for upload in &big_uploads {
                    let (width, height) = upload.size.dimensions();
                    let layer_byte_size = (width * height) as usize * TEXTURE_BYTES_PER_PIXEL;
                    let offset = upload.layer as usize * layer_byte_size;
                    data[offset..offset + layer_byte_size].copy_from_slice(&upload.bytes);
                }
            }
        }
        
        // --- Stage 3: Bookkeeping ---
        for upload in small_uploads.iter().chain(big_uploads.iter()) {
            self.update_bookkeeping(upload.texture_id, upload.size, upload.layer);
        }
    }

    /// Gets the layer for a single texture. If not resident, it will be loaded, causing an immediate GPU upload.
    pub fn get_texture_size_layer(
        &mut self,
        images_resmut: &mut ResMut<Assets<Image>>,
        texmap_2d: Arc<TexMap2D>,
        texture_id: u16,
    ) -> (LandTextureSize, u32) {
        // If texture is already resident, just return its info.
        if let Some(entry) = self.entry_by_id.get_mut(&texture_id) {
            entry.1.last_touch = Instant::now();
            return (entry.0, entry.1.layer);
        }

        // Otherwise, prepare it for upload.
        let prepared = self.prepare_texture_residency(texture_id, images_resmut, &texmap_2d).unwrap();

        // Perform the single upload.
        let array_handle = match prepared.size {
            LandTextureSize::Small => &self.small.image_handle,
            LandTextureSize::Big => &self.big.image_handle,
        };
        if let Some(data) = &mut images_resmut.get_mut(array_handle).unwrap().data {
            let (width, height) = prepared.size.dimensions();
            let layer_byte_size = (width * height) as usize * TEXTURE_BYTES_PER_PIXEL;
            let offset = prepared.layer as usize * layer_byte_size;
            data[offset..offset + layer_byte_size].copy_from_slice(&prepared.bytes);
        }

        // Update bookkeeping and return.
        self.update_bookkeeping(prepared.texture_id, prepared.size, prepared.layer);
        (prepared.size, prepared.layer)
    }

    /// Checks if a texture is resident. If not, allocates a layer and loads its data,
    /// returning a struct with all info needed to perform the upload and bookkeeping.
    fn prepare_texture_residency(
        &mut self,
        texture_id: u16,
        images_resmut: &mut ResMut<Assets<Image>>,
        texmap_2d: &Arc<TexMap2D>,
    ) -> Option<PreparedTextureUpload> {
        // If resident, touch timestamp and return None as no upload is needed.
        if let Some(entry) = self.entry_by_id.get_mut(&texture_id) {
            entry.1.last_touch = Instant::now();
            return None;
        }

        // --- If not resident, perform CPU-side work --- 

        // 1. Get the new texture data and metadata.
        let (texture_size, tile_handle) =
            texture_array::get_texmap_image(texture_id, images_resmut, texmap_2d);

        // 2. Allocate a layer, evicting an old one if necessary.
        let layer = self.allocate_layer(texture_size);

        // 3. Get the raw pixel data for the upload.
        let (width, height) = texture_size.dimensions();
        let tile_bytes: Vec<u8> = {
            let tile_img = images_resmut.get(&tile_handle).unwrap();
            assert_eq!(tile_img.texture_descriptor.size.width, width);
            assert_eq!(tile_img.texture_descriptor.size.height, height);
            tile_img.data.as_ref().unwrap().clone()
        };

        Some(PreparedTextureUpload {
            texture_id,
            layer,
            size: texture_size,
            bytes: tile_bytes,
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
