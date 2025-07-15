// Each art tile has to be its own entity.

/*
    // Load the ground textures in our UO Data Cache.
    logger::one(
        None,
        logger::Severity::Info,
        logger::About::Systems,
        "Startup: load_res_terrain"
    );

    let uo_data = uo_datafiles_manager::get_ref();
    let texmap_2d = uo_data.texmap_2d.read().unwrap();
    for i_tex in 0..texmap_2d.len() {
        let tex = match texmap_2d.element(i_tex) {
            None => continue,
            Some(tex) => tex,
        };

        let img = Image::new(
            Extent3d {
                width: tex.size_x(),
                height: tex.size_y(),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            tex.pixel_data.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD
        );

        // Now create a Bevy Asset containing the texture.
        let handle = assets_image.add(img);
        land_texture_assets
            .images_2d
            .insert(tex.id, asset::UoImageResource { id: tex.id, handle });
    }

    logger::one(
        None,
        logger::Severity::Info,
        logger::About::AppState,
        "Setting: SetupRender",
    );

    next_state.set(AppState::SetupRenderer);
}
*/

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use guillotiere::{AllocId, Allocation, AtlasAllocator, size2};
use lru::LruCache;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ATLAS_DIM_PX: u32 = 2048;
const NUM_ATLASES: usize = 2;
const ATLAS_TIMEOUT: Duration = Duration::from_secs(60 * 2);

#[derive(Clone, Copy, Debug)]
pub struct AtlasUVInfo {
    pub atlas_idx: usize,
    pub uv_rect: [f32; 4], // (u0,v0,u1,v1) normalized
}

/// Each cached entry points to a rect in a given atlas.
struct CachedTile {
    tile_id: u32,
    atlas_idx: usize,
    alloc_id: AllocId,
    allocation: Allocation,
    last_access: Instant,
}

pub struct TileAtlasSet {
    atlases: [Handle<Image>; NUM_ATLASES],
    images: [Image; NUM_ATLASES],
    allocators: [AtlasAllocator; NUM_ATLASES],
    lrus: [LruCache<u32, CachedTile>; NUM_ATLASES],
    tile_map: HashMap<u32, (usize, AllocId)>,
    alloc_lookup: [HashMap<AllocId, u32>; NUM_ATLASES],
}

impl TileAtlasSet {
    pub fn new(images: &mut Assets<Image>) -> Self {
        let mut out_handles = [Handle::default(), Handle::default()];
        let mut out_images: [Image; NUM_ATLASES] = std::array::from_fn(|_| {
            Image::new_fill(
                Extent3d {
                    width: ATLAS_DIM_PX,
                    height: ATLAS_DIM_PX,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0, 0, 0, 255], // opaque black
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            )
        });

        // Insert into asset storage for handles
        for (i, atlas_img) in out_images.iter_mut().enumerate() {
            out_handles[i] = images.add(atlas_img.clone());
        }

        let allocators =
            [0, 1].map(|_| AtlasAllocator::new(size2(ATLAS_DIM_PX as i32, ATLAS_DIM_PX as i32)));

        let lrus = [0, 1].map(|_| LruCache::unbounded());
        let alloc_lookup = [HashMap::default(), HashMap::default()];

        Self {
            atlases: out_handles,
            images: out_images,
            allocators,
            lrus,
            tile_map: HashMap::default(),
            alloc_lookup,
        }
    }

    pub fn get_handles(&self) -> [Handle<Image>; NUM_ATLASES] {
        self.atlases.clone()
    }

