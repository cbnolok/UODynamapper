pub mod cache;
pub mod texture_array;

use crate::prelude::*;
use crate::core::system_sets::*;
use bevy::prelude::*;
use uocf::geo::land_texture_2d::LandTextureSize;

pub struct LandTextureCachePlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(LandTextureCachePlugin);

impl Plugin for LandTextureCachePlugin {
    /// Allocate GPU texture array for terrain tiles and TileCache.
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<cache::TextureArrayImageHandles>::default())
           .add_systems(
            Startup,
            sys_setup_terrain_cache
                .in_set(StartupSysSet::SetupSceneStage1)
                .after(StartupSysSet::LoadStartupUOFiles)
        );

        // Clear pending uploads at the start of each frame (First schedule), which runs BEFORE Update.
        // This is correct: Extract runs AFTER Last (end of previous frame), so by the time we reach
        // First of the next frame, the previous frame's uploads have already been consumed by Extract.
        // Clearing here ensures Update fills a fresh list for the current frame.
        app.add_systems(First, cache::sys_clear_texture_array_uploads);

        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else { return; };
        render_app.init_resource::<cache::RenderTextureArrayUploads>();
        render_app.add_systems(bevy::render::ExtractSchedule, cache::sys_extract_texture_array_uploads);
        
        render_app.add_systems(bevy::render::Render, cache::sys_render_upload_texture_array.in_set(bevy::render::RenderSet::Queue));
    }
}

pub fn sys_setup_terrain_cache(
    mut cmd: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<crate::core::render::scene::world::land::mesh_material::LandCustomMaterial>>,
    settings: Res<crate::external_data::settings::Settings>,
) {
    log_system_add_startup::<LandTextureCachePlugin>(StartupSysSet::SetupSceneStage1, fname!());

    let lossy = settings.graphics.lossy_texture_compression;
    let handle_small = texture_array::create_gpu_texture_array("land_small_texture_cache", &mut images, LandTextureSize::Small, lossy);
    let handle_big = texture_array::create_gpu_texture_array("land_big_texture_cache", &mut images, LandTextureSize::Big, lossy);
    cmd.insert_resource(cache::LandTextureCache::new(handle_small.clone(), handle_big.clone()));
    // Store the compression setting so the per-tile upload path can encode BC7 when enabled.
    cmd.insert_resource(cache::LandTextureCacheSettings { lossy_texture_compression: lossy });

    use bevy::render::render_resource::{TextureDimension, TextureFormat, TextureUsages, Extent3d};
    use crate::core::render::scene::world::land::tile_atlas::{TileAtlas, TileAtlasImageHandle, AtlasParams};
    use crate::core::render::scene::world::land::draw_mesh::SharedLandMaterial;
    use crate::core::render::scene::world::land::mesh_material::{LandMaterialExtension, SceneUniform};

    cmd.insert_resource(cache::TextureArrayImageHandles {
        small: handle_small.clone(),
        big: handle_big.clone(),
    });

    let page_texels = UVec2::new(2048, 2048);
    let tiles_per_page = UVec2::new(2048, 2048);
    let max_layers = 16;
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
    let data = vec![0u8; size_bytes];
    
    let mut image = Image::new(
        extent,
        TextureDimension::D2,
        data,
        TextureFormat::Rg16Uint,
        bevy::render::render_asset::RenderAssetUsages::RENDER_WORLD | bevy::render::render_asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..Default::default()
    });

    let atlas_image_handle = images.add(image);
    cmd.insert_resource(TileAtlasImageHandle(atlas_image_handle.clone()));

    let shared_mat = crate::core::render::scene::world::land::mesh_material::LandCustomMaterial {
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
                camera_position: crate::core::render::scene::camera::PlayerCamera::BASE_OFFSET_FROM_PLAYER,
                light_direction: crate::core::constants::BAKED_GLOBAL_LIGHT.normalize(),
                time_seconds: 0.0,
                global_lighting: 1.0,
            },
            effects_uniform: Default::default(),
            lighting_uniform: Default::default(),
        },
    };
    
    let shared_mat_handle = materials.add(shared_mat);
    cmd.insert_resource(SharedLandMaterial(shared_mat_handle));
}
