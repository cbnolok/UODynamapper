//! When a frame is ready to capture: its chunks built, every land and art upload done, no render pipeline compiling and
//! the art collection settled on them.

use crate::render::assets::art::{GroundArtPageAtlas, SpriteArtPageAtlas};
use crate::render::assets::tile_atlas::{LandUploadTelemetry, TileAtlas};
use crate::render::pipeline_readiness::WaitingRenderPipelines;
use crate::render::statics::statics_collect::StaticArtCollectDebugState;
use crate::render::terrain::{LCMesh, draw_mesh::ChunkScale};
use crate::render::view::footprint::chunks_in_tile_rect;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::HashSet;
use super::ExportScene;
use super::job::ExportJob;
use super::placement::FramePlacement;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ReadySignature {
    visible_chunks: usize,
    emitted_instances: usize,
    /// Statics whose art the package does not have; they are drawn as nothing, as in the interactive view.
    pub(super) absent_art: usize,
    land_revision: u64,
}

#[derive(SystemParam)]
pub(super) struct ExportReadiness<'w, 's> {
    chunks: Query<'w, 's, (&'static LCMesh, Has<Mesh3d>)>,
    chunk_scale: Res<'w, ChunkScale>,
    tile_atlas: Option<Res<'w, TileAtlas>>,
    land_uploads: Option<Res<'w, LandUploadTelemetry>>,
    art_stats: Option<Res<'w, StaticArtCollectDebugState>>,
    sprite_atlas: Option<Res<'w, SpriteArtPageAtlas>>,
    ground_atlas: Option<Res<'w, GroundArtPageAtlas>>,
    waiting_pipelines: Res<'w, WaitingRenderPipelines>,
}

/// What must hold before a capture: the frame's chunks built, every land and art upload done and the art
/// collection settled on them. While anything is pending, the error says what.
pub(super) fn ready_signature(
    job: &ExportJob,
    placement: &FramePlacement,
    scene: &ExportScene,
    readiness: &ExportReadiness,
) -> Result<ReadySignature, String> {
    let waiting_pipelines = readiness.waiting_pipelines.count();
    if waiting_pipelines != 0 {
        return Err(format!("{waiting_pipelines} render pipelines to compile"));
    }
    let (player, _) = scene.player.single().map_err(|error| format!("the player: {error}"))?;
    if player.current_pos.is_none() || player.current_pos != player.prev_rendered_pos {
        return Err("the player to reach the anchor tile".to_owned());
    }
    let map = scene.world.maps.get(&job.facet).ok_or_else(|| format!("map {} to load", job.facet))?;
    let scale = readiness.chunk_scale.0;
    let built: HashSet<(u32, u32)> = readiness
        .chunks
        .iter()
        .filter(|(chunk, has_mesh)| *has_mesh && chunk.parent_map_id == job.facet && chunk.scale == scale)
        .map(|(chunk, _)| (chunk.gx, chunk.gy))
        .collect();
    let required = chunks_in_tile_rect(&placement.footprint, map.width, map.height, scale);
    let missing = required.iter().filter(|chunk| !built.contains(chunk)).count();
    if missing != 0 {
        return Err(format!("{missing} of {} land chunks to be built", required.len()));
    }
    let land_revision = match (&readiness.tile_atlas, &readiness.land_uploads) {
        (Some(atlas), Some(uploads)) => {
            let uploads = uploads.snapshot();
            if atlas.has_pending_uploads()
                || uploads.pending_ops != 0
                || uploads.submitted_metadata_revision != atlas.content_revision
            {
                return Err(format!(
                    "land uploads (atlas pending {}, upload ops {}, metadata revision {} of {})",
                    atlas.has_pending_uploads(),
                    uploads.pending_ops,
                    uploads.submitted_metadata_revision,
                    atlas.content_revision
                ));
            }
            atlas.content_revision
        }
        _ => 0,
    };
    let stats = readiness
        .art_stats
        .as_ref()
        .and_then(|art_stats| art_stats.last)
        .ok_or_else(|| "the first static art collection".to_owned())?;
    let pending_pages = readiness.sprite_atlas.as_ref().map_or(0, |atlas| atlas.0.pending_page_count())
        + readiness.ground_atlas.as_ref().map_or(0, |atlas| atlas.0.pending_page_count());
    // A miss with every requested page resident and none pending is art the package lacks, which stays
    // missing; the stable-signature rule makes sure the count has settled.
    if stats.map_id != job.facet
        || pending_pages != 0
        || stats.unique_requested_pages > stats.atlas_capacity_pages
        || stats.visible_chunks < required.len()
    {
        return Err(format!(
            "static art (map {}, atlas misses {} of {}, pending pages {pending_pages}, resident pages {} of {} requested, \
             capacity {}, collected chunks {} of {})",
            stats.map_id,
            stats.atlas_misses,
            stats.atlas_hits + stats.atlas_misses,
            stats.resident_pages,
            stats.unique_requested_pages,
            stats.atlas_capacity_pages,
            stats.visible_chunks,
            required.len()
        ));
    }
    Ok(ReadySignature {
        visible_chunks: stats.visible_chunks,
        emitted_instances: stats.emitted_instances,
        absent_art: stats.atlas_misses,
        land_revision,
    })
}
