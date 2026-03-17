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
use bevy::tasks::ComputeTaskPool;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use uocf::geo::land_texture_2d::{LandTextureSize, TexMap2D};

#[derive(Resource, Clone, ExtractResource)]
pub struct TextureArrayImageHandles {
    pub small: Handle<Image>,
    pub big: Handle<Image>,
}

#[derive(Clone)]
pub struct TextureArrayUpload {
    pub size: LandTextureSize,
    pub layer: u32,
    pub bytes: std::sync::Arc<Vec<u8>>,
    /// True if `bytes` contains BC7-compressed data instead of raw RGBA8.
    pub lossy_compressed: bool,
}

#[derive(Resource, Default)]
pub struct RenderTextureArrayUploads(pub Vec<TextureArrayUpload>);

const CACHE_EVICT_AFTER: Duration = Duration::from_secs(300);
const FALLBACK_BLACK_LAYER: u32 = 0;

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
        }
    }
}

#[derive(Resource)]
pub struct LandTextureCache {
    pub small: LandTextureArrayWrapper,
    pub big: LandTextureArrayWrapper,
    entry_by_id: HashMap<u16, (LandTextureSize, LandTextureEntry)>,
    pinned_visible_ids: HashSet<u16>,
    visible_hint_count: usize,
    pub pending_uploads: Vec<TextureArrayUpload>,
}

impl LandTextureCache {
    pub fn new(
        small_tex_image_handle: Handle<Image>,
        big_tex_image_handle: Handle<Image>,
        initial_layers: u32,
    ) -> Self {
        Self {
            small: LandTextureArrayWrapper::new(
                small_tex_image_handle,
                initial_layers,
                texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
            ),
            big: LandTextureArrayWrapper::new(
                big_tex_image_handle,
                initial_layers,
                texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
            ),
            entry_by_id: HashMap::default(),
            pinned_visible_ids: HashSet::default(),
            visible_hint_count: 0,
            pending_uploads: Vec::new(),
        }
    }

    pub fn set_visible_texture_usage_hint(&mut self, visible_texture_ids: &HashSet<u16>) {
        self.pinned_visible_ids.clear();
        self.pinned_visible_ids.extend(visible_texture_ids.iter().copied());
        self.visible_hint_count = visible_texture_ids.len();

        console_logger::one(
            None,
            LogSev::Debug,
            LogAbout::Performance,
            &format!(
                "Texture usage hint: visible_ids={}, active_layers(small={}, big={}), resident_textures={}",
                self.visible_hint_count,
                self.small.active_layers,
                self.big.active_layers,
                self.entry_by_id.len()
            ),
        );
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

        // Not resident: load metadata/data and attempt to allocate a cache layer.
        let (texture_size, raw_rgba8) = texture_array::get_texmap_raw_data(texture_id, &texmap_2d);
        let Some(layer) = self.allocate_layer(texture_size) else {
            // Expansion requested but not applied yet: render with black fallback layer.
            return (texture_size, FALLBACK_BLACK_LAYER);
        };

        let tile_bytes: Vec<u8> = if lossy_compression {
            texture_array::compress_rgba8_to_bc7(raw_rgba8.as_slice(), texture_size)
        } else {
            raw_rgba8.to_vec()
        };
        self.pending_uploads.push(TextureArrayUpload {
            layer,
            size: texture_size,
            bytes: std::sync::Arc::new(tile_bytes),
            lossy_compressed: lossy_compression,
        });

        // Update bookkeeping and return.
        self.update_bookkeeping(texture_id, texture_size, layer);
        (texture_size, layer)
    }

