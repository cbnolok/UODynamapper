pub mod cache;
pub mod texture_array;

use crate::core::render::scene::world::land::mesh_material::LandCustomMeshMaterial;
use crate::core::system_sets::*;
use crate::prelude::*;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use std::time::Duration;
use uocf::geo::land_texture_2d::LandTextureSize;

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
            sys_evict_idle_land_cache.run_if(on_timer(Duration::from_secs(5))),
        );
        app.add_systems(Update, sys_apply_texture_array_expansion);
    }
}

fn sys_evict_idle_land_cache(
    mut cache_r: ResMut<cache::LandTextureCache>,
    map_planes_r: ResMut<crate::core::uo_files_loader::MapPlanesRes>,
    texmap_2d_r: Res<crate::core::uo_files_loader::TexMap2DRes>,
    scene_state_r: Res<crate::core::render::scene::SceneStateData>,
) {
    // 1. Evict idle GPU layers from the Texture Array cache (VRAM/LRU management)
    let evicted_gpu_layers = cache_r.evict_idle_textures();
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
    let map_planes = map_planes_r.0.clone();
    if let Some(mut plane) = map_planes.get_mut(&scene_state_r.map_id) {
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
    // Store the compression setting so the per-tile upload path can encode BC7 when enabled.
    cmd.insert_resource(cache::LandTextureCacheSettings {
        lossy_texture_compression: lossy,
    });

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

    let page_texels = UVec2::new(2048, 2048);
    let tiles_per_page = UVec2::new(2048, 2048);
    // Reduce startup VRAM: 16 -> 8 layers halves atlas allocation.
    let max_layers = 8;
    let params = AtlasParams {
        page_texels,
        tiles_per_page,
        max_layers,
        world_pages_x: 16, // Fixed stride for up to 32k x 32k maps
        _pad: UVec2::ZERO,
        page_to_layer: [bevy::math::UVec4::MAX; 64],
    };

    let tile_atlas = TileAtlas::new(params);
    cmd.insert_resource(tile_atlas);

    let extent = Extent3d {
        width: page_texels.x,
        height: page_texels.y,
        depth_or_array_layers: max_layers,
    };
    let size_bytes = (extent.width * extent.height * extent.depth_or_array_layers * 4) as usize;
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
                lighting_uniform: Default::default(),
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
    cache_settings_r: Res<cache::LandTextureCacheSettings>,
) {
    let (small_req, big_req) = cache_r.take_resize_requests();
    if small_req.is_none() && big_req.is_none() {
        return;
    }

    let mut resized_small: Option<u32> = None;
    let mut resized_big: Option<u32> = None;

    if let Some(req) = small_req {
        let new_layers = req.clamp(
            cache_r.small.active_layers,
            texture_array::TEXARRAY_SMALL_MAX_TILE_LAYERS,
        );
        if new_layers > cache_r.small.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_small_texture_cache",
                &mut images,
                LandTextureSize::Small,
                cache_settings_r.lossy_texture_compression,
                new_layers,
            );
            handles_r.small = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Small, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Small,
                texmap_2d_r.0.clone(),
                cache_settings_r.lossy_texture_compression,
            );
            resized_small = Some(new_layers);
        }
    }

    if let Some(req) = big_req {
        let new_layers = req.clamp(
            cache_r.big.active_layers,
            texture_array::TEXARRAY_BIG_MAX_TILE_LAYERS,
        );
        if new_layers > cache_r.big.active_layers {
            let new_handle = texture_array::create_gpu_texture_array(
                "land_big_texture_cache",
                &mut images,
                LandTextureSize::Big,
                cache_settings_r.lossy_texture_compression,
                new_layers,
            );
            handles_r.big = new_handle.clone();
            cache_r.apply_array_resize(LandTextureSize::Big, new_handle, new_layers);
            cache_r.enqueue_reupload_for_size(
                LandTextureSize::Big,
                texmap_2d_r.0.clone(),
                cache_settings_r.lossy_texture_compression,
            );
            resized_big = Some(new_layers);
        }
    }

    if let Some(mat) = materials.get_mut(&shared_mat.0) {
        mat.extension.texarray_small = handles_r.small.clone();
        mat.extension.texarray_big = handles_r.big.clone();
    }

    if resized_small.is_some() || resized_big.is_some() {
        console_logger::one(
            None,
            LogSev::Info,
            LogAbout::Performance,
            &format!(
                "Expanded terrain texture arrays: small={} layers, big={} layers.",
                cache_r.small.active_layers,
                cache_r.big.active_layers
            ),
        );
    }
}
