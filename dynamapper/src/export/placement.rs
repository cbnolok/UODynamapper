//! Where a frame puts the camera: the job's projection, the tile the camera looks at, where the frame is cut out of the
//! padded capture, and which tiles the frame needs streamed.

use crate::prelude::*;
use crate::render::view::footprint::ExportChunkFootprint;
use super::job::{ExportJob, ExportJobFrame, TILE_HALF_WIDTH_PX};

/// How far below z = 0 a tile can be drawn, in Z steps (UO Z reaches -128).
const LOWEST_Z_STEPS: u32 = 128;
/// Extra target pixels on every side of the frame. The camera always centres a whole tile's top vertex on the
/// target, exactly as the interactive view does, so every frame rasterises the same camera-relative geometry; the
/// frame is then cut out around it. The nearest tile vertex lies within one half-width of the frame centre on
/// each axis, so a full tile width covers the cut on every side.
pub(super) const CAPTURE_PAD_PX: u32 = 2 * TILE_HALF_WIDTH_PX;

/// Where one frame puts the camera, and which tiles it needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FramePlacement {
    /// The tile whose top vertex the camera looks at, at the capture target's centre: the one nearest the frame
    /// centre, so the camera always sits on a whole tile.
    pub(super) look_at_tile: (i64, i64),
    /// The capture target pixel of the frame's top-left corner, where the frame is cut out.
    pub(super) crop_origin: (u32, u32),
    /// Tiles whose sprites can reach the frame, half-open and clamped to the map.
    pub(super) footprint: ExportChunkFootprint,
    /// A tile inside the map near the frame centre; the player stands there and the sidecar reports it.
    pub(super) anchor_tile: (u32, u32),
}

impl FramePlacement {
    pub(super) fn new(job: &ExportJob, frame: &ExportJobFrame) -> Self {
        let half = i64::from(job.tile_half_width_px);
        let size = i64::from(job.frame_size_px);
        let height = i64::from(job.map_height_tiles);
        let z_top = i64::from(job.z_top_px);
        let (centre_x, centre_y) = (frame.left_px + size / 2, frame.top_px + size / 2);
        // s = x − y and t = x + y of the frame centre's z = 0 ground point.
        let s = centre_x as f64 / half as f64 - height as f64;
        let t = (centre_y - z_top) as f64 / half as f64;
        let look_at_tile = (((s + t) / 2.0).round() as i64, ((t - s) / 2.0).round() as i64);
        let (vertex_x, vertex_y) = project(job, look_at_tile.0, look_at_tile.1);
        let target_centre = size / 2 + i64::from(CAPTURE_PAD_PX);
        let crop_origin = (
            (target_centre - (vertex_x - frame.left_px)) as u32,
            (target_centre - (vertex_y - frame.top_px)) as u32,
        );

        // Any sprite reaching the frame has its tile within the frame's z = 0 footprint widened by the deepest
        // Z drop plus the tallest rise, on every side (wide sprites spread sideways too).
        let margin = (i64::from(job.z_top_px) + i64::from(job.z_step_px * LOWEST_Z_STEPS)).div_euclid(half) + 1;
        let s0 = frame.left_px.div_euclid(half) - height - margin;
        let s1 = (frame.left_px + size).div_euclid(half) - height + 1 + margin;
        let t0 = (frame.top_px - z_top).div_euclid(half) - margin;
        let t1 = (frame.top_px + size - z_top).div_euclid(half) + 1 + margin;
        let clamp_x = |value: i64| value.clamp(0, i64::from(job.map_width_tiles)) as u32;
        let clamp_y = |value: i64| value.clamp(0, i64::from(job.map_height_tiles)) as u32;
        let footprint = ExportChunkFootprint {
            tile_x0: clamp_x((s0 + t0).div_euclid(2)),
            tile_x1: clamp_x((s1 + t1).div_euclid(2) + 1),
            tile_y0: clamp_y((t0 - s1).div_euclid(2)),
            tile_y1: clamp_y((t1 - s0).div_euclid(2) + 1),
        };
        let anchor_tile = (
            look_at_tile.0.clamp(0, i64::from(job.map_width_tiles) - 1) as u32,
            look_at_tile.1.clamp(0, i64::from(job.map_height_tiles) - 1) as u32,
        );
        Self { look_at_tile, crop_origin, footprint, anchor_tile }
    }

    pub(super) fn player_position(&self, job: &ExportJob) -> UOVec4 {
        UOVec4::new(self.anchor_tile.0 as u16, self.anchor_tile.1 as u16, 0, job.facet as u8)
    }
}

/// The base-level pixel of tile `(x, y)`'s top vertex at z = 0.
pub(super) fn project(job: &ExportJob, x: i64, y: i64) -> (i64, i64) {
    let half = i64::from(job.tile_half_width_px);
    ((x - y + i64::from(job.map_height_tiles)) * half, (x + y) * half + i64::from(job.z_top_px))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::capture::capture_size_px;
    use super::super::job::tests::{BRITANNIA_HEIGHT, BRITANNIA_WIDTH, job_text, parse};

    #[test]
    fn placement_puts_a_tile_vertex_near_the_frame_centre_on_whole_pixels() {
        let job = parse(&job_text(&std::env::temp_dir(), "")).unwrap();
        // On the diamond, at its left corner, and wholly outside it (no tile can reach the top-left frame).
        for (left_px, top_px) in [(85984, -32), (4064, 8160), (163808, 98272), (-32, 89088), (-32, -32)] {
            let frame = ExportJobFrame { id: "f".into(), left_px, top_px };
            let placement = FramePlacement::new(&job, &frame);
            let (vx, vy) = project(&job, placement.look_at_tile.0, placement.look_at_tile.1);
            let size = i64::from(job.frame_size_px);
            // The vertex lies within one tile of the centre.
            assert!((vx - (left_px + size / 2)).abs() <= 2 * i64::from(TILE_HALF_WIDTH_PX));
            assert!((vy - (top_px + size / 2)).abs() <= 2 * i64::from(TILE_HALF_WIDTH_PX));
            // The cut lies inside the padded target and puts the vertex at the target centre.
            let (cx, cy) = placement.crop_origin;
            assert!(cx <= 2 * CAPTURE_PAD_PX && cy <= 2 * CAPTURE_PAD_PX, "{:?}", placement.crop_origin);
            let centre = i64::from(capture_size_px(job.frame_size_px) / 2);
            assert_eq!((left_px - i64::from(cx) + centre, top_px - i64::from(cy) + centre), (vx, vy));
            // Every tile whose top vertex projects into the frame is inside the footprint.
            let f = placement.footprint;
            assert!(f.tile_x1 <= BRITANNIA_WIDTH && f.tile_y1 <= BRITANNIA_HEIGHT);
            for x in (0..BRITANNIA_WIDTH as i64).step_by(7) {
                for y in (0..BRITANNIA_HEIGHT as i64).step_by(7) {
                    let (px, py) = project(&job, x, y);
                    if px >= left_px && px < left_px + size && py >= top_px && py < top_px + size {
                        assert!((f.tile_x0 as i64..f.tile_x1 as i64).contains(&x) && (f.tile_y0 as i64..f.tile_y1 as i64).contains(&y),
                            "tile {x},{y} of frame {left_px},{top_px} is outside {f:?}");
                    }
                }
            }
        }
    }
}
