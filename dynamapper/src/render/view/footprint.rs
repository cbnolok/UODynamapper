//! What the view covers: the chunks the follow view streams around the player, the free camera's footprint on the
//! ground, and the export frame's (`ExportChunkFootprint`), which replace it.
use super::camera::{self, PlayerCamera};
use crate::render::terrain::TILE_NUM_PER_CHUNK_DIM;
use crate::world::coords::scale_uo_z_to_bevy_units;
use bevy::prelude::*;

/// Export mode only: the tiles whose chunks must be drawn, half-open. When present, it replaces the
/// window-derived chunk set, so the frame rectangle, not the window and player, decides what streams in.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportChunkFootprint {
    pub tile_x0: u32,
    pub tile_y0: u32,
    pub tile_x1: u32,
    pub tile_y1: u32,
}

/// Free camera only: the tiles the camera sees on the ground (`camera_ground_footprint`), on the chunk grid of the
/// current scale. Like `ExportChunkFootprint` it replaces the player-derived chunk set; the camera updates it, and asks
/// for a recompute, only when the view covers other chunks.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeCameraFootprint(pub ExportChunkFootprint);

/// How far the free camera streams: its footprint ends this many orthographic half views from the ground point at the
/// centre of its view, so a view towards the horizon does not stream the map up to it.
pub(super) const FREE_CAMERA_REACH_HALF_VIEWS: f32 = 3.0;

/// Half the side of the ground square the fixed isometric orthographic view covers, in tiles.
pub(super) fn orthographic_ground_half_extent(zoom: f32, window_width: f32, window_height: f32) -> f32 {
    // Compute orthographic half-extents in world/tile units, matching
    // sys_update_camera_projection_to_view exactly.
    let ortho_width = window_width / camera::ORTHO_SIZE_FACTOR;
    let ortho_height =
        (window_height / camera::ORTHO_WIDTH_SCALE_FACTOR) / camera::ORTHO_SIZE_FACTOR;
    let hw = ortho_width * zoom / 2.0;
    let hh = ortho_height * zoom / 2.0;

    // Camera basis vectors for the fixed isometric angle
    // (Transform::from_translation(player + (5,5,5)).looking_at(player, Vec3::Y)):
    //   right = (-1/√2,  0,     1/√2)      → only XZ component
    //   up    = ( 1/√6, -2/√6,  1/√6)
    //   fwd   = (-1/√3, -1/√3, -1/√3)
    //
    // For an ortho frustum corner at camera-local (u, v):
    //   P = cam_pos + u*right + v*up
    //   Ray along fwd hits Y=0 at:  hit = P - P.y * (1, 1, 1)
    //     ⟹ hit.x = P.x - P.y = (cam.x - cam.y) + u*(-1/√2) + v*(3/√6)
    //     ⟹ hit.z = P.z - P.y = (cam.z - cam.y) + u*( 1/√2) + v*(3/√6)
    //
    // AABB half-extent = hw/√2 + hh * 3/√6
    let inv_sqrt2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let three_over_sqrt6: f32 = 3.0 / 6.0_f32.sqrt();
    hw * inv_sqrt2 + hh * three_over_sqrt6
}

/// The tiles a chunk set reaches past its footprint, so its edges never show a gap.
pub(super) fn chunk_edge_pad_tiles(chunk_scale: u32) -> f32 {
    // Safety pad: at least 2 base chunks, with a small extra cushion at
    // wider scales to avoid edge gaps.
    let scaled_pad = if chunk_scale <= 2 {
        2
    } else {
        chunk_scale + (chunk_scale / 4).max(1)
    };
    (TILE_NUM_PER_CHUNK_DIM * scaled_pad) as f32
}

