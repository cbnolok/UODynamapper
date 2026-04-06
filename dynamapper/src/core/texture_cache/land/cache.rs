//! GPU texture array LRU cache supporting two texture sizes
//! Each texture_id can be either small or big and is mapped accordingly

#![allow(dead_code)]

use super::texture_array;
use crate::console_logger::{self, LogAbout, LogSev};
use bevy::prelude::*;
// use bevy::render::render_resource::*;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::RenderDevice;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::Extract;
use bevy::tasks::AsyncComputeTaskPool;
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};
use uddconv::bc7::{self, RawImageFormat, TextureUploadLayout};
use uocf::classic::land_texture_2d::{LandTextureSize, TexMap2D};

#[derive(Resource, Clone, ExtractResource)]
pub struct TextureArrayImageHandles {
    pub small: Handle<Image>,
    pub big: Handle<Image>,
}

#[derive(Clone)]
pub struct TextureArrayUpload {
    pub size: LandTextureSize,
    pub layer: u32,
    pub bytes: std::sync::Arc<[u8]>,
    pub upload_layout: TextureUploadLayout,
}

#[derive(Resource, Default)]
pub struct RenderTextureArrayUploads(pub Vec<TextureArrayUpload>);

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);
const FALLBACK_BLACK_LAYER: u32 = 0;
/// Number of textures to process per task in `precache_textures_parallel`.
/// Larger batches reduce task scheduling overhead and channel sends.
const PRECACHE_BATCH_SIZE: usize = 256;

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
    pub active_layers: u32,
    pub max_layers: u32,
    pub requested_resize_to: Option<u32>,
    pub last_requested_resize_logged: Option<u32>,
    free_layers: Vec<u32>,
    lru: VecDeque<u16>, // texture_id queue
    /// Timestamp of the last time usage was above `RESOURCE_SHRINK_THRESHOLD`.
    pub last_high_usage_instant: Instant,
}
impl LandTextureArrayWrapper {
    fn new(image_handle: Handle<Image>, initial_layers: u32, max_layers: u32) -> Self {
        Self {
            image_handle,
            active_layers: initial_layers,
            max_layers,
            requested_resize_to: None,
            last_requested_resize_logged: None,
            // Reserve layer 0 as a permanent black fallback tile.
            free_layers: (1..initial_layers).rev().collect(),
            lru: VecDeque::default(),
            last_high_usage_instant: Instant::now(),
        }
    }

    /// Number of layers currently in use (allocated – free).
    pub fn used_layers(&self) -> u32 {
        self.active_layers - self.free_layers.len() as u32
    }
}

#[derive(Resource)]
pub struct LandTextureCache {
    pub small: LandTextureArrayWrapper,
    pub big: LandTextureArrayWrapper,
    pub entry_by_id: Vec<Option<(LandTextureSize, LandTextureEntry)>>,
    pub pinned_visible_bits: Vec<u64>,
    pub visible_hint_count: usize,
    pub pending_uploads: Vec<TextureArrayUpload>,
    pub upload_receiver: std::sync::Mutex<std::sync::mpsc::Receiver<TextureArrayUpload>>,
    pub upload_sender: std::sync::mpsc::Sender<TextureArrayUpload>,
}

pub fn sys_drain_texture_compression_tasks(mut cache: ResMut<LandTextureCache>) {
    let mut batch = Vec::new();
    {
        let receiver = cache.upload_receiver.lock().unwrap();
        while let Ok(upload) = receiver.try_recv() {
            batch.push(upload);
        }
    }
    cache.pending_uploads.extend(batch);
}

fn prepare_texture_upload_bytes(
    raw_rgba8: std::sync::Arc<[u8]>,
    texture_size: LandTextureSize,
    compression: texture_array::TerrainTextureCompression,
) -> (std::sync::Arc<[u8]>, TextureUploadLayout) {
    let texture = bc7::encode_for_vram_arc(
        raw_rgba8,
        texture_array::texture_extent(texture_size),
        RawImageFormat::Rgba8888,
        texture_array::terrain_texture_vram_encoding(compression),
    )
    .expect("terrain texture VRAM encoding failed");

    let upload_layout = texture.upload_layout();
    (texture.into_bytes(), upload_layout)
}

