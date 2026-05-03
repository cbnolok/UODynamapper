use bevy::prelude::*;
use crate::core::uo_files_loader::{StaticsStoreRes, CcArtPackageRes, TileMetaPackageRes};
use crate::core::texture_cache::art::ArtPageAtlas;
use crate::core::render::scene::world::WorldGeoData;
use crate::core::render::scene::world::land::TILE_NUM_PER_CHUNK_DIM;
use crate::core::render::scene::SceneStateData;
use crate::core::render::scene::camera::RenderZoom;
use bytemuck::{Pod, Zeroable};
use bevy::render::render_resource::ShaderType;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
pub struct SpriteInstance {
    pub world_x: f32,       // tile_x + x_offset
    pub world_z: f32,       // tile_y + y_offset
    pub world_y: f32,       // z * height_scale (isometric altitude)
    pub layer: u32,         // atlas page layer
    pub uv_min: [f32; 2],   // normalized UV
    pub uv_max: [f32; 2],   // normalized UV
    pub pixel_size: [f32; 2], // width, height in world units (for quad sizing)
    pub _pad: [f32; 2],     // Pad to 16-byte alignment for color_rgba
    pub color_rgba: [f32; 4], // for dot mode
}

#[derive(Resource, Default)]
pub struct RenderStaticInstances(pub Vec<SpriteInstance>);

pub fn sys_collect_visible_statics(
    statics_res: Res<StaticsStoreRes>,
    cc_art_res: Option<Res<CcArtPackageRes>>,
    tilemeta_res: Option<Res<TileMetaPackageRes>>,
    mut art_atlas: ResMut<ArtPageAtlas>,
    scene_state: Res<SceneStateData>,
    zoom: Res<RenderZoom>,
    world_geo: Res<WorldGeoData>,
    mut instances: ResMut<RenderStaticInstances>,
    // TODO: Need a way to get the currently visible chunks from the terrain system.
    // We can iterate over existing land chunk entities to find which blocks to draw.
    chunks_q: Query<&crate::core::render::scene::world::land::LCMesh>,
) {
    instances.0.clear();
    
    let map_id = scene_state.map_id;
    let Some(statics_store) = statics_res.0.get(map_id as usize).and_then(|x| x.as_ref()) else {
        return;
    };
    
    let Some(cc_art) = cc_art_res.as_ref().map(|x| &x.0) else {
        return;
    };
    
    let is_dot_mode = zoom.0 >= 20.0;
    
    // Height conversion factor from UO units to our world Y units.
    // Usually z is roughly 1 unit = 0.1 world units (or similar).
    // The land shader does: world.y = z * 0.1
    let height_scale = 0.1;
    
    for tcm in chunks_q.iter() {
        if tcm.parent_map_id != map_id {
            continue;
        }
        
        let chunk_scale = tcm.scale;
        
        let start_gx = tcm.gx;
        let start_gy = tcm.gy;
        let end_gx = start_gx + chunk_scale;
        let end_gy = start_gy + chunk_scale;
        
        for gy in start_gy..end_gy {
            for gx in start_gx..end_gx {
                let tiles = statics_store.block_tiles(gx, gy);
                
                for tile in tiles {
                    // Very simple culling for dot mode: only render tall or wide things to save instances
                    if is_dot_mode && tile.z < 10 {
                        continue;
                    }
                
                    let world_x = (gx * TILE_NUM_PER_CHUNK_DIM) as f32 + tile.x_offset() as f32;
                    let world_z = (gy * TILE_NUM_PER_CHUNK_DIM) as f32 + tile.y_offset() as f32;
                    let world_y = (tile.z as f32) * height_scale;
                    
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
                                pixel_size: [1.0, 1.0], // 1x1 tile size
                                _pad: [0.0, 0.0],
                                color_rgba: [color[2] as f32 / 255.0, color[1] as f32 / 255.0, color[0] as f32 / 255.0, 1.0],
                            });
                        }
                    } else {
                        // Request texture from atlas
                        if let Some(resolved) = art_atlas.resolve(cc_art, tile.graphic) {
                            // Convert pixel size to world size.
                            // 1 tile is ~44 pixels wide in CC.
                            let world_w = resolved.pixel_width as f32 / 44.0;
                            let world_h = resolved.pixel_height as f32 / 44.0;
                            
                            instances.0.push(SpriteInstance {
                                world_x,
                                world_z,
                                world_y,
                                layer: resolved.layer,
                                uv_min: [resolved.uv_min.x, resolved.uv_min.y],
                                uv_max: [resolved.uv_max.x, resolved.uv_max.y],
                                pixel_size: [world_w, world_h],
                                _pad: [0.0, 0.0],
                                color_rgba: [1.0, 1.0, 1.0, 1.0],
                            });
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
}
