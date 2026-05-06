use bevy::prelude::*;
use crate::console_logger::{self, LogAbout, LogSev};
use crate::configs::settings::ClientTextureSource;
use crate::configs::settings::Settings;
use crate::core::statics::StaticsStoreRes;
use crate::core::uo_files_loader::{
    CcArtPackageRes, EcArtPackageRes, EcLandPackageRes, TileMetaPackageRes,
};
use crate::core::texture_cache::art::ArtPageAtlas;
use crate::core::render::scene::world::WorldGeoData;
use crate::core::render::scene::world::land::{CHUNK_STORAGE_BLOCKS_DIM, MAP_STORAGE_BLOCK_TILE_DIM};
use crate::core::render::scene::SceneStateData;
use crate::core::render::scene::camera::RenderZoom;
use bytemuck::{Pod, Zeroable};
use bevy::render::render_resource::ShaderType;
use std::collections::HashSet;

const CLASSIC_STATIC_ART_ID_OFFSET: u16 = 0x4000;
const CC_WORLD_XZ_PER_PIXEL: f32 = 1.41421356237 / 44.0;
const CC_WORLD_Y_PER_PIXEL: f32 = (7.5 * 0.1) * CC_WORLD_XZ_PER_PIXEL;
const STATIC_ART_Y_BIAS: f32 = 0.002;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticVisualKind {
    CcRegularArt { art_id: u16 },
    EcRegularArt { art_id: u32 },
    EcLandArt { art_id: u32 },
}

