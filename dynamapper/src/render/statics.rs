use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::ExtractSchedule;
use crate::app::system_sets::{StartupSystems, SceneRenderLandSystems, SceneRenderArtSystems};
use crate::app::app_states::AppState;
use crate::prelude::*;
use crate::util_lib::tracked_plugin::TrackedPlugin;
use crate::render::lighting::{client_light_sprites, light_scene, static_lights};

pub mod statics_collect;
pub mod static_placement;
pub mod static_source;
pub mod static_batches;
pub mod static_chunk_cache;
mod static_chunk_builder;
pub mod statics_draw;
pub mod client_grunge;
mod static_surface;
pub(crate) mod transparent_order;
#[cfg(test)]
mod shading_tests;
#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod projected_shadow_tests;

pub struct DrawStaticSpritesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawStaticSpritesPlugin);

impl Plugin for DrawStaticSpritesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<static_batches::RenderStaticInstances>();
        app.init_resource::<static_batches::RenderStaticLandInstances>();
        app.init_resource::<static_batches::RenderStaticChunkBatches>();
        app.init_resource::<static_chunk_cache::StaticChunkRenderCache>();
        app.insert_resource(static_chunk_builder::StaticChunkBuilder::spawn());
        app.init_resource::<static_source::StaticSurfaceLikeResolutionCache>();
        app.init_resource::<statics_collect::StaticArtCollectDebugState>();
        app.init_resource::<static_source::StaticArtSourceState>();
        crate::render::picking::gpu_picking::install(app);
        crate::render::picking::hovered_static::install(app);
        light_scene::install(app);
        app.init_resource::<statics_draw::StaticArtDrawDebugState>();
        app.init_resource::<statics_draw::StaticArtUploadCache>();
        app.init_resource::<statics_draw::StaticBatchEntitySyncState>();
        app.init_resource::<statics_draw::ArtBatchMeshes>();
        app.init_resource::<static_lights::RenderStaticLightInstances>();
        app.init_resource::<static_lights::StaticLightDrawDebugState>();
        app.init_resource::<client_light_sprites::RenderClientLightSprites>();
        app.init_resource::<client_light_sprites::ClientLightSpriteCache>();
        app.insert_resource(crate::render::lighting::light_scale::ItemLightScales::load());
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            transparent_order::StaticTransparentBatchOrder,
        >::default());
        app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            transparent_order::ClientLightSpriteOrder,
        >::default());
        app.init_resource::<static_lights::StaticLightMaterialCache>();
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            crate::render::assets::art::SpriteArtPageAtlasHandles,
        >::default());
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            crate::render::assets::art::GroundArtPageAtlasHandles,
        >::default());

        app.add_plugins((
               MaterialPlugin::<statics_draw::ArtSpriteMaterial>::default(),
               MaterialPlugin::<statics_draw::ArtGroundMaterial>::default(),
           ))
           .add_systems(Startup, statics_draw::sys_setup_art_page_atlas
               .in_set(StartupSystems::SetupSceneStage1)
               .after(crate::render::assets::land::sys_setup_terrain_cache)
               .after(StartupSystems::LoadStartupUOFiles))
           .add_systems(Update, (
               static_source::sys_sync_static_art_source,
               statics_draw::sys_sync_active_art_page_atlases,
               statics_draw::sys_apply_pending_art_page_atlas_resizes,
               crate::render::assets::art::sys_stage_sprite_art_page_uploads,
               crate::render::assets::art::sys_stage_ground_art_page_uploads,
               static_lights::sys_collect_visible_static_lights
                   .after(SceneRenderLandSystems::RenderLandChunks)
                   .after(client_light_sprites::sys_collect_client_light_sprites),
               statics_collect::sys_collect_visible_statics
                   .in_set(SceneRenderArtSystems::CollectVisibleStatics)
                   .after(static_lights::sys_collect_visible_static_lights),
               statics_draw::sys_sync_static_sprite_entities
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_sync_static_sprite_shadow_entities
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_sync_static_sprite_transparent_entities
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_sync_static_ground_entities
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_sync_static_ground_transparent_entities
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               static_lights::sys_sync_static_light_entities
                   .after(static_lights::sys_collect_visible_static_lights),
               statics_draw::sys_update_sprite_instance_buffer
                   .in_set(SceneRenderArtSystems::RenderStaticSprites)
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_update_ground_instance_buffer
                   .in_set(SceneRenderArtSystems::RenderStaticSprites)
                   .after(SceneRenderArtSystems::CollectVisibleStatics),
               statics_draw::sys_update_art_materials,
           ).chain().run_if(in_state(AppState::InGame)));
        app.add_systems(Update, (
               client_light_sprites::sys_collect_client_light_sprites
                   .after(SceneRenderLandSystems::RenderLandChunks),
               client_light_sprites::sys_sync_client_light_sprites,
           ).chain().run_if(in_state(AppState::InGame)));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.init_resource::<crate::render::assets::art::RenderSpriteArtPageUploads>();
        render_app.init_resource::<crate::render::assets::art::RenderGroundArtPageUploads>();
        render_app.add_systems(
            ExtractSchedule,
            (
                crate::render::assets::art::sys_extract_sprite_art_page_uploads,
                crate::render::assets::art::sys_extract_ground_art_page_uploads,
            ),
        );
        render_app.add_systems(
            bevy::render::Render,
            transparent_order::stabilize_equal_depth_batches
                .in_set(bevy::render::RenderSystems::PhaseSort)
                .after(bevy::render::render_phase::sort_phase_system::<
                    bevy::core_pipeline::core_3d::Transparent3d,
                >),
        );
        render_app.add_systems(
            bevy::render::Render,
            (
                crate::render::assets::art::sys_render_upload_sprite_art_pages,
                crate::render::assets::art::sys_render_upload_ground_art_pages,
            )
                .in_set(bevy::render::RenderSystems::Queue),
        );
    }
}
