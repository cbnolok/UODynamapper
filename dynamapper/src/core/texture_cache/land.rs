pub mod cache;
pub mod texture_array;

use crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use std::time::{Duration, Instant};
use uocf::geo::land_texture_2d::LandTextureSize;
use uocf::geo::map::MapBlockRelPos;

pub struct LandTextureCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(LandTextureCachePlugin);

impl Plugin for LandTextureCachePlugin {
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            cache::TextureArrayImageHandles,
        >::default())
            .add_systems(
                Startup,
                sys_setup_terrain_cache
                    .in_set(StartupSysSet::SetupSceneStage1)
                    .after(StartupSysSet::LoadStartupUOFiles),
            );

        // Clear pending uploads at the start of each frame (First schedule), which runs BEFORE Update.
        // This is correct: Extract runs AFTER Last (end of previous frame), so by the time we reach
        // First of the next frame, the previous frame's uploads have already been consumed by Extract.
        // Clearing here ensures Update fills a fresh list for the current frame.
        app.add_systems(First, cache::sys_clear_texture_array_uploads);
        app.add_systems(Update, cache::sys_drain_texture_compression_tasks);

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        render_app.init_resource::<cache::RenderTextureArrayUploads>();
        render_app.add_systems(
            bevy::render::ExtractSchedule,
            cache::sys_extract_texture_array_uploads,
        );

        render_app.add_systems(
            bevy::render::Render,
            cache::sys_render_upload_texture_array.in_set(bevy::render::RenderSystems::Queue),
        );

        app.add_systems(
            Update,
            (sys_pin_active_textures, sys_evict_idle_land_cache)
                .chain()
                .run_if(on_timer(Duration::from_secs(5))),
        );
        app.add_systems(Update, sys_apply_texture_array_expansion);
        app.add_systems(Update, sys_apply_tile_atlas_expansion);
    }
}

fn sys_pin_active_textures(
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut map_planes_r: ResMut<crate::core::uo_files_loader::MapPlanesRes>,
    scene_state_r: Res<crate::core::render::scene::SceneStateData>,
    chunk_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    cache_r.clear_pinned_textures();

    let plane = map_planes_r
        .0
        .get_mut(scene_state_r.map_id as usize)
        .and_then(|opt| opt.as_mut())
        .expect("Uncached Map in sys_pin_active_textures");

    for mesh in chunk_q.iter() {
        let gx = mesh.gx as i32;
        let gy = mesh.gy as i32;
        let scale = mesh.scale as i32;

        for sx in -1..=scale {
            for sz in -1..=scale {
                let bx = gx + sx;
                let bz = gy + sz;
                if bx >= 0
                    && bx < plane.size_blocks.width as i32
                    && bz >= 0
                    && bz < plane.size_blocks.height as i32
                {
                    let pos = MapBlockRelPos {
                        x: bx as u32,
                        y: bz as u32,
                    };
                    if let Some(block) = plane.block(pos) {
                        for cell in &block.cells {
                            let cell_id = cell.id as usize;
                            let word = cell_id >> 6;
                            if word < cache_r.pinned_visible_bits.len() {
                                cache_r.pinned_visible_bits[word] |= 1u64 << (cell_id & 63);
                            }
                        }
                    }
                }
            }
        }
    }

    cache_r.visible_hint_count = cache_r
        .pinned_visible_bits
        .iter()
        .map(|w| w.count_ones() as usize)
        .sum();
}