fn resolve_static_visual_kind(
    art_source: ClientTextureSource,
    tile_graphic: u16,
    tilemeta: Option<&uddconv::tilemeta::TileMetaItemTile>,
    ec_art: Option<&uddconv::ec_art::EcArtPackage>,
    ec_land: Option<&uddconv::ec_land::EcLandPackage>,
) -> StaticVisualKind {
    match art_source {
        ClientTextureSource::Cc => {
            let cc_texture_id = tilemeta
                .map(|meta| meta.cc_texture_id as u16)
                .unwrap_or(tile_graphic);
            StaticVisualKind::CcRegularArt {
                art_id: cc_texture_id.saturating_add(CLASSIC_STATIC_ART_ID_OFFSET),
            }
        }
        ClientTextureSource::Ec => {
            if ec_art.is_some_and(|package| package.present_slot(tile_graphic as u32).is_some()) {
                StaticVisualKind::EcRegularArt {
                    art_id: tile_graphic as u32,
                }
            } else if tilemeta.is_some_and(|meta| {
                ec_land.is_some_and(|package| {
                    package.resolve_runtime_slot_id(meta.cc_texture_id)
                        .is_some()
                })
            }) {
                StaticVisualKind::EcLandArt {
                    art_id: tile_graphic as u32,
                }
            } else {
                StaticVisualKind::EcRegularArt {
                    art_id: tile_graphic as u32,
                }
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
pub struct SpriteInstance {
    pub world_x: f32,       // tile_x + x_offset
    pub world_z: f32,       // tile_y + y_offset
    pub world_y: f32,       // z * height_scale (isometric altitude)
    pub layer: u32,         // atlas page layer
    pub uv_min: [f32; 2],   // normalized UV
    pub uv_max: [f32; 2],   // normalized UV
    pub local_min: [f32; 2], // local quad bounds from tile origin
    pub local_max: [f32; 2], // local quad bounds from tile origin
    pub color_rgba: [f32; 4], // for dot mode
}

#[derive(Resource, Default)]
pub struct RenderStaticInstances(pub Vec<SpriteInstance>);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StaticArtCollectStats {
    pub map_id: u32,
    pub dot_mode: bool,
    pub visible_chunks: usize,
    pub visited_blocks: usize,
    pub source_tiles: usize,
    pub unique_requested_pages: usize,
    pub resident_pages: usize,
    pub pending_pages: usize,
    pub atlas_capacity_pages: usize,
    pub atlas_hits: usize,
    pub atlas_misses: usize,
    pub emitted_instances: usize,
}

#[derive(Resource, Default)]
pub struct StaticArtCollectDebugState {
    pub last: Option<StaticArtCollectStats>,
}

#[derive(Resource, Default)]
pub struct StaticArtSourceState {
    pub active_source: Option<ClientTextureSource>,
    pub warned_unavailable_source: Option<ClientTextureSource>,
}

fn source_available(
    source: ClientTextureSource,
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
    ec_land_res: Option<&Res<EcLandPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> bool {
    match source {
        ClientTextureSource::Cc => cc_art_res.is_some(),
        ClientTextureSource::Ec => {
            ec_art_res.is_some() && ec_land_res.is_some() && tilemeta_res.is_some()
        }
    }
}

fn resolve_effective_art_source(
    requested_source: ClientTextureSource,
    cc_art_res: Option<&Res<CcArtPackageRes>>,
    ec_art_res: Option<&Res<EcArtPackageRes>>,
    ec_land_res: Option<&Res<EcLandPackageRes>>,
    tilemeta_res: Option<&Res<TileMetaPackageRes>>,
) -> Option<ClientTextureSource> {
    if source_available(
        requested_source,
        cc_art_res,
        ec_art_res,
        ec_land_res,
        tilemeta_res,
    ) {
        return Some(requested_source);
    }

    ClientTextureSource::ALL.into_iter().find(|candidate| {
        *candidate != requested_source
            && source_available(*candidate, cc_art_res, ec_art_res, ec_land_res, tilemeta_res)
    })
}

pub fn sys_sync_static_art_source(
    mut art_atlas: ResMut<ArtPageAtlas>,
    settings: Res<Settings>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut source_state: ResMut<StaticArtSourceState>,
) {
    let requested_source = settings.graphics.art_texture_source;
    let effective_source = resolve_effective_art_source(
        requested_source,
        cc_art_res.as_ref(),
        ec_art_res.as_ref(),
        ec_land_res.as_ref(),
        tilemeta_res.as_ref(),
    );

    if effective_source != Some(requested_source)
        && source_state.warned_unavailable_source != Some(requested_source)
    {
        source_state.warned_unavailable_source = Some(requested_source);
        let fallback_label = effective_source
            .map(|source| source.art_label())
            .unwrap_or("none");
        console_logger::one(
            LogSev::Warn,
            LogAbout::RenderWorldArt,
            &format!(
                "Requested {} for static sprites, but the required package/metadata is unavailable. Falling back to {}.",
                requested_source.art_label(),
                fallback_label,
            ),
        );
    } else if effective_source == Some(requested_source) {
        source_state.warned_unavailable_source = None;
    }

    if source_state.active_source == effective_source {
        return;
    }

    art_atlas.clear();
    source_state.active_source = effective_source;

    match effective_source {
        Some(source) if source == requested_source => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!("Switched static sprite source to {}.", source.art_label()),
            );
        }
        Some(source) => {
            console_logger::one(
                LogSev::Info,
                LogAbout::RenderWorldArt,
                &format!(
                    "Switched static sprite source to {} (requested {}).",
                    source.art_label(),
                    requested_source.art_label(),
                ),
            );
        }
        None => {
            console_logger::one(
                LogSev::Warn,
                LogAbout::RenderWorldArt,
                "No static sprite art package is available; non-dot statics will be skipped.",
            );
        }
    }
}

pub fn sys_collect_visible_statics(
    statics_res: Res<StaticsStoreRes>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    ec_art_res: Option<Res<EcArtPackageRes>>,
    ec_land_res: Option<Res<EcLandPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut art_atlas: ResMut<ArtPageAtlas>,
    settings: Res<crate::configs::settings::Settings>,
    scene_state: Res<SceneStateData>,
    zoom: Res<RenderZoom>,
    _world_geo: Res<WorldGeoData>,
    mut instances: ResMut<RenderStaticInstances>,
    mut debug_state: ResMut<StaticArtCollectDebugState>,
    source_state: Res<StaticArtSourceState>,
    // TODO: Need a way to get the currently visible chunks from the terrain system.
    // We can iterate over existing land chunk entities to find which blocks to draw.
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    instances.0.clear();

    if !settings.worldmap_rendering.enable_statics {
        return;
    }

    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    let mut statics_store = statics_store.lock();

    let requested_art_source = settings.graphics.art_texture_source;
    let art_source = resolve_effective_art_source(
        requested_art_source,
        cc_art_res.as_ref(),
        ec_art_res.as_ref(),
        ec_land_res.as_ref(),
        tilemeta_res.as_ref(),
    )
    .or(source_state.active_source);

    let is_dot_mode = zoom.0 >= 20.0;
    let mut visible_chunks = 0usize;
    let mut visited_blocks = 0usize;
    let mut source_tiles = 0usize;
    let mut atlas_hits = 0usize;
    let mut atlas_misses = 0usize;
    let mut unique_requested_pages = HashSet::new();

    // Height conversion factor from UO units to our world Y units.
    // Usually z is roughly 1 unit = 0.1 world units (or similar).
    // The land shader does: world.y = z * 0.1
    let height_scale = 0.1;

    for tcm in chunks_q.iter() {
        if tcm.parent_map_id != map_id {
            continue;
        }

        visible_chunks += 1;

        let chunk_scale = tcm.scale;

        let start_gx = tcm.gx * CHUNK_STORAGE_BLOCKS_DIM;
        let start_gy = tcm.gy * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gx = start_gx + chunk_scale * CHUNK_STORAGE_BLOCKS_DIM;
        let end_gy = start_gy + chunk_scale * CHUNK_STORAGE_BLOCKS_DIM;

        for gy in start_gy..end_gy {
            for gx in start_gx..end_gx {
                visited_blocks += 1;
                let Ok(tiles) = statics_store.block_tiles(gx, gy) else {
                    continue;
                };

                source_tiles += tiles.len();

                for tile in tiles {
                    // Very simple culling for dot mode: only render tall or wide things to save instances
                    if is_dot_mode && tile.z < 10 {
                        continue;
                    }

                    let world_x = (gx * MAP_STORAGE_BLOCK_TILE_DIM) as f32 + tile.x_offset() as f32;
                    let world_z = (gy * MAP_STORAGE_BLOCK_TILE_DIM) as f32 + tile.y_offset() as f32;
                    let world_y = (tile.z as f32) * height_scale + STATIC_ART_Y_BIAS;

                    if is_dot_mode {
                        if let Some(tilemeta) = tilemeta_res.as_ref() {
                            let color = tilemeta.0.item_tile(tile.graphic as u32).map(|t| t.radar_color).unwrap_or([0, 0, 0, 0]);
                            instances.0.push(SpriteInstance {
                                world_x,
                                world_z,
                                world_y,
                                layer: 0,
                                uv_min: [0.0, 0.0],
                                uv_max: [0.0, 0.0],
                                local_min: [0.0, 0.0],
                                local_max: [1.0, 1.0],
                                color_rgba: [color[2] as f32 / 255.0, color[1] as f32 / 255.0, color[0] as f32 / 255.0, 1.0],
                            });
                        }
                    } else {
                        let Some(art_source) = art_source else {
                            continue;
                        };
                        let tilemeta = tilemeta_res
                            .as_ref()
                            .and_then(|meta| meta.0.item_tile(tile.graphic as u32));
                        let visual_kind = resolve_static_visual_kind(
                            art_source,
                            tile.graphic,
                            tilemeta,
                            ec_art_res.as_ref().map(|package| &*package.0),
                            ec_land_res.as_ref().map(|package| &*package.0),
                        );
                        let (offset_x_world, offset_y_world, resolved_sprite) = match visual_kind {
                            StaticVisualKind::CcRegularArt { art_id } => {
                                let Some(cc_art) = cc_art_res.as_ref().map(|x| &x.0) else {
                                    continue;
                                };
                                let offset_x_world = tilemeta
                                    .map(|meta| meta.cc_offset_x as f32 * CC_WORLD_XZ_PER_PIXEL)
                                    .unwrap_or(0.0);
                                let offset_y_world = tilemeta
                                    .map(|meta| meta.cc_offset_y as f32 * CC_WORLD_Y_PER_PIXEL)
                                    .unwrap_or(0.0);

                                if let Some(slot) = cc_art.present_slot(art_id as u32) {
                                    unique_requested_pages.insert(
                                        ArtPageAtlas::cache_key_for_page(art_source, slot.page_index),
                                    );
                                }

                                (offset_x_world, offset_y_world, art_atlas.resolve_cc(cc_art, art_id))
                            }
                            StaticVisualKind::EcLandArt { .. } => {
                                let Some(ec_land) = ec_land_res.as_ref().map(|x| &x.0) else {
                                    continue;
                                };
                                let Some(runtime_slot_id) = tilemeta.and_then(|meta| {
                                    ec_land.resolve_runtime_slot_id(meta.cc_texture_id)
                                }) else {
                                    continue;
                                };
                                let offset_x_world = tilemeta
                                    .map(|meta| meta.ec_offset_x as f32 * CC_WORLD_XZ_PER_PIXEL)
                                    .unwrap_or(0.0);
                                let offset_y_world = tilemeta
                                    .map(|meta| meta.ec_offset_y as f32 * CC_WORLD_Y_PER_PIXEL)
                                    .unwrap_or(0.0);

                                if let Some(slot) = ec_land.present_slot(runtime_slot_id) {
                                    unique_requested_pages.insert(
                                        ArtPageAtlas::cache_key_for_ec_land_page(slot.page_index),
                                    );
                                }

                                (
                                    offset_x_world,
                                    offset_y_world,
                                    art_atlas.resolve_ec_land(ec_land, runtime_slot_id),
                                )
                            }
                            StaticVisualKind::EcRegularArt { art_id } => {
                                let Some(ec_art) = ec_art_res.as_ref().map(|x| &x.0) else {
                                    continue;
                                };
                                let offset_x_world = tilemeta
                                    .map(|meta| meta.ec_offset_x as f32 * CC_WORLD_XZ_PER_PIXEL)
                                    .unwrap_or(0.0);
                                let offset_y_world = tilemeta
                                    .map(|meta| meta.ec_offset_y as f32 * CC_WORLD_Y_PER_PIXEL)
                                    .unwrap_or(0.0);

                                if let Some(slot) = ec_art.present_slot(art_id) {
                                    unique_requested_pages.insert(
                                        ArtPageAtlas::cache_key_for_page(art_source, slot.page_index),
                                    );
                                }

                                (offset_x_world, offset_y_world, art_atlas.resolve_ec(ec_art, art_id))
                            }
                        };

                        if let Some(resolved) = resolved_sprite {
                            atlas_hits += 1;
                            let world_w = resolved.pixel_width as f32 * CC_WORLD_XZ_PER_PIXEL;
                            let world_h = resolved.pixel_height as f32 * CC_WORLD_Y_PER_PIXEL;
                            let local_min_x = offset_x_world;
                            let local_max_x = offset_x_world + world_w;
                            let local_min_y = -offset_y_world;
                            let local_max_y = local_min_y + world_h;

                            instances.0.push(SpriteInstance {
                                world_x,
                                world_z,
                                world_y,
                                layer: resolved.layer,
                                uv_min: [resolved.uv_min.x, resolved.uv_min.y],
                                uv_max: [resolved.uv_max.x, resolved.uv_max.y],
                                local_min: [local_min_x, local_min_y],
                                local_max: [local_max_x, local_max_y],
                                color_rgba: [1.0, 1.0, 1.0, 1.0],
                            });
                        } else {
                            atlas_misses += 1;
                        }
                    }
                }
            }
        }
    }

    // Sort instances by painter's algorithm order: Y_tile + Z_tile
    instances.0.sort_unstable_by(|a, b| {
        let depth_a = a.world_z + a.world_y / height_scale;
        let depth_b = b.world_z + b.world_y / height_scale;
        depth_a.partial_cmp(&depth_b).unwrap_or(std::cmp::Ordering::Equal)
    });

    let stats = StaticArtCollectStats {
        map_id,
        dot_mode: is_dot_mode,
        visible_chunks,
        visited_blocks,
        source_tiles,
        unique_requested_pages: unique_requested_pages.len(),
        resident_pages: art_atlas.resident_page_count(),
        pending_pages: art_atlas.pending_page_count(),
        atlas_capacity_pages: art_atlas.max_layers as usize,
        atlas_hits,
        atlas_misses,
        emitted_instances: instances.0.len(),
    };

    if debug_state.last != Some(stats) {
        console_logger::one(
            LogSev::Info,
            LogAbout::RenderWorldArt,
            &format!(
                "static art collect: map={} dot_mode={} chunks={} blocks={} tiles={} unique_pages={} resident_pages={} pending_pages={} capacity={} atlas_hits={} atlas_misses={} emitted={}",
                stats.map_id,
                stats.dot_mode,
                stats.visible_chunks,
                stats.visited_blocks,
                stats.source_tiles,
                stats.unique_requested_pages,
                stats.resident_pages,
                stats.pending_pages,
                stats.atlas_capacity_pages,
                stats.atlas_hits,
                stats.atlas_misses,
                stats.emitted_instances,
            ),
        );
        debug_state.last = Some(stats);
    }
}
