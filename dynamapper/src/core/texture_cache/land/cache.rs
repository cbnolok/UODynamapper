//! GPU texture array + LRU eviction.
//! of reading PNG files from disk.

#![allow(dead_code)]

use crate::core::uo_files_loader::UoFileData;
use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use super::texture_array;

pub const LAND_TEX_SIZE_SMALL: u32 = uocf::geo::land_texture_2d::TextureSize::SMALL_X;
pub const TEXARRAY_MAX_TILE_LAYERS: u32 = 2_048;

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct TextureEntry {
    layer: u32,
    last_touch: Instant,
}

#[derive(Resource)]
pub struct LandTextureCache {
    pub image_handle: Handle<Image>,
    map: HashMap<u16, TextureEntry>, // art_id → entry
    free_layers: Vec<u32>,
    lru: VecDeque<u16>, // queue of art_ids
}

impl LandTextureCache {
    pub fn new(image_handle: Handle<Image>) -> Self {
        Self {
            image_handle,
            map: HashMap::default(),
            free_layers: (0..TEXARRAY_MAX_TILE_LAYERS).rev().collect(),
            lru: VecDeque::default(),
        }
    }

    /// Ensure `art_id` is resident and return its layer index.
    pub fn layer_of(
        &mut self,
        commands: &mut Commands,
        images: &mut ResMut<Assets<Image>>,
        uo_data: &Res<UoFileData>,
        art_id: u16,
    ) -> u32 {
        // -----------------------------------------------------------------
        // 1. Fast-path: already resident?
        // -----------------------------------------------------------------
        if let Some(e) = self.map.get_mut(&art_id) {
            //println!("LandTextureCache: id {art_id} already cached in layer {}.", e.layer);
            e.last_touch = Instant::now();
            return e.layer;
        }
        //println!("LandTextureCache: id {art_id} not cached, inserting it.");

        // -----------------------------------------------------------------
        // 2. Pick a texture-array layer (free or by eviction)
        // -----------------------------------------------------------------
        let layer = if let Some(l) = self.free_layers.pop() {
            l
        } else {
            let victim_id = loop {
                let oldest = self.lru.pop_front().unwrap();
                let still = self.map.get(&oldest).unwrap();
                if Instant::now() - still.last_touch >= CACHE_EVICT_AFTER {
                    break oldest;
                }
                self.lru.push_back(oldest);
            };
            let victim_entry = self.map.remove(&victim_id).unwrap();
            victim_entry.layer
        };

        // -----------------------------------------------------------------
        // 3. Load (or generate) the source tile FIRST
        //    – this needs a &mut Assets<Image> because it may create assets
        // -----------------------------------------------------------------
        let tile_handle = texture_array::get_texmap_image(art_id, commands, images, &uo_data);

        // Grab the bytes we are going to copy, then drop the borrow
        let tile_bytes: Vec<u8> = {
            let tile_img = images.get(&tile_handle).unwrap(); // immutable borrow
            assert_eq!(tile_img.texture_descriptor.size.width, LAND_TEX_SIZE_SMALL);
            assert_eq!(tile_img.texture_descriptor.size.height, LAND_TEX_SIZE_SMALL);
            tile_img.data.as_ref().unwrap().clone()
        };

        // -----------------------------------------------------------------
        // 4. Now obtain a *mutable* borrow to the array texture and copy
        // -----------------------------------------------------------------
        {
            const BYTES_PER_PIXEL: usize = 4; // RGBA8888
            const LAYER_BYTE_SIZE: usize = LAND_TEX_SIZE_SMALL.pow(2) as usize * BYTES_PER_PIXEL;
            let offset = layer as usize * LAYER_BYTE_SIZE;
            let array_img = images.get_mut(&self.image_handle).unwrap();
            if let Some(data) = &mut array_img.data {
                data[offset..offset + LAYER_BYTE_SIZE].copy_from_slice(&tile_bytes);
            };
        } // `array_img` borrow ends here

        /*
        // Debug:
        dump_texture_array_layer(
            images,
            &self.image_handle,
            layer,
            LAND_TEX_SIZE_SMALL,
            &format!("tex_array_layer{layer}.png"),
        );
        */

        // -----------------------------------------------------------------
        // 5. Book-keeping
        // -----------------------------------------------------------------
        self.map.insert(
            art_id,
            TextureEntry {
                layer,
                last_touch: Instant::now(),
            },
        );
        self.lru.push_back(art_id);

        layer
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