impl LandTextureCache {
    pub const MAX_TILE_ID: usize = 65536;
    /// Shift by 6 is equivalent to division by 64 (the number of bits in a `u64`).
    pub const TILE_ID_WORD_SHIFT: u8 = 6;
    /// Masking with 63 (0x3F) is equivalent to modulo 64.
    pub const TILE_ID_BIT_MASK: usize = 63;
    pub const TILE_BITSET_SIZE: usize = Self::MAX_TILE_ID >> Self::TILE_ID_WORD_SHIFT;

    pub fn new(
        small_tex_image_handle: Handle<Image>,
        big_tex_image_handle: Handle<Image>,
        small_initial_layers: u32,
        big_initial_layers: u32,
    ) -> Self {
        let (upload_sender, upload_receiver) = std::sync::mpsc::channel();
        Self {
            small: LandTextureArrayWrapper::new(
                small_tex_image_handle,
                small_initial_layers,
                texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
            ),
            big: LandTextureArrayWrapper::new(
                big_tex_image_handle,
                big_initial_layers,
                texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
            ),
            entry_by_id: vec![None; Self::MAX_TILE_ID],
            pinned_visible_bits: vec![0u64; Self::TILE_BITSET_SIZE],
            visible_hint_count: 0,
            pending_uploads: Vec::new(),
            upload_receiver: std::sync::Mutex::new(upload_receiver),
            upload_sender,
        }
    }

    /// Clears all pinned texture IDs.  Call when the entire chunk set is
    /// invalidated (scale change, map switch) so stale pins don’t prevent
    /// the LRU from reclaiming layers that are no longer atlas-referenced.
    pub fn clear_pinned_textures(&mut self) {
        self.pinned_visible_bits.fill(0);
    }

    /// Gets the layer for a single texture. If not resident, it will be loaded, causing an async GPU upload.
    pub fn get_texture_size_layer(
        &mut self,
        texmap_2d: &Arc<TexMap2D>,
        texture_id: u16,
        compression: texture_array::TerrainTextureCompression,
        now: Instant,
    ) -> (LandTextureSize, u32) {
        // If texture is already resident, just return its info.
        if let Some(entry) = &mut self.entry_by_id[texture_id as usize] {
            entry.1.last_touch = now;
            return (entry.0, entry.1.layer);
        }

        // Not resident: load metadata and attempt to allocate a cache layer.
        let texture_size = texture_array::get_texmap_size_only(texture_id, &texmap_2d);
        let Some(layer) = self.allocate_layer(texture_size) else {
            // Expansion requested but not applied yet: render with black fallback layer.
            return (texture_size, FALLBACK_BLACK_LAYER);
        };

        let pool = AsyncComputeTaskPool::get();
        // Clone the Arc only on the slow (cache-miss) path.
        let texmap_2d_arc = texmap_2d.clone();
        let sender = self.upload_sender.clone();
        let task = pool.spawn(async move {
            let (_, raw_rgba8) =
                texture_array::get_texmap_raw_data(texture_id, &texmap_2d_arc, now);
            let (tile_bytes, upload_layout) =
                prepare_texture_upload_bytes(raw_rgba8, texture_size, compression);
            let _ = sender.send(TextureArrayUpload {
                layer,
                size: texture_size,
                bytes: tile_bytes,
                upload_layout,
            });
        });
        task.detach();

        // Update bookkeeping and return.
        self.update_bookkeeping(texture_id, texture_size, layer, now);
        (texture_size, layer)
    }