/// The tiles the camera sees on the ground, whatever its pose and projection: the box around where its view's corners
/// and centre meet the lowest and the highest land, padded like the follow view (`chunk_edge_pad_tiles`) and rounded
/// out to the chunk grid of `chunk_scale`, so it changes only when the view covers other chunks. Computed from this
/// frame's projection and transform, not from the camera's matrices, which are a frame late in `Update`.
pub(crate) fn camera_ground_footprint(
    projection: &Projection,
    camera_transform: &Transform,
    viewport: Vec2,
    zoom: f32,
    chunk_scale: u32,
) -> ExportChunkFootprint {
    /// Normalized device depths of two points on each view ray: Bevy's reverse-z puts the near plane at 1, and 0.5 is
    /// finite for both projections (twice the near distance in an infinite perspective).
    const NEAR_DEPTH: f32 = 1.0;
    const FAR_DEPTH: f32 = 0.5;
    const VIEW_POINTS: [Vec2; 5] =
        [Vec2::ZERO, Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(-1.0, 1.0), Vec2::new(1.0, 1.0)];
    let mut projection = projection.clone();
    projection.update(viewport.x, viewport.y);
    let world_from_clip = camera_transform.to_matrix() * projection.get_clip_from_view().inverse();
    let ground_levels = [i8::MIN, i8::MAX].map(|z| scale_uo_z_to_bevy_units(f32::from(z)));
    let reach = FREE_CAMERA_REACH_HALF_VIEWS * orthographic_ground_half_extent(zoom, viewport.x, viewport.y);
    // A view ray: its point on the near plane (behind the orthographic camera) and its direction.
    let view_ray = |ndc: Vec2| {
        let near = world_from_clip.project_point3(ndc.extend(NEAR_DEPTH));
        (near, (world_from_clip.project_point3(ndc.extend(FAR_DEPTH)) - near).normalize_or_zero())
    };
    // Where a view ray meets a ground level, if it goes down to it.
    let ground_hit = |(near, direction): (Vec3, Vec3), level: f32| {
        (direction.y < -f32::EPSILON).then(|| (near + direction * ((level - near.y) / direction.y).max(0.0)).xz())
    };
    let centre = ground_hit(view_ray(Vec2::ZERO), ground_levels[0]).unwrap_or(camera_transform.translation.xz());
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for ndc in VIEW_POINTS {
        let ray = view_ray(ndc);
        for level in ground_levels {
            // A ray above the ground reaches as far as the reach, towards where it looks; every point stays within
            // the reach around the view's centre on the ground.
            let point = ground_hit(ray, level).unwrap_or(centre + ray.1.xz().normalize_or_zero() * reach);
            let point = centre + (point - centre).clamp_length_max(reach);
            min = min.min(point);
            max = max.max(point);
        }
    }
    let pad = chunk_edge_pad_tiles(chunk_scale);
    let span = (TILE_NUM_PER_CHUNK_DIM * chunk_scale) as f32;
    let to_grid = |tiles: f32, round: fn(f32) -> f32| (round(tiles / span) * span).max(0.0) as u32;
    ExportChunkFootprint {
        tile_x0: to_grid(min.x - pad, f32::floor),
        tile_y0: to_grid(min.y - pad, f32::floor),
        tile_x1: to_grid(max.x + pad, f32::ceil),
        tile_y1: to_grid(max.y + pad, f32::ceil),
    }
}