fn sys_evict_idle_land_cache(
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut map_planes_r: ResMut<crate::core::uo_files_loader::MapPlanesRes>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    scene_state_r: Res<crate::core::render::scene::SceneStateData>,
    mut tile_atlas: ResMut<crate::core::render::scene::world::land::tile_atlas::TileAtlas>,
    time: Res<Time<Real>>,
) {
    let now = time.last_update().unwrap_or_else(|| Instant::now());
    // 1. Evict idle GPU layers from the Texture Array cache (VRAM/LRU management)
    let evicted_gpu_layers = cache_r.evict_idle_textures(now);
    if evicted_gpu_layers > 0 {
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Evicted {} idle textures from GPU cache.",
                evicted_gpu_layers
            ),
        );
    }

    // 2. Evict idle pixel data from the raw TexMap2D cache (CPU RAM)
    let evicted_pixel_buffers = texmap_2d_r.0.evict_idle_textures(Duration::from_secs(60));
    if evicted_pixel_buffers > 0 {
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Evicted {} idle pixel buffers from TexMap2D cache.",
                evicted_pixel_buffers
            ),
        );
    }

    // 3. Evict idle map blocks from the active map plane (CPU RAM)
    if let Some(plane) = map_planes_r
        .0
        .get_mut(scene_state_r.map_id as usize)
        .and_then(|opt| opt.as_mut())
    {
        let evicted_blocks = plane.evict_idle_blocks(Duration::from_secs(60));
        if evicted_blocks > 0 {
            console_logger::one(
                None,
                LogSev::Info,
                LogAbout::Performance,
                &format!(
                    "Evicted {} idle blocks from MapPlane {}.",
                    evicted_blocks, scene_state_r.map_id
                ),
            );
        }
    };

    // 4. Check whether the texture arrays can be shrunk (usage low for >2 min).
    let (shrink_small, shrink_big) = cache_r.check_shrink_opportunity(now);
    if let Some(target) = shrink_small {
        cache_r.small.requested_resize_to = Some(target);
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Requesting texture array shrink (Small): {} → {} layers.",
                cache_r.small.active_layers, target
            ),
        );
    }
    if let Some(target) = shrink_big {
        cache_r.big.requested_resize_to = Some(target);
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Requesting texture array shrink (Big): {} → {} layers.",
                cache_r.big.active_layers, target
            ),
        );
    }

    // 5. Check whether the tile metadata atlas can be shrunk.
    {
        let timeout = Duration::from_secs(texture_array::RESOURCE_SHRINK_TIMEOUT_SECS);
        let mapped = tile_atlas.mapped_page_count();
        let capacity = tile_atlas.params.max_layers;
        let usage_ratio = mapped as f32 / capacity.max(1) as f32;
        let elapsed = now.duration_since(tile_atlas.last_high_usage_instant);

        if usage_ratio < texture_array::RESOURCE_SHRINK_THRESHOLD
            && elapsed >= timeout
            && capacity > texture_array::TILE_ATLAS_INITIAL_LAYERS
            && tile_atlas.requested_expansion.is_none()
        {
            let target = ((mapped as f32 * 1.5).ceil() as u32)
                .next_power_of_two()
                .max(texture_array::TILE_ATLAS_INITIAL_LAYERS)
                .min(capacity);
            if target < capacity {
                tile_atlas.requested_expansion = Some(target);
                console_logger::one(
                    None,
                    LogSev::Info,
                    LogAbout::Performance,
                    &format!("Requesting tile atlas shrink: {capacity} → {target} layers.",),
                );
            }
        }
    }
}

