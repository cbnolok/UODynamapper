//! GPU texture array + LRU eviction.
//! of reading PNG files from disk.

#![allow(dead_code)]

use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use super::texarray;

pub const TILE_PX: u32                  = 44;
pub const TEXARRAY_MAX_TILE_LAYERS: u32 = 2_048;

const CACHE_EVICT_AFTER: Duration    = Duration::from_secs(300);


#[derive(Clone)]
struct TextureEntry {
    layer:        u32,
    last_touch:   Instant,
}

#[derive(Resource)]
pub struct TextureCache {
    pub image_handle:   Handle<Image>,
    map:                HashMap<u16, TextureEntry>,   // art_id → entry
    free_layers:        Vec<u32>,
    lru:                VecDeque<u16>,         // queue of art_ids
}

impl TextureCache {
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
        art_id: u16,
        commands: &mut Commands,
        images: &mut ResMut<Assets<Image>>,
    ) -> u32 {
        // -----------------------------------------------------------------
        // 1. Fast-path: already resident?
        // -----------------------------------------------------------------
        if let Some(e) = self.map.get_mut(&art_id) {
            e.last_touch = Instant::now();
            return e.layer;
        }

        // -----------------------------------------------------------------
        // 2. Pick a texture-array layer (free or by eviction)
        // -----------------------------------------------------------------
        let layer = if let Some(l) = self.free_layers.pop() {
            l
        } else {
            let victim_id = loop {
                let oldest = self.lru.pop_front().unwrap();
                let still  = self.map.get(&oldest).unwrap();
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
        let tile_handle = texarray::get_tile_image(art_id, commands, images);

        // Grab the bytes we are going to copy, then drop the borrow
        let tile_bytes: Vec<u8> = {
            let tile_img = images.get(&tile_handle).unwrap();   // immutable borrow
            assert_eq!(tile_img.texture_descriptor.size.width,  TILE_PX);
            assert_eq!(tile_img.texture_descriptor.size.height, TILE_PX);
            tile_img.data.as_ref().unwrap().clone()
        };

        // -----------------------------------------------------------------
        // 4. Now obtain a *mutable* borrow to the array texture and copy
        // -----------------------------------------------------------------
        {
            let slice      = (TILE_PX * TILE_PX * 4) as usize;  // TODO: why multiply by 4?
            let offset     = layer as usize * slice;
            let array_img = images.get_mut(&self.image_handle).unwrap();
            if let Some(data) = &mut array_img.data {
                data[offset..offset + slice].copy_from_slice(&tile_bytes)
            };
        } // `array_img` borrow ends here

        // -----------------------------------------------------------------
        // 5. Book-keeping
        // -----------------------------------------------------------------
        self.map.insert(
            art_id,
            TextureEntry { layer, last_touch: Instant::now() },
        );
        self.lru.push_back(art_id);

        layer
    }
}