    /// Optimized batch pre-caching with multithreaded BC7 compression.
    /// Used during large scene loads to avoid main-thread stalls.
    pub fn precache_textures_parallel(
        &mut self,
        texture_ids: &[u16],
        texmap_2d: Arc<TexMap2D>,
        compression: texture_array::TerrainTextureCompression,
        now: Instant,
    ) {
        let pool = AsyncComputeTaskPool::get();

        let mut to_upload: Vec<(u16, LandTextureSize, u32)> = Vec::new();
        let mut skipped_due_to_pressure = 0usize;

        for &id in texture_ids {
            if self.entry_by_id[id as usize].is_none() {
                let size = super::texture_array::get_texmap_size_only(id, &texmap_2d);
                let Some(layer) = self.allocate_layer(size) else {
                    skipped_due_to_pressure += 1;
                    continue;
                };

                self.update_bookkeeping(id, size, layer, now);
                to_upload.push((id, size, layer));
            }
        }

        if to_upload.is_empty() {
            return;
        }

        console_logger::one(
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Pre-caching {} textures (Async BC7={})...",
                to_upload.len(),
                compression.lossy_backend().is_some()
            ),
        );

        // Spawn one task per PRECACHE_BATCH_SIZE textures rather than one task per texture.
        // Reduces scheduling and allocation overhead by ~100x (100 spawns → 1).
        for chunk in to_upload.chunks(PRECACHE_BATCH_SIZE) {
            let chunk: Vec<(u16, LandTextureSize, u32)> = chunk.to_vec();
            let texmap_2d_arc = texmap_2d.clone();
            let sender = self.upload_sender.clone();
            let task = pool.spawn(async move {
                for (id, size, layer) in chunk {
                    let (_, rgba8) =
                        super::texture_array::get_texmap_raw_data(id, &texmap_2d_arc, now);
                    let (tile_bytes, upload_layout) =
                        prepare_texture_upload_bytes(rgba8, size, compression);
                    let _ = sender.send(TextureArrayUpload {
                        layer,
                        size,
                        bytes: tile_bytes,
                        upload_layout,
                    });
                }
            });
            task.detach();
        }