pub fn sys_setup_terrain_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    settings: Res<crate::external_data::settings::Settings>,
) {
    log_system_add_startup::<LandTextureCachePlugin>(StartupSysSet::SetupSceneStage1, fname!());

    let lossy: bool = settings.core.graphics.lossy_texture_compression;
    let handle_small = texture_array::create_gpu_texture_array(
        "land_small_texture_cache",
        &mut images,
        LandTextureSize::Small,
        lossy,
        texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
    );
    let handle_big = texture_array::create_gpu_texture_array(
        "land_big_texture_cache",
        &mut images,
        LandTextureSize::Big,
        lossy,
        texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS,
    );
    cmd.insert_resource(cache::LandTextureCache::new(
        handle_small.clone(),
        handle_big.clone(),
        texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
        texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS,
    ));

    use crate::core::render::scene::world::land::{
        draw_mesh::SharedLandMaterial,
        mesh_material::{LandMaterialExtension, SceneUniform},
        tile_atlas::{AtlasParams, TileAtlas, TileAtlasImageHandle},
    };
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    cmd.insert_resource(cache::TextureArrayImageHandles {
        small: handle_small.clone(),
        big: handle_big.clone(),
    });

    let page_texels = UVec2::splat(texture_array::TILE_ATLAS_PAGE_TEXELS);
    let tiles_per_page = UVec2::splat(texture_array::TILE_ATLAS_TILES_PER_PAGE);
    // Start small — the runtime will grow the atlas on demand if more pages
    // are needed (see sys_apply_tile_atlas_expansion).
    let max_layers = texture_array::TILE_ATLAS_INITIAL_LAYERS;
    let params = AtlasParams {
        page_texels,
        tiles_per_page,
        max_layers,
        world_pages_x: texture_array::TILE_ATLAS_WORLD_PAGES_X,
        _pad: UVec2::ZERO,
        page_to_layer: [bevy::math::UVec4::MAX; 64],
    };

    let tile_atlas = TileAtlas::new(params, texture_array::TILE_ATLAS_MAX_LAYERS);
    cmd.insert_resource(tile_atlas);

    let extent = Extent3d {
        width: page_texels.x,
        height: page_texels.y,
        depth_or_array_layers: max_layers,
    };
    let size_bytes = (extent.width
        * extent.height
        * extent.depth_or_array_layers
        * texture_array::TILE_ATLAS_BYTES_PER_TEXEL) as usize;
    let data: Vec<u8> = vec![0u8; size_bytes];

    let mut image = Image::new(
        extent,
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD, //| bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let atlas_image_handle = images.add(image);
    cmd.insert_resource(TileAtlasImageHandle(atlas_image_handle.clone()));

    let shared_mat =
        crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial {
            base: StandardMaterial {
                unlit: true,
                ..Default::default()
            },
            extension: LandMaterialExtension {
                texarray_small: handle_small,
                texarray_big: handle_big,
                tile_meta_atlas: atlas_image_handle,
                atlas_params: params,
                scene_uniform: SceneUniform {
                    camera_position:
                        crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER,
                    _pad_cam: 0.0,
                    light_direction: crate::core::constants::BAKED_GLOBAL_LIGHT.normalize(),
                    global_lighting: 1.0,
                    render_zoom: 1.0,
                    adaptive_zoom_simplification: 0.0,
                    _pad: Vec2::ZERO,
                },
                effects_uniform: Default::default(),
                global_lighting_uniform: Default::default(),
                land_lighting_uniform: Default::default(),
            },
        };

    let shared_mat_handle = materials.add(shared_mat);
    cmd.insert_resource(SharedLandMaterial(shared_mat_handle));
}

fn sys_apply_texture_array_expansion(
    mut images: ResMut<Assets<Image>>,
    mut cache_r: ResMut<cache::LandTextureCache>,
    mut handles_r: ResMut<cache::TextureArrayImageHandles>,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    settings: Res<crate::external_data::settings::Settings>,
    time: Res<Time<Real>>,
) {
    let now = time.last_update().unwrap_or_else(|| Instant::now());
    let lossy_compression = settings.core.graphics.lossy_texture_compression;

    let (small_req, big_req) = cache_r.take_resize_requests();
    if small_req.is_none() && big_req.is_none() {
        return;
    }

    let mut resized_small: Option<u32> = None;
    let mut resized_big: Option<u32> = None;

    if let Some(req) = small_req {
        let new_layers = req.clamp(
            texture_array::TEXARRAY_SMALL_INITIAL_TILE_LAYERS,
            texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
        );
        if new_layers != cache_r.small.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_small_texture_cache",
                &mut images,
                LandTextureSize::Small,
                lossy_compression,
                new_layers,
            );
            handles_r.small = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Small, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Small,
                texmap_2d_r.0.clone(),
                lossy_compression,
                now,
            );
            resized_small = Some(new_layers);
        }
    }

    if let Some(req) = big_req {
        let new_layers = req.clamp(
            texture_array::TEXARRAY_BIG_INITIAL_TILE_LAYERS,
            texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
        );
        if new_layers != cache_r.big.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_big_texture_cache",
                &mut images,
                LandTextureSize::Big,
                lossy_compression,
                new_layers,
            );
            handles_r.big = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Big, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Big,
                texmap_2d_r.0.clone(),
                lossy_compression,
                now,
            );
            resized_big = Some(new_layers);
        }
    }

    // CRITICAL: Only call materials.get_mut() when a resize actually happened.
    // Calling get_mut() every frame triggers Bevy's asset change detection,
    // which forces re-extraction of the entire material bind group (3 texture
    // arrays + 5 uniform buffers) for ALL chunk entities every frame. This was
    // the root cause of 95% GPU usage at idle — Bevy re-uploaded all texture
    // array bind groups every frame instead of reusing the cached GPU state.
    if resized_small.is_some() || resized_big.is_some() {
        if let Some(mat) = materials.get_mut(&shared_mat.0) {
            mat.extension.texarray_small = handles_r.small.clone();
            mat.extension.texarray_big = handles_r.big.clone();
        }

        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Resized terrain texture arrays: small={} layers, big={} layers.",
                cache_r.small.active_layers, cache_r.big.active_layers
            ),
        );
    }
}