/// Calculates the visible chunk set from current-frame parameters only.
///
/// Computes the orthographic camera footprint on the Y=0 ground plane
/// using the known isometric camera geometry (fixed offset `(5,5,5)`),
/// current zoom level, and window dimensions.  This avoids using
/// `Camera::viewport_to_world()` which relies on Bevy's internal
/// view-projection matrices that are 1 frame stale during `Update`.
///
/// When `chunk_scale > 1`, the grid is iterated at a coarser granularity.
/// Returned coordinates are in the base logical 32×32 grid, aligned to `chunk_scale` boundaries.
pub(crate) fn compute_visible_chunks(
    player_translation: Vec3,
    zoom: f32,
    window_width: f32,
    window_height: f32,
    map_width: u32,
    map_height: u32,
    chunk_scale: u32,
) -> Vec<(u32, u32)> {
    let base_chunk_size = TILE_NUM_PER_CHUNK_DIM;
    let scaled_tile_span = base_chunk_size * chunk_scale;
    let map_base_chunks_x = (map_width / base_chunk_size) as i32;
    let map_base_chunks_y = (map_height / base_chunk_size) as i32;

    let cam_pos = player_translation + PlayerCamera::BASE_OFFSET_FROM_PLAYER;
    let center_x = cam_pos.x - cam_pos.y;
    let center_z = cam_pos.z - cam_pos.y;
    let half_extent = orthographic_ground_half_extent(zoom, window_width, window_height);
    let edge_pad_tiles = chunk_edge_pad_tiles(chunk_scale);

    let tile_x0 = (center_x - half_extent - edge_pad_tiles).floor() as i32;
    let tile_x1 = (center_x + half_extent + edge_pad_tiles).ceil() as i32;
    let tile_y0 = (center_z - half_extent - edge_pad_tiles).floor() as i32;
    let tile_y1 = (center_z + half_extent + edge_pad_tiles).ceil() as i32;

    // Convert tile AABB to scaled chunk grid.
    // At scale > 1, iterate at coarser granularity (16- or 32-tile steps).
    let s = scaled_tile_span as i32;
    let chunk_x0 = tile_x0.div_euclid(s);
    let chunk_x1 = tile_x1.div_euclid(s) + 1;
    let chunk_y0 = tile_y0.div_euclid(s);
    let chunk_y1 = tile_y1.div_euclid(s) + 1;

    let mut chunks =
        Vec::with_capacity(((chunk_x1 - chunk_x0) * (chunk_y1 - chunk_y0)).max(0) as usize);
    for gx in chunk_x0.max(0)..chunk_x1 {
        for gy in chunk_y0.max(0)..chunk_y1 {
            // Coordinates in the base logical 32×32 grid, aligned to chunk_scale boundaries.
            let base_gx = (gx as u32) * chunk_scale;
            let base_gy = (gy as u32) * chunk_scale;
            // Ensure the ENTIRE super-chunk fits within map bounds.
            // A super-chunk at (base_gx, base_gy) covers base_gx..(base_gx+chunk_scale),
            // so the last base block is (base_gx + chunk_scale - 1).
            if (base_gx + chunk_scale) as i32 <= map_base_chunks_x
                && (base_gy + chunk_scale) as i32 <= map_base_chunks_y
            {
                chunks.push((base_gx, base_gy));
            }
        }
    }
    // Ensure the results are sorted for binary search later in sys_update_worldmap_chunks_to_render.
    chunks.sort_unstable();
    chunks
}

/// The chunks covering `footprint`, in the same form as [`compute_visible_chunks`]: base-grid coordinates
/// aligned to `chunk_scale`, whole super-chunks inside the map only, sorted.
pub(crate) fn chunks_in_tile_rect(
    footprint: &ExportChunkFootprint,
    map_width: u32,
    map_height: u32,
    chunk_scale: u32,
) -> Vec<(u32, u32)> {
    let span = TILE_NUM_PER_CHUNK_DIM * chunk_scale;
    let (map_base_chunks_x, map_base_chunks_y) = (map_width / TILE_NUM_PER_CHUNK_DIM, map_height / TILE_NUM_PER_CHUNK_DIM);
    let mut chunks = Vec::new();
    for gx in footprint.tile_x0 / span..footprint.tile_x1.div_ceil(span) {
        for gy in footprint.tile_y0 / span..footprint.tile_y1.div_ceil(span) {
            let (base_gx, base_gy) = (gx * chunk_scale, gy * chunk_scale);
            if base_gx + chunk_scale <= map_base_chunks_x && base_gy + chunk_scale <= map_base_chunks_y {
                chunks.push((base_gx, base_gy));
            }
        }
    }
    chunks.sort_unstable();
    chunks
}
