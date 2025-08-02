//! GPU texture array LRU cache supporting two texture sizes
//! Each texture_id can be either small or big and is mapped accordingly

#![allow(dead_code)]

use super::texture_array;
use crate::core::uo_files_loader::UoFileData;
use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use uocf::geo::land_texture_2d::LandTextureSize;

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug)]
pub struct LandTextureEntry {
    pub layer: u32,
    //pub tex_size: LandTextureSize,
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

    /// Ensure `texture_id` is resident and return (texture_size, layer)
    pub fn get_texture_size_layer(
        &mut self,
        images_resmut: &mut ResMut<Assets<Image>>,
        uo_data: &Res<UoFileData>,
        texture_id: u16,
    ) -> (LandTextureSize, u32) {
        // 1. Fast-path: already resident
        let e: Option<&mut (LandTextureSize, LandTextureEntry)> = self.entry_by_id.get_mut(&texture_id);
        if e.is_some() {
            let e: &mut (LandTextureSize, LandTextureEntry) = e.unwrap();
            e.1.last_touch = Instant::now();
            return (e.0, e.1.layer);
        }

        // 2. Get the new texture data and metadata.
        let (texture_size, tile_handle) =
            texture_array::get_texmap_image(texture_id, images_resmut, uo_data);

        // 2. Pick a tex array state
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        // 3. Choose or evict a layer
        let layer = if let Some(l) = array.free_layers.pop() {
            l
        } else {
            let victim_id = loop {
                let oldest = array
                    .lru
                    .pop_front()
                    .expect("LRU should not be empty at this stage");
                let still: &(LandTextureSize, LandTextureEntry) =
                    self.entry_by_id.get(&oldest).unwrap();
                if Instant::now() - still.1.last_touch >= CACHE_EVICT_AFTER {
                    break oldest;
                }
                array.lru.push_back(oldest);
            };
            let victim_entry: (LandTextureSize, LandTextureEntry) =
                self.entry_by_id.remove(&victim_id).unwrap();
            victim_entry.1.layer
        };

        // 4. Upload/copy texture data
        let (width, height) = texture_size.dimensions();
        let tile_bytes: Vec<u8> = {
            let tile_img = images_resmut.get(&tile_handle).unwrap();
            assert_eq!(tile_img.texture_descriptor.size.width, width);
            assert_eq!(tile_img.texture_descriptor.size.height, height);
            tile_img.data.as_ref().unwrap().clone()
        };
        {
            const BYTES_PER_PIXEL: usize = 4; // RGBA8888
            let layer_byte_size = (width * height) as usize * BYTES_PER_PIXEL;
            let offset = layer as usize * layer_byte_size;
            let array_img = images_resmut.get_mut(&array.image_handle).unwrap();
            if let Some(data) = &mut array_img.data {
                data[offset..offset + layer_byte_size].copy_from_slice(&tile_bytes);
            }
        }

        // 5. Bookkeeping
        self.entry_by_id.insert(
            texture_id,
            (
                texture_size,
                LandTextureEntry {
                    layer,
                    //texture_size,
                    last_touch: Instant::now(),
                },
            ),
        );
        array.lru.push_back(texture_id);

        (texture_size, layer)
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