    /// Optimized batch pre-caching with multithreaded BC7 compression.
    /// Used during large scene loads to avoid main-thread stalls.
    pub fn precache_textures_parallel(
        &mut self,
        texture_ids: &[u16],
        texmap_2d: Arc<TexMap2D>,
        lossy_compression: bool,
    ) {
        let pool = ComputeTaskPool::get();
        let mut to_load = Vec::new();

        for &id in texture_ids {
            if !self.entry_by_id.contains_key(&id) {
                to_load.push(id);
            }
        }

        if to_load.is_empty() {
            return;
        }

        // Determine if we should use multithreading based on the count (user threshold: 1000)
        let use_mt = to_load.len() > 1000;

        if use_mt {
            console_logger::one(
                None,
                LogSev::Info,
                LogAbout::Performance,
                &format!("Par-compressing {} textures to BC7...", to_load.len()),
            );

            // Collect raw data for all textures first (CPU work, can be parallelized too but mostly I/O or mem copy)
            let raw_data: Vec<_> = to_load
                .iter()
                .map(|&id| {
                    let (size, rgba8) = super::texture_array::get_texmap_raw_data(id, &texmap_2d);
                    (id, size, rgba8)
                })
                .collect();

            // Compress in parallel
            let compressed_results: Vec<_> = pool.scope(|s| {
                for (_, size, rgba8) in &raw_data {
                    s.spawn(async move {
                        if lossy_compression {
                            (
                                super::texture_array::compress_rgba8_to_bc7(
                                    rgba8.as_slice(),
                                    *size,
                                ),
                                true,
                            )
                        } else {
                            (rgba8.to_vec(), false)
                        }
                    });
                }
            });

            // Associate layers and update bookkeeping
            let mut skipped_due_to_pressure = 0usize;
            for (i, (id, size, _)) in raw_data.into_iter().enumerate() {
                let Some(layer) = self.allocate_layer(size) else {
                    skipped_due_to_pressure += 1;
                    continue;
                };
                let (bytes, compressed) = compressed_results[i].clone();
                let upload = TextureArrayUpload {
                    layer,
                    size,
                    bytes: std::sync::Arc::new(bytes),
                    lossy_compressed: compressed,
                };
                self.pending_uploads.push(upload);
                self.update_bookkeeping(id, size, layer);
            }

            if skipped_due_to_pressure > 0 {
                console_logger::one(
                    None,
                    LogSev::Warn,
                    LogAbout::Performance,
                    &format!(
                        "Skipped precache for {} textures this frame due to cache pressure (expansion pending).",
                        skipped_due_to_pressure
                    ),
                );
            }
        } else {
            // Normal sequential loading
            for id in to_load {
                self.get_texture_size_layer(texmap_2d.clone(), id, lossy_compression);
            }
        }
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
        let (texture_size, raw_rgba8) = texture_array::get_texmap_raw_data(texture_id, texmap_2d);

        // 2. Allocate a layer, evicting an old one if necessary.
        let layer = self.allocate_layer(texture_size)?;

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
            bytes: std::sync::Arc::new(tile_bytes),
            lossy_compressed: lossy_compression,
        })
    }

    /// Allocates a layer for a new texture, handling LRU eviction if the array is full.
    fn allocate_layer(&mut self, texture_size: LandTextureSize) -> Option<u32> {
        let array = match texture_size {
            LandTextureSize::Small => &mut self.small,
            LandTextureSize::Big => &mut self.big,
        };

        if let Some(l) = array.free_layers.pop() {
            return Some(l);
        }

        // 1) Prefer evicting oldest NON-visible texture.
        let lru_len = array.lru.len();
        for _ in 0..lru_len {
            let Some(oldest) = array.lru.pop_front() else { break; };
            let Some((size, _)) = self.entry_by_id.get(&oldest) else {
                continue;
            };

            if *size != texture_size {
                array.lru.push_back(oldest);
                continue;
            }

            if self.pinned_visible_ids.contains(&oldest) {
                array.lru.push_back(oldest);
                continue;
            }

            let victim_entry: (LandTextureSize, LandTextureEntry) =
                self.entry_by_id.remove(&oldest).unwrap();
            return Some(victim_entry.1.layer);
        }

        // 2) No evictable non-visible texture found: request GPU array expansion.
        let desired = ((self.visible_hint_count as f32) * 1.25).ceil() as u32;
        let target_layers = desired
            .max(array.active_layers + 64)
            .min(array.max_layers);
        if target_layers > array.active_layers {
            let previous_request = array.requested_resize_to;
            array.requested_resize_to = Some(previous_request.unwrap_or(target_layers).max(target_layers));

            if array.last_requested_resize_logged != Some(target_layers) {
                array.last_requested_resize_logged = Some(target_layers);
                console_logger::one(
                    None,
                    LogSev::Info,
                    LogAbout::Performance,
                    &format!(
                        "Texture cache pressure ({:?}): requested array expansion {} -> {} layers (visible hint: {}, desired={}).",
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
        if new_layers <= array.active_layers {
            return;
        }

        let old_layers = array.active_layers;
        array.image_handle = new_handle;
        array.active_layers = new_layers;
        array.last_requested_resize_logged = None;
        for layer in (old_layers..new_layers).rev() {
            array.free_layers.push(layer);
        }
    }

    pub fn enqueue_reupload_for_size(
        &mut self,
        size: LandTextureSize,
        texmap_2d: Arc<TexMap2D>,
        lossy_compression: bool,
    ) {
        let ids_to_restore: Vec<(u16, u32)> = self
            .entry_by_id
            .iter()
            .filter_map(|(id, (s, e))| {
                if *s == size {
                    Some((*id, e.layer))
                } else {
                    None
                }
            })
            .collect();

        for (texture_id, layer) in ids_to_restore {
            let (_actual_size, raw_rgba8) = texture_array::get_texmap_raw_data(texture_id, &texmap_2d);
            let tile_bytes: Vec<u8> = if lossy_compression {
                texture_array::compress_rgba8_to_bc7(raw_rgba8.as_slice(), size)
            } else {
                raw_rgba8.to_vec()
            };
            self.pending_uploads.push(TextureArrayUpload {
                size,
                layer,
                bytes: Arc::new(tile_bytes),
                lossy_compressed: lossy_compression,
            });
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

    pub fn evict_idle_textures(&mut self) -> usize {
        let now = Instant::now();
        let mut evicted_count = 0;
        let mut to_remove = Vec::new();

        for (&id, (size, entry)) in &self.entry_by_id {
            if now - entry.last_touch >= CACHE_EVICT_AFTER {
                to_remove.push((id, *size, *entry));
            }
        }

        for (id, size, entry) in to_remove {
            self.entry_by_id.remove(&id);
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
pub struct StagingResources {
    texture: wgpu::Texture,
    buffer: wgpu::Buffer,
}

pub fn sys_render_upload_texture_array(
    mut uploads: ResMut<RenderTextureArrayUploads>,
    handles: Res<TextureArrayImageHandles>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_queue: ResMut<RenderQueue>,
    render_device: Res<RenderDevice>,
    mut bc7_encoder: Local<Option<block_compression::GpuBlockCompressor>>,
    mut staging_cache: Local<HashMap<LandTextureSize, StagingResources>>,
) {
    if uploads.0.is_empty() {
        return;
    }

    console_logger::one(
        None,
        LogSev::Debug,
        LogAbout::Performance,
        &format!(
            "[DBG-render] Processing {} texture array uploads",
            uploads.0.len()
        ),
    );

    let small_gpu = gpu_images.get(&handles.small);
    let big_gpu = gpu_images.get(&handles.big);

    if small_gpu.is_none() {
        console_logger::one(
            None,
            LogSev::Warn,
            LogAbout::Performance,
            "[DBG-render] small_gpu image NOT FOUND in RenderAssets",
        );
    }
    if big_gpu.is_none() {
        console_logger::one(
            None,
            LogSev::Warn,
            LogAbout::Performance,
            "[DBG-render] big_gpu image NOT FOUND in RenderAssets",
        );
    }

    if small_gpu.is_none() && big_gpu.is_none() {
        uploads.0.clear();
        return;
    }

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

        if upload.lossy_compressed {
            // --- GPU-based BC7 Compression ---
            let device = render_device.wgpu_device();
            // Bevy's RenderQueue derefs to the underlying wgpu::Queue
            let queue = &render_queue;

            if bc7_encoder.is_none() {
                // block_compression 0.7 needs device AND queue in new()
                // Use explicit wgpu types to avoid mismatch between bevy/block_compression
                // Reach deep into Bevy's wrappers to get an owned wgpu::Queue
                let wgpu_device = render_device.wgpu_device().clone();
                let wgpu_queue = (***render_queue).clone();
                *bc7_encoder = Some(block_compression::GpuBlockCompressor::new(
                    wgpu_device,
                    wgpu_queue.into_inner(),
                ));
            }
            let compressor = bc7_encoder.as_mut().unwrap();

            // --- GPU-based BC7 Compression ---
            let variant = block_compression::CompressionVariant::BC7(
                block_compression::BC7Settings::alpha_basic(),
            );

            // Recycle or create the source texture and destination buffer for this size
            let resources = staging_cache.entry(upload.size).or_insert_with(|| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("BC7 Compression Source Staging"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm, // BC7 needs raw data, not Srgb
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });

                let required_buffer_size = variant.blocks_byte_size(width, height);
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("BC7 Compression Destination Staging"),
                    size: required_buffer_size as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });

                StagingResources { texture, buffer }
            });

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &resources.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &**upload.bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let src_view = resources
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            // Add task to compressor
            compressor.add_compression_task(
                variant,
                &src_view,
                width,
                height,
                &resources.buffer,
                None,
                None,
            );

            // Execute compression on GPU
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("BC7 Compression Encoder"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("BC7 Compression Pass"),
                    timestamp_writes: None,
                });
                compressor.compress(&mut pass);
            }

            // Copy from staging buffer to final texture array
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &resources.buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(variant.bytes_per_row(width)),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &*gpu_image.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: upload.layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            queue.submit(Some(encoder.finish()));
        } else {
            // --- Direct Copy (uncompressed) ---
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
            render_queue.write_texture(destination, &**upload.bytes, data_layout, extent);
        }
    }
}