/// Grows (or shrinks) the tile metadata atlas when `TileAtlas::requested_expansion`
/// has been set by the LRU paging logic (or when the shrink heuristic fires).
fn sys_apply_tile_atlas_expansion(
    mut images: ResMut<Assets<Image>>,
    mut tile_atlas: ResMut<crate::core::render::scene::world::land::tile_atlas::TileAtlas>,
    mut atlas_handle: ResMut<
        crate::core::render::scene::world::land::tile_atlas::TileAtlasImageHandle,
    >,
    mut materials: ResMut<Assets<LandCustomMeshMaterial>>,
    shared_mat: Res<crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial>,
    mut commands: Commands,
    chunks: Query<Entity, With<crate::core::render::scene::world::land::LCMesh>>,
) {
    let Some(new_layers) = tile_atlas.requested_expansion else {
        return;
    };
    let new_layers = new_layers.clamp(
        texture_array::TILE_ATLAS_INITIAL_LAYERS,
        tile_atlas.max_layers_limit,
    );
    if new_layers == tile_atlas.params.max_layers {
        tile_atlas.requested_expansion = None;
        return;
    }

    let old_layers = tile_atlas.params.max_layers;
    let page_texels = tile_atlas.params.page_texels;

    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let extent = Extent3d {
        width: page_texels.x,
        height: page_texels.y,
        depth_or_array_layers: new_layers,
    };
    let size_bytes = (extent.width
        * extent.height
        * extent.depth_or_array_layers
        * texture_array::TILE_ATLAS_BYTES_PER_TEXEL) as usize;
    let data: Vec<u8> = vec![0u8; size_bytes];

    let mut image = Image::new(
        extent,
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Uint,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let new_handle = images.add(image);
    atlas_handle.0 = new_handle.clone();

    // Clear all page mappings — the mesh renderer will re-populate them.
    tile_atlas.apply_resize(new_layers);

    for e in chunks.iter() {
        commands
            .entity(e)
            .insert(crate::core::render::scene::world::land::draw_mesh::PendingTextureBake);
    }

    // Update the shared material so the shader sees the new texture.
    if let Some(mat) = materials.get_mut(&shared_mat.0) {
        mat.extension.tile_meta_atlas = new_handle;
        mat.extension.atlas_params = tile_atlas.params;
    }

    let direction = if new_layers > old_layers {
        "Expanded"
    } else {
        "Shrunk"
    };
    console_logger::one(
        None,
        LogSev::Info,
        LogAbout::Performance,
        &format!(
            "{direction} tile metadata atlas: {old_layers} → {new_layers} layers \
             ({} MB VRAM).",
            (page_texels.x as u64
                * page_texels.y as u64
                * new_layers as u64
                * texture_array::TILE_ATLAS_BYTES_PER_TEXEL as u64)
                / (1024 * 1024)
        ),
    );
}