    /// Returns (atlas idx, uv rect) for a tile, inserting if needed
    /// - On miss: runs `get_image_from_id`
    pub fn get_tile_uv(
        &mut self,
        images: &mut Assets<Image>,
        get_image_from_id: impl Fn(u32) -> Image,
        tile_id: u32,
        tile_width: u32,
        tile_height: u32,
    ) -> Option<AtlasUVInfo> {
        // Step 1: Already present?
        if let Some(&(atlas_idx, _alloc_id)) = self.tile_map.get(&tile_id) {
            // Lookup the tile in the atlas' LRU cache to get the allocation rectangle.
            if let Some(cached) = self.lrus[atlas_idx].get_mut(&tile_id) {
                cached.last_access = Instant::now();
                return Some(AtlasUVInfo {
                    atlas_idx,
                    uv_rect: uv_rect_from_guillotiere(&cached.allocation.rectangle),
                });
            } else {
                // This shouldn't happen if the caches are consistent, but handle gracefully.
                return None;
            }
        }

        // Step 2: Try to allocate space in an atlas
        let req_size = size2(tile_width as i32, tile_height as i32);

        for atlas_idx in 0..NUM_ATLASES {
            if let Some(allocation) = self.allocators[atlas_idx].allocate(req_size) {
                // Insert the tile image!
                let tile_img = get_image_from_id(tile_id);
                patch_into_atlas_with_rect(
                    &mut self.images[atlas_idx],
                    &allocation.rectangle,
                    &tile_img,
                );
                if let Some(main_image) = images.get_mut(&self.atlases[atlas_idx]) {
                    patch_into_atlas_with_rect(main_image, &allocation.rectangle, &tile_img);
                }

                let cached = CachedTile {
                    tile_id,
                    atlas_idx,
                    alloc_id: allocation.id,
                    allocation: allocation.clone(),
                    last_access: Instant::now(),
                };
                self.lrus[atlas_idx].put(tile_id, cached);
                self.tile_map.insert(tile_id, (atlas_idx, allocation.id));
                self.alloc_lookup[atlas_idx].insert(allocation.id, tile_id);

                return Some(AtlasUVInfo {
                    atlas_idx,
                    uv_rect: uv_rect_from_guillotiere(&allocation.rectangle),
                });
            }
        }

        // Step 3: No room--evict LRU and try again
        for atlas_idx in 0..NUM_ATLASES {
            if let Some((_, victim)) = self.lrus[atlas_idx].pop_lru() {
                // Remove from mapping and allocator
                self.tile_map.remove(&victim.tile_id);
                self.allocators[atlas_idx].deallocate(victim.alloc_id);
                self.alloc_lookup[atlas_idx].remove(&victim.alloc_id);

                // Now retry allocation
                if let Some(allocation) = self.allocators[atlas_idx].allocate(req_size) {
                    // Insert the tile image!
                    let tile_img = get_image_from_id(tile_id);
                    patch_into_atlas_with_rect(
                        &mut self.images[atlas_idx],
                        &allocation.rectangle,
                        &tile_img,
                    );
                    if let Some(main_image) = images.get_mut(&self.atlases[atlas_idx]) {
                        patch_into_atlas_with_rect(main_image, &allocation.rectangle, &tile_img);
                    }

                    let cached = CachedTile {
                        tile_id,
                        atlas_idx,
                        alloc_id: allocation.id,
                        allocation: allocation.clone(),
                        last_access: Instant::now(),
                    };
                    self.lrus[atlas_idx].put(tile_id, cached);
                    self.tile_map.insert(tile_id, (atlas_idx, allocation.id));
                    self.alloc_lookup[atlas_idx].insert(allocation.id, tile_id);

                    return Some(AtlasUVInfo {
                        atlas_idx,
                        uv_rect: uv_rect_from_guillotiere(&allocation.rectangle),
                    });
                } // else, continue to next atlas
            }
        }

        // Step 4: Completely out of space in all atlases for a tile of this size
        None
    }

    /// (Optional) Call every frame to clean up rarely-used tiles.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        for atlas_idx in 0..NUM_ATLASES {
            let to_remove: Vec<u32> = self.lrus[atlas_idx]
                .iter()
                .filter(|(_, tile)| now - tile.last_access > ATLAS_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();
            for tile_id in to_remove {
                if let Some(cached) = self.lrus[atlas_idx].pop(&tile_id) {
                    self.tile_map.remove(&tile_id);
                    self.allocators[atlas_idx].deallocate(cached.alloc_id);
                    self.alloc_lookup[atlas_idx].remove(&cached.alloc_id);
                }
            }
        }
    }
}

fn uv_rect_from_guillotiere(rect: &guillotiere::Rectangle) -> [f32; 4] {
    let u0 = rect.min.x as f32 / ATLAS_DIM_PX as f32;
    let v0 = rect.min.y as f32 / ATLAS_DIM_PX as f32;
    let u1 = rect.max.x as f32 / ATLAS_DIM_PX as f32;
    let v1 = rect.max.y as f32 / ATLAS_DIM_PX as f32;
    [u0, v0, u1, v1]
}

/// This patches a (possibly non-square!) tile img onto atlas at 'rect'
fn patch_into_atlas_with_rect(
    atlas_img: &mut Image,
    rect: &guillotiere::Rectangle,
    tile_img: &Image,
) {
    // Assumes tile_img dimensions fit rect exactly! (caller responsibility)
    let width = (rect.max.x - rect.min.x) as u32;
    let height = (rect.max.y - rect.min.y) as u32;
    let src_bpr = tile_img.texture_descriptor.size.width * 4; // RGBA8888
    let dst_bpr = atlas_img.texture_descriptor.size.width * 4;

    let tile_img_data = tile_img.data.as_deref().unwrap();
    let atlas_img_data = atlas_img.data.as_deref_mut().unwrap();
    for row in 0..height {
        let src_start = (row * src_bpr) as usize;
        let dst_start = ((rect.min.y as u32 + row) * dst_bpr + (rect.min.x as u32) * 4) as usize;
        let src_slice = &tile_img_data[src_start..src_start + (width * 4) as usize];
        let dst_slice = &mut atlas_img_data[dst_start..dst_start + (width * 4) as usize];
        dst_slice.copy_from_slice(src_slice);
    }
}