        if skipped_due_to_pressure > 0 {
            console_logger::one(
                LogSev::Warn,
                LogAbout::Performance,
                &format!(
                    "Skipped precache for {} textures this frame due to cache pressure (expansion pending).",
                    skipped_due_to_pressure
                ),
            );
        }
    }

    /// Checks if a texture is resident. If not, allocates a layer and loads its data,
    /// returning a struct with all info needed to perform the upload and bookkeeping.
    fn prepare_texture_residency(
        &mut self,
        texture_id: u16,
        texmap_2d: &Arc<TexMap2D>,
        compression: texture_array::TerrainTextureCompression,
        now: Instant,
    ) -> Option<TextureArrayUpload> {
        // If resident, touch timestamp and return None as no upload is needed.
        if let Some(entry) = &mut self.entry_by_id[texture_id as usize] {
            entry.1.last_touch = now;
            return None;
        }

        // --- If not resident, perform CPU-side metadata lookup ---
        let texture_size = texture_array::get_texmap_size_only(texture_id, texmap_2d);
        let layer = self.allocate_layer(texture_size)?;

        let pool = AsyncComputeTaskPool::get();
        let texmap_2d_arc = texmap_2d.clone();
        let sender = self.upload_sender.clone();
        let task = pool.spawn(async move {
            let (_, raw_rgba8) =
                texture_array::get_texmap_raw_data(texture_id, &texmap_2d_arc, now);
            let (tile_bytes, upload_layout) =
                prepare_texture_upload_bytes(raw_rgba8, texture_size, compression);
            let _ = sender.send(TextureArrayUpload {
                layer,
                size: texture_size,
                bytes: tile_bytes,
                upload_layout,
            });
        });
        task.detach();

        None
    }

    /// Allocates a layer for a new texture, handling LRU eviction if the array is full.
    fn allocate_layer(&mut self, texture_size: LandTextureSize) -> Option<u32> {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        if let Some(l) = array.free_layers.pop() {
            // Track high usage for shrink heuristic.
            let usage = array.used_layers() as f32 / array.active_layers.max(1) as f32;
            if usage > texture_array::RESOURCE_SHRINK_THRESHOLD {
                array.last_high_usage_instant = Instant::now();
            }
            return Some(l);
        }

        // 1) Prefer evicting oldest NON-visible texture.
        let lru_len = array.lru.len();
        for _ in 0..lru_len {
            let Some(oldest) = array.lru.pop_front() else {
                break;
            };
            let Some(val) = &self.entry_by_id[oldest as usize] else {
                continue;
            };

            if val.0 != texture_size {
                array.lru.push_back(oldest);
                continue;
            }

            let word = (oldest as usize) >> 6;
            let bit = (oldest as usize) & 63;
            if (self.pinned_visible_bits[word] & (1u64 << bit)) != 0 {
                array.lru.push_back(oldest);
                continue;
            }

            let victim_entry = self.entry_by_id[oldest as usize].take().unwrap();
            return Some(victim_entry.1.layer);
        }

        // 2) No evictable non-visible texture found: request GPU array expansion.
        let desired = ((self.visible_hint_count as f32) * 1.5).ceil() as u32;
        let target_layers = desired.max(array.active_layers + 64).min(array.max_layers);
        if target_layers > array.active_layers {
            let previous_request = array.requested_resize_to;
            array.requested_resize_to =
                Some(previous_request.unwrap_or(target_layers).max(target_layers));

            if array.last_requested_resize_logged != Some(target_layers) {
                array.last_requested_resize_logged = Some(target_layers);
                console_logger::one(
                    LogSev::Info,
                    LogAbout::Performance,
                    &format!(
                        "Texture cache pressure ({:?}): expanding texarray {} -> {} layers (hinted: {}, desired={}).",
                        texture_size,
                        array.active_layers,
                        target_layers,
                        self.visible_hint_count,
                        desired
                    ),
                );
            }
        }

        // 3) No eviction fallback here: caller can skip/black-fallback until expansion is applied.
        None
    }

    pub fn take_resize_requests(&mut self) -> (Option<u32>, Option<u32>) {
        let small = self.small.requested_resize_to.take();
        let big = self.big.requested_resize_to.take();
        (small, big)
    }

    pub fn apply_array_resize(
        &mut self,
        size: LandTextureSize,
        new_handle: Handle<Image>,
        new_layers: u32,
    ) {
        let array = match size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };
        if new_layers == array.active_layers {
            return;
        }

        let old_layers = array.active_layers;
        array.image_handle = new_handle;
        array.active_layers = new_layers;
        array.last_requested_resize_logged = None;

        if new_layers > old_layers {
            // Growing: add newly available layers to the free list.
            for layer in (old_layers..new_layers).rev() {
                array.free_layers.push(layer);
            }
        } else {
            // Shrinking: evict entries whose layer index exceeds the new size,
            // rebuild the free list, and purge the LRU queue.
            let evicted: Vec<u16> = self
                .entry_by_id
                .iter()
                .enumerate()
                .filter_map(|(id, entry)| {
                    if let Some((s, e)) = entry {
                        if *s == size && e.layer >= new_layers {
                            Some(id as u16)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            for id in &evicted {
                self.entry_by_id[*id as usize] = None;
            }

            // Rebuild the free list from layers not occupied by surviving entries.
            // Using a bitmask instead of HashSet for higher performance and less allocation.
            let mut occupied_bitset = vec![0u64; (new_layers as usize >> 6) + 1];
            for entry in &self.entry_by_id {
                if let Some((s, e)) = entry {
                    if *s == size {
                        let word = e.layer as usize >> 6;
                        let bit = e.layer as usize & 63;
                        if word < occupied_bitset.len() {
                            occupied_bitset[word] |= 1 << bit;
                        }
                    }
                }
            }
            let arr = match size {
                LandTextureSize::Small => &mut self.small,
                LandTextureSize::Big => &mut self.big,
            };
            arr.free_layers.clear();
            // Layer 0 is reserved as fallback black.
            for l in (1..new_layers).rev() {
                let word = l as usize >> 6;
                let bit = l as usize & 63;
                if (occupied_bitset[word] & (1 << bit)) == 0 {
                    arr.free_layers.push(l);
                }
            }

            // Purge LRU queue of evicted entries.
            arr.lru.retain(|id| !evicted.contains(id));
            // Reset high-usage timestamp so we don't immediately re-shrink.
            arr.last_high_usage_instant = Instant::now();
        }
    }

    pub fn enqueue_reupload_for_size(
        &mut self,
        size: LandTextureSize,
        texmap_2d: Arc<TexMap2D>,
        compression: texture_array::TerrainTextureCompression,
        now: Instant,
    ) {
        let ids_to_restore: Vec<(u16, u32)> = self
            .entry_by_id
            .iter()
            .enumerate()
            .filter_map(|(id, entry)| {
                let (e_size, e_data) = entry.as_ref()?;
                if *e_size == size {
                    Some((id as u16, e_data.layer))
                } else {
                    None
                }
            })
            .collect();

        for (texture_id, layer) in ids_to_restore {
            let actual_size = texture_array::get_texmap_size_only(texture_id, &texmap_2d);
            let texmap_2d_arc = texmap_2d.clone();

            let pool = AsyncComputeTaskPool::get();
            let sender = self.upload_sender.clone();
            let task = pool.spawn(async move {
                let (_, raw_rgba8) =
                    texture_array::get_texmap_raw_data(texture_id, &texmap_2d_arc, now);
                let (tile_bytes, upload_layout) =
                    prepare_texture_upload_bytes(raw_rgba8, actual_size, compression);
                let _ = sender.send(TextureArrayUpload {
                    layer,
                    size: actual_size,
                    bytes: tile_bytes,
                    upload_layout,
                });
            });
            task.detach();
        }
    }

    /// Updates the cache's internal maps after a texture has been uploaded.
    fn update_bookkeeping(
        &mut self,
        texture_id: u16,
        texture_size: LandTextureSize,
        layer: u32,
        now: Instant,
    ) {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        self.entry_by_id[texture_id as usize] = Some((
            texture_size,
            LandTextureEntry {
                layer,
                last_touch: now,
            },
        ));
        array.lru.push_back(texture_id);
    }

    pub fn evict_idle_textures(&mut self, now: Instant) -> usize {
        let mut evicted_count = 0;
        let mut to_remove = Vec::new();

        for id in 0..self.entry_by_id.len() {
            if let Some((size, entry)) = &self.entry_by_id[id] {
                let word = id >> 6;
                let bit = id & 63;
                let is_pinned = (self.pinned_visible_bits[word] & (1u64 << bit)) != 0;

                if is_pinned {
                    self.entry_by_id[id].as_mut().unwrap().1.last_touch = now;
                } else if now - entry.last_touch >= CACHE_EVICT_AFTER {
                    to_remove.push((id as u16, *size, *entry));
                }
            }
        }

        for (id, size, entry) in to_remove {
            self.entry_by_id[id as usize] = None;
            self.free_layer_for_entry(size, entry);
            evicted_count += 1;
        }

        evicted_count
    }

    fn free_layer_for_entry(&mut self, texture_size: LandTextureSize, entry: LandTextureEntry) {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };
        array.free_layers.push(entry.layer);
        // Note: we don't bother scouring the LRU VecDeque for the ID.
        // allocate_layer handles stale/missing entries in its loop.
    }

    /// Returns a requested shrink target for each texture array if usage has
    /// been well below capacity for longer than `RESOURCE_SHRINK_TIMEOUT_SECS`.
    /// The caller is responsible for creating the new GPU image and calling
    /// `apply_array_resize` + `enqueue_reupload_for_size`.
    pub fn check_shrink_opportunity(&self, now: Instant) -> (Option<u32>, Option<u32>) {
        let timeout = Duration::from_secs(texture_array::RESOURCE_SHRINK_TIMEOUT_SECS);

        let check = |arr: &LandTextureArrayWrapper, initial: u32| -> Option<u32> {
            let used = arr.used_layers();
            let usage_ratio = used as f32 / arr.active_layers.max(1) as f32;
            if usage_ratio >= texture_array::RESOURCE_SHRINK_THRESHOLD {
                return None; // still busy
            }
            if now.duration_since(arr.last_high_usage_instant) < timeout {
                return None; // haven't been idle long enough
            }
            // Target: next power-of-two above (used * 1.5), but not below initial.
            let target = ((used as f32 * 1.5).ceil() as u32)
                .next_power_of_two()
                .max(initial)
                .min(arr.active_layers); // never grow
            if target < arr.active_layers {
                Some(target)
            } else {
                None
            }
        };

        let small = check(
            &self.small,
            texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
        );
        let big = check(&self.big, texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS);
        (small, big)
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
        // Since bytes is Arc<Vec<u8>>, cloning is now O(1) and ultra-light.
        render_uploads
            .0
            .extend(cache.pending_uploads.iter().cloned());
    }
}

pub fn sys_clear_texture_array_uploads(mut cache: ResMut<LandTextureCache>) {
    cache.pending_uploads.clear();
}
#[derive(Default)]
pub struct PersistentStaging {
    pub buffer: Option<wgpu::Buffer>,
    pub capacity: usize,
}

pub fn sys_render_upload_texture_array(
    mut uploads: ResMut<RenderTextureArrayUploads>,
    handles: Res<TextureArrayImageHandles>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
    render_device: Res<RenderDevice>,
    mut staging: Local<PersistentStaging>,
) {
    if uploads.0.is_empty() {
        return;
    }

    let small_gpu = gpu_images.get(&handles.small);
    let big_gpu = gpu_images.get(&handles.big);

    if small_gpu.is_none() && big_gpu.is_none() {
        uploads.0.clear();
        return;
    }

    let device = render_device.wgpu_device();

    let mut required_capacity = 0;
    for upload in &uploads.0 {
        let row_bytes = upload.upload_layout.bytes_per_row as usize;
        let padded_row_bytes = (row_bytes + 255) & !255;
        let rows = upload.upload_layout.rows_per_image as usize;
        required_capacity += padded_row_bytes * rows;
    }

    if staging.capacity < required_capacity {
        let new_cap = required_capacity.next_power_of_two().max(4 * 1024 * 1024);
        staging.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Persistent Texture Staging Buffer"),
            size: new_cap as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        staging.capacity = new_cap;
    }

    let mut staging_bytes = Vec::with_capacity(required_capacity);

    for upload in &uploads.0 {
        let row_bytes = upload.upload_layout.bytes_per_row as usize;
        let padded_row_bytes = (row_bytes + 255) & !255;
        let rows = upload.upload_layout.rows_per_image as usize;

        for r in 0..rows {
            let src_start = r * row_bytes;
            let src_end = src_start + row_bytes;
            staging_bytes.extend_from_slice(&upload.bytes[src_start..src_end]);
            staging_bytes.resize(staging_bytes.len() + (padded_row_bytes - row_bytes), 0);
        }
    }

    render_queue.write_buffer(staging.buffer.as_ref().unwrap(), 0, &staging_bytes);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Batched Texture Uploads Encoder"),
    });

    let mut current_offset = 0;

    use wgpu::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo};

    for upload in uploads.0.drain(..) {
        let gpu_image = match upload.size {
            LandTextureSize::Small => small_gpu,
            LandTextureSize::Big => big_gpu,
        };
        let Some(gpu_image) = gpu_image else {
            continue;
        };

        let (width, height) = upload.size.dimensions();
        let row_bytes = upload.upload_layout.bytes_per_row as usize;
        let padded_row_bytes = (row_bytes + 255) & !255;
        let rows = upload.upload_layout.rows_per_image as usize;

        let offset = current_offset as u64;
        current_offset += padded_row_bytes * rows;

        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: staging.buffer.as_ref().unwrap(),
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(padded_row_bytes as u32),
                    rows_per_image: Some(upload.upload_layout.rows_per_image),
                },
            },
            TexelCopyTextureInfo {
                texture: &*gpu_image.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: 0,
                    y: 0,
                    z: upload.layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    render_queue.submit(Some(encoder.finish()));
}
