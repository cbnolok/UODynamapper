use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::ExtractSchedule;
use crate::core::system_sets::{StartupSysSet, SceneRenderLandSysSet, SceneRenderArtSysSet};
use crate::core::app_states::AppState;
use crate::prelude::*;
use crate::util_lib::tracked_plugin::TrackedPlugin;

pub mod statics_collect;
pub mod statics_draw;

pub struct DrawStaticSpritesPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(DrawStaticSpritesPlugin);

impl Plugin for DrawStaticSpritesPlugin {
    fn build(&self, app: &mut App) {
        log_plugin_build(self);
        app.init_resource::<statics_collect::RenderStaticInstances>();
        app.init_resource::<statics_collect::RenderStaticLandInstances>();
        app.init_resource::<statics_collect::StaticArtCollectDebugState>();
        app.init_resource::<statics_collect::StaticArtSourceState>();
        app.init_resource::<statics_draw::StaticArtDrawDebugState>();
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            crate::core::texture_cache::art::ArtPageAtlasHandle,
        >::default());

        app.add_plugins((
               MaterialPlugin::<statics_draw::ArtSpriteMaterial>::default(),
               MaterialPlugin::<statics_draw::ArtGroundMaterial>::default(),
           ))
           .add_systems(Startup, statics_draw::sys_setup_art_page_atlas
               .in_set(StartupSysSet::SetupSceneStage1)
               .after(StartupSysSet::LoadStartupUOFiles))
           .add_systems(Update, (
               statics_collect::sys_sync_static_art_source,
               crate::core::texture_cache::art::sys_stage_art_page_uploads,
               statics_collect::sys_collect_visible_statics
                   .in_set(SceneRenderArtSysSet::CollectVisibleStatics)
                   .after(SceneRenderLandSysSet::RenderLandChunks),
               statics_draw::sys_sync_static_sprite_entities
                   .after(SceneRenderArtSysSet::CollectVisibleStatics),
               statics_draw::sys_sync_static_ground_entities
                   .after(SceneRenderArtSysSet::CollectVisibleStatics),
               statics_draw::sys_update_sprite_instance_buffer
                   .in_set(SceneRenderArtSysSet::RenderStaticSprites)
                   .after(SceneRenderArtSysSet::CollectVisibleStatics),
               statics_draw::sys_update_ground_instance_buffer
                   .in_set(SceneRenderArtSysSet::RenderStaticSprites)
                   .after(SceneRenderArtSysSet::CollectVisibleStatics),
           ).chain().run_if(in_state(AppState::InGame)));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.init_resource::<crate::core::texture_cache::art::RenderArtPageUploads>();
        render_app.add_systems(ExtractSchedule, crate::core::texture_cache::art::sys_extract_art_page_uploads);
        render_app.add_systems(
            bevy::render::Render,
            crate::core::texture_cache::art::sys_render_upload_art_pages
                .in_set(bevy::render::RenderSystems::Queue),
        );
    }
}
